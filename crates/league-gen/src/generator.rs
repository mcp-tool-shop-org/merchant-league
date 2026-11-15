//! Seeded chiral converter-ring table generator and Prioritized Level Replay (PLR) sampler.

use serde::{Deserialize, Serialize};

use crate::empirical::{compute_rollout_grading, verify_rollout_grading};
use crate::feasibility::{FeasibilityReport, check_feasibility};
use crate::rng::SeedStream;
use crate::schema::{
    ALLOWED_COMMODITY_GLOSSARY, ConverterSpec, CurrencyCode, FeasibilityError, GENERATOR_VERSION,
    GenError, GeneratorConfig, HorizonShockKind, MAX_ATTEMPTS_CAP, NUMERAIRE_CODE, RingTable,
    ScheduledShock, SeatCount, SeatSpec, default_mandate_deck_for_table,
    default_projects_for_seats,
};

/// Stochastic season close turn window `(24, 28)` from `docs/design-v2.md` §6.2.
const SEASON_TURN_WINDOW: (u8, u8) = (24, 28);

/// Base per-turn raw extraction yield range (provisional, calibrate in playtest 1).
const BASE_YIELD_RANGE: (f64, f64) = (9.0, 12.0);
/// Base primary converter input `R_{i+1}` stoichiometry range (provisional, calibrate in playtest 1;
/// offset from `R_{i+2}` keeps even-`N` chiral input matrices well-conditioned under shadow pricing).
const BASE_IN_A_RANGE: (f64, f64) = (1.25, 1.55);
/// Base primary converter input `R_{i+2}` stoichiometry range (provisional, calibrate in playtest 1).
const BASE_IN_B_RANGE: (f64, f64) = (2.45, 2.75);
/// Base primary converter output `M_i` batch range (provisional, calibrate in playtest 1).
const BASE_OUT_RANGE: (f64, f64) = (3.1, 3.9);
/// Base mandatory upkeep quantity range (provisional, calibrate in playtest 1).
const BASE_UPKEEP_RANGE: (f64, f64) = (1.6, 2.2);
/// Base Syndicate Project fundamental valuation range for `M_i` (provisional, calibrate in playtest 1).
const BASE_PROJECT_VAL_RANGE: (f64, f64) = (1.6, 2.8);

/// Stationary G1 per-seat stoichiometry and yield relative jitter span (`±0.35%` from attempt 0,
/// zero annealing; calibrated for matched variety and >=25% attempt-0 pass rate across `N = 4..=6`
/// under `ShadowPrice`).
const STOICH_JITTER_SPAN: f64 = 0.0035;
/// Stationary G1 per-seat upkeep and project valuation relative jitter span (`±0.35%` from attempt 0).
const VALUE_JITTER_SPAN: f64 = 0.0035;
/// Stationary G1 half-band fraction of `max_tariff` around `base_tariff` (capped at
/// [`MAX_TARIFF_JITTER_ABS`]).
const TARIFF_JITTER_HALF_FRAC: f64 = 0.08;
/// Maximum absolute half-band (`±0.018`) for per-seat baseline tariff jitter around `base_tariff`.
const MAX_TARIFF_JITTER_ABS: f64 = 0.018;
/// Stationary G1 half-band fraction of `(decay_hi - decay_lo)` around `base_decay`.
const DECAY_JITTER_HALF_FRAC: f64 = 0.20;

/// Primary converter capacity cap per turn (`4.0` batch runs; provisional, calibrate in playtest 1).
const PRIMARY_MAX_RUNS_PER_TURN: f64 = 4.0;
/// Secondary cross-converter input intensity multiplier (`1.12x`; provisional, calibrate in playtest 1).
const SECONDARY_INPUT_MULT: f64 = 1.12;
/// Secondary cross-converter output yield multiplier (`0.94x`; provisional, calibrate in playtest 1).
const SECONDARY_OUTPUT_MULT: f64 = 0.94;
/// Secondary cross-converter capacity cap per turn (`2.0` batch runs; provisional, calibrate in playtest 1).
const SECONDARY_MAX_RUNS_PER_TURN: f64 = 2.0;

/// Starting turns of the 4 non-overlapping 3-stage `OracleSet` shock windows across turns `2..=22`
/// (provisional schedule spacing within the `docs/design-v2.md` §6.1–§6.2 season).
const SHOCK_WINDOW_STARTS: [u8; 4] = [2, 7, 12, 18];
/// Supply freeze shock magnitude range (provisional, calibrate in playtest 1).
const SHOCK_SUPPLY_FREEZE_RANGE: (f64, f64) = (0.18, 0.35);
/// Tariff holiday reduction range (provisional, calibrate in playtest 1).
const SHOCK_TARIFF_HOLIDAY_RANGE: (f64, f64) = (0.50, 1.00);
/// Demand surge Syndicate Project bonus weight range (provisional, calibrate in playtest 1).
const SHOCK_DEMAND_SURGE_RANGE: (f64, f64) = (0.15, 0.30);
/// Spoilage spike extra decay range (provisional, calibrate in playtest 1).
const SHOCK_SPOILAGE_SPIKE_RANGE: (f64, f64) = (0.15, 0.30);

/// Generates a feasible chiral converter-ring [`RingTable`] deterministically from `seed`
/// and `config`, proving all three LP feasibility invariants, C6 pair-share cap, and B2.2
/// multi-agent rollout grading before accepting the seed.
pub fn generate_ring_table(
    seed: u64,
    config: &GeneratorConfig,
) -> Result<(RingTable, FeasibilityReport), GenError> {
    validate_generator_config(config)?;
    let mut stream = SeedStream::new(seed, b"ring-table");

    // Draw seat count strictly once per seed before the retry loop so filter rejections
    // never bias `seat_count: None` toward lower-seat-count rings.
    let seat_count = match config.seat_count {
        Some(sc) => sc,
        None => {
            let idx = stream.next_u32_bounded(SeatCount::ALL.len() as u32) as usize;
            SeatCount::ALL[idx]
        }
    };
    let effective_cap = config
        .max_pair_share_cap
        .unwrap_or_else(|| seat_count.default_max_pair_share_cap());

    let mut last_error =
        FeasibilityError::InvalidTable("no generation attempts executed".to_owned());

    for attempt in 0..config.max_attempts {
        let candidate = sample_candidate(seed, attempt, seat_count, config, &mut stream)?;
        match check_feasibility(&candidate, config) {
            Ok(report) => {
                let grading = compute_rollout_grading(&candidate, &report);
                match verify_rollout_grading(&candidate, &grading, effective_cap) {
                    Ok(()) => return Ok((candidate, report)),
                    Err(err) => last_error = err,
                }
            }
            Err(err) => last_error = err,
        }
    }

    Err(GenError::NoFeasibleTableFound {
        attempts: config.max_attempts,
        last_error,
    })
}

