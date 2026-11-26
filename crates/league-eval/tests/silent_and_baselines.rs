//! Integration tests for `--communication silent` (D7) and scripted `"file"` baselines (D8).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use league_core::{
    AssetId, EvalActSubmission, EvalHost, HARD_TURN_CAP, MarketAction, OfferProposal, OfferSide,
    PublicSweepReceipt, Seat, TableId, TurnPhase, replay_eval_season,
};
use league_eval::{
    ArmMapStateFile, BranchArmOutcome, BranchOptions, BuildBlindInputsOptions, CRATE_GIT_SHA,
    CommunicationMode, CrateGitShaSource, EvalArm, EvalDecisionRecord, EvalManifest, EvalSetupFile,
    FreezeOptions, JoinOptions, JoinedDecisionsFile, NoopTransport, OwnToSeatHistory, PeerStance,
    PerArmSummaryFile, PlayOptions, ScorerFreezeFile, VerifyCheckpointsOptions,
    compute_tit_for_tat_stances, decide_always_defect_act, decide_scripted_baseline_talk,
    decide_tit_for_tat_act, execute_cli_with_git_sha, parse_eval_act_reply_for_communication,
    parse_eval_silent_act_reply, persist_eval_game_state,
    run_branch_with_communication_and_git_sha, run_build_blind_inputs, run_freeze_with_provenance,
    run_join, run_play_with_communication_and_git_sha, run_play_with_transport_and_git_sha,
    run_verify_checkpoints,
};
use league_relay::{
    CONTROL_WORDING_ARM, MemoryMode, NegotiationInputs, RawHttpResponse, Transport, TransportError,
    UntrustedWhisper, sha256_hex,
};
use league_scorer::{BlindScoringRequest, ScorerConfig, execute_blind_request};
use league_seat_types::FinalScoreboard;
use league_seat_types::UNIT_SCALE;
use serde_json::{Value, json};

const FALLBACK_GIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const D902D04_GIT_SHA: &str = "d902d04a399530a3743e6223c7a653dd48f8e8c1";

fn must_ok<T, E: Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(e) => panic!("{ctx}: {e}"),
    }
}

fn fresh_tmp_dir(label: &str) -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ml_sb_{label}_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    must_ok(fs::create_dir_all(&dir), "create temp dir");
    dir
}

fn injected_git_sha(sha: &'static str) -> CrateGitShaSource<'static> {
    CrateGitShaSource::Injected {
        built_sha: sha,
        live_head: Some(sha),
        is_dirty: Some(false),
    }
}

fn test_git_sha() -> CrateGitShaSource<'static> {
    let sha = if CRATE_GIT_SHA.len() == 40 && CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()) {
        CRATE_GIT_SHA
    } else {
        FALLBACK_GIT_SHA
    };
    injected_git_sha(sha)
}

fn write_wrapper_for(dir: &Path, communication: CommunicationMode) -> PathBuf {
    let rel = match communication {
        CommunicationMode::Talk => "wrappers/v2_seat_brief_2026_11_21_r3.md",
        CommunicationMode::Silent => "wrappers/v2_seat_brief_silent_2026_11_26_r1.md",
    };
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let bytes = must_ok(fs::read(&src), "read committed wrapper");
    let dst = dir.join(format!("wrapper-{}.md", communication.as_str()));
    must_ok(fs::write(&dst, bytes), "write wrapper");
    dst
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
        fs::write(&p, must_ok(serde_json::to_string_pretty(&manifest), "ser")),
        "write serving manifest",
    );
    p
}

fn write_seats_json(dir: &Path, name: &str, value: Value) -> PathBuf {
    let p = dir.join(name);
    let txt = must_ok(serde_json::to_string_pretty(&value), "ser seats");
    must_ok(fs::write(&p, txt), "write seats");
    p
}

fn clubbot_seats_json() -> Value {
    json!({
        "seats": [
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    })
}

fn local_plus_clubbot_seats_json(notes: Option<bool>) -> Value {
    let mut s0 = json!({
        "guild": "Foundry",
        "backend": "local",
        "base_url": "http://127.0.0.1:18080/v1",
        "model": "Qwen/Qwen3-32B",
        "temperature": 0.0,
        "max_output_tokens": 512,
        "timeout_secs": 5.0,
        "max_gpu_secs_per_call": 10.0
    });
    if let Some(flag) = notes {
        s0["scratchpad"] = json!(flag);
    }
    json!({
        "seats": [
            s0,
            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    })
}

fn chat_resp(content: &str) -> Value {
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
    json!({
        "id": "mock-sb-1",
        "model": "Qwen/Qwen3-32B",
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
        "usage": {
            "prompt_tokens": 40,
            "completion_tokens": 15,
            "total_tokens": 55
        }
    })
}

fn tool_call_resp(tool_name: &str, args_json: &str) -> Value {
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
    json!({
        "id": "mock-sb-tool",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": args_json
                    }
                }]
            },
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "logprob": -0.01,
                    "top_logprobs": top_logprobs
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 40,
            "completion_tokens": 12,
            "total_tokens": 52
        }
    })
}

struct SilentAwareMockTransport {
    requests: Mutex<Vec<String>>,
    /// If `Some(tool_name)`, issues one tool call on first round call, then returns final JSON.
    first_tool: Option<&'static str>,
    act_reply: String,
}

impl SilentAwareMockTransport {
    fn new(first_tool: Option<&'static str>, act_reply: String) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            first_tool,
            act_reply,
        }
    }
}

impl Transport for SilentAwareMockTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        {
            let mut reqs = self
                .requests
                .lock()
                .map_err(|_| TransportError::Network("lock poisoned".to_owned()))?;
            reqs.push(body.to_owned());
        }
        let v: Value =
            serde_json::from_str(body).map_err(|e| TransportError::Network(e.to_string()))?;
        let has_tool_result = v["messages"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|m| m["role"] == "tool"));
        if !has_tool_result
            && let Some(tool_name) = self.first_tool
            && v.get("tools").is_some()
        {
            let args = json!({
                "market_actions": [],
                "bundle": {}
            })
            .to_string();
            return Ok(RawHttpResponse {
                status: 200,
                body: tool_call_resp(tool_name, &args).to_string(),
            });
        }
        Ok(RawHttpResponse {
            status: 200,
            body: chat_resp(&self.act_reply).to_string(),
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
        _add_special: bool,
        _parse_special: bool,
        _timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        Ok((100_u64..140).collect())
    }
}

