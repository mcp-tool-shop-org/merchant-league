//! Core domain types, integer fixed-point arithmetic helpers, and table specifications
//! for Merchant League v2.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::commit::{
    CommitmentHash, DrandSchedule, GradedMandateCard, RevealSalt, SealedEvalSeed,
    SealedMandateDeal, deal_graded_mandates, deal_graded_mandates_with_entropy,
    default_graded_mandate_deck, validate_eval_vrf_pubkey,
};

mod opt_hex32 {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &Option<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serializer.serialize_some(&hex::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        let opt = Option::<String>::deserialize(deserializer)?;
        match opt {
            None => Ok(None),
            Some(s) => {
                let decoded = hex::decode(&s).map_err(serde::de::Error::custom)?;
                if decoded.len() != 32 {
                    return Err(serde::de::Error::custom("expected 32 hex-encoded bytes"));
                }
                let mut out = [0u8; 32];
                out.copy_from_slice(&decoded);
                Ok(Some(out))
            }
        }
    }
}

/// Integer milli-unit quantity (`1_000 == 1.000` unit of `CRED`, commodity, or equity).
pub type Units = i64;

/// Scaling factor for fixed-point milli-unit quantities (`1_000 == 1.000`).
pub const UNIT_SCALE: Units = 1_000;

/// Rate expressed in integer basis points (`10_000 == 100.00%`).
pub type Bps = u16;

/// Scaling factor for basis points (`10_000 == 100.00%`).
pub const BPS_SCALE: i128 = 10_000;

/// Maximum political commodity export tariff (`35.00%` = `3_500 bps`).
pub const MAX_TARIFF_BPS: Bps = 3_500;

/// First turn on which the stochastic season closing bell roll runs (`Turn 24`; Turns `1..=23` stay open).
pub const CLOSING_ROLL_START_TURN: u8 = 24;

/// Hard terminal turn cap (`Turn 32`, `2026-11-15` ruling; unconditional close at Sweep Step 8).
pub const HARD_TURN_CAP: u8 = 32;

/// Number of stochastic closing-roll turns (`24..=31`, `8` turns before [`HARD_TURN_CAP`]).
pub const CLOSING_ROLL_WINDOW_LEN: usize = (HARD_TURN_CAP - CLOSING_ROLL_START_TURN) as usize;

/// Provisional per-turn closing bell hazard probabilities (in bps) for turns `24..=31`
/// (`35.00%` per turn -> expected season length `~25.80` turns and `~3.19%` cap arrival at Turn 32;
/// `2026-11-15` ruling, Seed-28 Report §4, pending playtest calibration).
pub const DEFAULT_CLOSING_HAZARD_BPS: [Bps; CLOSING_ROLL_WINDOW_LEN] =
    [3_500; CLOSING_ROLL_WINDOW_LEN];

/// Provisional mandate-point scaling factor `alpha = 0.025` (`250 bps = 2.5%` of
/// `max(Med(W), 0.5 * W_0)` per `1.000` mandate point; `2026-11-15` ruling, Seed-28 Report §6
/// Formula 3, pending playtest calibration).
pub const DEFAULT_MANDATE_ALPHA_BPS: Bps = 250;

/// Starting net-worth floor fraction (`5_000 bps = 50.00%` of `W_0`, the table-median starting
/// net worth) applied inside `max(Med(W), 0.5 * W_0)` when converting mandate points (`Seed-28
/// Report` §6 Formula 3).
pub const MANDATE_STARTING_NW_FLOOR_BPS: Bps = 5_000;

/// Provisional lookback window (`3` turns: `[T-2, T-1, T]`) for terminal volume-weighted holding
/// marks (`2026-11-15` and `2026-11-16` rulings).
pub const DEFAULT_TWAP_WINDOW_TURNS: u8 = 3;

/// Provisional minimum per-turn executed volume fraction (`200 bps = 2.00%` of total units of
/// that good held across all seats at the start of that turn) required in **every** turn of
/// `[T-2, T-1, T]` for a good to qualify for volume-weighted close marking (`2026-11-16` ruling).
pub const DEFAULT_TWAP_MIN_TURN_STOCK_BPS: Bps = 200;

/// Fixed salvage fraction (`5_000 bps = 50.00%` of base value / parity) applied at Season Close
/// to unsold units above executable non-self resting bid depth when a good does not qualify for
/// the 3-turn volume-weighted mark (`2026-11-16` ruling).
pub const UNSOLD_EXCESS_PARITY_BPS: Bps = 5_000;

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

