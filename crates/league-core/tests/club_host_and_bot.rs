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
            .own_mandate()
            .copied()
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
            .own_mandate()
            .copied()
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
        .own_mandate()
        .copied()
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

#[test]
fn club_bot_exercises_stale_cancels_debt_repay_checks_close_variety_council_charter_and_import() {
    use league_core::{
        CheckStatus, CredOffer, CreditCheck, CreditTier, MandateKind, MarketAction, OfferSide,
    };

    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x81 + i; 32])).collect();
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x8F; 32]), "eval_seed");
    let host = must_ok(
        ClubHost::from_explicit_setup(TableId(81_801), 4, 7, [0x71; 32], &salts, eval_seed, 5),
        "host setup",
    );

    let bot = ClubBot::new(11);
    let base_obs = must_ok(host.observe_seat(Seat(0)), "observe(0)");
    let mandate = base_obs
        .own_mandate()
        .copied()
        .unwrap_or_else(|| panic!("Seat(0) missing own_mandate"));

    // Case A: wash-frozen seat cancels stale Buy/Raw Sell and own upkeep Sell, then returns early.
    let mut frozen_obs = base_obs.clone();
    let upkeep_g = frozen_obs.spec.seats[0].upkeep_good;
    frozen_obs.turn = 3;
    frozen_obs.seats[0].wash_frozen_until_turn = 3;
    frozen_obs.offers.insert(
        101,
        CredOffer {
            offer_id: 101,
            maker: 0,
            side: OfferSide::Buy,
            good: AssetId::Raw(1),
            remaining_units: 500,
            price_per_unit_cred: 1_000,
            locked_cred: 500,
            domain_id: None,
            is_hybrid: false,
            created_turn: 1,
        },
    );
    frozen_obs.offers.insert(
        102,
        CredOffer {
            offer_id: 102,
            maker: 0,
            side: OfferSide::Sell,
            good: AssetId::Raw(0),
            remaining_units: 500,
            price_per_unit_cred: 1_000,
            locked_cred: 0,
            domain_id: None,
            is_hybrid: false,
            created_turn: 1,
        },
    );
    frozen_obs.offers.insert(
        103,
        CredOffer {
            offer_id: 103,
            maker: 0,
            side: OfferSide::Sell,
            good: AssetId::Refined(upkeep_g),
            remaining_units: 400,
            price_per_unit_cred: 1_000,
            locked_cred: 0,
            domain_id: None,
            is_hybrid: false,
            created_turn: 3,
        },
    );
    let pass_a_frozen = bot.plan_pass_a(&frozen_obs, &mandate);
    assert_eq!(pass_a_frozen.len(), 3);
    assert!(
        pass_a_frozen
            .iter()
            .all(|a| matches!(a, MarketAction::CancelOffer { seat: 0, .. }))
    );

    // Case B: near close, due check, CRED debt, turn-12 council vote + charter bid, variety buffer.
    let mut rich_obs = base_obs.clone();
    rich_obs.turn = 12;
    rich_obs
        .baseline_predicates
        .season_end
        .turns_until_latest_close = 2;
    rich_obs.seats[0].cred = 50_000;
    rich_obs.seats[0].cred_debt = 3_000;
    rich_obs.seats[0].raw[0] = 20_000;
    rich_obs.seats[0].refined[0] = 20_000;
    rich_obs.seats[0].refined[1] = 20_000;
    rich_obs.checks.insert(
        77,
        CreditCheck {
            check_id: 77,
            drawer: 1,
            payee: 0,
            asset: AssetId::Cred,
            send_max: 500,
            deliver_min: 100,
            due_turn: 10,
            expiration_turn: 15,
            status: CheckStatus::Open,
        },
    );
    let variety_mandate = league_core::PrivateMandate {
        kind: MandateKind::MinRefinedVariety {
            distinct_goods: 2,
            min_each: 1_500,
        },
        bonus_equity: 4_500,
    };
    let decision = bot.decide_full_turn(&rich_obs, &variety_mandate);
    assert!(!decision.pass_a_actions.is_empty());
    assert!(decision.bundle.check_cashes.contains(&77));
    assert_eq!(decision.bundle.debt_repayment_cred, 3_000);
    assert!(decision.bundle.council_vote.is_some());
    assert_eq!(decision.bundle.charter_bids.len(), 1);

    // Case C: Pass B TownImport triggers under long horizon + CreditTier::D, skipped at horizon 1.
    let mut import_obs = base_obs;
    import_obs.seats[0].cred = 50_000;
    import_obs.seats[0].credit_tier = CreditTier::D;
    import_obs.seats[0].refined[usize::from(upkeep_g)] = 0;
    import_obs.baseline_predicates.upkeep.shortfall_units = 1_000;
    import_obs.baseline_predicates.upkeep.owed_units = 1_000;
    import_obs
        .baseline_predicates
        .season_end
        .turns_until_latest_close = 10;
    let (pass_b_import, _) = bot.plan_pass_b_and_bundle(&import_obs, &mandate);
    assert!(
        pass_b_import
            .iter()
            .any(|a| matches!(a, MarketAction::TownImport { seat: 0, .. }))
    );

    import_obs
        .baseline_predicates
        .season_end
        .turns_until_latest_close = 1;
    let (pass_b_skip, _) = bot.plan_pass_b_and_bundle(&import_obs, &mandate);
    assert!(pass_b_skip.is_empty());
}

