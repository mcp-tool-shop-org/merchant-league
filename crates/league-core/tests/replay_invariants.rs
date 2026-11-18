//! Integration tests verifying v2 simultaneous turn engine mechanics,
//! dual-track settlement, 4-turn council and 6-turn charter politics,
//! 3-stage Horizon Shocks, PBRS anti-wash invariants, error contracts,
//! and bit-identical deterministic Steward replay from JSON logs.

use std::collections::BTreeMap;

use league_core::{
    ActiveCharter, AssetId, AtomicSwapProposal, BPS_SCALE, BondProposal, BondSettlementMode,
    BundlePreview, CLOSING_ROLL_WINDOW_LEN, CharterBidOrder, CharterRole, CheckOrderPreview,
    CheckPreviewOutcome, CheckProposal, CheckStatus, CommitmentHash, ContractDeliveryOrder,
    ConverterOrderPreview, ConverterRecipe, ConverterRunOrder, CouncilPlatform, CouncilVoteBallot,
    CreditTier, DEFAULT_CLOSING_HAZARD_BPS, DEFAULT_PBRS_GAMMA_BPS,
    DEFAULT_TOWN_IMPORT_PREMIUM_BPS, DEFAULT_WORKS_STIPEND_PER_UNIT_CRED,
    DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_PERIOD_SECS, DRAND_QUICKNET_SIGNATURE_BYTES,
    DrandBeaconAuthStatus, DrandSchedule, DualEscrowProposal, EngineError, EngineState,
    EscrowFinishUnlock, EscrowProposal, EscrowStatus, FillKind, FinalScoreboard,
    FundamentalParities, GradedMandateCard, HARD_TURN_CAP, IDLE_MISS_TAKEOVER_TURNS, IntentPledge,
    IntentTarget, LoggedTurn, MAX_CARTEL_DOMAINS, MAX_MANDATE_BONUS_EQUITY, MAX_TARIFF_BPS,
    MIN_MANDATE_BONUS_EQUITY, MandateKind, MandateReveal, MarketAction, OfferProposal, OfferSide,
    OrderReveal, PreimageSecret, PrivateMandate, ProjectDeliveryOrder, ProjectOrderPreview,
    ProjectRecipe, RegentDoctrine, ReplayBeaconMode, ReplayLayoutSeed, ReplayLayoutSeedError,
    ReplayRecord, RevealSalt, SEALED_LAYOUT_SEED_MIN, SEALED_SEED_POOL_NAME, ScheduledShockSpec,
    SealedEvalSeed, SealedOrderBundle, SeasonReplayLog, Seat, SeatIdentity, ShockKind,
    SwapPreviewOutcome, TableId, TableMode, TableSpec, Turn, TurnEngine, TurnPhase, UNIT_SCALE,
    Units, attach_state_predicates, bonus_equity_for_completion_bps, break_tie_with_beacon,
    break_tie_with_turn_beacon, ceil_bps_fee, ceil_units_parity, checked_ceil_units_parity,
    checked_mul_units_parity, compute_final_scoreboard, compute_mandate_commitment,
    compute_net_asset_equity, compute_order_commitment, compute_pbrs_reward, compute_potential,
    compute_preimage_condition, convert_mandate_points_to_equity, deal_graded_mandates,
    deal_graded_mandates_with_entropy, default_graded_mandate_deck, derive_drand_beacon,
    derive_turn_beacon, drand_quicknet_round_for_close, drand_quicknet_signature_status,
    evaluate_mandate_bonus, floor_bps_part, half_up_bps, layout_seed_digest, mul_units_parity,
    replay_season, replay_season_with_state_predicates, scale_bps_signed, town_import_cost_cred,
    verify_drand_quicknet_beacon, verify_order_reveal, verify_preimage_condition,
    verify_sealed_mandate_deal, width_independent_index,
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
    // Fixed fixture entropy per (seat_count, seed), so the season is the same on every run.
    let tag = seat_count.wrapping_mul(16).wrapping_add(seed as u8);
    let salts: Vec<RevealSalt> = (0..seat_count)
        .map(|s| RevealSalt([tag ^ (0x40 + s); 32]))
        .collect();
    let fixture_eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes([tag ^ 0xA7; 32]),
        "fixture eval seed",
    );
    let (spec, sealed_deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            table_id,
            seat_count,
            seed,
            [tag ^ 0x5D; 32],
            &salts,
            fixture_eval_seed,
        ),
        "standard_eval_chiral_ring_with_explicit_setup",
    );
    let eval_pk = spec.eval_vrf_pubkey;
    let n = usize::from(seat_count);
    let mut engine = must_ok(TurnEngine::new(spec.clone()), "TurnEngine::new");

    // Submit the table's pre-dealt graded Turn-1 mandate commitments (`3–8 VP`).
    let mut mandate_commits = vec![None; n];
    let mut mandate_reveals = vec![None; n];
    for s in 0..seat_count {
        let idx = usize::from(s);
        let commit = spec.dealt_mandate_commits[idx];
        must_ok(
            engine.submit_mandate_commitment(Seat(s), commit),
            "submit_mandate_commitment",
        );
        mandate_commits[idx] = Some(commit);
        mandate_reveals[idx] = Some(sealed_deal.seat_reveals[idx].clone());
    }

    let mut turns = Vec::new();
    let mut digests = Vec::new();
    let mut tracked_escrow_secret: Option<(u64, u8, PreimageSecret)> = None;
    let mut tracked_check_id: Option<(u64, u8, u8)> = None;

    for t in 1..=HARD_TURN_CAP {
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
                let trade_domain = if t >= 5 { Some(0) } else { None };
                let buy_act = MarketAction::PostOffer {
                    seat: buyer,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(seller),
                        amount: 2_000,
                        price_per_unit_cred: 1_400,
                        domain_id: trade_domain,
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
                        domain_id: trade_domain,
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
            let cond = must_ok(compute_preimage_condition(&secret), "preimage t2");
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
                    let swap_domain = if t >= 5 { Some(0) } else { None };
                    let swap_a = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + u64::from(s),
                        maker: s,
                        taker: next_1,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_000,
                        taker_asset: AssetId::Raw(next_1),
                        taker_amount: 2_000,
                        domain_id: swap_domain,
                    };
                    bundles[s_idx].atomic_swaps.push(swap_a.clone());
                    bundles[usize::from(next_1)].atomic_swaps.push(swap_a);
                }
                if engine.state().seats[s_idx].raw[s_idx] >= 2_000
                    && engine.state().seats[usize::from(next_2)].raw[usize::from(next_2)] >= 2_000
                    && s < next_2
                {
                    let swap_domain = if t >= 5 { Some(0) } else { None };
                    let swap_b = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + 50 + u64::from(s),
                        maker: s,
                        taker: next_2,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_000,
                        taker_asset: AssetId::Raw(next_2),
                        taker_amount: 2_000,
                        domain_id: swap_domain,
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
            for s in 0..seat_count {
                let platform = CouncilPlatform {
                    proposed_tariffs_bps: vec![(0, (700, 500))],
                    cartel_admissions: if s == 0 {
                        (0..seat_count).map(|m| (0, m)).collect()
                    } else {
                        Vec::new()
                    },
                    cartel_revocations: Vec::new(),
                };
                let stand = MarketAction::StandForOffice { seat: s, platform };
                must_ok(
                    engine.apply_market_action(stand.clone()),
                    "stand for office",
                );
                actions.push(stand);
            }
            for (s_idx, b) in bundles.iter_mut().enumerate() {
                b.council_vote = Some(CouncilVoteBallot {
                    trade_minister: ((s_idx % (n - 1)) + 1) as u8,
                    cartel_chair: 0,
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
        let (beacon, proof) = must_ok(eval_seed.prove_turn(table_id, Turn(t)), "prove_turn");
        let receipt = must_ok(engine.sweep_turn_with_eval_vrf(&proof), "sweep_turn_vrf");
        digests.push(receipt.post_state_digest);
        turns.push(LoggedTurn {
            turn: t,
            market_actions: actions,
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

    assert!((24..=usize::from(HARD_TURN_CAP)).contains(&turns.len()));
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
        mode: TableMode::Eval,
        seed: must_ok(ReplayLayoutSeed::try_clear(seed), "clear seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk,
        sealed_mandate_deal: sealed_deal,
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

        // Verify standings sort order: strictly descending total_equity. These fixed fixtures
        // produce no exact tie, so a tie here would mean the fixtures changed; the tie path itself
        // (closing-beacon `standings_tie_key`, then seat) is pinned by
        // `b7_final_standings_ties_break_by_beacon_not_seat_index` in
        // `core_card2_randomness_scoring.rs`.
        for pair in out_a.scoreboard.standings.windows(2) {
            assert!(pair[0].total_equity > pair[1].total_equity);
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
    for due in [1_u8, 2, 5] {
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

    // Check Open liability reduces Net Asset Equity, keeping PBRS accounting honest.
    let eq_with_open_checks = compute_net_asset_equity(engine.state(), 0);
    assert!(eq_with_open_checks < 500);

    // Bounce check 1 -> Bbb, check 2 -> B, found Domain 0 on Turn 4, post Domain-0 offer on Turn 5,
    // then bounce check 3 -> D & cartel eviction!
    for t in 1..=5_u8 {
        if t == 4 {
            must_ok(
                engine.apply_market_action(MarketAction::StandForOffice {
                    seat: 1,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(0, 0), (0, 1), (0, 2), (0, 3)],
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand to found domain 0",
            );
        }
        if t == 5 {
            // Seat 0 (now tier B) places a resting Domain-0 Buy offer before bouncing check 3.
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
        }
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        match t {
            1 | 2 => bundles[1].check_cashes.push(u64::from(t)),
            4 => {
                bundles[1].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 2,
                    cartel_chair: 1,
                });
            }
            5 => bundles[1].check_cashes.push(3),
            _ => {}
        }
        if t == 2 {
            bundles[0].debt_repayment_cred = 100;
        }
        commit_and_reveal_all(&mut engine, tid, t, bundles);
        let receipt = must_ok(
            engine.sweep_turn(derive_turn_beacon(5, tid, Turn(t))),
            "sweep",
        );
        match t {
            1 | 2 => assert_eq!(receipt.checks_bounced, vec![u64::from(t)]),
            5 => {
                assert_eq!(receipt.checks_bounced, vec![3]);
                assert_eq!(receipt.cartel_evictions, vec![0]);
            }
            _ => {}
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
    spec.shocks = vec![
        ScheduledShockSpec {
            hint_turn: 3,
            forecast_turn: 4,
            realize_turn: 5,
            target_seat: 0,
            kind: ShockKind::SupplyFreeze,
            magnitude_bps: 2_500,
        },
        ScheduledShockSpec {
            hint_turn: 8,
            forecast_turn: 9,
            realize_turn: 10,
            target_seat: 1,
            kind: ShockKind::TariffHoliday,
            magnitude_bps: 10_000,
        },
        ScheduledShockSpec {
            hint_turn: 13,
            forecast_turn: 14,
            realize_turn: 15,
            target_seat: 2,
            kind: ShockKind::DemandSurge,
            magnitude_bps: 2_000,
        },
        ScheduledShockSpec {
            hint_turn: 18,
            forecast_turn: 19,
            realize_turn: 20,
            target_seat: 3,
            kind: ShockKind::SpoilageSpike,
            magnitude_bps: 2_000,
        },
    ];
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
    let pre_cond = must_ok(compute_preimage_condition(&pre_secret), "pre_cond");
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
                condition_hash: Some(must_ok(
                    compute_preimage_condition(&PreimageSecret([0x22; 32])),
                    "bond cond",
                )),
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 1_000,
                    deadline_turn: 3,
                    mode: BondSettlementMode::SellerPerformanceBond,
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
            // Deliver >1_000 net Raw(1) from Seat 1 -> Seat 2 (public rail before Domain 0 is founded) to satisfy Bond #2 on Turn 3.
            let swap = AtomicSwapProposal {
                swap_id: 901,
                maker: 1,
                taker: 2,
                maker_asset: AssetId::Raw(1),
                maker_amount: 2_000,
                taker_asset: AssetId::Cred,
                taker_amount: 1_000,
                domain_id: None,
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
            // Seat 2 stands as Trade Minister proposing tariff changes; Seat 0 stands as Cartel Chair
            // founding Domain 0 ({0, 1, 2, 3}) and revoking Seat 3 -> breaks Pledge #2 too!
            must_ok(
                engine.apply_market_action(MarketAction::StandForOffice {
                    seat: 2,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (9_000, 1_200))],
                        cartel_admissions: Vec::new(),
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand tm 2",
            );
            must_ok(
                engine.apply_market_action(MarketAction::StandForOffice {
                    seat: 0,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (5_000, 4_000))],
                        cartel_admissions: vec![(0, 0), (0, 1), (0, 2), (0, 3)],
                        cartel_revocations: vec![(0, 3)],
                    },
                }),
                "stand cc 0",
            );
            // Seat 0 breaks VoteForSeat (votes TM=2 instead of 1) and CartelChair(0) revokes Seat 3!
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                cartel_chair: 0,
            });
            bundles[1].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                cartel_chair: 0,
            });
            bundles[2].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                cartel_chair: 0,
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
            let hints_1 = must_ok(
                league_core::engine_internals::private_hints_for_auditor(&engine, Seat(1)),
                "hints 1",
            );
            let hints_0 = must_ok(
                league_core::engine_internals::private_hints_for_auditor(&engine, Seat(0)),
                "hints 0",
            );
            assert_eq!(hints_1.len(), 1);
            assert_eq!(hints_1[0].kind, ShockKind::TariffHoliday);
            assert!(hints_0.is_empty());

            // Seat-safe path via SeatObservation::auditor_hints omits magnitude_bps before realize.
            let obs_1 = must_ok(engine.observe_seat(Seat(1)), "obs 1");
            let obs_0 = must_ok(engine.observe_seat(Seat(0)), "obs 0");
            assert_eq!(obs_1.auditor_hints().len(), 1);
            assert_eq!(obs_1.auditor_hints()[0].kind, ShockKind::TariffHoliday);
            assert_eq!(obs_1.auditor_hints()[0].hint_turn, hints_1[0].hint_turn);
            assert_eq!(
                obs_1.auditor_hints()[0].forecast_turn,
                hints_1[0].forecast_turn
            );
            assert_eq!(
                obs_1.auditor_hints()[0].realize_turn,
                hints_1[0].realize_turn
            );
            assert!(obs_0.auditor_hints().is_empty());

            // Readmit Seat 3 back into Domain 0 on Turn 8 via StandForOffice platform.
            must_ok(
                engine.apply_market_action(MarketAction::StandForOffice {
                    seat: 0,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(0, 3)],
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand readmit 3",
            );
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 1,
                cartel_chair: 0,
            });
        }

        if t == 10 {
            // ShockKind::TariffHoliday realizes on Turn 10 (target_seat = 1, magnitude = 10_000) -> 0% export tariff on Raw(1) outside domain!
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

        let raw0_before_t5 = engine.state().seats[0].raw[0];
        let raw3_before_t20 = engine.state().seats[3].raw[3];
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
            assert_eq!(receipt.elected_officers, Some((2, Some(0))));
            // Proposed 9_000 raw tariff was clamped down to MAX_TARIFF_BPS (3_500).
            assert_eq!(engine.state().seats[0].raw_tariff_bps, MAX_TARIFF_BPS);
            assert!(!engine.state().domains[0].members.contains(&3));
            assert_eq!(receipt.broken_promises.len(), 2);
        }
        if t == 5 {
            // SupplyFreeze (target_seat=0, magnitude=2_500, realize_turn=5) reduces Turn 5 mint to 7_500 instead of 10_000.
            let expected_after_decay =
                (raw0_before_t5 + 7_500) - (((raw0_before_t5 + 7_500) * 2_000) / 10_000);
            if engine.state().seats[0].raw[0] == expected_after_decay {
                saw_supply_freeze_reduction = true;
            }
        }
        if t == 6 {
            assert_eq!(receipt.charters_awarded.len(), 3);
        }
        if t == 16 && engine.state().seats[2].project_equity - proj2_before_t16 == 5_760 {
            saw_demand_surge_bonus = true;
        }
        if t == 20 {
            // SpoilageSpike (target_seat=3, magnitude=2_000, realize_turn=20) -> 20% + ceil(80%*20%) = 36% decay (leaving 64%).
            let expected_raw3 =
                (raw3_before_t20 + 10_000) - (((raw3_before_t20 + 10_000) * 3_600) / 10_000);
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
                    domain_id: None,
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
                    domain_id: None,
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
    for seat in &mut spec.seats {
        seat.regent = None;
    }
    let mut engine = must_ok(TurnEngine::new(spec), "new");
    commit_and_reveal_all(&mut engine, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(
        engine.sweep_turn(derive_turn_beacon(3, tid, Turn(1))),
        "sweep 1",
    );

    let mut eng_idle = engine.clone();
    commit_and_reveal_all(&mut eng_idle, tid, 2, vec![SealedOrderBundle::default(); 4]);
    must_ok(
        eng_idle.sweep_turn(derive_turn_beacon(3, tid, Turn(2))),
        "sweep 2 idle",
    );
    let pot_before_0 = compute_potential(eng_idle.state(), 0);
    let pot_before_1 = compute_potential(eng_idle.state(), 1);
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
                good: AssetId::Raw(1),
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
                good: AssetId::Raw(1),
                amount: 1_800,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "wash buy back",
    );
    commit_and_reveal_all(&mut engine, tid, 2, vec![SealedOrderBundle::default(); 4]);
    let rec_wash = must_ok(
        engine.sweep_turn(derive_turn_beacon(3, tid, Turn(2))),
        "sweep 2 wash",
    );
    assert_eq!(rec_wash.batch_trades_cleared, 2);

    let pot_after_0 = compute_potential(engine.state(), 0);
    let pot_after_1 = compute_potential(engine.state(), 1);
    let sum_after = pot_after_0 + pot_after_1;
    assert!(sum_after < sum_before);

    let f_undiscounted = compute_pbrs_reward(sum_before, sum_after, 10_000);
    let f_discounted = compute_pbrs_reward(0, sum_after, DEFAULT_PBRS_GAMMA_BPS)
        - compute_pbrs_reward(0, sum_before, DEFAULT_PBRS_GAMMA_BPS);
    assert!(f_undiscounted < 0);
    assert!(f_discounted < 0);

    // 2) Inside zero-tariff cartel Domain 0 (founded on Turn 4): circular AtomicSwap on Turn 5 creates zero equity surplus (sum <= 0).
    let mut clean_eng = must_ok(
        TurnEngine::new(must_ok(
            TableSpec::standard_chiral_ring(TableId(841), 4, 3),
            "s",
        )),
        "clean",
    );
    for t in 1..=4_u8 {
        if t == 4 {
            must_ok(
                clean_eng.apply_market_action(MarketAction::StandForOffice {
                    seat: 0,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(0, 0), (0, 1), (0, 2), (0, 3)],
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand",
            );
        }
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 4 {
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 1,
                cartel_chair: 0,
            });
        }
        commit_and_reveal_all(&mut clean_eng, TableId(841), t, bundles);
        must_ok(
            clean_eng.sweep_turn(derive_turn_beacon(3, TableId(841), Turn(t))),
            "setup turn",
        );
    }
    let mut wash_eng = clean_eng.clone();
    commit_and_reveal_all(
        &mut clean_eng,
        TableId(841),
        5,
        vec![SealedOrderBundle::default(); 4],
    );
    let clean_r5 = must_ok(
        clean_eng.sweep_turn(derive_turn_beacon(3, TableId(841), Turn(5))),
        "c5",
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
    commit_and_reveal_all(&mut wash_eng, TableId(841), 5, b);
    let wash_r5 = must_ok(
        wash_eng.sweep_turn(derive_turn_beacon(3, TableId(841), Turn(5))),
        "w5",
    );
    assert_eq!(wash_r5.atomic_swaps_settled, 2);
    assert!(
        wash_r5.pbrs_deltas[0] + wash_r5.pbrs_deltas[1]
            <= clean_r5.pbrs_deltas[0] + clean_r5.pbrs_deltas[1]
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
    spec3.shocks.push(ScheduledShockSpec {
        hint_turn: 21,
        forecast_turn: 22,
        realize_turn: 24,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_000,
    });
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
        league_core::engine_internals::private_hints_for_auditor(&eng, Seat(9)),
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
    let (valid_hc, dealt_hc) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(base_tid, 4, 2),
        "base",
    );
    let mut hard_close_spec = valid_hc;
    hard_close_spec.closing_hazard_bps = [0; CLOSING_ROLL_WINDOW_LEN];
    let mut eng = must_ok(TurnEngine::new(hard_close_spec.clone()), "hard_close_eng");

    // Seat 0 submits its dealt Turn-1 Private Mandate commitment; Seat 1 does NOT.
    let rev0 = dealt_hc.seat_reveals[0].clone();
    let cm0 = hard_close_spec.dealt_mandate_commits[0];
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

    // Step through Turns 1..=32 with zero closing hazards; mint Refined(0) on Turn 2 and
    // leave a resting OfferSide::Sell of AssetId::Refined(0) to exercise equity valuation.
    for t in 1..=HARD_TURN_CAP {
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
            // Transfer Raw(1) & Raw(2) to Seat 0 on public rail (2_500 -> 2_300 after 8% tariff >= 2_000) so Seat 0 runs converter -> mints Refined(0).
            let s_a = AtomicSwapProposal {
                swap_id: 2801,
                maker: 1,
                taker: 0,
                maker_asset: AssetId::Raw(1),
                maker_amount: 2_500,
                taker_asset: AssetId::Cred,
                taker_amount: 100,
                domain_id: None,
            };
            let s_b = AtomicSwapProposal {
                swap_id: 2802,
                maker: 2,
                taker: 0,
                maker_asset: AssetId::Raw(2),
                maker_amount: 2_500,
                taker_asset: AssetId::Cred,
                taker_amount: 100,
                domain_id: None,
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
        if t < HARD_TURN_CAP {
            assert!(!rec.season_closed);
        } else {
            assert!(rec.season_closed);
            assert!(eng.state().last_closing_roll_bps.is_none());
        }
    }

    assert_eq!(eng.state().turn, HARD_TURN_CAP);
    assert_eq!(eng.state().phase, TurnPhase::AwaitingMandateReveal);

    // Mandate reveal error branches & optional Turn-1 echo (F2): wrong seat's reveal triggers
    // CommitmentMismatch against pre-dealt commitment, while Seat 1's genuine reveal succeeds.
    assert!(matches!(
        eng.submit_mandate_reveal(Seat(1), rev0.clone()),
        Err(EngineError::CommitmentMismatch(1))
    ));
    must_ok(
        eng.submit_mandate_reveal(Seat(1), dealt_hc.seat_reveals[1].clone()),
        "F2 reveal without Turn-1 echo",
    );
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
    assert_eq!(board.closed_turn, HARD_TURN_CAP);
}

#[allow(clippy::too_many_lines)]
#[test]
fn dex_overflow_reproducer_and_cred_conservation_property() {
    // 1. Checked arithmetic helpers and clamped const fn saturation (A1).
    assert!(matches!(
        checked_mul_units_parity(-1, 1_000),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        checked_mul_units_parity(i64::MAX, 2_000),
        Err(EngineError::InvalidOrder(_))
    ));
    let opus_amt = (1_i64 << 62) - 250_000;
    assert!(matches!(
        checked_ceil_units_parity(opus_amt, 4_000),
        Err(EngineError::InvalidOrder(_))
    ));
    assert_eq!(mul_units_parity(i64::MAX, 4_000), i64::MAX);
    assert_eq!(ceil_units_parity(i64::MAX, 4_000), i64::MAX);
    assert_eq!(ceil_bps_fee(i64::MAX, 10_000), i64::MAX);
    assert_eq!(half_up_bps(i64::MAX, 10_000), i64::MAX);
    assert_eq!(scale_bps_signed(-1, 5_000), -1);

    let spec = must_ok(TableSpec::standard_chiral_ring(TableId(901), 4, 1), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "new");
    let digest_before = must_ok(eng.state_digest(), "digest");
    for side in [OfferSide::Buy, OfferSide::Sell] {
        let err = eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side,
                good: AssetId::Raw(1),
                amount: opus_amt,
                price_per_unit_cred: 4_000,
                domain_id: None,
                is_hybrid: false,
            },
        });
        assert!(matches!(err, Err(EngineError::InvalidOrder(_))));
        assert_eq!(must_ok(eng.state_digest(), "digest after"), digest_before);
    }

    // 2. Multi-turn randomized CRED conservation property across 8 seeds x 12 turns.
    fn total_cred_accounted(eng: &TurnEngine, cum_repaid: i64, cum_burned: i64) -> i64 {
        let free: i64 = eng.state().seats.iter().map(|s| s.cred).sum();
        let locked_offers: i64 = eng.state().offers.values().map(|o| o.locked_cred).sum();
        let locked_escrows: i64 = eng
            .state()
            .escrows
            .values()
            .filter(|e| e.status == league_core::EscrowStatus::Locked)
            .map(|e| e.amount_cred)
            .sum();
        let locked_dual: i64 = eng
            .state()
            .dual_escrows
            .values()
            .filter(|e| e.status == league_core::EscrowStatus::Locked)
            .map(|e| e.buyer_bond_cred + e.seller_bond_cred)
            .sum();
        free + locked_offers
            + locked_escrows
            + locked_dual
            + eng.state().pending_town_import_cred_spent
            + cum_repaid
            + cum_burned
    }

    for seed in 1..=8_u64 {
        let seat_count = 4 + (seed % 3) as u8;
        let tid = TableId(900 + seed);
        let spec = must_ok(
            TableSpec::standard_chiral_ring(tid, seat_count, seed),
            "spec",
        );
        let mut eng = must_ok(TurnEngine::new(spec), "new");
        let initial_total: i64 = eng.state().seats.iter().map(|s| s.cred).sum();
        let mut cum_repaid = 0_i64;
        let mut cum_burned = 0_i64;
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next_rng = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };

        for t in 1..=12_u8 {
            for _ in 0..8 {
                let s = (next_rng() % u64::from(seat_count)) as u8;
                let dest = (usize::from(s) + 1) % usize::from(seat_count);
                let action = match next_rng() % 5 {
                    0 => MarketAction::PostOffer {
                        seat: s,
                        order: OfferProposal {
                            side: if next_rng() & 1 == 0 {
                                OfferSide::Buy
                            } else {
                                OfferSide::Sell
                            },
                            good: AssetId::Raw((next_rng() % u64::from(seat_count)) as u8),
                            amount: 100 + (next_rng() % 1_500) as i64,
                            price_per_unit_cred: 500 + (next_rng() % 2_000) as i64,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    },
                    1 => {
                        let oid = eng.state().offers.keys().next().copied().unwrap_or(1);
                        MarketAction::CancelOffer {
                            seat: s,
                            offer_id: oid,
                        }
                    }
                    2 => MarketAction::CreateEscrow {
                        seat: s,
                        escrow: EscrowProposal {
                            destination: dest as u8,
                            asset: AssetId::Cred,
                            amount: 200 + (next_rng() % 500) as i64,
                            finish_after_turn: Some(t),
                            cancel_after_turn: t + 2,
                            condition_hash: None,
                            bond: None,
                        },
                    },
                    3 => {
                        let eid = eng.state().escrows.keys().next().copied().unwrap_or(1);
                        MarketAction::CancelExpiredEscrow {
                            seat: s,
                            escrow_id: eid,
                        }
                    }
                    _ => MarketAction::PostOffer {
                        seat: s,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(dest as u8),
                            amount: 250,
                            price_per_unit_cred: 1_200,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    },
                };
                let _ = eng.apply_market_action(action);
                assert_eq!(
                    total_cred_accounted(&eng, cum_repaid, cum_burned),
                    initial_total
                );
            }

            let mut bundles = vec![SealedOrderBundle::default(); usize::from(seat_count)];
            let locked_id = eng
                .state()
                .escrows
                .iter()
                .find(|(_, e)| {
                    e.status == league_core::EscrowStatus::Locked
                        && e.condition_hash.is_none()
                        && e.finish_after_turn.is_none_or(|fa| t >= fa)
                        && t < e.cancel_after_turn
                })
                .map(|(&id, e)| (id, e.destination));
            if let Some((eid, dest)) = locked_id {
                bundles[usize::from(dest)]
                    .escrow_finishes
                    .push(EscrowFinishUnlock {
                        escrow_id: eid,
                        preimage: None,
                    });
            }
            for s_idx in 0..usize::from(seat_count) {
                let target = ((s_idx + 1) % usize::from(seat_count)) as u8;
                let swap = AtomicSwapProposal {
                    swap_id: u64::from(t) * 10 + s_idx as u64,
                    maker: s_idx as u8,
                    taker: target,
                    maker_asset: AssetId::Cred,
                    maker_amount: 150,
                    taker_asset: AssetId::Raw(target),
                    taker_amount: 150,
                    domain_id: None,
                };
                bundles[s_idx].atomic_swaps.push(swap.clone());
                bundles[usize::from(target)].atomic_swaps.push(swap);
                bundles[s_idx].debt_repayment_cred = 200;
            }
            if t.is_multiple_of(6) {
                bundles[0].charter_bids.push(CharterBidOrder {
                    role: CharterRole::OracleAuditor,
                    bid_cred: 120,
                });
            }
            commit_and_reveal_all(&mut eng, tid, t, bundles);
            let receipt = must_ok(
                eng.sweep_turn(derive_turn_beacon(seed, tid, Turn(t))),
                "sweep",
            );
            cum_repaid += receipt.cred_debt_repaid + receipt.town_import_cred_spent;
            cum_burned += receipt.charter_cred_burned - receipt.works_stipend_cred_minted;
            assert_eq!(
                total_cred_accounted(&eng, cum_repaid, cum_burned),
                initial_total
            );
        }
    }
}

