//! State predicates (the approved Step 1): hand-worked cases, agreement with what the Sweep then
//! does, privacy (no predicate reads another seat's private data), determinism across `N = 4, 5, 6`,
//! the optional replay-record section, and the three paper-test metrics.

use std::collections::{BTreeMap, BTreeSet};

use league_core::{
    ActiveCharter, ActiveShock, AssetId, AtomicSwapProposal, CartelDomain, CharterBidOrder,
    CharterRole, CheckProposal, CheckStatus, CommitmentHash, ContractDeliveryOrder,
    ConverterRunOrder, CouncilPlatform, CouncilVoteBallot, CredOffer, CreditCheck, CreditTier,
    DualEscrowProposal, EngineState, EscrowFinishUnlock, EscrowStatus, HARD_TURN_CAP, LoggedTurn,
    MarketAction, OfferProposal, OfferSide, OrderReveal, PreimageSecret, ProjectDeliveryOrder,
    RegentDoctrine, ReplayBeaconMode, ReplayLayoutSeed, ReplayRecord, RevealSalt,
    ScheduledShockSpec, SealedEvalSeed, SealedMandateDeal, SealedOrderBundle, SeasonReplayLog,
    Seat, SeatIdentity, SeatStatePredicates, ShockKind, StatePredicateSection, TableId, TableMode,
    TableSpec, TokenEscrowVault, Turn, TurnEngine, TurnPhase, Units, attach_state_predicates,
    coalition_reach, compute_final_marked_net_asset_equity, compute_order_commitment,
    compute_season_metrics, compute_seat_state_predicates, covered_with_own_offers_but_short_turns,
    derive_turn_beacon, held_enough_but_short_turns, mark_good_holding_at_close, replay_season,
    replay_season_with_state_predicates, scarcity_and_tariffs, season_end_and_marks,
    spoilage_and_conversion, spoiled_units_over_season, stranded_stock_at_close, upkeep_and_debt,
};

fn must<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn json<T: serde::Serialize>(value: &T) -> Vec<u8> {
    must(serde_json::to_vec(value), "serialize")
}

/// Rebuilds a [`TurnEngine`] around an edited [`EngineState`] (the engine serializes as `{state}`).
fn engine_from_state(state: &EngineState) -> TurnEngine {
    let value = serde_json::json!({ "state": must(serde_json::to_value(state), "state value") });
    must(serde_json::from_value(value), "engine from state")
}

/// A deterministic Eval table (no VRF key) on the standard ring, with mandates dealt from fixed
/// fixture entropy and the Standing Regent switched off so an idle seat stays idle.
fn quiet_spec(tid: TableId, n: u8) -> (TableSpec, SealedMandateDeal) {
    let salts: Vec<RevealSalt> = (0..n).map(|s| RevealSalt([0x40 + s; 32])).collect();
    let (mut spec, deal) = must(
        TableSpec::standard_chiral_ring_with_explicit_deal(tid, n, 3, [0x5c; 32], &salts),
        "quiet spec",
    );
    for seat in &mut spec.seats {
        seat.regent = None;
    }
    (spec, deal)
}

fn commit_and_reveal(
    engine: &mut TurnEngine,
    tid: TableId,
    turn: u8,
    bundles: Vec<SealedOrderBundle>,
) -> (BTreeMap<u8, CommitmentHash>, BTreeMap<u8, OrderReveal>) {
    must(engine.advance_to_commit(), "advance_to_commit");
    let mut commits = BTreeMap::new();
    let mut reveals = BTreeMap::new();
    for (idx, bundle) in bundles.into_iter().enumerate() {
        let s = idx as u8;
        let rev = OrderReveal {
            bundle,
            salt: RevealSalt([s.wrapping_mul(29).wrapping_add(turn).max(1); 32]),
        };
        let ch = must(
            compute_order_commitment(tid, Turn(turn), Seat(s), &rev),
            "commit",
        );
        must(engine.submit_order_commitment(Seat(s), ch), "submit commit");
        commits.insert(s, ch);
        reveals.insert(s, rev);
    }
    must(engine.advance_to_reveal(), "advance_to_reveal");
    for (&s, rev) in &reveals {
        must(engine.submit_order_reveal(Seat(s), rev.clone()), "reveal");
    }
    (commits, reveals)
}

/// A mid-season state with holdings, offers from several seats, a cartel domain and active shocks,
/// built by editing a fresh Turn-1 state. Seat 0 is the asking seat in the tests below.
fn busy_state(n: u8) -> (EngineState, SealedMandateDeal) {
    let tid = TableId(7_000 + u64::from(n));
    let (spec, deal) = quiet_spec(tid, n);
    let engine = must(TurnEngine::new(spec), "engine");
    let mut st = engine.state().clone();
    st.turn = 9;
    let nn = usize::from(n);
    for (s, seat) in st.seats.iter_mut().enumerate() {
        for g in 0..nn {
            seat.raw[g] = 1_000 * (g as Units + 1) + 37 * s as Units;
            seat.refined[g] = 500 * ((g + s) % 3) as Units;
        }
        seat.cred = 9_000 + 100 * s as Units;
        seat.cred_debt = 250 * s as Units;
    }
    st.offers.insert(
        1,
        CredOffer {
            offer_id: 1,
            maker: 2,
            side: OfferSide::Buy,
            good: AssetId::Refined(0),
            remaining_units: 600,
            price_per_unit_cred: 1_500,
            locked_cred: 900,
            domain_id: None,
            is_hybrid: false,
            created_turn: 8,
        },
    );
    st.offers.insert(
        2,
        CredOffer {
            offer_id: 2,
            maker: 0,
            side: OfferSide::Sell,
            good: AssetId::Raw(0),
            remaining_units: 2_500,
            price_per_unit_cred: 9_000,
            locked_cred: 0,
            domain_id: None,
            is_hybrid: false,
            created_turn: 8,
        },
    );
    st.offers.insert(
        3,
        CredOffer {
            offer_id: 3,
            maker: 1,
            side: OfferSide::Sell,
            good: AssetId::Raw(2),
            remaining_units: 1_200,
            price_per_unit_cred: 8_000,
            locked_cred: 0,
            domain_id: Some(0),
            is_hybrid: false,
            created_turn: 8,
        },
    );
    st.domains.push(CartelDomain {
        domain_id: 0,
        members: BTreeSet::from([1, 2]),
    });
    st.active_shocks.push(ActiveShock {
        target_seat: 1,
        kind: ShockKind::SpoilageSpike,
        magnitude_bps: 5_000,
        expires_after_turn: 9,
    });
    st.active_shocks.push(ActiveShock {
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_500,
        expires_after_turn: 9,
    });
    st.active_shocks.push(ActiveShock {
        target_seat: 1,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 10_000,
        expires_after_turn: 9,
    });
    (st, deal)
}

// --- Hand-worked cases on the standard 4-seat ring --------------------------------------------
// Standard ring (types.rs `build_chiral_ring_spec_with_deal`): seat i owes 1_800 of Refined(i-1)
// per turn, yields 10_000 of Raw(i), decays raw at 2_000 bps, runs one recipe
// (Raw(i+1) 2_000 + Raw(i+2) 2_000 -> 3_500 Refined(i), at most 4 runs), tariffs 800 raw / 600
// refined. Parities: refined 2_000; raw 3_500 * 2_000 / 4_000 = 1_750 (scoring.rs `from_spec`).

#[test]
fn hand_worked_upkeep_and_debt() {
    let (spec, _) = quiet_spec(TableId(11), 4);
    let engine = must(TurnEngine::new(spec), "engine");
    let mut st = engine.state().clone();
    // Seat 0 owes Refined(3). Holds 500 of 1_800: shortfall 1_300;
    // debt = ceil(1_300 * 2_000 / 1_000) = 2_600; AAA interest 400 bps.
    st.seats[0].refined[3] = 500;
    let p = must(upkeep_and_debt(&st, Seat(0)), "upkeep");
    assert_eq!(p.upkeep_good, 3);
    assert_eq!(p.owed_units, 1_800);
    assert_eq!(p.held_units, 500);
    assert_eq!(p.shortfall_units, 1_300);
    assert_eq!(p.debt_added_at_sweep, 2_600);
    assert_eq!(p.interest_bps, 400);
    // Covered: 2_000 held >= 1_800 owed, no shortfall, no debt.
    st.seats[0].refined[3] = 2_000;
    st.seats[0].credit_tier = league_core::CreditTier::B;
    let p = must(upkeep_and_debt(&st, Seat(0)), "upkeep covered");
    assert_eq!((p.shortfall_units, p.debt_added_at_sweep), (0, 0));
    assert_eq!(p.interest_bps, 1_800);
    assert!(upkeep_and_debt(&st, Seat(4)).is_err());
}

