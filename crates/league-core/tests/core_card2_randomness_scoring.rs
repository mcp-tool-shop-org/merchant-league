//! Integration tests and seeded-season measurements for Core Card 2:
//! - Part 1: Game-mode randomness (#31) from pre-named drand beacons + per-use HKDF streams
//! - Part 2: Eval-mode randomness (#31 comment) from sealed OS CSPRNG VRF seed + RFC 9381 proofs
//! - Part 4: Mandate point conversion (`points * alpha * table_median_net_worth`, `N = 4, 5, 6`)
//! - Part 5: Seeded-season mandate variance share (`Var(B)/Var(S)`) & leader flip rate (`>= 200` seasons/N)
//! - Part 6: Hard cap at Turn 32 + `35%` closing hazard table (`E[T]`, `P(T == 32)`)
//! - Part 7: Final marks via 3-turn TWAP (`>= 2` liquid turns) or executable resting bid-book depth

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use league_core::{
    AssetId, AtomicSwapProposal, CLOSING_ROLL_START_TURN, CLOSING_ROLL_WINDOW_LEN, CharterBidOrder,
    CharterRole, CheckProposal, CommitmentHash, ConverterRunOrder, CouncilPlatform,
    CouncilVoteBallot, CreditTier, DEFAULT_CLOSING_HAZARD_BPS, DEFAULT_MANDATE_ALPHA_BPS,
    DEFAULT_SHOCK_DRAW_TURNS, DEFAULT_TWAP_MIN_TURN_VOLUME, DEFAULT_TWAP_WINDOW_TURNS,
    DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_SIGNATURE_BYTES, DrandSchedule, EngineError,
    EscrowFinishUnlock, EscrowProposal, FinalScoreboard, HARD_TURN_CAP, LoggedTurn, MandateKind,
    MarketAction, OfferProposal, OfferSide, OrderReveal, PreimageSecret, ProjectDeliveryOrder,
    ReplayBeaconMode, RevealSalt, RngDomain, ScheduledShockSpec, SealedEvalSeed, SealedOrderBundle,
    SeasonReplayLog, Seat, ShockKind, TableId, TableMode, TableSpec, Turn, TurnEngine, TurnPhase,
    UNIT_SCALE, Units, cap_reach_probability, close_roll_bps_from_beacon,
    compute_mandate_commitment, compute_order_commitment, compute_preimage_condition,
    convert_mandate_points_to_equity_with_median, deal_graded_mandates_with_entropy,
    derive_drand_beacon, derive_turn_beacon, draw_shocks_for_turn, expand_turn_stream,
    expected_season_length_turns, replay_season, table_median_net_worth,
};

trait MustGet<T> {
    fn must(self, ctx: &str) -> T;
}

impl<T, E: std::fmt::Display> MustGet<T> for Result<T, E> {
    fn must(self, ctx: &str) -> T {
        match self {
            Ok(v) => v,
            Err(err) => panic!("{ctx}: {err}"),
        }
    }
}

impl<T> MustGet<T> for Option<T> {
    fn must(self, ctx: &str) -> T {
        match self {
            Some(v) => v,
            None => panic!("{ctx}: unexpected None"),
        }
    }
}

fn commit_and_reveal_bundles(
    engine: &mut TurnEngine,
    tid: TableId,
    t: u8,
    bundles: Vec<SealedOrderBundle>,
) -> (BTreeMap<u8, CommitmentHash>, BTreeMap<u8, OrderReveal>) {
    engine.advance_to_commit().must("advance_to_commit");
    let mut commits = BTreeMap::new();
    let mut reveals = BTreeMap::new();
    for (idx, bundle) in bundles.into_iter().enumerate() {
        let s = idx as u8;
        let salt_byte = s.wrapping_mul(17).wrapping_add(t).max(1);
        let rev = OrderReveal {
            bundle,
            salt: RevealSalt([salt_byte; 32]),
        };
        let ch = compute_order_commitment(tid, Turn(t), Seat(s), &rev).must("order commit");
        engine
            .submit_order_commitment(Seat(s), ch)
            .must("submit_order_commitment");
        commits.insert(s, ch);
        reveals.insert(s, rev);
    }
    engine.advance_to_reveal().must("advance_to_reveal");
    for (&s, rev) in &reveals {
        engine
            .submit_order_reveal(Seat(s), rev.clone())
            .must("submit_order_reveal");
    }
    (commits, reveals)
}

fn step_empty_turn(engine: &mut TurnEngine, tid: TableId, t: u8, seed: u64) -> bool {
    let n = engine.state().seats.len();
    let _ = commit_and_reveal_bundles(engine, tid, t, vec![SealedOrderBundle::default(); n]);
    let rec = engine
        .sweep_turn(derive_turn_beacon(seed, tid, Turn(t)))
        .must("sweep");
    rec.season_closed
}

fn make_drand_sig(stream_tag: u8, turn: u8) -> Vec<u8> {
    let mut sig = vec![0u8; DRAND_QUICKNET_SIGNATURE_BYTES];
    for (idx, slot) in sig.iter_mut().enumerate() {
        let digest: [u8; 32] = Sha256::digest([stream_tag, turn, idx as u8]).into();
        *slot = digest[0];
    }
    if sig.iter().all(|&b| b == 0) {
        sig[0] = 1;
    }
    sig
}

fn run_game_season_from_sigs<F>(
    spec: &TableSpec,
    schedule: DrandSchedule,
    mut sig_for_turn: F,
) -> (SeasonReplayLog, FinalScoreboard, Vec<ScheduledShockSpec>)
where
    F: FnMut(u8) -> Vec<u8>,
{
    let tid = spec.table_id;
    let n = usize::from(spec.seat_count);
    let (_, deal) =
        TableSpec::standard_game_chiral_ring_with_mandates(tid, spec.seat_count, 0, schedule)
            .must("fresh deal");
    let mut spec_run = spec.clone();
    spec_run.dealt_mandate_commits = (0..n)
        .map(|s| {
            compute_mandate_commitment(tid, Seat(s as u8), &deal.seat_reveals[s])
                .must("mandate commit")
        })
        .collect();

    let mut eng = TurnEngine::new(spec_run.clone()).must("game engine");
    let mut turns = Vec::new();
    let mut all_drawn = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) =
            commit_and_reveal_bundles(&mut eng, tid, t, vec![SealedOrderBundle::default(); n]);
        let round = schedule
            .expected_round_for_turn(Turn(t))
            .must("scheduled round");
        let sig = sig_for_turn(t);
        let beacon = derive_drand_beacon(&sig).must("drand beacon");
        let rec = eng
            .sweep_turn_with_drand(round, &sig)
            .must("sweep_turn_with_drand");
        all_drawn.extend(rec.newly_drawn_shocks.iter().copied());
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
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
    let board = eng.finalize_season().must("finalize game");
    let log = SeasonReplayLog {
        spec: spec_run,
        mode: TableMode::Game,
        seed: 0,
        beacon_mode: ReplayBeaconMode::ExternalDrand,
        eval_vrf_pubkey: None,
        sealed_mandate_deal: deal,
        mandate_commits: vec![None; n],
        turns,
        mandate_reveals: vec![None; n],
    };
    (log, board, all_drawn)
}

