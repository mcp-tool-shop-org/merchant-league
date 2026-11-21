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
use sha2::{Digest, Sha256};

fn test_layout_seed_beacon_candidates(seed: u64) -> [[u8; 32]; 2] {
    let mut padded = [0_u8; 32];
    padded[..8].copy_from_slice(&seed.to_le_bytes());
    let hashed: [u8; 32] = Sha256::digest(seed.to_le_bytes()).into();
    [padded, hashed]
}

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

        // R6: sensitivity check on day 1 — changing one seat's bundle gives a different receipt.
        if day == 1 {
            let mut altered_fork = must_ok(
                EvalHost::fork_from_checkpoint(&pre_save, &pre_seal),
                "fork day 1 altered",
            );
            let mut altered_subs = must_ok(
                altered_fork.read_recorded_act_submissions(&rec_seal),
                "read_recorded_act_submissions day 1",
            );
            if let Some(sub0) = altered_subs.get_mut(&0) {
                sub0.market_actions.push(MarketAction::TownImport {
                    seat: 0,
                    good: AssetId::Raw(2),
                    amount: 200,
                });
                sub0.bundle.debt_repayment_cred = 500;
            }
            let _ = must_ok(
                altered_fork.apply_act_submissions(&altered_subs),
                "apply altered_subs",
            );
            let altered_receipt = must_ok(altered_fork.sweep_turn(), "sweep altered_fork");
            let altered_json = must_ok(serde_json::to_vec(&altered_receipt), "ser altered_receipt");
            assert_ne!(
                altered_json, orig_json,
                "changing one seat's bundle/orders must alter the day 1 receipt"
            );
        }
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
    assert_eq!(err.code(), "INVALID_CONFIG");
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
    assert_eq!(game_err.code(), "INVALID_CONFIG");
    assert!(game_err.to_string().contains("INVALID_TABLE_MODE"));

    // 3. Refuse a real ClubHost save+seal pair by version header error code (R2).
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
    let club_read_err = match EvalHost::read_save_and_seal(&club_save, &club_seal) {
        Ok(_) => panic!("EvalHost::read_save_and_seal must refuse ClubHost save+seal"),
        Err(err) => err,
    };
    assert_eq!(club_read_err.code(), "UNSUPPORTED_REPLAY_VERSION");
    let club_fork_err = match EvalHost::fork_from_checkpoint(&club_save, &club_seal) {
        Ok(_) => panic!("EvalHost::fork_from_checkpoint must refuse ClubHost save+seal"),
        Err(err) => err,
    };
    assert_eq!(club_fork_err.code(), "UNSUPPORTED_REPLAY_VERSION");

    // 4. R2: refuse TableMode::Club and TableMode::Game inside an Eval seal's JSON via
    // INVALID_SEAL_TABLE_MODE.
    let eval_host = must_ok(
        EvalHost::from_standard_setup(TableId(910_115), 4, 7, 1),
        "EvalHost::from_standard_setup",
    );
    let (eval_save, eval_seal) = must_ok(eval_host.write_game_save_and_seal(&tmp), "write eval");
    let eval_seal_bytes = must_ok(fs::read(&eval_seal), "read eval_seal");
    for bad_mode in ["Club", "Game"] {
        let mut tampered_seal: serde_json::Value =
            must_ok(serde_json::from_slice(&eval_seal_bytes), "parse eval seal");
        tampered_seal["engine"]["state"]["spec"]["mode"] = serde_json::json!(bad_mode);
        let tampered_path = tmp.join(format!("eval-bad-mode-{bad_mode}.seal"));
        must_ok(
            fs::write(
                &tampered_path,
                must_ok(serde_json::to_vec(&tampered_seal), "ser tampered seal"),
            ),
            "write tampered seal",
        );
        let mode_err = match EvalHost::read_save_and_seal(&eval_save, &tampered_path) {
            Ok(_) => panic!("EvalHost must refuse spec.mode={bad_mode} inside Eval seal"),
            Err(err) => err,
        };
        assert_eq!(mode_err.code(), "INVALID_CONFIG");
        assert!(
            mode_err.to_string().contains("INVALID_SEAL_TABLE_MODE"),
            "expected INVALID_SEAL_TABLE_MODE for mode={bad_mode}, got: {mode_err}"
        );
    }

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

struct BinaryCanary {
    label: &'static str,
    hex: String,
    json_array: String,
    decimal_list: String,
}

impl BinaryCanary {
    fn from_bytes(label: &'static str, bytes: &[u8; 32]) -> Self {
        let hex = hex::encode(bytes);
        let json_array = must_ok(
            serde_json::to_string(bytes.as_slice()),
            "serialize json array canary",
        );
        let decimal_list = format!("{bytes:?}");
        Self {
            label,
            hex,
            json_array,
            decimal_list,
        }
    }

