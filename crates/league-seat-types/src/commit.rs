use alloc::borrow::ToOwned;
use alloc::format;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::serde_helpers::{hex_encode_32, hex32};
use crate::types::{AssetId, Bps, CharterRole, CreditTier, EngineError, Turn, Units};

/// League of Entropy drand `quicknet` genesis Unix timestamp (`2023-08-23T15:09:27Z`, round `1`).
pub const DRAND_QUICKNET_GENESIS_UNIX: u64 = 1_692_803_367;

/// League of Entropy drand `quicknet` round period in seconds (`3s`).
pub const DRAND_QUICKNET_PERIOD_SECS: u64 = 3;

/// Secret 32-byte preimage unlocking a `PREIMAGE-SHA-256` [`crate::TokenEscrowVault`].
///
/// Redacts its raw bytes in `Debug` output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreimageSecret(#[serde(with = "hex32")] pub [u8; 32]);

impl PreimageSecret {
    /// Returns `true` when all 32 bytes are zero (degenerate preimage forbidden in escrows).
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Validates that the preimage is not all-zero (`[0u8; 32]`).
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.is_degenerate() {
            return Err(EngineError::InvalidOrder(
                "degenerate all-zero PreimageSecret is forbidden".to_owned(),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for PreimageSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreimageSecret(<hidden>)")
    }
}

/// 32-byte SHA-256 commitment digest (`PREIMAGE-SHA-256` condition or order/mandate seal).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommitmentHash(#[serde(with = "hex32")] pub [u8; 32]);

impl CommitmentHash {
    /// Returns `true` when all 32 bytes are zero.
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl fmt::Debug for CommitmentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitmentHash({})", hex_encode_32(&self.0))
    }
}

impl fmt::Display for CommitmentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex_encode_32(&self.0))
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
    /// Target [`crate::ProjectRecipe`] ID.
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

/// Sealed Phase-2 bilateral commodity delivery executed cleanly at Step 2 (`BatchV1_1`
/// one-way delivery), enabling Seller fulfillment of `BuyerDvpPayment` or `DualEscrowVault`
/// contracts without requiring Buyer to lock a second payment inside an `AtomicSwapProposal`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContractDeliveryOrder {
    /// Receiving buyer seat index.
    pub counterparty: u8,
    /// Commodity transferred (`Raw(i)` or `Refined(i)`).
    pub good: AssetId,
    /// Gross units debited from delivering seat (`> 0`).
    pub gross_amount: Units,
    /// Optional shared cartel domain ID waiving export tariffs.
    pub domain_id: Option<u8>,
    /// When `Some(id)`, ties delivery atomically to `DualEscrowVault { escrow_id: id }` (`C2`).
    #[serde(default)]
    pub dual_escrow_id: Option<u64>,
}

/// Co-signed dual delivery-versus-payment escrow proposal settled atomically in Step 2 when
/// both `buyer` and `seller` include identical terms in their revealed bundles (`C2`, `2026-11-15`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DualEscrowProposal {
    /// Deterministic proposal identifier matching between buyer and seller.
    pub proposal_id: u64,
    /// Buyer seat locking `buyer_bond_cred`.
    pub buyer: u8,
    /// Seller seat locking `seller_bond_cred`.
    pub seller: u8,
    /// Delivered commodity (`Raw(i)` or `Refined(i)`).
    pub good: AssetId,
    /// Minimum net units required on or before `deadline_turn`.
    pub required_units: Units,
    /// Full purchase payment in milli-`CRED` released on delivery.
    pub payment_cred: Units,
    /// Buyer locked `CRED` (`1..=payment_cred`).
    pub buyer_bond_cred: Units,
    /// Seller performance bond (`1..=payment_cred`), with `buyer_bond_cred + seller_bond_cred <= payment_cred`.
    pub seller_bond_cred: Units,
    /// Compliance evaluation deadline (`>= current_turn && <= HARD_TURN_CAP`).
    pub deadline_turn: u8,
    /// Optional cartel domain ID for tender tariff relief.
    pub domain_id: Option<u8>,
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
    /// Direct contractual commodity deliveries for `BuyerDvpPayment` / `DualEscrowVault` (`C2`).
    #[serde(default)]
    pub contract_deliveries: Vec<ContractDeliveryOrder>,
    /// Co-signed dual-escrow DvP contract openings (`C2`).
    #[serde(default)]
    pub dual_escrows: Vec<DualEscrowProposal>,
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