#[allow(clippy::too_many_lines)]
#[test]
fn invalid_assets_rejected_and_engine_is_panic_free_under_adversarial_inputs() {
    // 1. Exact Opus A2 & A3 reproducers + pledge & delivery_ledger JSON hardening.
    let spec = must_ok(TableSpec::standard_chiral_ring(TableId(902), 4, 3), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    assert!(matches!(
        eng.apply_market_action(MarketAction::CreateCheck {
            seat: 0,
            check: CheckProposal {
                payee: 1,
                asset: AssetId::Raw(99),
                send_max: 1_000,
                deliver_min: 1_000,
                due_turn: 1,
                expiration_turn: 2,
            },
        }),
        Err(EngineError::InvalidSeat(99))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::CreateCheck {
            seat: 0,
            check: CheckProposal {
                payee: 1,
                asset: AssetId::Refined(99),
                send_max: 1_000,
                deliver_min: 1_000,
                due_turn: 1,
                expiration_turn: 2,
            },
        }),
        Err(EngineError::InvalidSeat(99))
    ));
    assert!(matches!(
        eng.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: 1_000,
                finish_after_turn: Some(2),
                cancel_after_turn: 3,
                condition_hash: None,
                bond: Some(BondProposal {
                    good: AssetId::Raw(99),
                    required_units: 100,
                    deadline_turn: 2,
                    mode: BondSettlementMode::SellerPerformanceBond,
                }),
            },
        }),
        Err(EngineError::InvalidSeat(99))
    ));

    for bad_target in [
        IntentTarget::VoteForSeat {
            office_turn: 3,
            candidate: 1,
        },
        IntentTarget::VoteForSeat {
            office_turn: 4,
            candidate: 99,
        },
        IntentTarget::PreserveCartelMembership {
            office_turn: 5,
            domain_id: 0,
            beneficiary: 1,
        },
        IntentTarget::RevealPreimage {
            escrow_id: 0,
            condition: CommitmentHash([0x11; 32]),
        },
    ] {
        let res = eng.apply_market_action(MarketAction::RecordIntentPledge {
            seat: 0,
            pledge: IntentPledge {
                pledge_id: 77,
                promisor: 0,
                counterparty: 1,
                target: bad_target,
                honor_weight: 100,
                memo_digest: [0x22; 32],
            },
        });
        assert!(res.is_err());
    }

    // Duplicate delivery_ledger key rejected cleanly during deserialization.
    let mut val = must_ok(serde_json::to_value(eng.state()), "to_value");
    val["delivery_ledger"] = serde_json::json!([
        [[0, 1, {"Raw": 0}], 10],
        [[0, 1, {"Raw": 0}], 20]
    ]);
    assert!(serde_json::from_value::<EngineState>(val).is_err());

    // A3 reveal rejection for seat 0 while seat 1's valid reveal succeeds cleanly.
    must_ok(eng.advance_to_commit(), "commit");
    let bad_rev = OrderReveal {
        bundle: SealedOrderBundle {
            atomic_swaps: vec![AtomicSwapProposal {
                swap_id: 99,
                maker: 0,
                taker: 1,
                maker_asset: AssetId::Refined(200),
                maker_amount: 10,
                taker_asset: AssetId::Cred,
                taker_amount: 10,
                domain_id: None,
            }],
            ..SealedOrderBundle::default()
        },
        salt: RevealSalt([0x01; 32]),
    };
    let good_rev = OrderReveal {
        bundle: SealedOrderBundle::default(),
        salt: RevealSalt([0x02; 32]),
    };
    let c0 = must_ok(
        compute_order_commitment(TableId(902), Turn(1), Seat(0), &bad_rev),
        "c0",
    );
    let c1 = must_ok(
        compute_order_commitment(TableId(902), Turn(1), Seat(1), &good_rev),
        "c1",
    );
    must_ok(eng.submit_order_commitment(Seat(0), c0), "sub0");
    must_ok(eng.submit_order_commitment(Seat(1), c1), "sub1");
    must_ok(eng.advance_to_reveal(), "reveal");
    assert!(matches!(
        eng.submit_order_reveal(Seat(0), bad_rev),
        Err(EngineError::InvalidSeat(200))
    ));
    must_ok(eng.submit_order_reveal(Seat(1), good_rev), "rev1");
    must_ok(
        eng.sweep_turn(derive_turn_beacon(3, TableId(902), Turn(1))),
        "sweep",
    );

    // 2. Panic-freedom fuzz sweep via std::panic::catch_unwind (200 adversarial iterations).
    let mut fuzz_eng = must_ok(
        TurnEngine::new(must_ok(
            TableSpec::standard_chiral_ring(TableId(903), 4, 7),
            "fuzz spec",
        )),
        "fuzz eng",
    );
    let mut state_rng = 0x1234_5678_9ABC_DEF1_u64;
    let mut next_u64 = || {
        state_rng ^= state_rng << 13;
        state_rng ^= state_rng >> 7;
        state_rng ^= state_rng << 17;
        state_rng
    };
    let weird_assets = [
        AssetId::Cred,
        AssetId::Raw(0),
        AssetId::Raw(99),
        AssetId::Raw(255),
        AssetId::Refined(3),
        AssetId::Refined(200),
    ];
    let weird_amounts = [i64::MIN, -100, 0, 1, 1_000, i64::MAX / 2, i64::MAX];
    for iter in 0..200_u64 {
        let s = (next_u64() % 6) as u8; // includes invalid seats 4, 5
        let target = (next_u64() % 6) as u8;
        let asset = weird_assets[(next_u64() as usize) % weird_assets.len()];
        let amt = weird_amounts[(next_u64() as usize) % weird_amounts.len()];
        let price = weird_amounts[(next_u64() as usize) % weird_amounts.len()];
        let dom = if next_u64() % 3 == 0 {
            Some((next_u64() % 32) as u8)
        } else {
            None
        };
        let action = match iter % 5 {
            0 => MarketAction::PostOffer {
                seat: s,
                order: OfferProposal {
                    side: if next_u64() & 1 == 0 {
                        OfferSide::Buy
                    } else {
                        OfferSide::Sell
                    },
                    good: asset,
                    amount: amt,
                    price_per_unit_cred: price,
                    domain_id: dom,
                    is_hybrid: next_u64() & 1 == 0,
                },
            },
            1 => MarketAction::CreateCheck {
                seat: s,
                check: CheckProposal {
                    payee: target,
                    asset,
                    send_max: amt,
                    deliver_min: price,
                    due_turn: 1,
                    expiration_turn: 5,
                },
            },
            2 => MarketAction::CreateEscrow {
                seat: s,
                escrow: EscrowProposal {
                    destination: target,
                    asset,
                    amount: amt,
                    finish_after_turn: Some(1),
                    cancel_after_turn: 4,
                    condition_hash: None,
                    bond: Some(BondProposal {
                        good: asset,
                        required_units: amt,
                        deadline_turn: 2,
                        mode: BondSettlementMode::SellerPerformanceBond,
                    }),
                },
            },
            3 => MarketAction::RecordIntentPledge {
                seat: s,
                pledge: IntentPledge {
                    pledge_id: iter,
                    promisor: s,
                    counterparty: target,
                    target: IntentTarget::VoteForSeat {
                        office_turn: ((iter % 30) as u8),
                        candidate: target,
                    },
                    honor_weight: amt,
                    memo_digest: [0x55; 32],
                },
            },
            _ => MarketAction::StandForOffice {
                seat: s,
                platform: CouncilPlatform {
                    proposed_tariffs_bps: vec![(target, (9_000, 9_000))],
                    cartel_admissions: vec![(20, target)],
                    cartel_revocations: vec![(0, target)],
                },
            },
        };
        let no_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fuzz_eng.apply_market_action(action);
        }));
        assert!(no_panic.is_ok(), "iter {iter} panicked in Phase 1");
    }
    let no_panic_sweep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        must_ok(fuzz_eng.advance_to_commit(), "commit");
        let adv_rev = OrderReveal {
            bundle: SealedOrderBundle {
                converter_runs: vec![
                    ConverterRunOrder {
                        recipe_idx: 0,
                        runs: u32::MAX,
                    },
                    ConverterRunOrder {
                        recipe_idx: 99,
                        runs: 1,
                    },
                ],
                project_deliveries: vec![ProjectDeliveryOrder {
                    project_id: 1,
                    batches: u32::MAX,
                }],
                check_cashes: vec![999, 1],
                ..SealedOrderBundle::default()
            },
            salt: RevealSalt([0xAB; 32]),
        };
        let c = must_ok(
            compute_order_commitment(TableId(903), Turn(1), Seat(0), &adv_rev),
            "c",
        );
        must_ok(fuzz_eng.submit_order_commitment(Seat(0), c), "sub0");
        must_ok(fuzz_eng.advance_to_reveal(), "rev");
        must_ok(fuzz_eng.submit_order_reveal(Seat(0), adv_rev), "rev0");
        let _ = fuzz_eng.sweep_turn(derive_turn_beacon(7, TableId(903), Turn(1)));
    }));
    assert!(no_panic_sweep.is_ok(), "adversarial Sweep panicked");
}

#[test]
fn per_converter_turn_cap_blocks_duplicate_converter_run_orders() {
    // 1. Opus 5-copy reproducer against max_runs_per_turn = 1 -> 1 batch (3_500 output).
    let tid = TableId(904);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    spec.seats[0].converters[0].max_runs_per_turn = 1;
    spec.seats[1].raw_yield_per_turn = 20_000;
    spec.seats[2].raw_yield_per_turn = 20_000;
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    // Turn 1: empty yields so seats 1 and 2 mint 20_000 Raw(1) & Raw(2).
    commit_and_reveal_all(&mut eng, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(1))),
        "sweep 1",
    );

    // Turn 2: deliver > 10_000 Raw(1) & Raw(2) to Seat 0 via swaps, then submit 5 copies of runs: 1.
    let mut b2 = vec![SealedOrderBundle::default(); 4];
    let sw1 = AtomicSwapProposal {
        swap_id: 1,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Raw(1),
        maker_amount: 12_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    let sw2 = AtomicSwapProposal {
        swap_id: 2,
        maker: 2,
        taker: 0,
        maker_asset: AssetId::Raw(2),
        maker_amount: 12_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    b2[0].atomic_swaps = vec![sw1.clone(), sw2.clone()];
    b2[1].atomic_swaps = vec![sw1];
    b2[2].atomic_swaps = vec![sw2];
    b2[0].converter_runs = vec![
        ConverterRunOrder {
            recipe_idx: 0,
            runs: 1,
        };
        5
    ];
    commit_and_reveal_all(&mut eng, tid, 2, b2);
    let r2 = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(2))),
        "sweep 2",
    );
    assert_eq!(r2.converter_batches_run, 1);
    assert_eq!(eng.state().seats[0].refined[0], 3_500);

    // 2. Cumulative partial cap test: [2, 1, 1] against max_runs_per_turn = 3 -> 3 batches (10_500 output).
    let mut spec2 = must_ok(TableSpec::standard_chiral_ring(TableId(905), 4, 1), "spec2");
    spec2.seats[0].converters[0].max_runs_per_turn = 3;
    spec2.seats[1].raw_yield_per_turn = 20_000;
    spec2.seats[2].raw_yield_per_turn = 20_000;
    let mut eng2 = must_ok(TurnEngine::new(spec2), "new2");
    commit_and_reveal_all(
        &mut eng2,
        TableId(905),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(
        eng2.sweep_turn(derive_turn_beacon(1, TableId(905), Turn(1))),
        "s1",
    );
    let mut b_split = vec![SealedOrderBundle::default(); 4];
    let s_a = AtomicSwapProposal {
        swap_id: 1,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Raw(1),
        maker_amount: 12_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    let s_b = AtomicSwapProposal {
        swap_id: 2,
        maker: 2,
        taker: 0,
        maker_asset: AssetId::Raw(2),
        maker_amount: 12_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    b_split[0].atomic_swaps = vec![s_a.clone(), s_b.clone()];
    b_split[1].atomic_swaps = vec![s_a];
    b_split[2].atomic_swaps = vec![s_b];
    b_split[0].converter_runs = vec![
        ConverterRunOrder {
            recipe_idx: 0,
            runs: 2,
        },
        ConverterRunOrder {
            recipe_idx: 0,
            runs: 1,
        },
        ConverterRunOrder {
            recipe_idx: 0,
            runs: 1,
        },
    ];
    commit_and_reveal_all(&mut eng2, TableId(905), 2, b_split);
    let r2_split = must_ok(
        eng2.sweep_turn(derive_turn_beacon(1, TableId(905), Turn(2))),
        "s2",
    );
    assert_eq!(r2_split.converter_batches_run, 3);
    assert_eq!(eng2.state().seats[0].refined[0], 10_500);
}

#[test]
fn non_payee_check_listing_cannot_block_legitimate_payee_cashing() {
    let tid = TableId(906);
    let spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "new");
    must_ok(
        eng.apply_market_action(MarketAction::CreateCheck {
            seat: 1,
            check: CheckProposal {
                payee: 3,
                asset: AssetId::Cred,
                send_max: 500,
                deliver_min: 500,
                due_turn: 1,
                expiration_turn: 5,
            },
        }),
        "create check",
    );
    let cred3_before = eng.state().seats[3].cred;

    // Non-payee alone listing check #1 leaves it Open.
    let mut eng_attacker_only = eng.clone();
    let mut b_att = vec![SealedOrderBundle::default(); 4];
    b_att[0].check_cashes.push(1);
    commit_and_reveal_all(&mut eng_attacker_only, tid, 1, b_att);
    let r_att = must_ok(
        eng_attacker_only.sweep_turn(derive_turn_beacon(1, tid, Turn(1))),
        "sweep att",
    );
    assert!(r_att.checks_cashed.is_empty());
    assert_eq!(
        eng_attacker_only.state().checks[&1].status,
        CheckStatus::Open
    );

    // Non-payee Seat 0 + real payee Seat 3 both listing check #1 still cashes 500 cleanly!
    let mut b_both = vec![SealedOrderBundle::default(); 4];
    b_both[0].check_cashes.push(1);
    b_both[3].check_cashes.push(1);
    commit_and_reveal_all(&mut eng, tid, 1, b_both);
    let r_both = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(1))),
        "sweep both",
    );
    assert_eq!(r_both.checks_cashed, vec![1]);
    assert_eq!(eng.state().checks[&1].status, CheckStatus::Cashed(500));
    assert_eq!(eng.state().seats[3].cred, cred3_before + 500);
}

#[allow(clippy::too_many_lines)]
#[test]
fn phase1_actions_are_atomic_on_error_and_replay_handles_refused_actions() {
    // 1. Opus partial-fill rollback reproducer: Buy 3 at price 1 with initial_cred = 1 against 3 Sell 1s.
    let tid = TableId(907);
    let (mut spec, dealt_reveals) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(tid, 4, 2),
        "spec",
    );
    spec.seats[0].initial_cred = 1;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "new");

    // Advance Turn 1 so Seat 1 mints Raw(1).
    commit_and_reveal_all(&mut eng, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(eng.sweep_turn(derive_turn_beacon(2, tid, Turn(1))), "t1");

    for _ in 0..3 {
        must_ok(
            eng.apply_market_action(MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    amount: 1,
                    price_per_unit_cred: 1,
                    domain_id: None,
                    is_hybrid: false,
                },
            }),
            "maker sell 1",
        );
    }
    let digest_before = must_ok(eng.state_digest(), "digest before");
    let partial_buy = MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Buy,
            good: AssetId::Raw(1),
            amount: 1_001,
            price_per_unit_cred: 1_000,
            domain_id: None,
            is_hybrid: false,
        },
    };
    assert!(matches!(
        eng.apply_market_action(partial_buy.clone()),
        Err(EngineError::InsufficientBalance(_))
    ));
    assert_eq!(must_ok(eng.state_digest(), "digest after"), digest_before);
    assert_eq!(eng.state().seats[0].cred, 1);
    assert_eq!(eng.state().offers.len(), 3);
    assert!(eng.state().delivery_ledger.is_empty());

    // 2. Assert exact state_digest() invariance on Err across ALL 10 MarketAction variants.
    let too_many_domains = (0..=(MAX_CARTEL_DOMAINS as u8)).map(|d| (d, 0)).collect();
    let refused_variants = [
        partial_buy.clone(),
        MarketAction::CancelOffer {
            seat: 0,
            offer_id: 999,
        },
        MarketAction::CreateCheck {
            seat: 0,
            check: CheckProposal {
                payee: 0,
                asset: AssetId::Cred,
                send_max: 10,
                deliver_min: 5,
                due_turn: 2,
                expiration_turn: 3,
            },
        },
        MarketAction::CancelCheck {
            seat: 0,
            check_id: 999,
        },
        MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Raw(0),
                amount: 100,
                finish_after_turn: Some(2),
                cancel_after_turn: 4,
                condition_hash: None,
                bond: None,
            },
        },
        MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 999,
        },
        MarketAction::RecordIntentPledge {
            seat: 0,
            pledge: IntentPledge {
                pledge_id: 1,
                promisor: 1,
                counterparty: 0,
                target: IntentTarget::VoteForSeat {
                    office_turn: 4,
                    candidate: 0,
                },
                honor_weight: 50,
                memo_digest: [0x11; 32],
            },
        },
        MarketAction::FlagWashFreeze {
            inspector: 0,
            target_seat: 1,
        },
        MarketAction::StandForOffice {
            seat: 0,
            platform: CouncilPlatform {
                proposed_tariffs_bps: Vec::new(),
                cartel_admissions: too_many_domains,
                cartel_revocations: Vec::new(),
            },
        },
        MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(1),
            amount: 2_000,
        },
    ];
    for action in refused_variants {
        assert!(eng.apply_market_action(action).is_err());
        assert_eq!(must_ok(eng.state_digest(), "digest err"), digest_before);
    }

    // 3. Replay log containing the refused partial-fill action reproduces identical scoreboard & digests.
    let (spec2, dealt2, eval_seed2) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(TableId(908), 4, 2),
        "spec2",
    );
    let mut spec2 = spec2;
    let eval_pk2 = spec2.eval_vrf_pubkey;
    spec2.seats[0].initial_cred = 1;
    let mut live_eng = must_ok(TurnEngine::new(spec2.clone()), "live");
    let mc0 = spec2.dealt_mandate_commits[0];
    must_ok(live_eng.submit_mandate_commitment(Seat(0), mc0), "mc0");
    let mut logged_turns = Vec::new();
    let mut live_digests = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if live_eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut actions = Vec::new();
        if t == 2 {
            for _ in 0..3 {
                let sell = MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(1),
                        amount: 1,
                        price_per_unit_cred: 1,
                        domain_id: None,
                        is_hybrid: false,
                    },
                };
                must_ok(live_eng.apply_market_action(sell.clone()), "sell");
                actions.push(sell);
            }
            assert!(live_eng.apply_market_action(partial_buy.clone()).is_err());
            actions.push(partial_buy.clone());
        }
        let (commitments, reveals) = commit_and_reveal_all(
            &mut live_eng,
            TableId(908),
            t,
            vec![SealedOrderBundle::default(); 4],
        );
        let (beacon, proof) = must_ok(eval_seed2.prove_turn(TableId(908), Turn(t)), "prove");
        let r = must_ok(live_eng.sweep_turn_with_eval_vrf(&proof), "sweep");
        live_digests.push(r.post_state_digest);
        logged_turns.push(LoggedTurn {
            turn: t,
            market_actions: actions,
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(proof.to_vec()),
            drawn_shocks: r.newly_drawn_shocks.clone(),
        });
        if r.season_closed {
            break;
        }
    }
    must_ok(
        live_eng.submit_mandate_reveal(Seat(0), dealt2.seat_reveals[0].clone()),
        "rev0",
    );
    let live_board = must_ok(live_eng.finalize_season(), "fin");
    let mut mandate_commits = vec![None; 4];
    mandate_commits[0] = Some(mc0);
    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[0] = Some(dealt2.seat_reveals[0].clone());
    let _unused = dealt_reveals;
    let out = must_ok(
        replay_season(&SeasonReplayLog {
            spec: spec2,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(2), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk2,
            sealed_mandate_deal: dealt2,
            mandate_commits,
            turns: logged_turns,
            mandate_reveals,
        }),
        "replay refused action log",
    );
    assert_eq!(out.state_digests, live_digests);
    assert_eq!(out.scoreboard, live_board);
}

#[test]
fn cartel_domains_start_empty_chair_founds_multiple_domains_and_tier_d_broker_blocked() {
    let tid = TableId(909);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    spec.seats[0].initial_cred = 1_000;
    let mut eng = must_ok(TurnEngine::new(spec), "new");
    // T1: tables start with zero cartel domains.
    assert!(eng.state().domains.is_empty());

    // Queue 3 tiny checks on Seat 0 (400 < 500 each after spending) to drop Seat 0 to Tier D later.
    for due in 6..=8_u8 {
        must_ok(
            eng.apply_market_action(MarketAction::CreateCheck {
                seat: 0,
                check: CheckProposal {
                    payee: 1,
                    asset: AssetId::Cred,
                    send_max: 2_000,
                    deliver_min: 1_500,
                    due_turn: due,
                    expiration_turn: 12,
                },
            }),
            "check",
        );
    }

    for t in 1..=8_u8 {
        if t == 4 {
            // B4 / T1: elected Chair (Seat 2) founds Domain 0 ({0, 1}) and Domain 1 ({2, 3}).
            must_ok(
                eng.apply_market_action(MarketAction::StandForOffice {
                    seat: 2,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(0, 0), (0, 1), (1, 2), (1, 3)],
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand cc",
            );
        }
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 4 {
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 0,
                cartel_chair: 2,
            });
        }
        if t == 6 {
            bundles[0].charter_bids.push(CharterBidOrder {
                role: CharterRole::ClearinghouseBroker,
                bid_cred: 600,
            });
        }
        if (6..=8).contains(&t) {
            bundles[1].check_cashes.push(u64::from(t - 5));
        }
        commit_and_reveal_all(&mut eng, tid, t, bundles);
        must_ok(eng.sweep_turn(derive_turn_beacon(1, tid, Turn(t))), "sweep");
        if t == 4 {
            assert_eq!(eng.state().domains.len(), 2);
            assert!(eng.state().domains[0].members.contains(&0));
            assert!(eng.state().domains[0].members.contains(&1));
            assert!(eng.state().domains[1].members.contains(&2));
            assert!(eng.state().domains[1].members.contains(&3));
        }
    }

    // Seat 0 holds ClearinghouseBroker AND hit CreditTier::D (evicted from Domain 0).
    assert_eq!(eng.state().seats[0].credit_tier, CreditTier::D);
    // B5: Tier D broker CANNOT post domain-scoped offers even with is_hybrid = true.
    let err = eng.apply_market_action(MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Buy,
            good: AssetId::Raw(1),
            amount: 100,
            price_per_unit_cred: 1_000,
            domain_id: Some(0),
            is_hybrid: true,
        },
    });
    assert!(matches!(err, Err(EngineError::DomainAccessDenied(_))));

    // Also verify resting Domain-0 offer from Seat 1 does NOT cross with Tier-D broker public hybrid offer.
    must_ok(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(1),
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: Some(0),
                is_hybrid: false,
            },
        }),
        "dom 0 sell",
    );
    must_ok(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: None,
                is_hybrid: true,
            },
        }),
        "tier D public hybrid posts but does NOT cross Domain 0",
    );
    assert_eq!(eng.state().offers.len(), 2);
}

#[test]
fn council_zero_vote_keeps_incumbent_beacon_breaks_ties_and_platform_needs_no_ballot() {
    let tid = TableId(910);
    let spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    for t in 1..=11_u8 {
        if t == 3 {
            // T3: Seat 2 files StandForOffice in Turn 3 and abstains from voting in Turn 4.
            must_ok(
                eng.apply_market_action(MarketAction::StandForOffice {
                    seat: 2,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (1_200, 900))],
                        cartel_admissions: Vec::new(),
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand tm 2 on turn 3",
            );
            must_ok(
                eng.apply_market_action(MarketAction::StandForOffice {
                    seat: 3,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(0, 3)],
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand cc 3 on turn 3",
            );
        }
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 4 {
            // Seats 0 and 1 vote for TM=2, CC=3 while Seat 2 & 3 abstain -> platform still enacts (T3)!
            bundles[0].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                cartel_chair: 3,
            });
            bundles[1].council_vote = Some(CouncilVoteBallot {
                trade_minister: 2,
                cartel_chair: 3,
            });
        }
        // Turn 8 has zero ballots -> incumbents (2, 3) must be retained (T2 / B2)!
        commit_and_reveal_all(&mut eng, tid, t, bundles);
        let r = must_ok(eng.sweep_turn(derive_turn_beacon(1, tid, Turn(t))), "sweep");
        if t == 4 {
            assert_eq!(r.elected_officers, Some((2, Some(3))));
            assert_eq!(eng.state().seats[0].raw_tariff_bps, 1_200);
            assert_eq!(eng.state().seats[0].refined_tariff_bps, 900);
            assert!(eng.state().domains[0].members.contains(&3));
        }
        if t == 8 {
            assert_eq!(r.elected_officers, None);
            assert_eq!(eng.state().trade_minister, 2);
            assert_eq!(eng.state().cartel_chair, Some(3));
        }
    }

    // Turn 12: 2-2 tie between Seat 1 and incumbent Seat 3 broken by beacon (T2).
    let mut b_tie = vec![SealedOrderBundle::default(); 4];
    b_tie[0].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 1,
    });
    b_tie[1].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 1,
    });
    b_tie[2].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 3,
    });
    b_tie[3].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 3,
    });

    let mut beacon_win_1 = None;
    let mut beacon_win_3 = None;
    for s in 0..100_u64 {
        let beacon = derive_turn_beacon(s, tid, Turn(12));
        match break_tie_with_turn_beacon(beacon, tid, Turn(12), b"cartel_chair", &[1, 3]) {
            1 if beacon_win_1.is_none() => beacon_win_1 = Some(beacon),
            3 if beacon_win_3.is_none() => beacon_win_3 = Some(beacon),
            _ => {}
        }
        if beacon_win_1.is_some() && beacon_win_3.is_some() {
            break;
        }
    }
    let Some(b1) = beacon_win_1 else {
        panic!("found tie beacon selecting 1");
    };
    let Some(b3) = beacon_win_3 else {
        panic!("found tie beacon selecting 3");
    };

    let mut eng_a = eng.clone();
    commit_and_reveal_all(&mut eng_a, tid, 12, b_tie.clone());
    let r_a = must_ok(eng_a.sweep_turn(b1), "tie a");
    assert_eq!(r_a.elected_officers, Some((2, Some(1))));

    let mut eng_b = eng;
    commit_and_reveal_all(&mut eng_b, tid, 12, b_tie);
    let r_b = must_ok(eng_b.sweep_turn(b3), "tie b");
    assert_eq!(r_b.elected_officers, Some((2, Some(3))));
}