fn validate_generator_config(config: &GeneratorConfig) -> Result<(), GenError> {
    if config.max_attempts == 0 || config.max_attempts > MAX_ATTEMPTS_CAP {
        return Err(GenError::InvalidConfig(format!(
            "max_attempts must lie in 1..={MAX_ATTEMPTS_CAP}"
        )));
    }
    if !config.ev_tolerance.is_finite() || config.ev_tolerance <= 0.0 || config.ev_tolerance >= 1.0
    {
        return Err(GenError::InvalidConfig(
            "ev_tolerance must lie in (0.0, 1.0)".to_owned(),
        ));
    }
    if !config.min_surplus_ratio.is_finite() || config.min_surplus_ratio <= 0.0 {
        return Err(GenError::InvalidConfig(
            "min_surplus_ratio must be strictly positive".to_owned(),
        ));
    }
    if !config.max_tariff.is_finite() || !(0.0..=0.35).contains(&config.max_tariff) {
        return Err(GenError::InvalidConfig(
            "max_tariff must lie in [0.0, 0.35]".to_owned(),
        ));
    }
    if let Some(cap) = config.max_pair_share_cap
        && (!cap.is_finite() || cap <= 0.0 || cap > 1.0)
    {
        return Err(GenError::InvalidConfig(
            "max_pair_share_cap must lie in (0.0, 1.0]".to_owned(),
        ));
    }
    let (decay_lo, decay_hi) = config.decay_range;
    if !decay_lo.is_finite()
        || !decay_hi.is_finite()
        || decay_lo < 0.0
        || decay_hi >= 1.0
        || decay_lo > decay_hi
    {
        return Err(GenError::InvalidConfig(
            "decay_range must satisfy 0.0 <= low <= high < 1.0".to_owned(),
        ));
    }
    Ok(())
}

fn sample_candidate(
    seed: u64,
    attempt: u32,
    seat_count: SeatCount,
    config: &GeneratorConfig,
    stream: &mut SeedStream,
) -> Result<RingTable, GenError> {
    let n = seat_count.as_usize();

    // Sample `2 * N` distinct curated game-word commodity currency codes (`numeraire` is fixed `CRED`).
    let codes = sample_unique_codes(stream, 2 * n)?;

    let base_yield = stream.range_f64(BASE_YIELD_RANGE.0, BASE_YIELD_RANGE.1);
    let base_in_a = stream.range_f64(BASE_IN_A_RANGE.0, BASE_IN_A_RANGE.1);
    let base_in_b = stream.range_f64(BASE_IN_B_RANGE.0, BASE_IN_B_RANGE.1);
    let base_out = stream.range_f64(BASE_OUT_RANGE.0, BASE_OUT_RANGE.1);
    let base_upkeep = stream.range_f64(BASE_UPKEEP_RANGE.0, BASE_UPKEEP_RANGE.1);
    let base_project_val = stream.range_f64(BASE_PROJECT_VAL_RANGE.0, BASE_PROJECT_VAL_RANGE.1);
    let base_tariff = stream.range_f64(0.0, config.max_tariff);
    let base_decay = stream.range_f64(config.decay_range.0, config.decay_range.1);

    // 4-seat rings omit `(i+2, i+3)` secondary cross-converters because `(i + 3) % 4 == (i - 1) % 4`
    // wraps onto the seat's own upkeep producer rather than creating an independent cross-ring chord.
    let attach_secondary = config.enable_secondary_converters
        && n >= 5
        && (seat_count == SeatCount::Six || stream.next_u32_bounded(2) == 1);

    let tariff_half = (config.max_tariff * TARIFF_JITTER_HALF_FRAC).min(MAX_TARIFF_JITTER_ABS);
    let decay_half = (config.decay_range.1 - config.decay_range.0) * DECAY_JITTER_HALF_FRAC;
    let t_lo = (base_tariff - tariff_half).max(0.0);
    let t_hi = (base_tariff + tariff_half).min(config.max_tariff);
    let d_lo = (base_decay - decay_half).max(config.decay_range.0);
    let d_hi = (base_decay + decay_half).min(config.decay_range.1);

    let mut seats = Vec::with_capacity(n);
    for i in 0..n {
        let jitter = |s: &mut SeedStream, span: f64| 1.0 + s.range_f64(-span, span);
        let raw_yield_per_turn = round3(base_yield * jitter(stream, STOICH_JITTER_SPAN));
        let in_a = round3(base_in_a * jitter(stream, STOICH_JITTER_SPAN));
        let in_b = round3(base_in_b * jitter(stream, STOICH_JITTER_SPAN));
        let out_amt = round3(base_out * jitter(stream, STOICH_JITTER_SPAN));
        let upkeep_amount = round3(base_upkeep * jitter(stream, VALUE_JITTER_SPAN));
        let refined_project_value = round3(base_project_val * jitter(stream, VALUE_JITTER_SPAN));

        let raw_export_tariff = round4(stream.range_f64(t_lo, t_hi));
        let refined_export_tariff = round4(stream.range_f64(t_lo, t_hi));
        let perishable_decay_rate = round4(stream.range_f64(d_lo, d_hi));

        let next_1 = ((i + 1) % n) as u8;
        let next_2 = ((i + 2) % n) as u8;
        let next_3 = ((i + 3) % n) as u8;

        let mut converters = vec![ConverterSpec {
            owner: i as u8,
            inputs: vec![(next_1, in_a), (next_2, in_b)],
            output_good: i as u8,
            output_amount: out_amt,
            max_runs_per_turn: PRIMARY_MAX_RUNS_PER_TURN,
            is_secondary: false,
        }];

        if attach_secondary {
            converters.push(ConverterSpec {
                owner: i as u8,
                inputs: vec![
                    (next_2, round3(in_a * SECONDARY_INPUT_MULT)),
                    (next_3, round3(in_b * SECONDARY_INPUT_MULT)),
                ],
                output_good: i as u8,
                output_amount: round3(out_amt * SECONDARY_OUTPUT_MULT),
                max_runs_per_turn: SECONDARY_MAX_RUNS_PER_TURN,
                is_secondary: true,
            });
        }

        seats.push(SeatSpec {
            seat: i as u8,
            raw_good: i as u8,
            raw_code: codes[2 * i],
            raw_yield_per_turn,
            refined_good: i as u8,
            refined_code: codes[2 * i + 1],
            refined_project_value,
            upkeep_good: ((i + n - 1) % n) as u8,
            upkeep_amount,
            raw_export_tariff,
            refined_export_tariff,
            perishable_decay_rate,
            converters,
        });
    }

    let projects = default_projects_for_seats(&seats);
    let mandate_deck = default_mandate_deck_for_table(seat_count, &seats);
    let shocks = sample_shocks(stream, n);

    Ok(RingTable {
        generator_version: GENERATOR_VERSION.to_owned(),
        config: config.clone(),
        seed,
        attempt,
        seat_count,
        numeraire_code: NUMERAIRE_CODE.to_owned(),
        season_turn_window: SEASON_TURN_WINDOW,
        seats,
        projects,
        mandate_deck,
        shocks,
    })
}

