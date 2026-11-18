//! Wave 2 Card 3 host/seat & deterministic `ClubBot` integration suite (`league-core`):
//! - `ClubHost` lifecycle over `TableMode::Club` (`from_explicit_setup` / `from_standard_setup`,
//!   two-pass `TalkAndMarket -> begin_cancel_lock -> Commit -> Reveal -> sweep_turn ->
//!   finalize_club_season -> export_replay_log`);
//! - Mid-season atomic `.save.json` + `.seal` persistence, bit-identical resume parity, secret
//!   isolation checks, tamper rejection, and post-season `wipe_seal_file`;
//! - `ClubBot` determinism, `cancel_locked` / `wash_frozen_until_turn` compliance, zero refused
//!   actions, and empty `held_enough_but_short_turns`.

use std::collections::BTreeMap;

use league_core::{
    AssetId, CLUB_SAVE_FORMAT_VERSION, CLUB_SEAL_FORMAT_VERSION, ClubBot, ClubHost,
    DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE, HARD_TURN_CAP, OrderReveal, RevealSalt, SealedEvalSeed,
    Seat, TableId, TableMode, TurnPhase, replay_club_season,
};

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn must_err<T, E>(res: Result<T, E>, ctx: &str) -> E {
    match res {
        Ok(_) => panic!("{ctx}: expected Err"),
        Err(err) => err,
    }
}

fn drive_club_turn_with_bots(host: &mut ClubHost, bots: &[ClubBot], turn_tag: u8) -> usize {
    let n = host.seat_count();
    let mut refused = 0usize;

    // Pass A: sequential actions before cancel_lock.
    for (s_idx, bot) in bots.iter().enumerate().take(usize::from(n)) {
        let seat = Seat(s_idx as u8);
        let obs = must_ok(host.observe_seat(seat), "observe Pass A");
        let mandate = obs
            .own_mandate
            .unwrap_or_else(|| panic!("seat {s_idx} SeatObservation missing own_mandate Pass A"));
        for action in bot.plan_pass_a(&obs, &mandate) {
            if host.apply_market_action(action).is_err() {
                refused += 1;
            }
        }
    }

    must_ok(host.begin_cancel_lock(), "begin_cancel_lock");

    // Pass B + Phase-2 bundle: post-lock fallback and sealed bundle.
    let mut reveals = BTreeMap::new();
    for (s_idx, bot) in bots.iter().enumerate().take(usize::from(n)) {
        let seat = Seat(s_idx as u8);
        let obs = must_ok(host.observe_seat(seat), "observe Pass B");
        let mandate = obs
            .own_mandate
            .unwrap_or_else(|| panic!("seat {s_idx} SeatObservation missing own_mandate Pass B"));
        let (pass_b_actions, bundle) = bot.plan_pass_b_and_bundle(&obs, &mandate);
        for action in pass_b_actions {
            if host.apply_market_action(action).is_err() {
                refused += 1;
            }
        }
        let salt = RevealSalt([(s_idx as u8).wrapping_mul(17).wrapping_add(turn_tag).max(1); 32]);
        reveals.insert(s_idx as u8, OrderReveal { bundle, salt });
    }

    must_ok(
        host.commit_and_reveal_all(&reveals),
        "commit_and_reveal_all",
    );
    let _receipt = must_ok(host.sweep_turn(), "sweep_turn");
    refused
}

fn unique_tmp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ml_club_{label}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    must_ok(std::fs::create_dir_all(&dir), "create tmp dir");
    dir
}

