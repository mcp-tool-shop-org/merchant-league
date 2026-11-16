//! Dual-track contract primitives (`BatchV1_1` atomic swaps, `CRED`-priced DEX books,
//! `TokenEscrow` collateral vaults and Steward bonds, breakable `CheckCreate` notes,
//! cartel `PermissionedDomains`, and 6-turn Syndicate Charters).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::commit::{CommitmentHash, CouncilPlatform, IntentPledge};
use crate::types::{AssetId, Bps, CharterRole, CreditTier, EngineError, Units};

/// Maximum number of distinct `PermissionedDomain` cartel zones allowed on a table.
pub const MAX_CARTEL_DOMAINS: usize = 16;

/// Active cartel trade domain isolating zero-tariff insider order books (`PermissionedDomain`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartelDomain {
    /// Domain identifier (`0..`).
    pub domain_id: u8,
    /// Seat indices holding active domain access `Credentials`.
    pub members: BTreeSet<u8>,
}

/// Active time-bounded Syndicate Charter awarded in sealed equity auctions every 6 turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveCharter {
    /// Institutional role held.
    pub role: CharterRole,
    /// Authorized holder seat index.
    pub holder: u8,
    /// Turn when the charter expires (`granted_turn + 6`).
    pub expires_at_turn: u8,
}

/// Lifecycle state of an uncollateralized Track-2 [`CreditCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    /// Open awaiting presentation via `CheckCash` on or after `due_turn`.
    Open,
    /// Successfully settled at Sweep Step 3 (`amount_transferred` credited).
    Cashed(Units),
    /// Bounced on insufficient free liquidity (`HardCreditDefault` recorded on drawer).
    Bounced,
    /// Cancelled before cashing or pruned after `expiration_turn`.
    Cancelled,
}

/// Track-2 uncollateralized deferred credit note (`CheckCreate`, settled via `CheckCash`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditCheck {
    /// Deterministic table-assigned check identifier (`1..`).
    pub check_id: u64,
    /// Seat drawing/owing the note.
    pub drawer: u8,
    /// Authorized payee seat.
    pub payee: u8,
    /// Promised asset (`CRED` or commodity).
    pub asset: AssetId,
    /// Maximum deliverable amount.
    pub send_max: Units,
    /// Minimum acceptable delivery floor (`DeliverMin`); below this triggers a Hard Credit Default.
    pub deliver_min: Units,
    /// Earliest turn when `CheckCash` may clear.
    pub due_turn: u8,
    /// Turn at which uncashed note expires.
    pub expiration_turn: u8,
    /// Current lifecycle state.
    pub status: CheckStatus,
}

/// Lifecycle state of a [`TokenEscrowVault`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EscrowStatus {
    /// Non-perishable `CRED` collateral is currently locked in escrow.
    Locked,
    /// Released via preimage reveal (`EscrowFinish`) or compliant Steward bond evaluation.
    Finished,
    /// Returned to `sender` on timeout cancellation, or forfeited on bond breach.
    Cancelled,
}

/// Settlement rule governing a [`BondSpec`] attached to a `TokenEscrowVault`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BondSettlementMode {
    /// Legacy unilateral seller bond: seller locks `amount_cred`, recovers it at `deadline_turn`
    /// upon delivery, or forfeits to buyer (`destination`) on shortfall.
    #[default]
    SellerPerformanceBond,
    /// Self-liquidating Buyer delivery-versus-payment escrow (`C2`, `2026-11-15`): buyer locks
    /// `100%` of payment `CRED` naming seller (`destination`) as `obligor`; released immediately to
    /// seller on delivery or refunded to buyer at `deadline_turn` shortfall without a `200%` lock.
    BuyerDvpPayment,
}

/// Steward-refereed physical delivery bond attached to a `TokenEscrow` collateral vault.
///
/// Replayed deterministically from cumulative recorded bilateral deliveries without discretion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BondSpec {
    /// Obligated delivering seat (`sender` for seller bonds, `destination` for `BuyerDvpPayment`).
    pub obligor: u8,
    /// Counterparty entitled to receive `required_units` of `good`.
    pub beneficiary: u8,
    /// Promised delivery asset.
    pub good: AssetId,
    /// Minimum cumulative deliveries recorded from `created_turn` through `deadline_turn`.
    pub required_units: Units,
    /// Baseline bilateral deliveries recorded at the moment the bond was posted.
    pub baseline_delivered: Units,
    /// Turn at which Steward evaluates compliance at Sweep Step 2.
    pub deadline_turn: u8,
    /// Settlement mode (`SellerPerformanceBond` vs self-liquidating `BuyerDvpPayment`).
    #[serde(default)]
    pub mode: BondSettlementMode,
}