    fn all_encodings(&self) -> [(&'static str, &str); 3] {
        [
            ("hex", self.hex.as_str()),
            ("json_array", self.json_array.as_str()),
            ("decimal_list", self.decimal_list.as_str()),
        ]
    }
}

fn assert_canaries_absent(
    haystack: &[u8],
    canaries: &[BinaryCanary],
    label: &str,
) -> Result<(), String> {
    let text = String::from_utf8_lossy(haystack);
    for canary in canaries {
        for (enc_name, needle) in canary.all_encodings() {
            if text.contains(needle) {
                return Err(format!(
                    "canary `{}` ({enc_name}: `{needle}`) leaked into {label}",
                    canary.label
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn eval_host_canary_bytes_appear_in_seal_and_nowhere_in_save_json() {
    let tmp = unique_tmp_dir("canary");
    let ckpt_dir = tmp.join("checkpoints");

    let deal_seed = *b"CANARY_MANDATE_DEAL_SEED_CARD_10";
    let salt_bytes = [
        *b"CANARY_MANDATE_SALT_CARD10_SEAT0",
        *b"CANARY_MANDATE_SALT_CARD10_SEAT1",
        *b"CANARY_MANDATE_SALT_CARD10_SEAT2",
        *b"CANARY_MANDATE_SALT_CARD10_SEAT3",
    ];
    let seat_salts: Vec<RevealSalt> = salt_bytes.into_iter().map(RevealSalt).collect();

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

    let mut host = must_ok(
        EvalHost::from_explicit_setup(tid, 4, 7, deal_seed, &seat_salts, eval_seed, 99),
        "EvalHost::from_explicit_setup",
    );

    // Pre-act checkpoint canary scan (R6: hex + json_array + decimal_list).
    let (pre_save, pre_seal) = must_ok(
        host.write_pre_act_checkpoint(&ckpt_dir),
        "write_pre_act_checkpoint",
    );
    let pre_save_bytes = must_ok(fs::read(&pre_save), "read pre_save");
    let pre_seal_bytes = must_ok(fs::read(&pre_seal), "read pre_seal");

    let mandate_canaries = vec![
        BinaryCanary::from_bytes("deal_seed", &deal_seed),
        BinaryCanary::from_bytes("seat0_salt", &salt_bytes[0]),
        BinaryCanary::from_bytes("seat1_salt", &salt_bytes[1]),
        BinaryCanary::from_bytes("seat2_salt", &salt_bytes[2]),
        BinaryCanary::from_bytes("seat3_salt", &salt_bytes[3]),
        BinaryCanary::from_bytes("dealt_commit_0", &dealt_commits[0].0),
    ];
    must_ok(
        assert_canaries_absent(
            &pre_save_bytes,
            &mandate_canaries,
            "day-01-pre-act.save.json",
        ),
        "pre-act save canary scan",
    );
    // R6 negative control: the exact same scanner fails on .seal bytes and finds every canary.
    assert!(
        assert_canaries_absent(&pre_seal_bytes, &mandate_canaries, "day-01-pre-act.seal").is_err(),
        "scanner must detect mandate canaries inside .seal"
    );
    for canary in &mandate_canaries {
        let single = std::slice::from_ref(canary);
        assert!(
            assert_canaries_absent(&pre_seal_bytes, single, "pre-act .seal").is_err(),
            "scanner missed `{}` in pre-act .seal",
            canary.label
        );
        for (enc_name, needle) in canary.all_encodings() {
            assert!(
                assert_canaries_absent(needle.as_bytes(), single, enc_name).is_err(),
                "scanner missed `{}` encoding `{enc_name}`",
                canary.label
            );
        }
    }

    // Act-recorded seal canary scan (including a PreimageSecret in recorded submission).
    let act_preimage = PreimageSecret(*b"CANARY_ACT_PREIMAGE_CARD10_SEAT0");
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
                    preimage: Some(act_preimage.clone()),
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

    let (post_save, post_seal) = must_ok(
        host.write_game_save_and_seal(&tmp),
        "write_game_save_and_seal post",
    );
    let post_save_bytes = must_ok(fs::read(&post_save), "read post_save");
    let post_seal_bytes = must_ok(fs::read(&post_seal), "read post_seal");

    let mut all_canaries = mandate_canaries;
    all_canaries.push(BinaryCanary::from_bytes("act_preimage", &act_preimage.0));

    must_ok(
        assert_canaries_absent(&post_save_bytes, &all_canaries, "post .save.json"),
        "post save canary scan",
    );
    // R6 negative control: scan over recorded-act .seal and post .seal finds every canary across
    // checked encodings.
    assert!(assert_canaries_absent(&post_seal_bytes, &all_canaries, "post .seal").is_err());
    for canary in &all_canaries {
        let single = std::slice::from_ref(canary);
        assert!(
            assert_canaries_absent(&rec_seal_bytes, single, "act-recorded .seal").is_err(),
            "scanner missed `{}` in act-recorded .seal",
            canary.label
        );
        for (enc_name, needle) in canary.all_encodings() {
            assert!(
                assert_canaries_absent(needle.as_bytes(), single, enc_name).is_err(),
                "scanner missed `{}` encoding `{enc_name}`",
                canary.label
            );
        }
    }

    let _ = fs::remove_dir_all(&tmp);
}

// =============================================================================
// R3 & R5 dedicated guard tests (one named test per guard, asserting its error code)
// =============================================================================

fn write_mutated_eval_seal(
    tmp: &std::path::Path,
    name: &str,
    seal_bytes: &[u8],
    mutate: impl FnOnce(&mut serde_json::Value),
) -> PathBuf {
    let mut val: serde_json::Value = must_ok(serde_json::from_slice(seal_bytes), "parse eval seal");
    mutate(&mut val);
    let out_path = tmp.join(name);
    let encoded = must_ok(serde_json::to_vec(&val), "serialize mutated eval seal");
    must_ok(fs::write(&out_path, encoded), "write mutated eval seal");
    out_path
}

fn explicit_pre_act_host(tid: TableId, layout_seed: u64, tag: u8) -> EvalHost {
    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes([tag; 32]),
        "SealedEvalSeed::try_from_bytes",
    );
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([tag + 1 + i; 32])).collect();
    must_ok(
        EvalHost::from_explicit_setup(tid, 4, layout_seed, [tag + 9; 32], &salts, eval_seed, 55),
        "EvalHost::from_explicit_setup",
    )
}

#[test]
fn r3_fork_from_post_act_pair_refuses_invalid_checkpoint_phase() {
    let tmp = unique_tmp_dir("r3_post_act_fork");
    let mut host = explicit_pre_act_host(TableId(920_001), 7, 0x10);
    must_ok(
        host.apply_act_submissions(&BTreeMap::new()),
        "apply_act_submissions",
    );
    assert_eq!(host.phase(), TurnPhase::Reveal);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write post-act pair");
    let err = match EvalHost::fork_from_checkpoint(&save_p, &seal_p) {
        Ok(_) => panic!("fork_from_checkpoint must refuse post-act Reveal pair"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_PHASE");
    assert!(err.to_string().contains("INVALID_CHECKPOINT_PHASE"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_recorded_seal_table_mismatch_refused() {
    let tmp = unique_tmp_dir("r3_rec_table_mismatch");
    let mut host_a = explicit_pre_act_host(TableId(920_002), 7, 0x20);
    let mut host_b = explicit_pre_act_host(TableId(920_003), 7, 0x20);
    let ckpt_a = tmp.join("a/checkpoints");
    let ckpt_b = tmp.join("b/checkpoints");
    let _ = must_ok(host_a.write_pre_act_checkpoint(&ckpt_a), "ckpt_a");
    let (save_b, seal_b) = must_ok(host_b.write_pre_act_checkpoint(&ckpt_b), "ckpt_b");
    must_ok(host_a.apply_act_submissions(&BTreeMap::new()), "apply a");
    let rec_a = must_ok(host_a.write_act_recorded_seal(&ckpt_a), "write rec_a");

    let mut fork_b = must_ok(EvalHost::fork_from_checkpoint(&save_b, &seal_b), "fork_b");
    let err_read = match fork_b.read_recorded_act_submissions(&rec_a) {
        Ok(_) => panic!("read_recorded_act_submissions must refuse table mismatch"),
        Err(err) => err,
    };
    assert_eq!(err_read.code(), "INVALID_CONFIG");
    assert!(
        err_read
            .to_string()
            .contains("RECORDED_SEAL_TABLE_MISMATCH")
    );

    let err_apply = match fork_b.apply_recorded_act_submissions_from_seal(&rec_a) {
        Ok(_) => panic!("apply_recorded_act_submissions_from_seal must refuse table mismatch"),
        Err(err) => err,
    };
    assert_eq!(err_apply.code(), "INVALID_CONFIG");
    assert!(
        err_apply
            .to_string()
            .contains("RECORDED_SEAL_TABLE_MISMATCH")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_recorded_seal_origin_mismatch_refused() {
    let tmp = unique_tmp_dir("r3_rec_origin_mismatch");
    let mut host_a = explicit_pre_act_host(TableId(920_004), 7, 0x30);
    let mut host_b = explicit_pre_act_host(TableId(920_004), 7, 0x40);
    let ckpt_a = tmp.join("a/checkpoints");
    let ckpt_b = tmp.join("b/checkpoints");
    let _ = must_ok(host_a.write_pre_act_checkpoint(&ckpt_a), "ckpt_a");
    let (save_b, seal_b) = must_ok(host_b.write_pre_act_checkpoint(&ckpt_b), "ckpt_b");
    must_ok(host_a.apply_act_submissions(&BTreeMap::new()), "apply a");
    let rec_a = must_ok(host_a.write_act_recorded_seal(&ckpt_a), "write rec_a");

    let mut fork_b = must_ok(EvalHost::fork_from_checkpoint(&save_b, &seal_b), "fork_b");
    let err_read = match fork_b.read_recorded_act_submissions(&rec_a) {
        Ok(_) => panic!("read_recorded_act_submissions must refuse origin mismatch"),
        Err(err) => err,
    };
    assert_eq!(err_read.code(), "INVALID_CONFIG");
    assert!(
        err_read
            .to_string()
            .contains("RECORDED_SEAL_ORIGIN_MISMATCH")
    );

    let err_apply = match fork_b.apply_recorded_act_submissions_from_seal(&rec_a) {
        Ok(_) => panic!("apply_recorded_act_submissions_from_seal must refuse origin mismatch"),
        Err(err) => err,
    };
    assert_eq!(err_apply.code(), "INVALID_CONFIG");
    assert!(
        err_apply
            .to_string()
            .contains("RECORDED_SEAL_ORIGIN_MISMATCH")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_pre_act_seal_as_recorded_seal_refuses_invalid_act_recorded_seal_phase() {
    let tmp = unique_tmp_dir("r3_pre_as_rec");
    let mut host = explicit_pre_act_host(TableId(920_005), 7, 0x10);
    let ckpt = tmp.join("checkpoints");
    let (pre_save, pre_seal) = must_ok(host.write_pre_act_checkpoint(&ckpt), "write pre");
    let mut fork = must_ok(EvalHost::fork_from_checkpoint(&pre_save, &pre_seal), "fork");

    let err_read = match fork.read_recorded_act_submissions(&pre_seal) {
        Ok(_) => panic!("read_recorded_act_submissions must refuse pre-act seal"),
        Err(err) => err,
    };
    assert_eq!(err_read.code(), "INVALID_PHASE");
    assert!(
        err_read
            .to_string()
            .contains("INVALID_ACT_RECORDED_SEAL_PHASE")
    );

    let err_apply = match fork.apply_recorded_act_submissions_from_seal(&pre_seal) {
        Ok(_) => panic!("apply_recorded_act_submissions_from_seal must refuse pre-act seal"),
        Err(err) => err,
    };
    assert_eq!(err_apply.code(), "INVALID_PHASE");
    assert!(
        err_apply
            .to_string()
            .contains("INVALID_ACT_RECORDED_SEAL_PHASE")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_recorded_seal_empty_submissions_refuses_invalid_act_recorded_submissions() {
    let tmp = unique_tmp_dir("r3_empty_rec_subs");
    let mut host = explicit_pre_act_host(TableId(920_006), 7, 0x10);
    let ckpt = tmp.join("checkpoints");
    let (pre_save, pre_seal) = must_ok(host.write_pre_act_checkpoint(&ckpt), "write pre");
    must_ok(host.apply_act_submissions(&BTreeMap::new()), "apply");
    let rec_seal = must_ok(host.write_act_recorded_seal(&ckpt), "write rec");
    let rec_bytes = must_ok(fs::read(&rec_seal), "read rec_seal");
    let bad_rec = write_mutated_eval_seal(&tmp, "bad-empty-subs.seal", &rec_bytes, |v| {
        v["recorded_act_submissions"] = serde_json::json!({});
    });

    let mut fork = must_ok(EvalHost::fork_from_checkpoint(&pre_save, &pre_seal), "fork");
    let err_read = match fork.read_recorded_act_submissions(&bad_rec) {
        Ok(_) => panic!("must refuse incomplete recorded_act_submissions"),
        Err(err) => err,
    };
    assert_eq!(err_read.code(), "INVALID_CONFIG");
    assert!(
        err_read
            .to_string()
            .contains("INVALID_ACT_RECORDED_SUBMISSIONS")
    );

    let err_apply = match fork.apply_recorded_act_submissions_from_seal(&bad_rec) {
        Ok(_) => panic!("must refuse incomplete recorded_act_submissions"),
        Err(err) => err,
    };
    assert_eq!(err_apply.code(), "INVALID_CONFIG");
    assert!(
        err_apply
            .to_string()
            .contains("INVALID_ACT_RECORDED_SUBMISSIONS")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_vrf_proof_wrong_len_refused() {
    let tmp = unique_tmp_dir("r3_vrf_len");
    let host = explicit_pre_act_host(TableId(920_007), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-vrf-len.seal", &seal_bytes, |v| {
        v["vrf_proofs"][0] = serde_json::json!([1, 2, 3]);
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse wrong VRF proof len"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_SEAL_VRF_PROOF_LEN"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_vrf_proof_verification_failure_refused() {
    let tmp = unique_tmp_dir("r3_vrf_verify");
    let host = explicit_pre_act_host(TableId(920_008), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-vrf-verify.seal", &seal_bytes, |v| {
        if let Some(arr) = v["vrf_proofs"][0].as_array_mut()
            && let Some(first) = arr.first_mut()
        {
            let orig = first.as_u64().unwrap_or(0);
            *first = serde_json::json!(orig ^ 1);
        }
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse invalid VRF proof bytes"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_ORDER");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_invalid_sealed_mandate_deal_refused() {
    let tmp = unique_tmp_dir("r3_seal_mandate_deal");
    let host = explicit_pre_act_host(TableId(920_009), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-deal.seal", &seal_bytes, |v| {
        v["sealed_mandate_deal"]["deal_seed"] = serde_json::json!("ff".repeat(32));
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse mismatched sealed_mandate_deal on load"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_SEALED_MANDATE_DEAL"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_mandate_len_mismatch_refused() {
    let tmp = unique_tmp_dir("r3_seal_mandate_len");
    let host = explicit_pre_act_host(TableId(920_010), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-mandate-len.seal", &seal_bytes, |v| {
        v["mandate_reveals"] = serde_json::json!([]);
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse mandate_reveals length mismatch"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_SEAL_MANDATE_LEN"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_digest_len_mismatch_refused() {
    let tmp = unique_tmp_dir("r3_seal_digest_len");
    let host = explicit_pre_act_host(TableId(920_011), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-digest-len.seal", &seal_bytes, |v| {
        v["live_state_digests"] = serde_json::json!([vec![0_u8; 32]]);
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse live_state_digests length mismatch"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_SEAL_DIGEST_LEN"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_turn_salts_shape_refused() {
    let tmp = unique_tmp_dir("r3_seal_salts_shape");
    let host = explicit_pre_act_host(TableId(920_012), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-salts-shape.seal", &seal_bytes, |v| {
        v["turn_salts"][0] = serde_json::json!([]);
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse turn_salts per-turn shape mismatch"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_SEAL_TURN_SALTS"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_recorded_submission_seat_out_of_range_refused() {
    let tmp = unique_tmp_dir("r3_seal_sub_seat");
    let host = explicit_pre_act_host(TableId(920_013), 7, 0x10);
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-sub-seat.seal", &seal_bytes, |v| {
        v["recorded_act_submissions"] = serde_json::json!({
            "9": {
                "market_actions": [],
                "bundle": {
                    "converter_runs": [],
                    "project_deliveries": [],
                    "atomic_swaps": [],
                    "contract_deliveries": [],
                    "dual_escrows": [],
                    "debt_repayment_cred": 0,
                    "escrow_finishes": [],
                    "check_cashes": [],
                    "council_vote": null,
                    "charter_bids": []
                }
            }
        });
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse out-of-range seat in recorded_act_submissions"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_SEAT");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_seal_load_recorded_submissions_partial_len_refused() {
    let tmp = unique_tmp_dir("r3_seal_sub_len");
    let mut host = explicit_pre_act_host(TableId(920_014), 7, 0x10);
    must_ok(host.apply_act_submissions(&BTreeMap::new()), "apply");
    let (save_p, seal_p) = must_ok(host.write_game_save_and_seal(&tmp), "write");
    let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
    let bad_seal = write_mutated_eval_seal(&tmp, "bad-sub-len.seal", &seal_bytes, |v| {
        if let Some(obj) = v["recorded_act_submissions"].as_object_mut() {
            obj.remove("3");
        }
    });
    let err = match EvalHost::read_save_and_seal(&save_p, &bad_seal) {
        Ok(_) => panic!("must refuse partial recorded_act_submissions in Reveal phase"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_RECORDED_SUBMISSIONS_LEN"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r3_new_deal_seed_not_independent_refused() {
    for layout_seed in [league_core::SEALED_LAYOUT_SEED_MIN + 7, 7_u64] {
        let [cand_pad, _] = test_layout_seed_beacon_candidates(layout_seed);
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x21 + i; 32])).collect();
        let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x66; 32]), "eval_seed");
        let (spec, deal, eval_seed) = must_ok(
            TableSpec::standard_eval_chiral_ring_with_explicit_setup(
                TableId(920_015),
                4,
                layout_seed,
                cand_pad,
                &salts,
                eval_seed,
            ),
            "setup",
        );
        let err = match EvalHost::new(spec, layout_seed, deal, eval_seed, 1) {
            Ok(_) => panic!("must refuse non-independent deal_seed (layout_seed={layout_seed})"),
            Err(err) => err,
        };
        assert_eq!(err.code(), "INVALID_CONFIG");
        if layout_seed >= league_core::SEALED_LAYOUT_SEED_MIN {
            assert!(err.to_string().contains("SEALED_DEAL_SEED_NOT_INDEPENDENT"));
        } else {
            assert!(err.to_string().contains("deal_seed must be independent"));
        }
    }
}

#[test]
fn r3_new_eval_vrf_not_independent_refused() {
    for layout_seed in [league_core::SEALED_LAYOUT_SEED_MIN + 9, 9_u64] {
        let [_, cand_hash] = test_layout_seed_beacon_candidates(layout_seed);
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31 + i; 32])).collect();
        let non_indep_vrf = must_ok(SealedEvalSeed::try_from_bytes(cand_hash), "eval_seed");
        let (spec, deal, eval_seed) = must_ok(
            TableSpec::standard_eval_chiral_ring_with_explicit_setup(
                TableId(920_016),
                4,
                layout_seed,
                [0x99; 32],
                &salts,
                non_indep_vrf,
            ),
            "setup",
        );
        let err = match EvalHost::new(spec, layout_seed, deal, eval_seed, 1) {
            Ok(_) => {
                panic!("must refuse non-independent Eval VRF seed (layout_seed={layout_seed})")
            }
            Err(err) => err,
        };
        assert_eq!(err.code(), "INVALID_CONFIG");
        if layout_seed >= league_core::SEALED_LAYOUT_SEED_MIN {
            assert!(err.to_string().contains("SEALED_EVAL_VRF_NOT_INDEPENDENT"));
        } else {
            assert!(
                err.to_string()
                    .contains("must be independent of public layout seed")
            );
        }
    }
}

#[test]
fn r3_new_invalid_sealed_mandate_deal_refused() {
    let (spec, mut bad_deal, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(TableId(920_017), 4, 7),
        "setup",
    );
    bad_deal.deal_seed[0] ^= 0xFF;
    let err = match EvalHost::new(spec, 7, bad_deal, eval_seed, 1) {
        Ok(_) => panic!("must refuse mismatched sealed_mandate_deal in EvalHost::new"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_CONFIG");
    assert!(err.to_string().contains("INVALID_SEALED_MANDATE_DEAL"));
}

#[test]
fn r3_apply_act_submissions_out_of_range_seat_refused() {
    let mut host = explicit_pre_act_host(TableId(920_018), 7, 0x10);
    let subs = BTreeMap::from([(8_u8, EvalActSubmission::default())]);
    let err = match host.apply_act_submissions(&subs) {
        Ok(_) => panic!("must refuse out-of-range seat index in apply_act_submissions"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "INVALID_SEAT");
}

#[test]
fn r3_apply_act_submissions_pass_b_cross_seat_records_unauthorized() {
    let mut host = explicit_pre_act_host(TableId(920_019), 7, 0x10);
    let subs = BTreeMap::from([(
        0_u8,
        EvalActSubmission {
            market_actions: vec![],
            pass_b_actions: vec![MarketAction::PostOffer {
                seat: 1, // cross-seat in Pass B
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 100,
                    price_per_unit_cred: 1_000,
                    domain_id: None,
                    is_hybrid: false,
                },
            }],
            bundle: SealedOrderBundle::default(),
        },
    )]);
    let outcome = must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
    assert_eq!(
        outcome.seats.get(&0).map(|r| &r.refused_pass_b_actions),
        Some(&vec![(0, "UNAUTHORIZED".to_owned())])
    );
}

#[test]
fn r5_recorded_seal_from_one_d2_duplicate_refused_on_other_duplicate_checkpoint() {
    let tmp = unique_tmp_dir("r5_dup_isolation");
    let src_dir = tmp.join("source");
    let dup1_dir = tmp.join("dup1");
    let dup2_dir = tmp.join("dup2");

    let src = explicit_pre_act_host(TableId(920_020), 7, 0x10);
    let (src_save, src_seal) = must_ok(src.write_game_save_and_seal(&src_dir), "write src");

    let mut dup1 = must_ok(
        EvalHost::fork_duplicate_from_turn_0(&src_save, &src_seal),
        "fork dup1",
    );
    let mut dup2 = must_ok(
        EvalHost::fork_duplicate_from_turn_0(&src_save, &src_seal),
        "fork dup2",
    );
    assert_ne!(dup1.game_id(), src.game_id());
    assert_ne!(dup2.game_id(), src.game_id());
    assert_ne!(dup1.game_id(), dup2.game_id());

    let ckpt1 = dup1_dir.join("checkpoints");
    let ckpt2 = dup2_dir.join("checkpoints");
    let _ = must_ok(dup1.write_pre_act_checkpoint(&ckpt1), "ckpt1");
    let (d2_save, d2_seal) = must_ok(dup2.write_pre_act_checkpoint(&ckpt2), "ckpt2");
    must_ok(dup1.apply_act_submissions(&BTreeMap::new()), "apply dup1");
    let rec1 = must_ok(dup1.write_act_recorded_seal(&ckpt1), "rec1");

    let mut fork2 = must_ok(EvalHost::fork_from_checkpoint(&d2_save, &d2_seal), "fork2");
    let err_read = match fork2.read_recorded_act_submissions(&rec1) {
        Ok(_) => panic!("dup2 must refuse recorded seal from dup1"),
        Err(err) => err,
    };
    assert_eq!(err_read.code(), "INVALID_CONFIG");
    assert!(
        err_read
            .to_string()
            .contains("RECORDED_SEAL_GAME_ID_MISMATCH")
    );

    let err_apply = match fork2.apply_recorded_act_submissions_from_seal(&rec1) {
        Ok(_) => panic!("dup2 must refuse recorded seal from dup1"),
        Err(err) => err,
    };
    assert_eq!(err_apply.code(), "INVALID_CONFIG");
    assert!(
        err_apply
            .to_string()
            .contains("RECORDED_SEAL_GAME_ID_MISMATCH")
    );

    // Same game_id but wrong pre-act checkpoint SHA-256 -> RECORDED_SEAL_CHECKPOINT_MISMATCH.
    let rec1_bytes = must_ok(fs::read(&rec1), "read rec1");
    let same_game_bad_ckpt =
        write_mutated_eval_seal(&tmp, "same-game-bad-ckpt.seal", &rec1_bytes, |v| {
            v["game_id"] = serde_json::json!(fork2.game_id());
        });
    let err_ckpt = match fork2.read_recorded_act_submissions(&same_game_bad_ckpt) {
        Ok(_) => panic!("must refuse mismatched pre-act checkpoint sha256"),
        Err(err) => err,
    };
    assert_eq!(err_ckpt.code(), "INVALID_CONFIG");
    assert!(
        err_ckpt
            .to_string()
            .contains("RECORDED_SEAL_CHECKPOINT_MISMATCH")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r5_recorded_seal_from_another_day_refused_on_both_read_and_apply() {
    let tmp = unique_tmp_dir("r5_other_day");
    let ckpt = tmp.join("checkpoints");
    let mut host = explicit_pre_act_host(TableId(920_021), 7, 0x10);

    let (d1_save, d1_seal) = must_ok(host.write_pre_act_checkpoint(&ckpt), "d1 pre");
    must_ok(host.apply_act_submissions(&BTreeMap::new()), "d1 apply");
    let d1_rec = must_ok(host.write_act_recorded_seal(&ckpt), "d1 rec");
    let _ = must_ok(host.sweep_turn(), "d1 sweep");

    let (d2_save, d2_seal) = must_ok(host.write_pre_act_checkpoint(&ckpt), "d2 pre");
    must_ok(host.apply_act_submissions(&BTreeMap::new()), "d2 apply");
    let d2_rec = must_ok(host.write_act_recorded_seal(&ckpt), "d2 rec");

    let fork_d1 = must_ok(
        EvalHost::fork_from_checkpoint(&d1_save, &d1_seal),
        "fork d1",
    );
    let e1 = fork_d1
        .read_recorded_act_submissions(&d2_rec)
        .expect_err_or_msg("d1 read d2_rec");
    assert!(e1.contains("RECORDED_SEAL_TURN_MISMATCH"), "got {e1}");

    let mut fork_d2 = must_ok(
        EvalHost::fork_from_checkpoint(&d2_save, &d2_seal),
        "fork d2",
    );
    let e2 = fork_d2
        .apply_recorded_act_submissions_from_seal(&d1_rec)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(e2.contains("RECORDED_SEAL_TURN_MISMATCH"), "got {e2}");
    let _ = fs::remove_dir_all(&tmp);
}

trait ExpectErrString<T> {
    fn expect_err_or_msg(self, ctx: &str) -> String;
}

impl<T, E: std::fmt::Display> ExpectErrString<T> for Result<T, E> {
    fn expect_err_or_msg(self, ctx: &str) -> String {
        match self {
            Ok(_) => panic!("expected Err in {ctx}"),
            Err(err) => err.to_string(),
        }
    }
}

#[test]
fn eval_host_public_api_guard_and_persistence_edge_cases() {
    use league_core::{CommitmentHash, Seat};

    let tmp = unique_tmp_dir("pub_api_guards");
    assert!(EvalHost::from_standard_setup(TableId(920_099), 3, 7, 1).is_err());

    let mut manual = must_ok(
        EvalHost::from_standard_setup(TableId(920_040), 4, 7, 1),
        "setup",
    );
    assert_eq!(manual.layout_seed(), 7);
    assert_eq!(manual.public_spec().table_id, TableId(920_040));
    assert!(manual.preview_regent_bundle_for_seat(Seat(9)).is_err());
    assert!(manual.sweep_turn().is_err());
    assert!(manual.advance_to_reveal().is_err());
    assert!(
        manual
            .apply_market_action(MarketAction::TownImport {
                seat: 0,
                good: AssetId::Raw(0),
                amount: 0,
            })
            .is_err()
    );
    must_ok(
        manual.apply_market_action(MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(0),
            amount: 50,
        }),
        "valid TownImport",
    );
    assert!(
        manual
            .write_pre_act_checkpoint(&tmp.join("manual_ckpts"))
            .err()
            .is_some_and(|e| e.to_string().contains("INVALID_PRE_ACT_STATE"))
    );
    must_ok(manual.begin_cancel_lock(), "begin_cancel_lock");
    assert!(manual.cancel_locked());

    must_ok(manual.advance_to_commit(), "advance_to_commit");
    assert!(manual.advance_to_commit().is_err());
    assert!(
        manual
            .submit_order_commitment(Seat(9), CommitmentHash([0_u8; 32]))
            .is_err()
    );
    must_ok(manual.advance_to_reveal(), "advance_to_reveal");
    assert!(manual.advance_to_reveal().is_err());
    assert!(manual.begin_cancel_lock().is_err());
    assert!(
        manual
            .apply_act_submissions(&BTreeMap::new())
            .err()
            .is_some_and(|e| e.to_string().contains("INVALID_CHECKPOINT_PHASE"))
    );
    let _ = must_ok(manual.sweep_turn(), "sweep after manual reveal");

    // Pass A & Pass B invalid own action + MISSING_PRE_ACT_CHECKPOINT + pre-act mismatch + missing files.
    let mut act_host = must_ok(
        EvalHost::from_standard_setup(TableId(920_041), 4, 7, 1),
        "setup2",
    );
    assert!(
        act_host
            .read_recorded_act_submissions(&tmp.join("nonexistent.seal"))
            .err()
            .is_some_and(|e| e.to_string().contains("MISSING_PRE_ACT_CHECKPOINT"))
    );
    let bad_own_subs = BTreeMap::from([(
        0_u8,
        EvalActSubmission {
            market_actions: vec![MarketAction::CancelOffer {
                seat: 0,
                offer_id: 999,
            }],
            pass_b_actions: vec![MarketAction::CancelOffer {
                seat: 0,
                offer_id: 998,
            }],
            bundle: SealedOrderBundle::default(),
        },
    )]);
    let bad_outcome = must_ok(
        act_host.apply_act_submissions(&bad_own_subs),
        "apply bad own actions",
    );
    assert_eq!(
        bad_outcome.seats.get(&0).map(|r| (
            r.refused_market_actions.len(),
            r.refused_pass_b_actions.len()
        )),
        Some((1, 1))
    );
    assert!(
        act_host
            .write_act_recorded_seal(&tmp.join("no_pre_ckpts"))
            .err()
            .is_some_and(|e| e.to_string().contains("MISSING_PRE_ACT_CHECKPOINT"))
    );
    let (reveal_save, reveal_seal) =
        must_ok(act_host.write_game_save_and_seal(&tmp), "save reveal pair");

    let mut h_pre = must_ok(
        EvalHost::from_standard_setup(TableId(920_042), 4, 7, 1),
        "setup3",
    );
    let ckpt_dir = tmp.join("ckpts_42");
    let (pre_save, pre_seal) = must_ok(h_pre.write_pre_act_checkpoint(&ckpt_dir), "write pre");
    must_ok(h_pre.apply_act_submissions(&BTreeMap::new()), "apply empty");

    let pre_seal_bak = fs::read(&pre_seal).unwrap_or_default();
    let _ = fs::remove_file(&pre_seal);
    assert!(
        h_pre
            .write_act_recorded_seal(&ckpt_dir)
            .err()
            .is_some_and(|e| e.to_string().contains("PRE_ACT_CHECKPOINT_MISMATCH"))
    );
    must_ok(fs::write(&pre_seal, b"tampered"), "write tampered seal");
    assert!(
        h_pre
            .write_act_recorded_seal(&ckpt_dir)
            .err()
            .is_some_and(|e| e.to_string().contains("PRE_ACT_CHECKPOINT_MISMATCH"))
    );
    must_ok(fs::write(&pre_seal, &pre_seal_bak), "restore pre_seal");
    let _ = fs::remove_file(&pre_save);
    assert!(
        h_pre
            .write_act_recorded_seal(&ckpt_dir)
            .err()
            .is_some_and(|e| e.to_string().contains("PRE_ACT_CHECKPOINT_MISMATCH"))
    );

    let mut h_fork = must_ok(
        EvalHost::read_save_and_seal(&reveal_save, &reveal_seal),
        "load reveal",
    );
    let _ = must_ok(h_fork.sweep_turn(), "sweep to turn 2");
    let t2_pair = must_ok(
        h_fork.write_game_save_and_seal(&tmp.join("turn2_dir")),
        "write turn 2",
    );
    assert!(
        EvalHost::fork_duplicate_from_turn_0(&t2_pair.0, &t2_pair.1)
            .err()
            .is_some_and(|e| e.to_string().contains("INVALID_TURN_0_CHECKPOINT"))
    );
    let _ = must_ok(
        h_fork.write_pre_act_checkpoint(&tmp.join("ckpts_42_t2")),
        "write pre turn 2",
    );
    assert!(
        h_fork
            .read_recorded_act_submissions(&tmp.join("does_not_exist.seal"))
            .err()
            .is_some_and(|e| e
                .to_string()
                .contains("failed to read Eval recorded-act seal"))
    );
    assert!(
        h_fork
            .read_recorded_act_submissions(&reveal_seal)
            .err()
            .is_some_and(|e| {
                e.to_string()
                    .contains("MISSING_ACT_RECORDED_CHECKPOINT_SHA256")
            })
    );

    let turn0 = must_ok(
        EvalHost::from_standard_setup(TableId(920_043), 4, 7, 1),
        "setup4",
    );
    let t0_dir = tmp.join("t0_dir");
    let _ = must_ok(turn0.write_game_save_and_seal(&t0_dir), "write t0");
    let dup = must_ok(
        EvalHost::fork_duplicate_from_game_dir(&t0_dir, TableId(920_043)),
        "fork_duplicate_from_game_dir",
    );
    assert_ne!(dup.game_id(), turn0.game_id());

    assert!(
        EvalHost::read_save_and_seal(&tmp.join("no.save"), &reveal_seal)
            .err()
            .is_some_and(|e| e.to_string().contains("failed to read Eval save"))
    );
    assert!(
        EvalHost::read_save_and_seal(&reveal_save, &tmp.join("no.seal"))
            .err()
            .is_some_and(|e| e.to_string().contains("failed to read Eval seal"))
    );
    assert!(
        EvalHost::fork_from_checkpoint(&tmp.join("no.save"), &reveal_seal)
            .err()
            .is_some_and(|e| e
                .to_string()
                .contains("failed to read Eval checkpoint save"))
    );
    assert!(
        EvalHost::fork_from_checkpoint(&reveal_save, &tmp.join("no.seal"))
            .err()
            .is_some_and(|e| e
                .to_string()
                .contains("failed to read Eval checkpoint seal"))
    );

    let file_parent = tmp.join("regular_file_parent");
    must_ok(fs::write(&file_parent, b"x"), "write parent file");
    assert!(turn0.write_game_save_and_seal(&file_parent).is_err());

    let existing_target_dir = tmp.join("existing_dir_as_file");
    must_ok(
        fs::create_dir_all(&existing_target_dir),
        "mkdir existing target",
    );
    assert!(
        turn0
            .write_save_and_seal(&existing_target_dir, &tmp.join("ok.seal"))
            .is_err()
    );

    let _ = fs::remove_dir_all(&tmp);
}
