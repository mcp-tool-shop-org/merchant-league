//! Core domain types, integer fixed-point arithmetic helpers, and table specifications
//! for Merchant League v2.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub use league_seat_types::{
    ActiveShock, AssetId, BPS_SCALE, Bps, CLOSING_ROLL_START_TURN, CLOSING_ROLL_WINDOW_LEN,
    CharterRole, ConverterRecipe, CreditTier, EngineError, HARD_TURN_CAP,
    MANDATE_STARTING_NW_FLOOR_BPS, MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY, MarketDay,
    ProjectRecipe, RegentDoctrine, SeasonId, Seat, SeatConfig, SeatIdentity, ShockKind, TableId,
    TableMode, Turn, UNIT_SCALE, UNSOLD_EXCESS_PARITY_BPS, Units,
};

use crate::commit::{
    CommitmentHash, DrandSchedule, GradedMandateCard, RevealSalt, SealedEvalSeed,
    SealedMandateDeal, deal_graded_mandates, deal_graded_mandates_with_entropy,
    default_graded_mandate_deck, validate_eval_vrf_pubkey,
};

pub(crate) mod opt_hex32 {
    pub(crate) use league_seat_types::wire_serde::{
        deserialize_opt_hex32 as deserialize, serialize_opt_hex32 as serialize,
    };
}

/// Maximum political commodity export tariff (`35.00%` = `3_500 bps`).
pub const MAX_TARIFF_BPS: Bps = 3_500;

/// Provisional per-turn closing bell hazard probabilities (in bps) for turns `24..=31`
/// (`35.00%` per turn -> expected season length `~25.80` turns and `~3.19%` cap arrival at Turn 32;
/// `2026-11-15` ruling, Seed-28 Report §4, pending playtest calibration).
pub const DEFAULT_CLOSING_HAZARD_BPS: [Bps; CLOSING_ROLL_WINDOW_LEN] =
    [3_500; CLOSING_ROLL_WINDOW_LEN];

/// Provisional mandate-point scaling factor `alpha = 0.025` (`250 bps = 2.5%` of
/// `max(Med(W), 0.5 * W_0)` per `1.000` mandate point; `2026-11-15` ruling, Seed-28 Report §6
/// Formula 3, pending playtest calibration).
pub const DEFAULT_MANDATE_ALPHA_BPS: Bps = 250;

/// Computes exact expected season length `E[T]` from a per-turn closing hazard schedule over
/// Turns `24..=31` capped unconditionally at [`HARD_TURN_CAP`] (`Turn 32`).
#[must_use]
pub fn expected_season_length_turns(hazard_bps: &[Bps; CLOSING_ROLL_WINDOW_LEN]) -> f64 {
    let mut survival = 1.0_f64;
    let mut expected = f64::from(CLOSING_ROLL_START_TURN);
    for &hz in hazard_bps {
        let p_stop = (f64::from(hz) / (BPS_SCALE as f64)).clamp(0.0, 1.0);
        survival *= 1.0 - p_stop;
        expected += survival;
    }
    expected
}

/// Computes exact probability `P(T == HARD_TURN_CAP)` of reaching the hard cap Turn 32 without an
/// earlier stochastic close across Turns `24..=31`.
#[must_use]
pub fn cap_reach_probability(hazard_bps: &[Bps; CLOSING_ROLL_WINDOW_LEN]) -> f64 {
    let mut survival = 1.0_f64;
    for &hz in hazard_bps {
        let p_stop = (f64::from(hz) / (BPS_SCALE as f64)).clamp(0.0, 1.0);
        survival *= 1.0 - p_stop;
    }
    survival
}

/// Default PBRS discount factor `gamma = 0.95` (`9_500 bps`).
pub const DEFAULT_PBRS_GAMMA_BPS: Bps = 9_500;

/// Consecutive missed-commit or zero-production turns before the Standing Regent takes over
/// a seat and converts `SeatIdentity::Human -> AiSyndicate` (`C1`, `2026-11-15`).
pub const IDLE_MISS_TAKEOVER_TURNS: u8 = 3;

/// Provisional Authority town-import premium multiplier (`22_500 bps` = `2.25x` fundamental
/// shadow parity; `C1`, `2026-11-15`, pending Playtest 1 calibration).
///
/// Priced inside the `2.2x–2.5x` reference band and strictly above maximum `35%` bilateral export
/// tariffs (`1.35x` parity), so importing from Authority loses margin versus trading with peers
/// while preventing an idle supplier from freezing a converter or Syndicate Project.
pub const DEFAULT_TOWN_IMPORT_PREMIUM_BPS: Bps = 22_500;

/// Provisional working-capital `CRED` stipend recycled per `1_000` milli-units (`1.000` unit) of
/// Refined commodity burned into a Tier-3 Syndicate Project (`0.350 CRED`/unit; `C3`, `2026-11-15`,
/// pending Playtest 1 calibration).
///
/// Offsets secular `CRED` sinks (Charter auctions, auto-debt repayments, town imports) without
/// giving leftover `CRED` any standalone Patronage or Project VP bonus.
pub const DEFAULT_WORKS_STIPEND_PER_UNIT_CRED: Units = 350;

/// Provisional cancel-lock fraction of Phase 1 (`2_500 bps` = final `25.00%`; `C4` amended
/// `2026-11-17`; `0` disables the lock; provisional, pending playtest calibration).
pub const DEFAULT_CANCEL_LOCK_BPS: Bps = 2_500;

