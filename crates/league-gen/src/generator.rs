//! Seeded chiral converter-ring table generator and Prioritized Level Replay (PLR) sampler.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::feasibility::{FeasibilityReport, check_feasibility};
use crate::rng::SeedStream;
use crate::schema::{
    ConverterSpec, CurrencyCode, FeasibilityError, GENERATOR_VERSION, GenError, GeneratorConfig,
    HorizonShockKind, MAX_ATTEMPTS_CAP, NUMERAIRE_CODE, RingTable, ScheduledShock, SeatCount,
    SeatSpec, is_denied_currency_code,
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

/// Per-seat stoichiometry and yield relative jitter span (`±2.5%`; provisional, calibrate in playtest 1).
const STOICH_JITTER_SPAN: f64 = 0.025;
/// Per-seat upkeep and project valuation relative jitter span (`±2.0%`; provisional, calibrate in playtest 1).
const VALUE_JITTER_SPAN: f64 = 0.020;

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

/// Number of full-dispersion candidate attempts before progressive jitter annealing begins
/// (provisional retry-schedule parameter, calibrate in playtest 1).
const FULL_DISPERSION_ATTEMPTS: u32 = 24;

/// Generates a feasible chiral converter-ring [`RingTable`] deterministically from `seed`
/// and `config`, proving all three LP feasibility invariants via [`check_feasibility`].
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

    let mut last_error =
        FeasibilityError::InvalidTable("no generation attempts executed".to_owned());

    for attempt in 0..config.max_attempts {
        let candidate = sample_candidate(seed, attempt, seat_count, config, &mut stream)?;
        match check_feasibility(&candidate, config) {
            Ok(report) => return Ok((candidate, report)),
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

fn retry_dispersion(attempt: u32) -> f64 {
    if attempt < FULL_DISPERSION_ATTEMPTS {
        1.0
    } else {
        // Halve cross-seat asymmetry every 8 failed retries so tail seeds always converge
        // cleanly within `max_attempts` while preserving full variance on early candidates.
        let halvings = ((attempt - (FULL_DISPERSION_ATTEMPTS - 8)) / 8).min(20);
        1.0 / f64::from(1u32 << halvings)
    }
}

fn sample_candidate(
    seed: u64,
    attempt: u32,
    seat_count: SeatCount,
    config: &GeneratorConfig,
    stream: &mut SeedStream,
) -> Result<RingTable, GenError> {
    let n = seat_count.as_usize();

    // Sample `2 * N` distinct valid 3-char commodity currency codes (`numeraire` is fixed `CRED`).
    let codes = sample_unique_codes(stream, 2 * n)?;

    let disp = retry_dispersion(attempt);
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

    let mut seats = Vec::with_capacity(n);
    for i in 0..n {
        let stoich_span = STOICH_JITTER_SPAN * disp;
        let value_span = VALUE_JITTER_SPAN * disp;
        let jitter = |s: &mut SeedStream, span: f64| 1.0 + s.range_f64(-span, span);
        let raw_yield_per_turn = round3(base_yield * jitter(stream, stoich_span));
        let in_a = round3(base_in_a * jitter(stream, stoich_span));
        let in_b = round3(base_in_b * jitter(stream, stoich_span));
        let out_amt = round3(base_out * jitter(stream, stoich_span));
        let upkeep_amount = round3(base_upkeep * jitter(stream, value_span));
        let refined_project_value = round3(base_project_val * jitter(stream, value_span));

        let (raw_export_tariff, refined_export_tariff, perishable_decay_rate) = if disp >= 1.0 {
            (
                round4(stream.range_f64(0.0, config.max_tariff)),
                round4(stream.range_f64(0.0, config.max_tariff)),
                round4(stream.range_f64(config.decay_range.0, config.decay_range.1)),
            )
        } else {
            let tariff_half = 0.5 * config.max_tariff * disp;
            let decay_half = 0.5 * (config.decay_range.1 - config.decay_range.0) * disp;
            let t_lo = (base_tariff - tariff_half).max(0.0);
            let t_hi = (base_tariff + tariff_half).min(config.max_tariff);
            let d_lo = (base_decay - decay_half).max(config.decay_range.0);
            let d_hi = (base_decay + decay_half).min(config.decay_range.1);
            (
                round4(stream.range_f64(t_lo, t_hi)),
                round4(stream.range_f64(t_lo, t_hi)),
                round4(stream.range_f64(d_lo, d_hi)),
            )
        };

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

    let shock_scale = if attempt < 48 {
        1.0
    } else {
        retry_dispersion(attempt - 24).max(0.01)
    };
    let shocks = sample_shocks(stream, n, shock_scale);

    Ok(RingTable {
        generator_version: GENERATOR_VERSION.to_owned(),
        config: config.clone(),
        seed,
        attempt,
        seat_count,
        numeraire_code: NUMERAIRE_CODE.to_owned(),
        season_turn_window: SEASON_TURN_WINDOW,
        seats,
        shocks,
    })
}

fn sample_unique_codes(
    stream: &mut SeedStream,
    count: usize,
) -> Result<Vec<CurrencyCode>, GenError> {
    let mut seen = HashSet::with_capacity(count);
    let mut out = Vec::with_capacity(count);
    let mut guard = 0usize;
    while out.len() < count && guard < 10_000 {
        guard += 1;
        let b0 = b'A' + (stream.next_u32_bounded(26) as u8);
        let b1 = b'A' + (stream.next_u32_bounded(26) as u8);
        let b2 = b'A' + (stream.next_u32_bounded(26) as u8);
        let bytes = [b0, b1, b2];
        if !is_denied_currency_code(bytes)
            && let Ok(code) = CurrencyCode::from_bytes(bytes)
            && seen.insert(code)
        {
            out.push(code);
        }
    }
    if out.len() < count {
        return Err(GenError::InvalidConfig(
            "failed to draw unique valid currency codes".to_owned(),
        ));
    }
    Ok(out)
}

fn sample_shocks(stream: &mut SeedStream, n: usize, scale: f64) -> Vec<ScheduledShock> {
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
        let magnitude = round3(raw_mag * scale).max(0.001);
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

    use super::*;
    use crate::feasibility::{compute_shadow_price_seat_evs, compute_shapley_seat_evs};
    use crate::schema::EvMethod;

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
        ] {
            assert!(
                json.contains(expected_key),
                "serialized RingTable missing {expected_key}"
            );
        }
    }

    #[test]
    fn generated_codes_never_collide_with_iso_or_crypto_denylist() {
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
                assert!(
                    !is_denied_currency_code(seat.raw_code.as_bytes()),
                    "denied raw code {} on seed {seed}",
                    seat.raw_code
                );
                assert!(
                    !is_denied_currency_code(seat.refined_code.as_bytes()),
                    "denied refined code {} on seed {seed}",
                    seat.refined_code
                );
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
    }
}
