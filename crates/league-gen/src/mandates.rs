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
            CLOSING_ROLL_WINDOW_LEN, ConverterRecipe, DEFAULT_MANDATE_ALPHA_BPS,
            DEFAULT_TOWN_IMPORT_PREMIUM_BPS, DEFAULT_TWAP_MIN_TURN_STOCK_BPS,
            DEFAULT_TWAP_WINDOW_TURNS, DEFAULT_WORKS_STIPEND_PER_UNIT_CRED, ProjectRecipe,
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
            twap_window_turns: DEFAULT_TWAP_WINDOW_TURNS,
            twap_min_turn_stock_bps: DEFAULT_TWAP_MIN_TURN_STOCK_BPS,
        }
    }

    #[test]
    fn pr40_fix1_generated_table_wash_and_high_bid_residuals() {
        use league_core::{
            AssetId, BPS_SCALE, DEFAULT_TWAP_MIN_TURN_STOCK_BPS, MarketAction, OfferProposal,
            OfferSide, OrderReveal, SealedOrderBundle, Seat, Turn, TurnEngine,
            compute_order_commitment, derive_turn_beacon, mark_good_holding_at_close,
        };

        let initial_cred = 15_000_i64; // canonical 15.000 CRED starting capital (`types.rs:906`)
        println!(
            "\n=== PR40 FIX1 RESIDUAL MEASUREMENTS ON GENERATED TABLES (seed=42, initial_cred=15.000 CRED) ==="
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
                        let stock_start = eng.state().good_stock_at_turn_start(AssetId::Raw(0));
                        let min_q = ((i128::from(stock_start)
                            * i128::from(DEFAULT_TWAP_MIN_TURN_STOCK_BPS)
                            + (BPS_SCALE - 1))
                            / BPS_SCALE) as Units;
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

            assert!(eq_up > s0_eq_base);
            assert!(eq_dn < s0_eq_base);
            let expected_bid_mark = s1_cred_t24
                + ((i128::from(s0_hold_base - 1) * i128::from(parity_raw0) * 5_000) / 10_000_000)
                    as Units;
            assert_eq!(s0_eq_bid - s0_eq_base, expected_bid_mark - s0_mark_base);
            assert_eq!(s0_eq_sniped, s0_eq_base);
        }
    }
}
