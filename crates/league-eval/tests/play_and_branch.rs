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

use league_core::{EvalHost, HARD_TURN_CAP, PublicSweepReceipt, Seat, TableId, replay_eval_season};
use league_eval::{
    BranchArmOutcome, BranchOptions, BranchSummary, CRATE_GIT_SHA, CrateGitShaSource,
    EVAL_REPLY_WORDING_ARM, EvalArm, EvalDecisionRecord, EvalError, EvalManifest, NoopTransport,
    PlayOptions, RefusalCodeCounts, SelectOptions, SelectedPreActState, SelectionList, TurnStratum,
    WrongPartCounter, execute_cli_with_git_sha, expand_home_path_with_home,
    persist_eval_game_state, run_branch_with_transport_and_git_sha,
    run_play_with_transport_and_git_sha, run_select, sanitize_home_path_with_homes,
};
use league_relay::{
    ApiKey, CallLogEntry, CostUnit, RawHttpResponse, Round, Transport, TransportError, sha256_hex,
};
use serde_json::{Value, json};

const TEST_FALLBACK_GIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn test_git_sha_source() -> CrateGitShaSource<'static> {
    let built_sha =
        if CRATE_GIT_SHA.len() == 40 && CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()) {
            CRATE_GIT_SHA
        } else {
            TEST_FALLBACK_GIT_SHA
        };
    CrateGitShaSource::Injected {
        built_sha,
        live_head: Some(built_sha),
        is_dirty: Some(false),
    }
}

fn run_play_with_transport(
    opts: &PlayOptions,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<EvalManifest, EvalError> {
    run_play_with_transport_and_git_sha(opts, transport, redaction_key, &test_git_sha_source())
}

fn run_branch_with_transport(
    opts: &BranchOptions,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<Vec<BranchArmOutcome>, EvalError> {
    run_branch_with_transport_and_git_sha(opts, transport, redaction_key, &test_git_sha_source())
}

fn execute_cli<I>(args: I, stdout: &mut dyn std::io::Write, stderr: &mut dyn std::io::Write) -> i32
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    execute_cli_with_git_sha(args, stdout, stderr, &test_git_sha_source())
}

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

fn mock_chat_response_value(
    id: &str,
    model: &str,
    content: &str,
    prompt_tokens: u64,
    completion_tokens: u64,
    cost: Option<f64>,
) -> Value {
    let top_logprobs: Vec<Value> = (0_u64..5)
        .map(|k| {
            json!({
                "token": format!("t{k}"),
                "id": 200 + k,
                "token_id": 200 + k,
                "logprob": -0.05 * ((k + 1) as f64)
            })
        })
        .collect();
    let mut usage = json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens + completion_tokens
    });
    if let Some(c) = cost {
        usage["cost"] = json!(c);
    }
    json!({
        "id": id,
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "logprob": -0.01,
                    "top_logprobs": top_logprobs
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": usage
    })
}

fn write_wrapper(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("wrappers/v2_seat_brief_2026_11_21_r3.md");
    let bytes = must_ok(fs::read(&src), "read committed pinned r3 wrapper");
    let p = dir.join("wrapper.md");
    must_ok(fs::write(&p, bytes), "write wrapper");
    p
}

