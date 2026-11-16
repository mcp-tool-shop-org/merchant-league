//! Seeded chiral converter-ring table generator and Prioritized Level Replay (PLR) sampler.

use serde::{Deserialize, Serialize};

use crate::feasibility::{FeasibilityReport, check_feasibility};
use crate::rng::SeedStream;
use crate::schema::{
    ALLOWED_COMMODITY_GLOSSARY, ConverterSpec, CurrencyCode, FeasibilityError, GENERATOR_VERSION,
    GameTableRecord, GenError, GeneratorConfig, HorizonShockKind, MAX_ATTEMPTS_CAP, NUMERAIRE_CODE,
    RingTable, ScheduledShock, SeatCount, SeatSpec, SeedPool, TRAIN_SEED_MAX, TRAIN_SEED_MIN,
    TablePurpose, TableSetupOptions, default_mandate_deck_for_table, default_projects_for_seats,
    seed_pool,
};

/// Stochastic season close turn window `(24, 28)` from `docs/design-v2.md` §6.2.
const SEASON_TURN_WINDOW: (u8, u8) = (24, 28);

/// Base per-turn raw extraction yield range (provisional, calibrate in playtest 1).
const BASE_YIELD_RANGE: (f64, f64) = (9.0, 12.0);
/// Base total raw input stoichiometry sum per unscaled primary batch `in_a + in_b` (provisional,
/// calibrate in playtest 1; replaces the former shared `BASE_IN_A_RANGE`/`BASE_IN_B_RANGE` draw
/// alongside per-seat recipe splits and batch scales).
const BASE_STOICH_SUM_RANGE: (f64, f64) = (3.70, 4.30);
/// Per-seat primary input `R_{i+1}` share `w_prim = in_a / (in_a + in_b)` for `4`-seat rings
/// (`2` workshops per seat), drawn from overlapping stratified per-seat ranges and shuffled across
/// seats so all `5` live shadow-LP trade options compete smoothly around top-3 rankings (Route b +
/// G1 item 2; calibrated, provisional pending playtest 1).
const PRIMARY_INPUT_SHARE_RANGE_4P: (f64, f64) = (0.27, 0.73);
/// Per-seat symmetric clearing secondary input share for `4`-seat rings (`w in (0.24, 0.40)` keeps
/// even-`N` `det(A_sec)` well-conditioned for `ShadowPrice` attempt-0 acceptance without `0.20`
/// extreme stranding in `Shapley` coalitions; calibrated, provisional pending playtest 1).
const SECONDARY_INPUT_SHARE_RANGE_4P: (f64, f64) = (0.24, 0.40);
/// Per-seat primary `(R_{i+1}, R_{i+2})` and 3-input intermediate `S1(R_{i+1}, R_{i+2}, R_{i+3})`
/// outer-input share range for `5`-seat rings (`3` workshops per seat -> `8` positive options
/// across `6` distinct foreign supply labels plus `2` own capacity rents). Every `5p` converter
/// includes `R_{(i+2) mod 5}`, guaranteeing that any `4`-seat bloc covers at most `2` of its `4`
/// members' upkeep (`A-1`, Route b + G1 item 2; measured Shapley `±3%` agreement on ShadowPrice
/// tables is ~73% at `N = 6` and ~18–24% at `N = 4, 5`, `B-2`; provisional pending playtest 1).
const MULTI_WORKSHOP_SHARE_RANGE_5P: (f64, f64) = (0.365, 0.635);
/// Fixed required middle-input share on `R_{(i+2) mod 5}` inside `5`-seat intermediate `S1`
/// workshops `(R_{i+1}, R_{i+2}, R_{i+3})` (`0.16`, leaving `0.84` split across `(R_{i+1}, R_{i+3})`
/// via [`MULTI_WORKSHOP_SHARE_RANGE_5P`]), blocking converter owner `(k + 2) mod 5` whenever seat
/// `(k + 4) mod 5` is excluded from a `4`-seat bloc (`A-1`; provisional pending playtest 1).
const SECONDARY_5P_MID_INPUT_SHARE: f64 = 0.16;
/// Per-seat centered clearing `S2(R_{i+2}, R_{i+4})` input share for `5`-seat rings (`(0.44, 0.56)`
/// keeps residual `S2` raw demand balanced across seats without assuming tight ShadowPrice/Shapley
/// equivalence, `B-2`; calibrated, provisional pending playtest 1).
const SECONDARY_CLEARING_SHARE_RANGE_5P: (f64, f64) = (0.44, 0.56);
/// Per-seat primary and intermediate secondary input share range for `6`-seat rings (`3` workshops
/// per seat -> `8` distinct live options per seat; calibrated alongside `S1(1, 3)` and `S2(2, 4)`
/// for D1 variety with moderate measured ShadowPrice/Shapley agreement at `N = 6`, `B-2`;
/// provisional pending playtest 1).
const MULTI_WORKSHOP_SHARE_RANGE_6P: (f64, f64) = (0.362, 0.638);
/// Per-seat clearing `S2` input share for `6`-seat rings (`S2(2, 4)` has ring step difference `2`,
/// which is non-singular even at `w = 0.50` on `N=6`, letting all `6` supply options compete
/// symmetrically around `0.50`; calibrated, provisional pending playtest 1).
const SECONDARY_CLEARING_SHARE_RANGE_6P: (f64, f64) = (0.362, 0.638);
/// Per-seat primary input `R_{i+1}` share used only when secondary converters are explicitly
/// disabled, keeping `in_a < in_b` so single-converter even-`N` chiral input matrices remain
/// well-conditioned under shadow pricing (provisional, calibrate in playtest 1).
const SINGLE_CONVERTER_INPUT_SHARE_RANGE: (f64, f64) = (0.25, 0.44);
/// Per-seat base recipe batch-scale band multiplying `(inputs, output)` jointly (and inversely
/// scaling binding primary/secondary run capacity) so each seat's base recipe differs structurally
/// in input mix, batch size, and capacity while preserving shadow-LP equity balance
/// (calibrated, provisional pending playtest 1).
const RECIPE_BATCH_SCALE_RANGE: (f64, f64) = (0.985, 1.015);
/// Overlap fraction applied when stratifying per-seat base recipe ranges across `N` seats (so
/// per-seat ranges overlap cleanly without filtering low-variety tables; provisional, calibrate in
/// playtest 1).
const PER_SEAT_STRATA_OVERLAP_FRAC: f64 = 0.25;
/// Base primary converter output `M_i` batch range (provisional, calibrate in playtest 1).
const BASE_OUT_RANGE: (f64, f64) = (3.1, 3.9);
/// Base mandatory upkeep quantity range (provisional, calibrate in playtest 1).
const BASE_UPKEEP_RANGE: (f64, f64) = (1.6, 2.2);
/// Base Syndicate Project fundamental valuation range for `M_i` (provisional, calibrate in playtest 1).
const BASE_PROJECT_VAL_RANGE: (f64, f64) = (1.6, 2.8);

/// Fine per-converter efficiency jitter span (`±0.25%`) layered on top of per-seat recipe input
/// splits and batch scales (`RECIPE_BATCH_SCALE_RANGE`) so multi-tier shadow-LP and Shapley EV
/// checks stay well within attempt budgets while keeping per-seat yields at [`STOICH_JITTER_SPAN`]
/// (calibrated, provisional pending playtest 1).
const CONVERTER_RECIPE_JITTER_SPAN: f64 = 0.0025;
/// Stationary G1 per-seat raw extraction yield relative jitter span (`±0.875%` from attempt 0,
/// zero annealing; calibrated to the G1 variety floor: `yield intra_cv >= 0.0038` and `tariff
/// intra_cv >= 0.20` at every `N = 4, 5, 6` with across-`N` spread `(max - min) / min <= 10%`;
/// provisional pending playtest 1).
const STOICH_JITTER_SPAN: f64 = 0.00875;
/// Stationary G1 per-seat upkeep and project valuation relative jitter span (`±0.875%` from
/// attempt 0; calibrated to the G1 variety floor and provisional pending playtest 1).
const VALUE_JITTER_SPAN: f64 = 0.00875;
/// Stationary G1 base half-band fraction of `max_tariff` around `base_tariff` (capped at
/// [`MAX_TARIFF_JITTER_ABS`] and scaled by [`tariff_jitter_seat_scale`]; calibrated to the G1
/// variety floor and provisional pending playtest 1).
const TARIFF_JITTER_HALF_FRAC: f64 = 0.16;
/// Maximum base absolute half-band (`±0.036`) for per-seat baseline tariff jitter before
/// applying [`tariff_jitter_seat_scale`] (calibrated to the G1 variety floor and provisional
/// pending playtest 1).
const MAX_TARIFF_JITTER_ABS: f64 = 0.036;
/// Stationary G1 half-band fraction of `(decay_hi - decay_lo)` around `base_decay` (capped at
/// `0.50`; calibrated to the G1 variety floor and provisional pending playtest 1).
const DECAY_JITTER_HALF_FRAC: f64 = 0.50;