#[test]
fn game_mode_shocks_and_close_follow_logged_drand_beacons_and_refuse_seeded_shocks() {
    let schedule = DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 600,
        turn_period_secs: 60,
    };
    let tid = TableId(601);
    let (spec, _deal) =
        TableSpec::standard_game_chiral_ring_with_mandates(tid, 4, 7, schedule).must("game spec");
    assert!(
        spec.shocks.is_empty(),
        "Game-mode TableSpec must not pre-populate shocks from public layout seed (got {} seeded shocks)",
        spec.shocks.len()
    );

    // Per-use RFC 5869 HKDF streams for the exact same beacon & turn must be pairwise distinct.
    let sample_beacon = derive_drand_beacon(&make_drand_sig(0x11, 24)).must("beacon");
    let streams = [
        expand_turn_stream(sample_beacon, tid, Turn(24), RngDomain::Shocks, b""),
        expand_turn_stream(sample_beacon, tid, Turn(24), RngDomain::CloseRoll, b""),
        expand_turn_stream(
            sample_beacon,
            tid,
            Turn(24),
            RngDomain::Tiebreak,
            b"trade_minister",
        ),
        expand_turn_stream(
            sample_beacon,
            tid,
            Turn(24),
            RngDomain::BatchTie,
            &[1, 0, 0, 0],
        ),
    ];
    let unique_streams: BTreeSet<[u8; 32]> = streams.into_iter().collect();
    assert_eq!(
        unique_streams.len(),
        4,
        "all 4 HKDF domains must produce distinct 32-byte streams for the same beacon"
    );
    assert_ne!(
        expand_turn_stream(sample_beacon, tid, Turn(24), RngDomain::CloseRoll, b""),
        expand_turn_stream(sample_beacon, tid, Turn(25), RngDomain::CloseRoll, b""),
        "HKDF stream must separate turns for identical beacon bytes"
    );

    // Find two distinct logged drand beacon sequences that produce BOTH different shock schedules
    // AND different closing turns on the exact same Game TableSpec.
    let (log_a, board_a, shocks_a) =
        run_game_season_from_sigs(&spec, schedule, |t| make_drand_sig(1, t));
    let mut alt_found = None;
    for tag in 2..=64_u8 {
        let (cand_log, cand_board, cand_shocks) =
            run_game_season_from_sigs(&spec, schedule, |t| make_drand_sig(tag, t));
        if cand_shocks != shocks_a && cand_board.closed_turn != board_a.closed_turn {
            alt_found = Some((cand_log, cand_board, cand_shocks));
            break;
        }
    }
    let (log_b, board_b, shocks_b) =
        alt_found.must("distinct drand beacon logs drive distinct shocks and closing turns");
    assert_eq!(shocks_a.len(), DEFAULT_SHOCK_DRAW_TURNS.len());
    assert_eq!(shocks_b.len(), DEFAULT_SHOCK_DRAW_TURNS.len());
    assert_ne!(shocks_a, shocks_b);
    assert_ne!(board_a.closed_turn, board_b.closed_turn);

    // Verify every turn's drawn_shocks and closing turn match the logged drand beacons directly.
    for lt in &log_a.turns {
        let expected_shocks = draw_shocks_for_turn(tid, 4, Turn(lt.turn), lt.turn_beacon);
        assert_eq!(lt.drawn_shocks, expected_shocks);
        if (CLOSING_ROLL_START_TURN..HARD_TURN_CAP).contains(&lt.turn) {
            let roll = close_roll_bps_from_beacon(lt.turn_beacon, tid, Turn(lt.turn));
            let should_close = roll
                < log_a.spec.closing_hazard_bps[usize::from(lt.turn - CLOSING_ROLL_START_TURN)];
            assert_eq!(lt.turn == board_a.closed_turn, should_close);
        }
    }

    let replayed_a = replay_season(&log_a).must("valid Game drand replay");
    assert_eq!(replayed_a.scoreboard, board_a);
    let replayed_b = replay_season(&log_b).must("valid Game drand replay b");
    assert_eq!(replayed_b.scoreboard, board_b);

    // Game TableSpec & replay_season must refuse seeded shocks.
    let mut seeded_shock_spec = log_a.spec.clone();
    seeded_shock_spec.shocks.push(ScheduledShockSpec {
        hint_turn: 3,
        forecast_turn: 4,
        realize_turn: 5,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_500,
    });
    assert!(matches!(
        seeded_shock_spec.validate(),
        Err(EngineError::InvalidConfig(_))
    ));
    let mut seeded_shock_log = log_a.clone();
    seeded_shock_log.spec = seeded_shock_spec;
    assert!(matches!(
        replay_season(&seeded_shock_log),
        Err(EngineError::InvalidConfig(_))
    ));

    // Game replay refuses tampered drawn_shocks that don't match the logged drand beacon.
    let mut bad_shock_log = log_a.clone();
    bad_shock_log.turns[1].drawn_shocks[0].target_seat =
        (bad_shock_log.turns[1].drawn_shocks[0].target_seat + 1) % 4;
    assert!(matches!(
        replay_season(&bad_shock_log),
        Err(EngineError::InvalidOrder(_))
    ));

    // Game replay refuses mismatched closing turn (premature stop or extra post-close turn).
    let mut premature_close_log = log_a.clone();
    premature_close_log.turns.pop();
    assert!(matches!(
        replay_season(&premature_close_log),
        Err(EngineError::SeasonEnded(_))
    ));

    let mut post_close_log = log_a;
    let extra_t = post_close_log.turns.len() as u8 + 1;
    let extra_sig = make_drand_sig(99, extra_t);
    post_close_log.turns.push(LoggedTurn {
        turn: extra_t,
        market_actions: Vec::new(),
        commitments: BTreeMap::new(),
        reveals: BTreeMap::new(),
        turn_beacon: derive_drand_beacon(&extra_sig).must("sig"),
        drand_round: schedule.expected_round_for_turn(Turn(extra_t)).ok(),
        drand_signature: Some(extra_sig),
        eval_vrf_proof: None,
        drawn_shocks: Vec::new(),
    });
    assert!(matches!(
        replay_season(&post_close_log),
        Err(EngineError::SeasonEnded(_))
    ));
}

