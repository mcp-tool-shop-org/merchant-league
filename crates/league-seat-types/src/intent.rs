use serde::{Deserialize, Serialize};

use crate::commit::{
    AtomicSwapProposal, CharterBidOrder, ContractDeliveryOrder, ConverterRunOrder,
    CouncilVoteBallot, DualEscrowProposal, EscrowFinishUnlock, ProjectDeliveryOrder,
    SealedOrderBundle,
};
use crate::contracts::MarketAction;
use crate::observation::HumanWhatIfRequest;
use crate::types::Units;

/// Staging or clearing operation applied by the trusted host to a seat's pending
/// [`SealedOrderBundle`] draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Exits the interactive terminal session cleanly.
    Quit,
}

/// Plain owned seat-to-host intent returned by `league-seat-ui` without mutating engine state.
///
/// The trusted host (`league-tui`) validates every intent through `ClubHost` and `league-core`
/// previews/checks before applying any change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
