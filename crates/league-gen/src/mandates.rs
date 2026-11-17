//! Single canonical Turn-1 mandate deck shared between [`league_core`] and generated
//! [`crate::RingTable`] fixtures (`B-2`, #32 review).
//!
//! `league-gen` delegates mandate card types, deck generation, and CSPRNG deal permutations
//! directly to [`league_core`] so there is never a second diverging deck or shuffle implementation.
//! Note: conversion of provisional mandate points into terminal scoring lives separately in
//! [`league_core::convert_mandate_points_to_equity`].

use crate::schema::RingTable;
pub use league_core::{
    CommitmentHash, CreditTier, EngineError, GradedMandateCard, MandateKind, RevealSalt,
    SealedMandateDeal, TableId, UNIT_SCALE, Units, bonus_points_milli_for_completion_bps,
    deal_graded_mandates, deal_graded_mandates_with_entropy, deal_mandate_deck_indices,
    default_graded_mandate_deck, graded_mandate_deck_for_mean_project_value,
};

/// Returns the mean Tier-3 Syndicate Project point value in milli-units across `table`'s seats
/// (`round(mean(refined_project_value) * 1_000)`).
#[must_use]
pub fn mean_project_value_milli(table: &RingTable) -> Units {
    if table.seats.is_empty() {
        return 0;
    }
    let sum: f64 = table.seats.iter().map(|s| s.refined_project_value).sum();
    let mean = sum / (table.seats.len() as f64);
    ((mean * (UNIT_SCALE as f64)).round() as Units).max(1)
}

/// Builds the single canonical graded Turn-1 mandate deck for `table` directly from
/// [`league_core::graded_mandate_deck_for_mean_project_value`].
#[must_use]
pub fn mandate_deck_for_ring_table(table: &RingTable) -> Vec<GradedMandateCard> {
    let mean_milli = mean_project_value_milli(table);
    graded_mandate_deck_for_mean_project_value(table.seat_count.as_u8(), mean_milli)
}

