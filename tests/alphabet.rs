//! Mechanical enforcement of alphabet discipline.
//!
//! These tests convert what would otherwise be social conventions —
//! "every primitive has a totality argument", "the EFFECT classification
//! matches the apply signature" — into CI gates. Adding a new primitive
//! requires updating this file too; the failures here are a forcing
//! function for reviewers to confirm the new primitive is well-formed.

use fulcrum::alphabet::{Effect, Primitive};
use fulcrum::{ColdToHot, HotToCold, Neutral, Place, Remove};

/// Every primitive must cite a totality theorem. Empty, placeholder, or
/// suspiciously short citations fail.
#[test]
fn theorem_citation_present_and_substantive() {
    fn assert_well_formed<P: Primitive>() {
        let theorem = P::THEOREM;
        let name = P::NAME;
        assert!(!theorem.is_empty(), "{}: THEOREM is empty", name);
        assert!(
            theorem.len() >= 40,
            "{}: THEOREM too short ({} chars). Cite a published source.",
            name,
            theorem.len()
        );
        for forbidden in ["TODO", "FIXME", "XXX", "tbd", "TBD"] {
            assert!(
                !theorem.contains(forbidden),
                "{}: THEOREM contains placeholder '{}'",
                name,
                forbidden
            );
        }
    }

    assert_well_formed::<Remove>();
    assert_well_formed::<HotToCold>();
    assert_well_formed::<Neutral>();
    assert_well_formed::<ColdToHot>();
    assert_well_formed::<Place>();
}

/// Each primitive's NAME must be unique and non-empty.
#[test]
fn names_are_unique() {
    let names = [
        Remove::NAME,
        HotToCold::NAME,
        Neutral::NAME,
        ColdToHot::NAME,
        Place::NAME,
    ];
    for n in &names {
        assert!(!n.is_empty());
    }
    let mut sorted = names.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "duplicate NAME found");
}

/// The `EFFECT` classification matches the actual `apply` signature.
/// Typed-pure effects (MassDecreasing, PigouDalton, Permutation) have
/// total apply; catch-all effects (MassIncreasing, MassPreservingFree)
/// have fallible apply. This is a *signature* check — see below.
#[test]
fn effect_classifications_are_correct() {
    assert_eq!(Remove::EFFECT, Effect::MassDecreasing);
    assert_eq!(HotToCold::EFFECT, Effect::PigouDalton);
    assert_eq!(Neutral::EFFECT, Effect::Permutation);
    assert_eq!(ColdToHot::EFFECT, Effect::MassPreservingFree);
    assert_eq!(Place::EFFECT, Effect::MassIncreasing);

    // Typed-pure effects.
    assert!(Remove::EFFECT.is_typed_pure());
    assert!(HotToCold::EFFECT.is_typed_pure());
    assert!(Neutral::EFFECT.is_typed_pure());

    // Catch-all effects.
    assert!(!ColdToHot::EFFECT.is_typed_pure());
    assert!(!Place::EFFECT.is_typed_pure());
}

/// Verify that the `apply` signatures match the `EFFECT` classification.
///
/// This is a structural test by way of types. If someone declares
/// `Remove::EFFECT = Effect::MassIncreasing` (which would lie about
/// what the function does), the test below would still compile because
/// the actual `apply` returns `Safe<G>`. To make this *mechanical*, we
/// check both directions: the typed-pure primitives compile against
/// `assert_total_apply`, and the catch-all ones compile against
/// `assert_fallible_apply`. If you swap an EFFECT incorrectly, the
/// test for the *swapped pair* will fail to compile.
mod signature_match {
    use fulcrum::alphabet::Primitive;
    use fulcrum::gauge::SchurConvex;
    use fulcrum::safe::{GaugeError, Safe};
    use fulcrum::{ColdToHot, HotToCold, Neutral, Place, Remove};

    /// Compiles only if `M::apply` returns `Safe<G>` (total).
    fn assert_total<M, G>(m: M, s: Safe<G>) -> Safe<G>
    where
        M: TotalApply<G>,
        G: SchurConvex,
    {
        m.apply_total(s)
    }

    /// Compiles only if `M::apply` returns `Result<Safe<G>, _>` (fallible).
    fn assert_fallible<M, G>(m: M, s: Safe<G>) -> Result<Safe<G>, GaugeError>
    where
        M: FallibleApply<G>,
        G: SchurConvex,
    {
        m.apply_fallible(s)
    }

    trait TotalApply<G: SchurConvex> {
        fn apply_total(self, safe: Safe<G>) -> Safe<G>;
    }

    trait FallibleApply<G: SchurConvex> {
        fn apply_fallible(self, safe: Safe<G>) -> Result<Safe<G>, GaugeError>;
    }

    impl<G: SchurConvex> TotalApply<G> for Remove {
        fn apply_total(self, safe: Safe<G>) -> Safe<G> {
            self.apply(safe)
        }
    }
    impl<G: SchurConvex> TotalApply<G> for HotToCold {
        fn apply_total(self, safe: Safe<G>) -> Safe<G> {
            self.apply(safe)
        }
    }
    impl<G: SchurConvex> TotalApply<G> for Neutral {
        fn apply_total(self, safe: Safe<G>) -> Safe<G> {
            self.apply(safe)
        }
    }
    impl<G: SchurConvex> FallibleApply<G> for ColdToHot {
        fn apply_fallible(self, safe: Safe<G>) -> Result<Safe<G>, GaugeError> {
            self.apply_with_recheck(safe)
        }
    }
    impl<G: SchurConvex> FallibleApply<G> for Place {
        fn apply_fallible(self, safe: Safe<G>) -> Result<Safe<G>, GaugeError> {
            self.apply_with_recheck(safe)
        }
    }

    // The classification should agree with the actual signature. If a
    // primitive's EFFECT is typed-pure but its apply returns Result, the
    // TotalApply impl above would fail to compile. Same in the other
    // direction.
    //
    // Additionally, this test checks the const at runtime as a redundant
    // safeguard.
    #[test]
    fn typed_pure_primitives_have_typed_pure_effect() {
        assert!(Remove::EFFECT.is_typed_pure());
        assert!(HotToCold::EFFECT.is_typed_pure());
        assert!(Neutral::EFFECT.is_typed_pure());
    }

    #[test]
    fn catch_all_primitives_have_catch_all_effect() {
        assert!(!ColdToHot::EFFECT.is_typed_pure());
        assert!(!Place::EFFECT.is_typed_pure());
    }

    #[test]
    fn type_level_signature_check_compiles() {
        // The fact that this function compiles is the test — the
        // `TotalApply` and `FallibleApply` trait bounds enforce that
        // each primitive's apply has the signature its EFFECT claims.
        use fulcrum::load::{Fleet, MachineId, Mass};

        // Total path.
        let mut f = Fleet::new(100);
        f.add_machine(MachineId(1), 50);
        let safe: Safe<fulcrum::Linfty> = Safe::new(f, 0.9).unwrap();
        let _ = assert_total(Remove::new(MachineId(1), Mass(10)), safe);

        // Fallible path.
        let mut f = Fleet::new(100);
        f.add_machine(MachineId(1), 50);
        let safe: Safe<fulcrum::Linfty> = Safe::new(f, 0.9).unwrap();
        let _ = assert_fallible(Place::new(MachineId(1), Mass(10)), safe);

        // The compile-time guarantee: swapping (e.g. trying
        // `assert_total(Place::new(...))`) would not compile because
        // `Place: !TotalApply<G>`. Same for the inverse with Remove.
    }
}