fn run_eval_vrf_season(
    spec: TableSpec,
    deal: league_core::SealedMandateDeal,
    eval_seed: &SealedEvalSeed,
    layout_seed: u64,
) -> (
    SeasonReplayLog,
    FinalScoreboard,
    Vec<[u8; 32]>,
    Vec<ScheduledShockSpec>,
) {
    let tid = spec.table_id;
    let n = usize::from(spec.seat_count);
    let eval_pk = spec.eval_vrf_pubkey;
    let mut eng = TurnEngine::new(spec.clone()).must("eval engine");
    let mut turns = Vec::new();
    let mut digests = Vec::new();
    let mut drawn = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (commitments, reveals) =
            commit_and_reveal_bundles(&mut eng, tid, t, vec![SealedOrderBundle::default(); n]);
        let (beacon, proof) = eval_seed.prove_turn(tid, Turn(t)).must("prove_turn");
        let rec = eng
            .sweep_turn_with_eval_vrf(&proof)
            .must("sweep_turn_with_eval_vrf");
        digests.push(rec.post_state_digest);
        drawn.extend(rec.newly_drawn_shocks.iter().copied());
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
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
    let board = eng.finalize_season().must("finalize eval");
    let log = SeasonReplayLog {
        spec,
        mode: TableMode::Eval,
        seed: layout_seed,
        beacon_mode: ReplayBeaconMode::EvalVrf,
        eval_vrf_pubkey: eval_pk,
        sealed_mandate_deal: deal,
        mandate_commits: vec![None; n],
        turns,
        mandate_reveals: vec![None; n],
    };
    (log, board, digests, drawn)
}

#[test]
fn eval_mode_tables_with_same_layout_seed_get_independent_vrf_shocks_and_refuse_tampering() {
    let tid = TableId(602);
    let layout_seed = 42_u64;
    let (spec_a, deal_a, eval_seed_a) =
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 5, layout_seed).must("eval spec a");
    let (spec_b, deal_b, eval_seed_b) =
        TableSpec::standard_eval_chiral_ring_with_setup(tid, 5, layout_seed).must("eval spec b");

    assert!(
        spec_a.shocks.is_empty() && spec_b.shocks.is_empty(),
        "Eval-mode public TableSpec must not expose precomputed layout-seed shocks (got {} and {})",
        spec_a.shocks.len(),
        spec_b.shocks.len()
    );
    assert_eq!(spec_a.seats, spec_b.seats);
    assert_ne!(spec_a.eval_vrf_pubkey, spec_b.eval_vrf_pubkey);

    // Debug output on SealedEvalSeed must never leak secret bytes.
    let dbg_seed = format!("{eval_seed_a:?}");
    assert_eq!(dbg_seed, "SealedEvalSeed(<hidden>)");

    let (log_a, board_a, digests_a, shocks_a) =
        run_eval_vrf_season(spec_a, deal_a, &eval_seed_a, layout_seed);
    let (_log_b, _board_b, _digests_b, shocks_b) =
        run_eval_vrf_season(spec_b, deal_b, &eval_seed_b, layout_seed);

    // Two tables with the exact same public layout seed get different shock schedules.
    assert_eq!(shocks_a.len(), DEFAULT_SHOCK_DRAW_TURNS.len());
    assert_ne!(
        shocks_a, shocks_b,
        "two Eval tables with identical public layout seed must get independent OS-seeded shock schedules"
    );

    // Public layout seed alone does NOT reproduce either table's shocks or beacons.
    let layout_derived_shocks: Vec<ScheduledShockSpec> = DEFAULT_SHOCK_DRAW_TURNS
        .into_iter()
        .flat_map(|t| {
            let b = derive_turn_beacon(layout_seed, tid, Turn(t));
            draw_shocks_for_turn(tid, 5, Turn(t), b)
        })
        .collect();
    assert_ne!(shocks_a, layout_derived_shocks);
    assert_ne!(shocks_b, layout_derived_shocks);

    // Eval replay refuses SeedDerived mode and refuses layout-seed-derived VRF keys.
    let mut seed_mode_log = log_a.clone();
    seed_mode_log.beacon_mode = ReplayBeaconMode::SeedDerived;
    assert!(matches!(
        replay_season(&seed_mode_log),
        Err(EngineError::InvalidConfig(_))
    ));

    let hashed_layout: [u8; 32] = Sha256::digest(layout_seed.to_le_bytes()).into();
    let leaked_eval_seed = SealedEvalSeed::try_from_bytes(hashed_layout).must("non-zero");
    let salts5: Vec<RevealSalt> = (1..=5_u8).map(|i| RevealSalt([i * 11; 32])).collect();
    let (leaked_spec, leaked_deal, leaked_eval_seed) =
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            tid,
            5,
            layout_seed,
            [0xAB; 32],
            &salts5,
            leaked_eval_seed,
        )
        .must("explicit setup");
    let (leaked_log, _, _, _) =
        run_eval_vrf_season(leaked_spec, leaked_deal, &leaked_eval_seed, layout_seed);
    assert!(matches!(
        replay_season(&leaked_log),
        Err(EngineError::InvalidConfig(_))
    ));

    // Tampered VRF proof, tampered beacon draw, tampered drawn_shocks, or wrong pubkey refused on replay.
    let mut bad_proof = log_a.clone();
    bad_proof.turns[0].eval_vrf_proof.as_mut().must("proof")[12] ^= 0x01;
    assert!(matches!(
        replay_season(&bad_proof),
        Err(EngineError::InvalidOrder(_))
    ));

    let mut bad_beacon = log_a.clone();
    bad_beacon.turns[1].turn_beacon[0] ^= 0x80;
    assert!(matches!(
        replay_season(&bad_beacon),
        Err(EngineError::InvalidOrder(_))
    ));

    let mut bad_drawn = log_a.clone();
    bad_drawn.turns[1].drawn_shocks.clear();
    assert!(matches!(
        replay_season(&bad_drawn),
        Err(EngineError::InvalidOrder(_))
    ));

    let mut swapped_turn_proof = log_a.clone();
    swapped_turn_proof.turns[1].eval_vrf_proof = swapped_turn_proof.turns[0].eval_vrf_proof.clone();
    assert!(matches!(
        replay_season(&swapped_turn_proof),
        Err(EngineError::InvalidOrder(_))
    ));

    // Eval replay digest is bit-for-bit stable for a given SealedEvalSeed.
    let out_a1 = replay_season(&log_a).must("valid EvalVrf replay 1");
    let out_a2 = replay_season(&log_a).must("valid EvalVrf replay 2");
    assert_eq!(out_a1.state_digests, digests_a);
    assert_eq!(out_a1, out_a2);
    assert_eq!(out_a1.scoreboard, board_a);

    let fixed_seed = SealedEvalSeed::try_from_bytes([0x3C; 32]).must("fixed seed");
    let (exp_spec_1, exp_deal_1, fixed_1) =
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            TableId(603),
            5,
            layout_seed,
            [0x91; 32],
            &salts5,
            fixed_seed.clone(),
        )
        .must("exp 1");
    let (exp_spec_2, exp_deal_2, fixed_2) =
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            TableId(603),
            5,
            layout_seed,
            [0x91; 32],
            &salts5,
            fixed_seed,
        )
        .must("exp 2");
    let (_, _, dig_1, sh_1) = run_eval_vrf_season(exp_spec_1, exp_deal_1, &fixed_1, layout_seed);
    let (_, _, dig_2, sh_2) = run_eval_vrf_season(exp_spec_2, exp_deal_2, &fixed_2, layout_seed);
    assert_eq!(sh_1, sh_2);
    assert_eq!(dig_1, dig_2);
}

