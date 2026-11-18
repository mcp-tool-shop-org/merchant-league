use std::fmt;

use league_error::{StructuredError, ToStructuredError};
use serde::{Deserialize, Serialize};

/// Integer milli-unit quantity (`1_000 == 1.000` unit of `CRED`, commodity, or equity).
pub type Units = i64;

/// Scaling factor for fixed-point milli-unit quantities (`1_000 == 1.000`).
pub const UNIT_SCALE: Units = 1_000;

/// Rate expressed in integer basis points (`10_000 == 100.00%`).
pub type Bps = u16;

/// Scaling factor for basis points (`10_000 == 100.00%`).
pub const BPS_SCALE: i128 = 10_000;

/// First turn on which the stochastic season closing bell roll runs (`Turn 24`; Turns `1..=23` stay open).
pub const CLOSING_ROLL_START_TURN: u8 = 24;

/// Hard terminal turn cap (`Turn 32`, `2026-11-15` ruling; unconditional close at Sweep Step 8).
pub const HARD_TURN_CAP: u8 = 32;

/// Number of stochastic closing-roll turns (`24..=31`, `8` turns before [`HARD_TURN_CAP`]).
pub const CLOSING_ROLL_WINDOW_LEN: usize = (HARD_TURN_CAP - CLOSING_ROLL_START_TURN) as usize;

/// Starting net-worth floor fraction (`5_000 bps = 50.00%` of `W_0`, the table-median starting
/// net worth) applied inside `max(Med(W), 0.5 * W_0)` when converting mandate points.
pub const MANDATE_STARTING_NW_FLOOR_BPS: Bps = 5_000;

/// Fixed base-value floor and salvage fraction (`5_000 bps = 50.00%` of fixed base value / parity)
/// applied once at Season Close over all pooled non-winning bid slices and uncovered leftover units.
pub const UNSOLD_EXCESS_PARITY_BPS: Bps = 5_000;

/// Minimum Turn-1 Private Mandate bank bonus (`3.000 VP`).
pub const MIN_MANDATE_BONUS_EQUITY: Units = 3 * UNIT_SCALE;

/// Maximum Turn-1 Private Mandate bank bonus (`8.000 VP`).
pub const MAX_MANDATE_BONUS_EQUITY: Units = 8 * UNIT_SCALE;

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

/// Initial economic configuration for a single seat in a table specification.
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
    ///
    /// Intentionally public table configuration (`C4`): exposed through [`crate::PublicTableSpec`] and
    /// [`crate::SeatObservation`] so every seat can see upfront which seats have Standing Regent fallback.
    pub regent: Option<RegentDoctrine>,
    /// Converter recipes owned by this seat.
    pub converters: Vec<ConverterRecipe>,
}

/// Execution mode recorded on a table specification and replay log (`2026-11-15` ruling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableMode {
    /// Live Game Mode: requires `drand_schedule` and drand-signature-bound beacons.
    Game,
    /// Headless Evaluation Mode: uses sealed 256-bit OS CSPRNG entropy (`SealedEvalSeed`) with
    /// proofs under the RFC 9381 ECVRF construction with C2SP's `ristretto255` ciphersuite
    /// (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not registered in RFC 9381) in replay logs.
    Eval,
    /// Offline Club Mode (`v0.1.0`): local hot-seat humans and deterministic rules-only bots on one
    /// machine, paired strictly with `EvalVrf` proofs over OS-CSPRNG entropy and clear
    /// `SeedPool::Dev` (`0..=9_999`) layout seeds.
    Club,
}

/// Errors returned by turn simulation, preview execution, and season replay verification.
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
    /// Seat attempted `CancelOffer` after Phase-1 cancel lock engaged (`C4` amended `2026-11-17`).
    CancelLocked,
    /// Season replay log was incomplete or inconsistent with closed status.
    SeasonEnded(String),
    /// Persisted replay payload declared an unsupported `format_version` (`!= REPLAY_FORMAT_VERSION`,
    /// such as `0` or a newer unknown version).
    UnsupportedReplayVersion {
        /// Version number found in the serialized payload.
        found: u32,
        /// Newest replay format version known to this reader.
        newest_known: u32,
    },
}