#[test]
fn replay_layout_seed_errors_and_sealed_accessors_round_trip() {
    use league_core::{
        ReplayLayoutSeed, ReplayLayoutSeedError, SEALED_LAYOUT_SEED_MIN, SealedReplaySeed,
        SeasonReplayLog, TableSpec, layout_seed_digest,
    };

    let sealed_digest = layout_seed_digest(SEALED_LAYOUT_SEED_MIN);
    let variants = [
        (
            ReplayLayoutSeedError::SealedSeedInClear {
                seed_digest: sealed_digest.clone(),
            },
            "SEALED_SEED_IN_CLEAR",
        ),
        (
            ReplayLayoutSeedError::InvalidSealedPool {
                pool: "dev".to_owned(),
            },
            "INVALID_SEALED_SEED_POOL",
        ),
        (
            ReplayLayoutSeedError::InvalidSealedDigest,
            "INVALID_SEALED_SEED_DIGEST",
        ),
        (
            ReplayLayoutSeedError::NonSealedSeedInSealed { seed: 42 },
            "NON_SEALED_SEED_IN_SEALED",
        ),
    ];
    for (err, expected_code) in variants {
        assert_eq!(err.code(), expected_code);
        assert!(err.to_string().contains(expected_code));
    }

    let valid_json = format!(
        r#"{{"pool":"sealed","digest":"{sealed_digest}","independence_checked_at_write_unverified":true}}"#
    );
    let seed: ReplayLayoutSeed = must_ok(serde_json::from_str(&valid_json), "valid sealed json");
    assert!(seed.is_sealed());
    assert_eq!(seed.clear_seed(), None);
    assert_eq!(seed.pool(), Some("sealed"));
    assert_eq!(seed.sealed_digest(), Some(sealed_digest.as_str()));
    assert_eq!(seed.independence_checked_at_write_unverified(), Some(true));
    let inner: &SealedReplaySeed = seed
        .sealed()
        .unwrap_or_else(|| panic!("sealed() should be Some"));
    assert_eq!(inner.pool(), "sealed");
    assert_eq!(inner.digest(), sealed_digest);
    assert!(inner.independence_checked_at_write_unverified());

    let bad_pool = format!(r#"{{"pool":"dev","digest":"{sealed_digest}"}}"#);
    assert!(serde_json::from_str::<ReplayLayoutSeed>(&bad_pool).is_err());
    let bad_digest = r#"{"pool":"sealed","digest":"not-hex"}"#;
    assert!(serde_json::from_str::<ReplayLayoutSeed>(bad_digest).is_err());

    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x91 + i; 32])).collect();
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x9A; 32]), "eval_seed");
    let (spec, deal, _) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            TableId(81_802),
            4,
            1,
            [0x61; 32],
            &salts,
            eval_seed,
        ),
        "spec setup",
    );
    let err_new_sealed = must_err(
        SeasonReplayLog::new_sealed(spec, 1, deal, vec![], vec![], vec![]),
        "clear seed < 2^63 must fail new_sealed",
    );
    assert!(
        err_new_sealed
            .to_string()
            .contains("NON_SEALED_SEED_IN_SEALED")
    );
}