#[test]
fn mandate_points_scale_with_table_median_net_worth_across_4_5_6_seats() {
    // Direct unit checks of even/odd median and negative-median zero floor.
    assert_eq!(
        table_median_net_worth(&[10_000, 20_001, 30_000, 90_000]),
        25_001
    );
    assert_eq!(
        table_median_net_worth(&[10_000, 20_000, 35_000, 40_000, 90_000]),
        35_000
    );
    assert_eq!(
        table_median_net_worth(&[10_000, 20_000, 30_000, 40_000, 50_000, 60_000]),
        35_000
    );
    assert_eq!(
        convert_mandate_points_to_equity_with_median(5_000, -10_000, DEFAULT_MANDATE_ALPHA_BPS),
        0
    );

    // End-to-end season close mandate conversion across N = 4, 5, 6.
    for n in [4_u8, 5_u8, 6_u8] {
        let tid = TableId(604 + u64::from(n));
        let salts: Vec<RevealSalt> = (0..n).map(|s| RevealSalt([0x11 + s; 32])).collect();
        let (mut spec, deal, clean_seat) = (1_u8..=64)
            .find_map(|ds| {
                let (sp, dl) =
                    TableSpec::standard_chiral_ring_with_explicit_deal(tid, n, 9, [ds; 32], &salts)
                        .must("spec");
                let cs = (0..usize::from(n)).find(|&i| {
                    matches!(
                        dl.seat_reveals[i].mandate.kind,
                        MandateKind::MaintainCleanCredit { .. }
                    )
                })?;
                Some((sp, dl, cs))
            })
            .must("some deal seed deals MaintainCleanCredit");
        spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
        for (i, s) in spec.seats.iter_mut().enumerate() {
            // Distinct positive terminal CRED balances: 300k, 330k, 360k, ...
            s.initial_cred = (300 + (i as i64) * 30) * UNIT_SCALE;
            s.regent = None;
        }
        let pts_milli = deal.seat_reveals[clean_seat].mandate.bonus_equity;

        let mut eng = TurnEngine::new(spec.clone()).must("engine");
        for t in 1..=HARD_TURN_CAP {
            if eng.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            if step_empty_turn(&mut eng, tid, t, 9) {
                break;
            }
        }
        eng.submit_mandate_reveal(
            Seat(clean_seat as u8),
            deal.seat_reveals[clean_seat].clone(),
        )
        .must("reveal");
        let board = eng.finalize_season().must("finalize");

        let nws: Vec<Units> = board
            .standings
            .iter()
            .map(|e| e.project_equity + e.net_asset_equity)
            .collect();
        let median_nw = table_median_net_worth(&nws);
        assert!(median_nw > 0);
        let expected_bonus = ((i128::from(pts_milli)
            * i128::from(DEFAULT_MANDATE_ALPHA_BPS)
            * i128::from(median_nw)
            + 5_000_000)
            / 10_000_000) as Units;
        let actual_bonus = board
            .standings
            .iter()
            .find(|e| e.seat == clean_seat as u8)
            .must("entry")
            .mandate_bonus;
        assert_eq!(
            actual_bonus, expected_bonus,
            "N={n}: mandate bonus must equal points * 0.025 * table_median_net_worth ({expected_bonus}), got {actual_bonus}"
        );
    }

    // Verify a seat's own spending does NOT change per-point value except through the table median:
    // Seat 3 is the richest seat (initial_cred = 600_000 > median seats 1 & 2 at 340_000 & 380_000).
    // When Seat 3 holds MaintainCleanCredit and spends 40_000 CRED on TownImport (staying above 380_000),
    // Seat 3's own net worth drops, yet its awarded mandate_bonus stays 100% identical!
    let tid_inv = TableId(619);
    let salts4 = [
        RevealSalt([0x21; 32]),
        RevealSalt([0x22; 32]),
        RevealSalt([0x23; 32]),
        RevealSalt([0x24; 32]),
    ];
    let (mut base_spec, _) =
        TableSpec::standard_chiral_ring_with_explicit_deal(tid_inv, 4, 12, [0x61; 32], &salts4)
            .must("base_spec");
    base_spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
    let custom_deck: Vec<_> = (0..8_u16)
        .map(|cid| league_core::GradedMandateCard {
            card_id: cid + 1,
            kind: MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            difficulty_completion_bps: 4_000,
            bonus_equity: 6_000,
        })
        .collect();
    let (commits4, deal4) =
        deal_graded_mandates_with_entropy(tid_inv, 4, [0x61; 32], &salts4, &custom_deck)
            .must("deal4");
    base_spec.mandate_deck = custom_deck;
    base_spec.dealt_mandate_commits = commits4;
    let init_creds = [300_000, 340_000, 380_000, 600_000];
    for (i, sc) in base_spec.seats.iter_mut().enumerate() {
        sc.initial_cred = init_creds[i];
        sc.regent = None;
    }

    let run_seat3_spend =
        |seat3_town_import_units: Units, shift_median_cred: Units| -> (Units, Units) {
            let mut sp = base_spec.clone();
            sp.seats[1].initial_cred += shift_median_cred;
            let mut eng = TurnEngine::new(sp).must("eng");
            for t in 1..=HARD_TURN_CAP {
                if eng.state().phase != TurnPhase::TalkAndMarket {
                    break;
                }
                if t == 1 && seat3_town_import_units > 0 {
                    eng.apply_market_action(MarketAction::TownImport {
                        seat: 3,
                        good: AssetId::Raw(1),
                        amount: seat3_town_import_units,
                    })
                    .must("seat 3 spend");
                }
                if step_empty_turn(&mut eng, tid_inv, t, 12) {
                    break;
                }
            }
            eng.submit_mandate_reveal(Seat(3), deal4.seat_reveals[3].clone())
                .must("reveal seat 3");
            let board = eng.finalize_season().must("fin");
            let e3 = board.standings.iter().find(|e| e.seat == 3).must("seat 3");
            (e3.net_asset_equity, e3.mandate_bonus)
        };

    let (nw_no_spend, bonus_no_spend) = run_seat3_spend(0, 0);
    let (nw_with_spend, bonus_with_spend) = run_seat3_spend(8_000, 0);
    assert!(
        nw_with_spend < nw_no_spend,
        "Seat 3 spending CRED on TownImport reduces Seat 3's own net worth"
    );
    assert_eq!(
        bonus_with_spend, bonus_no_spend,
        "non-median seat spending must not change its mandate bonus ({bonus_with_spend} vs {bonus_no_spend})"
    );

    let (_nw_med_shifted, bonus_med_shifted) = run_seat3_spend(0, 40_000);
    assert!(
        bonus_med_shifted > bonus_no_spend,
        "shifting a median seat's net worth upward increases per-point value through the median"
    );
}

