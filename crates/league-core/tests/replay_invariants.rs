//! Integration tests verifying v2 simultaneous turn engine mechanics,
//! dual-track settlement, 4-turn council and 6-turn charter politics,
//! 3-stage Horizon Shocks, PBRS anti-wash invariants, error contracts,
//! and bit-identical deterministic Steward replay from JSON logs.

use std::collections::BTreeMap;

use league_core::{
    ActiveCharter, AssetId, AtomicSwapProposal, BPS_SCALE, BondProposal, CharterBidOrder,
    CharterRole, CheckProposal, CheckStatus, CommitmentHash, ConverterRecipe, ConverterRunOrder,
    CouncilVoteBallot, CreditTier, DEFAULT_PBRS_GAMMA_BPS, EngineError, EngineState,
    EscrowFinishUnlock, EscrowProposal, FinalScoreboard, FundamentalParities, IntentPledge,
    IntentTarget, LoggedTurn, MAX_TARIFF_BPS, MandateKind, MandateReveal, MarketAction,
    OfferProposal, OfferSide, OrderReveal, PreimageSecret, PrivateMandate, ProjectDeliveryOrder,
    ProjectRecipe, RegentDoctrine, RevealSalt, SealedOrderBundle, SeasonReplayLog, Seat, ShockKind,
    TableId, TableSpec, Turn, TurnEngine, TurnPhase, UNIT_SCALE, ceil_bps_fee, ceil_units_parity,
    compute_final_scoreboard, compute_mandate_commitment, compute_net_asset_equity,
    compute_order_commitment, compute_pbrs_reward, compute_potential, compute_preimage_condition,
    derive_turn_beacon, floor_bps_part, half_up_bps, mul_units_parity, replay_season,
    scale_bps_signed,
};

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn commit_and_reveal_all(
    engine: &mut TurnEngine,
    table_id: TableId,
    turn: u8,
    bundles: Vec<SealedOrderBundle>,
) -> (BTreeMap<u8, CommitmentHash>, BTreeMap<u8, OrderReveal>) {
    must_ok(engine.advance_to_commit(), "advance_to_commit");
    let mut commits = BTreeMap::new();
    let mut reveals = BTreeMap::new();
    for (s_idx, bundle) in bundles.into_iter().enumerate() {
        let s = s_idx as u8;
        let salt = RevealSalt([s.wrapping_mul(17).wrapping_add(turn); 32]);
        let rev = OrderReveal { bundle, salt };
        let ch = must_ok(
            compute_order_commitment(table_id, Turn(turn), Seat(s), &rev),
            "compute_order_commitment",
        );
        must_ok(
            engine.submit_order_commitment(Seat(s), ch),
            "submit_order_commitment",
        );
        commits.insert(s, ch);
        reveals.insert(s, rev);
    }
    must_ok(engine.advance_to_reveal(), "advance_to_reveal");
    for (&s, rev) in &reveals {
        must_ok(
            engine.submit_order_reveal(Seat(s), rev.clone()),
            "submit_order_reveal",
        );
    }
    (commits, reveals)
}