impl EngineError {
    /// Returns a stable SCREAMING_SNAKE error code string for this [`EngineError`] variant.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidConfig(_) => "INVALID_CONFIG",
            Self::InvalidPhase(_) => "INVALID_PHASE",
            Self::InvalidSeat(_) => "INVALID_SEAT",
            Self::InvalidOrder(_) => "INVALID_ORDER",
            Self::UnknownObject(_) => "UNKNOWN_OBJECT",
            Self::Unauthorized(_) => "UNAUTHORIZED",
            Self::PerishableEscrowForbidden => "PERISHABLE_ESCROW_FORBIDDEN",
            Self::InsufficientBalance(_) => "INSUFFICIENT_BALANCE",
            Self::AlreadySubmitted(_) => "ALREADY_SUBMITTED",
            Self::CommitmentMismatch(_) => "COMMITMENT_MISMATCH",
            Self::DomainAccessDenied(_) => "DOMAIN_ACCESS_DENIED",
            Self::CancelLocked => "CANCEL_LOCKED",
            Self::SeasonEnded(_) => "SEASON_ENDED",
            Self::UnsupportedReplayVersion { .. } => "UNSUPPORTED_REPLAY_VERSION",
        }
    }
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
            Self::CancelLocked => write!(f, "Phase-1 cancel lock is active"),
            Self::SeasonEnded(msg) => write!(f, "season replay/lifecycle error: {msg}"),
            Self::UnsupportedReplayVersion {
                found,
                newest_known,
            } => write!(
                f,
                "UNSUPPORTED_REPLAY_VERSION: replay format version {found} is not supported by this reader (newest known version is {newest_known})"
            ),
        }
    }
}

impl std::error::Error for EngineError {}

impl ToStructuredError for EngineError {
    fn to_structured_error(&self) -> StructuredError {
        let hint = match self {
            Self::InvalidConfig(_) => {
                "check TableSpec, mode, seed pool, and replay fields against table validation rules"
            }
            Self::InvalidPhase(_) => {
                "submit the operation during the matching TurnPhase (TalkAndMarket, Commit, Reveal, or AwaitingMandateReveal)"
            }
            Self::InvalidSeat(_) => "use a valid zero-based seat index in 0..seat_count",
            Self::InvalidOrder(_) => {
                "adjust quantities, deadlines, assets, or VRF/replay parameters to satisfy rule bounds"
            }
            Self::UnknownObject(_) => {
                "verify the offer, check, escrow, project, or cartel domain ID exists in current state"
            }
            Self::Unauthorized(_) => {
                "ensure the acting seat owns the object, holds the required charter, and is not wash-frozen"
            }
            Self::PerishableEscrowForbidden => {
                "lock CRED collateral rather than perishable raw commodities inside TokenEscrow"
            }
            Self::InsufficientBalance(_) => {
                "reduce the order size or acquire enough free CRED or commodity units first"
            }
            Self::AlreadySubmitted(_) => {
                "wait for the next turn or phase rather than submitting a duplicate commitment or reveal"
            }
            Self::CommitmentMismatch(_) => {
                "reveal the exact SealedOrderBundle or Mandate and RevealSalt committed earlier"
            }
            Self::DomainAccessDenied(_) => {
                "obtain cartel domain membership or an active ClearinghouseBroker charter before accessing this domain"
            }
            Self::CancelLocked => {
                "cancel resting offers before Phase-1 cancel lock engages; late cancellations are rejected"
            }
            Self::SeasonEnded(_) => {
                "verify the replay log contains a complete closed season from Turn 1 through AwaitingMandateReveal"
            }
            Self::UnsupportedReplayVersion { .. } => {
                "upgrade the reader to support this replay format_version before loading the file"
            }
        };
        StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}