/// Deals sealed Turn-1 mandates from `mandate_deck_for_ring_table(table)` via [`league_core`]'s
/// canonical CSPRNG-separated dealing routine.
pub fn deal_mandates_for_ring_table(
    table: &RingTable,
    table_id: TableId,
    deal_seed: [u8; 32],
    seat_salts: &[RevealSalt],
) -> Result<(Vec<CommitmentHash>, SealedMandateDeal), EngineError> {
    let deck = mandate_deck_for_ring_table(table);
    deal_graded_mandates_with_entropy(
        table_id,
        table.seat_count.as_u8(),
        deal_seed,
        seat_salts,
        &deck,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_ring_table;
    use crate::schema::{GeneratorConfig, SeatCount};
    use league_core::{MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY};

    #[test]
    fn b2_single_mandate_deck_matches_league_core_card_for_card_and_deal() {
        let cfg = GeneratorConfig {
            seat_count: Some(SeatCount::Four),
            ..GeneratorConfig::default()
        };
        let (table, _) = match generate_ring_table(20261115, &cfg) {
            Ok(pair) => pair,
            Err(err) => panic!("ring table failed: {err}"),
        };

        let gen_deck = mandate_deck_for_ring_table(&table);
        let expected_mean = mean_project_value_milli(&table);
        assert!(
            expected_mean > 0,
            "ring table must expose positive project value"
        );
        let core_deck = graded_mandate_deck_for_mean_project_value(4, expected_mean);
        assert!(!gen_deck.is_empty(), "league-gen deck must not be empty");
        assert_eq!(
            gen_deck, core_deck,
            "league-gen must reuse league-core's single deck"
        );
        let embedded_core_cards: Vec<GradedMandateCard> = table
            .mandate_deck
            .iter()
            .map(|c| c.to_core_card())
            .collect();
        assert_eq!(
            embedded_core_cards, core_deck,
            "RingTable.mandate_deck must match league-core's single deck card for card"
        );
        for tid_num in [42_u64, 99_u64] {
            let tid = TableId(tid_num);
            let schema_deal = match crate::schema::deal_mandate_indices_with_entropy(
                tid,
                &table.mandate_deck,
                SeatCount::Four,
                [0xA5; 32],
            ) {
                Ok(idxs) => idxs,
                Err(err) => panic!("schema deal failed for {tid:?}: {err}"),
            };
            let core_deal = match deal_mandate_deck_indices(tid, 4, [0xA5; 32], core_deck.len()) {
                Ok(idxs) => idxs,
                Err(err) => panic!("core deal failed for {tid:?}: {err}"),
            };
            assert_eq!(
                schema_deal, core_deal,
                "deal_mandate_indices_with_entropy must match league-core's deal for {tid:?}"
            );
            let salts = [
                RevealSalt([0x11; 32]),
                RevealSalt([0x22; 32]),
                RevealSalt([0x33; 32]),
                RevealSalt([0x44; 32]),
            ];
            let (_, sealed) = match deal_mandates_for_ring_table(&table, tid, [0xA5; 32], &salts) {
                Ok(pair) => pair,
                Err(err) => panic!("full deal failed for {tid:?}: {err}"),
            };
            let dealt_ids: Vec<u16> = sealed
                .seat_reveals
                .iter()
                .map(|r| {
                    core_deck
                        .iter()
                        .find(|c| {
                            c.kind == r.mandate.kind && c.bonus_equity == r.mandate.bonus_equity
                        })
                        .map_or(0, |c| c.card_id)
                })
                .collect();
            let indexed_ids: Vec<u16> = schema_deal
                .iter()
                .map(|&idx| table.mandate_deck[idx].card_id)
                .collect();
            assert_eq!(
                indexed_ids, dealt_ids,
                "schema deal indices for {tid:?} must select exact cards dealt by league-core"
            );
        }

        let std_core = default_graded_mandate_deck(4);
        assert_eq!(std_core.len(), gen_deck.len());
        for card in &gen_deck {
            assert!(
                (MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY)
                    .contains(&card.mandate_points_milli())
            );
            assert!(
                !matches!(
                    card.kind,
                    MandateKind::MaintainCleanCredit {
                        min_tier: CreditTier::B | CreditTier::D
                    }
                ),
                "B-2 single deck must never contain B or D credit tiers"
            );
            assert!(card.kind.validate_for_seat_count(4).is_ok());
        }

        // Verify deal permutation uses league-core's single shuffle direction & rejects zero deal_seed.
        assert!(deal_mandate_deck_indices(TableId(44), 4, [0u8; 32], gen_deck.len()).is_err());
        let expected_idxs =
            match deal_mandate_deck_indices(TableId(44), 4, [0xA5; 32], gen_deck.len()) {
                Ok(idxs) => idxs,
                Err(err) => panic!("deal_mandate_deck_indices failed: {err}"),
            };
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31 + i; 32])).collect();
        let (commits, deal) =
            match deal_mandates_for_ring_table(&table, TableId(44), [0xA5; 32], &salts) {
                Ok(res) => res,
                Err(err) => panic!("deal_mandates_for_ring_table failed: {err}"),
            };
        assert_eq!(commits.len(), 4);
        assert_eq!(deal.seat_reveals.len(), 4);
        for (seat_idx, rev) in deal.seat_reveals.iter().enumerate() {
            let expected_card = &core_deck[expected_idxs[seat_idx]];
            assert_eq!(rev.mandate.kind, expected_card.kind);
            assert_eq!(rev.mandate.bonus_equity, expected_card.bonus_equity);
        }
    }

    fn ring_table_to_core_spec(
        table: &RingTable,
        initial_cred_milli: Units,
    ) -> league_core::TableSpec {
        use league_core::{
            CLOSING_ROLL_WINDOW_LEN, ConverterRecipe, DEFAULT_CANCEL_LOCK_BPS,
            DEFAULT_MANDATE_ALPHA_BPS, DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
            DEFAULT_WASH_FREEZE_BOND_CRED, DEFAULT_WASH_FREEZE_BOUNTY_CRED,
            DEFAULT_WASH_ROUND_TRIP_MIN_UNITS, DEFAULT_WORKS_STIPEND_PER_UNIT_CRED, ProjectRecipe,
            ScheduledShockSpec, SeatConfig, SeatIdentity, ShockKind, TableMode, TableSpec,
        };

        let n = table.seat_count.as_u8();
        let tid = TableId(800 + u64::from(n));
        let salts: Vec<RevealSalt> = (0..n).map(|s| RevealSalt([0x41 + s; 32])).collect();
        let (dealt_mandate_commits, _) =
            match deal_mandates_for_ring_table(table, tid, [0x77; 32], &salts) {
                Ok(p) => p,
                Err(e) => panic!("deal for residual probe failed: {e}"),
            };
        let seats: Vec<SeatConfig> = table
            .seats
            .iter()
            .map(|s| SeatConfig {
                seat: s.seat,
                identity: SeatIdentity::AiSyndicate,
                initial_cred: initial_cred_milli,
                raw_yield_per_turn: ((s.raw_yield_per_turn * 1_000.0).round() as Units).max(1),
                refined_project_value: ((s.refined_project_value * 1_000.0).round() as Units)
                    .max(1),
                upkeep_good: s.upkeep_good,
                upkeep_amount: ((s.upkeep_amount * 1_000.0).round() as Units).max(1),
                initial_raw_tariff_bps: (s.raw_export_tariff * 10_000.0).round() as u16,
                initial_refined_tariff_bps: (s.refined_export_tariff * 10_000.0).round() as u16,
                perishable_decay_bps: ((s.perishable_decay_rate * 10_000.0).round() as u16)
                    .min(9_999),
                regent: None,
                converters: s
                    .converters
                    .iter()
                    .map(|c| ConverterRecipe {
                        owner: c.owner,
                        inputs: c
                            .inputs
                            .iter()
                            .map(|&(g, amt)| (g, ((amt * 1_000.0).round() as Units).max(1)))
                            .collect(),
                        output_good: c.output_good,
                        output_amount: ((c.output_amount * 1_000.0).round() as Units).max(1),
                        max_runs_per_turn: (c.max_runs_per_turn.ceil() as u32).max(1),
                        is_secondary: c.is_secondary,
                    })
                    .collect(),
            })
            .collect();
        let projects: Vec<ProjectRecipe> = table
            .projects
            .iter()
            .map(|p| ProjectRecipe {
                id: u16::from(p.id),
                inputs: p
                    .required_inputs
                    .iter()
                    .map(|&(g, _, amt)| (g, ((amt * 1_000.0).round() as Units).max(1)))
                    .collect(),
                equity_points: ((p.output_project_equity * 1_000.0).round() as Units).max(1),
            })
            .collect();

        TableSpec {
            table_id: tid,
            mode: TableMode::Eval,
            seat_count: n,
            seats,
            projects,
            shocks: vec![ScheduledShockSpec {
                hint_turn: 1,
                forecast_turn: 2,
                realize_turn: 3,
                target_seat: n - 1,
                kind: ShockKind::TariffHoliday,
                magnitude_bps: 100,
            }],
            closing_hazard_bps: [10_000; CLOSING_ROLL_WINDOW_LEN],
            mandate_deck: mandate_deck_for_ring_table(table),
            dealt_mandate_commits,
            drand_schedule: None,
            eval_vrf_pubkey: None,
            town_import_premium_bps: DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
            works_stipend_per_unit_cred: DEFAULT_WORKS_STIPEND_PER_UNIT_CRED,
            mandate_alpha_bps: DEFAULT_MANDATE_ALPHA_BPS,
            cancel_lock_bps: DEFAULT_CANCEL_LOCK_BPS,
            wash_round_trip_min_units: DEFAULT_WASH_ROUND_TRIP_MIN_UNITS,
            wash_freeze_bond_cred: DEFAULT_WASH_FREEZE_BOND_CRED,
            wash_freeze_bounty_cred: DEFAULT_WASH_FREEZE_BOUNTY_CRED,
        }
    }

    #[test]
    fn pr40_fix2_generated_table_wash_immunity_and_bid_bounds() {
        use league_core::{
            AssetId, BPS_SCALE, MarketAction, OfferProposal, OfferSide, OrderReveal,
            SealedOrderBundle, Seat, Turn, TurnEngine, compute_order_commitment,
            derive_turn_beacon, mark_good_holding_at_close,
        };

        let initial_cred = 15_000_i64; // canonical 15.000 CRED starting capital (`types.rs:906`)
        println!(
            "\n=== PR40 FIX2 GENERATED-TABLE CLOSE-MARK CHECKS (initial_cred=15.000 CRED) ==="
        );

        for sc in [SeatCount::Four, SeatCount::Five, SeatCount::Six] {
            let n = sc.as_u8();
            let cfg = GeneratorConfig {
                seat_count: Some(sc),
                ..GeneratorConfig::default()
            };
            let (table, _) = match generate_ring_table(42, &cfg) {
                Ok(p) => p,
                Err(e) => panic!("generate seed 42 N={n} failed: {e}"),
            };
            let base_spec = ring_table_to_core_spec(&table, initial_cred);
            let tid = base_spec.table_id;

            let step_turn = |eng: &mut TurnEngine, t: u8| {
                if eng.advance_to_commit().is_err() {
                    panic!("advance_to_commit");
                }
                let mut revs = Vec::new();
                for s in 0..n {
                    let rev = OrderReveal {
                        bundle: SealedOrderBundle::default(),
                        salt: RevealSalt([t.wrapping_add(s).max(1); 32]),
                    };
                    let Ok(c) = compute_order_commitment(tid, Turn(t), Seat(s), &rev) else {
                        panic!("commit hash");
                    };
                    if eng.submit_order_commitment(Seat(s), c).is_err() {
                        panic!("submit commit");
                    }
                    revs.push((s, rev));
                }
                if eng.advance_to_reveal().is_err() {
                    panic!("advance_to_reveal");
                }
                for (s, rev) in revs {
                    if eng.submit_order_reveal(Seat(s), rev).is_err() {
                        panic!("submit reveal");
                    }
                }
                match eng.sweep_turn(derive_turn_beacon(42, tid, Turn(t))) {
                    Ok(r) => r,
                    Err(e) => panic!("sweep failed: {e}"),
                }
            };

            // 1. Baseline (no trades in Raw(0) on T=22..24, no resting bids).
            let mut eng_base = match TurnEngine::new(base_spec.clone()) {
                Ok(e) => e,
                Err(e) => panic!("eng_base: {e}"),
            };
            let parity_raw0 = eng_base.state().parities.asset_parity(AssetId::Raw(0));
            for t in 1..=24_u8 {
                let r = step_turn(&mut eng_base, t);
                if r.season_closed {
                    break;
                }
            }
            let s0_hold_base = eng_base.state().seats[0].raw[0];
            let s0_mark_base =
                mark_good_holding_at_close(eng_base.state(), 0, AssetId::Raw(0), s0_hold_base);
            let board_base = match eng_base.finalize_season() {
                Ok(b) => b,
                Err(e) => panic!("finalize base: {e}"),
            };
            let s0_eq_base = board_base
                .standings
                .iter()
                .find(|x| x.seat == 0)
                .map_or(0, |x| x.net_asset_equity);

            // 2. Wash-trade residual (Seats 2 & 3 ping-pong >= 2% of start-of-turn Raw(0) stock across T=22,23,24).
            let run_wash = |pump_upward: bool| {
                let mut eng = match TurnEngine::new(base_spec.clone()) {
                    Ok(e) => e,
                    Err(e) => panic!("eng wash: {e}"),
                };
                let mut q_turns = Vec::new();
                let mut imported_total = 0_i64;
                let mut import_cred_total = 0_i64;
                let mut chosen_px = 1_i64;

                for t in 1..=24_u8 {
                    if (22..=24).contains(&t) {
                        let (seller, buyer) = if t % 2 == 0 {
                            (2_u8, 3_u8)
                        } else {
                            (3_u8, 2_u8)
                        };
                        let stock_start: Units = eng
                            .state()
                            .seats
                            .iter()
                            .map(|s| s.raw[0].max(0))
                            .sum::<Units>()
                            + eng
                                .state()
                                .offers
                                .values()
                                .filter(|o| {
                                    o.side == OfferSide::Sell
                                        && o.good == AssetId::Raw(0)
                                        && o.remaining_units > 0
                                })
                                .map(|o| o.remaining_units)
                                .sum::<Units>();
                        let min_q = ((i128::from(stock_start) * 200 + (BPS_SCALE - 1)) / BPS_SCALE)
                            as Units;
                        let q = min_q.max(1);
                        let have = eng.state().seats[usize::from(seller)].raw[0];
                        if have < q {
                            let need = q - have;
                            let cred_before = eng.state().seats[usize::from(seller)].cred;
                            if eng
                                .apply_market_action(MarketAction::TownImport {
                                    seat: seller,
                                    good: AssetId::Raw(0),
                                    amount: need,
                                })
                                .is_err()
                            {
                                panic!("wash import failed");
                            }
                            let cred_after = eng.state().seats[usize::from(seller)].cred;
                            imported_total += need;
                            import_cred_total += cred_before - cred_after;
                        }
                        if t == 22 {
                            if pump_upward {
                                // Leave 2_500 milli-CRED buffer on Seats 2 & 3 so T23/T24 TownImport top-ups succeed,
                                // and size price so buyer's CRED covers ceil(q_max * px / 1_000).
                                let buyer_cred = eng.state().seats[usize::from(buyer)].cred;
                                let usable_cred = (buyer_cred - 2_500).max(1_000);
                                let q_safe = q + 200; // allow slight stock drift across T22..24
                                chosen_px = ((i128::from(usable_cred) * 1_000) / i128::from(q_safe))
                                    as Units;
                            } else {
                                chosen_px = 1; // 1 milli-CRED/unit crash
                            }
                        }
                        if eng
                            .apply_market_action(MarketAction::PostOffer {
                                seat: buyer,
                                order: OfferProposal {
                                    side: OfferSide::Buy,
                                    good: AssetId::Raw(0),
                                    amount: q,
                                    price_per_unit_cred: chosen_px,
                                    domain_id: None,
                                    is_hybrid: false,
                                },
                            })
                            .is_err()
                        {
                            panic!("wash buy failed");
                        }
                        if eng
                            .apply_market_action(MarketAction::PostOffer {
                                seat: seller,
                                order: OfferProposal {
                                    side: OfferSide::Sell,
                                    good: AssetId::Raw(0),
                                    amount: q,
                                    price_per_unit_cred: chosen_px,
                                    domain_id: None,
                                    is_hybrid: false,
                                },
                            })
                            .is_err()
                        {
                            panic!("wash sell failed");
                        }
                        q_turns.push((stock_start, q));
                    }
                    let r = step_turn(&mut eng, t);
                    if r.season_closed {
                        break;
                    }
                }
                let s0_hold = eng.state().seats[0].raw[0];
                let s0_mk = mark_good_holding_at_close(eng.state(), 0, AssetId::Raw(0), s0_hold);
                let b = match eng.finalize_season() {
                    Ok(b) => b,
                    Err(e) => panic!("finalize wash: {e}"),
                };
                let s0_eq = b
                    .standings
                    .iter()
                    .find(|x| x.seat == 0)
                    .map_or(0, |x| x.net_asset_equity);
                (
                    q_turns,
                    imported_total,
                    import_cred_total,
                    chosen_px,
                    s0_mk,
                    s0_eq,
                )
            };

            let (q_up, imp_up, cost_up, px_up, mk_up, eq_up) = run_wash(true);
            let (_q_dn, imp_dn, cost_dn, px_dn, mk_dn, eq_dn) = run_wash(false);
            let gross_vol: Units = q_up.iter().map(|&(_, q)| q).sum();

            // 3. High-bid residual (friendly Seat 1 locks all its 15.000 CRED into a 1 milli-unit resting bid on Turn 24;
            // also measure rival Seat 2 sniping that 1 milli-unit bid in Phase 1).
            let mut eng_bid = match TurnEngine::new(base_spec.clone()) {
                Ok(e) => e,
                Err(e) => panic!("eng_bid: {e}"),
            };
            for t in 1..=23_u8 {
                let _ = step_turn(&mut eng_bid, t);
            }
            let s1_cred_t24 = eng_bid.state().seats[1].cred;
            let bid_px = s1_cred_t24 * 1_000; // 1 milli-unit * (cred * 1_000) / 1_000 == s1_cred_t24 locked!
            let mut eng_bid_uncrossed = eng_bid.clone();
            if eng_bid_uncrossed
                .apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 1,
                        price_per_unit_cred: bid_px,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .is_err()
            {
                panic!("friendly high bid failed");
            }
            let _ = step_turn(&mut eng_bid_uncrossed, 24);
            let board_bid = match eng_bid_uncrossed.finalize_season() {
                Ok(b) => b,
                Err(e) => panic!("fin bid: {e}"),
            };
            let s0_eq_bid = board_bid
                .standings
                .iter()
                .find(|x| x.seat == 0)
                .map_or(0, |x| x.net_asset_equity);
            let s1_eq_uncrossed = board_bid
                .standings
                .iter()
                .find(|x| x.seat == 1)
                .map_or(0, |x| x.net_asset_equity);

            // Sniped high-bid: rival Seat 2 imports 1 milli-unit of Raw(0) for 1 milli-CRED and crosses at bid_px!
            let mut eng_bid_sniped = eng_bid;
            let s1_eq_before_bid = board_base
                .standings
                .iter()
                .find(|x| x.seat == 1)
                .map_or(0, |x| x.net_asset_equity);
            let _ = eng_bid_sniped.apply_market_action(MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 1,
                    price_per_unit_cred: bid_px,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            let _ = eng_bid_sniped.apply_market_action(MarketAction::TownImport {
                seat: 2,
                good: AssetId::Raw(0),
                amount: 1,
            });
            let _ = eng_bid_sniped.apply_market_action(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(0),
                    amount: 1,
                    price_per_unit_cred: bid_px,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            let _ = step_turn(&mut eng_bid_sniped, 24);
            let board_sniped = match eng_bid_sniped.finalize_season() {
                Ok(b) => b,
                Err(e) => panic!("fin sniped: {e}"),
            };
            let s0_eq_sniped = board_sniped
                .standings
                .iter()
                .find(|x| x.seat == 0)
                .map_or(0, |x| x.net_asset_equity);
            let s1_eq_sniped = board_sniped
                .standings
                .iter()
                .find(|x| x.seat == 1)
                .map_or(0, |x| x.net_asset_equity);

            println!(
                "N={n}: Raw(0) parity={parity_raw0} s0_hold={s0_hold_base} base_mark={s0_mark_base} s0_base_eq={s0_eq_base}"
            );
            println!(
                "  WASH: T22..24 (stock, min_2pct_q)={q_up:?} gross_vol={gross_vol} pingpong_net_inv={imp_up} import_cred_cost={cost_up}"
            );
            println!(
                "    UP   (px={px_up}): s0_mark={mk_up} s0_eq={eq_up} delta_eq=+{}",
                eq_up - s0_eq_base
            );
            println!(
                "    DOWN (px={px_dn}, inv={imp_dn}, cost={cost_dn}): s0_mark={mk_dn} s0_eq={eq_dn} delta_eq={}",
                eq_dn - s0_eq_base
            );
            println!(
                "  HIGH-BID (1 milli-unit @ px={bid_px}, locked={s1_cred_t24}): s0_eq_uncrossed={s0_eq_bid} lift=+{} | s1_eq_base={s1_eq_before_bid} s1_eq_uncrossed={s1_eq_uncrossed} s1_eq_if_sniped={s1_eq_sniped} (s1_loss={}) s0_eq_if_sniped={s0_eq_sniped}",
                s0_eq_bid - s0_eq_base,
                s1_eq_sniped - s1_eq_before_bid
            );

            assert_eq!(mk_up, s0_mark_base);
            assert_eq!(mk_dn, s0_mark_base);
            assert_eq!(eq_up, s0_eq_base);
            assert_eq!(eq_dn, s0_eq_base);
            let expected_bid_mark = s1_cred_t24
                + ((i128::from(s0_hold_base - 1) * i128::from(parity_raw0) * 5_000) / 10_000_000)
                    as Units;
            assert_eq!(s0_eq_bid - s0_eq_base, expected_bid_mark - s0_mark_base);
            assert_eq!(s0_eq_sniped, s0_eq_base);
        }
    }

    fn det_bytes_32(tag: &[u8], n: u8, table_idx: u32, season_idx: u32, extra: u8) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"pr40-fix2/mandate-meas|");
        h.update(tag);
        h.update([n, extra]);
        h.update(table_idx.to_le_bytes());
        h.update(season_idx.to_le_bytes());
        h.finalize().into()
    }

    fn cross_seat_var(values: &[f64]) -> f64 {
        if values.len() < 2 {
            return 0.0;
        }
        let mean = values.iter().sum::<f64>() / (values.len() as f64);
        values.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / (values.len() as f64)
    }

    fn run_generated_table_season(
        table: &RingTable,
        table_idx: u32,
        season_idx: u32,
        active_ring_play: bool,
    ) -> league_core::FinalScoreboard {
        use league_core::{
            AssetId, AtomicSwapProposal, CharterBidOrder, CharterRole, ContractDeliveryOrder,
            ConverterRunOrder, CouncilPlatform, CouncilVoteBallot, DEFAULT_CLOSING_HAZARD_BPS,
            HARD_TURN_CAP, MarketAction, OfferProposal, OfferSide, OrderReveal,
            ProjectDeliveryOrder, SealedOrderBundle, Seat, Turn, TurnEngine, TurnPhase,
            compute_order_commitment,
        };
        use sha2::{Digest, Sha256};

        let n_u8 = table.seat_count.as_u8();
        let n = usize::from(n_u8);
        let tid =
            TableId(50_000 + u64::from(n_u8) * 10_000 + u64::from(table_idx * 100 + season_idx));
        let deal_seed = det_bytes_32(b"deal", n_u8, table_idx, season_idx, 0xA1);
        let salts: Vec<RevealSalt> = (0..n_u8)
            .map(|s| RevealSalt(det_bytes_32(b"salt", n_u8, table_idx, season_idx, s + 1)))
            .collect();
        let (commits, deal) = match deal_mandates_for_ring_table(table, tid, deal_seed, &salts) {
            Ok(p) => p,
            Err(e) => panic!("deal_mandates_for_ring_table failed: {e}"),
        };

        let mut spec = ring_table_to_core_spec(table, 15_000);
        spec.table_id = tid;
        spec.closing_hazard_bps = DEFAULT_CLOSING_HAZARD_BPS;
        spec.dealt_mandate_commits = commits.clone();
        // Use dynamic beacon shocks rather than static Turn-3 shock so seasons vary across draws.
        spec.shocks.clear();

        let mut engine = match TurnEngine::new(spec) {
            Ok(e) => e,
            Err(e) => panic!("TurnEngine::new failed: {e}"),
        };
        for s in 0..n_u8 {
            if let Err(e) = engine.submit_mandate_commitment(Seat(s), commits[usize::from(s)]) {
                panic!("submit_mandate_commitment failed: {e}");
            }
        }

        let eval_entropy = det_bytes_32(b"eval", n_u8, table_idx, season_idx, 0xFE);
        let rot = (season_idx as usize) % n;

        for t in 1..=HARD_TURN_CAP {
            if engine.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            let mut bundles = vec![SealedOrderBundle::default(); n];
            let p0 = rot as u8;
            let p1 = ((rot + 1) % n) as u8;
            let p2 = ((rot + 2) % n) as u8;

            if active_ring_play && t >= 2 {
                let trade_domain = if t >= 5 { Some(0) } else { None };
                // 1. Active DEX batch market across chiral ring: each seat `b` buys a 600 milli-unit
                //    slice of its primary converter's first Raw input via CRED DEX offers at 115% of
                //    base parity (`>= 2%` of start-of-turn Raw stock, exercising close-mark rules).
                for b in 0..n_u8 {
                    let first_in =
                        engine.state().spec.seats[usize::from(b)].converters[0].inputs[0];
                    let seller = first_in.0;
                    let q = 600_i64;
                    let parity = engine.state().parities.asset_parity(AssetId::Raw(seller));
                    let px = ((i128::from(parity) * 1_150) / 1_000).max(1) as Units;
                    let need_cred = ((i128::from(q) * i128::from(px) + 999) / 1_000 + 200) as Units;
                    if seller != b
                        && engine.state().seats[usize::from(b)].cred >= need_cred
                        && engine.state().seats[usize::from(seller)].raw[usize::from(seller)] >= q
                    {
                        let _ = engine.apply_market_action(MarketAction::PostOffer {
                            seat: b,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(seller),
                                amount: q,
                                price_per_unit_cred: px,
                                domain_id: trade_domain,
                                is_hybrid: false,
                            },
                        });
                        let _ = engine.apply_market_action(MarketAction::PostOffer {
                            seat: seller,
                            order: OfferProposal {
                                side: OfferSide::Sell,
                                good: AssetId::Raw(seller),
                                amount: q,
                                price_per_unit_cred: px,
                                domain_id: trade_domain,
                                is_hybrid: false,
                            },
                        });
                    }
                }
                // Role `(rot + 3) % n` also rests a lowball Buy offer at 30% of parity on Raw(p0)
                // on turns >= 22 so resting low bids are also exercised in realistic play.
                if t >= 22 {
                    let bidder = ((rot + 3) % n) as u8;
                    if bidder != p0 && engine.state().seats[usize::from(bidder)].cred >= 1_500 {
                        let parity0 = engine.state().parities.asset_parity(AssetId::Raw(p0));
                        let low_px = ((i128::from(parity0) * 300) / 1_000).max(1) as Units;
                        let _ = engine.apply_market_action(MarketAction::PostOffer {
                            seat: bidder,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(p0),
                                amount: 1_000,
                                price_per_unit_cred: low_px,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        });
                    }
                }

                // 2. Bilateral Raw swaps, converter runs, upkeep deliveries, and Works deliveries.
                if t >= 3 {
                    let swap_amt = if n >= 5 { 1_900_i64 } else { 2_400_i64 };
                    for s in 0..n_u8 {
                        let s_idx = usize::from(s);
                        let next_1 = ((s_idx + 1) % n) as u8;
                        if engine.state().seats[s_idx].raw[s_idx] >= 6_000
                            && engine.state().seats[usize::from(next_1)].raw[usize::from(next_1)]
                                >= 6_000
                        {
                            let sw = AtomicSwapProposal {
                                swap_id: u64::from(t) * 100 + u64::from(s),
                                maker: s,
                                taker: next_1,
                                maker_asset: AssetId::Raw(s),
                                maker_amount: swap_amt,
                                taker_asset: AssetId::Raw(next_1),
                                taker_amount: swap_amt,
                                domain_id: trade_domain,
                            };
                            bundles[s_idx].atomic_swaps.push(sw.clone());
                            bundles[usize::from(next_1)].atomic_swaps.push(sw);
                        }
                        let next_2 = ((s_idx + 2) % n) as u8;
                        if (n > 4 || s_idx < 2)
                            && engine.state().seats[s_idx].raw[s_idx] >= 6_000
                            && engine.state().seats[usize::from(next_2)].raw[usize::from(next_2)]
                                >= 6_000
                        {
                            let sw2 = AtomicSwapProposal {
                                swap_id: u64::from(t) * 100 + 50 + u64::from(s),
                                maker: s,
                                taker: next_2,
                                maker_asset: AssetId::Raw(s),
                                maker_amount: swap_amt,
                                taker_asset: AssetId::Raw(next_2),
                                taker_amount: swap_amt,
                                domain_id: trade_domain,
                            };
                            bundles[s_idx].atomic_swaps.push(sw2.clone());
                            bundles[usize::from(next_2)].atomic_swaps.push(sw2);
                        }
                        if n >= 5 && (n > 6 || s_idx < n / 2 || n == 5) {
                            let next_3 = ((s_idx + 3) % n) as u8;
                            if (n == 5 && s < next_3 || n == 6 && s_idx < 3)
                                && engine.state().seats[s_idx].raw[s_idx] >= 6_000
                                && engine.state().seats[usize::from(next_3)].raw
                                    [usize::from(next_3)]
                                    >= 6_000
                            {
                                let sw3 = AtomicSwapProposal {
                                    swap_id: u64::from(t) * 100 + 80 + u64::from(s),
                                    maker: s,
                                    taker: next_3,
                                    maker_asset: AssetId::Raw(s),
                                    maker_amount: 1_500,
                                    taker_asset: AssetId::Raw(next_3),
                                    taker_amount: 1_500,
                                    domain_id: trade_domain,
                                };
                                bundles[s_idx].atomic_swaps.push(sw3.clone());
                                bundles[usize::from(next_3)].atomic_swaps.push(sw3);
                            }
                        }
                    }

                    for s in 0..n_u8 {
                        let s_idx = usize::from(s);
                        let role = (s_idx + n - rot) % n;
                        // Deliver Refined(s) to whichever seat consumes Refined(s) as upkeep.
                        if let Some(consumer) = (0..n_u8).find(|&c| {
                            c != s && engine.state().spec.seats[usize::from(c)].upkeep_good == s
                        }) {
                            let need_up = engine.state().spec.seats[usize::from(consumer)]
                                .upkeep_amount
                                + 250;
                            if engine.state().seats[s_idx].refined[s_idx] >= need_up {
                                bundles[s_idx]
                                    .contract_deliveries
                                    .push(ContractDeliveryOrder {
                                        counterparty: consumer,
                                        good: AssetId::Refined(s),
                                        gross_amount: need_up,
                                        domain_id: trade_domain,
                                        dual_escrow_id: None,
                                    });
                            }
                        }
                        let conv_len = engine.state().spec.seats[s_idx].converters.len();
                        bundles[s_idx].converter_runs.push(ConverterRunOrder {
                            recipe_idx: 0,
                            runs: if role == 0 && n == 4 { 2 } else { 1 },
                        });
                        if conv_len >= 2 && (n >= 5 || role == 0) {
                            bundles[s_idx].converter_runs.push(ConverterRunOrder {
                                recipe_idx: 1,
                                runs: 1,
                            });
                        }
                        if conv_len >= 3 && role == 0 {
                            bundles[s_idx].converter_runs.push(ConverterRunOrder {
                                recipe_idx: 2,
                                runs: 1,
                            });
                        }
                        if engine.state().seats[s_idx].cred_debt > 0 && role != n - 1 {
                            bundles[s_idx].debt_repayment_cred = 600;
                        }
                    }

                    let pair_count = n / 2;
                    for k in 0..pair_count {
                        let lead = ((usize::from(t) + rot + 2 * k) % n) as u8;
                        let lead_role = (usize::from(lead) + n - rot) % n;
                        if lead_role == n - 1 && t.is_multiple_of(2) {
                            continue;
                        }
                        let proj = &engine.state().spec.projects[usize::from(lead)].clone();
                        if proj.inputs.len() == 2 {
                            let partner = if proj.inputs[0].0 == lead {
                                proj.inputs[1].0
                            } else {
                                proj.inputs[0].0
                            };
                            let req_amt = proj.inputs[0].1.max(proj.inputs[1].1) + 200;
                            if partner != lead
                                && engine.state().seats[usize::from(lead)].refined
                                    [usize::from(lead)]
                                    >= req_amt * 2
                                && engine.state().seats[usize::from(partner)].refined
                                    [usize::from(partner)]
                                    >= req_amt
                            {
                                let rsw = AtomicSwapProposal {
                                    swap_id: u64::from(t) * 1_000 + u64::from(lead),
                                    maker: partner,
                                    taker: lead,
                                    maker_asset: AssetId::Refined(partner),
                                    maker_amount: req_amt,
                                    taker_asset: AssetId::Refined(lead),
                                    taker_amount: req_amt,
                                    domain_id: trade_domain,
                                };
                                bundles[usize::from(partner)].atomic_swaps.push(rsw.clone());
                                bundles[usize::from(lead)].atomic_swaps.push(rsw);
                            }
                            bundles[usize::from(lead)].project_deliveries.push(
                                ProjectDeliveryOrder {
                                    project_id: proj.id,
                                    batches: 1,
                                },
                            );
                        }
                    }
                }

                if t.is_multiple_of(4) {
                    for s in 0..n_u8 {
                        let platform = CouncilPlatform {
                            proposed_tariffs_bps: vec![(p0, (700, 500))],
                            cartel_admissions: if s == p0 {
                                (0..n_u8).map(|m| (0, m)).collect()
                            } else {
                                Vec::new()
                            },
                            cartel_revocations: Vec::new(),
                        };
                        let _ = engine.apply_market_action(MarketAction::StandForOffice {
                            seat: s,
                            platform,
                        });
                    }
                    for (s_idx, b) in bundles.iter_mut().enumerate() {
                        b.council_vote = Some(CouncilVoteBallot {
                            trade_minister: ((s_idx + 1) % n) as u8,
                            cartel_chair: p0,
                        });
                    }
                }
                if t.is_multiple_of(6) {
                    bundles[usize::from(p0)].charter_bids.push(CharterBidOrder {
                        role: CharterRole::OracleAuditor,
                        bid_cred: 200,
                    });
                    bundles[usize::from(p1)].charter_bids.push(CharterBidOrder {
                        role: CharterRole::CustomsInspector,
                        bid_cred: 200,
                    });
                    bundles[usize::from(p2)].charter_bids.push(CharterBidOrder {
                        role: CharterRole::ClearinghouseBroker,
                        bid_cred: 200,
                    });
                }
            }

            if engine.advance_to_commit().is_err() {
                panic!("advance_to_commit failed");
            }
            let mut revs = Vec::with_capacity(n);
            for (s_idx, bundle) in bundles.into_iter().enumerate() {
                let s = s_idx as u8;
                let rev = OrderReveal {
                    bundle,
                    salt: RevealSalt(det_bytes_32(b"ord", n_u8, table_idx, season_idx, t ^ s)),
                };
                let Ok(c) = compute_order_commitment(tid, Turn(t), Seat(s), &rev) else {
                    panic!("compute_order_commitment failed");
                };
                if engine.submit_order_commitment(Seat(s), c).is_err() {
                    panic!("submit_order_commitment failed");
                }
                revs.push((s, rev));
            }
            if engine.advance_to_reveal().is_err() {
                panic!("advance_to_reveal failed");
            }
            for (s, rev) in revs {
                if engine.submit_order_reveal(Seat(s), rev).is_err() {
                    panic!("submit_order_reveal failed");
                }
            }
            let mut bh = Sha256::new();
            bh.update(b"pr40-fix2/turn-beacon|");
            bh.update(eval_entropy);
            bh.update(tid.0.to_le_bytes());
            bh.update([t]);
            let turn_beacon: [u8; 32] = bh.finalize().into();
            let rec = match engine.sweep_turn(turn_beacon) {
                Ok(r) => r,
                Err(e) => panic!("sweep_turn failed: {e}"),
            };
            if rec.season_closed {
                break;
            }
        }

        for s in 0..n_u8 {
            if let Err(e) =
                engine.submit_mandate_reveal(Seat(s), deal.seat_reveals[usize::from(s)].clone())
            {
                panic!("submit_mandate_reveal failed: {e}");
            }
        }
        match engine.finalize_season() {
            Ok(b) => b,
            Err(e) => panic!("finalize_season failed: {e}"),
        }
    }

    #[test]
    fn pr40_fix2_generated_tables_mandate_measurement_200_seasons_per_n() {
        use league_core::{DEFAULT_MANDATE_ALPHA_BPS, table_median_net_worth};

        const TABLE_SEEDS: [u64; 5] = [42, 1, 7, 2026, 31337];
        const SEASONS_PER_SEED: u32 = 40;
        const TOTAL_SEASONS_PER_N: u32 = (TABLE_SEEDS.len() as u32) * SEASONS_PER_SEED;

        for (label, active_ring_play) in [
            (
                "generated RingTables (5 tables x 40 seasons = 200/N) active chiral DEX + Works",
                true,
            ),
            (
                "generated RingTables (5 tables x 40 seasons = 200/N) idle unclosed-ring baseline",
                false,
            ),
        ] {
            println!(
                "\n=== PR40 FIX2 GENERATED-TABLE MANDATE MEASUREMENT [{label}] (alpha = {:.4}) ===",
                f64::from(DEFAULT_MANDATE_ALPHA_BPS) / 10_000.0,
            );
            for sc in [SeatCount::Four, SeatCount::Five, SeatCount::Six] {
                let n = sc.as_u8();
                let cfg = GeneratorConfig {
                    seat_count: Some(sc),
                    ..GeneratorConfig::default()
                };
                let mut leader_flips = 0_u32;
                let mut sum_var_b = 0.0_f64;
                let mut sum_var_s = 0.0_f64;
                let mut per_table_ratio_sum = 0.0_f64;
                let mut non_zero_var_tables = 0_u32;
                let mut positive_bonuses = 0_u32;
                let mut mean_median_nw = 0.0_f64;

                for (t_idx, &layout_seed) in TABLE_SEEDS.iter().enumerate() {
                    let (table, _) = match generate_ring_table(layout_seed, &cfg) {
                        Ok(p) => p,
                        Err(e) => panic!("generate_ring_table N={n} idx={t_idx} failed: {e}"),
                    };
                    for s_idx in 0..SEASONS_PER_SEED {
                        let board = run_generated_table_season(
                            &table,
                            t_idx as u32,
                            s_idx,
                            active_ring_play,
                        );
                        let total_leader = board.standings[0].seat;
                        let mut by_pre_nw: Vec<(u8, Units)> = board
                            .standings
                            .iter()
                            .map(|e| (e.seat, e.project_equity + e.net_asset_equity))
                            .collect();
                        let nws_only: Vec<Units> = by_pre_nw.iter().map(|&(_, w)| w).collect();
                        mean_median_nw += table_median_net_worth(&nws_only) as f64;

                        by_pre_nw.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                        if total_leader != by_pre_nw[0].0 {
                            leader_flips += 1;
                        }

                        let mut bs = Vec::with_capacity(usize::from(n));
                        let mut ss = Vec::with_capacity(usize::from(n));
                        for e in &board.standings {
                            if e.mandate_bonus > 0 {
                                positive_bonuses += 1;
                            }
                            bs.push(e.mandate_bonus as f64);
                            ss.push(e.total_equity as f64);
                        }
                        let vb = cross_seat_var(&bs);
                        let vs = cross_seat_var(&ss);
                        sum_var_b += vb;
                        sum_var_s += vs;
                        if vs > 0.0 {
                            per_table_ratio_sum += vb / vs;
                            non_zero_var_tables += 1;
                        }
                    }
                }

                mean_median_nw /= f64::from(TOTAL_SEASONS_PER_N);
                let pooled_var_share_pct = if sum_var_s > 0.0 {
                    100.0 * sum_var_b / sum_var_s
                } else {
                    0.0
                };
                let mean_table_var_share_pct = if non_zero_var_tables > 0 {
                    100.0 * per_table_ratio_sum / f64::from(non_zero_var_tables)
                } else {
                    0.0
                };
                let flip_rate_pct =
                    100.0 * f64::from(leader_flips) / f64::from(TOTAL_SEASONS_PER_N);
                let pos_bonus_pct = 100.0 * f64::from(positive_bonuses)
                    / f64::from(TOTAL_SEASONS_PER_N * u32::from(n));

                println!(
                    "N={n} (seasons={TOTAL_SEASONS_PER_N}): mean_Med(W) = {mean_median_nw:.1} milli, \
                     pooled Var(B)/Var(S) = {pooled_var_share_pct:.2}%, \
                     mean per-table Var(B)/Var(S) = {mean_table_var_share_pct:.2}%, \
                     leader_flip_rate = {flip_rate_pct:.2}% ({leader_flips}/{TOTAL_SEASONS_PER_N}), \
                     positive_bonus_rate = {pos_bonus_pct:.2}% ({positive_bonuses}/{})",
                    TOTAL_SEASONS_PER_N * u32::from(n),
                );
                assert_eq!(non_zero_var_tables, TOTAL_SEASONS_PER_N);
                assert!(positive_bonuses > 0);
            }
        }
    }
}