/// Provisional minimum bidirectional commodity flow (`1_000` milli-units = `1.000` unit) per
/// direction in one turn required to confirm a Customs Inspector wash-freeze round-trip audit
/// (`§5.2` amended `2026-11-17`; provisional, pending playtest calibration).
pub const DEFAULT_WASH_ROUND_TRIP_MIN_UNITS: Units = 1_000;

/// Provisional challenger bond (`500` milli-`CRED` = `0.500 CRED`) posted upfront by the active
/// `CustomsInspector` on `FlagWashFreeze` (`§5.2` amended `2026-11-17`; provisional, pending
/// playtest calibration). A clean audit pays the whole bond to the frozen seat as damages; a
/// confirmed audit returns it to the Inspector.
pub const DEFAULT_WASH_FREEZE_BOND_CRED: Units = 500;

/// Provisional confirmation bounty (`500` milli-`CRED` = `0.500 CRED`) paid from the frozen
/// seat's free `CRED` (capped at available free `CRED`) when a Customs Inspector wash freeze is
/// confirmed (`§5.2` amended `2026-11-17`; provisional, pending playtest calibration).
pub const DEFAULT_WASH_FREEZE_BOUNTY_CRED: Units = 500;

const fn default_town_import_premium_bps() -> Bps {
    DEFAULT_TOWN_IMPORT_PREMIUM_BPS
}

const fn default_works_stipend_per_unit_cred() -> Units {
    DEFAULT_WORKS_STIPEND_PER_UNIT_CRED
}

const fn default_mandate_alpha_bps() -> Bps {
    DEFAULT_MANDATE_ALPHA_BPS
}

const fn default_cancel_lock_bps() -> Bps {
    DEFAULT_CANCEL_LOCK_BPS
}

const fn default_wash_round_trip_min_units() -> Units {
    DEFAULT_WASH_ROUND_TRIP_MIN_UNITS
}

const fn default_wash_freeze_bond_cred() -> Units {
    DEFAULT_WASH_FREEZE_BOND_CRED
}

const fn default_wash_freeze_bounty_cred() -> Units {
    DEFAULT_WASH_FREEZE_BOUNTY_CRED
}

/// Computes `ceil(amount * asset_parity * premium_bps / (UNIT_SCALE * BPS_SCALE))` for Phase-1
/// `TownImport` orders (`>= 1 CRED`), rejecting non-positive quantities or non-premium rates (`<= 1.0x`).
pub fn town_import_cost_cred(
    amount: Units,
    asset_parity: Units,
    premium_bps: Bps,
) -> Result<Units, EngineError> {
    if amount <= 0 || asset_parity <= 0 {
        return Err(EngineError::InvalidOrder(
            "town import amount and parity must be positive".to_owned(),
        ));
    }
    if i128::from(premium_bps) <= BPS_SCALE {
        return Err(EngineError::InvalidConfig(format!(
            "town_import_premium_bps must exceed 10_000 (1.0x), got {premium_bps}"
        )));
    }
    let den = (UNIT_SCALE as i128) * BPS_SCALE;
    let num = (amount as i128)
        .checked_mul(asset_parity as i128)
        .and_then(|v| v.checked_mul(i128::from(premium_bps)))
        .ok_or_else(|| EngineError::InvalidOrder("town import cost overflow".to_owned()))?;
    let cost = num
        .checked_add(den - 1)
        .map(|v| v / den)
        .ok_or_else(|| EngineError::InvalidOrder("town import cost overflow".to_owned()))?;
    Units::try_from(cost.max(1))
        .map_err(|_| EngineError::InvalidOrder("town import cost exceeds i64".to_owned()))
}

const fn clamp_i128_to_units(value: i128) -> Units {
    if value > Units::MAX as i128 {
        Units::MAX
    } else if value < Units::MIN as i128 {
        Units::MIN
    } else {
        value as Units
    }
}

/// Computes `ceil(amount * bps / 10_000)` using `i128` intermediates, saturating into `Units` bounds.
#[must_use]
pub const fn ceil_bps_fee(amount: Units, bps: Bps) -> Units {
    if amount <= 0 || bps == 0 {
        return 0;
    }
    let num = (amount as i128) * (bps as i128) + (BPS_SCALE - 1);
    clamp_i128_to_units(num / BPS_SCALE)
}

/// Computes `floor(amount * bps / 10_000)` clamped into `0..=amount`.
#[must_use]
pub const fn floor_bps_part(amount: Units, bps: Bps) -> Units {
    if amount <= 0 || bps == 0 {
        return 0;
    }
    let part = clamp_i128_to_units(((amount as i128) * (bps as i128)) / BPS_SCALE);
    if part > amount { amount } else { part }
}

/// Computes half-up `round(amount * bps / 10_000)` using `i128` intermediates, saturating cleanly.
#[must_use]
pub const fn half_up_bps(amount: Units, bps: Bps) -> Units {
    if amount <= 0 || bps == 0 {
        return 0;
    }
    clamp_i128_to_units(((amount as i128) * (bps as i128) + (BPS_SCALE / 2)) / BPS_SCALE)
}

