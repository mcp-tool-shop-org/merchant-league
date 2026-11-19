use alloc::string::String;
use alloc::vec::Vec;

use league_seat_types::{
    AssetId, AtomicSwapProposal, BondProposal, BondSettlementMode, CharterBidOrder, CharterRole,
    CheckProposal, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder, CouncilPlatform,
    CouncilVoteBallot, DualEscrowProposal, EscrowFinishUnlock, EscrowProposal, IntentPledge,
    IntentTarget, MarketAction, OfferProposal, OfferSide, PreimageSecret, PreviewTieMode,
    ProjectDeliveryOrder, SealedOrderBundle, SeatIntent, SeatObservation, SeatScreenView, Units,
    WhatIfPreview,
};

/// Pass label rendered inside the Curtain hand-off screen and header banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassKind {
    /// Phase-1 Sub-Pass A before cancel lock engages.
    #[default]
    SubPassA,
    /// Phase-1 Sub-Pass B and Phase-2 bundle staging after cancel lock engages.
    SubPassB,
}

impl PassKind {
    /// Human-readable label for this sub-pass.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SubPassA => "Sub-Pass A (Open Market & Talk)",
            Self::SubPassB => "Sub-Pass B (Post-Lock & Sealed Commit)",
        }
    }
}

/// Active tab inside the Center Action & Order Composer zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CenterTab {
    /// Tab 1: Phase-1 `MarketAction` composer.
    #[default]
    MarketAndContracts,
    /// Tab 2: Phase-2 `SealedOrderBundle` composer (Turn-4n vote & Turn-6n charter bids).
    WorkshopDealsPolitics,
    /// Tab 3: Human What-If Sandbox (`preview_what_if`).
    WhatIfSandbox,
}

impl CenterTab {
    /// Zero-based index inside the center tab strip (`0..=2`).
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::MarketAndContracts => 0,
            Self::WorkshopDealsPolitics => 1,
            Self::WhatIfSandbox => 2,
        }
    }

    /// Steps forward (`forward = true`) or backward (`forward = false`) across enabled tabs.
    #[must_use]
    pub const fn cycle(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::MarketAndContracts, true) => Self::WorkshopDealsPolitics,
            (Self::WorkshopDealsPolitics, true) => Self::WhatIfSandbox,
            (Self::WhatIfSandbox, true) => Self::MarketAndContracts,
            (Self::MarketAndContracts, false) => Self::WhatIfSandbox,
            (Self::WorkshopDealsPolitics, false) => Self::MarketAndContracts,
            (Self::WhatIfSandbox, false) => Self::WorkshopDealsPolitics,
        }
    }
}

/// Focused scrollable zone when navigating with Up/Down/PageUp/PageDown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusZone {
    /// Left public table & ledger mirror pane.
    PublicLedger,
    /// Center composer/sandbox tab workspace.
    #[default]
    CenterTabs,
    /// Right seat diagnostics & private vault drawer.
    PrivateDrawer,
}

impl FocusZone {
    /// Cycles focus across the three scrollable turn zones.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::PublicLedger => Self::CenterTabs,
            Self::CenterTabs => Self::PrivateDrawer,
            Self::PrivateDrawer => Self::PublicLedger,
        }
    }
}

/// Selectable `MarketAction` builder variant in Tab 1 (`Market & Contracts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarketActionKind {
    /// `MarketAction::PostOffer`
    #[default]
    PostOffer,
    /// `MarketAction::CancelOffer` (blocked once `cancel_locked == true`)
    CancelOffer,
    /// `MarketAction::CreateCheck`
    CreateCheck,
    /// `MarketAction::CancelCheck`
    CancelCheck,
    /// `MarketAction::CreateEscrow`
    CreateEscrow,
    /// `MarketAction::CancelExpiredEscrow`
    CancelExpiredEscrow,
    /// `MarketAction::RecordIntentPledge`
    RecordIntentPledge,
    /// `MarketAction::StandForOffice`
    StandForOffice,
    /// `MarketAction::FlagWashFreeze`
    FlagWashFreeze,
    /// `MarketAction::TownImport`
    TownImport,
}

