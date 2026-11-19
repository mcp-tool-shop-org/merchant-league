//! Byte-identity and re-export `TypeId` proof against golden literals captured by running
//! unmodified `8abc9abca69a39c7b9758a329e54c3099ca01707` prior to moving types into
//! `crates/league-seat-types`.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};

use league_core::observation::*;
use league_core::*;
use sha2::{Digest, Sha256};

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[test]
fn moved_types_preserve_exact_type_id_across_reexports() {
    let mut type_id_count = 0_usize;
    macro_rules! assert_same_type {
        ($($ty:ident),+ $(,)?) => {
            $(
                assert_eq!(
                    TypeId::of::<league_core::$ty>(),
                    TypeId::of::<league_seat_types::$ty>(),
                    concat!("TypeId mismatch for re-exported type ", stringify!($ty))
                );
                type_id_count += 1;
            )+
        };
    }

    assert_same_type!(
        TableId,
        Seat,
        MarketDay,
        SeasonId,
        Turn,
        AssetId,
        CreditTier,
        SeatIdentity,
        RegentDoctrine,
        CharterRole,
        ShockKind,
        ActiveShock,
        ConverterRecipe,
        ProjectRecipe,
        SeatConfig,
        TableMode,
        EngineError,
        StructuredError,
        PreimageSecret,
        CommitmentHash,
        ConverterRunOrder,
        ProjectDeliveryOrder,
        AtomicSwapProposal,
        EscrowFinishUnlock,
        CouncilPlatform,
        CouncilVoteBallot,
        CharterBidOrder,
        ContractDeliveryOrder,
        DualEscrowProposal,
        SealedOrderBundle,
        IntentTarget,
        IntentPledge,
        MandateKind,
        PrivateMandate,
        GradedMandateCard,
        DrandSchedule,
        CartelDomain,
        ActiveCharter,
        CheckStatus,
        CreditCheck,
        EscrowStatus,
        BondSettlementMode,
        BondSpec,
        DualEscrowVault,
        TokenEscrowVault,
        OfferSide,
        CredOffer,
        CheckProposal,
        BondProposal,
        EscrowProposal,
        OfferProposal,
        MarketAction,
        FillKind,
        FillRecord,
        PendingWashFreeze,
        WashFreezeSettlement,
        TurnPhase,
        SeatState,
        BrokenPromiseRecord,
        ShockForecastEvent,
        ConverterOrderPreview,
        ProjectOrderPreview,
        EscrowOrderPreview,
        SeatUpkeepPreview,
        AuditorShockHint,
        PublicTableSpec,
        PublicSweepReceipt,
        SeatObservation,
        PublicPreviewState,
        PreviewTieMode,
        CounterpartyAssumptions,
        HumanWhatIfRequest,
        AiPreviewRequest,
        RefusedActionPreview,
        PublicSwapOutcome,
        PublicSwapOrderPreview,
        PublicCheckOutcome,
        PublicCheckOrderPreview,
        SeatSpoilagePreview,
        Step0To7SweepSummary,
        WhatIfPreview,
        UpkeepAndDebt,
        RecipeCover,
        SpoilageAndConversion,
        GoodScarcity,
        CoalitionReach,
        HoldingMark,
        ProjectInputCover,
        ProjectCover,
        SeasonEndAndMarks,
        SeatStatePredicates,
        SeatSweepOutcome,
        TurnPredicateRecord,
        StatePredicateSection,
        SeatSeasonMetrics,
        FundamentalParities,
        SeatScoreEntry,
        FinalScoreboard,
    );

    println!(
        "SEAT_TYPES_REEXPORT_PARITY: {type_id_count} moved TypeIds match league_seat_types == league_core"
    );
}