#[test]
fn committed_non_revealer_gets_default_bundle_while_non_committer_runs_all_regent_converters() {
    let tid = TableId(911);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    spec.seats[0].converters.push(ConverterRecipe {
        owner: 0,
        inputs: vec![(3, 2_000)],
        output_good: 0,
        output_amount: 1_500,
        max_runs_per_turn: 1,
        is_secondary: true,
    });
    spec.seats[0].regent = Some(RegentDoctrine {
        risk_tolerance_bps: 5_000,
        min_raw_reserve: 500,
        auto_cash_due_claims: true,
    });
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    // Turn 1 yields give Raw(1), Raw(2), Raw(3) to Seats 1, 2, 3; swap them to Seat 0 on Turn 1.
    // Wait: yields happen at Step 4 of Turn 1, so swap on Turn 2, and test miss on Turn 3!
    commit_and_reveal_all(&mut eng, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(eng.sweep_turn(derive_turn_beacon(1, tid, Turn(1))), "t1");

    let mut b2 = vec![SealedOrderBundle::default(); 4];
    for (idx, partner) in [1_u8, 2, 3].into_iter().enumerate() {
        let sw = AtomicSwapProposal {
            swap_id: (idx + 1) as u64,
            maker: partner,
            taker: 0,
            maker_asset: AssetId::Raw(partner),
            maker_amount: 4_000,
            taker_asset: AssetId::Cred,
            taker_amount: 100,
            domain_id: None,
        };
        b2[0].atomic_swaps.push(sw.clone());
        b2[usize::from(partner)].atomic_swaps.push(sw);
    }
    commit_and_reveal_all(&mut eng, tid, 2, b2);
    must_ok(eng.sweep_turn(derive_turn_beacon(1, tid, Turn(2))), "t2");

    // Clone A (T4): Seat 0 submits Phase-2 commit on Turn 3 but fails to reveal -> default bundle, NO Regent!
    let mut eng_committed = eng.clone();
    must_ok(eng_committed.advance_to_commit(), "commit a");
    let ch0 = must_ok(
        compute_order_commitment(
            tid,
            Turn(3),
            Seat(0),
            &OrderReveal {
                bundle: SealedOrderBundle {
                    converter_runs: vec![ConverterRunOrder {
                        recipe_idx: 0,
                        runs: 1,
                    }],
                    ..SealedOrderBundle::default()
                },
                salt: RevealSalt([0x33; 32]),
            },
        ),
        "ch0",
    );
    must_ok(eng_committed.submit_order_commitment(Seat(0), ch0), "sub0");
    must_ok(eng_committed.advance_to_reveal(), "rev a");
    let r_a = must_ok(
        eng_committed.sweep_turn(derive_turn_beacon(1, tid, Turn(3))),
        "sweep a",
    );
    assert!(r_a.non_reveal_seats.contains(&0));
    assert!(!r_a.regent_acted_seats.contains(&0));
    assert_eq!(r_a.converter_batches_run, 0);
    assert_eq!(eng_committed.state().seats[0].refined[0], 0);

    // Clone B (B11): Seat 0 skips Phase-2 commit too -> Regent acts AND runs BOTH converters!
    must_ok(eng.advance_to_commit(), "commit b");
    must_ok(eng.advance_to_reveal(), "rev b");
    let r_b = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(3))),
        "sweep b",
    );
    assert!(r_b.regent_acted_seats.contains(&0));
    assert_eq!(r_b.converter_batches_run, 2);
    assert_eq!(eng.state().seats[0].refined[0], 5_000);
}

#[test]
fn graded_mandate_deck_blocks_arbitrary_bonus_and_validates_foreign_converter_inputs() {
    // 1. Completion-bps -> 3..8 VP mapping and deck construction invariants (T5).
    assert_eq!(bonus_equity_for_completion_bps(0), MAX_MANDATE_BONUS_EQUITY);
    assert_eq!(
        bonus_equity_for_completion_bps(10_000),
        MIN_MANDATE_BONUS_EQUITY
    );
    let deck = default_graded_mandate_deck(4);
    assert_eq!(deck.len(), 8);
    assert!(
        deck.iter().all(
            |c| (MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY).contains(&c.bonus_equity)
        )
    );

    // 2. Opus bonus_equity = 1_000_000_000_000 exploit blocked at commit.
    let tid = TableId(912);
    let beacon_seed = 7_u64;
    let (mut spec, dealt_deal) = must_ok(
        (0..128_u32)
            .find_map(|_| {
                let (sp, deal) = must_ok(
                    TableSpec::standard_chiral_ring_with_mandates(tid, 4, beacon_seed),
                    "spec",
                );
                deal.seat_reveals
                    .iter()
                    .any(|r| matches!(r.mandate.kind, MandateKind::MaintainCleanCredit { .. }))
                    .then_some((sp, deal))
            })
            .ok_or("some OS deal draws MaintainCleanCredit from default deck"),
        "dealt_deal",
    );
    spec.closing_hazard_bps = DEFAULT_CLOSING_HAZARD_BPS;
    for sc in &mut spec.seats {
        sc.initial_cred = 500 * UNIT_SCALE;
    }
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "new");
    let exploit = MandateReveal {
        mandate: PrivateMandate {
            kind: MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            bonus_equity: 1_000_000_000_000,
        },
        salt: RevealSalt([0x77; 32]),
    };
    let exploit_hash = must_ok(
        compute_mandate_commitment(tid, Seat(0), &exploit),
        "exploit",
    );
    assert!(matches!(
        eng.submit_mandate_commitment(Seat(0), exploit_hash),
        Err(EngineError::CommitmentMismatch(0))
    ));

    // 3. Valid dealt mandate commits cleanly and awards exact deck bonus in 3_000..=8_000.
    // Find a seat holding MaintainCleanCredit so an empty season satisfies it deterministically.
    let clean_seat = must_ok(
        dealt_deal
            .seat_reveals
            .iter()
            .position(|r| matches!(r.mandate.kind, MandateKind::MaintainCleanCredit { .. }))
            .ok_or("selected deal holds MaintainCleanCredit"),
        "clean_seat",
    );
    let pts_milli = dealt_deal.seat_reveals[clean_seat].mandate.bonus_equity;
    assert!((MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY).contains(&pts_milli));
    must_ok(
        eng.submit_mandate_commitment(
            Seat(clean_seat as u8),
            spec.dealt_mandate_commits[clean_seat],
        ),
        "valid dealt commit",
    );
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let r = must_ok(
            eng.sweep_turn(derive_turn_beacon(beacon_seed, tid, Turn(t))),
            "sweep",
        );
        if r.season_closed {
            break;
        }
    }
    let expected_bonus = convert_mandate_points_to_equity(pts_milli, eng.state());
    assert!(expected_bonus > 0);
    must_ok(
        eng.submit_mandate_reveal(
            Seat(clean_seat as u8),
            dealt_deal.seat_reveals[clean_seat].clone(),
        ),
        "valid reveal",
    );
    let board = must_ok(eng.finalize_season(), "finalize");
    let clean_entry = must_ok(
        board
            .standings
            .iter()
            .find(|e| e.seat == clean_seat as u8)
            .ok_or("clean seat standing"),
        "clean_entry",
    );
    assert_eq!(clean_entry.mandate_bonus, expected_bonus);

    // 4. TableSpec::validate rejects out-of-range deck bonuses, bad deals, and non-foreign/duplicate primary inputs (B10).
    let mut bad_low = spec.clone();
    bad_low.mandate_deck[0].bonus_equity = MIN_MANDATE_BONUS_EQUITY - 1;
    assert!(matches!(
        bad_low.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    let mut bad_high = spec.clone();
    bad_high.mandate_deck[0].bonus_equity = MAX_MANDATE_BONUS_EQUITY + 1;
    assert!(matches!(
        bad_high.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    let mut bad_dup_id = spec.clone();
    bad_dup_id.mandate_deck[1].card_id = bad_dup_id.mandate_deck[0].card_id;
    assert!(matches!(
        bad_dup_id.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    let bad_deal = deal_graded_mandates(tid, 4, &spec.mandate_deck[0..3]);
    assert!(matches!(bad_deal, Err(EngineError::InvalidConfig(_))));
    let mut bad_self_conv = spec.clone();
    bad_self_conv.seats[0].converters[0].inputs.push((0, 500));
    assert!(matches!(
        bad_self_conv.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    let mut bad_dup_conv = spec;
    bad_dup_conv.seats[0].converters[0].inputs.push((1, 500));
    assert!(matches!(
        bad_dup_conv.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    let _touch_card: GradedMandateCard = deck[0];
}

#[test]
fn replay_verifies_seed_derived_and_external_drand_beacons() {
    let tid = TableId(913);
    let seed = 11_u64;
    let (spec, sealed_deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 4, seed),
        "spec",
    );
    let eval_pk = spec.eval_vrf_pubkey;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "new");
    let mut turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) =
            commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let (beacon, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
        let r = must_ok(eng.sweep_turn_with_eval_vrf(&proof), "sweep");
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
            drawn_shocks: r.newly_drawn_shocks.clone(),
        });
        if r.season_closed {
            break;
        }
    }
    let log = SeasonReplayLog {
        spec,
        mode: TableMode::Eval,
        seed: must_ok(ReplayLayoutSeed::try_clear(seed), "clear seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk,
        sealed_mandate_deal: sealed_deal.clone(),
        mandate_commits: vec![None; 4],
        turns,
        mandate_reveals: vec![None; 4],
    };
    assert!(replay_season(&log).is_ok());

    // Eval mode refuses SeedDerived beacon_mode (#31).
    assert!(matches!(
        replay_season(&SeasonReplayLog {
            beacon_mode: ReplayBeaconMode::SeedDerived,
            ..log.clone()
        }),
        Err(EngineError::InvalidConfig(_))
    ));

    // Tampered proof or tampered beacon rejected in EvalVrf mode (#31).
    let mut bad_proof = log.clone();
    if let Some(ref mut prf) = bad_proof.turns[0].eval_vrf_proof {
        prf[0] ^= 1;
    }
    assert!(matches!(
        replay_season(&bad_proof),
        Err(EngineError::InvalidOrder(_))
    ));

    let mut bad_beacon = log.clone();
    bad_beacon.turns[0].turn_beacon[0] ^= 1;
    assert!(matches!(
        replay_season(&bad_beacon),
        Err(EngineError::InvalidOrder(_))
    ));

    // Eval mode rejects ExternalDrand beacon_mode (2026-11-15 addendum).
    assert!(matches!(
        replay_season(&SeasonReplayLog {
            beacon_mode: ReplayBeaconMode::ExternalDrand,
            ..log
        }),
        Err(EngineError::InvalidConfig(_))
    ));

    let schedule = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
        turn_period_secs: 60,
    };
    let tid_drand = TableId(9130);
    let (spec_drand, deal_drand) = must_ok(
        TableSpec::standard_game_chiral_ring_with_mandates(tid_drand, 4, seed, schedule),
        "spec_drand",
    );
    let mut eng_drand = must_ok(TurnEngine::new(spec_drand.clone()), "eng_drand");
    let mut drand_turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng_drand.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) = commit_and_reveal_all(
            &mut eng_drand,
            tid_drand,
            t,
            vec![SealedOrderBundle::default(); 4],
        );
        let round = must_ok(schedule.expected_round_for_turn(Turn(t)), "expected round");
        let sig = vec![0x40_u8.wrapping_add(t); DRAND_QUICKNET_SIGNATURE_BYTES];
        let beacon = must_ok(derive_drand_beacon(&sig), "drand beacon");
        let r = must_ok(eng_drand.sweep_turn_with_drand(round, &sig), "sweep drand");
        drand_turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: Some(round),
            drand_signature: Some(sig),
            eval_vrf_proof: None,
            drawn_shocks: r.newly_drawn_shocks.clone(),
        });
        if r.season_closed {
            break;
        }
    }
    let drand_ok = SeasonReplayLog {
        spec: spec_drand,
        mode: TableMode::Game,
        seed: must_ok(ReplayLayoutSeed::try_clear(0), "clear seed"),
        beacon_mode: ReplayBeaconMode::ExternalDrand,
        eval_vrf_pubkey: None,
        sealed_mandate_deal: deal_drand,
        mandate_commits: vec![None; 4],
        turns: drand_turns,
        mandate_reveals: vec![None; 4],
    };
    assert!(replay_season(&drand_ok).is_ok());

    let mut drand_missing_round = drand_ok.clone();
    drand_missing_round.turns[0].drand_round = None;
    assert!(matches!(
        replay_season(&drand_missing_round),
        Err(EngineError::SeasonEnded(_))
    ));

    let mut drand_missing_sig = drand_ok.clone();
    drand_missing_sig.turns[0].drand_signature = None;
    assert!(matches!(
        replay_season(&drand_missing_sig),
        Err(EngineError::SeasonEnded(_))
    ));

    let mut drand_off_sched = drand_ok.clone();
    drand_off_sched.turns[0].drand_round = drand_off_sched.turns[0].drand_round.map(|r| r + 1);
    assert!(matches!(
        replay_season(&drand_off_sched),
        Err(EngineError::InvalidOrder(_))
    ));

    let mut drand_noninc = drand_ok.clone();
    drand_noninc.turns[1].drand_round = drand_noninc.turns[0].drand_round;
    assert!(matches!(
        replay_season(&drand_noninc),
        Err(EngineError::SeasonEnded(_))
    ));

    let mut drand_bad_beacon = drand_ok;
    drand_bad_beacon.turns[0].turn_beacon[0] ^= 1;
    assert!(matches!(
        replay_season(&drand_bad_beacon),
        Err(EngineError::InvalidOrder(_))
    ));
}

#[test]
fn hand_checked_turn_math_golden_digest_bond_swap_order_and_charter_expiry() {
    // 1. Exact Opus hand-checked Turn 1 & Turn 2 numbers on TableId(8), 4 seats, seed 42 (B3).
    let tid = TableId(8);
    let salts = [
        RevealSalt([0x11; 32]),
        RevealSalt([0x22; 32]),
        RevealSalt([0x33; 32]),
        RevealSalt([0x44; 32]),
    ];
    let (spec, _) = must_ok(
        TableSpec::standard_chiral_ring_with_explicit_deal(tid, 4, 42, [0x42; 32], &salts),
        "spec",
    );
    let mut eng = must_ok(TurnEngine::new(spec), "new");
    let mut final_digest = [0u8; 32];
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let r = must_ok(
            eng.sweep_turn(derive_turn_beacon(42, tid, Turn(t))),
            "sweep",
        );
        if t == 1 {
            for (s, &delta) in r.pbrs_deltas.iter().enumerate() {
                assert_eq!(eng.state().seats[s].cred_debt, 3_600);
                assert_eq!(eng.state().seats[s].raw[s], 8_000);
                assert_eq!(compute_potential(eng.state(), s as u8), 25_600);
                assert_eq!(delta, 9_120);
            }
        }
        if t == 2 {
            for (s, &delta) in r.pbrs_deltas.iter().enumerate() {
                assert_eq!(eng.state().seats[s].cred_debt, 7_344);
                assert_eq!(eng.state().seats[s].raw[s], 14_400);
                assert_eq!(compute_potential(eng.state(), s as u8), 33_056);
                assert_eq!(delta, 5_803);
            }
        }
        final_digest = r.post_state_digest;
        if r.season_closed {
            break;
        }
    }
    assert_eq!(
        hex::encode(final_digest),
        "74cdaac93a401c7a80b50a69afd82597072ad7c02bc6fbf166121d09fec2eacc"
    );

    // 2. B9: Same-turn Turn-2 AtomicSwap fulfills a bond whose deadline is Turn 2.
    let mut eng_b9 = must_ok(
        TurnEngine::new(must_ok(
            TableSpec::standard_chiral_ring(TableId(914), 4, 1),
            "spec b9",
        )),
        "b9",
    );
    must_ok(
        eng_b9.apply_market_action(MarketAction::CreateEscrow {
            seat: 1,
            escrow: EscrowProposal {
                destination: 2,
                asset: AssetId::Cred,
                amount: 1_500,
                finish_after_turn: None,
                cancel_after_turn: 4,
                condition_hash: Some(must_ok(
                    compute_preimage_condition(&PreimageSecret([0x44; 32])),
                    "bond t2 cond",
                )),
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 1_000,
                    deadline_turn: 2,
                    mode: BondSettlementMode::SellerPerformanceBond,
                }),
            },
        }),
        "bond t2",
    );
    commit_and_reveal_all(
        &mut eng_b9,
        TableId(914),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(
        eng_b9.sweep_turn(derive_turn_beacon(1, TableId(914), Turn(1))),
        "b9 t1",
    );
    let mut b2 = vec![SealedOrderBundle::default(); 4];
    let sw = AtomicSwapProposal {
        swap_id: 500,
        maker: 1,
        taker: 2,
        maker_asset: AssetId::Raw(1),
        maker_amount: 2_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    b2[1].atomic_swaps.push(sw.clone());
    b2[2].atomic_swaps.push(sw);
    commit_and_reveal_all(&mut eng_b9, TableId(914), 2, b2);
    let r2 = must_ok(
        eng_b9.sweep_turn(derive_turn_beacon(1, TableId(914), Turn(2))),
        "b9 t2",
    );
    assert!(r2.escrows_finished.contains(&1));
    assert_eq!(eng_b9.state().seats[1].bond_breaches, 0);

    // 3. Charter awarded on Turn 6 (expires_at_turn = 12) remains active during Turn 12 TalkAndMarket.
    let mut spec_ch = must_ok(
        TableSpec::standard_chiral_ring(TableId(915), 4, 1),
        "spec ch",
    );
    spec_ch.shocks.push(league_core::ScheduledShockSpec {
        hint_turn: 12,
        forecast_turn: 13,
        realize_turn: 15,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 1_500,
    });
    let mut eng_ch = must_ok(TurnEngine::new(spec_ch), "eng ch");
    for t in 1..=12_u8 {
        if t == 12 {
            // Turn 12 TalkAndMarket still honors Turn 6 charters!
            must_ok(
                eng_ch.apply_market_action(MarketAction::FlagWashFreeze {
                    inspector: 0,
                    target_seat: 2,
                }),
                "freeze active on turn 12",
            );
            let hints = must_ok(
                league_core::engine_internals::private_hints_for_auditor(&eng_ch, Seat(1)),
                "auditor active on turn 12",
            );
            assert_eq!(hints.len(), 1);
        }
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 6 {
            bundles[0].charter_bids.push(CharterBidOrder {
                role: CharterRole::CustomsInspector,
                bid_cred: 200,
            });
            bundles[1].charter_bids.push(CharterBidOrder {
                role: CharterRole::OracleAuditor,
                bid_cred: 200,
            });
        }
        commit_and_reveal_all(&mut eng_ch, TableId(915), t, bundles);
        must_ok(
            eng_ch.sweep_turn(derive_turn_beacon(1, TableId(915), Turn(t))),
            "ch sweep",
        );
    }
    // After Turn 12 Step 7 expires them with no Turn 12 bids, Turn 13 rejects FlagWashFreeze.
    assert!(matches!(
        eng_ch.apply_market_action(MarketAction::FlagWashFreeze {
            inspector: 0,
            target_seat: 2,
        }),
        Err(EngineError::Unauthorized(_))
    ));
}

#[test]
fn b1_1_clamps_over_ordered_converters_and_projects_counts_receipt_and_replays_stably() {
    let tid = TableId(920);
    let (spec, deal_b1_1a, eval_seed_a) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 4, 3),
        "spec",
    );
    let mut spec = spec;
    let eval_pk_a = spec.eval_vrf_pubkey;
    // Give seats 1 and 2 larger yields so Seat 0 can gather inputs for 3 whole converter runs,
    // while Seat 0 requests 6 runs (holding-clamped to 3 when cap is 5 -> k = 3 < 6).
    spec.seats[0].converters[0].max_runs_per_turn = 5;
    spec.seats[1].raw_yield_per_turn = 20_000;
    spec.seats[2].raw_yield_per_turn = 20_000;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "new");

    let mut logged_turns = Vec::new();
    // Turn 1: empty bundles so seats 1 & 2 accumulate 20_000 Raw(1) / Raw(2) and Seat 3 refines / trades.
    let t1_bundles = vec![SealedOrderBundle::default(); 4];
    let (c1, rev1) = commit_and_reveal_all(&mut eng, tid, 1, t1_bundles);
    let (b1, prf1) = must_ok(eval_seed_a.prove_turn(tid, Turn(1)), "prove 1");
    let r1 = must_ok(eng.sweep_turn_with_eval_vrf(&prf1), "sweep 1");
    assert_eq!(r1.converter_orders_clamped, 0);
    assert_eq!(r1.project_orders_clamped, 0);
    logged_turns.push(LoggedTurn {
        turn: 1,
        market_actions: Vec::new(),
        cancel_lock_at: None,
        commitments: c1,
        reveals: rev1,
        turn_beacon: b1,
        drand_round: None,
        drand_signature: None,
        eval_vrf_proof: Some(prf1.to_vec()),
        drawn_shocks: r1.newly_drawn_shocks.clone(),
    });

    // Turn 2: Swap 7_000 Raw(1) & 7_000 Raw(2) into Seat 0 (800 bps tariff leaves 6_440 net;
    // recipe uses 2_000 each -> k = 3 whole runs, 440 left before Step 6 20% spoilage -> 352),
    // over-order runs = 6 (holding-clamped to k = 3 < 6), plus a second runs = 4 order with
    // 0 capacity left -> 0 runs (unaffordable/cap-0, not clamped).
    let mut t2_bundles = vec![SealedOrderBundle::default(); 4];
    let sw1 = AtomicSwapProposal {
        swap_id: 1,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Raw(1),
        maker_amount: 7_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    let sw2 = AtomicSwapProposal {
        swap_id: 2,
        maker: 2,
        taker: 0,
        maker_asset: AssetId::Raw(2),
        maker_amount: 7_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    t2_bundles[0].atomic_swaps = vec![sw1.clone(), sw2.clone()];
    t2_bundles[1].atomic_swaps = vec![sw1];
    t2_bundles[2].atomic_swaps = vec![sw2];
    t2_bundles[0].converter_runs = vec![
        ConverterRunOrder {
            recipe_idx: 0,
            runs: 6, // 6_440 net raw permits 3 whole runs at 2_000 each -> clamped (0 < 3 < 6)
        },
        ConverterRunOrder {
            recipe_idx: 0,
            runs: 4, // 0 remaining capacity / 440 raw -> 0 runs (not clamped)
        },
    ];
    let (c2, rev2) = commit_and_reveal_all(&mut eng, tid, 2, t2_bundles);
    let (b2, prf2) = must_ok(eval_seed_a.prove_turn(tid, Turn(2)), "prove 2");
    let r2 = must_ok(eng.sweep_turn_with_eval_vrf(&prf2), "sweep 2");
    assert_eq!(r2.converter_batches_run, 3);
    assert_eq!(r2.converter_orders_clamped, 1);
    assert_eq!(eng.state().seats[0].refined[0], 10_500); // 3 * 3_500
    assert_eq!(eng.state().seats[0].raw[1], 352); // 440 after 20% spoilage -> 352
    logged_turns.push(LoggedTurn {
        turn: 2,
        market_actions: Vec::new(),
        cancel_lock_at: None,
        commitments: c2,
        reveals: rev2,
        turn_beacon: b2,
        drand_round: None,
        drand_signature: None,
        eval_vrf_proof: Some(prf2.to_vec()),
        drawn_shocks: r2.newly_drawn_shocks.clone(),
    });

    while eng.state().phase == TurnPhase::TalkAndMarket {
        let turn = eng.state().turn;
        let (c, rev) =
            commit_and_reveal_all(&mut eng, tid, turn, vec![SealedOrderBundle::default(); 4]);
        let (beacon, prf) = must_ok(eval_seed_a.prove_turn(tid, Turn(turn)), "prove tail");
        let rt = must_ok(eng.sweep_turn_with_eval_vrf(&prf), "empty tail sweep");
        logged_turns.push(LoggedTurn {
            turn,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments: c,
            reveals: rev,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(prf.to_vec()),
            drawn_shocks: rt.newly_drawn_shocks,
        });
    }
    let outcome = must_ok(
        replay_season(&SeasonReplayLog {
            spec,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(3), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk_a,
            sealed_mandate_deal: deal_b1_1a,
            mandate_commits: vec![None; 4],
            turns: logged_turns,
            mandate_reveals: vec![None; 4],
        }),
        "replay clamped converters",
    );
    assert_eq!(outcome.state_digests[1], r2.post_state_digest);
    assert_eq!(outcome.receipts[1], r2);

    // Separate table directly verifying capacity-clamped converter (0 < k=5 < requested 7)
    // AND holdings-clamped project delivery (0 < k=2 < requested 6), plus full replay stability.
    let tid_p = TableId(921);
    let (mut spec_p, deal_b1_1b, eval_seed_b) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid_p, 4, 7),
        "spec_p",
    );
    let eval_pk_b = spec_p.eval_vrf_pubkey;
    spec_p.seats[0].converters[0].max_runs_per_turn = 5;
    spec_p.seats[1].raw_yield_per_turn = 30_000;
    spec_p.seats[2].raw_yield_per_turn = 30_000;
    spec_p.seats[3].raw_yield_per_turn = 30_000;
    let mut eng_clean = must_ok(TurnEngine::new(spec_p.clone()), "eng_clean");
    let mut replay_turns = Vec::new();
    let (pc1, prev1) = commit_and_reveal_all(
        &mut eng_clean,
        tid_p,
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    let (rb1, rprf1) = must_ok(eval_seed_b.prove_turn(tid_p, Turn(1)), "prove p1");
    let rrc1 = must_ok(eng_clean.sweep_turn_with_eval_vrf(&rprf1), "rc1");
    replay_turns.push(LoggedTurn {
        turn: 1,
        market_actions: Vec::new(),
        cancel_lock_at: None,
        commitments: pc1,
        reveals: prev1,
        turn_beacon: rb1,
        drand_round: None,
        drand_signature: None,
        eval_vrf_proof: Some(rprf1.to_vec()),
        drawn_shocks: rrc1.newly_drawn_shocks,
    });

    // Turn 2: Give Seat 0 12_000 Raw(1)&(2) (8% tariff -> 11_040 net >= 10_000 for 5 runs) & request 7 against cap=5 -> k=5!
    // Give Seat 1 5_000 Raw(2)&(3) (4_600 net) & run 2 -> 7_000 Refined(1).
    let mut pb2 = vec![SealedOrderBundle::default(); 4];
    let q1 = AtomicSwapProposal {
        swap_id: 20,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Raw(1),
        maker_amount: 12_000,
        taker_asset: AssetId::Cred,
        taker_amount: 50,
        domain_id: None,
    };
    let q2 = AtomicSwapProposal {
        swap_id: 21,
        maker: 2,
        taker: 0,
        maker_asset: AssetId::Raw(2),
        maker_amount: 12_000,
        taker_asset: AssetId::Cred,
        taker_amount: 50,
        domain_id: None,
    };
    let q3 = AtomicSwapProposal {
        swap_id: 22,
        maker: 2,
        taker: 1,
        maker_asset: AssetId::Raw(2),
        maker_amount: 5_000,
        taker_asset: AssetId::Cred,
        taker_amount: 50,
        domain_id: None,
    };
    let q4 = AtomicSwapProposal {
        swap_id: 23,
        maker: 3,
        taker: 1,
        maker_asset: AssetId::Raw(3),
        maker_amount: 5_000,
        taker_asset: AssetId::Cred,
        taker_amount: 50,
        domain_id: None,
    };
    pb2[0].atomic_swaps = vec![q1.clone(), q2.clone()];
    pb2[1].atomic_swaps = vec![q1, q3.clone(), q4.clone()];
    pb2[2].atomic_swaps = vec![q2, q3];
    pb2[3].atomic_swaps = vec![q4];
    pb2[0].converter_runs = vec![ConverterRunOrder {
        recipe_idx: 0,
        runs: 7, // cap is 5 -> clamped k = 5 < 7
    }];
    pb2[1].converter_runs = vec![ConverterRunOrder {
        recipe_idx: 0,
        runs: 2,
    }];
    let (pc2, prev2) = commit_and_reveal_all(&mut eng_clean, tid_p, 2, pb2);
    let (rb2, rprf2) = must_ok(eval_seed_b.prove_turn(tid_p, Turn(2)), "prove p2");
    let rr2 = must_ok(eng_clean.sweep_turn_with_eval_vrf(&rprf2), "rc2");
    assert_eq!(rr2.converter_batches_run, 7);
    assert_eq!(rr2.converter_orders_clamped, 1);
    assert_eq!(eng_clean.state().seats[0].refined[0], 17_500);
    assert_eq!(eng_clean.state().seats[1].refined[1], 7_000);
    replay_turns.push(LoggedTurn {
        turn: 2,
        market_actions: Vec::new(),
        cancel_lock_at: None,
        commitments: pc2,
        reveals: prev2,
        turn_beacon: rb2,
        drand_round: None,
        drand_signature: None,
        eval_vrf_proof: Some(rprf2.to_vec()),
        drawn_shocks: rr2.newly_drawn_shocks.clone(),
    });

    // Turn 3: Seat 1 swaps 2_500 Refined(1) to Seat 0; Seat 0 requests 6 batches of Project 1
    // (inputs: 1_000 Refined(0), 1_000 Refined(1)), so 2_500 Refined(1) clamps cleanly to k = 2!
    let mut pb3 = vec![SealedOrderBundle::default(); 4];
    let pr_sw = AtomicSwapProposal {
        swap_id: 30,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Refined(1),
        maker_amount: 2_500,
        taker_asset: AssetId::Cred,
        taker_amount: 50,
        domain_id: None,
    };
    pb3[0].atomic_swaps = vec![pr_sw.clone()];
    pb3[1].atomic_swaps = vec![pr_sw];
    pb3[0].project_deliveries = vec![ProjectDeliveryOrder {
        project_id: 1,
        batches: 6, // holding-clamped to k = 2 whole batches (consumes 2_000 each)
    }];
    let (pc3, prev3) = commit_and_reveal_all(&mut eng_clean, tid_p, 3, pb3);
    let (rb3, rprf3) = must_ok(eval_seed_b.prove_turn(tid_p, Turn(3)), "prove p3");
    let rr3 = must_ok(eng_clean.sweep_turn_with_eval_vrf(&rprf3), "rc3");
    assert_eq!(rr3.projects_completed, 2);
    assert_eq!(rr3.project_orders_clamped, 1);
    assert_eq!(eng_clean.state().seats[0].project_equity, 9_600); // 2 * 4_800
    assert_eq!(eng_clean.state().seats[0].refined[1], 350); // 2_350 net after 6% tariff - 2_000 used
    replay_turns.push(LoggedTurn {
        turn: 3,
        market_actions: Vec::new(),
        cancel_lock_at: None,
        commitments: pc3,
        reveals: prev3,
        turn_beacon: rb3,
        drand_round: None,
        drand_signature: None,
        eval_vrf_proof: Some(rprf3.to_vec()),
        drawn_shocks: rr3.newly_drawn_shocks.clone(),
    });

    while eng_clean.state().phase == TurnPhase::TalkAndMarket {
        let turn = eng_clean.state().turn;
        let (c, rev) = commit_and_reveal_all(
            &mut eng_clean,
            tid_p,
            turn,
            vec![SealedOrderBundle::default(); 4],
        );
        let (beacon, rprft) = must_ok(eval_seed_b.prove_turn(tid_p, Turn(turn)), "prove pt");
        let rrt = must_ok(
            eng_clean.sweep_turn_with_eval_vrf(&rprft),
            "empty tail sweep clean",
        );
        replay_turns.push(LoggedTurn {
            turn,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments: c,
            reveals: rev,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(rprft.to_vec()),
            drawn_shocks: rrt.newly_drawn_shocks,
        });
    }
    let replayed = must_ok(
        replay_season(&SeasonReplayLog {
            spec: spec_p,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(7), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk_b,
            sealed_mandate_deal: deal_b1_1b,
            mandate_commits: vec![None; 4],
            turns: replay_turns,
            mandate_reveals: vec![None; 4],
        }),
        "replay b1_1",
    );
    assert_eq!(replayed.state_digests[1], rr2.post_state_digest);
    assert_eq!(replayed.state_digests[2], rr3.post_state_digest);
    assert_eq!(replayed.receipts[1], rr2);
    assert_eq!(replayed.receipts[2], rr3);
}