pub(crate) const ALL_MARKET_KINDS: [MarketActionKind; 10] = [
    MarketActionKind::PostOffer,
    MarketActionKind::CancelOffer,
    MarketActionKind::CreateCheck,
    MarketActionKind::CancelCheck,
    MarketActionKind::CreateEscrow,
    MarketActionKind::CancelExpiredEscrow,
    MarketActionKind::RecordIntentPledge,
    MarketActionKind::StandForOffice,
    MarketActionKind::FlagWashFreeze,
    MarketActionKind::TownImport,
];

impl MarketActionKind {
    /// Short human-readable title for this `MarketAction` variant.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PostOffer => "PostOffer",
            Self::CancelOffer => "CancelOffer",
            Self::CreateCheck => "CreateCheck",
            Self::CancelCheck => "CancelCheck",
            Self::CreateEscrow => "CreateEscrow",
            Self::CancelExpiredEscrow => "CancelExpiredEscrow",
            Self::RecordIntentPledge => "RecordIntentPledge",
            Self::StandForOffice => "StandForOffice",
            Self::FlagWashFreeze => "FlagWashFreeze",
            Self::TownImport => "TownImport",
        }
    }
}

/// Selectable section inside Tab 2 (`Workshop, Deals & Politics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BundleSection {
    /// Tier-2 `converter_runs`
    #[default]
    ConverterRuns,
    /// Tier-3 `project_deliveries`
    ProjectDeliveries,
    /// Bilateral `atomic_swaps`
    AtomicSwaps,
    /// Direct `contract_deliveries`
    ContractDeliveries,
    /// Co-signed `dual_escrows`
    DualEscrows,
    /// Voluntary `debt_repayment_cred`
    DebtRepayment,
    /// Conditional `escrow_finishes`
    EscrowFinishes,
    /// Track-2 `check_cashes`
    CheckCashes,
    /// Turn-`4n` `council_vote`
    CouncilVote,
    /// Turn-`6n` `charter_bids`
    CharterBids,
}

impl BundleSection {
    /// Short human-readable title for this `SealedOrderBundle` section.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ConverterRuns => "Converter Runs",
            Self::ProjectDeliveries => "Project Deliveries",
            Self::AtomicSwaps => "Atomic Swaps",
            Self::ContractDeliveries => "Contract Deliveries",
            Self::DualEscrows => "Dual Escrows",
            Self::DebtRepayment => "Debt Repayment",
            Self::EscrowFinishes => "Escrow Finishes",
            Self::CheckCashes => "Check Presentations",
            Self::CouncilVote => "Council Vote (Turn 4n)",
            Self::CharterBids => "Charter Bids (Turn 6n)",
        }
    }
}

pub(crate) fn visible_bundle_sections(turn: u8) -> Vec<BundleSection> {
    let mut out = alloc::vec![
        BundleSection::ConverterRuns,
        BundleSection::ProjectDeliveries,
        BundleSection::AtomicSwaps,
        BundleSection::ContractDeliveries,
        BundleSection::DualEscrows,
        BundleSection::DebtRepayment,
        BundleSection::EscrowFinishes,
        BundleSection::CheckCashes,
    ];
    if turn > 0 && turn.is_multiple_of(4) {
        out.push(BundleSection::CouncilVote);
    }
    if turn > 0 && turn.is_multiple_of(6) {
        out.push(BundleSection::CharterBids);
    }
    out
}