#[test]
fn table_spec_validation_math_helpers_and_persistence_error_branches() {
    use league_core::types::*;

    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0xA1 + i; 32])).collect();
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0xAA; 32]), "eval_seed");
    let (base_spec, _, _) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            TableId(81_901),
            4,
            1,
            [0x72; 32],
            &salts,
            eval_seed,
        ),
        "base_spec",
    );
    assert!(base_spec.validate().is_ok());

    let mut s = base_spec.clone();
    s.seat_count = 3;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats.pop();
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.projects.clear();
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.projects[0].equity_points = 0;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.projects[0].inputs = vec![(99, 100)];
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.closing_hazard_bps[0] = 10_001;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.mode = TableMode::Eval;
    s.shocks.push(ScheduledShockSpec {
        hint_turn: 0,
        forecast_turn: 2,
        realize_turn: 4,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 100,
    });
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.dealt_mandate_commits.pop();
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.dealt_mandate_commits[0] = league_core::CommitmentHash([0; 32]);
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.mode = TableMode::Game;
    s.drand_schedule = None;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.drand_schedule = Some(league_core::DrandSchedule {
        turn_1_close_unix: 1_700_000_000,
        turn_period_secs: 30,
    });
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.shocks.push(ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 3,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 100,
    });
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.mandate_alpha_bps = 0;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.mandate_deck.truncate(1);
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.mandate_deck[0].difficulty_completion_bps = 10_001;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.town_import_premium_bps = 10_000;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.works_stipend_per_unit_cred = -1;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.cancel_lock_bps = 10_001;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.wash_round_trip_min_units = 0;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.wash_freeze_bond_cred = 0;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.wash_freeze_bounty_cred = -1;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].seat = 9;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].initial_cred = -1;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].upkeep_good = 0;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].initial_raw_tariff_bps = 9_999;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].converters.clear();
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].converters[0].owner = 3;
    assert!(s.validate().is_err());

    let mut s = base_spec.clone();
    s.seats[0].converters[0].inputs = vec![(99, 100)];
    assert!(s.validate().is_err());

    let mut s = base_spec;
    s.seats[0].converters[0].inputs = vec![(0, 100)];
    assert!(s.validate().is_err());

    // Arithmetic & seed validation branches in `league_core::types`.
    assert!(town_import_cost_cred(0, 1_000, 22_500).is_err());
    assert!(town_import_cost_cred(1_000, 1_000, 10_000).is_err());
    assert!(town_import_cost_cred(i64::MAX, i64::MAX, 22_500).is_err());
    assert_eq!(ceil_bps_fee(0, 100), 0);
    assert_eq!(floor_bps_part(0, 100), 0);
    assert_eq!(floor_bps_part(1_000, 20_000), 1_000);
    assert_eq!(half_up_bps(0, 100), 0);
    assert_eq!(half_up_bps(1_000, 250), 25);
    assert_eq!(mul_units_parity(0, 1_000), 0);
    assert_eq!(ceil_units_parity(0, 1_000), 0);
    assert!(checked_mul_units_parity(-1, 100).is_err());
    assert_eq!(checked_mul_units_parity(0, 100), Ok(0));
    assert!(checked_mul_units_parity(i64::MAX, i64::MAX).is_err());
    assert!(checked_ceil_units_parity(-1, 100).is_err());
    assert_eq!(checked_ceil_units_parity(0, 100), Ok(0));
    assert!(checked_ceil_units_parity(i64::MAX, i64::MAX).is_err());
    assert_eq!(scale_bps_signed(-500, 10_000), -500);
    assert!(scale_bps_signed(-500, 9_500) <= -475);

    assert!(validate_club_dev_seed(1_000_000).is_err());
    assert!(validate_club_dev_seed(1_u64 << 63).is_err());
    assert!(validate_club_dev_seed(10_000).is_err());
    assert!(validate_club_dev_seed((1_u64 << 63) | 7).is_err());

    // Persistence missing-file error paths + idempotent wipe_seal_file + from_standard_setup.
    let tmp = std::env::temp_dir().join("ml_card5_missing_save_seal_probe");
    let _ = std::fs::remove_dir_all(&tmp);
    must_ok(std::fs::create_dir_all(&tmp), "mkdir tmp");
    let save_p = tmp.join("missing.save.json");
    let seal_p = tmp.join("missing.seal");
    assert!(ClubHost::read_save_and_seal(&save_p, &seal_p).is_err());
    must_ok(std::fs::write(&save_p, b"{}"), "write save dummy");
    assert!(ClubHost::read_save_and_seal(&save_p, &seal_p).is_err());
    must_ok(ClubHost::wipe_seal_file(&seal_p), "wipe absent seal ok");
    let _ = std::fs::remove_dir_all(&tmp);

    let std_host = must_ok(
        ClubHost::from_standard_setup(TableId(81_999), 4, 1, 0),
        "from_standard_setup",
    );
    assert_eq!(std_host.seat_count(), 4);
}