#[test]
fn hand_worked_spoilage_and_conversion() {
    let (spec, _) = quiet_spec(TableId(12), 4);
    let engine = must(TurnEngine::new(spec), "engine");
    let mut st = engine.state().clone();
    st.seats[0].raw[1] = 3_000;
    st.seats[0].raw[2] = 5_000;
    // Own good: 0 held + 10_000 yield -> floor(10_000 * 20%) = 2_000.
    // Raw(1): floor(3_000 * 20%) = 600. Raw(2): floor(5_000 * 20%) = 1_000. Total 3_600.
    // Recipe (Raw(1) 2_000, Raw(2) 2_000): min(3_000/2_000, 5_000/2_000) = min(1, 2) = 1 batch; cap 4.
    let p = must(spoilage_and_conversion(&st, Seat(0)), "spoilage");
    assert_eq!(p.own_raw_good, 0);
    assert_eq!(p.own_raw_yield_at_sweep, 10_000);
    assert_eq!(p.spoilage_bps_by_good, vec![2_000; 4]);
    assert_eq!(p.spoil_units_held_by_good, vec![2_000, 600, 1_000, 0]);
    assert_eq!(p.spoil_units_on_resting_offers, 0);
    assert_eq!(p.spoil_units_total, 3_600);
    assert_eq!(p.workshop_capacity_runs_left, 4);
    assert_eq!(p.recipes.len(), 1);
    assert_eq!(p.recipes[0].recipe_idx, 0);
    assert_eq!(p.recipes[0].batches_covered_by_inputs, 1);
    assert_eq!(p.recipes[0].output_good, 0);
    assert_eq!(p.recipes[0].output_units_per_batch, 3_500);

    // SpoilageSpike 5_000 on Raw(1): boost = ceil(8_000 * 5_000 / 10_000) = 4_000, rate 6_000:
    // floor(3_000 * 60%) = 1_800. SupplyFreeze 2_500 on Raw(0): yield 10_000 - 2_500 = 7_500,
    // spoil floor(7_500 * 20%) = 1_500. Resting sell of 2_500 Raw(0): floor(2_500 * 20%) = 500.
    st.active_shocks.push(ActiveShock {
        target_seat: 1,
        kind: ShockKind::SpoilageSpike,
        magnitude_bps: 5_000,
        expires_after_turn: 1,
    });
    st.active_shocks.push(ActiveShock {
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_500,
        expires_after_turn: 1,
    });
    st.offers.insert(
        9,
        CredOffer {
            offer_id: 9,
            maker: 0,
            side: OfferSide::Sell,
            good: AssetId::Raw(0),
            remaining_units: 2_500,
            price_per_unit_cred: 5_000,
            locked_cred: 0,
            domain_id: None,
            is_hybrid: false,
            created_turn: 1,
        },
    );
    let p = must(spoilage_and_conversion(&st, Seat(0)), "spoilage shocked");
    assert_eq!(p.own_raw_yield_at_sweep, 7_500);
    assert_eq!(p.spoilage_bps_by_good[1], 6_000);
    assert_eq!(p.spoil_units_held_by_good, vec![1_500, 1_800, 1_000, 0]);
    assert_eq!(p.spoil_units_on_resting_offers, 500);
    assert_eq!(p.spoil_units_total, 1_500 + 1_800 + 1_000 + 500);
    let p3 = must(spoilage_and_conversion(&st, Seat(3)), "seat 3");
    assert_eq!(p3.own_raw_good, 3);
}

#[test]
fn hand_worked_scarcity_and_tariffs() {
    let (st, _) = busy_state(4);
    let goods = must(scarcity_and_tariffs(&st, Seat(0)), "scarcity");
    assert_eq!(goods.len(), 8);
    let by_good: BTreeMap<AssetId, _> = goods.iter().map(|g| (g.good, g)).collect();
    // Raw(1) feeds seat 0 (next 1) and seat 3 (next 2 = 5 mod 4 = 1): 2 seats. Refined(3) is
    // seat 0's upkeep only: 1 seat.
    assert_eq!(by_good[&AssetId::Raw(1)].seats_needing, 2);
    assert_eq!(by_good[&AssetId::Refined(3)].seats_needing, 1);
    assert_eq!(by_good[&AssetId::Raw(1)].producer_seat, 1);
    // Seat 0's own holdings (busy_state: raw[g] = 1_000 (g + 1)).
    assert_eq!(by_good[&AssetId::Raw(1)].own_units, 2_000);
    // Open-book offers: seat 0's Raw(0) sell (2_500) and seat 2's Refined(0) bid (600). Seat 1's
    // Raw(2) sell sits in domain 0's book and is not counted.
    assert_eq!(by_good[&AssetId::Raw(0)].open_book_units_offered, 2_500);
    assert_eq!(by_good[&AssetId::Refined(0)].open_book_units_bid, 600);
    assert_eq!(by_good[&AssetId::Raw(2)].open_book_units_offered, 0);
    // Tariffs: base 800 raw / 600 refined; seat 1's goods under a 100% tariff holiday -> 0.
    assert_eq!(by_good[&AssetId::Raw(0)].open_book_tariff_bps, 800);
    assert_eq!(by_good[&AssetId::Refined(2)].open_book_tariff_bps, 600);
    assert_eq!(by_good[&AssetId::Raw(1)].open_book_tariff_bps, 0);
    assert_eq!(by_good[&AssetId::Refined(1)].open_book_tariff_bps, 0);
    // Table-wide holdings (busy_state: raw[g] = 1_000 (g + 1) + 37 s, refined[g] =
    // 500 ((g + s) % 3)), free plus resting sells:
    // Raw(1): 4 * 2_000 + 37 * (0 + 1 + 2 + 3) = 8_222.
    // Raw(0): 4 * 1_000 + 222 + seat 0's 2_500 on its open-book sell = 6_722.
    // Raw(2): 4 * 3_000 + 222 + seat 1's 1_200 on its domain-book sell = 13_422.
    // Refined(0): 0 + 500 + 1_000 + 0 = 1_500.
    assert_eq!(by_good[&AssetId::Raw(1)].table_units_held, 8_222);
    assert_eq!(by_good[&AssetId::Raw(0)].table_units_held, 6_722);
    assert_eq!(by_good[&AssetId::Raw(2)].table_units_held, 13_422);
    assert_eq!(by_good[&AssetId::Refined(0)].table_units_held, 1_500);
    // The domain book is counted apart from the open book.
    assert_eq!(by_good[&AssetId::Raw(2)].domain_book_units_offered, 1_200);
    assert_eq!(by_good[&AssetId::Raw(2)].open_book_units_offered, 0);
    assert_eq!(by_good[&AssetId::Raw(0)].domain_book_units_offered, 0);
    assert_eq!(by_good[&AssetId::Refined(0)].domain_book_units_bid, 0);
}

#[test]
fn hand_worked_coalition_reach() {
    let (spec, _) = quiet_spec(TableId(14), 4);
    let engine = must(TurnEngine::new(spec), "engine");
    let st = engine.state();
    // {0, 1, 2}: upkeep goods 3 (seat 0), 0 (seat 1), 1 (seat 2).
    // Good 0: seat 0's recipe takes Raw(1), Raw(2), both members' -> makeable.
    // Good 1: seat 1's recipe takes Raw(2), Raw(3); seat 3 is out -> not. Good 3: seat 3 is out -> not.
    let c = must(coalition_reach(st, Seat(1), &[2, 0, 1]), "coalition");
    assert_eq!(c.members, vec![0, 1, 2]);
    assert_eq!(c.upkeep_goods_makeable, vec![0]);
    assert_eq!(c.upkeep_goods_not_makeable, vec![1, 3]);
    // Any pair feeds no member (design-v2 1.3): {0, 1} upkeep goods 3 and 0; good 0 needs Raw(2).
    let pair = must(coalition_reach(st, Seat(0), &[0, 1]), "pair");
    assert!(pair.upkeep_goods_makeable.is_empty());
    assert_eq!(pair.upkeep_goods_not_makeable, vec![0, 3]);
    // Refusals: too small, too large, duplicate, outside the table, without the asking seat.
    assert!(coalition_reach(st, Seat(0), &[0]).is_err());
    assert!(coalition_reach(st, Seat(0), &[0, 1, 2, 3]).is_err());
    assert!(coalition_reach(st, Seat(0), &[0, 0]).is_err());
    assert!(coalition_reach(st, Seat(0), &[0, 7]).is_err());
    assert!(coalition_reach(st, Seat(0), &[1, 2]).is_err());
}