fn walk_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, cur: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(rd) = fs::read_dir(cur) else {
            return;
        };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|x| x.path())).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                visit(root, &p, out);
            } else if p.is_file() {
                let rel = p
                    .strip_prefix(root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if let Ok(bytes) = fs::read(&p) {
                    out.insert(rel, bytes);
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(dir, dir, &mut out);
    out
}

fn default_play_opts(
    game_dir: PathBuf,
    seats_path: PathBuf,
    wrapper_path: PathBuf,
    serving_manifest_path: Option<PathBuf>,
    arm: EvalArm,
    turns: u8,
) -> PlayOptions {
    PlayOptions {
        game_dir,
        seats_path,
        wrapper_path,
        wrapper_version: None,
        serving_manifest_path,
        arm,
        table_id: 1,
        seed: 4242,
        turns,
        memory_mode: MemoryMode::Snapshot,
        live: false,
        cap_credits: None,
        cap_gpu_secs: None,
        allowed_vm_hosts: Vec::new(),
    }
}

// ============================================================================
// 1. Silent-market condition: zero communication artefacts & act prohibitions
// ============================================================================

#[test]
fn silent_market_omits_talk_and_refuses_communication_actions()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = fresh_tmp_dir("silent_arms");
    let wrapper = write_wrapper_for(&tmp, CommunicationMode::Silent);
    let seats_path = write_seats_json(
        &tmp,
        "seats.json",
        local_plus_clubbot_seats_json(Some(false)),
    );
    let serving = write_valid_serving_manifest(&tmp);

    for (arm, tool_name) in [
        (EvalArm::Off, None),
        (EvalArm::Control, Some("draft_turn_orders")),
        (EvalArm::On, Some("preview_turn_orders")),
    ] {
        for memory_mode in [MemoryMode::Snapshot, MemoryMode::Emc, MemoryMode::Full] {
            if arm != EvalArm::Control && memory_mode != MemoryMode::Snapshot {
                continue;
            }
            let game_dir = tmp.join(format!("game_{}_{:?}", arm.as_str(), memory_mode));
            let mut opts = default_play_opts(
                game_dir.clone(),
                seats_path.clone(),
                wrapper.clone(),
                Some(serving.clone()),
                arm,
                2,
            );
            opts.memory_mode = memory_mode;

            let reply = json!({
                "market_actions": [],
                "bundle": {}
            })
            .to_string();
            let transport = SilentAwareMockTransport::new(tool_name, reply);
            let manifest = run_play_with_communication_and_git_sha(
                &opts,
                CommunicationMode::Silent,
                &transport,
                None,
                &test_git_sha(),
            )?;
            assert_eq!(manifest.communication, CommunicationMode::Silent);
            assert_eq!(manifest.played_turns, 2);

            let files = walk_files(&game_dir);
            for (rel, bytes) in &files {
                assert!(
                    !rel.contains("talk"),
                    "silent run `{rel}` unexpectedly created talk artefact"
                );
                assert!(
                    !rel.ends_with(".negotiation.json"),
                    "silent run `{rel}` unexpectedly wrote negotiation.json"
                );
                if !rel.ends_with(".seal") {
                    let text = String::from_utf8_lossy(bytes);
                    assert!(
                        !text.contains("eval_batna"),
                        "silent file `{rel}` leaked eval_batna"
                    );
                    assert!(
                        !text.contains("Negotiation & BATNA"),
                        "silent file `{rel}` leaked Negotiation & BATNA header"
                    );
                }
            }

            // Check relay transcript has zero Round::Talk entries and act has no messages key.
            let tx_path = game_dir.join("relay/seats/Foundry.jsonl");
            if tx_path.is_file() {
                let raw_tx = fs::read_to_string(&tx_path)?;
                for line in raw_tx.lines().filter(|l| !l.trim().is_empty()) {
                    assert!(
                        !line.contains("\"round\":\"talk\""),
                        "silent transcript contains Round::Talk: {line}"
                    );
                }
            }
            let act_dec: EvalDecisionRecord = serde_json::from_str(&fs::read_to_string(
                game_dir.join("decisions/day-01-act-Foundry.json"),
            )?)?;
            assert_eq!(act_dec.communication, CommunicationMode::Silent);
            assert!(
                !act_dec.no_reply,
                "unexpected refusal: {:?}",
                act_dec.refusal_reason
            );
            if tool_name.is_some() {
                assert_eq!(
                    act_dec.tool_calls_used, 1,
                    "preview/draft must remain enabled in silent"
                );
            }
        }
    }

    // Refusals in Silent Round::Act: non-empty messages -> ACT_REPLY_HAS_MESSAGES, empty messages -> ACT_REPLY_INVALID,
    // record_intent_pledge, condition_hash, and preimage -> ACT_REPLY_INVALID.
    let parsed_nonempty_msgs = parse_eval_silent_act_reply(
        r#"{"messages":[{"to_guild":"Quarry","body":"hi"}],"market_actions":[],"bundle":{}}"#,
        0,
        false,
    );
    assert_eq!(
        parsed_nonempty_msgs.refusal_code.map(|c| c.as_str()),
        Some("ACT_REPLY_HAS_MESSAGES")
    );
    let parsed_empty_msgs = parse_eval_silent_act_reply(
        r#"{"messages":[],"market_actions":[],"bundle":{}}"#,
        0,
        false,
    );
    assert_eq!(
        parsed_empty_msgs.refusal_code.map(|c| c.as_str()),
        Some("ACT_REPLY_INVALID")
    );

    let parsed_pledge = parse_eval_silent_act_reply(
        &json!({
            "market_actions": [{
                "action_kind": "record_intent_pledge",
                "counterparty_seat": 1,
                "memo_digest": "11".repeat(32)
            }],
            "bundle": {}
        })
        .to_string(),
        0,
        false,
    );
    assert_eq!(
        parsed_pledge.refusal_code.map(|c| c.as_str()),
        Some("ACT_REPLY_INVALID")
    );

    let parsed_escrow_cond = parse_eval_silent_act_reply(
        &json!({
            "market_actions": [{
                "action_kind": "create_escrow",
                "destination_seat": 1,
                "good": { "asset_kind": "cred" },
                "amount": 1_000,
                "finish_after_turn": 2,
                "cancel_after_turn": 5,
                "condition_hash": "22".repeat(32)
            }],
            "bundle": {}
        })
        .to_string(),
        0,
        false,
    );
    assert_eq!(
        parsed_escrow_cond.refusal_code.map(|c| c.as_str()),
        Some("ACT_REPLY_INVALID")
    );

    let parsed_preimage = parse_eval_silent_act_reply(
        &json!({
            "market_actions": [],
            "bundle": {
                "escrow_finishes": [{
                    "escrow_id": 1,
                    "preimage": "33".repeat(32)
                }]
            }
        })
        .to_string(),
        0,
        false,
    );
    assert_eq!(
        parsed_preimage.refusal_code.map(|c| c.as_str()),
        Some("ACT_REPLY_INVALID")
    );
    let parsed_notes = parse_eval_silent_act_reply(
        r#"{"market_actions":[],"bundle":{},"notes":"kept note"}"#,
        0,
        true,
    );
    assert!(parsed_notes.refusal_code.is_none() && parsed_notes.notes_present);

    // CommunicationMode parse/as_str + Talk view refusal under Silent.
    assert_eq!(
        CommunicationMode::parse("  SILENT ")?,
        CommunicationMode::Silent
    );
    assert_eq!(CommunicationMode::Talk.as_str(), "talk");
    assert_eq!(CommunicationMode::Silent.as_str(), "silent");
    assert!(CommunicationMode::parse("invalid-mode").is_err());
    let host_v = EvalHost::from_standard_setup_with_model_seats(
        TableId(77),
        4,
        1,
        2,
        &[true, true, true, true],
    )?;
    let obs_v = host_v.observe_seat(Seat(0))?;
    let Err(talk_in_silent) = league_eval::render_seat_view_markdown_for_communication(
        league_relay::Round::Talk,
        CommunicationMode::Silent,
        &obs_v,
    ) else {
        panic!("Round::Talk view must refuse under Silent");
    };
    assert_eq!(talk_in_silent.code(), "SCHEMA_VALIDATION_FAILED");

    Ok(())
}

// ============================================================================
// 2. Play-mode refusal via league-relay CLI entry
// ============================================================================

#[test]
fn play_mode_refuses_communication_silent_with_exit_1() -> Result<(), Box<dyn std::error::Error>> {
    let seats = league_relay::SeatFile::from_json_str(
        r#"{"seats":[{"guild":"Foundry","backend":"file","model":"ClubBot"}]}"#,
        None,
    )?;
    let cfg = league_relay::TurnConfig::new(
        fresh_tmp_dir("relay_play_refusal"),
        "01".to_owned(),
        league_relay::Round::Act,
        seats,
        None,
        false,
        None,
    )
    .with_silent_market(true);
    let Err(err) = league_relay::run_turn(
        &cfg,
        &SilentAwareMockTransport::new(None, String::new()),
        None,
    ) else {
        panic!("silent_market without eval_mode must refuse");
    };
    assert_eq!(err.code(), "SILENT_MARKET_EVAL_ONLY");
    assert_eq!(err.exit_code(), league_relay::EXIT_USER_ERROR);
    Ok(())
}

// ============================================================================
// 3. Full-season Talk byte-identity vs d902d04 golden fixture
// ============================================================================

const ML_D902D04_PARITY_GAME_DIR_ENV: &str = "ML_D902D04_PARITY_GAME_DIR";
const ML_D902D04_PARITY_TMP_DIR_ENV: &str = "ML_D902D04_PARITY_TMP_DIR";

#[test]
fn d902d04_talk_parity_child_helper() {
    let Ok(game_dir_raw) = std::env::var(ML_D902D04_PARITY_GAME_DIR_ENV) else {
        return;
    };
    let tmp_dir_raw = must_ok(
        std::env::var(ML_D902D04_PARITY_TMP_DIR_ENV),
        "read d902d04 parity tmp dir env",
    );
    let game_dir = PathBuf::from(game_dir_raw);
    let tmp = PathBuf::from(tmp_dir_raw);
    let wrapper = write_wrapper_for(&tmp, CommunicationMode::Talk);
    let seats_path = tmp.join("seats.json");
    let mut opts = default_play_opts(
        game_dir,
        seats_path,
        wrapper,
        None,
        EvalArm::Off,
        HARD_TURN_CAP,
    );
    opts.table_id = 77;

    let _manifest = must_ok(
        run_play_with_transport_and_git_sha(
            &opts,
            &NoopTransport,
            None,
            &injected_git_sha(D902D04_GIT_SHA),
        ),
        "run hermetic d902d04 parity child",
    );
}

#[test]
fn talk_full_season_byte_identical_to_d902d04() -> Result<(), Box<dyn std::error::Error>> {
    let fixture_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d902d04_talk_turn0");
    let turn0_dir = fixture_root.join("turn0_game");
    let manifest_tsv = fs::read_to_string(fixture_root.join("d902d04_manifest.tsv"))?;

    let tmp = fresh_tmp_dir("d902d04_parity");
    let game_dir = tmp.join("game");
    fs::create_dir_all(&game_dir)?;
    for name in ["eval-77.save.json", "eval-77.seal", "setup.json"] {
        fs::copy(turn0_dir.join(name), game_dir.join(name))?;
    }
    fs::copy(fixture_root.join("seats.json"), tmp.join("seats.json"))?;

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null())
        .args(["--exact", "d902d04_talk_parity_child_helper", "--nocapture"])
        .env(ML_D902D04_PARITY_GAME_DIR_ENV, &game_dir)
        .env(ML_D902D04_PARITY_TMP_DIR_ENV, &tmp);
    for (k, _) in std::env::vars() {
        let upper = k.to_ascii_uppercase();
        if upper.starts_with("LLAMA_")
            || upper.starts_with("CUDA_")
            || upper.starts_with("GGML_")
            || upper.starts_with("VLLM_")
            || matches!(
                upper.as_str(),
                "OPENROUTER_API_KEY"
                    | "OPENAI_API_KEY"
                    | "ANTHROPIC_API_KEY"
                    | "GEMINI_API_KEY"
                    | "XAI_API_KEY"
            )
        {
            cmd.env_remove(&k);
        }
    }
    let child_out = cmd.output()?;
    assert!(
        child_out.status.success(),
        "d902d04 parity child failed ({:?}):\nstdout:\n{}\nstderr:\n{}",
        child_out.status.code(),
        String::from_utf8_lossy(&child_out.stdout),
        String::from_utf8_lossy(&child_out.stderr),
    );

    let actual_files = walk_files(&game_dir);
    let mut expected_rows = BTreeMap::new();
    for line in manifest_tsv.lines().filter(|l| !l.trim().is_empty()) {
        let parts: Vec<&str> = line.split('\t').collect();
        assert_eq!(parts.len(), 3, "malformed manifest row: {line}");
        let len: usize = parts[1].parse()?;
        expected_rows.insert(parts[0].to_owned(), (len, parts[2].to_owned()));
    }

    assert_eq!(expected_rows.len(), 730, "expected 730 golden files");
    let actual_keys: BTreeSet<&String> = actual_files.keys().collect();
    let expected_keys: BTreeSet<&String> = expected_rows.keys().collect();
    assert_eq!(
        actual_keys, expected_keys,
        "relative file tree diverged from d902d04"
    );

    for (rel, (exp_len, exp_sha)) in &expected_rows {
        let bytes = &actual_files[rel];
        assert_eq!(
            bytes.len(),
            *exp_len,
            "byte length diverged for {rel} vs d902d04"
        );
        let actual_sha = sha256_hex(bytes);
        assert_eq!(&actual_sha, exp_sha, "sha256 diverged for {rel} vs d902d04");
    }

    Ok(())
}