/// Co-signed dual delivery-versus-payment escrow created atomically when both `buyer` and
/// `seller` sign identical terms (`C2`, `2026-11-15`).
///
/// Buyer locks `buyer_bond_cred <= payment_cred` (`100%` full prepay or earnest `< 100%`), and
/// Seller locks `seller_bond_cred <= payment_cred` with combined lock strictly `< 200%` of
/// `payment_cred`. Compliant delivery releases `payment_cred + seller_bond_cred` to Seller
/// immediately; Seller non-delivery forfeits `seller_bond_cred` to Buyer; Buyer non-payment on
/// tender forfeits `buyer_bond_cred` to Seller while leaving Seller's commodity untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DualEscrowVault {
    /// Deterministic table-assigned escrow ID shared with `TokenEscrowVault` IDs (`1..`).
    pub escrow_id: u64,
    /// Co-signing buyer seat locking `buyer_bond_cred`.
    pub buyer: u8,
    /// Co-signing seller seat locking `seller_bond_cred`.
    pub seller: u8,
    /// Promised physical commodity (`Raw(i)` or `Refined(i)`).
    pub good: AssetId,
    /// Minimum incremental net units required by `deadline_turn`.
    pub required_units: Units,
    /// Full purchase payment in milli-`CRED` released to `seller` upon delivery.
    pub payment_cred: Units,
    /// Upfront `CRED` locked by `buyer` (`1..=payment_cred`).
    pub buyer_bond_cred: Units,
    /// Upfront performance bond locked by `seller`.
    pub seller_bond_cred: Units,
    /// Baseline bilateral `(seller, buyer, good)` deliveries recorded at creation.
    pub baseline_delivered: Units,
    /// Compliance evaluation deadline turn (`<= 28`).
    pub deadline_turn: u8,
    /// Optional shared cartel `DomainID` waiving bilateral export tariffs.
    pub domain_id: Option<u8>,
    /// Current lifecycle state.
    pub status: EscrowStatus,
}

/// Track-1 collateralized conditional `TokenEscrow` vault (`XLS-85`, non-perishable `CRED` only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenEscrowVault {
    /// Deterministic table-assigned escrow ID (`1..`).
    pub escrow_id: u64,
    /// Funding seat locking `amount_cred`.
    pub sender: u8,
    /// Recipient on clean conditional finish (or bond forfeiture).
    pub destination: u8,
    /// Locked non-perishable `CRED` amount (`> 0`).
    pub amount_cred: Units,
    /// Optional earliest finish turn (`FinishAfter`).
    pub finish_after_turn: Option<u8>,
    /// Mandatory expiry turn (`CancelAfter > created_turn`).
    pub cancel_after_turn: u8,
    /// Optional `PREIMAGE-SHA-256` unlock condition hash.
    pub condition_hash: Option<CommitmentHash>,
    /// Optional deterministic Steward-refereed delivery bond.
    pub bond: Option<BondSpec>,
    /// Current vault state.
    pub status: EscrowStatus,
}

/// Direction of a [`CredOffer`] on the `CRED`-denominated DEX order book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OfferSide {
    /// Maker buys commodity using locked `CRED`.
    Buy,
    /// Maker sells commodity locking `good` units.
    Sell,
}

