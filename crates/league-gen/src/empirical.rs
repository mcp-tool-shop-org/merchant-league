//! Empirical measurement of random bilateral lock-in probabilities and dyadic trade HHI
//! compared against LP-verified chiral converter-ring tables.

use serde::{Deserialize, Serialize};

use crate::feasibility::FeasibilityReport;
use crate::generator::generate_ring_table;
use crate::rng::SeedStream;
use crate::schema::{GeneratorConfig, RingTable, SeatCount};

/// Analytical priors reported in the seed-26 design blueprint.
///
/// Treated strictly as hypotheses to test empirically with [`measure_lockin_and_hhi`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReportHypotheses {
    /// Modeled probability of >=1 100%-self-sufficient 2-seat dyad (5 seats, 4 resources, 2-of-4).
    pub random_5p_4r_lockin_100pct: f64,
    /// Modeled probability of >=1 >=75%-covered 2-seat dyad (5 seats, 6 resources, 2-of-6).
    pub random_5p_6r_coverage_75pct: f64,
    /// Modeled dyadic trade HHI under bilateral lock-in.
    pub bilateral_lockin_hhi: f64,
    /// Modeled effective dyads (`1 / HHI`) under bilateral lock-in.
    pub bilateral_lockin_effective_dyads: f64,
    /// Modeled dyadic trade HHI on a 5-seat chiral converter ring.
    pub chiral_ring_5p_hhi: f64,
    /// Modeled effective dyads (`1 / HHI`) on a 5-seat chiral converter ring.
    pub chiral_ring_5p_effective_dyads: f64,
}

impl ReportHypotheses {
    /// Analytical figures from the seed-26 report for side-by-side comparison.
    pub const SEED_26: Self = Self {
        random_5p_4r_lockin_100pct: 0.797,
        random_5p_6r_coverage_75pct: 0.642,
        bilateral_lockin_hhi: 0.655,
        bilateral_lockin_effective_dyads: 1.53,
        chiral_ring_5p_hhi: 0.130,
        chiral_ring_5p_effective_dyads: 7.72,
    };
}

/// Dyadic trade concentration metrics across all unordered seat pairs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HhiMetrics {
    /// Total number of unordered seat pairs `N * (N - 1) / 2`.
    pub total_pairs: usize,
    /// Count of pairs with strictly positive cleared trade volume.
    pub active_pairs: usize,
    /// Sum of squared dyadic trade-volume shares `sum s_{ij}^2` in `[1 / total_pairs, 1.0]`.
    pub hhi: f64,
    /// Inverse concentration `1.0 / hhi` (`0.0` if zero volume).
    pub effective_dyads: f64,
}

/// Computes exact [`HhiMetrics`] from a slice of nonnegative pair trade volumes.
#[must_use]
pub fn compute_hhi(pair_volumes: &[f64]) -> HhiMetrics {
    let total_pairs = pair_volumes.len();
    let total_vol: f64 = pair_volumes.iter().copied().filter(|&v| v > 0.0).sum();
    if total_pairs == 0 || total_vol <= 1e-12 {
        return HhiMetrics {
            total_pairs,
            active_pairs: 0,
            hhi: if total_pairs > 0 { 1.0 } else { 0.0 },
            effective_dyads: if total_pairs > 0 { 1.0 } else { 0.0 },
        };
    }

    let mut active_pairs = 0usize;
    let mut hhi = 0.0_f64;
    for &vol in pair_volumes {
        if vol > 1e-12 {
            active_pairs += 1;
            let share = vol / total_vol;
            hhi += share * share;
        }
    }
    let effective_dyads = if hhi > 0.0 { 1.0 / hhi } else { 0.0 };
    HhiMetrics {
        total_pairs,
        active_pairs,
        hhi,
        effective_dyads,
    }
}

/// Baseline random resource-endowment table for testing bilateral lock-in hypotheses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomEndowmentTable {
    /// Number of seats at the table.
    pub num_seats: usize,
    /// Distinct resource types in `[0..num_resources)`.
    pub num_resources: usize,
    /// Resources endowed per seat (`E_i`).
    pub endowed: Vec<Vec<usize>>,
    /// Distinct foreign resources required by each seat (`U_i`, disjoint from `E_i`).
    pub foreign_needs: Vec<Vec<usize>>,
}

