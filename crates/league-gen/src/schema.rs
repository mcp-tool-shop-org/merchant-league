//! Schema, configuration, and error types for Merchant League v2 chiral tables.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Validated seat count for a chiral converter ring (`4`, `5`, or `6`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SeatCount {
    /// Four-seat chiral converter ring.
    Four = 4,
    /// Five-seat chiral converter ring.
    Five = 5,
    /// Six-seat chiral converter ring.
    Six = 6,
}

impl SeatCount {
    /// All supported chiral ring seat counts in ascending order.
    pub const ALL: [Self; 3] = [Self::Four, Self::Five, Self::Six];

    /// Converts a raw integer count into a validated [`SeatCount`], or `None` if unsupported.
    #[must_use]
    pub const fn from_usize(n: usize) -> Option<Self> {
        match n {
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            6 => Some(Self::Six),
            _ => None,
        }
    }

    /// Returns the number of seats as `usize`.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self as usize
    }

    /// Returns the number of seats as `u8`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns the number of unordered seat pairs `N * (N - 1) / 2`.
    #[must_use]
    pub const fn pair_count(self) -> usize {
        let n = self.as_usize();
        n * (n - 1) / 2
    }
}

/// Three-character uppercase ASCII ledger currency code (`[A-Z]{3}`), excluding
/// native `"XRP"` and standard real-world ISO codes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CurrencyCode([u8; 3]);

/// Native and ISO currency codes excluded from table token generation.
const RESERVED_CURRENCY_CODES: &[[u8; 3]] = &[
    *b"XRP", *b"USD", *b"EUR", *b"GBP", *b"JPY", *b"CHF", *b"CAD", *b"AUD", *b"CNY", *b"KRW",
    *b"INR", *b"BRL", *b"MXN",
];

impl CurrencyCode {
    /// Constructs a [`CurrencyCode`] if `bytes` are 3 ASCII uppercase letters outside the
    /// reserved exclusion list.
    pub fn from_bytes(bytes: [u8; 3]) -> Result<Self, FeasibilityError> {
        if !bytes.iter().all(|b| b.is_ascii_uppercase()) {
            return Err(FeasibilityError::InvalidTable(
                "currency code must contain 3 uppercase ASCII letters".to_owned(),
            ));
        }
        if RESERVED_CURRENCY_CODES.contains(&bytes) {
            return Err(FeasibilityError::InvalidTable(
                "currency code uses a reserved code".to_owned(),
            ));
        }
        Ok(Self(bytes))
    }

    /// Returns the 3 ASCII bytes of this currency code.
    #[must_use]
    pub const fn as_bytes(&self) -> [u8; 3] {
        self.0
    }

    /// Formats the 3-character code as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Construction checks every byte is ASCII uppercase `0x41..=0x5A`, which is valid UTF-8.
        std::str::from_utf8(&self.0).unwrap_or("BAD")
    }
}

impl fmt::Debug for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CurrencyCode({})", self.as_str())
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<CurrencyCode> for String {
    fn from(code: CurrencyCode) -> Self {
        code.as_str().to_owned()
    }
}

impl TryFrom<String> for CurrencyCode {
    type Error = FeasibilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        if bytes.len() != 3 {
            return Err(FeasibilityError::InvalidTable(
                "currency code string must have byte length 3".to_owned(),
            ));
        }
        Self::from_bytes([bytes[0], bytes[1], bytes[2]])
    }
}

/// Kind of 3-stage telegraphed Horizon Shock anchored via `OracleSet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HorizonShockKind {
    /// Temporarily cuts Turn `t+2` raw extraction yield on the target seat's good.
    SupplyFreeze,
    /// Temporarily resets the target seat's commodity `TransferRate` tariff to zero.
    TariffHoliday,
    /// Temporarily boosts Turn `t+2`..`t+3` Syndicate Project valuation on the target refined good.
    DemandSurge,
    /// Temporarily elevates end-of-turn perishable `Clawback` spoilage rate on the target good.
    SpoilageSpike,
}

/// One telegraphed shock schedule (`hint_turn -> forecast_turn -> realize_turn`) targeting a seat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledShock {
    /// Turn when Authority commits the hidden hash and sends the private hint to Oracle Auditor.
    pub hint_turn: u8,
    /// Turn (`hint_turn + 1`) when Authority publishes the narrowed public `OracleSet` forecast.
    pub forecast_turn: u8,
    /// Turn (`hint_turn + 2`) when the shock executes deterministically on-ledger.
    pub realize_turn: u8,
    /// Target seat index (`0..N`) whose raw/refined commodity is affected.
    pub target_seat: u8,
    /// Shock effect category.
    pub kind: HorizonShockKind,
    /// Magnitude parameter in `(0.0, 1.0]` (fraction reduction or bonus weight depending on `kind`).
    pub magnitude: f64,
}

