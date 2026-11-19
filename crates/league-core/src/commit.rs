//! Cryptographic commit-reveal envelopes, sealed turn bundles, private charter mandates,
//! and typed diplomatic intent pledges (never storing raw chat prose).

use std::fmt;

use hkdf::Hkdf;
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use league_seat_types::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder,
    CouncilPlatform, CouncilVoteBallot, DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_PERIOD_SECS,
    DrandSchedule, DualEscrowProposal, EscrowFinishUnlock, GradedMandateCard, IntentPledge,
    IntentTarget, MandateKind, PreimageSecret, PrivateMandate, ProjectDeliveryOrder,
    SealedOrderBundle, drand_quicknet_round_for_close,
};

use crate::types::{
    BPS_SCALE, Bps, CreditTier, EngineError, MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY,
    ScheduledShockSpec, Seat, ShockKind, TableId, Turn, Units,
};

pub(crate) mod hex32 {
    pub(crate) use league_seat_types::wire_serde::{
        deserialize_hex32 as deserialize, serialize_hex32 as serialize,
    };
}

/// Secret 32-byte salt used inside Phase 2 commit-reveal bundles and Turn-1 mandate commits.
///
/// Redacts its raw bytes in `Debug` output (`CLAUDE.md` invariant).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealSalt(#[serde(with = "hex32")] pub [u8; 32]);

impl RevealSalt {
    /// Returns `true` when all 32 bytes are zero (degenerate salt forbidden in commits).
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Validates that the salt is not all-zero (`[0u8; 32]`).
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.is_degenerate() {
            return Err(EngineError::InvalidOrder(
                "degenerate all-zero RevealSalt is forbidden".to_owned(),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for RevealSalt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RevealSalt(<hidden>)")
    }
}

/// Revealed Phase 2 bundle plus its secret 32-byte salt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderReveal {
    /// Action orders executed at Sweep if reveal matches commitment.
    pub bundle: SealedOrderBundle,
    /// Secret salt committed in Phase 2.
    pub salt: RevealSalt,
}

/// Revealed Turn-1 Private Charter Mandate plus its secret 32-byte salt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateReveal {
    /// Mandate terms evaluated at close.
    pub mandate: PrivateMandate,
    /// Secret salt committed at Turn 1.
    pub salt: RevealSalt,
}

/// Maps empirical completion rate (`0..=10_000 bps`) monotonically into `3_000..=8_000`
/// milli-points (`3..=8` mandate points), separated from terminal net-worth conversion.
#[must_use]
pub fn bonus_points_milli_for_completion_bps(completion_bps: Bps) -> Units {
    let clamped = i64::from(completion_bps.min(10_000));
    let hard_bps = (BPS_SCALE as i64) - clamped;
    MIN_MANDATE_BONUS_EQUITY
        + ((MAX_MANDATE_BONUS_EQUITY - MIN_MANDATE_BONUS_EQUITY) * hard_bps) / (BPS_SCALE as i64)
}

/// Maps empirical completion rate (`0..=10_000 bps`) monotonically into `3–8 VP` (`3_000..=8_000`).
#[must_use]
pub fn bonus_equity_for_completion_bps(completion_bps: Bps) -> Units {
    bonus_points_milli_for_completion_bps(completion_bps)
}

/// Builds a graded mandate deck for `seat_count` (`4..=6`), scaling `MinProjectEquity` targets
/// against `mean_project_value_milli` (`7.0x`, `12.5x`, `16.5x`) while using canonical default
/// `[16_000, 28_000, 36_000]` targets via [`default_graded_mandate_deck`].
#[must_use]
pub fn graded_mandate_deck_for_mean_project_value(
    seat_count: u8,
    mean_project_value_milli: Units,
) -> Vec<GradedMandateCard> {
    let targets = if mean_project_value_milli <= 0 {
        [16_000, 28_000, 36_000]
    } else {
        let base = mean_project_value_milli as i128;
        [
            ((base * 7_000) / 1_000).max(1_000) as Units,
            ((base * 12_500) / 1_000).max(2_000) as Units,
            ((base * 16_500) / 1_000).max(3_000) as Units,
        ]
    };
    build_graded_mandate_deck_with_targets(seat_count, targets)
}

/// Builds the canonical 8-card graded mandate deck for `seat_count` (`4..=6`), covering all
/// [`MandateKind`] variants with monopoly-safe thresholds (`max_safe = seat_count - 2`) and
/// bonuses spanning `3_000..=8_000` (`3–8 VP`).
///
/// Provisional default deck (`F3`): the current `difficulty_completion_bps` numbers on these
/// cards are uncalibrated placeholders derived for structural balance only until empirical
/// playtest calibration replaces them.
#[must_use]
pub fn default_graded_mandate_deck(seat_count: u8) -> Vec<GradedMandateCard> {
    build_graded_mandate_deck_with_targets(seat_count, [16_000, 28_000, 36_000])
}

