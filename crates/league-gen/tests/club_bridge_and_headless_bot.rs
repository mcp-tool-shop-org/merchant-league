//! Wave 2 Card 3 acceptance tests (`league-gen` + `league-core`):
//! - `ring_table_to_table_spec` bridge (`TableMode::Club`, `SeedPool::Dev`, `is_reserved_test_seed`
//!   refusal, default human `RegentDoctrine`, `GENERATOR_VERSION = "league-gen/v2.5.0"`, dynamic
//!   shocks);
//! - Exhaustive `61`-code `CommodityCatalog` + `CRED` checks and copy-gate word hygiene;
//! - `ToStructuredError` contracts across `RecordedLayoutSeedError`, `FeasibilityError`, `GenError`,
//!   and `LpError` (with `sealed`-seed SHA-256 redaction);
//! - Headless `ClubBot` + `ClubHost` seasons of `24..=32` turns across `N in 4..=6` `dev` seeds
//!   finishing with `0` refused actions, `held_enough_but_short_turns == []`, and
//!   `replay_season_with_state_predicates` bit-identity under `TableMode::Club`.

use std::collections::{BTreeMap, BTreeSet};

use league_core::{
    AssetId, ClubBot, ClubHost, DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE, HARD_TURN_CAP, OrderReveal,
    RESERVED_TEST_LAYOUT_SEEDS, RevealSalt, SealedEvalSeed, SeasonReplayLog, Seat, SeatIdentity,
    TableId, TableMode, ToStructuredError as _, TurnPhase, compute_season_metrics,
    replay_club_season, replay_season_with_state_predicates,
};
use league_gen::{
    ALLOWED_COMMODITY_GLOSSARY, CommodityCatalog, CurrencyCode, FeasibilityError,
    GENERATOR_VERSION, GenError, GeneratorConfig, LpError, NUMERAIRE_DISPLAY_LABEL,
    NUMERAIRE_PLAIN_LABEL, RecordedLayoutSeedError, SeatCount, SeedPool, TablePurpose,
    TableSetupOptions, generate_ring_table_for_setup, is_reserved_test_seed,
    ring_table_to_table_spec, ring_table_to_table_spec_with_explicit_entropy, sealed_seed_sha256,
    seed_pool,
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

fn dev_table_for_seat_count(
    seat_count: SeatCount,
    start_seed: u64,
) -> (u64, league_gen::RingTable) {
    let cfg = GeneratorConfig {
        seat_count: Some(seat_count),
        ..GeneratorConfig::default()
    };
    let opts = TableSetupOptions {
        purpose: TablePurpose::Dev,
        training_logged: false,
    };
    for seed in start_seed..1_000 {
        if is_reserved_test_seed(seed) || seed_pool(seed) != SeedPool::Dev {
            continue;
        }
        if let Ok((checked, _, _)) = generate_ring_table_for_setup(seed, opts, &cfg)
            && let Some(table) = checked.into_clear_table()
        {
            return (seed, table);
        }
    }
    panic!(
        "no feasible dev table found for {:?} from seed {start_seed}",
        seat_count
    );
}

#[test]
fn exhaustive_commodity_catalog_61_entries_plus_cred_and_copy_clean() {
    let entries = CommodityCatalog::entries();
    assert_eq!(entries.len(), 61);
    assert_eq!(entries.len(), ALLOWED_COMMODITY_GLOSSARY.len());

    assert_eq!(NUMERAIRE_DISPLAY_LABEL, "Clearinghouse Marks [CRED]");
    assert_eq!(NUMERAIRE_PLAIN_LABEL, "CRED");
    assert_eq!(
        CommodityCatalog::format_numeraire(false),
        "Clearinghouse Marks [CRED]"
    );
    assert_eq!(CommodityCatalog::format_numeraire(true), "CRED");
    assert!(CommodityCatalog::lookup_str("CRED").is_none());

    let tmb = must_ok(CurrencyCode::from_bytes(*b"TMB"), "TMB code");
    assert_eq!(
        CommodityCatalog::format_raw(tmb, false),
        "Rough Timber [TMB]"
    );
    assert_eq!(
        CommodityCatalog::format_refined(tmb, false),
        "Seasoned Timber [TMB]"
    );
    assert_eq!(CommodityCatalog::format_raw(tmb, true), "TMB");
    assert_eq!(CommodityCatalog::format_refined(tmb, true), "TMB");

    let mut raw_names = BTreeSet::new();
    let mut refined_names = BTreeSet::new();
    let banned_copy_parts: [(&[u8], &[u8]); 15] = [
        (b"in", b"vest"),
        (b"pay", b"out"),
        (b"pay ", b"out"),
        (b"cash", b"out"),
        (b"cash ", b"out"),
        (b"buy-", b"in"),
        (b"buy_", b"in"),
        (b"wallet ", b"value"),
        (b"wa", b"ger"),
        (b"jack", b"pot"),
        (b"real ", b"money"),
        (b"real ", b"profit"),
        (b"$", b""),
        (b"us", b"d"),
        (b"eu", b"r"),
    ];
    let banned_copy_substrings: Vec<String> = banned_copy_parts
        .iter()
        .map(|(lhs, rhs)| {
            let mut bytes = lhs.to_vec();
            bytes.extend_from_slice(rhs);
            must_ok(String::from_utf8(bytes), "utf8 copy pattern")
        })
        .collect();

    for (entry, &(bytes, glossary_word)) in entries.iter().zip(ALLOWED_COMMODITY_GLOSSARY.iter()) {
        assert_eq!(entry.code.as_bytes(), bytes);
        assert_eq!(entry.glossary_word, glossary_word);
        assert_ne!(entry.raw_name, entry.refined_name);
        assert!(
            raw_names.insert(entry.raw_name),
            "duplicate raw_name: {}",
            entry.raw_name
        );
        assert!(
            refined_names.insert(entry.refined_name),
            "duplicate refined_name: {}",
            entry.refined_name
        );

        let looked = CommodityCatalog::lookup(entry.code);
        assert_eq!(looked, entry);
        assert_eq!(CommodityCatalog::lookup_bytes(bytes), Some(entry));
        assert_eq!(
            CommodityCatalog::lookup_str(entry.code.as_str()),
            Some(entry)
        );

        let raw_disp = CommodityCatalog::format_raw(entry.code, false);
        let ref_disp = CommodityCatalog::format_refined(entry.code, false);
        assert!(raw_disp.contains(&format!("[{}]", entry.code)));
        assert!(ref_disp.contains(&format!("[{}]", entry.code)));

        for label in [raw_disp, ref_disp, NUMERAIRE_DISPLAY_LABEL.to_owned()] {
            let lower = label.to_ascii_lowercase();
            for banned in &banned_copy_substrings {
                assert!(
                    !lower.contains(banned.as_str()),
                    "commodity label `{label}` tripped banned copy substring `{banned}`"
                );
            }
        }
    }
}

#[test]
fn league_gen_structured_errors_cover_all_variants_and_redact_sealed_seeds() {
    let sealed_seed = (1_u64 << 63) | 999_999;
    let sealed_digest = sealed_seed_sha256(sealed_seed);

    let rls_errors = [
        RecordedLayoutSeedError::SealedValueInClear {
            sealed_seed_sha256: sealed_digest.clone(),
        },
        RecordedLayoutSeedError::InvalidSealedDigest,
    ];
    for err in &rls_errors {
        let se = err.to_structured_error();
        assert_eq!(se.code, err.code());
        assert!(!se.hint.is_empty());
        assert!(!se.retryable);
        assert!(se.to_json_string().is_ok());
    }

    let feas_errors = [
        FeasibilityError::InvalidTable("bad table".to_owned()),
        FeasibilityError::PairCoversUpkeep {
            pair: (0, 1),
            coverage_ratio: 1.0,
        },
        FeasibilityError::InsufficientSurplus {
            net_surplus: -1.0,
            required_ratio: 0.15,
            actual_ratio: 0.0,
        },
        FeasibilityError::SeatEvOutOfTolerance {
            max_relative_deviation: 0.25,
            tolerance: 0.03,
            seat_evs: vec![1.0, 2.0, 1.0, 1.0],
        },
        FeasibilityError::ShapleyGuardExceeded {
            max_relative_deviation: 0.12,
            max_allowed_deviation: 0.07,
            shapley_seat_evs: vec![1.0, 1.5, 1.0, 1.0],
        },
        FeasibilityError::PairShareExceedsCap {
            pair: (0, 2),
            share: 0.60,
            cap: 0.45,
        },
        FeasibilityError::RolloutGateFailed("shortfall".to_owned()),
        FeasibilityError::SolverFailure("singular".to_owned()),
    ];
    for err in &feas_errors {
        let se = err.to_structured_error();
        assert_eq!(se.code, err.code());
        assert!(!se.hint.is_empty());
        assert!(!se.retryable);
        assert!(se.to_json_string().is_ok());
    }

    let gen_errors = [
        GenError::seed_pool_mismatch(sealed_seed, TablePurpose::Dev),
        GenError::training_logged_mismatch(sealed_seed, TablePurpose::Sealed),
        GenError::InvalidConfig("bad OPENROUTER_API_KEY=sk-or-secret999 config".to_owned()),
        GenError::NoFeasibleTableFound {
            attempts: 4,
            last_error: FeasibilityError::InsufficientSurplus {
                net_surplus: -0.5,
                required_ratio: 0.15,
                actual_ratio: 0.02,
            },
        },
    ];
    for (idx, err) in gen_errors.iter().enumerate() {
        let se = err.to_structured_error();
        assert_eq!(se.code, err.code());
        assert!(!se.hint.is_empty());
        assert!(!se.retryable);
        let text = se.format_text();
        let json = must_ok(se.to_json_string(), "StructuredError json");
        assert!(
            !text.contains(&sealed_seed.to_string()) && !json.contains(&sealed_seed.to_string()),
            "sealed seed appeared in clear in StructuredError #{idx}: {text}"
        );
        assert!(!text.contains("sk-or-secret999") && !json.contains("sk-or-secret999"));
        if idx < 2 {
            assert!(text.contains(&sealed_digest));
        }
        if idx == 3 {
            assert!(se.cause.is_some());
        }
    }

    let lp_err = LpError("dimension mismatch".to_owned());
    let lp_se = lp_err.to_structured_error();
    assert_eq!(lp_se.code, lp_err.code());
    assert_eq!(lp_se.code, "LP_ERROR");
    assert!(!lp_se.retryable);
}

#[test]
fn ring_table_to_table_spec_bridge_enforces_dev_pool_reserved_seeds_and_regent_doctrine() {
    for sc in SeatCount::ALL {
        let (seed, table) = dev_table_for_seat_count(sc, 1);
        assert_eq!(table.generator_version, GENERATOR_VERSION);
        assert_eq!(table.generator_version, "league-gen/v2.5.0");

        let tid = TableId(70_000 + u64::from(sc.as_u8()));
        let (spec, cmap, deal, eval_seed) = must_ok(
            ring_table_to_table_spec(&table, tid, &[0]),
            "ring_table_to_table_spec",
        );
        assert_eq!(spec.mode, TableMode::Club);
        assert_eq!(spec.seat_count, sc.as_u8());
        assert!(
            spec.shocks.is_empty(),
            "Club spec must preserve dynamic shocks"
        );
        assert!(spec.drand_schedule.is_none());
        let expected_pk = must_ok(eval_seed.public_key_bytes(), "eval_seed public_key_bytes");
        assert_eq!(spec.eval_vrf_pubkey, Some(expected_pk));
        assert!(spec.validate().is_ok());

        assert_eq!(spec.seats[0].identity, SeatIdentity::Human);
        assert_eq!(
            spec.seats[0].regent,
            Some(DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE)
        );
        for seat in &spec.seats[1..] {
            assert_eq!(seat.identity, SeatIdentity::AiSyndicate);
            assert_eq!(seat.regent, None);
        }

        assert_eq!(cmap.seats.len(), usize::from(sc.as_u8()));
        assert_eq!(
            cmap.format_asset(AssetId::Cred, false),
            Some("Clearinghouse Marks [CRED]".to_owned())
        );
        assert_eq!(
            cmap.format_asset(AssetId::Raw(0), false),
            Some(cmap.seats[0].raw_label.clone())
        );
        assert_eq!(
            cmap.format_asset(AssetId::Refined(0), false),
            Some(cmap.seats[0].refined_label.clone())
        );

        // Verify deal & eval_seed work end-to-end with ClubHost!
        let host = must_ok(
            ClubHost::new(spec, seed, deal, eval_seed, 42),
            "ClubHost::new from bridged spec",
        );
        assert_eq!(host.table_id(), tid);
        assert_eq!(host.turn(), 1);
    }

    // Refusals: reserved test seeds, non-dev pools, version mismatch, bad human seats, coupled entropy.
    let (valid_seed, valid_table) = dev_table_for_seat_count(SeatCount::Four, 1);
    for &reserved in &RESERVED_TEST_LAYOUT_SEEDS {
        assert!(is_reserved_test_seed(reserved));
        let mut bad = valid_table.clone();
        bad.seed = reserved;
        let err = must_err(
            ring_table_to_table_spec(&bad, TableId(71_001), &[0]),
            "reserved test seed must be refused",
        );
        let se = err.to_structured_error();
        if seed_pool(reserved) == SeedPool::Sealed {
            assert!(
                !se.format_text().contains(&reserved.to_string()),
                "reserved sealed seed must stay redacted"
            );
            assert!(se.format_text().contains(&sealed_seed_sha256(reserved)));
        }
    }

    for non_dev in [
        10_005_u64,
        1_000_005,
        2_000_005,
        (1_u64 << 63) | 777,
        1_500_000,
    ] {
        let mut bad = valid_table.clone();
        bad.seed = non_dev;
        let _ = must_err(
            ring_table_to_table_spec(&bad, TableId(71_002), &[0]),
            "non-dev pool seed must be refused",
        );
    }

    let mut bad_ver = valid_table.clone();
    bad_ver.generator_version = "league-gen/v0.0.0".to_owned();
    let _ = must_err(
        ring_table_to_table_spec(&bad_ver, TableId(71_003), &[0]),
        "wrong generator version must fail",
    );

    let _ = must_err(
        ring_table_to_table_spec(&valid_table, TableId(71_004), &[0, 0]),
        "duplicate human seat must fail",
    );
    let _ = must_err(
        ring_table_to_table_spec(&valid_table, TableId(71_005), &[4]),
        "out-of-range human seat must fail",
    );

    // Explicit entropy coupled to `table.seed` must fail.
    let mut padded = [0u8; 32];
    padded[..8].copy_from_slice(&valid_seed.to_le_bytes());
    let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x21 + i; 32])).collect();
    let ev = must_ok(SealedEvalSeed::try_from_bytes([0x66; 32]), "ev");
    let _ = must_err(
        ring_table_to_table_spec_with_explicit_entropy(
            &valid_table,
            TableId(71_006),
            &[],
            padded,
            &salts,
            ev,
        ),
        "coupled deal_seed must fail",
    );
}