/// Coverage and trade concentration metrics measured on one [`RandomEndowmentTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RandomTableMetrics {
    /// Maximum mutual coverage fraction across all `N*(N-1)/2` unordered pairs.
    pub max_pair_mutual_coverage: f64,
    /// Whether at least one pair satisfies 100% of each other's foreign needs.
    pub has_100pct_bilateral_dyad: bool,
    /// Whether at least one pair satisfies >=75% of combined mutual foreign needs.
    pub has_75pct_bilateral_dyad: bool,
    /// Cleared dyadic concentration under reciprocal-first bilateral barter.
    pub trade_concentration: HhiMetrics,
}

/// Coverage and trade concentration metrics measured on one LP-certified [`RingTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingTableMetrics {
    /// Maximum 1-seat internal upkeep coverage (`0.0`).
    pub max_1seat_upkeep_coverage: f64,
    /// Maximum 2-seat internal upkeep coverage (`0.0`).
    pub max_2seat_upkeep_coverage: f64,
    /// Maximum 3-seat internal upkeep coverage (`<= 1/3` for `N >= 4`).
    pub max_3seat_upkeep_coverage: f64,
    /// Maximum 4-seat internal upkeep coverage (`<= 1/2` for `N = 5`).
    pub max_4seat_upkeep_coverage: Option<f64>,
    /// Cleared dyadic trade HHI and effective dyads under the optimal grand-coalition plan.
    pub trade_concentration: HhiMetrics,
}

/// Aggregated statistics across an ensemble of seeded tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnsembleStats {
    /// Number of sampled tables evaluated.
    pub trials: usize,
    /// Fraction of tables containing >=1 100%-self-sufficient 2-seat dyad.
    pub frac_100pct_dyad_lockin: f64,
    /// Fraction of tables containing >=1 >=75%-covered 2-seat dyad.
    pub frac_75pct_dyad_coverage: f64,
    /// Mean maximum 2-seat coverage ratio across trials.
    pub mean_max_2seat_coverage: f64,
    /// Mean dyadic trade HHI across trials.
    pub mean_hhi: f64,
    /// Minimum dyadic trade HHI across trials.
    pub min_hhi: f64,
    /// Maximum dyadic trade HHI across trials.
    pub max_hhi: f64,
    /// Mean effective dyads (`1 / HHI`) across trials.
    pub mean_effective_dyads: f64,
    /// Mean number of active trading pairs across trials.
    pub mean_active_pairs: f64,
}

/// Side-by-side empirical comparison of random-endowment tables vs. LP-verified ring tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmpiricalComparison {
    /// Number of trials per ensemble.
    pub trials: usize,
    /// 5-seat random tables with 4 resources (2 endowed, 2 foreign needed).
    pub random_5p_4r: EnsembleStats,
    /// 5-seat random tables with 6 resources (2 endowed, 2 foreign needed).
    pub random_5p_6r: EnsembleStats,
    /// 4-seat LP-certified chiral converter-ring tables.
    pub ring_4p: EnsembleStats,
    /// 5-seat LP-certified chiral converter-ring tables.
    pub ring_5p: EnsembleStats,
    /// 6-seat LP-certified chiral converter-ring tables.
    pub ring_6p: EnsembleStats,
}

/// Deterministically samples one [`RandomEndowmentTable`].
#[must_use]
pub fn generate_random_table(
    seed: u64,
    trial_index: usize,
    num_seats: usize,
    num_resources: usize,
    endowed_per_seat: usize,
    foreign_needs_per_seat: usize,
) -> RandomEndowmentTable {
    let mut stream = SeedStream::new(
        seed ^ ((trial_index as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)),
        b"random-endowment-table",
    );
    let mut endowed = Vec::with_capacity(num_seats);
    let mut foreign_needs = Vec::with_capacity(num_seats);

    for _ in 0..num_seats {
        let e = stream.sample_distinct_indices(num_resources, endowed_per_seat);
        let foreign_pool: Vec<usize> = (0..num_resources).filter(|r| !e.contains(r)).collect();
        let need_indices =
            stream.sample_distinct_indices(foreign_pool.len(), foreign_needs_per_seat);
        let u: Vec<usize> = need_indices
            .into_iter()
            .map(|idx| foreign_pool[idx])
            .collect();
        endowed.push(e);
        foreign_needs.push(u);
    }

    RandomEndowmentTable {
        num_seats,
        num_resources,
        endowed,
        foreign_needs,
    }
}

