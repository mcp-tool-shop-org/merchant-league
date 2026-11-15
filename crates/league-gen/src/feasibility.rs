//! Three-invariant linear-programming feasibility filter for chiral ring tables.
//!
//! Proves on every candidate [`RingTable`]:
//! 1. **Zero 2-seat autarky:** every unordered seat pair `{i, j}` has no internal
//!    converter chain capable of supplying either seat's upkeep good (checked both
//!    structurally via [`pair_has_structural_autarky`] and quantitatively via
//!    [`coalition_upkeep_coverage`]).
//! 2. **Strict aggregate converter surplus:** full-table conversion covers 100%
//!    of mandatory upkeep with net refined surplus exceeding `min_surplus_ratio`.
//! 3. **Seat expected-value balance:** expected seat values per turn (computed via
//!    [`EvMethod::ShadowPrice`] by default per `docs/design-v2.md` §1.5, or
//!    [`EvMethod::Shapley`] when selected) fall within `±config.ev_tolerance`
//!    of the table mean.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::lp::{LpStatus, solve_lp};
use crate::schema::{
    ConverterSpec, EvMethod, FeasibilityError, GeneratorConfig, HorizonShockKind, NUMERAIRE_CODE,
    RingTable,
};

const PAIR_COVERAGE_TOL: f64 = 1e-7;

/// Expected turn count of a season (midpoint of the `(24, 28)` close window in
/// `docs/design-v2.md` §6.2; linear `1 / 26.0` shock amortization is provisional,
/// calibrate in playtest 1).
const SEASON_EXPECTED_TURNS: f64 = 26.0;

/// Relative valuation weight assigned to unconverted raw salvage compared to mean refined
/// Syndicate Project value (provisional, calibrate in playtest 1).
const RAW_SALVAGE_PROJECT_FRACTION: f64 = 0.25;

/// Sub-coalition unmet-upkeep penalty multiplier used by [`EvMethod::Shapley`] (`1.10`;
/// provisional, calibrate in playtest 1).
const SHAPLEY_UPKEEP_PENALTY_MULT: f64 = 1.10;

/// Certified LP metrics and optimal grand-coalition production plan for a [`RingTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeasibilityReport {
    /// Expected-value allocation method used to verify Invariant 3.
    pub ev_method: EvMethod,
    /// Maximum upkeep fraction satisfied internally by any 1-seat coalition (must be `0.0`).
    pub max_1seat_upkeep_coverage: f64,
    /// Maximum upkeep fraction satisfied internally by any 2-seat pair (must be `0.0`).
    pub max_2seat_upkeep_coverage: f64,
    /// Maximum upkeep fraction satisfied internally by any 3-seat bloc.
    pub max_3seat_upkeep_coverage: f64,
    /// Maximum upkeep fraction satisfied internally by any 4-seat bloc (when `N >= 5`).
    pub max_4seat_upkeep_coverage: Option<f64>,
    /// Total fundamental value of mandatory turn upkeep across all seats.
    pub total_upkeep_value: f64,
    /// Net refined Syndicate Project surplus value above all mandatory upkeep.
    pub net_converter_surplus: f64,
    /// Measured ratio `net_converter_surplus / total_upkeep_value`.
    pub surplus_to_upkeep_ratio: f64,
    /// Per-seat expected value per turn under [`Self::ev_method`].
    pub seat_expected_values: Vec<f64>,
    /// Mean seat expected value.
    pub mean_seat_ev: f64,
    /// Maximum relative deviation `|(ev_i - mean) / mean|` across seats.
    pub max_relative_ev_deviation: f64,
    /// Optimal grand-coalition converter run counts matching table converter order.
    pub optimal_converter_runs: Vec<f64>,
}

/// Evaluates all structural rules and proves the three LP feasibility invariants for `table`.
pub fn check_feasibility(
    table: &RingTable,
    config: &GeneratorConfig,
) -> Result<FeasibilityReport, FeasibilityError> {
    validate_config_and_table(table, config)?;
    let n = table.seat_count.as_usize();

    // Fast structural pass for Invariant 1 before heavier LP solves: if any 1-seat or 2-seat
    // subset already has an internal converter path to upkeep, compute its LP coverage and reject.
    for i in 0..n {
        if pair_has_structural_autarky(table, i as u8, i as u8)? {
            let cov = coalition_upkeep_coverage(table, &[i as u8])?;
            return Err(FeasibilityError::PairCoversUpkeep {
                pair: (i as u8, i as u8),
                coverage_ratio: cov,
            });
        }
        for j in (i + 1)..n {
            if pair_has_structural_autarky(table, i as u8, j as u8)? {
                let cov = coalition_upkeep_coverage(table, &[i as u8, j as u8])?;
                return Err(FeasibilityError::PairCoversUpkeep {
                    pair: (i as u8, j as u8),
                    coverage_ratio: cov,
                });
            }
        }
    }

    // Invariant 2: strict aggregate converter surplus over mandatory upkeep.
    let (total_upkeep_value, net_converter_surplus, optimal_converter_runs) =
        solve_grand_surplus(table)?;
    let surplus_to_upkeep_ratio = if total_upkeep_value > 0.0 {
        net_converter_surplus / total_upkeep_value
    } else {
        0.0
    };
    if net_converter_surplus <= 0.0 || surplus_to_upkeep_ratio < config.min_surplus_ratio {
        return Err(FeasibilityError::InsufficientSurplus {
            net_surplus: net_converter_surplus,
            required_ratio: config.min_surplus_ratio,
            actual_ratio: surplus_to_upkeep_ratio,
        });
    }

    // Invariant 3: Seat expected values within `±config.ev_tolerance`.
    let seat_expected_values = match config.ev_method {
        EvMethod::ShadowPrice => compute_shadow_price_seat_evs(table)?,
        EvMethod::Shapley => compute_shapley_seat_evs(table)?,
    };
    let mean_seat_ev = seat_expected_values.iter().sum::<f64>() / (n as f64);
    if !mean_seat_ev.is_finite() || mean_seat_ev <= 0.0 {
        return Err(FeasibilityError::SeatEvOutOfTolerance {
            max_relative_deviation: f64::INFINITY,
            tolerance: config.ev_tolerance,
            seat_evs: seat_expected_values,
        });
    }
    let max_relative_ev_deviation = seat_expected_values
        .iter()
        .map(|&ev| ((ev - mean_seat_ev) / mean_seat_ev).abs())
        .fold(0.0_f64, f64::max);

    if max_relative_ev_deviation > config.ev_tolerance + 1e-9 {
        return Err(FeasibilityError::SeatEvOutOfTolerance {
            max_relative_deviation: max_relative_ev_deviation,
            tolerance: config.ev_tolerance,
            seat_evs: seat_expected_values,
        });
    }

    // Invariant 1 LP certification + 3-seat / 4-seat bloc bounds for the accepted report.
    let mut max_1seat = 0.0_f64;
    for i in 0..n {
        let cov = coalition_upkeep_coverage(table, &[i as u8])?;
        max_1seat = max_1seat.max(cov);
        if cov > PAIR_COVERAGE_TOL {
            return Err(FeasibilityError::PairCoversUpkeep {
                pair: (i as u8, i as u8),
                coverage_ratio: cov,
            });
        }
    }

    let mut max_2seat = 0.0_f64;
    for i in 0..n {
        for j in (i + 1)..n {
            let cov = coalition_upkeep_coverage(table, &[i as u8, j as u8])?;
            max_2seat = max_2seat.max(cov);
            if cov > PAIR_COVERAGE_TOL {
                return Err(FeasibilityError::PairCoversUpkeep {
                    pair: (i as u8, j as u8),
                    coverage_ratio: cov,
                });
            }
        }
    }

    let max_3seat = max_subset_coverage(table, n, 3)?;
    let max_4seat = if n >= 5 {
        Some(max_subset_coverage(table, n, 4)?)
    } else {
        None
    };

    Ok(FeasibilityReport {
        ev_method: config.ev_method,
        max_1seat_upkeep_coverage: max_1seat,
        max_2seat_upkeep_coverage: max_2seat,
        max_3seat_upkeep_coverage: max_3seat,
        max_4seat_upkeep_coverage: max_4seat,
        total_upkeep_value,
        net_converter_surplus,
        surplus_to_upkeep_ratio,
        seat_expected_values,
        mean_seat_ev,
        max_relative_ev_deviation,
        optimal_converter_runs,
    })
}