/// Resting `CRED`-priced DEX limit order (`OfferCreate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredOffer {
    /// Deterministic order ID (`1..`).
    pub offer_id: u64,
    /// Maker seat index.
    pub maker: u8,
    /// Side (`Buy` or `Sell`).
    pub side: OfferSide,
    /// Traded commodity (`Raw(i)` or `Refined(i)`; never `Cred`).
    pub good: AssetId,
    /// Unfilled commodity quantity (`> 0`).
    pub remaining_units: Units,
    /// Limit price in milli-`CRED` per `1_000` commodity units (`> 0`).
    pub price_per_unit_cred: Units,
    /// Exact locked `CRED` remaining on a resting `Buy` offer (`0` on `Sell` offers).
    pub locked_cred: Units,
    /// Optional cartel `DomainID` scoping execution book and waiving tariffs inside the domain.
    pub domain_id: Option<u8>,
    /// Whether this order was tagged `tfHybrid` by an active `ClearinghouseBroker`.
    pub is_hybrid: bool,
    /// Turn when order entered the book.
    pub created_turn: u8,
}

/// Parameter bundle for creating an uncollateralized Track-2 `CheckCreate` credit note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckProposal {
    /// Counterparty receiving payment rights.
    pub payee: u8,
    /// Promised currency or commodity.
    pub asset: AssetId,
    /// Maximum face value (`> 0`).
    pub send_max: Units,
    /// Minimum deliverable units before bouncing (`1..=send_max`).
    pub deliver_min: Units,
    /// Earliest turn when `payee` may present `CheckCash`.
    pub due_turn: u8,
    /// Expiration turn (`> due_turn`).
    pub expiration_turn: u8,
}

/// Parameter bundle for locking `CRED` into a Track-1 `TokenEscrow` vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowProposal {
    /// Recipient if `EscrowFinish` succeeds or bond forfeits.
    pub destination: u8,
    /// Collateral asset (must strictly be `AssetId::Cred`; perishable assets are rejected).
    pub asset: AssetId,
    /// Collateral amount (`> 0`).
    pub amount: Units,
    /// Optional earliest release turn.
    pub finish_after_turn: Option<u8>,
    /// Mandatory cancellation expiry turn (`> current_turn`).
    pub cancel_after_turn: u8,
    /// Optional `PREIMAGE-SHA-256` condition digest.
    pub condition_hash: Option<CommitmentHash>,
    /// Optional Steward-refereed physical delivery bond (`None` for plain conditional escrow).
    pub bond: Option<BondProposal>,
}

/// Client parameters for attaching a deterministic Steward delivery bond to a `TokenEscrow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BondProposal {
    /// Commodity or asset that the obligor must deliver to the beneficiary.
    pub good: AssetId,
    /// Incremental delivered units required on or before `deadline_turn`.
    pub required_units: Units,
    /// Evaluation turn (`< cancel_after_turn`).
    pub deadline_turn: u8,
    /// Settlement mode (`SellerPerformanceBond` default, or `BuyerDvpPayment` under C2).
    #[serde(default)]
    pub mode: BondSettlementMode,
}

/// Limit order submission payload for `MarketAction::PostOffer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferProposal {
    /// Buy or sell commodity against `CRED`.
    pub side: OfferSide,
    /// Traded commodity (`Raw(i)` or `Refined(i)`).
    pub good: AssetId,
    /// Order size in milli-units (`> 0`).
    pub amount: Units,
    /// Limit unit price in milli-`CRED` per `1_000` commodity units (`> 0`).
    pub price_per_unit_cred: Units,
    /// Optional cartel `DomainID` restriction.
    pub domain_id: Option<u8>,
    /// `tfHybrid` cross-domain flag (requires active `ClearinghouseBroker` charter).
    pub is_hybrid: bool,
}

