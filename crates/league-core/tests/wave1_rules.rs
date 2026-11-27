//! Wave 1 Part B rule tests and cancel-lock spoof-bot measurements:
//! - B1: Cancel lock (`C4` amended `2026-11-17`), `EngineError::code()`, replay rejection parity,
//!   engine-cancel exemption, public predicate visibility, and spoof-bot + `#54` measurements.
//! - B2: One elected office per seat (`2026-11-17`): vacant chair start, Trade Minister first and
//!   excluded from Cartel Chair, vacancy when incumbent Chair becomes TM without votes, `M1`
//!   incumbent-kept, and beacon tiebreak over remaining candidates.
//! - B3: Bonded Customs Inspector freeze (`§5.2` amended `2026-11-17`): per-turn `FillRecord` log,
//!   `audit_wash_freeze` confirmed `(a)` round trip & `(b)` above `2.25x` import price, clean
//!   damages, insufficient bond CRED refusal, capped bounty, forward-only freeze, replay determinism,
//!   serde round-trip, and old log backwards compatibility.
//! - B4: `#29` `DrandBeaconAuthStatus` reported in `ReplayOutcome::drand_beacon_auth_status`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use league_core::{
    AssetId, AtomicSwapProposal, BondProposal, BondSettlementMode, CharterRole, CommitmentHash,
    ContractDeliveryOrder, ConverterRunOrder, CouncilPlatform, CouncilVoteBallot,
    DEFAULT_CANCEL_LOCK_BPS, DEFAULT_TOWN_IMPORT_PREMIUM_BPS, DEFAULT_WASH_FREEZE_BOND_CRED,
    DEFAULT_WASH_FREEZE_BOUNTY_CRED, DEFAULT_WASH_ROUND_TRIP_MIN_UNITS,
    DRAND_QUICKNET_GENESIS_UNIX, DrandBeaconAuthStatus, DrandSchedule, DualEscrowProposal,
    EngineError, EscrowProposal, EscrowStatus, FillKind, FillRecord, FundamentalParities,
    HARD_TURN_CAP, LoggedTurn, MarketAction, OfferProposal, OfferSide, OrderReveal,
    ReplayBeaconMode, ReplayLayoutSeed, ReplayOutcome, ReplayRecord, RevealSalt, SealedOrderBundle,
    SeasonReplayLog, Seat, SweepReceipt, TableId, TableMode, TableSpec, Turn, TurnEngine,
    TurnPhase, UNIT_SCALE, Units, attach_state_predicates, audit_wash_freeze,
    break_tie_with_turn_beacon, compute_order_commitment, derive_drand_beacon, derive_turn_beacon,
    mark_good_holding_at_close, replay_season, town_import_cost_cred,
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
        let rev = OrderReveal {
            bundle,
            salt: RevealSalt([0x21_u8.wrapping_add(s).wrapping_add(turn); 32]),
        };
        let commit = must_ok(
            compute_order_commitment(table_id, Turn(turn), Seat(s), &rev),
            "compute_order_commitment",
        );
        must_ok(
            engine.submit_order_commitment(Seat(s), commit),
            "submit_order_commitment",
        );
        commits.insert(s, commit);
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

fn game_spec_with_drand(
    table_id: TableId,
    seat_count: u8,
    seed: u64,
) -> (TableSpec, DrandSchedule, league_core::SealedMandateDeal) {
    let schedule = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX,
        turn_period_secs: 3,
    };
    let (spec, deal) = must_ok(
        TableSpec::standard_game_chiral_ring_with_mandates(table_id, seat_count, seed, schedule),
        "standard_game_chiral_ring_with_mandates",
    );
    (spec, schedule, deal)
}

fn quicknet_sig_for_round(round: u64) -> Vec<u8> {
    #[derive(serde::Deserialize)]
    struct BeaconVec {
        round: u64,
        signature: String,
    }
    #[derive(serde::Deserialize)]
    struct VectorFile {
        beacons: Vec<BeaconVec>,
    }
    let vf: VectorFile = must_ok(
        serde_json::from_str(include_str!("vectors/drand_quicknet_vectors.json")),
        "parse vendored quicknet vectors",
    );
    let Some(entry) = vf.beacons.into_iter().find(|b| b.round == round) else {
        panic!("missing vendored quicknet vector for round {round}");
    };
    must_ok(hex::decode(entry.signature), "decode quicknet sig hex")
}

// --- B1: Cancel lock + error codes + live/replay parity + engine exemption ----------------------

#[test]
fn b1_engine_error_codes_cover_every_variant_and_cancel_lock_state_lifecycle() {
    let all_variants = [
        EngineError::InvalidConfig("c".to_owned()),
        EngineError::InvalidPhase("p".to_owned()),
        EngineError::InvalidSeat(9),
        EngineError::InvalidOrder("o".to_owned()),
        EngineError::UnknownObject("u".to_owned()),
        EngineError::Unauthorized("x".to_owned()),
        EngineError::PerishableEscrowForbidden,
        EngineError::InsufficientBalance("b".to_owned()),
        EngineError::AlreadySubmitted(0),
        EngineError::CommitmentMismatch(0),
        EngineError::DomainAccessDenied("d".to_owned()),
        EngineError::CancelLocked,
        EngineError::SeasonEnded("s".to_owned()),
        EngineError::UnsupportedReplayVersion {
            found: 2,
            newest_known: 1,
        },
    ];
    let codes: BTreeSet<&'static str> = all_variants.iter().map(EngineError::code).collect();
    assert_eq!(codes.len(), all_variants.len());
    assert_eq!(EngineError::CancelLocked.code(), "CANCEL_LOCKED");
    assert_eq!(
        EngineError::UnsupportedReplayVersion {
            found: 2,
            newest_known: 1,
        }
        .code(),
        "UNSUPPORTED_REPLAY_VERSION"
    );

    let mut disabled_spec = must_ok(TableSpec::standard_chiral_ring(TableId(601), 4, 1), "spec");
    assert_eq!(disabled_spec.cancel_lock_bps, DEFAULT_CANCEL_LOCK_BPS);
    disabled_spec.cancel_lock_bps = 10_001;
    assert_eq!(
        disabled_spec.validate().err().map(|e| e.code()),
        Some("INVALID_CONFIG")
    );
    disabled_spec.cancel_lock_bps = 0;
    let mut eng_off = must_ok(TurnEngine::new(disabled_spec), "engine off");
    must_ok(
        eng_off.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 1_000,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "post offer off",
    );
    // With cancel_lock_bps == 0, begin_cancel_lock succeeds as a no-op and CancelOffer still works.
    must_ok(eng_off.begin_cancel_lock(), "lock noop when 0");
    assert!(!eng_off.state().cancel_locked);
    must_ok(
        eng_off.apply_market_action(MarketAction::CancelOffer {
            seat: 0,
            offer_id: 1,
        }),
        "cancel succeeds when lock disabled",
    );

    let spec_on = must_ok(
        TableSpec::standard_chiral_ring(TableId(602), 4, 2),
        "spec on",
    );
    let mut eng = must_ok(TurnEngine::new(spec_on), "engine on");
    must_ok(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 1_000,
                price_per_unit_cred: 1_600,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "post early offer",
    );
    must_ok(eng.begin_cancel_lock(), "begin lock");
    assert!(eng.state().cancel_locked);

    let cred_before = eng.state().seats[0].cred;
    let cancel_err = eng.apply_market_action(MarketAction::CancelOffer {
        seat: 0,
        offer_id: 1,
    });
    assert_eq!(cancel_err, Err(EngineError::CancelLocked));
    assert_eq!(eng.state().seats[0].cred, cred_before);
    assert!(eng.state().offers.contains_key(&1));

    // Posting remains permitted after cancel lock engages.
    must_ok(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 1_000,
                price_per_unit_cred: 1_500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "post after lock allowed",
    );

    must_ok(eng.advance_to_commit(), "commit clears lock");
    assert!(!eng.state().cancel_locked);
    assert_eq!(
        eng.begin_cancel_lock().err().map(|e| e.code()),
        Some("INVALID_PHASE")
    );
}