/// Exact structural check for whether pair `{i, j}` (`i <= j`, or `i == j` for 1-seat)
/// possesses any converter whose positive inputs and output satisfy an upkeep demand inside `{i, j}`.
pub fn pair_has_structural_autarky(
    table: &RingTable,
    i: u8,
    j: u8,
) -> Result<bool, FeasibilityError> {
    let n = validate_index_bounds(table)?;
    let ii = usize::from(i);
    let jj = usize::from(j);
    if ii >= n || jj >= n {
        return Err(FeasibilityError::InvalidTable(format!(
            "pair indices ({i}, {j}) out of range 0..{n}"
        )));
    }
    let upkeep_i = table.seats[ii].upkeep_good;
    let upkeep_j = table.seats[jj].upkeep_good;
    let need_i = table.seats[ii].upkeep_amount > 0.0;
    let need_j = table.seats[jj].upkeep_amount > 0.0;

    for seat in &table.seats {
        for conv in &seat.converters {
            if (conv.owner != i && conv.owner != j)
                || conv.output_amount <= 0.0
                || conv.max_runs_per_turn <= 0.0
            {
                continue;
            }
            let inputs_inside_pair = conv
                .inputs
                .iter()
                .all(|&(raw_g, amt)| amt <= 0.0 || raw_g == i || raw_g == j);
            let feeds_pair_upkeep = (need_i && conv.output_good == upkeep_i)
                || (need_j && conv.output_good == upkeep_j);
            if inputs_inside_pair && feeds_pair_upkeep {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Computes the exact maximum fraction `[0.0, 1.0]` of the coalition's combined
/// upkeep that `seats` can satisfy internally without trading outside `seats`.
pub fn coalition_upkeep_coverage(table: &RingTable, seats: &[u8]) -> Result<f64, FeasibilityError> {
    let n = validate_index_bounds(table)?;
    let mut active = vec![false; n];
    for &s in seats {
        let idx = usize::from(s);
        if idx >= n {
            return Err(FeasibilityError::InvalidTable(format!(
                "coalition seat index {s} out of range 0..{n}"
            )));
        }
        active[idx] = true;
    }

    let active_seats: Vec<usize> = (0..n).filter(|&i| active[i]).collect();
    let total_upkeep: f64 = active_seats
        .iter()
        .map(|&i| table.seats[i].upkeep_amount)
        .sum();
    if total_upkeep <= 0.0 {
        return Ok(0.0);
    }

    let active_convs: Vec<&ConverterSpec> = table
        .seats
        .iter()
        .flat_map(|s| s.converters.iter())
        .filter(|conv| {
            active[usize::from(conv.owner)]
                && conv
                    .inputs
                    .iter()
                    .all(|&(raw_g, amt)| amt <= 0.0 || active[usize::from(raw_g)])
        })
        .collect();

    // Rescale RHS quantities if very small or very large so simplex tolerances stay unit-invariant.
    let q_scale = quantity_scale(table);
    let num_conv = active_convs.len();
    let num_s = active_seats.len();
    let num_vars = num_conv + num_s;
    let mut a = Vec::new();
    let mut b = Vec::new();

    for (c_idx, conv) in active_convs.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[c_idx] = 1.0;
        a.push(row);
        b.push(effective_scaled_cap(table, conv, q_scale));
    }

    for (pos, &s) in active_seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[num_conv + pos] = 1.0;
        a.push(row);
        let producer_in_s = active_convs
            .iter()
            .any(|conv| conv.output_good == table.seats[s].upkeep_good);
        b.push(if producer_in_s {
            table.seats[s].upkeep_amount / q_scale
        } else {
            0.0
        });
    }

    for &k in &active_seats {
        let seat_k = &table.seats[k];
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in active_convs.iter().enumerate() {
            for &(raw_g, amt) in &conv.inputs {
                if usize::from(raw_g) == k && amt > 0.0 {
                    let tariff_mult = if usize::from(conv.owner) == k {
                        1.0
                    } else {
                        1.0 + seat_k.raw_export_tariff
                    };
                    row[c_idx] += amt * tariff_mult;
                }
            }
        }
        a.push(row);
        b.push(seat_k.raw_yield_per_turn / q_scale);
    }

    for m in 0..n {
        let has_conv = active_convs
            .iter()
            .any(|conv| usize::from(conv.output_good) == m);
        let has_consumer = active_seats
            .iter()
            .any(|&s| usize::from(table.seats[s].upkeep_good) == m);
        if !has_conv && !has_consumer {
            continue;
        }
        let seat_m = &table.seats[m];
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in active_convs.iter().enumerate() {
            if usize::from(conv.output_good) == m {
                row[c_idx] -= conv.output_amount;
            }
        }
        for (pos, &s) in active_seats.iter().enumerate() {
            if usize::from(table.seats[s].upkeep_good) == m {
                let tariff_mult = if s == m {
                    1.0
                } else {
                    1.0 + seat_m.refined_export_tariff
                };
                row[num_conv + pos] += tariff_mult;
            }
        }
        a.push(row);
        b.push(0.0);
    }

    let total_upkeep_scaled = total_upkeep / q_scale;
    let mut c = vec![0.0; num_vars];
    for cell in &mut c[num_conv..num_vars] {
        *cell = 1.0 / total_upkeep_scaled;
    }

    match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => Ok(sol.objective.clamp(0.0, 1.0)),
        LpStatus::Infeasible => Err(FeasibilityError::SolverFailure(
            "coalition upkeep LP reported infeasible despite zero-run feasibility".to_owned(),
        )),
        LpStatus::Unbounded => Err(FeasibilityError::SolverFailure(
            "coalition upkeep LP reported unbounded".to_owned(),
        )),
    }
}

/// Computes competitive/cooperative LP-dual shadow-price seat expected values per turn
/// (`docs/design-v2.md` §1.5), satisfying strong duality `sum(ev_i) == grand_net_surplus`.
pub fn compute_shadow_price_seat_evs(table: &RingTable) -> Result<Vec<f64>, FeasibilityError> {
    let n = validate_index_bounds(table)?;
    let converters = flatten_converters(table);
    let eff = effective_season_params(table);
    let mean_proj = eff.iter().map(|p| p.project_value).sum::<f64>() / (n as f64);

    let q_scale = eff_quantity_scale(table, &eff);
    let num_conv = converters.len();
    // Variables: x_0..x_{C-1} (converter runs), p_0..p_{N-1} (surplus projects), r_0..r_{N-1} (raw salvage).
    let p_off = num_conv;
    let r_off = num_conv + n;
    let num_vars = r_off + n;

    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut cap_eff_unscaled = Vec::with_capacity(num_conv);

    // Rows 0..C: converter capacity bounds `x_c <= max_runs_c` (scaled).
    for conv in &converters {
        let cap_scaled = effective_scaled_cap_with_eff(table, &eff, conv, q_scale);
        cap_eff_unscaled.push(cap_scaled * q_scale);
        let mut row = vec![0.0; num_vars];
        row[cap_eff_unscaled.len() - 1] = 1.0;
        a.push(row);
        b.push(cap_scaled);
    }

    // Rows C..C+N: raw material balances `sum_c A_{c,k} x_c + r_k <= raw_yield_k` (scaled).
    for k in 0..n {
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in converters.iter().enumerate() {
            for &(raw_g, amt) in &conv.inputs {
                if usize::from(raw_g) == k && amt > 0.0 {
                    let mult = if usize::from(conv.owner) == k {
                        1.0
                    } else {
                        1.0 + eff[k].raw_tariff
                    };
                    row[c_idx] += amt * mult;
                }
            }
        }
        row[r_off + k] = 1.0;
        a.push(row);
        b.push(eff[k].raw_yield / q_scale);
    }

    // Rows C+N..C+2N: refined project balances `-sum_{out=m} out_c x_c + p_m <= -upkeep_reqs[m]` (scaled).
    let mut upkeep_reqs_unscaled = vec![0.0_f64; n];
    for (s, seat_s) in table.seats.iter().enumerate() {
        let m = usize::from(seat_s.upkeep_good);
        let mult = if s == m {
            1.0
        } else {
            1.0 + eff[m].refined_tariff
        };
        upkeep_reqs_unscaled[m] += seat_s.upkeep_amount * mult;
    }

    for (m, &req) in upkeep_reqs_unscaled.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in converters.iter().enumerate() {
            if usize::from(conv.output_good) == m {
                row[c_idx] -= conv.output_amount;
            }
        }
        row[p_off + m] = 1.0;
        a.push(row);
        b.push(-req / q_scale);
    }

    let raw_salvage_weight = RAW_SALVAGE_PROJECT_FRACTION * mean_proj;
    let mut c = vec![0.0; num_vars];
    for m in 0..n {
        c[p_off + m] = eff[m].project_value;
        c[r_off + m] = raw_salvage_weight * (1.0 - eff[m].decay_rate);
    }

    let sol = match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => sol,
        LpStatus::Infeasible => {
            let total_upkeep_value: f64 = upkeep_reqs_unscaled
                .iter()
                .enumerate()
                .map(|(m, &req)| req * eff[m].project_value)
                .sum();
            return Err(FeasibilityError::InsufficientSurplus {
                net_surplus: -total_upkeep_value,
                required_ratio: 0.0,
                actual_ratio: 0.0,
            });
        }
        LpStatus::Unbounded => {
            return Err(FeasibilityError::SolverFailure(
                "shadow-price grand LP reported unbounded".to_owned(),
            ));
        }
    };

    // Build each seat's dual valuation functional `a_i` so `ev[i] = sum_row a_i[row] * y[row]`
    // and `sum_i a_i == unscaled b` identically.
    let m_rows = b.len();
    let mut seat_dual_coeffs = vec![vec![0.0_f64; m_rows]; n];
    let raw_row_off = num_conv;
    let ref_row_off = num_conv + n;

    for (c_idx, conv) in converters.iter().enumerate() {
        seat_dual_coeffs[usize::from(conv.owner)][c_idx] += cap_eff_unscaled[c_idx];
    }
    for (k, eff_k) in eff.iter().enumerate() {
        seat_dual_coeffs[k][raw_row_off + k] += eff_k.raw_yield;
    }
    for (s, seat_s) in table.seats.iter().enumerate() {
        let m = usize::from(seat_s.upkeep_good);
        let mult = if s == m {
            1.0
        } else {
            1.0 + eff[m].refined_tariff
        };
        seat_dual_coeffs[s][ref_row_off + m] -= seat_s.upkeep_amount * mult;
    }

    // Check strict primal non-degeneracy: if any basis constraint/variable is degenerate,
    // multiple optimal dual vectors exist (e.g., exact `in_a == in_b` on even-N symmetric tables).
    // Select the minimum-dispersion dual vector inside the exact optimal dual face `D*`.
    let y = if is_primal_degenerate(&a, &b, &sol.primal) {
        select_balanced_optimal_dual(&a, &b, &c, sol.objective, &seat_dual_coeffs, q_scale)?
            .unwrap_or(sol.dual)
    } else {
        sol.dual
    };

    let seat_evs = seat_dual_coeffs
        .iter()
        .map(|coeffs| {
            coeffs
                .iter()
                .zip(y.iter())
                .map(|(&a_r, &y_r)| a_r * y_r)
                .sum()
        })
        .collect();

    Ok(seat_evs)
}