#[test]
fn club_host_save_resume_parity_secret_isolation_and_tamper_rejection() {
    let eval_bytes = [0x6D; 32];
    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes(eval_bytes),
        "SealedEvalSeed::try_from_bytes",
    );
    let eval_seed_clone = must_ok(
        SealedEvalSeed::try_from_bytes(eval_bytes),
        "SealedEvalSeed::try_from_bytes clone",
    );
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31 + i; 32])).collect();
    let tid = TableId(903_001);

    let mut host_continuous = must_ok(
        ClubHost::from_explicit_setup(tid, 4, 7, [0x4A; 32], &salts, eval_seed, 55),
        "ClubHost::from_explicit_setup",
    );
    let host_for_resume = must_ok(
        ClubHost::from_explicit_setup(tid, 4, 7, [0x4A; 32], &salts, eval_seed_clone, 55),
        "ClubHost::from_explicit_setup 2",
    );

    assert_eq!(host_continuous.layout_seed(), 7);
    assert_eq!(host_continuous.bot_tie_seed(), 55);
    assert_eq!(host_continuous.seat_count(), 4);
    assert_eq!(host_continuous.turn(), 1);
    assert_eq!(host_continuous.phase(), TurnPhase::TalkAndMarket);
    assert!(!host_continuous.cancel_locked());
    assert_eq!(host_continuous.public_spec().mode, TableMode::Club);
    assert_eq!(
        host_continuous.public_spec().seats[0].regent,
        Some(DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE)
    );

    let bots: Vec<ClubBot> = (0..4).map(|i| ClubBot::new(100 + i as u64)).collect();
    let mut host_to_save = host_for_resume;

    let tmp_dir = unique_tmp_dir("save_resume");
    let save_path = tmp_dir.join("club-903001.save.json");
    let seal_path = tmp_dir.join("club-903001.seal");

    // Advance both hosts identically to Turn 6 and save mid-season.
    for t in 1..=5_u8 {
        assert_eq!(drive_club_turn_with_bots(&mut host_continuous, &bots, t), 0);
        assert_eq!(drive_club_turn_with_bots(&mut host_to_save, &bots, t), 0);
    }

    must_ok(
        host_to_save.write_save_and_seal(&save_path, &seal_path),
        "write_save_and_seal",
    );

    let save_text = must_ok(std::fs::read_to_string(&save_path), "read .save.json");
    let seal_text = must_ok(std::fs::read_to_string(&seal_path), "read .seal");

    // `.save.json` must hold strictly public projection without VRF proofs, drawn shocks, reveals,
    // raw TurnEngine, or post_state_digest, and neither file may ever hold the raw 32-byte seed.
    let save_json: serde_json::Value =
        must_ok(serde_json::from_str(&save_text), "parse .save.json");
    assert_eq!(
        save_json.get("format_version").and_then(|v| v.as_u64()),
        Some(u64::from(CLUB_SAVE_FORMAT_VERSION))
    );
    for forbidden_key in [
        "drawn_shocks",
        "newly_drawn_shocks",
        "vrf_proofs",
        "eval_vrf_proof",
        "eval_vrf_secret",
        "mandate_commits",
        "mandate_reveals",
        "dealt_mandate_commits",
        "turn_commits",
        "turn_reveals",
        "last_turn_beacon",
        "last_closing_roll_bps",
        "post_state_digest",
        "live_state_digests",
        "logged_turns",
    ] {
        assert!(
            !save_text.contains(forbidden_key),
            ".save.json must not contain secret/hidden key `{forbidden_key}`"
        );
    }
    let raw_seed_hex: String = eval_bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert!(
        !save_text.contains(&raw_seed_hex) && !seal_text.contains(&raw_seed_hex),
        "neither .save.json nor .seal may write the raw 32-byte EvalVrf seed"
    );

    let seal_json: serde_json::Value = must_ok(serde_json::from_str(&seal_text), "parse .seal");
    assert_eq!(
        seal_json.get("format_version").and_then(|v| v.as_u64()),
        Some(u64::from(CLUB_SEAL_FORMAT_VERSION))
    );

    // Tamper checks via public `read_save_and_seal`: mismatched turn or layout_seed must fail.
    let mut bad_save = save_json.clone();
    bad_save["turn"] = serde_json::json!(99);
    let bad_save_bytes = must_ok(serde_json::to_vec(&bad_save), "serialize bad_save");
    let bad_save_path = tmp_dir.join("bad-save.json");
    must_ok(
        std::fs::write(&bad_save_path, &bad_save_bytes),
        "write bad_save",
    );
    let _ = must_err(
        ClubHost::read_save_and_seal(&bad_save_path, &seal_path),
        "tampered turn in .save.json must be rejected",
    );

    let mut bad_seal = seal_json.clone();
    bad_seal["layout_seed"] = serde_json::json!(8);
    let bad_seal_bytes = must_ok(serde_json::to_vec(&bad_seal), "serialize bad_seal");
    let bad_seal_path = tmp_dir.join("bad-seal.seal");
    must_ok(
        std::fs::write(&bad_seal_path, &bad_seal_bytes),
        "write bad_seal",
    );
    let _ = must_err(
        ClubHost::read_save_and_seal(&save_path, &bad_seal_path),
        "tampered layout_seed in .seal must be rejected",
    );

    // Nit C3: pre-fix v1 seals (lacking `live_state_digests`) must be rejected with
    // `UnsupportedReplayVersion` rather than silently accepted or failing ambiguously.
    let mut v1_seal = seal_json;
    v1_seal["format_version"] = serde_json::json!(1);
    if let Some(obj) = v1_seal.as_object_mut() {
        obj.remove("live_state_digests");
    }
    let v1_seal_bytes = must_ok(serde_json::to_vec(&v1_seal), "serialize v1_seal");
    let v1_seal_path = tmp_dir.join("v1-seal.seal");
    must_ok(
        std::fs::write(&v1_seal_path, &v1_seal_bytes),
        "write v1_seal",
    );
    let v1_err = must_err(
        ClubHost::read_save_and_seal(&save_path, &v1_seal_path),
        "v1 .seal without live_state_digests must be rejected",
    );
    assert_eq!(CLUB_SEAL_FORMAT_VERSION, 2);
    assert_eq!(
        v1_err,
        league_core::EngineError::UnsupportedReplayVersion {
            found: 1,
            newest_known: 2,
        }
    );

    // Resume via `read_save_and_seal` and drive to close (`24..=32` turns), then compare.
    let mut host_resumed = must_ok(
        ClubHost::read_save_and_seal(&save_path, &seal_path),
        "read_save_and_seal",
    );
    // Test atomic overwrite on Windows/Unix as well.
    must_ok(
        host_resumed.write_save_and_seal(&save_path, &seal_path),
        "overwrite save/seal",
    );

    for t in 6..=HARD_TURN_CAP {
        if host_continuous.phase() != TurnPhase::TalkAndMarket {
            break;
        }
        assert_eq!(drive_club_turn_with_bots(&mut host_continuous, &bots, t), 0);
        assert_eq!(drive_club_turn_with_bots(&mut host_resumed, &bots, t), 0);
    }

    assert_eq!(host_continuous.phase(), TurnPhase::AwaitingMandateReveal);
    assert_eq!(host_resumed.phase(), TurnPhase::AwaitingMandateReveal);
    assert_eq!(
        host_continuous.public_receipts(),
        host_resumed.public_receipts()
    );

    let (board_a, log_a, _, metrics_a) =
        must_ok(host_continuous.finalize_club_season(), "finalize A");
    let (board_b, log_b, _, metrics_b) = must_ok(host_resumed.finalize_club_season(), "finalize B");
    assert_eq!(
        board_a, board_b,
        "save/resume scoreboard must match bit-for-bit"
    );
    assert_eq!(
        log_a, log_b,
        "save/resume replay log must match bit-for-bit"
    );
    assert_eq!(metrics_a, metrics_b);
    for m in &metrics_a {
        assert!(
            m.held_enough_but_short_turns.is_empty(),
            "seat {} had held_enough_but_short_turns: {:?}",
            m.seat,
            m.held_enough_but_short_turns
        );
    }

    let replay_out = must_ok(replay_club_season(&log_b), "replay_club_season");
    assert!(replay_out.rejected_market_actions.is_empty());
    assert_eq!(replay_out.scoreboard, board_b);

    must_ok(ClubHost::wipe_seal_file(&seal_path), "wipe_seal_file");
    assert!(!seal_path.exists());
    // Idempotent when already removed.
    must_ok(
        ClubHost::wipe_seal_file(&seal_path),
        "wipe_seal_file idempotent",
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn club_bot_determinism_and_phase_lock_freeze_compliance() {
    let mut host = must_ok(
        ClubHost::from_standard_setup(TableId(903_002), 4, 11, 777),
        "ClubHost::from_standard_setup",
    );
    let bot = ClubBot::new(777);
    assert_eq!(bot.bot_tie_seed(), 777);

    let obs_t1 = must_ok(host.observe_seat(Seat(0)), "observe t1");
    let mandate = obs_t1
        .own_mandate
        .unwrap_or_else(|| panic!("Seat(0) SeatObservation missing own_mandate"));
    let d1 = bot.decide_full_turn(&obs_t1, &mandate);
    let d2 = bot.decide_full_turn(&obs_t1, &mandate);
    assert_eq!(d1, d2, "ClubBot decisions must be strictly deterministic");

    // When `cancel_locked == true`, `plan_pass_a` emits nothing and `plan_pass_b_and_bundle`
    // emits zero `CancelOffer` actions.
    must_ok(host.begin_cancel_lock(), "begin_cancel_lock");
    let obs_locked = must_ok(host.observe_seat(Seat(0)), "observe locked");
    assert!(bot.plan_pass_a(&obs_locked, &mandate).is_empty());
    let (pass_b, _) = bot.plan_pass_b_and_bundle(&obs_locked, &mandate);
    assert!(
        pass_b
            .iter()
            .all(|a| !matches!(a, league_core::MarketAction::CancelOffer { .. }))
    );

    // When `wash_frozen_until_turn >= obs.turn`, `plan_pass_a` must emit zero `PostOffer`s.
    let mut obs_frozen = obs_t1;
    obs_frozen.seats[0].wash_frozen_until_turn = obs_frozen.turn;
    let frozen_pass_a = bot.plan_pass_a(&obs_frozen, &mandate);
    assert!(
        frozen_pass_a
            .iter()
            .all(|a| !matches!(a, league_core::MarketAction::PostOffer { .. }))
    );

    // If `SeatObservation` indicates `Completed` / `AwaitingMandateReveal`, bot returns empty plans.
    let mut obs_closed = obs_t1_clone(&obs_locked);
    obs_closed.phase = TurnPhase::AwaitingMandateReveal;
    let closed_decision = bot.decide_full_turn(&obs_closed, &mandate);
    assert!(closed_decision.pass_a_actions.is_empty());
    assert!(closed_decision.pass_b_actions.is_empty());

    // Sanity check AssetId::Cred is never emitted as a PostOffer good.
    assert!(d1.pass_a_actions.iter().all(|a| match a {
        league_core::MarketAction::PostOffer { order, .. } => order.good != AssetId::Cred,
        _ => true,
    }));
}

fn obs_t1_clone(obs: &league_core::SeatObservation) -> league_core::SeatObservation {
    obs.clone()
}

#[test]
fn seat_observation_carries_only_own_mandate_and_never_rival_mandates() {
    let eval_seed = must_ok(
        SealedEvalSeed::try_from_bytes([0x5A; 32]),
        "SealedEvalSeed::try_from_bytes",
    );
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x81 + i; 32])).collect();
    let host = must_ok(
        ClubHost::from_explicit_setup(TableId(903_003), 4, 7, [0x4D; 32], &salts, eval_seed, 99),
        "ClubHost::from_explicit_setup",
    );
    let mandates: Vec<_> = (0..4_u8)
        .map(|s| *must_ok(host.mandate_for_seat(Seat(s)), "host mandate_for_seat"))
        .collect();
    assert!(
        mandates.windows(2).any(|w| w[0] != w[1]),
        "setup should yield distinct mandates across seats"
    );

    let snap_json = must_ok(
        serde_json::to_string(&must_ok(host.public_save_snapshot(), "snapshot")),
        "serialize snapshot",
    );
    assert!(!snap_json.contains("own_mandate"));

    for s in 0..4_u8 {
        let obs = must_ok(host.observe_seat(Seat(s)), "observe_seat");
        assert_eq!(obs.own_mandate(), Some(&mandates[usize::from(s)]));

        let obs_json = must_ok(serde_json::to_string(&obs), "serialize SeatObservation");
        let own_json = must_ok(
            serde_json::to_string(&mandates[usize::from(s)]),
            "serialize own mandate",
        );
        assert!(obs_json.contains(&own_json));

        for r in 0..4_u8 {
            if r != s && mandates[usize::from(r)] != mandates[usize::from(s)] {
                let rival_json = must_ok(
                    serde_json::to_string(&mandates[usize::from(r)]),
                    "serialize rival mandate",
                );
                assert!(
                    !obs_json.contains(&rival_json),
                    "seat {s} SeatObservation leaked rival seat {r}'s mandate"
                );
            }
        }
        let salt_hex: String = salts[usize::from(s)]
            .0
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(!obs_json.contains(&salt_hex));

        let pub_preview_json = must_ok(
            serde_json::to_string(&obs.to_public_preview_state()),
            "serialize PublicPreviewState",
        );
        assert!(!pub_preview_json.contains("own_mandate"));
    }
}