#[test]
fn b1_2_mandate_validation_enforces_seat_count_minus_two_bounds() {
    for n in [4_u8, 5_u8, 6_u8] {
        let max_safe = n - 2;
        assert!(
            MandateKind::MinRefinedVariety {
                distinct_goods: max_safe,
                min_each: 1_000,
            }
            .validate_for_seat_count(n)
            .is_ok()
        );
        assert!(
            MandateKind::MinActivePartners {
                min_partners: max_safe,
            }
            .validate_for_seat_count(n)
            .is_ok()
        );
        for over in [0_u8, n - 1, n] {
            assert!(matches!(
                MandateKind::MinRefinedVariety {
                    distinct_goods: over,
                    min_each: 1_000,
                }
                .validate_for_seat_count(n),
                Err(EngineError::InvalidConfig(_))
            ));
            assert!(matches!(
                MandateKind::MinActivePartners { min_partners: over }.validate_for_seat_count(n),
                Err(EngineError::InvalidConfig(_))
            ));
        }

        // TableSpec::validate and deal_graded_mandates both reject seat_count - 1 thresholds.
        let mut spec = must_ok(
            TableSpec::standard_chiral_ring(TableId(925 + u64::from(n)), n, 1),
            "spec",
        );
        spec.mandate_deck[0].kind = MandateKind::MinRefinedVariety {
            distinct_goods: n - 1,
            min_each: 1_000,
        };
        assert!(matches!(
            spec.validate(),
            Err(EngineError::InvalidConfig(_))
        ));
        assert!(matches!(
            deal_graded_mandates(spec.table_id, n, &spec.mandate_deck),
            Err(EngineError::InvalidConfig(_))
        ));
    }
}

#[test]
fn b1_3_refuses_all_zero_salts_and_preimage_secrets_across_commit_and_escrow() {
    let tid = TableId(930);
    let zero_salt = RevealSalt([0u8; 32]);
    let zero_preimage = PreimageSecret([0u8; 32]);
    let zero_commit = CommitmentHash([0u8; 32]);
    assert!(zero_salt.is_degenerate());
    assert!(zero_preimage.is_degenerate());
    assert!(zero_commit.is_degenerate());

    let rev = OrderReveal {
        bundle: SealedOrderBundle::default(),
        salt: zero_salt.clone(),
    };
    assert!(matches!(
        compute_order_commitment(tid, Turn(1), Seat(0), &rev),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        verify_order_reveal(tid, Turn(1), Seat(0), CommitmentHash([0x11; 32]), &rev),
        Err(EngineError::InvalidOrder(_))
    ));
    let good_rev = OrderReveal {
        bundle: SealedOrderBundle::default(),
        salt: RevealSalt([0x22; 32]),
    };
    assert!(matches!(
        verify_order_reveal(tid, Turn(1), Seat(0), zero_commit, &good_rev),
        Err(EngineError::InvalidOrder(_))
    ));

    let m_rev = MandateReveal {
        mandate: PrivateMandate {
            kind: MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            bonus_equity: MIN_MANDATE_BONUS_EQUITY,
        },
        salt: zero_salt,
    };
    assert!(matches!(
        compute_mandate_commitment(tid, Seat(0), &m_rev),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        verify_preimage_condition(zero_commit, &PreimageSecret([0x33; 32])),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        compute_preimage_condition(&zero_preimage),
        Err(EngineError::InvalidOrder(_))
    ));

    // Engine rejects zero order commitment, zero mandate commitment, and zero-preimage escrows.
    let spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "new");
    assert!(matches!(
        eng.submit_mandate_commitment(Seat(0), zero_commit),
        Err(EngineError::InvalidOrder(_))
    ));
    let forged_zero_hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"league-core/preimage/v2");
        h.update([0u8; 32]);
        CommitmentHash(h.finalize().into())
    };
    for bad_hash in [zero_commit, forged_zero_hash] {
        assert!(matches!(
            eng.apply_market_action(MarketAction::CreateEscrow {
                seat: 0,
                escrow: EscrowProposal {
                    destination: 1,
                    asset: AssetId::Cred,
                    amount: 500,
                    finish_after_turn: Some(1),
                    cancel_after_turn: 3,
                    condition_hash: Some(bad_hash),
                    bond: None,
                },
            }),
            Err(EngineError::InvalidOrder(_))
        ));
    }
    must_ok(eng.advance_to_commit(), "to commit");
    assert!(matches!(
        eng.submit_order_commitment(Seat(0), zero_commit),
        Err(EngineError::InvalidOrder(_))
    ));
}

#[test]
fn b1_4_preview_bundle_leaves_state_digest_unchanged_and_matches_real_sweep() {
    let tid = TableId(935);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 9), "spec");
    spec.seats[0].converters[0].max_runs_per_turn = 4;
    spec.seats[1].raw_yield_per_turn = 20_000;
    spec.seats[2].raw_yield_per_turn = 20_000;
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    // Turn 1 empty so peers mint raw goods & Seat 0 builds debt for upkeep/repay preview coverage.
    commit_and_reveal_all(&mut eng, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(
        eng.sweep_turn(derive_turn_beacon(9, tid, Turn(1))),
        "sweep 1",
    );

    // Turn 2: Seat 1 posts a resting Raw(1) sell offer that Seat 0 can lift in Phase 1,
    // leaving 10_000 Raw(1) free on Seat 1 for the co-signed atomic swap below.
    must_ok(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(1),
                amount: 6_000,
                price_per_unit_cred: 2_000,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "sell offer",
    );
    let actions = vec![
        MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 2_000,
                price_per_unit_cred: 2_000,
                domain_id: None,
                is_hybrid: false,
            },
        },
        // Include one refused action to confirm preview records refused_market_actions cleanly.
        MarketAction::CancelOffer {
            seat: 0,
            offer_id: 99_999,
        },
    ];
    let sw = AtomicSwapProposal {
        swap_id: 77,
        maker: 2,
        taker: 0,
        maker_asset: AssetId::Raw(2),
        maker_amount: 5_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    let pre_raw_buy = vec![
        MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 1_000,
                price_per_unit_cred: 2_000,
                domain_id: None,
                is_hybrid: false,
            },
        },
        MarketAction::CancelOffer {
            seat: 0,
            offer_id: 99_999,
        },
    ];
    // Give Seat 0 4_000 Raw(1) (+ 1_000 bought = 5_000) & 5_000 Raw(2) via co-signed swaps in
    // `peer_bundles` (2_000 per run -> k = 2 runs < 3 requested!).
    let sw_raw1 = AtomicSwapProposal {
        swap_id: 76,
        maker: 1,
        taker: 0,
        maker_asset: AssetId::Raw(1),
        maker_amount: 4_000,
        taker_asset: AssetId::Cred,
        taker_amount: 100,
        domain_id: None,
    };
    let bundle_0 = SealedOrderBundle {
        atomic_swaps: vec![sw_raw1.clone(), sw.clone()],
        converter_runs: vec![ConverterRunOrder {
            recipe_idx: 0,
            runs: 3, // 5_000 Raw(1)&(2) at 2_000 each -> clamped to k = 2 < 3!
        }],
        debt_repayment_cred: 500,
        ..SealedOrderBundle::default()
    };
    let mut peer_bundles = BTreeMap::new();
    peer_bundles.insert(
        1_u8,
        SealedOrderBundle {
            atomic_swaps: vec![sw_raw1],
            ..SealedOrderBundle::default()
        },
    );
    peer_bundles.insert(
        2_u8,
        SealedOrderBundle {
            atomic_swaps: vec![sw],
            ..SealedOrderBundle::default()
        },
    );

    let digest_before = must_ok(eng.state_digest(), "digest before preview");
    let turn_beacon = derive_turn_beacon(9, tid, Turn(2));

    // 1. Standalone default live preview leaves engine bit-identical.
    let solo_preview: BundlePreview = must_ok(
        league_core::engine_internals::preview_bundle_live(&eng, Seat(0), &actions, &bundle_0),
        "solo preview",
    );
    assert_eq!(solo_preview.refused_market_actions.len(), 1);
    assert_eq!(solo_preview.refused_market_actions[0].0, 1);
    assert_eq!(
        must_ok(eng.state_digest(), "digest after solo preview"),
        digest_before
    );

    // 2. Context-aware live preview also leaves engine untouched and matches the real Sweep bit-for-bit.
    let ctx_preview = must_ok(
        league_core::engine_internals::preview_bundle_with_context_live(
            &eng,
            Seat(0),
            &pre_raw_buy,
            &bundle_0,
            &peer_bundles,
            turn_beacon,
        ),
        "ctx preview",
    );
    assert_eq!(
        must_ok(eng.state_digest(), "digest after ctx preview"),
        digest_before
    );
    assert_eq!(
        ctx_preview.converter_orders,
        vec![ConverterOrderPreview {
            recipe_idx: 0,
            requested_runs: 3,
            executed_runs: 2,
            clamped: true,
        }]
    );
    assert_eq!(
        ctx_preview
            .swap_orders
            .iter()
            .map(|s| s.outcome)
            .collect::<Vec<_>>(),
        vec![SwapPreviewOutcome::Settled, SwapPreviewOutcome::Settled]
    );
    assert_eq!(ctx_preview.upkeep.debt_repaid_cred, 500);

    // Now execute the real Turn 2 sequence on `eng` and assert exact prediction match.
    for action in &pre_raw_buy {
        let _ = eng.apply_market_action(action.clone());
    }
    let mut real_bundles = vec![SealedOrderBundle::default(); 4];
    real_bundles[0] = bundle_0;
    real_bundles[1] = peer_bundles[&1].clone();
    real_bundles[2] = peer_bundles[&2].clone();
    commit_and_reveal_all(&mut eng, tid, 2, real_bundles);
    let real_receipt = must_ok(eng.sweep_turn(turn_beacon), "real sweep 2");

    assert_eq!(ctx_preview.post_seat_state, eng.state().seats[0]);
    assert_eq!(ctx_preview.seat_pbrs_delta, real_receipt.pbrs_deltas[0]);
    assert_eq!(ctx_preview.sweep_receipt, real_receipt);

    // Check-order and project-order preview structs compile and cover CheckOrderPreview variants.
    let _check_sample = CheckOrderPreview {
        check_id: 1,
        outcome: CheckPreviewOutcome::NotEligibleOrIgnored,
    };
    let _proj_sample = ProjectOrderPreview {
        project_id: 1,
        requested_batches: 1,
        executed_batches: 1,
        clamped: false,
    };
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[test]
fn m1_unelected_and_zero_vote_officers_never_enact_platforms() {
    let tid = TableId(940);
    let spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    // Probe P8 reproducer (4 seats, Turn 4, zero ballots): Seat 0 starts in default officer slots
    // without ever winning a vote; its StandForOffice platform must NOT enact.
    for t in 1..=3_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let _ = must_ok(
            eng.sweep_turn(derive_turn_beacon(1, tid, Turn(t))),
            "sweep 1..3",
        );
    }
    must_ok(
        eng.apply_market_action(MarketAction::StandForOffice {
            seat: 0,
            platform: CouncilPlatform {
                proposed_tariffs_bps: vec![(1, (5_000, 5_000))],
                cartel_admissions: vec![(7, 0), (7, 1)],
                cartel_revocations: Vec::new(),
            },
        }),
        "stand 0 turn 4",
    );
    commit_and_reveal_all(&mut eng, tid, 4, vec![SealedOrderBundle::default(); 4]);
    let r4 = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(4))),
        "sweep 4",
    );
    assert_eq!(r4.elected_officers, None);
    assert!(eng.state().domains.is_empty());
    assert_eq!(eng.state().seats[1].raw_tariff_bps, 800);
    assert_eq!(eng.state().seats[1].refined_tariff_bps, 600);

    // Turns 5..8: Seat 0 files a platform again and receives >= 1 real ballot -> MUST enact.
    for t in 5..=7_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let _ = must_ok(
            eng.sweep_turn(derive_turn_beacon(1, tid, Turn(t))),
            "sweep 5..7",
        );
    }
    must_ok(
        eng.apply_market_action(MarketAction::StandForOffice {
            seat: 0,
            platform: CouncilPlatform {
                proposed_tariffs_bps: vec![(1, (5_000, 5_000))],
                cartel_admissions: Vec::new(),
                cartel_revocations: Vec::new(),
            },
        }),
        "stand 0 turn 8",
    );
    must_ok(
        eng.apply_market_action(MarketAction::StandForOffice {
            seat: 1,
            platform: CouncilPlatform {
                proposed_tariffs_bps: Vec::new(),
                cartel_admissions: vec![(7, 0), (7, 1)],
                cartel_revocations: Vec::new(),
            },
        }),
        "stand 1 turn 8",
    );
    let mut b8 = vec![SealedOrderBundle::default(); 4];
    b8[2].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 1,
    });
    commit_and_reveal_all(&mut eng, tid, 8, b8);
    let r8 = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(8))),
        "sweep 8",
    );
    assert_eq!(r8.elected_officers, Some((0, Some(1))));
    assert_eq!(eng.state().domains.len(), 1);
    assert_eq!(eng.state().domains[0].domain_id, 7);
    assert!(eng.state().domains[0].members.contains(&0));
    assert!(eng.state().domains[0].members.contains(&1));
    assert_eq!(eng.state().seats[1].raw_tariff_bps, MAX_TARIFF_BPS);
    assert_eq!(eng.state().seats[1].refined_tariff_bps, MAX_TARIFF_BPS);

    // Turns 9..12: Previously elected incumbents file new platforms on Turn 12 with ZERO votes ->
    // incumbents keep office indices, elected_officers is None, and unvoted platforms are ignored.
    for t in 9..=11_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let _ = must_ok(
            eng.sweep_turn(derive_turn_beacon(1, tid, Turn(t))),
            "sweep 9..11",
        );
    }
    must_ok(
        eng.apply_market_action(MarketAction::StandForOffice {
            seat: 0,
            platform: CouncilPlatform {
                proposed_tariffs_bps: vec![(2, (2_100, 1_900))],
                cartel_admissions: vec![(3, 2)],
                cartel_revocations: Vec::new(),
            },
        }),
        "stand 0 turn 12",
    );
    commit_and_reveal_all(&mut eng, tid, 12, vec![SealedOrderBundle::default(); 4]);
    let r12 = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(12))),
        "sweep 12",
    );
    assert_eq!(r12.elected_officers, None);
    assert_eq!(eng.state().trade_minister, 0);
    assert_eq!(eng.state().cartel_chair, Some(1));
    assert_eq!(eng.state().domains.len(), 1);
    assert_eq!(eng.state().seats[2].raw_tariff_bps, 800);
    assert_eq!(eng.state().seats[2].refined_tariff_bps, 600);
}

#[test]
fn m2_drand_quicknet_schedule_and_signature_bound_tie_break() {
    assert_eq!(
        drand_quicknet_round_for_close(DRAND_QUICKNET_GENESIS_UNIX),
        Ok(1)
    );
    assert_eq!(
        drand_quicknet_round_for_close(DRAND_QUICKNET_GENESIS_UNIX + 2),
        Ok(1)
    );
    assert_eq!(
        drand_quicknet_round_for_close(DRAND_QUICKNET_GENESIS_UNIX + 3),
        Ok(2)
    );
    assert!(matches!(
        drand_quicknet_round_for_close(DRAND_QUICKNET_GENESIS_UNIX - 1),
        Err(EngineError::InvalidOrder(_))
    ));

    let schedule = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 300,
        turn_period_secs: DRAND_QUICKNET_PERIOD_SECS * 10,
    };
    let tid = TableId(941);
    let (spec, _) = must_ok(
        TableSpec::standard_game_chiral_ring_with_mandates(tid, 4, 5, schedule),
        "spec",
    );
    let mut eng = must_ok(TurnEngine::new(spec), "new");
    for t in 1..=3_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let round = must_ok(schedule.expected_round_for_turn(Turn(t)), "round");
        let sig = vec![0x30_u8.wrapping_add(t); DRAND_QUICKNET_SIGNATURE_BYTES];
        let _ = must_ok(eng.sweep_turn_with_drand(round, &sig), "sweep drand 1..3");
    }

    // Turn 4: 2-2 tie between seats 1 and 2 settled deterministically by SHA-256(drand_signature).
    let mut t4 = vec![SealedOrderBundle::default(); 4];
    t4[0].council_vote = Some(CouncilVoteBallot {
        trade_minister: 1,
        cartel_chair: 1,
    });
    t4[1].council_vote = Some(CouncilVoteBallot {
        trade_minister: 1,
        cartel_chair: 1,
    });
    t4[2].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 2,
    });
    t4[3].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 2,
    });
    commit_and_reveal_all(&mut eng, tid, 4, t4);

    let r4_expected = must_ok(schedule.expected_round_for_turn(Turn(4)), "round 4");
    let sig_4 = vec![0x7B_u8; DRAND_QUICKNET_SIGNATURE_BYTES];
    let expected_beacon = must_ok(derive_drand_beacon(&sig_4), "beacon 4");
    assert_eq!(
        drand_quicknet_signature_status(r4_expected, &sig_4),
        Ok(DrandBeaconAuthStatus::HashBoundUnauthenticated)
    );
    assert_eq!(
        verify_drand_quicknet_beacon(&schedule, Turn(4), r4_expected, &sig_4, expected_beacon),
        Ok(DrandBeaconAuthStatus::HashBoundUnauthenticated)
    );

    // Off-schedule round, round 0, truncated signature, and all-zero signature must all be rejected.
    assert!(matches!(
        drand_quicknet_signature_status(0, &sig_4),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        drand_quicknet_signature_status(r4_expected, &[0x7B; 47]),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        drand_quicknet_signature_status(r4_expected, &[0u8; DRAND_QUICKNET_SIGNATURE_BYTES]),
        Err(EngineError::InvalidOrder(_))
    ));
    assert_eq!(
        DrandBeaconAuthStatus::HashBoundUnauthenticated.as_str(),
        "hash_bound_unauthenticated"
    );
    assert_eq!(
        DrandBeaconAuthStatus::HashBoundUnauthenticated.to_string(),
        "hash_bound_unauthenticated"
    );
    let encoded_status = must_ok(
        serde_json::to_string(&DrandBeaconAuthStatus::HashBoundUnauthenticated),
        "serialize auth status",
    );
    assert_eq!(encoded_status, "\"hash_bound_unauthenticated\"");
    let decoded_status: DrandBeaconAuthStatus = must_ok(
        serde_json::from_str(&encoded_status),
        "deserialize auth status",
    );
    assert_eq!(
        decoded_status,
        DrandBeaconAuthStatus::HashBoundUnauthenticated
    );
    assert!(matches!(
        eng.sweep_turn_with_drand(r4_expected + 1, &sig_4),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        eng.sweep_turn_with_drand(r4_expected, &[0x7B; 47]),
        Err(EngineError::InvalidOrder(_))
    ));
    assert!(matches!(
        eng.sweep_turn_with_drand(r4_expected, &[0u8; DRAND_QUICKNET_SIGNATURE_BYTES]),
        Err(EngineError::InvalidOrder(_))
    ));

    let expected_tm =
        break_tie_with_turn_beacon(expected_beacon, tid, Turn(4), b"trade_minister", &[1, 2]);
    let expected_cc = Some(if expected_tm == 1 { 2 } else { 1 });
    let _touch_compat = break_tie_with_beacon(expected_beacon, b"trade_minister", &[1, 2]);
    let r4 = must_ok(
        eng.sweep_turn_with_drand(r4_expected, &sig_4),
        "sweep 4 drand",
    );
    assert_eq!(r4.elected_officers, Some((expected_tm, expected_cc)));
}

#[test]
fn m3_mandates_use_os_csprng_and_resist_seed_bruteforce_probe_p9() {
    use sha2::{Digest, Sha256};

    let tid = TableId(777);
    let replay_seed = 27_431_u64;
    let (spec_a, deal_a, eval_seed_a) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 5, replay_seed),
        "deal a",
    );
    let (spec_b, deal_b, _eval_seed_b) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 5, replay_seed),
        "deal b",
    );

    // Public ring layout for identical table layout seed matches while private mandate entropy differs.
    assert_eq!(spec_a.seats, spec_b.seats);
    assert_eq!(spec_a.shocks, spec_b.shocks);
    assert_ne!(deal_a.deal_seed, deal_b.deal_seed);
    assert_ne!(spec_a.dealt_mandate_commits, spec_b.dealt_mandate_commits);
    for idx in 0..5 {
        assert_ne!(deal_a.seat_reveals[idx].salt, deal_b.seat_reveals[idx].salt);
    }

    // Debug formatting redacts raw deal_seed.
    let dbg = format!("{deal_a:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains(&hex::encode(deal_a.deal_seed)));

    assert!(matches!(
        verify_sealed_mandate_deal(
            tid,
            5,
            &spec_a.mandate_deck,
            &spec_a.dealt_mandate_commits,
            &deal_a,
        ),
        Ok(true)
    ));
    let mut tampered_deal = deal_a.clone();
    tampered_deal.deal_seed[0] ^= 1;
    assert!(matches!(
        verify_sealed_mandate_deal(
            tid,
            5,
            &spec_a.mandate_deck,
            &spec_a.dealt_mandate_commits,
            &tampered_deal,
        ),
        Ok(false)
    ));

    // Probe P9 brute-force scan across candidate integer seeds 0..30_000 (spanning 27_431)
    // trying legacy SHA-256("league-core/mandate-salt/v2" || seed || table || 0) must find 0 matches.
    let target_commit = spec_a.dealt_mandate_commits[0];
    let deck = &spec_a.mandate_deck;
    let mut recovered = false;
    'scan: for cand_seed in 0..30_000_u64 {
        let mut h = Sha256::new();
        h.update(b"league-core/mandate-salt/v2");
        h.update(cand_seed.to_le_bytes());
        h.update(tid.0.to_le_bytes());
        h.update([0_u8]);
        let mut salt_bytes: [u8; 32] = h.finalize().into();
        if salt_bytes == [0u8; 32] {
            salt_bytes[0] = 1;
        }
        let salt = RevealSalt(salt_bytes);
        for card in deck {
            let candidate_rev = MandateReveal {
                mandate: PrivateMandate {
                    kind: card.kind,
                    bonus_equity: card.bonus_equity,
                },
                salt: salt.clone(),
            };
            if let Ok(c) = compute_mandate_commitment(tid, Seat(0), &candidate_rev)
                && c == target_commit
            {
                recovered = true;
                break 'scan;
            }
        }
    }
    assert!(
        !recovered,
        "Probe P9 legacy seed scan must recover zero OS-entropy mandates"
    );

    // Also verify replay_season rejects a tampered sealed_mandate_deal.
    let eval_pk_a = spec_a.eval_vrf_pubkey;
    let mut eng = must_ok(TurnEngine::new(spec_a.clone()), "eng");
    let mut turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) =
            commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 5]);
        let (beacon, prf) = must_ok(eval_seed_a.prove_turn(tid, Turn(t)), "prove");
        let r = must_ok(eng.sweep_turn_with_eval_vrf(&prf), "sweep");
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(prf.to_vec()),
            drawn_shocks: r.newly_drawn_shocks,
        });
        if r.season_closed {
            break;
        }
    }
    let valid_log = SeasonReplayLog {
        spec: spec_a,
        mode: TableMode::Eval,
        seed: must_ok(ReplayLayoutSeed::try_clear(replay_seed), "clear seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk_a,
        sealed_mandate_deal: deal_a,
        mandate_commits: vec![None; 5],
        turns,
        mandate_reveals: vec![None; 5],
    };
    assert!(replay_season(&valid_log).is_ok());
    let mut bad_deal_log = valid_log;
    bad_deal_log.sealed_mandate_deal = tampered_deal;
    assert!(matches!(
        replay_season(&bad_deal_log),
        Err(EngineError::InvalidConfig(_))
    ));
}

