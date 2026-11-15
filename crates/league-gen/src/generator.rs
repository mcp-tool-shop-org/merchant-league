//! Seeded chiral converter-ring table generator and Prioritized Level Replay (PLR) sampler.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::feasibility::{FeasibilityReport, check_feasibility};
use crate::rng::SeedStream;
use crate::schema::{
    ConverterSpec, CurrencyCode, FeasibilityError, GenError, GeneratorConfig, HorizonShockKind,
    RingTable, ScheduledShock, SeatCount, SeatSpec,
};

/// Generates a feasible chiral converter-ring [`RingTable`] deterministically from `seed`
/// and `config`, proving all three LP feasibility invariants via [`check_feasibility`].
pub fn generate_ring_table(
    seed: u64,
    config: &GeneratorConfig,
) -> Result<(RingTable, FeasibilityReport), GenError> {
    validate_generator_config(config)?;
    let mut stream = SeedStream::new(seed, b"ring-table");
    let mut last_error =
        FeasibilityError::InvalidTable("no generation attempts executed".to_owned());

    for attempt in 0..config.max_attempts {
        let candidate = sample_candidate(seed, attempt, config, &mut stream)?;
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
    if config.max_attempts == 0 {
        return Err(GenError::InvalidConfig(
            "max_attempts must be at least 1".to_owned(),
        ));
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

fn sample_candidate(
    seed: u64,
    attempt: u32,
    config: &GeneratorConfig,
    stream: &mut SeedStream,
) -> Result<RingTable, GenError> {
    let seat_count = match config.seat_count {
        Some(sc) => sc,
        None => {
            let idx = stream.next_u32_bounded(SeatCount::ALL.len() as u32) as usize;
            SeatCount::ALL[idx]
        }
    };
    let n = seat_count.as_usize();

    // Sample `2 * N + 1` unique valid 3-char ledger currency codes.
    let codes = sample_unique_codes(stream, 2 * n + 1)?;
    let numeraire_code = codes[0];

    let base_yield = stream.range_f64(9.0, 12.0);
    let base_in_a = stream.range_f64(1.85, 2.25);
    let base_in_b = stream.range_f64(1.85, 2.25);
    let base_out = stream.range_f64(3.1, 3.9);
    let base_upkeep = stream.range_f64(1.6, 2.2);
    let base_project_val = stream.range_f64(1.6, 2.8);
    let attach_secondary = config.enable_secondary_converters
        && n >= 5
        && (seat_count == SeatCount::Six || stream.next_u32_bounded(2) == 1);

    let mut seats = Vec::with_capacity(n);
    for i in 0..n {
        let jitter = |s: &mut SeedStream, span: f64| 1.0 + s.range_f64(-span, span);
        let raw_yield_per_turn = round3(base_yield * jitter(stream, 0.025));
        let in_a = round3(base_in_a * jitter(stream, 0.025));
        let in_b = round3(base_in_b * jitter(stream, 0.025));
        let out_amt = round3(base_out * jitter(stream, 0.025));
        let upkeep_amount = round3(base_upkeep * jitter(stream, 0.020));
        let refined_project_value = round3(base_project_val * jitter(stream, 0.020));
        let raw_export_tariff = round4(stream.range_f64(0.0, config.max_tariff));
        let refined_export_tariff = round4(stream.range_f64(0.0, config.max_tariff));
        let perishable_decay_rate =
            round4(stream.range_f64(config.decay_range.0, config.decay_range.1));

        let next_1 = ((i + 1) % n) as u8;
        let next_2 = ((i + 2) % n) as u8;
        let next_3 = ((i + 3) % n) as u8;

        let mut converters = vec![ConverterSpec {
            owner: i as u8,
            inputs: vec![(next_1, in_a), (next_2, in_b)],
            output_good: i as u8,
            output_amount: out_amt,
            max_runs_per_turn: 4.0,
            is_secondary: false,
        }];

        if attach_secondary {
            converters.push(ConverterSpec {
                owner: i as u8,
                inputs: vec![(next_2, round3(in_a * 1.12)), (next_3, round3(in_b * 1.12))],
                output_good: i as u8,
                output_amount: round3(out_amt * 0.94),
                max_runs_per_turn: 2.0,
                is_secondary: true,
            });
        }

        seats.push(SeatSpec {
            seat: i as u8,
            raw_good: i as u8,
            raw_code: codes[1 + 2 * i],
            raw_yield_per_turn,
            refined_good: i as u8,
            refined_code: codes[2 + 2 * i],
            refined_project_value,
            upkeep_good: ((i + n - 1) % n) as u8,
            upkeep_amount,
            raw_export_tariff,
            refined_export_tariff,
            perishable_decay_rate,
            converters,
        });
    }

    let shocks = sample_shocks(stream, n);

    Ok(RingTable {
        seed,
        attempt,
        seat_count,
        numeraire_code,
        season_turn_window: (24, 28),
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
        if let Ok(code) = CurrencyCode::from_bytes([b0, b1, b2])
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

fn sample_shocks(stream: &mut SeedStream, n: usize) -> Vec<ScheduledShock> {
    let mut kinds = [
        HorizonShockKind::SupplyFreeze,
        HorizonShockKind::TariffHoliday,
        HorizonShockKind::DemandSurge,
        HorizonShockKind::SpoilageSpike,
    ];
    stream.shuffle(&mut kinds);

    let seat_order = stream.sample_distinct_indices(n, kinds.len());
    // Distribute 4 shocks across distinct 3-turn windows in turns 2..=22.
    let windows = [2u8, 7u8, 12u8, 18u8];
    let mut shocks = Vec::with_capacity(kinds.len());
    for (idx, (&kind, &seat)) in kinds.iter().zip(seat_order.iter()).enumerate() {
        let hint_turn = windows[idx] + (stream.next_u32_bounded(3) as u8);
        let magnitude = match kind {
            HorizonShockKind::SupplyFreeze => round3(stream.range_f64(0.18, 0.35)),
            HorizonShockKind::TariffHoliday => round3(stream.range_f64(0.50, 1.00)),
            HorizonShockKind::DemandSurge => round3(stream.range_f64(0.15, 0.30)),
            HorizonShockKind::SpoilageSpike => round3(stream.range_f64(0.15, 0.30)),
        };
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
            && let Some((worst_idx, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.score.total_cmp(&b.score))
        {
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
        for (rank, &idx) in order.iter().enumerate() {
            let w = (1.0 / ((rank + 1) as f64)).powf(1.0 / self.temperature);
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
    use super::*;

    #[test]
    fn generates_feasible_tables_for_all_seat_counts() {
        for sc in SeatCount::ALL {
            let cfg = GeneratorConfig {
                seat_count: Some(sc),
                ..GeneratorConfig::default()
            };
            for seed in [0u64, 1, 26, 20_261_115] {
                let Ok((table, report)) = generate_ring_table(seed, &cfg) else {
                    panic!("generation failed for seat_count={sc:?}, seed={seed}");
                };
                assert_eq!(table.seat_count, sc);
                assert_eq!(table.seats.len(), sc.as_usize());
                assert!(report.max_2seat_upkeep_coverage < 1e-9);
                assert!(report.surplus_to_upkeep_ratio >= cfg.min_surplus_ratio);
                assert!(report.max_relative_ev_deviation <= cfg.ev_tolerance + 1e-9);
            }
        }
    }

    #[test]
    fn generator_rejects_invalid_config_and_impossible_tolerance() {
        let bad_attempts = GeneratorConfig {
            max_attempts: 0,
            ..GeneratorConfig::default()
        };
        assert!(matches!(
            generate_ring_table(1, &bad_attempts),
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
    fn plr_sampler_prioritizes_higher_scores_and_evicts_lowest_on_capacity() {
        let mut plr = PlrSampler::new(123, 2, 0.2, 0.5, 0.9);
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
}