fn build_graded_mandate_deck_with_targets(
    seat_count: u8,
    project_targets: [Units; 3],
) -> Vec<GradedMandateCard> {
    let max_safe = seat_count.saturating_sub(2).max(1);
    let specs = [
        (
            1,
            MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Bbb,
            },
            10_000,
        ),
        (
            2,
            MandateKind::MinProjectEquity {
                target: project_targets[0],
            },
            8_000,
        ),
        (
            3,
            MandateKind::MinRefinedVariety {
                distinct_goods: max_safe.min(2),
                min_each: 1_500,
            },
            7_000,
        ),
        (
            4,
            MandateKind::MinActivePartners {
                min_partners: max_safe,
            },
            6_000,
        ),
        (
            5,
            MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            4_000,
        ),
        (
            6,
            MandateKind::MinRefinedVariety {
                distinct_goods: max_safe,
                min_each: 3_000,
            },
            2_500,
        ),
        (
            7,
            MandateKind::MinProjectEquity {
                target: project_targets[1],
            },
            1_500,
        ),
        (
            8,
            MandateKind::MinProjectEquity {
                target: project_targets[2],
            },
            0,
        ),
    ];
    specs
        .into_iter()
        .map(
            |(card_id, kind, difficulty_completion_bps)| GradedMandateCard {
                card_id,
                kind,
                difficulty_completion_bps,
                bonus_equity: bonus_points_milli_for_completion_bps(difficulty_completion_bps),
            },
        )
        .collect()
}

/// Sealed initial charter-mandate deal generated from fresh 256-bit OS CSPRNG entropy.
///
/// `deal_seed` shuffles the candidate deck independently of any public replay RNG `seed: u64`,
/// and each seat receives an independent 256-bit OS-drawn [`RevealSalt`] in `seat_reveals`.
/// Raw entropy and per-seat secrets are redacted in `Debug` output (`CLAUDE.md` invariant).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedMandateDeal {
    /// Fresh 256-bit shuffle entropy drawn independently of any table replay `seed`.
    #[serde(with = "hex32")]
    pub deal_seed: [u8; 32],
    /// Ordered private mandate reveals for seats `0..seat_count`.
    pub seat_reveals: Vec<MandateReveal>,
}

impl fmt::Debug for SealedMandateDeal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedMandateDeal")
            .field("deal_seed", &"[REDACTED]")
            .field("seat_reveals_len", &self.seat_reveals.len())
            .finish()
    }
}

/// Maps a 32-byte digest into `[0, upper_exclusive)` using `u64` modulo reduction
/// before narrowing to `usize`, guaranteeing identical selection on 32-bit and 64-bit targets.
///
/// # Panics
/// Panics if `upper_exclusive == 0`.
#[must_use]
pub fn width_independent_index(digest: [u8; 32], upper_exclusive: usize) -> usize {
    assert!(upper_exclusive > 0, "upper_exclusive must be non-zero");
    let mut le = [0u8; 8];
    le.copy_from_slice(&digest[0..8]);
    let bound = u64::try_from(upper_exclusive).unwrap_or(u64::MAX);
    let idx_u64 = u64::from_le_bytes(le) % bound;
    usize::try_from(idx_u64).unwrap_or(0)
}

pub(crate) fn draw_nonzero_256_bits() -> Result<[u8; 32], EngineError> {
    for _ in 0..8 {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|err| {
            EngineError::InvalidOrder(format!("OS CSPRNG failure drawing 256 bits: {err}"))
        })?;
        if bytes != [0u8; 32] {
            return Ok(bytes);
        }
    }
    Err(EngineError::InvalidOrder(
        "OS CSPRNG returned all-zero 256-bit block after 8 attempts".to_owned(),
    ))
}

/// Shuffles `deck` with fresh 256-bit OS CSPRNG entropy (`deal_seed` and per-seat [`RevealSalt`]s)
/// and deals one [`CommitmentHash`] and [`MandateReveal`] per seat (`M3`).
///
/// Never derives mandate order or salts from a table's public replay RNG `seed: u64`.
pub fn deal_graded_mandates(
    table_id: TableId,
    seat_count: u8,
    deck: &[GradedMandateCard],
) -> Result<(Vec<CommitmentHash>, SealedMandateDeal), EngineError> {
    let n = usize::from(seat_count);
    let deal_seed = draw_nonzero_256_bits()?;
    let mut seat_salts = Vec::with_capacity(n);
    for _ in 0..n {
        seat_salts.push(RevealSalt(draw_nonzero_256_bits()?));
    }
    deal_graded_mandates_with_entropy(table_id, seat_count, deal_seed, &seat_salts, deck)
}