/// Minimum Turn-1 Private Mandate bank bonus (`3.000 VP`).
pub const MIN_MANDATE_BONUS_EQUITY: Units = 3 * UNIT_SCALE;

/// Maximum Turn-1 Private Mandate bank bonus (`8.000 VP`).
pub const MAX_MANDATE_BONUS_EQUITY: Units = 8 * UNIT_SCALE;

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

const fn default_town_import_premium_bps() -> Bps {
    DEFAULT_TOWN_IMPORT_PREMIUM_BPS
}

const fn default_works_stipend_per_unit_cred() -> Units {
    DEFAULT_WORKS_STIPEND_PER_UNIT_CRED
}

const fn default_mandate_alpha_bps() -> Bps {
    DEFAULT_MANDATE_ALPHA_BPS
}

const fn default_twap_window_turns() -> u8 {
    DEFAULT_TWAP_WINDOW_TURNS
}

const fn default_twap_min_turn_stock_bps() -> Bps {
    DEFAULT_TWAP_MIN_TURN_STOCK_BPS
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

/// Identifies a table: one game of 4 to 6 merchants in v2 (or 2 to 6 legacy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TableId(pub u64);

/// A merchant's seat at a table, counted from 0 in seating order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Seat(pub u8);

/// A market day within a table, counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MarketDay(pub u16);

/// A season: the play identifier grouping tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeasonId(pub u32);

/// A simultaneous turn index within a season (`1..=32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Turn(pub u8);

/// Asset identifier for currencies and commodities at a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AssetId {
    /// Table numeraire coin (`CRED`).
    Cred,
    /// Seat `i` exclusive raw commodity `R_i` (perishable).
    Raw(u8),
    /// Seat `i` exclusive refined commodity `M_i`.
    Refined(u8),
}

impl AssetId {
    /// Returns `true` when this asset is perishable (`Raw(_)`) and ineligible for `TokenEscrow`.
    #[must_use]
    pub const fn is_perishable(self) -> bool {
        matches!(self, Self::Raw(_))
    }
}

/// On-ledger credit rating credential governing auto-debt rates and prime cartel access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CreditTier {
    /// Highest credit rating (`4.00%` / turn auto-debt rate).
    Aaa,
    /// Intermediate rating (`10.00%` / turn auto-debt rate).
    Bbb,
    /// Sub-prime rating (`18.00%` / turn auto-debt rate).
    B,
    /// Defaulted rating (`30.00%` / turn auto-debt rate; excluded from prime cartel domains).
    D,
}

impl CreditTier {
    /// Per-turn auto-debt interest rate in basis points.
    #[must_use]
    pub const fn interest_bps(self) -> Bps {
        match self {
            Self::Aaa => 400,
            Self::Bbb => 1_000,
            Self::B => 1_800,
            Self::D => 3_000,
        }
    }

    /// Steps credit tier down one notch upon a Hard Credit Default (`CheckCash` bounce).
    #[must_use]
    pub const fn downgrade_one(self) -> Self {
        match self {
            Self::Aaa => Self::Bbb,
            Self::Bbb => Self::B,
            Self::B | Self::D => Self::D,
        }
    }

    /// Returns `true` if this credit tier is permitted inside prime cartel domains.
    #[must_use]
    pub const fn may_access_prime_cartel(self) -> bool {
        !matches!(self, Self::D)
    }
}

/// Disclosed seat controller identity category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SeatIdentity {
    /// Human-controlled seat (`SEAT_HUMAN`), eligible for Standing Regent fallback.
    Human,
    /// Syndicate AI seat (`SEAT_AI_SYNDICATE`), disclosed or converted after 3 missed cutoffs.
    AiSyndicate,
}

/// Player-configured doctrine scoping a Standing Regent backup agent (`DelegateSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegentDoctrine {
    /// Willingness to consume input reserves when missed (`0..=10_000 bps`).
    pub risk_tolerance_bps: Bps,
    /// Minimum units of raw goods held back per input slot before Regent runs converters.
    pub min_raw_reserve: Units,
    /// Whether the Regent automatically presents due checks and maturing escrows for settlement.
    pub auto_cash_due_claims: bool,
}

