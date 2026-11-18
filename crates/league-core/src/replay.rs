//! Deterministic Steward replay from initial `TableSpec`, public or redacted layout seed,
//! Turn-1 mandate commitments, per-turn action/commit/reveal logs, and turn beacons (`EvalVrf`
//! verified under `vrf-r255`; `ExternalDrand` currently **hash-bound, not yet authenticated**
//! until `#29` wires BLS12-381 verification in [`crate::commit::drand_quicknet_signature_status`]).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeStruct};
use sha2::{Digest, Sha256};

use crate::commit::{
    CommitmentHash, DrandBeaconAuthStatus, MandateReveal, OrderReveal, SealedEvalSeed,
    SealedMandateDeal, draw_shocks_for_turn, validate_eval_vrf_pubkey,
    verify_drand_quicknet_beacon, verify_eval_vrf_beacon, verify_sealed_mandate_deal,
};
use crate::contracts::MarketAction;
use crate::engine::{SweepReceipt, TurnEngine, TurnPhase};
use crate::predicates::{
    STATE_PREDICATE_SECTION_VERSION, SeatStatePredicates, SeatSweepOutcome, StatePredicateSection,
    TurnPredicateRecord, compute_seat_state_predicates,
};
use crate::scoring::FinalScoreboard;
use crate::types::{
    EngineError, ScheduledShockSpec, Seat, TableMode, TableSpec, Turn, validate_club_dev_seed,
};

/// Current replay serialization format version written on [`SeasonReplayLog`] and [`ReplayOutcome`].
///
/// Pre-version logs without `"format_version"` still deserialize cleanly for backwards compatibility;
/// payloads declaring any unsupported `"format_version"` other than [`REPLAY_FORMAT_VERSION`]
/// (including `0` or newer versions) are refused by name via [`EngineError::UnsupportedReplayVersion`].
pub const REPLAY_FORMAT_VERSION: u32 = 1;

#[derive(Deserialize)]
struct FormatVersionHeader {
    #[serde(default)]
    format_version: Option<u32>,
}

/// Inspects `format_version` in a serialized JSON replay payload, accepting absent (pre-version) or
/// [`REPLAY_FORMAT_VERSION`] and refusing any other version via
/// [`EngineError::UnsupportedReplayVersion`].
pub fn check_replay_format_version_slice(bytes: &[u8]) -> Result<u32, EngineError> {
    let header: FormatVersionHeader = serde_json::from_slice(bytes).map_err(|err| {
        EngineError::InvalidConfig(format!("replay payload JSON header does not parse: {err}"))
    })?;
    match header.format_version {
        None => Ok(REPLAY_FORMAT_VERSION),
        Some(v) if v == REPLAY_FORMAT_VERSION => Ok(v),
        Some(found) => Err(EngineError::UnsupportedReplayVersion {
            found,
            newest_known: REPLAY_FORMAT_VERSION,
        }),
    }
}

fn validate_format_version<E: de::Error>(version: Option<u32>) -> Result<(), E> {
    match version {
        None => Ok(()),
        Some(v) if v == REPLAY_FORMAT_VERSION => Ok(()),
        Some(found) => Err(E::custom(EngineError::UnsupportedReplayVersion {
            found,
            newest_known: REPLAY_FORMAT_VERSION,
        })),
    }
}

/// Inclusive lower bound (`2^63`) of the `sealed` layout-seed pool (`2^63..=2^64 - 1`),
/// matching `league_gen::SEALED_SEED_MIN`.
pub const SEALED_LAYOUT_SEED_MIN: u64 = 1_u64 << 63;

/// Canonical pool name recorded in a sealed table's [`SeasonReplayLog`] in place of a clear seed.
pub const SEALED_SEED_POOL_NAME: &str = "sealed";

/// Computes the lowercase 64-character ASCII hex digest of `seed`'s 8 big-endian bytes
/// (`SHA-256(seed.to_be_bytes())`).
///
/// Both `league-core` replay redaction and `league-gen` (`sealed_seed_sha256`) delegate to this
/// single function so issue `#58` (switching from an unkeyed 8-byte SHA-256 to a rig-held HMAC key
/// before any sealed game's record leaves the rig) is a one-place change.
#[must_use]
pub fn layout_seed_digest(seed: u64) -> String {
    hex::encode(Sha256::digest(seed.to_be_bytes()))
}

/// Validation failure returned when constructing or deserializing a [`ReplayLayoutSeed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayLayoutSeedError {
    /// A `sealed`-range layout seed (`>= 2^63`) appeared in clear (`SEALED_SEED_IN_CLEAR`).
    /// Carries only [`layout_seed_digest`], never the clear `u64`.
    SealedSeedInClear {
        /// Digest from [`layout_seed_digest`].
        seed_digest: String,
    },
    /// A sealed layout-seed object carried a `pool` other than `"sealed"`
    /// (`INVALID_SEALED_SEED_POOL`).
    InvalidSealedPool {
        /// Rejected `pool` value.
        pool: String,
    },
    /// A sealed layout-seed object had a missing or malformed `digest` (must be 64 lowercase ASCII
    /// hex characters; `INVALID_SEALED_SEED_DIGEST`).
    InvalidSealedDigest,
    /// A non-sealed layout seed (`< 2^63`) was passed to [`SeasonReplayLog::new_sealed`]
    /// (`NON_SEALED_SEED_IN_SEALED`).
    NonSealedSeedInSealed {
        /// Clear non-sealed seed (`< 2^63`).
        seed: u64,
    },
}

impl ReplayLayoutSeedError {
    /// Returns the stable `SCREAMING_SNAKE_CASE` error identifier for this error variant.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SealedSeedInClear { .. } => "SEALED_SEED_IN_CLEAR",
            Self::InvalidSealedPool { .. } => "INVALID_SEALED_SEED_POOL",
            Self::InvalidSealedDigest => "INVALID_SEALED_SEED_DIGEST",
            Self::NonSealedSeedInSealed { .. } => "NON_SEALED_SEED_IN_SEALED",
        }
    }
}

impl fmt::Display for ReplayLayoutSeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SealedSeedInClear { seed_digest } => write!(
                f,
                "SEALED_SEED_IN_CLEAR: replay log cannot hold a pool 'sealed' layout seed in clear (digest={seed_digest})"
            ),
            Self::InvalidSealedPool { pool } => write!(
                f,
                "INVALID_SEALED_SEED_POOL: sealed replay log seed pool must be '{SEALED_SEED_POOL_NAME}', got '{pool}'"
            ),
            Self::InvalidSealedDigest => write!(
                f,
                "INVALID_SEALED_SEED_DIGEST: sealed replay log seed digest must be 64 lowercase hex characters"
            ),
            Self::NonSealedSeedInSealed { seed } => write!(
                f,
                "NON_SEALED_SEED_IN_SEALED: layout seed {seed} is below sealed range (>= 2^63)"
            ),
        }
    }
}

impl std::error::Error for ReplayLayoutSeedError {}

/// Redacted [`SeasonReplayLog`] seed payload written for a `sealed`-pool table (`>= 2^63`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedReplaySeed {
    pool: String,
    digest: String,
    /// Whether the deal-seed and Eval-VRF independence checks ran on the rig at write time before
    /// the layout seed was redacted.
    ///
    /// **This is a claim by the log writer, not something [`replay_season`] verifies:** a replayer
    /// of a sealed log cannot recompute those independence checks, because it does not have the
    /// clear layout seed. Making this verifiable off-rig belongs to issue `#58`, where a keyed
    /// commitment to the seed is introduced.
    independence_checked_at_write_unverified: bool,
}

impl SealedReplaySeed {
    /// Returns the seed pool name recorded in the log (`"sealed"`).
    #[must_use]
    pub fn pool(&self) -> &str {
        &self.pool
    }

