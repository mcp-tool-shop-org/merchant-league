//! Schema, configuration, and error types for Merchant League v2 chiral tables.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Generator release identifier embedded in every produced [`RingTable`] for cross-run comparability.
pub const GENERATOR_VERSION: &str = "league-gen/v2.5.0";

/// Maximum allowed relative deviation `max_i |(EV_i - mean(EV)) / mean(EV)|` (`7%`, plus the
/// `1e-9` floating-point comparison slack) of any seat's exact `2^N` sub-coalition Shapley
/// value ([`crate::compute_shapley_seat_evs`]) from the seats' Shapley mean, enforced beside
/// `±config.ev_tolerance` across every [`EvMethod`] (Director ruling, `2026-11-16`).
pub const SHAPLEY_GUARD_MAX_REL_DEV: f64 = 0.07;

/// Fixed ledger numeraire currency code specified by `docs/design-v2.md` §1.2.
pub const NUMERAIRE_CODE: &str = "CRED";

/// Provisional upper cap on `GeneratorConfig::max_attempts` preventing unbounded search loops.
pub const MAX_ATTEMPTS_CAP: u32 = 10_000;

/// Reserved tier-3 full-ring Syndicate Concordat project id (`N`-good collective assembly).
pub const PROJECT_CONCORDAT_ID: u8 = 100;

/// Minimum Turn-1 Private Charter Mandate completion bonus in Victory Points (`3.0 VP`).
pub const MIN_MANDATE_BONUS_VP: f64 = 3.0;

/// Maximum Turn-1 Private Charter Mandate completion bonus in Victory Points (`8.0 VP`).
pub const MAX_MANDATE_BONUS_VP: f64 = 8.0;

/// Basis-point scale (`10_000 = 100%`).
pub const BPS_SCALE: u16 = 10_000;

/// Inclusive lower bound of the `dev` layout-seed pool (`0..=9_999`).
pub const DEV_SEED_MIN: u64 = 0;
/// Inclusive upper bound of the `dev` layout-seed pool (`0..=9_999`).
pub const DEV_SEED_MAX: u64 = 9_999;
/// Inclusive lower bound of the `train` layout-seed pool (`10_000..=999_999`).
pub const TRAIN_SEED_MIN: u64 = 10_000;
/// Inclusive upper bound of the `train` layout-seed pool (`10_000..=999_999`).
pub const TRAIN_SEED_MAX: u64 = 999_999;
/// Inclusive lower bound of the `probe` layout-seed pool (`1_000_000..=1_099_999`).
pub const PROBE_SEED_MIN: u64 = 1_000_000;
/// Inclusive upper bound of the `probe` layout-seed pool (`1_000_000..=1_099_999`).
pub const PROBE_SEED_MAX: u64 = 1_099_999;
/// Inclusive lower bound of the `held_out` layout-seed pool (`2_000_000..=2_099_999`).
pub const HELD_OUT_SEED_MIN: u64 = 2_000_000;
/// Inclusive upper bound of the `held_out` layout-seed pool (`2_000_000..=2_099_999`).
pub const HELD_OUT_SEED_MAX: u64 = 2_099_999;
/// Inclusive lower bound of the `sealed` layout-seed pool (`2^63..=2^64 - 1`).
pub const SEALED_SEED_MIN: u64 = 1_u64 << 63;
/// Inclusive upper bound of the `sealed` layout-seed pool (`2^63..=2^64 - 1`).
pub const SEALED_SEED_MAX: u64 = u64::MAX;

/// Disjoint layout-seed pool classification (`docs/design-v2.md` §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedPool {
    /// Gate tests, reviews, tuning, stubs, and playtests while rules are still changing (`0..=9_999`).
    Dev,
    /// Training-logged games; the training curriculum draws strictly from here (`10_000..=999_999`).
    Train,
    /// Forward-pass checks during training with no gradient (`1_000_000..=1_099_999`).
    Probe,
    /// Counted evaluation of a trained model or rule change (`2_000_000..=2_099_999`).
    HeldOut,
    /// Final sealed evaluation range (`2^63..=2^64 - 1`); runs only for [`TablePurpose::Sealed`].
    Sealed,
    /// Every other `u64` value; refused for all [`TablePurpose`] values.
    Unassigned,
}

impl SeedPool {
    /// Lowercase canonical name matching this variant's serde representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Train => "train",
            Self::Probe => "probe",
            Self::HeldOut => "held_out",
            Self::Sealed => "sealed",
            Self::Unassigned => "unassigned",
        }
    }
}

impl fmt::Display for SeedPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Single canonical inclusive `(min, max, pool)` boundary table for [`seed_pool`].
pub const SEED_POOL_TABLE: [(u64, u64, SeedPool); 5] = [
    (DEV_SEED_MIN, DEV_SEED_MAX, SeedPool::Dev),
    (TRAIN_SEED_MIN, TRAIN_SEED_MAX, SeedPool::Train),
    (PROBE_SEED_MIN, PROBE_SEED_MAX, SeedPool::Probe),
    (HELD_OUT_SEED_MIN, HELD_OUT_SEED_MAX, SeedPool::HeldOut),
    (SEALED_SEED_MIN, SEALED_SEED_MAX, SeedPool::Sealed),
];

/// Classifies `seed` into its unique [`SeedPool`] from [`SEED_POOL_TABLE`] alone, independent of
/// seat count or [`GeneratorConfig`]. No other code decides a layout seed's pool.
///
/// # Rule for New Code and Tests
/// Tests and gates call the raw [`crate::generate_ring_table`] generator with `dev`-pool seeds
/// (`0..=9_999`) only; a test may use `train`-pool seeds (`10_000..=999_999`) only when it is
/// about the `train` pool or already did so before seed pools were introduced; `probe`,
/// `held_out`, and `sealed`-range seeds never appear in tests except the reserved test values in
/// the `docs/design-v2.md` §7.4 exceptions table ([`RESERVED_PROBE_HELD_OUT_TEST_SEEDS`] plus the
/// three made-up `sealed`-range values checked by [`is_reserved_test_seed`], along with
/// non-generating `seed_pool` / refusal checks), and the future game-table bootstrap must call
/// [`is_reserved_test_seed`] and never schedule a reserved value for a real game.
#[must_use]
pub const fn seed_pool(seed: u64) -> SeedPool {
    let mut idx = 0;
    while idx < SEED_POOL_TABLE.len() {
        let (lo, hi, pool) = SEED_POOL_TABLE[idx];
        if seed >= lo && seed <= hi {
            return pool;
        }
        idx += 1;
    }
    SeedPool::Unassigned
}

/// Reserved `probe` (`1_000_000`, `1_099_999`) and `held_out` (`2_000_000`, `2_099_999`) boundary
/// seeds exercised by `league-gen` acceptance tests (`docs/design-v2.md` §7.4).
///
/// The future game-table bootstrap must reject these seeds (via [`is_reserved_test_seed`]) when
/// scheduling real `probe` or `held_out` games.
pub const RESERVED_PROBE_HELD_OUT_TEST_SEEDS: [u64; 4] = [
    PROBE_SEED_MIN,
    PROBE_SEED_MAX,
    HELD_OUT_SEED_MIN,
    HELD_OUT_SEED_MAX,
];

/// Returns `true` when `seed` is one of the reserved test seeds listed in `docs/design-v2.md` §7.4:
/// [`RESERVED_PROBE_HELD_OUT_TEST_SEEDS`] (`1_000_000`, `1_099_999`, `2_000_000`, `2_099_999`) or
/// the three made-up `sealed`-range test values (`2^63`, `2^64 - 1`, and `2^63` with `42` set).
///
/// The future table bootstrap must refuse to schedule any seed for which this function returns `true`.
#[must_use]
pub const fn is_reserved_test_seed(seed: u64) -> bool {
    let mut idx = 0;
    while idx < RESERVED_PROBE_HELD_OUT_TEST_SEEDS.len() {
        if seed == RESERVED_PROBE_HELD_OUT_TEST_SEEDS[idx] {
            return true;
        }
        idx += 1;
    }
    seed == SEALED_SEED_MIN || seed == SEALED_SEED_MAX || seed == (SEALED_SEED_MIN | 42)
}

/// Declared role for which a table is being generated via [`crate::generate_ring_table_for_purpose`]
/// or [`crate::generate_ring_table_for_setup`].
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TablePurpose {
    /// Development, gate checks, rule tuning, stubs, and exploratory playtests (`dev` pool, default).
    #[default]
    Dev,
    /// Training-logged curriculum games (`train` pool).
    Train,
    /// No-gradient forward-pass probe games during training (`probe` pool).
    Probe,
    /// Counted held-out evaluation of a trained model or rule change (`held_out` pool).
    HeldOut,
    /// Final check on a seed from the sealed range (`sealed` pool).
    Sealed,
}

impl TablePurpose {
    /// All five valid table purposes in ascending pool order.
    pub const ALL: [Self; 5] = [
        Self::Dev,
        Self::Train,
        Self::Probe,
        Self::HeldOut,
        Self::Sealed,
    ];

    /// Returns the sole [`SeedPool`] accepted for this purpose.
    #[must_use]
    pub const fn expected_pool(self) -> SeedPool {
        match self {
            Self::Dev => SeedPool::Dev,
            Self::Train => SeedPool::Train,
            Self::Probe => SeedPool::Probe,
            Self::HeldOut => SeedPool::HeldOut,
            Self::Sealed => SeedPool::Sealed,
        }
    }

    /// Lowercase canonical name matching this variant's serde representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Train => "train",
            Self::Probe => "probe",
            Self::HeldOut => "held_out",
            Self::Sealed => "sealed",
        }
    }
}

impl fmt::Display for TablePurpose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Purpose and training-log flag passed to [`crate::generate_ring_table_for_setup`].
///
/// Defaults to `purpose = TablePurpose::Dev` and `training_logged = false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableSetupOptions {
    /// Declared role for the table (`dev` by default).
    pub purpose: TablePurpose,
    /// Whether game trajectories may be written to training logs (`false` by default; valid only
    /// when `purpose == TablePurpose::Train`).
    pub training_logged: bool,
}

/// Computes the lowercase 64-character hex SHA-256 digest of `seed`'s 8 big-endian bytes by
/// delegating directly to [`league_core::layout_seed_digest`].
///
/// Routing both crates through that single `league-core` function ensures issue `#58` (a rig-keyed
/// digest before any sealed record leaves the rig) is a one-place change.
#[must_use]
pub fn sealed_seed_sha256(seed: u64) -> String {
    league_core::layout_seed_digest(seed)
}

