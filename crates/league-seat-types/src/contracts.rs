use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::commit::{CommitmentHash, CouncilPlatform, IntentPledge};
use crate::types::{AssetId, CharterRole, Units};

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

impl MarketAction {
    /// Returns the acting seat index for this Phase-1 [`MarketAction`].
    #[must_use]
    pub const fn actor_seat(&self) -> u8 {
        match *self {
            Self::PostOffer { seat, .. }
            | Self::CancelOffer { seat, .. }
            | Self::CreateCheck { seat, .. }
            | Self::CancelCheck { seat, .. }
            | Self::CreateEscrow { seat, .. }
            | Self::CancelExpiredEscrow { seat, .. }
            | Self::RecordIntentPledge { seat, .. }
            | Self::StandForOffice { seat, .. }
            | Self::TownImport { seat, .. } => seat,
            Self::FlagWashFreeze { inspector, .. } => inspector,
        }
    }
}

/// Settlement category for an immutable [`FillRecord`] in public settlement logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillKind {
    /// Phase-1 resting DEX offer matched in Step 0 uniform-price batch clearing (`C4`).
    BatchFill,
    /// Bilateral `BatchV1_1` atomic swap settled in Step 2.
    AtomicSwap,
    /// Bilateral Step 2 contract delivery (`ContractDeliveryOrder`, optional `DualEscrow`).
    ContractDelivery,
    /// Track-2 deferred check (`CheckCreate` / `CheckCash`) cashed in Step 3 or C1a Regent sweep.
    CheckCashed,
    /// Track-1 `TokenEscrow` or `DualEscrow` vault unlocked cleanly in Step 2.
    EscrowFinished,
    /// Track-1 `SellerPerformanceBond` (`TokenEscrow`) or `DualEscrow` bond `CRED` forfeited to
    /// the counterparty seat (`from != to`) on a Step 2 delivery or buyer top-up default.
    EscrowForfeited,
    /// Escrowed `CRED` returned to its own sender (`from == to`) because the bond missed its
    /// deadline in Step 2; the bond default is recorded on the obligor (`bond_breaches += 1`,
    /// honour penalty via `honor_penalties`), distinct from [`Self::EscrowForfeited`] (value moves
    /// to another seat) and [`Self::EscrowRefunded`] (a clean cancel or expiry, no breach).
    EscrowBreachRefunded,
    /// Escrowed funds returned to their own sender (`from == to`) when a `TokenEscrow` is
    /// cancelled or expires unfinished, or when the non-breaching counterparty recovers its own
    /// bond (`from == to`) on a `DualEscrow` default in Step 2; a refund, not a forfeit (`2026-11-17`).
    EscrowRefunded,
    /// Authority emergency commodity purchase executed via `MarketAction::TownImport` (`C1`).
    TownImport,
}

/// Stable per-turn settlement log record (`§5.2` amended `2026-11-17`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillRecord {
    /// Turn (`1..=32`) during which this settlement occurred.
    pub turn: u8,
    /// Settlement category.
    pub kind: FillKind,
    /// Settled commodity (`Raw(_)` / `Refined(_)`) or `AssetId::Cred` for `CRED`-only check/escrow transfers.
    pub good: Option<AssetId>,
    /// Delivering / selling seat index, or `None` when sourced from Authority (`TownImport`).
    pub from: Option<u8>,
    /// Receiving / buying seat index.
    pub to: u8,
    /// Gross units of `good` settled (`CRED` milli-units when `good == Some(AssetId::Cred)`).
    pub units: Units,
    /// Net units received after commodity export tariff (`== units` for `CRED` or zero-tariff fills).
    pub net_units: Units,
    /// `CRED` leg transferred (`0` for non-`CRED` unpriced transfers).
    pub cred_amount: Units,
    /// Clearing or implied unit price in milli-`CRED` per `1_000` commodity units (`None` when unpriced).
    pub unit_price_cred: Option<Units>,
}

/// Upfront challenger bond posted by `CustomsInspector` on `FlagWashFreeze`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingWashFreeze {
    /// Turn in which `FlagWashFreeze` was invoked and settled.
    pub turn: u8,
    /// Seat holding an active `CustomsInspector` charter that posted `bond_cred`.
    pub inspector: u8,
    /// Frozen target seat index.
    pub target_seat: u8,
    /// Bond held from `inspector`'s free `CRED`.
    pub bond_cred: Units,
}

/// Deterministic Sweep settlement result for one [`PendingWashFreeze`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WashFreezeSettlement {
    /// Turn on which the freeze was enforced and settled.
    pub turn: u8,
    /// Seat holding the `CustomsInspector` charter.
    pub inspector: u8,
    /// Frozen target seat index.
    pub target_seat: u8,
    /// `true` when audit confirmed a prior-turn round trip or above-import peer price.
    pub confirmed: bool,
    /// Bond returned to `inspector` (`bond_cred` if confirmed; `0` if clean, since a clean audit
    /// pays the whole bond to `target_seat`).
    pub bond_returned_cred: Units,
    /// Bounty paid from `target_seat`'s free `CRED` (capped at available free `CRED`) when confirmed (`0` if clean).
    pub bounty_paid_cred: Units,
    /// Damages paid to `target_seat` when clean: the whole held bond (`bond_cred`); `0` if confirmed.
    pub damages_paid_cred: Units,
}