    /// Returns the lowercase 64-hex layout-seed digest from [`layout_seed_digest`].
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Returns the log writer's claim that layout-seed independence checks passed before redaction.
    ///
    /// **This is a claim by the log writer, not something [`replay_season`] verifies:** a replayer
    /// of a sealed log cannot recompute those checks, because it does not have the clear seed.
    /// Checking this independently requires a keyed commitment to the seed (issue `#58`).
    #[must_use]
    pub const fn independence_checked_at_write_unverified(&self) -> bool {
        self.independence_checked_at_write_unverified
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplayLayoutSeedRepr {
    Clear(u64),
    Sealed(SealedReplaySeed),
}

/// Validated layout-seed field carried in [`SeasonReplayLog::seed`] (`#56`).
///
/// For non-`sealed` tables (`< 2^63`), stores and serializes as the exact bare JSON `u64` number
/// for byte-identical compatibility with legacy logs and fixtures. For `sealed`-pool tables
/// (`>= 2^63`), stores only the pool name (`"sealed"`), [`layout_seed_digest`], and the log
/// writer's unverified write-time independence claim — never the clear `u64`.
///
/// Private inner representation makes it impossible to construct a `sealed`-range value in the
/// clear variant in code:
///
/// ```compile_fail
/// use league_core::ReplayLayoutSeed;
/// let _ = ReplayLayoutSeed(1_u64 << 63);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayLayoutSeed(ReplayLayoutSeedRepr);

impl ReplayLayoutSeed {
    /// Constructs a clear [`ReplayLayoutSeed`] for any non-sealed layout seed (`< 2^63`), refusing
    /// `sealed`-range values (`>= 2^63`) with [`ReplayLayoutSeedError::SealedSeedInClear`].
    pub fn try_clear(seed: u64) -> Result<Self, ReplayLayoutSeedError> {
        if seed >= SEALED_LAYOUT_SEED_MIN {
            return Err(ReplayLayoutSeedError::SealedSeedInClear {
                seed_digest: layout_seed_digest(seed),
            });
        }
        Ok(Self(ReplayLayoutSeedRepr::Clear(seed)))
    }

    pub(crate) fn sealed_checked(seed: u64) -> Result<Self, ReplayLayoutSeedError> {
        if seed < SEALED_LAYOUT_SEED_MIN {
            return Err(ReplayLayoutSeedError::NonSealedSeedInSealed { seed });
        }
        Ok(Self(ReplayLayoutSeedRepr::Sealed(SealedReplaySeed {
            pool: SEALED_SEED_POOL_NAME.to_owned(),
            digest: layout_seed_digest(seed),
            independence_checked_at_write_unverified: true,
        })))
    }

    /// Returns `Some(seed)` for a non-sealed clear seed, or `None` when redacted.
    #[must_use]
    pub const fn clear_seed(&self) -> Option<u64> {
        match &self.0 {
            ReplayLayoutSeedRepr::Clear(seed) => Some(*seed),
            ReplayLayoutSeedRepr::Sealed(_) => None,
        }
    }

    /// Returns the redacted [`SealedReplaySeed`] payload if this log is for a sealed table.
    #[must_use]
    pub const fn sealed(&self) -> Option<&SealedReplaySeed> {
        match &self.0 {
            ReplayLayoutSeedRepr::Clear(_) => None,
            ReplayLayoutSeedRepr::Sealed(sealed) => Some(sealed),
        }
    }

    /// Returns `Some("sealed")` when redacted, or `None` for a non-sealed clear seed.
    #[must_use]
    pub fn pool(&self) -> Option<&str> {
        self.sealed().map(SealedReplaySeed::pool)
    }

    /// Returns `Some(digest)` when redacted, or `None` for a non-sealed clear seed.
    #[must_use]
    pub fn sealed_digest(&self) -> Option<&str> {
        self.sealed().map(SealedReplaySeed::digest)
    }

    /// Returns `true` when this log carries a redacted `sealed`-pool digest instead of a clear `u64`.
    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        matches!(&self.0, ReplayLayoutSeedRepr::Sealed(_))
    }

    /// For a sealed log, returns the log writer's claim that write-time independence checks ran
    /// before redaction; returns `None` for a clear log (where replay verifies independence
    /// directly).
    ///
    /// **This is a claim by the log writer, not something [`replay_season`] verifies:** a replayer
    /// of a sealed log cannot recompute those checks, because it does not have the clear seed.
    /// Keyed commitments under issue `#58` are where this can become replay-verifiable.
    #[must_use]
    pub const fn independence_checked_at_write_unverified(&self) -> Option<bool> {
        match &self.0 {
            ReplayLayoutSeedRepr::Clear(_) => None,
            ReplayLayoutSeedRepr::Sealed(sealed) => {
                Some(sealed.independence_checked_at_write_unverified)
            }
        }
    }
}

impl TryFrom<u64> for ReplayLayoutSeed {
    type Error = ReplayLayoutSeedError;

    fn try_from(seed: u64) -> Result<Self, Self::Error> {
        Self::try_clear(seed)
    }
}

impl Serialize for ReplayLayoutSeed {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            ReplayLayoutSeedRepr::Clear(seed) => serializer.serialize_u64(*seed),
            ReplayLayoutSeedRepr::Sealed(sealed) => {
                let mut state = serializer.serialize_struct("SealedReplaySeed", 3)?;
                state.serialize_field("pool", &sealed.pool)?;
                state.serialize_field("digest", &sealed.digest)?;
                state.serialize_field(
                    "independence_checked_at_write_unverified",
                    &sealed.independence_checked_at_write_unverified,
                )?;
                state.end()
            }
        }
    }
}

struct DigestField(String);

impl<'de> Deserialize<'de> for DigestField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DigestVisitor;
        impl de::Visitor<'_> for DigestVisitor {
            type Value = DigestField;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a 64-character lowercase hex digest string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(DigestField(v.to_owned()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(DigestField(v))
            }
        }
        deserializer
            .deserialize_str(DigestVisitor)
            .map_err(|_| de::Error::custom(ReplayLayoutSeedError::InvalidSealedDigest))
    }
}

impl<'de> Deserialize<'de> for ReplayLayoutSeed {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        enum SealedSeedField {
            Pool,
            Digest,
            IndependenceCheckedAtWriteUnverified,
        }

        impl<'de> Deserialize<'de> for SealedSeedField {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct FieldVisitor;
                impl de::Visitor<'_> for FieldVisitor {
                    type Value = SealedSeedField;

                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str(
                            "`pool`, `digest`, or `independence_checked_at_write_unverified`",
                        )
                    }

                    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                        match value {
                            "pool" => Ok(SealedSeedField::Pool),
                            "digest" => Ok(SealedSeedField::Digest),
                            "independence_checked_at_write_unverified" => {
                                Ok(SealedSeedField::IndependenceCheckedAtWriteUnverified)
                            }
                            other => Err(de::Error::unknown_field(
                                other,
                                &["pool", "digest", "independence_checked_at_write_unverified"],
                            )),
                        }
                    }
                }
                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct ReplayLayoutSeedVisitor;

        impl<'de> de::Visitor<'de> for ReplayLayoutSeedVisitor {
            type Value = ReplayLayoutSeed;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a non-sealed u64 layout seed (< 2^63) or a sealed layout-seed object {pool, digest, independence_checked_at_write_unverified}",
                )
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                ReplayLayoutSeed::try_clear(value).map_err(de::Error::custom)
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                if value < 0 {
                    return Err(de::Error::custom("layout seed must be a non-negative u64"));
                }
                self.visit_u64(value as u64)
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut pool: Option<String> = None;
                let mut digest: Option<DigestField> = None;
                let mut checked_claim: Option<bool> = None;

                while let Some(key) = map.next_key::<SealedSeedField>()? {
                    match key {
                        SealedSeedField::Pool => {
                            if pool.is_some() {
                                return Err(de::Error::duplicate_field("pool"));
                            }
                            pool = Some(map.next_value()?);
                        }
                        SealedSeedField::Digest => {
                            if digest.is_some() {
                                return Err(de::Error::duplicate_field("digest"));
                            }
                            digest = Some(map.next_value()?);
                        }
                        SealedSeedField::IndependenceCheckedAtWriteUnverified => {
                            if checked_claim.is_some() {
                                return Err(de::Error::duplicate_field(
                                    "independence_checked_at_write_unverified",
                                ));
                            }
                            checked_claim = Some(map.next_value()?);
                        }
                    }
                }

                let pool = pool.ok_or_else(|| de::Error::missing_field("pool"))?;
                if pool != SEALED_SEED_POOL_NAME {
                    return Err(de::Error::custom(
                        ReplayLayoutSeedError::InvalidSealedPool { pool },
                    ));
                }

                let DigestField(digest) = digest
                    .ok_or_else(|| de::Error::custom(ReplayLayoutSeedError::InvalidSealedDigest))?;
                let valid_hex = digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
                if !valid_hex {
                    return Err(de::Error::custom(
                        ReplayLayoutSeedError::InvalidSealedDigest,
                    ));
                }

                let independence_checked_at_write_unverified = checked_claim.ok_or_else(|| {
                    de::Error::missing_field("independence_checked_at_write_unverified")
                })?;

                Ok(ReplayLayoutSeed(ReplayLayoutSeedRepr::Sealed(
                    SealedReplaySeed {
                        pool,
                        digest,
                        independence_checked_at_write_unverified,
                    },
                )))
            }
        }

        deserializer.deserialize_any(ReplayLayoutSeedVisitor)
    }
}