// ============================================================================
// 4. Resume lock refuses cross-condition resume (`INCOMPATIBLE_EVAL_SETUP`)
// ============================================================================

#[test]
fn resume_lock_refuses_mismatched_communication_condition() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = fresh_tmp_dir("resume_lock");
    let talk_w = write_wrapper_for(&tmp, CommunicationMode::Talk);
    let silent_w = write_wrapper_for(&tmp, CommunicationMode::Silent);
    let seats = write_seats_json(&tmp, "seats.json", clubbot_seats_json());

    let talk_dir = tmp.join("talk_game");
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            talk_dir.clone(),
            seats.clone(),
            talk_w.clone(),
            None,
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let Err(err_ts) = run_play_with_communication_and_git_sha(
        &default_play_opts(
            talk_dir,
            seats.clone(),
            silent_w.clone(),
            None,
            EvalArm::Off,
            2,
        ),
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("expected INCOMPATIBLE_EVAL_SETUP when resuming talk as silent");
    };
    assert_eq!(err_ts.code(), "INCOMPATIBLE_EVAL_SETUP");

    let silent_dir = tmp.join("silent_game");
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            silent_dir.clone(),
            seats.clone(),
            silent_w,
            None,
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let Err(err_st) = run_play_with_communication_and_git_sha(
        &default_play_opts(silent_dir, seats, talk_w, None, EvalArm::Off, 2),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("expected INCOMPATIBLE_EVAL_SETUP when resuming silent as talk");
    };
    assert_eq!(err_st.code(), "INCOMPATIBLE_EVAL_SETUP");

    Ok(())
}