/// Validation failure returned when deserializing an invalid [`RecordedLayoutSeed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedLayoutSeedError {
    /// A `sealed`-pool value (`>= 2^63`) appeared in the `"clear"` JSON variant (`SEALED_SEED_IN_CLEAR`).
    SealedValueInClear {
        /// Redacted 8-byte big-endian SHA-256 digest of the rejected seed.
        sealed_seed_sha256: String,
    },
    /// The `"sealed_sha256"` JSON variant was not 64 lowercase ASCII hex characters (`INVALID_SEALED_SEED_DIGEST`).
    InvalidSealedDigest,
}

impl fmt::Display for RecordedLayoutSeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SealedValueInClear { sealed_seed_sha256 } => write!(
                f,
                "SEALED_SEED_IN_CLEAR: clear layout seed cannot hold a pool 'sealed' value (sha256={sealed_seed_sha256})"
            ),
            Self::InvalidSealedDigest => write!(
                f,
                "INVALID_SEALED_SEED_DIGEST: sealed_sha256 must be 64 lowercase hex characters"
            ),
        }
    }
}

impl std::error::Error for RecordedLayoutSeedError {}

impl RecordedLayoutSeedError {
    /// Returns the stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SealedValueInClear { .. } => "SEALED_SEED_IN_CLEAR",
            Self::InvalidSealedDigest => "INVALID_SEALED_SEED_DIGEST",
        }
    }
}

impl league_core::ToStructuredError for RecordedLayoutSeedError {
    fn to_structured_error(&self) -> league_core::StructuredError {
        let hint = match self {
            Self::SealedValueInClear { .. } => {
                "record sealed-pool layout seeds with the sealed_sha256 variant rather than clear"
            }
            Self::InvalidSealedDigest => {
                "supply a 64-character lowercase hexadecimal SHA-256 digest"
            }
        };
        league_core::StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RecordedLayoutSeedRepr {
    Clear(u64),
    SealedSha256(String),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawRecordedLayoutSeed {
    Clear(u64),
    SealedSha256(String),
}

impl From<RecordedLayoutSeed> for RawRecordedLayoutSeed {
    fn from(value: RecordedLayoutSeed) -> Self {
        match value.0 {
            RecordedLayoutSeedRepr::Clear(seed) => Self::Clear(seed),
            RecordedLayoutSeedRepr::SealedSha256(digest) => Self::SealedSha256(digest),
        }
    }
}

impl TryFrom<RawRecordedLayoutSeed> for RecordedLayoutSeed {
    type Error = RecordedLayoutSeedError;

    fn try_from(raw: RawRecordedLayoutSeed) -> Result<Self, Self::Error> {
        match raw {
            RawRecordedLayoutSeed::Clear(seed) => {
                if seed_pool(seed) == SeedPool::Sealed {
                    return Err(RecordedLayoutSeedError::SealedValueInClear {
                        sealed_seed_sha256: sealed_seed_sha256(seed),
                    });
                }
                Ok(Self(RecordedLayoutSeedRepr::Clear(seed)))
            }
            RawRecordedLayoutSeed::SealedSha256(digest) => {
                let valid_hex = digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
                if !valid_hex {
                    return Err(RecordedLayoutSeedError::InvalidSealedDigest);
                }
                Ok(Self(RecordedLayoutSeedRepr::SealedSha256(digest)))
            }
        }
    }
}

/// Validated layout-seed representation inside [`GameTableRecord`] and [`GenError`] refusals.
///
/// Construction goes strictly through [`RecordedLayoutSeed::from_seed`] and validated serde
/// deserialization, making it impossible for a [`SeedPool::Sealed`] value (`>= 2^63`) to sit in
/// the clear branch.
///
/// ```compile_fail
/// use league_gen::RecordedLayoutSeed;
/// // Private inner representation prevents constructing a clear variant directly:
/// let _ = RecordedLayoutSeed(1_u64 << 63);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawRecordedLayoutSeed", into = "RawRecordedLayoutSeed")]
pub struct RecordedLayoutSeed(RecordedLayoutSeedRepr);

impl RecordedLayoutSeed {
    /// Encodes `seed` in clear unless `seed_pool(seed) == SeedPool::Sealed`, in which case only
    /// `SHA-256(seed.to_be_bytes())` is stored.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        if seed_pool(seed) == SeedPool::Sealed {
            Self(RecordedLayoutSeedRepr::SealedSha256(sealed_seed_sha256(
                seed,
            )))
        } else {
            Self(RecordedLayoutSeedRepr::Clear(seed))
        }
    }

    /// Returns `Some(seed)` for non-`sealed` pools, or `None` for [`SeedPool::Sealed`].
    #[must_use]
    pub const fn clear_seed(&self) -> Option<u64> {
        match &self.0 {
            RecordedLayoutSeedRepr::Clear(seed) => Some(*seed),
            RecordedLayoutSeedRepr::SealedSha256(_) => None,
        }
    }

    /// Returns `Some(sha256_hex)` of `seed.to_be_bytes()` for [`SeedPool::Sealed`], or `None` otherwise.
    #[must_use]
    pub fn sealed_sha256(&self) -> Option<&str> {
        match &self.0 {
            RecordedLayoutSeedRepr::Clear(_) => None,
            RecordedLayoutSeedRepr::SealedSha256(digest) => Some(digest.as_str()),
        }
    }

    /// Returns `true` when this seed is stored as a redacted [`SeedPool::Sealed`] digest.
    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        matches!(&self.0, RecordedLayoutSeedRepr::SealedSha256(_))
    }
}

/// Checked game-table setup record emitted only by [`crate::generate_ring_table_for_setup`] and
/// [`crate::generate_ring_table_for_purpose`].
///
/// Fields are private and `Deserialize` is intentionally omitted so external callers can obtain a
/// [`GameTableRecord`] solely through the checked table-generation entry point. For
/// [`SeedPool::Sealed`], `layout_seed` stores the 8-byte big-endian SHA-256 digest rather than the
/// raw `u64` seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameTableRecord {
    layout_seed: RecordedLayoutSeed,
    seat_count: SeatCount,
    pool: SeedPool,
    generator_version: String,
    training_logged: bool,
}

impl GameTableRecord {
    pub(crate) fn new_checked(
        seed: u64,
        seat_count: SeatCount,
        pool: SeedPool,
        generator_version: String,
        training_logged: bool,
    ) -> Self {
        // Guarded by `debug_assert!` only because the sole caller in the crate is
        // `generate_ring_table_for_setup` (`crates/league-gen/src/generator.rs:247`), right after
        // enforcing `pool == seed_pool(seed)` (`!= Unassigned`) and `!training_logged || pool == Train`.
        debug_assert_eq!(pool, seed_pool(seed));
        debug_assert_ne!(pool, SeedPool::Unassigned);
        debug_assert!(!training_logged || pool == SeedPool::Train);
        Self {
            layout_seed: RecordedLayoutSeed::from_seed(seed),
            seat_count,
            pool,
            generator_version,
            training_logged,
        }
    }

    /// Returns the recorded layout seed (clear `u64` for `dev`/`train`/`probe`/`held_out`, or
    /// 8-byte big-endian SHA-256 digest for `sealed`).
    #[must_use]
    pub const fn layout_seed(&self) -> &RecordedLayoutSeed {
        &self.layout_seed
    }

    /// Returns `Some(seed)` for non-`sealed` pools, or `None` for [`SeedPool::Sealed`].
    #[must_use]
    pub const fn clear_layout_seed(&self) -> Option<u64> {
        self.layout_seed.clear_seed()
    }

    /// Returns `Some(sha256_hex)` of `seed.to_be_bytes()` for [`SeedPool::Sealed`], or `None` otherwise.
    #[must_use]
    pub fn sealed_seed_sha256(&self) -> Option<&str> {
        self.layout_seed.sealed_sha256()
    }

    /// Returns the generated table's validated [`SeatCount`].
    #[must_use]
    pub const fn seat_count(&self) -> SeatCount {
        self.seat_count
    }

    /// Returns the validated [`SeedPool`] of the layout seed.
    #[must_use]
    pub const fn pool(&self) -> SeedPool {
        self.pool
    }

    /// Returns the [`GENERATOR_VERSION`] embedded in the generated [`RingTable`].
    #[must_use]
    pub fn generator_version(&self) -> &str {
        &self.generator_version
    }

    /// Returns whether this game's transcripts/trajectories are authorized for training logs.
    #[must_use]
    pub const fn training_logged(&self) -> bool {
        self.training_logged
    }
}

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

    /// Provisional C6 table-level top-pair share cap (`4p: 0.45`, `5p: 0.35`, `6p: 0.30`).
    ///
    /// Provisional pending playtest calibration: the 2026-11-15 ruling established the pair-share
    /// table check concept without specifying numeric cap values, and these thresholds do not bind
    /// on generated tables today (`static_max_pair_share` and rollout pair shares fall well below).
    #[must_use]
    pub const fn default_max_pair_share_cap(self) -> f64 {
        match self {
            Self::Four => 0.45,
            Self::Five => 0.35,
            Self::Six => 0.30,
        }
    }
}

/// Method used by [`crate::check_feasibility`] to compute per-seat expected value (Invariant 3).
///
/// Regardless of which method is active for `±config.ev_tolerance`, [`crate::check_feasibility`]
/// also enforces the 7% [`SHAPLEY_GUARD_MAX_REL_DEV`] sub-coalition Shapley guard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvMethod {
    /// Competitive/cooperative LP-dual shadow-price allocation (`docs/design-v2.md` §1.5 default).
    #[default]
    ShadowPrice,
    /// Exact `2^N` sub-coalition LP Shapley value allocation (retained for comparison).
    Shapley,
}

/// Three-character uppercase ASCII ledger commodity code (`[A-Z]{3}`) drawn strictly from the
/// curated game-word allow-list ([`ALLOWED_COMMODITY_GLOSSARY`]), never overlapping `"XRP"`,
/// active/historical ISO 4217 currency codes, crypto tickers, or reserved strings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CurrencyCode([u8; 3]);