fn run_scripted_full_season(
    seat_count: u8,
    seed: u64,
) -> (SeasonReplayLog, FinalScoreboard, Vec<[u8; 32]>) {
    let table_id = TableId(500 + u64::from(seat_count) * 10 + seed);
    let spec = must_ok(
        TableSpec::standard_chiral_ring(table_id, seat_count, seed),
        "standard_chiral_ring",
    );
    let n = usize::from(seat_count);
    let mut engine = must_ok(TurnEngine::new(spec.clone()), "TurnEngine::new");

    // Assign all 4 MandateKind variants across seats 0..seat_count.
    let mut mandate_commits = vec![None; n];
    let mut mandate_reveals = vec![None; n];
    for s in 0..seat_count {
        let kind = match s % 4 {
            0 => MandateKind::MinProjectEquity { target: 2_000 },
            1 => MandateKind::MinRefinedVariety {
                distinct_goods: 1,
                min_each: 1_000,
            },
            2 => MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            _ => MandateKind::MinActivePartners { min_partners: 2 },
        };
        let rev = MandateReveal {
            mandate: PrivateMandate {
                kind,
                bonus_equity: 3_500,
            },
            salt: RevealSalt([0xA0_u8.wrapping_add(s); 32]),
        };
        let commit = must_ok(
            compute_mandate_commitment(table_id, Seat(s), &rev),
            "compute_mandate_commitment",
        );
        must_ok(
            engine.submit_mandate_commitment(Seat(s), commit),
            "submit_mandate_commitment",
        );
        mandate_commits[usize::from(s)] = Some(commit);
        mandate_reveals[usize::from(s)] = Some(rev);
    }

    let mut turns = Vec::new();
    let mut digests = Vec::new();
    let mut tracked_escrow_secret: Option<(u64, u8, PreimageSecret)> = None;
    let mut tracked_check_id: Option<(u64, u8, u8)> = None;

    for t in 1..=28u8 {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut actions = Vec::new();
        let mut bundles = vec![SealedOrderBundle::default(); n];

        // Turn 2 onward: alternate circular DEX quotes between Seat 0 and Seat 1 so CRED recycles,
        // plus test a resting order CancelOffer on Turn 3.
        if t >= 2 {
            let (buyer, seller) = if t.is_multiple_of(2) {
                (0_u8, 1_u8)
            } else {
                (1_u8, 0_u8)
            };
            if engine.state().seats[usize::from(buyer)].cred >= 2_800
                && engine.state().seats[usize::from(seller)].raw[usize::from(seller)] >= 2_000
            {
                let buy_act = MarketAction::PostOffer {
                    seat: buyer,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(seller),
                        amount: 2_000,
                        price_per_unit_cred: 1_400,
                        domain_id: Some(0),
                        is_hybrid: false,
                    },
                };
                must_ok(engine.apply_market_action(buy_act.clone()), "buy_act");
                actions.push(buy_act);

                let sell_act = MarketAction::PostOffer {
                    seat: seller,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(seller),
                        amount: 2_000,
                        price_per_unit_cred: 1_400,
                        domain_id: Some(0),
                        is_hybrid: false,
                    },
                };
                must_ok(engine.apply_market_action(sell_act.clone()), "sell_act");
                actions.push(sell_act);
            }

            if t == 3 {
                let open_id = engine.state().next_offer_id;
                let rest = MarketAction::PostOffer {
                    seat: 2,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(2),
                        amount: 1_000,
                        price_per_unit_cred: 5_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                };
                must_ok(engine.apply_market_action(rest.clone()), "rest");
                actions.push(rest);

                let cancel = MarketAction::CancelOffer {
                    seat: 2,
                    offer_id: open_id,
                };
                must_ok(engine.apply_market_action(cancel.clone()), "cancel");
                actions.push(cancel);
            }
        }

        if t == 2 {
            // Conditional TokenEscrow unlocked cleanly in Turn 2 Reveal + CheckCreate cashed in Turn 3.
            let secret = PreimageSecret([0x42; 32]);
            let cond = compute_preimage_condition(&secret);
            let eid = engine.state().next_escrow_id;
            let esc_act = MarketAction::CreateEscrow {
                seat: 0,
                escrow: EscrowProposal {
                    destination: 2,
                    asset: AssetId::Cred,
                    amount: 800,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 6,
                    condition_hash: Some(cond),
                    bond: None,
                },
            };
            must_ok(engine.apply_market_action(esc_act.clone()), "create escrow");
            actions.push(esc_act);
            tracked_escrow_secret = Some((eid, 2, secret));

            let cid = engine.state().next_check_id;
            let chk_act = MarketAction::CreateCheck {
                seat: 1,
                check: CheckProposal {
                    payee: 0,
                    asset: AssetId::Cred,
                    send_max: 600,
                    deliver_min: 500,
                    due_turn: 3,
                    expiration_turn: 6,
                },
            };
            must_ok(engine.apply_market_action(chk_act.clone()), "create check");
            actions.push(chk_act);
            tracked_check_id = Some((cid, 1, 0));
        }

        if t == 2
            && let Some((eid, dest, secret)) = tracked_escrow_secret.take()
        {
            bundles[usize::from(dest)]
                .escrow_finishes
                .push(EscrowFinishUnlock {
                    escrow_id: eid,
                    preimage: Some(secret),
                });
        }
        if t == 3
            && let Some((cid, _drawer, payee)) = tracked_check_id.take()
        {
            bundles[usize::from(payee)].check_cashes.push(cid);
        }

        // Bilateral co-signed swaps feeding every seat's chiral converter inputs (i+1 and i+2)
        // when balances allow, plus converter and project execution & voluntary debt repayment.
        if t >= 2 {
            for s in 0..seat_count {
                let s_idx = usize::from(s);
                let next_1 = ((s_idx + 1) % n) as u8;
                let next_2 = ((s_idx + 2) % n) as u8;
                if engine.state().seats[s_idx].raw[s_idx] >= 2_500
                    && engine.state().seats[usize::from(next_1)].raw[usize::from(next_1)] >= 2_500
                    && s < next_1
                {
                    let swap_a = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + u64::from(s),
                        maker: s,
                        taker: next_1,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_000,
                        taker_asset: AssetId::Raw(next_1),
                        taker_amount: 2_000,
                        domain_id: Some(0),
                    };
                    bundles[s_idx].atomic_swaps.push(swap_a.clone());
                    bundles[usize::from(next_1)].atomic_swaps.push(swap_a);
                }
                if engine.state().seats[s_idx].raw[s_idx] >= 2_000
                    && engine.state().seats[usize::from(next_2)].raw[usize::from(next_2)] >= 2_000
                    && s < next_2
                {
                    let swap_b = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + 50 + u64::from(s),
                        maker: s,
                        taker: next_2,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_000,
                        taker_asset: AssetId::Raw(next_2),
                        taker_amount: 2_000,
                        domain_id: Some(0),
                    };
                    bundles[s_idx].atomic_swaps.push(swap_b.clone());
                    bundles[usize::from(next_2)].atomic_swaps.push(swap_b);
                }
                bundles[s_idx].converter_runs.push(ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 1,
                });
                if engine.state().seats[s_idx].refined[s_idx] >= 2_000
                    && engine.state().seats[s_idx].refined[usize::from(next_1)] >= 2_000
                {
                    bundles[s_idx]
                        .project_deliveries
                        .push(ProjectDeliveryOrder {
                            project_id: u16::from(s) + 1,
                            batches: 1,
                        });
                }
                if engine.state().seats[s_idx].cred_debt > 0 {
                    bundles[s_idx].debt_repayment_cred = 500;
                }
            }
        }

        if t.is_multiple_of(4) {
            for (s_idx, b) in bundles.iter_mut().enumerate() {
                b.council_vote = Some(CouncilVoteBallot {
                    trade_minister: ((s_idx + 1) % n) as u8,
                    proposed_tariffs_bps: vec![(0, (700, 500))],
                    cartel_chair: 0,
                    cartel_admissions: Vec::new(),
                    cartel_revocations: Vec::new(),
                });
            }
        }
        if t.is_multiple_of(6) {
            bundles[0].charter_bids.push(CharterBidOrder {
                role: CharterRole::OracleAuditor,
                bid_cred: 200,
            });
            bundles[1].charter_bids.push(CharterBidOrder {
                role: CharterRole::CustomsInspector,
                bid_cred: 200,
            });
            bundles[2].charter_bids.push(CharterBidOrder {
                role: CharterRole::ClearinghouseBroker,
                bid_cred: 200,
            });
        }

        let (commitments, reveals) = commit_and_reveal_all(&mut engine, table_id, t, bundles);
        let beacon = derive_turn_beacon(seed, table_id, Turn(t));
        let receipt = must_ok(engine.sweep_turn(beacon), "sweep_turn");
        digests.push(receipt.post_state_digest);
        turns.push(LoggedTurn {
            turn: t,
            market_actions: actions,
            commitments,
            reveals,
            turn_beacon: beacon,
        });
        if receipt.season_closed {
            break;
        }
    }

    assert!((24..=28).contains(&turns.len()));
    assert_eq!(engine.state().phase, TurnPhase::AwaitingMandateReveal);
    for (s_idx, rev_opt) in mandate_reveals.iter().enumerate() {
        if let Some(rev) = rev_opt {
            must_ok(
                engine.submit_mandate_reveal(Seat(s_idx as u8), rev.clone()),
                "submit_mandate_reveal",
            );
        }
    }
    let live_board = must_ok(engine.finalize_season(), "finalize_season");
    assert_eq!(engine.state().phase, TurnPhase::Completed);

    let log = SeasonReplayLog {
        spec,
        seed,
        mandate_commits,
        turns,
        mandate_reveals,
    };
    (log, live_board, digests)
}

#[test]
fn deterministic_full_seasons_4_5_6_seats_and_json_replay_match_bit_for_bit() {
    for &(seats, seed) in &[(4_u8, 11_u64), (5_u8, 29_u64), (6_u8, 73_u64)] {
        let (log, live_board, live_digests) = run_scripted_full_season(seats, seed);
        let json = must_ok(serde_json::to_string(&log), "serialize SeasonReplayLog");
        let parsed: SeasonReplayLog = must_ok(serde_json::from_str(&json), "deserialize log");

        let out_a = must_ok(replay_season(&log), "replay_a");
        let out_b = must_ok(replay_season(&parsed), "replay_b");
        assert_eq!(out_a.state_digests, live_digests);
        assert_eq!(out_a, out_b);
        assert_eq!(out_a.scoreboard, live_board);
        assert_eq!(out_a.scoreboard.standings.len(), usize::from(seats));

        // Verify standings sort order: descending total_equity, ties ascending by seat.
        for pair in out_a.scoreboard.standings.windows(2) {
            let cmp_ok = pair[0].total_equity > pair[1].total_equity
                || (pair[0].total_equity == pair[1].total_equity && pair[0].seat < pair[1].seat);
            assert!(cmp_ok);
        }
    }
}