#[test]
fn moved_types_and_full_season_match_captured_8abc9abc_literals_byte_for_byte() {
    let mut checked_types = 0_usize;

    macro_rules! check_json {
        ($ty:ty, $val:expr, $expected:expr) => {{
            let val = &$val;
            let json = must_ok(serde_json::to_string(val), stringify!($ty));
            assert_eq!(
                json, $expected,
                concat!("JSON mismatch for ", stringify!($ty))
            );
            let rt: $ty = must_ok(serde_json::from_str(&json), stringify!($ty));
            assert_eq!(
                val, &rt,
                concat!("round-trip mismatch for ", stringify!($ty))
            );
            checked_types += 1;
        }};
    }
    macro_rules! check_dbg {
        ($name:expr, $val:expr, $expected:expr) => {{
            let dbg = format!("{:?}", $val);
            assert_eq!(dbg, $expected, concat!("Debug mismatch for ", $name));
        }};
    }
    macro_rules! check_disp {
        ($name:expr, $val:expr, $expected:expr) => {{
            let disp = format!("{}", $val);
            assert_eq!(disp, $expected, concat!("Display mismatch for ", $name));
        }};
    }
    macro_rules! check_blob_hash {
        ($ty:ty, $val:expr, $expected_len:expr, $expected_sha:expr) => {{
            let val = &$val;
            let json = must_ok(serde_json::to_string(val), stringify!($ty));
            assert_eq!(
                json.len(),
                $expected_len,
                concat!("JSON byte length mismatch for ", stringify!($ty))
            );
            assert_eq!(
                sha256_hex(json.as_bytes()),
                $expected_sha,
                concat!("JSON SHA-256 mismatch for ", stringify!($ty))
            );
            let rt: $ty = must_ok(serde_json::from_str(&json), stringify!($ty));
            assert_eq!(
                val, &rt,
                concat!("round-trip mismatch for ", stringify!($ty))
            );
            checked_types += 1;
        }};
    }
    macro_rules! check_ser_blob_hash {
        ($ty:ty, $val:expr, $expected_len:expr, $expected_sha:expr) => {{
            let val = &$val;
            let json = must_ok(serde_json::to_string(val), stringify!($ty));
            assert_eq!(
                json.len(),
                $expected_len,
                concat!("JSON byte length mismatch for ", stringify!($ty))
            );
            assert_eq!(
                sha256_hex(json.as_bytes()),
                $expected_sha,
                concat!("JSON SHA-256 mismatch for ", stringify!($ty))
            );
            checked_types += 1;
        }};
    }

    // 1. Scalar / ID / Enum value types (lines 1..22 of captured_8abc9abc.txt)
    let tid = TableId(202_611);
    let seat = Seat(2);
    let mday = MarketDay(14);
    let sid = SeasonId(7);
    let turn = Turn(9);
    check_json!(TableId, tid, "202611");
    check_dbg!("TableId", tid, "TableId(202611)");
    check_json!(Seat, seat, "2");
    check_dbg!("Seat", seat, "Seat(2)");
    check_json!(MarketDay, mday, "14");
    check_json!(SeasonId, sid, "7");
    check_json!(Turn, turn, "9");

    let assets = [AssetId::Cred, AssetId::Raw(1), AssetId::Refined(3)];
    check_json!([AssetId; 3], assets, r#"["Cred",{"Raw":1},{"Refined":3}]"#);
    check_dbg!("AssetId", assets, "[Cred, Raw(1), Refined(3)]");

    let tiers = [
        CreditTier::Aaa,
        CreditTier::Bbb,
        CreditTier::B,
        CreditTier::D,
    ];
    check_json!([CreditTier; 4], tiers, r#"["Aaa","Bbb","B","D"]"#);

    let idents = [SeatIdentity::Human, SeatIdentity::AiSyndicate];
    check_json!([SeatIdentity; 2], idents, r#"["Human","AiSyndicate"]"#);

    let regent = RegentDoctrine {
        risk_tolerance_bps: 5_000,
        min_raw_reserve: UNIT_SCALE,
        auto_cash_due_claims: true,
    };
    check_json!(
        RegentDoctrine,
        regent,
        r#"{"risk_tolerance_bps":5000,"min_raw_reserve":1000,"auto_cash_due_claims":true}"#
    );

    let roles = [
        CharterRole::ClearinghouseBroker,
        CharterRole::CustomsInspector,
        CharterRole::OracleAuditor,
    ];
    check_json!(
        [CharterRole; 3],
        roles,
        r#"["ClearinghouseBroker","CustomsInspector","OracleAuditor"]"#
    );

    let kinds = [
        ShockKind::SupplyFreeze,
        ShockKind::TariffHoliday,
        ShockKind::DemandSurge,
        ShockKind::SpoilageSpike,
    ];
    check_json!(
        [ShockKind; 4],
        kinds,
        r#"["SupplyFreeze","TariffHoliday","DemandSurge","SpoilageSpike"]"#
    );

    let active_shock = ActiveShock {
        target_seat: 1,
        kind: ShockKind::DemandSurge,
        magnitude_bps: 1_500,
        expires_after_turn: 12,
    };
    check_json!(
        ActiveShock,
        active_shock,
        r#"{"target_seat":1,"kind":"DemandSurge","magnitude_bps":1500,"expires_after_turn":12}"#
    );

    let conv = ConverterRecipe {
        owner: 0,
        inputs: vec![(1, 2_000), (2, 2_000)],
        output_good: 0,
        output_amount: 3_500,
        max_runs_per_turn: 4,
        is_secondary: false,
    };
    check_json!(
        ConverterRecipe,
        conv,
        r#"{"owner":0,"inputs":[[1,2000],[2,2000]],"output_good":0,"output_amount":3500,"max_runs_per_turn":4,"is_secondary":false}"#
    );

    let proj = ProjectRecipe {
        id: 1,
        inputs: vec![(0, 1_000), (1, 1_000)],
        equity_points: 4_800,
    };
    check_json!(
        ProjectRecipe,
        proj,
        r#"{"id":1,"inputs":[[0,1000],[1,1000]],"equity_points":4800}"#
    );

    let seat_cfg = SeatConfig {
        seat: 0,
        identity: SeatIdentity::Human,
        initial_cred: 15_000,
        raw_yield_per_turn: 10_000,
        refined_project_value: 2_000,
        upkeep_good: 3,
        upkeep_amount: 1_800,
        initial_raw_tariff_bps: 800,
        initial_refined_tariff_bps: 600,
        perishable_decay_bps: 2_000,
        regent: Some(regent),
        converters: vec![conv.clone()],
    };
    check_json!(
        SeatConfig,
        seat_cfg,
        r#"{"seat":0,"identity":"Human","initial_cred":15000,"raw_yield_per_turn":10000,"refined_project_value":2000,"upkeep_good":3,"upkeep_amount":1800,"initial_raw_tariff_bps":800,"initial_refined_tariff_bps":600,"perishable_decay_bps":2000,"regent":{"risk_tolerance_bps":5000,"min_raw_reserve":1000,"auto_cash_due_claims":true},"converters":[{"owner":0,"inputs":[[1,2000],[2,2000]],"output_good":0,"output_amount":3500,"max_runs_per_turn":4,"is_secondary":false}]}"#
    );

    let modes = [TableMode::Game, TableMode::Eval, TableMode::Club];
    check_json!([TableMode; 3], modes, r#"["Game","Eval","Club"]"#);

    let err = EngineError::UnsupportedReplayVersion {
        found: 99,
        newest_known: 3,
    };
    check_json!(
        EngineError,
        err,
        r#"{"UnsupportedReplayVersion":{"found":99,"newest_known":3}}"#
    );
    check_disp!(
        "EngineError",
        err,
        "UNSUPPORTED_REPLAY_VERSION: replay format version 99 is not supported by this reader (newest known version is 3)"
    );
    let structured_json = must_ok(
        serde_json::to_string(&err.to_structured_error()),
        "StructuredError",
    );
    assert_eq!(
        structured_json,
        r#"{"code":"UNSUPPORTED_REPLAY_VERSION","message":"UNSUPPORTED_REPLAY_VERSION: replay format version 99 is not supported by this reader (newest known version is 3)","hint":"upgrade the reader to support this replay format_version before loading the file","retryable":false}"#
    );
    checked_types += 1;

    // 2. Commit & mandate types (lines 23..43)
    let preimage = PreimageSecret([0x5A; 32]);
    check_json!(
        PreimageSecret,
        preimage,
        r#""5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a""#
    );
    check_dbg!("PreimageSecret", preimage, "PreimageSecret(<hidden>)");

    let commit_hash = CommitmentHash([0xC3; 32]);
    check_json!(
        CommitmentHash,
        commit_hash,
        r#""c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3""#
    );
    check_dbg!(
        "CommitmentHash",
        commit_hash,
        "CommitmentHash(c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3)"
    );
    check_disp!(
        "CommitmentHash",
        commit_hash,
        "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3"
    );

    let run_order = ConverterRunOrder {
        recipe_idx: 0,
        runs: 2,
    };
    check_json!(ConverterRunOrder, run_order, r#"{"recipe_idx":0,"runs":2}"#);

    let proj_order = ProjectDeliveryOrder {
        project_id: 1,
        batches: 1,
    };
    check_json!(
        ProjectDeliveryOrder,
        proj_order,
        r#"{"project_id":1,"batches":1}"#
    );

    let swap = AtomicSwapProposal {
        swap_id: 11,
        maker: 0,
        taker: 1,
        maker_asset: AssetId::Raw(0),
        maker_amount: 1_000,
        taker_asset: AssetId::Refined(1),
        taker_amount: 1_200,
        domain_id: Some(2),
    };
    check_json!(
        AtomicSwapProposal,
        swap,
        r#"{"swap_id":11,"maker":0,"taker":1,"maker_asset":{"Raw":0},"maker_amount":1000,"taker_asset":{"Refined":1},"taker_amount":1200,"domain_id":2}"#
    );

    let escrow_unlock = EscrowFinishUnlock {
        escrow_id: 7,
        preimage: Some(preimage.clone()),
    };
    check_json!(
        EscrowFinishUnlock,
        escrow_unlock,
        r#"{"escrow_id":7,"preimage":"5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a"}"#
    );

    let platform = CouncilPlatform {
        proposed_tariffs_bps: vec![(0, (700, 500))],
        cartel_admissions: vec![(1, 2)],
        cartel_revocations: vec![(1, 3)],
    };
    check_json!(
        CouncilPlatform,
        platform,
        r#"{"proposed_tariffs_bps":[[0,[700,500]]],"cartel_admissions":[[1,2]],"cartel_revocations":[[1,3]]}"#
    );

    let ballot = CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 2,
    };
    check_json!(
        CouncilVoteBallot,
        ballot,
        r#"{"trade_minister":0,"cartel_chair":2}"#
    );

    let bid = CharterBidOrder {
        role: CharterRole::OracleAuditor,
        bid_cred: 900,
    };
    check_json!(
        CharterBidOrder,
        bid,
        r#"{"role":"OracleAuditor","bid_cred":900}"#
    );

    let contract_del = ContractDeliveryOrder {
        counterparty: 1,
        good: AssetId::Refined(0),
        gross_amount: 1_500,
        domain_id: None,
        dual_escrow_id: Some(4),
    };
    check_json!(
        ContractDeliveryOrder,
        contract_del,
        r#"{"counterparty":1,"good":{"Refined":0},"gross_amount":1500,"domain_id":null,"dual_escrow_id":4}"#
    );

    let dual_prop = DualEscrowProposal {
        proposal_id: 5,
        buyer: 0,
        seller: 1,
        good: AssetId::Refined(1),
        required_units: 1_000,
        payment_cred: 2_500,
        buyer_bond_cred: 500,
        seller_bond_cred: 500,
        deadline_turn: 4,
        domain_id: None,
    };
    check_json!(
        DualEscrowProposal,
        dual_prop,
        r#"{"proposal_id":5,"buyer":0,"seller":1,"good":{"Refined":1},"required_units":1000,"payment_cred":2500,"buyer_bond_cred":500,"seller_bond_cred":500,"deadline_turn":4,"domain_id":null}"#
    );

    let bundle = SealedOrderBundle {
        converter_runs: vec![run_order],
        project_deliveries: vec![proj_order],
        atomic_swaps: vec![swap],
        contract_deliveries: vec![contract_del],
        dual_escrows: vec![dual_prop],
        debt_repayment_cred: 250,
        escrow_finishes: vec![escrow_unlock],
        check_cashes: vec![3],
        council_vote: Some(ballot),
        charter_bids: vec![bid],
    };
    check_json!(
        SealedOrderBundle,
        bundle,
        r#"{"converter_runs":[{"recipe_idx":0,"runs":2}],"project_deliveries":[{"project_id":1,"batches":1}],"atomic_swaps":[{"swap_id":11,"maker":0,"taker":1,"maker_asset":{"Raw":0},"maker_amount":1000,"taker_asset":{"Refined":1},"taker_amount":1200,"domain_id":2}],"contract_deliveries":[{"counterparty":1,"good":{"Refined":0},"gross_amount":1500,"domain_id":null,"dual_escrow_id":4}],"dual_escrows":[{"proposal_id":5,"buyer":0,"seller":1,"good":{"Refined":1},"required_units":1000,"payment_cred":2500,"buyer_bond_cred":500,"seller_bond_cred":500,"deadline_turn":4,"domain_id":null}],"debt_repayment_cred":250,"escrow_finishes":[{"escrow_id":7,"preimage":"5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a"}],"check_cashes":[3],"council_vote":{"trade_minister":0,"cartel_chair":2},"charter_bids":[{"role":"OracleAuditor","bid_cred":900}]}"#
    );

    let targets = [
        IntentTarget::VoteForSeat {
            office_turn: 4,
            candidate: 1,
        },
        IntentTarget::PreserveCartelMembership {
            office_turn: 8,
            domain_id: 1,
            beneficiary: 2,
        },
        IntentTarget::RevealPreimage {
            escrow_id: 9,
            condition: commit_hash,
        },
    ];
    check_json!(
        [IntentTarget; 3],
        targets,
        r#"[{"VoteForSeat":{"office_turn":4,"candidate":1}},{"PreserveCartelMembership":{"office_turn":8,"domain_id":1,"beneficiary":2}},{"RevealPreimage":{"escrow_id":9,"condition":"c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3"}}]"#
    );

    let pledge = IntentPledge {
        pledge_id: 1,
        promisor: 0,
        counterparty: 1,
        target: targets[0].clone(),
        honor_weight: 500,
        memo_digest: [0x22; 32],
    };
    check_json!(
        IntentPledge,
        pledge,
        r#"{"pledge_id":1,"promisor":0,"counterparty":1,"target":{"VoteForSeat":{"office_turn":4,"candidate":1}},"honor_weight":500,"memo_digest":"2222222222222222222222222222222222222222222222222222222222222222"}"#
    );

    let m_kinds = [
        MandateKind::MinProjectEquity { target: 28_000 },
        MandateKind::MinRefinedVariety {
            distinct_goods: 2,
            min_each: 1_500,
        },
        MandateKind::MaintainCleanCredit {
            min_tier: CreditTier::Aaa,
        },
        MandateKind::MinActivePartners { min_partners: 2 },
    ];
    check_json!(
        [MandateKind; 4],
        m_kinds,
        r#"[{"MinProjectEquity":{"target":28000}},{"MinRefinedVariety":{"distinct_goods":2,"min_each":1500}},{"MaintainCleanCredit":{"min_tier":"Aaa"}},{"MinActivePartners":{"min_partners":2}}]"#
    );

    let pm = PrivateMandate {
        kind: m_kinds[0],
        bonus_equity: 7_000,
    };
    check_json!(
        PrivateMandate,
        pm,
        r#"{"kind":{"MinProjectEquity":{"target":28000}},"bonus_equity":7000}"#
    );

    let card = GradedMandateCard {
        card_id: 3,
        kind: m_kinds[1],
        difficulty_completion_bps: 7_000,
        bonus_equity: 4_500,
    };
    check_json!(
        GradedMandateCard,
        card,
        r#"{"card_id":3,"kind":{"MinRefinedVariety":{"distinct_goods":2,"min_each":1500}},"difficulty_completion_bps":7000,"bonus_equity":4500}"#
    );

    let drand_sched = DrandSchedule {
        turn_1_close_unix: 1_700_000_000,
        turn_period_secs: 600,
    };
    check_json!(
        DrandSchedule,
        drand_sched,
        r#"{"turn_1_close_unix":1700000000,"turn_period_secs":600}"#
    );

    // 3. Contract & Fill types (lines 44..63)
    let mut dom_members = BTreeSet::new();
    dom_members.insert(0);
    dom_members.insert(2);
    let dom = CartelDomain {
        domain_id: 1,
        members: dom_members,
    };
    check_json!(CartelDomain, dom, r#"{"domain_id":1,"members":[0,2]}"#);

    let act_charter = ActiveCharter {
        role: CharterRole::ClearinghouseBroker,
        holder: 1,
        expires_at_turn: 12,
    };
    check_json!(
        ActiveCharter,
        act_charter,
        r#"{"role":"ClearinghouseBroker","holder":1,"expires_at_turn":12}"#
    );

    let chk_statuses = [
        CheckStatus::Open,
        CheckStatus::Cashed(500),
        CheckStatus::Bounced,
        CheckStatus::Cancelled,
    ];
    check_json!(
        [CheckStatus; 4],
        chk_statuses,
        r#"["Open",{"Cashed":500},"Bounced","Cancelled"]"#
    );

    let chk = CreditCheck {
        check_id: 2,
        drawer: 0,
        payee: 1,
        asset: AssetId::Cred,
        send_max: 1_000,
        deliver_min: 800,
        due_turn: 2,
        expiration_turn: 5,
        status: CheckStatus::Open,
    };
    check_json!(
        CreditCheck,
        chk,
        r#"{"check_id":2,"drawer":0,"payee":1,"asset":"Cred","send_max":1000,"deliver_min":800,"due_turn":2,"expiration_turn":5,"status":"Open"}"#
    );

    let esc_statuses = [
        EscrowStatus::Locked,
        EscrowStatus::Finished,
        EscrowStatus::Cancelled,
    ];
    check_json!(
        [EscrowStatus; 3],
        esc_statuses,
        r#"["Locked","Finished","Cancelled"]"#
    );

    let bmodes = [
        BondSettlementMode::SellerPerformanceBond,
        BondSettlementMode::BuyerDvpPayment,
    ];
    check_json!(
        [BondSettlementMode; 2],
        bmodes,
        r#"["SellerPerformanceBond","BuyerDvpPayment"]"#
    );

    let bspec = BondSpec {
        obligor: 1,
        beneficiary: 0,
        good: AssetId::Refined(1),
        required_units: 1_000,
        baseline_delivered: 0,
        deadline_turn: 6,
        mode: BondSettlementMode::BuyerDvpPayment,
    };
    check_json!(
        BondSpec,
        bspec,
        r#"{"obligor":1,"beneficiary":0,"good":{"Refined":1},"required_units":1000,"baseline_delivered":0,"deadline_turn":6,"mode":"BuyerDvpPayment"}"#
    );

    let dual_v = DualEscrowVault {
        escrow_id: 10,
        buyer: 0,
        seller: 1,
        good: AssetId::Refined(1),
        required_units: 1_000,
        payment_cred: 2_000,
        buyer_bond_cred: 1_000,
        seller_bond_cred: 500,
        baseline_delivered: 100,
        deadline_turn: 6,
        domain_id: Some(1),
        status: EscrowStatus::Locked,
    };
    check_json!(
        DualEscrowVault,
        dual_v,
        r#"{"escrow_id":10,"buyer":0,"seller":1,"good":{"Refined":1},"required_units":1000,"payment_cred":2000,"buyer_bond_cred":1000,"seller_bond_cred":500,"baseline_delivered":100,"deadline_turn":6,"domain_id":1,"status":"Locked"}"#
    );

    let tok_v = TokenEscrowVault {
        escrow_id: 11,
        sender: 0,
        destination: 1,
        amount_cred: 1_500,
        finish_after_turn: Some(2),
        cancel_after_turn: 6,
        condition_hash: Some(commit_hash),
        bond: Some(bspec),
        status: EscrowStatus::Locked,
    };
    check_json!(
        TokenEscrowVault,
        tok_v,
        r#"{"escrow_id":11,"sender":0,"destination":1,"amount_cred":1500,"finish_after_turn":2,"cancel_after_turn":6,"condition_hash":"c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3","bond":{"obligor":1,"beneficiary":0,"good":{"Refined":1},"required_units":1000,"baseline_delivered":0,"deadline_turn":6,"mode":"BuyerDvpPayment"},"status":"Locked"}"#
    );

    let sides = [OfferSide::Buy, OfferSide::Sell];
    check_json!([OfferSide; 2], sides, r#"["Buy","Sell"]"#);

    let offer = CredOffer {
        offer_id: 1,
        maker: 0,
        side: OfferSide::Buy,
        good: AssetId::Raw(1),
        remaining_units: 2_000,
        price_per_unit_cred: 1_500,
        locked_cred: 3_000,
        domain_id: None,
        is_hybrid: false,
        created_turn: 1,
    };
    check_json!(
        CredOffer,
        offer,
        r#"{"offer_id":1,"maker":0,"side":"Buy","good":{"Raw":1},"remaining_units":2000,"price_per_unit_cred":1500,"locked_cred":3000,"domain_id":null,"is_hybrid":false,"created_turn":1}"#
    );

    let chk_prop = CheckProposal {
        payee: 1,
        asset: AssetId::Cred,
        send_max: 500,
        deliver_min: 400,
        due_turn: 2,
        expiration_turn: 5,
    };
    check_json!(
        CheckProposal,
        chk_prop,
        r#"{"payee":1,"asset":"Cred","send_max":500,"deliver_min":400,"due_turn":2,"expiration_turn":5}"#
    );

    let bprop = BondProposal {
        good: AssetId::Refined(1),
        required_units: 1_000,
        deadline_turn: 5,
        mode: BondSettlementMode::SellerPerformanceBond,
    };
    check_json!(
        BondProposal,
        bprop,
        r#"{"good":{"Refined":1},"required_units":1000,"deadline_turn":5,"mode":"SellerPerformanceBond"}"#
    );

    let eprop = EscrowProposal {
        destination: 1,
        asset: AssetId::Cred,
        amount: 1_000,
        finish_after_turn: Some(2),
        cancel_after_turn: 6,
        condition_hash: Some(commit_hash),
        bond: Some(bprop),
    };
    check_json!(
        EscrowProposal,
        eprop,
        r#"{"destination":1,"asset":"Cred","amount":1000,"finish_after_turn":2,"cancel_after_turn":6,"condition_hash":"c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3","bond":{"good":{"Refined":1},"required_units":1000,"deadline_turn":5,"mode":"SellerPerformanceBond"}}"#
    );

    let oprop = OfferProposal {
        side: OfferSide::Sell,
        good: AssetId::Refined(0),
        amount: 1_000,
        price_per_unit_cred: 2_100,
        domain_id: None,
        is_hybrid: false,
    };
    check_json!(
        OfferProposal,
        oprop,
        r#"{"side":"Sell","good":{"Refined":0},"amount":1000,"price_per_unit_cred":2100,"domain_id":null,"is_hybrid":false}"#
    );

    let m_actions = [
        MarketAction::PostOffer {
            seat: 0,
            order: oprop,
        },
        MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(1),
            amount: 500,
        },
        MarketAction::FlagWashFreeze {
            inspector: 2,
            target_seat: 3,
        },
    ];
    check_json!(
        [MarketAction; 3],
        m_actions,
        r#"[{"PostOffer":{"seat":0,"order":{"side":"Sell","good":{"Refined":0},"amount":1000,"price_per_unit_cred":2100,"domain_id":null,"is_hybrid":false}}},{"TownImport":{"seat":0,"good":{"Raw":1},"amount":500}},{"FlagWashFreeze":{"inspector":2,"target_seat":3}}]"#
    );

    let fkinds = [
        FillKind::BatchFill,
        FillKind::AtomicSwap,
        FillKind::ContractDelivery,
        FillKind::CheckCashed,
        FillKind::EscrowFinished,
        FillKind::EscrowForfeited,
        FillKind::EscrowBreachRefunded,
        FillKind::EscrowRefunded,
        FillKind::TownImport,
    ];
    check_json!(
        [FillKind; 9],
        fkinds,
        r#"["batch_fill","atomic_swap","contract_delivery","check_cashed","escrow_finished","escrow_forfeited","escrow_breach_refunded","escrow_refunded","town_import"]"#
    );

    let fill = FillRecord {
        turn: 1,
        kind: FillKind::BatchFill,
        good: Some(AssetId::Raw(1)),
        from: Some(1),
        to: 0,
        units: 1_000,
        net_units: 920,
        cred_amount: 1_500,
        unit_price_cred: Some(1_500),
    };
    check_json!(
        FillRecord,
        fill,
        r#"{"turn":1,"kind":"batch_fill","good":{"Raw":1},"from":1,"to":0,"units":1000,"net_units":920,"cred_amount":1500,"unit_price_cred":1500}"#
    );

    let pwf = PendingWashFreeze {
        turn: 2,
        inspector: 1,
        target_seat: 3,
        bond_cred: 500,
    };
    check_json!(
        PendingWashFreeze,
        pwf,
        r#"{"turn":2,"inspector":1,"target_seat":3,"bond_cred":500}"#
    );

    let wfs = WashFreezeSettlement {
        turn: 2,
        inspector: 1,
        target_seat: 3,
        confirmed: false,
        bond_returned_cred: 0,
        bounty_paid_cred: 0,
        damages_paid_cred: 500,
    };
    check_json!(
        WashFreezeSettlement,
        wfs,
        r#"{"turn":2,"inspector":1,"target_seat":3,"confirmed":false,"bond_returned_cred":0,"bounty_paid_cred":0,"damages_paid_cred":500}"#
    );

    // 4. Engine public types (lines 64..72)
    let phases = [
        TurnPhase::TalkAndMarket,
        TurnPhase::Commit,
        TurnPhase::Reveal,
        TurnPhase::AwaitingMandateReveal,
        TurnPhase::Completed,
    ];
    check_json!(
        [TurnPhase; 5],
        phases,
        r#"["TalkAndMarket","Commit","Reveal","AwaitingMandateReveal","Completed"]"#
    );

    let s_state = SeatState {
        seat: 0,
        identity: SeatIdentity::Human,
        cred: 15_000,
        cred_debt: 0,
        raw: vec![0, 500, 0, 0],
        refined: vec![1_000, 0, 0, 0],
        project_equity: 4_800,
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
    };
    check_json!(
        SeatState,
        s_state,
        r#"{"seat":0,"identity":"Human","cred":15000,"cred_debt":0,"raw":[0,500,0,0],"refined":[1000,0,0,0],"project_equity":4800,"raw_tariff_bps":800,"refined_tariff_bps":600,"credit_tier":"Aaa","hard_credit_defaults":0,"bond_breaches":0,"broken_promises":0,"honor_penalties":0,"consecutive_misses":0,"consecutive_zero_production_turns":0,"wash_frozen_until_turn":0}"#
    );

    let bpr = BrokenPromiseRecord {
        pledge_id: 1,
        promisor: 2,
        counterparty: 0,
        weight: 500,
    };
    check_json!(
        BrokenPromiseRecord,
        bpr,
        r#"{"pledge_id":1,"promisor":2,"counterparty":0,"weight":500}"#
    );

    let sfe = ShockForecastEvent {
        target_seat: 2,
        kind: ShockKind::SpoilageSpike,
        realize_turn: 5,
    };
    check_json!(
        ShockForecastEvent,
        sfe,
        r#"{"target_seat":2,"kind":"SpoilageSpike","realize_turn":5}"#
    );

    let cop = ConverterOrderPreview {
        recipe_idx: 0,
        requested_runs: 3,
        executed_runs: 2,
        clamped: true,
    };
    check_json!(
        ConverterOrderPreview,
        cop,
        r#"{"recipe_idx":0,"requested_runs":3,"executed_runs":2,"clamped":true}"#
    );

    let pop = ProjectOrderPreview {
        project_id: 1,
        requested_batches: 2,
        executed_batches: 1,
        clamped: true,
    };
    check_json!(
        ProjectOrderPreview,
        pop,
        r#"{"project_id":1,"requested_batches":2,"executed_batches":1,"clamped":true}"#
    );

    let eop = EscrowOrderPreview {
        escrow_id: 11,
        unlocked: true,
    };
    check_json!(
        EscrowOrderPreview,
        eop,
        r#"{"escrow_id":11,"unlocked":true}"#
    );

    let sup = SeatUpkeepPreview {
        upkeep_good: 3,
        upkeep_required: 1_800,
        paid_with_refined: 1_800,
        debt_interest_accrued: 0,
        new_shortfall_debt: 0,
        debt_repaid_cred: 250,
    };
    check_json!(
        SeatUpkeepPreview,
        sup,
        r#"{"upkeep_good":3,"upkeep_required":1800,"paid_with_refined":1800,"debt_interest_accrued":0,"new_shortfall_debt":0,"debt_repaid_cred":250}"#
    );

    let sweep_out = SeatSweepOutcome {
        seat: 0,
        upkeep_shortfall_units: 0,
        shortfall_debt_added: 0,
        spoiled_units: 120,
    };
    check_json!(
        SeatSweepOutcome,
        sweep_out,
        r#"{"seat":0,"upkeep_shortfall_units":0,"shortfall_debt_added":0,"spoiled_units":120}"#
    );

    // 5. Observation, Preview, Spec, Parities, Predicates (lines 73..89)
    let eval_seed_bytes = [0x77; 32];
    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes(eval_seed_bytes),
        "SealedEvalSeed",
    );
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31 + i; 32])).collect();
    let mut host = must_ok(
        ClubHost::from_explicit_setup(TableId(202_611), 4, 1, [0x55; 32], &salts, eval_seed, 42),
        "ClubHost::from_explicit_setup",
    );

    let (spec_raw, _, _) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            TableId(202_611),
            4,
            1,
            [0x55; 32],
            &salts,
            must_ok(SealedEvalSeed::try_from_bytes(eval_seed_bytes), "eval2"),
        ),
        "TableSpec explicit",
    );
    let eng_direct = must_ok(TurnEngine::new(spec_raw), "TurnEngine::new");
    let mut obs_none = must_ok(eng_direct.observe_seat(Seat(0)), "eng_direct.observe_seat");
    obs_none
        .delivery_ledger
        .insert((0, 1, AssetId::Raw(0)), 500);
    check_ser_blob_hash!(
        SeatObservation,
        obs_none,
        10_345,
        "54c93c215b3eab75a0ca803ec8d417cbdde837b56a3b6cbb9697b0a4d1d9bb78"
    );

    let mut obs0_t1 = must_ok(host.observe_seat(Seat(0)), "host observe_seat t1");
    obs0_t1.auditor_hints.push(AuditorShockHint {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 1,
        kind: ShockKind::TariffHoliday,
    });
    check_json!(
        AuditorShockHint,
        obs0_t1.auditor_hints[0],
        r#"{"hint_turn":1,"forecast_turn":2,"realize_turn":3,"target_seat":1,"kind":"TariffHoliday"}"#
    );
    check_blob_hash!(
        PublicTableSpec,
        obs0_t1.spec,
        3_350,
        "2095c3a739f976acde089b64806b72cb584eeac51600971d2de4baf094c0779b"
    );
    check_json!(
        FundamentalParities,
        obs0_t1.parities,
        r#"{"cred_per_unit":1000,"raw_per_unit":[1750,1750,1750,1750],"refined_per_unit":[2000,2000,2000,2000]}"#
    );
    check_blob_hash!(
        SeatStatePredicates,
        obs0_t1.baseline_predicates,
        4_785,
        "92fe660a831dc92ab6ad4dbbe182305b5d4848b0d340da4335cb7c519fc8d785"
    );
    let pub_prev = obs0_t1.to_public_preview_state();
    check_blob_hash!(
        PublicPreviewState,
        pub_prev,
        10_177,
        "553be611faa4c1e6a217fd2fdb5494681abe51c28187a924b82678524fb35d50"
    );

    let ai_req = AiPreviewRequest {
        seat: 0,
        market_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    };
    check_json!(
        AiPreviewRequest,
        ai_req,
        r#"{"seat":0,"market_actions":[],"bundle":{"converter_runs":[],"project_deliveries":[],"atomic_swaps":[],"contract_deliveries":[],"dual_escrows":[],"debt_repayment_cred":0,"escrow_finishes":[],"check_cashes":[],"council_vote":null,"charter_bids":[]}}"#
    );
    let ai_prev = must_ok(
        preview_seat_bundle(&pub_prev, &ai_req),
        "preview_seat_bundle",
    );
    check_blob_hash!(
        WhatIfPreview,
        ai_prev,
        11_874,
        "cf82c11bed261692b8561e7d94d7b0e4d6ac811e5b86f362b0ea03e02afd094f"
    );

    let bad_action = MarketAction::CancelOffer {
        seat: 0,
        offer_id: 999_999,
    };
    let human_req = HumanWhatIfRequest {
        seat: 0,
        market_actions: vec![bad_action],
        cancel_lock_at: Some(0),
        bundle: SealedOrderBundle::default(),
        tie_mode: PreviewTieMode::PessimisticForSeat,
        assumptions: CounterpartyAssumptions::default(),
    };
    check_json!(
        HumanWhatIfRequest,
        human_req,
        r#"{"seat":0,"market_actions":[{"CancelOffer":{"seat":0,"offer_id":999999}}],"cancel_lock_at":0,"bundle":{"converter_runs":[],"project_deliveries":[],"atomic_swaps":[],"contract_deliveries":[],"dual_escrows":[],"debt_repayment_cred":0,"escrow_finishes":[],"check_cashes":[],"council_vote":null,"charter_bids":[]},"tie_mode":"pessimistic_for_seat","assumptions":{"assumed_cancelled_offer_ids":[],"assumed_peer_market_actions":[],"assumed_cosigned_swaps":[],"assumed_cosigned_dual_escrows":[],"assumed_peer_deliveries":[],"assumed_peer_escrow_finishes":[],"assumed_bounced_check_ids":[],"assumed_peer_bundles":{}}}"#
    );
    check_json!(
        [PreviewTieMode; 2],
        [
            PreviewTieMode::NeutralSeed,
            PreviewTieMode::PessimisticForSeat
        ],
        r#"["neutral_seed","pessimistic_for_seat"]"#
    );
    check_json!(
        CounterpartyAssumptions,
        human_req.assumptions,
        r#"{"assumed_cancelled_offer_ids":[],"assumed_peer_market_actions":[],"assumed_cosigned_swaps":[],"assumed_cosigned_dual_escrows":[],"assumed_peer_deliveries":[],"assumed_peer_escrow_finishes":[],"assumed_bounced_check_ids":[],"assumed_peer_bundles":{}}"#
    );
    let human_prev = must_ok(preview_what_if(&pub_prev, &human_req), "preview_what_if");
    check_blob_hash!(
        WhatIfPreview,
        human_prev,
        11_963,
        "60f8e3f35a9cff0e0216ab5997dcb81d4d9fcff07d2d736eb123ff05614879ed"
    );
    check_json!(
        RefusedActionPreview,
        human_prev.refused_market_actions[0],
        r#"{"action_index":0,"error_code":"CANCEL_LOCKED","message":"Phase-1 cancel lock is active"}"#
    );
    check_json!(
        SeatSpoilagePreview,
        human_prev.spoilage,
        r#"{"perishable_spoiled_units":2000}"#
    );
    check_json!(
        Step0To7SweepSummary,
        human_prev.step0_to_7_receipt,
        r#"{"turn":1,"non_reveal_seats":[],"regent_acted_seats":[],"converted_to_ai":[],"escrows_finished":[],"escrows_cancelled_or_forfeited":[],"atomic_swaps_settled":0,"converter_batches_run":0,"converter_orders_clamped":0,"checks_cashed":[],"checks_bounced":[],"cartel_evictions":[],"projects_completed":0,"project_orders_clamped":0,"total_upkeep_shortfall_debt":14400,"total_spoilage_clawed_back":8000,"broken_promises":[],"elected_officers":null,"charters_awarded":[],"cred_debt_repaid":0,"charter_cred_burned":0,"town_import_cred_spent":0,"works_stipend_cred_minted":0,"batch_trades_cleared":0,"fills":[],"wash_freeze_settlements":[]}"#
    );

    let swap_prev = PublicSwapOrderPreview {
        swap_id: 11,
        maker: 0,
        taker: 1,
        outcome: PublicSwapOutcome::Settled,
    };
    check_json!(
        PublicSwapOrderPreview,
        swap_prev,
        r#"{"swap_id":11,"maker":0,"taker":1,"outcome":"settled"}"#
    );
    let chk_prev = PublicCheckOrderPreview {
        check_id: 2,
        outcome: PublicCheckOutcome::Cashed,
    };
    check_json!(
        PublicCheckOrderPreview,
        chk_prev,
        r#"{"check_id":2,"outcome":"cashed"}"#
    );

    // 6. Full 24-turn explicit season parity vs 8abc9abc
    let expected_s0_hashes_by_turn: [&str; 24] = [
        "4f0b8453619a0818685d9e94d211314addbc1b681e4d9f0a268ec40e28e29403",
        "4052da3508e2ea4bfaea0a24c2d196c09c31acb72812422adc742d18e3a8e8b4",
        "5d1d5fac4158917ef54e345586c9704754ff32f9e544f64502d3cf910608417e",
        "469aaf1f1243e9b13e92c1179e43bb12d4b4c6aab368725fd9f8844135def5a3",
        "35444bd080c156e8e7ec5311b5897a47cd6f73d00a5182108341614bad184cbb",
        "f9ed3e91ddc5cf86ca9a5c06be131792cd441f52a98a0b4088348a291c6334b1",
        "0c39d23929038d4af6366e78c489a5aac7b48b70da7c12491c0992eaf92f142b",
        "222c9df750df61d34fa6302b6ef38dca2df33f31996ae03df075e461f52f8e1d",
        "21a7f04b850314cd6d195204943875309b082301e921e5c005e4848afb99a74e",
        "e3d32cc74db8dfbd32cdc56e69b81014b8c71a1f65d8980de7b34053a3eb08af",
        "4b2090c443f43a36ccdc393332c26764cc977afd0651df62b75701369f97dc6c",
        "366582d5bf69092ea44f9aaf526f53b8241235d7b7b9073a6545f0afed0c34e8",
        "978c659ff44e405fe530484e9b768aa3deac37e2953f6eaa9699c58242df0ce9",
        "52c2748d2b88b0d74f376b4df0a46b147680f38e89ea9bc7dcc4b1c719bd89f9",
        "127106012e8b3db31994053b5f24f1bd6c03b8992dbf29bb25093f2b56fdc083",
        "f5ba95fe01006384ae274185d0595ddcdd3fede813123c38ec4e190e0135577d",
        "7176ee74d52bd088b89b228b745a5ce1cb208a88a2ad142324702f48192b8832",
        "b01993c24ef7f494c53481111cb5eaca66b1e1c2ce541d1a289af35658ad6fb8",
        "5e367e775a329e8bdb49b0865c5cf18e03713e5d8e3b49306c228930bff5fc78",
        "bc85b0e65dec7bb68131a72598bbaae5176dc951d221004094a6748bb311d7be",
        "abd0000d66740bae653d8a00f6e7077a22d08e5079ac52b584382ebe1ff1e5b3",
        "7e0271700ddbcc77ca8d7a3b6c19180868b01d18e0b6a2bb52c1e6cbe124dc6d",
        "fa3436ad247a741fbe2f4a281f3c042b67f57ddd6c7d53a45f7df11227e59d08",
        "08487064e53af7bdfd5d64bec26c9219833dc7e338a2447a61541f295c0d7b1d",
    ];
    let expected_rcpt_hashes_by_turn: [&str; 24] = [
        "685cd8298dfe7d949ac0be8ac1938d4232a942aadb89d8deb47611da49c4a9a1",
        "5eb8296d69df2143cf0536783af35dd7889c069322770d5b50f75340b7f46a17",
        "53f72c8c9c3f076611ad3929d7b650b146af8d0053c820c016ea2d9da8eb4f00",
        "333ba7502e28025355149b5b524f874de9baad81ea89ffad003e2537867009dc",
        "d4232679170787a9ac17d336d2b682feb802a0cee4e3c9c95d7ecabc38293dc2",
        "70f4b82d52f626b4cae3910adbd763d6e48d104f90bbf2e59242c18475755c73",
        "d069f7e4e98f0888544a1ad4398138bc1e26e39dc83a0727a1859f56b338c58f",
        "c07763489c49b4d66c247ac811a41caa859cef1db7ed6b1321d817251fe063e5",
        "772f2ee8ecb3e10a960216c5e4faa5112522b0a93bff905e6d9cc12f80b38a77",
        "6d89fb8b3109e669b7f80e548ccdd8b52e94fecbc59975728b4a78d2e4fdd5e6",
        "254d06b553771a4354996d0790cbe45cbde1cf71efa2ac93c15a2f40a5c98fee",
        "8ff46369b11117522787f0f13c06a550a4e2bd2d210cc5dd540a7b78dd90e4f4",
        "82b581e49fe6ebddc47e6254915f54414fc9e16a902675ab9e30d9d711cb5d3d",
        "e0a86e073f5270952240ddb32fc3143eaa4959bd5338158e46d6a15dbeafc1df",
        "0aaad9db91e07a450696f84eed33bfe7916e19bdd4c8949e6cdce88104e7f04b",
        "dcfd0640b2c8802b9b283b35167cfcf8218f4f4fb344ad233a143e82e9e30fdd",
        "4c46949f3320f0d4724009f92308616ea1a3df95a8a5d54133320f504cc1d8f4",
        "fa3c91b72bfd6c5d51f4c50403657f323aea87f0cbbc723e82614903d22fefd3",
        "a1121d4836e4e69a32191d83e01ce3dfcbc3207b7080901ebabb338deb777cb7",
        "bdc376810d87460c299328bc4db86614cb4a20fe92901f8e8adcf655ddc32292",
        "5fd74df450862fbe04656950706ab74e0915e922945a603b93a445fbefd90cae",
        "dc14eddbb777e853c4130c7b45902533ab454d195bd5679cdf96018aa99bcec6",
        "b2d8a1acd3bc783f1a82c8744dd65a185afa0b311dbdcf08b2f73d88d103ddd0",
        "42f7cc4622b72411c778961c009b7b9a8b9d9482d28c9656445caad199f388f4",
    ];

    let bots: Vec<ClubBot> = (0..4).map(|i| ClubBot::new(100 + i as u64)).collect();
    let mut season_hasher = Sha256::new();
    let mut turn_count = 0_u8;
    let mut receipts_json = Vec::new();

    while host.phase() == TurnPhase::TalkAndMarket {
        let cur_t = host.turn();
        for s in 0..4_u8 {
            let obs = must_ok(host.observe_seat(Seat(s)), "season pre-pass-a obs");
            let j = must_ok(serde_json::to_string(&obs), "obs json");
            season_hasher.update(format!("OBS|t={cur_t}|s={s}|{j}\n").as_bytes());
            if s == 0 {
                assert_eq!(
                    sha256_hex(j.as_bytes()),
                    expected_s0_hashes_by_turn[usize::from(cur_t - 1)],
                    "Seat(0) observation mismatch at turn {cur_t}"
                );
            }
            let mandate = obs.own_mandate().copied().unwrap_or_else(|| {
                panic!("SeatObservation missing own_mandate at t={cur_t} s={s}")
            });
            for act in bots[usize::from(s)].plan_pass_a(&obs, &mandate) {
                let _ = host.apply_market_action(act);
            }
        }
        must_ok(host.begin_cancel_lock(), "cancel_lock");
        let mut reveals = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = must_ok(host.observe_seat(Seat(s)), "season pass-b obs");
            let mandate = obs
                .own_mandate()
                .copied()
                .unwrap_or_else(|| panic!("SeatObservation missing own_mandate in pass B"));
            let (pass_b, bundle) = bots[usize::from(s)].plan_pass_b_and_bundle(&obs, &mandate);
            for act in pass_b {
                let _ = host.apply_market_action(act);
            }
            let salt = RevealSalt([s.wrapping_mul(17).wrapping_add(cur_t).max(1); 32]);
            reveals.insert(s, OrderReveal { bundle, salt });
        }
        must_ok(host.commit_and_reveal_all(&reveals), "commit_reveal");
        let receipt = must_ok(host.sweep_turn(), "sweep");
        let rj = must_ok(serde_json::to_string(&receipt), "receipt json");
        assert_eq!(
            sha256_hex(rj.as_bytes()),
            expected_rcpt_hashes_by_turn[usize::from(cur_t - 1)],
            "PublicSweepReceipt SHA-256 mismatch at turn {cur_t}"
        );
        season_hasher.update(format!("RCPT|t={cur_t}|{rj}\n").as_bytes());
        receipts_json.push(rj);
        turn_count = cur_t;
    }

    assert_eq!(turn_count, 24);
    assert_eq!(
        receipts_json[0],
        r#"{"turn":1,"non_reveal_seats":[],"regent_acted_seats":[],"converted_to_ai":[],"escrows_finished":[],"escrows_cancelled_or_forfeited":[],"atomic_swaps_settled":0,"converter_batches_run":0,"converter_orders_clamped":0,"checks_cashed":[],"checks_bounced":[],"cartel_evictions":[],"projects_completed":0,"project_orders_clamped":0,"total_upkeep_shortfall_debt":14400,"total_spoilage_clawed_back":8000,"broken_promises":[],"elected_officers":null,"charters_awarded":[],"public_forecasts":[],"newly_activated_shocks":[],"cred_debt_repaid":0,"charter_cred_burned":0,"town_import_cred_spent":0,"works_stipend_cred_minted":0,"batch_trades_cleared":0,"fills":[],"wash_freeze_settlements":[],"season_closed":false,"pbrs_deltas":[9125,9125,9125,9125]}"#
    );
    assert_eq!(
        receipts_json[23],
        r#"{"turn":24,"non_reveal_seats":[],"regent_acted_seats":[],"converted_to_ai":[],"escrows_finished":[],"escrows_cancelled_or_forfeited":[],"atomic_swaps_settled":0,"converter_batches_run":0,"converter_orders_clamped":0,"checks_cashed":[],"checks_bounced":[],"cartel_evictions":[],"projects_completed":0,"project_orders_clamped":0,"total_upkeep_shortfall_debt":14400,"total_spoilage_clawed_back":39281,"broken_promises":[],"elected_officers":[2,3],"charters_awarded":[],"public_forecasts":[],"newly_activated_shocks":[],"cred_debt_repaid":0,"charter_cred_burned":0,"town_import_cred_spent":0,"works_stipend_cred_minted":0,"batch_trades_cleared":8,"fills":[{"turn":24,"kind":"batch_fill","good":{"Raw":0},"from":0,"to":3,"units":1517,"net_units":1395,"cred_amount":2814,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":0},"from":0,"to":2,"units":1517,"net_units":1395,"cred_amount":2814,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":1},"from":1,"to":3,"units":1517,"net_units":1395,"cred_amount":2814,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":1},"from":1,"to":0,"units":1517,"net_units":1395,"cred_amount":2814,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":2},"from":2,"to":0,"units":1517,"net_units":1395,"cred_amount":2814,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":2},"from":2,"to":1,"units":1517,"net_units":1395,"cred_amount":2814,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":3},"from":3,"to":2,"units":1515,"net_units":1393,"cred_amount":2810,"unit_price_cred":1855},{"turn":24,"kind":"batch_fill","good":{"Raw":3},"from":3,"to":1,"units":1515,"net_units":1393,"cred_amount":2810,"unit_price_cred":1855}],"wash_freeze_settlements":[],"season_closed":true,"pbrs_deltas":[-6048,-6041,-6046,-6064]}"#
    );

    assert_eq!(host.phase(), TurnPhase::AwaitingMandateReveal);
    let close_obs0 = must_ok(host.observe_seat(Seat(0)), "close obs0");
    let close_obs0_json = must_ok(serde_json::to_string(&close_obs0), "close obs0 json");
    assert_eq!(close_obs0_json.len(), 57_461);
    assert_eq!(
        sha256_hex(close_obs0_json.as_bytes()),
        "9ec88e797ce72b7c2d63b9b1294f2f30886ba65bd28b90416db3d660f09775ec"
    );
    season_hasher.update(format!("CLOSE_OBS0|{close_obs0_json}\n").as_bytes());
    let actual_stream_sha256 = sha256_hex(&season_hasher.finalize());
    assert_eq!(
        actual_stream_sha256,
        "3d986245c0b1d9da84d61c269e70cc54ca0dd4130d868b36e83cd556cc5673ff"
    );

    let (scoreboard, _log, section, metrics) =
        must_ok(host.finalize_club_season(), "finalize_club_season");
    check_json!(
        FinalScoreboard,
        scoreboard,
        r#"{"table_id":202611,"closed_turn":24,"standings":[{"seat":0,"project_equity":0,"net_asset_equity":-86548,"mandate_bonus":1133,"total_equity":-85415,"credit_tier":"Aaa","hard_credit_defaults":0,"bond_breaches":0,"broken_promises":0,"honor_penalties":0},{"seat":2,"project_equity":0,"net_asset_equity":-86606,"mandate_bonus":944,"total_equity":-85662,"credit_tier":"Aaa","hard_credit_defaults":0,"bond_breaches":0,"broken_promises":0,"honor_penalties":0},{"seat":3,"project_equity":0,"net_asset_equity":-86683,"mandate_bonus":0,"total_equity":-86683,"credit_tier":"Aaa","hard_credit_defaults":0,"bond_breaches":0,"broken_promises":0,"honor_penalties":0},{"seat":1,"project_equity":0,"net_asset_equity":-86801,"mandate_bonus":0,"total_equity":-86801,"credit_tier":"Aaa","hard_credit_defaults":0,"bond_breaches":0,"broken_promises":0,"honor_penalties":0}]}"#
    );
    check_blob_hash!(
        StatePredicateSection,
        section,
        497_077,
        "6fd84fd6de6b68794a5b3775c6d1db96bfd5dfbc3fdac3084f9f0f5ecaac44b6"
    );
    check_json!(
        Vec<SeatSeasonMetrics>,
        metrics,
        r#"[{"seat":0,"held_enough_but_short_turns":[],"covered_with_own_offers_but_short_turns":[],"spoiled_units":197443,"stranded_units_at_close":39279,"stranded_value_at_close":34368},{"seat":1,"held_enough_but_short_turns":[],"covered_with_own_offers_but_short_turns":[],"spoiled_units":197539,"stranded_units_at_close":39285,"stranded_value_at_close":34374},{"seat":2,"held_enough_but_short_turns":[],"covered_with_own_offers_but_short_turns":[],"spoiled_units":197443,"stranded_units_at_close":39279,"stranded_value_at_close":34368},{"seat":3,"held_enough_but_short_turns":[],"covered_with_own_offers_but_short_turns":[],"spoiled_units":197488,"stranded_units_at_close":39330,"stranded_value_at_close":34413}]"#
    );

    println!(
        "SEASON_PARITY_8ABC9ABC: checked_shapes={checked_types} turns={turn_count} close_obs0_len={} stream_sha256={actual_stream_sha256}",
        close_obs0_json.len()
    );
}