#[test]
fn b1_live_and_replay_reject_same_locked_cancels_and_omit_empty_rejections() {
    let tid = TableId(603);
    let (mut spec, sealed_deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 4, 3),
        "setup",
    );
    spec.closing_hazard_bps = [10_000; 8];
    let eval_pk = spec.eval_vrf_pubkey;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "engine");

    let mut turns = Vec::new();
    let mut expected_live_rejections: Vec<(u8, u32, &'static str)> = Vec::new();

    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut actions = Vec::new();
        let mut lock_at = None;
        if t == 1 {
            // Pre-lock rejection with a code other than CANCEL_LOCKED: cancelling an offer that
            // does not exist, before the lock begins, gives UNKNOWN_OBJECT at index 0.
            let a_pre = MarketAction::CancelOffer {
                seat: 1,
                offer_id: 998,
            };
            if let Err(err) = eng.apply_market_action(a_pre.clone()) {
                expected_live_rejections.push((t, 0, err.code()));
            }
            actions.push(a_pre);

            let a0 = MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(1),
                    amount: 1_000,
                    price_per_unit_cred: 1_400,
                    domain_id: None,
                    is_hybrid: false,
                },
            };
            must_ok(eng.apply_market_action(a0.clone()), "a0");
            actions.push(a0);

            must_ok(eng.begin_cancel_lock(), "live lock turn 1");
            lock_at = Some(2_u32);

            let a1 = MarketAction::CancelOffer {
                seat: 0,
                offer_id: 1,
            };
            if let Err(err) = eng.apply_market_action(a1.clone()) {
                expected_live_rejections.push((t, 2, err.code()));
            }
            actions.push(a1);

            let a2 = MarketAction::CancelOffer {
                seat: 1,
                offer_id: 999,
            };
            if let Err(err) = eng.apply_market_action(a2.clone()) {
                expected_live_rejections.push((t, 3, err.code()));
            }
            actions.push(a2);
        }

        let (commitments, reveals) =
            commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let (beacon, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
        let rec = must_ok(eng.sweep_turn_with_eval_vrf(&proof), "sweep");
        turns.push(LoggedTurn {
            turn: t,
            market_actions: actions,
            cancel_lock_at: lock_at,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(proof.to_vec()),
            drawn_shocks: rec.newly_drawn_shocks,
        });
        if rec.season_closed {
            break;
        }
    }

    let rev0 = sealed_deal.seat_reveals[0].clone();
    must_ok(eng.submit_mandate_reveal(Seat(0), rev0.clone()), "mandate");
    let live_board = must_ok(eng.finalize_season(), "finalize");

    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[0] = Some(rev0);
    let log = SeasonReplayLog {
        spec: spec.clone(),
        mode: TableMode::Eval,
        seed: must_ok(league_core::ReplayLayoutSeed::try_clear(3), "seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk,
        sealed_mandate_deal: sealed_deal.clone(),
        mandate_commits: vec![None; 4],
        turns: turns.clone(),
        mandate_reveals: mandate_reveals.clone(),
    };
    let replayed = must_ok(replay_season(&log), "replay");
    assert_eq!(replayed.scoreboard, live_board);
    assert_eq!(replayed.drand_beacon_auth_status, None);
    let flat_replayed: Vec<(u8, u32, String)> = replayed
        .rejected_market_actions
        .iter()
        .flat_map(|t| {
            t.rejections
                .iter()
                .map(move |r| (t.turn, r.action_index, r.error_code.clone()))
        })
        .collect();
    let expected_owned: Vec<(u8, u32, String)> = expected_live_rejections
        .iter()
        .map(|&(t, i, c)| (t, i, c.to_owned()))
        .collect();
    assert_eq!(flat_replayed, expected_owned);
    assert_eq!(
        flat_replayed,
        vec![
            (1, 0, "UNKNOWN_OBJECT".to_owned()),
            (1, 2, "CANCEL_LOCKED".to_owned()),
            (1, 3, "CANCEL_LOCKED".to_owned()),
        ]
    );

    // Out-of-bounds cancel_lock_at (> len) must fail cleanly.
    let mut bad_lock = log.clone();
    bad_lock.turns[0].cancel_lock_at = Some(99);
    assert_eq!(
        replay_season(&bad_lock).err().map(|e| e.code()),
        Some("INVALID_ORDER")
    );

    // Clean log with zero rejections omits rejected_market_actions and drand_beacon_auth_status from JSON.
    let (clean_spec, clean_deal, clean_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(TableId(604), 4, 4),
        "clean setup",
    );
    let mut c_spec = clean_spec;
    c_spec.closing_hazard_bps = [10_000; 8];
    let c_pk = c_spec.eval_vrf_pubkey;
    let mut c_eng = must_ok(TurnEngine::new(c_spec.clone()), "clean eng");
    let mut c_turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if c_eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) = commit_and_reveal_all(
            &mut c_eng,
            TableId(604),
            t,
            vec![SealedOrderBundle::default(); 4],
        );
        let (beacon, proof) = must_ok(clean_seed.prove_turn(TableId(604), Turn(t)), "prove");
        let rec = must_ok(c_eng.sweep_turn_with_eval_vrf(&proof), "sweep");
        c_turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commitments,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(proof.to_vec()),
            drawn_shocks: rec.newly_drawn_shocks,
        });
        if rec.season_closed {
            break;
        }
    }
    let clean_log = SeasonReplayLog {
        spec: c_spec.clone(),
        mode: TableMode::Eval,
        seed: must_ok(league_core::ReplayLayoutSeed::try_clear(4), "seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: c_pk,
        sealed_mandate_deal: clean_deal,
        mandate_commits: vec![None; 4],
        turns: c_turns,
        mandate_reveals: vec![None; 4],
    };
    let clean_out = must_ok(replay_season(&clean_log), "clean replay");
    assert!(clean_out.rejected_market_actions.is_empty());
    let json_str = must_ok(serde_json::to_string(&clean_out), "serialize outcome");
    assert!(!json_str.contains("rejected_market_actions"));
    assert!(!json_str.contains("drand_beacon_auth_status"));
}

#[test]
fn b1_engine_initiated_domain_offer_cancels_stay_exempt() {
    // Cartel domain eviction (Check bounce downgrading drawer to Tier C) cancels resting domain
    // offers inside Sweep without being subject to Phase-1 CancelOffer locking.
    let spec = must_ok(TableSpec::standard_chiral_ring(TableId(605), 4, 5), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "engine");
    let tid = TableId(605);

    // Turns 1..=2: Seat 0 draws 3 huge checks and bounces checks 1 and 2 (Aaa -> Bbb -> B).
    for t in 1..=4_u8 {
        if t == 1 {
            for cid_due in [1_u8, 2, 5] {
                must_ok(
                    eng.apply_market_action(MarketAction::CreateCheck {
                        seat: 0,
                        check: league_core::CheckProposal {
                            payee: 1,
                            asset: AssetId::Cred,
                            send_max: 9_999_999,
                            deliver_min: 9_999_999,
                            due_turn: cid_due,
                            expiration_turn: 6,
                        },
                    }),
                    "create bouncing check",
                );
            }
        }
        if t == 3 {
            must_ok(
                eng.apply_market_action(MarketAction::StandForOffice {
                    seat: 1,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(0, 0), (0, 1)],
                        cartel_revocations: Vec::new(),
                    },
                }),
                "stand",
            );
        }
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        match t {
            1 | 2 => bundles[1].check_cashes.push(u64::from(t)),
            4 => {
                bundles[0].council_vote = Some(CouncilVoteBallot {
                    trade_minister: 2,
                    cartel_chair: 1,
                });
            }
            _ => {}
        }
        commit_and_reveal_all(&mut eng, tid, t, bundles);
        must_ok(eng.sweep_turn(derive_turn_beacon(1, tid, Turn(t))), "sweep");
    }

    // Turn 5: Seat 0 (still Tier B, allowed in Domain 0) posts a domain offer, cancel lock
    // engages (`CancelOffer` fails), then Check 3 bounces (`B -> D`), evicting Seat 0 cleanly.
    must_ok(
        eng.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 1_000,
                price_per_unit_cred: 1_500,
                domain_id: Some(0),
                is_hybrid: false,
            },
        }),
        "post domain offer",
    );
    must_ok(eng.begin_cancel_lock(), "lock");
    assert_eq!(
        eng.apply_market_action(MarketAction::CancelOffer {
            seat: 0,
            offer_id: 1,
        }),
        Err(EngineError::CancelLocked)
    );
    let mut b5 = vec![SealedOrderBundle::default(); 4];
    b5[1].check_cashes.push(3);
    commit_and_reveal_all(&mut eng, tid, 5, b5);
    let r5 = must_ok(
        eng.sweep_turn(derive_turn_beacon(1, tid, Turn(5))),
        "sweep 5",
    );
    assert_eq!(r5.cartel_evictions, vec![0]);
    assert!(eng.state().offers.is_empty());
}

// --- B2: One elected office per seat (2026-11-17) ----------------------------------------------