#[test]
fn hard_credit_default_cascade_evicts_cartel_cancels_checks_and_compounds_debt() {
    let tid = TableId(810);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 5), "spec");
    // Ensure Seat 0 has scarce free refined(3) & CRED for clean debt & bounce tests.
    spec.seats[0].initial_cred = 500;
    let mut engine = must_ok(TurnEngine::new(spec), "new");

    // Seat 0 creates 3 checks for 2_000 CRED (only holds 500 -> every CheckCash bounces) + 1 cancelled check.
    for due in 1..=3_u8 {
        must_ok(
            engine.apply_market_action(MarketAction::CreateCheck {
                seat: 0,
                check: CheckProposal {
                    payee: 1,
                    asset: AssetId::Cred,
                    send_max: 2_000,
                    deliver_min: 1_500,
                    due_turn: due,
                    expiration_turn: 10,
                },
            }),
            "create check",
        );
    }
    must_ok(
        engine.apply_market_action(MarketAction::CreateCheck {
            seat: 0,
            check: CheckProposal {
                payee: 2,
                asset: AssetId::Cred,
                send_max: 400,
                deliver_min: 300,
                due_turn: 1,
                expiration_turn: 2,
            },
        }),
        "create check 4",
    );
    must_ok(
        engine.apply_market_action(MarketAction::CancelCheck {
            seat: 0,
            check_id: 4,
        }),
        "cancel check 4",
    );
    assert_eq!(engine.state().checks[&4].status, CheckStatus::Cancelled);

    // Seat 0 places a resting Domain-0 Buy offer before bouncing; reaching D must cancel and refund it.
    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: Some(0),
                is_hybrid: false,
            },
        }),
        "post domain offer",
    );
    assert_eq!(engine.state().offers.len(), 1);

    // Check Open liability reduces Net Asset Equity, keeping PBRS accounting honest.
    let eq_with_open_checks = compute_net_asset_equity(engine.state(), 0);
    assert!(eq_with_open_checks < 500);

    // Bounce check 1 -> Bbb, check 2 -> B, check 3 -> D & cartel eviction!
    for t in 1..=3_u8 {
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        bundles[1].check_cashes.push(u64::from(t));
        if t == 2 {
            bundles[0].debt_repayment_cred = 100;
        }
        commit_and_reveal_all(&mut engine, tid, t, bundles);
        let receipt = must_ok(
            engine.sweep_turn(derive_turn_beacon(5, tid, Turn(t))),
            "sweep",
        );
        assert_eq!(receipt.checks_bounced, vec![u64::from(t)]);
        if t == 3 {
            assert_eq!(receipt.cartel_evictions, vec![0]);
        }
    }

    let s0 = &engine.state().seats[0];
    assert_eq!(s0.credit_tier, CreditTier::D);
    assert_eq!(s0.hard_credit_defaults, 3);
    assert!(s0.cred_debt > 0);
    // Domain-0 offer was cancelled and locked CRED refunded upon cartel eviction.
    assert!(engine.state().offers.is_empty());
    assert!(!engine.state().domains[0].members.contains(&0));

    // Seat 0 at Tier D cannot post prime Domain-0 offers anymore.
    let err = engine.apply_market_action(MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Buy,
            good: AssetId::Raw(1),
            amount: 100,
            price_per_unit_cred: 1_000,
            domain_id: Some(0),
            is_hybrid: false,
        },
    });
    assert!(matches!(err, Err(EngineError::DomainAccessDenied(_))));
}

