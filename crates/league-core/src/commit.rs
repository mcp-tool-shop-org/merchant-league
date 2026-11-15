//! Cryptographic commit-reveal envelopes, sealed turn bundles, private charter mandates,
//! and typed diplomatic intent pledges (never storing raw chat prose).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::types::{
    AssetId, BPS_SCALE, Bps, CharterRole, CreditTier, EngineError, MAX_MANDATE_BONUS_EQUITY,
    MIN_MANDATE_BONUS_EQUITY, Seat, TableId, Turn, Units,
};

mod hex32 {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(deserializer)?;
        let decoded = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if decoded.len() != 32 {
            return Err(serde::de::Error::custom("expected 32 hex-encoded bytes"));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&decoded);
        Ok(out)
    }
}

/// Secret 32-byte salt used inside Phase 2 commit-reveal bundles and Turn-1 mandate commits.
///
/// Redacts its raw bytes in `Debug` output (`CLAUDE.md` invariant).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealSalt(#[serde(with = "hex32")] pub [u8; 32]);

impl fmt::Debug for RevealSalt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RevealSalt(<hidden>)")
    }
}

/// Secret 32-byte preimage unlocking a `PREIMAGE-SHA-256` [`crate::contracts::TokenEscrowVault`].
///
/// Redacts its raw bytes in `Debug` output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreimageSecret(#[serde(with = "hex32")] pub [u8; 32]);

impl fmt::Debug for PreimageSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreimageSecret(<hidden>)")
    }
}

/// 32-byte SHA-256 commitment digest (`PREIMAGE-SHA-256` condition or order/mandate seal).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommitmentHash(#[serde(with = "hex32")] pub [u8; 32]);

impl fmt::Debug for CommitmentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitmentHash({})", hex::encode(self.0))
    }
}

impl fmt::Display for CommitmentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

/// Tier-2 converter execution order submitted in a sealed Phase 2 bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConverterRunOrder {
    /// Index into the seat's `converters` recipe vector.
    pub recipe_idx: usize,
    /// Requested number of atomic runs (`BatchV1_1` `tfAllOrNothing`).
    pub runs: u32,
}

/// Tier-3 Syndicate Project delivery order burning refined goods for Project Equity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDeliveryOrder {
    /// Target [`crate::types::ProjectRecipe`] ID.
    pub project_id: u16,
    /// Number of project batches delivered.
    pub batches: u32,
}

/// Symmetric two-party atomic swap proposal (`BatchV1_1` `tfAllOrNothing`).
///
/// Settles at Sweep Step 2 when both `maker` and `taker` include identical proposals in their
/// revealed Phase 2 bundles.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AtomicSwapProposal {
    /// Deterministic client-chosen proposal identifier matching between counterparties.
    pub swap_id: u64,
    /// Maker seat sending `maker_asset`.
    pub maker: u8,
    /// Taker seat sending `taker_asset`.
    pub taker: u8,
    /// Asset delivered by `maker` to `taker`.
    pub maker_asset: AssetId,
    /// Gross units sent by `maker`.
    pub maker_amount: Units,
    /// Asset delivered by `taker` to `maker`.
    pub taker_asset: AssetId,
    /// Gross units sent by `taker`.
    pub taker_amount: Units,
    /// Optional shared cartel domain ID waiving export tariffs if both seats belong.
    pub domain_id: Option<u8>,
}

/// Phase 2 `EscrowFinish` reveal unlocking an active `TokenEscrowVault`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowFinishUnlock {
    /// Identifier of the target escrow vault.
    pub escrow_id: u64,
    /// Secret preimage when the escrow requires a `PREIMAGE-SHA-256` condition.
    pub preimage: Option<PreimageSecret>,
}

/// Public council platform filed ahead of an election turn (`StandForOffice`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilPlatform {
    /// Preferred export tariff policy changes `(seat, (raw_bps, refined_bps))` enacted if elected Trade Minister.
    pub proposed_tariffs_bps: Vec<(u8, (Bps, Bps))>,
    /// Preferred domain admissions `(domain_id, seat)` enacted (founding new domains when needed) if elected Cartel Chair.
    pub cartel_admissions: Vec<(u8, u8)>,
    /// Preferred domain credential deletions `(domain_id, seat)` enacted if elected Cartel Chair.
    pub cartel_revocations: Vec<(u8, u8)>,
}