#[test]
fn coalition_bounds_hold_on_the_standard_ring_for_4_5_6() {
    // design-v2 1.3: no pair feeds a member; a 3-seat bloc feeds at most 1; a 4-seat bloc at most 2.
    for n in [4_u8, 5, 6] {
        let (spec, _) = quiet_spec(TableId(15), n);
        let engine = must(TurnEngine::new(spec), "engine");
        for s in 0..n {
            let all = must(
                compute_seat_state_predicates(engine.state(), Seat(s)),
                "predicates",
            )
            .coalitions;
            let expected: usize = (1..n - 1)
                .map(|k| {
                    // C(n - 1, k): sets of size k + 1 that include s.
                    let (top, bottom) = (u64::from(n - 1), u64::from(k));
                    ((top - bottom + 1)..=top).product::<u64>() as usize
                        / (1..=bottom).product::<u64>() as usize
                })
                .sum();
            assert_eq!(all.len(), expected);
            for c in &all {
                assert!(c.members.contains(&s));
                let fed = c.upkeep_goods_makeable.len();
                match c.members.len() {
                    2 => assert_eq!(fed, 0),
                    3 => assert!(fed <= 1),
                    4 => assert!(fed <= 2),
                    _ => {}
                }
            }
        }
    }
}

#[test]
fn hand_worked_season_end_and_marks() {
    let (st, _) = busy_state(4);
    let p = must(season_end_and_marks(&st, Seat(0)), "season end");
    // Turn 9: earliest close is turn 24's Sweep (hazard 3_500 > 0), 15 Sweeps after this one;
    // the latest is the hard cap 32 (no hazard is 10_000), 23 after.
    assert!(!p.season_closed);
    assert_eq!(p.turns_until_earliest_close, 15);
    assert_eq!(p.turns_until_latest_close, 23);
    let refined0 = p
        .holdings
        .iter()
        .find(|h| h.good == AssetId::Refined(0))
        .cloned();
    let Some(refined0) = refined0 else {
        panic!("Refined(0) holding")
    };
    // Seat 0 holds 0 Refined(0) in busy_state ((0 + 0) % 3 = 0) -> nothing to mark.
    assert_eq!(refined0.units, 0);
    assert_eq!(refined0.marked_value, 0);

    let mut st = st;
    st.seats[0].refined[0] = 1_000;
    let p = must(season_end_and_marks(&st, Seat(0)), "season end 2");
    let Some(h) = p.holdings.iter().find(|h| h.good == AssetId::Refined(0)) else {
        panic!("Refined(0) holding")
    };
    // Seat 2's bid: 600 at 1_500 -> 900 > floor 600 * 2_000 * 50% / 1_000 = 600, so 600 units at 900;
    // the other 400 at the floor: 400 * 2_000 * 50% / 1_000 = 400. Mark 1_300.
    assert_eq!(h.units, 1_000);
    assert_eq!(h.units_marked_by_bids, 600);
    assert_eq!(h.units_at_floor, 400);
    assert_eq!(h.value_at_floor, 400);
    assert_eq!(h.marked_value, 1_300);
    assert_eq!(
        h.marked_value,
        mark_good_holding_at_close(&st, 0, AssetId::Refined(0), 1_000)
    );
    // Raw(0): 1_000 free + 2_500 on seat 0's own resting sell = 3_500, all at the floor:
    // 3_500 * 1_750 * 50% / 1_000 = 3_062 (floor of 3_062.5).
    let Some(r0) = p.holdings.iter().find(|h| h.good == AssetId::Raw(0)) else {
        panic!("Raw(0) holding")
    };
    assert_eq!(
        (r0.units, r0.units_at_floor, r0.value_at_floor),
        (3_500, 3_500, 3_062)
    );
    // Project 1 takes Refined(0) 1_000 + Refined(1) 1_000; seat 0 holds 1_000 and 500 -> 0 batches.
    let Some(proj1) = p.projects.iter().find(|c| c.project_id == 1) else {
        panic!("project 1")
    };
    assert_eq!(proj1.inputs[0].held_units, 1_000);
    assert_eq!(proj1.inputs[1].held_units, 500);
    assert_eq!(proj1.batches_covered, 0);
    // 2 batches: 2_000 / 1_000 = 2 and 2_500 / 1_000 = 2 -> 2.
    let mut two = st.clone();
    two.seats[0].refined[0] = 2_000;
    two.seats[0].refined[1] = 2_500;
    let p2b = must(season_end_and_marks(&two, Seat(0)), "two batches");
    let Some(c) = p2b.projects.iter().find(|c| c.project_id == 1) else {
        panic!("project 1")
    };
    assert_eq!(c.batches_covered, 2);
    // The scarcer input binds: 3_500 / 1_000 = 3 but 1_200 / 1_000 = 1 -> 1.
    two.seats[0].refined[0] = 3_500;
    two.seats[0].refined[1] = 1_200;
    let p2c = must(season_end_and_marks(&two, Seat(0)), "scarcer binds");
    let Some(c) = p2c.projects.iter().find(|c| c.project_id == 1) else {
        panic!("project 1")
    };
    assert_eq!(c.batches_covered, 1);
    // Project 100 takes 500 of every refined good; seat 0 holds 0 Refined(3) -> 0.
    let Some(c) = p2c.projects.iter().find(|c| c.project_id == 100) else {
        panic!("project 100")
    };
    assert_eq!(c.batches_covered, 0);

    // Late season: turn 30 earliest 0 (this Sweep may close), latest 2 (turn 32).
    st.turn = 30;
    let p = must(season_end_and_marks(&st, Seat(0)), "late");
    assert_eq!(
        (p.turns_until_earliest_close, p.turns_until_latest_close),
        (0, 2)
    );
    // A certain hazard at turn 31 makes it the latest close; zero hazards push the earliest to 32.
    st.spec.closing_hazard_bps = [0; 8];
    st.spec.closing_hazard_bps[7] = 10_000;
    let p = must(season_end_and_marks(&st, Seat(0)), "certain");
    assert_eq!(
        (p.turns_until_earliest_close, p.turns_until_latest_close),
        (1, 1)
    );
    st.spec.closing_hazard_bps = [0; 8];
    let p = must(season_end_and_marks(&st, Seat(0)), "none");
    assert_eq!(
        (p.turns_until_earliest_close, p.turns_until_latest_close),
        (2, 2)
    );
    st.phase = TurnPhase::AwaitingMandateReveal;
    let p = must(season_end_and_marks(&st, Seat(0)), "closed");
    assert!(p.season_closed);
    assert_eq!(
        (p.turns_until_earliest_close, p.turns_until_latest_close),
        (0, 0)
    );
}

// --- What the Sweep then does ------------------------------------------------------------------

/// For a busy state where every seat reveals an empty bundle (nothing changes), predicates 1 and 2
/// equal what the Sweep then does, per seat.
#[test]
fn predicted_shortfall_debt_and_spoilage_equal_the_sweep() {
    for n in [4_u8, 5, 6] {
        let (st, _) = busy_state(n);
        let before: Vec<SeatStatePredicates> = (0..n)
            .map(|s| must(compute_seat_state_predicates(&st, Seat(s)), "predicates"))
            .collect();
        let mut engine = engine_from_state(&st);
        let tid = st.table_id;
        let _ = commit_and_reveal(
            &mut engine,
            tid,
            st.turn,
            vec![SealedOrderBundle::default(); usize::from(n)],
        );
        let receipt = must(
            engine.sweep_turn(derive_turn_beacon(5, tid, Turn(st.turn))),
            "sweep",
        );
        let after = engine.state();
        let mut debt_sum = 0;
        let mut spoil_sum = 0;
        for s in 0..usize::from(n) {
            let p = &before[s];
            let interest = league_core::half_up_bps(st.seats[s].cred_debt, p.upkeep.interest_bps);
            // Debt added = debt after - debt before - interest on the old debt.
            assert_eq!(
                after.seats[s].cred_debt - st.seats[s].cred_debt - interest,
                p.upkeep.debt_added_at_sweep,
                "n={n} seat {s} debt"
            );
            let g = usize::from(p.upkeep.upkeep_good);
            assert_eq!(
                p.upkeep.owed_units - (st.seats[s].refined[g] - after.seats[s].refined[g]),
                p.upkeep.shortfall_units,
                "n={n} seat {s} shortfall"
            );
            // Raw removed = held + yield - left, per good, plus the resting-offer shrink.
            for gg in 0..usize::from(n) {
                let minted = if gg == s {
                    p.spoilage.own_raw_yield_at_sweep
                } else {
                    0
                };
                assert_eq!(
                    st.seats[s].raw[gg] + minted - after.seats[s].raw[gg],
                    p.spoilage.spoil_units_held_by_good[gg],
                    "n={n} seat {s} good {gg} spoil"
                );
            }
            debt_sum += p.upkeep.debt_added_at_sweep;
            spoil_sum += p.spoilage.spoil_units_total;
        }
        assert_eq!(receipt.total_upkeep_shortfall_debt, debt_sum);
        assert_eq!(receipt.total_spoilage_clawed_back, spoil_sum);
    }
}

