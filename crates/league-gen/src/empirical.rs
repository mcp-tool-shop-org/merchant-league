//! Static model comparison of random bilateral lock-in probabilities and dyadic trade HHI
//! alongside LP-verified chiral converter-ring tables.
//!
//! Important scoping note (`docs/design-v2.md` §1.3 & Hard Invariant 3):
//! - Both the random-endowment clearing heuristic and the ring 1-turn LP optimal flow
//!   computed here are **static model outputs**, not empirical played-game telemetry; actual
//!   played-game dyadic HHI belongs to Part 3 / Mode A engine runs.
//! - Zero 2-seat lock-in (`0.000`) on accepted ring tables is **guaranteed by construction**
//!   after Invariant 1 of [`crate::check_feasibility`]; the meaningful empirical measurement
//!   reported by [`measure_lockin_and_hhi`] on the ring side is `filter_rejection_rate`
//!   (`candidates_rejected / candidates_sampled`).

use serde::{Deserialize, Serialize};

use crate::feasibility::FeasibilityReport;
use crate::generator::generate_ring_table;
use crate::rng::SeedStream;
use crate::schema::{
    CreditRatingTier, FeasibilityError, GeneratorConfig, HorizonShockKind, MandateObjective,
    PROJECT_CONCORDAT_ID, RingTable, SeatCount,
};

/// Analytical priors reported in the seed-26 design blueprint.
///
/// Treated strictly as hypotheses to compare against Mode A played-game telemetry and static models.
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

/// Dyadic trade concentration metrics across all unordered seat pairs (`B2.3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HhiMetrics {
    /// Total number of unordered seat pairs `N * (N - 1) / 2`.
    pub total_pairs: usize,
    /// Count of pairs with strictly positive modeled trade volume.
    pub active_pairs: usize,
    /// Sum of squared dyadic trade-volume shares `sum s_{ij}^2` in `[1 / total_pairs, 1.0]`.
    pub hhi: f64,
    /// Inverse concentration `1.0 / hhi` (`0.0` if zero pairs).
    pub effective_dyads: f64,
    /// Largest single unordered pair's share of total bilateral volume `max_ij s_{ij}` (`B2.3`
    /// unconditional telemetry; also enforced against C6 table caps via `check_feasibility`).
    pub max_pair_share: f64,
}

/// Computes exact [`HhiMetrics`] from a slice of nonnegative pair trade volumes.
#[must_use]
pub fn compute_hhi(pair_volumes: &[f64]) -> HhiMetrics {
    let total_pairs = pair_volumes.len();
    let total_vol: f64 = pair_volumes.iter().copied().filter(|&v| v > 0.0).sum();
    if total_pairs == 0 || total_vol <= 0.0 || !total_vol.is_finite() {
        return HhiMetrics {
            total_pairs,
            active_pairs: 0,
            hhi: if total_pairs > 0 { 1.0 } else { 0.0 },
            effective_dyads: if total_pairs > 0 { 1.0 } else { 0.0 },
            max_pair_share: if total_pairs > 0 { 1.0 } else { 0.0 },
        };
    }

    let active_cutoff = 1e-12 * total_vol;
    let mut active_pairs = 0usize;
    let mut hhi = 0.0_f64;
    let mut max_share = 0.0_f64;
    for &vol in pair_volumes {
        if vol > active_cutoff {
            active_pairs += 1;
            let share = vol / total_vol;
            if share > max_share {
                max_share = share;
            }
            hhi += share * share;
        }
    }
    let effective_dyads = if hhi > 0.0 { 1.0 / hhi } else { 0.0 };
    HhiMetrics {
        total_pairs,
        active_pairs,
        hhi,
        effective_dyads,
        max_pair_share: max_share,
    }
}

/// Scripted proxy rollout grading summary across deterministic 26-turn episodes for a candidate
/// [`RingTable`] (`B2.2`). Executed inside `league-gen` using heuristic cadence/barter rules; not
/// `league-core`'s `TurnEngine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutGradingReport {
    /// Number of seeded deterministic 26-turn multi-agent rollout episodes executed.
    pub episodes: usize,
    /// Observed completion frequency in basis points (`0..=10_000`) per card in `mandate_deck`.
    pub per_card_completion_bps: Vec<u16>,
    /// Arithmetic mean completion rate (`0..=10_000 bps`) across all cards in `mandate_deck`.
    pub mean_mandate_completion_bps: u16,
    /// Total Tier-3 Syndicate Project batches completed across all rollout episodes.
    pub total_project_batches: u32,
    /// Total full-ring Concordat (`id = 100`) batches completed across all rollout episodes.
    pub concordat_batches: u32,
    /// Count of distinct refined goods (`0..=N`) with positive rollout production.
    pub distinct_refined_goods_produced: usize,
    /// Aggregated bilateral trade concentration observed across all rollout episodes.
    pub rollout_trade_concentration: HhiMetrics,
}

/// Baseline random resource-endowment table for measuring combinatorial bilateral lock-in probabilities.
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