/// Curated, strictly sorted allow-list of 61 medieval-guild game-word commodity codes (`[A-Z]{3}`)
/// mapped to their descriptive English trade goods (`#26a`).
///
/// Generating and validating currency codes exclusively from this list guarantees that neither
/// active/historical ISO 4217 codes nor cryptocurrency/XRPL ecosystem tickers can ever appear.
pub const ALLOWED_COMMODITY_GLOSSARY: &[([u8; 3], &str)] = &[
    (*b"ALM", "alum"),
    (*b"ANV", "anvil"),
    (*b"BBL", "barrel"),
    (*b"BLW", "bellows"),
    (*b"BLY", "barley"),
    (*b"BRS", "brass"),
    (*b"CBL", "cobble"),
    (*b"CDR", "cedar"),
    (*b"CHK", "chalk"),
    (*b"CLY", "clay"),
    (*b"CNV", "canvas"),
    (*b"CPR", "copper"),
    (*b"CRL", "coral"),
    (*b"DYE", "dye"),
    (*b"ELM", "elm"),
    (*b"ENM", "enamel"),
    (*b"FIR", "fir"),
    (*b"FLN", "flint"),
    (*b"GLS", "glass"),
    (*b"GLZ", "glaze"),
    (*b"HMP", "hemp"),
    (*b"HNY", "honey"),
    (*b"HPS", "hops"),
    (*b"HRB", "herb"),
    (*b"IRN", "iron"),
    (*b"IVR", "ivory"),
    (*b"JDE", "jade"),
    (*b"KLN", "kiln"),
    (*b"KLP", "kelp"),
    (*b"LOM", "loom"),
    (*b"LTH", "leather"),
    (*b"LYE", "lye"),
    (*b"MLT", "malt"),
    (*b"MRB", "marble"),
    (*b"NTR", "nitre"),
    (*b"OAK", "oak"),
    (*b"OAT", "oat"),
    (*b"OLV", "olive"),
    (*b"PCH", "pitch"),
    (*b"PLK", "plank"),
    (*b"PWT", "pewter"),
    (*b"QRZ", "quartz"),
    (*b"ROP", "rope"),
    (*b"RSN", "resin"),
    (*b"RYE", "rye"),
    (*b"SLG", "slag"),
    (*b"SLK", "silk"),
    (*b"SLT", "salt"),
    (*b"SPN", "spindle"),
    (*b"SUF", "sulfur"),
    (*b"TLW", "tallow"),
    (*b"TMB", "timber"),
    (*b"TNG", "tongs"),
    (*b"TRF", "turf"),
    (*b"VLM", "vellum"),
    (*b"VLV", "velvet"),
    (*b"WHT", "wheat"),
    (*b"WLK", "whelk"),
    (*b"WOL", "wool"),
    (*b"YEW", "yew"),
    (*b"ZNC", "zinc"),
];

/// Returns `true` when `bytes` is present in the curated game-word allow-list [`ALLOWED_COMMODITY_GLOSSARY`].
#[must_use]
pub fn is_allowed_commodity_code(bytes: [u8; 3]) -> bool {
    ALLOWED_COMMODITY_GLOSSARY
        .binary_search_by_key(&bytes, |&(code, _)| code)
        .is_ok()
}

/// Returns the descriptive English trade good for an allow-listed [`CurrencyCode`].
#[must_use]
pub fn commodity_glossary_word(code: CurrencyCode) -> Option<&'static str> {
    ALLOWED_COMMODITY_GLOSSARY
        .binary_search_by_key(&code.0, |&(entry, _)| entry)
        .ok()
        .map(|idx| ALLOWED_COMMODITY_GLOSSARY[idx].1)
}

/// Sorted deny-list of 3-letter currency, precious-metal, crypto/ledger, and reserved codes
/// excluded from procedural commodity code generation (`docs/design-v2.md` Hard Invariant 1).
#[rustfmt::skip]
pub const DENIED_CURRENCY_CODES: &[[u8; 3]] = &[
    *b"AAV", *b"ADA", *b"ADP", *b"AED", *b"AFA", *b"AFN", *b"AKT", *b"ALG", *b"ALK", *b"ALL",
    *b"AMD", *b"ANC", *b"ANG", *b"ANK", *b"ANT", *b"AOA", *b"AOK", *b"AON", *b"AOR", *b"APE",
    *b"APR", *b"APT", *b"APY", *b"ARA", *b"ARB", *b"ARP", *b"ARS", *b"ARY", *b"ASR", *b"ASS",
    *b"ATM", *b"ATS", *b"AUD", *b"AVA", *b"AWG", *b"AXL", *b"AXS", *b"AZM", *b"AZN", *b"BAD",
    *b"BAL", *b"BAM", *b"BAT", *b"BBD", *b"BCH", *b"BDT", *b"BEC", *b"BEF", *b"BEL", *b"BEM",
    *b"BET", *b"BGL", *b"BGM", *b"BGN", *b"BGO", *b"BHD", *b"BIF", *b"BIS", *b"BLR", *b"BMD",
    *b"BNB", *b"BND", *b"BNK", *b"BNT", *b"BOB", *b"BOL", *b"BOP", *b"BOV", *b"BRB", *b"BRC",
    *b"BRE", *b"BRL", *b"BRN", *b"BRR", *b"BRZ", *b"BSD", *b"BSV", *b"BTC", *b"BTN", *b"BTT",
    *b"BUK", *b"BUY", *b"BWP", *b"BYB", *b"BYN", *b"BYR", *b"BZD", *b"CAD", *b"CAK", *b"CDF",
    *b"CEL", *b"CFX", *b"CHE", *b"CHF", *b"CHW", *b"CHZ", *b"CKB", *b"CLF", *b"CLP", *b"CMP",
    *b"CNY", *b"COP", *b"COR", *b"COU", *b"CPI", *b"CRC", *b"CRD", *b"CRO", *b"CRV", *b"CSC",
    *b"CSD", *b"CSJ", *b"CSK", *b"CUC", *b"CUP", *b"CVC", *b"CVE", *b"CYP", *b"CZK", *b"DAI",
    *b"DCR", *b"DDM", *b"DEM", *b"DGB", *b"DIE", *b"DJF", *b"DKK", *b"DOG", *b"DOP", *b"DOT",
    *b"DSH", *b"DYX", *b"DZD", *b"ECS", *b"ECV", *b"EEK", *b"EGL", *b"EGP", *b"EIG", *b"ELG",
    *b"ELS", *b"ENA", *b"ENJ", *b"EOS", *b"ERN", *b"ESA", *b"ESB", *b"ESP", *b"ETB", *b"ETC",
    *b"ETF", *b"ETH", *b"EUR", *b"EVR", *b"FCK", *b"FED", *b"FEE", *b"FET", *b"FIL", *b"FIM",
    *b"FJD", *b"FKP", *b"FLK", *b"FLR", *b"FLW", *b"FRF", *b"FTM", *b"FTT", *b"FUK", *b"GAL",
    *b"GBP", *b"GBX", *b"GDP", *b"GEK", *b"GEL", *b"GHC", *b"GHP", *b"GHS", *b"GIP", *b"GLM",
    *b"GMD", *b"GMT", *b"GMX", *b"GNE", *b"GNF", *b"GNO", *b"GNS", *b"GOD", *b"GQE", *b"GRD",
    *b"GRT", *b"GTQ", *b"GUN", *b"GWE", *b"GWP", *b"GYD", *b"HBR", *b"HKD", *b"HNL", *b"HOT",
    *b"HRD", *b"HRK", *b"HTG", *b"HTX", *b"HUF", *b"HYP", *b"ICP", *b"ICX", *b"IDR", *b"IEP",
    *b"ILP", *b"ILR", *b"ILS", *b"IMF", *b"IMX", *b"INJ", *b"INR", *b"IOT", *b"IOU", *b"IPO",
    *b"IQD", *b"IRR", *b"ISJ", *b"ISK", *b"ITL", *b"JMD", *b"JOD", *b"JPY", *b"JUN", *b"JUP",
    *b"KAS", *b"KAV", *b"KCS", *b"KES", *b"KGS", *b"KHR", *b"KKK", *b"KLY", *b"KMD", *b"KMF",
    *b"KNC", *b"KPW", *b"KRH", *b"KRO", *b"KRW", *b"KSM", *b"KWD", *b"KYD", *b"KZT", *b"LAJ",
    *b"LAK", *b"LBP", *b"LDO", *b"LEO", *b"LKR", *b"LNK", *b"LRC", *b"LRD", *b"LSK", *b"LSL",
    *b"LSM", *b"LTC", *b"LTL", *b"LTT", *b"LUC", *b"LUF", *b"LUL", *b"LUN", *b"LVL", *b"LVR",
    *b"LYD", *b"MAD", *b"MAF", *b"MAN", *b"MAT", *b"MCF", *b"MDC", *b"MDL", *b"MGA", *b"MGF",
    *b"MIN", *b"MIR", *b"MKD", *b"MKN", *b"MKR", *b"MLF", *b"MLN", *b"MMK", *b"MNA", *b"MNT",
    *b"MOP", *b"MRO", *b"MRU", *b"MTL", *b"MTP", *b"MUR", *b"MVP", *b"MVR", *b"MWK", *b"MXN",
    *b"MXP", *b"MXV", *b"MYR", *b"MZE", *b"MZM", *b"MZN", *b"NAD", *b"NEO", *b"NER", *b"NGN",
    *b"NIC", *b"NIO", *b"NLG", *b"NMR", *b"NOK", *b"NOT", *b"NPR", *b"NZD", *b"OGN", *b"OKB",
    *b"OMG", *b"OMR", *b"OND", *b"ONE", *b"ONT", *b"OPX", *b"ORC", *b"OSM", *b"PAB", *b"PAX",
    *b"PAY", *b"PEH", *b"PEI", *b"PEN", *b"PEP", *b"PES", *b"PGK", *b"PHP", *b"PKR", *b"PLN",
    *b"PLZ", *b"PND", *b"POL", *b"PTE", *b"PYG", *b"PYT", *b"QAR", *b"QNT", *b"QTU", *b"RAY",
    *b"RHD", *b"RND", *b"ROI", *b"ROK", *b"ROL", *b"RON", *b"RPL", *b"RSD", *b"RSE", *b"RUB",
    *b"RUN", *b"RUR", *b"RVN", *b"RWF", *b"SAR", *b"SBD", *b"SCR", *b"SDD", *b"SDG", *b"SDP",
    *b"SEC", *b"SEI", *b"SEK", *b"SEX", *b"SGB", *b"SGD", *b"SHB", *b"SHP", *b"SIT", *b"SKK",
    *b"SLE", *b"SLL", *b"SND", *b"SNX", *b"SOL", *b"SOS", *b"SRD", *b"SRG", *b"SRK", *b"SRM",
    *b"SSP", *b"SSV", *b"STD", *b"STN", *b"STX", *b"SUI", *b"SUN", *b"SUR", *b"SUS", *b"SVC",
    *b"SYP", *b"SZL", *b"TAO", *b"TAX", *b"THB", *b"TIA", *b"TJR", *b"TJS", *b"TMM", *b"TMT",
    *b"TND", *b"TON", *b"TOP", *b"TPE", *b"TRL", *b"TRX", *b"TRY", *b"TTD", *b"TWD", *b"TZS",
    *b"UAH", *b"UAK", *b"UGS", *b"UGX", *b"UNI", *b"USC", *b"USD", *b"USN", *b"USS", *b"UST",
    *b"UYI", *b"UYP", *b"UYU", *b"UYW", *b"UZS", *b"VEB", *b"VED", *b"VEF", *b"VEN", *b"VES",
    *b"VET", *b"VNC", *b"VND", *b"VUV", *b"WAR", *b"WAV", *b"WBT", *b"WIF", *b"WIN", *b"WLD",
    *b"WST", *b"WTC", *b"WTF", *b"XAF", *b"XAG", *b"XAH", *b"XAU", *b"XBA", *b"XBB", *b"XBC",
    *b"XBD", *b"XBT", *b"XCD", *b"XCG", *b"XCH", *b"XCN", *b"XDC", *b"XDR", *b"XEC", *b"XEM",
    *b"XEU", *b"XFO", *b"XFU", *b"XLM", *b"XMR", *b"XNO", *b"XOF", *b"XPD", *b"XPF", *b"XPR",
    *b"XPT", *b"XRE", *b"XRP", *b"XSU", *b"XTS", *b"XTZ", *b"XUA", *b"XVG", *b"XXX", *b"YDD",
    *b"YER", *b"YFI", *b"YUD", *b"YUM", *b"YUN", *b"YUR", *b"ZAL", *b"ZAR", *b"ZEC", *b"ZIL",
    *b"ZMK", *b"ZMW", *b"ZRN", *b"ZRX", *b"ZRZ", *b"ZWC", *b"ZWD", *b"ZWG", *b"ZWL", *b"ZWN",
    *b"ZWR",
];

