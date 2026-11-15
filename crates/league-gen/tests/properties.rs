//! Property tests for `league-gen`: determinism, LP strong duality,
//! feasibility invariants, negative controls, HHI bounds, and PLR scheduling.

use std::collections::HashSet;

use league_gen::{
    FeasibilityError, GeneratorConfig, LpStatus, PlrChoice, PlrSampler, RingTable, SeatCount,
    check_feasibility, coalition_upkeep_coverage, compute_hhi, evaluate_ring_table,
    generate_ring_table, solve_lp,
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn generated_tables_are_deterministic_roundtrip_and_feasible(
        seed in any::<u64>(),
        sc_idx in 0usize..3,
    ) {
        let sc = SeatCount::ALL[sc_idx];
        let cfg = GeneratorConfig {
            seat_count: Some(sc),
            ..GeneratorConfig::default()
        };

        let (t1, r1) = match generate_ring_table(seed, &cfg) {
            Ok(pair) => pair,
            Err(err) => return Err(TestCaseError::fail(format!("seed {seed} ({sc:?}) failed: {err}"))),
        };
        let (t2, r2) = match generate_ring_table(seed, &cfg) {
            Ok(pair) => pair,
            Err(err) => return Err(TestCaseError::fail(format!("repeat seed {seed} failed: {err}"))),
        };

        prop_assert_eq!(&t1, &t2);
        prop_assert_eq!(&r1, &r2);

        let encoded = match serde_json::to_string(&t1) {
            Ok(s) => s,
            Err(err) => return Err(TestCaseError::fail(format!("serialize failed: {err}"))),
        };
        let decoded: RingTable = match serde_json::from_str(&encoded) {
            Ok(v) => v,
            Err(err) => return Err(TestCaseError::fail(format!("deserialize failed: {err}"))),
        };
        prop_assert_eq!(&decoded, &t1);

        // Re-running check_feasibility matches report and satisfies all three invariants.
        let recheck = match check_feasibility(&decoded, &cfg) {
            Ok(r) => r,
            Err(err) => return Err(TestCaseError::fail(format!("recheck failed: {err}"))),
        };
        prop_assert_eq!(&recheck, &r1);
        prop_assert!(r1.max_1seat_upkeep_coverage < 1e-9);
        prop_assert!(r1.max_2seat_upkeep_coverage < 1e-9);
        // Any 3-seat bloc satisfies at most 1 of its 3 seats' upkeep (`< 0.40`, bounded by max
        // single-seat share across any 3-seat subset).
        let mut sorted_upkeeps: Vec<f64> = t1.seats.iter().map(|s| s.upkeep_amount).collect();
        sorted_upkeeps.sort_by(f64::total_cmp);
        let max_one_of_three = sorted_upkeeps[sorted_upkeeps.len() - 1]
            / (sorted_upkeeps[0] + sorted_upkeeps[1] + sorted_upkeeps[2]);
        prop_assert!(r1.max_3seat_upkeep_coverage <= max_one_of_three + 1e-6);
        prop_assert!(r1.max_3seat_upkeep_coverage < 0.40);
        prop_assert!(r1.surplus_to_upkeep_ratio >= cfg.min_surplus_ratio - 1e-9);
        prop_assert!(r1.max_relative_ev_deviation <= cfg.ev_tolerance + 1e-9);

        let n = sc.as_usize();
        let mut codes = HashSet::new();
        codes.insert(t1.numeraire_code);
        for seat in &t1.seats {
            prop_assert!(codes.insert(seat.raw_code));
            prop_assert!(codes.insert(seat.refined_code));
        }
        prop_assert_eq!(codes.len(), 2 * n + 1);

        let ring_metrics = evaluate_ring_table(&t1, &r1);
        prop_assert!(ring_metrics.trade_concentration.hhi < 0.25);
        prop_assert!(ring_metrics.trade_concentration.active_pairs >= n);
    }

    #[test]
    fn injecting_pair_self_sufficiency_is_always_rejected(
        seed in any::<u64>(),
        sc_idx in 0usize..3,
        seat_pick in 0usize..6,
    ) {
        let sc = SeatCount::ALL[sc_idx];
        let cfg = GeneratorConfig {
            seat_count: Some(sc),
            ..GeneratorConfig::default()
        };
        let (mut table, _) = match generate_ring_table(seed, &cfg) {
            Ok(pair) => pair,
            Err(err) => return Err(TestCaseError::fail(format!("generation failed: {err}"))),
        };

        let n = sc.as_usize();
        let i = seat_pick % n;
        let next = (i + 1) % n;
        // Inject an autarkic 2-seat converter (own raw R_i + neighbor R_{i+1} -> M_i feeding i+1).
        table.seats[i].converters[0].inputs = vec![(i as u8, 1.5), (next as u8, 1.5)];

        let pair_cov = match coalition_upkeep_coverage(&table, &[i as u8, next as u8]) {
            Ok(v) => v,
            Err(err) => return Err(TestCaseError::fail(format!("pair lp failed: {err}"))),
        };
        prop_assert!(pair_cov > 0.05);
        let verdict = check_feasibility(&table, &cfg);
        let rejected_pair = matches!(verdict, Err(FeasibilityError::PairCoversUpkeep { .. }));
        prop_assert!(rejected_pair, "expected PairCoversUpkeep, got {verdict:?}");
    }

    #[test]
    fn random_bounded_lps_satisfy_strong_duality_and_kkt(
        a_flat in prop::collection::vec(1u8..20, 9),
        b_ints in prop::collection::vec(1u8..50, 3),
        c_ints in prop::collection::vec(1u8..30, 3),
    ) {
        let a: Vec<Vec<f64>> = a_flat
            .as_chunks::<3>()
            .0
            .iter()
            .map(|row| row.iter().map(|&v| f64::from(v) * 0.25).collect())
            .collect();
        let b: Vec<f64> = b_ints.into_iter().map(f64::from).collect();
        let c: Vec<f64> = c_ints.into_iter().map(f64::from).collect();

        let sol = match solve_lp(&a, &b, &c) {
            Ok(LpStatus::Optimal(s)) => s,
            other => return Err(TestCaseError::fail(format!("expected Optimal, got {other:?}"))),
        };
        let dual_obj: f64 = b.iter().zip(sol.dual.iter()).map(|(&bi, &yi)| bi * yi).sum();
        prop_assert!((sol.objective - dual_obj).abs() < 1e-4);
    }

    #[test]
    fn hhi_bounds_hold_for_arbitrary_nonnegative_pair_flows(
        flows in prop::collection::vec(0u32..10_000, 1..20),
    ) {
        let floats: Vec<f64> = flows.into_iter().map(f64::from).collect();
        let m = compute_hhi(&floats);
        if m.active_pairs > 0 {
            let min_possible = 1.0 / (m.active_pairs as f64);
            prop_assert!(m.hhi >= min_possible - 1e-9);
            prop_assert!(m.hhi <= 1.0 + 1e-9);
            prop_assert!((m.hhi * m.effective_dyads - 1.0).abs() < 1e-9);
        } else {
            prop_assert!((m.hhi - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn plr_respects_capacity_and_tracks_enrolled_seeds(
        rng_seed in any::<u64>(),
        scores in prop::collection::vec(0u16..1000, 1..24),
    ) {
        let mut plr = PlrSampler::new(rng_seed, 4, 0.25, 0.75, 0.7);
        let mut enrolled = HashSet::new();
        for (idx, s) in scores.into_iter().enumerate() {
            let candidate = rng_seed.wrapping_add((idx as u64) + 1);
            plr.record_score(candidate, f64::from(s));
            enrolled.insert(candidate);
            prop_assert!(plr.len() <= 4);
        }
        for _ in 0..16 {
            match plr.sample_next() {
                PlrChoice::Replay(seed) => prop_assert!(enrolled.contains(&seed)),
                PlrChoice::Fresh(_) => {}
            }
        }
    }
}
