//! Evaluation Mode driver integration tests (Card 10 Part B Items 10–11, Part C Items 12–15, R7–R9).
//!
//! All tests run offline using `NoopTransport`, `MockDirTransport`, or scripted `Transport` mocks.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use league_core::{EvalHost, HARD_TURN_CAP, PublicSweepReceipt, TableId, replay_eval_season};
use league_eval::{
    BranchOptions, EVAL_REPLY_WORDING_ARM, EvalArm, EvalDecisionRecord, EvalError, EvalManifest,
    NoopTransport, PlayOptions, RefusalCodeCounts, SelectOptions, SelectedPreActState,
    SelectionList, TurnStratum, WrongPartCounter, execute_cli, persist_eval_game_state,
    run_branch_with_transport, run_play_with_transport, run_select,
};
use league_relay::{RawHttpResponse, Transport, TransportError};
use serde_json::{Value, json};

fn must_ok<T, E: Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(e) => panic!("{ctx}: {e}"),
    }
}

fn must_some<T>(opt: Option<T>, ctx: &str) -> T {
    match opt {
        Some(v) => v,
        None => panic!("expected Some({ctx}), got None"),
    }
}

fn fresh_tmp_dir(label: &str) -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ml_eval_pb_{label}_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    must_ok(fs::create_dir_all(&dir), "create temp dir");
    dir
}

fn write_wrapper(dir: &Path) -> PathBuf {
    let p = dir.join("wrapper.md");
    must_ok(
        fs::write(
            &p,
            "# Wrapper v1\nParticipate honestly under Merchant League v2 rules.\n",
        ),
        "write wrapper",
    );
    p
}

fn write_seats(dir: &Path, name: &str, value: Value) -> PathBuf {
    let p = dir.join(name);
    let txt = must_ok(serde_json::to_string_pretty(&value), "serialize seats");
    must_ok(fs::write(&p, txt), "write seats");
    p
}

fn all_clubbot_seats() -> Value {
    json!({
        "seats": [
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    })
}

fn focal_local_three_clubbot_seats() -> Value {
    json!({
        "seats": [
            {
                "guild": "Foundry",
                "backend": "local",
                "base_url": "http://127.0.0.1:18080/v1",
                "model": "Qwen/Qwen3-32B",
                "temperature": 0.0,
                "max_output_tokens": 512,
                "timeout_secs": 5.0,
                "max_gpu_secs_per_call": 10.0
            },
            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    })
}

struct DeterministicMockTransport {
    recorded_chat_bodies: Mutex<Vec<String>>,
    /// Maps `"day_u8:round"` (`"1:talk"`, `"2:act"`, ...) to reply content JSON.
    script_by_turn: Mutex<BTreeMap<String, String>>,
}

impl DeterministicMockTransport {
    fn new(items: Vec<(&str, String)>) -> Self {
        let mut map = BTreeMap::new();
        for (k, v) in items {
            map.insert((*k).to_owned(), v);
        }
        Self {
            recorded_chat_bodies: Mutex::new(Vec::new()),
            script_by_turn: Mutex::new(map),
        }
    }

    fn recorded_bodies(&self) -> Vec<String> {
        let g = match self.recorded_chat_bodies.lock() {
            Ok(g) => g,
            Err(e) => panic!("lock poisoned: {e}"),
        };
        g.clone()
    }
}

impl Transport for DeterministicMockTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        {
            let mut rec = self
                .recorded_chat_bodies
                .lock()
                .map_err(|_| TransportError::Network("lock poisoned".to_owned()))?;
            rec.push(body.to_owned());
        }
        let v: Value =
            serde_json::from_str(body).map_err(|e| TransportError::Network(e.to_string()))?;
        let user_msg = v["messages"]
            .as_array()
            .and_then(|arr| arr.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default();
        let is_talk = user_msg.contains("This is the talk round.");
        let day = if user_msg.contains("\"turn\": 3") {
            3
        } else if user_msg.contains("\"turn\": 2") {
            2
        } else {
            1
        };
        let key = format!("{day}:{}", if is_talk { "talk" } else { "act" });
        let map = self
            .script_by_turn
            .lock()
            .map_err(|_| TransportError::Network("lock poisoned".to_owned()))?;
        let content = map.get(&key).cloned().unwrap_or_else(|| {
            json!({"messages":[],"market_actions":[],"bundle":{},"notes":"fallback"}).to_string()
        });

        let resp = json!({
            "id": "mock-det-1",
            "model": "Qwen/Qwen3-32B",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 40, "completion_tokens": 15, "total_tokens": 55 }
        });
        Ok(RawHttpResponse {
            status: 200,
            body: resp.to_string(),
        })
    }

    fn count_tokens(
        &self,
        _tokenize_url: &str,
        _model: &str,
        text: &str,
        _timeout: Duration,
    ) -> Result<usize, TransportError> {
        Ok(text.split_whitespace().count())
    }
}

fn collect_all_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_all_files(&p, out);
        } else {
            out.push(p);
        }
    }
    out.sort();
}

