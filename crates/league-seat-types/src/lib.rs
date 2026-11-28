#![no_std]
#![forbid(unsafe_code)]
#![forbid(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
//! Seat-safe value, observation, preview, error, and commodity display types for Merchant League v2.
//!
//! This `no_std` + `alloc` leaf crate depends solely on `serde`, `serde_json`, and
//! `league-error` — never on `league-core`, `league-gen`, or `league-standalone` — so `rustc`
//! and target builds (`thumbv7em-none-eabi`) enforce the host/seat boundary at compile time
//! (`docs/club-mode-design.md` §2.3).

extern crate alloc;

mod commit;
mod contracts;
mod display;
mod engine_types;
mod intent;
mod observation;
mod predicates_types;
mod scoring_types;
mod serde_helpers;
mod types;
mod view;

pub use commit::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder,
    CouncilPlatform, CouncilVoteBallot, DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_PERIOD_SECS,
    DrandSchedule, DualEscrowProposal, EscrowFinishUnlock, GradedMandateCard, IntentPledge,
    IntentTarget, MandateKind, PreimageSecret, PrivateMandate, ProjectDeliveryOrder,
    SealedOrderBundle, drand_quicknet_round_for_close,
};
pub use contracts::{
    ActiveCharter, BondProposal, BondSettlementMode, BondSpec, CartelDomain, CheckProposal,
    CheckStatus, CredOffer, CreditCheck, DualEscrowVault, EscrowProposal, EscrowStatus, FillKind,
    FillRecord, MAX_CARTEL_DOMAINS, MarketAction, OfferProposal, OfferSide, PendingWashFreeze,
    TokenEscrowVault, WashFreezeSettlement,
};
pub use display::{
    COMMODITY_DISPLAY_CATALOG, CommodityDisplayEntry, CommodityDisplayMap, NUMERAIRE_CODE,
    NUMERAIRE_DISPLAY_LABEL, NUMERAIRE_PLAIN_LABEL, SeatCommodityDisplay, format_numeraire_display,
    lookup_commodity_display, lookup_commodity_display_bytes,
};
pub use engine_types::{
    BrokenPromiseRecord, ConverterOrderPreview, EscrowOrderPreview, ProjectOrderPreview, SeatState,
    SeatUpkeepPreview, ShockForecastEvent, TurnPhase,
};
pub use intent::{BundleDraftAction, IntentHeader, SeatIntent, SeatIntentKind, SwapLeg};
pub use league_error::{StructuredError, ToStructuredError, scrub_secret_text};
#[doc(hidden)]
pub use observation::SeatObservationPublicParts;
pub use observation::{
    AiPreviewRequest, AuditorShockHint, CounterpartyAssumptions, DEALT_MANDATE_COMMIT_SENTINEL,
    HYPOTHETICAL_CLOSE_MARK_CAVEAT, HumanWhatIfRequest, PREVIEW_NEUTRAL_BEACON_SEED,
    PreviewTieMode, PublicCheckOrderPreview, PublicCheckOutcome, PublicPreviewState,
    PublicSwapOrderPreview, PublicSwapOutcome, PublicSweepReceipt, PublicTableSpec,
    RefusedActionPreview, SeatObservation, SeatSpoilagePreview, Step0To7SweepSummary,
    WhatIfPreview,
};
pub use predicates_types::{
    CoalitionReach, GoodScarcity, HoldingMark, ProjectCover, ProjectInputCover, RecipeCover,
    STATE_PREDICATE_SECTION_VERSION, STATE_PREDICATES_VERSION, SeasonEndAndMarks,
    SeatSeasonMetrics, SeatStatePredicates, SeatSweepOutcome, SpoilageAndConversion,
    StatePredicateSection, TurnPredicateRecord, UpkeepAndDebt,
};
pub use scoring_types::{FinalScoreboard, FundamentalParities, ParitySpecSource, SeatScoreEntry};
pub use types::{
    ActiveShock, AssetId, BPS_SCALE, Bps, CLOSING_ROLL_START_TURN, CLOSING_ROLL_WINDOW_LEN,
    CharterRole, ConverterRecipe, CreditTier, EngineError, HARD_TURN_CAP,
    MANDATE_STARTING_NW_FLOOR_BPS, MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY, MarketDay,
    ProjectRecipe, RegentDoctrine, SeasonId, Seat, SeatConfig, SeatIdentity, ShockKind, TableId,
    TableMode, Turn, UNIT_SCALE, UNSOLD_EXCESS_PARITY_BPS, Units,
};
pub use view::{
    AwaitingRevealProgress, CurtainPinMode, CurtainPinPrompt, EndOfSeasonData, LobbyCliOverrides,
    ReplayExportState, ScreenLayout, SeatKeyCode, SeatKeyEvent, SeatKeyModifiers, SeatScreenView,
    TurnClockDisplay, ViewAlign, ViewAxis, ViewColor, ViewLabel, ViewLayoutMode, ViewModal,
    ViewPanel, ViewRow, ViewScroll, ViewSizeHint, ViewSpan, ViewStatusLine, ViewStyle, ViewTabBar,
    ViewTabItem, ViewTable, ViewTableCell, ViewTableColumn, ViewTableRow, ViewZoneGroup,
    ViewZoneId, ViewZonePanel, ViewZoneSpec, ViewportSize,
};

impl core::fmt::Debug for SeatScreenView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SeatScreenView")
            .field("layout", &self.layout)
            .field("zones_len", &self.zones.len())
            .field("has_status_line", &self.status_line.is_some())
            .field("has_modal", &self.modal.is_some())
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewSpan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewSpan")
            .field("text_len", &self.text.len())
            .field("style", &self.style)
            .field("text", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewRow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewRow")
            .field("spans_len", &self.spans.len())
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewLabel")
            .field("text_len", &self.text.len())
            .field("style", &self.style)
            .field("text", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewTableColumn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewTableColumn")
            .field("header", &self.header)
            .field("width", &self.width)
            .field("alignment", &self.alignment)
            .finish()
    }
}

impl core::fmt::Debug for ViewTableCell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewTableCell")
            .field("spans_len", &self.spans.len())
            .field("alignment", &self.alignment)
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewTableRow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewTableRow")
            .field("cells_len", &self.cells.len())
            .field("style", &self.style)
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewTable")
            .field("columns_len", &self.columns.len())
            .field("rows_len", &self.rows.len())
            .field("selected_row", &self.selected_row)
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewTabItem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewTabItem")
            .field("label", &self.label)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl core::fmt::Debug for ViewTabBar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewTabBar")
            .field("tabs_len", &self.tabs.len())
            .field("active_index", &self.active_index)
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewModal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewModal")
            .field("width_pct", &self.width_pct)
            .field("height_pct", &self.height_pct)
            .field("panel", &self.panel)
            .finish()
    }
}

impl core::fmt::Debug for ViewStatusLine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewStatusLine")
            .field("has_alert", &self.alert.is_some())
            .field("left_spans_len", &self.left.len())
            .field("right_spans_len", &self.right.len())
            .field("style", &self.style)
            .field("content", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for ViewZonePanel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewZonePanel")
            .field("zone", &self.zone)
            .field("panel", &self.panel)
            .finish()
    }
}

impl core::fmt::Debug for ViewPanel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewPanel")
            .field("has_title", &self.title.is_some())
            .field("alignment", &self.alignment)
            .field("bordered", &self.bordered)
            .field("has_tabs", &self.tabs.is_some())
            .field("has_table", &self.table.is_some())
            .field("rows_len", &self.rows.len())
            .field("selected_row", &self.selected_row)
            .field("scroll", &self.scroll)
            .field("content", &"<redacted>")
            .finish()
    }
}

/// Internal serde helpers re-exported within `crates/` via explicit crate usage where identical
/// `EngineState` / `SeasonReplayLog` JSON encodings are shared.
pub mod wire_serde {
    use alloc::collections::BTreeMap;

    use serde::{Deserializer, Serializer};

    use crate::serde_helpers;
    use crate::types::{AssetId, Units};

    /// Cumulative bilateral delivery ledger type `(sender, receiver, asset) -> units`.
    pub type DeliveryLedgerMap = BTreeMap<(u8, u8, AssetId), Units>;

    /// Serializes a 32-byte digest or commitment as 64 lowercase ASCII hex characters.
    pub fn serialize_hex32<S: Serializer>(
        bytes: &[u8; 32],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_helpers::hex32::serialize(bytes, serializer)
    }