#[test]
fn politics_charters_pledge_audits_hybrid_bridge_and_all_four_shocks_work() {
    let tid = TableId(820);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 0), "spec");
    // Guarantee non-zero initial balances & explicit shock timing across turns 2..=21.
    for (i, sc) in spec.seats.iter_mut().enumerate() {
        sc.initial_cred = 20_000;
        if i == 0 {
            sc.perishable_decay_bps = 2_000;
        }
    }
    let mut engine = must_ok(TurnEngine::new(spec), "new");

    // Seat 0 anchors Track-2 VoteForSeat pledge (Turn 4) and PreserveCartelMembership pledge (Turn 4)
    // plus RevealPreimage pledge tied to an expiring escrow (cancel_after_turn = 3).
    let pre_secret = PreimageSecret([0x19; 32]);
    let pre_cond = compute_preimage_condition(&pre_secret);
    must_ok(
        engine.apply_market_action(MarketAction::CreateEscrow {
            seat: 2,
            escrow: EscrowProposal {
                destination: 3,
                asset: AssetId::Cred,
                amount: 1_000,
                finish_after_turn: Some(1),
                cancel_after_turn: 3,
                condition_hash: Some(pre_cond),
                bond: None,
            },
        }),
        "expiring escrow",
    );
    for (pid, target, weight) in [
        (
            1,
            IntentTarget::VoteForSeat {
                office_turn: 4,
                candidate: 1,
            },
            400,
        ),
        (
            2,
            IntentTarget::PreserveCartelMembership {
                office_turn: 4,
                domain_id: 0,
                beneficiary: 3,
            },
            600,
        ),
        (
            3,
            IntentTarget::RevealPreimage {
                escrow_id: 1,
                condition: pre_cond,
            },
            500,
        ),
    ] {
        must_ok(
            engine.apply_market_action(MarketAction::RecordIntentPledge {
                seat: 0,
                pledge: IntentPledge {
                    pledge_id: pid,
                    promisor: 0,
                    counterparty: 1,
                    target,
                    honor_weight: weight,
                    memo_digest: [0xAB; 32],
                },
            }),
            "record pledge",
        );
    }

    // Track compliant Steward Delivery Bond created in Turn 1 (Seat 1 -> Seat 2, deadline Turn 3).
    must_ok(
        engine.apply_market_action(MarketAction::CreateEscrow {
            seat: 1,
            escrow: EscrowProposal {
                destination: 2,
                asset: AssetId::Cred,
                amount: 1_500,
                finish_after_turn: None,
                cancel_after_turn: 5,
                condition_hash: Some(compute_preimage_condition(&PreimageSecret([0x22; 32]))),
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 1_000,
                    deadline_turn: 3,
                }),
            },
        }),
        "delivery bond",
    );

    // Run turns 1..=21 observing shocks, politics, and charters.
    let mut saw_supply_freeze_reduction = false;
    let mut saw_tariff_holiday_zero = false;
    let mut saw_demand_surge_bonus = false;
    let mut saw_spoilage_spike = false;

    for t in 1..=21_u8 {
        let mut bundles = vec![SealedOrderBundle::default(); 4];

        if t == 2 {
            // Deliver >1_000 net Raw(1) from Seat 1 -> Seat 2 to satisfy Bond #2 on Turn 3.
            let swap = AtomicSwapProposal {
                swap_id: 901,
                maker: 1,
                taker: 2,
                maker_asset: AssetId::Raw(1),
                maker_amount: 2_000,
                taker_asset: AssetId::Cred,
                taker_amount: 1_000,
                domain_id: Some(0),
            };
            bundles[1].atomic_swaps.push(swap.clone());
            bundles[2].atomic_swaps.push(swap);

            // Also leave a Raw(0) resting Sell offer on the book to verify Step 6 Clawback spoilage anti-parking.
            must_ok(
                engine.apply_market_action(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 3_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }),
                "resting raw sell",
            );
        }

        if t == 4 {
            // Seat 0 breaks VoteForSeat (votes TM=2 instead of 1) and CartelChair(0) revokes Seat 3 -> breaks pledge 2 too!
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                proposed_tariffs_bps: vec![(0, (5_000, 4_000))], // clamped to MAX_TARIFF_BPS = 3_500
                cartel_chair: 0,
                cartel_admissions: Vec::new(),
                cartel_revocations: vec![(0, 3)],
            });
            bundles[1].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                proposed_tariffs_bps: Vec::new(),
                cartel_chair: 0,
                cartel_admissions: Vec::new(),
                cartel_revocations: Vec::new(),
            });
            bundles[2].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                proposed_tariffs_bps: vec![(0, (9_000, 1_200))],
                cartel_chair: 0,
                cartel_admissions: Vec::new(),
                cartel_revocations: Vec::new(),
            });
        }

        if t == 6 {
            // Seat 0 bids CustomsInspector, Seat 1 bids OracleAuditor, Seat 3 (outside Domain 0!) bids ClearinghouseBroker.
            bundles[0].charter_bids.push(CharterBidOrder {
                role: CharterRole::CustomsInspector,
                bid_cred: 300,
            });
            bundles[1].charter_bids.push(CharterBidOrder {
                role: CharterRole::OracleAuditor,
                bid_cred: 300,
            });
            bundles[3].charter_bids.push(CharterBidOrder {
                role: CharterRole::ClearinghouseBroker,
                bid_cred: 300,
            });
        }

        if t == 7 {
            // Seat 0 holds CustomsInspector -> freezes Seat 2 for Turn 7.
            assert!(matches!(
                engine.apply_market_action(MarketAction::FlagWashFreeze {
                    inspector: 0,
                    target_seat: 0,
                }),
                Err(EngineError::InvalidOrder(_))
            ));
            must_ok(
                engine.apply_market_action(MarketAction::FlagWashFreeze {
                    inspector: 0,
                    target_seat: 2,
                }),
                "freeze seat 2",
            );
            assert!(matches!(
                engine.apply_market_action(MarketAction::PostOffer {
                    seat: 2,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 100,
                        price_per_unit_cred: 1_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }),
                Err(EngineError::Unauthorized(_))
            ));

            // Seat 3 (outside Domain 0) holds ClearinghouseBroker -> bridges across Domain 0 via tfHybrid.
            must_ok(
                engine.apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(1),
                        amount: 500,
                        price_per_unit_cred: 1_200,
                        domain_id: Some(0),
                        is_hybrid: false,
                    },
                }),
                "domain 0 sell by member 1",
            );
            let cred3_before = engine.state().seats[3].cred;
            must_ok(
                engine.apply_market_action(MarketAction::PostOffer {
                    seat: 3,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(1),
                        amount: 500,
                        price_per_unit_cred: 1_200,
                        domain_id: Some(0),
                        is_hybrid: true,
                    },
                }),
                "hybrid bridge buy by non-member 3",
            );
            assert!(engine.state().seats[3].cred < cred3_before);
        }

        if t == 8 {
            // OracleAuditor (Seat 1) sees private shock hint scheduled for hint_turn = 8; non-auditor Seat 0 sees empty.
            let hints_1 = must_ok(engine.private_hints_for_auditor(Seat(1)), "hints 1");
            let hints_0 = must_ok(engine.private_hints_for_auditor(Seat(0)), "hints 0");
            assert_eq!(hints_1.len(), 1);
            assert_eq!(hints_1[0].kind, ShockKind::TariffHoliday);
            assert!(hints_0.is_empty());

            // Readmit Seat 3 back into Domain 0 on Turn 8.
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 0,
                proposed_tariffs_bps: Vec::new(),
                cartel_chair: 0,
                cartel_admissions: vec![(0, 3)],
                cartel_revocations: Vec::new(),
            });
        }

        if t == 11 {
            // ShockKind::TariffHoliday realized on Turn 10 (target_seat = 1, magnitude = 10_000) -> 0% export tariff on Raw(1) outside domain!
            let deliv_before = engine
                .state()
                .delivery_ledger
                .get(&(1, 2, AssetId::Raw(1)))
                .copied()
                .unwrap_or(0);
            let holiday_swap = AtomicSwapProposal {
                swap_id: 999,
                maker: 1,
                taker: 2,
                maker_asset: AssetId::Raw(1),
                maker_amount: 1_000,
                taker_asset: AssetId::Cred,
                taker_amount: 500,
                domain_id: None,
            };
            bundles[1].atomic_swaps.push(holiday_swap.clone());
            bundles[2].atomic_swaps.push(holiday_swap);
            commit_and_reveal_all(&mut engine, tid, t, bundles);
            let _ = must_ok(
                engine.sweep_turn(derive_turn_beacon(0, tid, Turn(t))),
                "sweep holiday",
            );
            let deliv_after = engine
                .state()
                .delivery_ledger
                .get(&(1, 2, AssetId::Raw(1)))
                .copied()
                .unwrap_or(0);
            if deliv_after - deliv_before == 1_000 {
                saw_tariff_holiday_zero = true;
            }
            continue;
        }

        if t == 15 {
            // Swap inputs for Seat 2 converter before DemandSurge (realize_turn 15, target_seat 2).
            let s1 = AtomicSwapProposal {
                swap_id: 1501,
                maker: 2,
                taker: 3,
                maker_asset: AssetId::Raw(2),
                maker_amount: 2_000,
                taker_asset: AssetId::Raw(3),
                taker_amount: 2_000,
                domain_id: Some(0),
            };
            let s2 = AtomicSwapProposal {
                swap_id: 1502,
                maker: 2,
                taker: 0,
                maker_asset: AssetId::Raw(2),
                maker_amount: 2_000,
                taker_asset: AssetId::Raw(0),
                taker_amount: 2_000,
                domain_id: Some(0),
            };
            bundles[2].atomic_swaps.push(s1.clone());
            bundles[3].atomic_swaps.push(s1);
            bundles[2].atomic_swaps.push(s2.clone());
            bundles[0].atomic_swaps.push(s2);
            bundles[2].converter_runs.push(ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            });
            // Also produce Refined(3) on Seat 3 so Turn 16 Project #3 (`inputs: [(2, 1000), (3, 1000)]`) can fire!
            let s4 = AtomicSwapProposal {
                swap_id: 1504,
                maker: 3,
                taker: 0,
                maker_asset: AssetId::Raw(3),
                maker_amount: 2_000,
                taker_asset: AssetId::Raw(0),
                taker_amount: 2_000,
                domain_id: Some(0),
            };
            let s5 = AtomicSwapProposal {
                swap_id: 1505,
                maker: 3,
                taker: 1,
                maker_asset: AssetId::Raw(3),
                maker_amount: 2_000,
                taker_asset: AssetId::Raw(1),
                taker_amount: 2_000,
                domain_id: Some(0),
            };
            bundles[3].atomic_swaps.push(s4.clone());
            bundles[0].atomic_swaps.push(s4);
            bundles[3].atomic_swaps.push(s5.clone());
            bundles[1].atomic_swaps.push(s5);
            bundles[3].converter_runs.push(ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            });
        }

        if t == 16 {
            let transfer_ref3 = AtomicSwapProposal {
                swap_id: 1601,
                maker: 3,
                taker: 2,
                maker_asset: AssetId::Refined(3),
                maker_amount: 1_000,
                taker_asset: AssetId::Cred,
                taker_amount: 100,
                domain_id: Some(0),
            };
            bundles[3].atomic_swaps.push(transfer_ref3.clone());
            bundles[2].atomic_swaps.push(transfer_ref3);
            bundles[2].project_deliveries.push(ProjectDeliveryOrder {
                project_id: 3, // inputs: (2, 1_000), (3, 1_000); base equity = 4_800 + 20% = 5_760
                batches: 1,
            });
        }

        let raw0_before_t6 = engine.state().seats[0].raw[0];
        let raw3_before_t21 = engine.state().seats[3].raw[3];
        let proj2_before_t16 = engine.state().seats[2].project_equity;
        commit_and_reveal_all(&mut engine, tid, t, bundles);
        let receipt = must_ok(
            engine.sweep_turn(derive_turn_beacon(0, tid, Turn(t))),
            "sweep",
        );

        if t == 2 {
            // Resting Raw(0) Sell offer (id 1) of 1_000 shrinks by 20% in Step 6 spoilage -> 800 remaining.
            assert_eq!(engine.state().offers[&1].remaining_units, 800);
        }
        if t == 3 {
            // Bond #2 completed via delivery, Pledge #3 broke due to escrow 1 expiry.
            assert!(receipt.escrows_finished.contains(&2));
            assert!(receipt.escrows_cancelled_or_forfeited.contains(&1));
            assert_eq!(receipt.broken_promises.len(), 1);
            assert_eq!(receipt.broken_promises[0].pledge_id, 3);
        }
        if t == 4 {
            assert_eq!(receipt.elected_officers, Some((2, 0)));
            // Proposed 9_000 raw tariff was clamped down to MAX_TARIFF_BPS (3_500).
            assert_eq!(engine.state().seats[0].raw_tariff_bps, MAX_TARIFF_BPS);
            assert!(!engine.state().domains[0].members.contains(&3));
            assert_eq!(receipt.broken_promises.len(), 2);
        }
        if t == 6 {
            assert_eq!(receipt.charters_awarded.len(), 3);
            // SupplyFreeze (target_seat=0, magnitude=2_500) realized on Turn 5 -> Turn 6 mints 7_500 instead of 10_000.
            let expected_after_decay =
                (raw0_before_t6 + 7_500) - (((raw0_before_t6 + 7_500) * 2_000) / 10_000);
            if engine.state().seats[0].raw[0] == expected_after_decay {
                saw_supply_freeze_reduction = true;
            }
        }
        if t == 16 && engine.state().seats[2].project_equity - proj2_before_t16 == 5_760 {
            saw_demand_surge_bonus = true;
        }
        if t == 21 {
            // SpoilageSpike (target_seat=3, magnitude=2_000) -> 20% + ceil(80%*20%) = 36% decay (leaving 64%).
            let expected_raw3 =
                (raw3_before_t21 + 10_000) - (((raw3_before_t21 + 10_000) * 3_600) / 10_000);
            if engine.state().seats[3].raw[3] == expected_raw3 {
                saw_spoilage_spike = true;
            }
        }
    }

    assert!(saw_supply_freeze_reduction);
    assert!(saw_tariff_holiday_zero);
    assert!(saw_demand_surge_bonus);
    assert!(saw_spoilage_spike);
    assert_eq!(engine.state().seats[0].broken_promises, 3);
    assert_eq!(engine.state().seats[0].honor_penalties, 1_500);
}