#[test]
fn headless_club_bot_seasons_24_to_32_turns_across_n4_to_6_zero_refused_zero_short_and_replay_identical()
 {
    let mut seasons_tested = 0usize;

    for sc in SeatCount::ALL {
        let mut cursor = 1_u64;
        for replica in 0..2_u64 {
            let (layout_seed, table) = dev_table_for_seat_count(sc, cursor);
            cursor = layout_seed + 1;

            let n = sc.as_u8();
            let tid = TableId(800_000 + u64::from(n) * 100 + replica);
            let bot_tie_base = 5_000 + u64::from(n) * 10 + replica;
            let deal_seed = [0x51_u8.wrapping_add(n).wrapping_add(replica as u8); 32];
            let salts: Vec<RevealSalt> = (0..n)
                .map(|s| RevealSalt([0x21_u8.wrapping_add(s).wrapping_add(replica as u8 * 7); 32]))
                .collect();
            let eval_seed = must_ok(
                SealedEvalSeed::try_from_bytes(
                    [0x71_u8.wrapping_add(n).wrapping_add(replica as u8); 32],
                ),
                "SealedEvalSeed::try_from_bytes",
            );
            let (spec, _cmap, deal, eval_seed) = must_ok(
                ring_table_to_table_spec_with_explicit_entropy(
                    &table,
                    tid,
                    &[],
                    deal_seed,
                    &salts,
                    eval_seed,
                ),
                "ring_table_to_table_spec_with_explicit_entropy",
            );

            // Drive engine STRICTLY through `ClubHost` and bot decisions STRICTLY through
            // `SeatObservation` + `PrivateMandate`.
            let mut host = must_ok(
                ClubHost::new(spec, layout_seed, deal, eval_seed, bot_tie_base),
                "ClubHost::new",
            );
            let bots: Vec<ClubBot> = (0..n)
                .map(|s| ClubBot::new(bot_tie_base + u64::from(s)))
                .collect();

            let mut live_actions_applied = 0usize;
            let mut turns_played = 0_u8;

            for t in 1..=HARD_TURN_CAP {
                if host.phase() != TurnPhase::TalkAndMarket {
                    break;
                }
                turns_played = t;

                for s_idx in 0..n {
                    let seat = Seat(s_idx);
                    let obs = must_ok(host.observe_seat(seat), "observe Pass A");
                    let mandate = *must_ok(host.own_mandate(seat), "own_mandate Pass A");
                    for act in bots[usize::from(s_idx)].plan_pass_a(&obs, &mandate) {
                        must_ok(
                            host.apply_market_action(act),
                            &format!("N={n} seed={layout_seed} turn={t} seat={s_idx} Pass A"),
                        );
                        live_actions_applied += 1;
                    }
                }

                must_ok(host.begin_cancel_lock(), "begin_cancel_lock");

                let mut reveals = BTreeMap::new();
                for s_idx in 0..n {
                    let seat = Seat(s_idx);
                    let obs = must_ok(host.observe_seat(seat), "observe Pass B");
                    let mandate = *must_ok(host.own_mandate(seat), "own_mandate Pass B");
                    let (pass_b_actions, bundle) =
                        bots[usize::from(s_idx)].plan_pass_b_and_bundle(&obs, &mandate);
                    for act in pass_b_actions {
                        must_ok(
                            host.apply_market_action(act),
                            &format!("N={n} seed={layout_seed} turn={t} seat={s_idx} Pass B"),
                        );
                        live_actions_applied += 1;
                    }
                    let salt =
                        RevealSalt([(s_idx.wrapping_mul(29) ^ t).wrapping_add(1).max(1); 32]);
                    reveals.insert(s_idx, OrderReveal { bundle, salt });
                }

                must_ok(
                    host.commit_and_reveal_all(&reveals),
                    "commit_and_reveal_all",
                );
                let _receipt = must_ok(host.sweep_turn(), "sweep_turn");
            }

            assert!(
                (24..=32).contains(&turns_played),
                "N={n} seed={layout_seed} closed at turn {turns_played} outside 24..=32"
            );
            assert!(
                live_actions_applied > 0,
                "bots must actually act over season"
            );
            assert_eq!(host.phase(), TurnPhase::AwaitingMandateReveal);

            let (scoreboard, log, section, metrics) =
                must_ok(host.finalize_club_season(), "finalize_club_season");
            assert_eq!(log.mode, TableMode::Club);
            assert_eq!(log.turns.len(), usize::from(turns_played));
            assert_eq!(metrics.len(), usize::from(n));

            // Verify mode-locked Club replay AND `replay_season_with_state_predicates` bit-identity
            // including JSON round-trip!
            let log_json = must_ok(serde_json::to_vec(&log), "serialize Club log");
            let decoded_log = must_ok(
                SeasonReplayLog::from_club_json_slice(&log_json),
                "SeasonReplayLog::from_club_json_slice",
            );
            let club_out = must_ok(replay_club_season(&decoded_log), "replay_club_season");
            let (pred_out, pred_section) = must_ok(
                replay_season_with_state_predicates(&decoded_log),
                "replay_season_with_state_predicates",
            );
            assert_eq!(club_out.scoreboard, scoreboard);
            assert_eq!(pred_out.scoreboard, scoreboard);
            assert_eq!(pred_out.state_digests, club_out.state_digests);
            assert_eq!(pred_out.receipts, club_out.receipts);
            assert_eq!(pred_section, section);
            assert!(
                pred_out.rejected_market_actions.is_empty(),
                "N={n} seed={layout_seed} rejected actions on replay: {:?}",
                pred_out.rejected_market_actions
            );

            let recomputed_metrics = compute_season_metrics(&pred_section);
            assert_eq!(recomputed_metrics, metrics);
            for m in &metrics {
                assert!(
                    m.held_enough_but_short_turns.is_empty(),
                    "N={n} seed={layout_seed} seat {} had held_enough_but_short_turns: {:?}",
                    m.seat,
                    m.held_enough_but_short_turns
                );
            }

            println!(
                "HEADLESS_SEASON N={n} seed={layout_seed} turns={turns_played} refused=0 held_enough_short=[] replay_bit_identical=true"
            );
            seasons_tested += 1;
        }
    }

    assert_eq!(seasons_tested, 6);
}