// ============================================================================
// 5. Branch refuses condition mismatch & runs clean silent branch
// ============================================================================

#[test]
fn branch_condition_mismatch_and_clean_silent_branch() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = fresh_tmp_dir("branch_comm");
    let talk_w = write_wrapper_for(&tmp, CommunicationMode::Talk);
    let silent_w = write_wrapper_for(&tmp, CommunicationMode::Silent);
    let seats = write_seats_json(
        &tmp,
        "seats.json",
        local_plus_clubbot_seats_json(Some(false)),
    );
    let serving = write_valid_serving_manifest(&tmp);

    let talk_dir = tmp.join("talk_src");
    let mock_talk = SilentAwareMockTransport::new(
        None,
        json!({"messages": [], "market_actions": [], "bundle": {}}).to_string(),
    );
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            talk_dir.clone(),
            seats.clone(),
            talk_w.clone(),
            Some(serving.clone()),
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Talk,
        &mock_talk,
        None,
        &test_git_sha(),
    )?;

    let Err(err_ts) = run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(talk_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats.clone(),
            wrapper_path: silent_w.clone(),
            serving_manifest_path: Some(serving.clone()),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("branch_mismatch_ts"),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("branching talk checkpoint as silent must refuse");
    };
    assert_eq!(err_ts.code(), "COMMUNICATION_CONDITION_MISMATCH");
    let msg_ts = err_ts.to_string();
    assert!(
        msg_ts.contains("talk") && msg_ts.contains("silent"),
        "{msg_ts}"
    );

    // Also test CLI path returns exit 1 + error code (using File seats so host API keys do not abort earlier).
    let cli_seats = write_seats_json(&tmp, "cli_clubbot.json", clubbot_seats_json());
    let mut cli_out = Vec::new();
    let mut cli_err = Vec::new();
    let code = execute_cli_with_git_sha(
        [
            "branch",
            "--checkpoint",
            &talk_dir
                .join("checkpoints/day-01-pre-act")
                .to_string_lossy(),
            "--seat",
            "Foundry",
            "--seats",
            &cli_seats.to_string_lossy(),
            "--wrapper",
            &talk_w.to_string_lossy(),
            "--communication",
            "silent",
            "--arms",
            "off",
            "--out",
            &tmp.join("branch_cli_mismatch").to_string_lossy(),
        ]
        .into_iter()
        .map(std::ffi::OsString::from),
        &mut cli_out,
        &mut cli_err,
        &test_git_sha(),
    );
    assert_eq!(code, 1);
    let cli_err_s = String::from_utf8_lossy(&cli_err);
    assert!(
        cli_err_s.contains("COMMUNICATION_CONDITION_MISMATCH"),
        "{cli_err_s}"
    );

    // Now silent source -> talk branch fails, silent source -> silent branch succeeds.
    let silent_dir = tmp.join("silent_src");
    let mock_silent = SilentAwareMockTransport::new(
        None,
        json!({"market_actions": [], "bundle": {}}).to_string(),
    );
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            silent_dir.clone(),
            seats.clone(),
            silent_w.clone(),
            Some(serving.clone()),
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Silent,
        &mock_silent,
        None,
        &test_git_sha(),
    )?;

    let Err(err_st) = run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(silent_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats.clone(),
            wrapper_path: talk_w,
            serving_manifest_path: Some(serving.clone()),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("branch_mismatch_st"),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("branching silent checkpoint as talk must refuse");
    };
    assert_eq!(err_st.code(), "COMMUNICATION_CONDITION_MISMATCH");

    let silent_branch_out = tmp.join("silent_branch_ok");
    let outcomes = run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(silent_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats,
            wrapper_path: silent_w,
            serving_manifest_path: Some(serving),
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: silent_branch_out.clone(),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Silent,
        &SilentAwareMockTransport::new(
            None,
            json!({"market_actions": [], "bundle": {}}).to_string(),
        ),
        None,
        &test_git_sha(),
    )?;
    assert_eq!(outcomes.len(), 3);
    for (rel, _) in walk_files(&silent_branch_out) {
        assert!(!rel.ends_with(".negotiation.json"), "unexpected {rel}");
    }

    Ok(())
}

// ============================================================================
// 6. Pipeline: MIXED_COMMUNICATION_CONDITIONS & full pure-silent pipeline
// ============================================================================