/// Computes the canonical `seat_count` dealt deck indices from `table_id`, `deal_seed`, and
/// `deck_len` (`league-core/mandate-deal/v2`, reverse Fisher-Yates without replacement).
///
/// Single canonical deal permutation shared across `league-core` and `league-gen` (`#32` `B-2`).
pub fn deal_mandate_deck_indices(
    table_id: TableId,
    seat_count: u8,
    deal_seed: [u8; 32],
    deck_len: usize,
) -> Result<Vec<usize>, EngineError> {
    let n = usize::from(seat_count);
    if !(4..=6).contains(&seat_count) || deck_len < n {
        return Err(EngineError::InvalidConfig(format!(
            "invalid mandate deal config: seat_count={seat_count}, deck_len={deck_len}"
        )));
    }
    if deal_seed == [0u8; 32] {
        return Err(EngineError::InvalidConfig(
            "degenerate all-zero mandate deal_seed is forbidden".to_owned(),
        ));
    }
    let mut indices: Vec<usize> = (0..deck_len).collect();
    for i in (1..indices.len()).rev() {
        let mut hasher = Sha256::new();
        hasher.update(b"league-core/mandate-deal/v2");
        hasher.update(deal_seed);
        hasher.update(table_id.0.to_le_bytes());
        hasher.update(u64::try_from(i).unwrap_or(u64::MAX).to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let j = width_independent_index(digest, i + 1);
        indices.swap(i, j);
    }
    Ok(indices.into_iter().take(n).collect())
}

/// Deterministically shuffles `deck` and deals one [`CommitmentHash`] and [`MandateReveal`] per
/// seat from caller-supplied 256-bit `deal_seed` and per-seat `seat_salts`.
pub fn deal_graded_mandates_with_entropy(
    table_id: TableId,
    seat_count: u8,
    deal_seed: [u8; 32],
    seat_salts: &[RevealSalt],
    deck: &[GradedMandateCard],
) -> Result<(Vec<CommitmentHash>, SealedMandateDeal), EngineError> {
    let n = usize::from(seat_count);
    if seat_salts.len() != n {
        return Err(EngineError::InvalidConfig(format!(
            "expected {n} mandate seat_salts, got {}",
            seat_salts.len()
        )));
    }
    for card in deck {
        if card.difficulty_completion_bps > 10_000
            || !(MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY).contains(&card.bonus_equity)
        {
            return Err(EngineError::InvalidConfig(format!(
                "invalid graded mandate card {} difficulty or bonus",
                card.card_id
            )));
        }
        card.kind.validate_for_seat_count(seat_count)?;
    }
    let dealt_indices = deal_mandate_deck_indices(table_id, seat_count, deal_seed, deck.len())?;

    let mut commits = Vec::with_capacity(n);
    let mut reveals = Vec::with_capacity(n);
    for (seat_idx, card_idx) in dealt_indices.into_iter().enumerate() {
        let salt = &seat_salts[seat_idx];
        salt.validate().map_err(|err| {
            EngineError::InvalidConfig(format!("invalid salt for seat {seat_idx}: {err}"))
        })?;
        let card = deck[card_idx];
        let seat = u8::try_from(seat_idx).unwrap_or(0);
        let reveal = MandateReveal {
            mandate: PrivateMandate {
                kind: card.kind,
                bonus_equity: card.bonus_equity,
            },
            salt: salt.clone(),
        };
        let commit = compute_mandate_commitment(table_id, Seat(seat), &reveal)?;
        commits.push(commit);
        reveals.push(reveal);
    }
    Ok((
        commits,
        SealedMandateDeal {
            deal_seed,
            seat_reveals: reveals,
        },
    ))
}

/// Verifies that `deal` reproduces `expected_commits` and the shuffled card assignments under
/// `deck`.
pub fn verify_sealed_mandate_deal(
    table_id: TableId,
    seat_count: u8,
    deck: &[GradedMandateCard],
    expected_commits: &[CommitmentHash],
    deal: &SealedMandateDeal,
) -> Result<bool, EngineError> {
    let n = usize::from(seat_count);
    if expected_commits.len() != n || deal.seat_reveals.len() != n {
        return Ok(false);
    }
    let salts: Vec<RevealSalt> = deal.seat_reveals.iter().map(|r| r.salt.clone()).collect();
    let Ok((recomputed_commits, recomputed_deal)) =
        deal_graded_mandates_with_entropy(table_id, seat_count, deal.deal_seed, &salts, deck)
    else {
        return Ok(false);
    };
    Ok(recomputed_commits == expected_commits && recomputed_deal == *deal)
}

#[must_use]
pub(crate) fn raw_zero_preimage_condition_hash() -> CommitmentHash {
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/preimage/v2");
    hasher.update([0u8; 32]);
    CommitmentHash(hasher.finalize().into())
}

/// Computes `SHA-256("league-core/preimage/v2" || secret)` for conditional `TokenEscrow`,
/// rejecting all-zero degenerate [`PreimageSecret`] values.
pub fn compute_preimage_condition(secret: &PreimageSecret) -> Result<CommitmentHash, EngineError> {
    secret.validate()?;
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/preimage/v2");
    hasher.update(secret.0);
    Ok(CommitmentHash(hasher.finalize().into()))
}

/// Verifies a `PREIMAGE-SHA-256` unlock preimage against `expected`, rejecting degenerate hashes
/// and all-zero secrets.
pub fn verify_preimage_condition(
    expected: CommitmentHash,
    secret: &PreimageSecret,
) -> Result<bool, EngineError> {
    if expected.is_degenerate() || expected == raw_zero_preimage_condition_hash() {
        return Err(EngineError::InvalidOrder(
            "degenerate escrow condition_hash is forbidden".to_owned(),
        ));
    }
    Ok(compute_preimage_condition(secret)? == expected)
}

/// Computes canonical Phase 2 SHA-256 commitment for `table_id`, `turn`, `seat`, and `reveal`.
pub fn compute_order_commitment(
    table_id: TableId,
    turn: Turn,
    seat: Seat,
    reveal: &OrderReveal,
) -> Result<CommitmentHash, EngineError> {
    reveal.salt.validate()?;
    let encoded = serde_json::to_vec(&reveal.bundle)
        .map_err(|err| EngineError::InvalidOrder(format!("failed to serialize bundle: {err}")))?;
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/order-commit/v2");
    hasher.update(table_id.0.to_le_bytes());
    hasher.update([turn.0, seat.0]);
    hasher.update(reveal.salt.0);
    hasher.update(encoded);
    Ok(CommitmentHash(hasher.finalize().into()))
}

/// Verifies a Phase 2 order reveal against a recorded `CommitmentHash`.
pub fn verify_order_reveal(
    table_id: TableId,
    turn: Turn,
    seat: Seat,
    commitment: CommitmentHash,
    reveal: &OrderReveal,
) -> Result<bool, EngineError> {
    if commitment.is_degenerate() {
        return Err(EngineError::InvalidOrder(
            "degenerate all-zero commitment hash is forbidden".to_owned(),
        ));
    }
    Ok(compute_order_commitment(table_id, turn, seat, reveal)? == commitment)
}

/// Computes canonical Turn-1 Private Mandate SHA-256 commitment.
pub fn compute_mandate_commitment(
    table_id: TableId,
    seat: Seat,
    reveal: &MandateReveal,
) -> Result<CommitmentHash, EngineError> {
    reveal.salt.validate()?;
    let encoded = serde_json::to_vec(&reveal.mandate)
        .map_err(|err| EngineError::InvalidOrder(format!("failed to serialize mandate: {err}")))?;
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/mandate-commit/v2");
    hasher.update(table_id.0.to_le_bytes());
    hasher.update([seat.0]);
    hasher.update(reveal.salt.0);
    hasher.update(encoded);
    Ok(CommitmentHash(hasher.finalize().into()))
}

/// Verifies a Season-Close Mandate reveal against a Turn-1 commitment.
pub fn verify_mandate_reveal(
    table_id: TableId,
    seat: Seat,
    commitment: CommitmentHash,
    reveal: &MandateReveal,
) -> Result<bool, EngineError> {
    if commitment.is_degenerate() {
        return Err(EngineError::InvalidOrder(
            "degenerate all-zero commitment hash is forbidden".to_owned(),
        ));
    }
    Ok(compute_mandate_commitment(table_id, seat, reveal)? == commitment)
}

/// Derives a deterministic standalone turn randomness beacon from `seed`, `table_id`, and `turn`.
#[must_use]
pub fn derive_turn_beacon(seed: u64, table_id: TableId, turn: Turn) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/beacon/v2");
    hasher.update(seed.to_le_bytes());
    hasher.update(table_id.0.to_le_bytes());
    hasher.update([turn.0]);
    hasher.finalize().into()
}