/// A quiet season replayed with the section: every turn's predicted shortfall, debt and spoilage
/// equal the Sweep's own per-seat outcome, and at the close the marks equal the close mark.
fn quiet_season_log(n: u8) -> (SeasonReplayLog, TurnEngine) {
    let tid = TableId(8_000 + u64::from(n));
    let salts: Vec<RevealSalt> = (0..n).map(|s| RevealSalt([0x21 + s; 32])).collect();
    let eval_seed = must(SealedEvalSeed::try_from_bytes([0x6d; 32]), "eval seed");
    let (mut spec, deal, eval_seed) = must(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            tid, n, 9, [0x3e; 32], &salts, eval_seed,
        ),
        "spec",
    );
    for seat in &mut spec.seats {
        seat.regent = None;
    }
    let mut engine = must(TurnEngine::new(spec.clone()), "engine");
    let mut turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) = commit_and_reveal(
            &mut engine,
            tid,
            t,
            vec![SealedOrderBundle::default(); usize::from(n)],
        );
        let (beacon, proof) = must(eval_seed.prove_turn(tid, Turn(t)), "prove");
        let receipt = must(engine.sweep_turn_with_eval_vrf(&proof), "sweep");
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(proof.to_vec()),
            drawn_shocks: receipt.newly_drawn_shocks.clone(),
        });
        if receipt.season_closed {
            break;
        }
    }
    let log = SeasonReplayLog {
        eval_vrf_pubkey: spec.eval_vrf_pubkey,
        spec,
        mode: TableMode::Eval,
        seed: must(ReplayLayoutSeed::try_clear(9), "clear seed 9"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        sealed_mandate_deal: deal,
        mandate_commits: vec![None; usize::from(n)],
        turns,
        mandate_reveals: vec![None; usize::from(n)],
    };
    (log, engine)
}

#[test]
fn quiet_seasons_predictions_match_every_sweep_and_the_close_mark() {
    for n in [4_u8, 5, 6] {
        let (log, mut live) = quiet_season_log(n);
        let (outcome, section) = must(replay_season_with_state_predicates(&log), "replay");
        assert_eq!(section.turns.len(), log.turns.len());
        let mut any_spoil = false;
        let mut any_shortfall = false;
        for (rec, receipt) in section.turns.iter().zip(&outcome.receipts) {
            assert_eq!(rec.turn, receipt.turn);
            let mut debt_sum = 0;
            let mut spoil_sum = 0;
            for (p, o) in rec.at_turn_start.iter().zip(&rec.sweep) {
                assert_eq!(p.seat, o.seat);
                assert_eq!(p.turn, rec.turn);
                assert_eq!(p.upkeep.shortfall_units, o.upkeep_shortfall_units);
                assert_eq!(p.upkeep.debt_added_at_sweep, o.shortfall_debt_added);
                assert_eq!(p.spoilage.spoil_units_total, o.spoiled_units);
                any_spoil |= o.spoiled_units > 0;
                any_shortfall |= o.upkeep_shortfall_units > 0;
                debt_sum += o.shortfall_debt_added;
                spoil_sum += o.spoiled_units;
            }
            assert_eq!(debt_sum, receipt.total_upkeep_shortfall_debt);
            assert_eq!(spoil_sum, receipt.total_spoilage_clawed_back);
        }
        assert!(any_spoil && any_shortfall);

        // The close: marks + CRED - debt (no offers, escrows or checks on a quiet table) equal the
        // close-marked net asset equity and the scoreboard's.
        let closed = live.state().clone();
        assert_eq!(closed.phase, TurnPhase::AwaitingMandateReveal);
        let board = must(live.finalize_season(), "finalize");
        for p in &section.at_close {
            assert!(p.season_end.season_closed);
            let s = usize::from(p.seat);
            let marks: Units = p.season_end.holdings.iter().map(|h| h.marked_value).sum();
            let nae = closed.seats[s].cred - closed.seats[s].cred_debt + marks;
            assert_eq!(nae, compute_final_marked_net_asset_equity(&closed, p.seat));
            let Some(entry) = board.standings.iter().find(|e| e.seat == p.seat) else {
                panic!("standing")
            };
            assert_eq!(entry.net_asset_equity, nae);
        }
    }
}

/// With bids, an own resting sell and a mid-season close forced at the hard cap: the predicted
/// marks in the closed state equal the close mark the scoreboard uses.
#[test]
fn predicted_marks_equal_the_close_mark_with_standing_bids() {
    let (mut st, _) = busy_state(4);
    st.turn = HARD_TURN_CAP;
    // Seat 0 owes Refined(3), so its 1_000 Refined(0) is still held at the close.
    st.seats[0].refined[0] = 1_000;
    let mut engine = engine_from_state(&st);
    let tid = st.table_id;
    let _ = commit_and_reveal(
        &mut engine,
        tid,
        st.turn,
        vec![SealedOrderBundle::default(); 4],
    );
    let receipt = must(
        engine.sweep_turn(derive_turn_beacon(6, tid, Turn(st.turn))),
        "sweep",
    );
    assert!(receipt.season_closed);
    let closed = engine.state().clone();
    let board = must(engine.finalize_season(), "finalize");
    for s in 0..4_u8 {
        let p = must(season_end_and_marks(&closed, Seat(s)), "marks");
        let marks: Units = p.holdings.iter().map(|h| h.marked_value).sum();
        let seat = &closed.seats[usize::from(s)];
        let own_buy_locks: Units = closed
            .offers
            .values()
            .filter(|o| o.maker == s && o.side == OfferSide::Buy && o.remaining_units > 0)
            .map(|o| o.locked_cred)
            .sum();
        let nae = seat.cred + own_buy_locks - seat.cred_debt + marks;
        let Some(entry) = board.standings.iter().find(|e| e.seat == s) else {
            panic!("standing")
        };
        assert_eq!(entry.net_asset_equity, nae, "seat {s}");
        for h in &p.holdings {
            assert_eq!(
                h.marked_value,
                mark_good_holding_at_close(&closed, s, h.good, h.units)
            );
            // Independent invariants: every unit is marked by a bid or at the floor, and the
            // floor value is 50% of base value over the pooled floor units, rounded down once.
            assert_eq!(h.units_marked_by_bids + h.units_at_floor, h.units.max(0));
            let parity = match h.good {
                AssetId::Raw(g) => closed.parities.raw_per_unit[usize::from(g)],
                AssetId::Refined(g) => closed.parities.refined_per_unit[usize::from(g)],
                AssetId::Cred => 0,
            };
            assert_eq!(h.value_at_floor, h.units_at_floor * parity / 2_000);
            let bid_part = h.marked_value - h.value_at_floor;
            if h.units_marked_by_bids == 0 {
                assert_eq!(bid_part, 0);
            }
        }
    }
    // Hand computation for seat 0's Refined(0) at the close: the only resting bid is seat 2's
    // 600 at 1_500 (locked 900). 600 * 1_500 / 1_000 = 900 > 600 * 2_000 * 50% / 1_000 = 600,
    // so 600 units mark at 900; the other 400 pool at the floor: 400 * 2_000 / 2_000 = 400.
    let p0 = must(season_end_and_marks(&closed, Seat(0)), "seat 0");
    let Some(h) = p0.holdings.iter().find(|h| h.good == AssetId::Refined(0)) else {
        panic!("Refined(0)")
    };
    assert_eq!(
        (h.units, h.units_marked_by_bids, h.units_at_floor),
        (1_000, 600, 400)
    );
    assert_eq!((h.value_at_floor, h.marked_value), (400, 1_300));
}

// --- Privacy -----------------------------------------------------------------------------------

type Predicate = fn(&EngineState, Seat) -> Vec<u8>;

fn p1(st: &EngineState, s: Seat) -> Vec<u8> {
    json(&must(upkeep_and_debt(st, s), "p1"))
}
fn p2(st: &EngineState, s: Seat) -> Vec<u8> {
    json(&must(spoilage_and_conversion(st, s), "p2"))
}
fn p3(st: &EngineState, s: Seat) -> Vec<u8> {
    json(&must(scarcity_and_tariffs(st, s), "p3"))
}
fn p4(st: &EngineState, s: Seat) -> Vec<u8> {
    json(&must(league_core::all_coalitions_reach(st, s), "p4"))
}
fn p5(st: &EngineState, s: Seat) -> Vec<u8> {
    json(&must(season_end_and_marks(st, s), "p5"))
}
fn all(st: &EngineState, s: Seat) -> Vec<u8> {
    json(&must(compute_seat_state_predicates(st, s), "all"))
}