fn deterministic_32_bytes(tag: &[u8], n: u8, season_idx: u32, extra: u8) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(tag);
    h.update([n, extra]);
    h.update(season_idx.to_le_bytes());
    let mut out: [u8; 32] = h.finalize().into();
    if out == [0u8; 32] {
        out[0] = 1;
    }
    out
}

fn run_scripted_measurement_season(
    seat_count: u8,
    season_idx: u32,
    complete_works_ring: bool,
) -> FinalScoreboard {
    let tid = TableId(10_000 + u64::from(seat_count) * 1_000 + u64::from(season_idx));
    let layout_seed = u64::from(season_idx).wrapping_mul(131).wrapping_add(17);
    let deal_seed = deterministic_32_bytes(b"card2-part5/deal", seat_count, season_idx, 0);
    let salts: Vec<RevealSalt> = (0..seat_count)
        .map(|s| {
            RevealSalt(deterministic_32_bytes(
                b"card2-part5/salt",
                seat_count,
                season_idx,
                s + 1,
            ))
        })
        .collect();
    let eval_entropy = deterministic_32_bytes(b"card2-part5/eval", seat_count, season_idx, 0xFE);
    let (spec, deal) = TableSpec::standard_chiral_ring_with_explicit_deal(
        tid,
        seat_count,
        layout_seed,
        deal_seed,
        &salts,
    )
    .must("explicit setup");
    let n = usize::from(seat_count);
    let mut engine = TurnEngine::new(spec.clone()).must("engine");
    for s in 0..seat_count {
        engine
            .submit_mandate_commitment(Seat(s), spec.dealt_mandate_commits[usize::from(s)])
            .must("mandate commit");
    }

    let mut tracked_escrow: Option<(u64, u8, PreimageSecret)> = None;
    let mut tracked_check: Option<(u64, u8)> = None;

    for t in 1..=HARD_TURN_CAP {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut bundles = vec![SealedOrderBundle::default(); n];
        let rot = if complete_works_ring {
            (season_idx as usize) % n
        } else {
            0
        };
        let p0 = rot as u8;
        let p1 = ((rot + 1) % n) as u8;
        let p2 = ((rot + 2) % n) as u8;

        if t >= 2 {
            let (buyer, seller) = if t.is_multiple_of(2) {
                (p0, p1)
            } else {
                (p1, p0)
            };
            if engine.state().seats[usize::from(buyer)].cred >= 2_800
                && engine.state().seats[usize::from(seller)].raw[usize::from(seller)] >= 2_000
            {
                let trade_domain = if t >= 5 { Some(0) } else { None };
                let _ = engine.apply_market_action(MarketAction::PostOffer {
                    seat: buyer,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(seller),
                        amount: 2_000,
                        price_per_unit_cred: 1_400,
                        domain_id: trade_domain,
                        is_hybrid: false,
                    },
                });
                let _ = engine.apply_market_action(MarketAction::PostOffer {
                    seat: seller,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(seller),
                        amount: 2_000,
                        price_per_unit_cred: 1_400,
                        domain_id: trade_domain,
                        is_hybrid: false,
                    },
                });
            }
        }

        if t == 2 {
            let secret = PreimageSecret([0x42; 32]);
            let cond = compute_preimage_condition(&secret).must("cond");
            let eid = engine.state().next_escrow_id;
            if engine
                .apply_market_action(MarketAction::CreateEscrow {
                    seat: p0,
                    escrow: EscrowProposal {
                        destination: p2,
                        asset: AssetId::Cred,
                        amount: 800,
                        finish_after_turn: Some(2),
                        cancel_after_turn: 6,
                        condition_hash: Some(cond),
                        bond: None,
                    },
                })
                .is_ok()
            {
                tracked_escrow = Some((eid, p2, secret));
            }
            let cid = engine.state().next_check_id;
            if engine
                .apply_market_action(MarketAction::CreateCheck {
                    seat: p1,
                    check: CheckProposal {
                        payee: p0,
                        asset: AssetId::Cred,
                        send_max: 600,
                        deliver_min: 500,
                        due_turn: 3,
                        expiration_turn: 6,
                    },
                })
                .is_ok()
            {
                tracked_check = Some((cid, p0));
            }
        }
        if t == 2
            && let Some((eid, dest, secret)) = tracked_escrow.take()
        {
            bundles[usize::from(dest)]
                .escrow_finishes
                .push(EscrowFinishUnlock {
                    escrow_id: eid,
                    preimage: Some(secret),
                });
        }
        if t == 3
            && let Some((cid, payee)) = tracked_check.take()
        {
            bundles[usize::from(payee)].check_cashes.push(cid);
        }

        if !complete_works_ring && t >= 2 {
            // Exact `run_scripted_full_season` baseline from `replay_invariants.rs`:
            for s in 0..seat_count {
                let s_idx = usize::from(s);
                let next_1 = ((s_idx + 1) % n) as u8;
                let next_2 = ((s_idx + 2) % n) as u8;
                let swap_domain = if t >= 5 { Some(0) } else { None };
                if engine.state().seats[s_idx].raw[s_idx] >= 2_500
                    && engine.state().seats[usize::from(next_1)].raw[usize::from(next_1)] >= 2_500
                    && s < next_1
                {
                    let sw_a = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + u64::from(s),
                        maker: s,
                        taker: next_1,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_000,
                        taker_asset: AssetId::Raw(next_1),
                        taker_amount: 2_000,
                        domain_id: swap_domain,
                    };
                    bundles[s_idx].atomic_swaps.push(sw_a.clone());
                    bundles[usize::from(next_1)].atomic_swaps.push(sw_a);
                }
                if engine.state().seats[s_idx].raw[s_idx] >= 2_000
                    && engine.state().seats[usize::from(next_2)].raw[usize::from(next_2)] >= 2_000
                    && s < next_2
                {
                    let sw_b = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + 50 + u64::from(s),
                        maker: s,
                        taker: next_2,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_000,
                        taker_asset: AssetId::Raw(next_2),
                        taker_amount: 2_000,
                        domain_id: swap_domain,
                    };
                    bundles[s_idx].atomic_swaps.push(sw_b.clone());
                    bundles[usize::from(next_2)].atomic_swaps.push(sw_b);
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
        } else if complete_works_ring && t >= 3 {
            // `c3`-style Works-completing chiral ring script generalized to N in {4, 5, 6}:
            // 1. 2_200-unit bilateral Raw swaps across all adjacent & +2 chords (`4 * 2_200 = 8_800`
            //    <= steady-state 12_480 Raw, delivering >= 2_024 net >= 2_000 converter input).
            let swap_domain = if t >= 5 { Some(0) } else { None };
            for s in 0..seat_count {
                let s_idx = usize::from(s);
                let next_1 = ((s_idx + 1) % n) as u8;
                if engine.state().seats[s_idx].raw[s_idx] >= 8_800
                    && engine.state().seats[usize::from(next_1)].raw[usize::from(next_1)] >= 8_800
                {
                    let sw_a = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + u64::from(s),
                        maker: s,
                        taker: next_1,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_200,
                        taker_asset: AssetId::Raw(next_1),
                        taker_amount: 2_200,
                        domain_id: swap_domain,
                    };
                    bundles[s_idx].atomic_swaps.push(sw_a.clone());
                    bundles[usize::from(next_1)].atomic_swaps.push(sw_a);
                }
                let next_2 = ((s_idx + 2) % n) as u8;
                if (n > 4 || s_idx < 2)
                    && engine.state().seats[s_idx].raw[s_idx] >= 8_800
                    && engine.state().seats[usize::from(next_2)].raw[usize::from(next_2)] >= 8_800
                {
                    let sw_b = AtomicSwapProposal {
                        swap_id: u64::from(t) * 100 + 50 + u64::from(s),
                        maker: s,
                        taker: next_2,
                        maker_asset: AssetId::Raw(s),
                        maker_amount: 2_200,
                        taker_asset: AssetId::Raw(next_2),
                        taker_amount: 2_200,
                        domain_id: swap_domain,
                    };
                    bundles[s_idx].atomic_swaps.push(sw_b.clone());
                    bundles[usize::from(next_2)].atomic_swaps.push(sw_b);
                }
            }

            // 2. One-way ContractDeliveryOrder of 1_950 Refined(s) -> next_1 (`s` is `prev` = upkeep_good of `next_1`,
            //    delivering >= 1_833 net >= 1_800 upkeep), plus rotating pair swap of 1_100 Refined(lead <-> partner)
            //    so `lead` delivers Project `lead + 1` (+4_800 ProjectEquity + Works stipend).
            for s in 0..seat_count {
                let s_idx = usize::from(s);
                let next_1 = ((s_idx + 1) % n) as u8;
                let role = (s_idx + n - rot) % n;
                if engine.state().seats[s_idx].refined[s_idx] >= 1_950 {
                    bundles[s_idx]
                        .contract_deliveries
                        .push(league_core::ContractDeliveryOrder {
                            counterparty: next_1,
                            good: AssetId::Refined(s),
                            gross_amount: 1_950,
                            domain_id: swap_domain,
                            dual_escrow_id: None,
                        });
                }
                bundles[s_idx].converter_runs.push(ConverterRunOrder {
                    recipe_idx: 0,
                    runs: if role == 0 { 2 } else { 1 },
                });
                if engine.state().seats[s_idx].cred_debt > 0 && role != n - 1 {
                    bundles[s_idx].debt_repayment_cred = 800;
                }
            }

            // Disjoint adjacent pairs `((t + rot + 2*k) % n, (t + rot + 2*k + 1) % n)` exchange 1_100 Refined
            // for Works project deliveries, giving cross-seat dispersion across builder/trader/saver roles.
            let pair_count = n / 2;
            for k in 0..pair_count {
                let lead = ((usize::from(t) + rot + 2 * k) % n) as u8;
                let partner = ((usize::from(lead) + 1) % n) as u8;
                let lead_role = (usize::from(lead) + n - rot) % n;
                // Role n-1 stockpiles refined variety every other turn instead of burning all surplus.
                if lead_role == n - 1 && t.is_multiple_of(2) {
                    continue;
                }
                if engine.state().seats[usize::from(lead)].refined[usize::from(lead)] >= 4_100
                    && engine.state().seats[usize::from(partner)].refined[usize::from(partner)]
                        >= 3_100
                {
                    let rsw = AtomicSwapProposal {
                        swap_id: u64::from(t) * 1_000 + u64::from(lead),
                        maker: partner,
                        taker: lead,
                        maker_asset: AssetId::Refined(partner),
                        maker_amount: 1_100,
                        taker_asset: AssetId::Refined(lead),
                        taker_amount: 1_100,
                        domain_id: swap_domain,
                    };
                    bundles[usize::from(partner)].atomic_swaps.push(rsw.clone());
                    bundles[usize::from(lead)].atomic_swaps.push(rsw);
                }
                bundles[usize::from(lead)]
                    .project_deliveries
                    .push(ProjectDeliveryOrder {
                        project_id: u16::from(lead) + 1,
                        batches: if lead_role == 0 { 2 } else { 1 },
                    });
            }
        }

        if t.is_multiple_of(4) {
            for s in 0..seat_count {
                let platform = CouncilPlatform {
                    proposed_tariffs_bps: vec![(p0, (700, 500))],
                    cartel_admissions: if s == p0 {
                        (0..seat_count).map(|m| (0, m)).collect()
                    } else {
                        Vec::new()
                    },
                    cartel_revocations: Vec::new(),
                };
                let _ =
                    engine.apply_market_action(MarketAction::StandForOffice { seat: s, platform });
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

        let _ = commit_and_reveal_bundles(&mut engine, tid, t, bundles);
        let mut bh = Sha256::new();
        bh.update(b"card2-part5/turn-beacon|");
        bh.update(eval_entropy);
        bh.update(tid.0.to_le_bytes());
        bh.update([t]);
        let turn_beacon: [u8; 32] = bh.finalize().into();
        let rec = engine.sweep_turn(turn_beacon).must("sweep");
        if rec.season_closed {
            break;
        }
    }

    for s in 0..seat_count {
        engine
            .submit_mandate_reveal(Seat(s), deal.seat_reveals[usize::from(s)].clone())
            .must("mandate reveal");
    }
    engine.finalize_season().must("finalize")
}