/// Rotating institutional charter auctioned every 6 turns (`Turns 6, 12, 18, 24, 30`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CharterRole {
    /// Holds `tfHybrid` bridging privilege across cartel domains and open books.
    ClearinghouseBroker,
    /// Can flag suspected wash loops for temporary market freezes during `TalkAndMarket`.
    CustomsInspector,
    /// Receives Turn `t` private `HorizonShock` preimages encrypted to its `MessageKey`.
    OracleAuditor,
}

/// Three-stage telegraphed Horizon Shock category (`OracleSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ShockKind {
    /// Reduces extraction yield on `target_seat`'s raw good by `magnitude_bps`.
    SupplyFreeze,
    /// Reduces `target_seat`'s commodity export `TransferRate` tariff by `magnitude_bps` (`10_000` = 0% holiday).
    TariffHoliday,
    /// Boosts Syndicate Project Equity points on deliveries including `target_seat`'s refined good by `magnitude_bps` for 2 turns.
    DemandSurge,
    /// Increases remaining raw spoilage rate by `magnitude_bps` of the headroom to `10_000 bps`.
    SpoilageSpike,
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

/// Active shock currently modifying gameplay mechanics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveShock {
    /// Affected seat's commodity index.
    pub target_seat: u8,
    /// Shock category.
    pub kind: ShockKind,
    /// Strength in basis points.
    pub magnitude_bps: Bps,
    /// Final turn index (inclusive) on which this active shock still applies.
    pub expires_after_turn: u8,
}

/// Chiral Tier-2 converter recipe operated by one seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConverterRecipe {
    /// Authorized seat index (`0..seat_count`).
    pub owner: u8,
    /// Required `(raw_good_index, units_per_run)` pairs.
    pub inputs: Vec<(u8, Units)>,
    /// Produced refined good index (`owner`).
    pub output_good: u8,
    /// Refined units minted per batch run.
    pub output_amount: Units,
    /// Maximum runs allowed in a single Sweep.
    pub max_runs_per_turn: u32,
    /// `false` for primary `(i+1, i+2) -> M_i`, `true` for secondary cross-converters.
    pub is_secondary: bool,
}

/// Tier-3 Syndicate Project bundle recipe sinking refined goods into irreversible Project Equity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecipe {
    /// Unique project identifier within the table.
    pub id: u16,
    /// Required `(refined_good_index, units_per_batch)` burned to Authority.
    pub inputs: Vec<(u8, Units)>,
    /// Irreversible Project Equity points credited per batch.
    pub equity_points: Units,
}

/// Initial economic configuration for a single seat in a [`TableSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatConfig {
    /// Zero-based seat index (`0..seat_count`).
    pub seat: u8,
    /// Starting identity (`Human` or `AiSyndicate`).
    pub identity: SeatIdentity,
    /// Initial `CRED` working capital at Turn 1 start.
    pub initial_cred: Units,
    /// Base raw commodity units extracted each turn at Sweep Step 4.
    pub raw_yield_per_turn: Units,
    /// Fundamental project value per unit of `Refined(seat)`.
    pub refined_project_value: Units,
    /// Required upkeep commodity index (`(seat + seat_count - 1) % seat_count`).
    pub upkeep_good: u8,
    /// Mandatory units owed each turn at Sweep Step 5.
    pub upkeep_amount: Units,
    /// Initial export `TransferRate` tariff on `Raw(seat)` in basis points (`<= 3_500`).
    pub initial_raw_tariff_bps: Bps,
    /// Initial export `TransferRate` tariff on `Refined(seat)` in basis points (`<= 3_500`).
    pub initial_refined_tariff_bps: Bps,
    /// Baseline per-turn perishable `Clawback` decay rate in basis points (`< 10_000`).
    pub perishable_decay_bps: Bps,
    /// Optional Standing Regent doctrine (`None` disables automatic non-reveal fallback).
    pub regent: Option<RegentDoctrine>,
    /// Converter recipes owned by this seat.
    pub converters: Vec<ConverterRecipe>,
}

/// Execution mode recorded on [`TableSpec`] and [`crate::SeasonReplayLog`] (`2026-11-15` ruling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableMode {
    /// Live Game Mode: requires `drand_schedule` and drand-signature-bound beacons (`M2`, `#31`).
    Game,
    /// Headless Evaluation Mode: uses sealed 256-bit OS CSPRNG entropy (`SealedEvalSeed`) with
    /// proofs under the RFC 9381 ECVRF construction with C2SP's `ristretto255` ciphersuite
    /// (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not registered in RFC 9381) in replay logs (`#31`).
    Eval,
}