/// Maps a 32-byte stream uniformly into basis points `[0, 10_000)` using rejection across its
/// four 64-bit little-endian lanes so modulo bias is strictly zero.
#[must_use]
pub fn beacon_roll_bps(beacon: [u8; 32]) -> Bps {
    const UNBIASED_LIMIT: u64 = (u64::MAX / 10_000) * 10_000;
    for &lane in beacon.as_chunks::<8>().0 {
        let v = u64::from_le_bytes(lane);
        if v < UNBIASED_LIMIT {
            return (v % 10_000) as Bps;
        }
    }
    let mut le = [0u8; 8];
    le.copy_from_slice(&beacon[0..8]);
    (u64::from_le_bytes(le) % 10_000) as Bps
}

/// Domain-separated RFC 5869 HKDF stream categories (`#31`, `2026-11-15` ruling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RngDomain {
    /// 3-stage Horizon Shock sampling (`"shocks"`).
    Shocks,
    /// Stochastic season closing bell roll (`"close_roll"`).
    CloseRoll,
    /// Council election plurality tie-breaker (`"tiebreak"`).
    Tiebreak,
    /// Phase-1 simultaneous DEX batch tie-breaker (`"batch_tie"`).
    BatchTie,
}

impl RngDomain {
    /// Canonical ASCII label embedded in RFC 5869 `HKDF-Expand` info strings.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shocks => "shocks",
            Self::CloseRoll => "close_roll",
            Self::Tiebreak => "tiebreak",
            Self::BatchTie => "batch_tie",
        }
    }
}