/// Per-`N` tariff jitter half-band normaliser applied to
/// `(config.max_tariff * TARIFF_JITTER_HALF_FRAC).min(MAX_TARIFF_JITTER_ABS)`.
///
/// Calibrated directly on **raw attempt-0 candidates** (`B-3`, rather than post-filter survivors,
/// placing `N = 5` at `0.96` near `1.0`) to compensate for population `std / mean`
/// (`sqrt((N - 1) / N)`) and `[0, max_tariff]` boundary clipping so both raw candidates and
/// accepted tables satisfy `tariff intra_cv >= 0.20` at every `N` with across-`N` spread
/// `(max - min) / min <= 10%` (provisional pending playtest 1).
const fn tariff_jitter_seat_scale(seat_count: SeatCount) -> f64 {
    match seat_count {
        SeatCount::Four => 1.14,
        SeatCount::Five => 0.96,
        SeatCount::Six => 0.95,
    }
}

/// Uncapacitated primary converter capacity cap per turn used when secondary converters are
/// disabled (`4.0` batch runs; provisional, calibrate in playtest 1).
const PRIMARY_UNCAPACITATED_MAX_RUNS_PER_TURN: f64 = 4.0;
/// Target share of baseline raw extraction processed by each `4`-seat binding primary workshop at
/// nominal `1.10x` export tariff (`0.54`, leaving `~0.46` for the clearing secondary cross-converter
/// `(R_{i+2}, R_{i+3})`; Route b, provisional pending playtest 1).
const PRIMARY_BINDING_YIELD_FRAC_4P: f64 = 0.54;
/// Target share of baseline raw extraction processed by each `5`-seat binding primary workshop
/// (`0.33`, Tier 1 of the 3-workshop guild ladder; Route b + `A-1`, provisional pending playtest 1).
const PRIMARY_BINDING_YIELD_FRAC_5P: f64 = 0.33;
/// Target share of baseline raw extraction processed by each `5`-seat binding 3-input intermediate
/// workshop `S1(R_{i+1}, R_{i+2}, R_{i+3})` (`0.37`, leaving `~0.30` residual for clearing `S2(2, 4)`;
/// Route b + `A-1`, provisional pending playtest 1).
const SECONDARY_TIER1_BINDING_YIELD_FRAC_5P: f64 = 0.37;
/// Target share of baseline raw extraction processed by each `6`-seat binding primary workshop
/// (`0.35`, Tier 1 of the 3-workshop guild ladder; Route b, provisional pending playtest 1).
const PRIMARY_BINDING_YIELD_FRAC_6P: f64 = 0.35;
/// Target share of baseline raw extraction processed by each `6`-seat binding intermediate
/// secondary workshop (`0.33`, leaving `~0.32` residual for symmetric `S2(2, 4)` so all `6` supply
/// options compete cleanly near `~0.16..0.175`; Route b, provisional pending playtest 1).
const SECONDARY_TIER1_BINDING_YIELD_FRAC_6P: f64 = 0.33;
/// Nominal raw tariff multiplier used to size binding workshop run caps relative to table yield
/// (`1.10`; provisional, calibrate in playtest 1).
const NOMINAL_TARIFF_MULT: f64 = 1.10;
/// Input/output stoichiometry multipliers `(in_mult, out_mult)` for `4`-seat clearing secondary
/// cross-converters (`1.17x` in / `0.84x` out so primary capacity rent `OwnP` competes directly at
/// the median of `4p` supply options; calibrated, provisional pending playtest 1).
const SECONDARY_4P_MULTS: (f64, f64) = (1.17, 0.84);
/// Input/output stoichiometry multipliers `(in_mult, out_mult)` for `5`-/`6`-seat binding
/// intermediate secondary workshops (`1.02x` in / `0.98x` out; calibrated, provisional pending
/// playtest 1).
const SECONDARY_TIER1_MULTS: (f64, f64) = (1.02, 0.98);
/// Input/output stoichiometry multipliers for `5`-/`6`-seat clearing secondary cross-converters
/// (`1.045x` in / `0.955x` out so `P` and `S1` capacity duals remain strictly positive above
/// recipe jitter; measured ShadowPrice vs Shapley `±3%` agreement on accepted tables is ~73% at
/// `N = 6` and ~18–24% at `N = 4, 5`, `B-2`; calibrated, provisional pending playtest 1).
const SECONDARY_TIER2_MULTS: (f64, f64) = (1.045, 0.955);
/// Secondary clearing converter capacity cap per turn (`4.5` batch runs so residual raw supply
/// clears cleanly without artificial salvage; provisional pending playtest 1).
const SECONDARY_CLEARING_MAX_RUNS_PER_TURN: f64 = 4.5;

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

    let mut last_error =
        FeasibilityError::InvalidTable("no generation attempts executed".to_owned());

    for attempt in 0..config.max_attempts {
        let candidate = sample_candidate(seed, attempt, seat_count, config, &mut stream)?;
        match check_feasibility(&candidate) {
            Ok(report) => return Ok((candidate, report)),
            Err(err) => last_error = err,
        }
    }

    Err(GenError::NoFeasibleTableFound {
        attempts: config.max_attempts,
        last_error,
    })
}

/// Generates a feasible [`RingTable`] for `purpose` with `training_logged = false`, checking
/// [`seed_pool`] first and returning [`GenError::SeedPoolMismatch`] (`SEED_POOL_MISMATCH`) when
/// `seed` does not belong to `purpose`'s pool (including [`SeedPool::Unassigned`]), then calling
/// [`generate_ring_table`] unchanged and emitting a checked [`GameTableRecord`].
pub fn generate_ring_table_for_purpose(
    seed: u64,
    purpose: TablePurpose,
    config: &GeneratorConfig,
) -> Result<(RingTable, FeasibilityReport, GameTableRecord), GenError> {
    generate_ring_table_for_setup(
        seed,
        TableSetupOptions {
            purpose,
            training_logged: false,
        },
        config,
    )
}