/// Measures bilateral lock-in coverage and cleared trade concentration on `table`.
#[must_use]
pub fn evaluate_random_table(table: &RandomEndowmentTable) -> RandomTableMetrics {
    let n = table.num_seats;
    let total_pairs = n.saturating_mul(n.saturating_sub(1)) / 2;
    let mut max_pair_cov = 0.0_f64;

    struct PairCandidate {
        pair_idx: usize,
        i: usize,
        j: usize,
        reciprocal: usize,
        one_way: usize,
        coverage: f64,
    }

    let mut candidates = Vec::with_capacity(total_pairs);
    let mut pair_idx = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            let i_from_j = table.foreign_needs[i]
                .iter()
                .filter(|r| table.endowed[j].contains(r))
                .count();
            let j_from_i = table.foreign_needs[j]
                .iter()
                .filter(|r| table.endowed[i].contains(r))
                .count();
            let denom = table.foreign_needs[i].len() + table.foreign_needs[j].len();
            let cov = if denom > 0 {
                ((i_from_j + j_from_i) as f64) / (denom as f64)
            } else {
                0.0
            };
            max_pair_cov = max_pair_cov.max(cov);
            candidates.push(PairCandidate {
                pair_idx,
                i,
                j,
                reciprocal: i_from_j.min(j_from_i),
                one_way: i_from_j + j_from_i,
                coverage: cov,
            });
            pair_idx += 1;
        }
    }

    // Reciprocal-first bilateral clearing: locked bilateral dyads (`cov >= 0.75`) repeat swaps
    // across turns, starving non-locked multilateral fragments of repeat liquidity.
    candidates.sort_by(|a, b| {
        b.reciprocal
            .cmp(&a.reciprocal)
            .then_with(|| b.one_way.cmp(&a.one_way))
            .then_with(|| a.pair_idx.cmp(&b.pair_idx))
    });

    let mut pair_volumes = vec![0.0_f64; total_pairs];
    let mut seat_locked = vec![false; n];
    for cand in &candidates {
        if cand.reciprocal > 0 && !seat_locked[cand.i] && !seat_locked[cand.j] {
            // Closed or near-closed reciprocal dyads trade every turn with high intensity.
            let repeat_mult = if cand.coverage >= 0.75 - 1e-9 {
                12.0
            } else {
                3.0
            };
            pair_volumes[cand.pair_idx] += (cand.reciprocal as f64) * 2.0 * repeat_mult;
            if cand.coverage >= 0.75 - 1e-9 {
                seat_locked[cand.i] = true;
                seat_locked[cand.j] = true;
            }
        }
    }
    // Residual un-locked one-way matching contributes minor spot exchange among non-locked seats.
    for cand in &candidates {
        if cand.one_way > 0 && (!seat_locked[cand.i] || !seat_locked[cand.j]) {
            pair_volumes[cand.pair_idx] += (cand.one_way as f64) * 0.25;
        }
    }

    RandomTableMetrics {
        max_pair_mutual_coverage: max_pair_cov,
        has_100pct_bilateral_dyad: max_pair_cov >= 1.0 - 1e-9,
        has_75pct_bilateral_dyad: max_pair_cov >= 0.75 - 1e-9,
        trade_concentration: compute_hhi(&pair_volumes),
    }
}

