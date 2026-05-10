//! Replay support: classify and apply a stream of fleet events through
//! the typed framework.
//!
//! This module exists to make `examples/borg_replay.rs` testable. It is
//! *not* part of the framework's algebraic core — it's a thin layer that
//! consumes a stream of `ParsedEvent`s and routes each one into the move
//! alphabet.
//!
//! The replay pipeline is pinned to `N = 1` because the Google 2019 Borg
//! trace subset only carries CPU demand. A multi-dim replay would
//! require trace data with both CPU and memory per event; that's a
//! Phase 4 (reconciler / event-stream generalization) concern, not a
//! Phase 2 one.
//!
//! Pre-1.0; subject to change. Not stable API.

use std::collections::HashMap;

use crate::gauge::SchurConvex;
use crate::load::{Fleet, MachineId, Mass};
use crate::move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};
use crate::safe::{GaugeError, Safe};
use crate::trace::{MoveHistory, MoveRecord};

/// What kind of state-changing event this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Instance is being scheduled / re-scheduled.
    Schedule,
    /// Instance is being removed (FINISH/FAIL/KILL/LOST/EVICT in Borg).
    Remove,
}

/// One event from the trace, parsed into the form the classifier consumes.
#[derive(Clone, Debug)]
pub struct ParsedEvent {
    /// Microseconds from trace start. Events must be processed in time order;
    /// see [`classify_and_apply`] for the in-place sort it performs.
    pub time: u64,
    pub kind: EventKind,
    /// Stable instance identifier (e.g., `(collection_id, instance_index)`).
    pub instance: String,
    /// Destination machine for `Schedule`; ignored for `Remove`.
    pub machine: MachineId,
    /// Resource demand for `Schedule`; ignored for `Remove`. 1D — see
    /// the module docs for why.
    pub mass: Mass<1>,
}

/// Classification counters. Each event the classifier processes is counted
/// in exactly one bucket — except events that are fully unprocessable
/// (e.g., REMOVE for an instance we've never seen), which fall into
/// `remove_unobservable`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub remove: u64,
    pub remove_unobservable: u64,
    pub place: u64,
    pub hot_to_cold: u64,
    pub neutral: u64,
    pub cold_to_hot: u64,
    /// Events skipped because applying them would have produced a fleet error
    /// (insufficient load, unknown machine). These indicate bookkeeping
    /// divergence between the visible event stream and the actual fleet.
    pub well_formedness_skipped: u64,
}

impl Counters {
    /// Events that ran through the framework's typed-pure paths.
    pub fn applied_typed_pure(&self) -> u64 {
        self.remove + self.hot_to_cold + self.neutral
    }

    /// Events that ran through the framework's catch-all paths.
    pub fn applied_catch_all(&self) -> u64 {
        self.place + self.cold_to_hot
    }

    /// All events the framework actually applied.
    pub fn applied_total(&self) -> u64 {
        self.applied_typed_pure() + self.applied_catch_all()
    }

    /// Typed-pure ratio over events the framework actually applied. This is
    /// the load-bearing metric — events that didn't make it through the
    /// framework (unobservable removes, well-formedness skips) are
    /// excluded.
    pub fn applied_typed_pure_ratio(&self) -> f64 {
        let total = self.applied_total();
        if total == 0 {
            0.0
        } else {
            self.applied_typed_pure() as f64 / total as f64
        }
    }
}

/// Result of running the classifier.
pub struct ReplayResult<G: SchurConvex<1>> {
    pub safe: Safe<G, 1>,
    pub history: MoveHistory<1>,
    pub counters: Counters,
}

/// Tracked state for one instance the classifier has seen scheduled.
#[derive(Clone, Copy, Debug)]
struct InstanceState {
    machine: MachineId,
    mass: Mass<1>,
}

