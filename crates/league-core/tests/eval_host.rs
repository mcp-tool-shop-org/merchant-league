//! Integration tests for [`EvalHost`] (Wave 2 Card 10 Part A, Items 1–5).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use league_core::{
    AssetId, ClubBot, ClubHost, ConverterRunOrder, DrandSchedule, EVAL_SAVE_FORMAT_VERSION,
    EVAL_SEAL_FORMAT_VERSION, EvalActSubmission, EvalHost, EvalSaveSnapshot, HARD_TURN_CAP,
    MarketAction, OfferProposal, OfferSide, PreimageSecret, RevealSalt, SealedEvalSeed,
    SealedOrderBundle, SeasonReplayLog, TableId, TableMode, TableSpec, TurnPhase,
    compute_preimage_condition, deal_graded_mandates_with_entropy, replay_eval_season,
};

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn unique_tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ml_eval_{label}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    must_ok(fs::create_dir_all(&dir), "create tmp dir");
    dir
}

fn collect_bot_submissions(host: &EvalHost, bots: &[ClubBot]) -> BTreeMap<u8, EvalActSubmission> {
    let mut subs = BTreeMap::new();
    for (s_idx, bot) in bots.iter().enumerate().take(usize::from(host.seat_count())) {
        let obs = must_ok(
            host.observe_seat(league_core::Seat(s_idx as u8)),
            "observe_seat",
        );
        let mandate = obs
            .own_mandate()
            .copied()
            .unwrap_or_else(|| panic!("seat {s_idx} missing own_mandate"));
        let decision = bot.decide_full_turn(&obs, &mandate);
        subs.insert(
            s_idx as u8,
            EvalActSubmission {
                market_actions: decision.pass_a_actions,
                pass_b_actions: decision.pass_b_actions,
                bundle: decision.bundle,
            },
        );
    }
    subs
}