fn write_valid_serving_manifest(dir: &Path) -> PathBuf {
    let p = dir.join("serving-manifest.json");
    let manifest = json!({
        "engine": "llama_server",
        "server_version": "b11433",
        "server_build_commit": "deadbeef",
        "server_info_provenance": "operator_supplied",
        "model": {
            "filename": "qwen3-32b-q4.gguf",
            "quant": "Q4_K_M",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "chat_template_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "provenance": "operator_supplied"
        },
        "machine": {
            "gpu_name": "NVIDIA RTX Test",
            "gpu_uuid": "GPU-test-0001",
            "sm_count": 128,
            "gpu_driver": "570.00",
            "cuda_runtime_version": "12.8",
            "vram_mib": 24576,
            "os_build": "Linux 6.8",
            "provenance": "operator_supplied"
        },
        "server_settings": {
            "parallel_slots": 1,
            "batch_size": 2048,
            "ubatch_size": 512,
            "kv_unified": false,
            "flash_attn": true,
            "cache_type_k": "f16",
            "cache_type_v": "f16",
            "speculative_decoding": "none",
            "ctx_size": 32768,
            "ctx_shift": false,
            "threads": 16,
            "provenance": "operator_supplied"
        },
        "cache_settings": {
            "per_request_cache_prompt": false,
            "no_cache_prompt_flag": true,
            "cache_ram_mib": 0,
            "cache_idle_slots": false,
            "provenance": "operator_supplied"
        },
        "sampling": {
            "temperature": 0.2,
            "top_k": 40,
            "top_p": 0.95,
            "min_p": 0.05,
            "repeat_penalty": 1.0,
            "presence_penalty": 0.0,
            "frequency_penalty": 0.0,
            "seed": 7,
            "stop_sequences": [],
            "max_tokens": 2048,
            "sampler_order": ["top_k", "top_p", "min_p", "temperature"],
            "provenance": "operator_supplied"
        }
    });
    must_ok(
        fs::write(
            &p,
            must_ok(serde_json::to_string_pretty(&manifest), "ser serving"),
        ),
        "write serving manifest",
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
    /// Optional checkpoints dir verified on every `act` request (`C1` pre-act timing check).
    pre_act_ckpts_dir: Option<PathBuf>,
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
            pre_act_ckpts_dir: None,
        }
    }

    fn with_pre_act_check(items: Vec<(&str, String)>, checkpoints_dir: PathBuf) -> Self {
        let mut s = Self::new(items);
        s.pre_act_ckpts_dir = Some(checkpoints_dir);
        s
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
        let day: u8 = if user_msg.contains("\"turn\": 3") {
            3
        } else if user_msg.contains("\"turn\": 2") {
            2
        } else {
            1
        };
        if !is_talk && let Some(ref ckpts) = self.pre_act_ckpts_dir {
            let pre_save = ckpts.join(format!("day-{day:02}-pre-act.save.json"));
            let pre_seal = ckpts.join(format!("day-{day:02}-pre-act.seal"));
            assert!(
                pre_save.exists() && pre_seal.exists(),
                "C1: pre-act checkpoint must exist on disk before day {day} act HTTP request ({}, {})",
                pre_save.display(),
                pre_seal.display()
            );
        }
        let key = format!("{day}:{}", if is_talk { "talk" } else { "act" });
        let map = self
            .script_by_turn
            .lock()
            .map_err(|_| TransportError::Network("lock poisoned".to_owned()))?;
        let content = map.get(&key).cloned().unwrap_or_else(|| {
            json!({"messages":[],"market_actions":[],"bundle":{},"notes":"fallback"}).to_string()
        });

        let body = mock_chat_response_value("mock-det-1", "Qwen/Qwen3-32B", &content, 40, 15, None)
            .to_string();
        Ok(RawHttpResponse { status: 200, body })
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

    fn apply_template(
        &self,
        _apply_template_url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<String, TransportError> {
        Ok(format!("TEMPLATED:{body}"))
    }

    fn tokenize_prompt(
        &self,
        _tokenize_url: &str,
        _content: &str,
        add_special: bool,
        parse_special: bool,
        _timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        assert!(add_special && parse_special);
        Ok((100_u64..140).collect())
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
        EvalHost::from_standard_setup_with_model_seats(
            TableId(11),
            4,
            7,
            12345,
            &[true, false, false, false],
        ),
        "from_standard_setup_with_model_seats",
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

    let transport_a =
        DeterministicMockTransport::with_pre_act_check(script.clone(), game_a.join("checkpoints"));
    let transport_b =
        DeterministicMockTransport::with_pre_act_check(script, game_b.join("checkpoints"));

    let opts_a = PlayOptions {
        game_dir: game_a.clone(),
        seats_path: seats_path.clone(),
        wrapper_path: wrapper_path.clone(),
        wrapper_version: None,
        serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
                if dec_a.seat_kind == "Model" {
                    assert!(dec_a.wall_secs.is_some());
                    assert!(dec_b.wall_secs.is_some());
                }
                dec_a.wall_secs = None;
                dec_b.wall_secs = None;
                assert_eq!(dec_a, dec_b, "normalized decision diverged at {rel_dec}");
            }
        }
    }

    // Identical wire chat-completion request bodies across dup_a & dup_b!
    assert_eq!(transport_a.recorded_bodies(), transport_b.recorded_bodies());

    // C3: Whole-tree recursive diff across `game_a` and `game_b`, masking ONLY:
    //   1. `game_id` in main `.seal`, checkpoint `.seal`s, `decisions/*.json`, `setup.json`, `manifest.json`
    //   2. `pre_act_checkpoint_sha256` in `day-DD-act-recorded.seal`
    //   3. 6 per-call timing/id keys in `relay/calls.jsonl`:
    //      `reservation_id`, `cost`, `charged_credits`, `wall_secs`, `latency_ms`, `time_utc`
    // Notice `EvalSaveSnapshot` (`.save.json`) contains NO `game_id` and must match byte-for-byte unmasked!
    let mut files_a = Vec::new();
    let mut files_b = Vec::new();
    collect_all_files(&game_a, &mut files_a);
    collect_all_files(&game_b, &mut files_b);
    let rels_a: Vec<String> = files_a
        .iter()
        .map(|p| {
            must_ok(p.strip_prefix(&game_a), "strip a")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let rels_b: Vec<String> = files_b
        .iter()
        .map(|p| {
            must_ok(p.strip_prefix(&game_b), "strip b")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(
        rels_a, rels_b,
        "duplicate game directory trees must contain identical file sets"
    );
    assert!(!rels_a.is_empty());

    for rel in &rels_a {
        let pa = game_a.join(rel);
        let pb = game_b.join(rel);
        let raw_a = must_ok(fs::read(&pa), &format!("read game_a/{rel}"));
        let raw_b = must_ok(fs::read(&pb), &format!("read game_b/{rel}"));

        if rel.ends_with(".save.json") {
            assert_eq!(
                raw_a, raw_b,
                "C3: EvalSaveSnapshot `{rel}` has no game_id and must be byte-identical without masking"
            );
        } else if rel == "relay/calls.jsonl" {
            let ta = String::from_utf8_lossy(&raw_a);
            let tb = String::from_utf8_lossy(&raw_b);
            let lines_a: Vec<&str> = ta.lines().collect();
            let lines_b: Vec<&str> = tb.lines().collect();
            assert_eq!(lines_a.len(), lines_b.len());
            for (la, lb) in lines_a.iter().zip(lines_b.iter()) {
                let mut va: Value = must_ok(serde_json::from_str(la), "parse calls a");
                let mut vb: Value = must_ok(serde_json::from_str(lb), "parse calls b");
                for key in [
                    "reservation_id",
                    "http_reply_sha256",
                    "cost",
                    "charged_credits",
                    "wall_secs",
                    "latency_ms",
                    "time_utc",
                ] {
                    va[key] = Value::Null;
                    vb[key] = Value::Null;
                }
                assert_eq!(va, vb, "calls.jsonl normalized row diverged");
            }
        } else if rel.starts_with("relay/seats/") && rel.contains(".http-reply-round-") {
            let mut va: Value = must_ok(serde_json::from_slice(&raw_a), "parse http reply a");
            let mut vb: Value = must_ok(serde_json::from_slice(&raw_b), "parse http reply b");
            assert_eq!(va["body"], vb["body"], "raw http body diverged in `{rel}`");
            assert_eq!(
                va["body_sha256"], vb["body_sha256"],
                "raw http body_sha256 diverged in `{rel}`"
            );
            for key in ["run_id", "reservation_id"] {
                va[key] = Value::Null;
                vb[key] = Value::Null;
            }
            assert_eq!(va, vb, "normalized `{rel}` diverged across duplicate runs");
        } else if rel == "setup.json"
            || rel == "manifest.json"
            || rel.starts_with("decisions/")
            || rel.ends_with(".seal")
        {
            let mut va: Value = must_ok(serde_json::from_slice(&raw_a), "parse json a");
            let mut vb: Value = must_ok(serde_json::from_slice(&raw_b), "parse json b");
            assert_ne!(
                va["game_id"], vb["game_id"],
                "expected distinct game_id in `{rel}`"
            );
            va["game_id"] = Value::Null;
            vb["game_id"] = Value::Null;
            if rel.starts_with("decisions/") {
                va["wall_secs"] = Value::Null;
                vb["wall_secs"] = Value::Null;
            }
            if rel.ends_with("-act-recorded.seal") {
                va["pre_act_checkpoint_sha256"] = Value::Null;
                vb["pre_act_checkpoint_sha256"] = Value::Null;
            }
            assert_eq!(va, vb, "normalized `{rel}` diverged across duplicate runs");
        } else {
            assert_eq!(
                raw_a, raw_b,
                "C3: unmasked file `{rel}` must be byte-identical across duplicate runs"
            );
        }
    }

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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
                serving_manifest_path: None,
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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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

fn encode_byte_needles(label: &str, hex_str: &str) -> Vec<(String, String)> {
    let bytes = must_ok(hex::decode(hex_str), &format!("decode hex `{label}`"));
    let dec_compact = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let dec_spaced = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let json_arr_compact = format!("[{dec_compact}]");
    let json_arr_spaced = format!("[{dec_spaced}]");
    vec![
        (format!("{label}:hex"), hex_str.to_owned()),
        (format!("{label}:json_compact"), json_arr_compact),
        (format!("{label}:json_spaced"), json_arr_spaced),
        (format!("{label}:dec_compact"), dec_compact),
        (format!("{label}:dec_spaced"), dec_spaced),
    ]
}

fn collect_u8_array_needles(label: &str, val: &Value) -> Vec<(String, String)> {
    let Some(arr) = val.as_array() else {
        return Vec::new();
    };
    let mut bytes = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(b) = item.as_u64().and_then(|u| u8::try_from(u).ok()) else {
            return Vec::new();
        };
        bytes.push(b);
    }
    if bytes.is_empty() {
        return Vec::new();
    }
    encode_byte_needles(label, &hex::encode(&bytes))
}

#[test]
fn item15_null_branch_matches_play_receipt_and_leaks_zero_seal_secrets_under_out() {
    let tmp = fresh_tmp_dir("item15_canary");
    let src_game = tmp.join("src_game");
    let branch_out = tmp.join("branch_out");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());

    // Build turn-0 state from fixed explicit entropy (`B6`) so peer vs focal mandates are distinct
    // and every secret material byte array is known structurally.
    let deal_seed = [0xA1_u8; 32];
    let seat_salts = [[0xB1_u8; 32], [0xB2_u8; 32], [0xB3_u8; 32], [0xB4_u8; 32]];
    let eval_seed = [0xC5_u8; 32];
    let turn0 = must_ok(
        EvalHost::from_explicit_entropy_with_model_seats(
            TableId(15),
            4,
            7,
            deal_seed,
            &seat_salts,
            eval_seed,
            0x777,
            &[true, false, false, false],
        ),
        "from_explicit_entropy_with_model_seats item15",
    );
    must_ok(
        persist_eval_game_state(&turn0, &src_game),
        "persist explicit turn0",
    );
    let focal_mandate_json = must_ok(
        serde_json::to_value(must_ok(turn0.mandate_for_seat(Seat(0)), "m0")),
        "ser m0",
    );
    let peer_mandate_jsons: Vec<Value> = (1..=3_u8)
        .map(|s| {
            let m = must_ok(turn0.mandate_for_seat(Seat(s)), "peer mandate");
            must_ok(serde_json::to_value(m), "ser peer mandate")
        })
        .collect();
    for peer_m in &peer_mandate_jsons {
        assert_ne!(
            peer_m, &focal_mandate_json,
            "fixed deal must give distinct mandates to focal and peer seats"
        );
    }

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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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

    // Null branch on focal seat `Foundry`: replays exact recorded turn-2 action across `Off` & `On`.
    let branch_transport = DeterministicMockTransport::new(vec![("2:act", d2_act_content)]);
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(src_game.join("checkpoints/day-02-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path,
                wrapper_path: wrapper_path.clone(),
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
        "null branch",
    );
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].receipt, play_d2_receipt);
    assert_eq!(outcomes[1].receipt, play_d2_receipt);

    // Collect EVERY cryptographic secret needle from `.seal` in hex, JSON array, and decimal list encodings (`B6`):
    //   - `deal_seed`, `eval_seed` (raw entropy)
    //   - all `seat_reveals[*].salt`
    //   - `turn_salts[*]`
    //   - `vrf_proofs[*]`
    let seal_text = must_ok(
        fs::read_to_string(src_game.join("checkpoints/day-02-pre-act.seal")),
        "read pre-act seal",
    );
    let seal_val: Value = must_ok(serde_json::from_str(&seal_text), "parse pre-act seal");
    let mut secret_needles: Vec<(String, String)> = Vec::new();
    let mut positive_in_seal: Vec<String> = Vec::new();

    if let Some(ds) = seal_val["sealed_mandate_deal"]["deal_seed"].as_str() {
        positive_in_seal.push(ds.to_owned());
        secret_needles.extend(encode_byte_needles("deal_seed", ds));
    }
    secret_needles.extend(encode_byte_needles("eval_seed", &hex::encode(eval_seed)));
    if let Some(reveals) = seal_val["sealed_mandate_deal"]["seat_reveals"].as_array() {
        for (idx, rev) in reveals.iter().enumerate() {
            if let Some(s) = rev["salt"].as_str() {
                positive_in_seal.push(s.to_owned());
                secret_needles.extend(encode_byte_needles(&format!("seat_salt_{idx}"), s));
            }
        }
    }
    if let Some(ts) = seal_val["turn_salts"].as_array()
        && let Some(row) = ts.first().and_then(Value::as_array)
        && let Some(salt_hex) = row.first().and_then(Value::as_str)
    {
        positive_in_seal.push(salt_hex.to_owned());
        secret_needles.extend(encode_byte_needles("turn_salt_0_0", salt_hex));
    }
    if let Some(vrf_0) = seal_val["vrf_proofs"].as_array().and_then(|a| a.first()) {
        if let Some(s) = vrf_0.as_str() {
            positive_in_seal.push(s.to_owned());
            secret_needles.extend(encode_byte_needles("vrf_proof_0", s));
        } else {
            let vrf_needles = collect_u8_array_needles("vrf_proof_0", vrf_0);
            if let Some((_, compact_arr)) = vrf_needles
                .iter()
                .find(|(tag, _)| tag.ends_with(":json_compact"))
            {
                positive_in_seal.push(compact_arr.clone());
            }
            secret_needles.extend(vrf_needles);
        }
    }
    assert!(
        positive_in_seal.len() >= 6,
        "expected >= 6 real secret needles in .seal; got {:?}",
        positive_in_seal
    );
    let seal_compact = seal_text.replace([' ', '\n', '\r'], "");
    for p in &positive_in_seal {
        assert!(
            seal_text.contains(p) || seal_compact.contains(p),
            "negative control failed: secret `{p}` missing from .seal"
        );
    }

    fn strip_mandate_deck_array(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(pos) = rest.find("\"mandate_deck\"") {
            let after_key = &rest[pos + "\"mandate_deck\"".len()..];
            let trimmed = after_key.trim_start();
            let Some(after_colon) = trimmed.strip_prefix(':') else {
                out.push_str(&rest[..pos + "\"mandate_deck\"".len()]);
                rest = after_key;
                continue;
            };
            let after_ws = after_colon.trim_start();
            if !after_ws.starts_with('[') {
                out.push_str(&rest[..pos + "\"mandate_deck\"".len()]);
                rest = after_key;
                continue;
            }
            out.push_str(&rest[..pos]);
            let mut depth = 0usize;
            let mut in_str = false;
            let mut escaped = false;
            let mut end_idx = None;
            for (idx, ch) in after_ws.char_indices() {
                if in_str {
                    if escaped {
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        in_str = false;
                    }
                } else {
                    match ch {
                        '"' => in_str = true,
                        '[' => depth += 1,
                        ']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                end_idx = Some(idx + 1);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            match end_idx {
                Some(end) => {
                    out.push_str("\"mandate_deck\":\"<STRIPPED>\"");
                    rest = &after_ws[end..];
                }
                None => {
                    rest = after_ws;
                    break;
                }
            }
        }
        out.push_str(rest);
        out
    }

    fn collect_peer_mandate_needles(
        peer_mandates: &[Value],
        focal_mandate: &Value,
    ) -> Vec<(String, String)> {
        let mut needles = Vec::new();
        let focal_bonus = focal_mandate["bonus_equity"].as_u64();
        for (idx, peer_m) in peer_mandates.iter().enumerate() {
            let compact = must_ok(serde_json::to_string(peer_m), "compact peer mandate");
            let pretty = must_ok(serde_json::to_string_pretty(peer_m), "pretty peer mandate");
            needles.push((format!("peer_{idx}_compact"), compact));
            needles.push((format!("peer_{idx}_pretty"), pretty));
            if let Some(bonus) = peer_m["bonus_equity"].as_u64()
                && Some(bonus) != focal_bonus
            {
                needles.push((
                    format!("peer_{idx}_bonus_pretty"),
                    format!("\"bonus_equity\": {bonus}"),
                ));
                needles.push((
                    format!("peer_{idx}_bonus_compact"),
                    format!("\"bonus_equity\":{bonus}"),
                ));
            }
        }
        needles
    }

    fn scan_single_mandate_surface(surface: &str, text: &str, peer_needles: &[(String, String)]) {
        let no_deck = strip_mandate_deck_array(text);
        let compact_haystack = no_deck.replace([' ', '\n', '\r', '\t'], "");
        for (tag, needle) in peer_needles {
            assert!(
                !no_deck.contains(needle.as_str()),
                "peer mandate `{tag}` leaked into {surface}"
            );
            if !needle.contains('\n') {
                let compact_needle = needle.replace([' ', '\n', '\r', '\t'], "");
                assert!(
                    !compact_haystack.contains(&compact_needle),
                    "peer mandate `{tag}` (whitespace-normalized) leaked into {surface}"
                );
            }
        }
    }

    fn scan_http_body_for_peer_mandates(
        origin: &str,
        body: &str,
        peer_needles: &[(String, String)],
    ) {
        let mut body_val: Value = must_ok(serde_json::from_str(body), "parse HTTP request JSON");
        if let Some(msgs) = body_val["messages"].as_array() {
            for (msg_idx, msg) in msgs.iter().enumerate() {
                if let Some(content) = msg["content"].as_str() {
                    scan_single_mandate_surface(
                        &format!("{origin} messages[{msg_idx}].content"),
                        content,
                        peer_needles,
                    );
                }
            }
        }
        if let Some(obj) = body_val.as_object_mut() {
            obj.remove("messages");
        }
        let non_msg_compact = must_ok(serde_json::to_string(&body_val), "compact non-msg body");
        scan_single_mandate_surface(
            &format!("{origin} non-messages"),
            &non_msg_compact,
            peer_needles,
        );
    }

    let peer_needles = collect_peer_mandate_needles(&peer_mandate_jsons, &focal_mandate_json);
    assert!(
        peer_needles.iter().any(|(tag, _)| tag.contains("_bonus_")),
        "expected distinct peer bonus_equity needles for positive control"
    );
    let seal_no_deck = strip_mandate_deck_array(&seal_text);
    let seal_no_deck_compact = seal_no_deck.replace([' ', '\n', '\r', '\t'], "");
    for (idx, peer_m) in peer_mandate_jsons.iter().enumerate() {
        let kind_compact = must_ok(serde_json::to_string(&peer_m["kind"]), "peer kind compact");
        assert!(
            seal_no_deck_compact.contains(&kind_compact),
            "positive control failed: peer_{idx}_kind (`{kind_compact}`) missing from .seal"
        );
    }
    for (tag, needle) in &peer_needles {
        if tag.contains("_bonus_") {
            let needle_compact = needle.replace([' ', '\n', '\r', '\t'], "");
            assert!(
                seal_no_deck.contains(needle.as_str())
                    || seal_no_deck_compact.contains(&needle_compact),
                "positive control failed: `{tag}` (`{needle}`) missing from .seal"
            );
        }
    }

    for (req_idx, body) in play_transport.recorded_bodies().iter().enumerate() {
        scan_http_body_for_peer_mandates(
            &format!("play_transport req {req_idx}"),
            body,
            &peer_needles,
        );
    }
    for (req_idx, body) in branch_transport.recorded_bodies().iter().enumerate() {
        scan_http_body_for_peer_mandates(
            &format!("branch_transport req {req_idx}"),
            body,
            &peer_needles,
        );
    }

    // Verify ZERO `.seal` files, ZERO encoded secret needles, and ZERO peer mandates anywhere under `--out`.
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
        for (tag, needle) in &secret_needles {
            assert!(
                !content.contains(needle),
                "secret `{tag}` (`{needle}`) leaked into {}",
                file.display()
            );
        }
        scan_single_mandate_surface(&file.display().to_string(), &content, &peer_needles);
        // Check structural peer mandate equality inside focal `to-seat/*.md` (`SeatObservation.own_mandate`),
        // and confirm NO mandate field appears inside public `receipt.json` / `branch_summary.json`.
        if name.ends_with(".md")
            && name.starts_with("day-")
            && let Some(json_part) = content.split("\n\n").nth(2)
        {
            let obs_val: Value = must_ok(serde_json::from_str(json_part), "parse view JSON");
            assert_eq!(obs_val["own_mandate"], focal_mandate_json);
            for (peer_idx, peer_m) in peer_mandate_jsons.iter().enumerate() {
                assert_ne!(
                    &obs_val["own_mandate"],
                    peer_m,
                    "peer seat {} mandate leaked into focal own_mandate",
                    peer_idx + 1
                );
            }
        }
        if name == "receipt.json" || name == "branch_summary.json" {
            let pretty_focal = must_ok(
                serde_json::to_string_pretty(&focal_mandate_json),
                "pretty focal mandate",
            );
            assert!(!content.contains(&pretty_focal));
            for peer_m in &peer_mandate_jsons {
                let pretty_peer =
                    must_ok(serde_json::to_string_pretty(peer_m), "pretty peer mandate");
                assert!(!content.contains(&pretty_peer));
            }
        }
    }

    // 2. B6 recorded peer `act` submission isolation scan:
    //    Play a 2-model-seat game (`Foundry` + peer `Quarry`) where Quarry submits unique distinguishable
    //    orders (`price_per_unit_cred: 9876, amount: 4321`) stored in `day-01-act-recorded.seal`,
    //    then branch focal `Foundry` across [Off, On] and assert focal HTTP request bodies never contain `"9876"` or `"4321"`.
    struct RoleAwareTransport {
        bodies: Mutex<Vec<String>>,
    }
    impl Transport for RoleAwareTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            {
                let mut g = match self.bodies.lock() {
                    Ok(g) => g,
                    Err(e) => panic!("lock: {e}"),
                };
                g.push(body.to_owned());
            }
            let v: Value =
                serde_json::from_str(body).map_err(|e| TransportError::Network(e.to_string()))?;
            let user_msg = v["messages"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default();
            let header = user_msg.split("\"phase\"").next().unwrap_or(user_msg);
            let is_talk = header.contains("This is the talk round.");
            let is_quarry = header.contains("\"seat\": 1,");
            let content = if !is_talk && is_quarry {
                json!({
                    "messages": [],
                    "market_actions": [{
                        "action_kind": "post_offer",
                        "side": "buy",
                        "good": { "asset_kind": "raw", "good_index": 0 },
                        "amount": 4321,
                        "price_per_unit_cred": 9876,
                        "domain_id": null,
                        "is_hybrid": false
                    }],
                    "bundle": {}
                })
                .to_string()
            } else {
                json!({ "messages": [], "market_actions": [], "bundle": {} }).to_string()
            };
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_response_value(
                    "b6-role",
                    "qwen/qwen3-32b",
                    &content,
                    20,
                    10,
                    Some(0.001),
                )
                .to_string(),
            })
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("TEMPLATED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            assert!(add_special && parse_special);
            Ok((100_u64..120).collect())
        }
    }

    let two_model_seats = write_seats(
        &tmp,
        "two_model_seats.json",
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
                    "max_gpu_secs_per_call": 10.0,
                    "scratchpad": false
                },
                {
                    "guild": "Quarry",
                    "backend": "local",
                    "base_url": "http://127.0.0.1:18080/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_gpu_secs_per_call": 10.0,
                    "scratchpad": false
                },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    let b6_src = tmp.join("b6_peer_orders_src");
    let b6_play_t = RoleAwareTransport {
        bodies: Mutex::new(Vec::new()),
    };
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: b6_src.clone(),
                seats_path: two_model_seats.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 16,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &b6_play_t,
            None,
        ),
        "b6 2-model play",
    );
    let rec_seal = must_ok(
        fs::read_to_string(b6_src.join("checkpoints/day-01-act-recorded.seal")),
        "read recorded seal",
    );
    assert!(
        rec_seal.contains("9876") && rec_seal.contains("4321"),
        "positive control failed: peer recorded act submission missing from recorded seal"
    );

    let b6_branch_out = tmp.join("b6_peer_orders_branch");
    let b6_branch_t = RoleAwareTransport {
        bodies: Mutex::new(Vec::new()),
    };
    let _ = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(b6_src.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: two_model_seats,
                wrapper_path,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: b6_branch_out.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &b6_branch_t,
            None,
        ),
        "b6 focal branch",
    );
    let b6_host = must_ok(
        EvalHost::read_game_save_and_seal(&b6_src, TableId(16)),
        "read b6 save and seal",
    );
    let b6_focal_mandate = must_ok(
        serde_json::to_value(must_ok(b6_host.mandate_for_seat(Seat(0)), "b6 m0")),
        "ser b6 m0",
    );
    let b6_peer_mandates: Vec<Value> = (1..=3_u8)
        .map(|s| {
            let m = must_ok(b6_host.mandate_for_seat(Seat(s)), "b6 peer mandate");
            must_ok(serde_json::to_value(m), "ser b6 peer mandate")
        })
        .collect();
    let b6_peer_needles = collect_peer_mandate_needles(&b6_peer_mandates, &b6_focal_mandate);
    let b6_seal_text = must_ok(
        fs::read_to_string(b6_src.join("checkpoints/day-01-pre-act.seal")),
        "read b6 pre-act seal",
    );
    let b6_seal_no_deck = strip_mandate_deck_array(&b6_seal_text);
    let b6_seal_compact = b6_seal_no_deck.replace([' ', '\n', '\r', '\t'], "");
    for (idx, peer_m) in b6_peer_mandates.iter().enumerate() {
        let kind_compact = must_ok(
            serde_json::to_string(&peer_m["kind"]),
            "b6 peer kind compact",
        );
        assert!(
            b6_seal_compact.contains(&kind_compact),
            "b6 positive control failed: peer_{idx}_kind (`{kind_compact}`) missing from .seal"
        );
    }
    for (tag, needle) in &b6_peer_needles {
        if tag.contains("_bonus_") {
            let needle_compact = needle.replace([' ', '\n', '\r', '\t'], "");
            assert!(
                b6_seal_no_deck.contains(needle.as_str())
                    || b6_seal_compact.contains(&needle_compact),
                "b6 positive control failed: `{tag}` (`{needle}`) missing from .seal"
            );
        }
    }

    let b6_play_bodies = match b6_play_t.bodies.lock() {
        Ok(g) => g.clone(),
        Err(e) => panic!("lock: {e}"),
    };
    let mut b6_focal_play_reqs = 0usize;
    for (idx, body) in b6_play_bodies.iter().enumerate() {
        let v: Value = must_ok(serde_json::from_str(body), "parse b6 play body");
        let user_msg = v["messages"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default();
        let header = user_msg.split("\"phase\"").next().unwrap_or(user_msg);
        if header.contains("\"seat\": 0,") || header.contains("\"seat\":0,") {
            b6_focal_play_reqs += 1;
            scan_http_body_for_peer_mandates(
                &format!("b6_play focal req {idx}"),
                body,
                &b6_peer_needles,
            );
        }
    }
    assert!(
        b6_focal_play_reqs >= 2,
        "expected >= 2 focal play requests in b6_play_t; got {b6_focal_play_reqs}"
    );

    let mut b6_out_files = Vec::new();
    collect_all_files(&b6_branch_out, &mut b6_out_files);
    assert!(!b6_out_files.is_empty());
    for file in b6_out_files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            !name.ends_with(".seal"),
            "private .seal file must never appear under --out: {}",
            file.display()
        );
        let content = must_ok(fs::read_to_string(&file), "read b6 out file");
        scan_single_mandate_surface(&file.display().to_string(), &content, &b6_peer_needles);
    }

    let focal_bodies = match b6_branch_t.bodies.lock() {
        Ok(g) => g.clone(),
        Err(e) => panic!("lock: {e}"),
    };
    assert_eq!(focal_bodies.len(), 2);
    for (idx, body) in focal_bodies.iter().enumerate() {
        assert!(
            !body.contains("9876") && !body.contains("4321"),
            "peer recorded act submission leaked into focal HTTP request body: {body}"
        );
        scan_http_body_for_peer_mandates(
            &format!("b6_branch focal req {idx}"),
            body,
            &b6_peer_needles,
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn item14_select_determinism_strata_and_branch_list_end_to_end() {
    struct AlwaysEmptyModelTransport;
    impl Transport for AlwaysEmptyModelTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_response_value(
                    "m-empty",
                    "qwen/qwen3-32b",
                    "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
                    10,
                    10,
                    Some(0.001),
                )
                .to_string(),
            })
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("TEMPLATED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            assert!(add_special && parse_special);
            Ok((100_u64..110).collect())
        }
    }

    let tmp = fresh_tmp_dir("item14_select");
    let game_a = tmp.join("game_a");
    let game_b = tmp.join("game_b");
    let wrapper_path = write_wrapper(&tmp);

    // Mixed 2-Model + 2-ClubBot table (`C8`): `Foundry` (seat 0) and `Harbor` (seat 2) are Model seats,
    // `Quarry` (seat 1) and `Mint` (seat 3) are `ClubBot` seats.
    let mixed_seats_path = write_seats(
        &tmp,
        "seats.json",
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
                    "max_gpu_secs_per_call": 10.0,
                    "scratchpad": false
                },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                {
                    "guild": "Harbor",
                    "backend": "local",
                    "base_url": "http://127.0.0.1:18080/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_gpu_secs_per_call": 10.0,
                    "scratchpad": false
                },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );

    let empty_t = AlwaysEmptyModelTransport;
    for (tid, gd) in [(21_u64, &game_a), (22_u64, &game_b)] {
        let _ = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: gd.clone(),
                    seats_path: mixed_seats_path.clone(),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
                &empty_t,
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

    // C8: select NEVER picks ClubBot seats (`Quarry`, `Mint`); every state must be `Foundry` or `Harbor`.
    for s in &sel_1.states {
        assert!(
            matches!(s.seat_guild.as_str(), "Foundry" | "Harbor"),
            "C8: select picked ClubBot seat {} (index {})",
            s.seat_guild,
            s.seat_index
        );
        assert!(matches!(s.seat_index, 0 | 2));
    }

    // C8 negative control: an all-ClubBot game is refused with NO_MODEL_SEATS_FOR_SELECT.
    let all_bot_seats = write_seats(&tmp, "all_bot_seats.json", all_clubbot_seats());
    let bot_only_game = tmp.join("bot_only_game");
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: bot_only_game.clone(),
                seats_path: all_bot_seats,
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 24,
                seed: 24,
                turns: 3,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        ),
        "all-ClubBot play for select refusal",
    );
    let no_model_err = run_select(&SelectOptions {
        game_dirs: vec![bot_only_game],
        per_game: 1,
        strata: [1, 0, 0],
        selection_seed: 1,
        out_path: tmp.join("no_model_sel.json"),
    });
    assert!(matches!(
        no_model_err,
        Err(EvalError::NoModelSeatsForSelect(_))
    ));

    // C12: verify `<home>` path sanitization & expansion (`sanitize_home_path*` & `expand_home_path*`).
    let sample_home = PathBuf::from("/home/evaluator");
    let sub_path = PathBuf::from("/home/evaluator/runs/g1/checkpoints/day-01-pre-act");
    let sanitized = sanitize_home_path_with_homes(&sub_path, std::slice::from_ref(&sample_home));
    assert_eq!(
        sanitized.to_string_lossy().replace('\\', "/"),
        "<home>/runs/g1/checkpoints/day-01-pre-act"
    );
    assert_eq!(
        expand_home_path_with_home(&sanitized, Some(&sample_home)),
        sub_path
    );
    assert_eq!(
        sanitize_home_path_with_homes(&sample_home, std::slice::from_ref(&sample_home)),
        PathBuf::from("<home>")
    );
    assert_eq!(
        expand_home_path_with_home(Path::new("<home>"), Some(&sample_home)),
        sample_home
    );
    assert_eq!(
        sanitize_home_path_with_homes(Path::new("/opt/other"), std::slice::from_ref(&sample_home)),
        PathBuf::from("/opt/other")
    );
    if let Some(env_home) = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
    {
        let home_pb = PathBuf::from(env_home);
        let inside = home_pb.join("eval_runs").join("g1");
        let clean = league_eval::sanitize_home_path(&inside);
        assert_eq!(
            clean.to_string_lossy().replace('\\', "/"),
            "<home>/eval_runs/g1"
        );
        assert_eq!(league_eval::expand_home_path(&clean), inside);
    }

    // Verify exact [2, 2, 2] early/mid/late counts per game (`early = 1..=3`, `mid = 4..=6`, `late = 7..=9`).
    for gd in [&game_a, &game_b] {
        let san_gd = league_eval::sanitize_home_path(gd);
        let game_states: Vec<&SelectedPreActState> = sel_1
            .states
            .iter()
            .filter(|s| s.game_dir == san_gd || &s.game_dir == gd)
            .collect();
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

    // Exercise `select` + `branch --list` via CLI with offline `--mock-dir`.
    let mock_dir = tmp.join("select_mock_dir");
    must_ok(fs::create_dir_all(&mock_dir), "mkdir select_mock_dir");
    let mock_resp = json!({
        "apply_template_prompt": "TEMPLATED MOCK PROMPT",
        "tokenize_ids": [101, 102, 103, 104, 105, 106, 107, 108, 109, 110],
        "rules": [{
            "when_contains": "This is the act round.",
            "status": 200,
            "response": mock_chat_response_value(
                "m-act",
                "qwen/qwen3-32b",
                "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
                10,
                10,
                Some(0.001),
            )
        }]
    });
    must_ok(
        fs::write(mock_dir.join("responses.json"), mock_resp.to_string()),
        "write select mock responses",
    );

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
            mixed_seats_path.clone().into_os_string(),
            "--wrapper".into(),
            wrapper_path.clone().into_os_string(),
            "--serving-manifest".into(),
            write_valid_serving_manifest(&tmp).into_os_string(),
            "--arms".into(),
            "off,on".into(),
            "--out".into(),
            list_branch_out.clone().into_os_string(),
            "--mock-dir".into(),
            mock_dir.into_os_string(),
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
    assert!(list_branch_out.join("branch_summary.json").exists());

    // Error paths on `select`: insufficient turns and strata sum mismatch.
    let short_game = tmp.join("short_game");
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: short_game.clone(),
                seats_path: mixed_seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
            &empty_t,
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

    // Control arm runs cleanly on all-ClubBot tables and unpinned wrapper refuses with UNPINNED_WRAPPER (`--error-format json`).
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", all_clubbot_seats());
    out.clear();
    err.clear();
    let ctrl_dir = tmp.join("ctrl_game");
    let ctrl_code = execute_cli(
        [
            "--error-format".into(),
            "json".into(),
            "play".into(),
            "--game-dir".into(),
            ctrl_dir.clone().into_os_string(),
            "--seats".into(),
            seats_path.clone().into_os_string(),
            "--wrapper".into(),
            wrapper_path.clone().into_os_string(),
            "--arm".into(),
            "control".into(),
            "--turns".into(),
            "1".into(),
            "--seed".into(),
            "7".into(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(ctrl_code, 0, "stderr: {}", String::from_utf8_lossy(&err));
    let ctrl_manifest: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(ctrl_dir.join("manifest.json")),
            "read ctrl manifest",
        )),
        "parse ctrl manifest",
    );
    assert_eq!(ctrl_manifest.arm, EvalArm::Control);
    assert_eq!(ctrl_manifest.relay_wording_arm, "v2_control_2026_11_21_r1");

    let unpinned_wrap = tmp.join("unpinned_cli.md");
    must_ok(fs::write(&unpinned_wrap, b"# unpinned"), "write unpinned");
    err.clear();
    let bad_wrap_code = execute_cli(
        [
            "--error-format".into(),
            "json".into(),
            "play".into(),
            "--game-dir".into(),
            tmp.join("unpinned_game").into_os_string(),
            "--seats".into(),
            seats_path.clone().into_os_string(),
            "--wrapper".into(),
            unpinned_wrap.into_os_string(),
            "--arm".into(),
            "control".into(),
            "--seed".into(),
            "7".into(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(bad_wrap_code, 1);
    let err_json: Value = must_ok(serde_json::from_slice(&err), "parse JSON error on stderr");
    assert_eq!(err_json["code"], json!("UNPINNED_WRAPPER"));

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

    // `--mock-dir` CLI execution on `play` & `branch` with a Local seat without notes!
    let mock_dir = tmp.join("mock_dir");
    must_ok(fs::create_dir_all(&mock_dir), "mkdir mock_dir");
    let mock_resp = json!({
        "apply_template_prompt": "TEMPLATED MOCK PROMPT",
        "tokenize_ids": [101, 102, 103, 104, 105, 106, 107, 108, 109, 110],
        "rules": [
            {
                "when_contains": "This is the talk round.",
                "status": 200,
                "response": mock_chat_response_value(
                    "m1",
                    "qwen/qwen3-32b",
                    "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
                    10,
                    10,
                    Some(0.001),
                )
            },
            {
                "when_contains": "This is the act round.",
                "status": 200,
                "response": mock_chat_response_value(
                    "m2",
                    "qwen/qwen3-32b",
                    "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
                    10,
                    10,
                    Some(0.001),
                )
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
                    "backend": "local",
                    "base_url": "http://127.0.0.1:18080/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_gpu_secs_per_call": 10.0,
                    "scratchpad": false
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
            "--serving-manifest".into(),
            write_valid_serving_manifest(&tmp).into_os_string(),
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
            "--serving-manifest".into(),
            write_valid_serving_manifest(&tmp).into_os_string(),
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

    // Cover all 31 `EvalError` variants across `code()`, `exit_code()`, `Display`, and `to_structured_error()`.
    let all_errors: [EvalError; 31] = [
        EvalError::Usage("bad flag OPENROUTER_API_KEY=sk-or-secret123".to_owned()),
        EvalError::InvalidSeatCount { actual: 3 },
        EvalError::UnsupportedFileSeatModel {
            guild: "Foundry".to_owned(),
            model: Some("Other".to_owned()),
        },
        EvalError::UnpinnedWrapper("unpinned wrapper".to_owned()),
        EvalError::MissingServingManifest("missing serving manifest".to_owned()),
        EvalError::PrefixCacheEnabled("prefix cache enabled".to_owned()),
        EvalError::ParallelSlotsExceeded("parallel slots 2".to_owned()),
        EvalError::MissingModelSha256("missing model sha".to_owned()),
        EvalError::MissingGpuName("missing gpu".to_owned()),
        EvalError::InvalidGpuUuidSha256("bad gpu uuid sha".to_owned()),
        EvalError::ModelSeatRegentIntervention("regent acted on model seat".to_owned()),
        EvalError::MissingWrapper("missing wrapper".to_owned()),
        EvalError::SchemaValidation("bad schema".to_owned()),
        EvalError::CheckpointNotFound("missing ckpt".to_owned()),
        EvalError::InsufficientTurnsForStrata("too short".to_owned()),
        EvalError::InvalidStrata("bad strata".to_owned()),
        EvalError::UnknownFocalSeat("Phantom".to_owned()),
        EvalError::EvalThinkStrippingForbidden("exclude true".to_owned()),
        EvalError::MissingBranchTranscript("missing transcript".to_owned()),
        EvalError::MissingBranchScratchpad("missing scratchpad".to_owned()),
        EvalError::MissingBranchNegotiation("missing neg".to_owned()),
        EvalError::BranchArmDirNotEmpty("dir non empty".to_owned()),
        EvalError::BranchSummaryExists("summary exists".to_owned()),
        EvalError::InvalidWrongPartRate("invalid rate".to_owned()),
        EvalError::MissingSourceSetup("missing setup".to_owned()),
        EvalError::IncompatibleEvalSetup("incompatible setup".to_owned()),
        EvalError::PartialTurnFromSeatExists("partial day reply".to_owned()),
        EvalError::NoModelSeatsForSelect("no model seats".to_owned()),
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
            serving_manifest_path: None,
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
    assert!(matches!(
        mismatch_err,
        Err(EvalError::IncompatibleEvalSetup(_))
    ));

    // Branch unknown focal seat & missing checkpoint prefix -> clean named error codes.
    let unk = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(game_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("NoSuchGuild".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: None,
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
            serving_manifest_path: None,
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

    let ctrl_ok = must_ok(
        Command::new(bin)
            .args([
                "play",
                "--game-dir",
                &tmp.join("bin_ctrl").to_string_lossy(),
                "--seats",
                &seats_path.to_string_lossy(),
                "--wrapper",
                &wrapper_path.to_string_lossy(),
                "--arm",
                "control",
                "--turns",
                "1",
                "--seed",
                "7",
            ])
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .output(),
        "run league-eval control arm binary",
    );
    assert!(ctrl_ok.status.success());

    let bad_wrap_bin = tmp.join("bad_wrap_bin.md");
    must_ok(fs::write(&bad_wrap_bin, b"# wrong"), "write bad_wrap_bin");
    let err_out = must_ok(
        Command::new(bin)
            .args([
                "--error-format",
                "json",
                "play",
                "--game-dir",
                &tmp.join("bin_bad_wrap").to_string_lossy(),
                "--seats",
                &seats_path.to_string_lossy(),
                "--wrapper",
                &bad_wrap_bin.to_string_lossy(),
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
    assert!(String::from_utf8_lossy(&err_out.stderr).contains("UNPINNED_WRAPPER"));

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
            "v2_seat_brief_2026_11_21_r3".into(),
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

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("TEMPLATED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            assert!(add_special && parse_special);
            Ok((100_u64..110).collect())
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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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

    // Copy checkpoints to a standalone directory (no `setup.json` or `to-seat/*.negotiation.json`):
    // B4 forbids silent fallbacks and refuses with MISSING_SOURCE_SETUP then MISSING_BRANCH_NEGOTIATION.
    let standalone_root = tmp.join("standalone");
    let standalone_ckpts = standalone_root.join("checkpoints");
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
    let missing_setup_err = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(standalone_ckpts.join("day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: None,
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
    );
    assert!(matches!(
        missing_setup_err,
        Err(EvalError::MissingSourceSetup(_))
    ));

    // Add matching setup.json without negotiation.json -> MISSING_BRANCH_NEGOTIATION (`B4`).
    must_ok(
        fs::copy(
            play_with_extra_flags.join("setup.json"),
            standalone_root.join("setup.json"),
        ),
        "copy setup.json into standalone",
    );
    let missing_neg_err = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(standalone_ckpts.join("day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: None,
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("standalone_branch_out_neg"),
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
        missing_neg_err,
        Err(EvalError::MissingBranchNegotiation(_))
    ));

    // Corrupt `setup.json` game_id, empty wrapper, `--turns 33`, malformed transcript, missing act-recorded seal.
    let empty_wrapper = tmp.join("empty_wrapper.md");
    must_ok(fs::write(&empty_wrapper, b""), "empty wrapper");
    for opts_err in [
        PlayOptions {
            game_dir: tmp.join("g_t33"),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: None,
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
            serving_manifest_path: None,
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
            serving_manifest_path: None,
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

    let orig_setup_raw = must_ok(
        fs::read_to_string(play_with_extra_flags.join("setup.json")),
        "read setup",
    );
    let mut bad_setup: Value = must_ok(serde_json::from_str(&orig_setup_raw), "parse setup");
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
                serving_manifest_path: None,
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
    must_ok(
        fs::write(play_with_extra_flags.join("setup.json"), &orig_setup_raw),
        "restore setup",
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
                serving_manifest_path: None,
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
                serving_manifest_path: None,
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
                serving_manifest_path: None,
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
                serving_manifest_path: None,
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
                serving_manifest_path: None,
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
                serving_manifest_path: None,
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
                seats_path: local_seats_path,
                wrapper_path,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
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
            game_dirs: vec![fail_game.clone()],
            per_game: 1,
            strata: [0, 1, 0],
            selection_seed: 1,
            out_path: tmp.join("sel_e6.json"),
        })
        .is_err()
    );

    // 5. Additional CLI (`cmd_branch` default arms, `--error-format=text`, missing value EOF) & `write_text_atomic` edge paths.
    assert_eq!(
        execute_cli(
            ["--error-format=text".into(), "--version".into()],
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(
        execute_cli(["play".into(), "--game-dir".into()], &mut out, &mut err),
        1
    );
    assert_eq!(
        execute_cli(
            [
                "branch".into(),
                "--error-format".into(),
                "text".into(),
                "--checkpoint".into(),
                play_with_extra_flags
                    .join("checkpoints/day-01-pre-act")
                    .into_os_string(),
                "--seat".into(),
                "Foundry".into(),
                "--seats".into(),
                tmp.join("seats.json").into_os_string(),
                "--wrapper".into(),
                tmp.join("wrapper.md").into_os_string(),
                "--out".into(),
                tmp.join("branch_default_arms").into_os_string(),
                "--memory-mode".into(),
                "snapshot".into(),
                "--cap-credits".into(),
                "1.5".into(),
                "--cap-gpu-secs".into(),
                "4.0".into(),
            ],
            &mut out,
            &mut err
        ),
        0
    );
    let bad_list = tmp.join("bad_list.json");
    must_ok(fs::write(&bad_list, b"{not-json"), "write bad_list");
    assert_eq!(
        execute_cli(
            [
                "branch".into(),
                "--list".into(),
                bad_list.into_os_string(),
                "--seats".into(),
                tmp.join("seats.json").into_os_string(),
                "--wrapper".into(),
                tmp.join("wrapper.md").into_os_string(),
                "--arm".into(),
                "off".into(),
                "--out".into(),
                tmp.join("branch_bad_list").into_os_string(),
            ],
            &mut out,
            &mut err
        ),
        2
    );

    // Pre-existing `.tmp.<pid>` cleanup and overwrite in `write_text_atomic` via `run_select`.
    let sel_overwrite = tmp.join("sel_overwrite.json");
    let stale_tmp = tmp.join(format!("sel_overwrite.json.tmp.{}", std::process::id()));
    must_ok(fs::write(&stale_tmp, b"stale"), "write stale tmp");
    must_ok(fs::write(&sel_overwrite, b"{}"), "write prior target");
    assert!(
        run_select(&SelectOptions {
            game_dirs: vec![fail_game],
            per_game: 1,
            strata: [1, 0, 0],
            selection_seed: 7,
            out_path: sel_overwrite,
        })
        .is_ok()
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn b4_b5_c2_c7_c11_end_to_end_contracts() {
    let tmp = fresh_tmp_dir("fix1_contracts");
    let game_dir = tmp.join("game");
    let wrapper_path = write_wrapper(&tmp);
    let alt_wrapper = tmp.join("alt_wrapper.md");
    must_ok(
        fs::write(&alt_wrapper, "# Alt Wrapper\nDifferent instructions.\n"),
        "write alt wrapper",
    );
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());
    let alt_seats_path = write_seats(
        &tmp,
        "alt_seats.json",
        json!({
            "seats": [
                {
                    "guild": "Foundry",
                    "backend": "local",
                    "base_url": "http://127.0.0.1:18080/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.25,
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

    // Build a 2-model-seat table (`Foundry` + `Quarry` Model, `Harbor` + `Mint` ClubBot) so `Quarry`
    // can deliver a real whisper to `Foundry` on Day 2 talk (`B4` whisper prompt reconstruction!),
    // AND `Foundry` can trigger `MESSAGE_RECIPIENT_INVALID` + wrong-part refusal for `B5` manifest recount!
    let two_model_path = write_seats(
        &tmp,
        "two_model_seats.json",
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
                {
                    "guild": "Quarry",
                    "backend": "local",
                    "base_url": "http://127.0.0.1:18081/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_gpu_secs_per_call": 10.0
                },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );

    struct ContractTransport {
        bodies: Mutex<Vec<String>>,
    }
    impl Transport for ContractTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            {
                let mut g = match self.bodies.lock() {
                    Ok(g) => g,
                    Err(e) => panic!("lock: {e}"),
                };
                g.push(body.to_owned());
            }
            let v: Value =
                serde_json::from_str(body).map_err(|e| TransportError::Network(e.to_string()))?;
            let user_msg = v["messages"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default();
            let header = user_msg.split("\"phase\"").next().unwrap_or(user_msg);
            let is_talk = header.contains("This is the talk round.");
            let is_turn_1 = header.contains("\"turn\": 1,");
            let is_turn_2 = header.contains("\"turn\": 2,");
            let is_foundry = header.contains("\"seat\": 0,");
            let content = match (is_foundry, is_turn_1, is_turn_2, is_talk) {
                (true, true, false, true) => json!({
                    "messages": [{ "to_guild": "Phantom", "body": "bad recp d1" }],
                    "market_actions": [],
                    "bundle": {},
                    "notes": "f d1 talk"
                })
                .to_string(),
                (false, _, true, true) => json!({
                    "messages": [{ "to_guild": "Foundry", "body": "WHISPER_TOKEN_B4_D2" }],
                    "market_actions": [],
                    "bundle": {},
                    "notes": "q d2 whisper"
                })
                .to_string(),
                (true, false, true, true) => json!({
                    "messages": [],
                    "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 0 }, "amount": 5 }],
                    "bundle": {},
                    "notes": "f d2 talk wrong part"
                })
                .to_string(),
                (true, _, _, false) => json!({
                    "messages": [],
                    "market_actions": [],
                    "bundle": {},
                    "notes": "f act clean"
                })
                .to_string(),
                _ => json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "clean" }).to_string(),
            };
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_response_value(
                    "ct-1",
                    "Qwen/Qwen3-32B",
                    &content,
                    20,
                    10,
                    Some(0.001),
                )
                .to_string(),
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

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("TEMPLATED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            assert!(add_special && parse_special);
            Ok((100_u64..120).collect())
        }
    }

    // 1. B5 per-turn manifest: play turn 1 -> manifest.json exists after turn 1; play turn 2 -> remove manifest.json ->
    //    resume turn 3 -> manifest.json recovers exact turn 1–2 counts (`MESSAGE_RECIPIENT_INVALID`, `TALK_REPLY_HAS_ORDERS`, replies).
    let play_t = ContractTransport {
        bodies: Mutex::new(Vec::new()),
    };
    let m1 = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: two_model_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 120,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_t,
            None,
        ),
        "b5 turn 1",
    );
    assert_eq!(m1.played_turns, 1);
    assert_eq!(
        m1.refusal_counts_by_seat["Foundry"].message_recipient_invalid,
        1
    );
    assert!(game_dir.join("manifest.json").exists());

    // Continue through turn 2.
    let m2 = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: two_model_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 120,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_t,
            None,
        ),
        "b5 turn 2",
    );
    assert_eq!(m2.played_turns, 2);
    assert_eq!(
        m2.refusal_counts_by_seat["Foundry"].talk_reply_has_orders,
        1
    );
    assert_eq!(
        m2.refusal_counts_by_seat["Foundry"].message_recipient_invalid,
        1
    );
    assert_eq!(m2.wrong_part_refusals.delivered_model_replies, 8);

    // Simulate crash right before final manifest write on turn 2 by deleting `manifest.json`,
    // then resume turn 3 and verify recounted `manifest.json` loses zero counts!
    must_ok(
        fs::remove_file(game_dir.join("manifest.json")),
        "delete manifest",
    );
    let m3 = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: two_model_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 120,
                seed: 7,
                turns: 3,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_t,
            None,
        ),
        "b5 turn 3 after deleted manifest",
    );
    assert_eq!(m3.played_turns, 3);
    assert_eq!(
        m3.refusal_counts_by_seat["Foundry"].talk_reply_has_orders,
        1
    );
    assert_eq!(
        m3.refusal_counts_by_seat["Foundry"].message_recipient_invalid,
        1
    );
    assert_eq!(m3.wrong_part_refusals.delivered_model_replies, 12);

    // 2. B5 partial-turn resume refusal (`PARTIAL_TURN_FROM_SEAT_EXISTS`) when day-04 `from-seat` file already exists.
    must_ok(
        fs::write(
            game_dir.join("from-seat/day-04-talk-Foundry.md"),
            b"{\"messages\":[],\"market_actions\":[],\"bundle\":{},\"notes\":\"x\"}",
        ),
        "plant partial day-04 reply",
    );
    let partial_err = run_play_with_transport(
        &PlayOptions {
            game_dir: game_dir.clone(),
            seats_path: two_model_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arm: EvalArm::Off,
            table_id: 120,
            seed: 7,
            turns: 4,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &play_t,
        None,
    );
    assert!(
        matches!(partial_err, Err(EvalError::PartialTurnFromSeatExists(_))),
        "expected PARTIAL_TURN_FROM_SEAT_EXISTS, got {partial_err:?}"
    );

    // 3. C11 resume & branch checks for mismatched `--memory-mode`, `--seats`, and `--wrapper`.
    // First confirm unpinned `alt_wrapper` is refused cleanly with `UNPINNED_WRAPPER`, then test
    // `INCOMPATIBLE_EVAL_SETUP` when `setup.json` records a different `wrapper_sha256`.
    let unpinned_resume = run_play_with_transport(
        &PlayOptions {
            game_dir: game_dir.clone(),
            seats_path: two_model_path.clone(),
            wrapper_path: alt_wrapper.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arm: EvalArm::Off,
            table_id: 120,
            seed: 7,
            turns: 4,
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
        unpinned_resume,
        Err(EvalError::UnpinnedWrapper(_))
    ));
    let unpinned_branch = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(game_dir.join("checkpoints/day-02-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: two_model_path.clone(),
            wrapper_path: alt_wrapper,
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("out_unpinned_branch"),
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
        unpinned_branch,
        Err(EvalError::UnpinnedWrapper(_))
    ));

    let setup_file_path = game_dir.join("setup.json");
    let orig_setup_bytes = must_ok(fs::read(&setup_file_path), "read original setup.json");
    for (label, tamper_wrapper_sha, bad_seats, bad_mem) in [
        (
            "resume_bad_wrapper",
            true,
            two_model_path.clone(),
            league_relay::MemoryMode::Snapshot,
        ),
        (
            "resume_bad_seats",
            false,
            alt_seats_path.clone(),
            league_relay::MemoryMode::Snapshot,
        ),
        (
            "resume_bad_mem",
            false,
            two_model_path.clone(),
            league_relay::MemoryMode::Emc,
        ),
    ] {
        if tamper_wrapper_sha {
            let mut v: Value = must_ok(serde_json::from_slice(&orig_setup_bytes), "parse setup");
            v["wrapper_sha256"] = json!("0".repeat(64));
            must_ok(
                fs::write(&setup_file_path, v.to_string()),
                "tamper setup.json",
            );
        }
        let e = run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: bad_seats,
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 120,
                seed: 7,
                turns: 4,
                memory_mode: bad_mem,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        );
        must_ok(
            fs::write(&setup_file_path, &orig_setup_bytes),
            "restore setup.json",
        );
        assert!(
            matches!(e, Err(EvalError::IncompatibleEvalSetup(_))),
            "{label}: expected INCOMPATIBLE_EVAL_SETUP, got {e:?}"
        );
    }
    for (label, tamper_wrapper_sha, bad_seats, bad_mem) in [
        (
            "branch_bad_wrapper",
            true,
            two_model_path.clone(),
            league_relay::MemoryMode::Snapshot,
        ),
        (
            "branch_bad_seats",
            false,
            alt_seats_path,
            league_relay::MemoryMode::Snapshot,
        ),
        (
            "branch_bad_mem",
            false,
            two_model_path.clone(),
            league_relay::MemoryMode::Full,
        ),
    ] {
        if tamper_wrapper_sha {
            let mut v: Value = must_ok(serde_json::from_slice(&orig_setup_bytes), "parse setup");
            v["wrapper_sha256"] = json!("0".repeat(64));
            must_ok(
                fs::write(&setup_file_path, v.to_string()),
                "tamper setup.json",
            );
        }
        let e = run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(game_dir.join("checkpoints/day-02-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: bad_seats,
                wrapper_path: wrapper_path.clone(),
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arms: vec![EvalArm::Off],
                out_dir: tmp.join(format!("out_{label}")),
                memory_mode: bad_mem,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &NoopTransport,
            None,
        );
        must_ok(
            fs::write(&setup_file_path, &orig_setup_bytes),
            "restore setup.json",
        );
        assert!(
            matches!(e, Err(EvalError::IncompatibleEvalSetup(_))),
            "{label}: expected INCOMPATIBLE_EVAL_SETUP, got {e:?}"
        );
    }

    // 4. B4 + C2: branch Day 2 (`Foundry` received `"WHISPER_TOKEN_B4_D2"` from `Quarry`) with TWO DISTINCT
    //    valid orders across `off` (`amount: 10`) and `on` (`amount: 45`) (different from recorded `0`!):
    //    - Verify whisper token `"WHISPER_TOKEN_B4_D2"` appears in both focal HTTP request bodies (`B4` + Answer 1).
    //    - Verify `loaded_history_sha256` records exact SHA-256 of negotiation, transcript, and scratchpad (`B4`).
    //    - Verify `off`, `on`, and recorded play receipts all differ (`C2`).
    //    - Verify re-running `branch` into the SAME non-empty `--out` is refused with `BRANCH_ARM_DIR_NOT_EMPTY` (`B4`).
    let branch_out = tmp.join("b4_c2_branch");
    struct DistinctOrderBranchTransport {
        calls: Mutex<Vec<String>>,
    }
    impl Transport for DistinctOrderBranchTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let call_idx = {
                let mut g = match self.calls.lock() {
                    Ok(g) => g,
                    Err(e) => panic!("lock: {e}"),
                };
                g.push(body.to_owned());
                g.len()
            };
            let amount = if call_idx == 1 { 10 } else { 45 };
            let content = json!({
                "messages": [],
                "market_actions": [{
                    "action_kind": "town_import",
                    "good": { "asset_kind": "raw", "good_index": 0 },
                    "amount": amount
                }],
                "bundle": {},
                "notes": format!("branch arm call {call_idx}")
            })
            .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_response_value(
                    &format!("b-{call_idx}"),
                    "Qwen/Qwen3-32B",
                    &content,
                    20,
                    10,
                    None,
                )
                .to_string(),
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

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("TEMPLATED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            assert!(add_special && parse_special);
            Ok((100_u64..120).collect())
        }
    }

    let c2_transport = DistinctOrderBranchTransport {
        calls: Mutex::new(Vec::new()),
    };
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(game_dir.join("checkpoints/day-02-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: two_model_path.clone(),
                wrapper_path: wrapper_path.clone(),
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: branch_out.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &c2_transport,
            None,
        ),
        "b4+c2 distinct branch",
    );
    assert_eq!(outcomes.len(), 2);

    let recorded_d2_receipt: PublicSweepReceipt = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("receipts/day-02.receipt.json")),
            "read recorded d2 receipt",
        )),
        "parse d2 receipt",
    );
    assert_ne!(outcomes[0].receipt, recorded_d2_receipt);
    assert_ne!(outcomes[1].receipt, recorded_d2_receipt);
    assert_ne!(outcomes[0].receipt, outcomes[1].receipt);

    // Check focal HTTP request bodies carry Quarry's whisper.
    let branch_reqs = match c2_transport.calls.lock() {
        Ok(g) => g.clone(),
        Err(e) => panic!("lock: {e}"),
    };
    assert_eq!(branch_reqs.len(), 2);
    for req in &branch_reqs {
        assert!(
            req.contains("WHISPER_TOKEN_B4_D2"),
            "focal branch HTTP request must deliver Peer's day-2 talk whisper"
        );
    }

    // Check `loaded_history_sha256` exact digests in decision.json (`B4`).
    let expected_neg_sha = sha256_hex(&must_ok(
        fs::read(game_dir.join("to-seat/day-02-act-Foundry.negotiation.json")),
        "read neg",
    ));
    let expected_tx_sha = sha256_hex(&must_ok(
        fs::read(game_dir.join("relay/seats/Foundry.jsonl")),
        "read tx",
    ));
    let expected_pad_sha = sha256_hex(&must_ok(
        fs::read(game_dir.join("checkpoints/day-02-pre-act.Foundry.scratchpad.md")),
        "read pad",
    ));
    for o in &outcomes {
        assert_eq!(
            o.decision
                .loaded_history_sha256
                .get("to-seat/day-02-act-Foundry.negotiation.json")
                .map(String::as_str),
            Some(expected_neg_sha.as_str())
        );
        assert_eq!(
            o.decision
                .loaded_history_sha256
                .get("relay/seats/Foundry.jsonl")
                .map(String::as_str),
            Some(expected_tx_sha.as_str())
        );
        assert_eq!(
            o.decision
                .loaded_history_sha256
                .get("checkpoints/day-02-pre-act.Foundry.scratchpad.md")
                .map(String::as_str),
            Some(expected_pad_sha.as_str())
        );
    }

    // Check Summary JSON (`branch_summary.json`) exists with delivered = 2, total_rounds = 2.
    let summary: BranchSummary = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(branch_out.join("branch_summary.json")),
            "read branch_summary",
        )),
        "parse branch_summary",
    );
    assert_eq!(summary.evaluated_arms, 2);
    assert_eq!(summary.wrong_part_refusals.delivered_model_replies, 2);
    assert_eq!(summary.wrong_part_refusals.total_model_rounds, 2);

    // B4: Re-running `branch` into non-empty `branch_out` must fail loudly with BRANCH_ARM_DIR_NOT_EMPTY.
    let reuse_err = run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(game_dir.join("checkpoints/day-02-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: two_model_path,
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arms: vec![EvalArm::Off, EvalArm::On],
            out_dir: branch_out,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    );
    assert!(matches!(reuse_err, Err(EvalError::BranchArmDirNotEmpty(_))));

    // 5. C7: two fresh games with identical `(seed, table_id)` draw independent non-zero OS `bot_tie_seed`s!
    let c7_a = tmp.join("c7_a");
    let c7_b = tmp.join("c7_b");
    let all_bot = write_seats(&tmp, "c7_bot_seats.json", all_clubbot_seats());
    for g in [&c7_a, &c7_b] {
        let _ = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: g.clone(),
                    seats_path: all_bot.clone(),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    serving_manifest_path: None,
                    arm: EvalArm::Off,
                    table_id: 777,
                    seed: 42,
                    turns: 1,
                    memory_mode: league_relay::MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &NoopTransport,
                None,
            ),
            "c7 fresh game",
        );
    }
    let seal_a: Value = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(c7_a.join("eval-777.seal")),
            "read c7 seal a",
        )),
        "parse c7 seal a",
    );
    let seal_b: Value = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(c7_b.join("eval-777.seal")),
            "read c7 seal b",
        )),
        "parse c7 seal b",
    );
    let tie_a = seal_a["bot_tie_seed"].as_u64();
    let tie_b = seal_b["bot_tie_seed"].as_u64();
    assert!(tie_a.is_some_and(|v| v != 0));
    assert!(tie_b.is_some_and(|v| v != 0));
    assert_ne!(
        tie_a, tie_b,
        "C7: fresh games must draw distinct OS CSPRNG bot_tie_seed"
    );
    let _unused_seats = seats_path;

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn reserved_and_cap_reached_attempt_filter_and_orphan_reconcile() {
    let tmp = fresh_tmp_dir("cap_and_orphan");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());

    // Part (a): pre-call CAP_REACHED must log to `failed_attempts` with outcome `"CAP_REACHED"`
    // and NOT increment `attempts` (`attempts == 0`), leaving `no_reply: true`.
    let cap_game = tmp.join("cap_game");
    let cap_transport = DeterministicMockTransport::new(Vec::new());
    let cap_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: cap_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 91,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: Some(1.0), // max_gpu_secs_per_call is 10.0 -> CAP_REACHED pre-call
                allowed_vm_hosts: Vec::new(),
            },
            &cap_transport,
            None,
        ),
        "capped play run",
    );
    assert_eq!(cap_manifest.played_turns, 1);
    assert!(
        cap_transport.recorded_bodies().is_empty(),
        "pre-call CAP_REACHED must make zero HTTP requests"
    );
    let cap_calls = must_ok(
        fs::read_to_string(cap_game.join("relay/calls.jsonl")),
        "read cap calls",
    );
    assert!(cap_calls.contains("\"outcome\":\"CAP_REACHED\""));
    for round_name in ["talk", "act"] {
        let dec: EvalDecisionRecord = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(
                    cap_game.join(format!("decisions/day-01-{round_name}-Foundry.json")),
                ),
                "read cap decision",
            )),
            "parse cap decision",
        );
        assert!(
            dec.no_reply,
            "{round_name} should record no_reply on CAP_REACHED"
        );
        assert_eq!(
            dec.attempts, 0,
            "CAP_REACHED refusal must not increment attempts in {round_name}"
        );
        assert_eq!(dec.failed_attempts.len(), 1);
        assert_eq!(dec.failed_attempts[0].outcome, "CAP_REACHED");
    }

    // Part (b1): orphaned `calls.reserved.jsonl` from an interrupted prior run reconciles into
    // `calls.jsonl` as a `"RESERVED"` row and appears in `failed_attempts` without counting in
    // `attempts` (`attempts == 1`) or blocking the resumed `"OK"` call.
    fn write_orphan_reservation(game_dir: &Path, day_str: &str, round: Round) {
        let reserved_path = game_dir.join("relay/calls.reserved.jsonl");
        let entry = CallLogEntry {
            time_utc: "2026-11-21T00:00:00.000Z".to_owned(),
            day: day_str.to_owned(),
            round,
            guild: "Foundry".to_owned(),
            attempt: 1,
            trigger: "turn".to_owned(),
            model_requested: "Qwen/Qwen3-32B".to_owned(),
            model_returned: None,
            request_body_sha256: "00".repeat(32),
            http_reply_sha256: None,
            reply_sha256: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            reasoning_tokens: None,
            cost_unit: CostUnit::GpuSeconds,
            wall_secs: None,
            cost: None,
            charged_credits: 10.0,
            latency_ms: 0,
            outcome: "RESERVED".to_owned(),
            error: Some("orphaned pre-call reservation from interrupted run".to_owned()),
            reservation_id: None,
        };
        let line = json!({
            "reservation_id": "resv-orphan-1",
            "run_id": "dead-run-1",
            "entry": entry,
        })
        .to_string();
        must_ok(
            fs::write(reserved_path, format!("{line}\n")),
            "write calls.reserved.jsonl",
        );
    }

    let ok_json = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {},
        "notes": "clean"
    })
    .to_string();
    let orphan_ok_game = tmp.join("orphan_ok_game");
    let t1 = DeterministicMockTransport::new(vec![
        ("1:talk", ok_json.clone()),
        ("1:act", ok_json.clone()),
        ("2:talk", ok_json.clone()),
        ("2:act", ok_json.clone()),
    ]);
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: orphan_ok_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 92,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &t1,
            None,
        ),
        "turn 1 before orphan",
    );
    write_orphan_reservation(&orphan_ok_game, "02", Round::Talk);
    let m2 = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: orphan_ok_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 92,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &t1,
            None,
        ),
        "resume turn 2 with orphan reservation",
    );
    assert_eq!(m2.played_turns, 2);
    assert!(
        !orphan_ok_game.join("relay/calls.reserved.jsonl").exists(),
        "calls.reserved.jsonl must be removed after reconciliation"
    );
    let orphan_calls = must_ok(
        fs::read_to_string(orphan_ok_game.join("relay/calls.jsonl")),
        "read orphan calls",
    );
    assert!(orphan_calls.contains("\"outcome\":\"RESERVED\""));
    assert!(orphan_calls.contains("orphaned pre-call reservation from interrupted run"));
    let d2_talk_ok: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(orphan_ok_game.join("decisions/day-02-talk-Foundry.json")),
            "read d2 talk ok",
        )),
        "parse d2 talk ok",
    );
    assert!(!d2_talk_ok.no_reply);
    assert_eq!(
        d2_talk_ok.attempts, 1,
        "reconciled RESERVED row must not increment attempts"
    );
    assert_eq!(d2_talk_ok.failed_attempts.len(), 1);
    assert_eq!(d2_talk_ok.failed_attempts[0].outcome, "RESERVED");
    assert_eq!(
        d2_talk_ok.failed_attempts[0].reason,
        "orphaned pre-call reservation from interrupted run"
    );

    // Part (b2): when the resumed round also fails twice, the reconciled `"RESERVED"` row must not
    // poison `first_failure` (attempt 1's wrong-round refusal still sets `TALK_REPLY_HAS_ORDERS`).
    struct WrongRoundThen500Transport {
        turn2_talk_calls: AtomicUsize,
    }
    impl Transport for WrongRoundThen500Transport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let v: Value =
                serde_json::from_str(body).map_err(|e| TransportError::Network(e.to_string()))?;
            let user_msg = v["messages"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default();
            if user_msg.contains("\"turn\": 2") && user_msg.contains("This is the talk round.") {
                let n = self.turn2_talk_calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    let wrong_round = json!({
                        "messages": [],
                        "market_actions": [{ "action_kind": "town_import", "good": { "asset_kind": "raw", "good_index": 0 }, "amount": 5 }],
                        "bundle": {},
                    })
                    .to_string();
                    return Ok(RawHttpResponse {
                        status: 200,
                        body: mock_chat_response_value(
                            "wrong-round-attempt-1",
                            "Qwen/Qwen3-32B",
                            &wrong_round,
                            10,
                            10,
                            None,
                        )
                        .to_string(),
                    });
                }
                return Ok(RawHttpResponse {
                    status: 500,
                    body: "{\"error\":\"down\"}".to_owned(),
                });
            }
            let ok = json!({
                "messages": [],
                "market_actions": [],
                "bundle": {},
                "notes": "ok"
            })
            .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_response_value("ok-fallback", "Qwen/Qwen3-32B", &ok, 10, 10, None)
                    .to_string(),
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

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("TEMPLATED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            assert!(add_special && parse_special);
            Ok((100_u64..110).collect())
        }
    }

    let orphan_fail_game = tmp.join("orphan_fail_game");
    let t2 = WrongRoundThen500Transport {
        turn2_talk_calls: AtomicUsize::new(0),
    };
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: orphan_fail_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 93,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &t2,
            None,
        ),
        "turn 1 before orphan fail",
    );
    write_orphan_reservation(&orphan_fail_game, "02", Round::Talk);
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: orphan_fail_game.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 93,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &t2,
            None,
        ),
        "resume turn 2 orphan fail",
    );
    let d2_talk_fail: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(orphan_fail_game.join("decisions/day-02-talk-Foundry.json")),
            "read d2 talk fail",
        )),
        "parse d2 talk fail",
    );
    assert!(d2_talk_fail.no_reply);
    assert_eq!(
        d2_talk_fail.attempts, 1,
        "only the 1 actual attempt counts in Eval mode, not the reconciled RESERVED row"
    );
    assert_eq!(
        d2_talk_fail.refusal_code.as_deref(),
        Some("TALK_REPLY_HAS_ORDERS"),
        "reconciled RESERVED row must not mask attempt 1 wrong-round classification"
    );
    assert!(
        d2_talk_fail
            .failed_attempts
            .iter()
            .any(|a| a.outcome == "RESERVED"
                && a.reason == "orphaned pre-call reservation from interrupted run")
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn branch_refuses_missing_history_uses_relative_pad_key_and_protects_summary() {
    fn copy_tree(src: &Path, dst: &Path) {
        must_ok(fs::create_dir_all(dst), "create copy dst");
        for entry in must_ok(fs::read_dir(src), "read copy src").flatten() {
            let src_p = entry.path();
            let dst_p = dst.join(entry.file_name());
            if src_p.is_dir() {
                copy_tree(&src_p, &dst_p);
            } else {
                must_ok(fs::copy(&src_p, &dst_p), "copy tree file");
            }
        }
    }

    let tmp = fresh_tmp_dir("branch_history_guards");
    let wrapper_path = write_wrapper(&tmp);
    let seats_path = write_seats(&tmp, "seats.json", focal_local_three_clubbot_seats());
    let src_game = tmp.join("src_game");

    let talk_ok = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {},
        "notes": "keep_pre_act_note"
    })
    .to_string();
    let act_ok = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {},
        "notes": "act_note"
    })
    .to_string();
    let play_t =
        DeterministicMockTransport::new(vec![("1:talk", talk_ok), ("1:act", act_ok.clone())]);
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: src_game.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arm: EvalArm::Off,
                table_id: 94,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_t,
            None,
        ),
        "play 1 turn for branch history checks",
    );

    // 1. Clean branch records relative scratchpad key `"checkpoints/day-01-pre-act.Foundry.scratchpad.md"`,
    //    never a local filesystem path.
    let out_clean = tmp.join("out_clean");
    let branch_t = DeterministicMockTransport::new(vec![("1:act", act_ok.clone())]);
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(src_game.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: out_clean.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &branch_t,
            None,
        ),
        "clean branch",
    );
    assert_eq!(outcomes.len(), 2);
    let pad_rel_key = "checkpoints/day-01-pre-act.Foundry.scratchpad.md";
    let pad_bytes = must_ok(
        fs::read(src_game.join(pad_rel_key)),
        "read source scratchpad",
    );
    let expected_pad_sha = sha256_hex(&pad_bytes);
    for outcome in &outcomes {
        assert_eq!(
            outcome.decision.loaded_history_sha256.get(pad_rel_key),
            Some(&expected_pad_sha),
            "loaded_history_sha256 must key pre-act scratchpad by relative path `{pad_rel_key}`"
        );
        for k in outcome.decision.loaded_history_sha256.keys() {
            assert!(
                !k.contains(':') && !k.contains('\\') && !k.starts_with('/'),
                "loaded_history_sha256 key must be relative, never local path: `{k}`"
            );
        }
    }

    // 1b. Branching from a checkpoint folder not named `checkpoints` (e.g. `ckpts`) must still
    //     record the relative key `"checkpoints/day-01-pre-act.Foundry.scratchpad.md"` in
    //     `decision.json`, with zero absolute local paths anywhere in `decision.json`.
    let src_ckpts = tmp.join("src_ckpts");
    copy_tree(&src_game, &src_ckpts);
    must_ok(
        fs::rename(src_ckpts.join("checkpoints"), src_ckpts.join("ckpts")),
        "rename checkpoints to ckpts",
    );
    let out_ckpts = tmp.join("out_ckpts");
    let ckpts_t = DeterministicMockTransport::new(vec![("1:act", act_ok)]);
    let ckpts_outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(src_ckpts.join("ckpts/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: out_ckpts.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &ckpts_t,
            None,
        ),
        "branch from renamed ckpts directory",
    );
    assert_eq!(ckpts_outcomes.len(), 2);
    let src_ckpts_str = src_ckpts.to_string_lossy();
    let src_ckpts_escaped = src_ckpts_str.replace('\\', "\\\\");
    for arm_name in ["off", "on"] {
        let dec_raw = must_ok(
            fs::read_to_string(out_ckpts.join(arm_name).join("decision.json")),
            "read ckpts branch decision.json",
        );
        let dec: EvalDecisionRecord =
            must_ok(serde_json::from_str(&dec_raw), "parse ckpts decision.json");
        assert_eq!(
            dec.loaded_history_sha256.get(pad_rel_key),
            Some(&expected_pad_sha),
            "decision.json for renamed checkpoint dir must still key pre-act scratchpad by `{pad_rel_key}`"
        );
        for k in dec.loaded_history_sha256.keys() {
            assert!(
                !Path::new(k).is_absolute()
                    && !k.contains(':')
                    && !k.contains('\\')
                    && !k.starts_with('/'),
                "loaded_history_sha256 key in decision.json must be relative, never local path: `{k}`"
            );
        }
        assert!(
            !dec_raw.contains(src_ckpts_str.as_ref()) && !dec_raw.contains(&src_ckpts_escaped),
            "absolute local path `{src_ckpts_str}` must never appear in decision.json"
        );
    }

    // 2. Existing `branch_summary.json` must be refused with `BRANCH_SUMMARY_EXISTS` and left unchanged.
    let out_existing_summary = tmp.join("out_existing_summary");
    must_ok(
        fs::create_dir_all(&out_existing_summary),
        "create existing out dir",
    );
    let summary_bytes = must_ok(
        fs::read(out_clean.join("branch_summary.json")),
        "read clean branch_summary.json",
    );
    let existing_summary_path = out_existing_summary.join("branch_summary.json");
    must_ok(
        fs::write(&existing_summary_path, &summary_bytes),
        "seed branch_summary.json",
    );
    let summary_err = match run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(src_game.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arms: vec![EvalArm::Off, EvalArm::On],
            out_dir: out_existing_summary,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    ) {
        Ok(_) => panic!("expected BRANCH_SUMMARY_EXISTS"),
        Err(e) => e,
    };
    assert_eq!(summary_err.code(), "BRANCH_SUMMARY_EXISTS");
    assert_eq!(
        must_ok(fs::read(&existing_summary_path), "re-read existing summary"),
        summary_bytes,
        "existing branch_summary.json must remain byte-for-byte unchanged on refusal"
    );

    // 3. Missing pre-act scratchpad on a note-keeping model seat must fail with `MISSING_BRANCH_SCRATCHPAD`.
    let src_missing_pad = tmp.join("src_missing_pad");
    copy_tree(&src_game, &src_missing_pad);
    must_ok(
        fs::remove_file(src_missing_pad.join(pad_rel_key)),
        "remove pre-act scratchpad",
    );
    let pad_err = match run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(src_missing_pad.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("out_missing_pad"),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    ) {
        Ok(_) => panic!("expected MISSING_BRANCH_SCRATCHPAD"),
        Err(e) => e,
    };
    assert_eq!(pad_err.code(), "MISSING_BRANCH_SCRATCHPAD");

    // 4. Missing pre-act transcript when a prior model round delivered must fail with `MISSING_BRANCH_TRANSCRIPT`.
    let src_missing_tx = tmp.join("src_missing_tx");
    copy_tree(&src_game, &src_missing_tx);
    must_ok(
        fs::remove_file(src_missing_tx.join("relay/seats/Foundry.jsonl")),
        "remove seat transcript",
    );
    let tx_err = match run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(src_missing_tx.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(write_valid_serving_manifest(&tmp)),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("out_missing_tx"),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
    ) {
        Ok(_) => panic!("expected MISSING_BRANCH_TRANSCRIPT"),
        Err(e) => e,
    };
    assert_eq!(tx_err.code(), "MISSING_BRANCH_TRANSCRIPT");

    let _ = fs::remove_dir_all(&tmp);
}