#[test]
fn b2_one_elected_office_per_seat_exclusion_vacancy_incumbent_and_tiebreak() {
    let tid = TableId(606);
    let spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 6), "spec");
    let mut eng = must_ok(TurnEngine::new(spec), "engine");
    assert_eq!(eng.state().trade_minister, 0);
    assert_eq!(eng.state().cartel_chair, None);

    // Turns 1..=3
    for t in 1..=3_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        must_ok(eng.sweep_turn(derive_turn_beacon(2, tid, Turn(t))), "sweep");
    }

    // Turn 4: Seat 0 wins BOTH TM (3 votes) and tops CC (2 votes), while Seat 2 has 1 CC vote ->
    // TM goes to Seat 0, Seat 0 is excluded from CC, so CC goes to next candidate Seat 2!
    // Seat 0's CC admission on its platform must NOT execute, while Seat 2's CC platform DOES.
    must_ok(
        eng.apply_market_action(MarketAction::StandForOffice {
            seat: 0,
            platform: CouncilPlatform {
                proposed_tariffs_bps: vec![(1, (1_100, 900))],
                cartel_admissions: vec![(9, 0)],
                cartel_revocations: Vec::new(),
            },
        }),
        "stand 0",
    );
    must_ok(
        eng.apply_market_action(MarketAction::StandForOffice {
            seat: 2,
            platform: CouncilPlatform {
                proposed_tariffs_bps: Vec::new(),
                cartel_admissions: vec![(2, 2), (2, 3)],
                cartel_revocations: Vec::new(),
            },
        }),
        "stand 2",
    );
    let mut b4 = vec![SealedOrderBundle::default(); 4];
    b4[0].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 0,
    });
    b4[1].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 0,
    });
    b4[2].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 2,
    });
    commit_and_reveal_all(&mut eng, tid, 4, b4);
    let r4 = must_ok(
        eng.sweep_turn(derive_turn_beacon(2, tid, Turn(4))),
        "sweep 4",
    );
    assert_eq!(r4.elected_officers, Some((0, Some(2))));
    assert_eq!(eng.state().trade_minister, 0);
    assert_eq!(eng.state().cartel_chair, Some(2));
    assert_eq!(eng.state().seats[1].raw_tariff_bps, 1_100);
    assert_eq!(eng.state().domains.len(), 1);
    assert_eq!(eng.state().domains[0].domain_id, 2);

    // Turns 5..=8: M1 incumbent-kept when no eligible CC receives a vote and incumbent CC != TM.
    for t in 5..=7_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        must_ok(
            eng.sweep_turn(derive_turn_beacon(2, tid, Turn(t))),
            "sweep 5..7",
        );
    }
    // Vote TM = 1, CC = 1 (all CC votes go to TM winner Seat 1 -> excluded, leaving 0 CC votes;
    // incumbent Seat 2 != 1 so M1 keeps Some(2)).
    let mut b8 = vec![SealedOrderBundle::default(); 4];
    b8[0].council_vote = Some(CouncilVoteBallot {
        trade_minister: 1,
        cartel_chair: 1,
    });
    commit_and_reveal_all(&mut eng, tid, 8, b8);
    let r8 = must_ok(
        eng.sweep_turn(derive_turn_beacon(2, tid, Turn(8))),
        "sweep 8",
    );
    assert_eq!(r8.elected_officers, Some((1, Some(2))));
    assert_eq!(eng.state().trade_minister, 1);
    assert_eq!(eng.state().cartel_chair, Some(2));

    // Turns 9..=12: Incumbent CC (Seat 2) wins TM, and all CC votes also went to Seat 2 ->
    // zero eligible CC candidates -> Cartel Chair becomes vacant (`None`).
    for t in 9..=11_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        must_ok(
            eng.sweep_turn(derive_turn_beacon(2, tid, Turn(t))),
            "sweep 9..11",
        );
    }
    let mut b12 = vec![SealedOrderBundle::default(); 4];
    b12[0].council_vote = Some(CouncilVoteBallot {
        trade_minister: 2,
        cartel_chair: 2,
    });
    commit_and_reveal_all(&mut eng, tid, 12, b12);
    let r12 = must_ok(
        eng.sweep_turn(derive_turn_beacon(2, tid, Turn(12))),
        "sweep 12",
    );
    assert_eq!(r12.elected_officers, Some((2, None)));
    assert_eq!(eng.state().trade_minister, 2);
    assert_eq!(eng.state().cartel_chair, None);

    // Turns 13..=16: Tie among remaining eligible CC candidates [1, 3] after excluding TM=0!
    for t in 13..=15_u8 {
        commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        must_ok(
            eng.sweep_turn(derive_turn_beacon(2, tid, Turn(t))),
            "sweep 13..15",
        );
    }
    let mut b16 = vec![SealedOrderBundle::default(); 4];
    // Seat 0 gets 3 TM votes + 2 CC votes (excluded as TM!), Seats 1 and 3 tie with 1 CC vote each!
    b16[0].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 0,
    });
    b16[1].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 0,
    });
    b16[2].council_vote = Some(CouncilVoteBallot {
        trade_minister: 0,
        cartel_chair: 1,
    });
    b16[3].council_vote = Some(CouncilVoteBallot {
        trade_minister: 1,
        cartel_chair: 3,
    });

    let mut beacon_pick_1 = None;
    let mut beacon_pick_3 = None;
    for s in 0..200_u64 {
        let b = derive_turn_beacon(s, tid, Turn(16));
        match break_tie_with_turn_beacon(b, tid, Turn(16), b"cartel_chair", &[1, 3]) {
            1 if beacon_pick_1.is_none() => beacon_pick_1 = Some(b),
            3 if beacon_pick_3.is_none() => beacon_pick_3 = Some(b),
            _ => {}
        }
        if beacon_pick_1.is_some() && beacon_pick_3.is_some() {
            break;
        }
    }
    let Some(b_win_1) = beacon_pick_1 else {
        panic!("missing tie beacon selecting seat 1");
    };
    let Some(b_win_3) = beacon_pick_3 else {
        panic!("missing tie beacon selecting seat 3");
    };

    let mut eng_a = eng.clone();
    commit_and_reveal_all(&mut eng_a, tid, 16, b16.clone());
    let r16_a = must_ok(eng_a.sweep_turn(b_win_1), "sweep 16a");
    assert_eq!(r16_a.elected_officers, Some((0, Some(1))));

    commit_and_reveal_all(&mut eng, tid, 16, b16);
    let r16_b = must_ok(eng.sweep_turn(b_win_3), "sweep 16b");
    assert_eq!(r16_b.elected_officers, Some((0, Some(3))));
}

// --- B3: Bonded Customs Inspector freeze (§5.2 amended 2026-11-17) ------------------------------

fn grant_inspector_charter(eng: &mut TurnEngine, inspector: u8, tid: TableId) {
    for t in 1..=6_u8 {
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        if t == 6 {
            bundles[usize::from(inspector)]
                .charter_bids
                .push(league_core::CharterBidOrder {
                    role: CharterRole::CustomsInspector,
                    bid_cred: 100,
                });
        }
        commit_and_reveal_all(eng, tid, t, bundles);
        must_ok(
            eng.sweep_turn(derive_turn_beacon(9, tid, Turn(t))),
            "setup sweep",
        );
    }
    assert!(
        eng.state()
            .charters
            .iter()
            .any(|c| c.role == CharterRole::CustomsInspector && c.holder == inspector)
    );
}