/// Classify a stream of events and apply them through the typed framework.
///
/// Sorts `events` in-place by time (events arrive out-of-order in some
/// traces, including the Google 2019 Borg subset). Pre-populates the fleet
/// with all machines mentioned by Schedule events at zero load, then walks
/// events in time order, routing each through the framework.
///
/// Threshold is `f64::INFINITY` — the replay is for *classification*, not
/// for enforcement; we don't want trace events that happen to have exceeded
/// capacity in the original Borg run to abort the replay.
pub fn classify_and_apply<G: SchurConvex<1> + Default>(
    mut events: Vec<ParsedEvent>,
    capacity: u64,
) -> Result<ReplayResult<G>, GaugeError> {
    // Sort by time — events in the source trace are not pre-sorted. Without
    // this, REMOVE for an instance whose SCHEDULE appears later in the file
    // misclassifies as remove_unobservable, and that SCHEDULE later
    // misclassifies as a fresh PLACE.
    events.sort_by_key(|e| e.time);

    // Pre-populate the fleet with every machine mentioned by a Schedule.
    // The Borg trace doesn't include per-machine capacity; the `capacity`
    // argument is a uniform value applied to every machine.
    let mut fleet: Fleet<1> = Fleet::new();
    let mut seen_machines: std::collections::HashSet<u64> =
        std::collections::HashSet::new();
    for ev in &events {
        if ev.kind == EventKind::Schedule && seen_machines.insert(ev.machine.0) {
            fleet.add_machine(ev.machine, [capacity], [0]);
        }
    }

    let mut safe: Safe<G, 1> = Safe::new(fleet, f64::INFINITY)?;
    let mut history: MoveHistory<1> = MoveHistory::new();
    let mut instances: HashMap<String, InstanceState> = HashMap::new();
    let mut counters = Counters::default();

    for ev in events {
        match ev.kind {
            EventKind::Schedule => {
                let machine = ev.machine;
                let mass = ev.mass;
                let inst = ev.instance.clone();

                if let Some(prior) = instances.get(&inst).copied() {
                    if prior.machine == machine {
                        // Reschedule on same machine; fleet state unchanged.
                        continue;
                    }
                    // Migration. Try HotToCold, then Neutral, fall through
                    // to ColdToHot. Always migrate `prior.mass` to keep
                    // bookkeeping consistent with the fleet state we own.
                    let src_load = safe.fleet().load(prior.machine).map(|l| l[0]).unwrap_or(0);
                    let dst_load = safe.fleet().load(machine).map(|l| l[0]).unwrap_or(0);

                    if src_load > dst_load {
                        if let Some(m) = HotToCold::witness(
                            prior.machine,
                            machine,
                            prior.mass,
                            safe.fleet(),
                        ) {
                            safe = m.apply(safe);
                            history.push(MoveRecord::HotToCold {
                                source: prior.machine,
                                destination: machine,
                                mass: prior.mass,
                            });
                            counters.hot_to_cold += 1;
                            instances.insert(
                                inst,
                                InstanceState { machine, mass: prior.mass },
                            );
                            continue;
                        }
                    } else if src_load == dst_load {
                        if let Some(m) = Neutral::witness(
                            prior.machine,
                            machine,
                            prior.mass,
                            safe.fleet(),
                        ) {
                            safe = m.apply(safe);
                            history.push(MoveRecord::Neutral {
                                source: prior.machine,
                                destination: machine,
                                mass: prior.mass,
                            });
                            counters.neutral += 1;
                            instances.insert(
                                inst,
                                InstanceState { machine, mass: prior.mass },
                            );
                            continue;
                        }
                    }

                    // Catch-all migration via ColdToHot.
                    let m = ColdToHot::new(prior.machine, machine, prior.mass);
                    match m.apply_with_recheck(safe) {
                        Ok(s) => {
                            safe = s;
                            history.push(MoveRecord::ColdToHot {
                                source: prior.machine,
                                destination: machine,
                                mass: prior.mass,
                            });
                            counters.cold_to_hot += 1;
                            instances.insert(
                                inst,
                                InstanceState { machine, mass: prior.mass },
                            );
                        }
                        Err(GaugeError::ThresholdExceeded { .. }) => {
                            unreachable!("infinite threshold cannot be exceeded")
                        }
                        Err(e) => {
                            // Well-formedness error means bookkeeping
                            // diverged from the fleet. Bail loudly — a
                            // skipped move would silently invalidate every
                            // subsequent classification.
                            panic!(
                                "ColdToHot::apply failed with {:?}; \
                                 bookkeeping diverged from fleet state",
                                e
                            )
                        }
                    }
                } else {
                    // Fresh placement.
                    match Place::new(machine, mass).apply_with_recheck(safe) {
                        Ok(s) => {
                            safe = s;
                            history.push(MoveRecord::Place { machine, mass });
                            counters.place += 1;
                            instances.insert(
                                inst,
                                InstanceState { machine, mass },
                            );
                        }
                        Err(GaugeError::ThresholdExceeded { .. }) => {
                            unreachable!("infinite threshold cannot be exceeded")
                        }
                        Err(e) => {
                            panic!(
                                "Place::apply failed with {:?}; this should \
                                 not happen for a pre-registered machine",
                                e
                            )
                        }
                    }
                }
            }
            EventKind::Remove => {
                if let Some(prior) = instances.remove(&ev.instance) {
                    let r = Remove::new(prior.machine, prior.mass);
                    safe = apply_remove_or_skip(r, safe, &mut counters);
                    history.push(MoveRecord::Remove {
                        machine: prior.machine,
                        mass: prior.mass,
                    });
                    counters.remove += 1;
                } else {
                    counters.remove_unobservable += 1;
                }
            }
        }
    }

    Ok(ReplayResult { safe, history, counters })
}