#[test]
fn pipeline_refuses_mixed_communication_and_runs_pure_silent()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = fresh_tmp_dir("pipeline_silent");
    let talk_w = write_wrapper_for(&tmp, CommunicationMode::Talk);
    let silent_w = write_wrapper_for(&tmp, CommunicationMode::Silent);
    let seats = write_seats_json(&tmp, "seats.json", clubbot_seats_json());

    let talk_dir = tmp.join("talk_src");
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            talk_dir.clone(),
            seats.clone(),
            talk_w.clone(),
            None,
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let talk_branch = tmp.join("talk_branch");
    run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(talk_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats.clone(),
            wrapper_path: talk_w,
            serving_manifest_path: None,
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: talk_branch.clone(),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;

    let silent_dir = tmp.join("silent_src");
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            silent_dir.clone(),
            seats.clone(),
            silent_w.clone(),
            None,
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let silent_branch = tmp.join("silent_branch");
    run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(silent_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats,
            wrapper_path: silent_w,
            serving_manifest_path: None,
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: silent_branch.clone(),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;

    let cfg = ScorerConfig {
        scenarios_m: 2,
        k_synth_max: 2,
        ..ScorerConfig::default()
    };
    let cfg_path = tmp.join("cfg.json");
    fs::write(&cfg_path, serde_json::to_string_pretty(&cfg)?)?;

    // Mixed build-blind-inputs must refuse.
    let Err(mixed_bi) = run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![
            talk_dir.clone(),
            talk_branch.clone(),
            silent_dir.clone(),
            silent_branch.clone(),
        ],
        out_blind_dir: tmp.join("mixed_blind"),
        out_arm_map_dir: tmp.join("mixed_arm_map"),
        out_checkpoints_dir: None,
        config_path: cfg_path.clone(),
        scorer_seed: 12,
    }) else {
        panic!("expected MIXED_COMMUNICATION_CONDITIONS in build-blind-inputs");
    };
    assert_eq!(mixed_bi.code(), "MIXED_COMMUNICATION_CONDITIONS");

    // Unexpected talk/negotiation artifact inside a silent source run must refuse (`blind_builder`).
    let planted_talk = silent_dir.join("decisions/day-01-talk-Foundry.json");
    fs::write(&planted_talk, "{}")?;
    let Err(planted_mixed_bi) = run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![silent_dir.clone(), silent_branch.clone()],
        out_blind_dir: tmp.join("mixed_blind_planted"),
        out_arm_map_dir: tmp.join("mixed_arm_map_planted"),
        out_checkpoints_dir: None,
        config_path: cfg_path.clone(),
        scorer_seed: 12,
    }) else {
        panic!("expected MIXED_COMMUNICATION_CONDITIONS for unexpected talk file in silent dir");
    };
    assert_eq!(planted_mixed_bi.code(), "MIXED_COMMUNICATION_CONDITIONS");
    fs::remove_file(&planted_talk)?;

    // Pure silent pipeline succeeds end-to-end.
    let blind_dir = tmp.join("blind");
    let arm_map_dir = tmp.join("arm_map");
    let ckpts_dir = tmp.join("ckpts");
    let report = run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![silent_dir.clone(), silent_branch.clone()],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 12,
    })?;
    assert_eq!(report.states_built, 1);

    let am_path = fs::read_dir(&arm_map_dir)?
        .filter_map(|e| e.ok().map(|x| x.path()))
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .unwrap_or_else(|| panic!("missing arm_map state json"));
    let am_state: ArmMapStateFile = serde_json::from_slice(&fs::read(&am_path)?)?;
    assert_eq!(am_state.communication, CommunicationMode::Silent);

    let vc_path = tmp.join("verified_checkpoints.json");
    let vc_report = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir.clone(),
            out_path: vc_path.clone(),
            blind_dir: Some(blind_dir.clone()),
        },
        &test_git_sha(),
    )?;
    assert_eq!(vc_report.verified_count, 1);

    // Score blind request directly using content-addressed checkpoints.
    let scores_dir = tmp.join("scores");
    fs::create_dir_all(&scores_dir)?;
    let blind_req_path = blind_dir.join(format!("{}.json", am_state.state_id));
    let blind_bytes = fs::read(&blind_req_path)?;
    let blind_req: BlindScoringRequest = serde_json::from_slice(&blind_bytes)?;
    let ca_save = ckpts_dir.join(format!("{}.save.json", am_state.pre_act_save_sha256));
    let ca_seal = ckpts_dir.join(format!("{}.seal", am_state.pre_act_seal_sha256));
    let scored = execute_blind_request(&blind_req, &cfg, &ca_save, &ca_seal)?;
    fs::write(
        scores_dir.join(format!("{}.json", am_state.state_id)),
        serde_json::to_string_pretty(&scored)?,
    )?;

    // Mixed freeze (--arm-map + talk --run) must refuse.
    let Err(mixed_freeze) = run_freeze_with_provenance(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: tmp.join("mixed_freeze.json"),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        Some(&arm_map_dir),
        std::slice::from_ref(&talk_dir),
        &test_git_sha(),
    ) else {
        panic!("expected MIXED_COMMUNICATION_CONDITIONS in freeze");
    };
    assert_eq!(mixed_freeze.code(), "MIXED_COMMUNICATION_CONDITIONS");

    // Clean silent freeze.
    let freeze_path = tmp.join("scorer_freeze.json");
    let freeze: ScorerFreezeFile = run_freeze_with_provenance(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        Some(&arm_map_dir),
        std::slice::from_ref(&silent_dir),
        &test_git_sha(),
    )?;
    assert_eq!(freeze.communication, CommunicationMode::Silent);

    // Mixed join (adding talk_dir in source_game_dirs) must refuse before writing outputs.
    let Err(mixed_join) = run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.join("mixed_join_out"),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: Some(blind_dir.clone()),
        source_game_dirs: vec![silent_dir.clone(), talk_dir],
    }) else {
        panic!("expected MIXED_COMMUNICATION_CONDITIONS in join");
    };
    assert_eq!(mixed_join.code(), "MIXED_COMMUNICATION_CONDITIONS");

    // Clean silent join succeeds and records communication == Silent across outputs.
    let join_out = tmp.join("join_out");
    let join_report = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: join_out.clone(),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: Some(blind_dir),
        source_game_dirs: vec![silent_dir],
    })?;
    assert_eq!(join_report.states_joined, 1);
    assert_eq!(join_report.decisions_joined, 3);

    let joined_file: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(join_out.join("joined_decisions.json"))?)?;
    let summary_file: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(join_out.join("per_arm_summary.json"))?)?;
    assert_eq!(joined_file.communication, CommunicationMode::Silent);
    assert_eq!(summary_file.communication, CommunicationMode::Silent);
    assert!(
        joined_file
            .rows
            .iter()
            .all(|r| r.communication == CommunicationMode::Silent)
    );
    assert!(
        summary_file
            .arms
            .iter()
            .all(|a| a.communication == CommunicationMode::Silent)
    );

    Ok(())
}

// ============================================================================
// 7. Full-season legality, replay, byte-identity, regents & refusals
// ============================================================================