#[test]
fn b3_customs_inspector_audit_confirmed_a_confirmed_b_clean_bounty_cap_insufficient_and_serde() {
    let tid = TableId(607);
    let spec = must_ok(TableSpec::standard_chiral_ring(tid, 4, 7), "spec");
    assert_eq!(
        spec.wash_round_trip_min_units,
        DEFAULT_WASH_ROUND_TRIP_MIN_UNITS
    );
    assert_eq!(spec.wash_freeze_bond_cred, DEFAULT_WASH_FREEZE_BOND_CRED);
    assert_eq!(
        spec.wash_freeze_bounty_cred,
        DEFAULT_WASH_FREEZE_BOUNTY_CRED
    );

    let parities = FundamentalParities::from_spec(&spec);
    let raw0_import_price = must_ok(
        town_import_cost_cred(
            UNIT_SCALE,
            parities.asset_parity(AssetId::Raw(0)),
            DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
        ),
        "import price",
    );

    // Pure predicate unit checks: Authority TownImport (from: None) is excluded from (a) and (b).
    let import_only = vec![FillRecord {
        turn: 1,
        kind: FillKind::TownImport,
        good: Some(AssetId::Raw(0)),
        from: None,
        to: 1,
        units: 2_000,
        net_units: 2_000,
        cred_amount: raw0_import_price * 2 + 100,
        unit_price_cred: Some(raw0_import_price + 50),
    }];
    assert!(!audit_wash_freeze(
        &import_only,
        1,
        &parities,
        DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
        DEFAULT_WASH_ROUND_TRIP_MIN_UNITS,
    ));

    // Serde round-trip on FillRecord & all FillKind variants.
    for kind in [
        FillKind::BatchFill,
        FillKind::AtomicSwap,
        FillKind::ContractDelivery,
        FillKind::CheckCashed,
        FillKind::EscrowFinished,
        FillKind::EscrowForfeited,
        FillKind::EscrowBreachRefunded,
        FillKind::EscrowRefunded,
        FillKind::TownImport,
    ] {
        let rec = FillRecord {
            turn: 3,
            kind,
            good: Some(AssetId::Raw(1)),
            from: Some(2),
            to: 3,
            units: 1_250,
            net_units: 1_125,
            cred_amount: 2_000,
            unit_price_cred: Some(1_600),
        };
        let bytes = must_ok(serde_json::to_vec(&rec), "ser fill");
        let parsed: FillRecord = must_ok(serde_json::from_slice(&bytes), "de fill");
        assert_eq!(parsed, rec);
    }

    let mut base_eng = must_ok(TurnEngine::new(spec.clone()), "engine");
    grant_inspector_charter(&mut base_eng, 0, tid);

    // Case 1: Clean challenge (Turn 7 freeze on seat 2 with zero suspicious Turn 6 fills) ->
    // Inspector loses the whole bond to the target seat as damages (none returns to the
    // Inspector); freeze itself stays forward-only on Turn 7.
    let mut clean_eng = base_eng.clone();
    let insp_cred_before = clean_eng.state().seats[0].cred;
    let target_cred_before = clean_eng.state().seats[2].cred;
    must_ok(
        clean_eng.apply_market_action(MarketAction::FlagWashFreeze {
            inspector: 0,
            target_seat: 2,
        }),
        "clean freeze",
    );
    assert_eq!(
        clean_eng.state().seats[0].cred,
        insp_cred_before - DEFAULT_WASH_FREEZE_BOND_CRED
    );
    assert_eq!(clean_eng.state().seats[2].wash_frozen_until_turn, 7);
    commit_and_reveal_all(
        &mut clean_eng,
        tid,
        7,
        vec![SealedOrderBundle::default(); 4],
    );
    let clean_r7 = must_ok(
        clean_eng.sweep_turn(derive_turn_beacon(3, tid, Turn(7))),
        "clean sweep 7",
    );
    assert_eq!(clean_r7.wash_freeze_settlements.len(), 1);
    let wf_clean = clean_r7.wash_freeze_settlements[0];
    assert!(!wf_clean.confirmed);
    assert_eq!(wf_clean.bond_returned_cred, 0);
    assert_eq!(wf_clean.bounty_paid_cred, 0);
    assert_eq!(wf_clean.damages_paid_cred, DEFAULT_WASH_FREEZE_BOND_CRED);
    assert_eq!(
        clean_eng.state().seats[0].cred,
        insp_cred_before - DEFAULT_WASH_FREEZE_BOND_CRED
    );
    assert_eq!(
        clean_eng.state().seats[2].cred,
        target_cred_before + DEFAULT_WASH_FREEZE_BOND_CRED
    );

    // Clean challenge with a non-default bond: the whole bond, whatever its size, goes to the
    // frozen seat and none returns to the Inspector (§5.2 amended 2026-11-17, Publisher ruling).
    let big_bond: Units = 700;
    let mut big_spec = spec.clone();
    big_spec.wash_freeze_bond_cred = big_bond;
    let mut big_eng = must_ok(TurnEngine::new(big_spec), "big-bond engine");
    grant_inspector_charter(&mut big_eng, 0, tid);
    let big_insp_before = big_eng.state().seats[0].cred;
    let big_target_before = big_eng.state().seats[2].cred;
    must_ok(
        big_eng.apply_market_action(MarketAction::FlagWashFreeze {
            inspector: 0,
            target_seat: 2,
        }),
        "big-bond clean freeze",
    );
    commit_and_reveal_all(&mut big_eng, tid, 7, vec![SealedOrderBundle::default(); 4]);
    let big_r7 = must_ok(
        big_eng.sweep_turn(derive_turn_beacon(3, tid, Turn(7))),
        "big-bond sweep 7",
    );
    let wf_big = big_r7.wash_freeze_settlements[0];
    assert!(!wf_big.confirmed);
    assert_eq!(wf_big.bond_returned_cred, 0);
    assert_eq!(wf_big.damages_paid_cred, big_bond);
    assert_eq!(big_eng.state().seats[0].cred, big_insp_before - big_bond);
    assert_eq!(big_eng.state().seats[2].cred, big_target_before + big_bond);

    // Insufficient bond CRED must refuse FlagWashFreeze with InsufficientBalance.
    let mut broke_eng = base_eng.clone();
    must_ok(
        broke_eng.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: league_core::EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: broke_eng.state().seats[0].cred - (DEFAULT_WASH_FREEZE_BOND_CRED - 1),
                finish_after_turn: Some(7),
                cancel_after_turn: 9,
                condition_hash: None,
                bond: None,
            },
        }),
        "drain free cred",
    );
    let broke_err = broke_eng.apply_market_action(MarketAction::FlagWashFreeze {
        inspector: 0,
        target_seat: 2,
    });
    assert_eq!(
        broke_err.err().map(|e| e.code()),
        Some("INSUFFICIENT_BALANCE")
    );

    // Case 2: Confirmed (a) round-trip in Turn 7 (Seat 1 <-> Seat 2 on Raw(1) >= 1_000 each way),
    // caught on Turn 8 FlagWashFreeze with bond returned + bounty paid; Turn 7 fills stay intact!
    let mut rt_eng = base_eng.clone();
    let rt_swap_1 = AtomicSwapProposal {
        swap_id: 701,
        maker: 1,
        taker: 2,
        maker_asset: AssetId::Raw(1),
        maker_amount: 1_200,
        taker_asset: AssetId::Cred,
        taker_amount: 1_200,
        domain_id: None,
    };
    let rt_swap_2 = AtomicSwapProposal {
        swap_id: 702,
        maker: 2,
        taker: 1,
        maker_asset: AssetId::Raw(1),
        maker_amount: 1_000,
        taker_asset: AssetId::Cred,
        taker_amount: 1_200,
        domain_id: None,
    };
    let mut b7 = vec![SealedOrderBundle::default(); 4];
    b7[1].atomic_swaps = vec![rt_swap_1.clone(), rt_swap_2.clone()];
    b7[2].atomic_swaps = vec![rt_swap_1, rt_swap_2];
    commit_and_reveal_all(&mut rt_eng, tid, 7, b7);
    let r7_rt = must_ok(
        rt_eng.sweep_turn(derive_turn_beacon(3, tid, Turn(7))),
        "sweep 7 rt",
    );
    assert_eq!(r7_rt.atomic_swaps_settled, 2);
    assert!(r7_rt.fills.iter().any(|f| f.kind == FillKind::AtomicSwap));

    let raw1_after_t7_target = rt_eng.state().seats[2].raw[1];
    let rt_insp_before = rt_eng.state().seats[0].cred;
    let rt_target_before = rt_eng.state().seats[2].cred;
    must_ok(
        rt_eng.apply_market_action(MarketAction::FlagWashFreeze {
            inspector: 0,
            target_seat: 2,
        }),
        "rt freeze turn 8",
    );
    // Forward-only: Turn 7 holdings/swaps were NOT rolled back!
    assert_eq!(rt_eng.state().seats[2].raw[1], raw1_after_t7_target);
    commit_and_reveal_all(&mut rt_eng, tid, 8, vec![SealedOrderBundle::default(); 4]);
    let r8_rt = must_ok(
        rt_eng.sweep_turn(derive_turn_beacon(3, tid, Turn(8))),
        "sweep 8 rt",
    );
    assert_eq!(r8_rt.wash_freeze_settlements.len(), 1);
    let wf_rt = r8_rt.wash_freeze_settlements[0];
    assert!(wf_rt.confirmed);
    assert_eq!(wf_rt.bond_returned_cred, DEFAULT_WASH_FREEZE_BOND_CRED);
    assert_eq!(wf_rt.bounty_paid_cred, DEFAULT_WASH_FREEZE_BOUNTY_CRED);
    assert_eq!(wf_rt.damages_paid_cred, 0);
    assert_eq!(
        rt_eng.state().seats[0].cred,
        rt_insp_before + DEFAULT_WASH_FREEZE_BOUNTY_CRED
    );
    assert_eq!(
        rt_eng.state().seats[2].cred,
        rt_target_before - DEFAULT_WASH_FREEZE_BOUNTY_CRED
    );

    // Case 3: Confirmed (b) peer batch fill above 2.25x parity in Turn 7, AND test bounty capped
    // when target locks almost all remaining CRED into escrow before Sweep on Turn 8!
    let mut hi_eng = base_eng;
    let raw3_import_price = must_ok(
        town_import_cost_cred(
            UNIT_SCALE,
            parities.asset_parity(AssetId::Raw(3)),
            DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
        ),
        "raw3 import price",
    );
    let over_import_price = raw3_import_price + 100;
    must_ok(
        hi_eng.apply_market_action(MarketAction::PostOffer {
            seat: 2,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(3),
                amount: 1_000,
                price_per_unit_cred: over_import_price,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "over-import buy",
    );
    must_ok(
        hi_eng.apply_market_action(MarketAction::PostOffer {
            seat: 3,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(3),
                amount: 1_000,
                price_per_unit_cred: over_import_price,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "over-import sell",
    );
    commit_and_reveal_all(&mut hi_eng, tid, 7, vec![SealedOrderBundle::default(); 4]);
    let r7_hi = must_ok(
        hi_eng.sweep_turn(derive_turn_beacon(4, tid, Turn(7))),
        "sweep 7 hi",
    );
    assert_eq!(r7_hi.batch_trades_cleared, 1);
    assert!(r7_hi.fills.iter().any(|f| {
        f.kind == FillKind::BatchFill && f.unit_price_cred == Some(over_import_price)
    }));

    // Leave target seat 2 with only 120 free CRED before Turn 8 sweep!
    let leave_cred = 120_i64;
    let lock_amt = hi_eng.state().seats[2].cred - leave_cred;
    must_ok(
        hi_eng.apply_market_action(MarketAction::CreateEscrow {
            seat: 2,
            escrow: league_core::EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: lock_amt,
                finish_after_turn: Some(8),
                cancel_after_turn: 12,
                condition_hash: None,
                bond: None,
            },
        }),
        "lock target cred",
    );
    must_ok(
        hi_eng.apply_market_action(MarketAction::FlagWashFreeze {
            inspector: 0,
            target_seat: 2,
        }),
        "freeze over-import",
    );
    let insp_t8_before = hi_eng.state().seats[0].cred;
    commit_and_reveal_all(&mut hi_eng, tid, 8, vec![SealedOrderBundle::default(); 4]);
    let r8_hi = must_ok(
        hi_eng.sweep_turn(derive_turn_beacon(4, tid, Turn(8))),
        "sweep 8 hi",
    );
    let wf_hi = r8_hi.wash_freeze_settlements[0];
    assert!(wf_hi.confirmed);
    assert_eq!(wf_hi.bounty_paid_cred, leave_cred);
    assert_eq!(
        hi_eng.state().seats[0].cred,
        insp_t8_before + DEFAULT_WASH_FREEZE_BOND_CRED + leave_cred
    );
    assert_eq!(hi_eng.state().seats[2].cred, 0);
}

#[test]
fn b3_old_logs_without_part_b_fields_still_replay_identically_and_b4_drand_status_is_recorded() {
    let tid = TableId(608);
    let (mut spec, schedule, deal) = game_spec_with_drand(tid, 4, 8);
    spec.closing_hazard_bps = [10_000; 8];

    // Serialize TableSpec and strip the new Part B fields to verify backwards-compatible serde
    // defaults on pre-Part-B logs.
    let mut spec_val = must_ok(serde_json::to_value(&spec), "spec json");
    if let Some(obj) = spec_val.as_object_mut() {
        obj.remove("cancel_lock_bps");
        obj.remove("wash_round_trip_min_units");
        obj.remove("wash_freeze_bond_cred");
        obj.remove("wash_freeze_bounty_cred");
    }
    let compat_spec: TableSpec = must_ok(serde_json::from_value(spec_val), "compat TableSpec");
    assert_eq!(compat_spec, spec);

    // Run Game mode season and verify ReplayOutcome::drand_beacon_auth_status == Some(Authenticated)
    // and deterministic replay on both JSON with and without `cancel_lock_at`.
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "game eng");
    let mut turns = Vec::new();
    let mut live_digests = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) =
            commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let round = must_ok(schedule.expected_round_for_turn(Turn(t)), "round");
        let sig = quicknet_sig_for_round(round);
        let beacon = must_ok(derive_drand_beacon(&sig), "beacon");
        let rec = must_ok(eng.sweep_turn_with_drand(round, &sig), "sweep drand");
        live_digests.push(rec.post_state_digest);
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
            drawn_shocks: rec.newly_drawn_shocks,
        });
        if rec.season_closed {
            break;
        }
    }
    let rev0 = deal.seat_reveals[0].clone();
    must_ok(eng.submit_mandate_reveal(Seat(0), rev0.clone()), "rev0");
    let live_board = must_ok(eng.finalize_season(), "finalize");

    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[0] = Some(rev0);
    let log = SeasonReplayLog {
        spec: spec.clone(),
        mode: TableMode::Game,
        seed: must_ok(league_core::ReplayLayoutSeed::try_clear(0), "game seed"),
        beacon_mode: ReplayBeaconMode::ExternalDrand,
        eval_vrf_pubkey: None,
        sealed_mandate_deal: deal,
        mandate_commits: vec![None; 4],
        turns,
        mandate_reveals,
    };
    let log_json = must_ok(serde_json::to_string(&log), "serialize game log");
    assert!(!log_json.contains("cancel_lock_at"));
    let parsed_log: SeasonReplayLog =
        must_ok(serde_json::from_str(&log_json), "parse old-shape log");
    let out = must_ok(replay_season(&parsed_log), "replay game");
    assert_eq!(out.state_digests, live_digests);
    assert_eq!(out.scoreboard, live_board);
    assert_eq!(
        out.drand_beacon_auth_status,
        Some(DrandBeaconAuthStatus::Authenticated)
    );
}