fn is_primal_degenerate(a: &[Vec<f64>], b: &[f64], primal: &[f64]) -> bool {
    let max_rhs = b.iter().map(|v| v.abs()).fold(0.0_f64, f64::max).max(1.0);
    let pos_tol = 1e-7 * max_rhs;
    let pos_primals = primal.iter().filter(|&&x| x > pos_tol).count();
    let pos_slacks = a
        .iter()
        .zip(b.iter())
        .filter(|(row, bi)| {
            let lhs: f64 = row
                .iter()
                .zip(primal.iter())
                .map(|(&aij, &xj)| aij * xj)
                .sum();
            (*bi - lhs) > pos_tol
        })
        .count();
    pos_primals + pos_slacks < b.len()
}

fn select_balanced_optimal_dual(
    a: &[Vec<f64>],
    b: &[f64],
    c: &[f64],
    primal_obj_scaled: f64,
    seat_dual_coeffs: &[Vec<f64>],
    q_scale: f64,
) -> Result<Option<Vec<f64>>, FeasibilityError> {
    let m_rows = b.len();
    let num_vars = c.len();
    let n_seats = seat_dual_coeffs.len();
    if n_seats == 0 {
        return Ok(None);
    }

    // Variables: y_0..y_{m_rows-1}, d >= 0 (scaled max deviation in scaled objective units).
    let d_col = m_rows;
    let lp_vars = m_rows + 1;
    let mut lp_a = Vec::with_capacity(num_vars + 1 + 2 * n_seats);
    let mut lp_b = Vec::with_capacity(num_vars + 1 + 2 * n_seats);

    // 1. Dual feasibility: -A^T y <= -c
    for (j, &cj) in c.iter().enumerate() {
        let mut row = vec![0.0; lp_vars];
        for (i, a_row) in a.iter().enumerate() {
            row[i] = -a_row[j];
        }
        lp_a.push(row);
        lp_b.push(-cj);
    }

    // 2. Optimality cut: b^T y <= primal_obj_scaled + cut_tol
    let cut_tol = 1e-7 * (1.0 + primal_obj_scaled.abs());
    let mut cut_row = vec![0.0; lp_vars];
    cut_row[..m_rows].copy_from_slice(b);
    lp_a.push(cut_row);
    lp_b.push(primal_obj_scaled + cut_tol);

    // 3. Minimax seat EV spread around the fixed dual mean: |ev_i_scaled(y) - mean_scaled| <= d.
    let mean_scaled = primal_obj_scaled / (n_seats as f64);
    for coeffs in seat_dual_coeffs {
        let mut upper = vec![0.0; lp_vars];
        let mut lower = vec![0.0; lp_vars];
        for i in 0..m_rows {
            let scaled_c = coeffs[i] / q_scale;
            upper[i] = scaled_c;
            lower[i] = -scaled_c;
        }
        upper[d_col] = -1.0;
        lower[d_col] = -1.0;
        lp_a.push(upper);
        lp_b.push(mean_scaled);
        lp_a.push(lower);
        lp_b.push(-mean_scaled);
    }

    let mut lp_c = vec![0.0; lp_vars];
    lp_c[d_col] = -1.0;

    match solve_lp(&lp_a, &lp_b, &lp_c) {
        Ok(LpStatus::Optimal(sol)) => Ok(Some(sol.primal[..m_rows].to_vec())),
        Ok(LpStatus::Infeasible | LpStatus::Unbounded) | Err(_) => Ok(None),
    }
}