/// Verification mode for per-turn beacons during [`replay_season`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayBeaconMode {
    /// Eval mode with proofs under the RFC 9381 ECVRF construction with C2SP's `ristretto255`
    /// ciphersuite (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not an IANA-registered suite in RFC 9381)
    /// verified against `eval_vrf_pubkey` (`#31` comment, `2026-11-15` ruling).
    #[default]
    EvalVrf,
    /// Game mode with League of Entropy drand `quicknet` beacons: requires `spec.drand_schedule`,
    /// scheduled `drand_round`, and `turn_beacon == SHA-256(drand_signature)`.
    ///
    /// **Deferred authentication note (`#29`):** Drand beacons in Game-mode replay are currently
    /// **hash-bound, not yet authenticated** — [`crate::commit::verify_drand_quicknet_beacon`]
    /// calls [`crate::commit::drand_quicknet_signature_status`], which returns
    /// [`crate::commit::DrandBeaconAuthStatus::HashBoundUnauthenticated`] until `#29` adds
    /// BLS12-381 signature verification against the pinned `quicknet` chain public key.
    ExternalDrand,
    /// Legacy seed-derived beacon mode — refused by [`replay_season`] in both `Game` and `Eval`
    /// modes because randomness must never derive from the public layout seed (`#31`).
    SeedDerived,
}

/// Recorded ordered inputs for a single simultaneous turn (`TalkAndMarket -> Commit -> Reveal -> Sweep`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedTurn {
    /// Expected turn number (`1..=32`).
    pub turn: u8,
    /// Ordered Phase 1 market, check, escrow, and intent actions.
    pub market_actions: Vec<MarketAction>,
    /// Optional zero-based index into `market_actions` (`0..=market_actions.len()`) at which
    /// Phase-1 cancel lock engaged (`TurnEngine::begin_cancel_lock`) on this turn (`C4` amended
    /// `2026-11-17`). Absent / `None` leaves cancel lock disengaged for the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_lock_at: Option<u32>,
    /// Phase 2 cryptographic order commitments keyed by seat index.
    pub commitments: BTreeMap<u8, CommitmentHash>,
    /// Phase 2 order reveals keyed by seat index.
    pub reveals: BTreeMap<u8, OrderReveal>,
    /// Publicly logged randomness beacon (`SHA-256(drand_signature)` in Game Mode or VRF `beta[..32]` in Eval Mode).
    pub turn_beacon: [u8; 32],
    /// Optional drand round number required when `beacon_mode == ReplayBeaconMode::ExternalDrand`.
    #[serde(default)]
    pub drand_round: Option<u64>,
    /// Optional 48-byte drand `quicknet` BLS12-381 signature required under `ExternalDrand` (`M2`).
    #[serde(default)]
    pub drand_signature: Option<Vec<u8>>,
    /// Optional 80-byte VRF proof (RFC 9381 ECVRF construction with C2SP's `ristretto255`
    /// ciphersuite, `vrf-r255 0.1.0`) required under `EvalVrf` (`#31`).
    #[serde(default)]
    pub eval_vrf_proof: Option<Vec<u8>>,
    /// 3-stage Horizon Shocks drawn from this turn's HKDF `"shocks"` stream (`#31`).
    #[serde(default)]
    pub drawn_shocks: Vec<ScheduledShockSpec>,
}

/// Complete portable audit log replaying an entire season without any ledger I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonReplayLog {
    /// Immutable table specification (`4..=6` seats).
    pub spec: TableSpec,
    /// Required execution mode matching `spec.mode` (`Game`, `Eval`, or `Club`; prevents mode switches).
    pub mode: TableMode,
    /// Layout seed field (`ReplayLayoutSeed`): clear `u64` for non-`sealed` pools (`< 2^63`), or
    /// redacted `{"pool":"sealed","digest":...,"independence_checked_at_write_unverified":...}`
    /// carrying no clear value for `sealed`-pool tables (`>= 2^63`; never used to derive randomness
    /// or deals).
    pub seed: ReplayLayoutSeed,
    /// Verification rule applied to each logged turn beacon.
    pub beacon_mode: ReplayBeaconMode,
    /// Optional 32-byte `ristretto255` VRF public key (RFC 9381 ECVRF construction with C2SP's
    /// `c2sp.org/vrf-r255` ciphersuite, `vrf-r255 0.1.0`) required in `Eval` and `Club` modes.
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Sealed OS-entropy Turn-1 mandate deal and per-seat salts bound to `spec.dealt_mandate_commits` (`M3`).
    pub sealed_mandate_deal: SealedMandateDeal,
    /// Optional Turn-1 Private Charter Mandate commitments per seat (`0..seat_count`).
    pub mandate_commits: Vec<Option<CommitmentHash>>,
    /// Chronological sequence of turns executed from Turn `1` through closing turn `T in 24..=32`.
    pub turns: Vec<LoggedTurn>,
    /// Terminal Private Charter Mandate reveals per seat (`0..seat_count`).
    pub mandate_reveals: Vec<Option<MandateReveal>>,
}

#[derive(Serialize)]
struct SeasonReplayLogSer<'a> {
    format_version: u32,
    spec: &'a TableSpec,
    mode: TableMode,
    seed: &'a ReplayLayoutSeed,
    beacon_mode: ReplayBeaconMode,
    #[serde(with = "crate::types::opt_hex32")]
    eval_vrf_pubkey: &'a Option<[u8; 32]>,
    sealed_mandate_deal: &'a SealedMandateDeal,
    mandate_commits: &'a [Option<CommitmentHash>],
    turns: &'a [LoggedTurn],
    mandate_reveals: &'a [Option<MandateReveal>],
}

#[derive(Deserialize)]
struct SeasonReplayLogDe {
    #[serde(default)]
    format_version: Option<u32>,
    spec: TableSpec,
    mode: TableMode,
    seed: ReplayLayoutSeed,
    beacon_mode: ReplayBeaconMode,
    #[serde(default, with = "crate::types::opt_hex32")]
    eval_vrf_pubkey: Option<[u8; 32]>,
    sealed_mandate_deal: SealedMandateDeal,
    mandate_commits: Vec<Option<CommitmentHash>>,
    turns: Vec<LoggedTurn>,
    mandate_reveals: Vec<Option<MandateReveal>>,
}

