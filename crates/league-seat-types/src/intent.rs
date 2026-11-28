use alloc::borrow::ToOwned;
use alloc::format;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::commit::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder,
    CouncilVoteBallot, DualEscrowProposal, EscrowFinishUnlock, IntentTarget, ProjectDeliveryOrder,
    SealedOrderBundle,
};
use crate::contracts::{BondProposal, MarketAction, OfferSide};
use crate::observation::HumanWhatIfRequest;
use crate::types::{AssetId, EngineError, HARD_TURN_CAP, UNIT_SCALE, Units};

/// One asset leg inside a typed [`IntentHeader::AtomicSwap`] proposal.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwapLeg {
    /// Asset offered or requested on this leg (`CRED`, `Raw(i)`, or `Refined(i)`).
    pub asset: AssetId,
    /// Positive milli-unit quantity for this leg (`UNIT_SCALE == 1_000`).
    pub units: Units,
}

impl fmt::Debug for SwapLeg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SwapLeg(<redacted>)")
    }
}

/// Closed 5-variant typed negotiation header paired with bounded off-ledger whispers
/// in `TableMode::Game` and opt-in `TableMode::Club` (`docs/game-mode-protocol.md` §4.2).
///
/// Arbitrary free-form JSON maps and any sixth variant are rejected at deserialization.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum IntentHeader {
    /// Bilateral spot market offer proposal (`OfferSide::{Buy, Sell}`).
    SpotOffer {
        /// Trade direction (`Buy` or `Sell`).
        side: OfferSide,
        /// Target commodity (`Raw(i)` or `Refined(i)`, never `CRED`).
        good: AssetId,
        /// Requested whole units (`>= 1`, scaled by `UNIT_SCALE` when drafted).
        whole_units: u32,
        /// Proposed limit price in milli-`CRED` per unit (`> 0`).
        unit_price_cred: Units,
        /// Optional cartel domain ID (`PermissionedDomain`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        domain_id: Option<u8>,
    },
    /// Linked multi-asset bilateral exchange legs.
    AtomicSwap {
        /// Legs delivered by the sender (`1..=4`).
        give_legs: Vec<SwapLeg>,
        /// Legs requested from the recipient (`1..=4`).
        receive_legs: Vec<SwapLeg>,
        /// Optional client-chosen swap identifier.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        swap_id: Option<u64>,
        /// Optional shared cartel domain ID waiving export tariffs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        domain_id: Option<u8>,
    },
    /// Conditional `TokenEscrow` proposal with delivery bond and/or SHA-256 preimage condition.
    EscrowDeal {
        /// Collateral amount locked in milli-`CRED` (`> 0`).
        amount_cred: Units,
        /// Optional earliest turn when `EscrowFinish` may execute (`1..cancel_after_turn`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_after_turn: Option<u8>,
        /// Expiry turn after which un-finished collateral cancels (`1..=HARD_TURN_CAP`).
        cancel_after_turn: u8,
        /// Optional `PREIMAGE-SHA-256` condition digest (must be non-degenerate if present).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition_hex: Option<CommitmentHash>,
        /// Optional Steward-refereed physical delivery bond (`SellerPerformanceBond` or `BuyerDvpPayment`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bond: Option<BondProposal>,
    },
    /// Deferred uncollateralized `CreditCheck` proposal.
    CreditCheck {
        /// Maximum deliverable milli-`CRED` (`> 0`).
        send_max_cred: Units,
        /// Optional minimum deliverable floor (`0 < deliver_min_cred <= send_max_cred`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deliver_min_cred: Option<Units>,
        /// Optional earliest turn when `CheckCash` may clear (`1..=expiration_turn`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        due_turn: Option<u8>,
        /// Expiry turn (`1..=HARD_TURN_CAP`).
        expiration_turn: u8,
    },
    /// Non-binding or pledged diplomatic alignment matching [`IntentTarget`].
    DiplomaticPledge {
        /// Auditable promise target (`VoteForSeat`, `PreserveCartelMembership`, or `RevealPreimage`).
        target: IntentTarget,
        /// Optional honor weight in equity points (`>= 0`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        honor_weight: Option<Units>,
    },
}