/// Apply a `Remove`. Total in the framework, but we wrap it so a
/// well-formedness error (e.g., the bookkeeping diverged from the fleet)
/// can be surfaced rather than panic in `expect`. In practice, this only
/// fires if the trace data is internally inconsistent; for the Borg
/// subset, it should never fire.
fn apply_remove_or_skip<G: SchurConvex<1>>(
    r: Remove<1>,
    safe: Safe<G, 1>,
    counters: &mut Counters,
) -> Safe<G, 1> {
    // Pre-check the remove against the fleet to avoid panicking inside
    // Remove::apply, which is total at the type level but has internal
    // expects on well-formedness.
    let load = safe.fleet().load(r.machine).map(|l| l[0]).unwrap_or(0);
    if load < r.mass.0[0] {
        counters.well_formedness_skipped += 1;
        // Drop the remove. The instance is removed from `instances` already;
        // the fleet stays as-is.
        return safe;
    }
    r.apply(safe)
}

/// Convenience: compute mass from a normalized cpus value (in [0, 1+]).
pub fn cpus_to_mass(cpus: f64, scale: u64) -> Mass<1> {
    if cpus.is_nan() || cpus < 0.0 {
        Mass([0])
    } else {
        Mass([(cpus * scale as f64).round() as u64])
    }
}

/// Parse `'cpus': X.XXX` out of a Python-dict-formatted resource_request.
/// Returns `None` if the key isn't present or the value is unparseable.
pub fn parse_cpus_from_resource_request(s: &str) -> Option<f64> {
    let key = "'cpus':";
    let i = s.find(key)?;
    let after = s[i + key.len()..].trim_start();
    let end = after
        .find(|c: char| {
            !c.is_ascii_digit() && c != '.' && c != 'e' && c != 'E' && c != '-' && c != '+'
        })
        .unwrap_or(after.len());
    after[..end].parse().ok()
}