impl Serialize for SeasonReplayLog {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SeasonReplayLogSer {
            format_version: REPLAY_FORMAT_VERSION,
            spec: &self.spec,
            mode: self.mode,
            seed: &self.seed,
            beacon_mode: self.beacon_mode,
            eval_vrf_pubkey: &self.eval_vrf_pubkey,
            sealed_mandate_deal: &self.sealed_mandate_deal,
            mandate_commits: &self.mandate_commits,
            turns: &self.turns,
            mandate_reveals: &self.mandate_reveals,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SeasonReplayLog {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let de = SeasonReplayLogDe::deserialize(deserializer)?;
        validate_format_version(de.format_version)?;
        Ok(Self {
            spec: de.spec,
            mode: de.mode,
            seed: de.seed,
            beacon_mode: de.beacon_mode,
            eval_vrf_pubkey: de.eval_vrf_pubkey,
            sealed_mandate_deal: de.sealed_mandate_deal,
            mandate_commits: de.mandate_commits,
            turns: de.turns,
            mandate_reveals: de.mandate_reveals,
        })
    }
}

pub(crate) fn layout_seed_beacon_candidates(seed: u64) -> [[u8; 32]; 2] {
    let mut padded_beacon_seed = [0u8; 32];
    padded_beacon_seed[..8].copy_from_slice(&seed.to_le_bytes());
    let hashed_beacon_seed: [u8; 32] = Sha256::digest(seed.to_le_bytes()).into();
    [padded_beacon_seed, hashed_beacon_seed]
}

pub(crate) fn check_deal_seed_independence(
    seed: u64,
    sealed_mandate_deal: &SealedMandateDeal,
    sealed_context: bool,
) -> Result<[[u8; 32]; 2], EngineError> {
    let candidates = layout_seed_beacon_candidates(seed);
    if sealed_mandate_deal.deal_seed == candidates[0]
        || sealed_mandate_deal.deal_seed == candidates[1]
    {
        let msg = if sealed_context {
            "SEALED_DEAL_SEED_NOT_INDEPENDENT: sealed_mandate_deal deal_seed must be independent of layout seed"
        } else {
            "sealed_mandate_deal deal_seed must be independent of public layout seed"
        };
        return Err(EngineError::InvalidConfig(msg.to_owned()));
    }
    Ok(candidates)
}

pub(crate) fn check_eval_vrf_independence(
    candidates: [[u8; 32]; 2],
    spec_pk: [u8; 32],
    sealed_context: bool,
) -> Result<(), EngineError> {
    for candidate in candidates {
        if let Ok(derived_seed) = SealedEvalSeed::try_from_bytes(candidate)
            && derived_seed.public_key_bytes().ok() == Some(spec_pk)
        {
            let msg = if sealed_context {
                "SEALED_EVAL_VRF_NOT_INDEPENDENT: Eval VRF key must be independent of layout seed"
            } else {
                "Eval VRF key must be independent of public layout seed"
            };
            return Err(EngineError::InvalidConfig(msg.to_owned()));
        }
    }
    Ok(())
}

impl SeasonReplayLog {
    /// Constructs a redacted [`SeasonReplayLog`] on the rig for a `sealed`-pool layout seed
    /// (`>= 2^63`).
    ///
    /// Runs the layout-seed independence checks (`deal_seed` and, in `TableMode::Eval`,
    /// `eval_vrf_pubkey`) against the clear `seed` *before* redacting it via
    /// [`layout_seed_digest`], refuses any non-independent table with [`EngineError::InvalidConfig`],
    /// and records `independence_checked_at_write_unverified = true`.
    ///
    /// Note: `independence_checked_at_write_unverified` in the emitted log is a **claim by the log
    /// writer, not something [`replay_season`] verifies**: a replayer of a sealed log cannot
    /// recompute the check without the seed (issue `#58` keyed commitments address off-rig
    /// verifiability).
    pub fn new_sealed(
        spec: TableSpec,
        seed: u64,
        sealed_mandate_deal: SealedMandateDeal,
        mandate_commits: Vec<Option<CommitmentHash>>,
        turns: Vec<LoggedTurn>,
        mandate_reveals: Vec<Option<MandateReveal>>,
    ) -> Result<Self, EngineError> {
        let redacted_seed = ReplayLayoutSeed::sealed_checked(seed)
            .map_err(|err| EngineError::InvalidConfig(err.to_string()))?;
        Self::build_checked(
            spec,
            seed,
            redacted_seed,
            true,
            sealed_mandate_deal,
            mandate_commits,
            turns,
            mandate_reveals,
        )
    }

    /// Constructs a [`SeasonReplayLog`] on the rig from any layout `seed`: uses
    /// [`SeasonReplayLog::new_sealed`] (redacting the seed) when `seed >= 2^63`, or stores the
    /// validated clear `u64` via [`ReplayLayoutSeed::try_clear`] after checking independence when
    /// `seed < 2^63`.
    pub fn from_rig_seed(
        spec: TableSpec,
        seed: u64,
        sealed_mandate_deal: SealedMandateDeal,
        mandate_commits: Vec<Option<CommitmentHash>>,
        turns: Vec<LoggedTurn>,
        mandate_reveals: Vec<Option<MandateReveal>>,
    ) -> Result<Self, EngineError> {
        if seed >= SEALED_LAYOUT_SEED_MIN {
            return Self::new_sealed(
                spec,
                seed,
                sealed_mandate_deal,
                mandate_commits,
                turns,
                mandate_reveals,
            );
        }
        let clear_seed = ReplayLayoutSeed::try_clear(seed)
            .map_err(|err| EngineError::InvalidConfig(err.to_string()))?;
        Self::build_checked(
            spec,
            seed,
            clear_seed,
            false,
            sealed_mandate_deal,
            mandate_commits,
            turns,
            mandate_reveals,
        )
    }

    /// Parses a serialized [`SeasonReplayLog`] JSON slice after verifying its `format_version`,
    /// accepting pre-version logs (absent `format_version`) and [`REPLAY_FORMAT_VERSION`] while
    /// refusing any unsupported version (`0`, newer, or unknown) via
    /// [`EngineError::UnsupportedReplayVersion`].
    ///
    /// Note: this generic parser accepts `Game`, `Eval`, and `Club` logs for Steward inspection;
    /// automated Evaluation dataset pipelines must call [`Self::from_eval_json_slice`] so Club logs
    /// are rejected (`C5`).
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        check_replay_format_version_slice(bytes)?;
        serde_json::from_slice(bytes).map_err(|err| {
            EngineError::InvalidConfig(format!("season replay log does not parse: {err}"))
        })
    }