/// Computes `floor(amount * parity_per_unit / UNIT_SCALE)` for positive valuations, saturating cleanly.
#[must_use]
pub const fn mul_units_parity(amount: Units, parity_per_unit: Units) -> Units {
    if amount == 0 || parity_per_unit == 0 {
        return 0;
    }
    clamp_i128_to_units(((amount as i128) * (parity_per_unit as i128)) / (UNIT_SCALE as i128))
}

/// Computes `ceil(shortfall * parity_per_unit / UNIT_SCALE)` for debt conversions, saturating cleanly.
#[must_use]
pub const fn ceil_units_parity(shortfall: Units, parity_per_unit: Units) -> Units {
    if shortfall <= 0 || parity_per_unit <= 0 {
        return 0;
    }
    let scale = UNIT_SCALE as i128;
    clamp_i128_to_units(((shortfall as i128) * (parity_per_unit as i128) + (scale - 1)) / scale)
}

/// Checked `floor(amount * parity_per_unit / UNIT_SCALE)` returning `Err(InvalidOrder)` on negative inputs or `i64` overflow.
pub fn checked_mul_units_parity(
    amount: Units,
    parity_per_unit: Units,
) -> Result<Units, EngineError> {
    if amount < 0 || parity_per_unit < 0 {
        return Err(EngineError::InvalidOrder(
            "negative units or price in units multiplication".to_owned(),
        ));
    }
    if amount == 0 || parity_per_unit == 0 {
        return Ok(0);
    }
    let raw = ((amount as i128) * (parity_per_unit as i128)) / (UNIT_SCALE as i128);
    Units::try_from(raw).map_err(|_| EngineError::InvalidOrder("units * price overflow".to_owned()))
}

/// Checked `ceil(shortfall * parity_per_unit / UNIT_SCALE)` returning `Err(InvalidOrder)` on negative inputs or `i64` overflow.
pub fn checked_ceil_units_parity(
    shortfall: Units,
    parity_per_unit: Units,
) -> Result<Units, EngineError> {
    if shortfall < 0 || parity_per_unit < 0 {
        return Err(EngineError::InvalidOrder(
            "negative units or price in ceil units multiplication".to_owned(),
        ));
    }
    if shortfall == 0 || parity_per_unit == 0 {
        return Ok(0);
    }
    let scale = UNIT_SCALE as i128;
    let raw = ((shortfall as i128) * (parity_per_unit as i128) + (scale - 1)) / scale;
    Units::try_from(raw)
        .map_err(|_| EngineError::InvalidOrder("ceil units * price overflow".to_owned()))
}

/// Scales a signed equity potential by a basis-point discount factor (`gamma <= 10_000`), rounding toward `-infinity`.
#[must_use]
pub const fn scale_bps_signed(value: i64, gamma_bps: Bps) -> i64 {
    if gamma_bps == 10_000 {
        return value;
    }
    clamp_i128_to_units(((value as i128) * (gamma_bps as i128)).div_euclid(BPS_SCALE))
}

/// Scheduled 3-stage Horizon Shock (`hint_turn -> forecast_turn -> realize_turn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledShockSpec {
    /// Turn when private preimage/hint becomes available to the `OracleAuditor`.
    pub hint_turn: u8,
    /// Turn (`> hint_turn`) when narrowed public forecast posts in Sweep Step 8.
    pub forecast_turn: u8,
    /// Turn (`> forecast_turn`, `<= 23`) when realized shock activates in Sweep Step 8.
    pub realize_turn: u8,
    /// Target seat (`0..seat_count`).
    pub target_seat: u8,
    /// Effect kind.
    pub kind: ShockKind,
    /// Shock strength (`1..=10_000 bps`).
    pub magnitude_bps: Bps,
}

impl league_seat_types::ParitySpecSource for TableSpec {
    fn spec_seat_count(&self) -> u8 {
        self.seat_count
    }

    fn spec_seats(&self) -> &[SeatConfig] {
        &self.seats
    }
}

/// Inclusive upper bound of the `dev` layout-seed pool (`0..=9_999`, matching `league_gen::DEV_SEED_MAX`).
pub const DEV_LAYOUT_SEED_MAX: u64 = 9_999;

/// Reserved probe, held-out, and sealed test layout seeds forbidden for real or Club tables
/// (matching `league_gen::is_reserved_test_seed`).
pub const RESERVED_TEST_LAYOUT_SEEDS: [u64; 7] = [
    1_000_000,
    1_099_999,
    2_000_000,
    2_099_999,
    1_u64 << 63,
    u64::MAX,
    (1_u64 << 63) | 42,
];

/// Returns `true` when `seed` is one of the reserved test layout seeds that must never be
/// scheduled for Club or real tables.
#[must_use]
pub const fn is_reserved_layout_test_seed(seed: u64) -> bool {
    let mut idx = 0;
    while idx < RESERVED_TEST_LAYOUT_SEEDS.len() {
        if seed == RESERVED_TEST_LAYOUT_SEEDS[idx] {
            return true;
        }
        idx += 1;
    }
    false
}