/// Selection indices preserved across a hot-seat hand-off (`docs/club-mode-design.md` §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeatUiSelection {
    /// Currently focused zone.
    pub focus_zone: FocusZone,
    /// Selected index into `ALL_MARKET_KINDS`.
    pub market_kind_idx: usize,
    /// Selected editable parameter field for the active `MarketActionKind`.
    pub market_field_idx: usize,
    /// Selected index into `visible_bundle_sections(obs.turn)`.
    pub bundle_section_idx: usize,
    /// Selected editable parameter field for the active `BundleSection`.
    pub bundle_field_idx: usize,
    /// Selected toggle row inside the What-If Sandbox.
    pub what_if_row_idx: usize,
    /// Selected row index inside the Public Ledger zone.
    pub ledger_row_idx: usize,
    /// Selected row index inside the Private Drawer zone.
    pub drawer_row_idx: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarketComposerFields {
    pub(crate) side_buy: bool,
    pub(crate) good_kind_raw: bool,
    pub(crate) check_asset_mode: u8,
    pub(crate) good_idx: u8,
    pub(crate) amount: Units,
    pub(crate) price_per_unit_cred: Units,
    pub(crate) domain_id: Option<u8>,
    pub(crate) is_hybrid: bool,
    pub(crate) target_id: u64,
    pub(crate) counterparty: u8,
    pub(crate) deliver_min: Units,
    pub(crate) due_turn: u8,
    pub(crate) expiration_turn: u8,
    pub(crate) finish_after_turn: Option<u8>,
    pub(crate) cancel_after_turn: u8,
    pub(crate) has_condition: bool,
    pub(crate) has_bond: bool,
    pub(crate) bond_buyer_dvp: bool,
    pub(crate) bond_required_units: Units,
    pub(crate) bond_deadline_turn: u8,
    pub(crate) pledge_target_kind: u8,
    pub(crate) honor_weight: Units,
    pub(crate) office_turn: u8,
    pub(crate) platform_raw_tariff_bps: u16,
    pub(crate) platform_refined_tariff_bps: u16,
    pub(crate) platform_admit_domain: Option<u8>,
    pub(crate) platform_revoke_domain: Option<u8>,
}

impl Default for MarketComposerFields {
    fn default() -> Self {
        Self {
            side_buy: true,
            good_kind_raw: true,
            check_asset_mode: 0,
            good_idx: 1,
            amount: 1_000,
            price_per_unit_cred: 1_200,
            domain_id: None,
            is_hybrid: false,
            target_id: 1,
            counterparty: 1,
            deliver_min: 800,
            due_turn: 0,
            expiration_turn: 0,
            finish_after_turn: None,
            cancel_after_turn: 0,
            has_condition: false,
            has_bond: false,
            bond_buyer_dvp: false,
            bond_required_units: 1_000,
            bond_deadline_turn: 0,
            pledge_target_kind: 0,
            honor_weight: 500,
            office_turn: 4,
            platform_raw_tariff_bps: 800,
            platform_refined_tariff_bps: 600,
            platform_admit_domain: None,
            platform_revoke_domain: None,
        }
    }
}

impl MarketComposerFields {
    pub(crate) fn selected_good(&self, seat_count: u8) -> AssetId {
        let g = self.good_idx % seat_count.max(1);
        if self.good_kind_raw {
            AssetId::Raw(g)
        } else {
            AssetId::Refined(g)
        }
    }

    pub(crate) fn selected_check_asset(&self, seat_count: u8) -> AssetId {
        let g = self.good_idx % seat_count.max(1);
        match self.check_asset_mode % 3 {
            0 => AssetId::Cred,
            1 => AssetId::Raw(g),
            _ => AssetId::Refined(g),
        }
    }