fn cross_seat_variance(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / (values.len() as f64);
    values.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / (values.len() as f64)
}

#[test]
fn mandate_measurement_prints_variance_share_and_leader_flip_rate_200_seasons_per_n() {
    const SEASONS_PER_N: u32 = 200;
    for (label, complete_works_ring) in [
        ("c3 Works-completing scripted ring seats", true),
        (
            "replay_invariants run_scripted_full_season unclosed-ring baseline (Med(W) <= 0)",
            false,
        ),
    ] {
        println!(
            "\n=== PART 5 MANDATE MEASUREMENT [{label}] (alpha = {:.4}, {} seasons/N) ===",
            f64::from(DEFAULT_MANDATE_ALPHA_BPS) / 10_000.0,
            SEASONS_PER_N
        );
        for n in [4_u8, 5_u8, 6_u8] {
            let mut leader_flips = 0_u32;
            let mut sum_var_b = 0.0_f64;
            let mut sum_var_s = 0.0_f64;
            let mut per_table_ratio_sum = 0.0_f64;
            let mut non_zero_var_tables = 0_u32;
            let mut completed_mandates = 0_u32;
            let mut mean_median_nw = 0.0_f64;

            for season_idx in 0..SEASONS_PER_N {
                let board = run_scripted_measurement_season(n, season_idx, complete_works_ring);
                let total_leader = board.standings[0].seat;

                let mut by_pre_nw: Vec<(u8, Units)> = board
                    .standings
                    .iter()
                    .map(|e| (e.seat, e.project_equity + e.net_asset_equity))
                    .collect();
                let nws_only: Vec<Units> = by_pre_nw.iter().map(|&(_, w)| w).collect();
                mean_median_nw += table_median_net_worth(&nws_only) as f64;

                by_pre_nw.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let pre_nw_leader = by_pre_nw[0].0;
                if total_leader != pre_nw_leader {
                    leader_flips += 1;
                }

                let mut bs = Vec::with_capacity(usize::from(n));
                let mut ss = Vec::with_capacity(usize::from(n));
                for e in &board.standings {
                    if e.mandate_bonus > 0 {
                        completed_mandates += 1;
                    }
                    bs.push(e.mandate_bonus as f64);
                    ss.push(e.total_equity as f64);
                }
                let vb = cross_seat_variance(&bs);
                let vs = cross_seat_variance(&ss);
                sum_var_b += vb;
                sum_var_s += vs;
                if vs > 0.0 {
                    per_table_ratio_sum += vb / vs;
                    non_zero_var_tables += 1;
                }
            }

            mean_median_nw /= f64::from(SEASONS_PER_N);
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
            let flip_rate_pct = 100.0 * f64::from(leader_flips) / f64::from(SEASONS_PER_N);
            let bonus_paid_rate_pct =
                100.0 * f64::from(completed_mandates) / f64::from(SEASONS_PER_N * u32::from(n));
            println!(
                "N={n} (seasons={SEASONS_PER_N}): mean_Med(W) = {mean_median_nw:.1} milli, \
                 pooled Var(B)/Var(S) = {pooled_var_share_pct:.2}%, \
                 mean per-table Var(B)/Var(S) = {mean_table_var_share_pct:.2}%, \
                 leader_flip_rate = {flip_rate_pct:.2}% ({leader_flips}/{SEASONS_PER_N}), \
                 positive_bonus_rate = {bonus_paid_rate_pct:.2}%"
            );
            assert_eq!(non_zero_var_tables, SEASONS_PER_N);
            if complete_works_ring {
                assert!(mean_median_nw > 0.0);
                assert!(completed_mandates > 0);
            }
        }
    }
}