/// `busy_state` with the bid walk in play for the asking seat 0: it holds 1_000 Refined(0), which
/// seat 2 bids for above the floor (offer 1), and 3_500 Raw(0) (free plus its own resting sell),
/// which seat 1 bids for above the floor (offer 4, open book).
fn privacy_state(n: u8) -> (EngineState, SealedMandateDeal) {
    let (mut st, deal) = busy_state(n);
    st.seats[0].refined[0] = 1_000;
    st.offers.insert(
        4,
        CredOffer {
            offer_id: 4,
            maker: 1,
            side: OfferSide::Buy,
            good: AssetId::Raw(0),
            remaining_units: 1_000,
            price_per_unit_cred: 1_500,
            locked_cred: 1_500,
            domain_id: None,
            is_hybrid: false,
            created_turn: 8,
        },
    );
    (st, deal)
}

/// Each change touches only data that stays private under the 2026-11-16 ruling (mandate cards and
/// their commitments, committed-but-unrevealed moves, unpublished randomness), on `other`.
fn private_variants(
    st: &EngineState,
    deal: &SealedMandateDeal,
    other: u8,
) -> Vec<(&'static str, EngineState)> {
    let o = usize::from(other);
    let mut out = Vec::new();

    let mut v = st.clone();
    v.spec.dealt_mandate_commits[o] = CommitmentHash([0x5a ^ other; 32]);
    out.push(("dealt mandate commitment", v));

    let mut v = st.clone();
    v.mandate_commits[o] = Some(CommitmentHash([0x6b ^ other; 32]));
    out.push(("Turn-1 mandate commitment", v));

    let mut v = st.clone();
    v.mandate_reveals[o] = Some(deal.seat_reveals[(o + 1) % deal.seat_reveals.len()].clone());
    out.push(("mandate card", v));

    let mut v = st.clone();
    v.turn_commits
        .insert(other, CommitmentHash([0x7c ^ other; 32]));
    out.push(("unrevealed turn commitment", v));

    let mut v = st.clone();
    // Every field the sealed bundle carries, none left at its default.
    let partner = if other == 1 { 2 } else { 1 };
    let bundle = SealedOrderBundle {
        converter_runs: vec![ConverterRunOrder {
            recipe_idx: 0,
            runs: 4,
        }],
        project_deliveries: vec![ProjectDeliveryOrder {
            project_id: 1,
            batches: 2,
        }],
        atomic_swaps: vec![AtomicSwapProposal {
            swap_id: 900 + u64::from(other),
            maker: other,
            taker: partner,
            maker_asset: AssetId::Raw(other),
            maker_amount: 1_000,
            taker_asset: AssetId::Cred,
            taker_amount: 1_300,
            domain_id: None,
        }],
        contract_deliveries: vec![ContractDeliveryOrder {
            counterparty: partner,
            good: AssetId::Refined(other),
            gross_amount: 700,
            domain_id: Some(0),
            dual_escrow_id: Some(41),
        }],
        dual_escrows: vec![DualEscrowProposal {
            proposal_id: 41,
            buyer: partner,
            seller: other,
            good: AssetId::Refined(other),
            required_units: 700,
            payment_cred: 1_400,
            buyer_bond_cred: 300,
            seller_bond_cred: 200,
            deadline_turn: 12,
            domain_id: Some(0),
        }],
        debt_repayment_cred: 200,
        escrow_finishes: vec![EscrowFinishUnlock {
            escrow_id: 5,
            preimage: Some(PreimageSecret([0x3c ^ other; 32])),
        }],
        check_cashes: vec![3],
        council_vote: Some(CouncilVoteBallot {
            trade_minister: other,
            cartel_chair: other,
        }),
        charter_bids: vec![CharterBidOrder {
            role: CharterRole::OracleAuditor,
            bid_cred: 300,
        }],
    };
    v.turn_reveals.insert(
        other,
        OrderReveal {
            bundle,
            salt: RevealSalt([0x8d ^ other; 32]),
        },
    );
    out.push(("sealed order bundle", v));

    let mut v = st.clone();
    v.drawn_shocks.push(ScheduledShockSpec {
        hint_turn: v.turn,
        forecast_turn: v.turn + 1,
        realize_turn: v.turn + 2,
        target_seat: other,
        kind: ShockKind::SpoilageSpike,
        magnitude_bps: 9_000,
    });
    v.drawn_shocks.push(ScheduledShockSpec {
        hint_turn: v.turn,
        forecast_turn: v.turn + 1,
        realize_turn: v.turn + 2,
        target_seat: 0,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 10_000,
    });
    out.push(("unpublished drawn shocks", v));
    out
}

#[test]
fn no_predicate_reads_another_seats_private_data() {
    let predicates: [(&str, Predicate); 6] = [
        ("p1 upkeep", p1),
        ("p2 spoilage", p2),
        ("p3 scarcity", p3),
        ("p4 coalitions", p4),
        ("p5 season end", p5),
        ("struct", all),
    ];
    for n in [4_u8, 5, 6] {
        let (st, deal) = privacy_state(n);
        // The bid walk runs for seat 0, against bids made by seats 1 and 2.
        let marks = must(season_end_and_marks(&st, Seat(0)), "marks");
        for good in [AssetId::Refined(0), AssetId::Raw(0)] {
            let Some(h) = marks.holdings.iter().find(|h| h.good == good) else {
                panic!("holding")
            };
            assert!(h.units_marked_by_bids > 0, "n={n}: no bid walk on {good:?}");
        }
        // Every other seat, `1..n`.
        for other in 1..n {
            for (vname, variant) in private_variants(&st, &deal, other) {
                assert_ne!(json(&variant), json(&st), "{vname} must change the state");
                for (pname, f) in predicates {
                    assert_eq!(
                        f(&variant, Seat(0)),
                        f(&st, Seat(0)),
                        "n={n}: {pname} changed with seat {other}'s {vname}"
                    );
                }
            }
        }
    }
}

/// The public set (`docs/design-v2.md` §2, "What One Seat May Know About Another"): another seat's
/// free balances and a cartel-domain offer DO change the scarcity predicate's table-wide fields,
/// and those reads leave the private set out: on top of the public change, every private variant
/// still changes nothing.
#[test]
fn public_balances_and_domain_offers_reach_the_table_wide_fields() {
    let field = |st: &EngineState, good: AssetId| {
        let goods = must(scarcity_and_tariffs(st, Seat(0)), "p3");
        let Some(g) = goods.into_iter().find(|g| g.good == good) else {
            panic!("good")
        };
        g
    };
    for n in [4_u8, 5, 6] {
        let (st, deal) = privacy_state(n);
        // Another seat's free balances.
        let mut richer = st.clone();
        richer.seats[2].raw[1] += 3_000;
        richer.seats[3].refined[0] += 400;
        richer.seats[2].cred += 9_999;
        richer.seats[3].cred_debt += 77;
        let before = field(&st, AssetId::Raw(1));
        let after = field(&richer, AssetId::Raw(1));
        assert_eq!(after.table_units_held, before.table_units_held + 3_000);
        assert_eq!(
            field(&richer, AssetId::Refined(0)).table_units_held,
            field(&st, AssetId::Refined(0)).table_units_held + 400
        );
        // Not read by the predicates that use only the asking seat's own state or the rules.
        assert_eq!(p1(&richer, Seat(0)), p1(&st, Seat(0)));
        assert_eq!(p2(&richer, Seat(0)), p2(&st, Seat(0)));
        assert_eq!(p4(&richer, Seat(0)), p4(&st, Seat(0)));

        // A domain-book offer, by a seat outside the asking seat's reach (domain 0 is {1, 2}).
        let mut domain = st.clone();
        domain.offers.insert(
            77,
            CredOffer {
                offer_id: 77,
                maker: 2,
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                remaining_units: 640,
                price_per_unit_cred: 1_200,
                locked_cred: 768,
                domain_id: Some(0),
                is_hybrid: false,
                created_turn: 8,
            },
        );
        let d = field(&domain, AssetId::Raw(1));
        assert_eq!(d.domain_book_units_bid, before.domain_book_units_bid + 640);
        assert_eq!(d.open_book_units_bid, before.open_book_units_bid);

        // On top of the public changes, the private set still changes nothing anywhere.
        for public in [&richer, &domain] {
            for other in 1..n {
                for (vname, variant) in private_variants(public, &deal, other) {
                    assert_eq!(
                        all(&variant, Seat(0)),
                        all(public, Seat(0)),
                        "n={n}: seat {other}'s {vname} changed the struct"
                    );
                }
            }
        }
    }
}