/// Checked game-table setup entry point enforcing both [`seed_pool`] (`SEED_POOL_MISMATCH`) and
/// the `training_logged` policy (`TRAINING_LOGGED_MISMATCH` unless `options.purpose == TablePurpose::Train`),
/// then delegating to [`generate_ring_table`] unchanged and returning the generated [`RingTable`],
/// [`FeasibilityReport`], and [`GameTableRecord`].
pub fn generate_ring_table_for_setup(
    seed: u64,
    options: TableSetupOptions,
    config: &GeneratorConfig,
) -> Result<(RingTable, FeasibilityReport, GameTableRecord), GenError> {
    let actual_pool = seed_pool(seed);
    if actual_pool != options.purpose.expected_pool() {
        return Err(GenError::seed_pool_mismatch(
            seed,
            actual_pool,
            options.purpose,
        ));
    }
    if options.training_logged && options.purpose != TablePurpose::Train {
        return Err(GenError::training_logged_mismatch(
            seed,
            actual_pool,
            options.purpose,
        ));
    }
    let (table, report) = generate_ring_table(seed, config)?;
    let record = GameTableRecord::new_checked(
        seed,
        table.seat_count,
        actual_pool,
        table.generator_version.clone(),
        options.training_logged,
    );
    Ok((table, report, record))
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

struct WorkshopTierPlan {
    offsets: (usize, usize),
    mid_input: Option<(usize, f64)>,
    is_secondary: bool,
    in_mult: f64,
    out_mult: f64,
    binding_yield_frac: Option<f64>,
    uncapacitated_cap: f64,
    weights: Vec<f64>,
    scales: Vec<f64>,
}

fn build_workshop_tiers(
    stream: &mut SeedStream,
    seat_count: SeatCount,
    attach_secondary: bool,
) -> Vec<WorkshopTierPlan> {
    let n = seat_count.as_usize();
    let mut tier = |offsets: (usize, usize),
                    mid_input: Option<(usize, f64)>,
                    is_secondary: bool,
                    mults: (f64, f64),
                    binding_yield_frac: Option<f64>,
                    uncapacitated_cap: f64,
                    share_range: (f64, f64)| WorkshopTierPlan {
        offsets,
        mid_input,
        is_secondary,
        in_mult: mults.0,
        out_mult: mults.1,
        binding_yield_frac,
        uncapacitated_cap,
        weights: sample_per_seat_stratified(stream, n, share_range, PER_SEAT_STRATA_OVERLAP_FRAC),
        scales: sample_per_seat_stratified(
            stream,
            n,
            RECIPE_BATCH_SCALE_RANGE,
            PER_SEAT_STRATA_OVERLAP_FRAC,
        ),
    };
    if !attach_secondary {
        return vec![tier(
            (1, 2),
            None,
            false,
            (1.0, 1.0),
            None,
            PRIMARY_UNCAPACITATED_MAX_RUNS_PER_TURN,
            SINGLE_CONVERTER_INPUT_SHARE_RANGE,
        )];
    }

    match seat_count {
        SeatCount::Four => vec![
            tier(
                (1, 2),
                None,
                false,
                (1.0, 1.0),
                Some(PRIMARY_BINDING_YIELD_FRAC_4P),
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                PRIMARY_INPUT_SHARE_RANGE_4P,
            ),
            tier(
                (2, 3),
                None,
                true,
                SECONDARY_4P_MULTS,
                None,
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                SECONDARY_INPUT_SHARE_RANGE_4P,
            ),
        ],
        SeatCount::Five => vec![
            tier(
                (1, 2),
                None,
                false,
                (1.0, 1.0),
                Some(PRIMARY_BINDING_YIELD_FRAC_5P),
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                MULTI_WORKSHOP_SHARE_RANGE_5P,
            ),
            tier(
                (1, 3),
                Some((2, SECONDARY_5P_MID_INPUT_SHARE)),
                true,
                SECONDARY_TIER1_MULTS,
                Some(SECONDARY_TIER1_BINDING_YIELD_FRAC_5P),
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                MULTI_WORKSHOP_SHARE_RANGE_5P,
            ),
            tier(
                (2, 4),
                None,
                true,
                SECONDARY_TIER2_MULTS,
                None,
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                SECONDARY_CLEARING_SHARE_RANGE_5P,
            ),
        ],
        SeatCount::Six => vec![
            tier(
                (1, 2),
                None,
                false,
                (1.0, 1.0),
                Some(PRIMARY_BINDING_YIELD_FRAC_6P),
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                MULTI_WORKSHOP_SHARE_RANGE_6P,
            ),
            tier(
                (1, 3),
                None,
                true,
                SECONDARY_TIER1_MULTS,
                Some(SECONDARY_TIER1_BINDING_YIELD_FRAC_6P),
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                MULTI_WORKSHOP_SHARE_RANGE_6P,
            ),
            tier(
                (2, 4),
                None,
                true,
                SECONDARY_TIER2_MULTS,
                None,
                SECONDARY_CLEARING_MAX_RUNS_PER_TURN,
                SECONDARY_CLEARING_SHARE_RANGE_6P,
            ),
        ],
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

    // Sample `2 * N` distinct curated game-word commodity currency codes (`numeraire` is fixed `CRED`).
    let codes = sample_unique_codes(stream, 2 * n)?;

    let base_yield = stream.range_f64(BASE_YIELD_RANGE.0, BASE_YIELD_RANGE.1);
    let base_stoich_sum = stream.range_f64(BASE_STOICH_SUM_RANGE.0, BASE_STOICH_SUM_RANGE.1);
    let base_out = stream.range_f64(BASE_OUT_RANGE.0, BASE_OUT_RANGE.1);
    let base_upkeep = stream.range_f64(BASE_UPKEEP_RANGE.0, BASE_UPKEEP_RANGE.1);
    let base_project_val = stream.range_f64(BASE_PROJECT_VAL_RANGE.0, BASE_PROJECT_VAL_RANGE.1);
    let base_tariff = stream.range_f64(0.0, config.max_tariff);
    let base_decay = stream.range_f64(config.decay_range.0, config.decay_range.1);

    // Under the 2026-11-15 rulings (Route b + A-1), enabled tables attach cyclic tiered guild
    // workshops with per-seat overlapping base recipe splits, batch scales, and binding caps:
    // - 4p: binding primary `(i+1, i+2)` + clearing secondary `(i+2, i+3)` (`5` positive options)
    // - 5p: binding `P (i+1, i+2)`, binding 3-input `S1 (i+1, i+2, i+3)`, clearing `S2 (i+2, i+4)`
    //       (every converter requires `R_{i+2}`, keeping 4-seat blocs at `<= 2/4` upkeep)
    // - 6p: binding `P (i+1, i+2)`, binding `S1 (i+1, i+3)`, clearing `S2 (i+2, i+4)`
    // Zero 1-/2-seat autarky and `<= 1/3` 3-seat upkeep hold structurally on every `N` because
    // all inputs are foreign and never match `{i-1, i+1}`.
    let tiers = build_workshop_tiers(stream, seat_count, config.enable_secondary_converters);

    let tariff_half = (config.max_tariff * TARIFF_JITTER_HALF_FRAC).min(MAX_TARIFF_JITTER_ABS)
        * tariff_jitter_seat_scale(seat_count);
    let decay_half = (config.decay_range.1 - config.decay_range.0) * DECAY_JITTER_HALF_FRAC;
    let t_lo = (base_tariff - tariff_half).max(0.0);
    let t_hi = (base_tariff + tariff_half).min(config.max_tariff);
    let d_lo = (base_decay - decay_half).max(config.decay_range.0);
    let d_hi = (base_decay + decay_half).min(config.decay_range.1);

    let mut seats = Vec::with_capacity(n);
    for i in 0..n {
        let jitter = |s: &mut SeedStream, span: f64| 1.0 + s.range_f64(-span, span);
        let raw_yield_per_turn = round3(base_yield * jitter(stream, STOICH_JITTER_SPAN));
        let upkeep_amount = round3(base_upkeep * jitter(stream, VALUE_JITTER_SPAN));
        let refined_project_value = round3(base_project_val * jitter(stream, VALUE_JITTER_SPAN));

        let raw_export_tariff = round4(stream.range_f64(t_lo, t_hi));
        let refined_export_tariff = round4(stream.range_f64(t_lo, t_hi));
        let perishable_decay_rate = round4(stream.range_f64(d_lo, d_hi));

        let mut converters = Vec::with_capacity(tiers.len());
        for tier in &tiers {
            let scale = tier.scales[i];
            let w = tier.weights[i];
            let g0 = ((i + tier.offsets.0) % n) as u8;
            let g1 = ((i + tier.offsets.1) % n) as u8;
            let inputs = match tier.mid_input {
                None => {
                    let in_a = round3(
                        base_stoich_sum
                            * scale
                            * tier.in_mult
                            * w
                            * jitter(stream, CONVERTER_RECIPE_JITTER_SPAN),
                    );
                    let in_b = round3(
                        base_stoich_sum
                            * scale
                            * tier.in_mult
                            * (1.0 - w)
                            * jitter(stream, CONVERTER_RECIPE_JITTER_SPAN),
                    );
                    vec![(g0, in_a), (g1, in_b)]
                }
                Some((mid_off, w_mid)) => {
                    let rem = 1.0 - w_mid;
                    let in_a = round3(
                        base_stoich_sum
                            * scale
                            * tier.in_mult
                            * (rem * w)
                            * jitter(stream, CONVERTER_RECIPE_JITTER_SPAN),
                    );
                    let in_mid = round3(
                        base_stoich_sum
                            * scale
                            * tier.in_mult
                            * w_mid
                            * jitter(stream, CONVERTER_RECIPE_JITTER_SPAN),
                    );
                    let in_b = round3(
                        base_stoich_sum
                            * scale
                            * tier.in_mult
                            * (rem * (1.0 - w))
                            * jitter(stream, CONVERTER_RECIPE_JITTER_SPAN),
                    );
                    let g_mid = ((i + mid_off) % n) as u8;
                    vec![(g0, in_a), (g_mid, in_mid), (g1, in_b)]
                }
            };
            let output_amount = round3(
                base_out * scale * tier.out_mult * jitter(stream, CONVERTER_RECIPE_JITTER_SPAN),
            );
            let max_runs_per_turn = match tier.binding_yield_frac {
                Some(frac) => {
                    let base_runs = (frac * base_yield)
                        / (NOMINAL_TARIFF_MULT * base_stoich_sum * tier.in_mult);
                    round3((base_runs / scale).max(0.25))
                }
                None => tier.uncapacitated_cap,
            };
            converters.push(ConverterSpec {
                owner: i as u8,
                inputs,
                output_good: i as u8,
                output_amount,
                max_runs_per_turn,
                is_secondary: tier.is_secondary,
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

fn sample_per_seat_stratified(
    stream: &mut SeedStream,
    n: usize,
    range: (f64, f64),
    overlap_frac: f64,
) -> Vec<f64> {
    let span = range.1 - range.0;
    let step = span / (n as f64);
    let pad = step * overlap_frac;
    let mut values = Vec::with_capacity(n);
    for r in 0..n {
        let sub_lo = (range.0 + (r as f64) * step - pad).max(range.0);
        let sub_hi = (range.0 + ((r + 1) as f64) * step + pad).min(range.1);
        values.push(stream.range_f64(sub_lo, sub_hi));
    }
    stream.shuffle(&mut values);
    values
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

/// Validated `train`-pool entry tracked inside a [`PlrSampler`] level buffer.
///
/// Fields are private and deserialization validates [`seed_pool`] so no seed outside
/// [`SeedPool::Train`] (`10_000..=999_999`) can ever be constructed or loaded into a buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawPlrEntry")]
pub struct PlrEntry {
    seed: u64,
    score: f64,
    last_seen_step: u64,
}

#[derive(Deserialize)]
struct RawPlrEntry {
    seed: u64,
    score: f64,
    last_seen_step: u64,
}

impl TryFrom<RawPlrEntry> for PlrEntry {
    type Error = GenError;

    fn try_from(raw: RawPlrEntry) -> Result<Self, Self::Error> {
        Self::new(raw.seed, raw.score, raw.last_seen_step)
    }
}

impl PlrEntry {
    /// Constructs a validated [`PlrEntry`] if `seed` belongs to [`SeedPool::Train`], or returns
    /// [`GenError::SeedPoolMismatch`] (`SEED_POOL_MISMATCH`) otherwise.
    pub fn new(seed: u64, score: f64, last_seen_step: u64) -> Result<Self, GenError> {
        let pool = seed_pool(seed);
        if pool != SeedPool::Train {
            return Err(GenError::seed_pool_mismatch(
                seed,
                pool,
                TablePurpose::Train,
            ));
        }
        let clean_score = if score.is_finite() {
            score.max(0.0)
        } else {
            0.0
        };
        Ok(Self {
            seed,
            score: clean_score,
            last_seen_step,
        })
    }

    /// Validated `train`-pool layout seed (`10_000..=999_999`).
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Non-negative priority score (e.g., policy value-loss / regret variance).
    #[must_use]
    pub const fn score(&self) -> f64 {
        self.score
    }

    /// Replay step count at the last time this seed was sampled or updated.
    #[must_use]
    pub const fn last_seen_step(&self) -> u64 {
        self.last_seen_step
    }
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
    /// `seed` must belong to [`SeedPool::Train`] (`10_000..=999_999`); any other pool returns
    /// [`GenError::SeedPoolMismatch`] (`SEED_POOL_MISMATCH`) without modifying the buffer.
    /// When the buffer is already at `max_buffer_len`, a new seed only evicts the lowest-priority
    /// entry if `clean_score >= lowest.score`, preventing low-value entries from displacing higher ones.
    pub fn record_score(&mut self, seed: u64, score: f64) -> Result<(), GenError> {
        let candidate_entry = PlrEntry::new(seed, score, self.step)?;
        let clean_score = candidate_entry.score;
        if let Some(existing) = self.entries.iter_mut().find(|e| e.seed == seed) {
            existing.score = clean_score;
            existing.last_seen_step = self.step;
            return Ok(());
        }
        if self.entries.len() >= self.max_buffer_len
            && let Some((worst_idx, worst_entry)) = self
                .entries
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.score.total_cmp(&b.score))
        {
            if clean_score < worst_entry.score {
                return Ok(());
            }
            self.entries.swap_remove(worst_idx);
        }
        self.entries.push(candidate_entry);
        Ok(())
    }

    /// Returns a read-only view of all replay buffer entries.
    #[must_use]
    pub fn entries(&self) -> &[PlrEntry] {
        &self.entries
    }

    /// Chooses whether to replay an existing `train`-pool seed (weighted by rank priority and
    /// staleness) or propose a fresh unseen seed strictly within [`TRAIN_SEED_MIN`]..=[`TRAIN_SEED_MAX`].
    pub fn sample_next(&mut self) -> PlrChoice {
        const TRAIN_POOL_SPAN: u64 = TRAIN_SEED_MAX - TRAIN_SEED_MIN + 1;

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
            debug_assert_eq!(seed_pool(self.entries[chosen_idx].seed), SeedPool::Train);
            return PlrChoice::Replay(self.entries[chosen_idx].seed);
        }

        let raw = self
            .stream
            .seed
            .wrapping_add(self.next_fresh_counter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        self.next_fresh_counter = self.next_fresh_counter.wrapping_add(1);
        let start_offset = raw % TRAIN_POOL_SPAN;
        let mut fresh_seed = TRAIN_SEED_MIN + start_offset;
        if self.entries.iter().any(|e| e.seed == fresh_seed) {
            let mut step = 1_u64;
            while step < TRAIN_POOL_SPAN {
                let candidate = TRAIN_SEED_MIN + ((start_offset + step) % TRAIN_POOL_SPAN);
                if !self.entries.iter().any(|e| e.seed == candidate) {
                    fresh_seed = candidate;
                    break;
                }
                step += 1;
            }
        }
        debug_assert_eq!(seed_pool(fresh_seed), SeedPool::Train);
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

        assert!(plr.record_score(10_010, 1.0).is_ok());
        assert!(plr.record_score(10_020, 10.0).is_ok());
        assert!(plr.record_score(10_020, f64::NAN).is_ok());
        assert!(plr.record_score(10_020, 12.0).is_ok());
        // Exceeding buffer size evicts lowest score (`10_010`).
        assert!(plr.record_score(10_030, 5.0).is_ok());
        assert_eq!(plr.len(), 2);
        assert!(
            plr.entries()
                .iter()
                .all(|e| e.seed == 10_020 || e.seed == 10_030)
        );

        // C9: a newcomer scoring below the current buffer minimum (`5.0`) does not evict `10_030`.
        assert!(plr.record_score(10_040, 2.0).is_ok());
        assert_eq!(plr.len(), 2);
        assert!(
            plr.entries()
                .iter()
                .all(|e| e.seed == 10_020 || e.seed == 10_030)
        );

        let mut replays = 0usize;
        let mut high_picks = 0usize;
        for _ in 0..80 {
            if let PlrChoice::Replay(seed) = plr.sample_next() {
                replays += 1;
                if seed == 10_020 {
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

        let seeds_per_n = 200usize;
        let t_sp_start = Instant::now();
        let mut tables = Vec::with_capacity(3 * seeds_per_n);
        for sc in SeatCount::ALL {
            let cfg_sp = GeneratorConfig {
                seat_count: Some(sc),
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            for seed in 0..(seeds_per_n as u64) {
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
            for seed in 0..60u64 {
                let Ok((t_sh, _)) = generate_ring_table(seed, &cfg_sh) else {
                    panic!("Shapley failed seed={seed} sc={sc:?}");
                };
                sh_attempts_sum += t_sh.attempt as usize;
            }
        }
        let sh_gen_elapsed_ms = t_sh_gen_start.elapsed().as_secs_f64() * 1_000.0;

        let mut sp_eval_us = 0.0_f64;
        let mut sh_eval_us = 0.0_f64;
        let mut sh_pass_by_n = [0usize; 3];
        let mut sh_rels_by_n: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut gap_sum_by_n = [0.0_f64; 3];
        for (idx, (table, report_sp)) in tables.iter().enumerate() {
            let n_bucket = idx / seeds_per_n;
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
            sh_rels_by_n[n_bucket].push(sh_rel_dev);
            if sh_rel_dev <= 0.03 + 1e-9 {
                sh_passes_when_sp_passes += 1;
                sh_pass_by_n[n_bucket] += 1;
            }

            let per_seat_gap = sp_evs
                .iter()
                .zip(sh_evs.iter())
                .map(|(&sp, &sh)| ((sp / sp_mean) - (sh / sh_mean)).abs())
                .fold(0.0_f64, f64::max);
            gap_sum_by_n[n_bucket] += per_seat_gap;
            max_normalized_diff = max_normalized_diff.max(per_seat_gap);
            sum_normalized_diff += per_seat_gap;
            total_tables += 1;

            // Asymmetry probe: boost seat 0 primary converter output +20%.
            let mut boosted = table.clone();
            boosted.seats[0].converters[0].output_amount *= 1.20;
            boosted.config.ev_method = EvMethod::ShadowPrice;
            let sp_rej = check_feasibility(&boosted).is_err();
            boosted.config.ev_method = EvMethod::Shapley;
            let sh_rej = check_feasibility(&boosted).is_err();
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

        let mut sp_attempt0_by_n = [0usize; 3];
        let mut sh_attempt0_by_n = [0usize; 3];
        let mut sp_attempt0_pass = 0usize;
        let mut sh_attempt0_pass = 0usize;
        let attempt0_total = 300usize;
        for (b_idx, sc) in SeatCount::ALL.iter().copied().enumerate() {
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
                    sp_attempt0_by_n[b_idx] += 1;
                }
                if generate_ring_table(seed, &cfg_sh_1).is_ok() {
                    sh_attempt0_pass += 1;
                    sh_attempt0_by_n[b_idx] += 1;
                }
            }
        }

        let mut sh_medians = [0.0_f64; 3];
        let mut sh_maxes = [0.0_f64; 3];
        for b in 0..3 {
            sh_rels_by_n[b].sort_by(f64::total_cmp);
            let mid = sh_rels_by_n[b].len() / 2;
            sh_medians[b] = (sh_rels_by_n[b][mid - 1] + sh_rels_by_n[b][mid]) / 2.0;
            sh_maxes[b] = *sh_rels_by_n[b].last().unwrap_or(&0.0);
        }

        let n_f = total_tables as f64;
        let per_n_f = seeds_per_n as f64;
        println!(
            "EV_COMPARISON n={total_tables} (N=4,5,6 x {seeds_per_n} seeds): sp_gen_ms={sp_elapsed_ms:.1} (avg_rej={:.2}) sh_gen_ms={sh_gen_elapsed_ms:.1} (avg_rej={:.2}) | attempt0_pass_100: sp={sp_attempt0_pass}/{attempt0_total} (by_N={sp_attempt0_by_n:?}) sh={sh_attempt0_pass}/{attempt0_total} (by_N={sh_attempt0_by_n:?}) | sp_eval_us={:.1} sh_eval_us={:.1} | sp_mean_max_rel={:.4} sh_mean_max_rel_on_sp={:.4} sh_pass_rate_on_sp={}/{} (by_N=[{}/{seeds_per_n}, {}/{seeds_per_n}, {}/{seeds_per_n}]) sh_median_max_rel_by_N=[{:.4}, {:.4}, {:.4}] sh_worst_max_rel_by_N=[{:.4}, {:.4}, {:.4}] | mean_norm_gap={:.4} (by_N=[{:.4}, {:.4}, {:.4}]) max_norm_gap={:.4} | plus20_out_rej: both={} sp_only={} sh_only={}",
            (sp_attempts_sum as f64) / n_f,
            (sh_attempts_sum as f64) / 180.0,
            sp_eval_us / n_f,
            sh_eval_us / n_f,
            sp_max_rel_sum / n_f,
            sh_max_rel_sum / n_f,
            sh_passes_when_sp_passes,
            total_tables,
            sh_pass_by_n[0],
            sh_pass_by_n[1],
            sh_pass_by_n[2],
            sh_medians[0],
            sh_medians[1],
            sh_medians[2],
            sh_maxes[0],
            sh_maxes[1],
            sh_maxes[2],
            sum_normalized_diff / n_f,
            gap_sum_by_n[0] / per_n_f,
            gap_sum_by_n[1] / per_n_f,
            gap_sum_by_n[2] / per_n_f,
            max_normalized_diff,
            boost_both_reject,
            boost_sp_reject_only,
            boost_sh_reject_only,
        );

        assert_eq!(total_tables, 600);
        // B-6: ratcheted per-N attempt-0 guards on 100 seeds/N (plus pooled guard) sitting just under measured counts.
        let min_sp_a0_100 = [20usize, 8, 9];
        for (b_idx, sc) in SeatCount::ALL.iter().copied().enumerate() {
            assert!(
                sp_attempt0_by_n[b_idx] >= min_sp_a0_100[b_idx],
                "N={} attempt-0 ShadowPrice pass count {}/100 fell below ratcheted guard {}",
                sc.as_usize(),
                sp_attempt0_by_n[b_idx],
                min_sp_a0_100[b_idx]
            );
        }
        assert!(
            sp_attempt0_pass >= 42,
            "pooled attempt-0 ShadowPrice pass count {sp_attempt0_pass}/{attempt0_total} fell below ratcheted guard 42"
        );
        // B-2: per-N Shapley agreement assertions on 200 ShadowPrice-accepted seeds/N (report/monitoring floors, not a fairness gate).
        let min_sh_on_sp_200 = [25usize, 35, 125];
        for (b_idx, sc) in SeatCount::ALL.iter().copied().enumerate() {
            assert!(
                sh_pass_by_n[b_idx] >= min_sh_on_sp_200[b_idx],
                "N={} Shapley ±3% agreement on ShadowPrice-accepted tables {}/{seeds_per_n} fell below per-N floor {}",
                sc.as_usize(),
                sh_pass_by_n[b_idx],
                min_sh_on_sp_200[b_idx]
            );
        }
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
        let mut mean_yield_cvs = Vec::new();
        let mut mean_tariff_cvs = Vec::new();
        let mut raw_mean_yield_cvs = Vec::new();
        let mut raw_mean_tariff_cvs = Vec::new();

        for sc in SeatCount::ALL {
            let cfg_sp_1 = GeneratorConfig {
                seat_count: Some(sc),
                max_attempts: 1,
                ev_method: EvMethod::ShadowPrice,
                ..GeneratorConfig::default()
            };
            let mut sp_a0 = 0usize;
            let mut raw_y_cvs = Vec::with_capacity(600);
            let mut raw_t_cvs = Vec::with_capacity(600);
            for seed in 0u64..600 {
                let mut raw_stream = SeedStream::new(seed, b"ring-table");
                let Ok(raw_cand) = sample_candidate(seed, 0, sc, &cfg_sp_1, &mut raw_stream) else {
                    panic!("raw candidate failed for seed {seed} ({sc:?})");
                };
                let rys: Vec<f64> = raw_cand
                    .seats
                    .iter()
                    .map(|s| s.raw_yield_per_turn)
                    .collect();
                let rts: Vec<f64> = raw_cand.seats.iter().map(|s| s.raw_export_tariff).collect();
                let (rym, rysd) = stddev_mean(&rys);
                let (rtm, rtsd) = stddev_mean(&rts);
                raw_y_cvs.push(rysd / rym.max(1e-12));
                raw_t_cvs.push(rtsd / rtm.max(1e-12));

                if check_feasibility(&raw_cand).is_ok() {
                    sp_a0 += 1;
                }
            }
            raw_mean_yield_cvs.push(raw_y_cvs.iter().sum::<f64>() / 600.0);
            raw_mean_tariff_cvs.push(raw_t_cvs.iter().sum::<f64>() / 600.0);

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
            mean_yield_cvs.push(mean_ycv);
            mean_tariff_cvs.push(mean_tcv);
            let min_sp_a0_600 = match sc {
                SeatCount::Four => 135,
                SeatCount::Five => 42,
                SeatCount::Six => 70,
            };
            assert!(
                sp_a0 >= min_sp_a0_600,
                "N={} attempt-0 ShadowPrice pass count {sp_a0}/600 fell below ratcheted per-N guard {min_sp_a0_600}",
                sc.as_usize()
            );
        }

        let ycv_min = mean_yield_cvs.iter().copied().fold(f64::INFINITY, f64::min);
        let ycv_max = mean_yield_cvs
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let ycv_spread = (ycv_max - ycv_min) / ycv_min;
        let tcv_min = mean_tariff_cvs
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let tcv_max = mean_tariff_cvs
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let tcv_spread = (tcv_max - tcv_min) / tcv_min;
        let raw_ycv_min = raw_mean_yield_cvs
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let raw_ycv_max = raw_mean_yield_cvs
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let raw_ycv_spread = (raw_ycv_max - raw_ycv_min) / raw_ycv_min;
        let raw_tcv_min = raw_mean_tariff_cvs
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let raw_tcv_max = raw_mean_tariff_cvs
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let raw_tcv_spread = (raw_tcv_max - raw_tcv_min) / raw_tcv_min;
        println!(
            "G1_RAW_CANDIDATES (600 seeds/N): yield_intra_cv={raw_mean_yield_cvs:.4?} spread={:.2}% | tariff_intra_cv={raw_mean_tariff_cvs:.4?} spread={:.2}%",
            100.0 * raw_ycv_spread,
            100.0 * raw_tcv_spread,
        );
        println!(
            "G1_SUMMARY (accepted, 600 seeds/N): yield_intra_cv={mean_yield_cvs:.4?} spread={:.2}% | tariff_intra_cv={mean_tariff_cvs:.4?} spread={:.2}%",
            100.0 * ycv_spread,
            100.0 * tcv_spread,
        );
        for (idx, sc) in SeatCount::ALL.iter().enumerate() {
            assert!(
                raw_mean_yield_cvs[idx] >= 0.0038,
                "N={} raw candidate yield intra_cv {:.4} below G1 floor 0.0038",
                sc.as_usize(),
                raw_mean_yield_cvs[idx]
            );
            assert!(
                raw_mean_tariff_cvs[idx] >= 0.20,
                "N={} raw candidate tariff intra_cv {:.4} below G1 floor 0.20",
                sc.as_usize(),
                raw_mean_tariff_cvs[idx]
            );
            assert!(
                mean_yield_cvs[idx] >= 0.0038,
                "N={} yield intra_cv {:.4} below G1 floor 0.0038",
                sc.as_usize(),
                mean_yield_cvs[idx]
            );
            assert!(
                mean_tariff_cvs[idx] >= 0.20,
                "N={} tariff intra_cv {:.4} below G1 floor 0.20",
                sc.as_usize(),
                mean_tariff_cvs[idx]
            );
        }
        assert!(
            raw_ycv_spread <= 0.10,
            "raw candidate yield intra_cv across-N spread {:.2}% exceeds 10%",
            100.0 * raw_ycv_spread
        );
        assert!(
            raw_tcv_spread <= 0.10,
            "raw candidate tariff intra_cv across-N spread {:.2}% exceeds 10%",
            100.0 * raw_tcv_spread
        );
        assert!(
            ycv_spread <= 0.10,
            "yield intra_cv across-N spread {:.2}% exceeds 10%",
            100.0 * ycv_spread
        );
        assert!(
            tcv_spread <= 0.10,
            "tariff intra_cv across-N spread {:.2}% exceeds 10%",
            100.0 * tcv_spread
        );

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
    fn g1_item2_distinct_best_trade_orderings_and_item3_inter_cv_floor() {
        let stddev_mean = |vals: &[f64]| -> (f64, f64) {
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            (mean, var.sqrt())
        };
        let main_s0_yield_cv = [0.0826_f64, 0.0793, 0.0805];
        let main_s0_tariff_cv = [0.5752_f64, 0.5985, 0.6113];
        let main_s0_share_cv = [0.0469_f64, 0.0473, 0.0463];
        // C-1: explicit 600-seed sampling allowances (~3 SE on CV at n = 600).
        const ITEM3_YIELD_CV_ALLOWANCE: f64 = 0.0060;
        const ITEM3_TARIFF_CV_ALLOWANCE: f64 = 0.0450;
        const ITEM3_SHARE_CV_ALLOWANCE: f64 = 0.0060;

        let mut per_n_stats = Vec::with_capacity(SeatCount::ALL.len());
        for (idx, sc) in SeatCount::ALL.iter().copied().enumerate() {
            let cfg_a0 = GeneratorConfig {
                seat_count: Some(sc),
                max_attempts: 1,
                ..GeneratorConfig::default()
            };
            let mut sp_a0 = 0usize;
            let mut raw_d1_pass = 0usize;
            for seed in 0u64..600 {
                let mut raw_stream = SeedStream::new(seed, b"ring-table");
                let Ok(raw_cand) = sample_candidate(seed, 0, sc, &cfg_a0, &mut raw_stream) else {
                    panic!("raw candidate failed on seed {seed} ({sc:?})");
                };
                if let Ok(raw_ord) = crate::feasibility::compute_seat_trade_orderings(&raw_cand)
                    && raw_ord.d1_all_seats_distinct
                {
                    raw_d1_pass += 1;
                }
                if check_feasibility(&raw_cand).is_ok() {
                    sp_a0 += 1;
                }
            }

            let cfg_full = GeneratorConfig {
                seat_count: Some(sc),
                ..GeneratorConfig::default()
            };
            let mut d1_pass = 0usize;
            let mut top1_pass = 0usize;
            let mut d2_pass = 0usize;
            let mut min_pos_opts = usize::MAX;
            let mut max_att = 0u32;
            let mut max_rel_ev_dev = 0.0_f64;
            let mut duals_all_pos_tables = 0usize;
            let mut s0_yields = Vec::with_capacity(600);
            let mut s0_tariffs = Vec::with_capacity(600);
            let mut s0_shares = Vec::with_capacity(600);
            let mut binding_caps = Vec::new();

            for seed in 0u64..600 {
                let Ok((t, rep)) = generate_ring_table(seed, &cfg_full) else {
                    panic!("seed {seed} failed for {sc:?}");
                };
                max_att = max_att.max(t.attempt);
                max_rel_ev_dev = max_rel_ev_dev.max(rep.max_relative_ev_deviation);
                s0_yields.push(t.seats[0].raw_yield_per_turn);
                s0_tariffs.push(t.seats[0].raw_export_tariff);
                let p_inputs = &t.seats[0].converters[0].inputs;
                let p_sum: f64 = p_inputs.iter().map(|&(_, amt)| amt).sum();
                s0_shares.push(p_inputs[0].1 / p_sum.max(1e-12));

                let binding_tiers = if sc == SeatCount::Four {
                    1usize
                } else {
                    2usize
                };
                let Ok(det) = crate::feasibility::solve_shadow_price_details(&t) else {
                    panic!("solve_shadow_price_details failed on seed {seed} ({sc:?})");
                };
                let convs_per_seat = t.seats[0].converters.len();
                let mut all_binding_pos = true;
                for (s_i, seat_spec) in t.seats.iter().enumerate() {
                    for b_t in 0..binding_tiers {
                        binding_caps.push(seat_spec.converters[b_t].max_runs_per_turn);
                        if det.converter_cap_duals[s_i * convs_per_seat + b_t] <= 1e-6 {
                            all_binding_pos = false;
                        }
                    }
                }
                if all_binding_pos {
                    duals_all_pos_tables += 1;
                }

                let Ok(ord) = crate::feasibility::compute_seat_trade_orderings(&t) else {
                    panic!("compute_seat_trade_orderings failed on accepted seed {seed} ({sc:?})");
                };
                min_pos_opts = min_pos_opts.min(ord.min_positive_options_per_seat);
                if ord.d1_all_seats_distinct {
                    d1_pass += 1;
                }
                if ord.top1_all_seats_distinct {
                    top1_pass += 1;
                }
                if ord.d2_all_seats_distinct {
                    d2_pass += 1;
                }
            }

            binding_caps.sort_by(f64::total_cmp);
            let cap_min = *binding_caps.first().unwrap_or(&0.0);
            let cap_max = *binding_caps.last().unwrap_or(&0.0);
            let cap_mid = binding_caps.len() / 2;
            let cap_med = if binding_caps.is_empty() {
                0.0
            } else {
                (binding_caps[cap_mid - 1] + binding_caps[cap_mid]) / 2.0
            };
            let below_1 = binding_caps.iter().filter(|&&c| c < 1.0).count();

            let (y_m, y_sd) = stddev_mean(&s0_yields);
            let (t_m, t_sd) = stddev_mean(&s0_tariffs);
            let (sh_m, sh_sd) = stddev_mean(&s0_shares);
            let s0_yield_cv = y_sd / y_m;
            let s0_tariff_cv = t_sd / t_m;
            let s0_share_cv = sh_sd / sh_m;
            println!(
                "G1_ITEM2 N={}: d1_pass={d1_pass}/600 ({:.1}%) raw_d1={raw_d1_pass}/600 ({:.1}%) top1_pass={top1_pass}/600 ({:.1}%) d2_pass={d2_pass}/600 ({:.1}%) min_pos_opts={min_pos_opts} binding_duals_pos={duals_all_pos_tables}/600 max_rel_ev={max_rel_ev_dev:.5} | sp_attempt0={sp_a0}/600 ({:.1}%) max_att={max_att} | inter_cv(s0_yield={s0_yield_cv:.4} vs_main={:.4}, s0_tariff={s0_tariff_cv:.4} vs_main={:.4}, s0_share={s0_share_cv:.4} vs_main={:.4})",
                sc.as_usize(),
                100.0 * (d1_pass as f64) / 600.0,
                100.0 * (raw_d1_pass as f64) / 600.0,
                100.0 * (top1_pass as f64) / 600.0,
                100.0 * (d2_pass as f64) / 600.0,
                100.0 * (sp_a0 as f64) / 600.0,
                main_s0_yield_cv[idx],
                main_s0_tariff_cv[idx],
                main_s0_share_cv[idx],
            );
            println!(
                "BINDING_CAPS N={} (600 tables, {} binding workshops): min={cap_min:.3} median={cap_med:.3} max={cap_max:.3} below_1.0={below_1}/{} ({:.1}%)",
                sc.as_usize(),
                binding_caps.len(),
                binding_caps.len(),
                100.0 * (below_1 as f64) / (binding_caps.len().max(1) as f64),
            );
            per_n_stats.push((
                idx,
                sc.as_usize(),
                d1_pass,
                min_pos_opts,
                duals_all_pos_tables,
                s0_yield_cv,
                s0_tariff_cv,
                s0_share_cv,
            ));
        }

        for (
            idx,
            n_seats,
            d1_pass,
            min_pos_opts,
            duals_all_pos_tables,
            s0_yield_cv,
            s0_tariff_cv,
            s0_share_cv,
        ) in per_n_stats
        {
            assert!(
                d1_pass >= 540,
                "N={n_seats} D1 distinct best-trade ordering rate {d1_pass}/600 ({:.1}%) below 90% floor",
                100.0 * (d1_pass as f64) / 600.0
            );
            assert!(
                min_pos_opts >= 3,
                "N={n_seats} min positive shadow-LP options per seat {min_pos_opts} < 3",
            );
            assert_eq!(
                duals_all_pos_tables, 600,
                "N={n_seats} binding workshop capacity duals must stay > 0 on all 600 accepted tables"
            );
            let min_yield_cv = main_s0_yield_cv[idx] - ITEM3_YIELD_CV_ALLOWANCE;
            assert!(
                s0_yield_cv >= min_yield_cv,
                "N={n_seats} inter-table yield CV {s0_yield_cv:.4} fell below main reference {:.4} minus sampling allowance {ITEM3_YIELD_CV_ALLOWANCE:.4} ({min_yield_cv:.4})",
                main_s0_yield_cv[idx]
            );
            let min_tariff_cv = main_s0_tariff_cv[idx] - ITEM3_TARIFF_CV_ALLOWANCE;
            assert!(
                s0_tariff_cv >= min_tariff_cv,
                "N={n_seats} inter-table tariff CV {s0_tariff_cv:.4} fell below main reference {:.4} minus sampling allowance {ITEM3_TARIFF_CV_ALLOWANCE:.4} ({min_tariff_cv:.4})",
                main_s0_tariff_cv[idx]
            );
            let min_share_cv = main_s0_share_cv[idx] - ITEM3_SHARE_CV_ALLOWANCE;
            assert!(
                s0_share_cv >= min_share_cv,
                "N={n_seats} inter-table primary input share CV {s0_share_cv:.4} fell below main reference {:.4} minus sampling allowance {ITEM3_SHARE_CV_ALLOWANCE:.4} ({min_share_cv:.4})",
                main_s0_share_cv[idx]
            );
        }
    }

    #[test]
    fn golden_v2_4_1_table_serialization_digest_matches_pinned_seed42() {
        let cfg = GeneratorConfig::default();
        let Ok((table, _)) = generate_ring_table(42, &cfg) else {
            panic!("seed 42 generates cleanly");
        };
        assert_eq!(table.generator_version, "league-gen/v2.4.1");
        let Ok(bytes) = serde_json::to_vec(&table) else {
            panic!("RingTable serializes deterministically");
        };
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        const EXPECTED_GOLDEN_SHA256: &str =
            "7bd0859fc41b599ef78c5b73840bad46a37de2393b386e5c99157c0ffdd12a5f";
        assert_eq!(
            hex, EXPECTED_GOLDEN_SHA256,
            "v2.4.1 golden digest mismatch (bump GENERATOR_VERSION if intentional)"
        );
    }

    #[test]
    fn checked_table_purpose_accepts_own_pool_boundaries_and_refuses_other_pools() {
        use crate::schema::{
            DEV_SEED_MAX, DEV_SEED_MIN, HELD_OUT_SEED_MAX, HELD_OUT_SEED_MIN, PROBE_SEED_MAX,
            PROBE_SEED_MIN, SEALED_SEED_MAX, SEALED_SEED_MIN,
        };

        let cfg = GeneratorConfig::default();
        let own_boundaries: [(TablePurpose, u64, u64); 5] = [
            (TablePurpose::Dev, DEV_SEED_MIN, DEV_SEED_MAX),
            (TablePurpose::Train, TRAIN_SEED_MIN, TRAIN_SEED_MAX),
            (TablePurpose::Probe, PROBE_SEED_MIN, PROBE_SEED_MAX),
            (TablePurpose::HeldOut, HELD_OUT_SEED_MIN, HELD_OUT_SEED_MAX),
            (TablePurpose::Sealed, SEALED_SEED_MIN, SEALED_SEED_MAX),
        ];

        for (purpose, first_seed, last_seed) in own_boundaries {
            for boundary_seed in [first_seed, last_seed] {
                let Ok((checked_table, checked_report, record)) =
                    generate_ring_table_for_purpose(boundary_seed, purpose, &cfg)
                else {
                    panic!("purpose failed to accept its own boundary seed");
                };
                let Ok((raw_table, raw_report)) = generate_ring_table(boundary_seed, &cfg) else {
                    panic!("raw generator failed on boundary seed");
                };
                assert_eq!(checked_table, raw_table);
                assert_eq!(checked_report, raw_report);
                assert_eq!(record.pool(), purpose.expected_pool());
                assert_eq!(record.seat_count(), checked_table.seat_count);
                assert_eq!(record.generator_version(), GENERATOR_VERSION);
                assert!(!record.training_logged());
            }
        }

        let pool_samples: [(SeedPool, u64); 6] = [
            (SeedPool::Dev, 42),
            (SeedPool::Train, 10_042),
            (SeedPool::Probe, 1_000_042),
            (SeedPool::HeldOut, 2_000_042),
            (SeedPool::Sealed, SEALED_SEED_MIN),
            (SeedPool::Unassigned, 1_500_000),
        ];

        for purpose in TablePurpose::ALL {
            for (sample_pool, sample_seed) in pool_samples {
                if sample_pool == purpose.expected_pool() {
                    continue;
                }
                let Err(err) = generate_ring_table_for_purpose(sample_seed, purpose, &cfg) else {
                    panic!("mismatched pool must be refused");
                };
                assert!(matches!(
                    err,
                    GenError::SeedPoolMismatch {
                        seed_pool: actual,
                        purpose: requested,
                        ..
                    } if actual == sample_pool && requested == purpose
                ));
                let msg = err.to_string();
                assert!(msg.contains("SEED_POOL_MISMATCH"));
                assert!(msg.contains(sample_pool.as_str()));
                assert!(msg.contains(purpose.as_str()));
            }
        }

        for non_sealed_purpose in [
            TablePurpose::Dev,
            TablePurpose::Train,
            TablePurpose::Probe,
            TablePurpose::HeldOut,
        ] {
            let Err(err) =
                generate_ring_table_for_purpose(SEALED_SEED_MIN, non_sealed_purpose, &cfg)
            else {
                panic!("sealed-range seed must be refused for non-sealed purpose");
            };
            assert!(err.to_string().contains("SEED_POOL_MISMATCH"));
            assert!(err.to_string().contains("sealed"));
            assert!(err.to_string().contains(non_sealed_purpose.as_str()));
        }
    }

    #[test]
    fn plr_sampler_accepts_train_boundaries_refuses_other_pools_and_bounds_fresh_draws() {
        use crate::schema::SEALED_SEED_MIN;

        let mut plr = PlrSampler::new(0, 8, 0.25, 1.0, 0.5);
        assert!(plr.record_score(TRAIN_SEED_MIN, 1.0).is_ok());
        assert!(plr.record_score(TRAIN_SEED_MAX, 2.0).is_ok());
        assert_eq!(plr.len(), 2);

        let Ok(encoded_plr) = serde_json::to_string(&plr) else {
            panic!("serialize PlrSampler");
        };
        let Ok(decoded_plr) = serde_json::from_str::<PlrSampler>(&encoded_plr) else {
            panic!("deserialize valid PlrSampler");
        };
        assert_eq!(decoded_plr.len(), 2);

        let refused_cases: [(u64, SeedPool); 5] = [
            (10, SeedPool::Dev),
            (1_000_000, SeedPool::Probe),
            (2_000_000, SeedPool::HeldOut),
            (SEALED_SEED_MIN, SeedPool::Sealed),
            (1_500_000, SeedPool::Unassigned),
        ];
        for (bad_seed, expected_pool) in refused_cases {
            let Err(err) = plr.record_score(bad_seed, 5.0) else {
                panic!("non-train seed must be refused by PlrSampler::record_score");
            };
            assert!(matches!(
                err,
                GenError::SeedPoolMismatch {
                    seed_pool,
                    purpose: TablePurpose::Train,
                    ..
                } if seed_pool == expected_pool
            ));
            let msg = err.to_string();
            assert!(msg.contains("SEED_POOL_MISMATCH"));
            assert!(msg.contains(expected_pool.as_str()));
            assert!(msg.contains("train"));
            assert_eq!(plr.len(), 2);

            assert!(PlrEntry::new(bad_seed, 5.0, 0).is_err());
        }

        assert!(
            serde_json::from_str::<PlrEntry>(r#"{"seed":10,"score":1.0,"last_seen_step":0}"#)
                .is_err()
        );

        for init_seed in [0_u64, 42, 1_500_000, SEALED_SEED_MIN, u64::MAX] {
            let mut fresh_only = PlrSampler::new(init_seed, 4, 0.2, 1.0, 0.0);
            assert!(fresh_only.record_score(TRAIN_SEED_MAX, 1.0).is_ok());
            for _ in 0..200 {
                match fresh_only.sample_next() {
                    PlrChoice::Fresh(s) | PlrChoice::Replay(s) => {
                        assert!((TRAIN_SEED_MIN..=TRAIN_SEED_MAX).contains(&s));
                        assert_eq!(seed_pool(s), SeedPool::Train);
                    }
                }
            }
        }
    }

    #[test]
    fn game_table_record_enforces_training_logged_and_redacts_sealed_seeds_everywhere() {
        use crate::schema::{RecordedLayoutSeed, SEALED_SEED_MIN};

        let cfg = GeneratorConfig::default();

        let Ok((dev_table, _, dev_record)) =
            generate_ring_table_for_setup(42, TableSetupOptions::default(), &cfg)
        else {
            panic!("default dev setup must accept seed 42");
        };
        assert_eq!(dev_record.layout_seed(), &RecordedLayoutSeed::Clear(42));
        assert_eq!(dev_record.clear_layout_seed(), Some(42));
        assert_eq!(dev_record.sealed_seed_sha256(), None);
        assert_eq!(dev_record.seat_count(), dev_table.seat_count);
        assert_eq!(dev_record.pool(), SeedPool::Dev);
        assert_eq!(dev_record.generator_version(), GENERATOR_VERSION);
        assert!(!dev_record.training_logged());

        let Ok((_, _, train_record)) = generate_ring_table_for_setup(
            10_042,
            TableSetupOptions {
                purpose: TablePurpose::Train,
                training_logged: true,
            },
            &cfg,
        ) else {
            panic!("train purpose must allow training_logged = true");
        };
        assert_eq!(train_record.pool(), SeedPool::Train);
        assert!(train_record.training_logged());

        let made_up_sealed: u64 = SEALED_SEED_MIN | 42;
        let expected_digest: String = Sha256::digest(made_up_sealed.to_be_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let forbidden_dec = made_up_sealed.to_string();
        let forbidden_hex_lo = format!("{made_up_sealed:x}");
        let forbidden_hex_hi = format!("{made_up_sealed:X}");

        for (non_train_purpose, matching_seed) in [
            (TablePurpose::Dev, 42_u64),
            (TablePurpose::Probe, 1_000_042_u64),
            (TablePurpose::HeldOut, 2_000_042_u64),
            (TablePurpose::Sealed, made_up_sealed),
        ] {
            let Err(err) = generate_ring_table_for_setup(
                matching_seed,
                TableSetupOptions {
                    purpose: non_train_purpose,
                    training_logged: true,
                },
                &cfg,
            ) else {
                panic!("training_logged = true must be refused outside train");
            };
            assert!(matches!(err, GenError::TrainingLoggedMismatch { .. }));
            assert!(err.to_string().contains("TRAINING_LOGGED_MISMATCH"));
            if non_train_purpose == TablePurpose::Sealed {
                let disp = err.to_string();
                let dbg = format!("{err:?}");
                assert!(disp.contains("sealed") && disp.contains(&expected_digest));
                assert!(dbg.contains("Sealed") && dbg.contains(&expected_digest));
                assert!(
                    !disp.contains(&forbidden_dec)
                        && !disp.contains(&forbidden_hex_lo)
                        && !disp.contains(&forbidden_hex_hi),
                    "sealed training_logged refusal leaked raw seed in Display"
                );
                assert!(
                    !dbg.contains(&forbidden_dec)
                        && !dbg.contains(&forbidden_hex_lo)
                        && !dbg.contains(&forbidden_hex_hi),
                    "sealed training_logged refusal leaked raw seed in Debug"
                );
            }
        }

        let Ok((sealed_table, _, sealed_record)) =
            generate_ring_table_for_purpose(made_up_sealed, TablePurpose::Sealed, &cfg)
        else {
            panic!("sealed purpose must accept made-up sealed-range value");
        };
        assert_eq!(sealed_record.pool(), SeedPool::Sealed);
        assert_eq!(sealed_record.seat_count(), sealed_table.seat_count);
        assert_eq!(sealed_record.generator_version(), GENERATOR_VERSION);
        assert!(!sealed_record.training_logged());
        assert_eq!(sealed_record.clear_layout_seed(), None);
        assert_eq!(
            sealed_record.sealed_seed_sha256(),
            Some(expected_digest.as_str())
        );

        let Ok(record_json) = serde_json::to_string(&sealed_record) else {
            panic!("serialize GameTableRecord");
        };
        let record_dbg = format!("{sealed_record:?}");
        assert!(record_json.contains("\"pool\":\"sealed\""));
        assert!(record_json.contains(&expected_digest));
        assert!(
            !record_json.contains(&forbidden_dec)
                && !record_json.contains(&forbidden_hex_lo)
                && !record_json.contains(&forbidden_hex_hi),
            "serialized GameTableRecord leaked raw sealed-range seed"
        );
        assert!(record_dbg.contains("Sealed") && record_dbg.contains(&expected_digest));
        assert!(
            !record_dbg.contains(&forbidden_dec)
                && !record_dbg.contains(&forbidden_hex_lo)
                && !record_dbg.contains(&forbidden_hex_hi),
            "Debug GameTableRecord leaked raw sealed-range seed"
        );

        let mut plr = PlrSampler::new(10_000, 4, 0.2, 1.0, 0.5);
        let Err(plr_refusal) = plr.record_score(made_up_sealed, 1.0) else {
            panic!("plr must refuse sealed seed");
        };
        let mut refusals = vec![plr_refusal];
        for wrong_purpose in [
            TablePurpose::Dev,
            TablePurpose::Train,
            TablePurpose::Probe,
            TablePurpose::HeldOut,
        ] {
            let Err(wrong_refusal) =
                generate_ring_table_for_purpose(made_up_sealed, wrong_purpose, &cfg)
            else {
                panic!("wrong purpose must refuse sealed seed");
            };
            refusals.push(wrong_refusal);
        }
        for refusal in refusals {
            let disp = refusal.to_string();
            let dbg = format!("{refusal:?}");
            assert!(disp.contains("SEED_POOL_MISMATCH"));
            assert!(disp.contains("sealed") && disp.contains(&expected_digest));
            assert!(dbg.contains("Sealed") && dbg.contains(&expected_digest));
            assert!(
                !disp.contains(&forbidden_dec)
                    && !disp.contains(&forbidden_hex_lo)
                    && !disp.contains(&forbidden_hex_hi),
                "SEED_POOL_MISMATCH Display leaked raw sealed-range seed"
            );
            assert!(
                !dbg.contains(&forbidden_dec)
                    && !dbg.contains(&forbidden_hex_lo)
                    && !dbg.contains(&forbidden_hex_hi),
                "SEED_POOL_MISMATCH Debug leaked raw sealed-range seed"
            );
        }
    }
}