/// Returns `true` when `bytes` is on the real-currency, crypto ticker, or reserved deny-list.
#[must_use]
pub fn is_denied_currency_code(bytes: [u8; 3]) -> bool {
    DENIED_CURRENCY_CODES.binary_search(&bytes).is_ok()
}

impl CurrencyCode {
    pub(crate) const fn from_allowed_bytes_unchecked(bytes: [u8; 3]) -> Self {
        Self(bytes)
    }

    /// Constructs a [`CurrencyCode`] if `bytes` are 3 ASCII uppercase letters strictly inside
    /// [`ALLOWED_COMMODITY_GLOSSARY`] and outside [`DENIED_CURRENCY_CODES`].
    pub fn from_bytes(bytes: [u8; 3]) -> Result<Self, FeasibilityError> {
        if !bytes.iter().all(|b| b.is_ascii_uppercase()) {
            return Err(FeasibilityError::InvalidTable(
                "currency code must contain 3 uppercase ASCII letters".to_owned(),
            ));
        }
        if is_denied_currency_code(bytes) {
            return Err(FeasibilityError::InvalidTable(
                "currency code uses a denied real-world currency, crypto ticker, or reserved code"
                    .to_owned(),
            ));
        }
        if !is_allowed_commodity_code(bytes) {
            return Err(FeasibilityError::InvalidTable(
                "currency code is not in the curated game-word allow-list".to_owned(),
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

/// Tier-3 Syndicate Project definition emitted for table setup (`1..=N` chiral pair charters
/// plus reserved id `100` Syndicate Concordat requiring all `N` refined goods).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSpec {
    /// Project id (`1..=N` for chiral neighbor pairs, `100` for the full-ring Concordat).
    pub id: u8,
    /// Human-readable project charter title.
    pub name: String,
    /// Required refined good inputs `(refined_good_index, code, units_per_batch)`.
    pub required_inputs: Vec<(u8, CurrencyCode, f64)>,
    /// Irreversible Project Equity points minted per completed batch.
    pub output_project_equity: f64,
    /// Optional per-turn completion limit (`None` = unconstrained).
    pub max_completions_per_turn: Option<u32>,
}

/// Terminal hard-credit solvency tier used by Turn-1 Charter Mandates (`MaintainCleanCredit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CreditRatingTier {
    /// Clean balance sheet with zero hard credit defaults or escrow breaches.
    Aaa = 0,
    /// Minor transitory distress resolved before close.
    Bbb = 1,
    /// Elevated credit stress.
    B = 2,
    /// Hard credit default.
    D = 3,
}

/// Objective predicate for a Turn-1 Private Charter Mandate card, keeping structural thresholds
/// within B1.2's monopoly-safe bounds (`1..=seat_count - 2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MandateObjective {
    /// Accumulate at least `target` irreversible Project Equity points by season close.
    MinProjectEquity {
        /// Minimum irreversible Project Equity threshold (`> 0`).
        target: f64,
    },
    /// Hold at least `min_each` units of at least `distinct_goods` (`1..=N-2`) refined goods.
    MinRefinedVariety {
        /// Required distinct refined goods held (`1..=seat_count - 2`).
        distinct_goods: u8,
        /// Minimum units required for each counted refined good (`> 0`).
        min_each: f64,
    },
    /// Maintain hard credit solvency at or above `min_tier` (`Aaa` or `Bbb`).
    MaintainCleanCredit {
        /// Cleanest tolerated terminal credit tier.
        min_tier: CreditRatingTier,
    },
    /// Complete positive deliveries with at least `min_partners` (`1..=N-2`) distinct counterparties.
    MinActivePartners {
        /// Required distinct counterparties (`1..=seat_count - 2`).
        min_partners: u8,
    },
}

impl MandateObjective {
    /// Validates objective parameters against `seat_count`, strictly enforcing B1.2's
    /// `1..=seat_count - 2` ceiling on `distinct_goods` and `min_partners`.
    pub fn validate_for_seat_count(&self, seat_count: SeatCount) -> Result<(), FeasibilityError> {
        let max_safe = seat_count.as_u8().saturating_sub(2);
        match *self {
            Self::MinProjectEquity { target } => {
                if !target.is_finite() || target <= 0.0 {
                    return Err(FeasibilityError::InvalidTable(
                        "mandate MinProjectEquity target must be finite and > 0".to_owned(),
                    ));
                }
            }
            Self::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => {
                if distinct_goods == 0 || distinct_goods > max_safe {
                    return Err(FeasibilityError::InvalidTable(format!(
                        "mandate MinRefinedVariety distinct_goods {distinct_goods} outside 1..={max_safe}"
                    )));
                }
                if !min_each.is_finite() || min_each <= 0.0 {
                    return Err(FeasibilityError::InvalidTable(
                        "mandate MinRefinedVariety min_each must be finite and > 0".to_owned(),
                    ));
                }
            }
            Self::MaintainCleanCredit { min_tier } => {
                if matches!(min_tier, CreditRatingTier::B | CreditRatingTier::D) {
                    return Err(FeasibilityError::InvalidTable(
                        "mandate MaintainCleanCredit must require Aaa or Bbb solvency".to_owned(),
                    ));
                }
            }
            Self::MinActivePartners { min_partners } => {
                if min_partners == 0 || min_partners > max_safe {
                    return Err(FeasibilityError::InvalidTable(format!(
                        "mandate MinActivePartners min_partners {min_partners} outside 1..={max_safe}"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// One card in the public Turn-1 graded Charter Mandate deck (`3.0..=8.0 VP` scale per T5 / A7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MandateCardSpec {
    /// Stable deck slot identifier (`1..=M`, widened to `u16` matching [`league_core::GradedMandateCard`]).
    pub card_id: u16,
    /// Structural win-condition objective.
    pub kind: MandateObjective,
    /// Provisional or rollout-measured completion frequency (`0..=10_000 bps`).
    pub difficulty_completion_bps: u16,
    /// Graded Victory Point bonus in `[MIN_MANDATE_BONUS_VP, MAX_MANDATE_BONUS_VP]` (`3.0..=8.0`).
    pub bonus_vp: f64,
}

/// Maps empirical/structural completion frequency (`0..=10_000 bps`) monotonically onto
/// `[MIN_MANDATE_BONUS_VP, MAX_MANDATE_BONUS_VP]` (`3.0..=8.0 VP`), delegating directly to
/// [`league_core::bonus_points_milli_for_completion_bps`] (`#32` `B-2`).
#[must_use]
pub fn bonus_vp_for_completion_bps(completion_bps: u16) -> f64 {
    (league_core::bonus_points_milli_for_completion_bps(completion_bps) as f64) / 1_000.0
}

impl From<league_core::CreditTier> for CreditRatingTier {
    fn from(tier: league_core::CreditTier) -> Self {
        match tier {
            league_core::CreditTier::Aaa => Self::Aaa,
            league_core::CreditTier::Bbb => Self::Bbb,
            league_core::CreditTier::B => Self::B,
            league_core::CreditTier::D => Self::D,
        }
    }
}

impl From<CreditRatingTier> for league_core::CreditTier {
    fn from(tier: CreditRatingTier) -> Self {
        match tier {
            CreditRatingTier::Aaa => Self::Aaa,
            CreditRatingTier::Bbb => Self::Bbb,
            CreditRatingTier::B => Self::B,
            CreditRatingTier::D => Self::D,
        }
    }
}

impl From<league_core::MandateKind> for MandateObjective {
    fn from(kind: league_core::MandateKind) -> Self {
        match kind {
            league_core::MandateKind::MinProjectEquity { target } => Self::MinProjectEquity {
                target: (target as f64) / 1_000.0,
            },
            league_core::MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => Self::MinRefinedVariety {
                distinct_goods,
                min_each: (min_each as f64) / 1_000.0,
            },
            league_core::MandateKind::MaintainCleanCredit { min_tier } => {
                Self::MaintainCleanCredit {
                    min_tier: CreditRatingTier::from(min_tier),
                }
            }
            league_core::MandateKind::MinActivePartners { min_partners } => {
                Self::MinActivePartners { min_partners }
            }
        }
    }
}

impl From<league_core::GradedMandateCard> for MandateCardSpec {
    fn from(card: league_core::GradedMandateCard) -> Self {
        Self {
            card_id: card.card_id,
            kind: MandateObjective::from(card.kind),
            difficulty_completion_bps: card.difficulty_completion_bps,
            bonus_vp: (card.bonus_equity as f64) / 1_000.0,
        }
    }
}

impl MandateCardSpec {
    /// Converts this schema card back into [`league_core::GradedMandateCard`] on the exact
    /// milli-unit integer grid (`#32` `B-2`).
    #[must_use]
    pub fn to_core_card(&self) -> league_core::GradedMandateCard {
        let kind = match self.kind {
            MandateObjective::MinProjectEquity { target } => {
                league_core::MandateKind::MinProjectEquity {
                    target: (target * 1_000.0).round() as league_core::Units,
                }
            }
            MandateObjective::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => league_core::MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each: (min_each * 1_000.0).round() as league_core::Units,
            },
            MandateObjective::MaintainCleanCredit { min_tier } => {
                league_core::MandateKind::MaintainCleanCredit {
                    min_tier: league_core::CreditTier::from(min_tier),
                }
            }
            MandateObjective::MinActivePartners { min_partners } => {
                league_core::MandateKind::MinActivePartners { min_partners }
            }
        };
        league_core::GradedMandateCard {
            card_id: self.card_id,
            kind,
            difficulty_completion_bps: self.difficulty_completion_bps,
            bonus_equity: (self.bonus_vp * 1_000.0).round() as league_core::Units,
        }
    }
}

/// Deals `seat_count` distinct [`MandateCardSpec`] indices for `table_id` using
/// [`league_core::deal_mandate_deck_indices`] (`#32` `B-2`, `B1.3`/`M3`), strictly separate from
/// the public table `seed: u64` and rejecting `[0u8; 32]`.
pub fn deal_mandate_indices_with_entropy(
    table_id: league_core::TableId,
    mandate_deck: &[MandateCardSpec],
    seat_count: SeatCount,
    deal_seed: [u8; 32],
) -> Result<Vec<usize>, FeasibilityError> {
    league_core::deal_mandate_deck_indices(
        table_id,
        seat_count.as_u8(),
        deal_seed,
        mandate_deck.len(),
    )
    .map_err(|err| FeasibilityError::InvalidTable(err.to_string()))
}

fn round3_schema(v: f64) -> f64 {
    (v * 1_000.0).round() / 1_000.0
}

/// Builds canonical Tier-3 Syndicate Projects for `seats`: `1..=N` adjacent chiral neighbor
/// pairs plus `100` Syndicate Concordat requiring all `N` distinct refined goods (`C1` held rule).
#[must_use]
pub fn default_projects_for_seats(seats: &[SeatSpec]) -> Vec<ProjectSpec> {
    let n = seats.len();
    let mut projects = Vec::with_capacity(n + 1);
    for i in 0..n {
        let j = (i + 1) % n;
        let eq =
            round3_schema((seats[i].refined_project_value + seats[j].refined_project_value) * 1.05);
        projects.push(ProjectSpec {
            id: (i + 1) as u8,
            name: format!("Pair Charter {}-{}", i + 1, j + 1),
            required_inputs: vec![
                (i as u8, seats[i].refined_code, 1.0),
                (j as u8, seats[j].refined_code, 1.0),
            ],
            output_project_equity: eq,
            max_completions_per_turn: None,
        });
    }
    let concordat_eq =
        round3_schema(seats.iter().map(|s| s.refined_project_value).sum::<f64>() * 1.15);
    let concordat_inputs = (0..n)
        .map(|g| (g as u8, seats[g].refined_code, 1.0))
        .collect();
    projects.push(ProjectSpec {
        id: PROJECT_CONCORDAT_ID,
        name: "Syndicate Concordat".to_owned(),
        required_inputs: concordat_inputs,
        output_project_equity: concordat_eq,
        max_completions_per_turn: None,
    });
    projects
}

/// Builds the canonical 8-card Turn-1 public graded Charter Mandate deck (`3.0..=8.0 VP` scale
/// per `T5`/`A7`), delegating directly to [`league_core::graded_mandate_deck_for_mean_project_value`]
/// so `league-gen` and `league-core` share a single mandate deck source (`#32` `B-2`).
#[must_use]
pub fn default_mandate_deck_for_table(
    seat_count: SeatCount,
    seats: &[SeatSpec],
) -> Vec<MandateCardSpec> {
    let mean_milli = if seats.is_empty() {
        0
    } else {
        let sum: f64 = seats.iter().map(|s| s.refined_project_value).sum();
        let mean = sum / (seats.len() as f64);
        ((mean * (league_core::UNIT_SCALE as f64)).round() as league_core::Units).max(1)
    };
    league_core::graded_mandate_deck_for_mean_project_value(seat_count.as_u8(), mean_milli)
        .into_iter()
        .map(MandateCardSpec::from)
        .collect()
}

/// Complete specification for a generated chiral converter-ring table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingTable {
    /// Generator version string (`GENERATOR_VERSION`) recording exact sampling/filter rules.
    pub generator_version: String,
    /// Full [`GeneratorConfig`] active when this table was sampled so runs stay comparable.
    pub config: GeneratorConfig,
    /// Seed passed to [`crate::generate_ring_table`].
    pub seed: u64,
    /// Zero-based search attempt index that passed the LP feasibility filter.
    pub attempt: u32,
    /// Validated number of seats (`4`, `5`, or `6`).
    pub seat_count: SeatCount,
    /// Fixed table numeraire currency code (`"CRED"` per `docs/design-v2.md` §1.2).
    pub numeraire_code: String,
    /// Inclusive turn bounds `(min_close, max_close)` for the stochastic season close (`(24, 28)`).
    pub season_turn_window: (u8, u8),
    /// Per-seat economic definitions in index order `0..N`.
    pub seats: Vec<SeatSpec>,
    /// Tier-3 Syndicate Projects (`1..=N` chiral pairs plus `100` full-ring Concordat).
    pub projects: Vec<ProjectSpec>,
    /// Public Turn-1 graded Charter Mandate deck (`3.0..=8.0 VP` scale).
    pub mandate_deck: Vec<MandateCardSpec>,
    /// Pre-sampled 3-stage `OracleSet` horizon shock schedule for the season.
    pub shocks: Vec<ScheduledShock>,
}

/// Redacted wrapper around a `sealed`-pool [`RingTable`] returned by
/// [`crate::generate_ring_table_for_purpose`] and [`crate::generate_ring_table_for_setup`] when
/// `purpose == TablePurpose::Sealed`.
///
/// `SealedRingTable` intentionally omits [`serde::Serialize`] and [`serde::Deserialize`], and its
/// [`fmt::Debug`] implementation replaces `table.seed` with `"sealed"` and the lowercase hex
/// SHA-256 of `seed.to_be_bytes()`, while keeping `table.seed` intact inside the inner [`RingTable`]
/// for in-memory table consumers that explicitly unseal it.
///
/// ```compile_fail
/// fn assert_serialize<T: serde::Serialize>() {}
/// assert_serialize::<league_gen::SealedRingTable>();
/// ```
#[derive(Clone, PartialEq)]
pub struct SealedRingTable {
    table: RingTable,
    seed_sha256: String,
}

impl SealedRingTable {
    pub(crate) fn new(table: RingTable) -> Self {
        debug_assert_eq!(seed_pool(table.seed), SeedPool::Sealed);
        let seed_sha256 = sealed_seed_sha256(table.seed);
        Self { table, seed_sha256 }
    }

    /// Returns [`SeedPool::Sealed`].
    #[must_use]
    pub const fn pool(&self) -> SeedPool {
        SeedPool::Sealed
    }

    /// Returns the lowercase 64-hex SHA-256 digest of `seed.to_be_bytes()`.
    #[must_use]
    pub fn sealed_seed_sha256(&self) -> &str {
        &self.seed_sha256
    }

    /// Returns the validated [`SeatCount`] of the wrapped table.
    #[must_use]
    pub const fn seat_count(&self) -> SeatCount {
        self.table.seat_count
    }

    /// Returns the [`GENERATOR_VERSION`] of the wrapped table.
    #[must_use]
    pub fn generator_version(&self) -> &str {
        &self.table.generator_version
    }

    /// Borrows the inner [`RingTable`], whose `seed` field holds the clear `u64` layout seed.
    ///
    /// # Clear-Seed Handling Warning
    /// The returned [`RingTable`] holds the clear `sealed`-range seed in `table.seed` and **must
    /// never be formatted (`Debug`), logged, or serialized (`serde`)**.
    #[must_use]
    pub const fn unseal_table_with_clear_seed(&self) -> &RingTable {
        &self.table
    }

    /// Consumes the wrapper and returns the inner [`RingTable`], whose `seed` field holds the
    /// clear `u64` layout seed.
    ///
    /// # Clear-Seed Handling Warning
    /// The returned [`RingTable`] holds the clear `sealed`-range seed in `table.seed` and **must
    /// never be formatted (`Debug`), logged, or serialized (`serde`)**.
    #[must_use]
    pub fn into_unsealed_table_with_clear_seed(self) -> RingTable {
        self.table
    }
}

impl fmt::Debug for SealedRingTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedRingTable")
            .field("generator_version", &self.table.generator_version)
            .field("config", &self.table.config)
            .field("pool", &SeedPool::Sealed.as_str())
            .field("sealed_seed_sha256", &self.seed_sha256)
            .field("attempt", &self.table.attempt)
            .field("seat_count", &self.table.seat_count)
            .field("numeraire_code", &self.table.numeraire_code)
            .field("season_turn_window", &self.table.season_turn_window)
            .field("seats", &self.table.seats)
            .field("projects", &self.table.projects)
            .field("mandate_deck", &self.table.mandate_deck)
            .field("shocks", &self.table.shocks)
            .finish()
    }
}

/// Purpose-checked table return value of [`crate::generate_ring_table_for_purpose`] and
/// [`crate::generate_ring_table_for_setup`].
///
/// Non-`sealed` purposes (`dev`, `train`, `probe`, `held_out`) return the bare [`RingTable`] inside
/// [`CheckedRingTable::Clear`]. [`TablePurpose::Sealed`] returns [`CheckedRingTable::Sealed`]
/// wrapping [`SealedRingTable`], so `CheckedRingTable` itself omits [`serde::Serialize`] and its
/// [`fmt::Debug`] output never prints a `sealed`-range seed in clear.
///
/// ```compile_fail
/// fn assert_serialize<T: serde::Serialize>() {}
/// assert_serialize::<league_gen::CheckedRingTable>();
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum CheckedRingTable {
    /// Non-`sealed` (`dev`, `train`, `probe`, `held_out`) generated [`RingTable`].
    Clear(RingTable),
    /// `sealed`-pool [`RingTable`] wrapped inside [`SealedRingTable`].
    Sealed(SealedRingTable),
}

impl CheckedRingTable {
    /// Returns `Some(&RingTable)` for non-`sealed` purposes, or `None` for [`CheckedRingTable::Sealed`].
    #[must_use]
    pub const fn clear_table(&self) -> Option<&RingTable> {
        match self {
            Self::Clear(table) => Some(table),
            Self::Sealed(_) => None,
        }
    }

    /// Consumes `self` and returns `Some(RingTable)` for non-`sealed` purposes, or `None` for
    /// [`CheckedRingTable::Sealed`].
    #[must_use]
    pub fn into_clear_table(self) -> Option<RingTable> {
        match self {
            Self::Clear(table) => Some(table),
            Self::Sealed(_) => None,
        }
    }

    /// Returns `Some(&SealedRingTable)` for [`TablePurpose::Sealed`], or `None` otherwise.
    #[must_use]
    pub const fn sealed_table(&self) -> Option<&SealedRingTable> {
        match self {
            Self::Clear(_) => None,
            Self::Sealed(sealed) => Some(sealed),
        }
    }

    /// Returns the validated [`SeatCount`] of the generated table regardless of pool.
    #[must_use]
    pub const fn seat_count(&self) -> SeatCount {
        match self {
            Self::Clear(table) => table.seat_count,
            Self::Sealed(sealed) => sealed.seat_count(),
        }
    }
}

/// Parameters controlling chiral table generation and LP feasibility filtering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Optional fixed seat count (`None` draws uniformly once per seed over `4`, `5`, `6`,
    /// per `docs/design-v2.md` §1.5 `N = 4, 5, 6`).
    pub seat_count: Option<SeatCount>,
    /// Maximum allowed relative deviation of any seat's EV from the table mean (`0.03` = ±3%,
    /// from `docs/design-v2.md` §1.5 item 3 and seed-26 report §"Static board layouts…").
    /// Enforced alongside the 7% [`SHAPLEY_GUARD_MAX_REL_DEV`] sub-coalition Shapley guard.
    pub ev_tolerance: f64,
    /// Minimum required `(net_converter_surplus_value / aggregate_upkeep_value)` margin (`0.15`;
    /// provisional, calibrate in playtest 1 — `docs/design-v2.md` §1.5 requires a positive margin).
    pub min_surplus_ratio: f64,
    /// Whether 4-, 5-, and 6-seat tables attach binding primary workshops paired with cyclic
    /// secondary cross-converters (`(R_{i+2}, R_{i+3}) -> M_i` on 4p; binding 3-input
    /// `(R_{i+1}, R_{i+2}, R_{i+3})` plus clearing `(R_{i+2}, R_{i+4})` on 5p so every 4-seat bloc
    /// covers at most 2/4 upkeep; binding `(R_{i+1}, R_{i+3})` plus clearing `(R_{i+2}, R_{i+4})`
    /// on 6p; `true` per `docs/design-v2.md` §1.3 & §1.5 and the 2026-11-15 rulings; provisional
    /// calibration, calibrate in playtest 1).
    pub enable_secondary_converters: bool,
    /// Upper bound on initial baseline export `TransferRate` tariffs (`0.20` = 20% from
    /// `docs/design-v2.md` §1.5 item 3 "0%–20% initial baseline", under the §5.1 35% cap).
    pub max_tariff: f64,
    /// Inclusive `(min, max)` bounds for initial perishable `Clawback` decay rates (`(0.15, 0.25)`,
    /// provisional ±5% band around the `20%`/turn baseline in `docs/design-v2.md` §1.4).
    pub decay_range: (f64, f64),
    /// Maximum candidate sampling attempts before returning [`GenError::NoFeasibleTableFound`]
    /// (`512`, capped at [`MAX_ATTEMPTS_CAP`]).
    pub max_attempts: u32,
    /// Seat expected-value allocation method for Invariant 3 ([`EvMethod::ShadowPrice`] default
    /// per `docs/design-v2.md` §1.5 "optimal competitive/cooperative shadow-price allocation");
    /// [`SHAPLEY_GUARD_MAX_REL_DEV`] (`7%`) is enforced across every method.
    pub ev_method: EvMethod,
    /// Optional override for the provisional C6 table-level top-pair share cap (`None` uses
    /// [`SeatCount::default_max_pair_share_cap`]: `4p: 0.45`, `5p: 0.35`, `6p: 0.30`; the 2026-11-15
    /// ruling gave no numeric cap values and these defaults do not bind on generated tables today).
    pub max_pair_share_cap: Option<f64>,
}

impl Default for GeneratorConfig {
    /// Defaults combine `docs/design-v2.md` §1.4–§1.5 baselines with Director v2 rulings (`G1`/`C6`):
    /// - `seat_count: None` (`docs/design-v2.md` §1.5: uniform draw over `N in {4, 5, 6}`)
    /// - `ev_tolerance: 0.03` (`docs/design-v2.md` §1.5 item 3 & seed-26 report)
    /// - `min_surplus_ratio: 0.15` (provisional, calibrate in playtest 1)
    /// - `enable_secondary_converters: true` (`docs/design-v2.md` §1.3 & §1.5)
    /// - `max_tariff: 0.20` (`docs/design-v2.md` §1.5 item 3: 0%–20% initial baseline)
    /// - `decay_range: (0.15, 0.25)` (provisional ±5% band around `docs/design-v2.md` §1.4 20%/turn)
    /// - `max_attempts: 512` (stationary sampling retry cap; G1 removes dispersion annealing)
    /// - `ev_method: EvMethod::ShadowPrice` (`docs/design-v2.md` §1.5 shadow-price allocation)
    /// - `max_pair_share_cap: None` (provisional defaults: `4p: 0.45`, `5p: 0.35`, `6p: 0.30`;
    ///   2026-11-15 ruling gave no values and these caps do not bind on generated tables today)
    fn default() -> Self {
        Self {
            seat_count: None,
            ev_tolerance: 0.03,
            min_surplus_ratio: 0.15,
            enable_secondary_converters: true,
            max_tariff: 0.20,
            decay_range: (0.15, 0.25),
            max_attempts: 512,
            ev_method: EvMethod::ShadowPrice,
            max_pair_share_cap: None,
        }
    }
}

/// Reasons a candidate [`RingTable`] can fail structural or LP feasibility checks.
#[derive(Debug, Clone, PartialEq)]
pub enum FeasibilityError {
    /// Candidate table failed structural validation (indices, codes, positive quantities, shock timing,
    /// converter positive clearing value-added, projects, or mandate bounds).
    InvalidTable(String),
    /// At least one 1-seat or 2-seat subset satisfied non-zero internal upkeep without external trade.
    PairCoversUpkeep {
        /// Ordered pair `(i, j)` of seats (`i <= j`).
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
    /// At least one seat's EV falls outside `±ev_tolerance` of the table mean.
    SeatEvOutOfTolerance {
        /// Maximum relative deviation `|(seat_ev - mean_ev) / mean_ev|` observed.
        max_relative_deviation: f64,
        /// Maximum allowed relative deviation (`config.ev_tolerance`).
        tolerance: f64,
        /// Per-seat expected values under the active [`EvMethod`].
        seat_evs: Vec<f64>,
    },
    /// At least one seat's exact `2^N` sub-coalition Shapley EV deviates by more than
    /// [`SHAPLEY_GUARD_MAX_REL_DEV`] (`7%`) from the seats' Shapley mean (Director ruling,
    /// `2026-11-16`).
    ShapleyGuardExceeded {
        /// Maximum relative Shapley deviation `max_i |(ev_i - mean_ev) / mean_ev|` observed.
        max_relative_deviation: f64,
        /// Maximum allowed relative Shapley deviation ([`SHAPLEY_GUARD_MAX_REL_DEV`]).
        max_allowed_deviation: f64,
        /// Per-seat Shapley expected values from [`crate::compute_shapley_seat_evs`].
        shapley_seat_evs: Vec<f64>,
    },
    /// Optimal ring LP trade share on a single unordered pair exceeded the C6 table cap.
    PairShareExceedsCap {
        /// Ordered seat pair `(i, j)` with `i < j`.
        pair: (u8, u8),
        /// Observed share of total bilateral volume carried by `(i, j)`.
        share: f64,
        /// Enforced upper cap (`0.45 / 0.35 / 0.30` or custom override).
        cap: f64,
    },
    /// Deterministic multi-agent rollout grading failed B2.2 acceptance checks.
    RolloutGateFailed(String),
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
            Self::ShapleyGuardExceeded {
                max_relative_deviation,
                max_allowed_deviation,
                ..
            } => write!(
                f,
                "seat Shapley EV deviation {max_relative_deviation:.4} exceeds guard limit {max_allowed_deviation:.4}"
            ),
            Self::PairShareExceedsCap { pair, share, cap } => write!(
                f,
                "pair ({}, {}) volume share {share:.4} exceeds cap {cap:.4}",
                pair.0, pair.1
            ),
            Self::RolloutGateFailed(msg) => write!(f, "rollout grading failed: {msg}"),
            Self::SolverFailure(msg) => write!(f, "LP solver error: {msg}"),
        }
    }
}

impl std::error::Error for FeasibilityError {}

impl FeasibilityError {
    /// Returns the stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidTable(_) => "INVALID_RING_TABLE",
            Self::PairCoversUpkeep { .. } => "PAIR_COVERS_UPKEEP",
            Self::InsufficientSurplus { .. } => "INSUFFICIENT_SURPLUS",
            Self::SeatEvOutOfTolerance { .. } => "SEAT_EV_OUT_OF_TOLERANCE",
            Self::ShapleyGuardExceeded { .. } => "SHAPLEY_GUARD_EXCEEDED",
            Self::PairShareExceedsCap { .. } => "PAIR_SHARE_EXCEEDED",
            Self::RolloutGateFailed(_) => "ROLLOUT_GATE_FAILED",
            Self::SolverFailure(_) => "LP_SOLVER_FAILURE",
        }
    }
}

impl league_core::ToStructuredError for FeasibilityError {
    fn to_structured_error(&self) -> league_core::StructuredError {
        let hint = match self {
            Self::InvalidTable(_) => {
                "fix structural RingTable fields, commodity codes, recipes, projects, or shock bounds"
            }
            Self::PairCoversUpkeep { .. } => {
                "resample chiral recipes so no 1- or 2-seat subset covers its own upkeep"
            }
            Self::InsufficientSurplus { .. } => {
                "increase converter yields or lower upkeep/min_surplus_ratio"
            }
            Self::SeatEvOutOfTolerance { .. } => {
                "resample candidate parameters or adjust GeneratorConfig::ev_tolerance"
            }
            Self::ShapleyGuardExceeded { .. } => {
                "resample table parameters so every seat stays within the 7% Shapley guard"
            }
            Self::PairShareExceedsCap { .. } => {
                "resample converter input shares or adjust GeneratorConfig::max_pair_share_cap"
            }
            Self::RolloutGateFailed(_) => {
                "inspect rollout grading metrics or resample the candidate RingTable"
            }
            Self::SolverFailure(_) => {
                "verify LP constraints are finite, bounded, and well-conditioned"
            }
        };
        league_core::StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}

/// Checked payload carried by [`GenError::SeedPoolMismatch`].
///
/// Fields are private so callers outside the crate can construct this payload only via
/// [`GenError::seed_pool_mismatch`], guaranteeing `seed_pool == seed_pool(raw_seed)` and
/// SHA-256 redaction for [`SeedPool::Sealed`].
///
/// ```compile_fail
/// use league_gen::{RecordedLayoutSeed, SeedPool, SeedPoolMismatchError, TablePurpose};
/// let _ = SeedPoolMismatchError {
///     seed: RecordedLayoutSeed::from_seed(42),
///     seed_pool: SeedPool::Sealed,
///     purpose: TablePurpose::Dev,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedPoolMismatchError {
    seed: RecordedLayoutSeed,
    seed_pool: SeedPool,
    purpose: TablePurpose,
}

impl SeedPoolMismatchError {
    /// Recorded layout seed (clear `u64` for non-`sealed` pools, 8-byte big-endian SHA-256 for `sealed`).
    #[must_use]
    pub const fn seed(&self) -> &RecordedLayoutSeed {
        &self.seed
    }