    /// Deserializes a 64-character hex string into 32 bytes.
    pub fn deserialize_hex32<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error> {
        serde_helpers::hex32::deserialize(deserializer)
    }

    /// Serializes an optional 32-byte digest as optional 64 lowercase ASCII hex characters.
    pub fn serialize_opt_hex32<S: Serializer>(
        value: &Option<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_helpers::opt_hex32::serialize(value, serializer)
    }

    /// Deserializes an optional 64-character hex string into optional 32 bytes.
    pub fn deserialize_opt_hex32<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        serde_helpers::opt_hex32::deserialize(deserializer)
    }

    /// Serializes the cumulative bilateral delivery ledger map as key-value entries.
    pub fn serialize_delivery_ledger<S: Serializer>(
        map: &DeliveryLedgerMap,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_helpers::delivery_ledger_serde::serialize(map, serializer)
    }

    /// Deserializes bilateral delivery ledger entries while rejecting duplicate tuple keys.
    pub fn deserialize_delivery_ledger<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DeliveryLedgerMap, D::Error> {
        serde_helpers::delivery_ledger_serde::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned;
    use alloc::collections::{BTreeMap, BTreeSet};
    use alloc::format;
    use alloc::vec;
    use alloc::vec::Vec;

    use alloc::string::ToString;

    use crate::{
        AssetId, AtomicSwapProposal, AuditorShockHint, BondProposal, BondSettlementMode, Bps,
        BundleDraftAction, COMMODITY_DISPLAY_CATALOG, CharterBidOrder, CharterRole, CoalitionReach,
        CommitmentHash, CommodityDisplayMap, ContractDeliveryOrder, ConverterRecipe,
        ConverterRunOrder, CouncilVoteBallot, CounterpartyAssumptions, CreditTier,
        DRAND_QUICKNET_GENESIS_UNIX, DrandSchedule, DualEscrowProposal, EngineError,
        EscrowFinishUnlock, FundamentalParities, GoodScarcity, GradedMandateCard,
        HumanWhatIfRequest, IntentHeader, IntentTarget, MandateKind, MarketAction, MarketDay,
        NUMERAIRE_CODE, NUMERAIRE_DISPLAY_LABEL, NUMERAIRE_PLAIN_LABEL, OfferProposal, OfferSide,
        ParitySpecSource, PreimageSecret, PreviewTieMode, PrivateMandate, ProjectDeliveryOrder,
        PublicSweepReceipt, PublicTableSpec, RefusedActionPreview, RegentDoctrine,
        STATE_PREDICATES_VERSION, ScreenLayout, SealedOrderBundle, SeasonEndAndMarks, SeasonId,
        Seat, SeatCommodityDisplay, SeatConfig, SeatIdentity, SeatIntent, SeatIntentKind,
        SeatKeyCode, SeatKeyEvent, SeatKeyModifiers, SeatObservation, SeatObservationPublicParts,
        SeatScreenView, SeatSpoilagePreview, SeatState, SeatStatePredicates, SeatUpkeepPreview,
        ShockKind, SpoilageAndConversion, Step0To7SweepSummary, SwapLeg, TableId, TableMode,
        ToStructuredError, Turn, TurnPhase, UpkeepAndDebt, ViewAlign, ViewAxis, ViewColor,
        ViewLabel, ViewLayoutMode, ViewModal, ViewPanel, ViewRow, ViewScroll, ViewSizeHint,
        ViewSpan, ViewStatusLine, ViewStyle, ViewTabBar, ViewTabItem, ViewTable, ViewTableCell,
        ViewTableColumn, ViewTableRow, ViewZoneId, ViewZonePanel, ViewportSize, WhatIfPreview,
        drand_quicknet_round_for_close, format_numeraire_display, lookup_commodity_display,
        lookup_commodity_display_bytes,
    };

    #[test]
    fn ids_assets_credit_and_commit_value_methods_work() {
        assert!(Seat(0) < Seat(1));
        assert!(MarketDay(1) < MarketDay(2));
        assert!(Turn(1) < Turn(2));
        assert!(TableId(1) < TableId(2));
        assert!(SeasonId(1) < SeasonId(2));

        assert!(!AssetId::Cred.is_perishable());
        assert!(AssetId::Raw(0).is_perishable());
        assert!(!AssetId::Refined(0).is_perishable());

        assert_eq!(CreditTier::Aaa.interest_bps(), 400);
        assert_eq!(CreditTier::Bbb.interest_bps(), 1_000);
        assert_eq!(CreditTier::B.interest_bps(), 1_800);
        assert_eq!(CreditTier::D.interest_bps(), 3_000);
        assert_eq!(CreditTier::Aaa.downgrade_one(), CreditTier::Bbb);
        assert_eq!(CreditTier::Bbb.downgrade_one(), CreditTier::B);
        assert_eq!(CreditTier::B.downgrade_one(), CreditTier::D);
        assert_eq!(CreditTier::D.downgrade_one(), CreditTier::D);
        assert!(CreditTier::Aaa.may_access_prime_cartel());
        assert!(CreditTier::Bbb.may_access_prime_cartel());
        assert!(CreditTier::B.may_access_prime_cartel());
        assert!(!CreditTier::D.may_access_prime_cartel());

        let zero_secret = PreimageSecret([0u8; 32]);
        assert!(zero_secret.is_degenerate());
        assert!(zero_secret.validate().is_err());
        let good_secret = PreimageSecret([0x5A; 32]);
        assert!(!good_secret.is_degenerate());
        assert!(good_secret.validate().is_ok());
        assert_eq!(format!("{good_secret:?}"), "PreimageSecret(<hidden>)");

        let zero_hash = CommitmentHash([0u8; 32]);
        assert!(zero_hash.is_degenerate());
        let hash = CommitmentHash([0xAB; 32]);
        assert!(!hash.is_degenerate());
        assert_eq!(format!("{hash}"), "ab".repeat(32));
        assert_eq!(
            format!("{hash:?}"),
            format!("CommitmentHash({})", "ab".repeat(32))
        );
    }

    #[test]
    fn mandate_drand_action_error_and_display_catalog_work() {
        assert!(
            MandateKind::MinProjectEquity { target: 0 }
                .validate_for_seat_count(4)
                .is_err()
        );
        assert!(
            MandateKind::MinProjectEquity { target: 100 }
                .validate_for_seat_count(2)
                .is_err()
        );
        assert!(
            MandateKind::MinRefinedVariety {
                distinct_goods: 3,
                min_each: 1_000,
            }
            .validate_for_seat_count(4)
            .is_err()
        );
        assert!(
            MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::D,
            }
            .validate_for_seat_count(4)
            .is_err()
        );
        assert!(
            MandateKind::MinActivePartners { min_partners: 3 }
                .validate_for_seat_count(4)
                .is_err()
        );
        assert!(
            MandateKind::MinActivePartners { min_partners: 2 }
                .validate_for_seat_count(4)
                .is_ok()
        );
        let card = GradedMandateCard {
            card_id: 1,
            kind: MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            difficulty_completion_bps: 5_000,
            bonus_equity: 5_500,
        };
        assert_eq!(card.mandate_points_milli(), 5_500);

        let ds = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
            turn_period_secs: 600,
        };
        assert!(ds.validate().is_ok());
        assert!(ds.close_unix_for_turn(Turn(0)).is_err());
        assert!(ds.close_unix_for_turn(Turn(1)).is_ok());
        assert!(ds.expected_round_for_turn(Turn(1)).is_ok());
        assert!(drand_quicknet_round_for_close(0).is_err());
        assert!(drand_quicknet_round_for_close(DRAND_QUICKNET_GENESIS_UNIX + 6).is_ok());
        assert!(
            DrandSchedule {
                turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX - 1,
                turn_period_secs: 60,
            }
            .validate()
            .is_err()
        );
        assert!(
            DrandSchedule {
                turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
                turn_period_secs: u64::MAX,
            }
            .validate()
            .is_err()
        );
        assert!(
            DrandSchedule {
                turn_1_close_unix: u64::MAX,
                turn_period_secs: 3,
            }
            .validate()
            .is_err()
        );

        let bad_ds = DrandSchedule {
            turn_1_close_unix: 10,
            turn_period_secs: 1,
        };
        assert!(bad_ds.validate().is_err());

        let act = MarketAction::FlagWashFreeze {
            inspector: 3,
            target_seat: 1,
        };
        assert_eq!(act.actor_seat(), 3);
        assert_eq!(
            MarketAction::TownImport {
                seat: 2,
                good: AssetId::Raw(0),
                amount: 100
            }
            .actor_seat(),
            2
        );

        let err = EngineError::CancelLocked;
        let se = err.to_structured_error();
        assert_eq!(se.code, "CANCEL_LOCKED");
        for e in [
            EngineError::InvalidConfig("x".to_owned()),
            EngineError::InvalidPhase("x".to_owned()),
            EngineError::InvalidSeat(9),
            EngineError::InvalidOrder("x".to_owned()),
            EngineError::UnknownObject("x".to_owned()),
            EngineError::Unauthorized("x".to_owned()),
            EngineError::PerishableEscrowForbidden,
            EngineError::InsufficientBalance("x".to_owned()),
            EngineError::AlreadySubmitted(0),
            EngineError::CommitmentMismatch(0),
            EngineError::DomainAccessDenied("x".to_owned()),
            EngineError::SeasonEnded("x".to_owned()),
            EngineError::UnsupportedReplayVersion {
                found: 0,
                newest_known: 3,
            },
        ] {
            let s = e.to_structured_error();
            assert_eq!(s.code, e.code());
            assert!(!s.message.is_empty());
        }

        assert_eq!(COMMODITY_DISPLAY_CATALOG.len(), 61);
        assert_eq!(format_numeraire_display(false), NUMERAIRE_DISPLAY_LABEL);
        assert_eq!(format_numeraire_display(true), NUMERAIRE_PLAIN_LABEL);
        assert!(lookup_commodity_display_bytes(*b"TMB").is_some());
        assert!(lookup_commodity_display_bytes(*b"ZZZ").is_none());
        let tmb_opt = lookup_commodity_display("TMB");
        assert!(tmb_opt.is_some());
        if let Some(tmb) = tmb_opt {
            assert_eq!(tmb.raw_display(false), "Rough Timber [TMB]");
            assert_eq!(tmb.raw_display(true), "TMB");
            assert_eq!(tmb.refined_display(false), "Seasoned Timber [TMB]");
            assert_eq!(tmb.refined_display(true), "TMB");
        }
        assert!(lookup_commodity_display("CRED").is_none());
        assert!(lookup_commodity_display("ZWZ").is_none());
        assert!(lookup_commodity_display("AB").is_none());

        let map = CommodityDisplayMap {
            numeraire_code: NUMERAIRE_CODE.to_owned(),
            numeraire_label: NUMERAIRE_DISPLAY_LABEL.to_owned(),
            seats: vec![SeatCommodityDisplay {
                seat: 0,
                raw_code: "TMB".to_owned(),
                raw_label: "Rough Timber [TMB]".to_owned(),
                refined_code: "SLT".to_owned(),
                refined_label: "Cured Fine Salt [SLT]".to_owned(),
            }],
        };
        assert_eq!(
            map.format_asset(AssetId::Cred, false).as_deref(),
            Some("Clearinghouse Marks [CRED]")
        );
        assert_eq!(
            map.format_asset(AssetId::Cred, true).as_deref(),
            Some("CRED")
        );
        assert_eq!(
            map.format_asset(AssetId::Raw(0), false).as_deref(),
            Some("Rough Timber [TMB]")
        );
        assert_eq!(
            map.format_asset(AssetId::Raw(0), true).as_deref(),
            Some("TMB")
        );
        assert_eq!(
            map.format_asset(AssetId::Refined(0), false).as_deref(),
            Some("Cured Fine Salt [SLT]")
        );
        assert_eq!(
            map.format_asset(AssetId::Refined(0), true).as_deref(),
            Some("SLT")
        );
        assert!(map.format_asset(AssetId::Raw(9), false).is_none());
        assert!(map.format_asset(AssetId::Refined(9), false).is_none());
    }

    #[test]
    fn observation_preview_summary_and_parities_cover_helpers_cleanly() {
        struct EmptySpec;
        impl ParitySpecSource for EmptySpec {
            fn spec_seat_count(&self) -> u8 {
                0
            }

            fn spec_seats(&self) -> &[SeatConfig] {
                &[]
            }
        }
        let empty_p = FundamentalParities::from_spec(&EmptySpec);
        assert_eq!(empty_p.cred_per_unit, 1_000);
        assert!(empty_p.raw_per_unit.is_empty());
        assert!(empty_p.refined_per_unit.is_empty());
        assert_eq!(empty_p.asset_parity(AssetId::Cred), 1_000);
        assert_eq!(empty_p.asset_parity(AssetId::Raw(5)), 0);
        assert_eq!(empty_p.asset_parity(AssetId::Refined(5)), 0);

        let seat_cfg = SeatConfig {
            seat: 0,
            identity: SeatIdentity::Human,
            initial_cred: 15_000,
            raw_yield_per_turn: 10_000,
            refined_project_value: 2_400,
            upkeep_good: 1,
            upkeep_amount: 1_800,
            initial_raw_tariff_bps: 800,
            initial_refined_tariff_bps: 600,
            perishable_decay_bps: 2_000,
            regent: Some(RegentDoctrine {
                risk_tolerance_bps: 5_000,
                min_raw_reserve: 1_000,
                auto_cash_due_claims: true,
            }),
            converters: vec![
                ConverterRecipe {
                    owner: 0,
                    inputs: vec![(0, 1_000)],
                    output_good: 0,
                    output_amount: 2_000,
                    max_runs_per_turn: 2,
                    is_secondary: true,
                },
                ConverterRecipe {
                    owner: 0,
                    inputs: vec![],
                    output_good: 0,
                    output_amount: 1_000,
                    max_runs_per_turn: 1,
                    is_secondary: false,
                },
            ],
        };
        let pub_spec = PublicTableSpec {
            table_id: TableId(55),
            mode: TableMode::Club,
            seat_count: 1,
            seats: vec![seat_cfg],
            projects: Vec::new(),
            closing_hazard_bps: [3_500; 8],
            mandate_deck: Vec::new(),
            drand_schedule: None,
            eval_vrf_pubkey: None,
            town_import_premium_bps: 22_500,
            works_stipend_per_unit_cred: 350,
            mandate_alpha_bps: 250,
            cancel_lock_bps: 2_500,
            wash_round_trip_min_units: 1_000,
            wash_freeze_bond_cred: 500,
            wash_freeze_bounty_cred: 500,
        };
        assert_eq!(pub_spec.spec_seat_count(), 1);
        assert_eq!(pub_spec.spec_seats().len(), 1);
        let parities = FundamentalParities::from_spec(&pub_spec);
        assert_eq!(parities.raw_per_unit, vec![2_400]);
        assert_eq!(parities.refined_per_unit, vec![2_400]);

        let predicates = SeatStatePredicates {
            version: STATE_PREDICATES_VERSION,
            table_id: TableId(55),
            turn: 2,
            phase: TurnPhase::TalkAndMarket,
            seat: 0,
            upkeep: UpkeepAndDebt {
                upkeep_good: 1,
                owed_units: 1_800,
                held_units: 0,
                held_on_own_resting_sells: 0,
                shortfall_units: 1_800,
                debt_added_at_sweep: 3_600,
                interest_bps: 400,
            },
            spoilage: SpoilageAndConversion {
                own_raw_good: 0,
                own_raw_yield_at_sweep: 10_000,
                spoilage_bps_by_good: vec![2_000],
                spoil_units_held_by_good: vec![0],
                spoil_units_on_resting_offers: 0,
                spoil_units_total: 0,
                workshop_capacity_runs_left: 2,
                recipes: Vec::new(),
            },
            scarcity: vec![GoodScarcity {
                good: AssetId::Raw(0),
                producer_seat: 0,
                own_units: 0,
                table_units_held: 0,
                open_book_units_offered: 0,
                open_book_units_bid: 0,
                domain_book_units_offered: 0,
                domain_book_units_bid: 0,
                seats_needing: 1,
                open_book_tariff_bps: 800,
            }],
            coalitions: vec![CoalitionReach {
                members: vec![0],
                upkeep_goods_makeable: Vec::new(),
                upkeep_goods_not_makeable: vec![1],
            }],
            season_end: SeasonEndAndMarks {
                season_closed: false,
                turns_until_earliest_close: 22,
                turns_until_latest_close: 30,
                holdings: Vec::new(),
                projects: Vec::new(),
            },
            cancel_locked: false,
        };
        let receipt = PublicSweepReceipt {
            turn: 1,
            non_reveal_seats: vec![],
            regent_acted_seats: vec![],
            converted_to_ai: vec![],
            escrows_finished: vec![],
            escrows_cancelled_or_forfeited: vec![],
            atomic_swaps_settled: 0,
            converter_batches_run: 1,
            converter_orders_clamped: 0,
            checks_cashed: vec![],
            checks_bounced: vec![],
            cartel_evictions: vec![],
            projects_completed: 0,
            project_orders_clamped: 0,
            total_upkeep_shortfall_debt: 0,
            total_spoilage_clawed_back: 0,
            broken_promises: vec![],
            elected_officers: None,
            charters_awarded: vec![],
            public_forecasts: vec![],
            newly_activated_shocks: vec![],
            cred_debt_repaid: 0,
            charter_cred_burned: 0,
            town_import_cred_spent: 0,
            works_stipend_cred_minted: 0,
            batch_trades_cleared: 0,
            fills: vec![],
            wash_freeze_settlements: vec![],
            season_closed: false,
            pbrs_deltas: vec![10],
        };
        let summary = Step0To7SweepSummary::from_public_sweep_receipt(&receipt);
        assert_eq!(summary.turn, 1);
        assert_eq!(summary.converter_batches_run, 1);

        let hint = AuditorShockHint {
            hint_turn: 2,
            forecast_turn: 3,
            realize_turn: 4,
            target_seat: 0,
            kind: ShockKind::DemandSurge,
        };
        let obs = SeatObservation::unmandated(SeatObservationPublicParts {
            seat: 0,
            table_id: TableId(55),
            turn: 2,
            phase: TurnPhase::TalkAndMarket,
            cancel_locked: false,
            trade_minister: 0,
            cartel_chair: None,
            spec: pub_spec,
            parities,
            seats: vec![SeatState {
                seat: 0,
                identity: SeatIdentity::Human,
                cred: 15_000,
                cred_debt: 0,
                raw: vec![0],
                refined: vec![0],
                project_equity: 0,
                raw_tariff_bps: 800,
                refined_tariff_bps: 600,
                credit_tier: CreditTier::Aaa,
                hard_credit_defaults: 0,
                bond_breaches: 0,
                broken_promises: 0,
                honor_penalties: 0,
                consecutive_misses: 0,
                consecutive_zero_production_turns: 0,
                wash_frozen_until_turn: 0,
            }],
            offers: BTreeMap::new(),
            checks: BTreeMap::new(),
            escrows: BTreeMap::new(),
            dual_escrows: BTreeMap::new(),
            pledges: BTreeMap::new(),
            audited_pledges: BTreeSet::new(),
            domains: Vec::new(),
            council_platforms: BTreeMap::new(),
            charters: Vec::new(),
            active_shocks: Vec::new(),
            public_forecasts: Vec::new(),
            auditor_hints: vec![hint],
            next_offer_id: 1,
            next_check_id: 1,
            next_escrow_id: 1,
            delivery_ledger: BTreeMap::new(),
            last_potentials: vec![15_000],
            pending_town_import_cred_spent: 0,
            fill_log: Vec::new(),
            pending_wash_freezes: Vec::new(),
            eval_vrf_pubkey: None,
            baseline_predicates: predicates,
            public_receipts: vec![receipt],
        });
        assert!(obs.own_mandate().is_none());
        assert_eq!(obs.auditor_hints(), &[hint]);
        let preview_state = obs.to_public_preview_state();
        assert_eq!(preview_state.table_id, TableId(55));
        assert_eq!(preview_state.turn, 2);
        let default_bundle = SealedOrderBundle::default();
        assert!(default_bundle.converter_runs.is_empty());
        let default_assumptions = CounterpartyAssumptions::default();
        assert!(default_assumptions.assumed_peer_bundles.is_empty());
        assert_eq!(PreviewTieMode::default(), PreviewTieMode::NeutralSeed);
        let _ = (
            None::<Bps>,
            PrivateMandate {
                kind: MandateKind::MinProjectEquity { target: 1_000 },
                bonus_equity: 3_000,
            },
        );
    }

    #[test]
    fn view_model_types_and_key_events_cover_plain_owned_helpers() {
        let style = ViewStyle::plain()
            .with_fg(ViewColor::Cyan)
            .with_bg(ViewColor::Rgb {
                r: 10,
                g: 20,
                b: 30,
            })
            .bold()
            .dim()
            .underlined()
            .reversed();
        assert_eq!(style.fg, ViewColor::Cyan);
        assert!(style.bold && style.dim && style.underlined && style.reversed);

        let row = ViewRow::from_spans(vec![
            ViewSpan::plain("CRED: ".to_owned()),
            ViewSpan::styled("15000".to_owned(), style),
        ]);
        assert_eq!(row.plain_text(), "CRED: 15000");

        let tabs = ViewTabBar {
            tabs: vec![
                ViewTabItem {
                    label: ViewLabel::plain("Market".to_owned()),
                    enabled: true,
                },
                ViewTabItem {
                    label: ViewLabel::plain("Workshop".to_owned()),
                    enabled: true,
                },
            ],
            active_index: 0,
        };
        assert_eq!(tabs.plain_text(), "[Market] | Workshop");

        let table = ViewTable {
            columns: vec![
                ViewTableColumn {
                    header: ViewLabel::plain("Seat".to_owned()),
                    width: ViewSizeHint::Fixed(6),
                    alignment: ViewAlign::Left,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("CRED".to_owned()),
                    width: ViewSizeHint::Ratio { num: 1, den: 1 },
                    alignment: ViewAlign::Right,
                },
            ],
            rows: vec![ViewTableRow {
                cells: vec![
                    ViewTableCell::plain("S0".to_owned()),
                    ViewTableCell::aligned("15000".to_owned(), ViewAlign::Right, style),
                ],
                style: ViewStyle::plain(),
            }],
            selected_row: Some(0),
        };
        assert_eq!(table.plain_text(), "Seat | CRED\nS0 | 15000");

        let panel = ViewPanel {
            title: Some(ViewLabel::styled("Header".to_owned(), style)),
            alignment: ViewAlign::Center,
            bordered: true,
            tabs: Some(tabs),
            table: Some(table),
            rows: vec![ViewRow::from_plain("Row 1".to_owned()), row],
            selected_row: Some(0),
            scroll: Some(ViewScroll {
                offset: 0,
                total: 2,
            }),
        };
        let panel_txt = panel.plain_text();
        assert!(panel_txt.contains("[Market] | Workshop"));
        assert!(panel_txt.contains("Seat | CRED\nS0 | 15000"));
        assert!(panel_txt.contains("Row 1\nCRED: 15000"));

        let vp = ViewportSize::new(100, 30);
        let wide = ScreenLayout::layout_for_viewport(vp);
        assert_eq!(wide.mode, ViewLayoutMode::FourZoneWide);
        assert_eq!(wide.bands.len(), 3);
        assert_eq!(wide.bands[1].axis, ViewAxis::Horizontal);
        assert_eq!(wide.bands[1].zones.len(), 3);

        let stacked = ScreenLayout::layout_for_viewport(ViewportSize::new(80, 24));
        assert_eq!(stacked.mode, ViewLayoutMode::StackedFallback);
        assert_eq!(stacked.bands.len(), 5);

        let small = ScreenLayout::layout_for_viewport(ViewportSize::new(79, 24));
        assert_eq!(small.mode, ViewLayoutMode::TooSmall);
        assert_eq!(small.bands.len(), 1);
        assert_eq!(small.bands[0].zones[0].zone, ViewZoneId::SingleFull);

        let view = SeatScreenView {
            layout: wide,
            zones: vec![
                ViewZonePanel {
                    zone: ViewZoneId::HeaderBar,
                    panel: ViewPanel {
                        title: Some(ViewLabel::plain("Top".to_owned())),
                        ..ViewPanel::default()
                    },
                },
                ViewZonePanel {
                    zone: ViewZoneId::CenterTabs,
                    panel: panel.clone(),
                },
            ],
            status_line: Some(ViewStatusLine {
                alert: Some(ViewRow::from_plain(
                    "REFUSED [SAMPLE]: check alert".to_owned(),
                )),
                left: vec![ViewSpan::plain("Keys".to_owned())],
                right: vec![ViewSpan::plain("Ready".to_owned())],
                style: ViewStyle::plain(),
            }),
            modal: Some(ViewModal {
                width_pct: 80,
                height_pct: 80,
                panel: ViewPanel {
                    title: Some(ViewLabel::plain("Chronicle".to_owned())),
                    rows: vec![ViewRow::from_plain("Body".to_owned())],
                    ..ViewPanel::default()
                },
            }),
        };
        assert!(view.zone(ViewZoneId::HeaderBar).is_some());
        assert!(view.zone(ViewZoneId::PrivateDrawer).is_none());
        let flat = view.plain_text();
        assert!(flat.contains("Top"));
        assert!(flat.contains("Header"));
        assert!(flat.contains("Row 1\nCRED: 15000"));
        assert!(flat.contains("REFUSED [SAMPLE]: check alert"));
        assert!(flat.contains("Keys | Ready"));
        assert!(flat.contains("Chronicle\nBody"));

        let single_status = ScreenLayout::single_panel_with_status(ViewportSize::new(90, 25));
        assert_eq!(single_status.mode, ViewLayoutMode::SinglePanel);
        assert_eq!(single_status.bands.len(), 2);

        let rep = SeatKeyEvent::repeated(SeatKeyCode::Down, SeatKeyModifiers::NONE);
        assert!(rep.is_repeat);
        assert!(!SeatKeyEvent::new(SeatKeyCode::Down, SeatKeyModifiers::NONE).is_repeat);

        let single = SeatScreenView::single_panel(panel);
        assert_eq!(single.layout.mode, ViewLayoutMode::SinglePanel);
        assert_eq!(single.zones.len(), 1);
        assert!(single.status_line.is_none());
        assert!(single.modal.is_none());

        let ev_q = SeatKeyEvent::new(SeatKeyCode::Char('q'), SeatKeyModifiers::NONE);
        let ev_ctrl = SeatKeyEvent::new(SeatKeyCode::Char('c'), SeatKeyModifiers::CTRL);
        assert_eq!(ev_q.code, SeatKeyCode::Char('q'));
        assert!(!ev_q.modifiers.ctrl);
        assert!(ev_ctrl.modifiers.ctrl);
        assert_eq!(SeatKeyEvent::default().code, SeatKeyCode::Unmapped);
        assert_eq!(ViewColor::default().to_owned(), ViewColor::Reset);
        assert_eq!(ViewLabel::plain("ok".to_owned()).text.to_string(), "ok");
    }

    #[test]
    fn seat_intent_and_bundle_draft_actions_cover_all_bundle_fields() {
        let mut bundle = SealedOrderBundle::default();

        BundleDraftAction::SetConverterRun(ConverterRunOrder {
            recipe_idx: 0,
            runs: 2,
        })
        .apply_to(&mut bundle);
        BundleDraftAction::SetConverterRun(ConverterRunOrder {
            recipe_idx: 0,
            runs: 3,
        })
        .apply_to(&mut bundle);
        assert_eq!(bundle.converter_runs.len(), 1);
        assert_eq!(bundle.converter_runs[0].runs, 3);
        BundleDraftAction::SetConverterRun(ConverterRunOrder {
            recipe_idx: 0,
            runs: 0,
        })
        .apply_to(&mut bundle);
        assert!(bundle.converter_runs.is_empty());
        BundleDraftAction::SetConverterRun(ConverterRunOrder {
            recipe_idx: 1,
            runs: 1,
        })
        .apply_to(&mut bundle);
        BundleDraftAction::ClearConverterRuns.apply_to(&mut bundle);
        assert!(bundle.converter_runs.is_empty());

        BundleDraftAction::SetProjectDelivery(ProjectDeliveryOrder {
            project_id: 1,
            batches: 2,
        })
        .apply_to(&mut bundle);
        BundleDraftAction::SetProjectDelivery(ProjectDeliveryOrder {
            project_id: 1,
            batches: 4,
        })
        .apply_to(&mut bundle);
        assert_eq!(bundle.project_deliveries[0].batches, 4);
        BundleDraftAction::SetProjectDelivery(ProjectDeliveryOrder {
            project_id: 1,
            batches: 0,
        })
        .apply_to(&mut bundle);
        assert!(bundle.project_deliveries.is_empty());
        BundleDraftAction::SetProjectDelivery(ProjectDeliveryOrder {
            project_id: 2,
            batches: 1,
        })
        .apply_to(&mut bundle);
        BundleDraftAction::ClearProjectDeliveries.apply_to(&mut bundle);
        assert!(bundle.project_deliveries.is_empty());

        let swap = AtomicSwapProposal {
            swap_id: 9,
            maker: 0,
            taker: 1,
            maker_asset: AssetId::Raw(0),
            maker_amount: 500,
            taker_asset: AssetId::Refined(1),
            taker_amount: 500,
            domain_id: None,
        };
        BundleDraftAction::StageAtomicSwap(swap.clone()).apply_to(&mut bundle);
        BundleDraftAction::StageAtomicSwap(swap).apply_to(&mut bundle);
        assert_eq!(bundle.atomic_swaps.len(), 1);
        BundleDraftAction::ClearAtomicSwaps(Some(0)).apply_to(&mut bundle);
        assert!(bundle.atomic_swaps.is_empty());

        let cd = ContractDeliveryOrder {
            counterparty: 1,
            good: AssetId::Refined(0),
            gross_amount: 500,
            domain_id: None,
            dual_escrow_id: None,
        };
        BundleDraftAction::StageContractDelivery(cd).apply_to(&mut bundle);
        assert_eq!(bundle.contract_deliveries.len(), 1);
        BundleDraftAction::ClearContractDeliveries(None).apply_to(&mut bundle);
        assert!(bundle.contract_deliveries.is_empty());

        let dual = DualEscrowProposal {
            proposal_id: 4,
            buyer: 0,
            seller: 1,
            good: AssetId::Refined(1),
            required_units: 1_000,
            payment_cred: 2_000,
            buyer_bond_cred: 500,
            seller_bond_cred: 500,
            deadline_turn: 4,
            domain_id: None,
        };
        BundleDraftAction::StageDualEscrow(dual).apply_to(&mut bundle);
        assert_eq!(bundle.dual_escrows.len(), 1);
        BundleDraftAction::ClearDualEscrows(Some(0)).apply_to(&mut bundle);
        assert!(bundle.dual_escrows.is_empty());

        BundleDraftAction::SetDebtRepaymentCred(500).apply_to(&mut bundle);
        assert_eq!(bundle.debt_repayment_cred, 500);

        let unlock = EscrowFinishUnlock {
            escrow_id: 1,
            preimage: Some(PreimageSecret([0x11; 32])),
        };
        BundleDraftAction::StageEscrowFinish(unlock).apply_to(&mut bundle);
        assert_eq!(bundle.escrow_finishes.len(), 1);
        BundleDraftAction::ClearEscrowFinishes(Some(0)).apply_to(&mut bundle);
        assert!(bundle.escrow_finishes.is_empty());

        BundleDraftAction::StageCheckCash(3).apply_to(&mut bundle);
        BundleDraftAction::StageCheckCash(3).apply_to(&mut bundle);
        assert_eq!(bundle.check_cashes, vec![3]);
        BundleDraftAction::ClearCheckCashes(Some(3)).apply_to(&mut bundle);
        assert!(bundle.check_cashes.is_empty());

        BundleDraftAction::SetCouncilVote(Some(CouncilVoteBallot {
            trade_minister: 0,
            cartel_chair: 1,
        }))
        .apply_to(&mut bundle);
        assert!(bundle.council_vote.is_some());

        BundleDraftAction::StageCharterBid(CharterBidOrder {
            role: CharterRole::OracleAuditor,
            bid_cred: 250,
        })
        .apply_to(&mut bundle);
        BundleDraftAction::StageCharterBid(CharterBidOrder {
            role: CharterRole::OracleAuditor,
            bid_cred: 300,
        })
        .apply_to(&mut bundle);
        assert_eq!(bundle.charter_bids.len(), 1);
        assert_eq!(bundle.charter_bids[0].bid_cred, 300);
        BundleDraftAction::StageCharterBid(CharterBidOrder {
            role: CharterRole::OracleAuditor,
            bid_cred: 0,
        })
        .apply_to(&mut bundle);
        assert!(bundle.charter_bids.is_empty());

        BundleDraftAction::ClearAll.apply_to(&mut bundle);
        assert_eq!(bundle, SealedOrderBundle::default());

        let intents = vec![
            SeatIntent::submit_market_action(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(1),
                    amount: 500,
                    price_per_unit_cred: 1_000,
                    domain_id: None,
                    is_hybrid: false,
                },
            }),
            SeatIntent::update_bundle(BundleDraftAction::ClearCharterBids),
            SeatIntent::request_what_if(HumanWhatIfRequest {
                seat: 0,
                market_actions: Vec::new(),
                cancel_lock_at: None,
                bundle: SealedOrderBundle::default(),
                tie_mode: PreviewTieMode::NeutralSeed,
                assumptions: CounterpartyAssumptions::default(),
            }),
            SeatIntent::lock_and_hand_off(),
            SeatIntent::open_chronicle_modal(),
            SeatIntent::close_chronicle_modal(),
            SeatIntent::quit(),
        ];
        assert_eq!(intents[0].kind, SeatIntentKind::SubmitMarketAction);
        assert_eq!(intents[1].kind, SeatIntentKind::UpdateBundle);
        assert_eq!(intents[2].kind, SeatIntentKind::RequestWhatIf);
        assert_eq!(intents[3].kind, SeatIntentKind::LockAndHandOff);
        assert_eq!(intents[4].kind, SeatIntentKind::OpenChronicleModal);
        assert_eq!(intents[5].kind, SeatIntentKind::CloseChronicleModal);
        assert_eq!(intents[6].kind, SeatIntentKind::Quit);
        let encoded = match serde_json::to_string(&intents) {
            Ok(s) => s,
            Err(err) => panic!("encode intents failed: {err}"),
        };
        let decoded: Vec<SeatIntent> = match serde_json::from_str(&encoded) {
            Ok(v) => v,
            Err(err) => panic!("decode intents failed: {err}"),
        };
        assert_eq!(decoded, intents);

        let receipt = PublicSweepReceipt {
            turn: 1,
            non_reveal_seats: vec![],
            regent_acted_seats: vec![],
            converted_to_ai: vec![],
            escrows_finished: vec![],
            escrows_cancelled_or_forfeited: vec![],
            atomic_swaps_settled: 0,
            converter_batches_run: 0,
            converter_orders_clamped: 0,
            checks_cashed: vec![],
            checks_bounced: vec![],
            cartel_evictions: vec![],
            projects_completed: 0,
            project_orders_clamped: 0,
            total_upkeep_shortfall_debt: 0,
            total_spoilage_clawed_back: 0,
            broken_promises: vec![],
            elected_officers: None,
            charters_awarded: vec![],
            public_forecasts: vec![],
            newly_activated_shocks: vec![],
            cred_debt_repaid: 0,
            charter_cred_burned: 0,
            town_import_cred_spent: 0,
            works_stipend_cred_minted: 0,
            batch_trades_cleared: 0,
            fills: vec![],
            wash_freeze_settlements: vec![],
            season_closed: false,
            pbrs_deltas: vec![0; 4],
        };
        let json_res = receipt.to_json_string();
        assert!(json_res.is_ok());
    }

    #[test]
    fn debug_and_pretty_debug_redact_all_private_seat_types() {
        let pm = PrivateMandate {
            kind: MandateKind::MinProjectEquity { target: 987_654 },
            bonus_equity: 7_777,
        };
        let bundle = SealedOrderBundle {
            converter_runs: vec![ConverterRunOrder {
                recipe_idx: 2,
                runs: 9,
            }],
            project_deliveries: vec![ProjectDeliveryOrder {
                project_id: 42,
                batches: 3,
            }],
            atomic_swaps: vec![],
            contract_deliveries: vec![],
            dual_escrows: vec![],
            debt_repayment_cred: 654_321,
            escrow_finishes: vec![EscrowFinishUnlock {
                escrow_id: 88,
                preimage: Some(PreimageSecret([0x5A; 32])),
            }],
            check_cashes: vec![55],
            council_vote: None,
            charter_bids: vec![],
        };
        let draft_act = BundleDraftAction::SetDebtRepaymentCred(765_432);
        let intent_bundle = SeatIntent::update_bundle(draft_act.clone());
        let what_if_req = HumanWhatIfRequest {
            seat: 0,
            market_actions: vec![MarketAction::TownImport {
                seat: 0,
                good: AssetId::Raw(0),
                amount: 876_543,
            }],
            cancel_lock_at: None,
            bundle: bundle.clone(),
            tie_mode: PreviewTieMode::NeutralSeed,
            assumptions: CounterpartyAssumptions {
                assumed_cancelled_offer_ids: vec![432_109],
                assumed_peer_market_actions: vec![],
                assumed_peer_bundles: BTreeMap::new(),
                assumed_cosigned_swaps: vec![],
                assumed_cosigned_dual_escrows: vec![],
                assumed_peer_deliveries: vec![],
                assumed_peer_escrow_finishes: vec![],
                assumed_bounced_check_ids: vec![321_987],
            },
        };
        let intent_whatif = SeatIntent::request_what_if(what_if_req.clone());
        let ai_req = crate::AiPreviewRequest {
            seat: 1,
            market_actions: vec![],
            bundle: bundle.clone(),
        };

        let seat_cfg = SeatConfig {
            seat: 0,
            identity: SeatIdentity::Human,
            initial_cred: 15_000,
            raw_yield_per_turn: 10_000,
            refined_project_value: 2_000,
            upkeep_good: 0,
            upkeep_amount: 1_800,
            initial_raw_tariff_bps: 800,
            initial_refined_tariff_bps: 600,
            perishable_decay_bps: 2_000,
            regent: None,
            converters: Vec::new(),
        };
        let spec = PublicTableSpec {
            table_id: TableId(55),
            mode: TableMode::Club,
            seat_count: 1,
            seats: vec![seat_cfg],
            projects: Vec::new(),
            closing_hazard_bps: [3_500; 8],
            mandate_deck: Vec::new(),
            drand_schedule: None,
            eval_vrf_pubkey: None,
            town_import_premium_bps: 22_500,
            works_stipend_per_unit_cred: 350,
            mandate_alpha_bps: 250,
            cancel_lock_bps: 2_500,
            wash_round_trip_min_units: 1_000,
            wash_freeze_bond_cred: 500,
            wash_freeze_bounty_cred: 500,
        };
        let preds = SeatStatePredicates {
            version: STATE_PREDICATES_VERSION,
            table_id: TableId(55),
            turn: 2,
            phase: TurnPhase::TalkAndMarket,
            seat: 0,
            upkeep: UpkeepAndDebt {
                upkeep_good: 0,
                owed_units: 1_800,
                held_units: 0,
                held_on_own_resting_sells: 0,
                shortfall_units: 1_800,
                debt_added_at_sweep: 3_600,
                interest_bps: 400,
            },
            spoilage: SpoilageAndConversion {
                own_raw_good: 0,
                own_raw_yield_at_sweep: 10_000,
                spoilage_bps_by_good: vec![2_000],
                spoil_units_held_by_good: vec![0],
                spoil_units_on_resting_offers: 0,
                spoil_units_total: 0,
                workshop_capacity_runs_left: 0,
                recipes: Vec::new(),
            },
            scarcity: Vec::new(),
            coalitions: Vec::new(),
            season_end: SeasonEndAndMarks {
                season_closed: false,
                turns_until_earliest_close: 23,
                turns_until_latest_close: 31,
                holdings: Vec::new(),
                projects: Vec::new(),
            },
            cancel_locked: false,
        };
        let obs = SeatObservation::unmandated(SeatObservationPublicParts {
            seat: 0,
            table_id: TableId(55),
            turn: 1,
            phase: TurnPhase::TalkAndMarket,
            cancel_locked: false,
            trade_minister: 0,
            cartel_chair: None,
            spec: spec.clone(),
            parities: FundamentalParities::from_spec(&spec),
            seats: vec![SeatState {
                seat: 0,
                identity: SeatIdentity::Human,
                cred: 15_000,
                cred_debt: 0,
                raw: vec![0],
                refined: vec![0],
                project_equity: 0,
                raw_tariff_bps: 800,
                refined_tariff_bps: 600,
                credit_tier: CreditTier::Aaa,
                hard_credit_defaults: 0,
                bond_breaches: 0,
                broken_promises: 0,
                honor_penalties: 0,
                consecutive_misses: 0,
                consecutive_zero_production_turns: 0,
                wash_frozen_until_turn: 0,
            }],
            offers: BTreeMap::new(),
            checks: BTreeMap::new(),
            escrows: BTreeMap::new(),
            dual_escrows: BTreeMap::new(),
            pledges: BTreeMap::new(),
            audited_pledges: BTreeSet::new(),
            domains: Vec::new(),
            council_platforms: BTreeMap::new(),
            charters: Vec::new(),
            active_shocks: Vec::new(),
            public_forecasts: Vec::new(),
            auditor_hints: Vec::new(),
            next_offer_id: 1,
            next_check_id: 1,
            next_escrow_id: 1,
            delivery_ledger: BTreeMap::new(),
            last_potentials: vec![15_000],
            pending_town_import_cred_spent: 0,
            fill_log: Vec::new(),
            pending_wash_freezes: Vec::new(),
            eval_vrf_pubkey: None,
            baseline_predicates: preds,
            public_receipts: Vec::new(),
        });

        let what_if_preview = WhatIfPreview {
            seat: 0,
            turn: 1,
            baseline_predicates: obs.baseline_predicates.clone(),
            pre_seat_state: obs.seats[0].clone(),
            post_market_seat_state: obs.seats[0].clone(),
            post_sweep_seat_state: obs.seats[0].clone(),
            post_sweep_predicates: obs.baseline_predicates.clone(),
            close_mark_caveat: "PLANTED_CAVEAT_918273".to_owned(),
            refused_market_actions: vec![RefusedActionPreview {
                action_index: 0,
                error_code: "REFUSED".to_owned(),
                message: "PLANTED_REFUSAL_827364".to_owned(),
            }],
            converter_orders: Vec::new(),
            project_orders: Vec::new(),
            swap_orders: Vec::new(),
            check_orders: Vec::new(),
            escrow_orders: Vec::new(),
            upkeep: SeatUpkeepPreview {
                upkeep_good: 0,
                upkeep_required: 1_800,
                paid_with_refined: 0,
                debt_interest_accrued: 0,
                new_shortfall_debt: 3_600,
                debt_repaid_cred: 765_432,
            },
            spoilage: SeatSpoilagePreview {
                perishable_spoiled_units: 0,
            },
            public_fills: Vec::new(),
            step0_to_7_receipt: Step0To7SweepSummary {
                turn: 1,
                non_reveal_seats: Vec::new(),
                regent_acted_seats: Vec::new(),
                converted_to_ai: Vec::new(),
                escrows_finished: Vec::new(),
                escrows_cancelled_or_forfeited: Vec::new(),
                atomic_swaps_settled: 0,
                converter_batches_run: 0,
                converter_orders_clamped: 0,
                checks_cashed: Vec::new(),
                checks_bounced: Vec::new(),
                cartel_evictions: Vec::new(),
                projects_completed: 0,
                project_orders_clamped: 0,
                total_upkeep_shortfall_debt: 0,
                total_spoilage_clawed_back: 0,
                broken_promises: Vec::new(),
                elected_officers: None,
                charters_awarded: Vec::new(),
                cred_debt_repaid: 765_432,
                charter_cred_burned: 0,
                town_import_cred_spent: 0,
                works_stipend_cred_minted: 0,
                batch_trades_cleared: 0,
                fills: Vec::new(),
                wash_freeze_settlements: Vec::new(),
            },
        };
        let label_title = ViewLabel::plain("PLANTED_VIEW_TITLE_645312".to_owned());
        let span_item = ViewSpan::plain("PLANTED_VIEW_SPAN_534219".to_owned());
        let row_note = ViewRow::from_spans(vec![
            ViewSpan::plain("PLANTED_VIEW_MANDATE_645312".to_owned()),
            span_item.clone(),
        ]);
        let tab_item = ViewTabItem {
            label: ViewLabel::plain("PLANTED_VIEW_TAB_246810".to_owned()),
            enabled: true,
        };
        let tab_bar = ViewTabBar {
            tabs: vec![tab_item.clone()],
            active_index: 0,
        };
        let table_col = ViewTableColumn {
            header: ViewLabel::plain("PLANTED_VIEW_COL_135791".to_owned()),
            width: ViewSizeHint::Min(10),
            alignment: ViewAlign::Left,
        };
        let table_cell = ViewTableCell::plain("PLANTED_VIEW_ORDER_765432".to_owned());
        let table_row = ViewTableRow {
            cells: vec![table_cell.clone()],
            style: ViewStyle::plain(),
        };
        let view_table = ViewTable {
            columns: vec![table_col.clone()],
            rows: vec![table_row.clone()],
            selected_row: Some(0),
        };
        let vault_panel = ViewPanel {
            title: Some(label_title.clone()),
            tabs: Some(tab_bar.clone()),
            table: Some(view_table.clone()),
            rows: vec![
                row_note.clone(),
                ViewRow::from_plain("PLANTED_VIEW_NOTE_534219".to_owned()),
            ],
            ..ViewPanel::default()
        };
        let zone_panel = ViewZonePanel {
            zone: ViewZoneId::PrivateDrawer,
            panel: vault_panel.clone(),
        };
        let view_modal = ViewModal {
            width_pct: 80,
            height_pct: 60,
            panel: ViewPanel {
                title: Some(ViewLabel::plain("PLANTED_VIEW_MODAL_908172".to_owned())),
                rows: vec![ViewRow::from_plain("PLANTED_VIEW_ORDER_765432".to_owned())],
                ..ViewPanel::default()
            },
        };
        let status_line = ViewStatusLine {
            alert: Some(ViewRow::from_plain("PLANTED_VIEW_ALERT_112233".to_owned())),
            left: vec![ViewSpan::plain("PLANTED_VIEW_LEFT_445566".to_owned())],
            right: vec![ViewSpan::plain("PLANTED_VIEW_RIGHT_778899".to_owned())],
            style: ViewStyle::plain(),
        };
        let screen_view = SeatScreenView {
            layout: ScreenLayout::layout_for_viewport(ViewportSize::new(100, 30)),
            zones: vec![zone_panel.clone()],
            status_line: Some(status_line.clone()),
            modal: Some(view_modal.clone()),
        };

        let markers = [
            "987654",
            "7777",
            "654321",
            "765432",
            "876543",
            "432109",
            "321987",
            "5a5a5a",
            "5A5A5A",
            "90, 90, 90",
            "PLANTED_CAVEAT_918273",
            "PLANTED_REFUSAL_827364",
            "PLANTED_VIEW_TITLE_645312",
            "PLANTED_VIEW_SPAN_534219",
            "PLANTED_VIEW_MANDATE_645312",
            "PLANTED_VIEW_TAB_246810",
            "PLANTED_VIEW_COL_135791",
            "PLANTED_VIEW_NOTE_534219",
            "PLANTED_VIEW_ORDER_765432",
            "PLANTED_VIEW_MODAL_908172",
            "PLANTED_VIEW_ALERT_112233",
            "PLANTED_VIEW_LEFT_445566",
            "PLANTED_VIEW_RIGHT_778899",
        ];
        let debug_strings = [
            alloc::format!("{pm:?}"),
            alloc::format!("{pm:#?}"),
            alloc::format!("{bundle:?}"),
            alloc::format!("{bundle:#?}"),
            alloc::format!("{draft_act:?}"),
            alloc::format!("{draft_act:#?}"),
            alloc::format!("{intent_bundle:?}"),
            alloc::format!("{intent_bundle:#?}"),
            alloc::format!("{:?}", what_if_req.assumptions),
            alloc::format!("{:#?}", what_if_req.assumptions),
            alloc::format!("{what_if_req:?}"),
            alloc::format!("{what_if_req:#?}"),
            alloc::format!("{intent_whatif:?}"),
            alloc::format!("{intent_whatif:#?}"),
            alloc::format!("{ai_req:?}"),
            alloc::format!("{ai_req:#?}"),
            alloc::format!("{obs:?}"),
            alloc::format!("{obs:#?}"),
            alloc::format!("{what_if_preview:?}"),
            alloc::format!("{what_if_preview:#?}"),
            alloc::format!("{screen_view:?}"),
            alloc::format!("{screen_view:#?}"),
            alloc::format!("{:?}", screen_view.zones),
            alloc::format!("{:#?}", screen_view.zones),
            alloc::format!("{zone_panel:?}"),
            alloc::format!("{zone_panel:#?}"),
            alloc::format!("{vault_panel:?}"),
            alloc::format!("{vault_panel:#?}"),
            alloc::format!("{label_title:?}"),
            alloc::format!("{label_title:#?}"),
            alloc::format!("{span_item:?}"),
            alloc::format!("{span_item:#?}"),
            alloc::format!("{row_note:?}"),
            alloc::format!("{row_note:#?}"),
            alloc::format!("{tab_item:?}"),
            alloc::format!("{tab_item:#?}"),
            alloc::format!("{tab_bar:?}"),
            alloc::format!("{tab_bar:#?}"),
            alloc::format!("{table_col:?}"),
            alloc::format!("{table_col:#?}"),
            alloc::format!("{table_cell:?}"),
            alloc::format!("{table_cell:#?}"),
            alloc::format!("{table_row:?}"),
            alloc::format!("{table_row:#?}"),
            alloc::format!("{view_table:?}"),
            alloc::format!("{view_table:#?}"),
            alloc::format!("{view_modal:?}"),
            alloc::format!("{view_modal:#?}"),
            alloc::format!("{status_line:?}"),
            alloc::format!("{status_line:#?}"),
        ];
        let swap_leg = SwapLeg {
            asset: AssetId::Raw(0),
            units: 765_432,
        };
        let intent_hdr = IntentHeader::SpotOffer {
            side: OfferSide::Buy,
            good: AssetId::Refined(1),
            whole_units: 7777,
            unit_price_cred: 654_321,
            domain_id: Some(2),
        };
        let intent_debugs = [
            alloc::format!("{swap_leg:?}"),
            alloc::format!("{swap_leg:#?}"),
            alloc::format!("{intent_hdr:?}"),
            alloc::format!("{intent_hdr:#?}"),
        ];
        for text in debug_strings.iter().chain(intent_debugs.iter()) {
            assert!(
                text.contains("<redacted>") || text.contains("own_mandate: None"),
                "expected <redacted> or own_mandate: None marker in Debug:\n{text}"
            );
            for m in &markers {
                assert!(
                    !text.contains(m),
                    "planted secret marker `{m}` leaked into Debug:\n{text}"
                );
            }
        }
    }

    #[test]
    fn intent_header_five_variants_round_trip_validate_and_reject_unknowns() {
        let cond = CommitmentHash([0xAB; 32]);
        let valid_headers = [
            IntentHeader::SpotOffer {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                whole_units: 3,
                unit_price_cred: 2_100,
                domain_id: Some(1),
            },
            IntentHeader::AtomicSwap {
                give_legs: vec![SwapLeg {
                    asset: AssetId::Raw(0),
                    units: 1_000,
                }],
                receive_legs: vec![SwapLeg {
                    asset: AssetId::Refined(1),
                    units: 1_500,
                }],
                swap_id: Some(9),
                domain_id: None,
            },
            IntentHeader::EscrowDeal {
                amount_cred: 4_000,
                finish_after_turn: Some(2),
                cancel_after_turn: 6,
                condition_hex: Some(cond),
                bond: Some(BondProposal {
                    good: AssetId::Refined(1),
                    required_units: 1_000,
                    deadline_turn: 5,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            },
            IntentHeader::CreditCheck {
                send_max_cred: 2_000,
                deliver_min_cred: Some(1_200),
                due_turn: Some(3),
                expiration_turn: 6,
            },
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::VoteForSeat {
                    office_turn: 4,
                    candidate: 2,
                },
                honor_weight: Some(500),
            },
        ];

        let expected_names = [
            "SpotOffer",
            "AtomicSwap",
            "EscrowDeal",
            "CreditCheck",
            "DiplomaticPledge",
        ];
        for (hdr, exp_name) in valid_headers.iter().zip(expected_names.iter()) {
            assert_eq!(hdr.variant_name(), *exp_name);
            assert!(hdr.validate(4).is_ok());
            let json =
                serde_json::to_string(hdr).unwrap_or_else(|e| panic!("serialize {exp_name}: {e}"));
            let rt: IntentHeader = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("deserialize {exp_name}: {e}"));
            assert_eq!(&rt, hdr);
        }

        // Unknown 6th variant, arbitrary JSON map, and extra fields must fail closed.
        for bad_json in [
            r#"{"OracleHint":{"turn":3}}"#,
            r#"{"freeform":"map","price":100}"#,
            r#"{"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000,"extra":1}}"#,
            r#"{"AtomicSwap":{"give_legs":[{"asset":"Cred","units":1000,"extra":1}],"receive_legs":[{"asset":{"Raw":1},"units":1000}]}}"#,
        ] {
            assert!(
                serde_json::from_str::<IntentHeader>(bad_json).is_err(),
                "expected rejection for {bad_json}"
            );
        }

        // Semantic validation refusals.
        assert!(
            IntentHeader::SpotOffer {
                side: OfferSide::Sell,
                good: AssetId::Cred,
                whole_units: 1,
                unit_price_cred: 1_000,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::SpotOffer {
                side: OfferSide::Buy,
                good: AssetId::Raw(4),
                whole_units: 1,
                unit_price_cred: 1_000,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::SpotOffer {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                whole_units: 0,
                unit_price_cred: 1_000,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::AtomicSwap {
                give_legs: Vec::new(),
                receive_legs: vec![SwapLeg {
                    asset: AssetId::Cred,
                    units: 1_000,
                }],
                swap_id: None,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: Some(6),
                cancel_after_turn: 6,
                condition_hex: Some(cond),
                bond: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: None,
                bond: Some(BondProposal {
                    good: AssetId::Raw(0),
                    required_units: 1_000,
                    deadline_turn: 5,
                    mode: BondSettlementMode::SellerPerformanceBond,
                }),
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: Some(CommitmentHash([0u8; 32])),
                bond: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::CreditCheck {
                send_max_cred: 500,
                deliver_min_cred: Some(600),
                due_turn: Some(2),
                expiration_turn: 4,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::RevealPreimage {
                    escrow_id: 0,
                    condition: cond,
                },
                honor_weight: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::VoteForSeat {
                    office_turn: 4,
                    candidate: 4,
                },
                honor_weight: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::SpotOffer {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                whole_units: 1,
                unit_price_cred: 1_000,
                domain_id: None,
            }
            .validate(0)
            .is_err()
        );
        assert!(
            IntentHeader::SpotOffer {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                whole_units: 1,
                unit_price_cred: 0,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::AtomicSwap {
                give_legs: vec![SwapLeg {
                    asset: AssetId::Cred,
                    units: 1_000,
                }],
                receive_legs: Vec::new(),
                swap_id: None,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::AtomicSwap {
                give_legs: vec![SwapLeg {
                    asset: AssetId::Cred,
                    units: 0,
                }],
                receive_legs: vec![SwapLeg {
                    asset: AssetId::Raw(0),
                    units: 1_000,
                }],
                swap_id: None,
                domain_id: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 0,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: Some(cond),
                bond: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 0,
                condition_hex: Some(cond),
                bond: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: None,
                bond: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: None,
                bond: Some(BondProposal {
                    good: AssetId::Cred,
                    required_units: 1_000,
                    deadline_turn: 5,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: None,
                bond: Some(BondProposal {
                    good: AssetId::Refined(0),
                    required_units: 0,
                    deadline_turn: 5,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::EscrowDeal {
                amount_cred: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 6,
                condition_hex: None,
                bond: Some(BondProposal {
                    good: AssetId::Refined(0),
                    required_units: 1_000,
                    deadline_turn: 0,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::CreditCheck {
                send_max_cred: 0,
                deliver_min_cred: None,
                due_turn: None,
                expiration_turn: 4,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::CreditCheck {
                send_max_cred: 500,
                deliver_min_cred: None,
                due_turn: None,
                expiration_turn: 0,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::CreditCheck {
                send_max_cred: 500,
                deliver_min_cred: Some(300),
                due_turn: Some(5),
                expiration_turn: 4,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::VoteForSeat {
                    office_turn: 0,
                    candidate: 1,
                },
                honor_weight: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::VoteForSeat {
                    office_turn: 4,
                    candidate: 1,
                },
                honor_weight: Some(-1),
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::PreserveCartelMembership {
                    office_turn: 0,
                    domain_id: 1,
                    beneficiary: 1,
                },
                honor_weight: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::PreserveCartelMembership {
                    office_turn: 4,
                    domain_id: 1,
                    beneficiary: 4,
                },
                honor_weight: None,
            }
            .validate(4)
            .is_err()
        );
        assert!(
            IntentHeader::DiplomaticPledge {
                target: IntentTarget::RevealPreimage {
                    escrow_id: 1,
                    condition: CommitmentHash([0u8; 32]),
                },
                honor_weight: None,
            }
            .validate(4)
            .is_err()
        );
    }
}
