//! Power quantities: a branded magnitude, per-board coefficients, and a
//! fleet-wide budget ceiling.
//!
//! Power is *not* a transferable `Mass` dimension and *not* a `SchurConvex`
//! gauge — you cannot migrate watts between nodes; draw is a consequence of
//! where compute lands. It is modeled as a budget the twin rechecks at
//! `Place` apply time (the only move that can raise power under the convex
//! model). See the milestone plan, decision #2.
//!
//! This module holds *type definitions + intrinsic algebra only* (it is
//! dictionary-layer, pure, std-only). Evaluation over fleet/utilization
//! state lives in `src/power_eval.rs` (laboratory).

use std::iter::Sum;
use std::ops::{Add, Mul};

/// A power magnitude, in milliwatts.
///
/// Branded so a function that means "power" cannot silently accept an
/// unrelated `f64` (per `basis.yaml`'s newtype governance). The unit lives
/// in the type, so call sites drop the `_mw` name suffixes.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Power(pub f64);

impl Power {
    /// Zero draw.
    pub const ZERO: Power = Power(0.0);

    /// The underlying magnitude in milliwatts.
    pub fn milliwatts(self) -> f64 {
        self.0
    }
}

impl Add for Power {
    type Output = Power;
    fn add(self, rhs: Power) -> Power {
        Power(self.0 + rhs.0)
    }
}

/// Scale a power magnitude by a dimensionless factor — e.g. the convex
/// `dynamic · util²` term.
impl Mul<f64> for Power {
    type Output = Power;
    fn mul(self, rhs: f64) -> Power {
        Power(self.0 * rhs)
    }
}

impl Sum for Power {
    fn sum<I: Iterator<Item = Power>>(iter: I) -> Power {
        iter.fold(Power::ZERO, Add::add)
    }
}

/// Per-board coefficients for the convex draw model
/// `power(node) = idle + dynamic · worst_utilization(node)²`.
///
/// At milestone 1 these are datasheet/published stand-ins per board model,
/// not measured ground truth; the honest upgrade once real Pis exist is to
/// least-squares fit `idle + dynamic·util²` from sampled wall-power.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PowerCoeffs {
    /// Baseline draw at zero utilization.
    pub idle: Power,
    /// Coefficient on the convex dynamic term; `idle + dynamic` is the
    /// full-load (util = 1.0) draw.
    pub dynamic: Power,
}

/// A fleet-wide power ceiling. The Turing Pi 2 feeds all four node slots
/// from one input, so total draw has a single shared budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PowerBudget(pub Power);

impl PowerBudget {
    /// Whether a proposed total fleet draw fits under the ceiling. A scalar
    /// compare — type-intrinsic, not a fleet evaluation.
    pub fn within(self, draw: Power) -> bool {
        draw <= self.0
    }

    /// The ceiling magnitude.
    pub fn ceiling(self) -> Power {
        self.0
    }
}