fn sample_unique_codes(
    stream: &mut SeedStream,
    count: usize,
) -> Result<Vec<CurrencyCode>, GenError> {
    if count > ALLOWED_COMMODITY_GLOSSARY.len() {
        return Err(GenError::InvalidConfig(format!(
            "requested {count} unique codes exceeds allow-list size {}",
            ALLOWED_COMMODITY_GLOSSARY.len()
        )));
    }
    let indices = stream.sample_distinct_indices(ALLOWED_COMMODITY_GLOSSARY.len(), count);
    let mut out = Vec::with_capacity(count);
    for idx in indices {
        let code = CurrencyCode::from_bytes(ALLOWED_COMMODITY_GLOSSARY[idx].0)
            .map_err(|err| GenError::InvalidConfig(format!("invalid allow-list code: {err}")))?;
        out.push(code);
    }
    Ok(out)
}

fn sample_shocks(stream: &mut SeedStream, n: usize) -> Vec<ScheduledShock> {
    let mut kinds = [
        HorizonShockKind::SupplyFreeze,
        HorizonShockKind::TariffHoliday,
        HorizonShockKind::DemandSurge,
        HorizonShockKind::SpoilageSpike,
    ];
    stream.shuffle(&mut kinds);

    let seat_order = stream.sample_distinct_indices(n, kinds.len());
    let mut shocks = Vec::with_capacity(kinds.len());
    for (idx, (&kind, &seat)) in kinds.iter().zip(seat_order.iter()).enumerate() {
        let hint_turn = SHOCK_WINDOW_STARTS[idx] + (stream.next_u32_bounded(3) as u8);
        let raw_mag = match kind {
            HorizonShockKind::SupplyFreeze => {
                stream.range_f64(SHOCK_SUPPLY_FREEZE_RANGE.0, SHOCK_SUPPLY_FREEZE_RANGE.1)
            }
            HorizonShockKind::TariffHoliday => {
                stream.range_f64(SHOCK_TARIFF_HOLIDAY_RANGE.0, SHOCK_TARIFF_HOLIDAY_RANGE.1)
            }
            HorizonShockKind::DemandSurge => {
                stream.range_f64(SHOCK_DEMAND_SURGE_RANGE.0, SHOCK_DEMAND_SURGE_RANGE.1)
            }
            HorizonShockKind::SpoilageSpike => {
                stream.range_f64(SHOCK_SPOILAGE_SPIKE_RANGE.0, SHOCK_SPOILAGE_SPIKE_RANGE.1)
            }
        };
        let magnitude = round3(raw_mag).max(0.001);
        shocks.push(ScheduledShock {
            hint_turn,
            forecast_turn: hint_turn + 1,
            realize_turn: hint_turn + 2,
            target_seat: seat as u8,
            kind,
            magnitude,
        });
    }
    shocks
}

fn round3(x: f64) -> f64 {
    (x * 1_000.0).round() / 1_000.0
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// Entry tracked inside a [`PlrSampler`] level buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlrEntry {
    /// Table generator seed.
    pub seed: u64,
    /// Priority score (e.g., policy value-loss / regret variance).
    pub score: f64,
    /// Replay step count at the last time this seed was sampled or updated.
    pub last_seen_step: u64,
}

/// Decision returned by [`PlrSampler::sample_next`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlrChoice {
    /// Sample a fresh unseen table seed and enroll it into the buffer.
    Fresh(u64),
    /// Replay an existing buffer seed selected via rank-prioritized staleness weighting.
    Replay(u64),
}

/// Deterministic Prioritized Level Replay (PLR) seed scheduler for headless training.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlrSampler {
    max_buffer_len: usize,
    staleness_weight: f64,
    temperature: f64,
    replay_prob: f64,
    step: u64,
    next_fresh_counter: u64,
    entries: Vec<PlrEntry>,
    stream: PlrRngState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlrRngState {
    seed: u64,
    draws: u64,
}

impl PlrRngState {
    fn next_unit(&mut self) -> f64 {
        // Advance deterministically by draw count in a stateless, serde-friendly manner.
        let mut step_stream = SeedStream::new(
            self.seed ^ self.draws.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            b"plr-draw",
        );
        self.draws = self.draws.wrapping_add(1);
        step_stream.next_f64_unit()
    }
}

impl PlrSampler {
    /// Creates a new PLR sampler with buffer capacity `max_buffer_len`, staleness weight
    /// `staleness_weight` in `[0.0, 1.0]`, rank exponent `temperature > 0.0`, and replay probability
    /// `replay_prob` in `[0.0, 1.0]`.
    ///
    /// Note: `temperature == 1.0` uses exact rational reciprocals `1 / rank`; non-unit `temperature`
    /// invokes platform `f64::powf`.
    #[must_use]
    pub fn new(
        rng_seed: u64,
        max_buffer_len: usize,
        staleness_weight: f64,
        temperature: f64,
        replay_prob: f64,
    ) -> Self {
        Self {
            max_buffer_len: max_buffer_len.max(1),
            staleness_weight: staleness_weight.clamp(0.0, 1.0),
            temperature: temperature.max(0.05),
            replay_prob: replay_prob.clamp(0.0, 1.0),
            step: 0,
            next_fresh_counter: 1,
            entries: Vec::new(),
            stream: PlrRngState {
                seed: rng_seed,
                draws: 0,
            },
        }
    }