/// Fields of other seats that are public or simply not read: escrows, checks, the Regent doctrine
/// and per-seat counters change nothing. A bid maker's wash freeze and credit tier do change the
/// mark, because the close rule reads them (`scoring.rs` `bid_executable_by_seller`).
#[test]
fn other_seats_escrows_checks_regent_and_counters_change_nothing() {
    let (st, _) = privacy_state(4);
    let base = all(&st, Seat(0));
    let mut v = st.clone();
    v.escrows.insert(
        5,
        TokenEscrowVault {
            escrow_id: 5,
            sender: 1,
            destination: 2,
            amount_cred: 700,
            finish_after_turn: None,
            cancel_after_turn: 12,
            condition_hash: None,
            bond: None,
            status: EscrowStatus::Locked,
        },
    );
    v.checks.insert(
        6,
        CreditCheck {
            check_id: 6,
            drawer: 2,
            payee: 1,
            asset: AssetId::Cred,
            send_max: 400,
            deliver_min: 400,
            due_turn: 10,
            expiration_turn: 12,
            status: CheckStatus::Open,
        },
    );
    for other in [1_usize, 2] {
        v.spec.seats[other].regent = Some(RegentDoctrine {
            risk_tolerance_bps: 9_000,
            min_raw_reserve: 100,
            auto_cash_due_claims: false,
        });
        let seat = &mut v.seats[other];
        seat.identity = SeatIdentity::Human;
        seat.consecutive_misses = 2;
        seat.consecutive_zero_production_turns = 2;
        seat.hard_credit_defaults = 1;
        seat.bond_breaches = 1;
        seat.broken_promises = 3;
        seat.honor_penalties = 900;
    }
    assert_ne!(json(&v), json(&st));
    assert_eq!(all(&v, Seat(0)), base);

    // Read by the close rule: a frozen maker's bid stops counting.
    let mut frozen = st.clone();
    frozen.seats[1].wash_frozen_until_turn = frozen.turn;
    assert_ne!(p5(&frozen, Seat(0)), p5(&st, Seat(0)));
}

fn raw0_mark(st: &EngineState) -> Units {
    let p = must(season_end_and_marks(st, Seat(0)), "marks");
    p.holdings
        .iter()
        .find(|h| h.good == AssetId::Raw(0))
        .map_or(0, |h| h.marked_value)
}

/// The close rule's executability filter for cartel-domain bids (`scoring.rs`
/// `bid_executable_by_seller`): a domain bid seat 0 cannot sell into changes none of its
/// predicates; one it can sell into changes its mark.
#[test]
fn domain_bids_count_in_the_mark_only_when_executable() {
    let (base, _) = busy_state(4);
    // Seat 0 holds 3_500 Raw(0) (1_000 free + 2_500 on its own sell); no open bid on it.
    // Floor 3_500 * 1_750 / 2_000 = 3_062.
    assert_eq!(raw0_mark(&base), 3_062);
    let domain_bid = |is_hybrid: bool| CredOffer {
        offer_id: 99,
        maker: 1,
        side: OfferSide::Buy,
        good: AssetId::Raw(0),
        remaining_units: 500,
        price_per_unit_cred: 1_500,
        locked_cred: 750,
        domain_id: Some(0),
        is_hybrid,
        created_turn: 8,
    };
    let mut with_bid = base.clone();
    with_bid.offers.insert(99, domain_bid(false));

    // Not executable: seat 0 is not in domain 0 ({1, 2}), holds no broker charter, bid not hybrid.
    // The mark and the predicates that do not count offers are unchanged; the scarcity predicate
    // counts the bid in its domain-book total only, since domain books are public by rule
    // (`docs/design-v2.md` §2, "What One Seat May Know About Another").
    for (pname, f) in [("p1", p1 as Predicate), ("p2", p2), ("p4", p4), ("p5", p5)] {
        assert_eq!(f(&with_bid, Seat(0)), f(&base, Seat(0)), "{pname}");
    }
    let raw0 = |st: &EngineState| {
        let goods = must(scarcity_and_tariffs(st, Seat(0)), "p3");
        let Some(g) = goods.into_iter().find(|g| g.good == AssetId::Raw(0)) else {
            panic!("Raw(0)")
        };
        g
    };
    let (b, w) = (raw0(&base), raw0(&with_bid));
    assert_eq!(w.domain_book_units_bid, b.domain_book_units_bid + 500);
    assert_eq!(w.open_book_units_bid, b.open_book_units_bid);

    // Executable, three ways. Mark: 500 at 750 (> 500 * 1_750 / 2_000 = 437) plus 3_000 at the
    // floor (3_000 * 1_750 / 2_000 = 2_625) = 3_375.
    let mut member = with_bid.clone();
    member.domains[0].members.insert(0);
    assert_eq!(raw0_mark(&member), 3_375);

    let mut broker_seller = with_bid.clone();
    broker_seller.charters.push(ActiveCharter {
        role: CharterRole::ClearinghouseBroker,
        holder: 0,
        expires_at_turn: 12,
    });
    assert_eq!(raw0_mark(&broker_seller), 3_375);

    let mut hybrid = base.clone();
    hybrid.offers.insert(99, domain_bid(true));
    hybrid.charters.push(ActiveCharter {
        role: CharterRole::ClearinghouseBroker,
        holder: 1,
        expires_at_turn: 12,
    });
    assert_eq!(raw0_mark(&hybrid), 3_375);
    // A hybrid flag without a broker maker is not executable.
    let mut hybrid_no_broker = base.clone();
    hybrid_no_broker.offers.insert(99, domain_bid(true));
    assert_eq!(raw0_mark(&hybrid_no_broker), 3_062);
    // A member seller whose own tier is D cannot use the domain.
    let mut member_d = member.clone();
    member_d.seats[0].credit_tier = CreditTier::D;
    assert_eq!(raw0_mark(&member_d), 3_062);
}

/// The privacy test is not vacuous: each predicate does move with the asking seat's own state.
#[test]
fn predicates_move_with_the_asking_seats_own_state() {
    let (st, _) = busy_state(4);
    let mut v = st.clone();
    v.seats[0].refined[3] += 900;
    v.seats[0].raw[1] += 4_000;
    v.seats[0].refined[0] += 1_000;
    assert_ne!(p1(&v, Seat(0)), p1(&st, Seat(0)));
    assert_ne!(p2(&v, Seat(0)), p2(&st, Seat(0)));
    assert_ne!(p3(&v, Seat(0)), p3(&st, Seat(0)));
    assert_ne!(p5(&v, Seat(0)), p5(&st, Seat(0)));
    // Predicate 4 reads only the rules: an own-holdings change leaves it alone.
    assert_eq!(p4(&v, Seat(0)), p4(&st, Seat(0)));
}

#[test]
fn every_predicate_refuses_a_seat_outside_the_table() {
    let (st, _) = busy_state(4);
    assert!(upkeep_and_debt(&st, Seat(4)).is_err());
    assert!(spoilage_and_conversion(&st, Seat(4)).is_err());
    assert!(scarcity_and_tariffs(&st, Seat(4)).is_err());
    assert!(coalition_reach(&st, Seat(4), &[0, 4]).is_err());
    assert!(league_core::all_coalitions_reach(&st, Seat(4)).is_err());
    assert!(season_end_and_marks(&st, Seat(4)).is_err());
    assert!(compute_seat_state_predicates(&st, Seat(4)).is_err());
}

// --- Determinism -------------------------------------------------------------------------------

#[test]
fn same_state_gives_the_same_serialized_struct_for_4_5_6() {
    for n in [4_u8, 5, 6] {
        let (st, _) = busy_state(n);
        for s in 0..n {
            let a = all(&st, Seat(s));
            let b = all(&st.clone(), Seat(s));
            assert_eq!(a, b);
            let parsed: SeatStatePredicates = must(serde_json::from_slice(&a), "parse");
            assert_eq!(json(&parsed), a);
            assert_eq!(parsed.version, league_core::STATE_PREDICATES_VERSION);
            assert_eq!(league_core::STATE_PREDICATES_VERSION, 3);
            assert!(!parsed.cancel_locked);
            assert_eq!(parsed.scarcity.len(), 2 * usize::from(n));
        }
    }
    // Public cancel-lock predicate flips when TurnEngine::begin_cancel_lock engages and clears on Commit.
    let (spec_lock, _) = quiet_spec(TableId(99), 4);
    let mut eng_lock = must(TurnEngine::new(spec_lock), "engine lock");
    assert!(!league_core::cancel_locked(eng_lock.state()));
    assert!(
        !must(
            compute_seat_state_predicates(eng_lock.state(), Seat(1)),
            "pred before lock"
        )
        .cancel_locked
    );
    must(eng_lock.begin_cancel_lock(), "begin lock");
    assert!(league_core::cancel_locked(eng_lock.state()));
    assert!(
        must(
            compute_seat_state_predicates(eng_lock.state(), Seat(1)),
            "pred after lock"
        )
        .cancel_locked
    );
    must(eng_lock.advance_to_commit(), "commit clears lock");
    assert!(!league_core::cancel_locked(eng_lock.state()));
    // Rebuilt from scratch: the same table and edits give the same bytes.
    let (a, _) = busy_state(5);
    let (b, _) = busy_state(5);
    assert_eq!(all(&a, Seat(3)), all(&b, Seat(3)));
}

