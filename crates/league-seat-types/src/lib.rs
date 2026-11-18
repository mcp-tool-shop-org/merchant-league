#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_macros)]
//! Seat-safe value, observation, preview, error, and commodity display types for Merchant League v2.
//!
//! This leaf crate depends solely on `serde`, `serde_json`, and `league-error` — never on
//! `league-core`, `league-gen`, or `league-standalone` — so `rustc` enforces the host/seat type
//! boundary at compile time (`docs/club-mode-design.md` §2.3).

mod commit;
mod contracts;
mod display;
mod engine_types;
mod observation;
mod predicates_types;
mod scoring_types;
mod serde_helpers;
mod types;

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
pub use league_error::{StructuredError, ToStructuredError, scrub_secret_text};
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

/// Internal serde helpers re-exported within `crates/` via explicit crate usage where identical
/// `EngineState` / `SeasonReplayLog` JSON encodings are shared.
pub mod wire_serde {
    use std::collections::BTreeMap;

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
    use super::*;
    use serde::{Deserialize, Serialize};

    fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
        match res {
            Ok(v) => v,
            Err(err) => panic!("{ctx}: {err}"),
        }
    }

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
    fn wire_serde_round_trips_and_rejects_invalid_inputs() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Holder {
            #[serde(
                serialize_with = "wire_serde::serialize_hex32",
                deserialize_with = "wire_serde::deserialize_hex32"
            )]
            fixed: [u8; 32],
        }
        let h_val = Holder { fixed: [0x11; 32] };
        let h_json = must_ok(serde_json::to_string(&h_val), "holder json");
        assert_eq!(
            must_ok(serde_json::from_str::<Holder>(&h_json), "holder back"),
            h_val
        );
        // Exercise direct deserialize_hex32 / opt_hex32 / delivery_ledger errors too.
        let p = PreimageSecret([0x1F; 32]);
        let j = must_ok(serde_json::to_string(&p), "preimage json");
        let parsed: PreimageSecret = must_ok(serde_json::from_str(&j), "preimage parse");
        assert_eq!(parsed, p);
        assert!(serde_json::from_str::<PreimageSecret>("\"abc\"").is_err());
        assert!(
            serde_json::from_str::<PreimageSecret>(&format!("\"{}\"", "00".repeat(31))).is_err()
        );
        assert!(
            serde_json::from_str::<PreimageSecret>(&format!("\"{}\"", "zz".repeat(32))).is_err()
        );

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct OptHolder {
            #[serde(
                serialize_with = "wire_serde::serialize_opt_hex32",
                deserialize_with = "wire_serde::deserialize_opt_hex32"
            )]
            key: Option<[u8; 32]>,
            #[serde(
                serialize_with = "wire_serde::serialize_delivery_ledger",
                deserialize_with = "wire_serde::deserialize_delivery_ledger"
            )]
            ledger: std::collections::BTreeMap<(u8, u8, AssetId), Units>,
        }
        let mut ledger = std::collections::BTreeMap::new();
        ledger.insert((0, 1, AssetId::Raw(0)), 500);
        let oh = OptHolder {
            key: Some([0xAB; 32]),
            ledger,
        };
        let oh_json = must_ok(serde_json::to_string(&oh), "oh json");
        assert_eq!(
            must_ok(serde_json::from_str::<OptHolder>(&oh_json), "oh back"),
            oh
        );
        let none_json = "{\"key\":null,\"ledger\":[]}";
        let none_parsed: OptHolder = must_ok(serde_json::from_str(none_json), "none json");
        assert!(none_parsed.key.is_none());
        let dup_json = "{\"key\":null,\"ledger\":[[[0,1,\"Cred\"],10],[[0,1,\"Cred\"],20]]}";
        assert!(serde_json::from_str::<OptHolder>(dup_json).is_err());
        let _ = core::mem::size_of::<Holder>();
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
        assert!(ds.expected_round_for_turn(Turn(1)).is_ok());
        assert!(drand_quicknet_round_for_close(0).is_err());

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
        let tmb = match lookup_commodity_display("TMB") {
            Some(entry) => entry,
            None => panic!("TMB missing from COMMODITY_DISPLAY_CATALOG"),
        };
        assert_eq!(tmb.raw_display(false), "Rough Timber [TMB]");
        assert_eq!(tmb.raw_display(true), "TMB");
        assert_eq!(tmb.refined_display(false), "Seasoned Timber [TMB]");
        assert_eq!(tmb.refined_display(true), "TMB");
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
    }
}