#[test]
fn eval_host_save_seal_round_trip_is_byte_exact_and_omits_raw_seed() {
    let tmp = unique_tmp_dir("save_seal_rt");
    let raw_eval_seed_bytes = [0x6D; 32];
    let raw_eval_seed_hex = "6d".repeat(32);
    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes(raw_eval_seed_bytes),
        "SealedEvalSeed::try_from_bytes",
    );
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31 + i; 32])).collect();
    let mut host = must_ok(
        EvalHost::from_explicit_setup(TableId(910_101), 4, 7, [0x4A; 32], &salts, eval_seed, 55),
        "EvalHost::from_explicit_setup",
    );

    // 1. Turn 0 round-trip byte-exact.
    let save0 = tmp.join("eval-910101-turn0.save.json");
    let seal0 = tmp.join("eval-910101-turn0.seal");
    must_ok(
        host.write_save_and_seal(&save0, &seal0),
        "write_save_and_seal turn 0",
    );
    let save0_bytes = must_ok(fs::read(&save0), "read save0");
    let seal0_bytes = must_ok(fs::read(&seal0), "read seal0");
    let loaded0 = must_ok(
        EvalHost::read_save_and_seal(&save0, &seal0),
        "read_save_and_seal turn 0",
    );
    let save0_rt = tmp.join("eval-910101-turn0-rt.save.json");
    let seal0_rt = tmp.join("eval-910101-turn0-rt.seal");
    must_ok(
        loaded0.write_save_and_seal(&save0_rt, &seal0_rt),
        "write_save_and_seal turn 0 rt",
    );
    assert_eq!(save0_bytes, must_ok(fs::read(&save0_rt), "read save0_rt"));
    assert_eq!(seal0_bytes, must_ok(fs::read(&seal0_rt), "read seal0_rt"));

    let save0_text = String::from_utf8_lossy(&save0_bytes);
    let seal0_text = String::from_utf8_lossy(&seal0_bytes);
    assert!(
        !save0_text.contains(&raw_eval_seed_hex),
        "raw eval seed leaked to .save.json"
    );
    assert!(
        !seal0_text.contains(&raw_eval_seed_hex),
        "raw eval seed leaked to .seal"
    );
    let save_json: EvalSaveSnapshot = must_ok(serde_json::from_slice(&save0_bytes), "parse save0");
    assert_eq!(save_json.format_version, EVAL_SAVE_FORMAT_VERSION);
    assert_eq!(save_json.spec.mode, TableMode::Eval);
    for forbidden_key in [
        "vrf_proofs",
        "sealed_mandate_deal",
        "turn_salts",
        "recorded_act_submissions",
        "bot_tie_seed",
        "dealt_mandate_commits",
    ] {
        assert!(
            !save0_text.contains(forbidden_key),
            "forbidden seal key `{forbidden_key}` appeared in .save.json"
        );
    }

    // 2. Advance two turns, save in Reveal phase (after apply_act_submissions) & check byte-exact round-trip.
    let bots: Vec<ClubBot> = (0..4).map(|i| ClubBot::new(100 + i as u64)).collect();
    for _ in 1..=2 {
        let subs = collect_bot_submissions(&host, &bots);
        must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
        let _ = must_ok(host.sweep_turn(), "sweep_turn");
    }
    let subs3 = collect_bot_submissions(&host, &bots);
    must_ok(host.apply_act_submissions(&subs3), "apply turn 3");
    assert_eq!(host.phase(), TurnPhase::Reveal);

    let save3 = tmp.join("eval-910101-turn3.save.json");
    let seal3 = tmp.join("eval-910101-turn3.seal");
    must_ok(
        host.write_save_and_seal(&save3, &seal3),
        "write_save_and_seal reveal",
    );
    let save3_bytes = must_ok(fs::read(&save3), "read save3");
    let seal3_bytes = must_ok(fs::read(&seal3), "read seal3");
    let loaded3 = must_ok(
        EvalHost::read_save_and_seal(&save3, &seal3),
        "read_save_and_seal reveal",
    );
    let save3_rt = tmp.join("eval-910101-turn3-rt.save.json");
    let seal3_rt = tmp.join("eval-910101-turn3-rt.seal");
    must_ok(
        loaded3.write_save_and_seal(&save3_rt, &seal3_rt),
        "write reveal rt",
    );
    assert_eq!(save3_bytes, must_ok(fs::read(&save3_rt), "read save3_rt"));
    assert_eq!(seal3_bytes, must_ok(fs::read(&seal3_rt), "read seal3_rt"));
    let seal3_val: serde_json::Value = must_ok(serde_json::from_slice(&seal3_bytes), "parse seal3");
    assert_eq!(
        seal3_val.get("format_version").and_then(|v| v.as_u64()),
        Some(u64::from(EVAL_SEAL_FORMAT_VERSION))
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn eval_host_pre_act_checkpoint_and_act_recorded_replay_produce_identical_receipt() {
    let tmp = unique_tmp_dir("ckpt_replay");
    let ckpt_dir = tmp.join("checkpoints");
    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes([0x6E; 32]),
        "SealedEvalSeed::try_from_bytes",
    );
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x41 + i; 32])).collect();
    let mut host = must_ok(
        EvalHost::from_explicit_setup(TableId(910_102), 4, 7, [0x4B; 32], &salts, eval_seed, 88),
        "EvalHost::from_explicit_setup",
    );
    let bots: Vec<ClubBot> = (0..4).map(|i| ClubBot::new(200 + i as u64)).collect();

    for day in 1..=3_u8 {
        assert_eq!(host.turn(), day);
        let (pre_save, pre_seal) = must_ok(
            host.write_pre_act_checkpoint(&ckpt_dir),
            "write_pre_act_checkpoint",
        );
        assert!(
            pre_save
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == format!("day-{day:02}-pre-act.save.json"))
        );
        assert!(
            pre_seal
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == format!("day-{day:02}-pre-act.seal"))
        );

        let mut subs = collect_bot_submissions(&host, &bots);
        if day == 1 {
            // Mix in explicit custom Pass A, Pass B, and bundle orders alongside ClubBot actions.
            if let Some(sub0) = subs.get_mut(&0) {
                sub0.market_actions.push(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 500,
                        price_per_unit_cred: 1_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                });
                sub0.pass_b_actions.push(MarketAction::TownImport {
                    seat: 0,
                    good: AssetId::Raw(1),
                    amount: 100,
                });
                if sub0.bundle.converter_runs.is_empty() {
                    sub0.bundle.converter_runs.push(ConverterRunOrder {
                        recipe_idx: 0,
                        runs: 1,
                    });
                }
            }
        }

        let orig_outcome = must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
        let rec_seal = must_ok(
            host.write_act_recorded_seal(&ckpt_dir),
            "write_act_recorded_seal",
        );
        assert!(
            rec_seal
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == format!("day-{day:02}-act-recorded.seal"))
        );
        let orig_receipt = must_ok(host.sweep_turn(), "sweep_turn original");

        // Replay strictly from pre-act checkpoint + act-recorded seal on a forked host.
        let mut forked = must_ok(
            EvalHost::fork_from_checkpoint(&pre_save, &pre_seal),
            "fork_from_checkpoint",
        );
        let replay_outcome = must_ok(
            forked.apply_recorded_act_submissions_from_seal(&rec_seal),
            "apply_recorded_act_submissions_from_seal",
        );
        assert_eq!(replay_outcome, orig_outcome);
        let replay_receipt = must_ok(forked.sweep_turn(), "sweep_turn replay");

        let orig_json = must_ok(serde_json::to_vec(&orig_receipt), "ser orig receipt");
        let replay_json = must_ok(serde_json::to_vec(&replay_receipt), "ser replay receipt");
        assert_eq!(
            replay_json, orig_json,
            "day {day} replayed receipt must be byte-identical to original"
        );
    }

    // Mismatched turn recorded seal is refused.
    let day1_save = ckpt_dir.join("day-01-pre-act.save.json");
    let day1_seal = ckpt_dir.join("day-01-pre-act.seal");
    let day2_rec = ckpt_dir.join("day-02-act-recorded.seal");
    let mut fork_day1 = must_ok(
        EvalHost::fork_from_checkpoint(&day1_save, &day1_seal),
        "fork day 1",
    );
    assert!(
        fork_day1
            .apply_recorded_act_submissions_from_seal(&day2_rec)
            .err()
            .is_some_and(|e| e.to_string().contains("RECORDED_SEAL_TURN_MISMATCH"))
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn eval_host_refuses_club_and_game_specs_and_seals() {
    let tmp = unique_tmp_dir("refuse_club");

    // 1. Refuse TableMode::Club spec in EvalHost::new.
    let (club_spec, club_deal, club_eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_setup(TableId(910_103), 4, 7),
        "standard_club_chiral_ring_with_setup",
    );
    let err = match EvalHost::new(club_spec, 7, club_deal, club_eval_seed, 1) {
        Ok(_) => panic!("EvalHost::new must refuse TableMode::Club"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("INVALID_TABLE_MODE"),
        "unexpected Club refusal: {err}"
    );

    // 2. Refuse TableMode::Game spec in EvalHost::new.
    let drand_sched = DrandSchedule {
        turn_1_close_unix: 1_700_000_000,
        turn_period_secs: 30,
    };
    let (mut game_spec, game_deal) = must_ok(
        TableSpec::standard_game_chiral_ring_with_mandates(TableId(910_104), 4, 7, drand_sched),
        "standard_game_chiral_ring_with_mandates",
    );
    let dummy_eval = must_ok(SealedEvalSeed::generate(), "SealedEvalSeed::generate");
    game_spec.eval_vrf_pubkey = Some(must_ok(dummy_eval.public_key_bytes(), "pk"));
    let game_err = match EvalHost::new(game_spec, 7, game_deal, dummy_eval, 1) {
        Ok(_) => panic!("EvalHost::new must refuse TableMode::Game"),
        Err(err) => err,
    };
    assert!(game_err.to_string().contains("INVALID_TABLE_MODE"));

    // 3. Refuse a real ClubHost save+seal pair.
    let club_host = must_ok(
        ClubHost::from_standard_setup(TableId(910_105), 4, 7, 1),
        "ClubHost::from_standard_setup",
    );
    let club_save = tmp.join("club.save.json");
    let club_seal = tmp.join("club.seal");
    must_ok(
        club_host.write_save_and_seal(&club_save, &club_seal),
        "ClubHost::write_save_and_seal",
    );
    assert!(EvalHost::read_save_and_seal(&club_save, &club_seal).is_err());
    assert!(EvalHost::fork_from_checkpoint(&club_save, &club_seal).is_err());

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn eval_host_tampered_save_returns_save_seal_snapshot_mismatch() {
    let tmp = unique_tmp_dir("tamper_save");
    let host = must_ok(
        EvalHost::from_standard_setup(TableId(910_106), 4, 7, 12),
        "EvalHost::from_standard_setup",
    );
    let save_path = tmp.join("eval-910106.save.json");
    let seal_path = tmp.join("eval-910106.seal");
    must_ok(
        host.write_save_and_seal(&save_path, &seal_path),
        "write_save_and_seal",
    );

    let mut tampered: serde_json::Value = must_ok(
        serde_json::from_slice(&must_ok(fs::read(&save_path), "read save")),
        "parse save",
    );
    tampered["seats"][0]["cred"] = serde_json::json!(999_999);
    must_ok(
        fs::write(
            &save_path,
            must_ok(serde_json::to_vec_pretty(&tampered), "ser tampered save"),
        ),
        "write tampered save",
    );

    let err = match EvalHost::read_save_and_seal(&save_path, &seal_path) {
        Ok(_) => panic!("tampered .save.json must fail SAVE_SEAL_SNAPSHOT_MISMATCH"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("SAVE_SEAL_SNAPSHOT_MISMATCH"),
        "expected SAVE_SEAL_SNAPSHOT_MISMATCH, got: {err}"
    );
    let fork_err = match EvalHost::fork_from_checkpoint(&save_path, &seal_path) {
        Ok(_) => panic!("tampered checkpoint must fail SAVE_SEAL_SNAPSHOT_MISMATCH"),
        Err(err) => err,
    };
    assert!(fork_err.to_string().contains("SAVE_SEAL_SNAPSHOT_MISMATCH"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn eval_host_turn_0_fork_deals_identical_mandates_and_runs_independently_to_eval_replay() {
    let tmp = unique_tmp_dir("turn0_fork");
    let source = must_ok(
        EvalHost::from_standard_setup(TableId(910_107), 4, 7, 33),
        "EvalHost::from_standard_setup",
    );
    let save0 = tmp.join("eval-910107.save.json");
    let seal0 = tmp.join("eval-910107.seal");
    must_ok(source.write_save_and_seal(&save0, &seal0), "write turn 0");

    let mut fork_a = must_ok(EvalHost::fork_from_checkpoint(&save0, &seal0), "fork_a");
    let mut fork_b = must_ok(EvalHost::fork_from_checkpoint(&save0, &seal0), "fork_b");

    // Identical Turn-1 mandates across source and forks.
    for s in 0..4_u8 {
        let m_src = must_ok(source.observe_seat(league_core::Seat(s)), "src obs")
            .own_mandate()
            .copied();
        let m_a = must_ok(fork_a.observe_seat(league_core::Seat(s)), "fork_a obs")
            .own_mandate()
            .copied();
        let m_b = must_ok(fork_b.observe_seat(league_core::Seat(s)), "fork_b obs")
            .own_mandate()
            .copied();
        assert_eq!(m_a, m_src);
        assert_eq!(m_b, m_src);
        assert!(m_src.is_some());
    }

    // Advancing fork_a leaves source and fork_b untouched.
    must_ok(
        fork_a.apply_act_submissions(&BTreeMap::new()),
        "fork_a act turn 1",
    );
    let _ = must_ok(fork_a.sweep_turn(), "fork_a sweep turn 1");
    assert_eq!(fork_a.turn(), 2);
    assert_eq!(source.turn(), 1);
    assert_eq!(fork_b.turn(), 1);

    // Run fork_b to Season Close and export through the Eval replay loader path.
    let bots: Vec<ClubBot> = (0..4)
        .map(|i| ClubBot::new(fork_b.bot_tie_seed() + i as u64))
        .collect();
    for _ in 1..=HARD_TURN_CAP {
        if fork_b.phase() != TurnPhase::TalkAndMarket {
            break;
        }
        let subs = collect_bot_submissions(&fork_b, &bots);
        must_ok(fork_b.apply_act_submissions(&subs), "fork_b act");
        let _ = must_ok(fork_b.sweep_turn(), "fork_b sweep");
    }
    assert_eq!(fork_b.phase(), TurnPhase::AwaitingMandateReveal);

    let (board, log, _section, metrics) = must_ok(fork_b.finalize_eval_season(), "finalize eval");
    assert_eq!(fork_b.phase(), TurnPhase::Completed);
    assert_eq!(board.standings.len(), 4);
    assert_eq!(metrics.len(), 4);

    let log_bytes = must_ok(log.to_json_pretty_bytes(), "to_json_pretty_bytes");
    let eval_loaded = must_ok(
        SeasonReplayLog::from_eval_json_slice(&log_bytes),
        "from_eval_json_slice",
    );
    let replayed = must_ok(replay_eval_season(&eval_loaded), "replay_eval_season");
    assert_eq!(replayed.scoreboard, board);
    assert!(SeasonReplayLog::from_club_json_slice(&log_bytes).is_err());

    let _ = fs::remove_dir_all(&tmp);
}

fn assert_canaries_absent(haystack: &[u8], canaries: &[&str], label: &str) -> Result<(), String> {
    let text = String::from_utf8_lossy(haystack);
    for canary in canaries {
        if text.contains(canary) {
            return Err(format!("canary `{canary}` leaked into {label}"));
        }
    }
    Ok(())
}

#[test]
fn eval_host_canary_bytes_appear_in_seal_and_nowhere_in_save_json() {
    let tmp = unique_tmp_dir("canary");
    let ckpt_dir = tmp.join("checkpoints");

    let deal_seed = *b"CANARY_MANDATE_DEAL_SEED_CARD_10";
    let deal_seed_canary = hex::encode(deal_seed);
    let salt_bytes = [
        *b"CANARY_MANDATE_SALT_CARD10_SEAT0",
        *b"CANARY_MANDATE_SALT_CARD10_SEAT1",
        *b"CANARY_MANDATE_SALT_CARD10_SEAT2",
        *b"CANARY_MANDATE_SALT_CARD10_SEAT3",
    ];
    let seat_salts: Vec<RevealSalt> = salt_bytes.into_iter().map(RevealSalt).collect();
    let salt_canaries: Vec<String> = salt_bytes.iter().map(hex::encode).collect();

    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes([0x6A; 32]),
        "SealedEvalSeed::try_from_bytes",
    );
    let tid = TableId(910_108);
    let deck = league_core::default_graded_mandate_deck(4);
    let (dealt_commits, _) = must_ok(
        deal_graded_mandates_with_entropy(tid, 4, deal_seed, &seat_salts, &deck),
        "deal_graded_mandates_with_entropy",
    );
    let commit_canary = dealt_commits[0].to_string();

    let mut host = must_ok(
        EvalHost::from_explicit_setup(tid, 4, 7, deal_seed, &seat_salts, eval_seed, 99),
        "EvalHost::from_explicit_setup",
    );

    // Pre-act checkpoint canary scan.
    let (pre_save, pre_seal) = must_ok(
        host.write_pre_act_checkpoint(&ckpt_dir),
        "write_pre_act_checkpoint",
    );
    let pre_save_bytes = must_ok(fs::read(&pre_save), "read pre_save");
    let pre_seal_bytes = must_ok(fs::read(&pre_seal), "read pre_seal");
    let pre_seal_text = String::from_utf8_lossy(&pre_seal_bytes);

    let mandate_canaries = [
        deal_seed_canary.as_str(),
        salt_canaries[0].as_str(),
        salt_canaries[1].as_str(),
        salt_canaries[2].as_str(),
        salt_canaries[3].as_str(),
        commit_canary.as_str(),
    ];
    for c in mandate_canaries {
        assert!(
            pre_seal_text.contains(c),
            "expected mandate canary `{c}` in .seal"
        );
    }
    must_ok(
        assert_canaries_absent(
            &pre_save_bytes,
            &mandate_canaries,
            "day-01-pre-act.save.json",
        ),
        "pre-act save canary scan",
    );

    // Act-recorded seal canary scan (including a PreimageSecret in recorded submission).
    let act_preimage = PreimageSecret(*b"CANARY_ACT_PREIMAGE_CARD10_SEAT0");
    let act_preimage_canary = hex::encode(act_preimage.0);
    let cond = must_ok(compute_preimage_condition(&act_preimage), "condition");
    let subs = BTreeMap::from([(
        0_u8,
        EvalActSubmission {
            market_actions: vec![MarketAction::CreateEscrow {
                seat: 0,
                escrow: league_core::EscrowProposal {
                    destination: 1,
                    asset: AssetId::Cred,
                    amount: 500,
                    finish_after_turn: None,
                    cancel_after_turn: 3,
                    condition_hash: Some(cond),
                    bond: None,
                },
            }],
            pass_b_actions: vec![],
            bundle: SealedOrderBundle {
                escrow_finishes: vec![league_core::EscrowFinishUnlock {
                    escrow_id: 1,
                    preimage: Some(act_preimage),
                }],
                ..SealedOrderBundle::default()
            },
        },
    )]);
    must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
    let rec_seal = must_ok(
        host.write_act_recorded_seal(&ckpt_dir),
        "write_act_recorded_seal",
    );
    let rec_seal_bytes = must_ok(fs::read(&rec_seal), "read rec_seal");
    let rec_seal_text = String::from_utf8_lossy(&rec_seal_bytes);
    assert!(rec_seal_text.contains(&act_preimage_canary));

    let post_save = tmp.join("eval-910108-post.save.json");
    let post_seal = tmp.join("eval-910108-post.seal");
    must_ok(
        host.write_save_and_seal(&post_save, &post_seal),
        "write post save/seal",
    );
    let post_save_bytes = must_ok(fs::read(&post_save), "read post_save");
    let all_canaries = [
        deal_seed_canary.as_str(),
        salt_canaries[0].as_str(),
        salt_canaries[1].as_str(),
        salt_canaries[2].as_str(),
        salt_canaries[3].as_str(),
        commit_canary.as_str(),
        act_preimage_canary.as_str(),
    ];
    must_ok(
        assert_canaries_absent(&post_save_bytes, &all_canaries, "post .save.json"),
        "post save canary scan",
    );

    // Negative control: prove the scanner catches a planted canary leak.
    let planted_leak = format!("{{\"leaked\":\"{}\"}}", salt_canaries[0]);
    assert!(assert_canaries_absent(planted_leak.as_bytes(), &all_canaries, "planted").is_err());

    let _ = fs::remove_dir_all(&tmp);
}