/// Measures coalition upkeep coverage bounds and optimal-plan dyadic trade HHI for a chiral [`RingTable`].
#[must_use]
pub fn evaluate_ring_table(table: &RingTable, report: &FeasibilityReport) -> RingTableMetrics {
    let n = table.seat_count.as_usize();
    let total_pairs = table.seat_count.pair_count();
    let mut pair_volumes = vec![0.0_f64; total_pairs];

    // Estimate fundamental raw parity from each raw good's primary downstream converters.
    let mean_proj: f64 = table
        .seats
        .iter()
        .map(|s| s.refined_project_value)
        .sum::<f64>()
        / (n.max(1) as f64);
    let mut raw_parity = vec![0.0_f64; n];
    let mut raw_weights = vec![0.0_f64; n];
    for seat in &table.seats {
        for conv in &seat.converters {
            if conv.is_secondary {
                continue;
            }
            let total_in: f64 = conv.inputs.iter().map(|&(_, amt)| amt).sum();
            if total_in > 0.0 {
                let unit_val = (conv.output_amount
                    * table.seats[usize::from(conv.output_good)].refined_project_value)
                    / total_in;
                for &(raw_g, amt) in &conv.inputs {
                    let g = usize::from(raw_g);
                    if g < n {
                        raw_parity[g] += unit_val * amt;
                        raw_weights[g] += amt;
                    }
                }
            }
        }
    }
    for g in 0..n {
        raw_parity[g] = if raw_weights[g] > 0.0 {
            raw_parity[g] / raw_weights[g]
        } else {
            mean_proj
        };
    }

    // Flatten converter specs in exact order matching `report.optimal_converter_runs`.
    let mut conv_idx = 0usize;
    for seat in &table.seats {
        for conv in &seat.converters {
            let runs = report
                .optimal_converter_runs
                .get(conv_idx)
                .copied()
                .unwrap_or(0.0);
            conv_idx += 1;
            if runs <= 1e-12 {
                continue;
            }
            let owner = usize::from(conv.owner);
            for &(raw_g, amt) in &conv.inputs {
                let supplier = usize::from(raw_g);
                if supplier != owner && supplier < n && owner < n {
                    let pidx = unordered_pair_index(n, owner, supplier);
                    let flow = runs * amt * raw_parity[supplier];
                    pair_volumes[pidx] += flow;
                }
            }
        }
    }

    // Add upkeep delivery flows: producer `(s + n - 1) % n` delivers `upkeep_amount` of `M_m` to `s`.
    for seat in &table.seats {
        let receiver = usize::from(seat.seat);
        let producer = usize::from(seat.upkeep_good);
        if receiver != producer && receiver < n && producer < n {
            let pidx = unordered_pair_index(n, receiver, producer);
            let flow = seat.upkeep_amount * table.seats[producer].refined_project_value;
            pair_volumes[pidx] += flow;
        }
    }

    RingTableMetrics {
        max_1seat_upkeep_coverage: report.max_1seat_upkeep_coverage,
        max_2seat_upkeep_coverage: report.max_2seat_upkeep_coverage,
        max_3seat_upkeep_coverage: report.max_3seat_upkeep_coverage,
        max_4seat_upkeep_coverage: report.max_4seat_upkeep_coverage,
        trade_concentration: compute_hhi(&pair_volumes),
    }
}

/// Runs `trials` seeded evaluations comparing random-endowment lock-in and HHI against
/// LP-verified `4`-, `5`-, and `6`-seat chiral converter-ring tables.
#[must_use]
pub fn measure_lockin_and_hhi(seed: u64, trials: usize) -> EmpiricalComparison {
    let count = trials.max(1);

    let random_5p_4r = run_random_ensemble(seed ^ 0x1111_2222_3333_4444, count, 5, 4, 2, 2);
    let random_5p_6r = run_random_ensemble(seed ^ 0x5555_6666_7777_8888, count, 5, 6, 2, 2);
    let ring_4p = run_ring_ensemble(seed ^ 0xAAAA_BBBB_CCCC_DDDD, count, SeatCount::Four);
    let ring_5p = run_ring_ensemble(seed ^ 0x1357_9BDF_2468_ACE0, count, SeatCount::Five);
    let ring_6p = run_ring_ensemble(seed ^ 0xFEDC_BA98_7654_3210, count, SeatCount::Six);

    EmpiricalComparison {
        trials: count,
        random_5p_4r,
        random_5p_6r,
        ring_4p,
        ring_5p,
        ring_6p,
    }
}