#[test]
fn f1_f2_scoring_refuses_undealt_card_and_reveal_succeeds_without_turn1_echo() {
    let tid = TableId(942);
    let salts = [
        RevealSalt([0x21; 32]),
        RevealSalt([0x22; 32]),
        RevealSalt([0x23; 32]),
        RevealSalt([0x24; 32]),
    ];
    let eval_seed_f1 = must_ok(SealedEvalSeed::try_from_bytes([0x66; 32]), "eval_seed_f1");
    let (mut spec, sealed_deal, eval_seed_f1) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            tid,
            4,
            9,
            [0x99; 32],
            &salts,
            eval_seed_f1,
        ),
        "explicit spec",
    );
    for sc in &mut spec.seats {
        sc.initial_cred = 500 * UNIT_SCALE;
    }
    let eval_pk_f1 = spec.eval_vrf_pubkey;
    let clean_seat = must_ok(
        sealed_deal
            .seat_reveals
            .iter()
            .position(|r| matches!(r.mandate.kind, MandateKind::MaintainCleanCredit { .. }))
            .ok_or("at least one seat dealt MaintainCleanCredit"),
        "clean_seat",
    );
    let undealt_seat = if clean_seat == 0 { 1 } else { 0 };
    assert_ne!(
        sealed_deal.seat_reveals[undealt_seat].mandate,
        sealed_deal.seat_reveals[clean_seat].mandate
    );

    let mut eng = must_ok(TurnEngine::new(spec.clone()), "eng");
    let mut turns = Vec::new();
    // F2: Do NOT call submit_mandate_commitment anywhere during Turn 1!
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) =
            commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let (beacon, prf) = must_ok(eval_seed_f1.prove_turn(tid, Turn(t)), "prove");
        let r = must_ok(eng.sweep_turn_with_eval_vrf(&prf), "sweep");
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(prf.to_vec()),
            drawn_shocks: r.newly_drawn_shocks,
        });
        if r.season_closed {
            break;
        }
    }

    // F1 (Probe P10): calling evaluate_mandate_bonus or compute_final_scoreboard directly with a
    // forged/undealt card + non-zero salt MUST score (false, 0) / 0.
    let forged_clean = MandateReveal {
        mandate: sealed_deal.seat_reveals[clean_seat].mandate,
        salt: RevealSalt([0xEE; 32]),
    };
    assert_eq!(
        evaluate_mandate_bonus(eng.state(), undealt_seat as u8, &forged_clean),
        (false, 0)
    );
    let mut forged_vec = vec![None; 4];
    forged_vec[undealt_seat] = Some(forged_clean);
    let forged_board = must_ok(
        compute_final_scoreboard(eng.state(), &forged_vec),
        "forged scoreboard call",
    );
    let undealt_entry = must_ok(
        forged_board
            .standings
            .iter()
            .find(|e| e.seat == undealt_seat as u8)
            .ok_or("undealt entry"),
        "find",
    );
    assert_eq!(undealt_entry.mandate_bonus, 0);

    // F2: Genuine dealt mandate reveal succeeds WITHOUT a Turn-1 echo commit and pays full bonus.
    let expected_bonus = convert_mandate_points_to_equity(
        sealed_deal.seat_reveals[clean_seat].mandate.bonus_equity,
        eng.state(),
    );
    assert!(expected_bonus > 0);
    must_ok(
        eng.submit_mandate_reveal(
            Seat(clean_seat as u8),
            sealed_deal.seat_reveals[clean_seat].clone(),
        ),
        "reveal without Turn-1 echo",
    );
    let live_board = must_ok(eng.finalize_season(), "finalize");
    let clean_entry = must_ok(
        live_board
            .standings
            .iter()
            .find(|e| e.seat == clean_seat as u8)
            .ok_or("clean entry"),
        "find",
    );
    assert_eq!(clean_entry.mandate_bonus, expected_bonus);

    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[clean_seat] = Some(sealed_deal.seat_reveals[clean_seat].clone());
    let replayed = must_ok(
        replay_season(&SeasonReplayLog {
            spec,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(9), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk_f1,
            sealed_mandate_deal: sealed_deal,
            mandate_commits: vec![None; 4],
            turns,
            mandate_reveals,
        }),
        "replay without Turn-1 echo",
    );
    assert_eq!(replayed.scoreboard, live_board);
}

#[test]
fn f4_width_independent_index_matches_u64_modulo_above_u32_max() {
    let value: u64 = (1_u64 << 33) + 5; // u64 % 3 == 1, whereas low u32 (5) % 3 == 2
    let mut digest = [0u8; 32];
    digest[0..8].copy_from_slice(&value.to_le_bytes());
    assert_eq!(width_independent_index(digest, 3), 1);

    // Explicitly verify deal_graded_mandates_with_entropy determinism matches across all seats.
    let deck = default_graded_mandate_deck(4);
    let salts = vec![
        RevealSalt([1; 32]),
        RevealSalt([2; 32]),
        RevealSalt([3; 32]),
        RevealSalt([4; 32]),
    ];
    let (c1, d1) = must_ok(
        deal_graded_mandates_with_entropy(TableId(943), 4, [0x55; 32], &salts, &deck),
        "deal 1",
    );
    let (c2, d2) = must_ok(
        deal_graded_mandates_with_entropy(TableId(943), 4, [0x55; 32], &salts, &deck),
        "deal 2",
    );
    assert_eq!(c1, c2);
    assert_eq!(d1, d2);
}

fn table_cred_total(eng: &TurnEngine, total_repaid: Units, total_burned: Units) -> Units {
    let free: Units = eng.state().seats.iter().map(|s| s.cred).sum();
    let offer_locked: Units = eng
        .state()
        .offers
        .values()
        .filter(|o| o.side == OfferSide::Buy)
        .map(|o| o.locked_cred)
        .sum();
    let escrow_locked: Units = eng
        .state()
        .escrows
        .values()
        .filter(|e| e.status == EscrowStatus::Locked)
        .map(|e| e.amount_cred)
        .sum();
    let dual_locked: Units = eng
        .state()
        .dual_escrows
        .values()
        .filter(|e| e.status == EscrowStatus::Locked)
        .map(|e| e.buyer_bond_cred + e.seller_bond_cred)
        .sum();
    let wash_freeze_locked: Units = eng
        .state()
        .pending_wash_freezes
        .iter()
        .map(|w| w.bond_cred)
        .sum();
    free + offer_locked
        + escrow_locked
        + dual_locked
        + wash_freeze_locked
        + eng.state().pending_town_import_cred_spent
        + total_repaid
        + total_burned
}

#[test]
fn f5_f6_random_multi_table_cred_conservation_and_panic_free_sweep_property() {
    let mut rng = 0x5EED_F5F6_2026_1115_u64;
    let mut total_turns_swept: usize = 0;

    for table_idx in 0..24_u64 {
        let seat_count = 4 + (splitmix64(&mut rng) % 3) as u8;
        let tid = TableId(10_000 + table_idx);
        let mut spec = must_ok(
            TableSpec::standard_chiral_ring(tid, seat_count, table_idx + 1),
            "spec",
        );
        spec.seats[0].initial_cred += 5_000;
        let initial_cred: Units = spec.seats.iter().map(|s| s.initial_cred).sum();
        let mut eng = must_ok(TurnEngine::new(spec), "eng");
        let mut total_repaid: Units = 0;
        let mut total_burned: Units = 0;
        let mut next_pledge_id: u64 = 1;
        let mut known_secrets: Vec<(u64, PreimageSecret)> = Vec::new();

        for t in 1..=HARD_TURN_CAP {
            if eng.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            let n_actions = 4 + (splitmix64(&mut rng) % 6);
            for _ in 0..n_actions {
                let seat = (splitmix64(&mut rng) % u64::from(seat_count)) as u8;
                let pick = splitmix64(&mut rng) % 10;
                let action = match pick {
                    0 => MarketAction::PostOffer {
                        seat,
                        order: OfferProposal {
                            side: if splitmix64(&mut rng).is_multiple_of(2) {
                                OfferSide::Buy
                            } else {
                                OfferSide::Sell
                            },
                            good: if splitmix64(&mut rng).is_multiple_of(2) {
                                AssetId::Raw((splitmix64(&mut rng) % u64::from(seat_count)) as u8)
                            } else {
                                AssetId::Refined(
                                    (splitmix64(&mut rng) % u64::from(seat_count)) as u8,
                                )
                            },
                            amount: 500 + (splitmix64(&mut rng) % 4_000) as Units,
                            price_per_unit_cred: 500 + (splitmix64(&mut rng) % 3_000) as Units,
                            domain_id: if splitmix64(&mut rng).is_multiple_of(4) {
                                Some((splitmix64(&mut rng) % 4) as u8)
                            } else {
                                None
                            },
                            is_hybrid: splitmix64(&mut rng).is_multiple_of(5),
                        },
                    },
                    1 => MarketAction::CancelOffer {
                        seat,
                        offer_id: eng.state().offers.keys().next().copied().unwrap_or(99_999),
                    },
                    2 => {
                        let payee = (seat + 1) % seat_count;
                        let send_max = 200 + (splitmix64(&mut rng) % 3_000) as Units;
                        let deliver_min = send_max / 2;
                        MarketAction::CreateCheck {
                            seat,
                            check: CheckProposal {
                                payee,
                                asset: if splitmix64(&mut rng).is_multiple_of(3) {
                                    AssetId::Raw(seat)
                                } else {
                                    AssetId::Cred
                                },
                                send_max,
                                deliver_min,
                                due_turn: t,
                                expiration_turn: t.saturating_add(3),
                            },
                        }
                    }
                    3 => MarketAction::CancelCheck {
                        seat,
                        check_id: eng.state().checks.keys().next().copied().unwrap_or(99_999),
                    },
                    4 => {
                        let dest = (seat + 1 + (splitmix64(&mut rng) % 2) as u8) % seat_count;
                        let with_bond = splitmix64(&mut rng).is_multiple_of(2);
                        let with_secret = splitmix64(&mut rng).is_multiple_of(2);
                        let mut sec_bytes = [(splitmix64(&mut rng) as u8).max(1); 32];
                        sec_bytes[0] = (t ^ seat).max(1);
                        let secret = PreimageSecret(sec_bytes);
                        let cond_hash = if with_secret {
                            must_ok(compute_preimage_condition(&secret), "preimage").into()
                        } else {
                            None
                        };
                        if with_secret {
                            let next_id =
                                eng.state().escrows.keys().last().copied().unwrap_or(0) + 1;
                            known_secrets.push((next_id, secret));
                        }
                        MarketAction::CreateEscrow {
                            seat,
                            escrow: EscrowProposal {
                                destination: dest,
                                asset: AssetId::Cred,
                                amount: 200 + (splitmix64(&mut rng) % 2_000) as Units,
                                finish_after_turn: Some(t),
                                cancel_after_turn: t.saturating_add(2),
                                condition_hash: cond_hash,
                                bond: with_bond.then_some(BondProposal {
                                    good: AssetId::Raw(seat),
                                    required_units: 500,
                                    deadline_turn: t.saturating_add(1),
                                    mode: BondSettlementMode::SellerPerformanceBond,
                                }),
                            },
                        }
                    }
                    5 => MarketAction::CancelExpiredEscrow {
                        seat,
                        escrow_id: eng.state().escrows.keys().next().copied().unwrap_or(99_999),
                    },
                    6 => {
                        let pid = next_pledge_id;
                        next_pledge_id += 1;
                        MarketAction::RecordIntentPledge {
                            seat,
                            pledge: IntentPledge {
                                pledge_id: pid,
                                promisor: seat,
                                counterparty: (seat + 1) % seat_count,
                                target: IntentTarget::VoteForSeat {
                                    office_turn: ((t / 4) + 1) * 4,
                                    candidate: seat,
                                },
                                honor_weight: 25,
                                memo_digest: [pid as u8; 32],
                            },
                        }
                    }
                    7 => MarketAction::FlagWashFreeze {
                        inspector: seat,
                        target_seat: (seat + 1) % seat_count,
                    },
                    8 => MarketAction::StandForOffice {
                        seat,
                        platform: CouncilPlatform {
                            proposed_tariffs_bps: vec![(
                                (seat + 1) % seat_count,
                                (
                                    (splitmix64(&mut rng) % 4_500) as u16,
                                    (splitmix64(&mut rng) % 4_500) as u16,
                                ),
                            )],
                            cartel_admissions: vec![(
                                (splitmix64(&mut rng) % 3) as u8,
                                (seat + (splitmix64(&mut rng) % 2) as u8) % seat_count,
                            )],
                            cartel_revocations: vec![((splitmix64(&mut rng) % 3) as u8, 0)],
                        },
                    },
                    _ => MarketAction::PostOffer {
                        seat: 250,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(99),
                            amount: i64::MAX,
                            price_per_unit_cred: 1_000,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    },
                };
                let _ = eng.apply_market_action(action);
                assert_eq!(
                    table_cred_total(&eng, total_repaid, total_burned),
                    initial_cred
                );
            }

            // Build randomized Phase-2 commitments & reveals covering every bundle field plus
            // extreme debt_repayment_cred and random non-commits / committed non-reveals.
            must_ok(eng.advance_to_commit(), "commit");
            let active_checks: Vec<u64> = eng.state().checks.keys().copied().collect();
            let active_escrows: Vec<(u64, PreimageSecret)> = known_secrets.clone();
            let mut planned_bundles = Vec::with_capacity(usize::from(seat_count));
            let mut commits = Vec::with_capacity(usize::from(seat_count));
            for s in 0..seat_count {
                let skip_commit = splitmix64(&mut rng).is_multiple_of(9);
                if skip_commit {
                    planned_bundles.push(None);
                    commits.push(None);
                    continue;
                }
                let mut bundle = SealedOrderBundle::default();
                if splitmix64(&mut rng).is_multiple_of(2) {
                    bundle.converter_runs.push(ConverterRunOrder {
                        recipe_idx: (splitmix64(&mut rng) % 4) as usize,
                        runs: 1 + (splitmix64(&mut rng) % 6) as u32,
                    });
                }
                if splitmix64(&mut rng).is_multiple_of(3) {
                    bundle.project_deliveries.push(ProjectDeliveryOrder {
                        project_id: 1 + (splitmix64(&mut rng) % 105) as u16,
                        batches: 1 + (splitmix64(&mut rng) % 5) as u32,
                    });
                }
                if splitmix64(&mut rng).is_multiple_of(3) {
                    let taker = (s + 1) % seat_count;
                    bundle.atomic_swaps.push(AtomicSwapProposal {
                        swap_id: splitmix64(&mut rng),
                        maker: s,
                        taker,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 500,
                        taker_asset: AssetId::Cred,
                        taker_amount: 200,
                        domain_id: None,
                    });
                }
                if splitmix64(&mut rng).is_multiple_of(2) {
                    bundle.debt_repayment_cred = if splitmix64(&mut rng).is_multiple_of(11) {
                        i64::MAX
                    } else if splitmix64(&mut rng).is_multiple_of(2) {
                        -10
                    } else {
                        (splitmix64(&mut rng) % 5_000) as Units
                    };
                }
                if let Some(&(eid, ref sec)) = active_escrows.first()
                    && splitmix64(&mut rng).is_multiple_of(2)
                {
                    bundle.escrow_finishes.push(EscrowFinishUnlock {
                        escrow_id: eid,
                        preimage: Some(sec.clone()),
                    });
                }
                if let Some(&cid) = active_checks.first()
                    && splitmix64(&mut rng).is_multiple_of(2)
                {
                    bundle.check_cashes.push(cid);
                }
                if t.is_multiple_of(4) && !splitmix64(&mut rng).is_multiple_of(5) {
                    bundle.council_vote = Some(CouncilVoteBallot {
                        trade_minister: (splitmix64(&mut rng) % u64::from(seat_count + 1)) as u8,
                        cartel_chair: (splitmix64(&mut rng) % u64::from(seat_count + 1)) as u8,
                    });
                }
                if t.is_multiple_of(6) && !splitmix64(&mut rng).is_multiple_of(4) {
                    bundle.charter_bids.push(CharterBidOrder {
                        role: match splitmix64(&mut rng) % 3 {
                            0 => CharterRole::ClearinghouseBroker,
                            1 => CharterRole::CustomsInspector,
                            _ => CharterRole::OracleAuditor,
                        },
                        bid_cred: if splitmix64(&mut rng).is_multiple_of(13) {
                            i64::MAX
                        } else {
                            100 + (splitmix64(&mut rng) % 2_000) as Units
                        },
                    });
                }
                let ord_rev = OrderReveal {
                    bundle: bundle.clone(),
                    salt: RevealSalt([(t.wrapping_mul(17)).wrapping_add(s).max(1); 32]),
                };
                let ch = must_ok(
                    compute_order_commitment(tid, Turn(t), Seat(s), &ord_rev),
                    "commit",
                );
                must_ok(eng.submit_order_commitment(Seat(s), ch), "sub commit");
                planned_bundles.push(Some(ord_rev));
                commits.push(Some(ch));
            }
            must_ok(eng.advance_to_reveal(), "advance reveal");
            for (s_idx, rev_opt) in planned_bundles.into_iter().enumerate() {
                if let Some(rev) = rev_opt
                    && !splitmix64(&mut rng).is_multiple_of(10)
                {
                    must_ok(eng.submit_order_reveal(Seat(s_idx as u8), rev), "sub rev");
                }
            }

            let beacon = derive_turn_beacon(table_idx + 1, tid, Turn(t));
            let rec = must_ok(eng.sweep_turn(beacon), "sweep never panics or errors");
            total_turns_swept += 1;
            total_repaid += rec.cred_debt_repaid + rec.town_import_cred_spent;
            total_burned += rec.charter_cred_burned - rec.works_stipend_cred_minted;
            assert_eq!(
                table_cred_total(&eng, total_repaid, total_burned),
                initial_cred
            );
            if rec.season_closed {
                break;
            }
        }
    }
    assert!(total_turns_swept >= 24 * 24);
}

#[test]
fn f7_rich_season_golden_digest_is_exact_and_reproducible() {
    let tid = TableId(944);
    let seed = 42_u64;
    let salts = [
        RevealSalt([0x31; 32]),
        RevealSalt([0x32; 32]),
        RevealSalt([0x33; 32]),
        RevealSalt([0x34; 32]),
    ];
    let eval_seed_rich = must_ok(SealedEvalSeed::try_from_bytes([0x88; 32]), "eval_seed_rich");
    let (spec, sealed_deal, eval_seed_rich) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            tid,
            4,
            seed,
            [0x77; 32],
            &salts,
            eval_seed_rich,
        ),
        "rich explicit spec",
    );
    let eval_pk_rich = spec.eval_vrf_pubkey;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "eng");
    let mut turns = Vec::new();
    let mut live_digests = Vec::new();

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
            // Intentionally invalid refused Phase-1 action included in the log (A6):
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
        let (commitments, reveals) = commit_and_reveal_all(&mut eng, tid, t, bundles);
        let (beacon, prf) = must_ok(eval_seed_rich.prove_turn(tid, Turn(t)), "prove rich");
        let receipt = must_ok(eng.sweep_turn_with_eval_vrf(&prf), "sweep rich");
        live_digests.push(receipt.post_state_digest);
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
    let rev0 = sealed_deal.seat_reveals[0].clone();
    mandate_reveals[0] = Some(rev0.clone());
    must_ok(eng.submit_mandate_reveal(Seat(0), rev0), "rev0");
    let live_board = must_ok(eng.finalize_season(), "finalize rich");
    let live_terminal = must_ok(eng.state_digest(), "terminal digest");

    let log = SeasonReplayLog {
        spec,
        mode: TableMode::Eval,
        seed: must_ok(ReplayLayoutSeed::try_clear(seed), "clear seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk_rich,
        sealed_mandate_deal: sealed_deal,
        mandate_commits: vec![None; 4],
        turns,
        mandate_reveals,
    };
    let replayed = must_ok(replay_season(&log), "replay rich");
    assert_eq!(replayed.state_digests, live_digests);
    assert_eq!(replayed.scoreboard, live_board);

    let last_turn_digest = must_ok(live_digests.last().copied().ok_or("last digest"), "last");
    assert_eq!(
        hex::encode(last_turn_digest),
        "7eaf7769631a1fff3194a2ae396725bac10b3e1272b03a46655b4daae189a3e3"
    );
    assert_eq!(
        hex::encode(live_terminal),
        "e423d54509bab25800d5b6651f49a48a46e1b572f718a8eeae97f8ce0e8730ff"
    );
}