// --- The replay record -------------------------------------------------------------------------

/// The rich season of `replay_invariants.rs::f7_rich_season_golden_digest_is_exact_and_reproducible`,
/// built the same way, so its golden digest pins the replay with the section on.
fn rich_season_log() -> SeasonReplayLog {
    let tid = TableId(944);
    let salts = [
        RevealSalt([0x31; 32]),
        RevealSalt([0x32; 32]),
        RevealSalt([0x33; 32]),
        RevealSalt([0x34; 32]),
    ];
    let eval_seed = must(SealedEvalSeed::try_from_bytes([0x88; 32]), "eval seed");
    let (spec, deal, eval_seed) = must(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            tid, 4, 42, [0x77; 32], &salts, eval_seed,
        ),
        "rich spec",
    );
    let mut eng = must(TurnEngine::new(spec.clone()), "eng");
    let mut turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut actions = Vec::new();
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 2 {
            actions.push(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(0),
                    amount: 2_000,
                    price_per_unit_cred: 1_500,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            actions.push(MarketAction::CreateCheck {
                seat: 1,
                check: CheckProposal {
                    payee: 0,
                    asset: AssetId::Cred,
                    send_max: 600,
                    deliver_min: 600,
                    due_turn: 3,
                    expiration_turn: 5,
                },
            });
            actions.push(MarketAction::CancelOffer {
                seat: 0,
                offer_id: 99_999,
            });
            let sw = AtomicSwapProposal {
                swap_id: 700,
                maker: 1,
                taker: 2,
                maker_asset: AssetId::Raw(1),
                maker_amount: 1_000,
                taker_asset: AssetId::Cred,
                taker_amount: 1_200,
                domain_id: None,
            };
            bundles[1].atomic_swaps.push(sw.clone());
            bundles[2].atomic_swaps.push(sw);
        }
        if t == 3 {
            actions.push(MarketAction::StandForOffice {
                seat: 2,
                platform: CouncilPlatform {
                    proposed_tariffs_bps: vec![(0, (1_100, 800))],
                    cartel_admissions: vec![(1, 2), (1, 0)],
                    cartel_revocations: Vec::new(),
                },
            });
            bundles[0].check_cashes.push(1);
        }
        if t == 4 {
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                cartel_chair: 2,
            });
        }
        if t == 6 {
            bundles[1].charter_bids.push(CharterBidOrder {
                role: CharterRole::ClearinghouseBroker,
                bid_cred: 400,
            });
        }
        for action in &actions {
            let _ = eng.apply_market_action(action.clone());
        }
        let (commitments, reveals) = commit_and_reveal_rich(&mut eng, tid, t, bundles);
        let (beacon, prf) = must(eval_seed.prove_turn(tid, Turn(t)), "prove");
        let receipt = must(eng.sweep_turn_with_eval_vrf(&prf), "sweep");
        turns.push(LoggedTurn {
            turn: t,
            market_actions: actions,
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(prf.to_vec()),
            drawn_shocks: receipt.newly_drawn_shocks,
        });
        if receipt.season_closed {
            break;
        }
    }
    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[0] = Some(deal.seat_reveals[0].clone());
    SeasonReplayLog {
        eval_vrf_pubkey: spec.eval_vrf_pubkey,
        spec,
        mode: TableMode::Eval,
        seed: must(ReplayLayoutSeed::try_clear(42), "clear seed 42"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        sealed_mandate_deal: deal,
        mandate_commits: vec![None; 4],
        turns,
        mandate_reveals,
    }
}

/// The salt rule of `replay_invariants.rs::commit_and_reveal_all`, which the golden digest binds.
fn commit_and_reveal_rich(
    engine: &mut TurnEngine,
    tid: TableId,
    turn: u8,
    bundles: Vec<SealedOrderBundle>,
) -> (BTreeMap<u8, CommitmentHash>, BTreeMap<u8, OrderReveal>) {
    must(engine.advance_to_commit(), "advance_to_commit");
    let mut commits = BTreeMap::new();
    let mut reveals = BTreeMap::new();
    for (idx, bundle) in bundles.into_iter().enumerate() {
        let s = idx as u8;
        let rev = OrderReveal {
            bundle,
            salt: RevealSalt([s.wrapping_mul(17).wrapping_add(turn); 32]),
        };
        let ch = must(
            compute_order_commitment(tid, Turn(turn), Seat(s), &rev),
            "commit",
        );
        must(engine.submit_order_commitment(Seat(s), ch), "submit");
        commits.insert(s, ch);
        reveals.insert(s, rev);
    }
    must(engine.advance_to_reveal(), "advance_to_reveal");
    for (&s, rev) in &reveals {
        must(engine.submit_order_reveal(Seat(s), rev.clone()), "reveal");
    }
    (commits, reveals)
}

#[test]
fn replay_section_off_is_byte_identical_and_on_leaves_the_engine_unchanged() {
    let log = rich_season_log();
    let plain = must(replay_season(&log), "plain replay");
    let (with, section) = must(replay_season_with_state_predicates(&log), "with section");
    // On: the engine computes exactly what it computes without the section.
    assert_eq!(with, plain);
    assert_eq!(
        hex::encode(with.state_digests[with.state_digests.len() - 1]),
        "7eaf7769631a1fff3194a2ae396725bac10b3e1272b03a46655b4daae189a3e3"
    );
    assert_eq!(section.turns.len(), log.turns.len());
    assert_eq!(section.at_close.len(), 4);

    // Off: the record is the bare log, byte for byte.
    let bare = json(&log);
    let off = ReplayRecord {
        log: log.clone(),
        state_predicates: None,
    };
    assert_eq!(json(&off), bare);
    let parsed_off = must(ReplayRecord::from_json_slice(&bare), "bare as record");
    assert_eq!(parsed_off, off);

    // On: only the added key differs, and a reader of the bare log still reads it.
    let on = must(attach_state_predicates(&log), "attach");
    assert_eq!(on.state_predicates.as_ref(), Some(&section));
    let on_bytes = json(&on);
    assert_ne!(on_bytes, bare);
    assert_eq!(
        &on_bytes[..bare.len() - 1],
        &bare[..bare.len() - 1],
        "the section is appended after the log's own keys"
    );
    assert!(on_bytes[bare.len() - 1..].starts_with(b",\"state_predicates\":"));
    let old_reader: SeasonReplayLog = must(serde_json::from_slice(&on_bytes), "old reader");
    assert_eq!(old_reader, log);
    let new_reader = must(ReplayRecord::from_json_slice(&on_bytes), "new reader");
    assert_eq!(new_reader, on);
    assert_eq!(must(replay_season(&old_reader), "replay again"), plain);

    // Unreadable input is refused, for the log and for the section.
    assert!(ReplayRecord::from_json_slice(b"not json").is_err());
    let mut bad_section = bare[..bare.len() - 1].to_vec();
    bad_section.extend_from_slice(b",\"state_predicates\":{\"version\":\"one\"}}");
    assert!(ReplayRecord::from_json_slice(&bad_section).is_err());
}

// --- Metrics -----------------------------------------------------------------------------------

#[test]
fn metrics_from_a_quiet_season() {
    let (log, _) = quiet_season_log(5);
    let (outcome, section) = must(replay_season_with_state_predicates(&log), "replay");
    let metrics = compute_season_metrics(&section);
    assert_eq!(metrics.len(), 5);
    let total_spoiled: Units = outcome
        .receipts
        .iter()
        .map(|r| r.total_spoilage_clawed_back)
        .sum();
    assert_eq!(
        metrics.iter().map(|m| m.spoiled_units).sum::<Units>(),
        total_spoiled
    );
    for m in &metrics {
        // Nobody held upkeep goods or offered them: neither count fires.
        assert!(m.held_enough_but_short_turns.is_empty());
        assert!(m.covered_with_own_offers_but_short_turns.is_empty());
        assert_eq!(m.spoiled_units, spoiled_units_over_season(&section, m.seat));
        // No bids at the close: every held unit is marked at the floor.
        let Some(close) = section.at_close.iter().find(|p| p.seat == m.seat) else {
            panic!("close record")
        };
        let units: Units = close.season_end.holdings.iter().map(|h| h.units).sum();
        let marks: Units = close
            .season_end
            .holdings
            .iter()
            .map(|h| h.marked_value)
            .sum();
        assert!(units > 0);
        assert_eq!(m.stranded_units_at_close, units);
        assert_eq!(m.stranded_value_at_close, marks);
        assert_eq!(stranded_stock_at_close(&section, m.seat), (units, marks));
    }
}