/// Verifiable breakable pledge target for Track-2 Soft Diplomatic Honor auditing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentTarget {
    /// Pledge to cast council vote in `office_turn` for `candidate`.
    VoteForSeat {
        /// Election turn (`4, 8, 12, 16, 20, 24, 28, 32`).
        office_turn: u8,
        /// Promised candidate seat (`trade_minister` or `cartel_chair`).
        candidate: u8,
    },
    /// Pledge not to revoke `beneficiary` from `domain_id` on `office_turn`.
    PreserveCartelMembership {
        /// Election turn (`4, 8, 12, 16, 20, 24, 28, 32`).
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

impl MandateKind {
    /// Validates monopoly-independent bounds (`distinct_goods`, `min_partners <= seat_count - 2`)
    /// so a single withholding seat cannot make a mandate unachievable.
    pub fn validate_for_seat_count(&self, seat_count: u8) -> Result<(), EngineError> {
        let max_threshold = seat_count.saturating_sub(2);
        if max_threshold == 0 {
            return Err(EngineError::InvalidConfig(format!(
                "seat_count must be >= 3 to bound mandate thresholds, got {seat_count}"
            )));
        }
        match *self {
            Self::MinProjectEquity { target } => {
                if target <= 0 {
                    return Err(EngineError::InvalidConfig(format!(
                        "MinProjectEquity target must be positive, got {target}"
                    )));
                }
            }
            Self::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => {
                if !(1..=max_threshold).contains(&distinct_goods) || min_each <= 0 {
                    return Err(EngineError::InvalidConfig(format!(
                        "MinRefinedVariety threshold invalid for {seat_count} seats: distinct_goods={distinct_goods} (must be 1..={max_threshold}), min_each={min_each}"
                    )));
                }
            }
            Self::MaintainCleanCredit { min_tier } => {
                if !matches!(min_tier, CreditTier::Aaa | CreditTier::Bbb) {
                    return Err(EngineError::InvalidConfig(format!(
                        "MaintainCleanCredit min_tier {min_tier:?} must be Aaa or Bbb"
                    )));
                }
            }
            Self::MinActivePartners { min_partners } => {
                if !(1..=max_threshold).contains(&min_partners) {
                    return Err(EngineError::InvalidConfig(format!(
                        "MinActivePartners min_partners={min_partners} out of bounds for {seat_count} seats (must be 1..={max_threshold})"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Secret Turn-1 Charter Mandate dealt to a seat and revealed upon Season Close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateMandate {
    /// Objective rule evaluated at terminal Turn `T`.
    pub kind: MandateKind,
    /// Irreversible equity bonus granted when objective holds and commitment reveals validly.
    pub bonus_equity: Units,
}

/// Card in a table's public Turn-1 Private Mandate deck with calibrated difficulty and bonus (`3–8 VP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradedMandateCard {
    /// Unique card identifier within the table deck.
    pub card_id: u16,
    /// Objective rule evaluated at terminal Turn `T`.
    pub kind: MandateKind,
    /// Uncalibrated placeholder difficulty estimate in basis points (`0..=10_000`) used to
    /// size provisional completion bonuses until empirical playtest calibration replaces these values.
    pub difficulty_completion_bps: Bps,
    /// Irreversible equity bonus (`3_000..=8_000`, i.e., `3–8 VP`) granted upon completion.
    pub bonus_equity: Units,
}

impl GradedMandateCard {
    /// Returns the provisional mandate point value in milli-points (`3_000..=8_000 == 3..=8 pts`),
    /// kept distinct from terminal net-worth scaling.
    #[must_use]
    pub const fn mandate_points_milli(&self) -> Units {
        self.bonus_equity
    }
}

/// Pre-named turn close schedule used in `ExternalDrand` tables so every turn's round number is
/// fixed deterministically before cards are dealt (`design-v2` §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrandSchedule {
    /// Scheduled Unix timestamp (UTC seconds) at which Turn 1 closes.
    pub turn_1_close_unix: u64,
    /// Seconds elapsed between consecutive turn close timestamps (must be `>= 1`).
    pub turn_period_secs: u64,
}

impl DrandSchedule {
    /// Validates that the schedule lies on or after drand `quicknet` genesis and advances at least
    /// one `quicknet` period per turn.
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.turn_period_secs < DRAND_QUICKNET_PERIOD_SECS {
            return Err(EngineError::InvalidConfig(format!(
                "DrandSchedule turn_period_secs must be >= {DRAND_QUICKNET_PERIOD_SECS}, got {}",
                self.turn_period_secs
            )));
        }
        if self.turn_1_close_unix < DRAND_QUICKNET_GENESIS_UNIX {
            return Err(EngineError::InvalidConfig(format!(
                "DrandSchedule turn_1_close_unix ({}) precedes quicknet genesis ({DRAND_QUICKNET_GENESIS_UNIX})",
                self.turn_1_close_unix
            )));
        }
        let _ = self.close_unix_for_turn(Turn(u8::MAX))?;
        Ok(())
    }

    /// Computes the scheduled close Unix timestamp for `turn` (`>= 1`).
    pub fn close_unix_for_turn(&self, turn: Turn) -> Result<u64, EngineError> {
        if turn.0 == 0 {
            return Err(EngineError::InvalidConfig(
                "turn must be >= 1 for DrandSchedule".to_owned(),
            ));
        }
        let delta = u64::from(turn.0 - 1)
            .checked_mul(self.turn_period_secs)
            .ok_or_else(|| {
                EngineError::InvalidConfig("DrandSchedule timestamp overflow".to_owned())
            })?;
        self.turn_1_close_unix.checked_add(delta).ok_or_else(|| {
            EngineError::InvalidConfig("DrandSchedule timestamp overflow".to_owned())
        })
    }

    /// Returns the expected drand `quicknet` round for `turn` per `design-v2` §6.1.
    pub fn expected_round_for_turn(&self, turn: Turn) -> Result<u64, EngineError> {
        let close_unix = self.close_unix_for_turn(turn)?;
        drand_quicknet_round_for_close(close_unix)
    }
}

/// Maps a Unix close timestamp (seconds) to its League of Entropy drand `quicknet` round number
/// per `design-v2` §6.1: `floor((close - 1692803367) / 3) + 1`.
pub fn drand_quicknet_round_for_close(close_unix_secs: u64) -> Result<u64, EngineError> {
    if close_unix_secs < DRAND_QUICKNET_GENESIS_UNIX {
        return Err(EngineError::InvalidOrder(format!(
            "close timestamp {close_unix_secs} precedes drand quicknet genesis {DRAND_QUICKNET_GENESIS_UNIX}"
        )));
    }
    Ok((close_unix_secs - DRAND_QUICKNET_GENESIS_UNIX) / DRAND_QUICKNET_PERIOD_SECS + 1)
}