fn validate_commodity_index(good: AssetId, seat_count: u8) -> Result<(), EngineError> {
    match good {
        AssetId::Cred => Ok(()),
        AssetId::Raw(idx) | AssetId::Refined(idx) => {
            if idx >= seat_count {
                return Err(EngineError::InvalidOrder(format!(
                    "commodity index {idx} exceeds table seat_count {seat_count}"
                )));
            }
            Ok(())
        }
    }
}

impl IntentHeader {
    /// Returns the canonical variant name of this [`IntentHeader`].
    #[must_use]
    pub const fn variant_name(&self) -> &'static str {
        match self {
            Self::SpotOffer { .. } => "SpotOffer",
            Self::AtomicSwap { .. } => "AtomicSwap",
            Self::EscrowDeal { .. } => "EscrowDeal",
            Self::CreditCheck { .. } => "CreditCheck",
            Self::DiplomaticPledge { .. } => "DiplomaticPledge",
        }
    }

    /// Validates semantic bounds against `seat_count` and [`HARD_TURN_CAP`].
    pub fn validate(&self, seat_count: u8) -> Result<(), EngineError> {
        if seat_count == 0 {
            return Err(EngineError::InvalidConfig(
                "table seat_count must be >= 1".to_owned(),
            ));
        }
        match self {
            Self::SpotOffer {
                good,
                whole_units,
                unit_price_cred,
                ..
            } => {
                if *good == AssetId::Cred {
                    return Err(EngineError::InvalidOrder(
                        "SpotOffer good must be Raw or Refined, never Cred".to_owned(),
                    ));
                }
                validate_commodity_index(*good, seat_count)?;
                if *whole_units == 0 {
                    return Err(EngineError::InvalidOrder(
                        "SpotOffer whole_units must be >= 1".to_owned(),
                    ));
                }
                if i64::from(*whole_units).checked_mul(UNIT_SCALE).is_none() {
                    return Err(EngineError::InvalidOrder(
                        "SpotOffer whole_units overflows milli-unit scale".to_owned(),
                    ));
                }
                if *unit_price_cred <= 0 {
                    return Err(EngineError::InvalidOrder(
                        "SpotOffer unit_price_cred must be > 0".to_owned(),
                    ));
                }
                Ok(())
            }
            Self::AtomicSwap {
                give_legs,
                receive_legs,
                ..
            } => {
                if give_legs.is_empty() || give_legs.len() > 4 {
                    return Err(EngineError::InvalidOrder(
                        "AtomicSwap give_legs must contain 1..=4 legs".to_owned(),
                    ));
                }
                if receive_legs.is_empty() || receive_legs.len() > 4 {
                    return Err(EngineError::InvalidOrder(
                        "AtomicSwap receive_legs must contain 1..=4 legs".to_owned(),
                    ));
                }
                for leg in give_legs.iter().chain(receive_legs.iter()) {
                    if leg.units <= 0 {
                        return Err(EngineError::InvalidOrder(
                            "AtomicSwap leg units must be > 0".to_owned(),
                        ));
                    }
                    validate_commodity_index(leg.asset, seat_count)?;
                }
                Ok(())
            }
            Self::EscrowDeal {
                amount_cred,
                finish_after_turn,
                cancel_after_turn,
                condition_hex,
                bond,
            } => {
                if *amount_cred <= 0 {
                    return Err(EngineError::InvalidOrder(
                        "EscrowDeal amount_cred must be > 0".to_owned(),
                    ));
                }
                if *cancel_after_turn == 0 || *cancel_after_turn > HARD_TURN_CAP {
                    return Err(EngineError::InvalidOrder(format!(
                        "EscrowDeal cancel_after_turn must be in 1..={HARD_TURN_CAP}"
                    )));
                }
                if let Some(ft) = *finish_after_turn
                    && (ft == 0 || ft >= *cancel_after_turn)
                {
                    return Err(EngineError::InvalidOrder(
                        "EscrowDeal finish_after_turn must satisfy 1 <= finish_after_turn < cancel_after_turn"
                            .to_owned(),
                    ));
                }
                if condition_hex.is_none() && bond.is_none() {
                    return Err(EngineError::InvalidOrder(
                        "EscrowDeal requires condition_hex and/or bond".to_owned(),
                    ));
                }
                if let Some(cond) = condition_hex
                    && cond.is_degenerate()
                {
                    return Err(EngineError::InvalidOrder(
                        "EscrowDeal condition_hex must not be all-zero".to_owned(),
                    ));
                }
                if let Some(b) = bond {
                    if b.good == AssetId::Cred {
                        return Err(EngineError::InvalidOrder(
                            "EscrowDeal bond good must be non-perishable commodity".to_owned(),
                        ));
                    }
                    if b.good.is_perishable() {
                        return Err(EngineError::PerishableEscrowForbidden);
                    }
                    validate_commodity_index(b.good, seat_count)?;
                    if b.required_units <= 0 {
                        return Err(EngineError::InvalidOrder(
                            "EscrowDeal bond required_units must be > 0".to_owned(),
                        ));
                    }
                    if b.deadline_turn == 0 || b.deadline_turn > HARD_TURN_CAP {
                        return Err(EngineError::InvalidOrder(format!(
                            "EscrowDeal bond deadline_turn must be in 1..={HARD_TURN_CAP}"
                        )));
                    }
                }
                Ok(())
            }
            Self::CreditCheck {
                send_max_cred,
                deliver_min_cred,
                due_turn,
                expiration_turn,
            } => {
                if *send_max_cred <= 0 {
                    return Err(EngineError::InvalidOrder(
                        "CreditCheck send_max_cred must be > 0".to_owned(),
                    ));
                }
                if *expiration_turn == 0 || *expiration_turn > HARD_TURN_CAP {
                    return Err(EngineError::InvalidOrder(format!(
                        "CreditCheck expiration_turn must be in 1..={HARD_TURN_CAP}"
                    )));
                }
                if let Some(dm) = *deliver_min_cred
                    && (dm <= 0 || dm > *send_max_cred)
                {
                    return Err(EngineError::InvalidOrder(
                        "CreditCheck deliver_min_cred must satisfy 0 < deliver_min_cred <= send_max_cred"
                            .to_owned(),
                    ));
                }
                if let Some(dt) = *due_turn
                    && (dt == 0 || dt > *expiration_turn)
                {
                    return Err(EngineError::InvalidOrder(
                        "CreditCheck due_turn must satisfy 1 <= due_turn <= expiration_turn"
                            .to_owned(),
                    ));
                }
                Ok(())
            }
            Self::DiplomaticPledge {
                target,
                honor_weight,
            } => {
                if let Some(hw) = *honor_weight
                    && hw < 0
                {
                    return Err(EngineError::InvalidOrder(
                        "DiplomaticPledge honor_weight must be >= 0".to_owned(),
                    ));
                }
                match target {
                    IntentTarget::VoteForSeat {
                        office_turn,
                        candidate,
                    } => {
                        if *office_turn == 0 || *office_turn > HARD_TURN_CAP {
                            return Err(EngineError::InvalidOrder(format!(
                                "VoteForSeat office_turn must be in 1..={HARD_TURN_CAP}"
                            )));
                        }
                        if *candidate >= seat_count {
                            return Err(EngineError::InvalidSeat(*candidate));
                        }
                    }
                    IntentTarget::PreserveCartelMembership {
                        office_turn,
                        beneficiary,
                        ..
                    } => {
                        if *office_turn == 0 || *office_turn > HARD_TURN_CAP {
                            return Err(EngineError::InvalidOrder(format!(
                                "PreserveCartelMembership office_turn must be in 1..={HARD_TURN_CAP}"
                            )));
                        }
                        if *beneficiary >= seat_count {
                            return Err(EngineError::InvalidSeat(*beneficiary));
                        }
                    }
                    IntentTarget::RevealPreimage {
                        escrow_id,
                        condition,
                    } => {
                        if *escrow_id == 0 {
                            return Err(EngineError::InvalidOrder(
                                "RevealPreimage escrow_id must be >= 1".to_owned(),
                            ));
                        }
                        if condition.is_degenerate() {
                            return Err(EngineError::InvalidOrder(
                                "RevealPreimage condition must not be all-zero".to_owned(),
                            ));
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

impl fmt::Debug for IntentHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntentHeader")
            .field("kind", &self.variant_name())
            .field("payload", &"<redacted>")
            .finish()
    }
}

/// Staging or clearing operation applied by the trusted host to a seat's pending
/// [`SealedOrderBundle`] draft.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BundleDraftAction {
    /// Sets or updates a converter batch run (`runs == 0` removes any entry for `recipe_idx`).
    SetConverterRun(ConverterRunOrder),
    /// Clears all staged converter runs.
    ClearConverterRuns,
    /// Sets or updates a Syndicate Project delivery (`batches == 0` removes `project_id`).
    SetProjectDelivery(ProjectDeliveryOrder),
    /// Clears all staged project deliveries.
    ClearProjectDeliveries,
    /// Stages a co-signed atomic swap proposal if not already present.
    StageAtomicSwap(AtomicSwapProposal),
    /// Removes a single staged atomic swap by 0-based index (`Some(idx)`) or clears all (`None`).
    ClearAtomicSwaps(Option<usize>),
    /// Stages a direct contractual commodity delivery order if not already present.
    StageContractDelivery(ContractDeliveryOrder),
    /// Removes a single staged contract delivery by 0-based index (`Some(idx)`) or clears all (`None`).
    ClearContractDeliveries(Option<usize>),
    /// Stages a co-signed dual delivery-versus-payment escrow proposal if not already present.
    StageDualEscrow(DualEscrowProposal),
    /// Removes a single staged dual escrow by 0-based index (`Some(idx)`) or clears all (`None`).
    ClearDualEscrows(Option<usize>),
    /// Sets voluntary repayment toward outstanding `CRED` auto-debt principal (`0` clears).
    SetDebtRepaymentCred(Units),
    /// Stages a conditional `TokenEscrow` finish reveal (`EscrowFinish`).
    StageEscrowFinish(EscrowFinishUnlock),
    /// Removes a single staged escrow finish by 0-based index (`Some(idx)`) or clears all (`None`).
    ClearEscrowFinishes(Option<usize>),
    /// Stages a maturing `check_id` for `CheckCash` clearing at Sweep Step 3.
    StageCheckCash(u64),
    /// Removes a single staged check ID (`Some(id)`) or clears all staged checks (`None`).
    ClearCheckCashes(Option<u64>),
    /// Sets (`Some`) or clears (`None`) the Turn-`4n` council election ballot.
    SetCouncilVote(Option<CouncilVoteBallot>),
    /// Sets or replaces a Turn-`6n` charter bid for `role` (`bid_cred <= 0` removes `role`).
    StageCharterBid(CharterBidOrder),
    /// Clears all staged Turn-`6n` charter bids.
    ClearCharterBids,
    /// Clears every entry in the pending [`SealedOrderBundle`].
    ClearAll,
}

impl BundleDraftAction {
    /// Applies this staging or clearing operation to `bundle`.
    pub fn apply_to(&self, bundle: &mut SealedOrderBundle) {
        match self {
            Self::SetConverterRun(order) => {
                if order.runs == 0 {
                    bundle
                        .converter_runs
                        .retain(|entry| entry.recipe_idx != order.recipe_idx);
                } else if let Some(existing) = bundle
                    .converter_runs
                    .iter_mut()
                    .find(|entry| entry.recipe_idx == order.recipe_idx)
                {
                    existing.runs = order.runs;
                } else {
                    bundle.converter_runs.push(order.clone());
                }
            }
            Self::ClearConverterRuns => bundle.converter_runs.clear(),
            Self::SetProjectDelivery(order) => {
                if order.batches == 0 {
                    bundle
                        .project_deliveries
                        .retain(|entry| entry.project_id != order.project_id);
                } else if let Some(existing) = bundle
                    .project_deliveries
                    .iter_mut()
                    .find(|entry| entry.project_id == order.project_id)
                {
                    existing.batches = order.batches;
                } else {
                    bundle.project_deliveries.push(order.clone());
                }
            }
            Self::ClearProjectDeliveries => bundle.project_deliveries.clear(),
            Self::StageAtomicSwap(swap) => {
                if !bundle.atomic_swaps.contains(swap) {
                    bundle.atomic_swaps.push(swap.clone());
                }
            }
            Self::ClearAtomicSwaps(Some(idx)) => {
                if *idx < bundle.atomic_swaps.len() {
                    bundle.atomic_swaps.remove(*idx);
                }
            }
            Self::ClearAtomicSwaps(None) => bundle.atomic_swaps.clear(),
            Self::StageContractDelivery(order) => {
                if !bundle.contract_deliveries.contains(order) {
                    bundle.contract_deliveries.push(order.clone());
                }
            }
            Self::ClearContractDeliveries(Some(idx)) => {
                if *idx < bundle.contract_deliveries.len() {
                    bundle.contract_deliveries.remove(*idx);
                }
            }
            Self::ClearContractDeliveries(None) => bundle.contract_deliveries.clear(),
            Self::StageDualEscrow(prop) => {
                if !bundle.dual_escrows.contains(prop) {
                    bundle.dual_escrows.push(prop.clone());
                }
            }
            Self::ClearDualEscrows(Some(idx)) => {
                if *idx < bundle.dual_escrows.len() {
                    bundle.dual_escrows.remove(*idx);
                }
            }
            Self::ClearDualEscrows(None) => bundle.dual_escrows.clear(),
            Self::SetDebtRepaymentCred(amount) => {
                bundle.debt_repayment_cred = (*amount).max(0);
            }
            Self::StageEscrowFinish(unlock) => {
                if let Some(existing) = bundle
                    .escrow_finishes
                    .iter_mut()
                    .find(|entry| entry.escrow_id == unlock.escrow_id)
                {
                    existing.preimage = unlock.preimage.clone();
                } else {
                    bundle.escrow_finishes.push(unlock.clone());
                }
            }
            Self::ClearEscrowFinishes(Some(idx)) => {
                if *idx < bundle.escrow_finishes.len() {
                    bundle.escrow_finishes.remove(*idx);
                }
            }
            Self::ClearEscrowFinishes(None) => bundle.escrow_finishes.clear(),
            Self::StageCheckCash(check_id) => {
                if !bundle.check_cashes.contains(check_id) {
                    bundle.check_cashes.push(*check_id);
                }
            }
            Self::ClearCheckCashes(Some(check_id)) => {
                bundle.check_cashes.retain(|id| id != check_id);
            }
            Self::ClearCheckCashes(None) => bundle.check_cashes.clear(),
            Self::SetCouncilVote(ballot) => {
                bundle.council_vote = ballot.clone();
            }
            Self::StageCharterBid(bid) => {
                if bid.bid_cred <= 0 {
                    bundle.charter_bids.retain(|entry| entry.role != bid.role);
                } else if let Some(existing) = bundle
                    .charter_bids
                    .iter_mut()
                    .find(|entry| entry.role == bid.role)
                {
                    existing.bid_cred = bid.bid_cred;
                } else {
                    bundle.charter_bids.push(bid.clone());
                }
            }
            Self::ClearCharterBids => bundle.charter_bids.clear(),
            Self::ClearAll => *bundle = SealedOrderBundle::default(),
        }
    }
}

impl fmt::Debug for BundleDraftAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BundleDraftAction(<redacted>)")
    }
}

/// Discriminant for [`SeatIntent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SeatIntentKind {
    /// Submits a Phase-1 [`MarketAction`] to be validated and applied by the host.
    SubmitMarketAction,
    /// Stages or clears entries in the active seat's pending [`SealedOrderBundle`].
    UpdateBundle,
    /// Requests a side-effect-free What-If Sandbox preview (`preview_what_if`) from the host.
    RequestWhatIf,
    /// Locks the current seat's pass orders and hands keyboard control through the Curtain.
    LockAndHandOff,
    /// Opens the Sweep Chronicle modal over historical [`crate::PublicSweepReceipt`] entries.
    OpenChronicleModal,
    /// Closes the Sweep Chronicle modal.
    CloseChronicleModal,
    /// Submits Turn-1 Private Charter Mandate reveals once the season enters `AwaitingMandateReveal`.
    SubmitMandateReveal,
    /// Exports the verified [`crate::SeasonReplayLog`] (or replay file) at `Completed` season close.
    ExportReplay,
    /// Exits the interactive terminal session cleanly.
    Quit,
}