#[test]
fn b3_bonded_freezes_confirmed_and_clean_replay_bit_for_bit() {
    let tid = TableId(609);
    let (mut spec, sealed_deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 4, 7),
        "setup",
    );
    spec.closing_hazard_bps = [10_000; 8];
    let eval_pk = spec.eval_vrf_pubkey;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "engine");

    // Turn 6: seat 0 buys the Customs Inspector charter. Turn 7: seats 1 and 2 round-trip Raw(1).
    // Turn 8: seat 0 freezes seat 2 (confirmed by the Turn 7 round trip) and seat 3 (clean).
    let rt_swap_1 = AtomicSwapProposal {
        swap_id: 901,
        maker: 1,
        taker: 2,
        maker_asset: AssetId::Raw(1),
        maker_amount: 1_200,
        taker_asset: AssetId::Cred,
        taker_amount: 1_200,
        domain_id: None,
    };
    let rt_swap_2 = AtomicSwapProposal {
        swap_id: 902,
        maker: 2,
        taker: 1,
        maker_asset: AssetId::Raw(1),
        maker_amount: 1_000,
        taker_asset: AssetId::Cred,
        taker_amount: 1_200,
        domain_id: None,
    };

    let mut turns = Vec::new();
    let mut live_receipts = Vec::new();
    let mut live_digests = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut actions = Vec::new();
        let mut bundles = vec![SealedOrderBundle::default(); 4];
        match t {
            6 => bundles[0].charter_bids.push(league_core::CharterBidOrder {
                role: CharterRole::CustomsInspector,
                bid_cred: 100,
            }),
            7 => {
                bundles[1].atomic_swaps = vec![rt_swap_1.clone(), rt_swap_2.clone()];
                bundles[2].atomic_swaps = vec![rt_swap_1.clone(), rt_swap_2.clone()];
            }
            8 => {
                for target_seat in [2_u8, 3] {
                    let action = MarketAction::FlagWashFreeze {
                        inspector: 0,
                        target_seat,
                    };
                    must_ok(eng.apply_market_action(action.clone()), "freeze");
                    actions.push(action);
                }
            }
            _ => {}
        }
        let (commitments, reveals) = commit_and_reveal_all(&mut eng, tid, t, bundles);
        let (beacon, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
        let rec = must_ok(eng.sweep_turn_with_eval_vrf(&proof), "sweep");
        live_digests.push(rec.post_state_digest);
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
            drawn_shocks: rec.newly_drawn_shocks.clone(),
        });
        let closed = rec.season_closed;
        live_receipts.push(rec);
        if closed {
            break;
        }
    }

    // The live season really holds one confirmed and one clean bonded freeze.
    let r8 = &live_receipts[7];
    assert_eq!(r8.wash_freeze_settlements.len(), 2);
    let confirmed = r8.wash_freeze_settlements[0];
    assert_eq!(confirmed.target_seat, 2);
    assert!(confirmed.confirmed);
    assert_eq!(confirmed.bond_returned_cred, DEFAULT_WASH_FREEZE_BOND_CRED);
    let clean = r8.wash_freeze_settlements[1];
    assert_eq!(clean.target_seat, 3);
    assert!(!clean.confirmed);
    assert_eq!(clean.bond_returned_cred, 0);
    assert_eq!(clean.damages_paid_cred, DEFAULT_WASH_FREEZE_BOND_CRED);
    assert!(
        live_receipts[6]
            .fills
            .iter()
            .any(|f| f.kind == FillKind::AtomicSwap)
    );

    let rev0 = sealed_deal.seat_reveals[0].clone();
    must_ok(eng.submit_mandate_reveal(Seat(0), rev0.clone()), "mandate");
    let live_board = must_ok(eng.finalize_season(), "finalize");

    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[0] = Some(rev0);
    let log = SeasonReplayLog {
        spec,
        mode: TableMode::Eval,
        seed: must_ok(league_core::ReplayLayoutSeed::try_clear(7), "seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk,
        sealed_mandate_deal: sealed_deal,
        mandate_commits: vec![None; 4],
        turns,
        mandate_reveals,
    };
    let log_json = must_ok(serde_json::to_string(&log), "serialize log");
    let parsed_log: SeasonReplayLog = must_ok(serde_json::from_str(&log_json), "parse log");
    for (label, replay_log) in [("in-memory", &log), ("json", &parsed_log)] {
        let out = must_ok(replay_season(replay_log), label);
        assert_eq!(out.state_digests, live_digests, "{label}: digests");
        assert_eq!(out.scoreboard, live_board, "{label}: scoreboard");
        assert!(
            out.rejected_market_actions.is_empty(),
            "{label}: rejections"
        );
        assert_eq!(
            out.receipts.len(),
            live_receipts.len(),
            "{label}: receipt count"
        );
        for (live, replayed) in live_receipts.iter().zip(&out.receipts) {
            assert_eq!(replayed.fills, live.fills, "{label}: fills");
            assert_eq!(
                replayed.wash_freeze_settlements, live.wash_freeze_settlements,
                "{label}: wash-freeze settlements"
            );
        }
        assert_eq!(out.receipts, live_receipts, "{label}: receipts");
    }
}

// --- B1 Spoof-Bot & #54 Close-Mark Measurements (league-core standard chiral ring) -------------
//
// What the spoof bot is, exactly:
// - The table is league-core's `TableSpec::standard_chiral_ring`, not a league-gen generated
//   table. In that builder the seed enters the layout only as `seed % 5` (a +/-100 milli-CRED
//   starting-CRED spread), so any number of seeds is five layouts; the seed also sets the
//   `TableId`, which varies the turn beacons and the closing turn.
// - The bot calls `begin_cancel_lock()` before every pull, unconditionally.
// - The bot supplies its own crossing ask (seat 1 asks at `parity + 200` against seat 0's
//   `parity + 500` bid), so every bid it does not pull fills at Step 0.
// So the figures show that the lock works as coded, not how a spoofer behaves. Measuring real
// bots on league-gen tables waits for a TrackerBot or OpportunistBot.

struct SpoofMeasurement {
    seeds_per_n: u64,
    pulls_attempted_off: u64,
    pulls_attempted_on: u64,
    pulls_refused_off: u64,
    pulls_refused_on: u64,
    bids_exec_step0_off: u64,
    bids_exec_step0_on: u64,
    cred_locked_off: i128,
    cred_locked_on: i128,
    total_equity_delta: i128,
    issue54_lift_off: i128,
    issue54_lift_on: i128,
}

fn run_single_season_for_spoof(
    seed: u64,
    n: u8,
    cancel_lock_bps: u16,
) -> (u64, u64, u64, i128, Units, i128) {
    // Seeds stay below 1_000 so the TableIds below stay distinct per N.
    assert!(
        seed < 1_000,
        "spoof measurement seeds must stay below 1_000"
    );
    let tid = TableId(20_000 + u64::from(n) * 1_000 + seed);
    let mut spec = must_ok(TableSpec::standard_chiral_ring(tid, n, seed), "ring spec");
    spec.cancel_lock_bps = cancel_lock_bps;

    let mut eng = must_ok(TurnEngine::new(spec.clone()), "engine");
    let mut pulls_attempted = 0_u64;
    let mut pulls_refused = 0_u64;
    let mut bids_exec_step0 = 0_u64;
    let mut cred_locked_total = 0_i128;

    let good = AssetId::Raw(1);
    let parity = eng.state().parities.asset_parity(good);
    let spoof_bid_price = parity + 500;
    let cross_ask_price = parity + 200;
    let bid_amount = 1_000_i64;
    let mut close_turn_mark_lift = 0_i128;

    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let turn_beacon = derive_turn_beacon(seed, tid, Turn(t));
        // Early in Phase 1: Seat 0 posts a visible above-parity support bid (when solvent), and
        // Seat 1 posts a competitive ask crossing it so any uncancelled bid executes at Step 0.
        let cred_needed = league_core::checked_ceil_units_parity(bid_amount, spoof_bid_price)
            .unwrap_or(Units::MAX);
        let mut posted_bid_id = None;
        if eng.state().seats[0].cred >= cred_needed && eng.state().seats[1].raw[1] >= bid_amount {
            let next_id = eng.state().next_offer_id;
            if eng
                .apply_market_action(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good,
                        amount: bid_amount,
                        price_per_unit_cred: spoof_bid_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .is_ok()
            {
                posted_bid_id = Some(next_id);
                let _ = eng.apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good,
                        amount: bid_amount,
                        price_per_unit_cred: cross_ask_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                });
            }
        }

        // Engage cancel lock at `(10_000 - cancel_lock_bps) / 10_000` (no-op when cancel_lock_bps == 0).
        must_ok(eng.begin_cancel_lock(), "begin lock");

        // Late in Phase 1: Seat 0 tries to pull its spoof support bid.
        if let Some(offer_id) = posted_bid_id {
            pulls_attempted += 1;
            match eng.apply_market_action(MarketAction::CancelOffer { seat: 0, offer_id }) {
                Ok(()) => {}
                Err(EngineError::CancelLocked) => pulls_refused += 1,
                Err(other) => panic!("unexpected cancel error: {other}"),
            }
            if let Some(offer) = eng.state().offers.get(&offer_id) {
                cred_locked_total += i128::from(offer.locked_cred);
            }
        }

        // Probe #54 close-mark effect: what if Seat 0 ALSO posts an unhittable 0.001-unit high bid
        // (amount = 1, price = 100_000) at the end of Phase 1 (after cancel lock already engaged)
        // targeting Seat 1's held Refined(1) units on a stochastic/cap closing turn?
        let mut sim_close_check = eng.clone();
        let _ = commit_and_reveal_all(
            &mut sim_close_check,
            tid,
            t,
            vec![SealedOrderBundle::default(); usize::from(n)],
        );
        let probe_rec = must_ok(sim_close_check.sweep_turn(turn_beacon), "probe sweep");
        if probe_rec.season_closed {
            // Compute Seat 1's Refined(1) holding mark without the 0.001-unit late bid vs with it.
            // Solvent non-self Seat 2 posts at the end of Phase 1 so spoof CRED drain cannot skip the probe.
            let target_good = AssetId::Refined(1);
            let target_units = eng.state().seats[1].refined[1].max(1_000);
            let target_parity = eng.state().parities.asset_parity(target_good);
            let mark_before = mark_good_holding_at_close(eng.state(), 1, target_good, target_units);
            let mut with_tiny_bid = eng.clone();
            let tiny_bid_price = target_parity.saturating_mul(20);
            must_ok(
                with_tiny_bid.apply_market_action(MarketAction::PostOffer {
                    seat: 2,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: target_good,
                        amount: 1, // 0.001 units (#54 unhittable dust bid)
                        price_per_unit_cred: tiny_bid_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }),
                "post #54 dust bid",
            );
            let mark_after =
                mark_good_holding_at_close(with_tiny_bid.state(), 1, target_good, target_units);
            close_turn_mark_lift = i128::from(mark_after - mark_before);
        }

        // Convert raw to refined cleanly so balances sustain across the season.
        let mut bundles = vec![SealedOrderBundle::default(); usize::from(n)];
        for (s_idx, b) in bundles.iter_mut().enumerate() {
            if let Some(conv) = spec.seats[s_idx].converters.first() {
                b.converter_runs.push(ConverterRunOrder {
                    recipe_idx: 0,
                    runs: conv.max_runs_per_turn,
                });
            }
        }
        commit_and_reveal_all(&mut eng, tid, t, bundles);
        let rec = must_ok(eng.sweep_turn(turn_beacon), "sweep turn");
        for fill in &rec.fills {
            if fill.kind == FillKind::BatchFill && fill.to == 0 && fill.good == Some(good) {
                bids_exec_step0 += 1;
            }
        }
        if rec.season_closed {
            break;
        }
    }

    let board = must_ok(eng.finalize_season(), "finalize");
    let spoofer_eq = board
        .standings
        .iter()
        .find(|e| e.seat == 0)
        .map_or(0, |e| e.total_equity);
    (
        pulls_attempted,
        pulls_refused,
        bids_exec_step0,
        cred_locked_total,
        spoofer_eq,
        close_turn_mark_lift,
    )
}