/// Expands a 32-byte pseudorandom stream from `turn_beacon` via RFC 5869 `HKDF-SHA256`,
/// domain-separated by `(table_id, domain, turn, subtag)` (`#31`, Seed-28 Report §2).
#[must_use]
pub fn expand_turn_stream(
    turn_beacon: [u8; 32],
    table_id: TableId,
    turn: Turn,
    domain: RngDomain,
    subtag: &[u8],
) -> [u8; 32] {
    let mut salt_hasher = Sha256::new();
    salt_hasher.update(b"league-core/hkdf-salt/v2|");
    salt_hasher.update(table_id.0.to_le_bytes());
    let salt: [u8; 32] = salt_hasher.finalize().into();

    let hk = Hkdf::<Sha256>::new(Some(&salt), &turn_beacon);
    let mut info = Vec::with_capacity(16 + domain.as_str().len() + subtag.len());
    info.extend_from_slice(b"v1|");
    info.extend_from_slice(domain.as_str().as_bytes());
    info.push(b'|');
    info.push(turn.0);
    if !subtag.is_empty() {
        info.push(b'|');
        info.extend_from_slice(subtag);
    }

    let mut okm = [0u8; 32];
    if hk.expand(&info, &mut okm).is_err() {
        unreachable!("32-byte HKDF-SHA256 output is always within 255 * 32 limit");
    }
    okm
}

/// Canonical Sweep turns on which a new 3-stage Horizon Shock `(t+1, t+2, t+3)` is drawn from the
/// turn's HKDF `"shocks"` stream (`#31`, realizing at turns `5, 10, 15, 20 <= 23`).
pub const DEFAULT_SHOCK_DRAW_TURNS: [u8; 4] = [2, 7, 12, 17];

/// Draws any 3-stage [`ScheduledShockSpec`] triggered at Sweep Step 8 of `turn` from `turn_beacon`
/// using the domain-separated HKDF `"shocks"` stream (`#31`).
#[must_use]
pub fn draw_shocks_for_turn(
    table_id: TableId,
    seat_count: u8,
    turn: Turn,
    turn_beacon: [u8; 32],
) -> Vec<ScheduledShockSpec> {
    if !(4..=6).contains(&seat_count) || !DEFAULT_SHOCK_DRAW_TURNS.contains(&turn.0) {
        return Vec::new();
    }
    let okm = expand_turn_stream(turn_beacon, table_id, turn, RngDomain::Shocks, b"");
    let mut b0 = [0u8; 8];
    let mut b1 = [0u8; 8];
    let mut b2 = [0u8; 8];
    b0.copy_from_slice(&okm[0..8]);
    b1.copy_from_slice(&okm[8..16]);
    b2.copy_from_slice(&okm[16..24]);

    let target_seat = (u64::from_le_bytes(b0) % u64::from(seat_count)) as u8;
    let kind = match u64::from_le_bytes(b1) % 4 {
        0 => ShockKind::SupplyFreeze,
        1 => ShockKind::TariffHoliday,
        2 => ShockKind::DemandSurge,
        _ => ShockKind::SpoilageSpike,
    };
    let (lo, hi): (u64, u64) = match kind {
        ShockKind::SupplyFreeze => (1_800, 3_500),
        ShockKind::TariffHoliday => (5_000, 10_000),
        ShockKind::DemandSurge | ShockKind::SpoilageSpike => (1_500, 3_000),
    };
    let span = hi - lo + 1;
    let magnitude_bps = (lo + (u64::from_le_bytes(b2) % span)) as Bps;

    vec![ScheduledShockSpec {
        hint_turn: turn.0 + 1,
        forecast_turn: turn.0 + 2,
        realize_turn: turn.0 + 3,
        target_seat,
        kind,
        magnitude_bps,
    }]
}

/// Evaluates the stochastic season closing roll (in `0..10_000 bps`) from `turn_beacon` using the
/// domain-separated HKDF `"close_roll"` stream (`#31`).
#[must_use]
pub fn close_roll_bps_from_beacon(turn_beacon: [u8; 32], table_id: TableId, turn: Turn) -> Bps {
    let okm = expand_turn_stream(turn_beacon, table_id, turn, RngDomain::CloseRoll, b"");
    beacon_roll_bps(okm)
}

/// Deterministically breaks an election plurality tie among `tied` seat indices using the
/// domain-separated HKDF `"tiebreak"` stream for `(table_id, turn, domain_tag)`.
#[must_use]
pub fn break_tie_with_turn_beacon(
    turn_beacon: [u8; 32],
    table_id: TableId,
    turn: Turn,
    domain_tag: &[u8],
    tied: &[u8],
) -> u8 {
    match tied {
        [] => 0,
        [only] => *only,
        _ => {
            let okm =
                expand_turn_stream(turn_beacon, table_id, turn, RngDomain::Tiebreak, domain_tag);
            tied[width_independent_index(okm, tied.len())]
        }
    }
}