    /// Parses a [`SeasonReplayLog`] strictly for Evaluation Mode (`TableMode::Eval`), refusing Club
    /// and Game logs so offline Club seasons can never enter Evaluation datasets (`Q2`, `C5`).
    pub fn from_eval_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        let log = Self::from_json_slice(bytes)?;
        if log.mode != TableMode::Eval || log.spec.mode != TableMode::Eval {
            return Err(EngineError::InvalidConfig(format!(
                "Eval replay log loader refuses {:?} log; TableMode::Eval is required",
                log.mode
            )));
        }
        Ok(log)
    }

    /// Parses a [`SeasonReplayLog`] strictly for Club Mode (`TableMode::Club`), refusing Eval and
    /// Game logs (`Q2`, `C5`).
    pub fn from_club_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        let log = Self::from_json_slice(bytes)?;
        if log.mode != TableMode::Club || log.spec.mode != TableMode::Club {
            return Err(EngineError::InvalidConfig(format!(
                "Club replay log loader refuses {:?} log; TableMode::Club is required",
                log.mode
            )));
        }
        Ok(log)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_checked(
        spec: TableSpec,
        raw_seed: u64,
        seed: ReplayLayoutSeed,
        sealed_context: bool,
        sealed_mandate_deal: SealedMandateDeal,
        mandate_commits: Vec<Option<CommitmentHash>>,
        turns: Vec<LoggedTurn>,
        mandate_reveals: Vec<Option<MandateReveal>>,
    ) -> Result<Self, EngineError> {
        let candidates =
            check_deal_seed_independence(raw_seed, &sealed_mandate_deal, sealed_context)?;
        let mode = spec.mode;
        let (beacon_mode, eval_vrf_pubkey) = match mode {
            TableMode::Game => (ReplayBeaconMode::ExternalDrand, None),
            TableMode::Eval => {
                let Some(spec_pk) = spec.eval_vrf_pubkey else {
                    return Err(EngineError::InvalidConfig(
                        "EvalVrf replay requires TableSpec::eval_vrf_pubkey".to_owned(),
                    ));
                };
                validate_eval_vrf_pubkey(spec_pk)?;
                check_eval_vrf_independence(candidates, spec_pk, sealed_context)?;
                (ReplayBeaconMode::EvalVrf, Some(spec_pk))
            }
            TableMode::Club => {
                if sealed_context || seed.is_sealed() {
                    return Err(EngineError::InvalidConfig(
                        "SEED_POOL_MISMATCH: TableMode::Club forbids sealed-range layout seeds"
                            .to_owned(),
                    ));
                }
                validate_club_dev_seed(raw_seed)?;
                if spec.drand_schedule.is_some() {
                    return Err(EngineError::InvalidConfig(
                        "Club mode replay log must not carry drand_schedule".to_owned(),
                    ));
                }
                if !spec.shocks.is_empty() {
                    return Err(EngineError::InvalidConfig(
                        "Club mode replay refuses pre-seeded shocks".to_owned(),
                    ));
                }
                let Some(spec_pk) = spec.eval_vrf_pubkey else {
                    return Err(EngineError::InvalidConfig(
                        "EvalVrf replay requires TableSpec::eval_vrf_pubkey".to_owned(),
                    ));
                };
                validate_eval_vrf_pubkey(spec_pk)?;
                check_eval_vrf_independence(candidates, spec_pk, false)?;
                (ReplayBeaconMode::EvalVrf, Some(spec_pk))
            }
        };
        Ok(Self {
            spec,
            mode,
            seed,
            beacon_mode,
            eval_vrf_pubkey,
            sealed_mandate_deal,
            mandate_commits,
            turns,
            mandate_reveals,
        })
    }
}

/// One rejected Phase-1 `MarketAction` recorded during replay (`C4` amended `2026-11-17`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedMarketAction {
    /// Zero-based index into [`LoggedTurn::market_actions`].
    pub action_index: u32,
    /// Stable SCREAMING_SNAKE [`EngineError::code`] returned by the engine.
    pub error_code: String,
}

/// Per-turn list of rejected Phase-1 market actions recorded during [`replay_season`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRejectedActions {
    /// Turn (`1..=32`) on which the rejections occurred.
    pub turn: u8,
    /// Rejected actions in submission order.
    pub rejections: Vec<RejectedMarketAction>,
}

/// Complete verified artifacts emitted by replaying a [`SeasonReplayLog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    /// Per-turn deterministic 8-step Sweep receipts in chronological order.
    pub receipts: Vec<SweepReceipt>,
    /// Canonical SHA-256 post-Sweep state digests (`len == receipts.len()`).
    pub state_digests: Vec<[u8; 32]>,
    /// Terminal Cardinal Net Equity scoreboard (`Completed`).
    pub scoreboard: FinalScoreboard,
    /// Per-turn rejected Phase-1 market action indices and error codes (omitted when empty so clean
    /// logs serialize byte-identically).
    pub rejected_market_actions: Vec<TurnRejectedActions>,
    /// `#29` drand `quicknet` authentication status for `Game`-mode (`ExternalDrand`) seasons,
    /// omitted (`None`) for `Eval`- and `Club`-mode seasons.
    pub drand_beacon_auth_status: Option<DrandBeaconAuthStatus>,
}

#[derive(Serialize)]
struct ReplayOutcomeSer<'a> {
    format_version: u32,
    receipts: &'a [SweepReceipt],
    state_digests: &'a [[u8; 32]],
    scoreboard: &'a FinalScoreboard,
    #[serde(skip_serializing_if = "<[TurnRejectedActions]>::is_empty")]
    rejected_market_actions: &'a [TurnRejectedActions],
    #[serde(skip_serializing_if = "Option::is_none")]
    drand_beacon_auth_status: &'a Option<DrandBeaconAuthStatus>,
}

#[derive(Deserialize)]
struct ReplayOutcomeDe {
    #[serde(default)]
    format_version: Option<u32>,
    receipts: Vec<SweepReceipt>,
    state_digests: Vec<[u8; 32]>,
    scoreboard: FinalScoreboard,
    #[serde(default)]
    rejected_market_actions: Vec<TurnRejectedActions>,
    #[serde(default)]
    drand_beacon_auth_status: Option<DrandBeaconAuthStatus>,
}

impl Serialize for ReplayOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ReplayOutcomeSer {
            format_version: REPLAY_FORMAT_VERSION,
            receipts: &self.receipts,
            state_digests: &self.state_digests,
            scoreboard: &self.scoreboard,
            rejected_market_actions: &self.rejected_market_actions,
            drand_beacon_auth_status: &self.drand_beacon_auth_status,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReplayOutcome {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let de = ReplayOutcomeDe::deserialize(deserializer)?;
        validate_format_version(de.format_version)?;
        Ok(Self {
            receipts: de.receipts,
            state_digests: de.state_digests,
            scoreboard: de.scoreboard,
            rejected_market_actions: de.rejected_market_actions,
            drand_beacon_auth_status: de.drand_beacon_auth_status,
        })
    }
}

impl ReplayOutcome {
    /// Parses a serialized [`ReplayOutcome`] JSON slice after verifying its `format_version`,
    /// accepting pre-version outcomes (absent `format_version`) and [`REPLAY_FORMAT_VERSION`] while
    /// refusing any unsupported version (`0`, newer, or unknown) via
    /// [`EngineError::UnsupportedReplayVersion`].
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        check_replay_format_version_slice(bytes)?;
        serde_json::from_slice(bytes).map_err(|err| {
            EngineError::InvalidConfig(format!("replay outcome does not parse: {err}"))
        })
    }
}

/// A replay record: the [`SeasonReplayLog`] plus the optional, additive state-predicate section.
///
/// The log's fields serialize at the top level exactly as a bare [`SeasonReplayLog`] does, and the
/// section is one extra key written only when present, so a record without it is byte-identical to
/// the bare log and a reader of the bare log ignores the section.
///
/// Read a record back with [`ReplayRecord::from_json_slice`] (serde's `flatten` cannot deserialize
/// the log's integer-keyed maps, so the record is read as the bare log plus the one extra key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplayRecord {
    /// The replay log, unchanged.
    #[serde(flatten)]
    pub log: SeasonReplayLog,
    /// State predicates derived from replaying `log` ([`attach_state_predicates`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_predicates: Option<StatePredicateSection>,
}

/// The one key a [`ReplayRecord`] adds to a bare log, read on its own.
#[derive(Deserialize)]
struct SectionOnly {
    #[serde(default)]
    state_predicates: Option<StatePredicateSection>,
}