/// Coverage and static-model trade concentration metrics for one [`RandomEndowmentTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RandomTableMetrics {
    /// Maximum mutual coverage fraction across all `N*(N-1)/2` unordered pairs.
    pub max_pair_mutual_coverage: f64,
    /// Whether at least one pair satisfies 100% of each other's foreign needs.
    pub has_100pct_bilateral_dyad: bool,
    /// Whether at least one pair satisfies >=75% of combined mutual foreign needs.
    pub has_75pct_bilateral_dyad: bool,
    /// Modeled dyadic concentration under a reciprocal-first clearing heuristic (`12x / 3x / 0.25x`).
    pub trade_concentration: HhiMetrics,
}

/// Coalition coverage bounds, 1-turn LP-flow concentration, and rollout grading for one [`RingTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingTableMetrics {
    /// Maximum 1-seat internal upkeep coverage (`0.0` by Invariant 1 construction).
    pub max_1seat_upkeep_coverage: f64,
    /// Maximum 2-seat internal upkeep coverage (`0.0` by Invariant 1 construction).
    pub max_2seat_upkeep_coverage: f64,
    /// Maximum 3-seat internal upkeep coverage (`<= 1/3` for `N >= 4`).
    pub max_3seat_upkeep_coverage: f64,
    /// Maximum 4-seat internal upkeep coverage (`<= 1/2` for `N = 5`).
    pub max_4seat_upkeep_coverage: Option<f64>,
    /// Static 1-turn LP grand-coalition optimal-flow dyadic concentration model output.
    pub trade_concentration: HhiMetrics,
    /// Scripted proxy rollout project and mandate grading telemetry inside `league-gen` (`B2.2`,
    /// not `league-core`'s `TurnEngine`).
    pub rollout_grading: RolloutGradingReport,
}

/// Aggregated statistics across an ensemble of seeded tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnsembleStats {
    /// Number of accepted tables evaluated.
    pub trials: usize,
    /// Total candidate tables sampled (equals `trials + candidates_rejected`).
    pub candidates_sampled: usize,
    /// Total candidate tables rejected by [`crate::check_feasibility`] prior to acceptance (`0` for random tables).
    pub candidates_rejected: usize,
    /// Empirical filter rejection rate `candidates_rejected / candidates_sampled`.
    pub filter_rejection_rate: f64,
    /// Fraction of tables containing >=1 100%-self-sufficient 2-seat dyad (`0.0` guaranteed by construction after Invariant 1 on accepted ring tables).
    pub frac_100pct_dyad_lockin: f64,
    /// Fraction of tables containing >=1 >=75%-covered 2-seat dyad (`0.0` guaranteed by construction after Invariant 1 on accepted ring tables).
    pub frac_75pct_dyad_coverage: f64,
    /// Mean maximum 2-seat coverage ratio across trials (`0.0` by construction on accepted ring tables).
    pub mean_max_2seat_coverage: f64,
    /// Mean modeled dyadic trade HHI across trials (static model output, not played-game HHI).
    pub mean_hhi: f64,
    /// Minimum modeled dyadic trade HHI across trials.
    pub min_hhi: f64,
    /// Maximum modeled dyadic trade HHI across trials.
    pub max_hhi: f64,
    /// Mean of per-table effective dyads (`mean(1.0 / HHI_t)`, distinct from `1.0 / mean_hhi`).
    pub mean_effective_dyads: f64,
    /// Mean number of active modeled trading pairs across trials.
    pub mean_active_pairs: f64,
    /// Mean top-pair share `max_ij s_{ij}` across trials (`B2.3` telemetry).
    pub mean_max_pair_share: f64,
    /// Peak top-pair share observed across any trial in the ensemble (`B2.3` telemetry).
    pub max_pair_share_observed: f64,
}

/// Side-by-side summary of random-endowment Monte Carlo rates vs. LP-certified ring rejection rates and static-model HHIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmpiricalComparison {
    /// Number of accepted tables per ensemble.
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

/// Measures combinatorial bilateral lock-in coverage and static clearing-model concentration on `table`.
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

    // Heuristic clearing model: locked bilateral dyads (`cov >= 0.75`) repeat swaps across turns.
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