    pub(crate) fn build_action(
        &self,
        obs: &SeatObservation,
        kind: MarketActionKind,
    ) -> MarketAction {
        let seat = obs.seat;
        let n = obs.spec.seat_count.max(1);
        let peer = self.counterparty % n;
        let peer_other = if peer == seat && n > 1 {
            (seat + 1) % n
        } else {
            peer
        };
        let good = self.selected_good(n);
        let due = if self.due_turn == 0 {
            obs.turn
        } else {
            self.due_turn
        };
        let exp = if self.expiration_turn == 0 {
            due.saturating_add(3).min(32).max(due.saturating_add(1))
        } else {
            self.expiration_turn
        };
        let cancel_after = if self.cancel_after_turn == 0 {
            obs.turn.saturating_add(4).min(32).max(obs.turn + 1)
        } else {
            self.cancel_after_turn
        };
        let bond_deadline = if self.bond_deadline_turn == 0 {
            obs.turn
                .saturating_add(2)
                .min(cancel_after.saturating_sub(1))
                .max(obs.turn)
        } else {
            self.bond_deadline_turn
        };
        let next_office_turn = ((obs.turn.saturating_add(3)) / 4)
            .saturating_mul(4)
            .clamp(4, 32);
        let office_turn = if self.office_turn == 0 {
            next_office_turn
        } else {
            self.office_turn
        };

        match kind {
            MarketActionKind::PostOffer => MarketAction::PostOffer {
                seat,
                order: OfferProposal {
                    side: if self.side_buy {
                        OfferSide::Buy
                    } else {
                        OfferSide::Sell
                    },
                    good,
                    amount: self.amount.max(1),
                    price_per_unit_cred: self.price_per_unit_cred.max(1),
                    domain_id: self.domain_id,
                    is_hybrid: self.is_hybrid,
                },
            },
            MarketActionKind::CancelOffer => MarketAction::CancelOffer {
                seat,
                offer_id: self.target_id,
            },
            MarketActionKind::CreateCheck => MarketAction::CreateCheck {
                seat,
                check: CheckProposal {
                    payee: peer_other,
                    asset: self.selected_check_asset(n),
                    send_max: self.amount.max(1),
                    deliver_min: self.deliver_min.clamp(1, self.amount.max(1)),
                    due_turn: due,
                    expiration_turn: exp,
                },
            },
            MarketActionKind::CancelCheck => MarketAction::CancelCheck {
                seat,
                check_id: self.target_id,
            },
            MarketActionKind::CreateEscrow => MarketAction::CreateEscrow {
                seat,
                escrow: EscrowProposal {
                    destination: peer_other,
                    asset: AssetId::Cred,
                    amount: self.amount.max(1),
                    finish_after_turn: self.finish_after_turn,
                    cancel_after_turn: cancel_after,
                    condition_hash: if self.has_condition {
                        Some(CommitmentHash([0x5A; 32]))
                    } else {
                        None
                    },
                    bond: if self.has_bond {
                        Some(BondProposal {
                            good,
                            required_units: self.bond_required_units.max(1),
                            deadline_turn: bond_deadline,
                            mode: if self.bond_buyer_dvp {
                                BondSettlementMode::BuyerDvpPayment
                            } else {
                                BondSettlementMode::SellerPerformanceBond
                            },
                        })
                    } else {
                        None
                    },
                },
            },
            MarketActionKind::CancelExpiredEscrow => MarketAction::CancelExpiredEscrow {
                seat,
                escrow_id: self.target_id,
            },
            MarketActionKind::RecordIntentPledge => {
                let target = match self.pledge_target_kind % 3 {
                    0 => IntentTarget::VoteForSeat {
                        office_turn,
                        candidate: peer_other,
                    },
                    1 => IntentTarget::PreserveCartelMembership {
                        office_turn,
                        domain_id: self.domain_id.unwrap_or(0),
                        beneficiary: peer_other,
                    },
                    _ => IntentTarget::RevealPreimage {
                        escrow_id: self.target_id.max(1),
                        condition: CommitmentHash([0x5A; 32]),
                    },
                };
                MarketAction::RecordIntentPledge {
                    seat,
                    pledge: IntentPledge {
                        pledge_id: self.target_id.max(1),
                        promisor: seat,
                        counterparty: peer_other,
                        target,
                        honor_weight: self.honor_weight.max(1),
                        memo_digest: [0x22; 32],
                    },
                }
            }
            MarketActionKind::StandForOffice => MarketAction::StandForOffice {
                seat,
                platform: CouncilPlatform {
                    proposed_tariffs_bps: alloc::vec![(
                        seat,
                        (
                            self.platform_raw_tariff_bps.min(3_500),
                            self.platform_refined_tariff_bps.min(3_500)
                        )
                    )],
                    cartel_admissions: self
                        .platform_admit_domain
                        .map_or_else(Vec::new, |d| alloc::vec![(d, peer_other)]),
                    cartel_revocations: self
                        .platform_revoke_domain
                        .map_or_else(Vec::new, |d| alloc::vec![(d, peer_other)]),
                },
            },
            MarketActionKind::FlagWashFreeze => MarketAction::FlagWashFreeze {
                inspector: seat,
                target_seat: peer_other,
            },
            MarketActionKind::TownImport => MarketAction::TownImport {
                seat,
                good,
                amount: self.amount.max(1),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundleComposerFields {
    pub(crate) recipe_idx: usize,
    pub(crate) runs: u32,
    pub(crate) project_id: u16,
    pub(crate) batches: u32,
    pub(crate) swap_id: u64,
    pub(crate) swap_taker: u8,
    pub(crate) swap_maker_raw: bool,
    pub(crate) swap_maker_good: u8,
    pub(crate) swap_maker_amount: Units,
    pub(crate) swap_taker_raw: bool,
    pub(crate) swap_taker_good: u8,
    pub(crate) swap_taker_amount: Units,
    pub(crate) cd_counterparty: u8,
    pub(crate) cd_raw: bool,
    pub(crate) cd_good: u8,
    pub(crate) cd_amount: Units,
    pub(crate) cd_dual_escrow_id: Option<u64>,
    pub(crate) dual_id: u64,
    pub(crate) dual_seller: u8,
    pub(crate) dual_raw: bool,
    pub(crate) dual_good: u8,
    pub(crate) dual_required_units: Units,
    pub(crate) dual_payment_cred: Units,
    pub(crate) dual_buyer_bond_cred: Units,
    pub(crate) dual_seller_bond_cred: Units,
    pub(crate) dual_deadline_turn: u8,
    pub(crate) debt_repayment_cred: Units,
    pub(crate) escrow_finish_id: u64,
    pub(crate) escrow_use_preimage: bool,
    pub(crate) escrow_preimage_byte: u8,
    pub(crate) check_cash_id: u64,
    pub(crate) vote_tm: u8,
    pub(crate) vote_cc: u8,
    pub(crate) charter_role_idx: u8,
    pub(crate) charter_bid_cred: Units,
}

impl Default for BundleComposerFields {
    fn default() -> Self {
        Self {
            recipe_idx: 0,
            runs: 1,
            project_id: 1,
            batches: 1,
            swap_id: 1,
            swap_taker: 1,
            swap_maker_raw: false,
            swap_maker_good: 0,
            swap_maker_amount: 1_000,
            swap_taker_raw: false,
            swap_taker_good: 1,
            swap_taker_amount: 1_000,
            cd_counterparty: 1,
            cd_raw: false,
            cd_good: 0,
            cd_amount: 1_000,
            cd_dual_escrow_id: None,
            dual_id: 1,
            dual_seller: 1,
            dual_raw: false,
            dual_good: 1,
            dual_required_units: 1_000,
            dual_payment_cred: 2_000,
            dual_buyer_bond_cred: 500,
            dual_seller_bond_cred: 500,
            dual_deadline_turn: 0,
            debt_repayment_cred: 500,
            escrow_finish_id: 1,
            escrow_use_preimage: false,
            escrow_preimage_byte: 0x11,
            check_cash_id: 1,
            vote_tm: 0,
            vote_cc: 1,
            charter_role_idx: 0,
            charter_bid_cred: 200,
        }
    }
}

impl BundleComposerFields {
    pub(crate) fn charter_role(&self) -> CharterRole {
        match self.charter_role_idx % 3 {
            0 => CharterRole::ClearinghouseBroker,
            1 => CharterRole::CustomsInspector,
            _ => CharterRole::OracleAuditor,
        }
    }

    pub(crate) fn build_stage_op(
        &self,
        obs: &SeatObservation,
        section: BundleSection,
    ) -> league_seat_types::BundleDraftAction {
        use league_seat_types::BundleDraftAction;
        let seat = obs.seat;
        let n = obs.spec.seat_count.max(1);
        let other = |candidate: u8| -> u8 {
            let c = candidate % n;
            if c == seat && n > 1 {
                (seat + 1) % n
            } else {
                c
            }
        };
        let asset_of = |is_raw: bool, idx: u8| -> AssetId {
            let g = idx % n;
            if is_raw {
                AssetId::Raw(g)
            } else {
                AssetId::Refined(g)
            }
        };

        match section {
            BundleSection::ConverterRuns => BundleDraftAction::SetConverterRun(ConverterRunOrder {
                recipe_idx: self.recipe_idx,
                runs: self.runs,
            }),
            BundleSection::ProjectDeliveries => {
                BundleDraftAction::SetProjectDelivery(ProjectDeliveryOrder {
                    project_id: self.project_id,
                    batches: self.batches,
                })
            }
            BundleSection::AtomicSwaps => BundleDraftAction::StageAtomicSwap(AtomicSwapProposal {
                swap_id: self.swap_id.max(1),
                maker: seat,
                taker: other(self.swap_taker),
                maker_asset: asset_of(self.swap_maker_raw, self.swap_maker_good),
                maker_amount: self.swap_maker_amount.max(1),
                taker_asset: asset_of(self.swap_taker_raw, self.swap_taker_good),
                taker_amount: self.swap_taker_amount.max(1),
                domain_id: None,
            }),
            BundleSection::ContractDeliveries => {
                BundleDraftAction::StageContractDelivery(ContractDeliveryOrder {
                    counterparty: other(self.cd_counterparty),
                    good: asset_of(self.cd_raw, self.cd_good),
                    gross_amount: self.cd_amount.max(1),
                    domain_id: None,
                    dual_escrow_id: self.cd_dual_escrow_id,
                })
            }
            BundleSection::DualEscrows => BundleDraftAction::StageDualEscrow(DualEscrowProposal {
                proposal_id: self.dual_id.max(1),
                buyer: seat,
                seller: other(self.dual_seller),
                good: asset_of(self.dual_raw, self.dual_good),
                required_units: self.dual_required_units.max(1),
                payment_cred: self.dual_payment_cred.max(2),
                buyer_bond_cred: self.dual_buyer_bond_cred.max(1),
                seller_bond_cred: self.dual_seller_bond_cred.max(1),
                deadline_turn: if self.dual_deadline_turn == 0 {
                    obs.turn.saturating_add(2).min(32)
                } else {
                    self.dual_deadline_turn
                },
                domain_id: None,
            }),
            BundleSection::DebtRepayment => {
                BundleDraftAction::SetDebtRepaymentCred(self.debt_repayment_cred.max(0))
            }
            BundleSection::EscrowFinishes => {
                BundleDraftAction::StageEscrowFinish(EscrowFinishUnlock {
                    escrow_id: self.escrow_finish_id.max(1),
                    preimage: if self.escrow_use_preimage {
                        Some(PreimageSecret([self.escrow_preimage_byte.max(1); 32]))
                    } else {
                        None
                    },
                })
            }
            BundleSection::CheckCashes => {
                BundleDraftAction::StageCheckCash(self.check_cash_id.max(1))
            }
            BundleSection::CouncilVote => {
                let tm = self.vote_tm % n;
                let mut cc = self.vote_cc % n;
                if cc == tm && n > 1 {
                    cc = (tm + 1) % n;
                }
                BundleDraftAction::SetCouncilVote(Some(CouncilVoteBallot {
                    trade_minister: tm,
                    cartel_chair: cc,
                }))
            }
            BundleSection::CharterBids => BundleDraftAction::StageCharterBid(CharterBidOrder {
                role: self.charter_role(),
                bid_cred: self.charter_bid_cred.max(0),
            }),
        }
    }

    pub(crate) fn build_clear_op(
        &self,
        section: BundleSection,
    ) -> league_seat_types::BundleDraftAction {
        use league_seat_types::BundleDraftAction;
        match section {
            BundleSection::ConverterRuns => BundleDraftAction::ClearConverterRuns,
            BundleSection::ProjectDeliveries => BundleDraftAction::ClearProjectDeliveries,
            BundleSection::AtomicSwaps => BundleDraftAction::ClearAtomicSwaps(None),
            BundleSection::ContractDeliveries => BundleDraftAction::ClearContractDeliveries(None),
            BundleSection::DualEscrows => BundleDraftAction::ClearDualEscrows(None),
            BundleSection::DebtRepayment => BundleDraftAction::SetDebtRepaymentCred(0),
            BundleSection::EscrowFinishes => BundleDraftAction::ClearEscrowFinishes(None),
            BundleSection::CheckCashes => BundleDraftAction::ClearCheckCashes(None),
            BundleSection::CouncilVote => BundleDraftAction::SetCouncilVote(None),
            BundleSection::CharterBids => BundleDraftAction::ClearCharterBids,
        }
    }
}

/// Host-owned per-seat UI state passed into `league-seat-ui` strictly for one seat at a time.
///
/// Every non-scalar / text-bearing field stays `pub(crate)` behind typed accessors, and
/// [`Self::clear_for_hand_off`] drops all free text, drafts, notes, scrolls, and previews while
/// keeping only [`Self::active_tab`] and [`Self::selection`] (`docs/club-mode-design.md` §1.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SeatUiState {
    pub(crate) active_tab: CenterTab,
    pub(crate) selection: SeatUiSelection,
    pub(crate) public_scroll: usize,
    pub(crate) center_scroll: usize,
    pub(crate) drawer_scroll: usize,
    pub(crate) chronicle_scroll: usize,
    pub(crate) chronicle_receipt_idx: usize,
    pub(crate) chronicle_open: bool,
    pub(crate) plain_codes: bool,
    pub(crate) composer_edit_active: bool,
    pub(crate) notes_edit_active: bool,
    pub(crate) composer_draft: String,
    pub(crate) whisper_draft: String,
    pub(crate) notes_draft: String,
    pub(crate) market_fields: MarketComposerFields,
    pub(crate) bundle_fields: BundleComposerFields,
    pub(crate) tie_mode: PreviewTieMode,
    pub(crate) assumed_cancelled_offer_ids: Vec<u64>,
    pub(crate) assumed_bounced_check_ids: Vec<u64>,
    pub(crate) assume_cosign_swaps: bool,
    pub(crate) assume_cosign_dual_escrows: bool,
    pub(crate) assume_peer_deliveries: bool,
    pub(crate) include_draft_market_action: bool,
    pub(crate) staged_bundle: SealedOrderBundle,
    pub(crate) last_what_if: Option<WhatIfPreview>,
}

impl SeatUiState {
    /// Creates a clean initial [`SeatUiState`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears every free-text field (`composer_draft`, `whisper_draft`, `notes_draft`), what-if
    /// toggle/preview, scroll offset, and modal flag ahead of a Curtain hand-off, retaining only
    /// `active_tab` and `selection`.
    pub fn clear_for_hand_off(&mut self) {
        let active_tab = self.active_tab;
        let selection = self.selection;
        *self = Self::new();
        self.active_tab = active_tab;
        self.selection = selection;
    }

    /// Updates the host-validated staged [`SealedOrderBundle`] mirrored in Tab 2 and Tab 3.
    pub fn set_staged_bundle(&mut self, bundle: SealedOrderBundle) {
        self.staged_bundle = bundle;
    }

    /// Stores the latest host-computed [`WhatIfPreview`] result (or clears it with `None`).
    pub fn set_what_if_preview(&mut self, preview: Option<WhatIfPreview>) {
        self.last_what_if = preview;
    }

    /// Sets whether the Sweep Chronicle modal is open.
    pub fn set_chronicle_open(&mut self, open: bool) {
        self.chronicle_open = open;
        if open {
            self.chronicle_scroll = 0;
        }
    }

    /// Sets whether commodity labels render as plain 3-letter codes (`true`) or in-world names (`false`).
    pub fn set_plain_codes(&mut self, plain_codes: bool) {
        self.plain_codes = plain_codes;
    }

    /// Returns the currently selected center tab.
    #[must_use]
    pub const fn active_tab(&self) -> CenterTab {
        self.active_tab
    }

    /// Returns the current [`SeatUiSelection`] indices.
    #[must_use]
    pub const fn selection(&self) -> SeatUiSelection {
        self.selection
    }

    /// Returns the active free-text composer draft buffer.
    #[must_use]
    pub fn composer_draft(&self) -> &str {
        &self.composer_draft
    }

    /// Returns the active free-text whisper buffer (always empty in Part B1).
    #[must_use]
    pub fn whisper_draft(&self) -> &str {
        &self.whisper_draft
    }

    /// Returns the active personal free-text notes buffer.
    #[must_use]
    pub fn notes_draft(&self) -> &str {
        &self.notes_draft
    }

    /// Returns `true` when the Sweep Chronicle modal is open.
    #[must_use]
    pub const fn chronicle_open(&self) -> bool {
        self.chronicle_open
    }

    /// Returns `true` when plain 3-letter tickers are displayed instead of in-world names.
    #[must_use]
    pub const fn plain_codes(&self) -> bool {
        self.plain_codes
    }

    /// Returns the cached host-computed [`WhatIfPreview`], if any.
    #[must_use]
    pub const fn what_if_preview(&self) -> Option<&WhatIfPreview> {
        self.last_what_if.as_ref()
    }

    /// Returns the host-synced staged [`SealedOrderBundle`].
    #[must_use]
    pub const fn staged_bundle(&self) -> &SealedOrderBundle {
        &self.staged_bundle
    }
}

/// Complete step result emitted by `handle_seat_key` without mutating host or engine state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatUiStep {
    /// Updated screen view model for the host to draw.
    pub view: SeatScreenView,
    /// Updated per-seat UI state owned by the host.
    pub state: SeatUiState,
    /// Optional seat-to-host intent for validation and application.
    pub intent: Option<SeatIntent>,
}