#[test]
fn standing_regent_synthesizes_checks_and_converters_when_seat_skips_reveal() {
    let tid = TableId(830);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    spec.seats[0].regent = Some(RegentDoctrine {
        risk_tolerance_bps: 8_000,
        min_raw_reserve: 500,
        auto_cash_due_claims: true,
    });
    let mut engine = must_ok(TurnEngine::new(spec), "new");

    // Seat 1 draws a check to Seat 0 due on Turn 2; deliver raw 1 & 2 to Seat 0 on Turn 2.
    must_ok(
        engine.apply_market_action(MarketAction::CreateCheck {
            seat: 1,
            check: CheckProposal {
                payee: 0,
                asset: AssetId::Cred,
                send_max: 500,
                deliver_min: 400,
                due_turn: 2,
                expiration_turn: 5,
            },
        }),
        "check to seat 0",
    );
    commit_and_reveal_all(&mut engine, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(
        engine.sweep_turn(derive_turn_beacon(1, tid, Turn(1))),
        "sweep 1",
    );

    // On Turn 2, Seats 1 and 2 sell Raw(1) & Raw(2) to Seat 0 via DEX during TalkAndMarket,
    // then Seat 0 skips commit/reveal on Turn 2 -> Regent auto-cashes Check 1 and runs Converter!
    for &(seller, good) in &[(1_u8, AssetId::Raw(1)), (2_u8, AssetId::Raw(2))] {
        must_ok(
            engine.apply_market_action(MarketAction::PostOffer {
                seat: seller,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good,
                    amount: 3_000,
                    price_per_unit_cred: 1_000,
                    domain_id: Some(0),
                    is_hybrid: false,
                },
            }),
            "sell raw",
        );
        must_ok(
            engine.apply_market_action(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good,
                    amount: 3_000,
                    price_per_unit_cred: 1_000,
                    domain_id: Some(0),
                    is_hybrid: false,
                },
            }),
            "buy raw",
        );
    }

    must_ok(engine.advance_to_commit(), "commit");
    // Only seats 1..4 submit reveals on Turn 2.
    for s in 1..4_u8 {
        let r = OrderReveal {
            bundle: SealedOrderBundle::default(),
            salt: RevealSalt([s; 32]),
        };
        let ch = must_ok(
            compute_order_commitment(tid, Turn(2), Seat(s), &r),
            "commit s",
        );
        must_ok(engine.submit_order_commitment(Seat(s), ch), "sub");
    }
    must_ok(engine.advance_to_reveal(), "reveal");
    for s in 1..4_u8 {
        let r = OrderReveal {
            bundle: SealedOrderBundle::default(),
            salt: RevealSalt([s; 32]),
        };
        must_ok(engine.submit_order_reveal(Seat(s), r), "rev s");
    }
    let r2 = must_ok(
        engine.sweep_turn(derive_turn_beacon(1, tid, Turn(2))),
        "sweep 2",
    );
    assert!(r2.regent_acted_seats.contains(&0));
    assert!(r2.checks_cashed.contains(&1));
    assert!(r2.converter_batches_run >= 1);
    assert!(engine.state().seats[0].refined[0] > 0);
}

