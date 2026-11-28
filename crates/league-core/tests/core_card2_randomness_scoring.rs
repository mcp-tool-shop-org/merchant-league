//! Integration tests and seeded-season measurements for Core Card 2 + PR #40 Fixes 1 & 2:
//! - Part 1: Game-mode randomness (#31) from pre-named drand beacons + per-use HKDF streams
//! - Part 2: Eval-mode randomness (#31 comment) from sealed OS CSPRNG VRF seed + C2SP ristretto255 ECVRF proofs (`B-3`, `B-5`, `C-3`)
//! - Part 4: Mandate point conversion (`points * alpha * max(Med(W), 0.5 * W_0)`, `N = 4, 5, 6`, `A-3`)
//! - Part 5: Seeded-season mandate variance share (`Var(B)/Var(S)`) & leader flip rate (`>= 200` seasons/N)
//! - Part 6: Hard cap at Turn 32 + `35%` closing hazard table (`E[T]`, `P(T == 32)`)
//! - Part 7: Final close-marking rule (Fix 2: trade prices do not count; standing bids first floored at `50%` of base value; rest at `50%` of base value)
//! - Fix 1 `A-2` (`C1a` active-trading exemption + Turn-1 skip) and `B-7` (final-standings beacon tie-break)

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use league_core::{
    AssetId, AtomicSwapProposal, BPS_SCALE, CLOSING_ROLL_START_TURN, CLOSING_ROLL_WINDOW_LEN,
    CharterBidOrder, CharterRole, CheckProposal, CommitmentHash, ConverterRunOrder,
    CouncilPlatform, CouncilVoteBallot, CreditTier, DEFAULT_CANCEL_LOCK_BPS,
    DEFAULT_CLOSING_HAZARD_BPS, DEFAULT_MANDATE_ALPHA_BPS, DEFAULT_SHOCK_DRAW_TURNS,
    DEFAULT_WASH_FREEZE_BOND_CRED, DEFAULT_WASH_FREEZE_BOUNTY_CRED,
    DEFAULT_WASH_ROUND_TRIP_MIN_UNITS, DRAND_QUICKNET_GENESIS_UNIX, DrandSchedule, EngineError,
    EscrowFinishUnlock, EscrowProposal, FinalScoreboard, HARD_TURN_CAP, LoggedTurn,
    MANDATE_STARTING_NW_FLOOR_BPS, MandateKind, MarketAction, OfferProposal, OfferSide,
    OrderReveal, PreimageSecret, ProjectDeliveryOrder, ReplayBeaconMode, ReplayLayoutSeed,
    RevealSalt, RngDomain, ScheduledShockSpec, SealedEvalSeed, SealedOrderBundle, SeasonReplayLog,
    Seat, SeatIdentity, ShockKind, TableId, TableMode, TableSpec, Turn, TurnEngine, TurnPhase,
    UNIT_SCALE, UNSOLD_EXCESS_PARITY_BPS, Units, cap_reach_probability, close_roll_bps_from_beacon,
    compute_mandate_commitment, compute_order_commitment, compute_preimage_condition,
    compute_table_median_starting_net_worth, convert_mandate_points_to_equity_with_floor,
    convert_mandate_points_to_equity_with_median, deal_graded_mandates_with_entropy,
    derive_drand_beacon, derive_turn_beacon, draw_shocks_for_turn, expand_turn_stream,
    expected_season_length_turns, mark_good_holding_at_close, replay_season, standings_tie_key,
    table_median_net_worth,
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
    let vf: VectorFile = serde_json::from_str(include_str!("vectors/drand_quicknet_vectors.json"))
        .must("parse vendored quicknet vectors");
    let entry = vf
        .beacons
        .into_iter()
        .find(|b| b.round == round)
        .must("vendored quicknet round present");
    hex::decode(entry.signature).must("decode quicknet signature hex")
}