/// Phase 1 (`TalkAndMarket`) actions submitted by individual seats before the Commit cutoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketAction {
    /// Post a `CRED`-priced DEX limit order (`OfferCreate`), immediately matching crossable depth.
    PostOffer {
        /// Acting seat index.
        seat: u8,
        /// Order parameters.
        order: OfferProposal,
    },
    /// Cancel one's own resting DEX offer (`OfferCancel`), unlocking residual balance.
    CancelOffer {
        /// Acting maker seat.
        seat: u8,
        /// Target offer ID.
        offer_id: u64,
    },
    /// Draw a Track-2 uncollateralized `CheckCreate` credit note (zero upfront funds locked).
    CreateCheck {
        /// Drawing seat.
        seat: u8,
        /// Check specification.
        check: CheckProposal,
    },
    /// Cancel an uncashed `CheckCreate` previously drawn by `seat` (`CheckCancel`).
    CancelCheck {
        /// Drawing seat.
        seat: u8,
        /// Check ID.
        check_id: u64,
    },
    /// Lock non-perishable `CRED` into a Track-1 `TokenEscrow` or Steward delivery bond.
    CreateEscrow {
        /// Funding sender seat.
        seat: u8,
        /// Vault/bond specification.
        escrow: EscrowProposal,
    },
    /// Refund an expired, un-finished `TokenEscrow` vault after `cancel_after_turn`.
    CancelExpiredEscrow {
        /// Seat triggering cleanup (sender or destination).
        seat: u8,
        /// Target escrow ID.
        escrow_id: u64,
    },
    /// Anchor a signed Track-2 diplomatic intent header + salted memo hash.
    RecordIntentPledge {
        /// Promisor seat (must equal `pledge.promisor`).
        seat: u8,
        /// Signed structured intent header.
        pledge: IntentPledge,
    },
    /// Temporary freeze triggered by the active `CustomsInspector` against a target seat
    /// suspected of self-dealing or wash loops (freezes target DEX offers for next turn's cutoff).
    FlagWashFreeze {
        /// Acting seat holding the active `CustomsInspector` charter.
        inspector: u8,
        /// Suspected seat placed under temporary market hold for the remainder of the turn.
        target_seat: u8,
    },
    /// File or update a public council platform ahead of a Turn-`4n` council election (`T3`).
    StandForOffice {
        /// Candidate seat filing its tariff / cartel platform.
        seat: u8,
        /// Public platform enacted without requiring a Turn-`4n` ballot if `seat` wins.
        platform: CouncilPlatform,
    },
    /// Emergency fallback purchase directly from Authority at `town_import_premium_bps`
    /// (`C1`, `2026-11-15`), letting a seat keep a converter or project unblocked if a peer
    /// idles or withholds supply.
    TownImport {
        /// Purchasing seat index.
        seat: u8,
        /// Commodity purchased (`Raw(i)` or `Refined(i)`; never `Cred`).
        good: AssetId,
        /// Quantity in milli-units (`> 0`).
        amount: Units,
    },
}

/// Validates and computes required tariff rate (`bps`) for a commodity transfer between two seats.
#[must_use]
pub fn effective_export_tariff_bps(
    base_tariff_bps: Bps,
    domain_id: Option<u8>,
    domains: &[CartelDomain],
    sender: u8,
    receiver: u8,
    holiday_bps: Option<Bps>,
) -> Bps {
    if let Some(d_id) = domain_id
        && let Some(dom) = domains.iter().find(|d| d.domain_id == d_id)
        && dom.members.contains(&sender)
        && dom.members.contains(&receiver)
    {
        return 0;
    }
    match holiday_bps {
        Some(reduction_bps) => {
            let remaining = 10_000_i32 - i32::from(reduction_bps.min(10_000));
            ((i32::from(base_tariff_bps) * remaining) / 10_000) as Bps
        }
        None => base_tariff_bps,
    }
}

/// Validates domain access credentials (`PermissionedDomain` or `ClearinghouseBroker` `tfHybrid`).
pub fn check_offer_domain_access(
    seat: u8,
    domain_id: Option<u8>,
    is_hybrid: bool,
    credit_tier: CreditTier,
    domains: &[CartelDomain],
    has_broker_charter: bool,
) -> Result<(), EngineError> {
    if is_hybrid && !has_broker_charter {
        return Err(EngineError::Unauthorized(format!(
            "seat {seat} attempted tfHybrid without active ClearinghouseBroker charter"
        )));
    }
    let Some(d_id) = domain_id else {
        return Ok(());
    };
    let Some(dom) = domains.iter().find(|d| d.domain_id == d_id) else {
        return Err(EngineError::UnknownObject(format!("cartel domain {d_id}")));
    };
    if !credit_tier.may_access_prime_cartel() {
        return Err(EngineError::DomainAccessDenied(format!(
            "seat {seat} with tier {credit_tier:?} cannot access cartel domain {d_id}"
        )));
    }
    if !dom.members.contains(&seat) {
        if is_hybrid && has_broker_charter {
            return Ok(());
        }
        return Err(EngineError::DomainAccessDenied(format!(
            "seat {seat} lacks credential for cartel domain {d_id}"
        )));
    }
    Ok(())
}