#[test]
fn pbrs_zero_surplus_anti_wash_invariant_holds_across_dex_and_atomic_swaps() {
    // 1) Open book with positive export tariffs: circular DEX wash yields strictly negative net equity & shaped sum.
    let tid = TableId(840);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 3), "spec");
    spec.seats[0].initial_cred = 50_000;
    spec.seats[1].initial_cred = 50_000;
    let mut engine = must_ok(TurnEngine::new(spec), "new");
    commit_and_reveal_all(&mut engine, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(
        engine.sweep_turn(derive_turn_beacon(3, tid, Turn(1))),
        "sweep 1",
    );

    let pot_before_0 = compute_potential(engine.state(), 0);
    let pot_before_1 = compute_potential(engine.state(), 1);
    let sum_before = pot_before_0 + pot_before_1;

    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 2_000,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "wash sell",
    );
    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: 2_000,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "wash buy",
    );
    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 1_800,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "wash sell back",
    );
    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: 1_800,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "wash buy back",
    );

    let pot_after_0 = compute_potential(engine.state(), 0);
    let pot_after_1 = compute_potential(engine.state(), 1);
    let sum_after = pot_after_0 + pot_after_1;
    assert!(sum_after < sum_before);

    let f_undiscounted = compute_pbrs_reward(pot_before_0, pot_after_0, 10_000)
        + compute_pbrs_reward(pot_before_1, pot_after_1, 10_000);
    let f_discounted = compute_pbrs_reward(pot_before_0, pot_after_0, DEFAULT_PBRS_GAMMA_BPS)
        + compute_pbrs_reward(pot_before_1, pot_after_1, DEFAULT_PBRS_GAMMA_BPS);
    assert!(f_undiscounted < 0);
    assert!(
        f_discounted
            < compute_pbrs_reward(pot_before_0, pot_before_0, DEFAULT_PBRS_GAMMA_BPS)
                + compute_pbrs_reward(pot_before_1, pot_before_1, DEFAULT_PBRS_GAMMA_BPS)
    );

    // 2) Inside zero-tariff cartel Domain 0: circular AtomicSwap creates zero equity surplus (sum <= 0).
    let mut clean_eng = must_ok(
        TurnEngine::new(must_ok(
            TableSpec::standard_chiral_ring(TableId(841), 4, 3),
            "s",
        )),
        "clean",
    );
    commit_and_reveal_all(
        &mut clean_eng,
        TableId(841),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(
        clean_eng.sweep_turn(derive_turn_beacon(3, TableId(841), Turn(1))),
        "c1",
    );
    let mut wash_eng = clean_eng.clone();
    commit_and_reveal_all(
        &mut clean_eng,
        TableId(841),
        2,
        vec![SealedOrderBundle::default(); 4],
    );
    let clean_r2 = must_ok(
        clean_eng.sweep_turn(derive_turn_beacon(3, TableId(841), Turn(2))),
        "c2",
    );

    let mut b = vec![SealedOrderBundle::default(); 4];
    let swap_out = AtomicSwapProposal {
        swap_id: 77,
        maker: 0,
        taker: 1,
        maker_asset: AssetId::Cred,
        maker_amount: 1_000,
        taker_asset: AssetId::Raw(1),
        taker_amount: 1_000,
        domain_id: Some(0),
    };
    let swap_back = AtomicSwapProposal {
        swap_id: 78,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Cred,
        maker_amount: 1_000,
        taker_asset: AssetId::Raw(1),
        taker_amount: 1_000,
        domain_id: Some(0),
    };
    b[0].atomic_swaps = vec![swap_out.clone(), swap_back.clone()];
    b[1].atomic_swaps = vec![swap_out, swap_back];
    commit_and_reveal_all(&mut wash_eng, TableId(841), 2, b);
    let wash_r2 = must_ok(
        wash_eng.sweep_turn(derive_turn_beacon(3, TableId(841), Turn(2))),
        "w2",
    );
    assert_eq!(wash_r2.atomic_swaps_settled, 2);
    assert!(
        wash_r2.pbrs_deltas[0] + wash_r2.pbrs_deltas[1]
            <= clean_r2.pbrs_deltas[0] + clean_r2.pbrs_deltas[1]
    );
    const {
        assert!(BPS_SCALE == 10_000 && UNIT_SCALE == 1_000);
    }
}