impl ReplayRecord {
    /// Reads a record written as JSON: checks `format_version` first, then reads the bare log
    /// (unknown keys ignored) plus the optional section. A bare log reads as a record without a
    /// section.
    ///
    /// Note: this generic loader accepts `Game`, `Eval`, and `Club` records for Steward inspection;
    /// automated Evaluation dataset pipelines must call [`Self::from_eval_json_slice`] so Club
    /// records are refused (`C5`).
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        check_replay_format_version_slice(bytes)?;
        let log: SeasonReplayLog = serde_json::from_slice(bytes).map_err(|err| {
            EngineError::InvalidConfig(format!("replay record log does not parse: {err}"))
        })?;
        let extra: SectionOnly = serde_json::from_slice(bytes).map_err(|err| {
            EngineError::InvalidConfig(format!("replay record section does not parse: {err}"))
        })?;
        if let Some(section) = &extra.state_predicates
            && section.version != STATE_PREDICATE_SECTION_VERSION
        {
            return Err(EngineError::InvalidConfig(format!(
                "replay record section version {} is not the version this code reads ({})",
                section.version, STATE_PREDICATE_SECTION_VERSION
            )));
        }
        if let Some(section) = &extra.state_predicates
            && section.table_id != log.spec.table_id
        {
            return Err(EngineError::InvalidConfig(format!(
                "replay record section table_id {} does not match the log's table_id {}",
                section.table_id.0, log.spec.table_id.0
            )));
        }
        Ok(Self {
            log,
            state_predicates: extra.state_predicates,
        })
    }

    /// Parses a [`ReplayRecord`] strictly for Evaluation Mode (`TableMode::Eval`), refusing Club
    /// and Game records so Club games can never enter automated Evaluation datasets (`Q2`).
    pub fn from_eval_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        let record = Self::from_json_slice(bytes)?;
        if record.log.mode != TableMode::Eval || record.log.spec.mode != TableMode::Eval {
            return Err(EngineError::InvalidConfig(format!(
                "Eval replay record loader refuses {:?} log; TableMode::Eval is required",
                record.log.mode
            )));
        }
        Ok(record)
    }

    /// Parses a [`ReplayRecord`] strictly for Club Mode (`TableMode::Club`), refusing Eval and
    /// Game records (`Q2`).
    pub fn from_club_json_slice(bytes: &[u8]) -> Result<Self, EngineError> {
        let record = Self::from_json_slice(bytes)?;
        if record.log.mode != TableMode::Club || record.log.spec.mode != TableMode::Club {
            return Err(EngineError::InvalidConfig(format!(
                "Club replay record loader refuses {:?} log; TableMode::Club is required",
                record.log.mode
            )));
        }
        Ok(record)
    }
}

/// Replays an entire season deterministically from `log`, verifying every phase transition,
/// cryptographic commitment, 8-step Sweep, closing bell roll, and final scoreboard.
///
/// Note: this generic entry point accepts `Game`, `Eval`, and `Club` logs for Steward inspection;
/// automated Evaluation pipelines must use [`replay_eval_season`] so Club logs are refused (`C5`).
pub fn replay_season(log: &SeasonReplayLog) -> Result<ReplayOutcome, EngineError> {
    replay_season_inner(log, None)
}

/// Replays strictly a [`TableMode::Eval`] season log, refusing [`TableMode::Club`] and
/// [`TableMode::Game`] logs so offline Club seasons cannot enter Eval pipelines (`Q2`).
pub fn replay_eval_season(log: &SeasonReplayLog) -> Result<ReplayOutcome, EngineError> {
    if log.mode != TableMode::Eval || log.spec.mode != TableMode::Eval {
        return Err(EngineError::InvalidConfig(format!(
            "Eval replay loader refuses {:?} log; TableMode::Eval is required",
            log.mode
        )));
    }
    replay_season(log)
}

/// Replays strictly a [`TableMode::Club`] season log, refusing [`TableMode::Eval`] and
/// [`TableMode::Game`] logs (`Q2`).
pub fn replay_club_season(log: &SeasonReplayLog) -> Result<ReplayOutcome, EngineError> {
    if log.mode != TableMode::Club || log.spec.mode != TableMode::Club {
        return Err(EngineError::InvalidConfig(format!(
            "Club replay loader refuses {:?} log; TableMode::Club is required",
            log.mode
        )));
    }
    replay_season(log)
}

/// [`replay_season`], also deriving the state-predicate section: every seat's predicates at the
/// start of every turn, each Sweep's per-seat outcome, and every seat's predicates at the close.
///
/// Deriving the section reads the engine state and changes nothing: the returned
/// [`ReplayOutcome`] equals [`replay_season`]'s for the same log. Like [`replay_season`], this
/// generic audit entry point accepts `Game`, `Eval`, and `Club` logs; automated Eval loaders must
/// use [`replay_eval_season`] (`C5`).
pub fn replay_season_with_state_predicates(
    log: &SeasonReplayLog,
) -> Result<(ReplayOutcome, StatePredicateSection), EngineError> {
    let mut section = StatePredicateSection {
        version: STATE_PREDICATE_SECTION_VERSION,
        table_id: log.spec.table_id,
        turns: Vec::with_capacity(log.turns.len()),
        at_close: Vec::new(),
    };
    let outcome = replay_season_inner(log, Some(&mut section))?;
    Ok((outcome, section))
}

/// Replays `log` and returns it as a [`ReplayRecord`] carrying the state-predicate section.
///
/// Accepts `Game`, `Eval`, and `Club` logs for Steward audit; automated Eval loaders must use
/// [`ReplayRecord::from_eval_json_slice`] / [`replay_eval_season`] (`C5`).
pub fn attach_state_predicates(log: &SeasonReplayLog) -> Result<ReplayRecord, EngineError> {
    let (_, section) = replay_season_with_state_predicates(log)?;
    Ok(ReplayRecord {
        log: log.clone(),
        state_predicates: Some(section),
    })
}

fn all_seat_predicates(engine: &TurnEngine) -> Result<Vec<SeatStatePredicates>, EngineError> {
    (0..engine.state().spec.seat_count)
        .map(|s| compute_seat_state_predicates(engine.state(), Seat(s)))
        .collect()
}