    /// Returns the number of tracked seeds currently in the replay buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when the replay buffer contains no seeds yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records or updates `score` (non-negative regret/value-loss metric) for `seed`.
    ///
    /// When the buffer is already at `max_buffer_len`, a new seed only evicts the lowest-priority
    /// entry if `clean_score >= lowest.score`, preventing low-value entries from displacing higher ones.
    pub fn record_score(&mut self, seed: u64, score: f64) {
        let clean_score = if score.is_finite() {
            score.max(0.0)
        } else {
            0.0
        };
        if let Some(existing) = self.entries.iter_mut().find(|e| e.seed == seed) {
            existing.score = clean_score;
            existing.last_seen_step = self.step;
            return;
        }
        if self.entries.len() >= self.max_buffer_len
            && let Some((worst_idx, worst_entry)) = self
                .entries
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.score.total_cmp(&b.score))
        {
            if clean_score < worst_entry.score {
                return;
            }
            self.entries.swap_remove(worst_idx);
        }
        self.entries.push(PlrEntry {
            seed,
            score: clean_score,
            last_seen_step: self.step,
        });
    }

    /// Returns a read-only view of all replay buffer entries.
    #[must_use]
    pub fn entries(&self) -> &[PlrEntry] {
        &self.entries
    }

    /// Chooses whether to replay an existing seed (weighted by rank priority and staleness)
    /// or generate a fresh seed.
    pub fn sample_next(&mut self) -> PlrChoice {
        self.step = self.step.saturating_add(1);
        let fill_fraction = (self.entries.len() as f64) / (self.max_buffer_len as f64);
        let effective_replay_prob = self.replay_prob * fill_fraction;
        let roll = self.stream.next_unit();

        if !self.entries.is_empty() && roll < effective_replay_prob {
            let weights = self.compute_weights();
            let total_w: f64 = weights.iter().sum();
            let pick_val = self.stream.next_unit() * total_w;
            let mut acc = 0.0;
            let mut chosen_idx = weights.len().saturating_sub(1);
            for (idx, &w) in weights.iter().enumerate() {
                acc += w;
                if pick_val <= acc {
                    chosen_idx = idx;
                    break;
                }
            }
            self.entries[chosen_idx].last_seen_step = self.step;
            return PlrChoice::Replay(self.entries[chosen_idx].seed);
        }

        let mut fresh_seed = self
            .stream
            .seed
            .wrapping_add(self.next_fresh_counter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        self.next_fresh_counter = self.next_fresh_counter.wrapping_add(1);
        while self.entries.iter().any(|e| e.seed == fresh_seed) {
            fresh_seed = fresh_seed.wrapping_add(1);
        }
        PlrChoice::Fresh(fresh_seed)
    }

    fn compute_weights(&self) -> Vec<f64> {
        let m = self.entries.len();
        let mut order: Vec<usize> = (0..m).collect();
        order.sort_by(|&a, &b| self.entries[b].score.total_cmp(&self.entries[a].score));

        let mut score_weights = vec![0.0_f64; m];
        let mut score_sum = 0.0_f64;
        let unit_temp = (self.temperature - 1.0).abs() <= 1e-12;
        for (rank, &idx) in order.iter().enumerate() {
            let inv_rank = 1.0 / ((rank + 1) as f64);
            let w = if unit_temp {
                inv_rank
            } else {
                inv_rank.powf(1.0 / self.temperature)
            };
            score_weights[idx] = w;
            score_sum += w;
        }

        let mut stale_weights = vec![0.0_f64; m];
        let mut stale_sum = 0.0_f64;
        for (idx, entry) in self.entries.iter().enumerate() {
            let age = self.step.saturating_sub(entry.last_seen_step) as f64;
            stale_weights[idx] = age.max(1.0);
            stale_sum += stale_weights[idx];
        }

        let rho = self.staleness_weight;
        (0..m)
            .map(|idx| {
                let ps = if score_sum > 0.0 {
                    score_weights[idx] / score_sum
                } else {
                    1.0 / (m as f64)
                };
                let pc = if stale_sum > 0.0 {
                    stale_weights[idx] / stale_sum
                } else {
                    1.0 / (m as f64)
                };
                (1.0 - rho) * ps + rho * pc
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::feasibility::{compute_shadow_price_seat_evs, compute_shapley_seat_evs};
    use crate::schema::{
        EvMethod, PROJECT_CONCORDAT_ID, commodity_glossary_word, is_allowed_commodity_code,
        is_denied_currency_code,
    };

    #[test]
    fn generates_feasible_tables_for_all_seat_counts_and_both_ev_methods() {
        for ev_method in [EvMethod::ShadowPrice, EvMethod::Shapley] {
            for sc in SeatCount::ALL {
                let cfg = GeneratorConfig {
                    seat_count: Some(sc),
                    ev_method,
                    ..GeneratorConfig::default()
                };
                for seed in [0u64, 1, 26, 20_261_115] {
                    let Ok((table, report)) = generate_ring_table(seed, &cfg) else {
                        panic!(
                            "generation failed for seat_count={sc:?}, method={ev_method:?}, seed={seed}"
                        );
                    };
                    assert_eq!(table.seat_count, sc);
                    assert_eq!(table.numeraire_code, NUMERAIRE_CODE);
                    assert_eq!(table.generator_version, GENERATOR_VERSION);
                    assert_eq!(table.config, cfg);
                    assert_eq!(table.seats.len(), sc.as_usize());
                    assert_eq!(report.ev_method, ev_method);
                    assert!(report.max_2seat_upkeep_coverage < 1e-9);
                    assert!(report.surplus_to_upkeep_ratio >= cfg.min_surplus_ratio);
                    assert!(report.max_relative_ev_deviation <= cfg.ev_tolerance + 1e-9);
                }
            }
        }
    }

    #[test]
    fn default_none_seat_count_is_uniform_across_fixed_seed_sweep() {
        let cfg = GeneratorConfig::default();
        let mut counts = [0usize; 3];
        for seed in 0u64..400 {
            let Ok((table, _)) = generate_ring_table(seed, &cfg) else {
                panic!("default generation failed for seed={seed}");
            };
            match table.seat_count {
                SeatCount::Four => counts[0] += 1,
                SeatCount::Five => counts[1] += 1,
                SeatCount::Six => counts[2] += 1,
            }
        }
        assert_eq!(counts.iter().sum::<usize>(), 400);
        // Expected 400 / 3 = 133.3 per bucket (binomial SD ~9.4). The old biased sampler
        // produced {4: 298, 5: 72, 6: 30}, failing all three bounds by > 6 SDs.
        for (idx, &c) in counts.iter().enumerate() {
            assert!(
                (105..=162).contains(&c),
                "seat_count bucket {idx} count {c} out of uniform [105..=162]; full={counts:?}"
            );
        }
        println!(
            "SEAT_UNIFORMITY_400: N4={} N5={} N6={}",
            counts[0], counts[1], counts[2]
        );
    }

    #[test]
    fn generated_table_embeds_config_version_and_cred_numeraire() {
        let cfg = GeneratorConfig::default();
        let Ok((table, _)) = generate_ring_table(42, &cfg) else {
            panic!("seed 42 must succeed");
        };
        let Ok(json) = serde_json::to_string(&table) else {
            panic!("RingTable must serialize");
        };
        for expected_key in [
            "generator_version",
            GENERATOR_VERSION,
            "CRED",
            "ev_tolerance",
            "min_surplus_ratio",
            "max_tariff",
            "decay_range",
            "enable_secondary_converters",
            "max_attempts",
            "ev_method",
            "max_pair_share_cap",
            "projects",
            "mandate_deck",
        ] {
            assert!(
                json.contains(expected_key),
                "serialized RingTable missing {expected_key}"
            );
        }
        assert!(
            table.projects.iter().any(|p| p.id == PROJECT_CONCORDAT_ID),
            "generated table missing Concordat project"
        );
        assert!(
            !table.mandate_deck.is_empty(),
            "generated table missing public mandate deck"
        );
    }

    #[test]
    fn generated_codes_always_use_curated_allowlist_and_pass_external_iso_crypto_audit() {
        // Independent external ISO 4217 (active + historical) and XRPL/crypto ticker audit list
        // including all tickers flagged in #26 and PR #18 Opus re-review B-2.
        const EXTERNAL_TICKER_AUDIT: &[[u8; 3]] = &[
            *b"AED", *b"ALL", *b"ANG", *b"AOA", *b"ARS", *b"ATS", *b"AUD", *b"AZM", *b"AZN",
            *b"BAM", *b"BBD", *b"BDT", *b"BEC", *b"BEF", *b"BEL", *b"BGB", *b"BGL", *b"BGN",
            *b"BHD", *b"BNB", *b"BRL", *b"BSD", *b"BSV", *b"BTC", *b"BTG", *b"BWP", *b"BYN",
            *b"CAD", *b"CHF", *b"CLP", *b"CNY", *b"COP", *b"CRC", *b"CRD", *b"CSD", *b"CUP",
            *b"CYP", *b"CZK", *b"DAI", *b"DEM", *b"DKK", *b"DOT", *b"DZD", *b"EEK", *b"EGP",
            *b"ELS", *b"ESP", *b"ETB", *b"ETH", *b"EUR", *b"EVR", *b"FIM", *b"FJD", *b"FLR",
            *b"FRF", *b"FTT", *b"GBP", *b"GBX", *b"GEL", *b"GHC", *b"GHS", *b"GRD", *b"GTQ",
            *b"HKD", *b"HRK", *b"HUF", *b"IDR", *b"IEP", *b"ILS", *b"INR", *b"IQD", *b"IRR",
            *b"ISK", *b"ITL", *b"JMD", *b"JOD", *b"JPY", *b"KES", *b"KGS", *b"KRW", *b"KWD",
            *b"KZT", *b"LEO", *b"LKR", *b"LTC", *b"LTL", *b"LUF", *b"LVL", *b"LYD", *b"MAD",
            *b"MDL", *b"MKD", *b"MMK", *b"MOP", *b"MRO", *b"MRU", *b"MTL", *b"MUR", *b"MXN",
            *b"MYR", *b"MZM", *b"MZN", *b"NAD", *b"NGN", *b"NIO", *b"NLG", *b"NMR", *b"NOK",
            *b"NPR", *b"NZD", *b"OMR", *b"PAB", *b"PAX", *b"PEN", *b"PHP", *b"PKR", *b"PLN",
            *b"PTE", *b"PYG", *b"QAR", *b"ROK", *b"ROL", *b"RON", *b"RPL", *b"RSD", *b"RUB",
            *b"RUR", *b"SAR", *b"SDG", *b"SEK", *b"SGB", *b"SGD", *b"SIT", *b"SKK", *b"SLL",
            *b"SOL", *b"SOS", *b"SRD", *b"SSP", *b"STD", *b"STN", *b"SYP", *b"THB", *b"TJS",
            *b"TMT", *b"TND", *b"TRL", *b"TRY", *b"TTD", *b"TWD", *b"TZS", *b"UAH", *b"UGX",
            *b"UNI", *b"USD", *b"UST", *b"UYU", *b"UZS", *b"VEB", *b"VEF", *b"VES", *b"VND",
            *b"WBT", *b"XAF", *b"XAG", *b"XAU", *b"XCD", *b"XCG", *b"XDR", *b"XLM", *b"XMR",
            *b"XOF", *b"XPD", *b"XPF", *b"XPT", *b"XRE", *b"XRP", *b"XTZ", *b"YER", *b"ZAL",
            *b"ZAR", *b"ZEC", *b"ZMW", *b"ZWD", *b"ZWL",
        ];

        let mut cases: Vec<(u64, GeneratorConfig)> = vec![
            (3, GeneratorConfig::default()),
            (41, GeneratorConfig::default()),
            (42, GeneratorConfig::default()),
            (50, GeneratorConfig::default()),
            (
                6,
                GeneratorConfig {
                    seat_count: Some(SeatCount::Six),
                    ..GeneratorConfig::default()
                },
            ),
            (
                10,
                GeneratorConfig {
                    seat_count: Some(SeatCount::Five),
                    ..GeneratorConfig::default()
                },
            ),
            (
                12,
                GeneratorConfig {
                    seat_count: Some(SeatCount::Six),
                    ..GeneratorConfig::default()
                },
            ),
            (
                78,
                GeneratorConfig {
                    seat_count: Some(SeatCount::Four),
                    ..GeneratorConfig::default()
                },
            ),
        ];
        for seed in 0u64..500 {
            cases.push((seed, GeneratorConfig::default()));
        }

        for (seed, cfg) in cases {
            let Ok((table, _)) = generate_ring_table(seed, &cfg) else {
                panic!("seed {seed} failed under config {:?}", cfg.seat_count);
            };
            assert_eq!(table.numeraire_code, NUMERAIRE_CODE);
            for seat in &table.seats {
                for (kind, code) in [("raw", seat.raw_code), ("refined", seat.refined_code)] {
                    let bytes = code.as_bytes();
                    assert!(
                        is_allowed_commodity_code(bytes),
                        "seed {seed} {kind} code {code} is outside ALLOWED_COMMODITY_GLOSSARY"
                    );
                    assert!(
                        commodity_glossary_word(code).is_some(),
                        "seed {seed} {kind} code {code} has no glossary word"
                    );
                    assert!(
                        !EXTERNAL_TICKER_AUDIT.contains(&bytes),
                        "seed {seed} {kind} code {code} collided with external ticker audit"
                    );
                    assert!(
                        !is_denied_currency_code(bytes),
                        "seed {seed} {kind} code {code} hit deny-list"
                    );
                }
            }
        }
    }

    #[test]
    fn generator_rejects_invalid_config_and_impossible_tolerance() {
        let bad_attempts_zero = GeneratorConfig {
            max_attempts: 0,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_attempts_zero),
            Err(GenError::InvalidConfig(_))
        ));

        let bad_attempts_cap = GeneratorConfig {
            max_attempts: MAX_ATTEMPTS_CAP + 1,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_attempts_cap),
            Err(GenError::InvalidConfig(_))
        ));

        let bad_ev = GeneratorConfig {
            ev_tolerance: 0.0,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_ev),
            Err(GenError::InvalidConfig(_))
        ));

        let bad_surplus = GeneratorConfig {
            min_surplus_ratio: -0.1,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_surplus),
            Err(GenError::InvalidConfig(_))
        ));

        let bad_tariff = GeneratorConfig {
            max_tariff: 0.5,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_tariff),
            Err(GenError::InvalidConfig(_))
        ));

        let bad_decay = GeneratorConfig {
            decay_range: (0.4, 0.2),
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_decay),
            Err(GenError::InvalidConfig(_))
        ));

        let impossible = GeneratorConfig {
            min_surplus_ratio: 100.0,
            max_attempts: 2,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(7, &impossible),
            Err(GenError::NoFeasibleTableFound { attempts: 2, .. })
        ));
    }

    #[test]
    fn plr_sampler_prioritizes_higher_scores_and_ignores_weaker_entries_when_full() {
        let mut plr = PlrSampler::new(123, 2, 0.2, 1.0, 0.9);
        assert!(plr.is_empty());
        assert!(matches!(plr.sample_next(), PlrChoice::Fresh(_)));

        plr.record_score(10, 1.0);
        plr.record_score(20, 10.0);
        plr.record_score(20, f64::NAN);
        plr.record_score(20, 12.0);
        // Exceeding buffer size evicts lowest score (`10`).
        plr.record_score(30, 5.0);
        assert_eq!(plr.len(), 2);
        assert!(plr.entries().iter().all(|e| e.seed == 20 || e.seed == 30));

        // C9: a newcomer scoring below the current buffer minimum (`5.0`) does not evict `30`.
        plr.record_score(40, 2.0);
        assert_eq!(plr.len(), 2);
        assert!(plr.entries().iter().all(|e| e.seed == 20 || e.seed == 30));

        let mut replays = 0usize;
        let mut high_picks = 0usize;
        for _ in 0..80 {
            if let PlrChoice::Replay(seed) = plr.sample_next() {
                replays += 1;
                if seed == 20 {
                    high_picks += 1;
                }
            }
        }
        assert!(replays > 20);
        assert!(high_picks > replays / 2);
    }

    #[test]
    fn shadow_price_and_shapley_comparison_across_same_seeds() {
        let mut sp_max_rel_sum = 0.0_f64;
        let mut sh_max_rel_sum = 0.0_f64;
        let mut max_normalized_diff = 0.0_f64;
        let mut sum_normalized_diff = 0.0_f64;
        let mut sh_passes_when_sp_passes = 0usize;
        let mut sp_attempts_sum = 0usize;
        let mut sh_attempts_sum = 0usize;
        let mut total_tables = 0usize;
        let mut boost_both_reject = 0usize;
        let mut boost_sp_reject_only = 0usize;
        let mut boost_sh_reject_only = 0usize;

        let t_sp_start = Instant::now();
        let mut tables = Vec::with_capacity(180);
        for sc in SeatCount::ALL {
            let cfg_sp = GeneratorConfig {
                seat_count: Some(sc),
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            for seed in 0u64..60 {
                let Ok((table, report_sp)) = generate_ring_table(seed, &cfg_sp) else {
                    panic!("ShadowPrice failed seed={seed} sc={sc:?}");
                };
                sp_attempts_sum += table.attempt as usize;
                sp_max_rel_sum += report_sp.max_relative_ev_deviation;
                tables.push((table, report_sp));
            }
        }
        let sp_elapsed_ms = t_sp_start.elapsed().as_secs_f64() * 1_000.0;

        let t_sh_gen_start = Instant::now();
        for sc in SeatCount::ALL {
            let cfg_sh = GeneratorConfig {
                seat_count: Some(sc),
                ev_method: EvMethod::Shapley,
                ..GeneratorConfig::default()
            };
            for seed in 0u64..60 {
                let Ok((t_sh, _)) = generate_ring_table(seed, &cfg_sh) else {
                    panic!("Shapley failed seed={seed} sc={sc:?}");
                };
                sh_attempts_sum += t_sh.attempt as usize;
            }
        }
        let sh_gen_elapsed_ms = t_sh_gen_start.elapsed().as_secs_f64() * 1_000.0;

        let mut sp_eval_us = 0.0_f64;
        let mut sh_eval_us = 0.0_f64;
        for (table, report_sp) in &tables {
            let t1 = Instant::now();
            let Ok(sp_evs) = compute_shadow_price_seat_evs(table) else {
                panic!("ShadowPrice re-eval failed");
            };
            sp_eval_us += t1.elapsed().as_secs_f64() * 1_000_000.0;

            let t2 = Instant::now();
            let Ok(sh_evs) = compute_shapley_seat_evs(table) else {
                panic!("Shapley re-eval failed");
            };
            sh_eval_us += t2.elapsed().as_secs_f64() * 1_000_000.0;

            let sp_mean = sp_evs.iter().sum::<f64>() / (sp_evs.len() as f64);
            let sh_mean = sh_evs.iter().sum::<f64>() / (sh_evs.len() as f64);
            let sh_rel_dev = sh_evs
                .iter()
                .map(|&v| ((v - sh_mean) / sh_mean).abs())
                .fold(0.0_f64, f64::max);
            sh_max_rel_sum += sh_rel_dev;
            if sh_rel_dev <= 0.03 + 1e-9 {
                sh_passes_when_sp_passes += 1;
            }

            let per_seat_gap = sp_evs
                .iter()
                .zip(sh_evs.iter())
                .map(|(&sp, &sh)| ((sp / sp_mean) - (sh / sh_mean)).abs())
                .fold(0.0_f64, f64::max);
            max_normalized_diff = max_normalized_diff.max(per_seat_gap);
            sum_normalized_diff += per_seat_gap;
            total_tables += 1;

            // Asymmetry probe: boost seat 0 primary converter output +20%.
            let mut boosted = table.clone();
            boosted.seats[0].converters[0].output_amount *= 1.20;
            let cfg_sp = GeneratorConfig {
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            let cfg_sh = GeneratorConfig {
                ev_method: EvMethod::Shapley,
                ..GeneratorConfig::default()
            };
            let sp_rej = check_feasibility(&boosted, &cfg_sp).is_err();
            let sh_rej = check_feasibility(&boosted, &cfg_sh).is_err();
            match (sp_rej, sh_rej) {
                (true, true) => boost_both_reject += 1,
                (true, false) => boost_sp_reject_only += 1,
                (false, true) => boost_sh_reject_only += 1,
                (false, false) => {}
            }
            assert!(
                (report_sp.mean_seat_ev - sp_mean).abs() < 1e-9,
                "consistent mean EV across calls"
            );
        }

        let mut sp_attempt0_pass = 0usize;
        let mut sh_attempt0_pass = 0usize;
        let attempt0_total = 300usize;
        for sc in SeatCount::ALL {
            let cfg_sp_1 = GeneratorConfig {
                seat_count: Some(sc),
                max_attempts: 1,
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            let cfg_sh_1 = GeneratorConfig {
                seat_count: Some(sc),
                max_attempts: 1,
                ev_method: EvMethod::Shapley,
                ..GeneratorConfig::default()
            };
            for seed in 0u64..100 {
                if generate_ring_table(seed, &cfg_sp_1).is_ok() {
                    sp_attempt0_pass += 1;
                }
                if generate_ring_table(seed, &cfg_sh_1).is_ok() {
                    sh_attempt0_pass += 1;
                }
            }
        }

        let n_f = total_tables as f64;
        println!(
            "EV_COMPARISON n={total_tables} (N=4,5,6 x 60 seeds): sp_gen_ms={sp_elapsed_ms:.1} (avg_rej={:.2}) sh_gen_ms={sh_gen_elapsed_ms:.1} (avg_rej={:.2}) | attempt0_pass: sp={sp_attempt0_pass}/{attempt0_total} sh={sh_attempt0_pass}/{attempt0_total} | sp_eval_us={:.1} sh_eval_us={:.1} | sp_mean_max_rel={:.4} sh_mean_max_rel_on_sp={:.4} sh_pass_rate_on_sp={}/{} | mean_norm_gap={:.4} max_norm_gap={:.4} | plus20_out_rej: both={} sp_only={} sh_only={}",
            (sp_attempts_sum as f64) / n_f,
            (sh_attempts_sum as f64) / n_f,
            sp_eval_us / n_f,
            sh_eval_us / n_f,
            sp_max_rel_sum / n_f,
            sh_max_rel_sum / n_f,
            sh_passes_when_sp_passes,
            total_tables,
            sum_normalized_diff / n_f,
            max_normalized_diff,
            boost_both_reject,
            boost_sp_reject_only,
            boost_sh_reject_only,
        );

        assert_eq!(total_tables, 180);
        assert!(
            sp_attempt0_pass >= 75,
            "G1 stationary sampling must accept >= 25% of seeds at attempt 0 under ShadowPrice, got {sp_attempt0_pass}/{attempt0_total}"
        );
        assert!(
            sh_passes_when_sp_passes >= total_tables / 2,
            "Shapley pass rate on ShadowPrice-accepted tables expected >= 50%, got {sh_passes_when_sp_passes}/{total_tables}"
        );
        assert!(
            boost_both_reject + boost_sp_reject_only >= total_tables / 2,
            "asymmetry +20% output probe must trigger rejections reliably"
        );
    }

    #[test]
    fn g1_stationary_dispersion_and_attempt0_acceptance() {
        let stddev_mean = |vals: &[f64]| -> (f64, f64) {
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            (mean, var.sqrt())
        };
        let min_max = |vals: &[f64]| -> (f64, f64) {
            let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (lo, hi)
        };

        let mut sp_attempt0_rates = Vec::new();
        let mut mean_tariff_ranges = Vec::new();
        let mut mean_yield_spreads = Vec::new();

        for sc in SeatCount::ALL {
            let cfg_sp_1 = GeneratorConfig {
                seat_count: Some(sc),
                max_attempts: 1,
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            let mut sp_a0 = 0usize;
            for seed in 0u64..600 {
                if generate_ring_table(seed, &cfg_sp_1).is_ok() {
                    sp_a0 += 1;
                }
            }

            let cfg_sh_1 = GeneratorConfig {
                seat_count: Some(sc),
                max_attempts: 1,
                ev_method: EvMethod::Shapley,
                ..GeneratorConfig::default()
            };
            let mut sh_a0 = 0usize;
            for seed in 0u64..200 {
                if generate_ring_table(seed, &cfg_sh_1).is_ok() {
                    sh_a0 += 1;
                }
            }

            let cfg_full = GeneratorConfig {
                seat_count: Some(sc),
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            let mut full_pass = 0usize;
            let mut attempts_sum = 0u64;
            let mut max_att = 0u32;
            let mut yield_cvs = Vec::new();
            let mut tariff_cvs = Vec::new();
            let mut out_cvs = Vec::new();
            let mut upkeep_cvs = Vec::new();
            let mut yield_spreads = Vec::new();
            let mut tariff_ranges = Vec::new();
            let mut decay_ranges = Vec::new();
            let mut s0_yields = Vec::new();
            let mut s0_tariffs = Vec::new();
            let mut s0_outs = Vec::new();
            let mut s0_decays = Vec::new();
            let mut sh0_mags = Vec::new();

            for seed in 0u64..600 {
                let Ok((t, _)) = generate_ring_table(seed, &cfg_full) else {
                    panic!("G1 seed {seed} failed for {sc:?}");
                };
                full_pass += 1;
                attempts_sum += u64::from(t.attempt);
                max_att = max_att.max(t.attempt);
                let ys: Vec<f64> = t.seats.iter().map(|s| s.raw_yield_per_turn).collect();
                let ts: Vec<f64> = t.seats.iter().map(|s| s.raw_export_tariff).collect();
                let os: Vec<f64> = t
                    .seats
                    .iter()
                    .map(|s| s.converters[0].output_amount)
                    .collect();
                let us: Vec<f64> = t.seats.iter().map(|s| s.upkeep_amount).collect();
                let ds: Vec<f64> = t.seats.iter().map(|s| s.perishable_decay_rate).collect();

                let (ym, ysd) = stddev_mean(&ys);
                let (tm, tsd) = stddev_mean(&ts);
                let (om, osd) = stddev_mean(&os);
                let (um, usd) = stddev_mean(&us);
                yield_cvs.push(ysd / ym.max(1e-12));
                tariff_cvs.push(tsd / tm.max(1e-12));
                out_cvs.push(osd / om.max(1e-12));
                upkeep_cvs.push(usd / um.max(1e-12));

                let (y_lo, y_hi) = min_max(&ys);
                let (t_lo, t_hi) = min_max(&ts);
                let (d_lo, d_hi) = min_max(&ds);
                yield_spreads.push((y_hi / y_lo) - 1.0);
                tariff_ranges.push(t_hi - t_lo);
                decay_ranges.push(d_hi - d_lo);

                s0_yields.push(ys[0]);
                s0_tariffs.push(ts[0]);
                s0_outs.push(os[0]);
                s0_decays.push(ds[0]);
                sh0_mags.push(t.shocks[0].magnitude);
            }

            let nf = full_pass as f64;
            let mean_ycv = yield_cvs.iter().sum::<f64>() / nf;
            let mean_tcv = tariff_cvs.iter().sum::<f64>() / nf;
            let mean_ocv = out_cvs.iter().sum::<f64>() / nf;
            let mean_ucv = upkeep_cvs.iter().sum::<f64>() / nf;
            let mean_yspread = yield_spreads.iter().sum::<f64>() / nf;
            let mean_trange = tariff_ranges.iter().sum::<f64>() / nf;
            let mean_drange = decay_ranges.iter().sum::<f64>() / nf;
            let (s0y_m, s0y_sd) = stddev_mean(&s0_yields);
            let (s0t_m, s0t_sd) = stddev_mean(&s0_tariffs);
            let (s0o_m, s0o_sd) = stddev_mean(&s0_outs);
            let (s0d_m, s0d_sd) = stddev_mean(&s0_decays);
            let (sh0_m, sh0_sd) = stddev_mean(&sh0_mags);
            println!(
                "G1_AFTER N={}: sp_attempt0={sp_a0}/600 ({:.1}%) sh_attempt0={sh_a0}/200 ({:.1}%) full_pass={full_pass}/600 mean_att={:.2} max_att={max_att} annealed=0 | intra_cv(yield={mean_ycv:.4} tariff={mean_tcv:.4} out={mean_ocv:.4} upkeep={mean_ucv:.4}) | spread(yield={mean_yspread:.4} tariff_rng={mean_trange:.4} decay_rng={mean_drange:.4}) | inter_cv(s0_yield={:.4} s0_tariff={:.4} s0_out={:.4} s0_decay={:.4} shock0_mag={:.4})",
                sc.as_usize(),
                100.0 * sp_a0 as f64 / 600.0,
                100.0 * sh_a0 as f64 / 200.0,
                attempts_sum as f64 / nf,
                s0y_sd / s0y_m,
                s0t_sd / s0t_m,
                s0o_sd / s0o_m,
                s0d_sd / s0d_m,
                sh0_sd / sh0_m,
            );
            sp_attempt0_rates.push(sp_a0);
            mean_yield_spreads.push(mean_yspread);
            mean_tariff_ranges.push(mean_trange);
            assert!(
                sp_a0 >= 150,
                "N={} attempt-0 ShadowPrice pass count {sp_a0}/600 must be >= 150 (25%)",
                sc.as_usize()
            );
        }

        // Verify N=6 retains matched variety relative to N=4 (no more 24% collapse).
        assert!(
            mean_tariff_ranges[2] >= 0.85 * mean_tariff_ranges[0],
            "N=6 tariff range {} dropped vs N=4 {}",
            mean_tariff_ranges[2],
            mean_tariff_ranges[0]
        );
        assert!(
            mean_yield_spreads[2] >= 0.85 * mean_yield_spreads[0],
            "N=6 yield spread {} dropped vs N=4 {}",
            mean_yield_spreads[2],
            mean_yield_spreads[0]
        );

        // Edge configs under stationary jitter must all still accept cleanly.
        for edge_cfg in [
            GeneratorConfig {
                seat_count: Some(SeatCount::Six),
                max_tariff: 0.0,
                decay_range: (0.0, 0.0),
                ..GeneratorConfig::default()
            },
            GeneratorConfig {
                seat_count: Some(SeatCount::Six),
                max_tariff: 0.35,
                ..GeneratorConfig::default()
            },
            GeneratorConfig {
                seat_count: Some(SeatCount::Six),
                decay_range: (0.99, 0.999),
                ..GeneratorConfig::default()
            },
            GeneratorConfig {
                seat_count: Some(SeatCount::Six),
                enable_secondary_converters: false,
                ..GeneratorConfig::default()
            },
        ] {
            for seed in 0u64..20 {
                assert!(
                    generate_ring_table(seed, &edge_cfg).is_ok(),
                    "edge config {edge_cfg:?} failed on seed {seed}"
                );
            }
        }
    }

    #[test]
    fn golden_v2_2_0_table_serialization_digest_matches_pinned_seed42() {
        let cfg = GeneratorConfig::default();
        let Ok((table, _)) = generate_ring_table(42, &cfg) else {
            panic!("seed 42 generates cleanly");
        };
        assert_eq!(table.generator_version, "league-gen/v2.2.0");
        let Ok(bytes) = serde_json::to_vec(&table) else {
            panic!("RingTable serializes deterministically");
        };
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        const EXPECTED_GOLDEN_SHA256: &str =
            "b8535a4484ccd278c4adfb262fc88c56245470414704d85bb3293a3ef3fe0090";
        assert_eq!(
            hex, EXPECTED_GOLDEN_SHA256,
            "v2.2.0 golden digest mismatch (bump GENERATOR_VERSION if intentional)"
        );
    }
}