/// Sealed council election ballot submitted on turns divisible by `4`.
///
/// Tariffs and cartel membership changes come from the winner's filed [`CouncilPlatform`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilVoteBallot {
    /// Preferred seat index for Trade Minister (`0..seat_count`).
    pub trade_minister: u8,
    /// Preferred seat index for Cartel Chair (`0..seat_count`).
    pub cartel_chair: u8,
}

/// Sealed equity bid for a 6-turn Syndicate Charter on turns divisible by `6`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharterBidOrder {
    /// Charter role bid on.
    pub role: CharterRole,
    /// `CRED` bid escrowed on Sweep if the seat wins the slot.
    pub bid_cred: Units,
}

/// Simultaneous discretionary actions committed in Phase 2 (`Commit`) and disclosed in `Reveal`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedOrderBundle {
    /// Tier-2 converter batches executed via `BatchV1_1`.
    pub converter_runs: Vec<ConverterRunOrder>,
    /// Tier-3 Syndicate Project bundles delivered.
    pub project_deliveries: Vec<ProjectDeliveryOrder>,
    /// Co-signed atomic bilateral swaps (`tfAllOrNothing`).
    pub atomic_swaps: Vec<AtomicSwapProposal>,
    /// Voluntary repayment toward outstanding `CRED` auto-debt principal.
    pub debt_repayment_cred: Units,
    /// Conditional `TokenEscrow` finish reveals (`EscrowFinish`).
    pub escrow_finishes: Vec<EscrowFinishUnlock>,
    /// Maturing `CheckCreate` IDs presented for `CheckCash` clearing at Sweep Step 3.
    pub check_cashes: Vec<u64>,
    /// Simultaneous council election ballot (effective on `turn % 4 == 0`).
    pub council_vote: Option<CouncilVoteBallot>,
    /// Sealed-bid Syndicate Charter offers (effective on `turn % 6 == 0`).
    pub charter_bids: Vec<CharterBidOrder>,
}

/// Revealed Phase 2 bundle plus its secret 32-byte salt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderReveal {
    /// Action orders executed at Sweep if reveal matches commitment.
    pub bundle: SealedOrderBundle,
    /// Secret salt committed in Phase 2.
    pub salt: RevealSalt,
}

/// Verifiable breakable pledge target for Track-2 Soft Diplomatic Honor auditing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentTarget {
    /// Pledge to cast council vote in `office_turn` for `candidate`.
    VoteForSeat {
        /// Election turn (`4, 8, 12, 16, 20, 24`).
        office_turn: u8,
        /// Promised candidate seat (`trade_minister` or `cartel_chair`).
        candidate: u8,
    },
    /// Pledge not to revoke `beneficiary` from `domain_id` on `office_turn`.
    PreserveCartelMembership {
        /// Election turn (`4, 8, 12, 16, 20, 24`).
        office_turn: u8,
        /// Target cartel domain ID.
        domain_id: u8,
        /// Beneficiary seat protected by the pledge.
        beneficiary: u8,
    },
    /// Pledge to reveal the preimage condition for `escrow_id` before its expiration.
    RevealPreimage {
        /// Escrow ID tied to the preimage.
        escrow_id: u64,
        /// Condition digest expected upon finish.
        condition: CommitmentHash,
    },
}

/// Signed Typed JSON Intent Header anchored alongside a memo digest (never raw chat text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentPledge {
    /// Client/table-scoped pledge ID.
    pub pledge_id: u64,
    /// Promisor seat making the Track-2 pledge.
    pub promisor: u8,
    /// Counterparty seat receiving the pledge.
    pub counterparty: u8,
    /// Auditable promise target.
    pub target: IntentTarget,
    /// Stake weight in equity points used to score betrayal penalties if broken.
    pub honor_weight: Units,
    /// Salted SHA-256 memo digest referencing off-ledger free-text conversation.
    #[serde(with = "hex32")]
    pub memo_digest: [u8; 32],
}