/// Deterministically breaks an election plurality tie among `tied` seat indices using `beacon`
/// expanded via RFC 5869 HKDF (`RngDomain::Tiebreak`).
#[must_use]
pub fn break_tie_with_beacon(beacon: [u8; 32], domain_tag: &[u8], tied: &[u8]) -> u8 {
    break_tie_with_turn_beacon(beacon, TableId(0), Turn(0), domain_tag, tied)
}

/// Derives a deterministic 256-bit final-standings tie-break key for `seat` at terminal `turn`
/// from `turn_beacon` via RFC 5869 `HKDF-SHA256` (`RngDomain::Tiebreak`, subtag `b"standings|<seat>"`).
#[must_use]
pub fn standings_tie_key(
    turn_beacon: [u8; 32],
    table_id: TableId,
    turn: Turn,
    seat: u8,
) -> [u8; 32] {
    let subtag = [
        b's', b't', b'a', b'n', b'd', b'i', b'n', b'g', b's', b'|', seat,
    ];
    expand_turn_stream(turn_beacon, table_id, turn, RngDomain::Tiebreak, &subtag)
}

/// Expected byte length of a compressed BLS12-381 G1 drand `quicknet` signature.
pub const DRAND_QUICKNET_SIGNATURE_BYTES: usize = 48;

/// Authentication status of a drand `quicknet` round signature (`#29`).
///
/// Until `#29` lands with BLS12-381 pairing verification against the League of Entropy
/// `quicknet` chain public key, `league-core` verifies round-schedule binding and
/// `turn_beacon == SHA-256(drand_signature)` only — beacons in replay are **hash-bound, not yet
/// authenticated**.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrandBeaconAuthStatus {
    /// Structural signature check only — no signature authentication against the `quicknet` chain
    /// public key is performed yet (`#29`). Hash binding (`turn_beacon == SHA-256(drand_signature)`)
    /// and schedule-round binding are established only by [`verify_drand_quicknet_beacon`].
    HashBoundUnauthenticated,
}

impl DrandBeaconAuthStatus {
    /// Returns the stable `snake_case` status string (`"hash_bound_unauthenticated"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HashBoundUnauthenticated => "hash_bound_unauthenticated",
        }
    }
}

impl fmt::Display for DrandBeaconAuthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returns the `#29` drand `quicknet` BLS12-381 authentication status for
/// `(drand_round, drand_signature)`.
///
/// Rejects round `0`, signatures whose byte length is not [`DRAND_QUICKNET_SIGNATURE_BYTES`]
/// (`48`), and the degenerate all-zero slice. Until `#29` lands with the pinned `quicknet` chain
/// public key and BLS12-381 pairing check, every well-formed non-zero input returns
/// [`DrandBeaconAuthStatus::HashBoundUnauthenticated`] (no cryptographic signature authentication
/// performed).
pub fn drand_quicknet_signature_status(
    drand_round: u64,
    drand_signature: &[u8],
) -> Result<DrandBeaconAuthStatus, EngineError> {
    if drand_round == 0 {
        return Err(EngineError::InvalidOrder(
            "drand quicknet round must be >= 1".to_owned(),
        ));
    }
    if drand_signature.len() != DRAND_QUICKNET_SIGNATURE_BYTES {
        return Err(EngineError::InvalidOrder(format!(
            "drand quicknet signature must be {DRAND_QUICKNET_SIGNATURE_BYTES} bytes, got {}",
            drand_signature.len()
        )));
    }
    if drand_signature.iter().all(|&b| b == 0) {
        return Err(EngineError::InvalidOrder(
            "degenerate all-zero drand signature is forbidden".to_owned(),
        ));
    }
    Ok(DrandBeaconAuthStatus::HashBoundUnauthenticated)
}

/// Derives the 32-byte turn randomness beacon as `SHA-256(drand_signature)` (`design-v2` §6.1).
///
/// **Deferred authentication notice (`#29`):** Drand beacons in Game mode are currently
/// **hash-bound, not yet authenticated** — `league-core` checks `turn_beacon ==
/// SHA-256(drand_signature)` on the scheduled `drand_round` and routes signature checking through
/// [`drand_quicknet_signature_status`], which returns
/// [`DrandBeaconAuthStatus::HashBoundUnauthenticated`] until `#29` adds BLS12-381 pairing
/// verification against the `quicknet` chain public key.
pub fn derive_drand_beacon(drand_signature: &[u8]) -> Result<[u8; 32], EngineError> {
    if drand_signature.len() != DRAND_QUICKNET_SIGNATURE_BYTES {
        return Err(EngineError::InvalidOrder(format!(
            "drand quicknet signature must be {DRAND_QUICKNET_SIGNATURE_BYTES} bytes, got {}",
            drand_signature.len()
        )));
    }
    if drand_signature.iter().all(|&b| b == 0) {
        return Err(EngineError::InvalidOrder(
            "degenerate all-zero drand signature is forbidden".to_owned(),
        ));
    }
    Ok(Sha256::digest(drand_signature).into())
}