/// One converter recipe owned by a seat on the ring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConverterSpec {
    /// Seat index (`0..N`) authorized to run this converter.
    pub owner: u8,
    /// Required `(raw_good_index, quantity_per_run)` inputs.
    pub inputs: Vec<(u8, f64)>,
    /// Produced refined good index (`0..N`).
    pub output_good: u8,
    /// Quantity of `output_good` minted per batch run.
    pub output_amount: f64,
    /// Maximum runs per turn for this converter recipe.
    pub max_runs_per_turn: f64,
    /// Whether this is an optional secondary cross-converter (`false` = primary ring converter).
    pub is_secondary: bool,
}

/// Economic parameters for a single seat in a [`RingTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeatSpec {
    /// Zero-based seat index (`0..N`).
    pub seat: u8,
    /// Exclusive raw good index extracted by this seat (`seat`).
    pub raw_good: u8,
    /// 3-character ledger IOU currency code for this seat's raw good.
    pub raw_code: CurrencyCode,
    /// Units of `raw_good` extracted per turn.
    pub raw_yield_per_turn: f64,
    /// Exclusive refined good index produced by this seat's converters (`seat`).
    pub refined_good: u8,
    /// 3-character ledger IOU currency code for this seat's refined good.
    pub refined_code: CurrencyCode,
    /// Fundamental Syndicate Project value per unit of `refined_good`.
    pub refined_project_value: f64,
    /// Neighbor refined good owed for upkeep (`(seat + N - 1) % N`).
    pub upkeep_good: u8,
    /// Mandatory units of `upkeep_good` owed at Sweep each turn.
    pub upkeep_amount: f64,
    /// Initial commodity export `TransferRate` tariff (`0.0..=0.35`) on `raw_good`.
    pub raw_export_tariff: f64,
    /// Initial commodity export `TransferRate` tariff (`0.0..=0.35`) on `refined_good`.
    pub refined_export_tariff: f64,
    /// Per-turn perishable `Clawback` spoilage rate (`0.0..1.0`) on unconverted `raw_good`.
    pub perishable_decay_rate: f64,
    /// Converter recipes operated by this seat.
    pub converters: Vec<ConverterSpec>,
}

/// Complete specification for a generated chiral converter-ring table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingTable {
    /// Seed passed to [`crate::generate_ring_table`].
    pub seed: u64,
    /// Zero-based search attempt index that passed the LP feasibility filter.
    pub attempt: u32,
    /// Validated number of seats (`4`, `5`, or `6`).
    pub seat_count: SeatCount,
    /// 3-character currency code for the table numeraire coin (distinct from all commodity codes).
    pub numeraire_code: CurrencyCode,
    /// Inclusive turn bounds `(min_close, max_close)` for the stochastic season close (`(24, 28)`).
    pub season_turn_window: (u8, u8),
    /// Per-seat economic definitions in index order `0..N`.
    pub seats: Vec<SeatSpec>,
    /// Pre-sampled 3-stage `OracleSet` horizon shock schedule for the season.
    pub shocks: Vec<ScheduledShock>,
}

/// Parameters controlling chiral table generation and LP feasibility filtering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Optional fixed seat count (`None` samples uniformly over `4`, `5`, `6`).
    pub seat_count: Option<SeatCount>,
    /// Maximum allowed relative deviation of any seat's Shapley EV from the table mean (`0.03` = ±3%).
    pub ev_tolerance: f64,
    /// Minimum required `(net_converter_surplus_value / aggregate_upkeep_value)` margin (`0.15`).
    pub min_surplus_ratio: f64,
    /// Whether 6-seat tables may attach secondary chiral cross-converters.
    pub enable_secondary_converters: bool,
    /// Upper bound on initial baseline export `TransferRate` tariffs (`<= 0.20`).
    pub max_tariff: f64,
    /// Inclusive `(min, max)` bounds for initial perishable `Clawback` decay rates.
    pub decay_range: (f64, f64),
    /// Maximum candidate sampling attempts before returning [`GenError::NoFeasibleTableFound`].
    pub max_attempts: u32,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            seat_count: None,
            ev_tolerance: 0.03,
            min_surplus_ratio: 0.15,
            enable_secondary_converters: true,
            max_tariff: 0.20,
            decay_range: (0.15, 0.25),
            max_attempts: 256,
        }
    }
}

/// Reasons a candidate [`RingTable`] can fail structural or LP feasibility checks.
#[derive(Debug, Clone, PartialEq)]
pub enum FeasibilityError {
    /// Candidate table failed structural validation (indices, codes, positive quantities, shock timing).
    InvalidTable(String),
    /// At least one 2-seat pair satisfied non-zero internal upkeep without external trade.
    PairCoversUpkeep {
        /// Ordered pair `(i, j)` of seats (`i < j`).
        pair: (u8, u8),
        /// Maximum fraction of the pair's combined upkeep satisfied internally.
        coverage_ratio: f64,
    },
    /// Aggregate table production either cannot cover mandatory upkeep or lacks required surplus.
    InsufficientSurplus {
        /// Net refined surplus value remaining after full table upkeep (negative if deficit).
        net_surplus: f64,
        /// Minimum required surplus ratio (`config.min_surplus_ratio`).
        required_ratio: f64,
        /// Measured `(net_surplus / total_upkeep_value)` ratio (or `0.0` if infeasible).
        actual_ratio: f64,
    },
    /// At least one seat's Shapley EV falls outside `±ev_tolerance` of the table mean.
    SeatEvOutOfTolerance {
        /// Maximum relative deviation `|(seat_ev - mean_ev) / mean_ev|` observed.
        max_relative_deviation: f64,
        /// Maximum allowed relative deviation (`config.ev_tolerance`).
        tolerance: f64,
        /// Per-seat Shapley expected values.
        seat_evs: Vec<f64>,
    },
    /// Underlying simplex LP solver hit an unexpected numerical or unbounded state.
    SolverFailure(String),
}