#[test]
fn addendum_game_vs_eval_mode_ties_and_replay_mode_lock() {
    use sha2::{Digest, Sha256};

    // 1. Validate DrandSchedule cadence minimum and required Game drand_schedule.
    let bad_period = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
        turn_period_secs: DRAND_QUICKNET_PERIOD_SECS - 1,
    };
    assert!(matches!(
        bad_period.validate(),
        Err(EngineError::InvalidConfig(_))
    ));

    let tid_eval = TableId(950);
    let schedule = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 300,
        turn_period_secs: DRAND_QUICKNET_PERIOD_SECS * 10,
    };

    let (mut missing_sched_spec, _) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(tid_eval, 4, 5),
        "base eval spec",
    );
    missing_sched_spec.mode = TableMode::Game;
    missing_sched_spec.drand_schedule = None;
    assert!(matches!(
        missing_sched_spec.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    assert!(matches!(
        TurnEngine::new(missing_sched_spec),
        Err(EngineError::InvalidConfig(_))
    ));

    // Helper running a 4-seat season with distinct StandForOffice platforms for seats 1 & 2 on Turn 3
    // and a 2-2 plurality tie between seats 1 & 2 on Turn 4.
    let run_eval_tie_season = |eval_seed_bytes: [u8; 32]| {
        let eval_seed = must_ok(
            SealedEvalSeed::try_from_bytes(eval_seed_bytes),
            "eval_seed_bytes",
        );
        let (mut spec, deal) = must_ok(
            TableSpec::standard_chiral_ring_with_mandates(tid_eval, 4, 5),
            "eval spec",
        );
        let eval_pk = must_ok(eval_seed.public_key_bytes(), "eval pk");
        spec.eval_vrf_pubkey = Some(eval_pk);
        assert_eq!(spec.mode, TableMode::Eval);
        let mut eng = must_ok(TurnEngine::new(spec.clone()), "eval eng");
        let mut turns = Vec::new();
        let mut live_digests = Vec::new();
        let mut t4_elected = None;
        let mut t4_beacon = [0u8; 32];

        for t in 1..=HARD_TURN_CAP {
            if eng.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            let mut actions = Vec::new();
            let mut bundles = vec![SealedOrderBundle::default(); 4];
            if t == 3 {
                actions.push(MarketAction::StandForOffice {
                    seat: 1,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (1_500, 1_100))],
                        cartel_admissions: vec![(4, 1)],
                        cartel_revocations: Vec::new(),
                    },
                });
                actions.push(MarketAction::StandForOffice {
                    seat: 2,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (2_100, 1_700))],
                        cartel_admissions: vec![(5, 2)],
                        cartel_revocations: Vec::new(),
                    },
                });
            }
            if t == 4 {
                bundles[0].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 1,
                    cartel_chair: 1,
                });
                bundles[1].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 1,
                    cartel_chair: 1,
                });
                bundles[2].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 2,
                    cartel_chair: 2,
                });
                bundles[3].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 2,
                    cartel_chair: 2,
                });
            }
            for action in &actions {
                must_ok(eng.apply_market_action(action.clone()), "eval action");
            }
            let (commitments, reveals) = commit_and_reveal_all(&mut eng, tid_eval, t, bundles);
            if t == 1 {
                let wrong_sig = [0x55_u8; DRAND_QUICKNET_SIGNATURE_BYTES];
                let wrong_round = must_ok(schedule.expected_round_for_turn(Turn(1)), "r1");
                assert!(matches!(
                    eng.sweep_turn_with_drand(wrong_round, &wrong_sig),
                    Err(EngineError::InvalidConfig(_))
                ));
            }
            let (beacon, prf) = must_ok(eval_seed.prove_turn(tid_eval, Turn(t)), "prove");
            let r = must_ok(eng.sweep_turn_with_eval_vrf(&prf), "sweep eval vrf");
            if t == 4 {
                t4_elected = r.elected_officers;
                t4_beacon = beacon;
            }
            live_digests.push(r.post_state_digest);
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
                drawn_shocks: r.newly_drawn_shocks,
            });
            if r.season_closed {
                break;
            }
        }
        let board = must_ok(eng.finalize_season(), "finalize eval");
        let elected_tm_tariff = eng.state().seats[0].raw_tariff_bps;
        let domain_ids: Vec<u8> = eng.state().domains.iter().map(|d| d.domain_id).collect();
        let log = SeasonReplayLog {
            spec,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(5), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: Some(eval_pk),
            sealed_mandate_deal: deal,
            mandate_commits: vec![None; 4],
            turns,
            mandate_reveals: vec![None; 4],
        };
        (
            log,
            live_digests,
            board,
            t4_elected,
            t4_beacon,
            elected_tm_tariff,
            domain_ids,
        )
    };

    // 2. Deterministic Eval tie + independent OS mandate deal entropy + replay digest match.
    let beacon_seed = 404_u64;
    let eval_seed_bytes = [0x4D_u8; 32];
    let (
        eval_log_a,
        eval_digests_a,
        eval_board_a,
        eval_t4_a,
        eval_t4_beacon,
        eval_tariff_a,
        eval_domains_a,
    ) = run_eval_tie_season(eval_seed_bytes);
    let (eval_log_b, _eval_digests_b, _eval_board_b, eval_t4_b, _, eval_tariff_b, eval_domains_b) =
        run_eval_tie_season(eval_seed_bytes);

    let expected_eval_tm = break_tie_with_turn_beacon(
        eval_t4_beacon,
        tid_eval,
        Turn(4),
        b"trade_minister",
        &[1, 2],
    );
    let expected_eval_cc = Some(if expected_eval_tm == 1 { 2 } else { 1 });
    assert_eq!(eval_t4_a, Some((expected_eval_tm, expected_eval_cc)));
    assert_eq!(eval_t4_a, eval_t4_b);
    assert_eq!(eval_tariff_a, eval_tariff_b);
    assert_eq!(eval_domains_a, eval_domains_b);
    assert_ne!(
        eval_log_a.sealed_mandate_deal.deal_seed,
        eval_log_b.sealed_mandate_deal.deal_seed
    );
    assert_ne!(
        eval_log_a.spec.dealt_mandate_commits,
        eval_log_b.spec.dealt_mandate_commits
    );

    let replayed_eval = must_ok(replay_season(&eval_log_a), "replay eval");
    assert_eq!(replayed_eval.state_digests, eval_digests_a);
    assert_eq!(replayed_eval.scoreboard, eval_board_a);

    // M3 independence check: deal_seed == SHA-256(layout_seed) is refused by replay_season.
    let coupled_deal_seed: [u8; 32] = Sha256::digest(beacon_seed.to_le_bytes()).into();
    let explicit_salts = [
        RevealSalt([0x11; 32]),
        RevealSalt([0x22; 32]),
        RevealSalt([0x33; 32]),
        RevealSalt([0x44; 32]),
    ];
    let coupled_eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x5E; 32]), "coupled eval");
    let (coupled_spec, coupled_deal, coupled_eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            TableId(952),
            4,
            beacon_seed,
            coupled_deal_seed,
            &explicit_salts,
            coupled_eval_seed,
        ),
        "coupled spec",
    );
    let coupled_pk = coupled_spec.eval_vrf_pubkey;
    let mut eng_coupled = must_ok(TurnEngine::new(coupled_spec.clone()), "coupled eng");
    let mut coupled_turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng_coupled.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) = commit_and_reveal_all(
            &mut eng_coupled,
            TableId(952),
            t,
            vec![SealedOrderBundle::default(); 4],
        );
        let (b, prf) = must_ok(coupled_eval_seed.prove_turn(TableId(952), Turn(t)), "prf");
        let r = must_ok(eng_coupled.sweep_turn_with_eval_vrf(&prf), "sweep coupled");
        coupled_turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: b,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(prf.to_vec()),
            drawn_shocks: r.newly_drawn_shocks,
        });
        if r.season_closed {
            break;
        }
    }
    let coupled_log = SeasonReplayLog {
        spec: coupled_spec,
        mode: TableMode::Eval,
        seed: must_ok(ReplayLayoutSeed::try_clear(beacon_seed), "clear seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: coupled_pk,
        sealed_mandate_deal: coupled_deal,
        mandate_commits: vec![None; 4],
        turns: coupled_turns,
        mandate_reveals: vec![None; 4],
    };
    assert!(matches!(
        replay_season(&coupled_log),
        Err(EngineError::InvalidConfig(_))
    ));

    // Required serde field: stripping "mode" from serialized TableSpec or SeasonReplayLog must fail.
    let mut spec_json = must_ok(serde_json::to_value(&eval_log_a.spec), "spec json");
    if let Some(obj) = spec_json.as_object_mut() {
        obj.remove("mode");
    }
    assert!(serde_json::from_value::<TableSpec>(spec_json).is_err());

    let mut log_json = must_ok(serde_json::to_value(&eval_log_a), "log json");
    if let Some(obj) = log_json.as_object_mut() {
        obj.remove("mode");
    }
    assert!(serde_json::from_value::<SeasonReplayLog>(log_json).is_err());

    // 3. Deterministic Game tie using drand-signature-bound beacons.
    let tid_game = TableId(951);
    let run_game_tie_season = || {
        let (spec, deal) = must_ok(
            TableSpec::standard_game_chiral_ring_with_mandates(tid_game, 4, 5, schedule),
            "game spec",
        );
        assert_eq!(spec.mode, TableMode::Game);
        let mut eng = must_ok(TurnEngine::new(spec.clone()), "game eng");
        let mut turns = Vec::new();
        let mut live_digests = Vec::new();
        let mut t4_elected = None;
        let mut t4_sig = Vec::new();

        for t in 1..=HARD_TURN_CAP {
            if eng.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            let mut actions = Vec::new();
            let mut bundles = vec![SealedOrderBundle::default(); 4];
            if t == 3 {
                actions.push(MarketAction::StandForOffice {
                    seat: 1,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (1_500, 1_100))],
                        cartel_admissions: vec![(4, 1)],
                        cartel_revocations: Vec::new(),
                    },
                });
                actions.push(MarketAction::StandForOffice {
                    seat: 2,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(0, (2_100, 1_700))],
                        cartel_admissions: vec![(5, 2)],
                        cartel_revocations: Vec::new(),
                    },
                });
            }
            if t == 4 {
                bundles[0].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 1,
                    cartel_chair: 1,
                });
                bundles[1].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 1,
                    cartel_chair: 1,
                });
                bundles[2].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 2,
                    cartel_chair: 2,
                });
                bundles[3].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 2,
                    cartel_chair: 2,
                });
            }
            for action in &actions {
                must_ok(eng.apply_market_action(action.clone()), "game action");
            }
            let (commitments, reveals) = commit_and_reveal_all(&mut eng, tid_game, t, bundles);
            if t == 1 {
                assert!(matches!(
                    eng.sweep_turn(derive_turn_beacon(beacon_seed, tid_game, Turn(1))),
                    Err(EngineError::InvalidConfig(_))
                ));
                assert!(matches!(
                    eng.sweep_turn_seeded(beacon_seed),
                    Err(EngineError::InvalidConfig(_))
                ));
            }
            let round = must_ok(schedule.expected_round_for_turn(Turn(t)), "round");
            let sig = vec![0x71_u8.wrapping_add(t); DRAND_QUICKNET_SIGNATURE_BYTES];
            let beacon = must_ok(derive_drand_beacon(&sig), "beacon");
            let r = must_ok(eng.sweep_turn_with_drand(round, &sig), "sweep drand");
            if t == 4 {
                t4_elected = r.elected_officers;
                t4_sig = sig.clone();
            }
            live_digests.push(r.post_state_digest);
            turns.push(LoggedTurn {
                turn: t,
                market_actions: actions,
                cancel_lock_at: None,
                commitments,
                reveals,
                turn_beacon: beacon,
                drand_round: Some(round),
                drand_signature: Some(sig),
                eval_vrf_proof: None,
                drawn_shocks: r.newly_drawn_shocks,
            });
            if r.season_closed {
                break;
            }
        }
        let board = must_ok(eng.finalize_season(), "finalize game");
        let log = SeasonReplayLog {
            spec,
            mode: TableMode::Game,
            seed: must_ok(ReplayLayoutSeed::try_clear(0), "clear seed"),
            beacon_mode: ReplayBeaconMode::ExternalDrand,
            eval_vrf_pubkey: None,
            sealed_mandate_deal: deal,
            mandate_commits: vec![None; 4],
            turns,
            mandate_reveals: vec![None; 4],
        };
        (log, live_digests, board, t4_elected, t4_sig)
    };

    let (game_log_a, game_digests_a, game_board_a, game_t4_a, sig_4) = run_game_tie_season();
    let (_game_log_b, _game_digests_b, _game_board_b, game_t4_b, _) = run_game_tie_season();
    let drand_t4_beacon = must_ok(derive_drand_beacon(&sig_4), "drand t4 beacon");
    let expected_game_tm = break_tie_with_turn_beacon(
        drand_t4_beacon,
        tid_game,
        Turn(4),
        b"trade_minister",
        &[1, 2],
    );
    let expected_game_cc = Some(if expected_game_tm == 1 { 2 } else { 1 });
    assert_eq!(game_t4_a, Some((expected_game_tm, expected_game_cc)));
    assert_eq!(game_t4_a, game_t4_b);

    let replayed_game = must_ok(replay_season(&game_log_a), "replay game");
    assert_eq!(replayed_game.state_digests, game_digests_a);
    assert_eq!(replayed_game.scoreboard, game_board_a);

    // 4. Refuse Game-mode logs carrying seeded beacons or unbound beacons.
    let mut game_seeded_mode = game_log_a.clone();
    game_seeded_mode.beacon_mode = ReplayBeaconMode::SeedDerived;
    game_seeded_mode.seed = must_ok(ReplayLayoutSeed::try_clear(beacon_seed), "clear seed");
    game_seeded_mode.turns[0].turn_beacon = derive_turn_beacon(beacon_seed, tid_game, Turn(1));
    assert!(matches!(
        replay_season(&game_seeded_mode),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut game_missing_sig_seeded = game_log_a.clone();
    game_missing_sig_seeded.turns[0].drand_signature = None;
    game_missing_sig_seeded.turns[0].turn_beacon =
        derive_turn_beacon(beacon_seed, tid_game, Turn(1));
    assert!(matches!(
        replay_season(&game_missing_sig_seeded),
        Err(EngineError::SeasonEnded(_))
    ));

    let mut game_unbound_seeded_beacon = game_log_a.clone();
    game_unbound_seeded_beacon.turns[0].turn_beacon =
        derive_turn_beacon(beacon_seed, tid_game, Turn(1));
    assert!(matches!(
        replay_season(&game_unbound_seeded_beacon),
        Err(EngineError::InvalidOrder(_))
    ));

    // 5. Refuse mode mismatches between TableSpec and SeasonReplayLog (both directions), plus
    // drand metadata injected into an Eval log.
    let mut switched_to_eval = game_log_a;
    switched_to_eval.mode = TableMode::Eval;
    switched_to_eval.beacon_mode = ReplayBeaconMode::EvalVrf;
    assert!(matches!(
        replay_season(&switched_to_eval),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut switched_to_game = eval_log_a.clone();
    switched_to_game.mode = TableMode::Game;
    switched_to_game.beacon_mode = ReplayBeaconMode::ExternalDrand;
    assert!(matches!(
        replay_season(&switched_to_game),
        Err(EngineError::InvalidConfig(_))
    ));

    let mut eval_with_drand = eval_log_a;
    eval_with_drand.turns[0].drand_round = schedule.expected_round_for_turn(Turn(1)).ok();
    assert!(matches!(
        replay_season(&eval_with_drand),
        Err(EngineError::InvalidOrder(_))
    ));
}

#[test]
fn c1a_three_zero_production_turns_hand_seat_to_regent() {
    let tid = TableId(960);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 3), "spec");
    for s in &mut spec.seats {
        s.perishable_decay_bps = 0;
    }
    let mut eng = must_ok(TurnEngine::new(spec), "new");

    // Turn 1: seats start with CRED only (`types.rs:495`), so Turn 1 does not count as idle (`A-2`).
    commit_and_reveal_all(&mut eng, tid, 1, vec![SealedOrderBundle::default(); 4]);
    let r1 = must_ok(eng.sweep_turn_seeded(3), "sweep 1");
    assert!(!r1.regent_acted_seats.contains(&0));
    assert!(!r1.converted_to_ai.contains(&0));
    assert_eq!(eng.state().seats[0].consecutive_zero_production_turns, 0);

    // Turn 2: Seats 1 & 2 swap Raw(1) & Raw(2) with Seat 0 (active swap delivery resets counter to 0).
    let mut b2 = vec![SealedOrderBundle::default(); 4];
    for (idx, partner) in [1_u8, 2].into_iter().enumerate() {
        let sw = AtomicSwapProposal {
            swap_id: 9600 + idx as u64,
            maker: partner,
            taker: 0,
            maker_asset: AssetId::Raw(partner),
            maker_amount: 6_000,
            taker_asset: AssetId::Cred,
            taker_amount: 200,
            domain_id: None,
        };
        b2[0].atomic_swaps.push(sw.clone());
        b2[usize::from(partner)].atomic_swaps.push(sw);
    }
    commit_and_reveal_all(&mut eng, tid, 2, b2);
    let r2 = must_ok(eng.sweep_turn_seeded(3), "sweep 2");
    assert!(!r2.regent_acted_seats.contains(&0));
    assert!(!r2.converted_to_ai.contains(&0));
    assert_eq!(eng.state().seats[0].consecutive_zero_production_turns, 0);
    assert_eq!(eng.state().seats[0].refined[0], 0);

    // Turns 3 and 4: Seat 0 submits valid empty bundles with zero converters, zero deliveries,
    // zero batch fills, and zero works -> consecutive_zero_production_turns hits 1 then 2.
    for t in [3_u8, 4_u8] {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let r = must_ok(eng.sweep_turn_seeded(3), "sweep 3/4");
        assert!(!r.regent_acted_seats.contains(&0));
        assert!(!r.converted_to_ai.contains(&0));
        assert_eq!(
            eng.state().seats[0].consecutive_zero_production_turns,
            t - 2
        );
    }

    // Turn 5: Seat 0's 3rd consecutive truly idle turn -> Regent takes over and converts Human -> AiSyndicate.
    commit_and_reveal_all(&mut eng, tid, 5, vec![SealedOrderBundle::default(); 4]);
    let r5 = must_ok(eng.sweep_turn_seeded(3), "sweep 5");
    assert!(
        r5.regent_acted_seats.contains(&0),
        "expected Regent takeover on 3rd consecutive idle turn"
    );
    assert!(
        r5.converted_to_ai.contains(&0),
        "expected Human -> AiSyndicate conversion on 3rd idle turn"
    );
    assert_eq!(eng.state().seats[0].identity, SeatIdentity::AiSyndicate);
    assert!(eng.state().seats[0].refined[0] > 0);
}

#[test]
fn c4_phase1_batch_clearing_is_arrival_order_invariant_and_beacon_tie_broken() {
    let tid = TableId(961);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 7), "spec");
    for seat in &mut spec.seats {
        seat.regent = None;
        seat.perishable_decay_bps = 0;
        seat.initial_raw_tariff_bps = 0;
    }
    let mut base_eng = must_ok(TurnEngine::new(spec), "new");
    commit_and_reveal_all(&mut base_eng, tid, 1, vec![SealedOrderBundle::default(); 4]);
    must_ok(base_eng.sweep_turn_seeded(7), "sweep 1");

    // Same two crossing orders submitted in opposite arrival orders:
    // Order A: Seat 0 buys 1_000 Raw(1) up to 1_500 CRED/unit.
    // Order B: Seat 1 sells 1_000 Raw(1) down to 1_000 CRED/unit.
    let buy_0 = MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Buy,
            good: AssetId::Raw(1),
            amount: 1_000,
            price_per_unit_cred: 1_500,
            domain_id: None,
            is_hybrid: false,
        },
    };
    let sell_1 = MarketAction::PostOffer {
        seat: 1,
        order: OfferProposal {
            side: OfferSide::Sell,
            good: AssetId::Raw(1),
            amount: 1_000,
            price_per_unit_cred: 1_000,
            domain_id: None,
            is_hybrid: false,
        },
    };

    let mut eng_ab = base_eng.clone();
    must_ok(eng_ab.apply_market_action(buy_0.clone()), "ab buy");
    must_ok(eng_ab.apply_market_action(sell_1.clone()), "ab sell");
    commit_and_reveal_all(&mut eng_ab, tid, 2, vec![SealedOrderBundle::default(); 4]);
    let r_ab = must_ok(eng_ab.sweep_turn_seeded(7), "sweep ab");

    let mut eng_ba = base_eng;
    must_ok(eng_ba.apply_market_action(sell_1), "ba sell");
    must_ok(eng_ba.apply_market_action(buy_0), "ba buy");
    commit_and_reveal_all(&mut eng_ba, tid, 2, vec![SealedOrderBundle::default(); 4]);
    let r_ba = must_ok(eng_ba.sweep_turn_seeded(7), "sweep ba");

    assert_eq!(
        eng_ab.state().seats[1].cred,
        eng_ba.state().seats[1].cred,
        "seller CRED must be arrival-order invariant under batch clearing"
    );
    assert_eq!(r_ab.post_state_digest, r_ba.post_state_digest);
    // Uniform midpoint clearing price = (1_000 + 1_500) / 2 = 1_250 CRED/unit -> 1_250 CRED traded.
    assert_eq!(eng_ab.state().seats[0].raw[1], 1_000);
    assert_eq!(eng_ab.state().seats[0].cred, 15_200 - 1_250);
    assert_eq!(eng_ab.state().seats[1].cred, 15_200 + 1_250);
}

#[test]
fn b2_single_mandate_deck_rejects_low_credit_tiers() {
    for tier in [CreditTier::B, CreditTier::D] {
        assert!(
            MandateKind::MaintainCleanCredit { min_tier: tier }
                .validate_for_seat_count(4)
                .is_err(),
            "MaintainCleanCredit below Bbb ({tier:?}) must be rejected"
        );
    }
    for tier in [CreditTier::Aaa, CreditTier::Bbb] {
        assert!(
            MandateKind::MaintainCleanCredit { min_tier: tier }
                .validate_for_seat_count(4)
                .is_ok()
        );
    }
}

#[test]
fn c1b_town_import_fallback_runs_works_when_supplier_idles_and_pays_premium() {
    let (mut spec, _deal) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(TableId(960), 4, 1),
        "spec",
    );
    spec.seats[0].initial_cred = 20_000;
    let mut engine = must_ok(TurnEngine::new(spec.clone()), "engine");

    // Seat 0 needs Raw(1) and Raw(2) (2_000 each) for primary converter -> Refined(0),
    // and Refined(1) (1_000) + Refined(0) (1_000) to test fallback purchase.
    // Suppliers idle; Seat 0 uses TownImport at the configured premium (22_500 bps = 2.25x).
    assert_eq!(
        spec.town_import_premium_bps,
        DEFAULT_TOWN_IMPORT_PREMIUM_BPS
    );
    let raw1_parity = engine.state().parities.asset_parity(AssetId::Raw(1));
    let expected_cost_each = must_ok(
        town_import_cost_cred(2_000, raw1_parity, DEFAULT_TOWN_IMPORT_PREMIUM_BPS),
        "town import cost",
    );
    assert_eq!(expected_cost_each, 7_875);

    assert!(
        engine
            .apply_market_action(MarketAction::TownImport {
                seat: 0,
                good: AssetId::Cred,
                amount: 1_000,
            })
            .is_err(),
        "TownImport must reject Cred as target asset"
    );
    assert!(
        engine
            .apply_market_action(MarketAction::TownImport {
                seat: 0,
                good: AssetId::Raw(1),
                amount: 0,
            })
            .is_err(),
        "TownImport must reject non-positive amount"
    );

    // Note Seat 0 starts with 15_000+ CRED so buying both costs 15_750 unless we test 1_000 each first;
    // actually let's test 1_000 Raw(1) + 1_000 Raw(2) if 15_750 > initial_cred, or test two tables/seats.
    // Wait: Seat 0 starts with 15_100 CRED (spread_mod=1 -> 15_100), so let's import 1_000 each cleanly!
    // No — converter requires 2_000 each! Let's import 1_000 by Seat 0 + top up or use Seat 0 buying 2_000 Raw(1)
    // and let Seat 1 Transfer / Buy? Wait: let's check cred cost of 2_000: 7_875 * 2 = 15_750 > 15_100!
    // Instead, give Seat 0 a 2_000 CRED gift via escrow/swap OR have Seat 0 start with initial_cred or custom spec!
    let cred_before = engine.state().seats[0].cred;
    must_ok(
        engine.apply_market_action(MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(1),
            amount: 2_000,
        }),
        "town import raw 1",
    );
    must_ok(
        engine.apply_market_action(MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(2),
            amount: 2_000,
        }),
        "town import raw 2",
    );
    assert_eq!(
        cred_before - engine.state().seats[0].cred,
        2 * expected_cost_each
    );
    assert_eq!(engine.state().seats[0].raw[1], 2_000);
    assert_eq!(engine.state().seats[0].raw[2], 2_000);

    // Seat 0 can now convert those town-imported raw inputs into Refined(0) on Turn 1.
    let mut bundles = vec![SealedOrderBundle::default(); 4];
    bundles[0].converter_runs.push(ConverterRunOrder {
        recipe_idx: 0,
        runs: 1,
    });
    commit_and_reveal_all(&mut engine, TableId(960), 1, bundles);
    let receipt = must_ok(engine.sweep_turn_seeded(1), "sweep turn 1");
    assert_eq!(receipt.town_import_cred_spent, 2 * expected_cost_each);
    assert_eq!(engine.state().seats[0].refined[0], 3_500);
}