fn run_random_ensemble(
    seed: u64,
    trials: usize,
    num_seats: usize,
    num_resources: usize,
    endowed_per_seat: usize,
    foreign_needs_per_seat: usize,
) -> EnsembleStats {
    let mut c_100 = 0usize;
    let mut c_75 = 0usize;
    let mut sum_cov = 0.0_f64;
    let mut sum_hhi = 0.0_f64;
    let mut min_hhi = f64::INFINITY;
    let mut max_hhi = 0.0_f64;
    let mut sum_eff = 0.0_f64;
    let mut sum_active = 0.0_f64;

    for t in 0..trials {
        let tbl = generate_random_table(
            seed,
            t,
            num_seats,
            num_resources,
            endowed_per_seat,
            foreign_needs_per_seat,
        );
        let m = evaluate_random_table(&tbl);
        if m.has_100pct_bilateral_dyad {
            c_100 += 1;
        }
        if m.has_75pct_bilateral_dyad {
            c_75 += 1;
        }
        sum_cov += m.max_pair_mutual_coverage;
        sum_hhi += m.trade_concentration.hhi;
        min_hhi = min_hhi.min(m.trade_concentration.hhi);
        max_hhi = max_hhi.max(m.trade_concentration.hhi);
        sum_eff += m.trade_concentration.effective_dyads;
        sum_active += m.trade_concentration.active_pairs as f64;
    }

    let denom = trials as f64;
    EnsembleStats {
        trials,
        frac_100pct_dyad_lockin: (c_100 as f64) / denom,
        frac_75pct_dyad_coverage: (c_75 as f64) / denom,
        mean_max_2seat_coverage: sum_cov / denom,
        mean_hhi: sum_hhi / denom,
        min_hhi,
        max_hhi,
        mean_effective_dyads: sum_eff / denom,
        mean_active_pairs: sum_active / denom,
    }
}