/// Computes exact `2^N` sub-coalition LP Shapley seat expected values per turn.
pub fn compute_shapley_seat_evs(table: &RingTable) -> Result<Vec<f64>, FeasibilityError> {
    let n = validate_index_bounds(table)?;
    let converters = flatten_converters(table);
    let eff = effective_season_params(table);
    let total_masks = 1usize << n;
    let mut coalition_values = vec![0.0_f64; total_masks];
    let q_scale = eff_quantity_scale(table, &eff);

    let mean_proj = eff.iter().map(|p| p.project_value).sum::<f64>() / (n as f64);

    for (mask, slot) in coalition_values.iter_mut().enumerate().skip(1) {
        *slot = evaluate_coalition_net_equity(table, &converters, &eff, mean_proj, mask, q_scale)?;
    }

    let mut fact = vec![1.0_f64; n + 1];
    for k in 1..=n {
        fact[k] = fact[k - 1] * (k as f64);
    }
    let n_fact = fact[n];

    let mut shapley = vec![0.0_f64; n];
    for (i, ev_i) in shapley.iter_mut().enumerate() {
        let bit = 1usize << i;
        let mut sum = 0.0_f64;
        for s_mask in 0..total_masks {
            if (s_mask & bit) != 0 {
                continue;
            }
            let s_len = s_mask.count_ones() as usize;
            let weight = (fact[s_len] * fact[n - s_len - 1]) / n_fact;
            let marginal = coalition_values[s_mask | bit] - coalition_values[s_mask];
            sum += weight * marginal;
        }
        *ev_i = sum;
    }

    Ok(shapley)
}

fn max_subset_coverage(table: &RingTable, n: usize, k: usize) -> Result<f64, FeasibilityError> {
    let mut best = 0.0_f64;
    for mask in 1usize..(1usize << n) {
        if mask.count_ones() as usize != k {
            continue;
        }
        let seats: Vec<u8> = (0..n)
            .filter(|&i| (mask & (1 << i)) != 0)
            .map(|i| i as u8)
            .collect();
        let cov = coalition_upkeep_coverage(table, &seats)?;
        best = best.max(cov);
    }
    Ok(best)
}