fn replay_season_inner(
    log: &SeasonReplayLog,
    mut section: Option<&mut StatePredicateSection>,
) -> Result<ReplayOutcome, EngineError> {
    if log.mode != log.spec.mode {
        return Err(EngineError::InvalidConfig(format!(
            "replay log mode {:?} does not match TableSpec mode {:?}",
            log.mode, log.spec.mode
        )));
    }
    if !log.spec.shocks.is_empty() {
        return Err(EngineError::InvalidConfig(
            "replay_season refuses pre-seeded spec.shocks; shocks must draw from verified per-turn beacons"
                .to_owned(),
        ));
    }
    match log.mode {
        TableMode::Game if log.beacon_mode != ReplayBeaconMode::ExternalDrand => {
            return Err(EngineError::InvalidConfig(
                "Game mode replay refuses non-drand beacon_mode; ExternalDrand is required"
                    .to_owned(),
            ));
        }
        TableMode::Eval if log.beacon_mode != ReplayBeaconMode::EvalVrf => {
            return Err(EngineError::InvalidConfig(
                "Eval mode replay refuses non-VRF beacon_mode; EvalVrf is required".to_owned(),
            ));
        }
        TableMode::Club if log.beacon_mode != ReplayBeaconMode::EvalVrf => {
            return Err(EngineError::InvalidConfig(
                "Club mode replay refuses non-VRF beacon_mode; EvalVrf is required".to_owned(),
            ));
        }
        _ => {}
    }

    let mut engine = TurnEngine::new(log.spec.clone())?;
    let n = usize::from(log.spec.seat_count);
    if log.mandate_commits.len() != n || log.mandate_reveals.len() != n {
        return Err(EngineError::SeasonEnded(
            "mandate commit/reveal vectors must match seat_count".to_owned(),
        ));
    }
    if !verify_sealed_mandate_deal(
        log.spec.table_id,
        log.spec.seat_count,
        &log.spec.mandate_deck,
        &log.spec.dealt_mandate_commits,
        &log.sealed_mandate_deal,
    )? {
        return Err(EngineError::InvalidConfig(
            "replay log sealed_mandate_deal does not match spec.dealt_mandate_commits and deck"
                .to_owned(),
        ));
    }
    let clear_layout_candidates = match &log.seed.0 {
        ReplayLayoutSeedRepr::Clear(clear_seed) => {
            if log.mode == TableMode::Club {
                validate_club_dev_seed(*clear_seed)?;
            }
            Some(check_deal_seed_independence(
                *clear_seed,
                &log.sealed_mandate_deal,
                false,
            )?)
        }
        ReplayLayoutSeedRepr::Sealed(sealed) => {
            if log.mode == TableMode::Club {
                return Err(EngineError::InvalidConfig(
                    "SEED_POOL_MISMATCH: TableMode::Club replay refuses sealed-range layout seeds"
                        .to_owned(),
                ));
            }
            if sealed.pool != SEALED_SEED_POOL_NAME {
                return Err(EngineError::InvalidConfig(
                    ReplayLayoutSeedError::InvalidSealedPool {
                        pool: sealed.pool.clone(),
                    }
                    .to_string(),
                ));
            }
            // Replay cannot recompute `deal_seed` or Eval-VRF independence against the clear seed
            // because a sealed log carries only the pool and digest; it records and requires the
            // writer's unverified write-time claim (`SealedReplaySeed::independence_checked_at_write_unverified`).
            // Off-rig verification awaits issue `#58` keyed commitments.
            if !sealed.independence_checked_at_write_unverified {
                return Err(EngineError::InvalidConfig(
                    "SEALED_INDEPENDENCE_UNCHECKED: sealed replay log requires writer claim independence_checked_at_write_unverified=true"
                        .to_owned(),
                ));
            }
            None
        }
    };

    let (drand_schedule, eval_vrf_pk) = match log.mode {
        TableMode::Game => {
            if log.eval_vrf_pubkey.is_some() {
                return Err(EngineError::InvalidConfig(
                    "Game mode replay log must not carry eval_vrf_pubkey".to_owned(),
                ));
            }
            let Some(schedule) = log.spec.drand_schedule else {
                return Err(EngineError::InvalidConfig(
                    "ExternalDrand replay requires TableSpec::drand_schedule".to_owned(),
                ));
            };
            schedule.validate()?;
            (Some(schedule), None)
        }
        TableMode::Eval | TableMode::Club => {
            let Some(spec_pk) = log.spec.eval_vrf_pubkey else {
                return Err(EngineError::InvalidConfig(
                    "EvalVrf replay requires TableSpec::eval_vrf_pubkey".to_owned(),
                ));
            };
            if log.eval_vrf_pubkey != Some(spec_pk) {
                return Err(EngineError::InvalidConfig(
                    "SeasonReplayLog::eval_vrf_pubkey must match TableSpec::eval_vrf_pubkey"
                        .to_owned(),
                ));
            }
            validate_eval_vrf_pubkey(spec_pk)?;
            if let Some(candidates) = clear_layout_candidates {
                check_eval_vrf_independence(candidates, spec_pk, false)?;
            }
            (None, Some(spec_pk))
        }
    };
    for (s_idx, commit_opt) in log.mandate_commits.iter().enumerate() {
        if let Some(commit) = commit_opt {
            engine.submit_mandate_commitment(Seat(s_idx as u8), *commit)?;
        }
    }
    if log.turns.is_empty() {
        return Err(EngineError::SeasonEnded(
            "season replay log contains zero turns".to_owned(),
        ));
    }

    let mut receipts = Vec::with_capacity(log.turns.len());
    let mut state_digests = Vec::with_capacity(log.turns.len());
    let mut prev_drand_round: Option<u64> = None;
    let mut rejected_market_actions = Vec::new();
    let mut drand_beacon_auth_status = None;

    enum TurnSource<'a> {
        Drand(u64, &'a [u8]),
        EvalVrf(&'a [u8]),
    }

    for logged in &log.turns {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            return Err(EngineError::SeasonEnded(format!(
                "unexpected turn {} after season already entered {:?}",
                logged.turn,
                engine.state().phase
            )));
        }
        if logged.turn != engine.state().turn {
            return Err(EngineError::SeasonEnded(format!(
                "expected turn {}, logged turn {}",
                engine.state().turn,
                logged.turn
            )));
        }
        let verified_source = match log.mode {
            TableMode::Eval | TableMode::Club => {
                if logged.drand_round.is_some() || logged.drand_signature.is_some() {
                    return Err(EngineError::InvalidOrder(format!(
                        "turn {} carries drand fields in {:?} mode",
                        logged.turn, log.mode
                    )));
                }
                let Some(ref vrf_proof) = logged.eval_vrf_proof else {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} missing eval_vrf_proof in EvalVrf mode",
                        logged.turn
                    )));
                };
                let Some(pk) = eval_vrf_pk else {
                    return Err(EngineError::InvalidConfig(
                        "EvalVrf replay requires eval_vrf_pubkey".to_owned(),
                    ));
                };
                verify_eval_vrf_beacon(
                    pk,
                    log.spec.table_id,
                    Turn(logged.turn),
                    vrf_proof,
                    logged.turn_beacon,
                )?;
                TurnSource::EvalVrf(vrf_proof.as_slice())
            }
            TableMode::Game => {
                if logged.eval_vrf_proof.is_some() {
                    return Err(EngineError::InvalidOrder(format!(
                        "turn {} carries eval_vrf_proof in Game mode",
                        logged.turn
                    )));
                }
                let Some(round) = logged.drand_round else {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} missing drand_round in ExternalDrand mode",
                        logged.turn
                    )));
                };
                if prev_drand_round.is_some_and(|prev| round <= prev) {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} drand_round {round} is not strictly increasing",
                        logged.turn
                    )));
                }
                let Some(ref drand_signature) = logged.drand_signature else {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} missing drand_signature in ExternalDrand mode",
                        logged.turn
                    )));
                };
                let Some(ref schedule) = drand_schedule else {
                    return Err(EngineError::InvalidConfig(
                        "ExternalDrand replay requires TableSpec::drand_schedule".to_owned(),
                    ));
                };
                let auth_status @ DrandBeaconAuthStatus::HashBoundUnauthenticated =
                    verify_drand_quicknet_beacon(
                        schedule,
                        Turn(logged.turn),
                        round,
                        drand_signature,
                        logged.turn_beacon,
                    )?;
                drand_beacon_auth_status = Some(auth_status);
                prev_drand_round = Some(round);
                TurnSource::Drand(round, drand_signature.as_slice())
            }
        };

        let expected_drawn_shocks = draw_shocks_for_turn(
            log.spec.table_id,
            log.spec.seat_count,
            Turn(logged.turn),
            logged.turn_beacon,
        );
        if logged.drawn_shocks != expected_drawn_shocks {
            return Err(EngineError::InvalidOrder(format!(
                "turn {} logged drawn_shocks do not match verified beacon HKDF shocks stream",
                logged.turn
            )));
        }

        let at_turn_start = match section {
            Some(_) => Some(all_seat_predicates(&engine)?),
            None => None,
        };
        let action_len = logged.market_actions.len();
        let lock_at = match logged.cancel_lock_at {
            Some(idx) => {
                let idx_usize = usize::try_from(idx).map_err(|_| {
                    EngineError::InvalidOrder(format!(
                        "turn {} cancel_lock_at {idx} exceeds usize range",
                        logged.turn
                    ))
                })?;
                if idx_usize > action_len {
                    return Err(EngineError::InvalidOrder(format!(
                        "turn {} cancel_lock_at {idx} exceeds market_actions.len() ({action_len})",
                        logged.turn
                    )));
                }
                Some(idx_usize)
            }
            None => None,
        };
        let mut turn_rejections = Vec::new();
        for (idx, action) in logged.market_actions.iter().enumerate() {
            if lock_at == Some(idx) {
                engine.begin_cancel_lock()?;
            }
            if let Err(err) = engine.apply_market_action(action.clone()) {
                turn_rejections.push(RejectedMarketAction {
                    action_index: idx as u32,
                    error_code: err.code().to_owned(),
                });
            }
        }
        if lock_at == Some(action_len) {
            engine.begin_cancel_lock()?;
        }
        if !turn_rejections.is_empty() {
            rejected_market_actions.push(TurnRejectedActions {
                turn: logged.turn,
                rejections: turn_rejections,
            });
        }
        engine.advance_to_commit()?;
        for (&seat_idx, &commit) in &logged.commitments {
            engine.submit_order_commitment(Seat(seat_idx), commit)?;
        }
        engine.advance_to_reveal()?;
        for (&seat_idx, reveal) in &logged.reveals {
            engine.submit_order_reveal(Seat(seat_idx), reveal.clone())?;
        }
        let (receipt, details) = match verified_source {
            TurnSource::Drand(round, sig) => engine.sweep_turn_with_drand_detailed(round, sig)?,
            TurnSource::EvalVrf(proof) => engine.sweep_turn_with_eval_vrf_detailed(proof)?,
        };
        if let (Some(sec), Some(at_turn_start)) = (section.as_deref_mut(), at_turn_start) {
            let sweep = details
                .upkeep_by_seat
                .iter()
                .zip(&details.spoilage_by_seat)
                .enumerate()
                .map(|(s_idx, (upkeep, &spoiled))| SeatSweepOutcome {
                    seat: s_idx as u8,
                    upkeep_shortfall_units: upkeep.upkeep_required - upkeep.paid_with_refined,
                    shortfall_debt_added: upkeep.new_shortfall_debt,
                    spoiled_units: spoiled,
                })
                .collect();
            sec.turns.push(TurnPredicateRecord {
                turn: logged.turn,
                at_turn_start,
                sweep,
            });
        }
        state_digests.push(receipt.post_state_digest);
        receipts.push(receipt);
    }

    if engine.state().phase != TurnPhase::AwaitingMandateReveal {
        return Err(EngineError::SeasonEnded(
            "replay log ended before stochastic or Turn-32 close occurred".to_owned(),
        ));
    }

    if let Some(sec) = section {
        sec.at_close = all_seat_predicates(&engine)?;
    }

    for (s_idx, rev_opt) in log.mandate_reveals.iter().enumerate() {
        if let Some(rev) = rev_opt {
            engine.submit_mandate_reveal(Seat(s_idx as u8), rev.clone())?;
        }
    }

    let scoreboard = engine.finalize_season()?;
    Ok(ReplayOutcome {
        receipts,
        state_digests,
        scoreboard,
        rejected_market_actions,
        drand_beacon_auth_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::{RevealSalt, SealedOrderBundle, compute_order_commitment};
    use crate::types::{TableId, Turn};

    #[test]
    fn replay_rejects_incomplete_tampered_and_post_close_logs() {
        let Ok((spec, sealed_deal, eval_seed)) =
            TableSpec::standard_eval_chiral_ring_with_setup(TableId(101), 4, 7)
        else {
            panic!("valid spec");
        };
        let Ok(clear_seed) = ReplayLayoutSeed::try_clear(7) else {
            panic!("clear dev seed");
        };
        let eval_pk = spec.eval_vrf_pubkey;
        let empty = SeasonReplayLog {
            spec: spec.clone(),
            mode: TableMode::Eval,
            seed: clear_seed.clone(),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk,
            sealed_mandate_deal: sealed_deal.clone(),
            mandate_commits: vec![None; 4],
            turns: Vec::new(),
            mandate_reveals: vec![None; 4],
        };
        assert!(matches!(
            replay_season(&empty),
            Err(EngineError::SeasonEnded(_))
        ));

        let bad_len = SeasonReplayLog {
            spec: spec.clone(),
            mode: TableMode::Eval,
            seed: clear_seed.clone(),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk,
            sealed_mandate_deal: sealed_deal.clone(),
            mandate_commits: vec![None; 3],
            turns: Vec::new(),
            mandate_reveals: vec![None; 4],
        };
        assert!(matches!(
            replay_season(&bad_len),
            Err(EngineError::SeasonEnded(_))
        ));

        // Full minimal 24..=32 turn close via VRF-verified beacons, checking replay & errors.
        let mut engine = match TurnEngine::new(spec.clone()) {
            Ok(e) => e,
            Err(err) => panic!("init failed: {err}"),
        };
        let mut mandate_commits = vec![None; 4];
        let mut mandate_reveals = vec![None; 4];
        let rev0 = sealed_deal.seat_reveals[0].clone();
        let mc0 = spec.dealt_mandate_commits[0];
        mandate_commits[0] = Some(mc0);
        mandate_reveals[0] = Some(rev0.clone());
        assert!(engine.submit_mandate_commitment(Seat(0), mc0).is_ok());

        let mut turns = Vec::new();
        for t in 1..=32u8 {
            if engine.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            assert!(engine.advance_to_commit().is_ok());
            let mut commitments = BTreeMap::new();
            let mut reveals = BTreeMap::new();
            for s in 0..4u8 {
                let ord_rev = OrderReveal {
                    bundle: SealedOrderBundle::default(),
                    salt: RevealSalt([s.wrapping_add(t); 32]),
                };
                let Ok(ch) = compute_order_commitment(TableId(101), Turn(t), Seat(s), &ord_rev)
                else {
                    panic!("commit");
                };
                assert!(engine.submit_order_commitment(Seat(s), ch).is_ok());
                commitments.insert(s, ch);
                reveals.insert(s, ord_rev);
            }
            assert!(engine.advance_to_reveal().is_ok());
            for (&s, r) in &reveals {
                assert!(engine.submit_order_reveal(Seat(s), r.clone()).is_ok());
            }
            let Ok((beacon, proof)) = eval_seed.prove_turn(TableId(101), Turn(t)) else {
                panic!("vrf prove");
            };
            let Ok(receipt) = engine.sweep_turn_with_eval_vrf(&proof) else {
                panic!("sweep");
            };
            turns.push(LoggedTurn {
                turn: t,
                market_actions: Vec::new(),
                cancel_lock_at: None,
                commitments,
                reveals,
                turn_beacon: beacon,
                drand_round: None,
                drand_signature: None,
                eval_vrf_proof: Some(proof.to_vec()),
                drawn_shocks: receipt.newly_drawn_shocks.clone(),
            });
            if receipt.season_closed {
                break;
            }
        }
        assert!(engine.submit_mandate_reveal(Seat(0), rev0).is_ok());
        let Ok(live_board) = engine.finalize_season() else {
            panic!("finalize");
        };

        let log = SeasonReplayLog {
            spec,
            mode: TableMode::Eval,
            seed: clear_seed,
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk,
            sealed_mandate_deal: sealed_deal,
            mandate_commits,
            turns: turns.clone(),
            mandate_reveals,
        };
        let Ok(out1) = replay_season(&log) else {
            panic!("replay ok");
        };
        assert_eq!(out1.scoreboard, live_board);

        // Truncated log before closing must fail cleanly.
        let mut trunc = log.clone();
        trunc.turns.pop();
        assert!(matches!(
            replay_season(&trunc),
            Err(EngineError::SeasonEnded(_))
        ));

        // Extra turn after close must fail cleanly.
        let mut extra = log.clone();
        let next_t = extra.turns.len() as u8 + 1;
        extra.turns.push(LoggedTurn {
            turn: next_t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments: BTreeMap::new(),
            reveals: BTreeMap::new(),
            turn_beacon: [0; 32],
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: None,
            drawn_shocks: Vec::new(),
        });
        assert!(matches!(
            replay_season(&extra),
            Err(EngineError::SeasonEnded(_))
        ));

        // Wrong turn index must fail cleanly.
        let mut bad_idx = log;
        bad_idx.turns[0].turn = 9;
        assert!(matches!(
            replay_season(&bad_idx),
            Err(EngineError::SeasonEnded(_))
        ));
    }
}