#[test]
fn held_enough_but_short_counts_held_or_convertible_cover_only() {
    let (log, _) = quiet_season_log(4);
    let (_, base) = must(replay_season_with_state_predicates(&log), "replay");
    let mut section: StatePredicateSection = base.clone();
    // Turn 3: seat 0 had a shortfall at the Sweep. Pretend it held its full upkeep at the start.
    let rec = &mut section.turns[2];
    assert!(rec.sweep[0].upkeep_shortfall_units > 0);
    rec.at_turn_start[0].upkeep.held_units = rec.at_turn_start[0].upkeep.owed_units;
    // Turn 5: seat 1 held 1_000 of 1_800 and a recipe making its upkeep good covers 1 batch of
    // 3_500 (runs left 4): 1_000 + 3_500 >= 1_800 -> could have converted.
    let rec = &mut section.turns[4];
    assert!(rec.sweep[1].upkeep_shortfall_units > 0);
    let p = &mut rec.at_turn_start[1];
    p.upkeep.held_units = 1_000;
    p.spoilage.recipes[0].output_good = p.upkeep.upkeep_good;
    p.spoilage.recipes[0].batches_covered_by_inputs = 1;
    // Turn 6: the same recipe with no runs left does not count.
    let rec = &mut section.turns[5];
    let p = &mut rec.at_turn_start[1];
    p.upkeep.held_units = 1_000;
    p.spoilage.recipes[0].output_good = p.upkeep.upkeep_good;
    p.spoilage.recipes[0].batches_covered_by_inputs = 1;
    p.spoilage.recipes[0].capacity_runs_left = 0;
    // Turn 7: covered at the start but no shortfall at the Sweep does not count either.
    let rec = &mut section.turns[6];
    rec.at_turn_start[2].upkeep.held_units = 5_000;
    rec.sweep[2].upkeep_shortfall_units = 0;

    assert_eq!(held_enough_but_short_turns(&section, 0), vec![3]);
    assert_eq!(held_enough_but_short_turns(&section, 1), vec![5]);
    assert!(held_enough_but_short_turns(&section, 2).is_empty());
    assert!(held_enough_but_short_turns(&base, 0).is_empty());
    // Free units that cover also count for the offers metric; the recipe-only turn 5 does not.
    assert_eq!(
        covered_with_own_offers_but_short_turns(&section, 0),
        vec![3]
    );
    assert!(covered_with_own_offers_but_short_turns(&section, 1).is_empty());
    assert_eq!(stranded_stock_at_close(&section, 9), (0, 0));
}

// --- Fix 1 additions -----------------------------------------------------------------------------

/// Predicate 2's batches against a Sweep that runs a converter order: the Sweep runs what the held
/// inputs cover when the cap does not bind, and the cap when it does.
#[test]
fn predicted_converter_cover_equals_the_sweep_runs() {
    // (raw held of Raw(1) and Raw(2), runs ordered, expected runs). Recipe takes 2_000 of each.
    for (held, ordered, expected) in [(5_000, 4, 2_u32), (20_000, 9, 4_u32)] {
        let (spec, _) = quiet_spec(TableId(21), 4);
        let engine = must(TurnEngine::new(spec), "engine");
        let mut st = engine.state().clone();
        st.turn = 2;
        st.seats[0].raw[1] = held;
        st.seats[0].raw[2] = held;
        let p = must(spoilage_and_conversion(&st, Seat(0)), "p2");
        let r = &p.recipes[0];
        let predicted = r.batches_covered_by_inputs.min(r.capacity_runs_left);
        assert_eq!(predicted, expected);
        let mut eng = engine_from_state(&st);
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        bundles[0].converter_runs.push(ConverterRunOrder {
            recipe_idx: 0,
            runs: ordered,
        });
        let _ = commit_and_reveal(&mut eng, st.table_id, 2, bundles);
        let receipt = must(
            eng.sweep_turn(derive_turn_beacon(3, st.table_id, Turn(2))),
            "sweep",
        );
        assert_eq!(receipt.converter_batches_run, expected as usize);
        assert_eq!(
            eng.state().seats[0].refined[0] - st.seats[0].refined[0],
            3_500 * Units::from(expected)
        );
    }
}

/// The review's case: seat 0 owes 1_800 Refined(3), holds 0 free and 2_000 on its own resting sell,
/// leaves the offer up and is short by 1_800 at the Sweep.
#[test]
fn own_offer_cover_counts_a_shortfall_free_holdings_miss() {
    let (spec, _) = quiet_spec(TableId(22), 4);
    let engine = must(TurnEngine::new(spec), "engine");
    let mut st = engine.state().clone();
    st.turn = 3;
    st.seats[0].refined[3] = 0;
    st.offers.insert(
        1,
        CredOffer {
            offer_id: 1,
            maker: 0,
            side: OfferSide::Sell,
            good: AssetId::Refined(3),
            remaining_units: 2_000,
            price_per_unit_cred: 9_000,
            locked_cred: 0,
            domain_id: None,
            is_hybrid: false,
            created_turn: 3,
        },
    );
    let at_turn_start: Vec<SeatStatePredicates> = (0..4)
        .map(|s| must(compute_seat_state_predicates(&st, Seat(s)), "predicates"))
        .collect();
    assert_eq!(at_turn_start[0].upkeep.held_units, 0);
    assert_eq!(at_turn_start[0].upkeep.held_on_own_resting_sells, 2_000);
    let mut eng = engine_from_state(&st);
    let _ = commit_and_reveal(
        &mut eng,
        st.table_id,
        3,
        vec![SealedOrderBundle::default(); 4],
    );
    let _ = must(
        eng.sweep_turn(derive_turn_beacon(4, st.table_id, Turn(3))),
        "sweep",
    );
    let after = eng.state();
    let sweep: Vec<league_core::SeatSweepOutcome> = (0..4)
        .map(|s| {
            let p = &at_turn_start[s];
            let g = usize::from(p.upkeep.upkeep_good);
            let paid = st.seats[s].refined[g] - after.seats[s].refined[g];
            league_core::SeatSweepOutcome {
                seat: s as u8,
                upkeep_shortfall_units: p.upkeep.owed_units - paid,
                shortfall_debt_added: 0,
                spoiled_units: 0,
            }
        })
        .collect();
    assert_eq!(sweep[0].upkeep_shortfall_units, 1_800);
    let section = StatePredicateSection {
        version: league_core::STATE_PREDICATE_SECTION_VERSION,
        table_id: st.table_id,
        turns: vec![league_core::TurnPredicateRecord {
            turn: 3,
            at_turn_start,
            sweep,
        }],
        at_close: Vec::new(),
    };
    assert!(held_enough_but_short_turns(&section, 0).is_empty());
    assert_eq!(
        covered_with_own_offers_but_short_turns(&section, 0),
        vec![3]
    );
    // Seat 1 held nothing and offered nothing: neither count.
    assert!(covered_with_own_offers_but_short_turns(&section, 1).is_empty());
}

/// Malformed states (the review's two inputs) give results, not panics.
#[test]
fn predicates_do_not_panic_on_truncated_state() {
    let (spec, _) = quiet_spec(TableId(23), 4);
    let engine = must(TurnEngine::new(spec), "engine");
    let mut st = engine.state().clone();
    st.parities.refined_per_unit = Vec::new();
    st.seats[0].refined = vec![2_000; 4];
    let p = must(upkeep_and_debt(&st, Seat(0)), "covered, no parity");
    assert_eq!((p.shortfall_units, p.debt_added_at_sweep), (0, 0));

    let mut st = engine.state().clone();
    st.seats[0].raw = vec![1_000];
    let p = must(spoilage_and_conversion(&st, Seat(0)), "truncated raw");
    assert_eq!(p.recipes[0].batches_covered_by_inputs, 0);
}

#[test]
fn replay_record_refuses_a_section_for_another_table() {
    let (log, _) = quiet_season_log(4);
    let mut record = must(attach_state_predicates(&log), "attach");
    let good = json(&record);
    assert_eq!(
        must(ReplayRecord::from_json_slice(&good), "same table"),
        record
    );
    if let Some(section) = record.state_predicates.as_mut() {
        section.table_id = TableId(log.spec.table_id.0 + 1);
    }
    let bad = json(&record);
    let Err(err) = ReplayRecord::from_json_slice(&bad) else {
        panic!("a section for another table must be refused")
    };
    assert!(err.to_string().contains("table_id"));
    // A section version this code does not read is refused, by name.
    if let Some(section) = record.state_predicates.as_mut() {
        section.table_id = log.spec.table_id;
        section.version = league_core::STATE_PREDICATE_SECTION_VERSION + 1;
    }
    let Err(err) = ReplayRecord::from_json_slice(&json(&record)) else {
        panic!("an unknown section version must be refused")
    };
    assert!(err.to_string().contains("version"));
}