/// Plain owned seat-to-host intent returned by `league-seat-ui` without mutating engine state.
///
/// The trusted host (`league-tui`) validates every intent through `ClubHost` and `league-core`
/// previews/checks before applying any change.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatIntent {
    /// Which host action the seat is requesting.
    pub kind: SeatIntentKind,
    /// Phase-1 action payload when `kind` is [`SeatIntentKind::SubmitMarketAction`].
    pub market_action: Option<MarketAction>,
    /// Sealed bundle draft mutation when `kind` is [`SeatIntentKind::UpdateBundle`].
    pub bundle_action: Option<BundleDraftAction>,
    /// What-If Sandbox query when `kind` is [`SeatIntentKind::RequestWhatIf`].
    pub what_if: Option<HumanWhatIfRequest>,
}

impl SeatIntent {
    /// Constructs a Phase-1 [`MarketAction`] submission intent.
    #[must_use]
    pub const fn submit_market_action(action: MarketAction) -> Self {
        Self {
            kind: SeatIntentKind::SubmitMarketAction,
            market_action: Some(action),
            bundle_action: None,
            what_if: None,
        }
    }

    /// Constructs a pending [`SealedOrderBundle`] draft update intent.
    #[must_use]
    pub const fn update_bundle(action: BundleDraftAction) -> Self {
        Self {
            kind: SeatIntentKind::UpdateBundle,
            market_action: None,
            bundle_action: Some(action),
            what_if: None,
        }
    }