/// Parse a Borg-trace CSV at `path` into the `ParsedEvent` stream the
/// classifier consumes. Skips events that don't affect fleet state
/// (`ENABLE`, `UPDATE_*`, `QUEUE`) and events with no/zero mass on a
/// known machine.
///
/// Reports `(events, dropped)` where `dropped` is a coarse count of rows
/// that were not turned into ParsedEvents. Useful for diagnostics.
pub fn parse_csv(path: &std::path::Path) -> Result<(Vec<ParsedEvent>, u64), Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::BufReader;

    let mut rdr = csv::Reader::from_reader(BufReader::new(File::open(path)?));
    let headers = rdr.headers()?.clone();
    let col = |name: &str| -> Result<usize, String> {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| format!("missing column '{}'", name))
    };
    let c_time = col("time")?;
    let c_event = col("event")?;
    let c_machine = col("machine_id")?;
    let c_collection = col("collection_id")?;
    let c_index = col("instance_index")?;
    let c_request = col("resource_request")?;

    let scale: u64 = 1_000_000;
    let mut events = Vec::with_capacity(450_000);
    let mut dropped = 0_u64;

    for rec in rdr.records() {
        let rec = rec?;
        let event = rec.get(c_event).unwrap_or("");
        let kind = match event {
            "SCHEDULE" => EventKind::Schedule,
            "FINISH" | "FAIL" | "KILL" | "LOST" | "EVICT" => EventKind::Remove,
            _ => {
                dropped += 1;
                continue;
            }
        };
        let time = rec.get(c_time).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let inst = format!(
            "{}|{}",
            rec.get(c_collection).unwrap_or(""),
            rec.get(c_index).unwrap_or("")
        );
        let machine_u64 = rec.get(c_machine).and_then(|s| s.parse::<u64>().ok());
        let machine = match (kind, machine_u64) {
            (EventKind::Schedule, Some(m)) if m != 0 => MachineId(m),
            (EventKind::Remove, Some(m)) if m != 0 => MachineId(m),
            (EventKind::Remove, _) => {
                // Remove with no machine — still useful, we'll look up by
                // instance. Use a sentinel; the classifier only reads
                // machine for Schedule events.
                MachineId(0)
            }
            _ => {
                dropped += 1;
                continue;
            }
        };
        let mass = match kind {
            EventKind::Schedule => {
                let cpus = parse_cpus_from_resource_request(rec.get(c_request).unwrap_or(""))
                    .unwrap_or(0.0);
                let m = cpus_to_mass(cpus, scale);
                if m.0[0] == 0 {
                    dropped += 1;
                    continue;
                }
                m
            }
            EventKind::Remove => Mass([0]), // unused for remove
        };

        events.push(ParsedEvent { time, kind, instance: inst, machine, mass });
    }

    Ok((events, dropped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gauge::Linfty;

    fn ev_schedule(time: u64, inst: &str, machine: u64, mass: u64) -> ParsedEvent {
        ParsedEvent {
            time,
            kind: EventKind::Schedule,
            instance: inst.to_string(),
            machine: MachineId(machine),
            mass: Mass([mass]),
        }
    }

    fn ev_remove(time: u64, inst: &str) -> ParsedEvent {
        ParsedEvent {
            time,
            kind: EventKind::Remove,
            instance: inst.to_string(),
            machine: MachineId(0),
            mass: Mass([0]),
        }
    }

    #[test]
    fn empty_stream() {
        let r: ReplayResult<Linfty<1>> = classify_and_apply(vec![], 100).unwrap();
        assert_eq!(r.counters, Counters::default());
    }

    #[test]
    fn place_then_remove_classifies_both() {
        let events = vec![
            ev_schedule(1, "a", 1, 30),
            ev_remove(2, "a"),
        ];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.place, 1);
        assert_eq!(r.counters.remove, 1);
        assert_eq!(r.counters.applied_total(), 2);
        assert!((r.counters.applied_typed_pure_ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn remove_for_unseen_instance_is_unobservable() {
        let events = vec![ev_remove(1, "ghost")];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.remove_unobservable, 1);
        assert_eq!(r.counters.applied_total(), 0);
    }

    #[test]
    fn migrate_hot_to_cold() {
        // Place a heavy instance on m1, a smaller one on m2, then migrate the
        // smaller one to m1's neighbor in a HotToCold direction. We need
        // src_load > dst_load AND mass ≤ src_load - dst_load.
        //
        // Setup: m1 has 60 (heavy), m2 has 0. Place a small instance "small"
        // on m1 with mass=20. m1 now 60+20=80. Then migrate "small" to m2:
        // src=80, dst=0, gap=80, mass=20 ≤ 80 → HotToCold.
        let events = vec![
            ev_schedule(1, "heavy", 1, 60),
            ev_schedule(2, "small", 1, 20),
            ev_schedule(3, "small", 2, 20),
        ];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.place, 2);
        assert_eq!(r.counters.hot_to_cold, 1);
        assert_eq!(r.counters.cold_to_hot, 0);
    }

    #[test]
    fn migrate_cold_to_hot() {
        // Reverse: place light on m1, heavy on m2, migrate light from m1 to m2.
        // src_load=10, dst_load=50. ColdToHot.
        let events = vec![
            ev_schedule(1, "light", 1, 10),
            ev_schedule(2, "heavy", 2, 50),
            ev_schedule(3, "light", 2, 10),
        ];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.place, 2);
        assert_eq!(r.counters.cold_to_hot, 1);
        assert_eq!(r.counters.hot_to_cold, 0);
    }

    #[test]
    fn migrate_neutral_when_loads_equal() {
        // Two instances of the same mass on different machines, then move one
        // to the other's machine. Wait — that would put both on one machine.
        // Better: three machines with two equal-mass instances each in a way
        // that produces equal loads at migration time.
        //
        // Setup: m1=10, m2=10. Migrate from m1 to m3 (m3=0). HotToCold.
        // Place on m3=10. Now m1=0, m2=10, m3=10. Migrate from m2 to m3 with
        // src=10, dst=10. Neutral.
        let events = vec![
            ev_schedule(1, "a", 1, 10),
            ev_schedule(2, "b", 2, 10),
            ev_schedule(3, "a", 3, 10),  // HotToCold (m1=10 > m3=0)
            ev_schedule(4, "b", 3, 10),  // m2=10 == m3=10, Neutral
        ];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.place, 2);
        assert_eq!(r.counters.hot_to_cold, 1);
        assert_eq!(r.counters.neutral, 1);
    }

    #[test]
    fn time_sort_corrects_out_of_order_input() {
        // Same events as place_then_remove but with the remove first in the
        // input vector. The sort should put schedule first.
        let events = vec![
            ev_remove(2, "a"),
            ev_schedule(1, "a", 1, 30),
        ];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.place, 1);
        assert_eq!(r.counters.remove, 1);
        assert_eq!(r.counters.remove_unobservable, 0);
    }

    #[test]
    fn same_machine_reschedule_no_op() {
        let events = vec![
            ev_schedule(1, "a", 1, 30),
            ev_schedule(2, "a", 1, 30),  // reschedule on same machine
        ];
        let r: ReplayResult<Linfty<1>> = classify_and_apply(events, 100).unwrap();
        assert_eq!(r.counters.place, 1);
        assert_eq!(r.counters.hot_to_cold, 0);
        assert_eq!(r.counters.neutral, 0);
        assert_eq!(r.counters.cold_to_hot, 0);
    }

    #[test]
    fn parse_cpus_handles_dict_format() {
        assert_eq!(
            parse_cpus_from_resource_request("{'cpus': 0.020660400390625, 'memory': 0.014434814453125}"),
            Some(0.020660400390625)
        );
        assert_eq!(
            parse_cpus_from_resource_request("{'cpus': 1.23e-5, 'memory': 0.001}"),
            Some(1.23e-5)
        );
        assert_eq!(parse_cpus_from_resource_request("{'memory': 0.5}"), None);
    }
}