#[test]
fn baselines_full_season_legality_replay_determinism_and_refusals()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = fresh_tmp_dir("baselines_season");
    let talk_w = write_wrapper_for(&tmp, CommunicationMode::Talk);
    let silent_w = write_wrapper_for(&tmp, CommunicationMode::Silent);

    // Unknown file model rejected.
    let unknown_seats = write_seats_json(
        &tmp,
        "unknown.json",
        json!({
            "seats": [
                { "guild": "Foundry", "backend": "file", "model": "mystery-bot" },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    let Err(err_unk) = run_play_with_communication_and_git_sha(
        &default_play_opts(
            tmp.join("unk_dir"),
            unknown_seats,
            talk_w.clone(),
            None,
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("unknown file seat model must refuse");
    };
    assert_eq!(err_unk.code(), "UNSUPPORTED_FILE_SEAT_MODEL");

    // `tit-for-tat` in Seat 0 + `always-defect` + `random-legal` + `ClubBot`:
    let mixed_seats = write_seats_json(
        &tmp,
        "mixed_baselines.json",
        json!({
            "seats": [
                { "guild": "Foundry", "backend": "file", "model": "tit-for-tat" },
                { "guild": "Quarry", "backend": "file", "model": "always-defect" },
                { "guild": "Harbor", "backend": "file", "model": "random-legal" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );

    // `tit-for-tat` refused in silent mode!
    let Err(err_tft_silent) = run_play_with_communication_and_git_sha(
        &default_play_opts(
            tmp.join("tft_silent"),
            mixed_seats.clone(),
            silent_w.clone(),
            None,
            EvalArm::Off,
            1,
        ),
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("tit-for-tat must refuse in silent mode");
    };
    assert_eq!(err_tft_silent.code(), "BASELINE_REQUIRES_TALK");

    // Two identical Talk 32-turn seasons from a shared Turn-0 fork -> byte-identical!
    let base_turn0 = tmp.join("shared_turn0");
    fs::create_dir_all(&base_turn0)?;
    let mut host0 = EvalHost::from_standard_setup_with_model_seats(
        TableId(1),
        4,
        4242,
        7777,
        &[true, true, true, false],
    )?;
    host0.set_engine_commit(FALLBACK_GIT_SHA)?;
    persist_eval_game_state(&host0, &base_turn0)?;

    let run_a = tmp.join("talk_run_a");
    let run_b = tmp.join("talk_run_b");
    fs::create_dir_all(&run_a)?;
    fs::create_dir_all(&run_b)?;
    for f in ["eval-1.save.json", "eval-1.seal"] {
        fs::copy(base_turn0.join(f), run_a.join(f))?;
        fs::copy(base_turn0.join(f), run_b.join(f))?;
    }

    let man_a = run_play_with_communication_and_git_sha(
        &default_play_opts(
            run_a.clone(),
            mixed_seats.clone(),
            talk_w,
            None,
            EvalArm::Off,
            HARD_TURN_CAP,
        ),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let man_b = run_play_with_communication_and_git_sha(
        &default_play_opts(
            run_b.clone(),
            mixed_seats.clone(),
            write_wrapper_for(&tmp, CommunicationMode::Talk),
            None,
            EvalArm::Off,
            HARD_TURN_CAP,
        ),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;

    assert!(man_a.played_turns >= 24);
    assert_eq!(man_a.played_turns, man_b.played_turns);
    assert!(
        !man_a.seats[0].has_regent,
        "tit-for-tat in seat 0 must have regent: None"
    );
    assert!(
        !man_a.seats[1].has_regent,
        "always-defect must have regent: None"
    );
    assert!(
        !man_a.seats[2].has_regent,
        "random-legal must have regent: None"
    );

    // Check full-season legality + zero tokens/latency on all baseline decisions.
    let replay_a = fs::read_to_string(run_a.join("eval-1.replay.json"))?;
    let replay_b = fs::read_to_string(run_b.join("eval-1.replay.json"))?;
    assert_eq!(
        replay_a, replay_b,
        "two full-season baseline runs must be byte-identical"
    );
    let replayed = replay_eval_season(&serde_json::from_str(&replay_a)?)?;
    let sb: FinalScoreboard =
        serde_json::from_str(&fs::read_to_string(run_a.join("scoreboard.json"))?)?;
    assert_eq!(replayed.scoreboard, sb);

    for d in 1..=man_a.played_turns {
        for (guild, kind) in [
            ("Foundry", "tit-for-tat"),
            ("Quarry", "always-defect"),
            ("Harbor", "random-legal"),
            ("Mint", "ClubBot"),
        ] {
            let dec_path = run_a.join(format!("decisions/day-{d:02}-act-{guild}.json"));
            let dec_b_path = run_b.join(format!("decisions/day-{d:02}-act-{guild}.json"));
            let rec: EvalDecisionRecord = serde_json::from_slice(&fs::read(&dec_path)?)?;
            let mut rec_b: EvalDecisionRecord = serde_json::from_slice(&fs::read(&dec_b_path)?)?;
            // Compare all decision fields deterministically (Turn-0 forks assign fresh random game_id per R5).
            rec_b.game_id = rec.game_id.clone();
            assert_eq!(
                rec, rec_b,
                "{kind} day {d} diverged between duplicate Turn-0 runs"
            );
            assert_eq!(rec.seat_kind, kind);
            assert_eq!(
                rec.refused_market_actions, 0,
                "{kind} day {d} refused market action"
            );
            assert_eq!(
                rec.refused_bundle_legs, 0,
                "{kind} day {d} refused bundle leg"
            );
            assert!(!rec.no_reply && !rec.parse_failure);
            assert_eq!(rec.prompt_tokens, 0);
            assert_eq!(rec.completion_tokens, 0);
            assert_eq!(rec.wall_secs, None);
            if kind != "ClubBot" {
                let raw = rec.raw_reply.as_deref().unwrap_or("");
                let parsed = parse_eval_act_reply_for_communication(
                    CommunicationMode::Talk,
                    raw,
                    rec.seat,
                    false,
                );
                assert!(
                    parsed.refusal_code.is_none(),
                    "invalid raw_reply on {kind} day {d}: {raw}"
                );
            }
        }
    }

    // Silent run using `random-legal` + `always-defect` + `ClubBot` cleanly runs to close.
    let silent_seats = write_seats_json(
        &tmp,
        "silent_baselines.json",
        json!({
            "seats": [
                { "guild": "Foundry", "backend": "file", "model": "random-legal" },
                { "guild": "Quarry", "backend": "file", "model": "always-defect" },
                { "guild": "Harbor", "backend": "file", "model": "random-legal" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    let silent_game = tmp.join("silent_baselines_game");
    let s_man = run_play_with_communication_and_git_sha(
        &default_play_opts(
            silent_game.clone(),
            silent_seats,
            silent_w,
            None,
            EvalArm::Off,
            HARD_TURN_CAP,
        ),
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    assert_eq!(s_man.communication, CommunicationMode::Silent);
    assert!(s_man.played_turns >= 24);
    assert!(!s_man.seats[0].has_regent);
    assert!(!s_man.seats[1].has_regent);
    assert!(!s_man.seats[2].has_regent);

    // Branch refusal when branching Silent checkpoint with tit-for-tat seats.
    let Err(err_branch_tft) = run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(silent_game.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: mixed_seats,
            wrapper_path: write_wrapper_for(&tmp, CommunicationMode::Silent),
            serving_manifest_path: None,
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("branch_tft_silent"),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    ) else {
        panic!("tit-for-tat must refuse in silent branch");
    };
    assert_eq!(err_branch_tft.code(), "BASELINE_REQUIRES_TALK");

    Ok(())
}

// ============================================================================
// 8. Defining scenarios: `always-defect` and 3-day `tit-for-tat` reciprocity
// ============================================================================

#[test]
fn always_defect_and_tit_for_tat_defining_scenarios() -> Result<(), Box<dyn std::error::Error>> {
    let guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Mint".to_owned(),
    ];
    let mut host = EvalHost::from_standard_setup_with_model_seats(
        TableId(55),
        4,
        4242,
        1234,
        &[true, true, true, true],
    )?;

    // Part A: `always-defect` promises a trade in Talk and withholds it in Act.
    let obs1 = host.observe_seat(Seat(1))?;
    let mandate1 = *host.mandate_for_seat(Seat(1))?;
    let (talk_msgs, whispers, _) = decide_scripted_baseline_talk(
        league_eval::EvalFileSeatModel::AlwaysDefect,
        &obs1,
        &guilds,
    )?;
    assert!(
        !talk_msgs.is_empty() && !whispers.is_empty(),
        "always-defect must emit extractable Talk promises"
    );
    let promised_goods: BTreeSet<AssetId> = league_eval::plan_always_defect_talk(&obs1, &guilds)?
        .1
        .into_iter()
        .map(|(_, g)| g)
        .collect();
    let def_act = decide_always_defect_act(
        4242,
        &obs1,
        &mandate1,
        &guilds,
        CommunicationMode::Talk,
        Some(&obs1),
    )?;
    for a in &def_act.market_actions {
        if let MarketAction::PostOffer { order, .. } = a {
            assert!(
                !promised_goods.contains(&order.good),
                "always-defect must withhold promised offer on {:?}",
                order.good
            );
        }
    }

    // Part B: 3-day `tit-for-tat` reciprocity state machine on Seat 0 (`Foundry`) vs Seat 1 (`Quarry`).
    let obs0_day1 = host.observe_seat(Seat(0))?;
    let mandate0_day1 = *host.mandate_for_seat(Seat(0))?;
    let tmp = fresh_tmp_dir("tft_3day");
    let to_seat = tmp.join("to-seat");
    fs::create_dir_all(&to_seat)?;

    // Day 1: fresh table -> Cooperate, Seat 0 posts its promised offer toward Quarry.
    // Quarry sends a valid applicable buy promise for Raw(0) using its starting CRED.
    let raw0_parity = obs0_day1.parities.asset_parity(AssetId::Raw(0)).max(1);
    let raw0_whole = (raw0_parity + UNIT_SCALE / 2) / UNIT_SCALE;
    assert!(
        obs0_day1.seats[1].cred >= raw0_whole * UNIT_SCALE,
        "Quarry seat must hold CRED to back its Day-1 buy promise"
    );
    fs::write(
        to_seat.join("day-01-act-Foundry.md"),
        league_eval::render_seat_view_markdown_for_communication(
            league_relay::Round::Act,
            CommunicationMode::Talk,
            &obs0_day1,
        )?,
    )?;
    fs::write(
        to_seat.join("day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "n".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: format!("I will buy 1 raw 0 at {raw0_whole}."),
            }],
        })?,
    )?;
    let hist_d1 = OwnToSeatHistory::load(&to_seat, "Foundry", 1)?;
    let stances_d1 = compute_tit_for_tat_stances(&hist_d1, &obs0_day1, &guilds)?;
    assert_eq!(stances_d1.get(&1), Some(&PeerStance::Cooperate));
    let sub_d1 = decide_tit_for_tat_act(1234, &hist_d1, &obs0_day1, &mandate0_day1, &guilds)?;
    assert!(
        sub_d1.market_actions.iter().any(|a| matches!(
            a,
            MarketAction::PostOffer { order, .. }
                if (order.side == OfferSide::Sell && order.good == AssetId::Raw(0))
                    || (order.side == OfferSide::Buy && order.good == AssetId::Raw(1))
        )),
        "Day 1 cooperating tit-for-tat should post its promised offer toward Quarry: {:?}",
        sub_d1.market_actions
    );

    // Apply only Foundry's order (Quarry submits empty -> breaks its Day-1 promise!).
    let mut subs_d1 = BTreeMap::new();
    subs_d1.insert(0, sub_d1);
    for s in 1..4 {
        subs_d1.insert(s, EvalActSubmission::default());
    }
    host.apply_act_submissions(&subs_d1)?;
    let _rec_d1: PublicSweepReceipt = host.sweep_turn()?;

    // Day 2: Foundry sees Day-1 break -> PeerStance::Defect -> withholds raw 1 AND cancels resting raw-1 offer!
    let obs0_day2 = host.observe_seat(Seat(0))?;
    let mandate0_day2 = *host.mandate_for_seat(Seat(0))?;
    let d2_raw0_whole =
        (obs0_day2.parities.asset_parity(AssetId::Raw(0)).max(1) + UNIT_SCALE / 2) / UNIT_SCALE;
    fs::write(
        to_seat.join("day-02-act-Foundry.md"),
        league_eval::render_seat_view_markdown_for_communication(
            league_relay::Round::Act,
            CommunicationMode::Talk,
            &obs0_day2,
        )?,
    )?;
    fs::write(
        to_seat.join("day-02-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "n".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: format!("I will buy 1 raw 0 at {d2_raw0_whole}."),
            }],
        })?,
    )?;
    let hist_d2 = OwnToSeatHistory::load(&to_seat, "Foundry", 2)?;
    let stances_d2 = compute_tit_for_tat_stances(&hist_d2, &obs0_day2, &guilds)?;
    assert_eq!(stances_d2.get(&1), Some(&PeerStance::Defect));
    let sub_d2 = decide_tit_for_tat_act(1234, &hist_d2, &obs0_day2, &mandate0_day2, &guilds)?;
    assert!(
        sub_d2
            .market_actions
            .iter()
            .any(|a| matches!(a, MarketAction::CancelOffer { .. })),
        "Day 2 defect retaliation must cancel resting matching offer"
    );
    assert!(
        !sub_d2.market_actions.iter().any(|a| matches!(
            a,
            MarketAction::PostOffer { order, .. }
                if (order.side == OfferSide::Sell && order.good == AssetId::Raw(0))
                    || (order.side == OfferSide::Buy && order.good == AssetId::Raw(1))
        )),
        "Day 2 defect retaliation must withhold promised offer toward Quarry: {:?}",
        sub_d2.market_actions
    );

    // Day 2 resolution: Quarry actually posts its promised buy offer for raw 0 (either fills or rests).
    let mut subs_d2 = BTreeMap::new();
    subs_d2.insert(0, sub_d2);
    subs_d2.insert(
        1,
        EvalActSubmission {
            market_actions: vec![MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: UNIT_SCALE,
                    price_per_unit_cred: d2_raw0_whole * UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
            }],
            ..EvalActSubmission::default()
        },
    );
    subs_d2.insert(2, EvalActSubmission::default());
    subs_d2.insert(3, EvalActSubmission::default());
    host.apply_act_submissions(&subs_d2)?;
    let _rec_d2 = host.sweep_turn()?;

    // Day 3 observation: Quarry kept its Day-2 promise on the live engine, so tit-for-tat forgives immediately!
    let obs0_day3 = host.observe_seat(Seat(0))?;
    let mandate0_day3 = *host.mandate_for_seat(Seat(0))?;
    let hist_d3 = OwnToSeatHistory::load(&to_seat, "Foundry", 3)?;
    let stances_d3 = compute_tit_for_tat_stances(&hist_d3, &obs0_day3, &guilds)?;
    assert_eq!(
        stances_d3.get(&1),
        Some(&PeerStance::Cooperate),
        "Day 3 must forgive Quarry and return to Cooperate after Quarry kept on Day 2"
    );
    assert_eq!(obs0_day3.phase, TurnPhase::TalkAndMarket);
    let sub_d3 = decide_tit_for_tat_act(1234, &hist_d3, &obs0_day3, &mandate0_day3, &guilds)?;
    assert!(
        sub_d3.market_actions.iter().any(|a| matches!(
            a,
            MarketAction::PostOffer { order, .. }
                if (order.side == OfferSide::Sell && order.good == AssetId::Raw(0))
                    || (order.side == OfferSide::Buy && order.good == AssetId::Raw(1))
        )),
        "Day 3 forgiven tit-for-tat must resume posting promised offer toward Quarry: {:?}",
        sub_d3.market_actions
    );

    Ok(())
}

// ============================================================================
// 9. `tit-for-tat` information-limit test (own `to-seat/` only)
// ============================================================================

#[test]
fn tit_for_tat_ignores_peer_decisions_from_seat_and_relay() -> Result<(), Box<dyn std::error::Error>>
{
    let guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Mint".to_owned(),
    ];
    let host = EvalHost::from_standard_setup_with_model_seats(
        TableId(56),
        4,
        4242,
        5555,
        &[true, true, true, true],
    )?;
    let obs = host.observe_seat(Seat(0))?;
    let mandate = *host.mandate_for_seat(Seat(0))?;

    let tmp_clean = fresh_tmp_dir("tft_clean");
    let to_clean = tmp_clean.join("to-seat");
    fs::create_dir_all(&to_clean)?;
    fs::write(
        to_clean.join("day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "n".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "I will sell 1 raw 1 at 10.".to_owned(),
            }],
        })?,
    )?;

    let tmp_poison = fresh_tmp_dir("tft_poison");
    let to_poison = tmp_poison.join("to-seat");
    fs::create_dir_all(&to_poison)?;
    fs::copy(
        to_clean.join("day-01-act-Foundry.negotiation.json"),
        to_poison.join("day-01-act-Foundry.negotiation.json"),
    )?;
    // Poison peer `to-seat/`, `decisions/`, `from-seat/`, `relay/` files!
    fs::write(
        to_poison.join("day-01-act-Quarry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "n".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Harbor".to_owned(),
                intent_header: None,
                body: "I will sell 1 raw 2 at 10.".to_owned(),
            }],
        })?,
    )?;
    fs::create_dir_all(tmp_poison.join("decisions"))?;
    fs::write(
        tmp_poison.join("decisions/day-01-talk-Quarry.json"),
        r#"{"messages":[{"to_guild":"Harbor","body":"I will sell 1 raw 1 at 10."}]}"#,
    )?;
    fs::create_dir_all(tmp_poison.join("from-seat"))?;
    fs::write(
        tmp_poison.join("from-seat/day-01-talk-Quarry.json"),
        r#"{"messages":[{"to_guild":"Mint","body":"I will sell 1 raw 1 at 10."}]}"#,
    )?;
    fs::create_dir_all(tmp_poison.join("relay/seats"))?;
    fs::write(
        tmp_poison.join("relay/seats/Quarry.jsonl"),
        "{\"role\":\"assistant\",\"content\":\"secret\"}\n",
    )?;

    let h_clean = OwnToSeatHistory::load(&to_clean, "Foundry", 1)?;
    let h_poison = OwnToSeatHistory::load(&to_poison, "Foundry", 1)?;

    let sub_clean = decide_tit_for_tat_act(5555, &h_clean, &obs, &mandate, &guilds)?;
    let sub_poison = decide_tit_for_tat_act(5555, &h_poison, &obs, &mandate, &guilds)?;
    assert_eq!(sub_clean, sub_poison);

    Ok(())
}

// ============================================================================
// 10. `(communication, arm, seat_kind)` key recovery across artefacts
// ============================================================================

#[test]
fn key_recovery_across_setup_manifest_outcome_and_decisions()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = fresh_tmp_dir("key_recovery");
    let talk_w = write_wrapper_for(&tmp, CommunicationMode::Talk);
    let silent_w = write_wrapper_for(&tmp, CommunicationMode::Silent);
    let seats = write_seats_json(
        &tmp,
        "seats.json",
        json!({
            "seats": [
                { "guild": "Foundry", "backend": "file", "model": "always-defect" },
                { "guild": "Quarry", "backend": "file", "model": "random-legal" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );

    // Talk run under `--arm control` (omits `"communication"` on JSON wire, defaults to Talk).
    let talk_dir = tmp.join("talk_ctrl");
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            talk_dir.clone(),
            seats.clone(),
            talk_w.clone(),
            None,
            EvalArm::Control,
            1,
        ),
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let setup_raw = fs::read_to_string(talk_dir.join("setup.json"))?;
    assert!(
        !setup_raw.contains("\"communication\""),
        "talk must omit communication key: {setup_raw}"
    );
    let setup_t: EvalSetupFile = serde_json::from_str(&setup_raw)?;
    let manifest_t: EvalManifest =
        serde_json::from_slice(&fs::read(talk_dir.join("manifest.json"))?)?;
    let dec_t: EvalDecisionRecord = serde_json::from_slice(&fs::read(
        talk_dir.join("decisions/day-01-act-Foundry.json"),
    )?)?;
    assert_eq!(
        (setup_t.communication, setup_t.arm),
        (CommunicationMode::Talk, EvalArm::Control)
    );
    assert_eq!(
        (manifest_t.communication, manifest_t.arm),
        (CommunicationMode::Talk, EvalArm::Control)
    );
    assert_eq!(
        (dec_t.communication, dec_t.arm, dec_t.seat_kind.as_str()),
        (CommunicationMode::Talk, EvalArm::Control, "always-defect")
    );

    // Silent run under `--arm on` (explicitly serializes `"communication": "silent"`).
    let silent_dir = tmp.join("silent_on");
    run_play_with_communication_and_git_sha(
        &default_play_opts(
            silent_dir.clone(),
            seats.clone(),
            silent_w.clone(),
            None,
            EvalArm::On,
            1,
        ),
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let s_setup: EvalSetupFile = serde_json::from_slice(&fs::read(silent_dir.join("setup.json"))?)?;
    let s_manifest: EvalManifest =
        serde_json::from_slice(&fs::read(silent_dir.join("manifest.json"))?)?;
    let s_dec: EvalDecisionRecord = serde_json::from_slice(&fs::read(
        silent_dir.join("decisions/day-01-act-Quarry.json"),
    )?)?;
    assert_eq!(
        (s_setup.communication, s_setup.arm),
        (CommunicationMode::Silent, EvalArm::On)
    );
    assert_eq!(
        (s_manifest.communication, s_manifest.arm),
        (CommunicationMode::Silent, EvalArm::On)
    );
    assert_eq!(
        (s_dec.communication, s_dec.arm, s_dec.seat_kind.as_str()),
        (CommunicationMode::Silent, EvalArm::On, "random-legal")
    );

    let branch_out = tmp.join("branch_kr");
    run_branch_with_communication_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(silent_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats,
            wrapper_path: silent_w,
            serving_manifest_path: None,
            arms: vec![EvalArm::Control],
            out_dir: branch_out.clone(),
            memory_mode: MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Silent,
        &NoopTransport,
        None,
        &test_git_sha(),
    )?;
    let outcome_s: BranchArmOutcome =
        serde_json::from_slice(&fs::read(branch_out.join("control/outcome.json"))?)?;
    assert_eq!(
        (
            outcome_s.communication,
            outcome_s.arm,
            outcome_s.decision.seat_kind.as_str(),
        ),
        (CommunicationMode::Silent, EvalArm::Control, "always-defect")
    );

    Ok(())
}