/// Computes 1-turn optimal LP-flow dyadic trade volumes across all `N*(N-1)/2` unordered pairs.
#[must_use]
pub(crate) fn compute_ring_lp_pair_volumes(
    table: &RingTable,
    optimal_converter_runs: &[f64],
) -> Vec<f64> {
    let n = table.seat_count.as_usize();
    let total_pairs = table.seat_count.pair_count();
    let mut pair_volumes = vec![0.0_f64; total_pairs];

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
            let out_proj = table
                .seats
                .get(usize::from(conv.output_good))
                .map_or(mean_proj, |s| s.refined_project_value);
            if total_in > 0.0 {
                let unit_val = (conv.output_amount * out_proj) / total_in;
                for &(raw_g, amt) in &conv.inputs {
                    let g = usize::from(raw_g);
                    if g < n && amt > 0.0 {
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

    // Flatten converter specs in exact order matching `optimal_converter_runs`.
    let mean_upkeep: f64 = table
        .seats
        .iter()
        .take(n)
        .map(|s| s.upkeep_amount)
        .sum::<f64>()
        / (n.max(1) as f64);
    let min_output_tol = 1e-12 * mean_upkeep.max(f64::MIN_POSITIVE);
    let mut conv_idx = 0usize;
    for seat in &table.seats {
        for conv in &seat.converters {
            let runs = optimal_converter_runs.get(conv_idx).copied().unwrap_or(0.0);
            conv_idx += 1;
            if runs <= 0.0 || runs * conv.output_amount <= min_output_tol {
                continue;
            }
            let owner = usize::from(conv.owner);
            for &(raw_g, amt) in &conv.inputs {
                let supplier = usize::from(raw_g);
                if supplier != owner && supplier < n && owner < n {
                    let pidx = unordered_pair_index(n, owner, supplier);
                    let flow = runs * amt * raw_parity[supplier];
                    if pidx < pair_volumes.len() {
                        pair_volumes[pidx] += flow;
                    }
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
            let producer_proj = table
                .seats
                .get(producer)
                .map_or(mean_proj, |s| s.refined_project_value);
            let flow = seat.upkeep_amount * producer_proj;
            if pidx < pair_volumes.len() {
                pair_volumes[pidx] += flow;
            }
        }
    }

    pair_volumes
}

/// Returns the `(pair, share)` with the largest volume fraction across unordered pairs.
#[must_use]
pub(crate) fn top_pair_share(n: usize, pair_volumes: &[f64]) -> ((u8, u8), f64) {
    let total: f64 = pair_volumes.iter().copied().filter(|&v| v > 0.0).sum();
    if n < 2 || pair_volumes.is_empty() || total <= 0.0 || !total.is_finite() {
        return ((0, 1), 0.0);
    }
    let mut best_pair = (0u8, 1u8);
    let mut best_vol = -1.0_f64;
    let mut idx = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            if idx < pair_volumes.len() && pair_volumes[idx] > best_vol {
                best_vol = pair_volumes[idx];
                best_pair = (i as u8, j as u8);
            }
            idx += 1;
        }
    }
    (best_pair, best_vol.max(0.0) / total)
}

const ROLLOUT_EPISODES: usize = 8;
const ROLLOUT_TURNS: u8 = 26;
const MAX_SEATS: usize = 6;

/// Runs a dimensionless scripted proxy rollout across deterministic 26-turn episodes inside
/// `league-gen` (`B2.2`, `B-1`) to grade Syndicate Project completion, full-ring Concordat
/// activity, per-card mandate difficulty, and rollout pair shares.
///
/// Normalizes raw, refined, project-value, project-equity, and per-converter batch-run quantities
/// unconditionally by table means/stoichiometries so restating a [`RingTable`] in any positive unit
/// system (either scaling `max_runs_per_turn` or scaling per-run `inputs`/`output_amount`) yields
/// an identical rollout grading report and verdict. This is a self-contained proxy heuristic, not
/// `league-core`'s `TurnEngine`.
#[must_use]
pub fn compute_rollout_grading(
    table: &RingTable,
    report: &FeasibilityReport,
) -> RolloutGradingReport {
    let n = table.seat_count.as_usize().min(MAX_SEATS);
    let denom = n.max(1) as f64;
    let mean_yield = table
        .seats
        .iter()
        .take(n)
        .map(|s| s.raw_yield_per_turn)
        .sum::<f64>()
        / denom;
    let mean_upkeep = table
        .seats
        .iter()
        .take(n)
        .map(|s| s.upkeep_amount)
        .sum::<f64>()
        / denom;
    let mean_proj_val = table
        .seats
        .iter()
        .take(n)
        .map(|s| s.refined_project_value)
        .sum::<f64>()
        / denom;
    let mean_proj_eq = if table.projects.is_empty() {
        0.0
    } else {
        table
            .projects
            .iter()
            .map(|p| p.output_project_equity)
            .sum::<f64>()
            / (table.projects.len() as f64)
    };

    // Unconditional characteristic unit scales (`~1.0` on nominal generated tables).
    let r_unit = if mean_yield > 0.0 && mean_yield.is_finite() {
        mean_yield / 10.5
    } else {
        1.0
    };
    let m_unit = if mean_upkeep > 0.0 && mean_upkeep.is_finite() {
        mean_upkeep / 2.0
    } else {
        1.0
    };
    let v_unit = if mean_proj_val > 0.0 && mean_proj_val.is_finite() {
        mean_proj_val / 2.2
    } else {
        1.0
    };
    let eq_unit = if mean_proj_eq > 0.0 && mean_proj_eq.is_finite() {
        mean_proj_eq
    } else {
        1.0
    };

    let deck_len = table.mandate_deck.len();
    let total_pairs = table.seat_count.pair_count();
    let mut pair_volumes = vec![0.0_f64; total_pairs];
    let mut card_completions = vec![0usize; deck_len];
    let mut total_project_batches = 0u32;
    let mut concordat_batches = 0u32;
    let mut goods_produced_mask = 0u8;

    let concordat_opt = table.projects.iter().find(|p| p.id == PROJECT_CONCORDAT_ID);
    let conv_base_runs = &report.optimal_converter_runs;

    let mut stream = SeedStream::new(table.seed, b"MLV2|ROLLOUT_GRADING");

    for ep in 0..ROLLOUT_EPISODES {
        // Dimensionless inventories: `raw_inv` in `r_unit`, `ref_owned` in `m_unit`, `project_equity` in `eq_unit`.
        let mut raw_inv = [0.0_f64; MAX_SEATS];
        let mut ref_owned = [[0.0_f64; MAX_SEATS]; MAX_SEATS];
        let mut project_equity = [0.0_f64; MAX_SEATS];
        let mut credit_defaults = [0u32; MAX_SEATS];
        let mut partners_mask = [0u8; MAX_SEATS];

        // Episode policy spread exercises tight/relaxed conversion cadence, partner focus, and hoarding.
        let mut seat_cadence = [1.0_f64; MAX_SEATS];
        let mut seat_project_propensity = [1.0_f64; MAX_SEATS];
        for s in 0..n {
            seat_cadence[s] = stream.range_f64(0.68, 1.04);
            seat_project_propensity[s] = stream.range_f64(0.35, 1.00);
        }
        let diversify_refined_barter = (ep % 2) == 0;
        let prefer_concordat = (ep % 3) != 2;

        for turn in 1..=ROLLOUT_TURNS {
            let mut yield_mult = [1.0_f64; MAX_SEATS];
            let mut tariff_mult = [1.0_f64; MAX_SEATS];
            let mut proj_mult = [1.0_f64; MAX_SEATS];
            let mut extra_decay = [0.0_f64; MAX_SEATS];

            for shock in &table.shocks {
                let target = usize::from(shock.target_seat);
                if target >= n {
                    continue;
                }
                match shock.kind {
                    HorizonShockKind::SupplyFreeze if turn == shock.realize_turn => {
                        yield_mult[target] *= (1.0 - shock.magnitude).max(0.20);
                    }
                    HorizonShockKind::TariffHoliday
                        if turn >= shock.realize_turn && turn <= shock.realize_turn + 1 =>
                    {
                        tariff_mult[target] *= (1.0 - shock.magnitude).max(0.0);
                    }
                    HorizonShockKind::DemandSurge
                        if turn >= shock.realize_turn && turn <= shock.realize_turn + 1 =>
                    {
                        proj_mult[target] *= 1.0 + shock.magnitude;
                    }
                    HorizonShockKind::SpoilageSpike if turn == shock.realize_turn => {
                        extra_decay[target] += shock.magnitude * 0.5;
                    }
                    _ => {}
                }
            }

            // 1. Raw extraction (normalized by `r_unit`).
            for s in 0..n {
                let pulse = if ((ep + usize::from(turn) + s) % 9) == 0 {
                    0.78
                } else {
                    1.0
                };
                raw_inv[s] += (table.seats[s].raw_yield_per_turn / r_unit)
                    * yield_mult[s]
                    * seat_cadence[s]
                    * pulse;
            }

            // 2. Chiral raw imports & converter batch execution (normalized by per-converter `run_unit`).
            let mut c_cursor = 0usize;
            for owner in 0..n {
                for conv in &table.seats[owner].converters {
                    let base_run = conv_base_runs.get(c_cursor).copied().unwrap_or(0.0);
                    c_cursor += 1;
                    let in_sum: f64 = conv
                        .inputs
                        .iter()
                        .filter(|&&(_, amt)| amt > 0.0 && amt.is_finite())
                        .map(|&(_, amt)| amt)
                        .sum();
                    if in_sum <= 0.0 || conv.output_amount <= 0.0 {
                        continue;
                    }
                    let run_unit = (4.0 * r_unit) / in_sum;
                    let base_run_norm = base_run / run_unit;
                    let max_runs_norm = conv.max_runs_per_turn / run_unit;
                    let target_run_norm = if conv.is_secondary {
                        if base_run_norm > 1e-6 || yield_mult.iter().any(|&y| y < 0.99) {
                            base_run_norm.max(0.35) * seat_cadence[owner]
                        } else {
                            0.0
                        }
                    } else {
                        base_run_norm * seat_cadence[owner]
                    };
                    let mut affordable_norm = target_run_norm.min(max_runs_norm);
                    for &(raw_g, amt) in &conv.inputs {
                        let k = usize::from(raw_g);
                        if k >= n || amt <= 0.0 {
                            affordable_norm = 0.0;
                            continue;
                        }
                        let mult = if k == owner {
                            1.0
                        } else {
                            1.0 + table.seats[k].raw_export_tariff * tariff_mult[k]
                        };
                        let in_norm = (4.0 * amt) / in_sum;
                        let cap_k_norm = raw_inv[k] / (in_norm * mult);
                        if cap_k_norm < affordable_norm {
                            affordable_norm = cap_k_norm;
                        }
                    }
                    if affordable_norm > 1e-6 {
                        for &(raw_g, amt) in &conv.inputs {
                            let k = usize::from(raw_g);
                            if k >= n || amt <= 0.0 {
                                continue;
                            }
                            let mult = if k == owner {
                                1.0
                            } else {
                                1.0 + table.seats[k].raw_export_tariff * tariff_mult[k]
                            };
                            let in_norm = (4.0 * amt) / in_sum;
                            let shipped_norm = affordable_norm * in_norm * mult;
                            raw_inv[k] = (raw_inv[k] - shipped_norm).max(0.0);
                            if k != owner {
                                partners_mask[owner] |= 1u8 << k;
                                partners_mask[k] |= 1u8 << owner;
                                let pidx = unordered_pair_index(n, owner, k);
                                if pidx < pair_volumes.len() {
                                    pair_volumes[pidx] += affordable_norm
                                        * in_norm
                                        * (table.seats[k].refined_project_value / v_unit);
                                }
                            }
                        }
                        let out_norm = (conv.output_amount * run_unit) / m_unit;
                        ref_owned[owner][owner] += affordable_norm * out_norm;
                        goods_produced_mask |= 1u8 << owner;
                    }
                }
            }

            // 3. Mandatory neighbor upkeep settlement (`M_{(s+n-1)%n}` -> `s`, normalized by `m_unit`).
            for s in 0..n {
                let prod = usize::from(table.seats[s].upkeep_good);
                if prod >= n {
                    continue;
                }
                let mult = if prod == s {
                    1.0
                } else {
                    1.0 + table.seats[prod].refined_export_tariff * tariff_mult[prod]
                };
                let upkeep_norm = table.seats[s].upkeep_amount / m_unit;
                let needed_gross_norm = upkeep_norm * mult;
                if ref_owned[prod][prod] + 1e-9 >= needed_gross_norm {
                    ref_owned[prod][prod] = (ref_owned[prod][prod] - needed_gross_norm).max(0.0);
                    if prod != s {
                        partners_mask[s] |= 1u8 << prod;
                        partners_mask[prod] |= 1u8 << s;
                        let pidx = unordered_pair_index(n, s, prod);
                        if pidx < pair_volumes.len() {
                            pair_volumes[pidx] +=
                                upkeep_norm * (table.seats[prod].refined_project_value / v_unit);
                        }
                    }
                } else {
                    credit_defaults[s] += 1;
                }
            }

            // 4. Multilateral refined swaps for variety mandates & Tier-3 projects (in `m_unit`).
            if diversify_refined_barter && (turn % 2) == 0 {
                let hop = 1 + ((ep + usize::from(turn)) % (n.saturating_sub(1).max(1)));
                for s in 0..n {
                    let partner = (s + hop) % n;
                    if partner == s {
                        continue;
                    }
                    let swap_qty_norm =
                        (ref_owned[s][s].min(ref_owned[partner][partner]) * 0.20).min(0.85);
                    if swap_qty_norm > 0.05 {
                        ref_owned[s][s] -= swap_qty_norm;
                        ref_owned[partner][partner] -= swap_qty_norm;
                        ref_owned[s][partner] += swap_qty_norm;
                        ref_owned[partner][s] += swap_qty_norm;
                        partners_mask[s] |= 1u8 << partner;
                        partners_mask[partner] |= 1u8 << s;
                        let pidx = unordered_pair_index(n, s, partner);
                        if pidx < pair_volumes.len() {
                            pair_volumes[pidx] += swap_qty_norm
                                * ((table.seats[s].refined_project_value
                                    + table.seats[partner].refined_project_value)
                                    / v_unit);
                        }
                    }
                }
            }

            // 5. Periodic Tier-3 Syndicate Project completion (Concordat 100 & neighbor pairs 1..=N).
            if let Some(concordat) = concordat_opt
                && prefer_concordat
                && (turn % 3) == 0
                && concordat.required_inputs.iter().all(|&(g, _, qty)| {
                    usize::from(g) < n && ref_owned[usize::from(g)][usize::from(g)] >= qty / m_unit
                })
            {
                let mut boost_sum = 0.0_f64;
                for &(g, _, qty) in &concordat.required_inputs {
                    let gi = usize::from(g);
                    ref_owned[gi][gi] -= qty / m_unit;
                    boost_sum += proj_mult[gi];
                }
                let mean_boost = boost_sum / (n as f64);
                let per_seat_eq_norm =
                    ((concordat.output_project_equity / eq_unit) * mean_boost) / (n as f64);
                for s in 0..n {
                    project_equity[s] += per_seat_eq_norm * seat_project_propensity[s];
                }
                concordat_batches += 1;
                total_project_batches += 1;
            }

            if (turn % 2) == 1 {
                for proj in &table.projects {
                    if proj.id == PROJECT_CONCORDAT_ID {
                        continue;
                    }
                    let can_run = proj.required_inputs.iter().all(|&(g, _, qty)| {
                        let gi = usize::from(g);
                        gi < n
                            && ref_owned[gi][gi] >= qty / m_unit
                            && seat_project_propensity[gi] > 0.40
                    });
                    if can_run {
                        let k_parts = proj.required_inputs.len() as f64;
                        let mut avg_boost = 0.0_f64;
                        for &(g, _, qty) in &proj.required_inputs {
                            let gi = usize::from(g);
                            ref_owned[gi][gi] -= qty / m_unit;
                            avg_boost += proj_mult[gi];
                        }
                        let share_eq_norm = ((proj.output_project_equity / eq_unit)
                            * (avg_boost / k_parts))
                            / k_parts;
                        for &(g, _, _) in &proj.required_inputs {
                            let gi = usize::from(g);
                            project_equity[gi] +=
                                share_eq_norm * (0.65 + 0.35 * seat_project_propensity[gi]);
                        }
                        total_project_batches += 1;
                    }
                }
            }

            // 6. Perishable end-of-turn Clawback spoilage on remaining raw stock.
            for s in 0..n {
                let decay = (table.seats[s].perishable_decay_rate + extra_decay[s]).min(0.99);
                raw_inv[s] *= 1.0 - decay;
            }
        }

        // Terminal Turn-26 Mandate grading across all N seats (normalized by `eq_unit` and `m_unit`).
        for s in 0..n {
            let tier = match credit_defaults[s] {
                0 => CreditRatingTier::Aaa,
                1 => CreditRatingTier::Bbb,
                2 => CreditRatingTier::B,
                _ => CreditRatingTier::D,
            };
            let distinct_partners = partners_mask[s].count_ones() as u8;
            for (c_idx, card) in table.mandate_deck.iter().enumerate() {
                let met = match card.kind {
                    MandateObjective::MinProjectEquity { target } => {
                        project_equity[s] >= target / eq_unit
                    }
                    MandateObjective::MinRefinedVariety {
                        distinct_goods,
                        min_each,
                    } => {
                        let min_each_norm = min_each / m_unit;
                        let count = (0..n).filter(|&g| ref_owned[s][g] >= min_each_norm).count();
                        count >= usize::from(distinct_goods)
                    }
                    MandateObjective::MaintainCleanCredit { min_tier } => tier <= min_tier,
                    MandateObjective::MinActivePartners { min_partners } => {
                        distinct_partners >= min_partners
                    }
                };
                if met {
                    card_completions[c_idx] += 1;
                }
            }
        }
    }

    let total_seat_eps = (ROLLOUT_EPISODES * n).max(1);
    let per_card_completion_bps: Vec<u16> = card_completions
        .into_iter()
        .map(|wins| ((wins * 10_000) / total_seat_eps).min(10_000) as u16)
        .collect();
    let mean_mandate_completion_bps = if per_card_completion_bps.is_empty() {
        0
    } else {
        let sum: u32 = per_card_completion_bps.iter().map(|&b| u32::from(b)).sum();
        (sum / (per_card_completion_bps.len() as u32)) as u16
    };

    RolloutGradingReport {
        episodes: ROLLOUT_EPISODES,
        per_card_completion_bps,
        mean_mandate_completion_bps,
        total_project_batches,
        concordat_batches,
        distinct_refined_goods_produced: goods_produced_mask.count_ones() as usize,
        rollout_trade_concentration: compute_hhi(&pair_volumes),
    }
}

/// Validates B2.2 scripted proxy rollout grading gates before seed acceptance (uses
/// `league-gen`'s proxy rollout model, not `league-core`'s `TurnEngine`).
pub fn verify_rollout_grading(
    table: &RingTable,
    grading: &RolloutGradingReport,
    max_pair_share_cap: f64,
) -> Result<(), FeasibilityError> {
    let n = table.seat_count.as_usize();
    if grading.distinct_refined_goods_produced < n {
        return Err(FeasibilityError::RolloutGateFailed(format!(
            "only {} of {n} distinct refined goods produced in rollouts",
            grading.distinct_refined_goods_produced
        )));
    }
    if grading.total_project_batches == 0 || grading.concordat_batches == 0 {
        return Err(FeasibilityError::RolloutGateFailed(format!(
            "projects stagnated in rollouts (total={}, concordat={})",
            grading.total_project_batches, grading.concordat_batches
        )));
    }
    if grading.mean_mandate_completion_bps == 0 || grading.mean_mandate_completion_bps >= 10_000 {
        return Err(FeasibilityError::RolloutGateFailed(format!(
            "degenerate mandate rollout completion bps: {}",
            grading.mean_mandate_completion_bps
        )));
    }
    if grading.rollout_trade_concentration.max_pair_share > max_pair_share_cap + 1e-9 {
        return Err(FeasibilityError::RolloutGateFailed(format!(
            "rollout max_pair_share {:.4} exceeds cap {:.4}",
            grading.rollout_trade_concentration.max_pair_share, max_pair_share_cap
        )));
    }
    Ok(())
}

/// Evaluates coalition coverage, 1-turn LP flow HHI, and scripted proxy rollout grading (`B2.2`).
#[must_use]
pub fn evaluate_ring_table(table: &RingTable, report: &FeasibilityReport) -> RingTableMetrics {
    let pair_volumes = compute_ring_lp_pair_volumes(table, &report.optimal_converter_runs);
    let rollout_grading = compute_rollout_grading(table, report);

    RingTableMetrics {
        max_1seat_upkeep_coverage: report.max_1seat_upkeep_coverage,
        max_2seat_upkeep_coverage: report.max_2seat_upkeep_coverage,
        max_3seat_upkeep_coverage: report.max_3seat_upkeep_coverage,
        max_4seat_upkeep_coverage: report.max_4seat_upkeep_coverage,
        trade_concentration: compute_hhi(&pair_volumes),
        rollout_grading,
    }
}

/// Runs `trials` seeded evaluations measuring random-endowment lock-in rates alongside
/// ring-generator feasibility rejection rates and 1-turn static-model HHIs.
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
    let mut sum_max_pair = 0.0_f64;
    let mut max_pair_obs = 0.0_f64;

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
        sum_max_pair += m.trade_concentration.max_pair_share;
        max_pair_obs = max_pair_obs.max(m.trade_concentration.max_pair_share);
    }

    let denom = trials as f64;
    EnsembleStats {
        trials,
        candidates_sampled: trials,
        candidates_rejected: 0,
        filter_rejection_rate: 0.0,
        frac_100pct_dyad_lockin: (c_100 as f64) / denom,
        frac_75pct_dyad_coverage: (c_75 as f64) / denom,
        mean_max_2seat_coverage: sum_cov / denom,
        mean_hhi: sum_hhi / denom,
        min_hhi,
        max_hhi,
        mean_effective_dyads: sum_eff / denom,
        mean_active_pairs: sum_active / denom,
        mean_max_pair_share: sum_max_pair / denom,
        max_pair_share_observed: max_pair_obs,
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
    let mut sum_max_pair = 0.0_f64;
    let mut max_pair_obs = 0.0_f64;
    let mut completed = 0usize;
    let mut total_rejected = 0usize;
    let mut total_sampled = 0usize;

    // Documented seed-pool exception (`docs/design-v2.md` §7.4): `measure_lockin_and_hhi` XORs its
    // base seed with three fixed per-seat-count 64-bit masks and steps `sub_seed` by `t` times the
    // 64-bit golden-ratio increment before calling unchecked `generate_ring_table`, and the date
    // seed `20_261_115` (`generator.rs`, `mandates.rs`, `empirical.rs`) falls in `unassigned`.
    // All derived ensemble seeds were verified out-of-repo to be disjoint from the sealed set.
    for t in 0..trials {
        let sub_seed = seed.wrapping_add((t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if let Ok((table, report)) = generate_ring_table(sub_seed, &cfg) {
            let rejected = table.attempt as usize;
            total_rejected += rejected;
            total_sampled += rejected + 1;

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
            sum_max_pair += m.trade_concentration.max_pair_share;
            max_pair_obs = max_pair_obs.max(m.trade_concentration.max_pair_share);
            completed += 1;
        }
    }

    let denom = completed.max(1) as f64;
    let sampled_denom = total_sampled.max(1) as f64;
    EnsembleStats {
        trials: completed,
        candidates_sampled: total_sampled,
        candidates_rejected: total_rejected,
        filter_rejection_rate: (total_rejected as f64) / sampled_denom,
        frac_100pct_dyad_lockin: (c_100 as f64) / denom,
        frac_75pct_dyad_coverage: (c_75 as f64) / denom,
        mean_max_2seat_coverage: sum_cov / denom,
        mean_hhi: sum_hhi / denom,
        min_hhi: if completed > 0 { min_hhi } else { 0.0 },
        max_hhi,
        mean_effective_dyads: sum_eff / denom,
        mean_active_pairs: sum_active / denom,
        mean_max_pair_share: sum_max_pair / denom,
        max_pair_share_observed: max_pair_obs,
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
    fn evaluate_ring_table_guards_out_of_bounds_indices_without_panic() {
        let cfg = GeneratorConfig {
            seat_count: Some(SeatCount::Four),
            ..GeneratorConfig::default()
        };
        let Ok((mut table, report)) = generate_ring_table(11, &cfg) else {
            panic!("seed 11 must generate cleanly");
        };
        table.seats[0].converters[0].output_good = 99;
        table.seats[0].converters[0].inputs.push((99, 1.0));
        table.seats[1].upkeep_good = 99;
        let m = evaluate_ring_table(&table, &report);
        assert!(m.trade_concentration.hhi.is_finite());
    }

    #[test]
    fn empirical_comparison_measures_lockin_rejection_rates_and_static_model_hhi() {
        let cmp = measure_lockin_and_hhi(20_261_115, 200);
        let hyp = ReportHypotheses::SEED_26;

        // Accepted ring tables have `0.0` 2-seat coverage by construction after Invariant 1
        // (regression check on accepted tables; filter cost is reported via `filter_rejection_rate`).
        assert_eq!(cmp.ring_4p.trials, 200);
        assert_eq!(cmp.ring_5p.trials, 200);
        assert_eq!(cmp.ring_6p.trials, 200);
        assert_eq!(
            cmp.ring_4p.candidates_sampled,
            cmp.ring_4p.trials + cmp.ring_4p.candidates_rejected
        );
        assert_eq!(
            cmp.ring_5p.candidates_sampled,
            cmp.ring_5p.trials + cmp.ring_5p.candidates_rejected
        );
        assert_eq!(
            cmp.ring_6p.candidates_sampled,
            cmp.ring_6p.trials + cmp.ring_6p.candidates_rejected
        );
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

        // Static-model regression check: 1-turn LP ring HHI lies in Mode A target band (< 0.18).
        assert!(
            cmp.ring_5p.mean_hhi < cmp.random_5p_4r.mean_hhi,
            "ring_5p static-model HHI {} must be lower than random_5p_4r static-model HHI {}",
            cmp.ring_5p.mean_hhi,
            cmp.random_5p_4r.mean_hhi
        );
        assert!(
            cmp.ring_5p.mean_hhi < 0.18,
            "ring_5p static-model mean HHI {} should lie under Mode A prior < 0.18 (report ref {})",
            cmp.ring_5p.mean_hhi,
            hyp.chiral_ring_5p_hhi
        );
        assert!(
            cmp.ring_4p.max_pair_share_observed
                <= SeatCount::Four.default_max_pair_share_cap() + 1e-9,
            "ring_4p observed max_pair_share {} exceeded C6 cap",
            cmp.ring_4p.max_pair_share_observed
        );
        assert!(
            cmp.ring_5p.max_pair_share_observed
                <= SeatCount::Five.default_max_pair_share_cap() + 1e-9,
            "ring_5p observed max_pair_share {} exceeded C6 cap",
            cmp.ring_5p.max_pair_share_observed
        );
        assert!(
            cmp.ring_6p.max_pair_share_observed
                <= SeatCount::Six.default_max_pair_share_cap() + 1e-9,
            "ring_6p observed max_pair_share {} exceeded C6 cap",
            cmp.ring_6p.max_pair_share_observed
        );

        println!(
            "EMPIRICAL n={}: rand_5p_4r lockin={:.3} cov75={:.3} model_hhi={:.3} mean_inv_hhi={:.2} max_pair={:.3} | rand_5p_6r cov75={:.3} model_hhi={:.3} mean_inv_hhi={:.2} max_pair={:.3} | ring_4p rej={:.3} ({}/{}) model_hhi={:.3} mean_inv_hhi={:.2} act={:.1} max_pair={:.3} | ring_5p rej={:.3} ({}/{}) model_hhi={:.3} mean_inv_hhi={:.2} act={:.1} max_pair={:.3} | ring_6p rej={:.3} ({}/{}) model_hhi={:.3} mean_inv_hhi={:.2} act={:.1} max_pair={:.3}",
            cmp.trials,
            cmp.random_5p_4r.frac_100pct_dyad_lockin,
            cmp.random_5p_4r.frac_75pct_dyad_coverage,
            cmp.random_5p_4r.mean_hhi,
            cmp.random_5p_4r.mean_effective_dyads,
            cmp.random_5p_4r.mean_max_pair_share,
            cmp.random_5p_6r.frac_75pct_dyad_coverage,
            cmp.random_5p_6r.mean_hhi,
            cmp.random_5p_6r.mean_effective_dyads,
            cmp.random_5p_6r.mean_max_pair_share,
            cmp.ring_4p.filter_rejection_rate,
            cmp.ring_4p.candidates_rejected,
            cmp.ring_4p.candidates_sampled,
            cmp.ring_4p.mean_hhi,
            cmp.ring_4p.mean_effective_dyads,
            cmp.ring_4p.mean_active_pairs,
            cmp.ring_4p.mean_max_pair_share,
            cmp.ring_5p.filter_rejection_rate,
            cmp.ring_5p.candidates_rejected,
            cmp.ring_5p.candidates_sampled,
            cmp.ring_5p.mean_hhi,
            cmp.ring_5p.mean_effective_dyads,
            cmp.ring_5p.mean_active_pairs,
            cmp.ring_5p.mean_max_pair_share,
            cmp.ring_6p.filter_rejection_rate,
            cmp.ring_6p.candidates_rejected,
            cmp.ring_6p.candidates_sampled,
            cmp.ring_6p.mean_hhi,
            cmp.ring_6p.mean_effective_dyads,
            cmp.ring_6p.mean_active_pairs,
            cmp.ring_6p.mean_max_pair_share,
        );
    }

    #[test]
    fn hhi_max_pair_share_and_rollout_grading_are_deterministic_and_bounded() {
        let uniform = compute_hhi(&[10.0, 10.0, 10.0, 10.0]);
        assert!((uniform.max_pair_share - 0.25).abs() < 1e-12);
        let skewed = compute_hhi(&[70.0, 20.0, 10.0]);
        assert!((skewed.max_pair_share - 0.70).abs() < 1e-12);
        let empty = compute_hhi(&[]);
        assert_eq!(empty.max_pair_share, 0.0);

        for sc in SeatCount::ALL {
            let cfg = GeneratorConfig {
                seat_count: Some(sc),
                ..GeneratorConfig::default()
            };
            let Ok((table, report)) = generate_ring_table(42 + u64::from(sc.as_u8()), &cfg) else {
                panic!("table generates");
            };
            let g1 = compute_rollout_grading(&table, &report);
            let g2 = compute_rollout_grading(&table, &report);
            assert_eq!(g1, g2);
            assert_eq!(g1.per_card_completion_bps.len(), table.mandate_deck.len());
            assert_eq!(g1.distinct_refined_goods_produced, sc.as_usize());
            assert!(g1.total_project_batches > 0);
            assert!(g1.concordat_batches > 0);
            assert!(verify_rollout_grading(&table, &g1, sc.default_max_pair_share_cap()).is_ok());

            let strict_fail = verify_rollout_grading(&table, &g1, 0.05);
            assert!(matches!(
                strict_fail,
                Err(FeasibilityError::RolloutGateFailed(_))
            ));
        }
    }
}