#[test]
fn c2_buyer_dvp_and_cosigned_dual_escrow_delivery_defaults_and_lock_cap() {
    let (spec, _deal) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(TableId(961), 4, 1),
        "spec",
    );
    let mut engine = must_ok(TurnEngine::new(spec), "engine");
    // Turn 1: disable Regents by clearing regent configs on a clean copy or simply act cleanly
    // with explicit SealedOrderBundles. Give Seat 1 some Raw(1) -> already has 0, so yield on Turn 1 gives 10_000.
    // First test BuyerDvpPayment delivery (deadline Turn 2) & default (deadline Turn 3).
    let buyer_cred_t1 = engine.state().seats[0].cred;
    must_ok(
        engine.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 4,
                condition_hash: None,
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 1_000,
                    deadline_turn: 2,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            },
        }),
        "create BuyerDvpPayment escrow 1",
    );
    must_ok(
        engine.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 5,
                condition_hash: None,
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 1_000,
                    deadline_turn: 3,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            },
        }),
        "create BuyerDvpPayment escrow 2",
    );
    assert_eq!(buyer_cred_t1 - engine.state().seats[0].cred, 2_000);

    // Reject DualEscrowProposal exceeding <= 100% combined lock (buyer + seller > payment_cred).
    let mut bad_bundle_0 = SealedOrderBundle::default();
    bad_bundle_0.dual_escrows.push(DualEscrowProposal {
        proposal_id: 9001,
        buyer: 0,
        seller: 1,
        good: AssetId::Raw(1),
        required_units: 500,
        payment_cred: 1_000,
        buyer_bond_cred: 600,
        seller_bond_cred: 500, // 600 + 500 = 1_100 > 1_000 (exceeds 100% combined cap)
        deadline_turn: 3,
        domain_id: None,
    });
    commit_and_reveal_all(
        &mut engine,
        TableId(961),
        1,
        vec![
            SealedOrderBundle::default(),
            SealedOrderBundle::default(),
            SealedOrderBundle::default(),
            SealedOrderBundle::default(),
        ],
    );
    must_ok(engine.sweep_turn_seeded(1), "sweep turn 1");

    // Advance to Reveal on Turn 2 with an over-capped dual escrow -> must return InvalidOrder.
    must_ok(engine.advance_to_commit(), "commit t2");
    must_ok(
        engine.submit_order_commitment(
            Seat(0),
            must_ok(
                compute_order_commitment(
                    TableId(961),
                    Turn(2),
                    Seat(0),
                    &OrderReveal {
                        bundle: bad_bundle_0.clone(),
                        salt: RevealSalt([0x11; 32]),
                    },
                ),
                "ch0",
            ),
        ),
        "c0",
    );
    must_ok(
        engine.submit_order_commitment(
            Seat(1),
            must_ok(
                compute_order_commitment(
                    TableId(961),
                    Turn(2),
                    Seat(1),
                    &OrderReveal {
                        bundle: bad_bundle_0.clone(),
                        salt: RevealSalt([0x12; 32]),
                    },
                ),
                "ch1",
            ),
        ),
        "c1",
    );
    must_ok(engine.advance_to_reveal(), "reveal t2");
    assert!(
        matches!(
            engine.submit_order_reveal(
                Seat(0),
                OrderReveal {
                    bundle: bad_bundle_0,
                    salt: RevealSalt([0x11; 32]),
                },
            ),
            Err(EngineError::InvalidOrder(_))
        ),
        "co-signed DualEscrow exceeding 100% combined lock cap must be rejected"
    );

    // Build fresh Turn 2 engine where Seat 1 delivers 1_000 Raw(1) -> frees BuyerDvpPayment #1 on Turn 2,
    // plus opens a valid 50%/50% co-signed DualEscrow (#77, 1_000 total lock = 500 buyer + 500 seller <= 1_000),
    // and then leaves BuyerDvpPayment #2 unfulfilled so #2 refunds to Buyer on Turn 3 (no seller windfall).
    let (spec2, _) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(TableId(962), 4, 1),
        "spec2",
    );
    let mut eng2 = must_ok(TurnEngine::new(spec2), "eng2");
    must_ok(
        eng2.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 4,
                condition_hash: None,
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 1_000,
                    deadline_turn: 2,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            },
        }),
        "dvp 1",
    );
    must_ok(
        eng2.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: 1_000,
                finish_after_turn: None,
                cancel_after_turn: 5,
                condition_hash: None,
                bond: Some(BondProposal {
                    good: AssetId::Raw(1),
                    required_units: 2_500,
                    deadline_turn: 3,
                    mode: BondSettlementMode::BuyerDvpPayment,
                }),
            },
        }),
        "dvp 2",
    );
    commit_and_reveal_all(
        &mut eng2,
        TableId(962),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(eng2.sweep_turn_seeded(1), "sweep t1 eng2");

    // Turn 2: Seat 1 delivers 1_012 net Raw(1) fulfilling dvp 1 (1_000) while leaving dvp 2 (2_500)
    // and newly opened DualEscrow 77 (2_500) unfulfilled.
    let dual_77 = DualEscrowProposal {
        proposal_id: 77,
        buyer: 0,
        seller: 1,
        good: AssetId::Raw(1),
        required_units: 2_500,
        payment_cred: 1_000,
        buyer_bond_cred: 500,
        seller_bond_cred: 500,
        deadline_turn: 3,
        domain_id: None,
    };
    let seller_cred_before_t2 = eng2.state().seats[1].cred;
    let mut t2_bundles = vec![SealedOrderBundle::default(); 4];
    t2_bundles[0].dual_escrows.push(dual_77.clone());
    t2_bundles[1].dual_escrows.push(dual_77);
    t2_bundles[1]
        .contract_deliveries
        .push(ContractDeliveryOrder {
            counterparty: 0,
            good: AssetId::Raw(1),
            gross_amount: 1_100, // net after 800 bps raw tariff = 1_100 - 88 = 1_012 >= 1_000 required
            domain_id: None,
            dual_escrow_id: None,
        });
    commit_and_reveal_all(&mut eng2, TableId(962), 2, t2_bundles);
    must_ok(eng2.sweep_turn_seeded(2), "sweep t2 eng2");

    // Verify BuyerDvpPayment #1 released 1_000 CRED to Seat 1 while Seat 1 locked 500 CRED for DualEscrow 77.
    assert_eq!(
        eng2.state().seats[1].cred,
        seller_cred_before_t2 + 1_000 - 500,
        "Seller should receive 1_000 CRED DvP payment on delivery and lock 500 CRED for DualEscrow 77"
    );
    assert_eq!(eng2.state().dual_escrows.len(), 1);

    // Turn 3: Seller does NOT deliver for DvP #2 or DualEscrow 77 -> BOTH default on Turn 3 sweep!
    let buyer_cred_before_t3 = eng2.state().seats[0].cred;
    commit_and_reveal_all(
        &mut eng2,
        TableId(962),
        3,
        vec![SealedOrderBundle::default(); 4],
    );
    let r3 = must_ok(eng2.sweep_turn_seeded(3), "sweep t3 eng2");
    assert!(!r3.escrows_cancelled_or_forfeited.is_empty());
    assert!(r3.fills.iter().any(|f| {
        f.kind == FillKind::EscrowBreachRefunded
            && f.good == Some(AssetId::Cred)
            && f.from == Some(0)
            && f.to == 0
            && f.units == 1_000
            && f.net_units == 1_000
            && f.cred_amount == 1_000
    }));
    assert!(r3.fills.iter().any(|f| {
        f.kind == FillKind::EscrowForfeited
            && f.good == Some(AssetId::Cred)
            && f.from == Some(1)
            && f.to == 0
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert!(r3.fills.iter().any(|f| {
        f.kind == FillKind::EscrowRefunded
            && f.good == Some(AssetId::Cred)
            && f.from == Some(0)
            && f.to == 0
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert!(
        eng2.state().seats[1].bond_breaches >= 1,
        "defaulted Seller on DvP/DualEscrow must record bond breach"
    );
    // Buyer receives back dvp #2 payment (1_000) + DualEscrow 77 buyer_bond_cred (500) + forfeited seller_bond_cred (500) = 2_000!
    assert_eq!(
        eng2.state().seats[0].cred,
        buyer_cred_before_t3 + 2_000,
        "Buyer must recover unfunded DvP #2 plus both DualEscrow bonds on seller default"
    );

    // Turn 4: Co-signed DualEscrow (#78, 50% + 50% = 100% combined lock) WITH successful Seller ContractDeliveryOrder.
    let expected_eid_78 = eng2.state().next_escrow_id;
    let dual_78 = DualEscrowProposal {
        proposal_id: 78,
        buyer: 0,
        seller: 1,
        good: AssetId::Raw(1),
        required_units: 1_000,
        payment_cred: 1_000,
        buyer_bond_cred: 500,
        seller_bond_cred: 500,
        deadline_turn: 4,
        domain_id: None,
    };
    let buyer_cred_t4 = eng2.state().seats[0].cred;
    let seller_cred_t4 = eng2.state().seats[1].cred;
    let mut t4_bundles = vec![SealedOrderBundle::default(); 4];
    t4_bundles[0].dual_escrows.push(dual_78.clone());
    t4_bundles[1].dual_escrows.push(dual_78);
    t4_bundles[1]
        .contract_deliveries
        .push(ContractDeliveryOrder {
            counterparty: 0,
            good: AssetId::Raw(1),
            gross_amount: 1_100, // after 800 bps raw tariff = 1_100 - 88 = 1_012 >= 1_000 required
            domain_id: None,
            dual_escrow_id: Some(expected_eid_78),
        });
    commit_and_reveal_all(&mut eng2, TableId(962), 4, t4_bundles);
    let r4 = must_ok(eng2.sweep_turn_seeded(4), "sweep t4 eng2");
    assert!(r4.escrows_finished.contains(&expected_eid_78));
    assert_eq!(
        eng2.state().dual_escrows[&expected_eid_78].status,
        EscrowStatus::Finished
    );
    assert_eq!(eng2.state().seats[0].cred, buyer_cred_t4 - 1_000);
    assert_eq!(eng2.state().seats[1].cred, seller_cred_t4 + 1_000);

    // Buyer default test: Buyer locks 500 bond on DualEscrow (payment=1_000), leaving only 200 free CRED (< 500 top-up);
    // Seller attempts ContractDeliveryOrder tied to the DualEscrow -> Buyer defaults, forfeiting 500 buyer_bond_cred to Seller
    // while Seller keeps its commodity!
    let (mut spec3, _) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(TableId(966), 4, 1),
        "spec3",
    );
    spec3.seats[0].initial_cred = 700;
    let mut eng3 = must_ok(TurnEngine::new(spec3), "eng3");
    commit_and_reveal_all(
        &mut eng3,
        TableId(966),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(eng3.sweep_turn_seeded(1), "sweep t1 eng3");

    let expected_eid_88 = eng3.state().next_escrow_id;
    let dual_88 = DualEscrowProposal {
        proposal_id: 88,
        buyer: 0,
        seller: 1,
        good: AssetId::Raw(1),
        required_units: 1_000,
        payment_cred: 1_000,
        buyer_bond_cred: 500,
        seller_bond_cred: 500,
        deadline_turn: 2,
        domain_id: None,
    };
    let seller_raw1_before = eng3.state().seats[1].raw[1];
    let seller_cred_before_def = eng3.state().seats[1].cred;
    let mut b_def = vec![SealedOrderBundle::default(); 4];
    b_def[0].dual_escrows.push(dual_88.clone());
    b_def[1].dual_escrows.push(dual_88);
    b_def[1].contract_deliveries.push(ContractDeliveryOrder {
        counterparty: 0,
        good: AssetId::Raw(1),
        gross_amount: 1_100,
        domain_id: None,
        dual_escrow_id: Some(expected_eid_88),
    });
    commit_and_reveal_all(&mut eng3, TableId(966), 2, b_def);
    let r_def = must_ok(eng3.sweep_turn_seeded(2), "sweep buyer default");
    assert!(
        r_def
            .escrows_cancelled_or_forfeited
            .contains(&expected_eid_88)
    );
    assert!(r_def.fills.iter().any(|f| {
        f.kind == FillKind::EscrowForfeited
            && f.good == Some(AssetId::Cred)
            && f.from == Some(0)
            && f.to == 1
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert!(r_def.fills.iter().any(|f| {
        f.kind == FillKind::EscrowRefunded
            && f.good == Some(AssetId::Cred)
            && f.from == Some(1)
            && f.to == 1
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert_eq!(
        eng3.state().dual_escrows[&expected_eid_88].status,
        EscrowStatus::Cancelled
    );
    assert_eq!(eng3.state().seats[0].bond_breaches, 1);
    assert_eq!(eng3.state().seats[1].cred, seller_cred_before_def + 500);
    // Seller retained its Raw(1) (buyer received 0 Raw(1); after 20% Turn-2 spoilage on 8_000 + 10_000 = 14_400):
    assert_eq!(seller_raw1_before, 8_000);
    assert_eq!(eng3.state().seats[0].raw[1], 0);
    assert_eq!(eng3.state().seats[1].raw[1], 14_400);
}

#[test]
fn c3_works_stipend_reduces_season_cred_drain_without_scoring_coin_vp() {
    // Full 24..=28 turn seeded seasons (seed = 27) comparing stipend = 0 vs DEFAULT (350).
    let table_id = TableId(963);
    let deal_seed = [0x55; 32];
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x70 + i; 32])).collect();
    let (mut spec_zero, _deal_zero) = must_ok(
        TableSpec::standard_chiral_ring_with_explicit_deal(table_id, 4, 27, deal_seed, &salts),
        "spec zero",
    );
    spec_zero.works_stipend_per_unit_cred = 0;

    let (spec_stip, _) = must_ok(
        TableSpec::standard_chiral_ring_with_explicit_deal(table_id, 4, 27, deal_seed, &salts),
        "spec stip",
    );
    assert_eq!(
        spec_stip.works_stipend_per_unit_cred,
        DEFAULT_WORKS_STIPEND_PER_UNIT_CRED
    );

    let run_season_drain = |spec: TableSpec| -> (i64, i64, i64, Units, Units) {
        let initial_cred: i64 = spec.seats.iter().map(|s| s.initial_cred).sum();
        let mut eng = must_ok(TurnEngine::new(spec), "engine");
        for s in 0..4u8 {
            let c = eng.state().spec.dealt_mandate_commits[usize::from(s)];
            must_ok(eng.submit_mandate_commitment(Seat(s), c), "mc");
        }
        let mut minted_total = 0i64;
        for t in 1..=HARD_TURN_CAP {
            if eng.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            let mut bundles = vec![SealedOrderBundle::default(); 4];
            // Circulate raw goods around the 4-seat chiral ring via atomic swaps so every seat
            // can run converters, then trade adjacent refined goods to deliver Works projects.
            if t >= 2 {
                for i in 0..4u8 {
                    let next1 = (i + 1) % 4;
                    let sw = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + u64::from(i),
                        maker: i,
                        taker: next1,
                        maker_asset: AssetId::Raw(i),
                        maker_amount: 2_500,
                        taker_asset: AssetId::Raw(next1),
                        taker_amount: 2_500,
                        domain_id: None,
                    };
                    if eng.state().seats[usize::from(i)].raw[usize::from(i)] >= 7_500
                        && eng.state().seats[usize::from(next1)].raw[usize::from(next1)] >= 7_500
                    {
                        bundles[usize::from(i)].atomic_swaps.push(sw.clone());
                        bundles[usize::from(next1)].atomic_swaps.push(sw);
                    }
                    let next2 = (i + 2) % 4;
                    if i < 2 {
                        let sw2 = AtomicSwapProposal {
                            swap_id: u64::from(t) * 100 + 20 + u64::from(i),
                            maker: i,
                            taker: next2,
                            maker_asset: AssetId::Raw(i),
                            maker_amount: 2_500,
                            taker_asset: AssetId::Raw(next2),
                            taker_amount: 2_500,
                            domain_id: None,
                        };
                        if eng.state().seats[usize::from(i)].raw[usize::from(i)] >= 7_500
                            && eng.state().seats[usize::from(next2)].raw[usize::from(next2)]
                                >= 7_500
                        {
                            bundles[usize::from(i)].atomic_swaps.push(sw2.clone());
                            bundles[usize::from(next2)].atomic_swaps.push(sw2);
                        }
                    }
                }
            }
            for bundle in &mut bundles {
                bundle.converter_runs.push(ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 1,
                });
            }
            if t >= 3 {
                // Deliver refined swap (0 <-> 1 for Project 1; 2 <-> 3 for Project 3) and deliver projects.
                for (lead, partner, proj_id) in [(0_u8, 1_u8, 1_u16), (2_u8, 3_u8, 3_u16)] {
                    if eng.state().seats[usize::from(partner)].refined[usize::from(partner)]
                        >= 1_200
                        && eng.state().seats[usize::from(lead)].refined[usize::from(lead)] >= 2_200
                    {
                        let rsw = AtomicSwapProposal {
                            swap_id: u64::from(t) * 100 + 50 + u64::from(lead),
                            maker: partner,
                            taker: lead,
                            maker_asset: AssetId::Refined(partner),
                            maker_amount: 1_200,
                            taker_asset: AssetId::Refined(lead),
                            taker_amount: 1_200,
                            domain_id: None,
                        };
                        bundles[usize::from(partner)].atomic_swaps.push(rsw.clone());
                        bundles[usize::from(lead)].atomic_swaps.push(rsw);
                        bundles[usize::from(lead)]
                            .project_deliveries
                            .push(ProjectDeliveryOrder {
                                project_id: proj_id,
                                batches: 1,
                            });
                    }
                }
            }
            if t % 6 == 0 {
                // Charter bids drain circulating CRED on turns 6, 12, 18, 24 (recreating seed-27 drain).
                let roles = [
                    CharterRole::OracleAuditor,
                    CharterRole::ClearinghouseBroker,
                    CharterRole::CustomsInspector,
                ];
                for s in 0..3usize {
                    let bid = 3_600.min(eng.state().seats[s].cred);
                    if bid > 0 {
                        bundles[s].charter_bids.push(CharterBidOrder {
                            role: roles[s],
                            bid_cred: bid,
                        });
                    }
                }
            }
            commit_and_reveal_all(&mut eng, table_id, t, bundles);
            let rec = must_ok(eng.sweep_turn_seeded(u64::from(t)), "sweep");
            minted_total += rec.works_stipend_cred_minted;
        }
        let end_cred: i64 = eng.state().seats.iter().map(|s| s.cred).sum();
        let drain_bps = ((initial_cred - end_cred) * 10_000) / initial_cred;
        let total_vp: Units = eng.state().seats.iter().map(|s| s.project_equity).sum();
        (initial_cred, end_cred, drain_bps, minted_total, total_vp)
    };

    let (init_cred, end_before, drain_before_bps, minted_before, season_vp_before) =
        run_season_drain(spec_zero);
    let (_, end_after, drain_after_bps, minted_after, season_vp_after) =
        run_season_drain(spec_stip);
    println!(
        "C3_CRED_DRAIN: initial_cred={init_cred} end_cred_before={end_before} drain_before_bps={drain_before_bps} end_cred_after={end_after} drain_after_bps={drain_after_bps} stipend_minted={minted_after}"
    );
    assert_eq!(minted_before, 0);
    assert!(minted_after > 0, "Works deliveries must mint CRED stipend");
    assert!(
        drain_before_bps >= 5_000,
        "un-stipended season must reproduce heavy CRED drain ({drain_before_bps} bps)"
    );
    assert!(
        drain_after_bps < drain_before_bps,
        "Works stipend must reduce season CRED drain ({drain_after_bps} vs {drain_before_bps} bps)"
    );
    assert_eq!(
        season_vp_before, season_vp_after,
        "Works CRED stipend must never award extra Project VP"
    );

    // Also verify controlled exact stipend arithmetic (2_000 refined units * 350 / 1_000 = 700 CRED).
    let run_controlled_works = |stipend_per_unit: Units| -> (i64, Units, Units) {
        let tid = TableId(964);
        let (mut sp, _dl) = must_ok(
            TableSpec::standard_chiral_ring_with_explicit_deal(tid, 4, 3, deal_seed, &salts),
            "sp",
        );
        sp.works_stipend_per_unit_cred = stipend_per_unit;
        let initial_cred: i64 = sp.seats.iter().map(|s| s.initial_cred).sum();
        let mut eng = must_ok(TurnEngine::new(sp), "eng");
        must_ok(
            eng.apply_market_action(MarketAction::TownImport {
                seat: 0,
                good: AssetId::Refined(0),
                amount: 1_000,
            }),
            "ti r0",
        );
        must_ok(
            eng.apply_market_action(MarketAction::TownImport {
                seat: 0,
                good: AssetId::Refined(1),
                amount: 1_000,
            }),
            "ti r1",
        );
        let mut b = vec![SealedOrderBundle::default(); 4];
        b[0].project_deliveries.push(ProjectDeliveryOrder {
            project_id: 1,
            batches: 1,
        });
        commit_and_reveal_all(&mut eng, tid, 1, b);
        let rec = must_ok(eng.sweep_turn_seeded(1), "sw 1");
        let end_cred: i64 = eng.state().seats.iter().map(|s| s.cred).sum();
        (
            initial_cred - end_cred,
            rec.works_stipend_cred_minted,
            eng.state().seats[0].project_equity,
        )
    };
    let (ctrl_drain_0, ctrl_mint_0, ctrl_vp_0) = run_controlled_works(0);
    let (ctrl_drain_def, ctrl_mint_def, ctrl_vp_def) =
        run_controlled_works(DEFAULT_WORKS_STIPEND_PER_UNIT_CRED);
    assert_eq!(ctrl_mint_0, 0);
    assert_eq!(ctrl_mint_def, 700);
    assert_eq!(ctrl_drain_0 - ctrl_drain_def, 700);
    assert_eq!(ctrl_vp_0, ctrl_vp_def);
}

#[test]
fn c4_batch_clearing_beacon_tie_break_on_oversubscribed_level() {
    // One Seller offers 1_000 units of Raw(0) at ask 1_100.
    // Two Buyers (Seat 1 and Seat 2) bid at identical limit 1_300 for 1_000 each (oversubscribed 2:1).
    let mut winners = std::collections::BTreeSet::new();
    for seed in 0..16u64 {
        let mut seat1_first = false;
        let mut seat2_first = false;
        for reverse_arrival in [false, true] {
            let (mut spec, _) = must_ok(
                TableSpec::standard_chiral_ring_with_mandates(TableId(965), 4, seed),
                "spec",
            );
            for s in &mut spec.seats {
                s.initial_raw_tariff_bps = 0;
                s.raw_yield_per_turn = 10_000;
                s.perishable_decay_bps = 0;
                s.regent = None;
            }
            let mut engine = must_ok(TurnEngine::new(spec), "engine");
            commit_and_reveal_all(
                &mut engine,
                TableId(965),
                1,
                vec![SealedOrderBundle::default(); 4],
            );
            must_ok(engine.sweep_turn_seeded(seed), "sweep 1");

            // Turn 2: Sell 1_000 Raw(0) at 1_100; Seat 1 & Seat 2 both Buy 1_000 at 1_300.
            let sell_0 = MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(0),
                    amount: 1_000,
                    price_per_unit_cred: 1_100,
                    domain_id: None,
                    is_hybrid: false,
                },
            };
            let buy_1 = MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 1_000,
                    price_per_unit_cred: 1_300,
                    domain_id: None,
                    is_hybrid: false,
                },
            };
            let buy_2 = MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 1_000,
                    price_per_unit_cred: 1_300,
                    domain_id: None,
                    is_hybrid: false,
                },
            };
            must_ok(engine.apply_market_action(sell_0), "sell_0");
            if reverse_arrival {
                must_ok(engine.apply_market_action(buy_2), "buy_2");
                must_ok(engine.apply_market_action(buy_1), "buy_1");
            } else {
                must_ok(engine.apply_market_action(buy_1), "buy_1");
                must_ok(engine.apply_market_action(buy_2), "buy_2");
            }
            commit_and_reveal_all(
                &mut engine,
                TableId(965),
                2,
                vec![SealedOrderBundle::default(); 4],
            );
            must_ok(engine.sweep_turn_seeded(seed), "sweep 2");
            let w = if engine.state().seats[1].raw[0] == 1_000 {
                assert_eq!(engine.state().seats[2].raw[0], 0);
                if !reverse_arrival {
                    seat1_first = true;
                }
                1u8
            } else {
                assert_eq!(engine.state().seats[2].raw[0], 1_000);
                if !reverse_arrival {
                    seat2_first = true;
                }
                2u8
            };
            winners.insert((reverse_arrival, seed, w));
        }
        let w_fwd = must_ok(
            winners.iter().find(|x| !x.0 && x.1 == seed).ok_or("fwd"),
            "fwd",
        )
        .2;
        let w_rev = must_ok(
            winners.iter().find(|x| x.0 && x.1 == seed).ok_or("rev"),
            "rev",
        )
        .2;
        assert_eq!(
            w_fwd, w_rev,
            "tied buyer winner on seed {seed} must be arrival-order invariant"
        );
        let _ = (seat1_first, seat2_first);
    }
    let distinct_winners: std::collections::BTreeSet<u8> = winners.iter().map(|x| x.2).collect();
    assert_eq!(
        distinct_winners.len(),
        2,
        "beacon tie-break across seeds must select both Seat 1 and Seat 2"
    );
    assert_eq!(IDLE_MISS_TAKEOVER_TURNS, 3);
}

const SEALED_TEST_VALUES: [u64; 3] = [1_u64 << 63, u64::MAX, (1_u64 << 63) | 42];

fn assert_seed_absent(haystack: &str, seed: u64, context: &str) {
    let dec = seed.to_string();
    let be_hex_padded = format!("{seed:016x}");
    let be_hex_compact = format!("{seed:x}");
    let be_hex_upper = format!("{seed:016X}");
    let be_hex_compact_upper = format!("{seed:X}");
    let le_hex_lower: String = seed
        .to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let le_hex_upper = le_hex_lower.to_ascii_uppercase();
    for needle in [
        dec,
        be_hex_padded,
        be_hex_compact,
        be_hex_upper,
        be_hex_compact_upper,
        le_hex_lower,
        le_hex_upper,
    ] {
        assert!(
            !haystack.contains(&needle),
            "{context}: unexpected clear layout-seed representation {needle:?} leaked in output"
        );
    }
}

fn clean_32(tag: u8) -> [u8; 32] {
    std::array::from_fn(|i| {
        0x21_u8
            .wrapping_add(tag.wrapping_mul(13))
            .wrapping_add((i as u8).wrapping_mul(7))
    })
}

fn clean_drand_sig(tag: u8, turn: u8) -> Vec<u8> {
    (0..DRAND_QUICKNET_SIGNATURE_BYTES)
        .map(|i| {
            0x35_u8
                .wrapping_add(tag.wrapping_mul(11))
                .wrapping_add(turn.wrapping_mul(5))
                .wrapping_add((i as u8).wrapping_mul(3))
        })
        .collect()
}

fn coupled_layout_seed_candidates(seed: u64) -> [([u8; 32], &'static str); 2] {
    use sha2::{Digest as _, Sha256};
    let mut padded = [0_u8; 32];
    padded[..8].copy_from_slice(&seed.to_le_bytes());
    let hashed: [u8; 32] = Sha256::digest(seed.to_le_bytes()).into();
    [(padded, "padded_le_bytes"), (hashed, "sha256_le_bytes")]
}

fn must_err<T: std::fmt::Debug, E>(res: Result<T, E>, ctx: &str) -> E {
    match res {
        Err(err) => err,
        Ok(val) => panic!("{ctx}: expected Err, got Ok({val:?})"),
    }
}

struct SeasonParts {
    spec: TableSpec,
    deal: league_core::SealedMandateDeal,
    mandate_commits: Vec<Option<CommitmentHash>>,
    turns: Vec<LoggedTurn>,
    mandate_reveals: Vec<Option<MandateReveal>>,
    board: FinalScoreboard,
    digests: Vec<[u8; 32]>,
}

fn run_eval_season_parts(
    table_id: TableId,
    layout_seed: u64,
    deal_seed: [u8; 32],
    eval_bytes: [u8; 32],
    salt_base: u8,
) -> SeasonParts {
    let salts: Vec<RevealSalt> = (0..4_u8)
        .map(|s| RevealSalt(clean_32(salt_base.wrapping_add(s))))
        .collect();
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes(eval_bytes), "eval_seed");
    let (spec, deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            table_id,
            4,
            layout_seed,
            deal_seed,
            &salts,
            eval_seed,
        ),
        "standard_eval_chiral_ring_with_explicit_setup",
    );
    let mut engine = must_ok(TurnEngine::new(spec.clone()), "TurnEngine::new eval");
    let mut mandate_commits = vec![None; 4];
    let mut mandate_reveals = vec![None; 4];
    for s in 0..4_u8 {
        let idx = usize::from(s);
        let commit = spec.dealt_mandate_commits[idx];
        must_ok(
            engine.submit_mandate_commitment(Seat(s), commit),
            "submit_mandate_commitment",
        );
        mandate_commits[idx] = Some(commit);
        mandate_reveals[idx] = Some(deal.seat_reveals[idx].clone());
    }
    let mut turns = Vec::new();
    let mut digests = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) = commit_and_reveal_all(
            &mut engine,
            table_id,
            t,
            vec![SealedOrderBundle::default(); 4],
        );
        let (beacon, proof) = must_ok(eval_seed.prove_turn(table_id, Turn(t)), "prove_turn");
        let receipt = must_ok(
            engine.sweep_turn_with_eval_vrf(&proof),
            "sweep_turn_with_eval_vrf",
        );
        digests.push(receipt.post_state_digest);
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
    assert_eq!(engine.state().phase, TurnPhase::AwaitingMandateReveal);
    for (s_idx, rev_opt) in mandate_reveals.iter().enumerate() {
        if let Some(rev) = rev_opt {
            must_ok(
                engine.submit_mandate_reveal(Seat(s_idx as u8), rev.clone()),
                "submit_mandate_reveal",
            );
        }
    }
    let board = must_ok(engine.finalize_season(), "finalize_season eval");
    SeasonParts {
        spec,
        deal,
        mandate_commits,
        turns,
        mandate_reveals,
        board,
        digests,
    }
}

fn run_game_season_parts(table_id: TableId, layout_seed: u64, sig_tag: u8) -> SeasonParts {
    let schedule = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
        turn_period_secs: 60,
    };
    let (spec, deal) = must_ok(
        TableSpec::standard_game_chiral_ring_with_mandates(table_id, 4, layout_seed, schedule),
        "standard_game_chiral_ring_with_mandates",
    );
    let mut engine = must_ok(TurnEngine::new(spec.clone()), "TurnEngine::new game");
    let mut mandate_commits = vec![None; 4];
    let mut mandate_reveals = vec![None; 4];
    for s in 0..4_u8 {
        let idx = usize::from(s);
        let commit = spec.dealt_mandate_commits[idx];
        must_ok(
            engine.submit_mandate_commitment(Seat(s), commit),
            "submit_mandate_commitment",
        );
        mandate_commits[idx] = Some(commit);
        mandate_reveals[idx] = Some(deal.seat_reveals[idx].clone());
    }
    let mut turns = Vec::new();
    let mut digests = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) = commit_and_reveal_all(
            &mut engine,
            table_id,
            t,
            vec![SealedOrderBundle::default(); 4],
        );
        let round = must_ok(schedule.expected_round_for_turn(Turn(t)), "expected_round");
        let sig = clean_drand_sig(sig_tag, t);
        let beacon = must_ok(derive_drand_beacon(&sig), "derive_drand_beacon");
        let receipt = must_ok(
            engine.sweep_turn_with_drand(round, &sig),
            "sweep_turn_with_drand",
        );
        digests.push(receipt.post_state_digest);
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: Some(round),
            drand_signature: Some(sig),
            eval_vrf_proof: None,
            drawn_shocks: receipt.newly_drawn_shocks.clone(),
        });
        if receipt.season_closed {
            break;
        }
    }
    assert_eq!(engine.state().phase, TurnPhase::AwaitingMandateReveal);
    for (s_idx, rev_opt) in mandate_reveals.iter().enumerate() {
        if let Some(rev) = rev_opt {
            must_ok(
                engine.submit_mandate_reveal(Seat(s_idx as u8), rev.clone()),
                "submit_mandate_reveal",
            );
        }
    }
    let board = must_ok(engine.finalize_season(), "finalize_season game");
    SeasonParts {
        spec,
        deal,
        mandate_commits,
        turns,
        mandate_reveals,
        board,
        digests,
    }
}

#[test]
fn sealed_replay_log_roundtrips_and_omits_clear_seed_dec_and_hex() {
    assert_eq!(SEALED_LAYOUT_SEED_MIN, 1_u64 << 63);
    assert_eq!(SEALED_SEED_POOL_NAME, "sealed");

    for (idx, &sealed_val) in SEALED_TEST_VALUES.iter().enumerate() {
        let base_tag = (idx as u8).wrapping_mul(7).wrapping_add(1);
        let SeasonParts {
            spec: spec_e,
            deal: deal_e,
            mandate_commits: commits_e,
            turns: turns_e,
            mandate_reveals: reveals_e,
            board: board_e,
            digests: digests_e,
        } = run_eval_season_parts(
            TableId(9_100 + idx as u64),
            sealed_val,
            clean_32(base_tag),
            clean_32(base_tag.wrapping_add(1)),
            base_tag.wrapping_add(10),
        );
        let log_new_sealed = must_ok(
            SeasonReplayLog::new_sealed(
                spec_e.clone(),
                sealed_val,
                deal_e.clone(),
                commits_e.clone(),
                turns_e.clone(),
                reveals_e.clone(),
            ),
            "new_sealed eval",
        );
        let log_from_rig = must_ok(
            SeasonReplayLog::from_rig_seed(
                spec_e, sealed_val, deal_e, commits_e, turns_e, reveals_e,
            ),
            "from_rig_seed sealed eval",
        );
        assert_eq!(log_new_sealed, log_from_rig);

        let SeasonParts {
            spec: spec_g,
            deal: deal_g,
            mandate_commits: commits_g,
            turns: turns_g,
            mandate_reveals: reveals_g,
            board: board_g,
            digests: digests_g,
        } = run_game_season_parts(
            TableId(9_200 + idx as u64),
            sealed_val,
            base_tag.wrapping_add(30),
        );
        let log_game = must_ok(
            SeasonReplayLog::new_sealed(spec_g, sealed_val, deal_g, commits_g, turns_g, reveals_g),
            "new_sealed game",
        );

        let expected_digest = layout_seed_digest(sealed_val);
        for (kind, log, live_board, live_digests) in [
            ("eval", &log_new_sealed, &board_e, &digests_e),
            ("game", &log_game, &board_g, &digests_g),
        ] {
            assert!(log.seed.is_sealed());
            assert_eq!(log.seed.clear_seed(), None);
            assert_eq!(log.seed.pool(), Some(SEALED_SEED_POOL_NAME));
            assert_eq!(log.seed.sealed_digest(), Some(expected_digest.as_str()));
            assert_eq!(
                log.seed.independence_checked_at_write_unverified(),
                Some(true)
            );
            let sealed_inner = must_ok(log.seed.sealed().ok_or("sealed payload"), "sealed()");
            assert_eq!(sealed_inner.pool(), SEALED_SEED_POOL_NAME);
            assert_eq!(sealed_inner.digest(), expected_digest);
            assert!(sealed_inner.independence_checked_at_write_unverified());

            let json_str = must_ok(serde_json::to_string(log), "serialize sealed log");
            let json_val: serde_json::Value =
                must_ok(serde_json::from_str(&json_str), "parse sealed log value");
            let seed_obj = must_ok(
                json_val["seed"].as_object().ok_or("seed json object"),
                "seed object",
            );
            assert_eq!(seed_obj.len(), 3);
            assert_eq!(seed_obj["pool"], SEALED_SEED_POOL_NAME);
            assert_eq!(seed_obj["digest"], expected_digest);
            assert_eq!(seed_obj["independence_checked_at_write_unverified"], true);

            let debug_log = format!("{log:?}");
            let debug_seed = format!("{:?}", log.seed);
            assert_seed_absent(&json_str, sealed_val, &format!("{kind} log json"));
            assert_seed_absent(&debug_log, sealed_val, &format!("{kind} log debug"));
            assert_seed_absent(&debug_seed, sealed_val, &format!("{kind} seed debug"));

            let roundtripped: SeasonReplayLog =
                must_ok(serde_json::from_str(&json_str), "roundtrip sealed log");
            assert_eq!(&roundtripped, log);
            let out = must_ok(
                replay_season(&roundtripped),
                "replay roundtripped sealed log",
            );
            assert_eq!(&out.scoreboard, live_board);
            assert_eq!(&out.state_digests, live_digests);

            let record = must_ok(
                attach_state_predicates(log),
                "attach_state_predicates sealed",
            );
            let record_json = must_ok(serde_json::to_string(&record), "serialize ReplayRecord");
            let debug_record = format!("{record:?}");
            assert_seed_absent(&record_json, sealed_val, &format!("{kind} record json"));
            assert_seed_absent(&debug_record, sealed_val, &format!("{kind} record debug"));
            let rt_record = must_ok(
                ReplayRecord::from_json_slice(record_json.as_bytes()),
                "ReplayRecord::from_json_slice sealed",
            );
            assert_eq!(rt_record, record);
            let (out_with_pred, _) = must_ok(
                replay_season_with_state_predicates(&rt_record.log),
                "replay_season_with_state_predicates sealed",
            );
            assert_eq!(&out_with_pred, &out);
        }
    }
}