#[test]
fn season_end_enforces_turn_32_hard_cap_and_35pct_hazard_table() {
    let expected_len = expected_season_length_turns(&DEFAULT_CLOSING_HAZARD_BPS);
    let cap_prob = cap_reach_probability(&DEFAULT_CLOSING_HAZARD_BPS);
    println!(
        "\n=== PART 6 SEASON END HAZARD TABLE ===\n\
         DEFAULT_CLOSING_HAZARD_BPS (Turns 24..=31) = {:?}\n\
         HARD_TURN_CAP = {}\n\
         E[T] = {:.6} turns\n\
         P(T == {}) = {:.6} ({:.4}%)",
        DEFAULT_CLOSING_HAZARD_BPS,
        HARD_TURN_CAP,
        expected_len,
        HARD_TURN_CAP,
        cap_prob,
        cap_prob * 100.0
    );

    assert_eq!(HARD_TURN_CAP, 32);
    assert_eq!(CLOSING_ROLL_START_TURN, 24);
    assert_eq!(DEFAULT_CLOSING_HAZARD_BPS, [3_500; 8]);
    // Exact analytical check: 24 + sum_{k=1..=8} 0.65^k = 24 + 0.65 * (1 - 0.65^8) / 0.35 = 25.797965963320312
    assert!((expected_len - 25.797_966).abs() < 1e-5);
    // Exact analytical check: 0.65^8 = 0.0318644812890625 (~3.19%)
    assert!((cap_prob - 0.031_864_48).abs() < 1e-6);

    // 1. Zero closing hazard runs strictly to hard cap Turn 32 and closes unconditionally.
    let tid = TableId(606);
    let mut spec = TableSpec::standard_chiral_ring(tid, 4, 3).must("spec");
    spec.closing_hazard_bps = [0; CLOSING_ROLL_WINDOW_LEN];
    let mut eng = TurnEngine::new(spec).must("engine");
    for t in 1..=HARD_TURN_CAP {
        assert_eq!(eng.state().phase, TurnPhase::TalkAndMarket);
        let closed = step_empty_turn(&mut eng, tid, t, 3);
        if t < HARD_TURN_CAP {
            assert!(!closed, "zero hazard must not close on Turn {t}");
        } else {
            assert!(closed, "hard cap Turn 32 must close unconditionally");
        }
    }
    let board = eng.finalize_season().must("finalize");
    assert_eq!(
        board.closed_turn, 32,
        "zero closing hazard must run to hard cap Turn 32, closed early at Turn {}",
        board.closed_turn
    );

    // 2. Maximum 100% hazard ([10_000; 8]) STILL never closes before Turn 24 (`Turns 1..=23` stay open),
    //    and closes immediately on Turn 24.
    let tid_early = TableId(616);
    let mut max_hz_spec = TableSpec::standard_chiral_ring(tid_early, 4, 4).must("spec");
    max_hz_spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
    let mut eng_early = TurnEngine::new(max_hz_spec).must("engine");
    for t in 1..=24_u8 {
        let closed = step_empty_turn(&mut eng_early, tid_early, t, 4);
        if t < 24 {
            assert!(
                !closed,
                "season must never close before Turn 24 (closed on Turn {t})"
            );
            assert!(eng_early.state().last_closing_roll_bps.is_none());
        } else {
            assert!(closed, "10_000 bps hazard must close on Turn 24");
            assert!(eng_early.state().last_closing_roll_bps.is_some());
        }
    }
    let early_board = eng_early.finalize_season().must("finalize early");
    assert_eq!(early_board.closed_turn, 24);
}