fn solve_grand_surplus(table: &RingTable) -> Result<(f64, f64, Vec<f64>), FeasibilityError> {
    let n = table.seat_count.as_usize();
    let converters = flatten_converters(table);
    let num_conv = converters.len();
    let q_scale = quantity_scale(table);
    // Variables: x_0..x_{C-1} (converter runs), s_0..s_{N-1} (net refined surplus per good).
    let num_vars = num_conv + n;
    let mut a = Vec::new();
    let mut b = Vec::new();

    for (c_idx, conv) in converters.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[c_idx] = 1.0;
        a.push(row);
        b.push(effective_scaled_cap(table, conv, q_scale));
    }

    for (k, seat_k) in table.seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in converters.iter().enumerate() {
            for &(raw_g, amt) in &conv.inputs {
                if usize::from(raw_g) == k && amt > 0.0 {
                    let tariff_mult = if usize::from(conv.owner) == k {
                        1.0
                    } else {
                        1.0 + seat_k.raw_export_tariff
                    };
                    row[c_idx] += amt * tariff_mult;
                }
            }
        }
        a.push(row);
        b.push(seat_k.raw_yield_per_turn / q_scale);
    }

    let mut upkeep_reqs_unscaled = vec![0.0; n];
    for (s, seat_s) in table.seats.iter().enumerate() {
        let m = usize::from(seat_s.upkeep_good);
        let tariff_mult = if s == m {
            1.0
        } else {
            1.0 + table.seats[m].refined_export_tariff
        };
        upkeep_reqs_unscaled[m] += seat_s.upkeep_amount * tariff_mult;
    }

    let mut total_upkeep_value = 0.0;
    for (m, &req_unscaled) in upkeep_reqs_unscaled.iter().enumerate() {
        total_upkeep_value += req_unscaled * table.seats[m].refined_project_value;
        let req_scaled = req_unscaled / q_scale;
        let mut le_row = vec![0.0; num_vars];
        let mut ge_row = vec![0.0; num_vars];
        for (c_idx, conv) in converters.iter().enumerate() {
            if usize::from(conv.output_good) == m {
                le_row[c_idx] += conv.output_amount;
                ge_row[c_idx] -= conv.output_amount;
            }
        }
        le_row[num_conv + m] = -1.0;
        ge_row[num_conv + m] = 1.0;
        a.push(le_row);
        b.push(req_scaled);
        a.push(ge_row);
        b.push(-req_scaled);
    }

    let mut c = vec![0.0; num_vars];
    for (m, seat_m) in table.seats.iter().enumerate() {
        c[num_conv + m] = seat_m.refined_project_value;
    }

    match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => Ok((
            total_upkeep_value,
            sol.objective * q_scale,
            sol.primal[..num_conv]
                .iter()
                .map(|&x| x * q_scale)
                .collect(),
        )),
        LpStatus::Infeasible => Err(FeasibilityError::InsufficientSurplus {
            net_surplus: -total_upkeep_value,
            required_ratio: 0.0,
            actual_ratio: 0.0,
        }),
        LpStatus::Unbounded => Err(FeasibilityError::SolverFailure(
            "grand surplus LP reported unbounded".to_owned(),
        )),
    }
}

struct EffectiveSeatParams {
    raw_yield: f64,
    raw_tariff: f64,
    refined_tariff: f64,
    decay_rate: f64,
    project_value: f64,
}

fn effective_season_params(table: &RingTable) -> Vec<EffectiveSeatParams> {
    let n = table.seat_count.as_usize();
    let mut params: Vec<EffectiveSeatParams> = table
        .seats
        .iter()
        .map(|s| EffectiveSeatParams {
            raw_yield: s.raw_yield_per_turn,
            raw_tariff: s.raw_export_tariff,
            refined_tariff: s.refined_export_tariff,
            decay_rate: s.perishable_decay_rate,
            project_value: s.refined_project_value,
        })
        .collect();

    for shock in &table.shocks {
        let idx = usize::from(shock.target_seat);
        if idx >= n {
            continue;
        }
        let p = &mut params[idx];
        match shock.kind {
            HorizonShockKind::SupplyFreeze => {
                p.raw_yield *= (1.0 - shock.magnitude / SEASON_EXPECTED_TURNS).max(0.1);
            }
            HorizonShockKind::TariffHoliday => {
                let factor = (1.0 - shock.magnitude / SEASON_EXPECTED_TURNS).max(0.0);
                p.raw_tariff *= factor;
                p.refined_tariff *= factor;
            }
            HorizonShockKind::DemandSurge => {
                p.project_value *= 1.0 + (2.0 * shock.magnitude) / SEASON_EXPECTED_TURNS;
            }
            HorizonShockKind::SpoilageSpike => {
                p.decay_rate = (p.decay_rate
                    + shock.magnitude * (1.0 - p.decay_rate) / SEASON_EXPECTED_TURNS)
                    .min(0.99);
            }
        }
    }

    params
}