#[test]
fn debug_and_pretty_debug_redact_club_checkpoint_mandate_and_reveals() {
    use league_core::club::host::{ClubPauseCursor, ClubSessionCheckpoint};
    use league_core::{
        MandateKind, MandateReveal, PrivateMandate, SealedMandateDeal, SealedOrderBundle,
        ToStructuredError,
    };

    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x7A + i; 32])).collect();
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x6D; 32]), "eval_seed");
    let host = must_ok(
        ClubHost::from_explicit_setup(TableId(903_701), 4, 7, [0x4B; 32], &salts, eval_seed, 55),
        "host setup",
    );
    let obs = must_ok(host.observe_seat(Seat(0)), "observe seat 0");

    let ckpt = ClubSessionCheckpoint {
        pause_cursor: Some(ClubPauseCursor {
            sub_pass_b: false,
            pass_offset: 1,
            curtain_next_seat: Some(1),
        }),
        pending_bundles: vec![SealedOrderBundle {
            debt_repayment_cred: 777_111,
            ..SealedOrderBundle::default()
        }],
        sub_pass_b_committed: vec![false, true, false, false],
        seat_notes: vec!["PLANTED_CKPT_NOTE_8421".to_owned()],
        seat_pins: vec![Some("9317".to_owned()), None, None, None],
        pins_enabled: true,
        turn_clock_secs: Some(210),
        untimed: false,
    };
    let deal = SealedMandateDeal {
        deal_seed: [0x6C; 32],
        seat_reveals: vec![MandateReveal {
            mandate: PrivateMandate {
                kind: MandateKind::MinProjectEquity { target: 654_987 },
                bonus_equity: 4_500,
            },
            salt: RevealSalt([0x7F; 32]),
        }],
    };
    let order_rev = OrderReveal {
        bundle: SealedOrderBundle {
            debt_repayment_cred: 888_222,
            ..SealedOrderBundle::default()
        },
        salt: RevealSalt([0x5E; 32]),
    };

    for dbg in [format!("{ckpt:?}"), format!("{ckpt:#?}")] {
        assert!(
            dbg.contains("<redacted>"),
            "ckpt missing <redacted>:\n{dbg}"
        );
        for secret in ["777111", "PLANTED_CKPT_NOTE_8421", "9317"] {
            assert!(!dbg.contains(secret), "ckpt leaked `{secret}`:\n{dbg}");
        }
    }
    for dbg in [format!("{host:?}"), format!("{host:#?}")] {
        assert!(dbg.contains("<hidden>"), "host missing <hidden>:\n{dbg}");
    }
    for dbg in [format!("{obs:?}"), format!("{obs:#?}")] {
        assert!(dbg.contains("<redacted>"), "obs missing <redacted>:\n{dbg}");
    }
    for dbg in [format!("{deal:?}"), format!("{deal:#?}")] {
        assert!(
            dbg.contains("[REDACTED]"),
            "deal missing [REDACTED]:\n{dbg}"
        );
        assert!(!dbg.contains("6c6c6c"));
        assert!(!dbg.contains("654987"));
    }
    for dbg in [format!("{order_rev:?}"), format!("{order_rev:#?}")] {
        assert!(dbg.contains("<redacted>") && dbg.contains("<hidden>"));
        assert!(!dbg.contains("888222"));
        assert!(!dbg.contains("5e5e5e"));
    }
    let err = league_core::EngineError::InvalidConfig("secret_leak_check".to_owned());
    assert_eq!(err.to_structured_error().code, "INVALID_CONFIG");
}