/// Validates an `ExternalDrand` turn beacon against a pre-named `DrandSchedule`, confirming
/// `drand_round == expected_round_for_turn(turn)` and `expected_beacon == SHA-256(drand_signature)`,
/// and calling [`drand_quicknet_signature_status`] (`#29`'s home).
///
/// Until `#29` lands, drand beacons in replay are **hash-bound, not yet authenticated** against the
/// `quicknet` chain public key; this returns
/// [`DrandBeaconAuthStatus::HashBoundUnauthenticated`].
pub fn verify_drand_quicknet_beacon(
    schedule: &DrandSchedule,
    turn: Turn,
    drand_round: u64,
    drand_signature: &[u8],
    expected_beacon: [u8; 32],
) -> Result<DrandBeaconAuthStatus, EngineError> {
    schedule.validate()?;
    let expected_round = schedule.expected_round_for_turn(turn)?;
    if drand_round != expected_round {
        return Err(EngineError::InvalidOrder(format!(
            "turn {} expected drand quicknet round {expected_round}, got {drand_round}",
            turn.0
        )));
    }
    let auth_status = drand_quicknet_signature_status(drand_round, drand_signature)?;
    let derived = derive_drand_beacon(drand_signature)?;
    if derived != expected_beacon {
        return Err(EngineError::InvalidOrder(format!(
            "turn {} turn_beacon does not equal SHA-256(drand_signature)",
            turn.0
        )));
    }
    Ok(auth_status)
}

/// Byte length of a compressed `ristretto255` public key for the RFC 9381 ECVRF construction
/// with C2SP's `ristretto255` ciphersuite (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not an
/// IANA-registered suite in RFC 9381).
pub const EVAL_VRF_PUBKEY_BYTES: usize = 32;

/// Byte length of a proof `pi_string` (`Gamma || c || s`) for the RFC 9381 ECVRF construction
/// with C2SP's `ristretto255` ciphersuite (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not an
/// IANA-registered suite in RFC 9381).
pub const EVAL_VRF_PROOF_BYTES: usize = 80;

/// Sealed 256-bit orchestrator entropy seed for `TableMode::Eval` randomness (`#31` comment,
/// `2026-11-15` ruling).
///
/// Held strictly by the isolated Eval orchestrator (never derived from the public layout `seed`,
/// never stored in `SeasonReplayLog`, and non-serializable so raw bytes cannot be written in clear).
/// Redacts raw bytes in `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedEvalSeed([u8; 32]);

impl fmt::Debug for SealedEvalSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SealedEvalSeed(<hidden>)")
    }
}

impl SealedEvalSeed {
    /// Returns `true` when all 32 bytes are zero.
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Wraps caller-supplied 256-bit entropy after rejecting `[0u8; 32]`.
    pub fn try_from_bytes(bytes: [u8; 32]) -> Result<Self, EngineError> {
        if bytes == [0u8; 32] {
            return Err(EngineError::InvalidConfig(
                "degenerate all-zero SealedEvalSeed is forbidden".to_owned(),
            ));
        }
        Ok(Self(bytes))
    }

    /// Draws fresh non-zero 256-bit Eval entropy from OS CSPRNG (`getrandom`).
    pub fn generate() -> Result<Self, EngineError> {
        Ok(Self(draw_nonzero_256_bits()?))
    }

    fn secret_key(&self) -> Result<vrf_r255::SecretKey, EngineError> {
        if self.is_degenerate() {
            return Err(EngineError::InvalidConfig(
                "degenerate all-zero SealedEvalSeed is forbidden".to_owned(),
            ));
        }
        let rng = ChaCha20Rng::from_seed(self.0);
        Ok(vrf_r255::SecretKey::generate(rng))
    }

    /// Returns the 32-byte `ristretto255` VRF public key (`PK_eval`, RFC 9381 ECVRF construction
    /// with C2SP's `c2sp.org/vrf-r255` ciphersuite, `vrf-r255 0.1.0`) committed in
    /// `TableSpec::eval_vrf_pubkey` and `SeasonReplayLog::eval_vrf_pubkey`.
    pub fn public_key_bytes(&self) -> Result<[u8; EVAL_VRF_PUBKEY_BYTES], EngineError> {
        let sk = self.secret_key()?;
        Ok(vrf_r255::PublicKey::from(sk).to_bytes())
    }