#[test]
fn deserializing_numeric_sealed_seed_or_malformed_digest_is_refused_by_name() {
    let max_clear = SEALED_LAYOUT_SEED_MIN - 1;
    assert_eq!(max_clear, (1_u64 << 63) - 1);

    let clear_boundary = must_ok(ReplayLayoutSeed::try_clear(max_clear), "try_clear boundary");
    assert_eq!(clear_boundary.clear_seed(), Some(max_clear));
    assert!(!clear_boundary.is_sealed());
    assert_eq!(
        clear_boundary.independence_checked_at_write_unverified(),
        None
    );
    assert_eq!(
        must_ok(ReplayLayoutSeed::try_from(max_clear), "try_from boundary"),
        clear_boundary
    );
    let boundary_json = must_ok(serde_json::to_string(&clear_boundary), "serialize boundary");
    assert_eq!(boundary_json, max_clear.to_string());
    assert_eq!(
        must_ok(
            serde_json::from_str::<ReplayLayoutSeed>(&boundary_json),
            "deserialize boundary"
        ),
        clear_boundary
    );

    let SeasonParts {
        spec: spec_b,
        deal: deal_b,
        mandate_commits: commits_b,
        turns: turns_b,
        mandate_reveals: reveals_b,
        board: board_b,
        digests: _,
    } = run_eval_season_parts(TableId(9_300), max_clear, clean_32(41), clean_32(42), 50);
    let boundary_log = must_ok(
        SeasonReplayLog::from_rig_seed(spec_b, max_clear, deal_b, commits_b, turns_b, reveals_b),
        "from_rig_seed max_clear",
    );
    assert_eq!(boundary_log.seed.clear_seed(), Some(max_clear));
    assert_eq!(
        must_ok(replay_season(&boundary_log), "replay boundary").scoreboard,
        board_b
    );

    let base_clear_json: serde_json::Value = must_ok(
        serde_json::from_slice(&must_ok(
            serde_json::to_vec(&boundary_log),
            "serialize boundary_log",
        )),
        "value boundary_log",
    );

    for &sealed_val in &SEALED_TEST_VALUES {
        let expected_digest = layout_seed_digest(sealed_val);

        let err_clear = must_err(
            ReplayLayoutSeed::try_clear(sealed_val),
            "sealed-range u64 must be refused in try_clear",
        );
        assert_eq!(
            err_clear,
            ReplayLayoutSeedError::SealedSeedInClear {
                seed_digest: expected_digest.clone()
            }
        );
        let msg_clear = err_clear.to_string();
        assert!(msg_clear.contains("SEALED_SEED_IN_CLEAR"));
        assert!(msg_clear.contains(&expected_digest));
        assert_seed_absent(&msg_clear, sealed_val, "try_clear Display");
        assert_seed_absent(&format!("{err_clear:?}"), sealed_val, "try_clear Debug");

        let err_try_from = must_err(
            ReplayLayoutSeed::try_from(sealed_val),
            "sealed-range u64 must be refused in TryFrom",
        );
        assert!(err_try_from.to_string().contains("SEALED_SEED_IN_CLEAR"));
        assert_seed_absent(&err_try_from.to_string(), sealed_val, "TryFrom Display");

        let err_de_seed = must_err(
            serde_json::from_str::<ReplayLayoutSeed>(&sealed_val.to_string()),
            "numeric sealed seed JSON must fail deserialization",
        );
        let msg_de_seed = err_de_seed.to_string();
        assert!(msg_de_seed.contains("SEALED_SEED_IN_CLEAR"));
        assert_seed_absent(&msg_de_seed, sealed_val, "ReplayLayoutSeed JSON err");

        let mut bad_log_val = base_clear_json.clone();
        bad_log_val["seed"] = serde_json::Value::Number(serde_json::Number::from(sealed_val));
        let bad_log_bytes = must_ok(serde_json::to_vec(&bad_log_val), "bad_log_val bytes");
        let err_de_log = must_err(
            serde_json::from_slice::<SeasonReplayLog>(&bad_log_bytes),
            "SeasonReplayLog with numeric sealed seed must fail",
        );
        assert!(err_de_log.to_string().contains("SEALED_SEED_IN_CLEAR"));
        assert_seed_absent(
            &err_de_log.to_string(),
            sealed_val,
            "SeasonReplayLog JSON err",
        );

        let err_rec = must_err(
            ReplayRecord::from_json_slice(&bad_log_bytes),
            "ReplayRecord with numeric sealed seed must fail",
        );
        let err_rec_str = err_rec.to_string();
        assert!(err_rec_str.contains("SEALED_SEED_IN_CLEAR"));
        assert_seed_absent(&err_rec_str, sealed_val, "ReplayRecord JSON err");
    }

    // Passing a non-sealed seed (< 2^63) to new_sealed is refused by name.
    for non_sealed in [0_u64, 42, max_clear] {
        let err_non_sealed = must_err(
            SeasonReplayLog::new_sealed(
                boundary_log.spec.clone(),
                non_sealed,
                boundary_log.sealed_mandate_deal.clone(),
                boundary_log.mandate_commits.clone(),
                boundary_log.turns.clone(),
                boundary_log.mandate_reveals.clone(),
            ),
            "new_sealed must reject < 2^63 seed",
        );
        assert!(
            matches!(&err_non_sealed, EngineError::InvalidConfig(m) if m.contains("NON_SEALED_SEED_IN_SEALED")),
            "expected NON_SEALED_SEED_IN_SEALED, got {err_non_sealed:?}"
        );
    }

    let valid_digest = layout_seed_digest(SEALED_TEST_VALUES[0]);
    for bad_pool in ["dev", "probe", "held_out", "unassigned", "Sealed", ""] {
        let raw = serde_json::json!({
            "pool": bad_pool,
            "digest": valid_digest,
            "independence_checked_at_write_unverified": true,
        });
        let err_seed = must_err(
            serde_json::from_value::<ReplayLayoutSeed>(raw.clone()),
            "invalid pool must fail",
        );
        assert!(
            err_seed.to_string().contains("INVALID_SEALED_SEED_POOL"),
            "pool={bad_pool:?}: got {err_seed}"
        );
        let mut bad_log_val = base_clear_json.clone();
        bad_log_val["seed"] = raw;
        let bad_bytes = must_ok(serde_json::to_vec(&bad_log_val), "bad pool log bytes");
        let err_log = must_err(
            serde_json::from_slice::<SeasonReplayLog>(&bad_bytes),
            "log with invalid pool must fail",
        );
        assert!(err_log.to_string().contains("INVALID_SEALED_SEED_POOL"));
        let err_rec = must_err(
            ReplayRecord::from_json_slice(&bad_bytes),
            "record with invalid pool must fail",
        );
        assert!(err_rec.to_string().contains("INVALID_SEALED_SEED_POOL"));
    }

    let bad_digests: Vec<serde_json::Value> = vec![
        serde_json::json!(&valid_digest[..63]),
        serde_json::json!(format!("{valid_digest}0")),
        serde_json::json!(valid_digest.to_ascii_uppercase()),
        serde_json::json!("g".repeat(64)),
        serde_json::json!(""),
        serde_json::json!(12345),
    ];
    for bad_digest in bad_digests {
        let raw = serde_json::json!({
            "pool": "sealed",
            "digest": bad_digest,
            "independence_checked_at_write_unverified": true,
        });
        let err_seed = must_err(
            serde_json::from_value::<ReplayLayoutSeed>(raw.clone()),
            "malformed digest must fail",
        );
        assert!(
            err_seed.to_string().contains("INVALID_SEALED_SEED_DIGEST"),
            "digest={bad_digest}: got {err_seed}"
        );
        let mut bad_log_val = base_clear_json.clone();
        bad_log_val["seed"] = raw;
        let bad_bytes = must_ok(serde_json::to_vec(&bad_log_val), "bad digest bytes");
        let err_log = must_err(
            serde_json::from_slice::<SeasonReplayLog>(&bad_bytes),
            "log with malformed digest must fail",
        );
        assert!(err_log.to_string().contains("INVALID_SEALED_SEED_DIGEST"));
    }

    let missing_digest = serde_json::json!({
        "pool": "sealed",
        "independence_checked_at_write_unverified": true,
    });
    assert!(
        must_err(
            serde_json::from_value::<ReplayLayoutSeed>(missing_digest),
            "missing digest must fail",
        )
        .to_string()
        .contains("INVALID_SEALED_SEED_DIGEST")
    );

    let missing_pool = serde_json::json!({
        "digest": valid_digest,
        "independence_checked_at_write_unverified": true,
    });
    assert!(serde_json::from_value::<ReplayLayoutSeed>(missing_pool).is_err());

    let missing_claim = serde_json::json!({
        "pool": "sealed",
        "digest": valid_digest,
    });
    assert!(serde_json::from_value::<ReplayLayoutSeed>(missing_claim).is_err());

    let extra_field = serde_json::json!({
        "pool": "sealed",
        "digest": valid_digest,
        "independence_checked_at_write_unverified": true,
        "clear_seed": 42,
    });
    assert!(serde_json::from_value::<ReplayLayoutSeed>(extra_field).is_err());
}

#[test]
fn non_sealed_log_json_and_replay_outcome_match_v1_wire_schema_and_read_pre_version() {
    use sha2::{Digest as _, Sha256};

    #[derive(serde::Serialize)]
    struct ExpectedV1SeasonReplayLog<'a> {
        format_version: u32,
        spec: &'a TableSpec,
        mode: TableMode,
        seed: u64,
        beacon_mode: ReplayBeaconMode,
        eval_vrf_pubkey: Option<String>,
        sealed_mandate_deal: &'a league_core::SealedMandateDeal,
        mandate_commits: &'a [Option<CommitmentHash>],
        turns: &'a [LoggedTurn],
        mandate_reveals: &'a [Option<MandateReveal>],
    }

    #[derive(serde::Serialize)]
    struct PreVersionSeasonReplayLog<'a> {
        spec: &'a TableSpec,
        mode: TableMode,
        seed: u64,
        beacon_mode: ReplayBeaconMode,
        eval_vrf_pubkey: Option<String>,
        sealed_mandate_deal: &'a league_core::SealedMandateDeal,
        mandate_commits: &'a [Option<CommitmentHash>],
        turns: &'a [LoggedTurn],
        mandate_reveals: &'a [Option<MandateReveal>],
    }

    #[derive(serde::Serialize)]
    struct ExpectedV1ReplayRecord<'a, S: serde::Serialize> {
        #[serde(flatten)]
        log: ExpectedV1SeasonReplayLog<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_predicates: Option<&'a S>,
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    // Covers the clear scripted fixtures `(4, 11)`, `(5, 29)`, `(6, 73)` plus a 4-seat
    // `SeedDerived`-free rich fixture `(4, 42)` also constructed via `from_rig_seed`.
    for &(seats, clear_seed) in &[(4_u8, 11_u64), (5, 29), (6, 73), (4, 42)] {
        let (log, live_board, live_digests) = run_scripted_full_season(seats, clear_seed);
        assert_ne!(log.beacon_mode, ReplayBeaconMode::SeedDerived);
        assert_eq!(log.seed.clear_seed(), Some(clear_seed));
        assert!(!log.seed.is_sealed());
        assert_eq!(log.seed.independence_checked_at_write_unverified(), None);

        if (seats, clear_seed) == (4, 42) {
            let from_rig = must_ok(
                SeasonReplayLog::from_rig_seed(
                    log.spec.clone(),
                    clear_seed,
                    log.sealed_mandate_deal.clone(),
                    log.mandate_commits.clone(),
                    log.turns.clone(),
                    log.mandate_reveals.clone(),
                ),
                "from_rig_seed clear (4, 42)",
            );
            assert_eq!(from_rig, log);
        }

        let eval_pk_hex = log
            .eval_vrf_pubkey
            .map(|bytes| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let expected_v1_log = ExpectedV1SeasonReplayLog {
            format_version: league_core::REPLAY_FORMAT_VERSION,
            spec: &log.spec,
            mode: log.mode,
            seed: clear_seed,
            beacon_mode: log.beacon_mode,
            eval_vrf_pubkey: eval_pk_hex.clone(),
            sealed_mandate_deal: &log.sealed_mandate_deal,
            mandate_commits: &log.mandate_commits,
            turns: &log.turns,
            mandate_reveals: &log.mandate_reveals,
        };
        let pre_version_log = PreVersionSeasonReplayLog {
            spec: &log.spec,
            mode: log.mode,
            seed: clear_seed,
            beacon_mode: log.beacon_mode,
            eval_vrf_pubkey: eval_pk_hex,
            sealed_mandate_deal: &log.sealed_mandate_deal,
            mandate_commits: &log.mandate_commits,
            turns: &log.turns,
            mandate_reveals: &log.mandate_reveals,
        };

        let expected_v1_bytes = must_ok(
            serde_json::to_vec(&expected_v1_log),
            "serialize expected_v1_log",
        );
        let new_bytes = must_ok(serde_json::to_vec(&log), "serialize log");
        assert_eq!(
            new_bytes, expected_v1_bytes,
            "clear ({seats}, {clear_seed}) bare SeasonReplayLog JSON drifted from v1 schema"
        );

        let val: serde_json::Value = must_ok(serde_json::from_slice(&new_bytes), "parse log json");
        assert_eq!(val["seed"], serde_json::json!(clear_seed));
        assert_eq!(
            val["format_version"],
            serde_json::json!(league_core::REPLAY_FORMAT_VERSION)
        );

        let bare_record = ReplayRecord {
            log: log.clone(),
            state_predicates: None,
        };
        let bare_record_bytes = must_ok(serde_json::to_vec(&bare_record), "bare record bytes");
        assert_eq!(
            bare_record_bytes, expected_v1_bytes,
            "clear ({seats}, {clear_seed}) bare ReplayRecord drifted from v1 schema"
        );

        let pre_version_bytes = must_ok(
            serde_json::to_vec(&pre_version_log),
            "serialize pre_version_log",
        );
        let prefix = format!(
            "{{\"format_version\":{},",
            league_core::REPLAY_FORMAT_VERSION
        );
        assert!(
            new_bytes.starts_with(prefix.as_bytes()),
            "clear ({seats}, {clear_seed}) log must start with leading format_version field"
        );
        assert_eq!(
            &new_bytes[prefix.len()..],
            &pre_version_bytes[1..],
            "clear ({seats}, {clear_seed}) body after format_version must stay byte-identical to pre-version schema"
        );
        let rt_from_pre: SeasonReplayLog = must_ok(
            SeasonReplayLog::from_json_slice(&pre_version_bytes),
            "parse pre_version_bytes",
        );
        assert_eq!(rt_from_pre, log);
        let rt_rec_from_pre = must_ok(
            ReplayRecord::from_json_slice(&pre_version_bytes),
            "ReplayRecord::from_json_slice(pre_version_bytes)",
        );
        assert_eq!(rt_rec_from_pre, bare_record);

        let rt_from_v1: SeasonReplayLog =
            must_ok(serde_json::from_slice(&expected_v1_bytes), "parse v1_bytes");
        assert_eq!(rt_from_v1, log);
        let rt_rec_from_v1 = must_ok(
            ReplayRecord::from_json_slice(&expected_v1_bytes),
            "ReplayRecord::from_json_slice(v1_bytes)",
        );
        assert_eq!(rt_rec_from_v1, bare_record);

        let out = must_ok(replay_season(&rt_from_v1), "replay clear log");
        assert_eq!(out.scoreboard, live_board);
        assert_eq!(out.state_digests, live_digests);

        let (out_with_pred, section) = must_ok(
            replay_season_with_state_predicates(&log),
            "replay_season_with_state_predicates clear",
        );
        assert_eq!(out_with_pred, out);

        let attached_record = must_ok(attach_state_predicates(&log), "attach_state_predicates");
        let attached_bytes = must_ok(serde_json::to_vec(&attached_record), "attached_bytes");
        let expected_v1_record = ExpectedV1ReplayRecord {
            log: expected_v1_log,
            state_predicates: Some(&section),
        };
        let expected_v1_record_bytes = must_ok(
            serde_json::to_vec(&expected_v1_record),
            "expected_v1_record_bytes",
        );
        assert_eq!(
            attached_bytes, expected_v1_record_bytes,
            "clear ({seats}, {clear_seed}) ReplayRecord with predicates drifted from v1 schema"
        );

        let out_bytes = must_ok(serde_json::to_vec(&out), "serialize ReplayOutcome");
        println!(
            "v1-schema non-sealed ({seats}, {clear_seed}): log_sha256={} outcome_sha256={}",
            sha256_hex(&new_bytes),
            sha256_hex(&out_bytes)
        );
    }
}

#[test]
fn deal_seed_and_vrf_independence_checked_on_both_clear_and_sealed_paths() {
    let clear_seed = 42_u64;
    for (cand_idx, (coupled_bytes, label)) in coupled_layout_seed_candidates(clear_seed)
        .into_iter()
        .enumerate()
    {
        // Clear path 1: coupled deal_seed is caught at write time (`from_rig_seed`) AND replay time.
        let SeasonParts {
            spec: spec_cd,
            deal: deal_cd,
            mandate_commits: commits_cd,
            turns: turns_cd,
            mandate_reveals: reveals_cd,
            board: _,
            digests: _,
        } = run_eval_season_parts(
            TableId(9_400 + cand_idx as u64),
            clear_seed,
            coupled_bytes,
            clean_32(60 + cand_idx as u8),
            70 + cand_idx as u8,
        );
        let err_write_cd = must_err(
            SeasonReplayLog::from_rig_seed(
                spec_cd.clone(),
                clear_seed,
                deal_cd.clone(),
                commits_cd.clone(),
                turns_cd.clone(),
                reveals_cd.clone(),
            ),
            "from_rig_seed must reject clear coupled deal_seed",
        );
        assert!(
            matches!(&err_write_cd, EngineError::InvalidConfig(m) if m.contains("deal_seed must be independent")),
            "clear coupled deal_seed ({label}) from_rig_seed got {err_write_cd:?}"
        );
        let direct_clear_cd = SeasonReplayLog {
            spec: spec_cd,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(clear_seed), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: None,
            sealed_mandate_deal: deal_cd,
            mandate_commits: commits_cd,
            turns: turns_cd,
            mandate_reveals: reveals_cd,
        };
        let err_replay_cd = must_err(
            replay_season(&direct_clear_cd),
            "replay_season must reject clear coupled deal_seed",
        );
        assert!(
            matches!(&err_replay_cd, EngineError::InvalidConfig(m) if m.contains("deal_seed must be independent")),
            "clear coupled deal_seed ({label}) replay got {err_replay_cd:?}"
        );

        // Clear path 2: coupled Eval VRF key is caught at write time AND replay time.
        let SeasonParts {
            spec: spec_cv,
            deal: deal_cv,
            mandate_commits: commits_cv,
            turns: turns_cv,
            mandate_reveals: reveals_cv,
            board: _,
            digests: _,
        } = run_eval_season_parts(
            TableId(9_410 + cand_idx as u64),
            clear_seed,
            clean_32(80 + cand_idx as u8),
            coupled_bytes,
            90 + cand_idx as u8,
        );
        let eval_pk_cv = spec_cv.eval_vrf_pubkey;
        let err_write_cv = must_err(
            SeasonReplayLog::from_rig_seed(
                spec_cv.clone(),
                clear_seed,
                deal_cv.clone(),
                commits_cv.clone(),
                turns_cv.clone(),
                reveals_cv.clone(),
            ),
            "from_rig_seed must reject clear coupled Eval VRF key",
        );
        assert!(
            matches!(&err_write_cv, EngineError::InvalidConfig(m) if m.contains("Eval VRF key must be independent")),
            "clear coupled VRF ({label}) from_rig_seed got {err_write_cv:?}"
        );
        let direct_clear_cv = SeasonReplayLog {
            spec: spec_cv,
            mode: TableMode::Eval,
            seed: must_ok(ReplayLayoutSeed::try_clear(clear_seed), "clear seed"),
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk_cv,
            sealed_mandate_deal: deal_cv,
            mandate_commits: commits_cv,
            turns: turns_cv,
            mandate_reveals: reveals_cv,
        };
        let err_replay_cv = must_err(
            replay_season(&direct_clear_cv),
            "replay_season must reject clear coupled Eval VRF key",
        );
        assert!(
            matches!(&err_replay_cv, EngineError::InvalidConfig(m) if m.contains("Eval VRF key must be independent")),
            "clear coupled VRF ({label}) replay got {err_replay_cv:?}"
        );
    }

    // Sealed path: checked at write time (`new_sealed` & `from_rig_seed`), never leaks clear value,
    // and tampering `independence_checked_at_write_unverified` to false is refused at replay time.
    for (s_idx, &sealed_val) in SEALED_TEST_VALUES.iter().enumerate() {
        for (cand_idx, (coupled_bytes, label)) in coupled_layout_seed_candidates(sealed_val)
            .into_iter()
            .enumerate()
        {
            let tag_offset = (s_idx as u8).wrapping_mul(10).wrapping_add(cand_idx as u8);
            let SeasonParts {
                spec: spec_sd,
                deal: deal_sd,
                mandate_commits: commits_sd,
                turns: turns_sd,
                mandate_reveals: reveals_sd,
                board: _,
                digests: _,
            } = run_eval_season_parts(
                TableId(9_500 + s_idx as u64 * 10 + cand_idx as u64),
                sealed_val,
                coupled_bytes,
                clean_32(110 + tag_offset),
                120 + tag_offset,
            );
            for (ctor_name, res) in [
                (
                    "new_sealed",
                    SeasonReplayLog::new_sealed(
                        spec_sd.clone(),
                        sealed_val,
                        deal_sd.clone(),
                        commits_sd.clone(),
                        turns_sd.clone(),
                        reveals_sd.clone(),
                    ),
                ),
                (
                    "from_rig_seed",
                    SeasonReplayLog::from_rig_seed(
                        spec_sd, sealed_val, deal_sd, commits_sd, turns_sd, reveals_sd,
                    ),
                ),
            ] {
                let err = must_err(res, "sealed coupled deal_seed must fail");
                match &err {
                    EngineError::InvalidConfig(msg) => {
                        assert!(
                            msg.contains("SEALED_DEAL_SEED_NOT_INDEPENDENT"),
                            "{ctor_name} ({label}) unexpected error: {msg}"
                        );
                        assert_seed_absent(msg, sealed_val, &format!("{ctor_name} deal error"));
                    }
                    other => panic!("{ctor_name} ({label}) expected InvalidConfig, got {other:?}"),
                }
            }

            let SeasonParts {
                spec: spec_sv,
                deal: deal_sv,
                mandate_commits: commits_sv,
                turns: turns_sv,
                mandate_reveals: reveals_sv,
                board: _,
                digests: _,
            } = run_eval_season_parts(
                TableId(9_600 + s_idx as u64 * 10 + cand_idx as u64),
                sealed_val,
                clean_32(140 + tag_offset),
                coupled_bytes,
                150 + tag_offset,
            );
            for (ctor_name, res) in [
                (
                    "new_sealed",
                    SeasonReplayLog::new_sealed(
                        spec_sv.clone(),
                        sealed_val,
                        deal_sv.clone(),
                        commits_sv.clone(),
                        turns_sv.clone(),
                        reveals_sv.clone(),
                    ),
                ),
                (
                    "from_rig_seed",
                    SeasonReplayLog::from_rig_seed(
                        spec_sv, sealed_val, deal_sv, commits_sv, turns_sv, reveals_sv,
                    ),
                ),
            ] {
                let err = must_err(res, "sealed coupled Eval VRF must fail");
                match &err {
                    EngineError::InvalidConfig(msg) => {
                        assert!(
                            msg.contains("SEALED_EVAL_VRF_NOT_INDEPENDENT"),
                            "{ctor_name} ({label}) unexpected error: {msg}"
                        );
                        assert_seed_absent(msg, sealed_val, &format!("{ctor_name} vrf error"));
                    }
                    other => panic!("{ctor_name} ({label}) expected InvalidConfig, got {other:?}"),
                }
            }
        }

        // Independent sealed setup succeeds, records claim `true`, and false claim fails replay.
        let base_tag = 180_u8.wrapping_add(s_idx as u8 * 4);
        let SeasonParts {
            spec: spec_ok,
            deal: deal_ok,
            mandate_commits: commits_ok,
            turns: turns_ok,
            mandate_reveals: reveals_ok,
            board: board_ok,
            digests: digests_ok,
        } = run_eval_season_parts(
            TableId(9_700 + s_idx as u64),
            sealed_val,
            clean_32(base_tag),
            clean_32(base_tag + 1),
            base_tag + 2,
        );
        let sealed_ok = must_ok(
            SeasonReplayLog::new_sealed(
                spec_ok, sealed_val, deal_ok, commits_ok, turns_ok, reveals_ok,
            ),
            "independent new_sealed",
        );
        assert_eq!(
            sealed_ok.seed.independence_checked_at_write_unverified(),
            Some(true)
        );
        let ok_out = must_ok(replay_season(&sealed_ok), "replay independent sealed");
        assert_eq!(ok_out.scoreboard, board_ok);
        assert_eq!(ok_out.state_digests, digests_ok);

        let mut tampered_val: serde_json::Value = must_ok(
            serde_json::from_slice(&must_ok(
                serde_json::to_vec(&sealed_ok),
                "serialize sealed_ok",
            )),
            "value sealed_ok",
        );
        tampered_val["seed"]["independence_checked_at_write_unverified"] =
            serde_json::Value::Bool(false);
        let tampered_bytes = must_ok(serde_json::to_vec(&tampered_val), "tampered_bytes");
        let tampered_log: SeasonReplayLog = must_ok(
            serde_json::from_slice(&tampered_bytes),
            "parse tampered claim=false log",
        );
        assert_eq!(
            tampered_log.seed.independence_checked_at_write_unverified(),
            Some(false)
        );
        let err_replay = must_err(
            replay_season(&tampered_log),
            "replay_season must reject independence_checked_at_write_unverified=false",
        );
        assert!(
            matches!(&err_replay, EngineError::InvalidConfig(m) if m.contains("SEALED_INDEPENDENCE_UNCHECKED")),
            "expected SEALED_INDEPENDENCE_UNCHECKED, got {err_replay:?}"
        );
        let err_pred = must_err(
            replay_season_with_state_predicates(&tampered_log),
            "replay_season_with_state_predicates must reject unchecked sealed log",
        );
        assert!(
            matches!(&err_pred, EngineError::InvalidConfig(m) if m.contains("SEALED_INDEPENDENCE_UNCHECKED")),
            "expected SEALED_INDEPENDENCE_UNCHECKED with predicates, got {err_pred:?}"
        );
        let tampered_rec = must_ok(
            ReplayRecord::from_json_slice(&tampered_bytes),
            "ReplayRecord::from_json_slice tampered",
        );
        assert!(matches!(
            replay_season(&tampered_rec.log),
            Err(EngineError::InvalidConfig(m)) if m.contains("SEALED_INDEPENDENCE_UNCHECKED")
        ));
    }
}