/// Turn-1 Private Charter Mandate endgame objective variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MandateKind {
    /// Bonus awarded if final irreversible `Project Equity >= target`.
    MinProjectEquity {
        /// Minimum Project Equity threshold.
        target: Units,
    },
    /// Bonus awarded if seat ends season holding at least `distinct_goods` refined types each `>= min_each`.
    MinRefinedVariety {
        /// Required count of distinct refined goods held at end.
        distinct_goods: u8,
        /// Minimum units per counted refined good.
        min_each: Units,
    },
    /// Bonus awarded if credit rating ends at or above `min_tier`.
    MaintainCleanCredit {
        /// Required terminal credit tier (ordered `Aaa < Bbb < B < D`).
        min_tier: CreditTier,
    },
    /// Bonus awarded if seat recorded positive settled deliveries with at least `min_partners` distinct counterparties.
    MinActivePartners {
        /// Minimum distinct counterparties.
        min_partners: u8,
    },
}

/// Secret Turn-1 Charter Mandate dealt to a seat and revealed upon Season Close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateMandate {
    /// Objective rule evaluated at terminal Turn `T`.
    pub kind: MandateKind,
    /// Irreversible equity bonus granted when objective holds and commitment reveals validly.
    pub bonus_equity: Units,
}

/// Revealed Turn-1 Private Charter Mandate plus its secret 32-byte salt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateReveal {
    /// Mandate terms evaluated at close.
    pub mandate: PrivateMandate,
    /// Secret salt committed at Turn 1.
    pub salt: RevealSalt,
}

/// Card in a table's public Turn-1 Private Mandate deck with calibrated difficulty and bonus (`3–8 VP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradedMandateCard {
    /// Unique card identifier within the table deck.
    pub card_id: u16,
    /// Objective rule evaluated at terminal Turn `T`.
    pub kind: MandateKind,
    /// Playtest completion rate (`0..=10_000 bps`) used to grade card difficulty.
    pub difficulty_completion_bps: Bps,
    /// Irreversible equity bonus (`3_000..=8_000`, i.e., `3–8 VP`) granted upon completion.
    pub bonus_equity: Units,
}

/// Maps empirical completion rate (`0..=10_000 bps`) monotonically into `3–8 VP` (`3_000..=8_000`).
#[must_use]
pub fn bonus_equity_for_completion_bps(completion_bps: Bps) -> Units {
    let clamped = i64::from(completion_bps.min(10_000));
    let hard_bps = (BPS_SCALE as i64) - clamped;
    MIN_MANDATE_BONUS_EQUITY
        + ((MAX_MANDATE_BONUS_EQUITY - MIN_MANDATE_BONUS_EQUITY) * hard_bps) / (BPS_SCALE as i64)
}

/// Builds the canonical 8-card graded mandate deck for `seat_count` (`4..=6`), covering all
/// [`MandateKind`] variants with monopoly-safe thresholds (`max_safe = seat_count - 2`) and
/// bonuses spanning `3_000..=8_000` (`3–8 VP`).
#[must_use]
pub fn default_graded_mandate_deck(seat_count: u8) -> Vec<GradedMandateCard> {
    let max_safe = seat_count.saturating_sub(2).max(1);
    let specs = [
        (
            1,
            MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Bbb,
            },
            10_000,
        ),
        (2, MandateKind::MinProjectEquity { target: 16_000 }, 8_000),
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
        (7, MandateKind::MinProjectEquity { target: 28_000 }, 1_500),
        (8, MandateKind::MinProjectEquity { target: 36_000 }, 0),
    ];
    specs
        .into_iter()
        .map(
            |(card_id, kind, difficulty_completion_bps)| GradedMandateCard {
                card_id,
                kind,
                difficulty_completion_bps,
                bonus_equity: bonus_equity_for_completion_bps(difficulty_completion_bps),
            },
        )
        .collect()
}