impl fmt::Display for FeasibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTable(msg) => write!(f, "invalid ring table: {msg}"),
            Self::PairCoversUpkeep {
                pair,
                coverage_ratio,
            } => write!(
                f,
                "2-seat pair ({}, {}) covers {:.4} of its own upkeep (must be 0.0)",
                pair.0, pair.1, coverage_ratio
            ),
            Self::InsufficientSurplus {
                net_surplus,
                required_ratio,
                actual_ratio,
            } => write!(
                f,
                "insufficient aggregate converter surplus: net={net_surplus:.4}, ratio={actual_ratio:.4} < {required_ratio:.4}"
            ),
            Self::SeatEvOutOfTolerance {
                max_relative_deviation,
                tolerance,
                ..
            } => write!(
                f,
                "seat EV deviation {max_relative_deviation:.4} exceeds tolerance {tolerance:.4}"
            ),
            Self::SolverFailure(msg) => write!(f, "LP solver error: {msg}"),
        }
    }
}

impl std::error::Error for FeasibilityError {}

/// Errors returned by table generation and feasibility configuration checks.
#[derive(Debug, Clone, PartialEq)]
pub enum GenError {
    /// Generator configuration contained invalid bounds or zero attempts.
    InvalidConfig(String),
    /// No feasible table passed all three LP invariants within `max_attempts`.
    NoFeasibleTableFound {
        /// Number of attempts tested.
        attempts: u32,
        /// Failure recorded on the final attempt.
        last_error: FeasibilityError,
    },
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid generator config: {msg}"),
            Self::NoFeasibleTableFound {
                attempts,
                last_error,
            } => write!(
                f,
                "no feasible chiral table found in {attempts} attempts (last: {last_error})"
            ),
        }
    }
}

impl std::error::Error for GenError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_count_and_currency_code_validation() {
        assert_eq!(SeatCount::from_usize(3), None);
        assert_eq!(SeatCount::from_usize(4), Some(SeatCount::Four));
        assert_eq!(SeatCount::from_usize(5), Some(SeatCount::Five));
        assert_eq!(SeatCount::from_usize(6), Some(SeatCount::Six));
        assert_eq!(SeatCount::from_usize(7), None);
        assert_eq!(SeatCount::Five.as_u8(), 5);
        assert_eq!(SeatCount::Five.pair_count(), 10);
        assert_eq!(SeatCount::Six.pair_count(), 15);

        let Ok(code) = CurrencyCode::from_bytes(*b"SLT") else {
            panic!("SLT must be valid");
        };
        assert_eq!(code.as_str(), "SLT");
        assert_eq!(code.to_string(), "SLT");
        assert_eq!(format!("{code:?}"), "CurrencyCode(SLT)");
        let s: String = code.into();
        assert_eq!(s, "SLT");
        assert_eq!(CurrencyCode::try_from("SLT".to_owned()), Ok(code));

        assert!(CurrencyCode::from_bytes(*b"slt").is_err());
        assert!(CurrencyCode::from_bytes(*b"XRP").is_err());
        assert!(CurrencyCode::try_from("AB".to_owned()).is_err());
    }

    #[test]
    fn error_display_messages_are_non_empty() {
        let errs = [
            FeasibilityError::InvalidTable("bad".to_owned()),
            FeasibilityError::PairCoversUpkeep {
                pair: (0, 1),
                coverage_ratio: 0.5,
            },
            FeasibilityError::InsufficientSurplus {
                net_surplus: -1.0,
                required_ratio: 0.15,
                actual_ratio: 0.0,
            },
            FeasibilityError::SeatEvOutOfTolerance {
                max_relative_deviation: 0.08,
                tolerance: 0.03,
                seat_evs: vec![10.0, 12.0],
            },
            FeasibilityError::SolverFailure("singular".to_owned()),
        ];
        for err in &errs {
            assert!(!err.to_string().is_empty());
        }
        let g1 = GenError::InvalidConfig("bad cfg".to_owned());
        let g2 = GenError::NoFeasibleTableFound {
            attempts: 5,
            last_error: errs[0].clone(),
        };
        assert!(!g1.to_string().is_empty());
        assert!(!g2.to_string().is_empty());
    }
}