    /// Constructs a side-effect-free What-If Sandbox preview request.
    #[must_use]
    pub const fn request_what_if(request: HumanWhatIfRequest) -> Self {
        Self {
            kind: SeatIntentKind::RequestWhatIf,
            market_action: None,
            bundle_action: None,
            what_if: Some(request),
        }
    }

    /// Constructs a lock-and-hand-off intent to pass control through the Curtain.
    #[must_use]
    pub const fn lock_and_hand_off() -> Self {
        Self {
            kind: SeatIntentKind::LockAndHandOff,
            market_action: None,
            bundle_action: None,
            what_if: None,
        }
    }

    /// Constructs an intent to open the Sweep Chronicle modal.
    #[must_use]
    pub const fn open_chronicle_modal() -> Self {
        Self {
            kind: SeatIntentKind::OpenChronicleModal,
            market_action: None,
            bundle_action: None,
            what_if: None,
        }
    }

    /// Constructs an intent to close the Sweep Chronicle modal.
    #[must_use]
    pub const fn close_chronicle_modal() -> Self {
        Self {
            kind: SeatIntentKind::CloseChronicleModal,
            market_action: None,
            bundle_action: None,
            what_if: None,
        }
    }

    /// Constructs an intent to submit the next Turn-1 Private Mandate reveal at season close.
    #[must_use]
    pub const fn submit_mandate_reveal() -> Self {
        Self {
            kind: SeatIntentKind::SubmitMandateReveal,
            market_action: None,
            bundle_action: None,
            what_if: None,
        }
    }

    /// Constructs an intent to export the verified `SeasonReplayLog` at `Completed` season close.
    #[must_use]
    pub const fn export_replay() -> Self {
        Self {
            kind: SeatIntentKind::ExportReplay,
            market_action: None,
            bundle_action: None,
            what_if: None,
        }
    }

    /// Constructs an intent to exit the interactive terminal session cleanly.
    #[must_use]
    pub const fn quit() -> Self {
        Self {
            kind: SeatIntentKind::Quit,
            market_action: None,
            bundle_action: None,
            what_if: None,
        }
    }
}

impl fmt::Debug for SeatIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeatIntent")
            .field("kind", &self.kind)
            .field(
                "market_action",
                &self.market_action.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "bundle_action",
                &self.bundle_action.as_ref().map(|_| "<redacted>"),
            )
            .field("what_if", &self.what_if.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}