/// Deterministically shuffles `deck` and deals one [`MandateReveal`] and [`CommitmentHash`] per seat.
pub fn deal_graded_mandates(
    table_id: TableId,
    seat_count: u8,
    seed: u64,
    deck: &[GradedMandateCard],
) -> Result<(Vec<CommitmentHash>, Vec<MandateReveal>), EngineError> {
    let n = usize::from(seat_count);
    if n == 0 || deck.len() < n {
        return Err(EngineError::InvalidConfig(format!(
            "mandate deck ({}) smaller than seat_count ({seat_count})",
            deck.len()
        )));
    }
    let mut indices: Vec<usize> = (0..deck.len()).collect();
    for i in (1..indices.len()).rev() {
        let mut hasher = Sha256::new();
        hasher.update(b"league-core/mandate-deal/v2");
        hasher.update(seed.to_le_bytes());
        hasher.update(table_id.0.to_le_bytes());
        hasher.update((i as u32).to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut le = [0u8; 8];
        le.copy_from_slice(&digest[0..8]);
        let j = (u64::from_le_bytes(le) as usize) % (i + 1);
        indices.swap(i, j);
    }

    let mut commits = Vec::with_capacity(n);
    let mut reveals = Vec::with_capacity(n);
    for (seat_idx, &card_idx) in indices.iter().take(n).enumerate() {
        let card = deck[card_idx];
        let mut hasher = Sha256::new();
        hasher.update(b"league-core/mandate-salt/v2");
        hasher.update(seed.to_le_bytes());
        hasher.update(table_id.0.to_le_bytes());
        hasher.update([seat_idx as u8]);
        let mut salt: [u8; 32] = hasher.finalize().into();
        if salt.iter().all(|&b| b == 0) {
            salt[0] = 1;
        }
        let reveal = MandateReveal {
            mandate: PrivateMandate {
                kind: card.kind,
                bonus_equity: card.bonus_equity,
            },
            salt: RevealSalt(salt),
        };
        let commit = compute_mandate_commitment(table_id, Seat(seat_idx as u8), &reveal)?;
        commits.push(commit);
        reveals.push(reveal);
    }
    Ok((commits, reveals))
}

/// Computes `SHA-256("league-core/preimage/v2" || secret)` for conditional `TokenEscrow`.
#[must_use]
pub fn compute_preimage_condition(secret: &PreimageSecret) -> CommitmentHash {
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/preimage/v2");
    hasher.update(secret.0);
    CommitmentHash(hasher.finalize().into())
}

/// Computes canonical Phase 2 SHA-256 commitment for `table_id`, `turn`, `seat`, and `reveal`.
pub fn compute_order_commitment(
    table_id: TableId,
    turn: Turn,
    seat: Seat,
    reveal: &OrderReveal,
) -> Result<CommitmentHash, EngineError> {
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
    Ok(compute_order_commitment(table_id, turn, seat, reveal)? == commitment)
}

/// Computes canonical Turn-1 Private Mandate SHA-256 commitment.
pub fn compute_mandate_commitment(
    table_id: TableId,
    seat: Seat,
    reveal: &MandateReveal,
) -> Result<CommitmentHash, EngineError> {
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

/// Maps a 32-byte beacon uniformly into basis points `[0, 10_000)` for stochastic close checks.
#[must_use]
pub fn beacon_roll_bps(beacon: [u8; 32]) -> Bps {
    let mut le = [0u8; 8];
    le.copy_from_slice(&beacon[0..8]);
    (u64::from_le_bytes(le) % 10_000) as Bps
}

/// Deterministically breaks an election plurality tie among `tied` seat indices using `turn_beacon`.
#[must_use]
pub fn break_tie_with_beacon(beacon: [u8; 32], domain_tag: &[u8], tied: &[u8]) -> u8 {
    match tied {
        [] => 0,
        [only] => *only,
        _ => {
            let mut hasher = Sha256::new();
            hasher.update(b"league-core/tie-break/v2");
            hasher.update(domain_tag);
            hasher.update(beacon);
            let digest: [u8; 32] = hasher.finalize().into();
            let mut le = [0u8; 8];
            le.copy_from_slice(&digest[0..8]);
            tied[(u64::from_le_bytes(le) as usize) % tied.len()]
        }
    }
}