#[test]
fn wire_serde_round_trips_and_rejects_invalid_inputs() {
    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Holder {
        #[serde(
            serialize_with = "league_seat_types::wire_serde::serialize_hex32",
            deserialize_with = "league_seat_types::wire_serde::deserialize_hex32"
        )]
        fixed: [u8; 32],
    }

    let h_val = Holder { fixed: [0x11; 32] };
    let h_json = must_ok(serde_json::to_string(&h_val), "serialize Holder");
    assert_eq!(
        must_ok(
            serde_json::from_str::<Holder>(&h_json),
            "deserialize Holder"
        ),
        h_val
    );
    let p = PreimageSecret([0x1F; 32]);
    let j = must_ok(serde_json::to_string(&p), "serialize PreimageSecret");
    assert_eq!(
        must_ok(
            serde_json::from_str::<PreimageSecret>(&j),
            "deserialize PreimageSecret"
        ),
        p
    );
    assert!(serde_json::from_str::<PreimageSecret>("\"abc\"").is_err());
    assert!(serde_json::from_str::<PreimageSecret>(&format!("\"{}\"", "00".repeat(31))).is_err());
    assert!(serde_json::from_str::<PreimageSecret>(&format!("\"{}\"", "zz".repeat(32))).is_err());

    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct OptHolder {
        #[serde(
            serialize_with = "league_seat_types::wire_serde::serialize_opt_hex32",
            deserialize_with = "league_seat_types::wire_serde::deserialize_opt_hex32"
        )]
        key: Option<[u8; 32]>,
        #[serde(
            serialize_with = "league_seat_types::wire_serde::serialize_delivery_ledger",
            deserialize_with = "league_seat_types::wire_serde::deserialize_delivery_ledger"
        )]
        ledger: BTreeMap<(u8, u8, AssetId), Units>,
    }

    let mut ledger = BTreeMap::new();
    ledger.insert((0, 1, AssetId::Raw(0)), 500);
    let oh = OptHolder {
        key: Some([0xAB; 32]),
        ledger,
    };
    let oh_json = must_ok(serde_json::to_string(&oh), "serialize OptHolder");
    assert_eq!(
        must_ok(
            serde_json::from_str::<OptHolder>(&oh_json),
            "deserialize OptHolder"
        ),
        oh
    );
    let none_json = "{\"key\":null,\"ledger\":[]}";
    let none_parsed: OptHolder = must_ok(
        serde_json::from_str(none_json),
        "deserialize OptHolder with null key",
    );
    assert_eq!(none_parsed.key, None);
    let dup_json = "{\"key\":null,\"ledger\":[[[0,1,\"Cred\"],10],[[0,1,\"Cred\"],20]]}";
    assert!(serde_json::from_str::<OptHolder>(dup_json).is_err());

    // Exercise PublicPreviewState round-trips with `eval_vrf_pubkey: Some(...)` and `None`.
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x66; 32]), "SealedEvalSeed");
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x41 + i; 32])).collect();
    let host = must_ok(
        ClubHost::from_explicit_setup(TableId(202_699), 4, 1, [0x55; 32], &salts, eval_seed, 42),
        "ClubHost::from_explicit_setup",
    );
    let obs = must_ok(host.observe_seat(Seat(0)), "observe_seat(0)");
    let mut prev = obs.to_public_preview_state();
    let prev_json = must_ok(serde_json::to_string(&prev), "serialize PublicPreviewState");
    let prev_rt: PublicPreviewState = must_ok(
        serde_json::from_str(&prev_json),
        "deserialize PublicPreviewState",
    );
    assert_eq!(prev_rt.spec.eval_vrf_pubkey, prev.spec.eval_vrf_pubkey);

    prev.spec.eval_vrf_pubkey = None;
    let prev_none_json = must_ok(
        serde_json::to_string(&prev),
        "serialize PublicPreviewState None",
    );
    let prev_none_rt: PublicPreviewState = must_ok(
        serde_json::from_str(&prev_none_json),
        "deserialize PublicPreviewState None",
    );
    assert_eq!(prev_none_rt.spec.eval_vrf_pubkey, None);

    // Exercise seat-types commodity display lookup helpers across hit/miss inputs.
    assert!(league_seat_types::lookup_commodity_display("ALM").is_some());
    assert!(league_seat_types::lookup_commodity_display("AB").is_none());
    assert!(league_seat_types::lookup_commodity_display_bytes(*b"ZZZ").is_none());
    assert_eq!(
        league_seat_types::format_numeraire_display(false),
        league_seat_types::NUMERAIRE_DISPLAY_LABEL
    );
    assert_eq!(
        league_seat_types::format_numeraire_display(true),
        league_seat_types::NUMERAIRE_PLAIN_LABEL
    );
}