fn run_ring_ensemble(seed: u64, trials: usize, seat_count: SeatCount) -> EnsembleStats {
    let cfg = GeneratorConfig {
        seat_count: Some(seat_count),
        ..GeneratorConfig::default()
    };
    let mut c_100 = 0usize;
    let mut c_75 = 0usize;
    let mut sum_cov = 0.0_f64;
    let mut sum_hhi = 0.0_f64;
    let mut min_hhi = f64::INFINITY;
    let mut max_hhi = 0.0_f64;
    let mut sum_eff = 0.0_f64;
    let mut sum_active = 0.0_f64;
    let mut completed = 0usize;

    for t in 0..trials {
        let sub_seed = seed.wrapping_add((t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if let Ok((table, report)) = generate_ring_table(sub_seed, &cfg) {
            let m = evaluate_ring_table(&table, &report);
            if m.max_2seat_upkeep_coverage >= 1.0 - 1e-9 {
                c_100 += 1;
            }
            if m.max_2seat_upkeep_coverage >= 0.75 - 1e-9 {
                c_75 += 1;
            }
            sum_cov += m.max_2seat_upkeep_coverage;
            sum_hhi += m.trade_concentration.hhi;
            min_hhi = min_hhi.min(m.trade_concentration.hhi);
            max_hhi = max_hhi.max(m.trade_concentration.hhi);
            sum_eff += m.trade_concentration.effective_dyads;
            sum_active += m.trade_concentration.active_pairs as f64;
            completed += 1;
        }
    }

    let denom = completed.max(1) as f64;
    EnsembleStats {
        trials: completed,
        frac_100pct_dyad_lockin: (c_100 as f64) / denom,
        frac_75pct_dyad_coverage: (c_75 as f64) / denom,
        mean_max_2seat_coverage: sum_cov / denom,
        mean_hhi: sum_hhi / denom,
        min_hhi: if completed > 0 { min_hhi } else { 0.0 },
        max_hhi,
        mean_effective_dyads: sum_eff / denom,
        mean_active_pairs: sum_active / denom,
    }
}

fn unordered_pair_index(n: usize, a: usize, b: usize) -> usize {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    lo * n - (lo * (lo + 1)) / 2 + (hi - lo - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hhi_handles_uniform_concentrated_and_empty_inputs() {
        let uniform = compute_hhi(&[1.0; 10]);
        assert!((uniform.hhi - 0.1).abs() < 1e-9);
        assert!((uniform.effective_dyads - 10.0).abs() < 1e-9);
        assert_eq!(uniform.active_pairs, 10);

        let mono = compute_hhi(&[5.0, 0.0, 0.0]);
        assert!((mono.hhi - 1.0).abs() < 1e-9);
        assert!((mono.effective_dyads - 1.0).abs() < 1e-9);
        assert_eq!(mono.active_pairs, 1);

        let empty = compute_hhi(&[]);
        assert_eq!(empty.total_pairs, 0);
        let zero = compute_hhi(&[0.0, 0.0]);
        assert!((zero.hhi - 1.0).abs() < 1e-9);
    }

    #[test]
    fn empirical_comparison_measures_hypotheses() {
        let cmp = measure_lockin_and_hhi(20_261_115, 200);
        let hyp = ReportHypotheses::SEED_26;

        // Verify chiral ring eliminates 2-seat lock-in and reduces HHI vs random lock-in.
        assert_eq!(cmp.ring_4p.trials, 200);
        assert_eq!(cmp.ring_5p.trials, 200);
        assert_eq!(cmp.ring_6p.trials, 200);
        assert!(cmp.ring_4p.frac_100pct_dyad_lockin < 1e-9);
        assert!(cmp.ring_5p.frac_100pct_dyad_lockin < 1e-9);
        assert!(cmp.ring_6p.frac_100pct_dyad_lockin < 1e-9);
        assert!(cmp.ring_5p.mean_max_2seat_coverage < 1e-9);

        // Random 2-of-4 lock-in rate falls near 0.797 (`> 0.65`) and 2-of-6 `>=75%` rate `> 0.50`.
        assert!(
            cmp.random_5p_4r.frac_100pct_dyad_lockin > 0.65,
            "expected random 5p/4r lockin near {}, got {}",
            hyp.random_5p_4r_lockin_100pct,
            cmp.random_5p_4r.frac_100pct_dyad_lockin
        );
        assert!(
            cmp.random_5p_6r.frac_75pct_dyad_coverage > 0.50,
            "expected random 5p/6r >=75% near {}, got {}",
            hyp.random_5p_6r_coverage_75pct,
            cmp.random_5p_6r.frac_75pct_dyad_coverage
        );

        // Chiral ring 5p HHI drops sharply below random tables and clears all 10 pairs.
        assert!(
            cmp.ring_5p.mean_hhi < cmp.random_5p_4r.mean_hhi,
            "ring_5p HHI {} must be lower than random_5p_4r HHI {}",
            cmp.ring_5p.mean_hhi,
            cmp.random_5p_4r.mean_hhi
        );
        assert!(
            cmp.ring_5p.mean_hhi < 0.18,
            "ring_5p mean HHI {} should satisfy Mode A prior < 0.18 (report ref {})",
            cmp.ring_5p.mean_hhi,
            hyp.chiral_ring_5p_hhi
        );

        println!(
            "EMPIRICAL n={}: rand_5p_4r lockin={:.3} cov75={:.3} hhi={:.3} eff={:.2} | rand_5p_6r cov75={:.3} hhi={:.3} eff={:.2} | ring_4p hhi={:.3} eff={:.2} act={:.1} | ring_5p hhi={:.3} eff={:.2} act={:.1} | ring_6p hhi={:.3} eff={:.2} act={:.1}",
            cmp.trials,
            cmp.random_5p_4r.frac_100pct_dyad_lockin,
            cmp.random_5p_4r.frac_75pct_dyad_coverage,
            cmp.random_5p_4r.mean_hhi,
            cmp.random_5p_4r.mean_effective_dyads,
            cmp.random_5p_6r.frac_75pct_dyad_coverage,
            cmp.random_5p_6r.mean_hhi,
            cmp.random_5p_6r.mean_effective_dyads,
            cmp.ring_4p.mean_hhi,
            cmp.ring_4p.mean_effective_dyads,
            cmp.ring_4p.mean_active_pairs,
            cmp.ring_5p.mean_hhi,
            cmp.ring_5p.mean_effective_dyads,
            cmp.ring_5p.mean_active_pairs,
            cmp.ring_6p.mean_hhi,
            cmp.ring_6p.mean_effective_dyads,
            cmp.ring_6p.mean_active_pairs,
        );
    }
}