/// Validates that `seed` lies in clear `SeedPool::Dev` (`0..=9_999`) and is not a reserved test
/// seed before starting or replaying a [`TableMode::Club`] table.
///
/// Sealed-range values (`>= 2^63`) are redacted to their SHA-256 digest in error messages.
pub fn validate_club_dev_seed(seed: u64) -> Result<(), EngineError> {
    if is_reserved_layout_test_seed(seed) {
        let desc = if seed >= (1_u64 << 63) {
            format!(
                "sealed-range digest={}",
                crate::replay::layout_seed_digest(seed)
            )
        } else {
            seed.to_string()
        };
        return Err(EngineError::InvalidConfig(format!(
            "RESERVED_SEED_FORBIDDEN: layout seed {desc} is a reserved test seed and cannot be used for TableMode::Club"
        )));
    }
    if seed > DEV_LAYOUT_SEED_MAX {
        let desc = if seed >= (1_u64 << 63) {
            format!(
                "sealed-range seed (digest={})",
                crate::replay::layout_seed_digest(seed)
            )
        } else {
            seed.to_string()
        };
        return Err(EngineError::InvalidConfig(format!(
            "SEED_POOL_MISMATCH: TableMode::Club requires clear SeedPool::Dev (0..={DEV_LAYOUT_SEED_MAX}), got {desc}"
        )));
    }
    Ok(())
}

/// Full deterministic rule specification for a v2 chiral table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSpec {
    /// Unique table identifier.
    pub table_id: TableId,
    /// Required execution mode (`Game`, `Eval`, or `Club`) binding live sweeps and replay logs.
    pub mode: TableMode,
    /// Number of seats (`4`, `5`, or `6`).
    pub seat_count: u8,
    /// Per-seat economic definitions in ascending index order `0..seat_count`.
    pub seats: Vec<SeatConfig>,
    /// Available Tier-3 Syndicate Project sinks.
    pub projects: Vec<ProjectRecipe>,
    /// Optional fixture-only 3-stage `OracleSet` shocks (must be empty in `TableMode::Game` and
    /// on standard/replayed tables, where shocks draw dynamically from per-turn HKDF beacons).
    pub shocks: Vec<ScheduledShockSpec>,
    /// Stochastic season closing hazards (in bps) evaluated on Turns `24..=31` before hard cap `32`.
    pub closing_hazard_bps: [Bps; CLOSING_ROLL_WINDOW_LEN],
    /// Public graded Turn-1 mandate deck defining valid cards and `3–8` point bonuses (`T5`).
    pub mandate_deck: Vec<GradedMandateCard>,
    /// Pre-dealt Turn-1 mandate commitments indexed by seat (`0..seat_count`).
    pub dealt_mandate_commits: Vec<CommitmentHash>,
    /// Optional pre-named drand `quicknet` close schedule required for `ExternalDrand` tables (`M2`).
    #[serde(default)]
    pub drand_schedule: Option<DrandSchedule>,
    /// Optional `ristretto255` VRF public key (`PK_eval`, RFC 9381 ECVRF construction with C2SP's
    /// `c2sp.org/vrf-r255` ciphersuite, `vrf-r255 0.1.0`) binding Eval-mode VRF randomness proofs.
    #[serde(default, with = "opt_hex32")]
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Provisional town-import premium multiplier in basis points (`DEFAULT_TOWN_IMPORT_PREMIUM_BPS`).
    #[serde(default = "default_town_import_premium_bps")]
    pub town_import_premium_bps: Bps,
    /// Provisional `CRED` stipend per `1_000` milli-units of refined goods delivered to Works
    /// (`DEFAULT_WORKS_STIPEND_PER_UNIT_CRED`).
    #[serde(default = "default_works_stipend_per_unit_cred")]
    pub works_stipend_per_unit_cred: Units,
    /// Provisional mandate-point conversion factor `alpha` in bps of `max(Med(W), 0.5 * W_0)`
    /// (`DEFAULT_MANDATE_ALPHA_BPS = 250` = `2.5%`).
    #[serde(default = "default_mandate_alpha_bps")]
    pub mandate_alpha_bps: Bps,
    /// Provisional cancel-lock fraction of Phase-1 duration (`DEFAULT_CANCEL_LOCK_BPS = 2_500`;
    /// `0` disables the lock; `C4` amended `2026-11-17`, pending playtest calibration).
    #[serde(default = "default_cancel_lock_bps")]
    pub cancel_lock_bps: Bps,
    /// Provisional minimum per-direction commodity flow in one turn confirming a Customs Inspector
    /// wash-freeze round-trip audit (`DEFAULT_WASH_ROUND_TRIP_MIN_UNITS = 1_000`; `§5.2` amended
    /// `2026-11-17`, pending playtest calibration).
    #[serde(default = "default_wash_round_trip_min_units")]
    pub wash_round_trip_min_units: Units,
    /// Provisional challenger bond posted upfront by `CustomsInspector` on `FlagWashFreeze`
    /// (`DEFAULT_WASH_FREEZE_BOND_CRED = 500`; `§5.2` amended `2026-11-17`, pending playtest
    /// calibration). The bond is the one source of truth for both outcomes: a clean audit pays
    /// all of it to the frozen seat as damages, and a confirmed audit returns all of it to the
    /// Inspector.
    #[serde(default = "default_wash_freeze_bond_cred")]
    pub wash_freeze_bond_cred: Units,
    /// Provisional bounty transferred from confirmed wash target free `CRED` (capped at balance)
    /// to `CustomsInspector` (`DEFAULT_WASH_FREEZE_BOUNTY_CRED = 500`; `§5.2` amended `2026-11-17`,
    /// pending playtest calibration).
    #[serde(default = "default_wash_freeze_bounty_cred")]
    pub wash_freeze_bounty_cred: Units,
}