#[test]
fn final_marks_use_3turn_twap_or_executable_bid_depth_ignoring_thin_last_turn_spike() {
    assert_eq!(DEFAULT_TWAP_WINDOW_TURNS, 3);
    assert_eq!(DEFAULT_TWAP_MIN_TURN_VOLUME, 500);

    // Case A: Good with NO recent trades in the last 3 turns is marked by executable resting bid depth
    // (ignoring self-bids and valuing unfillable excess above resting bid depth at 0).
    let tid = TableId(607);
    let mut spec = TableSpec::standard_chiral_ring(tid, 4, 5).must("spec");
    spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
    for s in &mut spec.seats {
        s.initial_cred = 50 * UNIT_SCALE;
        s.perishable_decay_bps = 0;
        s.regent = None;
    }
    let mut eng = TurnEngine::new(spec.clone()).must("engine");
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        if t == 24 {
            // Seat 1 posts a resting Buy bid for 5_000 milli-units of Raw(0) at 1_200 milli-CRED/unit.
            eng.apply_market_action(MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 5_000,
                    price_per_unit_cred: 1_200,
                    domain_id: None,
                    is_hybrid: false,
                },
            })
            .must("resting bid");
            // Seat 0 also posts a self-bid at 9_999 milli-CRED/unit — must be ignored when marking Seat 0!
            eng.apply_market_action(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 2_000,
                    price_per_unit_cred: 9_999,
                    domain_id: None,
                    is_hybrid: false,
                },
            })
            .must("self bid");
        }
        if step_empty_turn(&mut eng, tid, t, 5) {
            break;
        }
    }
    // Seat 0 holds 240_000 Raw(0), locks 20_000 CRED in its resting Buy offer (counted in locked_cred),
    // and can sell at most 5_000 Raw(0) into Seat 1's non-self bid at 1_200 (= 6_000 milli-CRED).
    let s0 = &eng.state().seats[0];
    let locked_self_bid: Units = eng
        .state()
        .offers
        .values()
        .filter(|o| o.maker == 0 && o.side == OfferSide::Buy)
        .map(|o| o.locked_cred)
        .sum();
    let expected_marked_equity = s0.cred + locked_self_bid - s0.cred_debt + 6_000;
    let board = eng.finalize_season().must("finalize");
    let s0_entry = board.standings.iter().find(|e| e.seat == 0).must("seat 0");
    assert_eq!(
        s0_entry.net_asset_equity, expected_marked_equity,
        "good with no recent trades must be marked to executable bid depth ({expected_marked_equity}), got {}",
        s0_entry.net_asset_equity
    );

    // Case B: Liquid 3-turn TWAP (`Turns 22` and `23` trade >= 500 milli-units at 1_200 & 1_400 -> TWAP 1_300)
    // is completely immune to a last-turn (`Turn 24`) price spike on a thin book (`100 < 500` milli-units at 50_000)!
    let run_twap_vs_thin_spike = |with_thin_spike: bool| -> Units {
        let tid_twap = TableId(608);
        let mut sp = spec.clone();
        sp.table_id = tid_twap;
        // Disable shocks so Seat 0's Raw(0) yield is identical across turns.
        sp.shocks = vec![ScheduledShockSpec {
            hint_turn: 1,
            forecast_turn: 2,
            realize_turn: 3,
            target_seat: 3,
            kind: ShockKind::TariffHoliday,
            magnitude_bps: 100,
        }];
        let mut e = TurnEngine::new(sp).must("twap eng");
        for t in 1..=24_u8 {
            if t == 22 {
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 1_200,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("b22");
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 1_200,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("s22");
            }
            if t == 23 {
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 1_400,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("b23");
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 1_400,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("s23");
            }
            if t == 24 && with_thin_spike {
                // Seat 2 & Seat 3 execute a thin 100 milli-unit (< 500 min volume) trade on Raw(0)
                // at an extreme 50_000 CRED/unit spike! First give Seat 2 200 Raw(0) via TownImport.
                e.apply_market_action(MarketAction::TownImport {
                    seat: 2,
                    good: AssetId::Raw(0),
                    amount: 200,
                })
                .must("import raw0");
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 3,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 100,
                        price_per_unit_cred: 50_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("spike buy");
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 2,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 100,
                        price_per_unit_cred: 50_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("spike sell");
            }
            let closed = step_empty_turn(&mut e, tid_twap, t, 5);
            if closed {
                break;
            }
        }
        let b = e.finalize_season().must("fin twap");
        b.standings
            .iter()
            .find(|x| x.seat == 0)
            .must("seat 0")
            .net_asset_equity
    };

    let mark_clean = run_twap_vs_thin_spike(false);
    let mark_spiked = run_twap_vs_thin_spike(true);
    assert_eq!(
        mark_spiked, mark_clean,
        "a last-turn price spike on a thin book (< 500 milli-units) must NOT move the 3-turn TWAP mark ({mark_spiked} vs {mark_clean})"
    );

    // Case C: A lone last-turn trade (even >= 500 milli-units, with 0 trades on Turns 22 & 23)
    // ALSO never sets the mark ("never to the last trade"), falling back to resting bid depth (`0` when book is empty).
    let tid_lone = TableId(609);
    let mut sp_lone = spec;
    sp_lone.table_id = tid_lone;
    let mut e_lone = TurnEngine::new(sp_lone).must("lone eng");
    for t in 1..=24_u8 {
        if t == 24 {
            e_lone
                .apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 40_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("lone buy");
            e_lone
                .apply_market_action(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 1_000,
                        price_per_unit_cred: 40_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("lone sell");
        }
        if step_empty_turn(&mut e_lone, tid_lone, t, 5) {
            break;
        }
    }
    let s0_lone = &e_lone.state().seats[0];
    let expected_zero_depth = s0_lone.cred - s0_lone.cred_debt;
    let b_lone = e_lone.finalize_season().must("fin lone");
    let s0_lone_entry = b_lone.standings.iter().find(|x| x.seat == 0).must("seat 0");
    assert_eq!(
        s0_lone_entry.net_asset_equity, expected_zero_depth,
        "a single last-turn trade must never set a TWAP mark"
    );
}