fn run_game_season_from_schedule(
    spec: &TableSpec,
    schedule: DrandSchedule,
) -> (SeasonReplayLog, FinalScoreboard, Vec<ScheduledShockSpec>) {
    let tid = spec.table_id;
    let n = usize::from(spec.seat_count);
    let (_, deal) =
        TableSpec::standard_game_chiral_ring_with_mandates(tid, spec.seat_count, 0, schedule)
            .must("fresh deal");
    let mut spec_run = spec.clone();
    spec_run.drand_schedule = Some(schedule);
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
        let sig = quicknet_sig_for_round(round);
        let beacon = derive_drand_beacon(&sig).must("drand beacon");
        let rec = eng
            .sweep_turn_with_drand(round, &sig)
            .must("sweep_turn_with_drand");
        all_drawn.extend(rec.newly_drawn_shocks.iter().copied());
        turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commit_deadline_unix: None,
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
        seed: ReplayLayoutSeed::try_clear(0).must("clear seed 0"),
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
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX,
        turn_period_secs: 3,
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
    let sample_beacon = derive_drand_beacon(&quicknet_sig_for_round(24)).must("beacon");
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
    // AND different closing turns on the exact same Game TableSpec layout.
    let (log_a, board_a, shocks_a) = run_game_season_from_schedule(&spec, schedule);
    let mut alt_found = None;
    for offset in 1..=128_u64 {
        let cand_schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + offset * 3,
            turn_period_secs: 3,
        };
        let (cand_log, cand_board, cand_shocks) =
            run_game_season_from_schedule(&spec, cand_schedule);
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
    let extra_round = schedule
        .expected_round_for_turn(Turn(extra_t))
        .must("extra round");
    let extra_sig = quicknet_sig_for_round(extra_round);
    post_close_log.turns.push(LoggedTurn {
        turn: extra_t,
        market_actions: Vec::new(),
        cancel_lock_at: None,
        commit_deadline_unix: None,
        commitments: BTreeMap::new(),
        reveals: BTreeMap::new(),
        turn_beacon: derive_drand_beacon(&extra_sig).must("sig"),
        drand_round: Some(extra_round),
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
            cancel_lock_at: None,
            commit_deadline_unix: None,
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
        seed: ReplayLayoutSeed::try_clear(layout_seed).must("clear layout seed"),
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

    // B-3: Live TurnEngine must refuse both raw `sweep_turn` and `sweep_turn_seeded` when
    // `eval_vrf_pubkey` is present, before replay is ever reached.
    let mut b3_eng = TurnEngine::new(spec_a.clone()).must("b3 eval eng");
    let _ = commit_and_reveal_bundles(&mut b3_eng, tid, 1, vec![SealedOrderBundle::default(); 5]);
    assert!(
        matches!(
            b3_eng.sweep_turn([0x42; 32]),
            Err(EngineError::InvalidConfig(_))
        ),
        "B-3: sweep_turn must refuse unverified raw beacon when eval_vrf_pubkey is set"
    );
    assert!(
        matches!(
            b3_eng.sweep_turn_seeded(42),
            Err(EngineError::InvalidConfig(_))
        ),
        "B-3: sweep_turn_seeded must refuse u64-derived beacon when eval_vrf_pubkey is set"
    );
    let (_, b3_prf) = eval_seed_a.prove_turn(tid, Turn(1)).must("b3 prove");
    b3_eng
        .sweep_turn_with_eval_vrf(&b3_prf)
        .must("B-3: verified VRF proof succeeds after raw/seeded refusals");

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
    assert_eq!(MANDATE_STARTING_NW_FLOOR_BPS, 5_000);
    // A-3 Formula 3 floor: when Med(W) <= 0.5 * W_0 (including negative Med(W)),
    // B_i = round_half_up(m_i * alpha * 0.5 * W_0).
    // Example: m_i = 5.000 (5_000), W_0 = 60_800 -> 0.5 * W_0 = 30_400, alpha = 250 bps -> 3_800.
    assert_eq!(
        convert_mandate_points_to_equity_with_floor(
            5_000,
            -129_000,
            60_800,
            DEFAULT_MANDATE_ALPHA_BPS,
        ),
        3_800
    );
    assert_eq!(
        convert_mandate_points_to_equity_with_floor(
            5_000,
            20_000,
            60_800,
            DEFAULT_MANDATE_ALPHA_BPS,
        ),
        3_800
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
        let w0 = compute_table_median_starting_net_worth(&spec);
        assert!(median_nw > w0 / 2);
        let expected_bonus = convert_mandate_points_to_equity_with_floor(
            pts_milli,
            median_nw,
            w0,
            DEFAULT_MANDATE_ALPHA_BPS,
        );
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

    // A-3 end-to-end distressed table check: when terminal Med(W) < 0 (well below 0.5 * W_0),
    // completed mandates still pay positive bonus `round_half_up(points * alpha * 0.5 * W_0)`.
    let mut distressed_spec = base_spec;
    for sc in &mut distressed_spec.seats {
        sc.initial_cred = 20_000;
        sc.perishable_decay_bps = 9_500;
    }
    let w0_distressed = compute_table_median_starting_net_worth(&distressed_spec);
    assert_eq!(w0_distressed, 20_000);
    let mut eng_dist = TurnEngine::new(distressed_spec).must("distressed eng");
    for t in 1..=HARD_TURN_CAP {
        if eng_dist.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        if step_empty_turn(&mut eng_dist, tid_inv, t, 12) {
            break;
        }
    }
    eng_dist
        .submit_mandate_reveal(Seat(0), deal4.seat_reveals[0].clone())
        .must("reveal distressed seat 0");
    let dist_board = eng_dist.finalize_season().must("finalize distressed");
    let dist_nws: Vec<Units> = dist_board
        .standings
        .iter()
        .map(|e| e.project_equity + e.net_asset_equity)
        .collect();
    let dist_med = table_median_net_worth(&dist_nws);
    assert!(
        dist_med < 0,
        "distressed table must have negative Med(W), got {dist_med}"
    );
    let dist_s0 = dist_board
        .standings
        .iter()
        .find(|e| e.seat == 0)
        .must("dist s0");
    assert_eq!(
        dist_s0.mandate_bonus,
        convert_mandate_points_to_equity_with_floor(
            6_000,
            dist_med,
            w0_distressed,
            DEFAULT_MANDATE_ALPHA_BPS
        ),
        "A-3: distressed negative-median table must pay 6.000 * 0.025 * (0.5 * 20_000) = 1_500"
    );
    assert_eq!(dist_s0.mandate_bonus, 1_500);
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
fn final_marks_use_standing_bids_with_50pct_floor_and_ignore_all_trades() {
    assert_eq!(UNSOLD_EXCESS_PARITY_BPS, 5_000);

    // Case A: Good with resting bid above floor + self-bid (`TableId(607)`).
    // Units covered by non-self resting bids above 50% of base value mark at bid price (`5_000` @ `1_200` = `6_000`),
    // self-bids are ignored, and the remaining `235_000` unsold units above bid depth mark at
    // 50% of fixed base value (`parity = 1_750` -> `875` per unit -> `205_625` milli-CRED).
    let tid = TableId(607);
    let mut spec = TableSpec::standard_chiral_ring(tid, 4, 5).must("spec");
    spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
    spec.shocks = vec![ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 3,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 100,
    }];
    for s in &mut spec.seats {
        s.initial_cred = 50 * UNIT_SCALE;
        s.perishable_decay_bps = 0;
        s.regent = None;
    }
    let mut eng = TurnEngine::new(spec.clone()).must("engine");
    let raw0_parity = eng.state().parities.asset_parity(AssetId::Raw(0));
    assert_eq!(raw0_parity, 1_750);
    for t in 1..=HARD_TURN_CAP {
        if eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        if t == 23 {
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
        }
        if t == 24 {
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
    let s0 = &eng.state().seats[0];
    assert_eq!(s0.raw[0], 240_000);
    let locked_self_bid: Units = eng
        .state()
        .offers
        .values()
        .filter(|o| o.maker == 0 && o.side == OfferSide::Buy)
        .map(|o| o.locked_cred)
        .sum();
    let expected_unsold_half_base =
        ((235_000_i128 * i128::from(raw0_parity) * 5_000) / (1_000 * 10_000)) as Units;
    assert_eq!(expected_unsold_half_base, 205_625);
    let expected_marked_equity =
        s0.cred + locked_self_bid - s0.cred_debt + 6_000 + expected_unsold_half_base;
    let board = eng.finalize_season().must("finalize");
    let s0_entry = board.standings.iter().find(|e| e.seat == 0).must("seat 0");
    assert_eq!(
        s0_entry.net_asset_equity, expected_marked_equity,
        "non-qualifying good must mark bid-covered units at bid price and excess at 50% base value ({expected_marked_equity}), got {}",
        s0_entry.net_asset_equity
    );

    // B-6: Pulling every resting bid on the last turn cannot take a seat's stock below half base value.
    let tid_pull = TableId(617);
    let mut sp_pull = spec.clone();
    sp_pull.table_id = tid_pull;
    let mut eng_pull = TurnEngine::new(sp_pull).must("pull eng");
    let mut posted_bid_ids = Vec::new();
    for t in 1..=24_u8 {
        if t == 23 {
            for (buyer, amt, px) in [(1_u8, 10_000, 1_800), (2_u8, 15_000, 1_600)] {
                let next_id = eng_pull.state().next_offer_id;
                eng_pull
                    .apply_market_action(MarketAction::PostOffer {
                        seat: buyer,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(0),
                            amount: amt,
                            price_per_unit_cred: px,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("post bid t23");
                posted_bid_ids.push((buyer, next_id));
            }
        }
        if t == 24 {
            for &(buyer, oid) in &posted_bid_ids {
                eng_pull
                    .apply_market_action(MarketAction::CancelOffer {
                        seat: buyer,
                        offer_id: oid,
                    })
                    .must("pull bid t24");
            }
        }
        if step_empty_turn(&mut eng_pull, tid_pull, t, 5) {
            break;
        }
    }
    let pulled_stock_mark =
        mark_good_holding_at_close(eng_pull.state(), 0, AssetId::Raw(0), 240_000);
    let exact_half_base_240 =
        ((240_000_i128 * i128::from(raw0_parity) * 5_000) / (1_000 * 10_000)) as Units;
    assert_eq!(pulled_stock_mark, exact_half_base_240);
    assert_eq!(pulled_stock_mark, 210_000);

    // Case B (`A-1` + `B-6`, `TableId(608)`): Seat 0 is NOT party to Turn-24 spike between Seats 2 & 3.
    // Test spike sizes 0.499, 0.500, and 50.000 units, upward (50 CRED/unit = 50_000) and downward
    // (1 milli-CRED/unit), each with and without the Turn-22 trade.
    let run_case_b = |turn_22_trade: bool,
                      spike_opt: Option<(Units, Units)>|
     -> (Units, Units, Units) {
        let tid_b = TableId(608);
        let mut sp = spec.clone();
        sp.table_id = tid_b;
        // Seat 0 keeps initial_cred = 50_000; Seats 2 & 3 hold working capital sufficient to
        // clear the 50.000-unit upward spike (`2_500_000` milli-CRED + `TownImport`).
        sp.seats[2].initial_cred = 3_000 * UNIT_SCALE;
        sp.seats[3].initial_cred = 3_000 * UNIT_SCALE;
        sp.shocks = vec![ScheduledShockSpec {
            hint_turn: 1,
            forecast_turn: 2,
            realize_turn: 3,
            target_seat: 3,
            kind: ShockKind::TariffHoliday,
            magnitude_bps: 100,
        }];
        let mut e = TurnEngine::new(sp).must("case b eng");
        for t in 1..=24_u8 {
            if t == 22 && turn_22_trade {
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
            if t == 24
                && let Some((spike_units, spike_price)) = spike_opt
            {
                e.apply_market_action(MarketAction::TownImport {
                    seat: 2,
                    good: AssetId::Raw(0),
                    amount: spike_units,
                })
                .must("import raw0 for spike");
                e.apply_market_action(MarketAction::PostOffer {
                    seat: 3,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: spike_units,
                        price_per_unit_cred: spike_price,
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
                        amount: spike_units,
                        price_per_unit_cred: spike_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("spike sell");
            }
            let _ =
                commit_and_reveal_bundles(&mut e, tid_b, t, vec![SealedOrderBundle::default(); 4]);
            let rec = e
                .sweep_turn(derive_turn_beacon(5, tid_b, Turn(t)))
                .must("sweep case b");
            if t == 24 && spike_opt.is_some() {
                assert_eq!(
                    rec.batch_trades_cleared, 1,
                    "Turn-24 spike order must actually clear in Step 0 batch"
                );
            }
            if rec.season_closed {
                break;
            }
        }
        let s0_units = e.state().seats[0].raw[0];
        let s0_holding_mark = mark_good_holding_at_close(e.state(), 0, AssetId::Raw(0), s0_units);
        let b = e.finalize_season().must("fin case b");
        let s0_eq = b
            .standings
            .iter()
            .find(|x| x.seat == 0)
            .must("seat 0")
            .net_asset_equity;
        (s0_units, s0_holding_mark, s0_eq)
    };

    println!("\n=== A-1 CASE B SPIKE MATRIX (TableId 608, N=4) ===");
    for turn_22_trade in [true, false] {
        let (base_units, base_mark, base_eq) = run_case_b(turn_22_trade, None);
        println!(
            "turn_22={turn_22_trade:<5} spike=none                        s0_raw0={base_units} mark={base_mark} s0_equity={base_eq}"
        );
        if !turn_22_trade {
            // B-6: 239-units-with-no-bids case has non-negative holding value = half base value * 239.
            assert_eq!(base_units, 239_000);
            let expected_239 =
                ((239_000_i128 * i128::from(raw0_parity) * 5_000) / (1_000 * 10_000)) as Units;
            assert_eq!(base_mark, expected_239);
            assert_eq!(base_mark, 209_125);
            assert!(base_mark > 0);
            assert!(base_eq > 0);
        } else {
            assert_eq!(base_units, 238_000);
            assert_eq!(base_mark, 208_250);
        }
        for (dir_label, px) in [("up_50_cred", 50_000_i64), ("down_1_milli", 1_i64)] {
            for spike_u in [499_i64, 500_i64, 50_000_i64] {
                let (u, mk, eq) = run_case_b(turn_22_trade, Some((spike_u, px)));
                println!(
                    "turn_22={turn_22_trade:<5} dir={dir_label:<12} spike={spike_u:>6} s0_raw0={u} mark={mk} s0_equity={eq} delta={}",
                    eq - base_eq
                );
                assert_eq!(u, base_units);
                assert_eq!(mk, base_mark);
                assert_eq!(
                    eq, base_eq,
                    "A-1: non-qualifying spike ({dir_label}, {spike_u} milli-units, turn_22={turn_22_trade}) must leave Seat 0 equity identical ({eq} vs {base_eq})"
                );
            }
        }
    }

    // Rewritten Fix-1 3-turn heavy-volume trade case (`TableId(609)`): Turns 22, 23, 24 all clear
    // >= 2% of table stock (`6_000 @ 1_200`, `10_000 @ 1_500`, `8_000 @ 2_100`). Under Fix 2's
    // final close-marking rule, trade prices never enter the close mark: Seat 0's 240.000 units
    // with no standing bids mark strictly at 50% of base value (`210_000`), not `390_000`.
    let tid_qual = TableId(609);
    let mut sp_qual = spec.clone();
    sp_qual.table_id = tid_qual;
    sp_qual.seats[1].initial_cred = 200 * UNIT_SCALE;
    sp_qual.seats[2].initial_cred = 200 * UNIT_SCALE;
    sp_qual.shocks = vec![ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 3,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 100,
    }];
    let mut e_qual_base = TurnEngine::new(sp_qual.clone()).must("qual base eng");
    for t in 1..=24_u8 {
        if step_empty_turn(&mut e_qual_base, tid_qual, t, 5) {
            break;
        }
    }
    let s0_qual_base_eq = e_qual_base
        .finalize_season()
        .must("qual base fin")
        .standings
        .iter()
        .find(|x| x.seat == 0)
        .must("seat 0")
        .net_asset_equity;

    let mut e_qual = TurnEngine::new(sp_qual).must("qual eng");
    for t in 1..=24_u8 {
        let trade_spec = match t {
            22 => Some((6_000_i64, 1_200_i64)),
            23 => Some((10_000_i64, 1_500_i64)),
            24 => Some((8_000_i64, 2_100_i64)),
            _ => None,
        };
        if let Some((qty, px)) = trade_spec {
            // Seats 1 & 2 trade Raw(0) (sourced via TownImport by Seat 2) so Seat 0 is not party!
            e_qual
                .apply_market_action(MarketAction::TownImport {
                    seat: 2,
                    good: AssetId::Raw(0),
                    amount: qty,
                })
                .must("qual import");
            e_qual
                .apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: qty,
                        price_per_unit_cred: px,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("qual buy");
            e_qual
                .apply_market_action(MarketAction::PostOffer {
                    seat: 2,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: qty,
                        price_per_unit_cred: px,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("qual sell");
        }
        if step_empty_turn(&mut e_qual, tid_qual, t, 5) {
            break;
        }
    }
    let s0_qual_units = e_qual.state().seats[0].raw[0];
    assert_eq!(s0_qual_units, 240_000);
    let qual_mark = mark_good_holding_at_close(e_qual.state(), 0, AssetId::Raw(0), s0_qual_units);
    assert_eq!(
        qual_mark, 210_000,
        "Fix 2: 3-turn heavy trade volume must be ignored completely (50% base * 240 = 210_000)"
    );
    let s0_qual_eq = e_qual
        .finalize_season()
        .must("qual fin")
        .standings
        .iter()
        .find(|x| x.seat == 0)
        .must("seat 0")
        .net_asset_equity;
    assert_eq!(s0_qual_eq, s0_qual_base_eq);
}

#[test]
fn pr40_fix2_close_marking_attack_and_bid_matrix_matches_director_rule() {
    let half_base = |units: Units, parity: Units| -> Units {
        ((i128::from(units) * i128::from(parity) * i128::from(UNSOLD_EXCESS_PARITY_BPS))
            / (i128::from(UNIT_SCALE) * BPS_SCALE)) as Units
    };

    // Shared 4-seat chiral ring fixture (`TableId(620)`) with real perishable spoilage (`20%`/turn),
    // real export tariffs (`8%` raw / `6%` refined), `15.000 CRED` initial capital, closing at Turn 24.
    let tid = TableId(620);
    let mut base_spec = TableSpec::standard_chiral_ring(tid, 4, 5).must("spec 620");
    base_spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
    base_spec.shocks = vec![ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 3,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 100,
    }];
    for s in &mut base_spec.seats {
        s.initial_cred = 15_000;
        s.regent = None;
    }

    let step_t = |eng: &mut TurnEngine, t: u8| -> league_core::SweepReceipt {
        let _ = commit_and_reveal_bundles(eng, tid, t, vec![SealedOrderBundle::default(); 4]);
        eng.sweep_turn(derive_turn_beacon(5, tid, Turn(t)))
            .must("sweep 620")
    };
    let total_good_stock = |eng: &TurnEngine, good: AssetId| -> Units {
        let idx = match good {
            AssetId::Raw(g) => usize::from(g),
            _ => panic!("expected Raw"),
        };
        let free: Units = eng.state().seats.iter().map(|s| s.raw[idx].max(0)).sum();
        let resting: Units = eng
            .state()
            .offers
            .values()
            .filter(|o| o.side == OfferSide::Sell && o.good == good && o.remaining_units > 0)
            .map(|o| o.remaining_units)
            .sum();
        free + resting
    };

    // Baseline (no attack, no bids).
    let mut eng_base = TurnEngine::new(base_spec.clone()).must("eng_base");
    let parity_raw0 = eng_base.state().parities.asset_parity(AssetId::Raw(0));
    let parity_raw2 = eng_base.state().parities.asset_parity(AssetId::Raw(2));
    assert_eq!(parity_raw0, 1_750);
    assert_eq!(parity_raw2, 1_750);
    for t in 1..=24_u8 {
        let r = step_t(&mut eng_base, t);
        if r.season_closed {
            break;
        }
    }
    let s0_hold_base = eng_base.state().seats[0].raw[0];
    let s2_hold_base = eng_base.state().seats[2].raw[2];
    let s2_cred_base = eng_base.state().seats[2].cred;
    let s2_debt_base = eng_base.state().seats[2].cred_debt;
    let board_base = eng_base.finalize_season().must("fin base");
    let seat_eq = |b: &FinalScoreboard, seat: u8| -> Units {
        b.standings
            .iter()
            .find(|e| e.seat == seat)
            .must("seat")
            .net_asset_equity
    };
    let s0_base_eq = seat_eq(&board_base, 0);
    let s1_base_eq = seat_eq(&board_base, 1);
    let s2_base_eq = seat_eq(&board_base, 2);
    let s3_base_eq = seat_eq(&board_base, 3);

    println!(
        "\n=== PR40 FIX2 ITEM-3 CLOSE-MARKING MATRIX (TableId 620, N=4, parity=1,750, initial_cred=15,000) ==="
    );

    // 1 & 2. Wash up & Wash down: Seats 2 and 3 ping-pong >= 2% of table's Raw(0) stock on T=22, 23, 24.
    for (case_name, upward) in [
        ("1. Wash up (2<->3 >=2% stock T22..T24, max px)", true),
        ("2. Wash down (2<->3 >=2% stock T22..T24, 1 milli)", false),
    ] {
        let mut eng = TurnEngine::new(base_spec.clone()).must("wash eng");
        let mut chosen_px = 1_i64;
        for t in 1..=24_u8 {
            if (22..=24).contains(&t) {
                let (seller, buyer) = if t % 2 == 0 {
                    (2_u8, 3_u8)
                } else {
                    (3_u8, 2_u8)
                };
                let stock = total_good_stock(&eng, AssetId::Raw(0));
                let q = (((i128::from(stock) * 200 + 9_999) / 10_000) as Units).max(1);
                let have = eng.state().seats[usize::from(seller)].raw[0];
                if have < q {
                    eng.apply_market_action(MarketAction::TownImport {
                        seat: seller,
                        good: AssetId::Raw(0),
                        amount: q - have,
                    })
                    .must("wash import");
                }
                if t == 22 {
                    chosen_px = if upward {
                        let usable =
                            (eng.state().seats[usize::from(buyer)].cred - 2_500).max(1_000);
                        ((i128::from(usable) * 1_000) / i128::from(q + 200)) as Units
                    } else {
                        1
                    };
                }
                eng.apply_market_action(MarketAction::PostOffer {
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
                .must("wash buy");
                eng.apply_market_action(MarketAction::PostOffer {
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
                .must("wash sell");
            }
            let r = step_t(&mut eng, t);
            if (22..=24).contains(&t) {
                assert_eq!(r.batch_trades_cleared, 1, "wash trade must clear on T={t}");
            }
            if r.season_closed {
                break;
            }
        }
        let b = eng.finalize_season().must("fin wash");
        let s0_eq = seat_eq(&b, 0);
        println!(
            "{case_name:<52} | seat=0 | eq={s0_eq:>8} | base={s0_base_eq:>8} | delta={:+}",
            s0_eq - s0_base_eq
        );
        assert_eq!(
            s0_eq, s0_base_eq,
            "{case_name}: Seat 0 equity must equal baseline"
        );
    }

    // 3. 3-seat ring: 2 -> 3 -> 1 -> 2 on T=22, 23, 24 at highest price all three can pay.
    {
        let mut eng = TurnEngine::new(base_spec.clone()).must("ring3 eng");
        let mut chosen_px = 1_i64;
        for t in 1..=24_u8 {
            if (22..=24).contains(&t) {
                let (seller, buyer) = match t {
                    22 => (2_u8, 3_u8),
                    23 => (3_u8, 1_u8),
                    _ => (1_u8, 2_u8),
                };
                let stock = total_good_stock(&eng, AssetId::Raw(0));
                let q = (((i128::from(stock) * 200 + 9_999) / 10_000) as Units).max(1);
                let have = eng.state().seats[usize::from(seller)].raw[0];
                if have < q {
                    eng.apply_market_action(MarketAction::TownImport {
                        seat: seller,
                        good: AssetId::Raw(0),
                        amount: q - have,
                    })
                    .must("ring3 import");
                }
                if t == 22 {
                    let min_cred = [1_usize, 2, 3]
                        .into_iter()
                        .map(|i| eng.state().seats[i].cred)
                        .min()
                        .unwrap_or(1_000);
                    let usable = (min_cred - 3_500).max(1_000);
                    chosen_px = ((i128::from(usable) * 1_000) / i128::from(q + 200)) as Units;
                }
                eng.apply_market_action(MarketAction::PostOffer {
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
                .must("ring3 buy");
                eng.apply_market_action(MarketAction::PostOffer {
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
                .must("ring3 sell");
            }
            let r = step_t(&mut eng, t);
            if (22..=24).contains(&t) {
                assert_eq!(
                    r.batch_trades_cleared, 1,
                    "3-seat ring trade must clear on T={t}"
                );
            }
            if r.season_closed {
                break;
            }
        }
        let b = eng.finalize_season().must("fin ring3");
        let s0_eq = seat_eq(&b, 0);
        println!(
            "{:<52} | seat=0 | eq={s0_eq:>8} | base={s0_base_eq:>8} | delta={:+}",
            "3. 3-seat ring (2->3->1->2 T22..T24, up)",
            s0_eq - s0_base_eq
        );
        assert_eq!(
            s0_eq, s0_base_eq,
            "3-seat ring: Seat 0 equity must equal baseline"
        );
    }

    // 4. Self-pump: Seats 2 and 3 wash Seat 2's own Raw(2) upward across T=22, 23, 24.
    // Verify Raw(2)'s mark is strictly 50% of base value (zero mark lift) and Seat 2's equity
    // delta equals the exact CRED transferred from Seat 3 minus imports/spoilage/tariff on the
    // traded parcel (and pair sum s2_delta + s3_delta < 0).
    {
        let mut eng = TurnEngine::new(base_spec.clone()).must("self-pump eng");
        let mut chosen_px = 1_i64;
        let mut s2_import_cred = 0_i64;
        let mut s3_import_cred = 0_i64;
        let mut s2_net_trade_cred = 0_i64;
        for t in 1..=24_u8 {
            if (22..=24).contains(&t) {
                let (seller, buyer) = if t % 2 == 0 {
                    (2_u8, 3_u8)
                } else {
                    (3_u8, 2_u8)
                };
                let stock = total_good_stock(&eng, AssetId::Raw(2));
                let q = (((i128::from(stock) * 200 + 9_999) / 10_000) as Units).max(1);
                let have = eng.state().seats[usize::from(seller)].raw[2];
                if have < q {
                    let before = eng.state().seats[usize::from(seller)].cred;
                    eng.apply_market_action(MarketAction::TownImport {
                        seat: seller,
                        good: AssetId::Raw(2),
                        amount: q - have,
                    })
                    .must("self-pump import");
                    let paid = before - eng.state().seats[usize::from(seller)].cred;
                    if seller == 2 {
                        s2_import_cred += paid;
                    } else {
                        s3_import_cred += paid;
                    }
                }
                if t == 22 {
                    let s3_usable = (eng.state().seats[3].cred - 2_000).max(1_000);
                    chosen_px = ((i128::from(s3_usable) * 1_000) / i128::from(q + 200)) as Units;
                }
                let s2_cred_pre = eng.state().seats[2].cred;
                eng.apply_market_action(MarketAction::PostOffer {
                    seat: buyer,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(2),
                        amount: q,
                        price_per_unit_cred: chosen_px,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("self-pump buy");
                eng.apply_market_action(MarketAction::PostOffer {
                    seat: seller,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(2),
                        amount: q,
                        price_per_unit_cred: chosen_px,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("self-pump sell");
                let r = step_t(&mut eng, t);
                assert_eq!(
                    r.batch_trades_cleared, 1,
                    "self-pump trade must clear on T={t}"
                );
                s2_net_trade_cred += eng.state().seats[2].cred - s2_cred_pre;
                if r.season_closed {
                    break;
                }
            } else {
                let r = step_t(&mut eng, t);
                if r.season_closed {
                    break;
                }
            }
        }
        let s2_hold = eng.state().seats[2].raw[2];
        let s2_raw2_mark = mark_good_holding_at_close(eng.state(), 2, AssetId::Raw(2), s2_hold);
        assert_eq!(
            s2_raw2_mark,
            half_base(s2_hold, parity_raw2),
            "Self-pump must get zero mark inflation: Raw(2) marks strictly at 50% of base value"
        );
        let s2_cred_end = eng.state().seats[2].cred;
        let s2_debt_end = eng.state().seats[2].cred_debt;
        let b = eng.finalize_season().must("fin self-pump");
        let s2_eq = seat_eq(&b, 2);
        let s3_eq = seat_eq(&b, 3);
        let s2_delta = s2_eq - s2_base_eq;
        let s3_delta = s3_eq - s3_base_eq;
        let cred_delta = (s2_cred_end - s2_cred_base) - (s2_debt_end - s2_debt_base);
        let inv_floor_delta =
            half_base(s2_hold, parity_raw2) - half_base(s2_hold_base, parity_raw2);
        println!(
            "{:<52} | seat=2 | eq={s2_eq:>8} | base={s2_base_eq:>8} | delta={s2_delta:+} (s2_net_cred={s2_net_trade_cred:+}, s2_imports={s2_import_cred}, s2_inv_50pct={inv_floor_delta:+}, partner_s3_delta={s3_delta:+}, s3_imports={s3_import_cred})",
            "4. Self-pump (2<->3 Raw(2) T22..T24, up)"
        );
        assert_eq!(
            s2_cred_end - s2_cred_base,
            s2_net_trade_cred - s2_import_cred
        );
        assert_eq!(s2_delta, cred_delta + inv_floor_delta);
        assert!(s2_delta <= s2_net_trade_cred - s2_import_cred);
        assert!(
            s2_delta + s3_delta < 0,
            "pair net equity must be negative (pure transfer + import/tariff/spoilage loss): s2={s2_delta}, s3={s3_delta}"
        );
        // Pin exact deterministic self-pump transfer figures.
        assert_eq!(s2_net_trade_cred, 10_420);
        assert_eq!(s2_import_cred, 0);
        assert_eq!(s3_import_cred, 827);
        assert_eq!(inv_floor_delta, -504);
        assert_eq!(s2_delta, 9_916);
        assert_eq!(s3_delta, -10_734);
    }

    // 5a & 5b. Lowball bid at 1 milli-CRED/unit on Turn 24:
    // (a) for less than Seat 0's holding (`10_000` milli-units < `s0_hold_base = 39_915`)
    // (b) for 100 times total stock that exists (`100 * total_stock`)
    for (label, mult_100x) in [
        ("5a. Lowball bid @ 1 milli (< s0 holding: 10.000 u)", false),
        ("5b. Lowball bid @ 1 milli (100x total stock)", true),
    ] {
        let mut sp_lb = base_spec.clone();
        if mult_100x {
            // Ensure Seat 3 has enough liquid CRED to post 100x total stock @ 1 milli-CRED/unit;
            // compute baseline with identical Seat-3 starting CRED for exact bidder comparison.
            sp_lb.seats[3].initial_cred = 50_000;
        }
        let mut eng_lb_base = TurnEngine::new(sp_lb.clone()).must("lb base");
        for t in 1..=24_u8 {
            if step_t(&mut eng_lb_base, t).season_closed {
                break;
            }
        }
        let b_lb_base = eng_lb_base.finalize_season().must("fin lb base");
        let s0_lb_base = seat_eq(&b_lb_base, 0);
        let s3_lb_base = seat_eq(&b_lb_base, 3);

        let mut eng_lb = TurnEngine::new(sp_lb).must("lb eng");
        for t in 1..=24_u8 {
            if t == 24 {
                let stock = total_good_stock(&eng_lb, AssetId::Raw(0));
                let amt = if mult_100x { stock * 100 } else { 10_000 };
                assert!(
                    !mult_100x && amt < eng_lb.state().seats[0].raw[0] || mult_100x,
                    "5a bid must be strictly below Seat 0's holding"
                );
                eng_lb
                    .apply_market_action(MarketAction::PostOffer {
                        seat: 3,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(0),
                            amount: amt,
                            price_per_unit_cred: 1,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("post lowball bid");
            }
            if step_t(&mut eng_lb, t).season_closed {
                break;
            }
        }
        let b_lb = eng_lb.finalize_season().must("fin lb");
        let s0_eq = seat_eq(&b_lb, 0);
        let s3_eq = seat_eq(&b_lb, 3);
        println!(
            "{label:<52} | seat=0 | eq={s0_eq:>8} | base={s0_lb_base:>8} | delta={:+} (bidder s3: {s3_eq} vs {s3_lb_base}, delta={:+})",
            s0_eq - s0_lb_base,
            s3_eq - s3_lb_base
        );
        assert_eq!(
            s0_eq, s0_lb_base,
            "{label}: Seat 0 equity must equal baseline"
        );
        assert_eq!(
            s3_eq, s3_lb_base,
            "{label}: bidder equity must equal baseline"
        );
    }

    // 6. Honest low bid: bid at 40% of base value (`700` milli-CRED/unit vs `1_750` base) for part
    //    of Seat 0's holding (`10_000` milli-units < `s0_hold_base`).
    {
        let mut eng = TurnEngine::new(base_spec.clone()).must("honest low eng");
        let bid_40pct = (parity_raw0 * 40) / 100;
        assert_eq!(bid_40pct, 700);
        for t in 1..=24_u8 {
            if t == 24 {
                eng.apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 10_000,
                        price_per_unit_cred: bid_40pct,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("honest 40% bid");
            }
            if step_t(&mut eng, t).season_closed {
                break;
            }
        }
        let b = eng.finalize_season().must("fin honest low");
        let s0_eq = seat_eq(&b, 0);
        let s1_eq = seat_eq(&b, 1);
        println!(
            "{:<52} | seat=0 | eq={s0_eq:>8} | base={s0_base_eq:>8} | delta={:+} (bidder s1 delta={:+})",
            "6. Honest low bid (10.000 u @ 40% base = 700)",
            s0_eq - s0_base_eq,
            s1_eq - s1_base_eq
        );
        assert_eq!(
            s0_eq, s0_base_eq,
            "Honest 40% bid: Seat 0 equity must equal baseline"
        );
        assert_eq!(
            s1_eq, s1_base_eq,
            "Honest 40% bid: bidder equity must equal baseline"
        );
    }

    // 7a & 7b. High bid above base value (`#54` prior-turn resting + whole-unit rule):
    // (a) Closing-turn (`t == 24`) high bids (`0.001` u @ `15,000,000` and `10.000` u @ `3,000`)
    //     give zero close-mark lift (`lift_7a == 0`, `units_marked_by_bids == 0`).
    // (b) Earlier resting (`t == 23`, `created_turn = 23 < 24`) `10.000` u @ `3,000` (`locked_cred = 30,000`)
    //     lifts by `+21_250`.
    {
        // 7a: Turn-24 closing-turn bids (`0.001` unit AND `10.000` units) give 0 lift.
        let mut sp_7a_whole = base_spec.clone();
        sp_7a_whole.seats[1].initial_cred = 35_000;
        let mut eng_7a = TurnEngine::new(base_spec.clone()).must("7a eng");
        let mut eng_7a_whole = TurnEngine::new(sp_7a_whole).must("7a whole eng");
        for t in 1..=24_u8 {
            if t == 24 {
                eng_7a
                    .apply_market_action(MarketAction::PostOffer {
                        seat: 1,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(0),
                            amount: 1,
                            price_per_unit_cred: 15_000_000,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("7a high bid");
                eng_7a_whole
                    .apply_market_action(MarketAction::PostOffer {
                        seat: 1,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(0),
                            amount: 10_000,
                            price_per_unit_cred: 3_000,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("7a whole closing bid");
            }
            if step_t(&mut eng_7a, t).season_closed {
                assert!(step_t(&mut eng_7a_whole, t).season_closed);
                break;
            }
            let _ = step_t(&mut eng_7a_whole, t);
        }
        let locked_7a = eng_7a
            .state()
            .offers
            .values()
            .next()
            .must("offer 7a")
            .locked_cred;
        assert_eq!(locked_7a, 15_000);
        let pred_7a = league_core::season_end_and_marks(eng_7a.state(), Seat(0)).must("pred 7a");
        let raw0_7a = pred_7a
            .holdings
            .iter()
            .find(|h| h.good == AssetId::Raw(0))
            .must("raw0 pred 7a");
        assert_eq!(raw0_7a.units_marked_by_bids, 0);
        assert_eq!(raw0_7a.marked_value, half_base(s0_hold_base, parity_raw0));
        let b_7a = eng_7a.finalize_season().must("fin 7a");
        let s0_eq_7a = seat_eq(&b_7a, 0);
        let lift_7a = s0_eq_7a - s0_base_eq;
        assert_eq!(lift_7a, 0);
        let pred_7a_w =
            league_core::season_end_and_marks(eng_7a_whole.state(), Seat(0)).must("pred 7a whole");
        let raw0_7a_w = pred_7a_w
            .holdings
            .iter()
            .find(|h| h.good == AssetId::Raw(0))
            .must("raw0 pred 7a whole");
        assert_eq!(raw0_7a_w.units_marked_by_bids, 0);
        assert_eq!(raw0_7a_w.marked_value, half_base(s0_hold_base, parity_raw0));
        println!(
            "{:<52} | seat=0 | eq={s0_eq_7a:>8} | base={s0_base_eq:>8} | delta={lift_7a:+} (locked_cred={locked_7a})",
            "7a. Closing-turn bids (0.001 & 10.000 u -> 0 lift)"
        );

        // 7b: Earlier resting (`created_turn = 23 < 24`) 10.000 units @ 3_000 (`locked_cred = 30_000`).
        let mut sp_7b = base_spec.clone();
        sp_7b.seats[1].initial_cred = 35_000;
        let mut eng_7b_base = TurnEngine::new(sp_7b.clone()).must("7b base");
        for t in 1..=24_u8 {
            if step_t(&mut eng_7b_base, t).season_closed {
                break;
            }
        }
        let s0_7b_base = seat_eq(&eng_7b_base.finalize_season().must("fin 7b base"), 0);

        let mut eng_7b = TurnEngine::new(sp_7b).must("7b eng");
        for t in 1..=24_u8 {
            if t == 23 {
                eng_7b
                    .apply_market_action(MarketAction::PostOffer {
                        seat: 1,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(0),
                            amount: 10_000,
                            price_per_unit_cred: 3_000,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("7b high bid");
            }
            if step_t(&mut eng_7b, t).season_closed {
                break;
            }
        }
        let locked_7b = eng_7b
            .state()
            .offers
            .values()
            .next()
            .must("offer 7b")
            .locked_cred;
        assert_eq!(locked_7b, 30_000);
        let b_7b = eng_7b.finalize_season().must("fin 7b");
        let s0_eq_7b = seat_eq(&b_7b, 0);
        let lift_7b = s0_eq_7b - s0_7b_base;
        // Exact formula: min(10.000 * 3_000, 30_000) - 10.000 * 875 = 30_000 - 8_750 = +21_250 <= 30_000.
        assert_eq!(lift_7b, 30_000 - half_base(10_000, parity_raw0));
        assert_eq!(lift_7b, 21_250);
        assert!(lift_7b <= locked_7b);
        println!(
            "{:<52} | seat=0 | eq={s0_eq_7b:>8} | base={s0_7b_base:>8} | delta={lift_7b:+} (locked_cred={locked_7b})",
            "7b. High bid (10.000 u @ 3,000, lock 30,000)"
        );

        // Must-2 locked_cred slice-floor check: when `locked_cred` cuts a slice below `take * 50% of base`,
        // the slice is still floored at `take * 50% of base` (never below).
        let mut st_cut = eng_7b.state().clone();
        let oid = *st_cut.offers.keys().next().must("oid");
        st_cut.offers.get_mut(&oid).must("mut offer").locked_cred = 1_000; // < 8_750 floor for 10.000 u
        let mark_cut_below = mark_good_holding_at_close(&st_cut, 0, AssetId::Raw(0), 240_000);
        assert_eq!(
            mark_cut_below, 210_000,
            "locked_cred cut below 50% base must floor at 210_000"
        );
        st_cut.offers.get_mut(&oid).must("mut offer").locked_cred = 20_000; // between 8_750 and 30_000
        let mark_cut_above = mark_good_holding_at_close(&st_cut, 0, AssetId::Raw(0), 240_000);
        assert_eq!(
            mark_cut_above - 210_000,
            20_000 - 8_750,
            "locked_cred cap above floor must lift by min(take*px, locked_cred) - take*50% base = 11_250"
        );
    }

    // 8. Pulling all bids on 239 units (`perishable_decay_bps = 0`, 1.000 unit sold on Turn 23 -> 239.000 units):
    //    239 units with no bids == 209_125 == pulling every bid on Turn 24.
    {
        let mut sp_239 = base_spec.clone();
        for s in &mut sp_239.seats {
            s.initial_cred = 50_000;
            s.perishable_decay_bps = 0;
        }
        let run_239 = |post_and_pull_bids: bool| -> (Units, Units) {
            let mut e = TurnEngine::new(sp_239.clone()).must("239 eng");
            let mut ids = Vec::new();
            for t in 1..=24_u8 {
                if t == 22 {
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
                    .must("239 t22 buy");
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
                    .must("239 t22 sell");
                }
                if t == 23 && post_and_pull_bids {
                    for (buyer, amt, px) in [(2_u8, 10_000, 1_800), (3_u8, 15_000, 1_600)] {
                        let next_id = e.state().next_offer_id;
                        e.apply_market_action(MarketAction::PostOffer {
                            seat: buyer,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(0),
                                amount: amt,
                                price_per_unit_cred: px,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        })
                        .must("post resting bid");
                        ids.push((buyer, next_id));
                    }
                }
                if t == 24 && post_and_pull_bids {
                    for &(buyer, oid) in &ids {
                        e.apply_market_action(MarketAction::CancelOffer {
                            seat: buyer,
                            offer_id: oid,
                        })
                        .must("cancel resting bid t24");
                    }
                }
                if step_t(&mut e, t).season_closed {
                    break;
                }
            }
            assert_eq!(e.state().seats[0].raw[0], 239_000);
            let mk = mark_good_holding_at_close(e.state(), 0, AssetId::Raw(0), 239_000);
            let eq = seat_eq(&e.finalize_season().must("fin 239"), 0);
            (mk, eq)
        };
        let (mk_nobids, eq_nobids) = run_239(false);
        let (mk_pulled, eq_pulled) = run_239(true);
        assert_eq!(mk_nobids, 209_125);
        assert_eq!(mk_pulled, 209_125);
        assert_eq!(eq_pulled, eq_nobids);
        println!(
            "{:<52} | seat=0 | eq={eq_pulled:>8} | base={eq_nobids:>8} | delta={:+} (mark={mk_pulled} == 0.5*1750*239)",
            "8. Pulling all bids on T24 (239.000 u)",
            eq_pulled - eq_nobids
        );
    }

    // 9. An earlier resting bid above the floor (`created_turn = 23 < 24`, `1_200 > 875`) for part
    //    of Seat 0's holding (`10.000` units): covered `10.000` units at `1_200` (`12_000`),
    //    remaining `s0_hold_base - 10_000` at `50%` base.
    {
        let mut eng = TurnEngine::new(base_spec).must("real bid eng");
        for t in 1..=24_u8 {
            if t == 23 {
                eng.apply_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 10_000,
                        price_per_unit_cred: 1_200,
                        domain_id: None,
                        is_hybrid: false,
                    },
                })
                .must("real bid 1200");
            }
            if step_t(&mut eng, t).season_closed {
                break;
            }
        }
        let mk = mark_good_holding_at_close(eng.state(), 0, AssetId::Raw(0), s0_hold_base);
        let expected_mk = 12_000 + half_base(s0_hold_base - 10_000, parity_raw0);
        assert_eq!(mk, expected_mk);
        let s0_eq = seat_eq(&eng.finalize_season().must("fin real bid"), 0);
        let lift = s0_eq - s0_base_eq;
        assert_eq!(lift, 12_000 - half_base(10_000, parity_raw0));
        assert_eq!(lift, 3_250);
        println!(
            "{:<52} | seat=0 | eq={s0_eq:>8} | base={s0_base_eq:>8} | delta={lift:+} (10.000 u @ 1,200 + rest @ 875)",
            "9. Real bid above floor (10.000 u @ 1,200)"
        );
    }
}

#[test]
fn a2_c1a_active_trading_seat_stays_human_while_truly_idle_seat_is_seized() {
    // 1. Exact Opus probe (`TableId(700)`, N=5, layout seed 9): Seat 0 is Human, sells 1.000 Raw(0)
    // to Seat 1 on Turns 2 and 3 (both clearing in Step 0 batch), and runs no converter.
    // Seat 0 must stay Human with consecutive_zero_production_turns == 0.
    let tid = TableId(700);
    let mut spec = TableSpec::standard_chiral_ring(tid, 5, 9).must("spec 700");
    assert_eq!(spec.seats[0].identity, SeatIdentity::Human);
    // Also make Seat 4 Human so we simultaneously verify a truly idle Human seat IS seized!
    spec.seats[4].identity = SeatIdentity::Human;
    let mut eng = TurnEngine::new(spec).must("eng 700");

    // Turn 1: seats start with CRED only (`types.rs:495`), so Turn 1 must NOT count as zero-production.
    let _ = commit_and_reveal_bundles(&mut eng, tid, 1, vec![SealedOrderBundle::default(); 5]);
    let r1 = eng.sweep_turn_seeded(9).must("sweep t1");
    assert_eq!(eng.state().seats[0].consecutive_zero_production_turns, 0);
    assert_eq!(eng.state().seats[4].consecutive_zero_production_turns, 0);
    assert!(!r1.converted_to_ai.contains(&0));
    assert!(!r1.converted_to_ai.contains(&4));

    // Turns 2 and 3: Seat 0 sells 1.000 Raw(0) to Seat 1; Seat 4 remains completely idle.
    for t in [2_u8, 3_u8] {
        eng.apply_market_action(MarketAction::PostOffer {
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
        .must("s0 sell");
        eng.apply_market_action(MarketAction::PostOffer {
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
        .must("s1 buy");

        let _ = commit_and_reveal_bundles(&mut eng, tid, t, vec![SealedOrderBundle::default(); 5]);
        let r = eng.sweep_turn_seeded(9).must("sweep t2/t3");
        assert_eq!(r.batch_trades_cleared, 1);
        assert_eq!(
            eng.state().seats[0].identity,
            SeatIdentity::Human,
            "A-2: actively trading Seat 0 must stay Human on Turn {t}"
        );
        assert_eq!(eng.state().seats[0].consecutive_zero_production_turns, 0);
        assert!(!r.converted_to_ai.contains(&0));
        assert!(!r.regent_acted_seats.contains(&0));
        // Truly idle Seat 4 accumulates idle count 1 on Turn 2 and 2 on Turn 3 (not yet 3).
        assert_eq!(
            eng.state().seats[4].consecutive_zero_production_turns,
            t - 1
        );
        assert_eq!(eng.state().seats[4].identity, SeatIdentity::Human);
    }

    // Turn 4: Seat 0 trades again (stays Human); truly idle Seat 4 hits its 3rd consecutive
    // post-Turn-1 idle turn (`Turns 2, 3, 4`) and IS taken over (`Human -> AiSyndicate`).
    eng.apply_market_action(MarketAction::PostOffer {
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
    .must("s0 sell t4");
    eng.apply_market_action(MarketAction::PostOffer {
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
    .must("s1 buy t4");
    let _ = commit_and_reveal_bundles(&mut eng, tid, 4, vec![SealedOrderBundle::default(); 5]);
    let r4 = eng.sweep_turn_seeded(9).must("sweep t4");
    assert_eq!(eng.state().seats[0].identity, SeatIdentity::Human);
    assert!(!r4.converted_to_ai.contains(&0));
    assert!(
        r4.converted_to_ai.contains(&4),
        "A-2: truly idle Human Seat 4 must be converted to AiSyndicate on Turn 4"
    );
    assert!(r4.regent_acted_seats.contains(&4));
    assert_eq!(eng.state().seats[4].identity, SeatIdentity::AiSyndicate);
}

#[test]
fn b7_final_standings_ties_break_by_beacon_not_seat_index() {
    let tid = TableId(707);
    let mut spec = TableSpec::standard_chiral_ring(tid, 4, 2).must("spec 707");
    spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
    spec.shocks = vec![ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 3,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 100,
    }];
    // Make Seats 0 and 1 economically symmetric and strictly richer than Seats 2 and 3.
    for (i, s) in spec.seats.iter_mut().enumerate() {
        s.initial_cred = if i < 2 { 500_000 } else { 100_000 };
        s.raw_yield_per_turn = 10_000;
        s.refined_project_value = 2_000;
        s.upkeep_amount = 1_800;
        s.perishable_decay_bps = 2_000;
        s.regent = None;
    }

    let mut beacon_0_wins = None;
    let mut beacon_1_wins = None;
    for cand in 1_u64..500 {
        let b = derive_turn_beacon(cand, tid, Turn(24));
        let k0 = standings_tie_key(b, tid, Turn(24), 0);
        let k1 = standings_tie_key(b, tid, Turn(24), 1);
        if k0 < k1 && beacon_0_wins.is_none() {
            beacon_0_wins = Some(b);
        } else if k1 < k0 && beacon_1_wins.is_none() {
            beacon_1_wins = Some(b);
        }
        if beacon_0_wins.is_some() && beacon_1_wins.is_some() {
            break;
        }
    }
    let b0 = beacon_0_wins.must("beacon where seat 0 wins tie");
    let b1 = beacon_1_wins.must("beacon where seat 1 wins tie");

    let run_with_closing_beacon = |close_beacon: [u8; 32]| -> FinalScoreboard {
        let mut eng = TurnEngine::new(spec.clone()).must("tie eng");
        for t in 1..=24_u8 {
            let _ =
                commit_and_reveal_bundles(&mut eng, tid, t, vec![SealedOrderBundle::default(); 4]);
            let b = if t == 24 {
                close_beacon
            } else {
                derive_turn_beacon(1, tid, Turn(t))
            };
            let r = eng.sweep_turn(b).must("sweep tie");
            if t == 24 {
                assert!(r.season_closed);
            }
        }
        eng.finalize_season().must("finalize tie")
    };

    let board_0 = run_with_closing_beacon(b0);
    let board_1 = run_with_closing_beacon(b1);

    assert_eq!(
        board_0.standings[0].total_equity,
        board_0.standings[1].total_equity
    );
    assert_eq!(
        board_1.standings[0].total_equity,
        board_1.standings[1].total_equity
    );
    assert_eq!(board_0.standings[0].seat, 0);
    assert_eq!(board_0.standings[1].seat, 1);
    assert_eq!(
        board_1.standings[0].seat, 1,
        "B-7: closing beacon must break FinalScoreboard ties so Seat 1 ranks #1 ahead of Seat 0"
    );
    assert_eq!(board_1.standings[1].seat, 0);
}

fn n4_generated_converted_table_spec(initial_cred: Units) -> TableSpec {
    use league_core::{
        ConverterRecipe, DEFAULT_TOWN_IMPORT_PREMIUM_BPS, DEFAULT_WORKS_STIPEND_PER_UNIT_CRED,
        ProjectRecipe, SeatConfig, graded_mandate_deck_for_mean_project_value,
    };

    let tid = TableId(804);
    let seats = vec![
        SeatConfig {
            seat: 0,
            identity: SeatIdentity::AiSyndicate,
            initial_cred,
            raw_yield_per_turn: 10_361,
            refined_project_value: 2_157,
            upkeep_good: 3,
            upkeep_amount: 1_705,
            initial_raw_tariff_bps: 1_550,
            initial_refined_tariff_bps: 1_623,
            perishable_decay_bps: 1_900,
            regent: None,
            converters: vec![
                ConverterRecipe {
                    owner: 0,
                    inputs: vec![(1, 2_400), (2, 1_843)],
                    output_good: 0,
                    output_amount: 3_266,
                    max_runs_per_turn: 2,
                    is_secondary: false,
                },
                ConverterRecipe {
                    owner: 0,
                    inputs: vec![(2, 1_246), (3, 3_722)],
                    output_good: 0,
                    output_amount: 2_755,
                    max_runs_per_turn: 5,
                    is_secondary: true,
                },
            ],
        },
        SeatConfig {
            seat: 1,
            identity: SeatIdentity::AiSyndicate,
            initial_cred,
            raw_yield_per_turn: 10_289,
            refined_project_value: 2_170,
            upkeep_good: 0,
            upkeep_amount: 1_692,
            initial_raw_tariff_bps: 1_562,
            initial_refined_tariff_bps: 1_615,
            perishable_decay_bps: 2_259,
            regent: None,
            converters: vec![
                ConverterRecipe {
                    owner: 1,
                    inputs: vec![(2, 1_253), (3, 2_889)],
                    output_good: 1,
                    output_amount: 3_191,
                    max_runs_per_turn: 2,
                    is_secondary: false,
                },
                ConverterRecipe {
                    owner: 1,
                    inputs: vec![(3, 1_557), (0, 3_355)],
                    output_good: 1,
                    output_amount: 2_727,
                    max_runs_per_turn: 5,
                    is_secondary: true,
                },
            ],
        },
        SeatConfig {
            seat: 2,
            identity: SeatIdentity::AiSyndicate,
            initial_cred,
            raw_yield_per_turn: 10_320,
            refined_project_value: 2_162,
            upkeep_good: 1,
            upkeep_amount: 1_695,
            initial_raw_tariff_bps: 1_618,
            initial_refined_tariff_bps: 1_985,
            perishable_decay_bps: 2_304,
            regent: None,
            converters: vec![
                ConverterRecipe {
                    owner: 2,
                    inputs: vec![(3, 1_866), (0, 2_366)],
                    output_good: 2,
                    output_amount: 3_256,
                    max_runs_per_turn: 2,
                    is_secondary: false,
                },
                ConverterRecipe {
                    owner: 2,
                    inputs: vec![(0, 1_433), (1, 3_505)],
                    output_good: 2,
                    output_amount: 2_729,
                    max_runs_per_turn: 5,
                    is_secondary: true,
                },
            ],
        },
        SeatConfig {
            seat: 3,
            identity: SeatIdentity::AiSyndicate,
            initial_cred,
            raw_yield_per_turn: 10_443,
            refined_project_value: 2_159,
            upkeep_good: 2,
            upkeep_amount: 1_698,
            initial_raw_tariff_bps: 1_615,
            initial_refined_tariff_bps: 1_845,
            perishable_decay_bps: 2_418,
            regent: None,
            converters: vec![
                ConverterRecipe {
                    owner: 3,
                    inputs: vec![(0, 2_574), (1, 1_598)],
                    output_good: 3,
                    output_amount: 3_221,
                    max_runs_per_turn: 2,
                    is_secondary: false,
                },
                ConverterRecipe {
                    owner: 3,
                    inputs: vec![(1, 1_825), (2, 3_040)],
                    output_good: 3,
                    output_amount: 2_694,
                    max_runs_per_turn: 5,
                    is_secondary: true,
                },
            ],
        },
    ];
    let projects = vec![
        ProjectRecipe {
            id: 1,
            inputs: vec![(0, 1_000), (1, 1_000)],
            equity_points: 4_543,
        },
        ProjectRecipe {
            id: 2,
            inputs: vec![(1, 1_000), (2, 1_000)],
            equity_points: 4_549,
        },
        ProjectRecipe {
            id: 3,
            inputs: vec![(2, 1_000), (3, 1_000)],
            equity_points: 4_537,
        },
        ProjectRecipe {
            id: 4,
            inputs: vec![(3, 1_000), (0, 1_000)],
            equity_points: 4_532,
        },
        ProjectRecipe {
            id: 100,
            inputs: vec![(0, 1_000), (1, 1_000), (2, 1_000), (3, 1_000)],
            equity_points: 9_945,
        },
    ];
    let mean_proj = (seats.iter().map(|s| s.refined_project_value).sum::<Units>() + 2) / 4;
    assert_eq!(mean_proj, 2_162);
    let mandate_deck = graded_mandate_deck_for_mean_project_value(4, mean_proj);
    let salts: Vec<RevealSalt> = (0..4).map(|s| RevealSalt([0x41 + s; 32])).collect();
    let (dealt_mandate_commits, _) =
        deal_graded_mandates_with_entropy(tid, 4, [0x77; 32], &salts, &mandate_deck)
            .must("deal 804");

    TableSpec {
        table_id: tid,
        mode: TableMode::Eval,
        seat_count: 4,
        seats,
        projects,
        shocks: vec![ScheduledShockSpec {
            hint_turn: 1,
            forecast_turn: 2,
            realize_turn: 3,
            target_seat: 3,
            kind: ShockKind::TariffHoliday,
            magnitude_bps: 100,
        }],
        closing_hazard_bps: [10_000; CLOSING_ROLL_WINDOW_LEN],
        mandate_deck,
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
fn pr40_fix3_pooled_floor_dust_wall_worst_slice_0_006_units_and_nonround_bids() {
    let denom = i128::from(UNIT_SCALE) * BPS_SCALE;

    // -------------------------------------------------------------------------
    // 1. Non-round holding (`39.917` units = `39_917` milli-units) with `3,000`
    //    buy offers of `0.001` unit at `1` milli-CRED/unit (`TableId(631)`):
    //    Seat 0 sells `200_083` of its `240_000` Raw(0) on Turn 22 so Seat 0
    //    holds exact `39_917` milli-units (`39.917` units) at Turn-24 close.
    //    Baseline mark: `floor(39_917 * 1_750 * 5_000 / 10_000_000) = 34_927`.
    // -------------------------------------------------------------------------
    {
        let tid = TableId(631);
        let mut sp = TableSpec::standard_chiral_ring(tid, 4, 5).must("spec 631");
        sp.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
        sp.shocks = vec![ScheduledShockSpec {
            hint_turn: 1,
            forecast_turn: 2,
            realize_turn: 3,
            target_seat: 3,
            kind: ShockKind::TariffHoliday,
            magnitude_bps: 100,
        }];
        for s in &mut sp.seats {
            s.initial_cred = 350_000;
            s.perishable_decay_bps = 0;
            s.regent = None;
        }
        let run_39917 = |dust_bids: usize| -> (Units, FinalScoreboard) {
            let mut eng = TurnEngine::new(sp.clone()).must("eng 631");
            for t in 1..=24_u8 {
                if t == 23 {
                    // Seat 0 holds `220_000` entering Turn 23, sells `200_083` (`-> 19_917`), and
                    // receives `+10_000` in T23 Step 4b + `+10_000` in T24 Step 4b = `39_917`.
                    eng.apply_market_action(MarketAction::PostOffer {
                        seat: 1,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: AssetId::Raw(0),
                            amount: 200_083,
                            price_per_unit_cred: 1_400,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("631 buy");
                    eng.apply_market_action(MarketAction::PostOffer {
                        seat: 0,
                        order: OfferProposal {
                            side: OfferSide::Sell,
                            good: AssetId::Raw(0),
                            amount: 200_083,
                            price_per_unit_cred: 1_400,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    })
                    .must("631 sell");
                }
                if t == 24 {
                    for _ in 0..dust_bids {
                        eng.apply_market_action(MarketAction::PostOffer {
                            seat: 3,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(0),
                                amount: 1,
                                price_per_unit_cred: 1,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        })
                        .must("631 dust bid");
                    }
                }
                if step_empty_turn(&mut eng, tid, t, 5) {
                    break;
                }
            }
            assert_eq!(eng.state().seats[0].raw[0], 39_917);
            let mk = mark_good_holding_at_close(eng.state(), 0, AssetId::Raw(0), 39_917);
            (mk, eng.finalize_season().must("fin 631"))
        };
        let (mk_base, b_base) = run_39917(0);
        let (mk_3k, b_3k) = run_39917(3_000);
        assert_eq!(mk_base, 34_927);
        assert_eq!(mk_3k, mk_base);
        let s0_base = b_base.standings.iter().find(|e| e.seat == 0).must("s0");
        let s0_3k = b_3k.standings.iter().find(|e| e.seat == 0).must("s0 3k");
        let s3_base = b_base.standings.iter().find(|e| e.seat == 3).must("s3");
        let s3_3k = b_3k.standings.iter().find(|e| e.seat == 3).must("s3 3k");
        assert_eq!(s0_3k.net_asset_equity, s0_base.net_asset_equity);
        assert_eq!(s0_3k.total_equity, s0_base.total_equity);
        assert_eq!(s3_3k.net_asset_equity, s3_base.net_asset_equity);
        assert_eq!(s3_3k.total_equity, s3_base.total_equity);
    }

    // -------------------------------------------------------------------------
    // 2 & 3 & 4a. Review's exact generated `N = 4` table (`TableId(804)`,
    //             `Raw(0)` parity = `1_664`, `s0_hold = 43_892` = `43.892` u):
    //   - (2) `15,000` offers of `0.001` u @ `1` milli-CRED on Turn 24:
    //         Seat 0 == baseline (`-92,382`, rank 1), bidder Seat 3 == baseline.
    //   - (3) Worst slice size over `0.001..=2.000` units (`1..=2_000` milli-units)
    //         at `parity = 1_664`: `6` milli-units (`0.006` units), where exact
    //         unrounded `50%` floor is `6 * 832 / 1_000 = 4.992` milli-CRED so
    //         the old per-slice floor (`4`) lost `0.992` milli-CRED per slice
    //         (`9_920_000 / 10_000_000`, `1,984` milli-CRED across `2,000` offers).
    //         Posting `2,000` offers of `0.006` units at `831` milli-CRED/unit
    //         (just under `50%` of base = `832`): Seat 0 == baseline, bidder == baseline.
    //   - (4a) Non-round lowball half-holding (`21.947` units = `21_947` milli-units
    //          @ `1` milli-CRED) and non-round honest `40%`-of-base bid (`17.557`
    //          units = `17_557` milli-units @ `665` milli-CRED, where both slices
    //          lost `-1` milli-CRED under Fix 2's per-slice rounding):
    //          Seat 0 == baseline, bidder == baseline.
    // -------------------------------------------------------------------------
    {
        let spec_804 = n4_generated_converted_table_spec(15_000);
        let tid = spec_804.table_id;
        let step_804 = |eng: &mut TurnEngine, t: u8| -> bool {
            eng.advance_to_commit().must("adv commit 804");
            let mut revs = Vec::with_capacity(4);
            for s in 0..4_u8 {
                let rev = OrderReveal {
                    bundle: SealedOrderBundle::default(),
                    salt: RevealSalt([t.wrapping_add(s).max(1); 32]),
                };
                let c = compute_order_commitment(tid, Turn(t), Seat(s), &rev).must("commit 804");
                eng.submit_order_commitment(Seat(s), c)
                    .must("sub commit 804");
                revs.push((s, rev));
            }
            eng.advance_to_reveal().must("adv reveal 804");
            for (s, rev) in revs {
                eng.submit_order_reveal(Seat(s), rev).must("sub reveal 804");
            }
            eng.sweep_turn(derive_turn_beacon(42, tid, Turn(t)))
                .must("sweep 804")
                .season_closed
        };

        let run_804_with_t24_bids =
            |offers: &[(u8, Units, Units)]| -> (Units, Units, FinalScoreboard) {
                let mut eng = TurnEngine::new(spec_804.clone()).must("eng 804");
                for t in 1..=24_u8 {
                    if t == 24 {
                        for &(maker, amount, price_per_unit_cred) in offers {
                            eng.apply_market_action(MarketAction::PostOffer {
                                seat: maker,
                                order: OfferProposal {
                                    side: OfferSide::Buy,
                                    good: AssetId::Raw(0),
                                    amount,
                                    price_per_unit_cred,
                                    domain_id: None,
                                    is_hybrid: false,
                                },
                            })
                            .must("post offer 804");
                        }
                    }
                    if step_804(&mut eng, t) {
                        break;
                    }
                }
                let parity = eng.state().parities.asset_parity(AssetId::Raw(0));
                let hold = eng.state().seats[0].raw[0];
                let mk = mark_good_holding_at_close(eng.state(), 0, AssetId::Raw(0), hold);
                (parity, mk, eng.finalize_season().must("fin 804"))
            };

        let (parity_804, base_mk_804, board_base_804) = run_804_with_t24_bids(&[]);
        assert_eq!(parity_804, 1_664);
        assert_eq!(base_mk_804, 36_518);
        assert_eq!(
            board_base_804.standings[0].seat, 0,
            "Seat 0 starts at rank 1"
        );
        let s0_base = board_base_804
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .must("s0 base");
        let s3_base = board_base_804
            .standings
            .iter()
            .find(|e| e.seat == 3)
            .must("s3 base");
        assert_eq!(s0_base.net_asset_equity, -92_382);

        // (2) 15,000 offers of 0.001 unit @ 1 milli-CRED/unit from Seat 3.
        let dust_15k: Vec<(u8, Units, Units)> = vec![(3, 1, 1); 15_000];
        let (_, mk_15k, board_15k) = run_804_with_t24_bids(&dust_15k);
        assert_eq!(mk_15k, base_mk_804);
        assert_eq!(board_15k.standings[0].seat, 0, "Seat 0 must remain rank 1");
        let s0_15k = board_15k
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .must("s0 15k");
        let s3_15k = board_15k
            .standings
            .iter()
            .find(|e| e.seat == 3)
            .must("s3 15k");
        assert_eq!(s0_15k.net_asset_equity, s0_base.net_asset_equity);
        assert_eq!(s0_15k.total_equity, s0_base.total_equity);
        assert_eq!(s3_15k.net_asset_equity, s3_base.net_asset_equity);
        assert_eq!(s3_15k.total_equity, s3_base.total_equity);

        // (3) Find worst slice size in `0.001..=2.000` units (`1..=2_000` milli-units)
        //     for `parity = 1_664` under the old per-slice floor:
        //     `take = 6` (`0.006` units) loses `9_920_000 / 10_000_000 = 0.992` milli-CRED/slice.
        let mut worst_take: Units = 1;
        let mut max_rem_num: i128 = -1;
        for take in 1_i64..=2_000 {
            let rem_num =
                (i128::from(take) * i128::from(parity_804) * i128::from(UNSOLD_EXCESS_PARITY_BPS))
                    % denom;
            if rem_num > max_rem_num {
                max_rem_num = rem_num;
                worst_take = take;
            }
        }
        assert_eq!(
            worst_take, 6,
            "worst slice at parity 1,664 is 0.006 units (6 milli-units)"
        );
        assert_eq!(
            max_rem_num, 9_920_000,
            "old code lost 0.992 milli-CRED per 0.006-unit slice"
        );
        let half_base_px_804 = (parity_804 * UNSOLD_EXCESS_PARITY_BPS as i64) / 10_000;
        assert_eq!(half_base_px_804, 832);
        let just_under_half_px = half_base_px_804 - 1; // 831 milli-CRED/unit
        let worst_2k: Vec<(u8, Units, Units)> = vec![(3, worst_take, just_under_half_px); 2_000];
        let (_, mk_worst, board_worst) = run_804_with_t24_bids(&worst_2k);
        assert_eq!(mk_worst, base_mk_804);
        let s0_w = board_worst
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .must("s0 w");
        let s3_w = board_worst
            .standings
            .iter()
            .find(|e| e.seat == 3)
            .must("s3 w");
        assert_eq!(s0_w.net_asset_equity, s0_base.net_asset_equity);
        assert_eq!(s0_w.total_equity, s0_base.total_equity);
        assert_eq!(s3_w.net_asset_equity, s3_base.net_asset_equity);
        assert_eq!(s3_w.total_equity, s3_base.total_equity);

        // (4a) Non-round half-holding lowball (`21_947` milli-units @ `1`) and
        //      non-round honest `40%`-of-base (`17_557` milli-units @ `665`) on `TableId(804)`.
        //      Note: `(21_947 * 832) % 1_000 = 904 > 144` and `(17_557 * 832) % 1_000 = 424 > 144`,
        //      so BOTH lost `-1` milli-CRED under Fix 2's per-slice rounding!
        for (amt, px) in [(21_947_i64, 1_i64), (17_557_i64, (parity_804 * 40) / 100)] {
            let (_, mk_nr, b_nr) = run_804_with_t24_bids(&[(3, amt, px)]);
            assert_eq!(mk_nr, base_mk_804);
            let s0_nr = b_nr.standings.iter().find(|e| e.seat == 0).must("s0 nr");
            let s3_nr = b_nr.standings.iter().find(|e| e.seat == 3).must("s3 nr");
            assert_eq!(s0_nr.net_asset_equity, s0_base.net_asset_equity);
            assert_eq!(s0_nr.total_equity, s0_base.total_equity);
            assert_eq!(s3_nr.net_asset_equity, s3_base.net_asset_equity);
            assert_eq!(s3_nr.total_equity, s3_base.total_equity);
        }
    }

    // -------------------------------------------------------------------------
    // 4b, 5, & 6 on `TableId(620)` (`parity = 1_750`, non-round `s0_hold = 39_812`,
    // `base_mark = floor(39_812 * 875 / 1_000) = 34_835`, `s0_base_eq = -90_864`):
    // -------------------------------------------------------------------------
    {
        let tid = TableId(620);
        let mut base_spec = TableSpec::standard_chiral_ring(tid, 4, 5).must("spec 620 fix3");
        base_spec.closing_hazard_bps = [10_000; CLOSING_ROLL_WINDOW_LEN];
        base_spec.shocks = vec![ScheduledShockSpec {
            hint_turn: 1,
            forecast_turn: 2,
            realize_turn: 3,
            target_seat: 3,
            kind: ShockKind::TariffHoliday,
            magnitude_bps: 100,
        }];
        for s in &mut base_spec.seats {
            s.initial_cred = 15_000;
            s.regent = None;
        }
        let run_620_turns =
            |offers_by_turn: &[(u8, u8, Units, Units)]| -> (Units, Units, FinalScoreboard) {
                let mut eng = TurnEngine::new(base_spec.clone()).must("eng 620 fix3");
                for t in 1..=24_u8 {
                    for &(post_turn, maker, amount, price_per_unit_cred) in offers_by_turn {
                        if t == post_turn {
                            eng.apply_market_action(MarketAction::PostOffer {
                                seat: maker,
                                order: OfferProposal {
                                    side: OfferSide::Buy,
                                    good: AssetId::Raw(0),
                                    amount,
                                    price_per_unit_cred,
                                    domain_id: None,
                                    is_hybrid: false,
                                },
                            })
                            .must("post 620 fix3");
                        }
                    }
                    let _ = commit_and_reveal_bundles(
                        &mut eng,
                        tid,
                        t,
                        vec![SealedOrderBundle::default(); 4],
                    );
                    if eng
                        .sweep_turn(derive_turn_beacon(5, tid, Turn(t)))
                        .must("sweep 620 fix3")
                        .season_closed
                    {
                        break;
                    }
                }
                let hold = eng.state().seats[0].raw[0];
                let mk = mark_good_holding_at_close(eng.state(), 0, AssetId::Raw(0), hold);
                (hold, mk, eng.finalize_season().must("fin 620 fix3"))
            };
        let run_620 = |offers: &[(u8, Units, Units)]| -> (Units, Units, FinalScoreboard) {
            let timed: Vec<(u8, u8, Units, Units)> =
                offers.iter().map(|&(m, a, p)| (23, m, a, p)).collect();
            run_620_turns(&timed)
        };

        let (hold_base, mk_base, b_base) = run_620(&[]);
        assert_eq!(hold_base, 39_812);
        assert_eq!(mk_base, 34_835);
        let s0_base = b_base.standings.iter().find(|e| e.seat == 0).must("s0");
        let s1_base = b_base.standings.iter().find(|e| e.seat == 1).must("s1");
        let s3_base = b_base.standings.iter().find(|e| e.seat == 3).must("s3");
        assert_eq!(s0_base.net_asset_equity, -90_864);

        // (4b) Non-round half-holding lowball (`19.907` units = `19_907` milli-units @ `1`)
        //      and non-round honest `40%`-of-base (`15.927` units = `15_927` milli-units @ `700`):
        //      both had `(take * 875) % 1_000 == 625 > 500` (losing `-1` milli-CRED under Fix 2).
        for (amt, px) in [(19_907_i64, 1_i64), (15_927_i64, 700_i64)] {
            let (_, mk_nr, b_nr) = run_620(&[(3, amt, px)]);
            assert_eq!(mk_nr, mk_base);
            let s0_nr = b_nr.standings.iter().find(|e| e.seat == 0).must("s0 nr");
            let s3_nr = b_nr.standings.iter().find(|e| e.seat == 3).must("s3 nr");
            assert_eq!(s0_nr.net_asset_equity, s0_base.net_asset_equity);
            assert_eq!(s3_nr.net_asset_equity, s3_base.net_asset_equity);
        }

        // (5) Mix: 1 bid above the floor for non-round `7.333` units (`7_333` milli-units)
        //     @ `1_200` milli-CRED/unit PLUS `1,000` dust bids of `0.001` unit @ `1` milli-CRED:
        //     - Winning bid slice (`take = 7_333`):
        //         `bid_cred = floor(7_333 * 1_200 / 1_000) = 8_799` milli-CRED
        //         (`8_799 * 10_000_000 = 87_990_000_000 > 7_333 * 8_750_000 = 64_163_750_000`).
        //     - `1,000` dust bids (`1_000 * 1` milli-unit): `bid_cred = 0` -> `floored_units = 1_000`.
        //     - Uncovered remainder: `rem_units = 39_812 - 7_333 - 1_000 = 31_479`.
        //     - Pooled floor over all other units (`1_000 + 31_479 = 32_479` milli-units):
        //         `floor(32_479 * 1_750 * 5_000 / 10_000_000) = floor(28_419_125 / 1_000) = 28_419`.
        //     - Exact total mark = `8_799 + 28_419 = 37_218` (`+2_383` over `34_835` baseline,
        //       so `s0_eq = -90_864 + 2_383 = -88_481`; Fix 2 per-slice gave `36_343`, `-875` lower).
        let mut mix_offers = Vec::with_capacity(1_001);
        mix_offers.push((1_u8, 7_333_i64, 1_200_i64));
        mix_offers.extend(std::iter::repeat_n((3_u8, 1_i64, 1_i64), 1_000));
        let (_, mk_mix, b_mix) = run_620(&mix_offers);
        assert_eq!(mk_mix, 37_218);
        let s0_mix = b_mix.standings.iter().find(|e| e.seat == 0).must("s0 mix");
        let s3_mix = b_mix.standings.iter().find(|e| e.seat == 3).must("s3 mix");
        assert_eq!(s0_mix.net_asset_equity, -88_481);
        assert_eq!(s0_mix.net_asset_equity - s0_base.net_asset_equity, 2_383);
        assert_eq!(s3_mix.net_asset_equity, s3_base.net_asset_equity);

        // (6) Exact unrounded-floor tie: `take = 8` milli-units (`0.008` units) @ `price = 875`:
        //     `bid_cred = floor(8 * 875 / 1_000) = 7` (`locked_cred = 7`), so:
        //       LHS = `7 * 1_000 * 10_000 = 70_000_000`
        //       RHS = `8 * 1_750 * 5_000  = 70_000_000` (`LHS == RHS`!).
        //     Because comparison is strict `>`, it counts as floored (`floored_units += 8`),
        //     and total mark (`34_835`) and equity (`-90_864`) equal baseline!
        let lhs_tie = 7_i128 * denom;
        let rhs_tie = 8_i128 * 1_750 * i128::from(UNSOLD_EXCESS_PARITY_BPS);
        assert_eq!(lhs_tie, rhs_tie);
        let (_, mk_tie, b_tie) = run_620(&[(1, 8, 875)]);
        assert_eq!(mk_tie, mk_base);
        let s0_tie = b_tie.standings.iter().find(|e| e.seat == 0).must("s0 tie");
        let s1_tie = b_tie.standings.iter().find(|e| e.seat == 1).must("s1 tie");
        assert_eq!(s0_tie.net_asset_equity, s0_base.net_asset_equity);
        assert_eq!(s0_tie.total_equity, s0_base.total_equity);
        assert_eq!(s1_tie.net_asset_equity, s1_base.net_asset_equity);
    }
}

#[test]
fn close_marks_turn_32_sub_unit_vs_whole_unit_bids_issue_54() {
    let tid = TableId(854);
    let mut base_spec = TableSpec::standard_chiral_ring(tid, 4, 5).must("spec 854");
    base_spec.closing_hazard_bps = [0; CLOSING_ROLL_WINDOW_LEN];
    base_spec.shocks = vec![ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 3,
        kind: ShockKind::TariffHoliday,
        magnitude_bps: 100,
    }];
    for s in &mut base_spec.seats {
        s.initial_cred = 25_000;
        s.regent = None;
    }

    // `bids`: `(post_turn, maker, amount, price_per_unit_cred)` posted before Turn-32 Sweep.
    let run_turn32 =
        |bids: &[(u8, u8, Units, Units)]| -> (Units, Units, Units, Units, FinalScoreboard) {
            let mut eng = TurnEngine::new(base_spec.clone()).must("eng 854");
            for t in 1..=HARD_TURN_CAP {
                for &(post_turn, maker, amount, price_per_unit_cred) in bids {
                    if t == post_turn {
                        eng.apply_market_action(MarketAction::PostOffer {
                            seat: maker,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(0),
                                amount,
                                price_per_unit_cred,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        })
                        .must("post t31/t32 bid");
                    }
                }
                let closed = step_empty_turn(&mut eng, tid, t, 5);
                if t < HARD_TURN_CAP {
                    assert!(!closed, "season must stay open until Turn 32");
                } else {
                    assert!(closed, "Turn 32 hard cap must close season");
                }
            }
            let parity = eng.state().parities.asset_parity(AssetId::Raw(0));
            let hold = eng.state().seats[0].raw[0];
            let pred = league_core::season_end_and_marks(eng.state(), Seat(0)).must("pred 854");
            let h = pred
                .holdings
                .iter()
                .find(|m| m.good == AssetId::Raw(0))
                .must("raw0 holding");
            let board = eng.finalize_season().must("fin 854");
            (parity, hold, h.units_marked_by_bids, h.marked_value, board)
        };

    let (parity, hold, marked_base, val_base, board_base) = run_turn32(&[]);
    assert_eq!(board_base.closed_turn, HARD_TURN_CAP);
    assert_eq!(marked_base, 0);
    let s0_base = board_base.standings.iter().find(|e| e.seat == 0).must("s0");
    let expected_floor =
        ((i128::from(hold) * i128::from(parity) * i128::from(UNSOLD_EXCESS_PARITY_BPS))
            / (i128::from(UNIT_SCALE) * BPS_SCALE)) as Units;
    assert_eq!(val_base, expected_floor);

    // 1. High closing-turn (`Turn 32`) bids — both `0.001` u (`15_000_000`) AND whole-unit
    //    (`1.000` u @ `15_000` and `10.000` u @ `2_500`) — must never move the close mark.
    for (label, bid) in [
        ("t32 0.001u high", (32, 1, 1, 15_000_000)),
        ("t32 1.000u high", (32, 1, UNIT_SCALE, 15_000)),
        ("t32 10.000u high", (32, 1, 10 * UNIT_SCALE, 2_500)),
    ] {
        let (_, _, marked, val, board) = run_turn32(&[bid]);
        assert_eq!(marked, 0, "{label}");
        assert_eq!(val, val_base, "{label}");
        let s0 = board.standings.iter().find(|e| e.seat == 0).must("s0");
        assert_eq!(s0.net_asset_equity, s0_base.net_asset_equity, "{label}");
        assert_eq!(s0.total_equity, s0_base.total_equity, "{label}");
    }

    // 2. Earlier resting (`Turn 31 < 32`) sub-unit bids (`0.001` u, `0.999` u, and split `1 + 999`)
    //    must still leave `units_marked_by_bids == 0`.
    let (_, _, marked_31_001, val_31_001, _) = run_turn32(&[(31, 1, 1, 15_000_000)]);
    assert_eq!(marked_31_001, 0);
    assert_eq!(val_31_001, val_base);
    let (_, _, marked_999, val_999, board_999) = run_turn32(&[(31, 1, UNIT_SCALE - 1, 15_000)]);
    assert_eq!(marked_999, 0);
    assert_eq!(val_999, val_base);
    assert_eq!(
        board_999
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .must("s0 999")
            .net_asset_equity,
        s0_base.net_asset_equity
    );
    let (_, _, marked_split, val_split, _) =
        run_turn32(&[(31, 1, 1, 14_000_000), (31, 2, UNIT_SCALE - 1, 2_000)]);
    assert_eq!(marked_split, 0);
    assert_eq!(val_split, val_base);

    // 3. Earlier resting (`Turn 31`) `1.000`-unit (`UNIT_SCALE`) high bid (`15_000`, `locked_cred = 15_000`):
    //    marks `1.000` u.
    let (_, _, marked_1000, val_1000, board_1000) = run_turn32(&[(31, 1, UNIT_SCALE, 15_000)]);
    let rest_floor = ((i128::from(hold - UNIT_SCALE)
        * i128::from(parity)
        * i128::from(UNSOLD_EXCESS_PARITY_BPS))
        / (i128::from(UNIT_SCALE) * BPS_SCALE)) as Units;
    assert_eq!(marked_1000, UNIT_SCALE);
    assert_eq!(val_1000, 15_000 + rest_floor);
    let s0_1000 = board_1000
        .standings
        .iter()
        .find(|e| e.seat == 0)
        .must("s0 1000");
    assert_eq!(
        s0_1000.net_asset_equity - s0_base.net_asset_equity,
        val_1000 - val_base
    );

    // 4. Neither an earlier sub-unit high bid (`t=31`, `500` @ `20_000`) nor a closing-turn whole-unit
    //    high bid (`t=32`, `UNIT_SCALE` @ `25_000`) blocks/shadows an earlier `1.000`-unit bid (`t=31`, `UNIT_SCALE` @ `3_000`).
    let (_, _, marked_mix, val_mix, _) = run_turn32(&[
        (31, 2, 500, 20_000),
        (32, 3, UNIT_SCALE, 25_000),
        (31, 1, UNIT_SCALE, 3_000),
    ]);
    assert_eq!(marked_mix, UNIT_SCALE);
    assert_eq!(val_mix, 3_000 + rest_floor);
}