    /// Pool returned by [`seed_pool`].
    #[must_use]
    pub const fn seed_pool(&self) -> SeedPool {
        self.seed_pool
    }

    /// Requested [`TablePurpose`].
    #[must_use]
    pub const fn purpose(&self) -> TablePurpose {
        self.purpose
    }
}

/// Checked payload carried by [`GenError::TrainingLoggedMismatch`].
///
/// Fields are private so callers outside the crate can construct this payload only via
/// [`GenError::training_logged_mismatch`], guaranteeing `seed_pool == seed_pool(raw_seed)` and
/// SHA-256 redaction for [`SeedPool::Sealed`].
///
/// ```compile_fail
/// use league_gen::{RecordedLayoutSeed, SeedPool, TablePurpose, TrainingLoggedMismatchError};
/// let _ = TrainingLoggedMismatchError {
///     seed: RecordedLayoutSeed::from_seed(42),
///     seed_pool: SeedPool::Sealed,
///     purpose: TablePurpose::Sealed,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingLoggedMismatchError {
    seed: RecordedLayoutSeed,
    seed_pool: SeedPool,
    purpose: TablePurpose,
}

impl TrainingLoggedMismatchError {
    /// Recorded layout seed (clear `u64` for non-`sealed` pools, 8-byte big-endian SHA-256 for `sealed`).
    #[must_use]
    pub const fn seed(&self) -> &RecordedLayoutSeed {
        &self.seed
    }