#[test]
fn validation_error_paths_fixed_point_helpers_and_debug_redaction_are_sound() {
    // Fixed-point math edge cases
    assert_eq!(ceil_bps_fee(0, 500), 0);
    assert_eq!(ceil_bps_fee(1, 1), 1);
    assert_eq!(floor_bps_part(1_000, 10_000), 1_000);
    assert_eq!(floor_bps_part(-5, 500), 0);
    assert_eq!(half_up_bps(1_000, 505), 51);
    assert_eq!(mul_units_parity(0, 2_000), 0);
    assert_eq!(ceil_units_parity(1, 1), 1);
    assert_eq!(scale_bps_signed(-2_000, 9_500), -1_900);

    // Redaction & commitment Display
    let salt = RevealSalt([0xAB; 32]);
    assert!(format!("{salt:?}").contains("<hidden>"));
    assert!(!format!("{salt:?}").contains("ab"));
    let ch = CommitmentHash([0x01; 32]);
    assert_eq!(format!("{ch}").len(), 64);
    assert!(format!("{ch:?}").starts_with("CommitmentHash("));

    // Config validation failures
    let bad_n = TableSpec::standard_chiral_ring(TableId(1), 3, 1);
    assert!(matches!(bad_n, Err(EngineError::InvalidConfig(_))));
    let mut spec = must_ok(TableSpec::standard_chiral_ring(TableId(2), 4, 1), "valid");
    spec.seats.pop();
    assert!(matches!(
        spec.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut spec2 = must_ok(TableSpec::standard_chiral_ring(TableId(3), 4, 1), "valid");
    spec2.projects.clear();
    assert!(matches!(
        spec2.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut spec3 = must_ok(TableSpec::standard_chiral_ring(TableId(4), 4, 1), "valid");
    spec3.shocks[0].realize_turn = 24;
    assert!(matches!(
        spec3.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut spec4 = must_ok(TableSpec::standard_chiral_ring(TableId(5), 4, 1), "valid");
    spec4.seats[0].initial_raw_tariff_bps = 4_000;
    assert!(matches!(
        spec4.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    // Engine runtime error paths + Display coverage
    let valid_spec = must_ok(TableSpec::standard_chiral_ring(TableId(6), 4, 1), "valid");
    let mut eng = must_ok(TurnEngine::new(valid_spec), "new");
    assert!(matches!(
        eng.private_hints_for_auditor(Seat(9)),
        Err(EngineError::InvalidSeat(9))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelOffer {
            seat: 0,
            offer_id: 999
        }),
        Err(EngineError::UnknownObject(_))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Cred,
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: None,
                is_hybrid: false,
            }
        }),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: None,
                is_hybrid: true,
            }
        }),
        Err(EngineError::Unauthorized(_))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: 100_000,
                price_per_unit_cred: 5_000,
                domain_id: None,
                is_hybrid: false,
            }
        }),
        Err(EngineError::InsufficientBalance(_))
    ));
    assert!(matches!(
        eng.sweep_turn([0; 32]),
        Err(EngineError::InvalidPhase(_))
    ));
    assert!(matches!(
        eng.finalize_season(),
        Err(EngineError::InvalidPhase(_))
    ));
    assert!(matches!(
        eng.submit_order_commitment(Seat(0), ch),
        Err(EngineError::InvalidPhase(_))
    ));

    must_ok(eng.advance_to_commit(), "commit");
    must_ok(eng.submit_order_commitment(Seat(0), ch), "sub0");
    assert!(matches!(
        eng.submit_order_commitment(Seat(0), ch),
        Err(EngineError::AlreadySubmitted(0))
    ));
    must_ok(eng.advance_to_reveal(), "reveal");
    let bad_rev = OrderReveal {
        bundle: SealedOrderBundle::default(),
        salt: RevealSalt([0xEE; 32]),
    };
    assert!(matches!(
        eng.submit_order_reveal(Seat(0), bad_rev),
        Err(EngineError::CommitmentMismatch(0))
    ));

    // Verify Display formatting on every EngineError variant
    let all_errs = [
        EngineError::InvalidConfig("c".into()),
        EngineError::InvalidPhase("p".into()),
        EngineError::InvalidSeat(4),
        EngineError::InvalidOrder("o".into()),
        EngineError::UnknownObject("u".into()),
        EngineError::Unauthorized("a".into()),
        EngineError::PerishableEscrowForbidden,
        EngineError::InsufficientBalance("b".into()),
        EngineError::AlreadySubmitted(1),
        EngineError::CommitmentMismatch(2),
        EngineError::DomainAccessDenied("d".into()),
        EngineError::SeasonEnded("s".into()),
    ];
    for e in all_errs {
        assert!(!e.to_string().is_empty());
    }

    // Touch ActiveCharter & ConverterRecipe & ProjectRecipe & EngineState imports cleanly
    let _st: &EngineState = eng.state();
    let _ch_obj = ActiveCharter {
        role: CharterRole::OracleAuditor,
        holder: 0,
        expires_at_turn: 12,
    };
    let _cr = ConverterRecipe {
        owner: 0,
        inputs: vec![(1, 1_000)],
        output_good: 0,
        output_amount: 1_000,
        max_runs_per_turn: 1,
        is_secondary: false,
    };
    let _pr = ProjectRecipe {
        id: 1,
        inputs: vec![(0, 1_000)],
        equity_points: 1_000,
    };
}