fn evaluate_coalition_net_equity(
    table: &RingTable,
    converters: &[&ConverterSpec],
    eff: &[EffectiveSeatParams],
    mean_proj: f64,
    mask: usize,
    q_scale: f64,
) -> Result<f64, FeasibilityError> {
    let n = table.seat_count.as_usize();
    let in_s = |seat: usize| (mask & (1usize << seat)) != 0;
    let active_seats: Vec<usize> = (0..n).filter(|&s| in_s(s)).collect();
    let active_convs: Vec<&ConverterSpec> = converters
        .iter()
        .copied()
        .filter(|conv| {
            in_s(usize::from(conv.owner))
                && conv
                    .inputs
                    .iter()
                    .all(|&(g, amt)| amt <= 0.0 || in_s(usize::from(g)))
        })
        .collect();

    let num_conv = active_convs.len();
    let num_s = active_seats.len();
    let f_off = num_conv;
    let p_off = f_off + num_s;
    let r_off = p_off + num_s;
    let num_vars = r_off + num_s;

    let mut a = Vec::new();
    let mut b = Vec::new();

    for (c_idx, conv) in active_convs.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[c_idx] = 1.0;
        a.push(row);
        b.push(effective_scaled_cap_with_eff(table, eff, conv, q_scale));
    }

    for (pos, &s) in active_seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[f_off + pos] = 1.0;
        let producer_in_s = in_s(usize::from(table.seats[s].upkeep_good));
        a.push(row);
        b.push(if producer_in_s {
            table.seats[s].upkeep_amount / q_scale
        } else {
            0.0
        });
    }

    for (pos, &k) in active_seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in active_convs.iter().enumerate() {
            for &(raw_g, amt) in &conv.inputs {
                if usize::from(raw_g) == k && amt > 0.0 {
                    let mult = if usize::from(conv.owner) == k {
                        1.0
                    } else {
                        1.0 + eff[k].raw_tariff
                    };
                    row[c_idx] += amt * mult;
                }
            }
        }
        row[r_off + pos] = 1.0;
        a.push(row);
        b.push(eff[k].raw_yield / q_scale);
    }

    for (m_pos, &m) in active_seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        for (c_idx, conv) in active_convs.iter().enumerate() {
            if usize::from(conv.output_good) == m {
                row[c_idx] -= conv.output_amount;
            }
        }
        for (s_pos, &s) in active_seats.iter().enumerate() {
            if usize::from(table.seats[s].upkeep_good) == m {
                let mult = if s == m {
                    1.0
                } else {
                    1.0 + eff[m].refined_tariff
                };
                row[f_off + s_pos] += mult;
            }
        }
        row[p_off + m_pos] = 1.0;
        a.push(row);
        b.push(0.0);
    }

    let raw_salvage_weight = RAW_SALVAGE_PROJECT_FRACTION * mean_proj;
    let mut c = vec![0.0; num_vars];
    let mut base_upkeep_debt_scaled = 0.0;

    for (pos, &s) in active_seats.iter().enumerate() {
        let m = usize::from(table.seats[s].upkeep_good);
        let mult = if s == m {
            1.0
        } else {
            1.0 + eff[m].refined_tariff
        };
        let upkeep_unit_penalty = eff[m].project_value * mult * SHAPLEY_UPKEEP_PENALTY_MULT;
        c[f_off + pos] = upkeep_unit_penalty;
        base_upkeep_debt_scaled += upkeep_unit_penalty * (table.seats[s].upkeep_amount / q_scale);
        c[p_off + pos] = eff[s].project_value;
        c[r_off + pos] = raw_salvage_weight * (1.0 - eff[s].decay_rate);
    }

    match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => Ok((sol.objective - base_upkeep_debt_scaled) * q_scale),
        LpStatus::Infeasible | LpStatus::Unbounded => Err(FeasibilityError::SolverFailure(
            "coalition equity LP failed to solve cleanly".to_owned(),
        )),
    }
}

fn quantity_scale(table: &RingTable) -> f64 {
    let max_qty = table
        .seats
        .iter()
        .map(|s| s.raw_yield_per_turn.max(s.upkeep_amount))
        .fold(0.0_f64, f64::max);
    if max_qty > 0.0 && !(1e-3..=1e6).contains(&max_qty) {
        max_qty
    } else {
        1.0
    }
}

fn eff_quantity_scale(table: &RingTable, eff: &[EffectiveSeatParams]) -> f64 {
    let max_qty = table
        .seats
        .iter()
        .zip(eff.iter())
        .map(|(s, e)| e.raw_yield.max(s.upkeep_amount))
        .fold(0.0_f64, f64::max);
    if max_qty > 0.0 && !(1e-3..=1e6).contains(&max_qty) {
        max_qty
    } else {
        1.0
    }
}

fn effective_scaled_cap(table: &RingTable, conv: &ConverterSpec, q_scale: f64) -> f64 {
    let scaled_cap = conv.max_runs_per_turn / q_scale;
    if (q_scale - 1.0).abs() <= f64::EPSILON {
        return scaled_cap;
    }
    let min_pos_in = conv
        .inputs
        .iter()
        .filter(|&&(_, amt)| amt > 0.0)
        .map(|&(raw_g, amt)| {
            let k = usize::from(raw_g);
            amt / (table.seats[k].raw_yield_per_turn / q_scale).max(1e-12)
        })
        .fold(0.0_f64, f64::max);
    let slack_cap = if min_pos_in > 0.0 {
        (2.0 / min_pos_in).max(4.0)
    } else {
        4.0
    };
    scaled_cap.min(slack_cap)
}

fn effective_scaled_cap_with_eff(
    _table: &RingTable,
    eff: &[EffectiveSeatParams],
    conv: &ConverterSpec,
    q_scale: f64,
) -> f64 {
    let scaled_cap = conv.max_runs_per_turn / q_scale;
    if (q_scale - 1.0).abs() <= f64::EPSILON {
        return scaled_cap;
    }
    let min_pos_in = conv
        .inputs
        .iter()
        .filter(|&&(_, amt)| amt > 0.0)
        .map(|&(raw_g, amt)| {
            let k = usize::from(raw_g);
            amt / (eff[k].raw_yield / q_scale).max(1e-12)
        })
        .fold(0.0_f64, f64::max);
    let slack_cap = if min_pos_in > 0.0 {
        (2.0 / min_pos_in).max(4.0)
    } else {
        4.0
    };
    scaled_cap.min(slack_cap)
}

fn flatten_converters(table: &RingTable) -> Vec<&ConverterSpec> {
    table
        .seats
        .iter()
        .flat_map(|s| s.converters.iter())
        .collect()
}

fn validate_index_bounds(table: &RingTable) -> Result<usize, FeasibilityError> {
    let n = table.seat_count.as_usize();
    if table.seats.len() != n {
        return Err(FeasibilityError::InvalidTable(format!(
            "expected {n} seats, got {}",
            table.seats.len()
        )));
    }
    for (idx, seat) in table.seats.iter().enumerate() {
        if usize::from(seat.upkeep_good) >= n {
            return Err(FeasibilityError::InvalidTable(format!(
                "seat {idx} upkeep_good {} out of range 0..{n}",
                seat.upkeep_good
            )));
        }
        for conv in &seat.converters {
            if usize::from(conv.owner) >= n || usize::from(conv.output_good) >= n {
                return Err(FeasibilityError::InvalidTable(format!(
                    "seat {idx} converter owner/output_good out of range 0..{n}"
                )));
            }
            for &(raw_g, _) in &conv.inputs {
                if usize::from(raw_g) >= n {
                    return Err(FeasibilityError::InvalidTable(format!(
                        "seat {idx} converter input raw_g {raw_g} out of range 0..{n}"
                    )));
                }
            }
        }
    }
    Ok(n)
}

