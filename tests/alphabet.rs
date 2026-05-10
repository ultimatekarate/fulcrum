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

    assert_well_formed::<Remove<1>>();
    assert_well_formed::<HotToCold<1>>();
    assert_well_formed::<Neutral<1>>();
    assert_well_formed::<ColdToHot<1>>();
    assert_well_formed::<Place<1>>();
    // Spot-check at a higher dim — the impls are blanket over N so the
    // citations are the same, but verifying reminds reviewers that the
    // theorem must apply across all dimensions.
    assert_well_formed::<Remove<3>>();
    assert_well_formed::<HotToCold<3>>();
}

/// Each primitive's NAME must be unique and non-empty.
#[test]
fn names_are_unique() {
    let names = [
        Remove::<1>::NAME,
        HotToCold::<1>::NAME,
        Neutral::<1>::NAME,
        ColdToHot::<1>::NAME,
        Place::<1>::NAME,
    ];
    for n in &names {
        assert!(!n.is_empty());
    }
    let mut sorted = names.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "duplicate NAME found");
}

#[test]
fn effect_classifications_are_correct() {
    assert_eq!(Remove::<1>::EFFECT, Effect::MassDecreasing);
    assert_eq!(HotToCold::<1>::EFFECT, Effect::PigouDalton);
    assert_eq!(Neutral::<1>::EFFECT, Effect::Permutation);
    assert_eq!(ColdToHot::<1>::EFFECT, Effect::MassPreservingFree);
    assert_eq!(Place::<1>::EFFECT, Effect::MassIncreasing);

    assert!(Remove::<1>::EFFECT.is_typed_pure());
    assert!(HotToCold::<1>::EFFECT.is_typed_pure());
    assert!(Neutral::<1>::EFFECT.is_typed_pure());

    assert!(!ColdToHot::<1>::EFFECT.is_typed_pure());
    assert!(!Place::<1>::EFFECT.is_typed_pure());
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

    /// Compiles only if `M::apply` returns `Safe<G, N>` (total).
    fn assert_total<M, G, const N: usize>(m: M, s: Safe<G, N>) -> Safe<G, N>
    where
        M: TotalApply<G, N>,
        G: SchurConvex<N>,
    {
        m.apply_total(s)
    }

    /// Compiles only if `M::apply` returns `Result<Safe<G, N>, _>` (fallible).
    fn assert_fallible<M, G, const N: usize>(
        m: M,
        s: Safe<G, N>,
    ) -> Result<Safe<G, N>, GaugeError>
    where
        M: FallibleApply<G, N>,
        G: SchurConvex<N>,
    {
        m.apply_fallible(s)
    }

    trait TotalApply<G: SchurConvex<N>, const N: usize> {
        fn apply_total(self, safe: Safe<G, N>) -> Safe<G, N>;
    }

    trait FallibleApply<G: SchurConvex<N>, const N: usize> {
        fn apply_fallible(self, safe: Safe<G, N>) -> Result<Safe<G, N>, GaugeError>;
    }

    impl<G: SchurConvex<N>, const N: usize> TotalApply<G, N> for Remove<N> {
        fn apply_total(self, safe: Safe<G, N>) -> Safe<G, N> {
            self.apply(safe)
        }
    }
    impl<G: SchurConvex<N>, const N: usize> TotalApply<G, N> for HotToCold<N> {
        fn apply_total(self, safe: Safe<G, N>) -> Safe<G, N> {
            self.apply(safe)
        }
    }
    impl<G: SchurConvex<N>, const N: usize> TotalApply<G, N> for Neutral<N> {
        fn apply_total(self, safe: Safe<G, N>) -> Safe<G, N> {
            self.apply(safe)
        }
    }
    impl<G: SchurConvex<N>, const N: usize> FallibleApply<G, N> for ColdToHot<N> {
        fn apply_fallible(self, safe: Safe<G, N>) -> Result<Safe<G, N>, GaugeError> {
            self.apply_with_recheck(safe)
        }
    }
    impl<G: SchurConvex<N>, const N: usize> FallibleApply<G, N> for Place<N> {
        fn apply_fallible(self, safe: Safe<G, N>) -> Result<Safe<G, N>, GaugeError> {
            self.apply_with_recheck(safe)
        }
    }

    #[test]
    fn typed_pure_primitives_have_typed_pure_effect() {
        assert!(Remove::<1>::EFFECT.is_typed_pure());
        assert!(HotToCold::<1>::EFFECT.is_typed_pure());
        assert!(Neutral::<1>::EFFECT.is_typed_pure());
    }

    #[test]
    fn catch_all_primitives_have_catch_all_effect() {
        assert!(!ColdToHot::<1>::EFFECT.is_typed_pure());
        assert!(!Place::<1>::EFFECT.is_typed_pure());
    }

    #[test]
    fn type_level_signature_check_compiles() {
        use fulcrum::load::{Fleet, MachineId, Mass};

        // Total path.
        let mut f: Fleet<1> = Fleet::new();
        f.add_machine(MachineId(1), [100], [50]);
        let safe: Safe<fulcrum::Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();
        let _ = assert_total(Remove::new(MachineId(1), Mass([10])), safe);

        // Fallible path.
        let mut f: Fleet<1> = Fleet::new();
        f.add_machine(MachineId(1), [100], [50]);
        let safe: Safe<fulcrum::Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();
        let _ = assert_fallible(Place::new(MachineId(1), Mass([10])), safe);

        // Multi-dim variant — same trait bounds, just different N.
        let mut f2: Fleet<2> = Fleet::new();
        f2.add_machine(MachineId(1), [100, 100], [50, 30]);
        let safe2: Safe<fulcrum::Linfty<2>, 2> = Safe::new(f2, 0.9).unwrap();
        let _ = assert_total(Remove::new(MachineId(1), Mass([5, 5])), safe2);
    }
}