#[test]
fn additional_branch_and_turn_28_hard_close_coverage() {
    // 1. AssetId::is_perishable + hex32 deserialization errors on CommitmentHash.
    assert!(AssetId::Raw(0).is_perishable());
    assert!(!AssetId::Refined(0).is_perishable());
    assert!(!AssetId::Cred.is_perishable());
    assert!(serde_json::from_str::<CommitmentHash>("\"not-hex\"").is_err());
    assert!(serde_json::from_str::<CommitmentHash>("\"deadbeef\"").is_err());

    // 2. TableSpec::validate() remaining error branches + secondary converter in FundamentalParities.
    let base_tid = TableId(990);
    let valid = must_ok(TableSpec::standard_chiral_ring(base_tid, 4, 2), "base");

    let mut s_idx_bad = valid.clone();
    s_idx_bad.seats[1].seat = 2;
    assert!(matches!(
        s_idx_bad.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_yield_bad = valid.clone();
    s_yield_bad.seats[0].raw_yield_per_turn = 0;
    assert!(matches!(
        s_yield_bad.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_upkeep_bad = valid.clone();
    s_upkeep_bad.seats[0].upkeep_good = 0;
    assert!(matches!(
        s_upkeep_bad.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_empty_conv = valid.clone();
    s_empty_conv.seats[0].converters.clear();
    assert!(matches!(
        s_empty_conv.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_conv_owner = valid.clone();
    s_conv_owner.seats[0].converters[0].owner = 1;
    assert!(matches!(
        s_conv_owner.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_conv_input = valid.clone();
    s_conv_input.seats[0].converters[0].inputs.push((9, 100));
    assert!(matches!(
        s_conv_input.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_dup_proj = valid.clone();
    let dup = s_dup_proj.projects[0].clone();
    s_dup_proj.projects.push(dup);
    assert!(matches!(
        s_dup_proj.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut s_bad_hz = valid.clone();
    s_bad_hz.closing_hazard_bps[0] = 10_001;
    assert!(matches!(
        s_bad_hz.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut with_secondary = valid.clone();
    with_secondary.seats[0].converters.push(ConverterRecipe {
        owner: 0,
        inputs: vec![(3, 4_000)],
        output_good: 0,
        output_amount: 1_000,
        max_runs_per_turn: 2,
        is_secondary: true,
    });
    must_ok(with_secondary.validate(), "secondary valid");
    let p_primary = FundamentalParities::from_spec(&valid);
    let p_with_sec = FundamentalParities::from_spec(&with_secondary);
    assert_eq!(p_with_sec, p_primary);

    // 3. CancelExpiredEscrow, CancelCheck, RecordIntentPledge, PostOffer domain 99,
    //    resting Refined OfferSide::Sell in equity, and Turn-28 hard close + mandate reveal errors.
    let mut hard_close_spec = valid;
    hard_close_spec.closing_hazard_bps = [0, 0, 0, 0];
    let mut eng = must_ok(TurnEngine::new(hard_close_spec), "hard_close_eng");

    // Seat 0 submits a Turn-1 Private Mandate commitment; Seat 1 does NOT.
    let rev0 = MandateReveal {
        mandate: PrivateMandate {
            kind: MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            bonus_equity: 2_000,
        },
        salt: RevealSalt([0x11; 32]),
    };
    let cm0 = must_ok(compute_mandate_commitment(base_tid, Seat(0), &rev0), "cm0");
    must_ok(eng.submit_mandate_commitment(Seat(0), cm0), "sub cm0");

    // Unknown domain_id 99
    assert!(matches!(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: Some(99),
                is_hybrid: false,
            },
        }),
        Err(EngineError::UnknownObject(_))
    ));

    // Check error paths: Create valid check 1, then Unauthorized / UnknownObject / already Cancelled.
    must_ok(
        eng.apply_market_action(MarketAction::CreateCheck {
            seat: 0,
            check: CheckProposal {
                payee: 1,
                asset: AssetId::Cred,
                send_max: 200,
                deliver_min: 100,
                due_turn: 1,
                expiration_turn: 4,
            },
        }),
        "chk1",
    );
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelCheck {
            seat: 0,
            check_id: 999,
        }),
        Err(EngineError::UnknownObject(_))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelCheck {
            seat: 1,
            check_id: 1,
        }),
        Err(EngineError::Unauthorized(_))
    ));
    must_ok(
        eng.apply_market_action(MarketAction::CancelCheck {
            seat: 0,
            check_id: 1,
        }),
        "cancel chk1",
    );
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelCheck {
            seat: 0,
            check_id: 1,
        }),
        Err(EngineError::InvalidOrder(_))
    ));

    // IntentPledge duplicate ID & bad promisor
    let pledge = IntentPledge {
        pledge_id: 42,
        promisor: 0,
        counterparty: 1,
        target: IntentTarget::VoteForSeat {
            office_turn: 4,
            candidate: 1,
        },
        honor_weight: 100,
        memo_digest: [0x22; 32],
    };
    assert!(matches!(
        eng.apply_market_action(MarketAction::RecordIntentPledge {
            seat: 1,
            pledge: pledge.clone(),
        }),
        Err(EngineError::InvalidOrder(_))
    ));
    must_ok(
        eng.apply_market_action(MarketAction::RecordIntentPledge {
            seat: 0,
            pledge: pledge.clone(),
        }),
        "record pledge 42",
    );
    assert!(matches!(
        eng.apply_market_action(MarketAction::RecordIntentPledge { seat: 0, pledge }),
        Err(EngineError::InvalidOrder(_))
    ));

    // Escrow 1 for CancelExpiredEscrow tests (cancel_after_turn = 2)
    must_ok(
        eng.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: 500,
                finish_after_turn: Some(1),
                cancel_after_turn: 2,
                condition_hash: None,
                bond: None,
            },
        }),
        "create escrow 1",
    );
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 999,
        }),
        Err(EngineError::UnknownObject(_))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelExpiredEscrow {
            seat: 2,
            escrow_id: 1,
        }),
        Err(EngineError::Unauthorized(_))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 1,
        }),
        Err(EngineError::InvalidPhase(_))
    ));

    // Step through Turns 1..=28 with zero closing hazards; mint Refined(0) on Turn 2 and
    // leave a resting OfferSide::Sell of AssetId::Refined(0) to exercise equity valuation.
    for t in 1..=28_u8 {
        if t == 2 {
            // Re-lock escrow 2 (cancel_after_turn = 3) so Turn 3 TalkAndMarket can cancel & refund it cleanly.
            must_ok(
                eng.apply_market_action(MarketAction::CreateEscrow {
                    seat: 0,
                    escrow: EscrowProposal {
                        destination: 1,
                        asset: AssetId::Cred,
                        amount: 300,
                        finish_after_turn: Some(2),
                        cancel_after_turn: 3,
                        condition_hash: None,
                        bond: None,
                    },
                }),
                "create escrow 2",
            );
        }
        if t == 3 {
            let cred_before_refund = eng.state().seats[0].cred;
            must_ok(
                eng.apply_market_action(MarketAction::CancelExpiredEscrow {
                    seat: 1,
                    escrow_id: 2,
                }),
                "refund expired escrow 2",
            );
            assert_eq!(eng.state().seats[0].cred, cred_before_refund + 300);
        }

        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 2 {
            // Transfer Raw(1) & Raw(2) to Seat 0 so Seat 0 runs converter -> mints Refined(0).
            let s_a = AtomicSwapProposal {
                swap_id: 2801,
                maker: 1,
                taker: 0,
                maker_asset: AssetId::Raw(1),
                maker_amount: 2_000,
                taker_asset: AssetId::Cred,
                taker_amount: 100,
                domain_id: Some(0),
            };
            let s_b = AtomicSwapProposal {
                swap_id: 2802,
                maker: 2,
                taker: 0,
                maker_asset: AssetId::Raw(2),
                maker_amount: 2_000,
                taker_asset: AssetId::Cred,
                taker_amount: 100,
                domain_id: Some(0),
            };
            bundles[0].atomic_swaps = vec![s_a.clone(), s_b.clone()];
            bundles[1].atomic_swaps = vec![s_a];
            bundles[2].atomic_swaps = vec![s_b];
            bundles[0].converter_runs.push(ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            });
        }
        if t == 3 {
            // Post resting Refined(0) Sell offer so compute_net_asset_equity values locked refined inventory.
            must_ok(
                eng.apply_market_action(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Refined(0),
                        amount: 1_000,
                        price_per_unit_cred: 9_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }),
                "sell refined 0",
            );
            let eq_with_resting_refined = compute_net_asset_equity(eng.state(), 0);
            assert!(eq_with_resting_refined > 0);
        }

        commit_and_reveal_all(&mut eng, base_tid, t, bundles);
        let rec = must_ok(
            eng.sweep_turn(derive_turn_beacon(2, base_tid, Turn(t))),
            "sweep hard close",
        );
        if t < 28 {
            assert!(!rec.season_closed);
        } else {
            assert!(rec.season_closed);
            assert!(eng.state().last_closing_roll_bps.is_none());
        }
    }

    assert_eq!(eng.state().turn, 28);
    assert_eq!(eng.state().phase, TurnPhase::AwaitingMandateReveal);

    // Mandate reveal error branches
    assert!(matches!(
        eng.submit_mandate_reveal(Seat(1), rev0.clone()),
        Err(EngineError::UnknownObject(_))
    ));
    let wrong_salt = MandateReveal {
        salt: RevealSalt([0x99; 32]),
        ..rev0.clone()
    };
    assert!(matches!(
        eng.submit_mandate_reveal(Seat(0), wrong_salt),
        Err(EngineError::CommitmentMismatch(0))
    ));
    assert!(matches!(
        compute_final_scoreboard(eng.state(), &[]),
        Err(EngineError::SeasonEnded(_))
    ));

    must_ok(eng.submit_mandate_reveal(Seat(0), rev0), "reveal 0");
    let board = must_ok(eng.finalize_season(), "finalize");
    assert_eq!(board.closed_turn, 28);
}