fn measure_spoof_bot(seeds_per_n: u64) -> BTreeMap<u8, SpoofMeasurement> {
    let mut out = BTreeMap::new();
    for n in [4_u8, 5, 6] {
        let mut agg = SpoofMeasurement {
            seeds_per_n,
            pulls_attempted_off: 0,
            pulls_attempted_on: 0,
            pulls_refused_off: 0,
            pulls_refused_on: 0,
            bids_exec_step0_off: 0,
            bids_exec_step0_on: 0,
            cred_locked_off: 0,
            cred_locked_on: 0,
            total_equity_delta: 0,
            issue54_lift_off: 0,
            issue54_lift_on: 0,
        };
        for seed in 0..seeds_per_n {
            let (pa_off, pr_off, ex_off, lock_off, eq_off, lift_off) =
                run_single_season_for_spoof(seed, n, 0);
            let (pa_on, pr_on, ex_on, lock_on, eq_on, lift_on) =
                run_single_season_for_spoof(seed, n, DEFAULT_CANCEL_LOCK_BPS);
            agg.pulls_attempted_off += pa_off;
            agg.pulls_attempted_on += pa_on;
            agg.pulls_refused_off += pr_off;
            agg.pulls_refused_on += pr_on;
            agg.bids_exec_step0_off += ex_off;
            agg.bids_exec_step0_on += ex_on;
            agg.cred_locked_off += lock_off;
            agg.cred_locked_on += lock_on;
            agg.total_equity_delta += i128::from(eq_on) - i128::from(eq_off);
            agg.issue54_lift_off += lift_off;
            agg.issue54_lift_on += lift_on;
        }
        eprintln!(
            "SPOOF_BOT N={n} seeds={}: pulls_att(off={},on={}) refused(off={},on={}) step0_exec(off={},on={}) cred_locked(off={},on={}) spoofer_eq_delta_total={} mean_eq_delta={:.2} issue54_total_lift(off={},on={})",
            agg.seeds_per_n,
            agg.pulls_attempted_off,
            agg.pulls_attempted_on,
            agg.pulls_refused_off,
            agg.pulls_refused_on,
            agg.bids_exec_step0_off,
            agg.bids_exec_step0_on,
            agg.cred_locked_off,
            agg.cred_locked_on,
            agg.total_equity_delta,
            (agg.total_equity_delta as f64) / (seeds_per_n as f64),
            agg.issue54_lift_off,
            agg.issue54_lift_on,
        );
        out.insert(n, agg);
    }
    out
}

#[test]
fn cancel_lock_spoof_bot_5_ring_seeds_per_n_direction_and_issue54_report() {
    let summary = measure_spoof_bot(5);
    for (n, m) in &summary {
        assert_eq!(m.pulls_refused_off, 0, "N={n}: off refused pulls");
        assert!(m.pulls_refused_on > 0, "N={n}: lock must refuse late pulls");
        assert_eq!(m.bids_exec_step0_off, 0, "N={n}: off executes pulled bids");
        assert!(
            m.bids_exec_step0_on > 0,
            "N={n}: uncancelled spoof bids must execute at Step 0"
        );
        assert!(m.cred_locked_on > m.cred_locked_off);
        assert_eq!(
            m.issue54_lift_off, 0,
            "N={n}: #54 close-mark dust bid lift == 0"
        );
        assert_eq!(
            m.issue54_lift_on, 0,
            "N={n}: #54 close-mark dust bid lift == 0 under cancel lock ({} vs {})",
            m.issue54_lift_on, m.issue54_lift_off
        );
    }
}

#[test]
#[ignore = "explicit run: 200 standard-ring seeds per N (five layouts, seed % 5), -- --ignored --nocapture"]
fn cancel_lock_spoof_bot_200_ring_seeds_per_n_full_measurement() {
    let summary = measure_spoof_bot(200);
    for (n, m) in &summary {
        assert_eq!(m.pulls_refused_off, 0, "N={n}");
        assert!(m.pulls_refused_on > 0, "N={n}");
        assert!(m.bids_exec_step0_on > m.bids_exec_step0_off, "N={n}");
        assert_eq!(m.issue54_lift_off, 0, "N={n}");
        assert_eq!(m.issue54_lift_on, 0, "N={n}");
    }
}

// --- Wave 2 Card 1: FillKind::EscrowBreachRefunded vs EscrowForfeited vs EscrowRefunded --------

