//! Three-invariant linear-programming feasibility filter for chiral ring tables.
//!
//! Proves on every candidate [`RingTable`]:
//! 1. **Zero 2-seat autarky:** every unordered seat pair `{i, j}` covers `0.0`
//!    of its own turn upkeep internally via [`coalition_upkeep_coverage`].
//! 2. **Strict aggregate converter surplus:** full-table conversion covers 100%
//!    of mandatory upkeep with net refined surplus exceeding `min_surplus_ratio`.
//! 3. **Seat expected-value balance:** Shapley season expected values across
//!    the `2^N` coalition LPs fall within `±config.ev_tolerance` of the table mean.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::lp::{LpStatus, solve_lp};
use crate::schema::{
    ConverterSpec, FeasibilityError, GeneratorConfig, HorizonShockKind, RingTable,
};

const ZERO_TOL: f64 = 1e-7;
const SEASON_EXPECTED_TURNS: f64 = 26.0;

/// Certified LP metrics and optimal grand-coalition production plan for a [`RingTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeasibilityReport {
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
    /// Per-seat expected value per turn under the `2^N` coalition Shapley evaluation.
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

    // Invariant 1: 0.0 upkeep coverage for every 1-seat and 2-seat subset.
    let mut max_1seat = 0.0_f64;
    for i in 0..n {
        let cov = coalition_upkeep_coverage(table, &[i as u8])?;
        max_1seat = max_1seat.max(cov);
        if cov > ZERO_TOL {
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
            if cov > ZERO_TOL {
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

    // Invariant 2: strict aggregate converter surplus over mandatory upkeep.
    let (total_upkeep_value, net_converter_surplus, optimal_converter_runs) =
        solve_grand_surplus(table)?;
    let surplus_to_upkeep_ratio = if total_upkeep_value > 0.0 {
        net_converter_surplus / total_upkeep_value
    } else {
        0.0
    };
    if net_converter_surplus <= ZERO_TOL || surplus_to_upkeep_ratio < config.min_surplus_ratio {
        return Err(FeasibilityError::InsufficientSurplus {
            net_surplus: net_converter_surplus,
            required_ratio: config.min_surplus_ratio,
            actual_ratio: surplus_to_upkeep_ratio,
        });
    }

    // Invariant 3: Shapley seat expected values within `±config.ev_tolerance`.
    let seat_expected_values = compute_shapley_seat_evs(table)?;
    let mean_seat_ev = seat_expected_values.iter().sum::<f64>() / (n as f64);
    if !mean_seat_ev.is_finite() || mean_seat_ev <= ZERO_TOL {
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

    Ok(FeasibilityReport {
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

/// Computes the exact maximum fraction `[0.0, 1.0]` of the coalition's combined
/// upkeep that `seats` can satisfy internally without trading outside `seats`.
pub fn coalition_upkeep_coverage(table: &RingTable, seats: &[u8]) -> Result<f64, FeasibilityError> {
    let n = table.seat_count.as_usize();
    if table.seats.len() != n {
        return Err(FeasibilityError::InvalidTable(format!(
            "expected {n} seats, got {}",
            table.seats.len()
        )));
    }
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
    if total_upkeep <= ZERO_TOL {
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

    let num_conv = active_convs.len();
    let num_s = active_seats.len();
    let mut seat_pos = vec![None; n];
    for (pos, &s) in active_seats.iter().enumerate() {
        seat_pos[s] = Some(pos);
    }

    // Variables: x_0..x_{num_conv-1} (active converter runs), f_0..f_{num_s-1} (upkeep satisfied).
    let num_vars = num_conv + num_s;
    let mut a = Vec::new();
    let mut b = Vec::new();

    for (c_idx, conv) in active_convs.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[c_idx] = 1.0;
        a.push(row);
        b.push(conv.max_runs_per_turn);
    }

    for (pos, &s) in active_seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[num_conv + pos] = 1.0;
        a.push(row);
        let producer_in_s = active_convs
            .iter()
            .any(|conv| conv.output_good == table.seats[s].upkeep_good);
        b.push(if producer_in_s {
            table.seats[s].upkeep_amount
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
        b.push(seat_k.raw_yield_per_turn);
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

    let mut c = vec![0.0; num_vars];
    for cell in &mut c[num_conv..num_vars] {
        *cell = 1.0 / total_upkeep;
    }

    match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => Ok(sol.objective.clamp(0.0, 1.0)),
        LpStatus::Infeasible => Ok(0.0),
        LpStatus::Unbounded => Err(FeasibilityError::SolverFailure(
            "coalition upkeep LP reported unbounded".to_owned(),
        )),
    }
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
    // Variables: x_0..x_{C-1} (converter runs), s_0..s_{N-1} (net refined surplus per good).
    let num_vars = num_conv + n;
    let mut a = Vec::new();
    let mut b = Vec::new();

    for (c_idx, conv) in converters.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[c_idx] = 1.0;
        a.push(row);
        b.push(conv.max_runs_per_turn);
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
        b.push(seat_k.raw_yield_per_turn);
    }

    let mut upkeep_reqs = vec![0.0; n];
    for (s, seat_s) in table.seats.iter().enumerate() {
        let m = usize::from(seat_s.upkeep_good);
        let tariff_mult = if s == m {
            1.0
        } else {
            1.0 + table.seats[m].refined_export_tariff
        };
        upkeep_reqs[m] += seat_s.upkeep_amount * tariff_mult;
    }

    let mut total_upkeep_value = 0.0;
    for m in 0..n {
        total_upkeep_value += upkeep_reqs[m] * table.seats[m].refined_project_value;
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
        b.push(upkeep_reqs[m]);
        a.push(ge_row);
        b.push(-upkeep_reqs[m]);
    }

    let mut c = vec![0.0; num_vars];
    for (m, seat_m) in table.seats.iter().enumerate() {
        c[num_conv + m] = seat_m.refined_project_value;
    }

    match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => Ok((
            total_upkeep_value,
            sol.objective,
            sol.primal[..num_conv].to_vec(),
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

fn compute_shapley_seat_evs(table: &RingTable) -> Result<Vec<f64>, FeasibilityError> {
    let n = table.seat_count.as_usize();
    let converters = flatten_converters(table);
    let eff = effective_season_params(table);
    let total_masks = 1usize << n;
    let mut coalition_values = vec![0.0_f64; total_masks];

    let mean_proj = eff.iter().map(|p| p.project_value).sum::<f64>() / (n as f64);

    for (mask, slot) in coalition_values.iter_mut().enumerate().skip(1) {
        *slot = evaluate_coalition_net_equity(table, &converters, &eff, mean_proj, mask)?;
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

fn evaluate_coalition_net_equity(
    table: &RingTable,
    converters: &[&ConverterSpec],
    eff: &[EffectiveSeatParams],
    mean_proj: f64,
    mask: usize,
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
    // Compressed variables strictly over active converters and active seats in coalition `S`:
    //   x_0..x_{num_conv-1}     (runnable converter runs)
    //   f_0..f_{num_s-1}        (satisfied upkeep for seats in S)
    //   p_0..p_{num_s-1}        (refined syndicate project output for M_m with m in S)
    //   r_0..r_{num_s-1}        (unconverted raw salvage remaining for R_k with k in S)
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
        b.push(conv.max_runs_per_turn);
    }

    for (pos, &s) in active_seats.iter().enumerate() {
        let mut row = vec![0.0; num_vars];
        row[f_off + pos] = 1.0;
        let producer_in_s = in_s(usize::from(table.seats[s].upkeep_good));
        a.push(row);
        b.push(if producer_in_s {
            table.seats[s].upkeep_amount
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
        b.push(eff[k].raw_yield);
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

    // Raw salvage value strictly lower than conversion parity so conversion dominates hoarding.
    let raw_salvage_weight = 0.25 * mean_proj;
    let mut c = vec![0.0; num_vars];
    let mut base_upkeep_debt = 0.0;

    for (pos, &s) in active_seats.iter().enumerate() {
        let m = usize::from(table.seats[s].upkeep_good);
        let mult = if s == m {
            1.0
        } else {
            1.0 + eff[m].refined_tariff
        };
        let upkeep_unit_penalty = eff[m].project_value * mult * 1.10;
        c[f_off + pos] = upkeep_unit_penalty;
        base_upkeep_debt += upkeep_unit_penalty * table.seats[s].upkeep_amount;
        c[p_off + pos] = eff[s].project_value;
        c[r_off + pos] = raw_salvage_weight * (1.0 - eff[s].decay_rate);
    }

    match solve_lp(&a, &b, &c).map_err(|e| FeasibilityError::SolverFailure(e.0))? {
        LpStatus::Optimal(sol) => Ok(sol.objective - base_upkeep_debt),
        LpStatus::Infeasible | LpStatus::Unbounded => Err(FeasibilityError::SolverFailure(
            "coalition equity LP failed to solve cleanly".to_owned(),
        )),
    }
}

fn flatten_converters(table: &RingTable) -> Vec<&ConverterSpec> {
    table
        .seats
        .iter()
        .flat_map(|s| s.converters.iter())
        .collect()
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
    if table.season_turn_window.0 < 24 || table.season_turn_window.1 < table.season_turn_window.0 {
        return Err(FeasibilityError::InvalidTable(
            "invalid season_turn_window bounds".to_owned(),
        ));
    }

    let mut used_codes = HashSet::new();
    used_codes.insert(table.numeraire_code);

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
                "commodity and numeraire currency codes must be pairwise distinct".to_owned(),
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
    use crate::schema::{CurrencyCode, SeatCount, SeatSpec};

    fn make_symmetric_table(seat_count: SeatCount) -> RingTable {
        let n = seat_count.as_usize();
        let Ok(numeraire_code) = CurrencyCode::from_bytes(*b"CRD") else {
            panic!("valid numeraire");
        };
        let mut seats = Vec::with_capacity(n);
        for i in 0..n {
            let r_code = CurrencyCode::from_bytes([b'R', b'A', b'A' + (i as u8)]);
            let m_code = CurrencyCode::from_bytes([b'M', b'A', b'A' + (i as u8)]);
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
                    inputs: vec![(((i + 1) % n) as u8, 2.0), (((i + 2) % n) as u8, 2.0)],
                    output_good: i as u8,
                    output_amount: 3.2,
                    max_runs_per_turn: 4.0,
                    is_secondary: false,
                }],
            });
        }
        RingTable {
            seed: 1,
            attempt: 0,
            seat_count,
            numeraire_code,
            season_turn_window: (24, 28),
            seats,
            shocks: Vec::new(),
        }
    }

    #[test]
    fn symmetric_ring_tables_satisfy_all_three_invariants() {
        let cfg = GeneratorConfig::default();
        for sc in SeatCount::ALL {
            let table = make_symmetric_table(sc);
            let Ok(report) = check_feasibility(&table, &cfg) else {
                panic!("symmetric table for {sc:?} must be feasible");
            };
            assert!(report.max_1seat_upkeep_coverage < 1e-9);
            assert!(report.max_2seat_upkeep_coverage < 1e-9);
            assert!((report.max_3seat_upkeep_coverage - (1.0 / 3.0)).abs() < 1e-6);
            if sc.as_usize() >= 5 {
                assert!((report.max_4seat_upkeep_coverage.unwrap_or(0.0) - 0.5).abs() < 1e-6);
            }
            assert!(report.surplus_to_upkeep_ratio >= cfg.min_surplus_ratio);
            assert!(report.max_relative_ev_deviation < 1e-9);
        }
    }

    #[test]
    fn negative_controls_reject_each_invariant_violation() {
        let cfg = GeneratorConfig::default();

        // 1. Pair covers upkeep when converter uses own raw + adjacent raw.
        let mut t1 = make_symmetric_table(SeatCount::Five);
        t1.seats[0].converters[0].inputs = vec![(0, 2.0), (1, 2.0)];
        assert!(matches!(
            check_feasibility(&t1, &cfg),
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

        // 3. Seat EV imbalance when one seat gets a huge yield boost.
        let mut t3 = make_symmetric_table(SeatCount::Five);
        t3.seats[0].raw_yield_per_turn = 25.0;
        assert!(matches!(
            check_feasibility(&t3, &cfg),
            Err(FeasibilityError::SeatEvOutOfTolerance { .. })
        ));
    }
}
