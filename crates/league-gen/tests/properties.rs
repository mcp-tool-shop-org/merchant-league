//! Deterministic property tests, fixed-range sweeps, regression checks, and
//! independent oracles for `league-gen`.

use std::collections::HashSet;

use league_gen::{
    EvMethod, FeasibilityError, GENERATOR_VERSION, GeneratorConfig, LpStatus, MAX_MANDATE_BONUS_VP,
    MIN_MANDATE_BONUS_VP, NUMERAIRE_CODE, PROJECT_CONCORDAT_ID, PlrChoice, PlrSampler, RingTable,
    SeatCount, check_feasibility, coalition_upkeep_coverage, compute_hhi, evaluate_ring_table,
    generate_ring_table, is_allowed_commodity_code, is_denied_currency_code,
    pair_has_structural_autarky, solve_lp,
};
use proptest::prelude::*;
use proptest::test_runner::{RngAlgorithm, RngSeed};

fn det_config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: None,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(0x2026_1115_0000_0026),
        ..ProptestConfig::default()
    }
}

/// Independent combinatorial oracle deciding whether unordered pair `{i, j}` can produce
/// either member's upkeep good using only raw extraction belonging to `{i, j}`.
fn independent_pair_autarky_oracle(table: &RingTable, i: u8, j: u8) -> bool {
    let ii = usize::from(i);
    let jj = usize::from(j);
    let targets = [table.seats[ii].upkeep_good, table.seats[jj].upkeep_good];
    table
        .seats
        .iter()
        .flat_map(|s| s.converters.iter())
        .any(|c| {
            (c.owner == i || c.owner == j)
                && c.output_amount > 0.0
                && c.max_runs_per_turn > 0.0
                && targets.contains(&c.output_good)
                && c.inputs
                    .iter()
                    .all(|&(g, amt)| amt <= 0.0 || g == i || g == j)
        })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum OracleLpVerdict {
    Optimal(f64),
    Infeasible,
    Unbounded,
}

/// Independent 2-variable exact geometry vertex-and-ray LP oracle for small integer LPs
/// `max c^T x  s.t.  A x <= b, x >= 0`.
fn solve_2d_vertex_ray_oracle(a: &[Vec<f64>], b: &[f64], c: &[f64]) -> OracleLpVerdict {
    let mut halfplanes: Vec<(f64, f64, f64)> = a
        .iter()
        .zip(b.iter())
        .map(|(row, &rhs)| (row[0], row[1], rhs))
        .collect();
    halfplanes.push((-1.0, 0.0, 0.0));
    halfplanes.push((0.0, -1.0, 0.0));

    let is_feasible = |x0: f64, x1: f64| -> bool {
        halfplanes
            .iter()
            .all(|&(a0, a1, rhs)| a0 * x0 + a1 * x1 <= rhs + 1e-8 * (1.0 + rhs.abs()))
    };

    let mut vertices = Vec::new();
    if is_feasible(0.0, 0.0) {
        vertices.push((0.0, 0.0));
    }
    for p in 0..halfplanes.len() {
        for q in (p + 1)..halfplanes.len() {
            let (a1, b1, r1) = halfplanes[p];
            let (a2, b2, r2) = halfplanes[q];
            let det = a1 * b2 - a2 * b1;
            if det.abs() < 1e-9 {
                continue;
            }
            let x0 = (r1 * b2 - r2 * b1) / det;
            let x1 = (a1 * r2 - a2 * r1) / det;
            if x0.is_finite() && x1.is_finite() && is_feasible(x0, x1) {
                vertices.push((x0, x1));
            }
        }
    }

    if vertices.is_empty() {
        return OracleLpVerdict::Infeasible;
    }

    // Check every extreme boundary tangent direction as a potential recession ray.
    for &(a0, a1, _) in &halfplanes {
        for (dx, dy) in [(a1, -a0), (-a1, a0)] {
            if dx.hypot(dy) < 1e-9 {
                continue;
            }
            let in_recession = halfplanes
                .iter()
                .all(|&(h0, h1, _)| h0 * dx + h1 * dy <= 1e-9);
            if in_recession && (c[0] * dx + c[1] * dy) > 1e-9 {
                return OracleLpVerdict::Unbounded;
            }
        }
    }

    let best = vertices
        .into_iter()
        .map(|(x0, x1)| c[0] * x0 + c[1] * x1)
        .fold(f64::NEG_INFINITY, f64::max);
    OracleLpVerdict::Optimal(best)
}

#[test]
fn regression_seeds_2497_and_2604_six_seat_succeed() {
    for ev_method in [EvMethod::ShadowPrice, EvMethod::Shapley] {
        let cfg = GeneratorConfig {
            seat_count: Some(SeatCount::Six),
            ev_method,
            ..GeneratorConfig::default()
        };
        for seed in [2497u64, 2604u64] {
            let Ok((table, report)) = generate_ring_table(seed, &cfg) else {
                panic!("regression seed {seed} ({ev_method:?}) failed at default config");
            };
            assert_eq!(table.seat_count, SeatCount::Six);
            assert_eq!(table.numeraire_code, NUMERAIRE_CODE);
            assert!(report.max_2seat_upkeep_coverage < 1e-9);
            assert!(report.max_relative_ev_deviation <= cfg.ev_tolerance + 1e-9);
        }
    }
}

#[test]
fn deterministic_sweep_zero_failures_across_3000_six_seat_and_default_seeds() {
    let default_cfg = GeneratorConfig::default();
    for seed in 0u64..400 {
        assert!(
            generate_ring_table(seed, &default_cfg).is_ok(),
            "default config failed at seed {seed}"
        );
    }
    for sc in [SeatCount::Four, SeatCount::Five] {
        let cfg = GeneratorConfig {
            seat_count: Some(sc),
            ..GeneratorConfig::default()
        };
        for seed in 0u64..400 {
            assert!(
                generate_ring_table(seed, &cfg).is_ok(),
                "{sc:?} failed at seed {seed}"
            );
        }
    }
    let six_cfg = GeneratorConfig {
        seat_count: Some(SeatCount::Six),
        ..GeneratorConfig::default()
    };
    for seed in 0u64..3_000 {
        assert!(
            generate_ring_table(seed, &six_cfg).is_ok(),
            "Six failed at seed {seed} (ShadowPrice)"
        );
    }
    // Check edge boundary seeds under Shapley as well, including the previously failing range.
    let six_shapley = GeneratorConfig {
        seat_count: Some(SeatCount::Six),
        ev_method: EvMethod::Shapley,
        ..GeneratorConfig::default()
    };
    for seed in 2_450u64..2_650 {
        assert!(
            generate_ring_table(seed, &six_shapley).is_ok(),
            "Six failed at seed {seed} (Shapley)"
        );
    }
}

proptest! {
    #![proptest_config(det_config(48))]

    #[test]
    fn generated_tables_are_deterministic_roundtrip_and_feasible(
        seed in 0u64..25_000,
        sc_idx in 0usize..3,
        use_shapley in any::<bool>(),
    ) {
        let sc = SeatCount::ALL[sc_idx];
        let cfg = GeneratorConfig {
            seat_count: Some(sc),
            ev_method: if use_shapley { EvMethod::Shapley } else { EvMethod::ShadowPrice },
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
        prop_assert_eq!(&t1.generator_version, GENERATOR_VERSION);
        prop_assert_eq!(&t1.config, &cfg);
        prop_assert_eq!(&t1.numeraire_code, NUMERAIRE_CODE);

        let encoded = match serde_json::to_string(&t1) {
            Ok(s) => s,
            Err(err) => return Err(TestCaseError::fail(format!("serialize failed: {err}"))),
        };
        let decoded: RingTable = match serde_json::from_str(&encoded) {
            Ok(v) => v,
            Err(err) => return Err(TestCaseError::fail(format!("deserialize failed: {err}"))),
        };
        prop_assert_eq!(&decoded, &t1);

        let recheck = match check_feasibility(&decoded) {
            Ok(r) => r,
            Err(err) => return Err(TestCaseError::fail(format!("recheck failed: {err}"))),
        };
        prop_assert_eq!(&recheck, &r1);
        prop_assert!(r1.max_1seat_upkeep_coverage < 1e-9);
        prop_assert!(r1.max_2seat_upkeep_coverage < 1e-9);

        let mut sorted_upkeeps: Vec<f64> = t1.seats.iter().map(|s| s.upkeep_amount).collect();
        sorted_upkeeps.sort_by(f64::total_cmp);
        let max_one_of_three = sorted_upkeeps[sorted_upkeeps.len() - 1]
            / (sorted_upkeeps[0] + sorted_upkeeps[1] + sorted_upkeeps[2]);
        prop_assert!(r1.max_3seat_upkeep_coverage <= max_one_of_three + 1e-6);
        prop_assert!(r1.max_3seat_upkeep_coverage < 0.40);
        if sc.as_usize() >= 5 {
            let un = sorted_upkeeps.len();
            let max_two_of_four = (sorted_upkeeps[un - 1] + sorted_upkeeps[un - 2])
                / (sorted_upkeeps[0]
                    + sorted_upkeeps[1]
                    + sorted_upkeeps[un - 2]
                    + sorted_upkeeps[un - 1]);
            let cov4 = r1.max_4seat_upkeep_coverage.unwrap_or(1.0);
            prop_assert!(cov4 <= max_two_of_four + 1e-6);
            prop_assert!(cov4 <= 0.5 + 0.005);
        }
        prop_assert!(r1.surplus_to_upkeep_ratio >= cfg.min_surplus_ratio - 1e-9);
        prop_assert!(r1.max_relative_ev_deviation <= cfg.ev_tolerance + 1e-9);

        let n = sc.as_usize();
        let mut codes = HashSet::new();
        for seat in &t1.seats {
            prop_assert!(is_allowed_commodity_code(seat.raw_code.as_bytes()));
            prop_assert!(is_allowed_commodity_code(seat.refined_code.as_bytes()));
            prop_assert!(!is_denied_currency_code(seat.raw_code.as_bytes()));
            prop_assert!(!is_denied_currency_code(seat.refined_code.as_bytes()));
            prop_assert!(codes.insert(seat.raw_code));
            prop_assert!(codes.insert(seat.refined_code));
        }
        prop_assert_eq!(codes.len(), 2 * n);

        // B2.2: Project 1..=N pairs + 100 full-ring Concordat and 3..=8 VP mandates.
        prop_assert_eq!(t1.projects.len(), n + 1);
        let concordat = t1
            .projects
            .iter()
            .find(|p| p.id == PROJECT_CONCORDAT_ID)
            .ok_or_else(|| TestCaseError::fail("missing Concordat project"))?;
        prop_assert_eq!(concordat.required_inputs.len(), n);
        prop_assert!(t1.mandate_deck.len() >= n);
        for card in &t1.mandate_deck {
            prop_assert!(card.kind.validate_for_seat_count(sc).is_ok());
            prop_assert!(
                (MIN_MANDATE_BONUS_VP..=MAX_MANDATE_BONUS_VP).contains(&card.bonus_vp)
            );
        }

        // B2.1 + #26b + C6: positive converter clearing margins, shadow-LP conservation, pair-share cap.
        prop_assert!(r1.min_converter_clearing_margin > 0.0);
        if cfg.ev_method == EvMethod::ShadowPrice {
            let sum_evs: f64 = r1.seat_expected_values.iter().sum();
            let rel_shadow = ((sum_evs - r1.season_effective_net_surplus)
                / r1.season_effective_net_surplus)
                .abs();
            prop_assert!(rel_shadow < 1e-11, "shadow LP rel gap={rel_shadow}");
        }
        prop_assert!(r1.static_max_pair_share <= sc.default_max_pair_share_cap() + 1e-9);

        let ring_metrics = evaluate_ring_table(&t1, &r1);
        prop_assert!(ring_metrics.trade_concentration.hhi < 0.25);
        prop_assert!(ring_metrics.trade_concentration.active_pairs >= n);
        prop_assert!(
            ring_metrics.trade_concentration.max_pair_share <= sc.default_max_pair_share_cap() + 1e-9
        );
        prop_assert!(ring_metrics.rollout_grading.total_project_batches > 0);
        prop_assert!(ring_metrics.rollout_grading.concordat_batches > 0);
    }

    #[test]
    fn independent_structural_oracle_matches_pair_filter_and_coverage(
        seed in 0u64..10_000,
        sc_idx in 0usize..3,
        seat_pick in 0usize..6,
        in_a in 0usize..6,
        in_b in 0usize..6,
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
        let owner = seat_pick % n;
        let g0 = (in_a % n) as u8;
        let g1 = if in_b % n == in_a % n { ((in_a + 1) % n) as u8 } else { (in_b % n) as u8 };
        // Mutate primary converter inputs across arbitrary distinct raw goods.
        table.seats[owner].converters[0].inputs = vec![(g0, 1.5), (g1, 1.5)];

        let mut oracle_any_pair = false;
        for i in 0..n {
            for j in (i + 1)..n {
                let oracle = independent_pair_autarky_oracle(&table, i as u8, j as u8);
                let struct_chk = match pair_has_structural_autarky(&table, i as u8, j as u8) {
                    Ok(v) => v,
                    Err(err) => return Err(TestCaseError::fail(format!("struct check failed: {err}"))),
                };
                let cov = match coalition_upkeep_coverage(&table, &[i as u8, j as u8]) {
                    Ok(v) => v,
                    Err(err) => return Err(TestCaseError::fail(format!("pair lp failed: {err}"))),
                };
                prop_assert_eq!(struct_chk, oracle);
                prop_assert_eq!(cov > 1e-7, oracle);
                if oracle {
                    oracle_any_pair = true;
                }
            }
        }

        let verdict = check_feasibility(&table);
        if oracle_any_pair {
            let rejected = matches!(verdict, Err(FeasibilityError::PairCoversUpkeep { .. }));
            prop_assert!(rejected, "expected PairCoversUpkeep, got {verdict:?}");
        }
    }

    #[test]
    fn negative_controls_always_reject_surplus_and_ev_violations(
        seed in 0u64..10_000,
        sc_idx in 0usize..3,
        seat_pick in 0usize..6,
        use_shapley in any::<bool>(),
    ) {
        let sc = SeatCount::ALL[sc_idx];
        let cfg = GeneratorConfig {
            seat_count: Some(sc),
            ev_method: if use_shapley { EvMethod::Shapley } else { EvMethod::ShadowPrice },
            ..GeneratorConfig::default()
        };
        let (table, _) = match generate_ring_table(seed, &cfg) {
            Ok(pair) => pair,
            Err(err) => return Err(TestCaseError::fail(format!("generation failed: {err}"))),
        };

        // Invariant 2 negative control: collapse all yields below upkeep needs.
        let mut low_yield = table.clone();
        for s in &mut low_yield.seats {
            s.raw_yield_per_turn *= 0.05;
        }
        let low_res = check_feasibility(&low_yield);
        let rejected_surplus = matches!(low_res, Err(FeasibilityError::InsufficientSurplus { .. }));
        prop_assert!(rejected_surplus, "expected InsufficientSurplus, got {low_res:?}");

        // Invariant 3 negative control: massive asymmetric extraction endowment on one seat.
        let mut asym = table;
        let idx = seat_pick % sc.as_usize();
        asym.seats[idx].raw_yield_per_turn *= 3.0;
        let asym_res = check_feasibility(&asym);
        let rejected_ev = matches!(asym_res, Err(FeasibilityError::SeatEvOutOfTolerance { .. }));
        prop_assert!(rejected_ev, "expected SeatEvOutOfTolerance, got {asym_res:?}");
    }

    #[test]
    fn random_signed_phase_one_lps_match_2d_vertex_and_ray_oracle(
        a_ints in prop::collection::vec(-6i16..=6, 8),
        b_ints in prop::collection::vec(-8i16..=12, 4),
        c_ints in prop::collection::vec(-5i16..=5, 2),
    ) {
        let a: Vec<Vec<f64>> = a_ints
            .as_chunks::<2>()
            .0
            .iter()
            .map(|r| vec![f64::from(r[0]), f64::from(r[1])])
            .collect();
        let b: Vec<f64> = b_ints.into_iter().map(f64::from).collect();
        let c: Vec<f64> = c_ints.into_iter().map(f64::from).collect();

        let oracle = solve_2d_vertex_ray_oracle(&a, &b, &c);
        let lp = match solve_lp(&a, &b, &c) {
            Ok(status) => status,
            Err(err) => return Err(TestCaseError::fail(format!("lp failed: {err}"))),
        };

        match (lp, oracle) {
            (LpStatus::Infeasible, OracleLpVerdict::Infeasible) => {}
            (LpStatus::Unbounded, OracleLpVerdict::Unbounded) => {}
            (LpStatus::Optimal(sol), OracleLpVerdict::Optimal(opt)) => {
                prop_assert!(
                    (sol.objective - opt).abs() < 1e-4 * (1.0 + opt.abs()),
                    "LP objective {} != oracle {opt}",
                    sol.objective
                );
            }
            (got, expected) => {
                return Err(TestCaseError::fail(format!(
                    "LP status {got:?} disagreed with 2D oracle {expected:?} for a={a:?}, b={b:?}, c={c:?}"
                )));
            }
        }
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
            prop_assert!(m.max_pair_share >= min_possible - 1e-9);
            prop_assert!(m.max_pair_share <= 1.0 + 1e-9);
            prop_assert!(m.hhi >= m.max_pair_share * m.max_pair_share - 1e-9);
        } else {
            prop_assert!((m.hhi - 1.0).abs() < 1e-9);
            prop_assert!((m.max_pair_share - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn plr_respects_capacity_and_tracks_enrolled_seeds(
        rng_seed in 0u64..100_000,
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