#[test]
fn every_checkpoint_and_seal_validation_rule_refuses_bad_seal_by_name() {
    use league_core::ToStructuredError;

    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x41 + i; 32])).collect();
    let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x5C; 32]), "eval_seed");
    let mut host = must_ok(
        ClubHost::from_explicit_setup(TableId(903_702), 4, 7, [0x4A; 32], &salts, eval_seed, 55),
        "setup",
    );
    let dir = unique_tmp_dir("bad_seal_rules");
    let save_path = dir.join("club-903702.save.json");
    let seal_path = dir.join("club-903702.seal");

    // Populate one Turn sweep digest + checkpoint so every digest & checkpoint field is exercised.
    let bots: Vec<ClubBot> = (0..4).map(|i| ClubBot::new(100 + i as u64)).collect();
    assert_eq!(drive_club_turn_with_bots(&mut host, &bots, 1), 0);
    must_ok(
        host.write_save_and_seal(&save_path, &seal_path),
        "write valid",
    );

    let base_save: serde_json::Value = must_ok(
        serde_json::from_str(&must_ok(std::fs::read_to_string(&save_path), "read save")),
        "parse save",
    );
    let base_seal: serde_json::Value = must_ok(
        serde_json::from_str(&must_ok(std::fs::read_to_string(&seal_path), "read seal")),
        "parse seal",
    );

    let check_case = |idx: usize, expected_code: &str, save_bytes: Vec<u8>, seal_bytes: Vec<u8>| {
        let sp = dir.join(format!("save-{idx}.json"));
        let sl = dir.join(format!("seal-{idx}.seal"));
        must_ok(std::fs::write(&sp, save_bytes), "write save case");
        must_ok(std::fs::write(&sl, seal_bytes), "write seal case");
        let err = must_err(
            ClubHost::read_save_and_seal(&sp, &sl),
            &format!("case {idx} ({expected_code})"),
        );
        let structured = err.to_structured_error();
        let combined = format!("{} | {} | {}", structured.code, err, structured.message);
        assert!(
            combined.contains(expected_code),
            "case {idx}: expected `{expected_code}` in `{combined}`"
        );
    };

    let mut idx = 0_usize;
    let mut run_seal = |expected_code: &str, mutate_seal: &dyn Fn(&mut serde_json::Value)| {
        let mut s = base_seal.clone();
        mutate_seal(&mut s);
        check_case(
            idx,
            expected_code,
            must_ok(serde_json::to_vec_pretty(&base_save), "vec"),
            must_ok(serde_json::to_vec(&s), "vec"),
        );
        idx += 1;
    };

    // Checkpoint rules (one test per rule).
    run_seal("INVALID_CHECKPOINT_PASS_OFFSET", &|s| {
        s["session_pause_cursor"] =
            serde_json::json!({"sub_pass_b": false, "pass_offset": 9, "curtain_next_seat": null});
    });
    run_seal("INVALID_SEAT", &|s| {
        s["session_pause_cursor"] =
            serde_json::json!({"sub_pass_b": false, "pass_offset": 0, "curtain_next_seat": 9});
    });
    run_seal("INVALID_CHECKPOINT_BUNDLES_LEN", &|s| {
        let one = s["session_pending_bundles"][0].clone();
        s["session_pending_bundles"] = serde_json::json!([one.clone(), one]);
    });
    run_seal("INVALID_CHECKPOINT_COMMITTED_LEN", &|s| {
        s["session_sub_pass_b_committed"] = serde_json::json!([false, true]);
    });
    run_seal("INVALID_CHECKPOINT_NOTES_LEN", &|s| {
        s["session_seat_notes"] = serde_json::json!(["a", "b"]);
    });
    run_seal("INVALID_CHECKPOINT_PINS_LEN", &|s| {
        s["session_seat_pins"] = serde_json::json!([null, null]);
    });
    run_seal("INVALID_CHECKPOINT_PINS_LEN", &|s| {
        s["session_pins_enabled"] = serde_json::json!(true);
        s["session_seat_pins"] = serde_json::json!([]);
    });
    run_seal("INVALID_CHECKPOINT_PIN_FORMAT", &|s| {
        s["session_pins_enabled"] = serde_json::json!(true);
        s["session_seat_pins"] = serde_json::json!(["12a4", null, null, null]);
    });
    run_seal("CONFLICTING_CHECKPOINT_CLOCK", &|s| {
        s["session_untimed"] = serde_json::json!(true);
        s["session_turn_clock_secs"] = serde_json::json!(210);
    });
    run_seal("INVALID_CHECKPOINT_TURN_CLOCK", &|s| {
        s["session_turn_clock_secs"] = serde_json::json!(179);
    });
    run_seal("INVALID_CHECKPOINT_TURN_CLOCK", &|s| {
        s["session_turn_clock_secs"] = serde_json::json!(301);
    });

    run_seal("RESERVED_SEED_FORBIDDEN", &|s| {
        s["layout_seed"] = serde_json::json!(1_000_000);
    });
    run_seal("SEED_POOL_MISMATCH", &|s| {
        s["layout_seed"] = serde_json::json!(10_000);
    });
    run_seal("SEAL_LAYOUT_SEED_MISMATCH", &|s| {
        s["layout_seed"] = serde_json::json!(8);
    });
    run_seal("INVALID_SEAL_TABLE_MODE", &|s| {
        s["engine"]["state"]["spec"]["mode"] = serde_json::json!("Eval");
    });
    run_seal("MISSING_SEAL_EVAL_VRF_PUBKEY", &|s| {
        s["engine"]["state"]["spec"]["eval_vrf_pubkey"] = serde_json::Value::Null;
    });
    run_seal("INVALID_SEAL_VRF_PROOF_COUNT", &|s| {
        let Some(arr) = s["vrf_proofs"].as_array_mut() else {
            panic!("missing vrf_proofs array");
        };
        arr.pop();
    });
    run_seal("INVALID_SEAL_VRF_PROOF_LEN", &|s| {
        s["vrf_proofs"][0] = serde_json::json!([]);
    });
    run_seal("INVALID_ORDER", &|s| {
        let Some(arr) = s["vrf_proofs"][0].as_array_mut() else {
            panic!("missing vrf_proofs[0] bytes");
        };
        let Some(first) = arr[0].as_u64() else {
            panic!("missing u64 proof byte");
        };
        arr[0] = serde_json::json!(first ^ 1);
    });
    run_seal("INVALID_SEALED_MANDATE_DEAL", &|s| {
        s["sealed_mandate_deal"]["seat_reveals"][0]["mandate"]["bonus_equity"] =
            serde_json::json!(9_999);
    });
    run_seal("INVALID_SEAL_MANDATE_LEN", &|s| {
        let Some(arr) = s["mandate_commits"].as_array_mut() else {
            panic!("missing mandate_commits array");
        };
        arr.pop();
    });
    run_seal("INVALID_SEAL_DIGEST_LEN", &|s| {
        let Some(arr) = s["live_state_digests"].as_array_mut() else {
            panic!("missing live_state_digests array");
        };
        arr.push(serde_json::json!(vec![1_u8; 32]));
    });
    run_seal("DEGENERATE_COMMIT_SALT_ENTROPY", &|s| {
        s["commit_salt_entropy"] = serde_json::json!("0".repeat(64));
    });
    run_seal("INVALID_CURRENT_TURN_SALTS", &|s| {
        let zero = "0".repeat(64);
        s["current_turn_salts"] = serde_json::json!([zero, zero, zero, zero]);
    });
    run_seal("INVALID_CURRENT_TURN_SALTS", &|s| {
        let one = "1".repeat(64);
        s["current_turn_salts"] = serde_json::json!([one, one]);
    });
    run_seal("UNSUPPORTED_REPLAY_VERSION", &|s| {
        s["format_version"] = serde_json::json!(99);
    });

    check_case(
        idx,
        "MALFORMED_SAVE_JSON",
        b"{not_json".to_vec(),
        must_ok(serde_json::to_vec(&base_seal), "vec"),
    );
    idx += 1;
    check_case(
        idx,
        "MALFORMED_SEAL_JSON",
        must_ok(serde_json::to_vec_pretty(&base_save), "vec"),
        b"{not_json".to_vec(),
    );
    idx += 1;
    // C5: Serde type mismatch inside `.seal` must return MALFORMED_SEAL_JSON without quoting
    // raw field values from the malformed JSON.
    {
        let mut quoted_seal = base_seal.clone();
        quoted_seal["session_turn_clock_secs"] = serde_json::json!("SEAL_VALUE_QUOTE_SECRET_4411");
        let sp = dir.join(format!("save-{idx}.json"));
        let sl = dir.join(format!("seal-{idx}.seal"));
        must_ok(
            std::fs::write(&sp, must_ok(serde_json::to_vec_pretty(&base_save), "vec")),
            "write save c5",
        );
        must_ok(
            std::fs::write(&sl, must_ok(serde_json::to_vec(&quoted_seal), "vec")),
            "write seal c5",
        );
        let err = must_err(ClubHost::read_save_and_seal(&sp, &sl), "c5 seal type err");
        let structured = err.to_structured_error();
        let combined = format!("{} | {} | {}", structured.code, err, structured.message);
        assert!(combined.contains("MALFORMED_SEAL_JSON"), "{combined}");
        assert!(
            !combined.contains("SEAL_VALUE_QUOTE_SECRET_4411"),
            "MALFORMED_SEAL_JSON must never echo malformed seal field values: {combined}"
        );
        idx += 1;
    }
    check_case(
        idx,
        "UNSUPPORTED_REPLAY_VERSION",
        {
            let mut sv = base_save.clone();
            sv["format_version"] = serde_json::json!(99);
            must_ok(serde_json::to_vec_pretty(&sv), "vec")
        },
        must_ok(serde_json::to_vec(&base_seal), "vec"),
    );
    idx += 1;
    check_case(
        idx,
        "SAVE_SEAL_SNAPSHOT_MISMATCH",
        {
            let mut sv = base_save;
            sv["turn"] = serde_json::json!(9);
            must_ok(serde_json::to_vec_pretty(&sv), "vec")
        },
        must_ok(serde_json::to_vec(&base_seal), "vec"),
    );

    let _ = std::fs::remove_dir_all(&dir);
}