    /// Pool returned by [`seed_pool`].
    #[must_use]
    pub const fn seed_pool(&self) -> SeedPool {
        self.seed_pool
    }

    /// Requested non-`train` [`TablePurpose`].
    #[must_use]
    pub const fn purpose(&self) -> TablePurpose {
        self.purpose
    }
}

/// Errors returned by table generation, seed-pool checks, and feasibility configuration checks.
#[derive(Debug, Clone, PartialEq)]
pub enum GenError {
    /// Layout seed's [`SeedPool`] did not match the requested [`TablePurpose`] (`SEED_POOL_MISMATCH`).
    SeedPoolMismatch(SeedPoolMismatchError),
    /// `training_logged = true` was requested for a [`TablePurpose`] other than [`TablePurpose::Train`].
    TrainingLoggedMismatch(TrainingLoggedMismatchError),
    /// Generator configuration contained invalid bounds, zero attempts, or exceeded `MAX_ATTEMPTS_CAP`.
    InvalidConfig(String),
    /// No feasible table passed all three LP invariants within `max_attempts`.
    NoFeasibleTableFound {
        /// Number of attempts tested.
        attempts: u32,
        /// Failure recorded on the final attempt.
        last_error: FeasibilityError,
    },
}

impl GenError {
    /// Constructs a [`GenError::SeedPoolMismatch`] from `seed` and `purpose`, computing
    /// [`seed_pool`] and redacting `sealed`-pool seeds to the SHA-256 of their 8 big-endian bytes.
    #[must_use]
    pub fn seed_pool_mismatch(seed: u64, purpose: TablePurpose) -> Self {
        Self::SeedPoolMismatch(SeedPoolMismatchError {
            seed: RecordedLayoutSeed::from_seed(seed),
            seed_pool: seed_pool(seed),
            purpose,
        })
    }