/// Full deterministic rule specification for a v2 chiral table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSpec {
    /// Unique table identifier.
    pub table_id: TableId,
    /// Required execution mode (`Game` or `Eval`) binding live sweeps and replay logs.
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
    /// Provisional terminal volume-weighted mark lookback window in turns (`DEFAULT_TWAP_WINDOW_TURNS = 3`).
    #[serde(default = "default_twap_window_turns")]
    pub twap_window_turns: u8,
    /// Provisional minimum per-turn traded volume as a basis-point share of start-of-turn table
    /// stock (`DEFAULT_TWAP_MIN_TURN_STOCK_BPS = 200` = `2.00%`) required in every turn of the
    /// window for a good to qualify for volume-weighted close marking (`2026-11-16` ruling).
    #[serde(default = "default_twap_min_turn_stock_bps")]
    pub twap_min_turn_stock_bps: Bps,
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
        }
        if self.mandate_alpha_bps == 0 || i128::from(self.mandate_alpha_bps) > BPS_SCALE {
            return Err(EngineError::InvalidConfig(format!(
                "mandate_alpha_bps must lie in 1..=10_000, got {}",
                self.mandate_alpha_bps
            )));
        }
        if self.twap_window_turns == 0 || self.twap_window_turns > HARD_TURN_CAP {
            return Err(EngineError::InvalidConfig(format!(
                "twap_window_turns must lie in 1..={HARD_TURN_CAP}, got {}",
                self.twap_window_turns
            )));
        }
        if self.twap_min_turn_stock_bps == 0 || i128::from(self.twap_min_turn_stock_bps) > BPS_SCALE
        {
            return Err(EngineError::InvalidConfig(format!(
                "twap_min_turn_stock_bps must lie in 1..=10_000, got {}",
                self.twap_min_turn_stock_bps
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
        Ok(())
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
            twap_window_turns: DEFAULT_TWAP_WINDOW_TURNS,
            twap_min_turn_stock_bps: DEFAULT_TWAP_MIN_TURN_STOCK_BPS,
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

/// Errors returned by [`crate::TurnEngine`] and [`crate::replay_season`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineError {
    /// Table specification failed structural validation.
    InvalidConfig(String),
    /// Caller attempted an operation in an incompatible turn phase or after close.
    InvalidPhase(String),
    /// Referenced seat index does not exist on the table.
    InvalidSeat(u8),
    /// Submitted market action, order, or reveal parameter is invalid.
    InvalidOrder(String),
    /// Referenced offer, check, escrow, domain, or project ID was not found.
    UnknownObject(String),
    /// Seat lacked permission (e.g., wrong owner, unauthorized payee, or customs freeze).
    Unauthorized(String),
    /// Attempted to lock a perishable raw commodity inside `TokenEscrow`.
    PerishableEscrowForbidden,
    /// Account lacked sufficient free `CRED` or commodity balance for the action.
    InsufficientBalance(String),
    /// Commitment already recorded for this seat in this phase.
    AlreadySubmitted(u8),
    /// Reveal payload and salt did not match the recorded SHA-256 commitment.
    CommitmentMismatch(u8),
    /// Seat lacks required cartel domain credential or `ClearinghouseBroker` bridge.
    DomainAccessDenied(String),
    /// Season replay log was incomplete or inconsistent with closed status.
    SeasonEnded(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid table config: {msg}"),
            Self::InvalidPhase(msg) => write!(f, "invalid turn phase: {msg}"),
            Self::InvalidSeat(seat) => write!(f, "invalid seat index: {seat}"),
            Self::InvalidOrder(msg) => write!(f, "invalid order: {msg}"),
            Self::UnknownObject(msg) => write!(f, "unknown object: {msg}"),
            Self::Unauthorized(msg) => write!(f, "unauthorized action: {msg}"),
            Self::PerishableEscrowForbidden => {
                write!(f, "perishable raw goods cannot lock inside TokenEscrow")
            }
            Self::InsufficientBalance(msg) => write!(f, "insufficient free balance: {msg}"),
            Self::CommitmentMismatch(seat) => {
                write!(f, "seat {seat} reveal did not match Phase 2 commitment")
            }
            Self::AlreadySubmitted(seat) => {
                write!(f, "seat {seat} already submitted for this turn phase")
            }
            Self::DomainAccessDenied(msg) => write!(f, "cartel domain access denied: {msg}"),
            Self::SeasonEnded(msg) => write!(f, "season replay/lifecycle error: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}