impl TableSpec {
    /// Validates structural invariants of the table specification before starting an engine.
    pub fn validate(&self) -> Result<(), EngineError> {
        if !(4..=6).contains(&self.seat_count) {
            return Err(EngineError::InvalidConfig(format!(
                "seat_count must be 4..=6, got {}",
                self.seat_count
            )));
        }
        let n = usize::from(self.seat_count);
        if self.seats.len() != n {
            return Err(EngineError::InvalidConfig(format!(
                "expected {n} seats, got {}",
                self.seats.len()
            )));
        }
        for (idx, seat) in self.seats.iter().enumerate() {
            validate_seat_config(seat, idx, self.seat_count)?;
        }
        if self.projects.is_empty() {
            return Err(EngineError::InvalidConfig(
                "table must define at least one Tier-3 project".to_owned(),
            ));
        }
        let mut seen_proj = BTreeSet::new();
        for proj in &self.projects {
            if !seen_proj.insert(proj.id) || proj.inputs.is_empty() || proj.equity_points <= 0 {
                return Err(EngineError::InvalidConfig(format!(
                    "invalid project recipe {}",
                    proj.id
                )));
            }
            for &(good, amt) in &proj.inputs {
                if good >= self.seat_count || amt <= 0 {
                    return Err(EngineError::InvalidConfig(format!(
                        "project {} has invalid input ({good}, {amt})",
                        proj.id
                    )));
                }
            }
        }
        for &hz in &self.closing_hazard_bps {
            if hz as i128 > BPS_SCALE {
                return Err(EngineError::InvalidConfig(format!(
                    "closing hazard {hz} bps exceeds 10_000"
                )));
            }
        }
        for shock in &self.shocks {
            if shock.target_seat >= self.seat_count
                || shock.hint_turn == 0
                || !(shock.hint_turn < shock.forecast_turn
                    && shock.forecast_turn < shock.realize_turn)
                || shock.realize_turn > 23
                || shock.magnitude_bps == 0
                || shock.magnitude_bps as i128 > BPS_SCALE
            {
                return Err(EngineError::InvalidConfig(
                    "invalid HorizonShock schedule parameters".to_owned(),
                ));
            }
        }
        if self.dealt_mandate_commits.len() != n {
            return Err(EngineError::InvalidConfig(format!(
                "expected {n} dealt_mandate_commits, got {}",
                self.dealt_mandate_commits.len()
            )));
        }
        for (seat_idx, commit) in self.dealt_mandate_commits.iter().enumerate() {
            if commit.is_degenerate() {
                return Err(EngineError::InvalidConfig(format!(
                    "degenerate all-zero dealt_mandate_commit for seat {seat_idx}"
                )));
            }
        }
        match self.mode {
            TableMode::Game => {
                let Some(schedule) = self.drand_schedule else {
                    return Err(EngineError::InvalidConfig(
                        "Game mode TableSpec requires drand_schedule".to_owned(),
                    ));
                };
                schedule.validate()?;
                if !self.shocks.is_empty() {
                    return Err(EngineError::InvalidConfig(
                        "Game mode TableSpec forbids pre-seeded shocks; shocks must draw from logged drand beacons".to_owned(),
                    ));
                }
                if self.eval_vrf_pubkey.is_some() {
                    return Err(EngineError::InvalidConfig(
                        "Game mode TableSpec must not carry eval_vrf_pubkey".to_owned(),
                    ));
                }
            }
            TableMode::Eval => {
                if let Some(schedule) = self.drand_schedule {
                    schedule.validate()?;
                }
                if let Some(pk) = self.eval_vrf_pubkey {
                    validate_eval_vrf_pubkey(pk)?;
                }
            }
            TableMode::Club => {
                if self.drand_schedule.is_some() {
                    return Err(EngineError::InvalidConfig(
                        "Club mode TableSpec must not carry drand_schedule".to_owned(),
                    ));
                }
                if !self.shocks.is_empty() {
                    return Err(EngineError::InvalidConfig(
                        "Club mode TableSpec forbids pre-seeded shocks; shocks must draw from verified EvalVrf beacons".to_owned(),
                    ));
                }
                let Some(pk) = self.eval_vrf_pubkey else {
                    return Err(EngineError::InvalidConfig(
                        "Club mode TableSpec requires eval_vrf_pubkey".to_owned(),
                    ));
                };
                validate_eval_vrf_pubkey(pk)?;
            }
        }
        if self.mandate_alpha_bps == 0 || i128::from(self.mandate_alpha_bps) > BPS_SCALE {
            return Err(EngineError::InvalidConfig(format!(
                "mandate_alpha_bps must lie in 1..=10_000, got {}",
                self.mandate_alpha_bps
            )));
        }
        if self.mandate_deck.len() < n {
            return Err(EngineError::InvalidConfig(format!(
                "mandate_deck size ({}) must be >= seat_count ({n})",
                self.mandate_deck.len()
            )));
        }
        let mut seen_cards = BTreeSet::new();
        for card in &self.mandate_deck {
            if !seen_cards.insert(card.card_id)
                || card.difficulty_completion_bps > 10_000
                || !(MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY)
                    .contains(&card.bonus_equity)
            {
                return Err(EngineError::InvalidConfig(format!(
                    "invalid graded mandate card {}",
                    card.card_id
                )));
            }
            card.kind.validate_for_seat_count(self.seat_count)?;
        }
        if i128::from(self.town_import_premium_bps) <= BPS_SCALE {
            return Err(EngineError::InvalidConfig(format!(
                "town_import_premium_bps must exceed 10_000 (1.0x), got {}",
                self.town_import_premium_bps
            )));
        }
        let min_ref_val = self
            .seats
            .iter()
            .map(|s| s.refined_project_value)
            .min()
            .unwrap_or(0);
        if self.works_stipend_per_unit_cred < 0 || self.works_stipend_per_unit_cred >= min_ref_val {
            return Err(EngineError::InvalidConfig(format!(
                "works_stipend_per_unit_cred {} must lie in 0..{min_ref_val}",
                self.works_stipend_per_unit_cred
            )));
        }
        if i128::from(self.cancel_lock_bps) > BPS_SCALE {
            return Err(EngineError::InvalidConfig(format!(
                "cancel_lock_bps must lie in 0..={BPS_SCALE}, got {}",
                self.cancel_lock_bps
            )));
        }
        if self.wash_round_trip_min_units <= 0 {
            return Err(EngineError::InvalidConfig(format!(
                "wash_round_trip_min_units must be > 0, got {}",
                self.wash_round_trip_min_units
            )));
        }
        if self.wash_freeze_bond_cred <= 0 {
            return Err(EngineError::InvalidConfig(format!(
                "wash_freeze_bond_cred must be > 0, got {}",
                self.wash_freeze_bond_cred
            )));
        }
        if self.wash_freeze_bounty_cred < 0 {
            return Err(EngineError::InvalidConfig(format!(
                "wash_freeze_bounty_cred must be >= 0, got {}",
                self.wash_freeze_bounty_cred
            )));
        }
        Ok(())
    }