    /// Produces the deterministic Turn `turn` `(turn_beacon, vrf_proof)` pair over
    /// [`eval_vrf_alpha`], where `turn_beacon` is the leading 32 bytes of the 64-byte
    /// `ECVRF_proof_to_hash` output `beta_t` under the RFC 9381 ECVRF construction with C2SP's
    /// `ristretto255` ciphersuite (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`).
    pub fn prove_turn(
        &self,
        table_id: TableId,
        turn: Turn,
    ) -> Result<([u8; 32], [u8; EVAL_VRF_PROOF_BYTES]), EngineError> {
        if turn.0 == 0 {
            return Err(EngineError::InvalidOrder(
                "Eval VRF turn must be >= 1".to_owned(),
            ));
        }
        let sk = self.secret_key()?;
        let pk = vrf_r255::PublicKey::from(sk);
        let alpha = eval_vrf_alpha(table_id, turn);
        let proof = sk.prove(&alpha);
        let proof_bytes = proof.to_bytes();
        let Some(beta) = Option::<[u8; 64]>::from(pk.verify(&alpha, &proof)) else {
            return Err(EngineError::InvalidOrder(
                "internal Eval VRF self-verification failed".to_owned(),
            ));
        };
        let mut beacon = [0u8; 32];
        beacon.copy_from_slice(&beta[..32]);
        Ok((beacon, proof_bytes))
    }
}

/// Canonical domain-separated VRF input message `alpha_t` for `(table_id, turn)`.
#[must_use]
pub fn eval_vrf_alpha(table_id: TableId, turn: Turn) -> [u8; 33] {
    let mut alpha = [0u8; 33];
    alpha[..24].copy_from_slice(b"league-core/eval-vrf/v1|");
    alpha[24..32].copy_from_slice(&table_id.0.to_le_bytes());
    alpha[32] = turn.0;
    alpha
}

/// Validates `eval_vrf_pubkey` (`validate_key = TRUE` per the RFC 9381 §5.4.5 ECVRF construction
/// over C2SP's `ristretto255` ciphersuite).
pub fn validate_eval_vrf_pubkey(
    eval_vrf_pubkey: [u8; EVAL_VRF_PUBKEY_BYTES],
) -> Result<(), EngineError> {
    if vrf_r255::PublicKey::from_bytes(eval_vrf_pubkey).is_none() {
        return Err(EngineError::InvalidConfig(
            "invalid or identity Eval VRF ristretto255 public key".to_owned(),
        ));
    }
    Ok(())
}

/// Verifies an `EVAL_VRF_PROOF_BYTES` proof under the RFC 9381 ECVRF construction with C2SP's
/// `ristretto255` ciphersuite (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not registered in RFC 9381)
/// against `eval_vrf_pubkey` for `(table_id, turn)` and extracts the unique 32-byte `turn_beacon`.
pub fn verify_and_extract_eval_vrf_beacon(
    eval_vrf_pubkey: [u8; EVAL_VRF_PUBKEY_BYTES],
    table_id: TableId,
    turn: Turn,
    vrf_proof: &[u8],
) -> Result<[u8; 32], EngineError> {
    if turn.0 == 0 {
        return Err(EngineError::InvalidOrder(
            "Eval VRF turn must be >= 1".to_owned(),
        ));
    }
    let pk = vrf_r255::PublicKey::from_bytes(eval_vrf_pubkey).ok_or_else(|| {
        EngineError::InvalidConfig("invalid Eval VRF ristretto255 public key".to_owned())
    })?;
    let proof_arr: [u8; EVAL_VRF_PROOF_BYTES] = vrf_proof.try_into().map_err(|_| {
        EngineError::InvalidOrder(format!(
            "Eval VRF proof must be {EVAL_VRF_PROOF_BYTES} bytes, got {}",
            vrf_proof.len()
        ))
    })?;
    let proof = vrf_r255::Proof::from_bytes(proof_arr).ok_or_else(|| {
        EngineError::InvalidOrder("malformed or non-canonical Eval VRF proof".to_owned())
    })?;
    let alpha = eval_vrf_alpha(table_id, turn);
    let beta: [u8; 64] = Option::from(pk.verify(&alpha, &proof)).ok_or_else(|| {
        EngineError::InvalidOrder(format!(
            "turn {} Eval VRF proof failed verification against eval_vrf_pubkey",
            turn.0
        ))
    })?;
    let mut beacon = [0u8; 32];
    beacon.copy_from_slice(&beta[..32]);
    Ok(beacon)
}

/// Verifies an Eval-mode VRF proof and confirms the derived beacon equals `expected_beacon`.
pub fn verify_eval_vrf_beacon(
    eval_vrf_pubkey: [u8; EVAL_VRF_PUBKEY_BYTES],
    table_id: TableId,
    turn: Turn,
    vrf_proof: &[u8],
    expected_beacon: [u8; 32],
) -> Result<(), EngineError> {
    let derived = verify_and_extract_eval_vrf_beacon(eval_vrf_pubkey, table_id, turn, vrf_proof)?;
    if derived != expected_beacon {
        return Err(EngineError::InvalidOrder(format!(
            "turn {} turn_beacon does not match verified Eval VRF proof output",
            turn.0
        )));
    }
    Ok(())
}