#[test]
fn escrow_breach_refunded_forfeited_and_refunded_fills_and_old_log_compat() {
    // 1. Variant serialization tag & trait coverage.
    let encoded_kind = must_ok(
        serde_json::to_string(&FillKind::EscrowBreachRefunded),
        "serialize FillKind::EscrowBreachRefunded",
    );
    assert_eq!(encoded_kind, "\"escrow_breach_refunded\"");
    let decoded_kind: FillKind = must_ok(
        serde_json::from_str(&encoded_kind),
        "deserialize FillKind::EscrowBreachRefunded",
    );
    assert_eq!(decoded_kind, FillKind::EscrowBreachRefunded);
    let copied = decoded_kind;
    let mut kind_set = HashSet::new();
    kind_set.insert(copied);
    kind_set.insert(FillKind::EscrowForfeited);
    kind_set.insert(FillKind::EscrowRefunded);
    assert_eq!(kind_set.len(), 3);

    // 2. Full Eval-VRF season exercising:
    //    - Turn 2 Step 2b BuyerDvpPayment deadline miss -> FillKind::EscrowBreachRefunded (from == to)
    //    - Turn 3 Phase 1 CancelExpiredEscrow -> FillKind::EscrowRefunded (from == to, no breach)
    //    - Turn 3 Step 2b SellerPerformanceBond deadline miss -> FillKind::EscrowForfeited (from != to)
    //    - Turn 4 Step 2b unbonded expiry -> FillKind::EscrowRefunded (from == to, no breach)
    let tid = TableId(650);
    let (mut spec, sealed_deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 4, 7),
        "setup",
    );
    spec.closing_hazard_bps = [10_000; 8];
    let eval_pk = spec.eval_vrf_pubkey;
    let mut eng = must_ok(TurnEngine::new(spec.clone()), "engine");

    let mut turns = Vec::new();
    let mut live_receipts = Vec::new();
    let mut live_digests = Vec::new();
    let mut buyer_cred_after_t1_lock = 0_i64;
    let mut seller_cred_before_t2_breach = 0_i64;
    let mut cancel_sender_cred_before_t3 = 0_i64;
    let mut perf_beneficiary_cred_before_t3 = 0_i64;
    let mut exp_sender_cred_before_t4 = 0_i64;
    let mut eng_t2_snapshot = None;

    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut actions = Vec::new();
        match t {
            1 => {
                // Escrow 1: BuyerDvpPayment by Seat 0 -> Seat 1, deadline Turn 2.
                // Escrow 2: SellerPerformanceBond by Seat 1 -> Seat 0, deadline Turn 3.
                // Escrow 3: Unbonded by Seat 2 -> Seat 3, cancelled in Turn 3 Phase 1.
                // Escrow 4: Unbonded by Seat 3 -> Seat 2, auto-expired in Turn 4 Step 2b.
                for action in [
                    MarketAction::CreateEscrow {
                        seat: 0,
                        escrow: EscrowProposal {
                            destination: 1,
                            asset: AssetId::Cred,
                            amount: 1_250,
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
                    },
                    MarketAction::CreateEscrow {
                        seat: 1,
                        escrow: EscrowProposal {
                            destination: 0,
                            asset: AssetId::Cred,
                            amount: 900,
                            finish_after_turn: None,
                            cancel_after_turn: 5,
                            condition_hash: None,
                            bond: Some(BondProposal {
                                good: AssetId::Raw(1),
                                required_units: 1_000,
                                deadline_turn: 3,
                                mode: BondSettlementMode::SellerPerformanceBond,
                            }),
                        },
                    },
                    MarketAction::CreateEscrow {
                        seat: 2,
                        escrow: EscrowProposal {
                            destination: 3,
                            asset: AssetId::Cred,
                            amount: 600,
                            finish_after_turn: Some(1),
                            cancel_after_turn: 3,
                            condition_hash: None,
                            bond: None,
                        },
                    },
                    MarketAction::CreateEscrow {
                        seat: 3,
                        escrow: EscrowProposal {
                            destination: 2,
                            asset: AssetId::Cred,
                            amount: 750,
                            finish_after_turn: Some(1),
                            cancel_after_turn: 4,
                            condition_hash: None,
                            bond: None,
                        },
                    },
                ] {
                    must_ok(eng.apply_market_action(action.clone()), "create escrow");
                    actions.push(action);
                }
                buyer_cred_after_t1_lock = eng.state().seats[0].cred;
                seller_cred_before_t2_breach = eng.state().seats[1].cred;
            }
            3 => {
                cancel_sender_cred_before_t3 = eng.state().seats[2].cred;
                perf_beneficiary_cred_before_t3 = eng.state().seats[0].cred;
                let action = MarketAction::CancelExpiredEscrow {
                    seat: 2,
                    escrow_id: 3,
                };
                must_ok(
                    eng.apply_market_action(action.clone()),
                    "cancel expired escrow",
                );
                actions.push(action);
            }
            4 => {
                exp_sender_cred_before_t4 = eng.state().seats[3].cred;
            }
            _ => {}
        }

        let (commitments, reveals) =
            commit_and_reveal_all(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        let (beacon, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
        let rec = must_ok(eng.sweep_turn_with_eval_vrf(&proof), "sweep");
        if t == 2 {
            eng_t2_snapshot = Some(eng.clone());
        }
        live_digests.push(rec.post_state_digest);
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
            drawn_shocks: rec.newly_drawn_shocks.clone(),
        });
        let closed = rec.season_closed;
        live_receipts.push(rec);
        if closed {
            break;
        }
    }

    // Turn 2: BuyerDvpPayment deadline miss refunds full amount to sender and penalizes obligor.
    let eng_t2 = must_ok(
        eng_t2_snapshot.ok_or("missing Turn 2 engine snapshot"),
        "Turn 2 engine snapshot",
    );
    let r2 = &live_receipts[1];
    assert_eq!(r2.escrows_cancelled_or_forfeited, vec![1]);
    let t2_breach_fills: Vec<&FillRecord> = r2
        .fills
        .iter()
        .filter(|f| f.kind == FillKind::EscrowBreachRefunded)
        .collect();
    assert_eq!(t2_breach_fills.len(), 1);
    assert_eq!(
        t2_breach_fills[0],
        &FillRecord {
            turn: 2,
            kind: FillKind::EscrowBreachRefunded,
            good: Some(AssetId::Cred),
            from: Some(0),
            to: 0,
            units: 1_250,
            net_units: 1_250,
            cred_amount: 1_250,
            unit_price_cred: None,
        }
    );
    assert!(
        !r2.fills.iter().any(|f| f.kind == FillKind::EscrowForfeited),
        "BuyerDvpPayment deadline miss must not log EscrowForfeited"
    );
    assert_eq!(
        eng_t2.state().seats[0].cred,
        buyer_cred_after_t1_lock + 1_250
    );
    assert_eq!(eng_t2.state().seats[0].bond_breaches, 0);
    assert_eq!(eng_t2.state().seats[0].honor_penalties, 0);
    assert_eq!(eng_t2.state().seats[1].cred, seller_cred_before_t2_breach);
    assert_eq!(eng_t2.state().seats[1].bond_breaches, 1);
    assert_eq!(eng_t2.state().seats[1].honor_penalties, 1_250);
    assert_eq!(eng_t2.state().escrows[&1].status, EscrowStatus::Cancelled);

    // Turn 3: Phase-1 CancelExpiredEscrow logs EscrowRefunded (no breach), and Step-2b
    // SellerPerformanceBond deadline miss logs EscrowForfeited (from != to).
    let r3 = &live_receipts[2];
    let t3_refunds: Vec<&FillRecord> = r3
        .fills
        .iter()
        .filter(|f| f.kind == FillKind::EscrowRefunded)
        .collect();
    assert_eq!(t3_refunds.len(), 1);
    assert_eq!(
        t3_refunds[0],
        &FillRecord {
            turn: 3,
            kind: FillKind::EscrowRefunded,
            good: Some(AssetId::Cred),
            from: Some(2),
            to: 2,
            units: 600,
            net_units: 600,
            cred_amount: 600,
            unit_price_cred: None,
        }
    );
    let t3_forfeits: Vec<&FillRecord> = r3
        .fills
        .iter()
        .filter(|f| f.kind == FillKind::EscrowForfeited)
        .collect();
    assert_eq!(t3_forfeits.len(), 1);
    assert_eq!(
        t3_forfeits[0],
        &FillRecord {
            turn: 3,
            kind: FillKind::EscrowForfeited,
            good: Some(AssetId::Cred),
            from: Some(1),
            to: 0,
            units: 900,
            net_units: 900,
            cred_amount: 900,
            unit_price_cred: None,
        }
    );
    assert_eq!(eng.state().seats[2].bond_breaches, 0);
    assert_eq!(eng.state().seats[3].bond_breaches, 0);
    assert_eq!(eng.state().seats[1].bond_breaches, 2);
    assert_eq!(
        eng.state().seats[0].cred,
        perf_beneficiary_cred_before_t3 + 900
    );
    assert_eq!(
        cancel_sender_cred_before_t3 + 600,
        // Seat 2 did not gain or lock any CRED after Turn 3 CancelExpiredEscrow.
        eng.state().seats[2].cred
    );

    // Turn 4: Step-2b unbonded expiry logs EscrowRefunded (from == to, no breach).
    let r4 = &live_receipts[3];
    let t4_refunds: Vec<&FillRecord> = r4
        .fills
        .iter()
        .filter(|f| f.kind == FillKind::EscrowRefunded)
        .collect();
    assert_eq!(t4_refunds.len(), 1);
    assert_eq!(
        t4_refunds[0],
        &FillRecord {
            turn: 4,
            kind: FillKind::EscrowRefunded,
            good: Some(AssetId::Cred),
            from: Some(3),
            to: 3,
            units: 750,
            net_units: 750,
            cred_amount: 750,
            unit_price_cred: None,
        }
    );
    assert_eq!(exp_sender_cred_before_t4 + 750, eng.state().seats[3].cred);

    // 3. Full replay parity & proof that SeasonReplayLog / ReplayRecord do NOT serialize FillKind.
    let rev0 = sealed_deal.seat_reveals[0].clone();
    must_ok(eng.submit_mandate_reveal(Seat(0), rev0.clone()), "mandate");
    let live_board = must_ok(eng.finalize_season(), "finalize");
    let mut mandate_reveals = vec![None; 4];
    mandate_reveals[0] = Some(rev0);
    let log = SeasonReplayLog {
        spec,
        mode: TableMode::Eval,
        seed: must_ok(ReplayLayoutSeed::try_clear(7), "clear seed"),
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk,
        sealed_mandate_deal: sealed_deal,
        mandate_commits: vec![None; 4],
        turns,
        mandate_reveals,
    };
    let log_json = must_ok(serde_json::to_string(&log), "serialize log");
    let record = must_ok(attach_state_predicates(&log), "attach predicates");
    let record_json = must_ok(serde_json::to_string(&record), "serialize record");
    for needle in [
        "escrow_breach_refunded",
        "escrow_forfeited",
        "escrow_refunded",
    ] {
        assert!(!log_json.contains(needle), "log should not carry {needle}");
        assert!(
            !record_json.contains(needle),
            "record should not carry {needle}"
        );
    }

    let parsed_log: SeasonReplayLog = must_ok(serde_json::from_str(&log_json), "parse log");
    let parsed_record = must_ok(
        ReplayRecord::from_json_slice(record_json.as_bytes()),
        "parse record",
    );
    let out_live = must_ok(replay_season(&parsed_log), "replay log");
    assert_eq!(out_live.state_digests, live_digests);
    assert_eq!(out_live.scoreboard, live_board);
    assert_eq!(out_live.receipts, live_receipts);
    let out_rec = must_ok(replay_season(&parsed_record.log), "replay record");
    assert_eq!(out_rec.receipts, live_receipts);

    // Keep eng_t2 in scope to verify Turn-2 balances without mutating persisted payloads via `.replace`.
    let _ = eng_t2;

    // 4. Committed pre-Card-1 fixture captured from base sha 94c73626f8cf1555a1f6f58da4f0cbe8fdcba80e
    //    (pre-version JSON with no `format_version`, carrying `"escrow_forfeited"` for the Turn-2
    //    DvP breach refund) deserializes cleanly and replays with identical digests and scoreboard.
    const PRE_CARD1_FIXTURE: &str = include_str!("fixtures/pre_card1_94c73626_replay_bundle.json");
    let fixture_val: serde_json::Value = must_ok(
        serde_json::from_str(PRE_CARD1_FIXTURE),
        "parse pre-Card-1 fixture JSON",
    );
    assert_eq!(
        fixture_val["base_sha"].as_str(),
        Some("94c73626f8cf1555a1f6f58da4f0cbe8fdcba80e")
    );
    assert!(fixture_val["log"].get("format_version").is_none());
    assert!(fixture_val["record"].get("format_version").is_none());
    assert!(fixture_val["outcome"].get("format_version").is_none());
    assert!(!PRE_CARD1_FIXTURE.contains("escrow_breach_refunded"));
    assert!(PRE_CARD1_FIXTURE.contains("\"escrow_forfeited\""));

    let old_log_bytes = must_ok(serde_json::to_vec(&fixture_val["log"]), "old log bytes");
    let old_record_bytes = must_ok(
        serde_json::to_vec(&fixture_val["record"]),
        "old record bytes",
    );
    let old_outcome_bytes = must_ok(
        serde_json::to_vec(&fixture_val["outcome"]),
        "old outcome bytes",
    );
    let old_fill_bytes = must_ok(
        serde_json::to_vec(&fixture_val["outcome"]["receipts"][1]["fills"][0]),
        "old fill bytes",
    );
    let old_r2_bytes = must_ok(
        serde_json::to_vec(&fixture_val["outcome"]["receipts"][1]),
        "old r2 bytes",
    );

    let old_fill: FillRecord = must_ok(serde_json::from_slice(&old_fill_bytes), "parse old fill");
    assert_eq!(old_fill.kind, FillKind::EscrowForfeited);
    let old_r2: SweepReceipt = must_ok(serde_json::from_slice(&old_r2_bytes), "parse old receipt");
    assert_eq!(old_r2.fills[0].kind, FillKind::EscrowForfeited);
    let old_out: ReplayOutcome = must_ok(
        ReplayOutcome::from_json_slice(&old_outcome_bytes),
        "parse old outcome",
    );
    assert_eq!(old_out.receipts[1].fills[0].kind, FillKind::EscrowForfeited);
    let old_log: SeasonReplayLog = must_ok(
        SeasonReplayLog::from_json_slice(&old_log_bytes),
        "parse old log",
    );
    let old_record: ReplayRecord = must_ok(
        ReplayRecord::from_json_slice(&old_record_bytes),
        "parse old record",
    );
    let replayed_old = must_ok(replay_season(&old_log), "replay pre-Card-1 log");
    // Turn 1 digest matches identically; from Turn 2 onward Card 1 emits `EscrowBreachRefunded`
    // in `EngineState::fill_log` where pre-Card-1 recorded `EscrowForfeited`, while all
    // balances and the terminal scoreboard stay identical.
    assert_eq!(replayed_old.state_digests[0], old_out.state_digests[0]);
    assert_eq!(replayed_old.scoreboard, old_out.scoreboard);
    assert_eq!(replayed_old.receipts.len(), old_out.receipts.len());
    assert_eq!(
        replayed_old.receipts[1].fills[0].kind,
        FillKind::EscrowBreachRefunded
    );
    assert_eq!(old_out.receipts[1].fills[0].kind, FillKind::EscrowForfeited);
    assert_eq!(
        replayed_old.receipts[1].fills[0].cred_amount,
        old_out.receipts[1].fills[0].cred_amount
    );
    let replayed_old_record = must_ok(replay_season(&old_record.log), "replay pre-Card-1 record");
    assert_eq!(replayed_old_record, replayed_old);

    // 5. All three DualEscrow forfeit sites emit BOTH EscrowForfeited (from != to) for the defaulter's
    //    forfeited bond AND EscrowRefunded (from == to) for the counterparty's returned bond, with
    //    unchanged seat balances (Card 1 review B2 + Card 2 Must item 6 & 7):
    //    - step2b_evaluate_bonds_and_expired_escrows buyer top-up default after unlinked delivery
    //    - step2_settle_contract_deliveries linked ContractDeliveryOrder buyer top-up default
    //      (distinguished by setting deadline_turn = 4 > turn 2 so step2b cannot fire on Turn 2)
    //    - step2b_evaluate_bonds_and_expired_escrows seller delivery deadline miss
    let (mut spec_dual, _) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(TableId(651), 4, 1),
        "spec dual",
    );
    spec_dual.seats[0].initial_cred = 700;
    let mut eng_dual = must_ok(TurnEngine::new(spec_dual), "eng dual");
    commit_and_reveal_all(
        &mut eng_dual,
        TableId(651),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(eng_dual.sweep_turn_seeded(1), "sweep t1 eng dual");

    let dual_buyer0_before = eng_dual.state().seats[0].cred;
    let dual_seller1_before = eng_dual.state().seats[1].cred;
    let dual_step2b = DualEscrowProposal {
        proposal_id: 91,
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
    let mut b_step2b = vec![SealedOrderBundle::default(); 4];
    b_step2b[0].dual_escrows.push(dual_step2b.clone());
    b_step2b[1].dual_escrows.push(dual_step2b);
    // Unlinked delivery satisfies required_units in delivery_ledger before Step 2b evaluates the
    // DualEscrow; Buyer locked 500 CRED and has only 200 (< 500 top-up), hitting step2b.
    b_step2b[1].contract_deliveries.push(ContractDeliveryOrder {
        counterparty: 0,
        good: AssetId::Raw(1),
        gross_amount: 1_100,
        domain_id: None,
        dual_escrow_id: None,
    });
    commit_and_reveal_all(&mut eng_dual, TableId(651), 2, b_step2b);
    let r_step2b = must_ok(eng_dual.sweep_turn_seeded(2), "sweep t2 eng dual");
    assert!(
        r_step2b
            .fills
            .iter()
            .any(|f| { f.kind == FillKind::ContractDelivery && f.good == Some(AssetId::Raw(1)) })
    );
    assert!(r_step2b.fills.iter().any(|f| {
        f.kind == FillKind::EscrowForfeited
            && f.good == Some(AssetId::Cred)
            && f.from == Some(0)
            && f.to == 1
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert!(r_step2b.fills.iter().any(|f| {
        f.kind == FillKind::EscrowRefunded
            && f.good == Some(AssetId::Cred)
            && f.from == Some(1)
            && f.to == 1
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert_eq!(eng_dual.state().seats[0].cred, dual_buyer0_before - 500);
    assert_eq!(eng_dual.state().seats[1].cred, dual_seller1_before + 500);
    assert_eq!(eng_dual.state().seats[0].bond_breaches, 1);

    // Distinct proof for `step2_settle_contract_deliveries` vs `step2b_evaluate_bonds_and_expired_escrows`:
    // `dual_buyer_def` has `deadline_turn: 4` while swept on Turn 2 (`turn < deadline_turn`), so
    // `step2b` cannot touch it on Turn 2; cancellation + empty `delivery_ledger` prove Step 2 ran.
    let (mut spec_both, _) = must_ok(
        TableSpec::standard_chiral_ring_with_mandates(TableId(652), 4, 1),
        "spec both",
    );
    spec_both.seats[0].initial_cred = 700;
    let mut eng_both = must_ok(TurnEngine::new(spec_both), "eng both");
    commit_and_reveal_all(
        &mut eng_both,
        TableId(652),
        1,
        vec![SealedOrderBundle::default(); 4],
    );
    must_ok(eng_both.sweep_turn_seeded(1), "sweep t1 eng both");

    let both_b0_before = eng_both.state().seats[0].cred;
    let both_s1_before = eng_both.state().seats[1].cred;
    let both_b2_before = eng_both.state().seats[2].cred;
    let both_s3_before = eng_both.state().seats[3].cred;
    let eid_buyer_def = eng_both.state().next_escrow_id;
    let dual_buyer_def = DualEscrowProposal {
        proposal_id: 92,
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
    let dual_seller_def = DualEscrowProposal {
        proposal_id: 93,
        buyer: 2,
        seller: 3,
        good: AssetId::Raw(3),
        required_units: 1_000,
        payment_cred: 1_000,
        buyer_bond_cred: 400,
        seller_bond_cred: 600,
        deadline_turn: 2,
        domain_id: None,
    };
    let mut b_both = vec![SealedOrderBundle::default(); 4];
    b_both[0].dual_escrows.push(dual_buyer_def.clone());
    b_both[1].dual_escrows.push(dual_buyer_def);
    b_both[1].contract_deliveries.push(ContractDeliveryOrder {
        counterparty: 0,
        good: AssetId::Raw(1),
        gross_amount: 1_100,
        domain_id: None,
        dual_escrow_id: Some(eid_buyer_def),
    });
    b_both[2].dual_escrows.push(dual_seller_def.clone());
    b_both[3].dual_escrows.push(dual_seller_def);
    commit_and_reveal_all(&mut eng_both, TableId(652), 2, b_both);
    let r_both = must_ok(eng_both.sweep_turn_seeded(2), "sweep t2 eng both");
    // Step 2 site check: `eid_buyer_def` cancelled on Turn 2 despite `deadline_turn == 4`, with 0 Raw(1) delivered.
    assert!(
        r_both
            .escrows_cancelled_or_forfeited
            .contains(&eid_buyer_def)
    );
    assert_eq!(
        eng_both.state().dual_escrows[&eid_buyer_def].status,
        EscrowStatus::Cancelled
    );
    assert!(eng_both.state().delivery_ledger.is_empty());
    assert_eq!(eng_both.state().seats[0].raw[1], 0);
    assert!(
        !r_both
            .fills
            .iter()
            .any(|f| f.kind == FillKind::ContractDelivery)
    );
    // Buyer-default fills at Step 2 (`step2_settle_contract_deliveries`):
    assert!(r_both.fills.iter().any(|f| {
        f.kind == FillKind::EscrowForfeited
            && f.good == Some(AssetId::Cred)
            && f.from == Some(0)
            && f.to == 1
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    assert!(r_both.fills.iter().any(|f| {
        f.kind == FillKind::EscrowRefunded
            && f.good == Some(AssetId::Cred)
            && f.from == Some(1)
            && f.to == 1
            && f.units == 500
            && f.net_units == 500
            && f.cred_amount == 500
    }));
    // Seller-default fills at Step 2b (`step2b_evaluate_bonds_and_expired_escrows`):
    assert!(r_both.fills.iter().any(|f| {
        f.kind == FillKind::EscrowForfeited
            && f.good == Some(AssetId::Cred)
            && f.from == Some(3)
            && f.to == 2
            && f.units == 600
            && f.net_units == 600
            && f.cred_amount == 600
    }));
    assert!(r_both.fills.iter().any(|f| {
        f.kind == FillKind::EscrowRefunded
            && f.good == Some(AssetId::Cred)
            && f.from == Some(2)
            && f.to == 2
            && f.units == 400
            && f.net_units == 400
            && f.cred_amount == 400
    }));
    // Balances remain identical before and after adding the counterparty EscrowRefunded fill.
    assert_eq!(eng_both.state().seats[0].cred, both_b0_before - 500);
    assert_eq!(eng_both.state().seats[1].cred, both_s1_before + 500);
    assert_eq!(eng_both.state().seats[2].cred, both_b2_before + 600);
    assert_eq!(eng_both.state().seats[3].cred, both_s3_before - 600);
    assert_eq!(eng_both.state().seats[0].bond_breaches, 1);
    assert_eq!(eng_both.state().seats[3].bond_breaches, 1);
}