    /// Constructs the canonical Club-mode `TableSpec` over a validated `SeedPool::Dev` seed
    /// (`0..=9_999`, `!is_reserved_layout_test_seed`) together with OS-CSPRNG Turn-1 mandates
    /// (`SealedMandateDeal`) and `SealedEvalSeed`.
    pub fn standard_club_chiral_ring_with_setup(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
    ) -> Result<(Self, SealedMandateDeal, SealedEvalSeed), EngineError> {
        validate_club_dev_seed(seed)?;
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) =
            deal_graded_mandates(table_id, seat_count, &mandate_deck)?;
        let eval_seed = SealedEvalSeed::generate()?;
        let eval_pk = eval_seed.public_key_bytes()?;
        let (mut spec, deal) = Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Club,
            None,
            Some(eval_pk),
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )?;
        for s in &mut spec.seats {
            s.regent = match s.identity {
                SeatIdentity::Human => Some(RegentDoctrine {
                    risk_tolerance_bps: 5_000,
                    min_raw_reserve: UNIT_SCALE,
                    auto_cash_due_claims: true,
                }),
                SeatIdentity::AiSyndicate => None,
            };
        }
        spec.validate()?;
        Ok((spec, deal, eval_seed))
    }

    /// Constructs the canonical Club-mode `TableSpec` from caller-supplied 256-bit `deal_seed`,
    /// per-seat `seat_salts`, and 256-bit `eval_seed` for reproducible Club tests, enforcing
    /// `SeedPool::Dev` (`0..=9_999`).
    pub fn standard_club_chiral_ring_with_explicit_setup(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
        deal_seed: [u8; 32],
        seat_salts: &[RevealSalt],
        eval_seed: SealedEvalSeed,
    ) -> Result<(Self, SealedMandateDeal, SealedEvalSeed), EngineError> {
        validate_club_dev_seed(seed)?;
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) = deal_graded_mandates_with_entropy(
            table_id,
            seat_count,
            deal_seed,
            seat_salts,
            &mandate_deck,
        )?;
        let eval_pk = eval_seed.public_key_bytes()?;
        let (mut spec, deal) = Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Club,
            None,
            Some(eval_pk),
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )?;
        for s in &mut spec.seats {
            s.regent = match s.identity {
                SeatIdentity::Human => Some(RegentDoctrine {
                    risk_tolerance_bps: 5_000,
                    min_raw_reserve: UNIT_SCALE,
                    auto_cash_due_claims: true,
                }),
                SeatIdentity::AiSyndicate => None,
            };
        }
        spec.validate()?;
        Ok((spec, deal, eval_seed))
    }

    /// Constructs the canonical Eval-mode `TableSpec` together with both sealed orchestrator
    /// artifacts: private Turn-1 mandates (`SealedMandateDeal`) and fresh 256-bit Eval randomness
    /// (`SealedEvalSeed`), both drawn from OS CSPRNG entropy and independent of `seed` (`#31`).
    pub fn standard_eval_chiral_ring_with_setup(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
    ) -> Result<(Self, SealedMandateDeal, SealedEvalSeed), EngineError> {
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) =
            deal_graded_mandates(table_id, seat_count, &mandate_deck)?;
        let eval_seed = SealedEvalSeed::generate()?;
        let eval_pk = eval_seed.public_key_bytes()?;
        let (spec, deal) = Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Eval,
            None,
            Some(eval_pk),
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )?;
        Ok((spec, deal, eval_seed))
    }

    /// Constructs the canonical Eval-mode `TableSpec` with no Standing Regent attached to any
    /// model-driven seat (`model_seats[i] == true` forces `regent: None`), while `ClubBot` seats
    /// (`model_seats[i] == false`) retain the Club-mode Regent policy by seat identity.
    pub fn standard_eval_chiral_ring_with_model_seats(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
        model_seats: &[bool],
    ) -> Result<(Self, SealedMandateDeal, SealedEvalSeed), EngineError> {
        if model_seats.len() != usize::from(seat_count) {
            return Err(EngineError::InvalidConfig(format!(
                "INVALID_MODEL_SEATS_LEN: expected {seat_count} model-seat flags, got {}",
                model_seats.len()
            )));
        }
        let (mut spec, deal, eval_seed) =
            Self::standard_eval_chiral_ring_with_setup(table_id, seat_count, seed)?;
        for (seat_cfg, &is_model) in spec.seats.iter_mut().zip(model_seats.iter()) {
            seat_cfg.regent = if is_model {
                None
            } else {
                match seat_cfg.identity {
                    SeatIdentity::Human => Some(RegentDoctrine {
                        risk_tolerance_bps: 5_000,
                        min_raw_reserve: UNIT_SCALE,
                        auto_cash_due_claims: true,
                    }),
                    SeatIdentity::AiSyndicate => None,
                }
            };
        }
        spec.validate()?;
        Ok((spec, deal, eval_seed))
    }

    /// Constructs the canonical Eval-mode `TableSpec` from caller-supplied 256-bit `deal_seed`,
    /// per-seat `seat_salts`, and 256-bit `eval_seed` for reproducible Eval test fixtures.
    pub fn standard_eval_chiral_ring_with_explicit_setup(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
        deal_seed: [u8; 32],
        seat_salts: &[RevealSalt],
        eval_seed: SealedEvalSeed,
    ) -> Result<(Self, SealedMandateDeal, SealedEvalSeed), EngineError> {
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) = deal_graded_mandates_with_entropy(
            table_id,
            seat_count,
            deal_seed,
            seat_salts,
            &mandate_deck,
        )?;
        let eval_pk = eval_seed.public_key_bytes()?;
        let (spec, deal) = Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Eval,
            None,
            Some(eval_pk),
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )?;
        Ok((spec, deal, eval_seed))
    }

    /// Constructs a standalone Eval-mode canonical `TableSpec` parameterized by public layout
    /// `seed`, dealing private Turn-1 mandates from fresh OS CSPRNG entropy (`M3`) without
    /// binding an orchestrator VRF keypair (for VRF-bound Eval tables and replay logs, use
    /// [`TableSpec::standard_eval_chiral_ring_with_setup`]).
    pub fn standard_chiral_ring_with_mandates(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
    ) -> Result<(Self, SealedMandateDeal), EngineError> {
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) =
            deal_graded_mandates(table_id, seat_count, &mandate_deck)?;
        Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Eval,
            None,
            None,
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )
    }

    /// Constructs the canonical Game-mode `TableSpec` requiring a pre-named `DrandSchedule`
    /// (`M2`, `#31`), dealing private Turn-1 mandates from fresh OS CSPRNG entropy (`M3`).
    pub fn standard_game_chiral_ring_with_mandates(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
        drand_schedule: DrandSchedule,
    ) -> Result<(Self, SealedMandateDeal), EngineError> {
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) =
            deal_graded_mandates(table_id, seat_count, &mandate_deck)?;
        Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Game,
            Some(drand_schedule),
            None,
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )
    }

    /// Constructs the canonical Eval-mode `TableSpec` using caller-supplied `deal_seed` and
    /// `seat_salts` for deterministic fixture/golden tests without coupling mandates to a replay
    /// `seed: u64`.
    pub fn standard_chiral_ring_with_explicit_deal(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
        deal_seed: [u8; 32],
        seat_salts: &[RevealSalt],
    ) -> Result<(Self, SealedMandateDeal), EngineError> {
        let mandate_deck = default_graded_mandate_deck(seat_count);
        let (dealt_mandate_commits, sealed_deal) = deal_graded_mandates_with_entropy(
            table_id,
            seat_count,
            deal_seed,
            seat_salts,
            &mandate_deck,
        )?;
        Self::build_chiral_ring_spec_with_deal(
            table_id,
            TableMode::Eval,
            None,
            None,
            seat_count,
            seed,
            mandate_deck,
            dealt_mandate_commits,
            sealed_deal,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_chiral_ring_spec_with_deal(
        table_id: TableId,
        mode: TableMode,
        drand_schedule: Option<DrandSchedule>,
        eval_vrf_pubkey: Option<[u8; 32]>,
        seat_count: u8,
        seed: u64,
        mandate_deck: Vec<GradedMandateCard>,
        dealt_mandate_commits: Vec<CommitmentHash>,
        sealed_deal: SealedMandateDeal,
    ) -> Result<(Self, SealedMandateDeal), EngineError> {
        if !(4..=6).contains(&seat_count) {
            return Err(EngineError::InvalidConfig(format!(
                "seat_count must be 4..=6, got {seat_count}"
            )));
        }
        let n = usize::from(seat_count);
        let mut seats = Vec::with_capacity(n);
        let mut projects = Vec::with_capacity(n + 2);
        let spread_mod = (seed % 5) as i64;

        for i in 0..n {
            let next_1 = ((i + 1) % n) as u8;
            let next_2 = ((i + 2) % n) as u8;
            let prev = ((i + n - 1) % n) as u8;
            let converters = vec![ConverterRecipe {
                owner: i as u8,
                inputs: vec![(next_1, 2_000), (next_2, 2_000)],
                output_good: i as u8,
                output_amount: 3_500,
                max_runs_per_turn: 4,
                is_secondary: false,
            }];
            seats.push(SeatConfig {
                seat: i as u8,
                identity: if i == 0 {
                    SeatIdentity::Human
                } else {
                    SeatIdentity::AiSyndicate
                },
                initial_cred: 15_000 + spread_mod * 100,
                raw_yield_per_turn: 10_000,
                refined_project_value: 2_000,
                upkeep_good: prev,
                upkeep_amount: 1_800,
                initial_raw_tariff_bps: 800,
                initial_refined_tariff_bps: 600,
                perishable_decay_bps: 2_000,
                regent: Some(RegentDoctrine {
                    risk_tolerance_bps: 5_000,
                    min_raw_reserve: 2_000,
                    auto_cash_due_claims: true,
                }),
                converters,
            });
            // Pairwise-adjacent Syndicate Project + solo surplus sink.
            projects.push(ProjectRecipe {
                id: i as u16 + 1,
                inputs: vec![(i as u8, 1_000), (next_1, 1_000)],
                equity_points: 4_800,
            });
        }
        projects.push(ProjectRecipe {
            id: 100,
            inputs: (0..seat_count).map(|s| (s, 500)).collect(),
            equity_points: (seat_count as Units) * 1_400,
        });

        let spec = Self {
            table_id,
            mode,
            seat_count,
            seats,
            projects,
            shocks: Vec::new(),
            closing_hazard_bps: DEFAULT_CLOSING_HAZARD_BPS,
            mandate_deck,
            dealt_mandate_commits,
            drand_schedule,
            eval_vrf_pubkey,
            town_import_premium_bps: DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
            works_stipend_per_unit_cred: DEFAULT_WORKS_STIPEND_PER_UNIT_CRED,
            mandate_alpha_bps: DEFAULT_MANDATE_ALPHA_BPS,
            cancel_lock_bps: DEFAULT_CANCEL_LOCK_BPS,
            wash_round_trip_min_units: DEFAULT_WASH_ROUND_TRIP_MIN_UNITS,
            wash_freeze_bond_cred: DEFAULT_WASH_FREEZE_BOND_CRED,
            wash_freeze_bounty_cred: DEFAULT_WASH_FREEZE_BOUNTY_CRED,
        };
        spec.validate()?;
        Ok((spec, sealed_deal))
    }

    /// Constructs a canonical balanced chiral converter-ring [`TableSpec`] for tests and
    /// standalone evaluation, parameterized by public layout `seed` (`seat_count` in `4..=6`),
    /// with private mandates dealt from OS CSPRNG entropy.
    pub fn standard_chiral_ring(
        table_id: TableId,
        seat_count: u8,
        seed: u64,
    ) -> Result<Self, EngineError> {
        Ok(Self::standard_chiral_ring_with_mandates(table_id, seat_count, seed)?.0)
    }
}

fn validate_seat_config(
    seat: &SeatConfig,
    expected_idx: usize,
    seat_count: u8,
) -> Result<(), EngineError> {
    if usize::from(seat.seat) != expected_idx {
        return Err(EngineError::InvalidConfig(format!(
            "seat index mismatch: expected {expected_idx}, got {}",
            seat.seat
        )));
    }
    if seat.initial_cred < 0
        || seat.raw_yield_per_turn <= 0
        || seat.refined_project_value <= 0
        || seat.upkeep_amount <= 0
    {
        return Err(EngineError::InvalidConfig(format!(
            "seat {expected_idx} has non-positive required economic rate"
        )));
    }
    if seat.upkeep_good >= seat_count || seat.upkeep_good == seat.seat {
        return Err(EngineError::InvalidConfig(format!(
            "seat {expected_idx} upkeep_good must reference another seat"
        )));
    }
    if seat.initial_raw_tariff_bps > MAX_TARIFF_BPS
        || seat.initial_refined_tariff_bps > MAX_TARIFF_BPS
        || seat.perishable_decay_bps >= 10_000
    {
        return Err(EngineError::InvalidConfig(format!(
            "seat {expected_idx} tariff or decay rate out of range"
        )));
    }
    if seat.converters.is_empty() {
        return Err(EngineError::InvalidConfig(format!(
            "seat {expected_idx} must own at least one converter"
        )));
    }
    for conv in &seat.converters {
        if conv.owner != seat.seat
            || conv.output_good != seat.seat
            || conv.output_amount <= 0
            || conv.max_runs_per_turn == 0
            || conv.inputs.is_empty()
        {
            return Err(EngineError::InvalidConfig(format!(
                "invalid converter ownership or yield on seat {expected_idx}"
            )));
        }
        let mut seen_inputs = BTreeSet::new();
        for &(raw_g, amt) in &conv.inputs {
            if raw_g >= seat_count || amt <= 0 {
                return Err(EngineError::InvalidConfig(format!(
                    "invalid converter input ({raw_g}, {amt}) on seat {expected_idx}"
                )));
            }
            if !conv.is_secondary && (raw_g == seat.seat || !seen_inputs.insert(raw_g)) {
                return Err(EngineError::InvalidConfig(format!(
                    "primary converter on seat {expected_idx} requires distinct foreign inputs (got {raw_g})"
                )));
            }
        }
    }
    Ok(())
}