    /// Constructs a [`GenError::TrainingLoggedMismatch`] from `seed` and `purpose`, computing
    /// [`seed_pool`] and redacting `sealed`-pool seeds to the SHA-256 of their 8 big-endian bytes.
    #[must_use]
    pub fn training_logged_mismatch(seed: u64, purpose: TablePurpose) -> Self {
        Self::TrainingLoggedMismatch(TrainingLoggedMismatchError {
            seed: RecordedLayoutSeed::from_seed(seed),
            seed_pool: seed_pool(seed),
            purpose,
        })
    }
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeedPoolMismatch(err) => match &err.seed.0 {
                RecordedLayoutSeedRepr::Clear(raw_seed) => write!(
                    f,
                    "SEED_POOL_MISMATCH: layout seed {raw_seed} belongs to pool '{}', refused for purpose '{}'",
                    err.seed_pool, err.purpose
                ),
                RecordedLayoutSeedRepr::SealedSha256(digest) => write!(
                    f,
                    "SEED_POOL_MISMATCH: layout seed in pool '{}' (sha256={digest}) refused for purpose '{}'",
                    err.seed_pool, err.purpose
                ),
            },
            Self::TrainingLoggedMismatch(err) => match &err.seed.0 {
                RecordedLayoutSeedRepr::Clear(raw_seed) => write!(
                    f,
                    "TRAINING_LOGGED_MISMATCH: training_logged=true requires purpose 'train', refused for purpose '{}' (pool '{}', seed {raw_seed})",
                    err.purpose, err.seed_pool
                ),
                RecordedLayoutSeedRepr::SealedSha256(digest) => write!(
                    f,
                    "TRAINING_LOGGED_MISMATCH: training_logged=true requires purpose 'train', refused for purpose '{}' (pool '{}', sha256={digest})",
                    err.purpose, err.seed_pool
                ),
            },
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

impl GenError {
    /// Returns the stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SeedPoolMismatch(_) => "SEED_POOL_MISMATCH",
            Self::TrainingLoggedMismatch(_) => "TRAINING_LOGGED_MISMATCH",
            Self::InvalidConfig(_) => "INVALID_GENERATOR_CONFIG",
            Self::NoFeasibleTableFound { .. } => "NO_FEASIBLE_TABLE_FOUND",
        }
    }
}