#[test]
fn item10_full_32_turn_clubbot_season_exports_verified_replay_log() {
    let tmp = fresh_tmp_dir("item10_full");
    let game_dir = tmp.join("full_game");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", all_clubbot_seats());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = execute_cli(
        [
            "play".into(),
            "--game-dir".into(),
            game_dir.clone().into_os_string(),
            "--seats".into(),
            seats_path.into_os_string(),
            "--wrapper".into(),
            wrapper_path.into_os_string(),
            "--arm".into(),
            "off".into(),
            "--table-id".into(),
            "10".into(),
            "--seed".into(),
            "7".into(),
            "--turns".into(),
            "32".into(),
        ],
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(
        code,
        0,
        "CLI play exited {code}: stderr={}",
        String::from_utf8_lossy(&stderr)
    );
    let out_str = String::from_utf8_lossy(&stdout);
    assert!(out_str.contains("arm=off"), "stdout missing arm: {out_str}");

    let manifest: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("manifest.json")),
            "read manifest",
        )),
        "parse manifest",
    );
    assert!(manifest.played_turns >= 20 && manifest.played_turns <= HARD_TURN_CAP);

    let replay_path = game_dir.join("eval-10.replay.json");
    assert!(replay_path.exists(), "missing eval-10.replay.json");
    assert!(game_dir.join("scoreboard.json").exists());
    assert!(game_dir.join("metrics.json").exists());

    let replay_bytes = must_ok(fs::read(&replay_path), "read replay log");
    let replay_outcome = must_ok(
        replay_eval_season(
            &serde_json::from_slice(&replay_bytes).unwrap_or_else(|e| panic!("{e}")),
        ),
        "replay_eval_season",
    );
    assert_eq!(
        replay_outcome.state_digests.len(),
        usize::from(manifest.played_turns)
    );
    assert_eq!(replay_outcome.scoreboard.standings.len(), 4);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn item11_duplicate_tables_determinism_and_r5_r8_r9_checks() {
    let tmp = fresh_tmp_dir("item11_dup");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());

    // Prepare turn-0 sealed source once using EvalHost + persist_eval_game_state (`R8`).
    let turn0_host = must_ok(
        EvalHost::from_standard_setup(TableId(11), 4, 7, 12345),
        "from_standard_setup",
    );
    let game_a = tmp.join("dup_a");
    let game_b = tmp.join("dup_b");
    let (save_a, seal_a) = must_ok(
        persist_eval_game_state(&turn0_host, &game_a),
        "persist game_a",
    );
    must_ok(fs::create_dir_all(&game_b), "mkdir game_b");
    must_ok(
        fs::copy(
            &save_a,
            game_b.join(must_some(save_a.file_name(), "save file name")),
        ),
        "copy turn0 save to dup_b",
    );
    must_ok(
        fs::copy(
            &seal_a,
            game_b.join(must_some(seal_a.file_name(), "seal file name")),
        ),
        "copy turn0 seal to dup_b",
    );

    let script = vec![
        (
            "1:talk",
            json!({
                "messages": [{ "to_guild": "Quarry", "body": "day 1 hi" }],
                "market_actions": [],
                "bundle": {},
                "notes": "d1 talk"
            })
            .to_string(),
        ),
        (
            "1:act",
            json!({
                "messages": [],
                "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 0 }, "amount": 25 }],
                "bundle": {},
                "notes": "d1 act"
            })
            .to_string(),
        ),
        (
            "2:talk",
            json!({
                "messages": [{ "to_guild": "Harbor", "body": "day 2 hi" }],
                "market_actions": [],
                "bundle": {},
                "notes": "d2 talk"
            })
            .to_string(),
        ),
        (
            "2:act",
            json!({
                "messages": [],
                "market_actions": [],
                "bundle": { "debt_repayment_cred": 10 },
                "notes": "d2 act"
            })
            .to_string(),
        ),
        (
            "3:talk",
            json!({
                "messages": [],
                "market_actions": [],
                "bundle": {},
                "notes": "d3 talk"
            })
            .to_string(),
        ),
        (
            "3:act",
            json!({
                "messages": [],
                "market_actions": [],
                "bundle": {},
                "notes": "d3 act"
            })
            .to_string(),
        ),
    ];

    let transport_a = DeterministicMockTransport::new(script.clone());
    let transport_b = DeterministicMockTransport::new(script);

    let opts_a = PlayOptions {
        game_dir: game_a.clone(),
        seats_path: seats_path.clone(),
        wrapper_path: wrapper_path.clone(),
        wrapper_version: None,
        arm: EvalArm::Off,
        table_id: 11,
        seed: 7,
        turns: 3,
        memory_mode: league_relay::MemoryMode::Snapshot,
        live: false,
        cap_credits: None,
        cap_gpu_secs: None,
        allowed_vm_hosts: Vec::new(),
    };
    let mut opts_b = opts_a.clone();
    opts_b.game_dir = game_b.clone();

    let manifest_a = must_ok(
        run_play_with_transport(&opts_a, &transport_a, None),
        "play dup_a",
    );
    let manifest_b = must_ok(
        run_play_with_transport(&opts_b, &transport_b, None),
        "play dup_b",
    );

    // R5: distinct per-game `game_id` across duplicate games even when forked from identical turn 0.
    assert_ne!(manifest_a.game_id, manifest_b.game_id);

    // Wall-clock / per-instance metadata differences normalized here for determinism comparison:
    //   1. `setup.json` / `manifest.json` / `decisions/*.json`: `game_id` (32-hex OS-CSPRNG ID per duplicate run)
    //   2. `relay/calls.jsonl`: `run_id` and `time_utc` RFC3339 timestamps, `latency_ms` / elapsed timings
    assert_eq!(manifest_a.played_turns, 3);
    assert_eq!(manifest_b.played_turns, 3);
    assert_eq!(manifest_a.seats, manifest_b.seats);
    assert_eq!(
        manifest_a.refusal_counts_by_seat,
        manifest_b.refusal_counts_by_seat
    );
    assert_eq!(
        manifest_a.wrong_part_refusals,
        manifest_b.wrong_part_refusals
    );

    // Check R9 checkpoint timing & byte-exact receipt/view/decision parity across all 3 turns.
    for day in 1..=3_u8 {
        let day_str = format!("{day:02}");
        for suffix in ["pre-act.save.json", "pre-act.seal", "act-recorded.seal"] {
            assert!(
                game_a
                    .join(format!("checkpoints/day-{day_str}-{suffix}"))
                    .exists(),
                "game_a missing day-{day_str}-{suffix}"
            );
            assert!(
                game_b
                    .join(format!("checkpoints/day-{day_str}-{suffix}"))
                    .exists(),
                "game_b missing day-{day_str}-{suffix}"
            );
        }

        // Verify pre-act checkpoint recorded BEFORE any act actions (`phase == TalkAndMarket`, zero applied actions).
        let pre_host = must_ok(
            EvalHost::fork_from_checkpoint(
                &game_a.join(format!("checkpoints/day-{day_str}-pre-act.save.json")),
                &game_a.join(format!("checkpoints/day-{day_str}-pre-act.seal")),
            ),
            "fork pre-act checkpoint",
        );
        assert_eq!(pre_host.turn(), day);
        assert_eq!(pre_host.phase(), league_core::TurnPhase::TalkAndMarket);
        assert!(!pre_host.cancel_locked());

        // Byte-exact receipts, public save snapshots, and rendered prompts across dup_a & dup_b.
        assert_eq!(
            must_ok(
                fs::read(game_a.join(format!("receipts/day-{day_str}.receipt.json"))),
                "read ra"
            ),
            must_ok(
                fs::read(game_b.join(format!("receipts/day-{day_str}.receipt.json"))),
                "read rb"
            ),
        );
        for round_lbl in ["talk", "act"] {
            for guild in ["Foundry", "Quarry", "Harbor", "Mint"] {
                let rel_prompt = format!("to-seat/day-{day_str}-{round_lbl}-{guild}.md");
                assert_eq!(
                    must_ok(fs::read(game_a.join(&rel_prompt)), "read prompt a"),
                    must_ok(fs::read(game_b.join(&rel_prompt)), "read prompt b"),
                );

                let rel_dec = format!("decisions/day-{day_str}-{round_lbl}-{guild}.json");
                let mut dec_a: EvalDecisionRecord = must_ok(
                    serde_json::from_str(&must_ok(
                        fs::read_to_string(game_a.join(&rel_dec)),
                        "dec a",
                    )),
                    "parse dec a",
                );
                let mut dec_b: EvalDecisionRecord = must_ok(
                    serde_json::from_str(&must_ok(
                        fs::read_to_string(game_b.join(&rel_dec)),
                        "dec b",
                    )),
                    "parse dec b",
                );
                assert_eq!(dec_a.game_id, manifest_a.game_id);
                assert_eq!(dec_b.game_id, manifest_b.game_id);
                dec_a.game_id.clear();
                dec_b.game_id.clear();
                assert_eq!(dec_a, dec_b, "normalized decision diverged at {rel_dec}");
            }
        }
    }

    // Identical wire chat-completion request bodies across dup_a & dup_b!
    assert_eq!(transport_a.recorded_bodies(), transport_b.recorded_bodies());

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r7_refused_bundle_records_all_legs_and_leg_not_named() {
    let tmp = fresh_tmp_dir("r7_bundle");
    let game_dir = tmp.join("game");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());

    // Multi-leg bundle: legitimate debt repayment + converter run + over-capped DualEscrow
    // (combined bond > payment_cred -> `validate_bundle_orders` refuses the whole bundle with `INVALID_ORDER`).
    let talk_reply = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {},
        "notes": "n"
    })
    .to_string();
    let act_bad_bundle = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {
            "debt_repayment_cred": 10,
            "converter_runs": [{ "recipe_idx": 0, "runs": 1 }],
            "dual_escrows": [{
                "proposal_id": 1,
                "buyer_seat": 0,
                "seller_seat": 1,
                "good": { "asset_kind": "raw", "good_index": 0 },
                "required_units": 10,
                "payment_cred": 100,
                "buyer_bond_cred": 80,
                "seller_bond_cred": 80,
                "deadline_turn": 3,
                "domain_id": null
            }]
        },
        "notes": "bad multi-leg"
    })
    .to_string();

    let transport =
        DeterministicMockTransport::new(vec![("1:talk", talk_reply), ("1:act", act_bad_bundle)]);
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 77,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &transport,
            None,
        ),
        "run_play refused_bundle",
    );

    let dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-01-act-Foundry.json")),
            "read Foundry act decision",
        )),
        "parse decision",
    );
    let rb = must_some(dec.refused_bundle, "refused_bundle must be recorded");
    assert_eq!(rb.bad_leg_index, "leg not named");
    assert_eq!(rb.error_code, "INVALID_ORDER");
    assert_eq!(rb.submitted_bundle.debt_repayment_cred, 10);
    assert_eq!(rb.submitted_bundle.converter_runs.len(), 1);
    assert_eq!(rb.submitted_bundle.dual_escrows.len(), 1);
    assert_eq!(rb.submitted_bundle.dual_escrows[0].proposal_id, 1);

    assert_eq!(
        manifest.refusal_counts_by_seat["Foundry"].refused_bundles,
        1
    );
    assert_eq!(manifest.refusal_counts_by_arm["off"].refused_bundles, 1);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn r9_d6_seat_count_must_be_four() {
    let tmp = fresh_tmp_dir("d6_seats");
    let wrapper_path = write_wrapper(&tmp);
    for (tag, count) in [("three_seats", 3_usize), ("five_seats", 5_usize)] {
        let mut entries = Vec::new();
        for i in 0..count {
            entries.push(json!({
                "guild": format!("Seat{i}"),
                "backend": "file",
                "model": "ClubBot"
            }));
        }
        let seats_path = write_seats(
            &tmp,
            &format!("seats_{tag}.json"),
            json!({ "seats": entries }),
        );
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: tmp.join(format!("g_{tag}")),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 9,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        ) {
            Err(e) => e,
            Ok(_) => panic!("{tag} must fail INVALID_EVAL_SEAT_COUNT"),
        };
        assert_eq!(err.code(), "INVALID_EVAL_SEAT_COUNT");
        assert!(matches!(err, EvalError::InvalidSeatCount { actual } if actual == count));
        assert_eq!(err.exit_code(), 1);
    }
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn items_12_and_13_branch_prompt_parity_off_and_on_single_diff() {
    let tmp = fresh_tmp_dir("items12_13");
    let src_game = tmp.join("src_game");
    let branch_out = tmp.join("branches");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());

    let script = vec![
        (
            "1:talk",
            json!({
                "messages": [{ "to_guild": "Quarry", "body": "t1 msg" }],
                "market_actions": [],
                "bundle": {},
                "notes": "d1_talk_note"
            })
            .to_string(),
        ),
        (
            "1:act",
            json!({
                "messages": [],
                "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 0 }, "amount": 20 }],
                "bundle": {},
                "notes": "d1_act_note"
            })
            .to_string(),
        ),
        (
            "2:talk",
            json!({
                "messages": [],
                "market_actions": [],
                "bundle": {},
                "notes": "d2_talk_note"
            })
            .to_string(),
        ),
        (
            "2:act",
            json!({
                "messages": [],
                "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 1 }, "amount": 15 }],
                "bundle": {},
                "notes": "d2_act_note"
            })
            .to_string(),
        ),
    ];

    let play_transport = DeterministicMockTransport::new(script.clone());
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: src_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 12,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_transport,
            None,
        ),
        "play 2 turns",
    );
    let play_bodies = play_transport.recorded_bodies();
    // 0 = d1:talk, 1 = d1:act, 2 = d2:talk, 3 = d2:act.
    assert_eq!(play_bodies.len(), 4);
    let play_d2_act_body: Value =
        must_ok(serde_json::from_str(&play_bodies[3]), "parse play d2 act");

    // Branch from `checkpoints/day-02-pre-act` on focal seat `Foundry` across `[Off, On]`.
    let branch_transport = DeterministicMockTransport::new(vec![(
        "2:act",
        json!({
            "messages": [],
            "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 1 }, "amount": 15 }],
            "bundle": {},
            "notes": "d2_act_note"
        })
        .to_string(),
    )]);
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(src_game.join("checkpoints/day-02-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path,
                wrapper_path,
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: branch_out.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &branch_transport,
            None,
        ),
        "branch day-02 off,on",
    );
    assert_eq!(outcomes.len(), 2);

    let branch_bodies = branch_transport.recorded_bodies();
    assert_eq!(branch_bodies.len(), 2);

    // Item 12: `branch --arm off` HTTP request body equals `play`'s `(day 02, act)` HTTP request body byte-for-byte!
    assert_eq!(
        branch_bodies[0], play_bodies[3],
        "Item 12: branch off HTTP request body must match play (day 02, act) byte-for-byte"
    );

    // Item 13: Diff `branch off` vs `branch on` HTTP request JSON -> ONLY `messages[0].content` (system
    // prompt preview call-budget suffix) and `tools` (`preview_turn_orders`) differ!
    let off_val: Value = must_ok(serde_json::from_str(&branch_bodies[0]), "parse off body");
    let on_val: Value = must_ok(serde_json::from_str(&branch_bodies[1]), "parse on body");

    let mut off_norm = off_val.clone();
    let mut on_norm = on_val.clone();

    let off_sys = must_some(
        off_val["messages"][0]["content"].as_str(),
        "off system message",
    );
    let on_sys = must_some(
        on_val["messages"][0]["content"].as_str(),
        "on system message",
    );
    assert_ne!(
        off_sys, on_sys,
        "Item 13: on arm must include preview call-budget instruction in messages[0].content"
    );
    assert!(on_sys.starts_with(off_sys));

    assert!(off_val.get("tools").is_none());
    let tools = must_some(on_val["tools"].as_array(), "on arm must attach tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["function"]["name"], json!("preview_turn_orders"));

    // Normalize ONLY `messages[0].content` and `tools`, then confirm EVERY other field (including
    // user message, prior turns, scratchpad, response_format, model, temperature) is identical!
    off_norm["messages"][0]["content"] = Value::Null;
    on_norm["messages"][0]["content"] = Value::Null;
    if let Some(obj) = on_norm.as_object_mut() {
        obj.remove("tools");
    }
    assert_eq!(
        off_norm, on_norm,
        "Item 13: off vs on HTTP requests must differ ONLY in messages[0].content and tools"
    );
    assert_eq!(play_d2_act_body["messages"], off_val["messages"]);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn item15_null_branch_matches_play_receipt_and_leaks_zero_seal_secrets_under_out() {
    let tmp = fresh_tmp_dir("item15_canary");
    let src_game = tmp.join("src_game");
    let branch_out = tmp.join("branch_out");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());

    let d2_act_content = json!({
        "messages": [],
        "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 0 }, "amount": 30 }],
        "bundle": {},
        "notes": "d2_act_note"
    })
    .to_string();
    let script = vec![
        (
            "1:talk",
            json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "n1" })
                .to_string(),
        ),
        (
            "1:act",
            json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "n2" })
                .to_string(),
        ),
        (
            "2:talk",
            json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "n3" })
                .to_string(),
        ),
        ("2:act", d2_act_content.clone()),
    ];
    let play_transport = DeterministicMockTransport::new(script);
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: src_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 15,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_transport,
            None,
        ),
        "play item 15",
    );

    let play_d2_receipt: PublicSweepReceipt = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(src_game.join("receipts/day-02.receipt.json")),
            "read play d2 receipt",
        )),
        "parse play receipt",
    );

    // Null branch on focal seat `Foundry`: replays exact recorded turn-2 action -> byte-exact receipt match!
    let branch_transport = DeterministicMockTransport::new(vec![("2:act", d2_act_content)]);
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(src_game.join("checkpoints/day-02-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path,
                wrapper_path,
                arms: vec![EvalArm::Off],
                out_dir: branch_out.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &branch_transport,
            None,
        ),
        "null branch",
    );
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].receipt, play_d2_receipt);

    // Extract REAL cryptographic secrets & peer private mandates from source `.seal` JSON:
    //   - `sealed_mandate_deal.deal_seed` (64-hex string)
    //   - `sealed_mandate_deal.seat_reveals[1..4].salt` (64-hex strings for all non-focal seats)
    //   - `eval_vrf` `vrf_proofs[1]` hex representation / array
    //   - `turn_salts[0]` hex strings
    let seal_text = must_ok(
        fs::read_to_string(src_game.join("checkpoints/day-02-pre-act.seal")),
        "read pre-act seal",
    );
    let seal_val: Value = must_ok(serde_json::from_str(&seal_text), "parse pre-act seal");
    let mut secret_needles: Vec<String> = Vec::new();
    if let Some(ds) = seal_val["sealed_mandate_deal"]["deal_seed"].as_str() {
        secret_needles.push(ds.to_owned());
    }
    if let Some(reveals) = seal_val["sealed_mandate_deal"]["seat_reveals"].as_array() {
        for rev in reveals {
            if let Some(s) = rev["salt"].as_str() {
                secret_needles.push(s.to_owned());
            }
        }
    }
    if let Some(ts) = seal_val["turn_salts"].as_array()
        && let Some(row) = ts.first().and_then(Value::as_array)
        && let Some(salt_hex) = row.first().and_then(Value::as_str)
    {
        secret_needles.push(salt_hex.to_owned());
    }
    assert!(
        secret_needles.len() >= 5,
        "expected >= 5 real secret needles from .seal; got {:?}",
        secret_needles
    );

    // Negative control: every needle DOES exist in `seal_text` itself!
    for needle in &secret_needles {
        assert!(
            seal_text.contains(needle),
            "negative control failed: secret `{needle}` missing from .seal"
        );
    }

    // Verify ZERO `.seal` files and ZERO occurrences of any secret needle anywhere under `--out`.
    let mut branch_files = Vec::new();
    collect_all_files(&branch_out, &mut branch_files);
    assert!(!branch_files.is_empty());
    for file in branch_files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            !name.ends_with(".seal"),
            "private .seal file must never appear under --out: {}",
            file.display()
        );
        let content = must_ok(fs::read_to_string(&file), "read branch out file");
        for needle in &secret_needles {
            assert!(
                !content.contains(needle),
                "secret `{needle}` leaked into {}",
                file.display()
            );
        }
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn item14_select_determinism_strata_and_branch_list_end_to_end() {
    let tmp = fresh_tmp_dir("item14_select");
    let game_a = tmp.join("game_a");
    let game_b = tmp.join("game_b");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", all_clubbot_seats());

    for (tid, gd) in [(21_u64, &game_a), (22_u64, &game_b)] {
        let _ = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: gd.clone(),
                    seats_path: seats_path.clone(),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    arm: EvalArm::Off,
                    table_id: tid,
                    seed: tid,
                    turns: 9,
                    memory_mode: league_relay::MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &NoopTransport,
                None,
            ),
            "play 9 turns for select",
        );
    }

    let list_1 = tmp.join("selection_1.json");
    let list_2 = tmp.join("selection_2.json");

    // Same seed -> byte-identical selection list JSON.
    let sel_1 = must_ok(
        run_select(&SelectOptions {
            game_dirs: vec![game_a.clone(), game_b.clone()],
            per_game: 6,
            strata: [2, 2, 2],
            selection_seed: 20_261_121,
            out_path: list_1.clone(),
        }),
        "run_select 1",
    );
    let sel_2 = must_ok(
        run_select(&SelectOptions {
            game_dirs: vec![game_a.clone(), game_b.clone()],
            per_game: 6,
            strata: [2, 2, 2],
            selection_seed: 20_261_121,
            out_path: list_2.clone(),
        }),
        "run_select 2",
    );
    assert_eq!(sel_1, sel_2);
    assert_eq!(
        must_ok(fs::read(&list_1), "read l1"),
        must_ok(fs::read(&list_2), "read l2")
    );
    assert_eq!(sel_1.states.len(), 12);

    // Verify exact [2, 2, 2] early/mid/late counts per game (`early = 1..=3`, `mid = 4..=6`, `late = 7..=9`).
    for gd in [&game_a, &game_b] {
        let game_states: Vec<&SelectedPreActState> =
            sel_1.states.iter().filter(|s| &s.game_dir == gd).collect();
        assert_eq!(game_states.len(), 6);
        let early: Vec<u8> = game_states
            .iter()
            .filter(|s| s.stratum == TurnStratum::Early)
            .map(|s| s.day)
            .collect();
        let mid: Vec<u8> = game_states
            .iter()
            .filter(|s| s.stratum == TurnStratum::Mid)
            .map(|s| s.day)
            .collect();
        let late: Vec<u8> = game_states
            .iter()
            .filter(|s| s.stratum == TurnStratum::Late)
            .map(|s| s.day)
            .collect();
        assert_eq!(early.len(), 2);
        assert_eq!(mid.len(), 2);
        assert_eq!(late.len(), 2);
        assert!(early.iter().all(|&d| (1..=3).contains(&d)));
        assert!(mid.iter().all(|&d| (4..=6).contains(&d)));
        assert!(late.iter().all(|&d| (7..=9).contains(&d)));
    }

    // Exercise `select` + `branch --list` via CLI!
    let list_cli_path = tmp.join("cli_list.json");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let sel_code = execute_cli(
        [
            "select".into(),
            "--games".into(),
            format!("{},{}", game_a.display(), game_b.display()).into(),
            "--per-game".into(),
            "6".into(),
            "--strata".into(),
            "2,2,2".into(),
            "--selection-seed".into(),
            "20261121".into(),
            "--out".into(),
            list_cli_path.clone().into_os_string(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(
        sel_code,
        0,
        "select stderr={}",
        String::from_utf8_lossy(&err)
    );
    assert_eq!(
        must_ok(fs::read(&list_cli_path), "cli sel"),
        must_ok(fs::read(&list_1), "sel 1")
    );

    // Write a 2-state selection subset and run `league-eval branch --list <subset.json>`.
    let subset = SelectionList {
        version: sel_1.version,
        selection_seed: sel_1.selection_seed,
        per_game: 2,
        strata: [2, 0, 0],
        states: sel_1.states[..2].to_vec(),
    };
    let subset_path = tmp.join("subset.json");
    must_ok(
        fs::write(
            &subset_path,
            must_ok(serde_json::to_string_pretty(&subset), "ser subset"),
        ),
        "write subset",
    );
    let list_branch_out = tmp.join("list_branch_out");
    out.clear();
    err.clear();
    let br_code = execute_cli(
        [
            "branch".into(),
            "--list".into(),
            subset_path.into_os_string(),
            "--seats".into(),
            seats_path.into_os_string(),
            "--wrapper".into(),
            wrapper_path.into_os_string(),
            "--arms".into(),
            "off,on".into(),
            "--out".into(),
            list_branch_out.clone().into_os_string(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(
        br_code,
        0,
        "branch --list stderr={}",
        String::from_utf8_lossy(&err)
    );
    assert!(String::from_utf8_lossy(&out).contains("branched_outcomes=4"));

    // Error paths on `select`: insufficient turns and strata sum mismatch.
    let short_game = tmp.join("short_game");
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: short_game.clone(),
                seats_path: tmp.join("seats.json"),
                wrapper_path: tmp.join("wrapper.md"),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 23,
                seed: 23,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        ),
        "short game",
    );
    let insuff = run_select(&SelectOptions {
        game_dirs: vec![short_game],
        per_game: 6,
        strata: [2, 2, 2],
        selection_seed: 1,
        out_path: tmp.join("unused.json"),
    });
    assert!(matches!(
        insuff,
        Err(EvalError::InsufficientTurnsForStrata(_))
    ));

    let bad_strata = run_select(&SelectOptions {
        game_dirs: vec![game_a],
        per_game: 6,
        strata: [1, 1, 1],
        selection_seed: 1,
        out_path: tmp.join("unused2.json"),
    });
    assert!(matches!(bad_strata, Err(EvalError::InvalidStrata(_))));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn cli_hygiene_mock_dir_and_error_surfaces_work() {
    let tmp = fresh_tmp_dir("cli_hygiene");
    let mut out = Vec::new();
    let mut err = Vec::new();

    // --help & --version exit 0 without touching filesystem.
    assert_eq!(execute_cli(["--help".into()], &mut out, &mut err), 0);
    assert!(String::from_utf8_lossy(&out).contains("league-eval"));
    out.clear();
    assert_eq!(execute_cli(["--version".into()], &mut out, &mut err), 0);
    assert!(String::from_utf8_lossy(&out).contains("league-eval"));

    // Control arm refusal formatted as JSON (`--error-format json`) with secret redaction.
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", all_clubbot_seats());
    err.clear();
    let ctrl_code = execute_cli(
        [
            "--error-format".into(),
            "json".into(),
            "play".into(),
            "--game-dir".into(),
            tmp.join("ctrl_game").into_os_string(),
            "--seats".into(),
            seats_path.clone().into_os_string(),
            "--wrapper".into(),
            wrapper_path.clone().into_os_string(),
            "--arm".into(),
            "control".into(),
            "--seed".into(),
            "7".into(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(ctrl_code, 1);
    let err_json: Value = must_ok(serde_json::from_slice(&err), "parse JSON error on stderr");
    assert_eq!(err_json["code"], json!("CONTROL_ARM_NOT_IMPLEMENTED"));

    // Missing wrapper refused with MISSING_WRAPPER.
    err.clear();
    let miss_code = execute_cli(
        [
            "play".into(),
            "--game-dir".into(),
            tmp.join("miss_wrap").into_os_string(),
            "--seats".into(),
            seats_path.clone().into_os_string(),
            "--wrapper".into(),
            tmp.join("missing.md").into_os_string(),
            "--arm".into(),
            "off".into(),
            "--seed".into(),
            "7".into(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(miss_code, 1);
    assert!(String::from_utf8_lossy(&err).contains("MISSING_WRAPPER"));

    // `--mock-dir` CLI execution on `play` & `branch` with an openrouter seat!
    let mock_dir = tmp.join("mock_dir");
    must_ok(fs::create_dir_all(&mock_dir), "mkdir mock_dir");
    let mock_resp = json!({
        "rules": [
            {
                "when_contains": "This is the talk round.",
                "status": 200,
                "response": {
                    "id": "m1",
                    "model": "qwen/qwen3-32b",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}" },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20, "cost": 0.001 }
                }
            },
            {
                "when_contains": "This is the act round.",
                "status": 200,
                "response": {
                    "id": "m2",
                    "model": "qwen/qwen3-32b",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}" },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20, "cost": 0.001 }
                }
            }
        ]
    });
    must_ok(
        fs::write(mock_dir.join("responses.json"), mock_resp.to_string()),
        "write mock responses.json",
    );
    let or_seats_path = write_seats(
        &tmp,
        "or_seats.json",
        json!({
            "seats": [
                {
                    "guild": "Foundry",
                    "backend": "openrouter",
                    "model": "qwen/qwen3-32b",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_cost_per_call": 0.05
                },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    let mock_game = tmp.join("mock_game");
    out.clear();
    err.clear();
    let mock_play_code = execute_cli(
        [
            "play".into(),
            "--game-dir".into(),
            mock_game.clone().into_os_string(),
            "--seats".into(),
            or_seats_path.clone().into_os_string(),
            "--wrapper".into(),
            wrapper_path.clone().into_os_string(),
            "--arm".into(),
            "off".into(),
            "--seed".into(),
            "7".into(),
            "--turns".into(),
            "1".into(),
            "--memory-mode".into(),
            "snapshot".into(),
            "--mock-dir".into(),
            mock_dir.clone().into_os_string(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(
        mock_play_code,
        0,
        "mock play stderr={}",
        String::from_utf8_lossy(&err)
    );

    out.clear();
    err.clear();
    let mock_branch_code = execute_cli(
        [
            "branch".into(),
            "--checkpoint".into(),
            mock_game
                .join("checkpoints/day-01-pre-act")
                .into_os_string(),
            "--seat".into(),
            "0".into(),
            "--seats".into(),
            or_seats_path.into_os_string(),
            "--wrapper".into(),
            wrapper_path.into_os_string(),
            "--arms".into(),
            "off".into(),
            "--out".into(),
            tmp.join("mock_branch_out").into_os_string(),
            "--mock-dir".into(),
            mock_dir.into_os_string(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(
        mock_branch_code,
        0,
        "mock branch stderr={}",
        String::from_utf8_lossy(&err)
    );

    // Cover helper merge methods & EvalError variants for 92%+ coverage hygiene.
    let mut c1 = RefusalCodeCounts::default();
    let mut c2 = RefusalCodeCounts::default();
    c2.record(league_eval::ReplyRefusalCode::ActReplyInvalid);
    c1.merge_from(&c2);
    assert_eq!(c1.act_reply_invalid, 1);

    let mut w1 = WrongPartCounter::default();
    let mut w2 = WrongPartCounter::default();
    w2.record_reply(Some(league_eval::ReplyRefusalCode::TalkReplyHasOrders));
    w1.merge_from(&w2);
    assert_eq!(w1.total_wrong_part_refusals, 1);

    assert_eq!(EVAL_REPLY_WORDING_ARM, "v2_eval_reply_2026_11_21_r1");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn error_surfaces_cli_variants_and_resume_validation_coverage() {
    use league_core::{EngineError, ToStructuredError as _};
    use league_eval::{BranchArmOutcome, ReplyRefusalCode, extract_wrapper_metadata};
    use league_relay::RelayError;

    // Note: cover all 13 `EvalError` variants across `code()`, `exit_code()`, `Display`, and `to_structured_error()`.
    let all_errors: [EvalError; 13] = [
        EvalError::Usage("bad flag OPENROUTER_API_KEY=sk-or-secret123".to_owned()),
        EvalError::InvalidSeatCount { actual: 3 },
        EvalError::UnsupportedFileSeatModel {
            guild: "Foundry".to_owned(),
            model: Some("Other".to_owned()),
        },
        EvalError::ControlArmNotImplemented,
        EvalError::MissingWrapper("missing wrapper".to_owned()),
        EvalError::SchemaValidation("bad schema".to_owned()),
        EvalError::CheckpointNotFound("missing ckpt".to_owned()),
        EvalError::InsufficientTurnsForStrata("too short".to_owned()),
        EvalError::InvalidStrata("bad strata".to_owned()),
        EvalError::UnknownFocalSeat("Phantom".to_owned()),
        EvalError::Engine(EngineError::InvalidConfig("bad config".to_owned())),
        EvalError::Relay(RelayError::Usage("bad relay".to_owned())),
        EvalError::Io("io error".to_owned()),
    ];
    for e in &all_errors {
        assert!(!e.code().is_empty());
        assert!(matches!(e.exit_code(), 1..=3));
        assert!(!e.to_string().contains("sk-or-secret123"));
        let s = e.to_structured_error();
        assert_eq!(s.code, e.code());
    }
    let from_eng: EvalError = EngineError::InvalidConfig("c".to_owned()).into();
    assert_eq!(from_eng.code(), "INVALID_CONFIG");
    let from_relay: EvalError = RelayError::Usage("r".to_owned()).into();
    assert!(!from_relay.code().is_empty());

    // `extract_wrapper_metadata` & `EvalArm::parse` / `NoopTransport` error branches.
    assert!(EvalArm::parse("bogus").is_err());
    assert_eq!(EvalArm::parse("On"), Ok(EvalArm::On));
    let (v_hash, sha) = extract_wrapper_metadata(b"   \n#   \n", None);
    assert!(v_hash.starts_with("sha256:"));
    assert_eq!(sha.len(), 64);
    let noop = NoopTransport;
    assert!(
        noop.post_chat_completions("http://127.0.0.1", "{}", Duration::from_secs(1))
            .is_err()
    );

    // Cover CLI branches (`--error-format=json`, subcommand `--help`/`-V`, `--memory-mode emc|full`, missing args).
    let mut out = Vec::new();
    let mut err = Vec::new();
    assert_eq!(
        execute_cli(["play".into(), "--help".into()], &mut out, &mut err),
        0
    );
    assert_eq!(
        execute_cli(["branch".into(), "-V".into()], &mut out, &mut err),
        0
    );
    assert_eq!(
        execute_cli(
            ["--error-format=json".into(), "unknowncmd".into()],
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(
        execute_cli(
            ["--error-format".into(), "bogus".into(), "play".into()],
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(execute_cli([], &mut out, &mut err), 1);

    let tmp = fresh_tmp_dir("cov_cli");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", all_clubbot_seats());
    let game_dir = tmp.join("resume_game");

    // Bad `--turns 0`, bad `--seed`, bad `--memory-mode`, missing `--seat` on branch, etc.
    for bad_args in [
        vec!["play", "--turns", "0"],
        vec!["play", "--turns", "abc"],
        vec!["play", "--seed", "nope"],
        vec!["play", "--table-id", "nope"],
        vec!["play", "--cap-credits", "nope"],
        vec!["play", "--cap-gpu-secs", "nope"],
        vec!["play", "--memory-mode", "bad"],
        vec!["play", "--arm", "bad"],
        vec!["play", "--unknown-flag"],
        vec!["branch", "--unknown-flag"],
        vec![
            "branch",
            "--arms",
            "control",
            "--seats",
            "x",
            "--wrapper",
            "y",
            "--out",
            "z",
        ],
        vec!["select", "--strata", "1,2"],
        vec!["select", "--strata", "1,a,2"],
        vec!["select", "--per-game", "nope"],
        vec!["select", "--selection-seed", "nope"],
        vec!["select", "--unknown-flag"],
    ] {
        let os_args: Vec<std::ffi::OsString> = bad_args.into_iter().map(Into::into).collect();
        assert_eq!(execute_cli(os_args, &mut out, &mut err), 1);
    }

    // Play 1 turn with `--memory-mode emc`, then resume turn 2 with matching `--memory-mode emc`;
    // also run a separate 1-turn `--memory-mode full` table.
    assert_eq!(
        execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                game_dir.clone().into_os_string(),
                "--seats".into(),
                seats_path.clone().into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "7".into(),
                "--turns".into(),
                "1".into(),
                "--memory-mode".into(),
                "emc".into(),
            ],
            &mut out,
            &mut err,
        ),
        0
    );
    assert_eq!(
        execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                game_dir.clone().into_os_string(),
                "--seats".into(),
                seats_path.clone().into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "7".into(),
                "--turns".into(),
                "2".into(),
                "--memory-mode".into(),
                "emc".into(),
            ],
            &mut out,
            &mut err,
        ),
        0
    );
    assert_eq!(
        execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                tmp.join("full_mode_game").into_os_string(),
                "--seats".into(),
                seats_path.clone().into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "8".into(),
                "--turns".into(),
                "1".into(),
                "--memory-mode".into(),
                "full".into(),
            ],
            &mut out,
            &mut err,
        ),
        0
    );
    let m: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("manifest.json")),
            "m",
        )),
        "parse m",
    );
    assert_eq!(m.played_turns, 2);

    // Resuming with mismatched `--arm` fails loudly.
    let mismatch_err = run_play_with_transport(
        &PlayOptions {
            game_dir: game_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            arm: EvalArm::On,
            table_id: 1,
            seed: 7,
            turns: 3,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    );
    assert!(matches!(mismatch_err, Err(EvalError::Usage(_))));

    // Branch unknown focal seat & missing checkpoint prefix -> clean named error codes.
    let unk = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(game_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("NoSuchGuild".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("b_unk"),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    );
    assert!(matches!(unk, Err(EvalError::UnknownFocalSeat(_))));

    let missing_ckpt = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(game_dir.join("checkpoints/day-99-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("b_miss"),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    );
    assert!(matches!(
        missing_ckpt,
        Err(EvalError::CheckpointNotFound(_))
    ));

    // Exercise ReplayRefusalCode::as_str & BranchArmOutcome + persist_eval_game_state type checks cleanly.
    let _: (&str, Option<BranchArmOutcome>) =
        (ReplyRefusalCode::MessageRecipientInvalid.as_str(), None);
    let h = must_ok(EvalHost::from_standard_setup(TableId(99), 4, 1, 2), "h");
    let _ = must_ok(persist_eval_game_state(&h, &tmp.join("p_test")), "persist");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn coverage_subprocess_cli_gates_noreplies_and_select_guards() {
    use std::process::Command;

    let tmp = fresh_tmp_dir("cov_extra");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", all_clubbot_seats());

    // 1. Subprocess binary test covering `src/main.rs` (`CARGO_BIN_EXE_league-eval`).
    // Do NOT `.env_clear()` so `LLVM_PROFILE_FILE` passes through cleanly during CI coverage.
    let bin = env!("CARGO_BIN_EXE_league-eval");
    let ver_out = must_ok(
        Command::new(bin)
            .arg("--version")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .output(),
        "run league-eval --version",
    );
    assert!(ver_out.status.success());
    assert!(String::from_utf8_lossy(&ver_out.stdout).contains("league-eval"));

    let err_out = must_ok(
        Command::new(bin)
            .args([
                "--error-format",
                "json",
                "play",
                "--game-dir",
                &tmp.join("bin_ctrl").to_string_lossy(),
                "--seats",
                &seats_path.to_string_lossy(),
                "--wrapper",
                &wrapper_path.to_string_lossy(),
                "--arm",
                "control",
                "--seed",
                "7",
            ])
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .output(),
        "run league-eval binary error exit",
    );
    assert_eq!(err_out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&err_out.stderr).contains("CONTROL_ARM_NOT_IMPLEMENTED"));

    // 2. Remaining `cli.rs` flags, help/version subcommands, and missing required options.
    let mut out = Vec::new();
    let mut err = Vec::new();
    for ok_cli in [
        vec!["--error-format", "text", "--version"],
        vec!["--error-format=text", "--help"],
        vec!["select", "--help"],
        vec!["select", "-V"],
    ] {
        out.clear();
        err.clear();
        assert_eq!(
            execute_cli(ok_cli.into_iter().map(Into::into), &mut out, &mut err),
            0
        );
    }

    let play_with_extra_flags = tmp.join("extra_flags_game");
    out.clear();
    err.clear();
    let extra_code = execute_cli(
        [
            "play".into(),
            "--error-format".into(),
            "json".into(),
            "--game-dir".into(),
            play_with_extra_flags.clone().into_os_string(),
            "--seats".into(),
            seats_path.clone().into_os_string(),
            "--wrapper".into(),
            wrapper_path.clone().into_os_string(),
            "--wrapper-version".into(),
            "v99".into(),
            "--arm".into(),
            "off".into(),
            "--table-id".into(),
            "55".into(),
            "--seed".into(),
            "7".into(),
            "--turns".into(),
            "1".into(),
            "--cap-credits".into(),
            "5.0".into(),
            "--cap-gpu-secs".into(),
            "10.0".into(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(
        extra_code,
        0,
        "extra_code stderr={}",
        String::from_utf8_lossy(&err)
    );
    // Unused `--allow-vm-host` with all-ClubBot seats fails cleanly (`INVALID_SEAT_CONFIG`), covering the flag parser + validator loop.
    assert_eq!(
        execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                tmp.join("unused_vm").into_os_string(),
                "--seats".into(),
                seats_path.clone().into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "7".into(),
                "--allow-vm-host".into(),
                "127.0.0.1".into(),
            ],
            &mut out,
            &mut err,
        ),
        1
    );

    for bad_cli in [
        vec!["play", "--error-format", "bogus"],
        vec!["branch", "--error-format", "bogus"],
        vec!["select", "--error-format", "bogus"],
        vec![
            "play",
            "--seats",
            "a",
            "--wrapper",
            "b",
            "--arm",
            "off",
            "--seed",
            "1",
        ],
        vec![
            "play",
            "--game-dir",
            "g",
            "--wrapper",
            "b",
            "--arm",
            "off",
            "--seed",
            "1",
        ],
        vec![
            "play",
            "--game-dir",
            "g",
            "--seats",
            "a",
            "--arm",
            "off",
            "--seed",
            "1",
        ],
        vec![
            "play",
            "--game-dir",
            "g",
            "--seats",
            "a",
            "--wrapper",
            "b",
            "--seed",
            "1",
        ],
        vec![
            "play",
            "--game-dir",
            "g",
            "--seats",
            "a",
            "--wrapper",
            "b",
            "--arm",
            "off",
        ],
        vec!["branch", "--wrapper", "b", "--out", "o"],
        vec!["branch", "--seats", "a", "--out", "o"],
        vec!["branch", "--seats", "a", "--wrapper", "b"],
        vec!["branch", "--memory-mode", "bad"],
        vec!["branch", "--cap-credits", "bad"],
        vec!["branch", "--cap-gpu-secs", "bad"],
        vec!["branch", "--table-id", "5"],
        vec!["select", "--out", "o"],
        vec!["select", "--selection-seed", "1"],
    ] {
        out.clear();
        err.clear();
        assert_eq!(
            execute_cli(bad_cli.into_iter().map(Into::into), &mut out, &mut err),
            1
        );
    }

    // Exercise `execute_with_transport` local remote-host refusal and openrouter missing `--live` refusal (zero network).
    let remote_local_seats = write_seats(
        &tmp,
        "remote_local.json",
        json!({
            "seats": [
                {
                    "guild": "Foundry",
                    "backend": "local",
                    "base_url": "http://198.51.100.10:8080/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_gpu_secs_per_call": 10.0
                },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    assert_ne!(
        execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                tmp.join("g_remote").into_os_string(),
                "--seats".into(),
                remote_local_seats.into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "7".into(),
            ],
            &mut out,
            &mut err,
        ),
        0
    );

    let or_seats = write_seats(
        &tmp,
        "or_seats.json",
        json!({
            "seats": [
                {
                    "guild": "Foundry",
                    "backend": "openrouter",
                    "model": "qwen/qwen3-32b",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_cost_per_call": 0.05
                },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    assert_ne!(
        execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                tmp.join("g_or_nolive").into_os_string(),
                "--seats".into(),
                or_seats.clone().into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "7".into(),
            ],
            &mut out,
            &mut err,
        ),
        0
    );

    // 3. `driver.rs` HTTP-500 (`no_reply: true` across talk, act, and branch), `EvalArm::On` preview-state,
    //    `branch` default arms `[Off, On]`, standalone checkpoint without `negotiation.json`, refused bundle in `branch`,
    //    empty/dir wrapper, `turns=33`, `setup.json` mismatched `game_id`, and malformed transcript.
    struct AlwaysFailTransport;
    impl Transport for AlwaysFailTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            Ok(RawHttpResponse {
                status: 500,
                body: "{\"error\":\"down\"}".to_owned(),
            })
        }
    }

    let local_seats_path = write_seats(&tmp, "local_seats.json", focal_local_three_clubbot_seats());
    let fail_game = tmp.join("fail_on_game");
    let fail_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: fail_game.clone(),
                seats_path: local_seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::On,
                table_id: 71,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &AlwaysFailTransport,
            None,
        ),
        "run_play AlwaysFailTransport",
    );
    assert_eq!(fail_manifest.played_turns, 1);
    assert!(
        fail_game
            .join("to-seat/day-01-act-Foundry.preview-state.json")
            .exists()
    );
    let talk_fail_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(fail_game.join("decisions/day-01-talk-Foundry.json")),
            "read fail talk dec",
        )),
        "parse fail talk dec",
    );
    let act_fail_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(fail_game.join("decisions/day-01-act-Foundry.json")),
            "read fail act dec",
        )),
        "parse fail act dec",
    );
    assert!(talk_fail_dec.no_reply);
    assert!(act_fail_dec.no_reply);

    // Branch with `AlwaysFailTransport` -> `no_reply: true` in `BranchArmOutcome`, default arms `[Off, On]`.
    let fail_branch = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(fail_game.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: local_seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: tmp.join("fail_branch_out"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &AlwaysFailTransport,
            None,
        ),
        "branch AlwaysFailTransport",
    );
    assert_eq!(fail_branch.len(), 2);
    assert!(fail_branch[0].decision.no_reply);

    // Branch where focal seat submits an over-capped DualEscrow bundle -> `refused_bundle` recorded in `branch`!
    let bad_bundle_act = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {
            "dual_escrows": [{
                "proposal_id": 1,
                "buyer_seat": 0,
                "seller_seat": 1,
                "good": { "asset_kind": "raw", "good_index": 0 },
                "required_units": 10,
                "payment_cred": 100,
                "buyer_bond_cred": 80,
                "seller_bond_cred": 80,
                "deadline_turn": 3,
                "domain_id": null
            }]
        },
        "notes": "bad bundle in branch"
    })
    .to_string();
    let bad_bundle_transport = DeterministicMockTransport::new(vec![("1:act", bad_bundle_act)]);
    let bad_branch = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(fail_game.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("0".to_owned()),
                seats_path: local_seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("bad_bundle_branch"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &bad_bundle_transport,
            None,
        ),
        "branch refused bundle",
    );
    assert!(bad_branch[0].decision.refused_bundle.is_some());

    // Copy checkpoints to a standalone directory (no `to-seat/*.negotiation.json`) so branch exercises fallback negotiation synthesis!
    let standalone_ckpts = tmp.join("standalone/checkpoints");
    must_ok(fs::create_dir_all(&standalone_ckpts), "mkdir standalone");
    for fname in [
        "day-01-pre-act.save.json",
        "day-01-pre-act.seal",
        "day-01-act-recorded.seal",
    ] {
        must_ok(
            fs::copy(
                play_with_extra_flags.join("checkpoints").join(fname),
                standalone_ckpts.join(fname),
            ),
            fname,
        );
    }
    let standalone_branch = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(standalone_ckpts.join("day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("standalone_branch_out"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        ),
        "standalone branch without negotiation.json",
    );
    assert_eq!(standalone_branch.len(), 1);

    // Corrupt `setup.json` game_id, empty wrapper, `--turns 33`, malformed transcript, missing act-recorded seal.
    let empty_wrapper = tmp.join("empty_wrapper.md");
    must_ok(fs::write(&empty_wrapper, b""), "empty wrapper");
    for opts_err in [
        PlayOptions {
            game_dir: tmp.join("g_t33"),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            arm: EvalArm::Off,
            table_id: 1,
            seed: 1,
            turns: HARD_TURN_CAP + 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        PlayOptions {
            game_dir: tmp.join("g_empty_wrap"),
            seats_path: seats_path.clone(),
            wrapper_path: empty_wrapper,
            wrapper_version: None,
            arm: EvalArm::Off,
            table_id: 1,
            seed: 1,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        PlayOptions {
            game_dir: tmp.join("g_dir_wrap"),
            seats_path: seats_path.clone(),
            wrapper_path: tmp.clone(),
            wrapper_version: None,
            arm: EvalArm::Off,
            table_id: 1,
            seed: 1,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
    ] {
        assert!(run_play_with_transport(&opts_err, &NoopTransport, None).is_err());
    }

    let mut bad_setup: Value = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(play_with_extra_flags.join("setup.json")),
            "read setup",
        )),
        "parse setup",
    );
    bad_setup["game_id"] = json!("wrong-game-id");
    must_ok(
        fs::write(
            play_with_extra_flags.join("setup.json"),
            bad_setup.to_string(),
        ),
        "write bad setup",
    );
    assert!(
        run_play_with_transport(
            &PlayOptions {
                game_dir: play_with_extra_flags.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 55,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );

    // Branch validation errors & malformed transcript JSONL.
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: None,
                list_path: None,
                focal_seat: None,
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![],
                out_dir: tmp.join("b_e1"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(fail_game.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: None,
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("b_e2"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: None,
                list_path: Some(tmp.join("missing_list.json")),
                focal_seat: None,
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("b_e3"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );
    let empty_list = tmp.join("empty_list.json");
    must_ok(
        fs::write(
            &empty_list,
            json!({
                "version": 1,
                "selection_seed": 1,
                "per_game": 0,
                "strata": [0, 0, 0],
                "states": []
            })
            .to_string(),
        ),
        "write empty_list",
    );
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: None,
                list_path: Some(empty_list.clone()),
                focal_seat: None,
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("b_e4"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(fail_game.join("checkpoints/day-01-pre-act")),
                list_path: Some(empty_list),
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("b_e5"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );

    let _ = fs::remove_file(standalone_ckpts.join("day-01-act-recorded.seal"));
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(standalone_ckpts.join("day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("b_no_rec"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );

    // Malformed transcript JSONL under `fail_game/relay/seats/Foundry.jsonl`.
    must_ok(
        fs::write(
            fail_game.join("relay/seats/Foundry.jsonl"),
            "\n{corrupt-jsonl\n",
        ),
        "write corrupt jsonl",
    );
    assert!(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(fail_game.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path,
                wrapper_path,
                arms: vec![EvalArm::Off],
                out_dir: tmp.join("b_bad_jsonl"),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        )
        .is_err()
    );

    // 4. `select.rs` remaining error branches (`game_dirs` empty, missing/corrupt manifest, bad seat count, missing checkpoints, stratum pool < need).
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![],
            per_game: 6,
            strata: [2, 2, 2],
            selection_seed: 1,
            out_path: tmp.join("sel_e1.json"),
        })
        .is_err()
    );
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![tmp.join("no_such_game")],
            per_game: 6,
            strata: [2, 2, 2],
            selection_seed: 1,
            out_path: tmp.join("sel_e2.json"),
        })
        .is_err()
    );
    let corrupt_manifest_game = tmp.join("corrupt_m_game");
    must_ok(fs::create_dir_all(&corrupt_manifest_game), "mkdir cm");
    must_ok(
        fs::write(corrupt_manifest_game.join("manifest.json"), b"{bad"),
        "write bad manifest",
    );
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![corrupt_manifest_game],
            per_game: 6,
            strata: [2, 2, 2],
            selection_seed: 1,
            out_path: tmp.join("sel_e3.json"),
        })
        .is_err()
    );

    // Missing checkpoints dir & per-stratum pool exhaustion on `fail_game` (only 1 turn played).
    let no_ckpts = tmp.join("no_ckpts_game");
    must_ok(fs::create_dir_all(&no_ckpts), "mkdir nck");
    let mut m_copy: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(fail_game.join("manifest.json")),
            "read fm",
        )),
        "parse fm",
    );
    let mut m_bad_seats = m_copy.clone();
    m_bad_seats.seats.pop();
    must_ok(
        fs::write(
            no_ckpts.join("manifest.json"),
            must_ok(serde_json::to_string(&m_bad_seats), "ser bad seats"),
        ),
        "write bad seats manifest",
    );
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![no_ckpts.clone()],
            per_game: 1,
            strata: [1, 0, 0],
            selection_seed: 1,
            out_path: tmp.join("sel_e4.json"),
        })
        .is_err()
    );
    m_copy.played_turns = 1;
    must_ok(
        fs::write(
            no_ckpts.join("manifest.json"),
            must_ok(serde_json::to_string(&m_copy), "ser m_copy"),
        ),
        "write ok manifest",
    );
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![no_ckpts],
            per_game: 1,
            strata: [1, 0, 0],
            selection_seed: 1,
            out_path: tmp.join("sel_e5.json"),
        })
        .is_err()
    );
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![fail_game],
            per_game: 1,
            strata: [0, 1, 0],
            selection_seed: 1,
            out_path: tmp.join("sel_e6.json"),
        })
        .is_err()
    );

    let _ = fs::remove_dir_all(&tmp);
}