fn validate_config_and_table(
    table: &RingTable,
    config: &GeneratorConfig,
) -> Result<(), FeasibilityError> {
    if !config.ev_tolerance.is_finite() || config.ev_tolerance <= 0.0 || config.ev_tolerance >= 1.0
    {
        return Err(FeasibilityError::InvalidTable(
            "ev_tolerance must lie in (0.0, 1.0)".to_owned(),
        ));
    }
    if !config.min_surplus_ratio.is_finite() || config.min_surplus_ratio <= 0.0 {
        return Err(FeasibilityError::InvalidTable(
            "min_surplus_ratio must be strictly positive".to_owned(),
        ));
    }

    let n = table.seat_count.as_usize();
    if table.seats.len() != n {
        return Err(FeasibilityError::InvalidTable(format!(
            "seat vector length {} does not match seat_count {n}",
            table.seats.len()
        )));
    }
    if table.numeraire_code != NUMERAIRE_CODE {
        return Err(FeasibilityError::InvalidTable(format!(
            "numeraire_code must be fixed {NUMERAIRE_CODE:?}, got {:?}",
            table.numeraire_code
        )));
    }
    if table.season_turn_window.0 < 24 || table.season_turn_window.1 < table.season_turn_window.0 {
        return Err(FeasibilityError::InvalidTable(
            "invalid season_turn_window bounds".to_owned(),
        ));
    }

    let mut used_codes = HashSet::new();
    for (idx, seat) in table.seats.iter().enumerate() {
        if usize::from(seat.seat) != idx {
            return Err(FeasibilityError::InvalidTable(format!(
                "seat index mismatch at slot {idx}: found {}",
                seat.seat
            )));
        }
        if usize::from(seat.raw_good) != idx || usize::from(seat.refined_good) != idx {
            return Err(FeasibilityError::InvalidTable(format!(
                "seat {idx} good indices must match seat index"
            )));
        }
        let expected_upkeep_good = ((idx + n - 1) % n) as u8;
        if seat.upkeep_good != expected_upkeep_good {
            return Err(FeasibilityError::InvalidTable(format!(
                "seat {idx} upkeep_good must be chiral predecessor {expected_upkeep_good}, got {}",
                seat.upkeep_good
            )));
        }
        if !used_codes.insert(seat.raw_code) || !used_codes.insert(seat.refined_code) {
            return Err(FeasibilityError::InvalidTable(
                "commodity currency codes must be pairwise distinct".to_owned(),
            ));
        }
        for (label, val) in [
            ("raw_yield_per_turn", seat.raw_yield_per_turn),
            ("refined_project_value", seat.refined_project_value),
            ("upkeep_amount", seat.upkeep_amount),
        ] {
            if !val.is_finite() || val <= 0.0 {
                return Err(FeasibilityError::InvalidTable(format!(
                    "seat {idx} {label} must be finite and positive"
                )));
            }
        }
        for tariff in [seat.raw_export_tariff, seat.refined_export_tariff] {
            if !tariff.is_finite() || !(0.0..=0.35).contains(&tariff) {
                return Err(FeasibilityError::InvalidTable(format!(
                    "seat {idx} export tariff {tariff} out of [0.0, 0.35]"
                )));
            }
        }
        if !seat.perishable_decay_rate.is_finite()
            || !(0.0..1.0).contains(&seat.perishable_decay_rate)
        {
            return Err(FeasibilityError::InvalidTable(format!(
                "seat {idx} perishable_decay_rate out of [0.0, 1.0)"
            )));
        }
        if seat.converters.is_empty() {
            return Err(FeasibilityError::InvalidTable(format!(
                "seat {idx} must have at least one converter"
            )));
        }
        for conv in &seat.converters {
            if usize::from(conv.owner) != idx || usize::from(conv.output_good) != idx {
                return Err(FeasibilityError::InvalidTable(format!(
                    "seat {idx} converter owner/output_good must belong to seat {idx}"
                )));
            }
            if conv.inputs.len() < 2
                || !conv.output_amount.is_finite()
                || conv.output_amount <= 0.0
                || !conv.max_runs_per_turn.is_finite()
                || conv.max_runs_per_turn <= 0.0
            {
                return Err(FeasibilityError::InvalidTable(format!(
                    "seat {idx} converter has invalid stoichiometry or capacity"
                )));
            }
            let mut seen_inputs = HashSet::new();
            for &(raw_g, amt) in &conv.inputs {
                if usize::from(raw_g) >= n
                    || !amt.is_finite()
                    || amt <= 0.0
                    || !seen_inputs.insert(raw_g)
                {
                    return Err(FeasibilityError::InvalidTable(format!(
                        "seat {idx} converter input ({raw_g}, {amt}) is invalid"
                    )));
                }
            }
        }
    }

    for shock in &table.shocks {
        if usize::from(shock.target_seat) >= n
            || shock.forecast_turn != shock.hint_turn.saturating_add(1)
            || shock.realize_turn != shock.forecast_turn.saturating_add(1)
            || shock.realize_turn > table.season_turn_window.1
            || !shock.magnitude.is_finite()
            || !(0.0..=1.0).contains(&shock.magnitude)
            || shock.magnitude <= 0.0
        {
            return Err(FeasibilityError::InvalidTable(
                "invalid horizon shock schedule entry".to_owned(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CurrencyCode, GENERATOR_VERSION, SeatCount, SeatSpec};

    fn make_symmetric_table(seat_count: SeatCount) -> RingTable {
        let n = seat_count.as_usize();
        let mut seats = Vec::with_capacity(n);
        for i in 0..n {
            let r_code = CurrencyCode::from_bytes([b'Q', b'R', b'A' + (i as u8)]);
            let m_code = CurrencyCode::from_bytes([b'Q', b'M', b'A' + (i as u8)]);
            let (Ok(raw_code), Ok(refined_code)) = (r_code, m_code) else {
                panic!("valid commodity code");
            };
            seats.push(SeatSpec {
                seat: i as u8,
                raw_good: i as u8,
                raw_code,
                raw_yield_per_turn: 10.0,
                refined_good: i as u8,
                refined_code,
                refined_project_value: 2.0,
                upkeep_good: ((i + n - 1) % n) as u8,
                upkeep_amount: 2.0,
                raw_export_tariff: 0.10,
                refined_export_tariff: 0.10,
                perishable_decay_rate: 0.20,
                converters: vec![ConverterSpec {
                    owner: i as u8,
                    inputs: vec![(((i + 1) % n) as u8, 1.7), (((i + 2) % n) as u8, 2.3)],
                    output_good: i as u8,
                    output_amount: 3.2,
                    max_runs_per_turn: 4.0,
                    is_secondary: false,
                }],
            });
        }
        RingTable {
            generator_version: GENERATOR_VERSION.to_owned(),
            config: GeneratorConfig::default(),
            seed: 1,
            attempt: 0,
            seat_count,
            numeraire_code: NUMERAIRE_CODE.to_owned(),
            season_turn_window: (24, 28),
            seats,
            shocks: Vec::new(),
        }
    }

    #[test]
    fn symmetric_ring_tables_satisfy_all_three_invariants_under_both_ev_methods() {
        for ev_method in [EvMethod::ShadowPrice, EvMethod::Shapley] {
            let cfg = GeneratorConfig {
                ev_method,
                ..GeneratorConfig::default()
            };
            for sc in SeatCount::ALL {
                let mut table = make_symmetric_table(sc);
                table.config = cfg.clone();
                let Ok(report) = check_feasibility(&table, &cfg) else {
                    panic!("symmetric table for {sc:?} / {ev_method:?} must be feasible");
                };
                assert_eq!(report.ev_method, ev_method);
                assert!(report.max_1seat_upkeep_coverage < 1e-9);
                assert!(report.max_2seat_upkeep_coverage < 1e-9);
                assert!((report.max_3seat_upkeep_coverage - (1.0 / 3.0)).abs() < 1e-6);
                if sc.as_usize() >= 5 {
                    assert!((report.max_4seat_upkeep_coverage.unwrap_or(0.0) - 0.5).abs() < 1e-6);
                }
                assert!(report.surplus_to_upkeep_ratio >= cfg.min_surplus_ratio);
                assert!(report.max_relative_ev_deviation < 1e-9);

                if ev_method == EvMethod::ShadowPrice {
                    let sum_evs: f64 = report.seat_expected_values.iter().sum();
                    assert!(
                        (sum_evs - report.net_converter_surplus).abs() < 1e-5,
                        "shadow-price EVs must conserve grand net surplus: {sum_evs} vs {}",
                        report.net_converter_surplus
                    );
                }
            }
        }
    }

    #[test]
    fn symmetric_equal_input_even_ring_resolves_degenerate_dual_symmetrically() {
        let mut table = make_symmetric_table(SeatCount::Four);
        for (i, seat) in table.seats.iter_mut().enumerate().take(4) {
            seat.converters[0].inputs =
                vec![(((i + 1) % 4) as u8, 2.0), (((i + 2) % 4) as u8, 2.0)];
        }
        let cfg = GeneratorConfig::default();
        let Ok(report) = check_feasibility(&table, &cfg) else {
            panic!("degenerate symmetric N=4 ring must balance dual face symmetrically");
        };
        assert!(report.max_relative_ev_deviation < 1e-9);
    }

    #[test]
    fn negative_controls_reject_each_invariant_and_edge_cases() {
        let cfg = GeneratorConfig::default();

        // 0. Non-CRED numeraire rejected.
        let mut t0 = make_symmetric_table(SeatCount::Four);
        t0.numeraire_code = "XRP".to_owned();
        assert!(matches!(
            check_feasibility(&t0, &cfg),
            Err(FeasibilityError::InvalidTable(_))
        ));

        // 1a. Pair covers upkeep when converter uses own raw + adjacent raw.
        let mut t1 = make_symmetric_table(SeatCount::Five);
        t1.seats[0].converters[0].inputs = vec![(0, 2.0), (1, 2.0)];
        assert!(matches!(
            check_feasibility(&t1, &cfg),
            Err(FeasibilityError::PairCoversUpkeep { pair: (0, 1), .. })
        ));

        // 1b. Tiny-capacity secondary autarkic converter below 1e-7 LP coverage still rejected structurally (C3).
        let mut t_tiny = make_symmetric_table(SeatCount::Five);
        t_tiny.seats[0].converters.push(ConverterSpec {
            owner: 0,
            inputs: vec![(1, 1.0), (0, 1.0)],
            output_good: 0,
            output_amount: 1e-5,
            max_runs_per_turn: 1e-4,
            is_secondary: true,
        });
        assert!(matches!(
            check_feasibility(&t_tiny, &cfg),
            Err(FeasibilityError::PairCoversUpkeep { pair: (0, 1), .. })
        ));

        // 2. Insufficient surplus when raw yields are too low to pay upkeep.
        let mut t2 = make_symmetric_table(SeatCount::Five);
        for s in &mut t2.seats {
            s.raw_yield_per_turn = 1.0;
        }
        assert!(matches!(
            check_feasibility(&t2, &cfg),
            Err(FeasibilityError::InsufficientSurplus { .. })
        ));

        // 3. Seat EV imbalance when one seat gets a huge yield boost (both EV methods).
        for ev_method in [EvMethod::ShadowPrice, EvMethod::Shapley] {
            let mut t3 = make_symmetric_table(SeatCount::Five);
            t3.seats[0].raw_yield_per_turn = 25.0;
            let c_ev = GeneratorConfig {
                ev_method,
                ..GeneratorConfig::default()
            };
            assert!(matches!(
                check_feasibility(&t3, &c_ev),
                Err(FeasibilityError::SeatEvOutOfTolerance { .. })
            ));
        }

        // 4. Small-unit scale invariance (`x1e-12` yield and upkeep, C4).
        let mut t_small = make_symmetric_table(SeatCount::Five);
        for s in &mut t_small.seats {
            s.raw_yield_per_turn *= 1e-12;
            s.upkeep_amount *= 1e-12;
        }
        assert!(check_feasibility(&t_small, &cfg).is_ok());
        let shapley_cfg = GeneratorConfig {
            ev_method: EvMethod::Shapley,
            ..GeneratorConfig::default()
        };
        assert!(check_feasibility(&t_small, &shapley_cfg).is_ok());

        // 5. Public coalition_upkeep_coverage validates malformed indices instead of panicking (C5).
        let mut t_bad = make_symmetric_table(SeatCount::Four);
        t_bad.seats[0].converters[0].owner = 99;
        assert!(matches!(
            coalition_upkeep_coverage(&t_bad, &[0, 1]),
            Err(FeasibilityError::InvalidTable(_))
        ));
    }
}