impl league_core::ToStructuredError for GenError {
    fn to_structured_error(&self) -> league_core::StructuredError {
        match self {
            Self::SeedPoolMismatch(_) => league_core::StructuredError::new(
                self.code(),
                self.to_string(),
                "choose a non-reserved layout seed belonging to the requested TablePurpose pool",
                false,
            ),
            Self::TrainingLoggedMismatch(_) => league_core::StructuredError::new(
                self.code(),
                self.to_string(),
                "set training_logged = false for any TablePurpose other than Train",
                false,
            ),
            Self::InvalidConfig(_) => league_core::StructuredError::new(
                self.code(),
                self.to_string(),
                "check GeneratorConfig bounds, RingTable fields, and seed eligibility",
                false,
            ),
            Self::NoFeasibleTableFound {
                attempts,
                last_error,
            } => league_core::StructuredError::new(
                self.code(),
                format!("no feasible chiral table found in {attempts} attempts"),
                "increase max_attempts or relax GeneratorConfig tolerances",
                false,
            )
            .with_cause(last_error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

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
    fn denied_currency_codes_are_strictly_sorted_and_reject_real_and_crypto_tickers() {
        assert!(
            DENIED_CURRENCY_CODES
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "DENIED_CURRENCY_CODES must remain strictly sorted for binary_search"
        );
        for ticker in [
            *b"XRP", *b"BTC", *b"ETH", *b"SOL", *b"XLM", *b"DOT", *b"KGS", *b"AZN", *b"BHD",
            *b"TWD", *b"CZK", *b"NGN", *b"DZD", *b"USD", *b"EUR", *b"GBP", *b"JPY", *b"CHF",
            *b"XAU", *b"XAG", *b"CRD", *b"LEO", *b"BSV", *b"FTT", *b"FLR", *b"SGB", *b"EVR",
            *b"GBX", *b"ELS", *b"ANC",
        ] {
            assert!(is_denied_currency_code(ticker));
            assert!(CurrencyCode::from_bytes(ticker).is_err());
        }
        assert_eq!(NUMERAIRE_CODE, "CRED");
        assert_eq!(EvMethod::default(), EvMethod::ShadowPrice);
    }

    #[test]
    fn allowed_commodity_glossary_is_strictly_sorted_disjoint_and_wide() {
        assert!(
            ALLOWED_COMMODITY_GLOSSARY.len() >= 60,
            "glossary must hold at least 60 allow-listed goods (has {})",
            ALLOWED_COMMODITY_GLOSSARY.len()
        );
        assert!(
            ALLOWED_COMMODITY_GLOSSARY
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "ALLOWED_COMMODITY_GLOSSARY must remain strictly sorted for binary_search"
        );
        for &(code, word) in ALLOWED_COMMODITY_GLOSSARY {
            assert!(
                code.iter().all(|b| b.is_ascii_uppercase()),
                "allow-list code {code:?} must be uppercase ASCII"
            );
            assert!(
                !word.is_empty(),
                "allow-list code {code:?} must have non-empty word"
            );
            assert!(
                !is_denied_currency_code(code),
                "allow-list code {code:?} collides with DENIED_CURRENCY_CODES"
            );
            let Ok(parsed) = CurrencyCode::from_bytes(code) else {
                panic!("allow-listed code {code:?} must parse");
            };
            assert_eq!(commodity_glossary_word(parsed), Some(word));
        }
        // Arbitrary non-allow-listed 3-letter codes are rejected even when absent from deny-list.
        assert!(CurrencyCode::from_bytes(*b"QRA").is_err());
        assert!(CurrencyCode::from_bytes(*b"ZZQ").is_err());
    }

    #[test]
    fn seat_count_pair_share_caps_and_mandate_validation() {
        assert!((SeatCount::Four.default_max_pair_share_cap() - 0.45).abs() < 1e-12);
        assert!((SeatCount::Five.default_max_pair_share_cap() - 0.35).abs() < 1e-12);
        assert!((SeatCount::Six.default_max_pair_share_cap() - 0.30).abs() < 1e-12);
        assert!((bonus_vp_for_completion_bps(0) - MAX_MANDATE_BONUS_VP).abs() < 1e-12);
        assert!((bonus_vp_for_completion_bps(BPS_SCALE) - MIN_MANDATE_BONUS_VP).abs() < 1e-12);

        for sc in SeatCount::ALL {
            let max_safe = sc.as_u8() - 2;
            assert!(
                MandateObjective::MinRefinedVariety {
                    distinct_goods: max_safe,
                    min_each: 1.5,
                }
                .validate_for_seat_count(sc)
                .is_ok()
            );
            assert!(
                MandateObjective::MinRefinedVariety {
                    distinct_goods: 0,
                    min_each: 1.5,
                }
                .validate_for_seat_count(sc)
                .is_err()
            );
            assert!(
                MandateObjective::MinRefinedVariety {
                    distinct_goods: max_safe + 1,
                    min_each: 1.5,
                }
                .validate_for_seat_count(sc)
                .is_err()
            );
            assert!(
                MandateObjective::MinActivePartners {
                    min_partners: max_safe + 1,
                }
                .validate_for_seat_count(sc)
                .is_err()
            );
        }
    }

    #[test]
    fn mandate_entropy_deal_rejects_zero_seed_and_deals_distinct_cards() {
        let deck: Vec<MandateCardSpec> = (1..=8u16)
            .map(|id| MandateCardSpec {
                card_id: id,
                kind: MandateObjective::MaintainCleanCredit {
                    min_tier: CreditRatingTier::Bbb,
                },
                difficulty_completion_bps: 5_000,
                bonus_vp: bonus_vp_for_completion_bps(5_000),
            })
            .collect();
        assert!(
            deal_mandate_indices_with_entropy(
                league_core::TableId(42),
                &deck,
                SeatCount::Five,
                [0u8; 32],
            )
            .is_err()
        );

        let Ok(deal) = deal_mandate_indices_with_entropy(
            league_core::TableId(42),
            &deck,
            SeatCount::Six,
            [0xA5u8; 32],
        ) else {
            panic!("non-zero 256-bit entropy deals cleanly");
        };
        assert_eq!(deal.len(), 6);
        let uniq: std::collections::BTreeSet<usize> = deal.iter().copied().collect();
        assert_eq!(uniq.len(), 6);

        // C-2: card_id > u8::MAX (e.g. 300) round-trips losslessly without saturating to 255.
        let wide_core = league_core::GradedMandateCard {
            card_id: 300,
            kind: league_core::MandateKind::MaintainCleanCredit {
                min_tier: league_core::CreditTier::Aaa,
            },
            difficulty_completion_bps: 4_000,
            bonus_equity: 6_000,
        };
        let wide_spec = MandateCardSpec::from(wide_core);
        assert_eq!(wide_spec.card_id, 300);
        assert_eq!(wide_spec.to_core_card(), wide_core);
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
            FeasibilityError::ShapleyGuardExceeded {
                max_relative_deviation: 0.085,
                max_allowed_deviation: SHAPLEY_GUARD_MAX_REL_DEV,
                shapley_seat_evs: vec![10.0, 11.7],
            },
            FeasibilityError::PairShareExceedsCap {
                pair: (0, 1),
                share: 0.52,
                cap: 0.45,
            },
            FeasibilityError::RolloutGateFailed("low completion".to_owned()),
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

    #[test]
    fn seed_pool_pins_every_boundary_and_lowercase_serde_names() {
        let boundary_cases: [(u64, SeedPool); 14] = [
            (0, SeedPool::Dev),
            (9_999, SeedPool::Dev),
            (10_000, SeedPool::Train),
            (999_999, SeedPool::Train),
            (1_000_000, SeedPool::Probe),
            (1_099_999, SeedPool::Probe),
            (1_100_000, SeedPool::Unassigned),
            (1_999_999, SeedPool::Unassigned),
            (2_000_000, SeedPool::HeldOut),
            (2_099_999, SeedPool::HeldOut),
            (2_100_000, SeedPool::Unassigned),
            ((1_u64 << 63) - 1, SeedPool::Unassigned),
            (1_u64 << 63, SeedPool::Sealed),
            (u64::MAX, SeedPool::Sealed),
        ];
        for (idx, (seed, expected)) in boundary_cases.into_iter().enumerate() {
            assert_eq!(
                seed_pool(seed),
                expected,
                "boundary table mismatch at row {idx}"
            );
        }

        let serde_cases = [
            (SeedPool::Dev, "\"dev\""),
            (SeedPool::Train, "\"train\""),
            (SeedPool::Probe, "\"probe\""),
            (SeedPool::HeldOut, "\"held_out\""),
            (SeedPool::Sealed, "\"sealed\""),
            (SeedPool::Unassigned, "\"unassigned\""),
        ];
        for (pool, expected_json) in serde_cases {
            let Ok(encoded) = serde_json::to_string(&pool) else {
                panic!("serialize SeedPool");
            };
            assert_eq!(encoded, expected_json);
            let Ok(decoded) = serde_json::from_str::<SeedPool>(&encoded) else {
                panic!("deserialize SeedPool");
            };
            assert_eq!(decoded, pool);
        }

        let purpose_cases = [
            (TablePurpose::Dev, "\"dev\"", SeedPool::Dev),
            (TablePurpose::Train, "\"train\"", SeedPool::Train),
            (TablePurpose::Probe, "\"probe\"", SeedPool::Probe),
            (TablePurpose::HeldOut, "\"held_out\"", SeedPool::HeldOut),
            (TablePurpose::Sealed, "\"sealed\"", SeedPool::Sealed),
        ];
        for (purpose, expected_json, expected_pool) in purpose_cases {
            let Ok(encoded) = serde_json::to_string(&purpose) else {
                panic!("serialize TablePurpose");
            };
            assert_eq!(encoded, expected_json);
            let Ok(decoded) = serde_json::from_str::<TablePurpose>(&encoded) else {
                panic!("deserialize TablePurpose");
            };
            assert_eq!(decoded, purpose);
            assert_eq!(purpose.expected_pool(), expected_pool);
        }

        assert_eq!(
            TableSetupOptions::default(),
            TableSetupOptions {
                purpose: TablePurpose::Dev,
                training_logged: false,
            }
        );
    }

    #[test]
    fn recorded_layout_seed_deserialization_refuses_sealed_values_in_clear() {
        let Ok(ok_clear) = serde_json::from_str::<RecordedLayoutSeed>(r#"{"clear":42}"#) else {
            panic!("dev clear seed must deserialize");
        };
        assert_eq!(ok_clear.clear_seed(), Some(42));
        assert!(!ok_clear.is_sealed());

        for made_up_sealed in [SEALED_SEED_MIN, SEALED_SEED_MAX, SEALED_SEED_MIN | 42] {
            let payload = format!(r#"{{"clear":{made_up_sealed}}}"#);
            let Err(err) = serde_json::from_str::<RecordedLayoutSeed>(&payload) else {
                panic!("sealed-range value in clear variant must be refused");
            };
            let msg = err.to_string();
            let expected_hash = sealed_seed_sha256(made_up_sealed);
            let dec = made_up_sealed.to_string();
            let hex_lo = format!("{made_up_sealed:x}");
            let hex_hi = format!("{made_up_sealed:X}");
            assert!(msg.contains("SEALED_SEED_IN_CLEAR"));
            assert!(msg.contains("sealed"));
            assert!(msg.contains(&expected_hash));
            assert!(
                !msg.contains(&dec) && !msg.contains(&hex_lo) && !msg.contains(&hex_hi),
                "deserialization refusal leaked raw sealed-range value"
            );
        }

        let Err(bad_digest_err) =
            serde_json::from_str::<RecordedLayoutSeed>(r#"{"sealed_sha256":"not-valid-hex"}"#)
        else {
            panic!("malformed sealed digest must be refused");
        };
        assert!(
            bad_digest_err
                .to_string()
                .contains("INVALID_SEALED_SEED_DIGEST")
        );
    }

    #[test]
    fn reserved_test_seeds_constant_and_predicate_match_documented_exceptions() {
        assert_eq!(
            RESERVED_PROBE_HELD_OUT_TEST_SEEDS,
            [1_000_000, 1_099_999, 2_000_000, 2_099_999]
        );
        for reserved in [
            1_000_000_u64,
            1_099_999,
            2_000_000,
            2_099_999,
            SEALED_SEED_MIN,
            SEALED_SEED_MAX,
            SEALED_SEED_MIN | 42,
        ] {
            assert!(is_reserved_test_seed(reserved));
        }
        for non_reserved in [
            0_u64,
            42,
            9_999,
            10_000,
            999_999,
            1_000_001,
            1_099_998,
            1_500_000,
            2_000_001,
            2_099_998,
            SEALED_SEED_MIN - 1,
            SEALED_SEED_MIN | 1,
            SEALED_SEED_MAX - 1,
        ] {
            assert!(!is_reserved_test_seed(non_reserved));
        }
    }

    #[test]
    fn layout_seed_digest_matches_sealed_seed_sha256_on_reserved_sealed_test_values() {
        assert_eq!(league_core::SEALED_LAYOUT_SEED_MIN, SEALED_SEED_MIN);
        for made_up_sealed in [SEALED_SEED_MIN, SEALED_SEED_MAX, SEALED_SEED_MIN | 42] {
            assert!(is_reserved_test_seed(made_up_sealed));
            let expected_bytes = Sha256::digest(made_up_sealed.to_be_bytes());
            let mut expected_hex = String::with_capacity(64);
            for byte in expected_bytes {
                use std::fmt::Write as _;
                let _ = write!(&mut expected_hex, "{byte:02x}");
            }
            let from_gen = sealed_seed_sha256(made_up_sealed);
            let from_core = league_core::layout_seed_digest(made_up_sealed);
            assert_eq!(from_gen, expected_hex);
            assert_eq!(from_core, from_gen);
        }
    }
}
