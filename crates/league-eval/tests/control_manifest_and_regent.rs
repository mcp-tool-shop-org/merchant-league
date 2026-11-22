//! Tests for the inert control arm, pinned wrapper, serving manifests, telemetry, and model-seat Regent rules.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use league_core::{EvalHost, Seat, TableId};
use league_eval::{
    BranchArmOutcome, BranchManifest, BranchOptions, CRATE_GIT_SHA, CrateGitShaSource, EvalArm,
    EvalDecisionRecord, EvalError, EvalManifest, MetadataProvenance, NoopTransport,
    PINNED_WRAPPERS, PlayOptions, REJECTED_WRAPPER_HASHES, SelectedPreActState, SelectionList,
    TurnStratum, WordingArmDigests, check_wrapper_digest, eval_wording_slot,
    execute_cli_with_git_sha, run_branch_with_transport_and_git_sha,
    run_play_with_transport_and_git_sha,
};
use league_relay::{ApiKey, RawHttpResponse, Transport, TransportError, sha256_hex};
use serde_json::{Value, json};

const TEST_FALLBACK_GIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ML_EVAL_TEST_SEAM_ARGS_FILE_ENV: &str = "ML_EVAL_TEST_SEAM_ARGS_FILE";

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

#[test]
fn eval_cli_seam_child_helper() {
    let Ok(args_file) = std::env::var(ML_EVAL_TEST_SEAM_ARGS_FILE_ENV) else {
        return;
    };
    let raw = must_ok(fs::read_to_string(&args_file), "read seam CLI args file");
    let cli_args: Vec<String> = must_ok(serde_json::from_str(&raw), "parse seam CLI args JSON");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = execute_cli_with_git_sha(
        cli_args.into_iter().map(std::ffi::OsString::from),
        &mut out,
        &mut err,
        &test_git_sha_source(),
    );
    let _ = std::io::stdout().write_all(&out);
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(&err);
    let _ = std::io::stderr().flush();
    std::process::exit(code);
}

fn run_cli_via_test_seam(
    tmp: &Path,
    tag: &str,
    cli_args: &[&str],
    envs: &[(&str, &str)],
    env_removes: &[&str],
) -> Output {
    let args_path = tmp.join(format!("seam_args_{tag}.json"));
    must_ok(
        fs::write(&args_path, json!(cli_args).to_string()),
        "write seam CLI args file",
    );
    let exe = must_ok(std::env::current_exe(), "resolve test binary path");
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "eval_cli_seam_child_helper", "--nocapture"])
        .env(ML_EVAL_TEST_SEAM_ARGS_FILE_ENV, &args_path);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    for k in env_removes {
        cmd.env_remove(k);
    }
    must_ok(cmd.output(), tag)
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

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
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
    let dir =
        std::env::temp_dir().join(format!("ml_eval_cmr_{label}_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    must_ok(fs::create_dir_all(&dir), "create temp dir");
    dir
}

fn mock_chat_with_telemetry(
    id: &str,
    model: &str,
    content: &str,
    prompt_tokens: u64,
    completion_tokens: u64,
) -> String {
    let top_logprobs: Vec<Value> = (0_u64..5)
        .map(|k| {
            json!({
                "token": format!("t{k}"),
                "id": 500 + k,
                "token_id": 500 + k,
                "logprob": -0.05 * ((k + 1) as f64)
            })
        })
        .collect();
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
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    })
    .to_string()
}

fn copy_pinned_r3_wrapper(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("wrappers/v2_seat_brief_2026_11_21_r3.md");
    let bytes = must_ok(fs::read(&src), "read committed pinned r3 wrapper");
    let dst = dir.join("wrapper.md");
    must_ok(fs::write(&dst, &bytes), "copy pinned r3 wrapper");
    dst
}

fn write_seats(dir: &Path, value: Value) -> PathBuf {
    must_ok(fs::create_dir_all(dir), "create seats dir");
    let p = dir.join("seats.json");
    let text = must_ok(serde_json::to_string_pretty(&value), "serialize seats");
    must_ok(fs::write(&p, text), "write seats");
    p
}

fn local_foundry_three_clubbot_seats() -> Value {
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

fn base_llama_serving_json() -> Value {
    json!({
        "engine": "llama_server",
        "server_version": "b11433",
        "server_build_commit": "0123abcd",
        "server_info_provenance": "server_reported",
        "model": {
            "filename": "Qwen3-32B-Q4_K_M.gguf",
            "quant": "Q4_K_M",
            "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
            "chat_template_sha256": "2222222222222222222222222222222222222222222222222222222222222222",
            "provenance": "operator_supplied"
        },
        "machine": {
            "gpu_name": "NVIDIA RTX 5090",
            "gpu_uuid": "GPU-RAW-SECRET-UUID-99887766",
            "sm_count": 170,
            "gpu_driver": "570.12",
            "cuda_runtime_version": "12.8",
            "vram_mib": 32768,
            "os_build": "Windows-11-24H2",
            "wsl_kernel_version": "6.6.87.2-microsoft-standard-WSL2",
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
            "provenance": "server_reported"
        },
        "cache_settings": {
            "per_request_cache_prompt": false,
            "no_cache_prompt_flag": true,
            "cache_ram_mib": 0,
            "cache_idle_slots": false,
            "provenance": "server_reported"
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
            "stop_sequences": ["<|im_end|>"],
            "max_tokens": 1024,
            "sampler_order": ["top_k", "top_p", "min_p", "temperature"],
            "provenance": "server_reported"
        }
    })
}

fn write_serving_manifest_json(dir: &Path, name: &str, value: &Value) -> PathBuf {
    let p = dir.join(name);
    let txt = must_ok(
        serde_json::to_string_pretty(value),
        "serialize serving manifest",
    );
    must_ok(fs::write(&p, txt), "write serving manifest");
    p
}

#[test]
fn pinned_wrappers_enforce_r3_reject_r1_r2_and_embed_zero_prose() {
    assert_eq!(PINNED_WRAPPERS.len(), 1);
    assert_eq!(PINNED_WRAPPERS[0].0, "v2_seat_brief_2026_11_21_r3");
    assert_eq!(
        PINNED_WRAPPERS[0].1,
        "66d5665950b2ef9bad177dbcfbe132912fa99f09c328a40866502dbf16d6a06d"
    );

    let r3_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("wrappers/v2_seat_brief_2026_11_21_r3.md");
    let r3_bytes = must_ok(fs::read(&r3_path), "read committed r3 wrapper");
    assert_eq!(r3_bytes.len(), 12_320);
    assert_eq!(sha256_hex(&r3_bytes), PINNED_WRAPPERS[0].1);

    assert_eq!(
        REJECTED_WRAPPER_HASHES,
        [
            (
                "v2_seat_brief_2026_11_17_r1",
                "53124129dc2bf06dc97c624ab2509e062b5882b058a9d56502fc28f729975f4b",
            ),
            (
                "v2_seat_brief_2026_11_20_r2",
                "1fecf981f1df39dfc552cef90c24430aff4faa0f8990d77cfafee62a7ef74268",
            ),
        ]
    );
    for (old_ver, rejected_sha) in REJECTED_WRAPPER_HASHES {
        let err = match check_wrapper_digest(rejected_sha, None) {
            Err(e) => e,
            Ok(_) => panic!("rejected wrapper digest {old_ver} ({rejected_sha}) must fail"),
        };
        assert_eq!(err.code(), "UNPINNED_WRAPPER");
        assert!(matches!(err, EvalError::UnpinnedWrapper(_)));
    }
    let bad_ver = match check_wrapper_digest(PINNED_WRAPPERS[0].1, Some("wrong-ver")) {
        Err(e) => e,
        Ok(_) => panic!("mismatched --wrapper-version must fail"),
    };
    assert_eq!(bad_ver.code(), "UNPINNED_WRAPPER");

    let tmp = fresh_tmp_dir("pinned_wrappers");
    let r3_copy = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, all_clubbot_seats());
    let alt_wrap = tmp.join("unpinned.md");
    must_ok(
        fs::write(&alt_wrap, b"# Unpinned brief\n"),
        "write alt wrapper",
    );

    // Wrong wrapper bytes AND mismatched --wrapper-version both fail cleanly in play & branch.
    for (label, wrap_path, ver) in [
        ("unpinned_bytes", alt_wrap.clone(), None),
        (
            "wrong_version",
            r3_copy.clone(),
            Some("v2_seat_brief_wrong"),
        ),
    ] {
        let play_err = match run_play_with_transport(
            &PlayOptions {
                game_dir: tmp.join(format!("g_{label}")),
                seats_path: seats_path.clone(),
                wrapper_path: wrap_path.clone(),
                wrapper_version: ver.map(str::to_owned),
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 11,
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
            Ok(_) => panic!("{label}: expected UNPINNED_WRAPPER in play"),
        };
        assert_eq!(play_err.code(), "UNPINNED_WRAPPER");
    }

    // Seed a valid game so branch setup checks cannot mask wrapper check.
    let ok_dir = tmp.join("ok_clubbot_game");
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: ok_dir.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: r3_copy,
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 12,
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
        ),
        "seed ok ClubBot game",
    );
    let branch_err = match run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(ok_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path: alt_wrap,
            serving_manifest_path: None,
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("branch_unpinned"),
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
        Ok(_) => panic!("unpinned wrapper must fail branch"),
    };
    assert_eq!(branch_err.code(), "UNPINNED_WRAPPER");

    // Verify no wrapper prose from v2_seat_brief_2026_11_21_r3.md is embedded inside Rust sources.
    let r3_text = String::from_utf8_lossy(&r3_bytes);
    let distinctive_lines: Vec<&str> = r3_text
        .lines()
        .map(str::trim)
        .filter(|line| line.len() >= 80)
        .collect();
    assert!(
        !distinctive_lines.is_empty(),
        "r3 brief must yield distinctive lines for source check"
    );
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for sub in ["src", "tests"] {
        for entry in must_ok(fs::read_dir(manifest_dir.join(sub)), "read dir") {
            let p = must_ok(entry, "dir entry").path();
            if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                let src_text = must_ok(fs::read_to_string(&p), "read rs source");
                for frag in &distinctive_lines {
                    assert!(
                        !src_text.contains(frag),
                        "wrapper brief prose leaked into {}",
                        p.display()
                    );
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&tmp);
}

struct RecordedRequest {
    body_json: Value,
    raw_body: String,
}

struct TelemetryThreeArmTransport {
    requests: Mutex<Vec<RecordedRequest>>,
    script_step: AtomicUsize,
}

impl TelemetryThreeArmTransport {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            script_step: AtomicUsize::new(0),
        }
    }

    fn take_requests(&self) -> Vec<RecordedRequest> {
        let mut g = match self.requests.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        std::mem::take(&mut *g)
    }
}

impl Transport for TelemetryThreeArmTransport {
    fn count_tokens(
        &self,
        _url: &str,
        _model: &str,
        _content: &str,
        _timeout: Duration,
    ) -> Result<usize, TransportError> {
        Ok(8)
    }

    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        let is_act = body.contains(eval_wording_slot("eval_act_instruction"));
        let parsed: Value = serde_json::from_str(body)
            .map_err(|e| TransportError::Network(format!("bad json body: {e}")))?;
        let has_tools = parsed.get("tools").is_some();
        let tool_name = parsed
            .get("tools")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|t| t.get("function"))
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_owned();
        let tool_msgs_count = parsed
            .get("messages")
            .and_then(|m| m.as_array())
            .map_or(0, |arr| {
                arr.iter()
                    .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
                    .count()
            });

        if let Ok(mut g) = self.requests.lock() {
            g.push(RecordedRequest {
                body_json: parsed,
                raw_body: body.to_owned(),
            });
        }

        let _step = self.script_step.fetch_add(1, Ordering::SeqCst);
        let logprob_cands: Vec<Value> = (0..5)
            .map(|k| {
                json!({
                    "token": format!("t{k}"),
                    "id": 500 + k,
                    "logprob": -0.05 * f64::from(k + 1)
                })
            })
            .collect();

        let bundle_arg = |amt: u64| {
            json!({
                "market_actions": [],
                "bundle": { "debt_repayment_cred": amt }
            })
            .to_string()
        };

        let message_obj = if has_tools && tool_msgs_count < 3 {
            let amt = u64::try_from(tool_msgs_count + 1).unwrap_or(1) * 5;
            json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": format!("call_{}", tool_msgs_count + 1),
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": bundle_arg(amt)
                    }
                }]
            })
        } else if is_act {
            let content = json!({
                "messages": [],
                "market_actions": [{
                    "action_kind": "town_import",
                    "good": { "asset_kind": "raw", "good_index": 0 },
                    "amount": 5
                }],
                "bundle": {},
                "notes": "final_turn_notes"
            })
            .to_string();
            json!({
                "role": "assistant",
                "content": content
            })
        } else {
            let content = json!({
                "messages": [],
                "market_actions": [],
                "bundle": {},
                "notes": "talk_clean_notes"
            })
            .to_string();
            json!({
                "role": "assistant",
                "content": content
            })
        };

        let resp = json!({
            "id": "mock-resp",
            "model": "Qwen/Qwen3-32B",
            "choices": [{
                "index": 0,
                "message": message_obj,
                "logprobs": {
                    "content": [{
                        "token": "{",
                        "id": 11,
                        "logprob": -0.01,
                        "top_logprobs": logprob_cands
                    }]
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 25,
                "total_tokens": 145
            }
        });

        Ok(RawHttpResponse {
            status: 200,
            body: resp.to_string(),
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
        Ok((100_u64..220).collect())
    }
}

#[test]
fn control_arm_branch_and_telemetry_item20_plus_token_logprobs() {
    let tmp = fresh_tmp_dir("ctrl_branch_telem");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let src_dir = tmp.join("src_game");
    let play_transport = TelemetryThreeArmTransport::new();
    let play_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: src_dir.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(serving_path.clone()),
                arm: EvalArm::Off,
                table_id: 41,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &play_transport,
            None,
        ),
        "play source game",
    );
    assert_eq!(play_manifest.played_turns, 1);

    let act_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(src_dir.join("decisions/day-01-act-Foundry.json")),
            "read play act decision",
        )),
        "parse play act decision",
    );
    assert!(act_dec.prompt_tokens > 0);
    assert!(act_dec.completion_tokens > 0);
    assert!(act_dec.wall_secs.is_some());
    assert!(!act_dec.parse_failure);
    assert!(
        act_dec.cleared_non_empty_actions,
        "expected cleared_non_empty_actions=true, got act_dec={act_dec:?}"
    );
    assert!(act_dec.club_bot_baseline.is_some());
    assert!(!act_dec.prompt_token_ids_sha256s.is_empty());
    assert_eq!(
        act_dec.prompt_token_ids_source.as_deref(),
        Some("server_tokenize")
    );
    assert_eq!(
        act_dec.prompt_token_ids_template_check.as_deref(),
        Some("double_apply_template_byte_equal")
    );
    assert!(!act_dec.reply_token_ids.is_empty());
    assert!(!act_dec.reply_top_logprobs.is_empty());
    // Exact top-5 logprob entries per generated token.
    assert_eq!(act_dec.reply_top_logprobs[0][0].top_logprobs.len(), 5);

    // 3-arm branch: Off, Control, On.
    let branch_out = tmp.join("branch_3arms");
    let branch_transport = TelemetryThreeArmTransport::new();
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(src_dir.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                serving_manifest_path: Some(serving_path.clone()),
                arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
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
        "branch 3 arms",
    );
    assert_eq!(outcomes.len(), 3);

    // Requests order: Off (1 req), Control (4 reqs: 3 tool rounds + 1 final no-tools), On (4 reqs).
    let reqs = branch_transport.take_requests();
    assert_eq!(reqs.len(), 1 + 4 + 4);

    for (idx, req) in reqs.iter().enumerate() {
        assert_eq!(
            req.body_json.get("cache_prompt"),
            Some(&json!(false)),
            "Local eval request {idx} must emit cache_prompt: false"
        );
    }

    let ctrl_reqs = &reqs[1..5];
    let on_reqs = &reqs[5..9];
    assert!(ctrl_reqs[0].body_json.get("tools").is_some());
    assert!(ctrl_reqs[2].body_json.get("tools").is_some());
    assert!(
        ctrl_reqs[3].body_json.get("tools").is_none(),
        "Control arm 4th request after 3 calls must omit `tools`"
    );
    assert!(
        on_reqs[3].body_json.get("tools").is_none(),
        "On arm 4th request after 3 calls must omit `tools`"
    );

    // Verify bare fixed reply on `role: "tool"` and zero preview leakage in control arm.
    let last_ctrl_msgs = must_some(
        ctrl_reqs[3]
            .body_json
            .get("messages")
            .and_then(|v| v.as_array()),
        "control final messages",
    );
    let ctrl_tool_contents: Vec<&str> = last_ctrl_msgs
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect();
    assert_eq!(ctrl_tool_contents.len(), 3);
    for c in ctrl_tool_contents {
        assert_eq!(
            c,
            "Draft recorded. The orders are allowed; nothing was simulated or submitted."
        );
    }
    for req in ctrl_reqs {
        for forbidden in [
            "preview_turn_orders",
            "preview_seat_bundle",
            "ending_cred",
            "after_sweep",
            "INVALID_ORDER",
        ] {
            assert!(
                !req.raw_body.contains(forbidden),
                "control request leaked preview string '{forbidden}'"
            );
        }
    }

    // Off = 0 tools, Control = 3 tools, On = 3 tools; matching intermediate bundle hashes!
    let off_dec = &outcomes[0].decision;
    let ctrl_dec = &outcomes[1].decision;
    let on_dec = &outcomes[2].decision;
    assert_eq!(off_dec.arm, EvalArm::Off);
    assert_eq!(ctrl_dec.arm, EvalArm::Control);
    assert_eq!(on_dec.arm, EvalArm::On);
    assert_eq!(off_dec.tool_calls_used, 0);
    assert_eq!(ctrl_dec.tool_calls_used, 3);
    assert_eq!(on_dec.tool_calls_used, 3);
    assert!(off_dec.intermediate_bundle_sha256s.is_empty());
    assert_eq!(ctrl_dec.intermediate_bundle_sha256s.len(), 3);
    assert_eq!(
        ctrl_dec.intermediate_bundle_sha256s,
        on_dec.intermediate_bundle_sha256s
    );
    assert!(ctrl_dec.prompt_tokens > 0 && ctrl_dec.completion_tokens > 0);
    assert!(ctrl_dec.wall_secs.is_some());
    assert!(ctrl_dec.club_bot_baseline.is_some());
    assert!(!ctrl_dec.prompt_token_ids_sha256s.is_empty());
    assert!(!ctrl_dec.reply_token_ids.is_empty());
    assert!(!ctrl_dec.reply_top_logprobs.is_empty());

    // Verify BranchManifest and per-arm EvalManifest metadata + wording digests + git SHA.
    let branch_manifest: BranchManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(branch_out.join("manifest.json")),
            "read branch manifest.json",
        )),
        "parse branch manifest.json",
    );
    assert!(!branch_manifest.crate_git_sha.is_empty());
    assert_eq!(
        branch_manifest.arms,
        vec![EvalArm::Off, EvalArm::Control, EvalArm::On]
    );
    assert!(!branch_manifest.voided);
    let digests: WordingArmDigests = must_some(branch_manifest.wording_digests, "wording_digests");
    assert_eq!(digests.neutral_arm, "v2_neutral_2026_11_15_r6");
    assert_eq!(digests.control_arm, "v2_control_2026_11_22_r2");
    assert_eq!(digests.preview_arm, "v2_preview_2026_11_20_r2");
    assert_eq!(digests.eval_reply_arm, "v2_eval_reply_2026_11_21_r1");
    assert_eq!(digests.wrapper_version, "v2_seat_brief_2026_11_21_r3");
    assert_eq!(digests.neutral_wording_sha256.len(), 64);
    assert_eq!(digests.control_wording_sha256.len(), 64);
    assert_eq!(digests.preview_wording_sha256.len(), 64);
    assert_eq!(digests.eval_reply_wording_sha256.len(), 64);
    assert_eq!(digests.response_schema_sha256.len(), 64);

    let arm_ctrl_manifest: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(branch_out.join("control/manifest.json")),
            "read control arm manifest",
        )),
        "parse control arm manifest",
    );
    assert_eq!(arm_ctrl_manifest.arm, EvalArm::Control);
    assert_eq!(
        arm_ctrl_manifest.relay_wording_arm,
        "v2_control_2026_11_22_r2"
    );

    // Test refused market actions, refused bundle legs, cleared_non_empty_actions == false, and parse_failure == true!
    struct MixedFailureTransport {
        call: AtomicUsize,
    }
    impl Transport for MixedFailureTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(8)
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let step = self.call.fetch_add(1, Ordering::SeqCst);
            let content = match step {
                // Turn 1 Talk -> clean
                0 => json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string(),
                // Turn 1 Act -> valid schema, invalid market action + invalid 2-leg bundle -> refused!
                1 => json!({
                    "messages": [],
                    "market_actions": [{
                        "action_kind": "cancel_offer",
                        "offer_id": 999999_u64
                    }],
                    "bundle": {
                        "debt_repayment_cred": 10,
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
                    "notes": "refused orders"
                })
                .to_string(),
                // Turn 2 Talk -> non-JSON parse failure!
                2 => "{not valid json".to_owned(),
                // Turn 2 Act -> clean empty
                _ => json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok2" })
                    .to_string(),
            };
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_with_telemetry(
                    &format!("mix-{step}"),
                    "Qwen/Qwen3-32B",
                    &content,
                    50,
                    10,
                ),
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
            Ok((100_u64..150).collect())
        }
    }

    let mix_dir = tmp.join("mix_telemetry_game");
    let mix_t = MixedFailureTransport {
        call: AtomicUsize::new(0),
    };
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: mix_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: Some(serving_path),
                arm: EvalArm::Control,
                table_id: 42,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &mix_t,
            None,
        ),
        "mixed telemetry game",
    );

    let d1_act: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(mix_dir.join("decisions/day-01-act-Foundry.json")),
            "read mix d1 act",
        )),
        "parse mix d1 act",
    );
    assert!(
        !d1_act.parse_failure,
        "unexpected parse failure: {d1_act:?}"
    );
    assert_eq!(d1_act.refused_market_actions, 1);
    assert_eq!(d1_act.refused_bundle_legs, 2);
    assert!(!d1_act.cleared_non_empty_actions);

    let d2_talk: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(mix_dir.join("decisions/day-02-talk-Foundry.json")),
            "read mix d2 talk",
        )),
        "parse mix d2 talk",
    );
    assert!(d2_talk.parse_failure);
    assert_eq!(
        d2_talk.refusal_code.as_deref(),
        Some("REPLY_SCHEMA_INVALID")
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn serving_manifest_andons_provenance_vllm_and_subprocess_env_snapshot() {
    let tmp = fresh_tmp_dir("serving_andons");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let local_seats = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let clubbot_dir = tmp.join("clubbot_seats.json");
    must_ok(
        fs::write(
            &clubbot_dir,
            must_ok(
                serde_json::to_string_pretty(&all_clubbot_seats()),
                "ser clubbot",
            ),
        ),
        "write clubbot seats",
    );

    // 1. Missing --serving-manifest on Model seat run fails with MISSING_SERVING_MANIFEST.
    let miss_err = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("miss_serving"),
            seats_path: local_seats.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: None,
            arm: EvalArm::Off,
            table_id: 50,
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
        Ok(_) => panic!("missing serving manifest must fail"),
    };
    assert_eq!(miss_err.code(), "MISSING_SERVING_MANIFEST");

    // 2. Startup Andons across all bad serving configs:
    let mut bad_cache_1 = base_llama_serving_json();
    bad_cache_1["cache_settings"]["per_request_cache_prompt"] = json!(true);
    let mut bad_cache_2 = base_llama_serving_json();
    bad_cache_2["cache_settings"]["cache_ram_mib"] = json!(512);
    let mut bad_cache_3 = base_llama_serving_json();
    bad_cache_3["cache_settings"]["cache_idle_slots"] = json!(true);
    let mut bad_vllm_apc = base_llama_serving_json();
    bad_vllm_apc["engine"] = json!("vllm");
    bad_vllm_apc["cache_settings"] = json!({
        "enable_prefix_caching": true,
        "provenance": "server_reported"
    });
    let mut bad_slots = base_llama_serving_json();
    bad_slots["server_settings"]["parallel_slots"] = json!(2);
    let mut bad_model_sha = base_llama_serving_json();
    bad_model_sha["model"]["sha256"] = json!("short");
    let mut bad_gpu = base_llama_serving_json();
    bad_gpu["machine"]["gpu_name"] = json!("   ");
    let mut bad_uuid = base_llama_serving_json();
    if let Some(m) = bad_uuid["machine"].as_object_mut() {
        m.remove("gpu_uuid");
    }
    bad_uuid["machine"]["gpu_uuid_sha256"] = json!("not-hex");

    for (tag, val, expected_code) in [
        ("per_req_cache", bad_cache_1, "PREFIX_CACHE_ENABLED"),
        ("cache_ram", bad_cache_2, "PREFIX_CACHE_ENABLED"),
        ("idle_slots", bad_cache_3, "PREFIX_CACHE_ENABLED"),
        ("vllm_apc", bad_vllm_apc, "PREFIX_CACHE_ENABLED"),
        ("slots_2", bad_slots, "PARALLEL_SLOTS_EXCEEDED"),
        ("bad_model_sha", bad_model_sha, "MISSING_MODEL_SHA256"),
        ("missing_gpu", bad_gpu, "MISSING_GPU_NAME"),
        ("bad_gpu_uuid", bad_uuid, "INVALID_GPU_UUID_SHA256"),
    ] {
        let p = write_serving_manifest_json(&tmp, &format!("bad_{tag}.json"), &val);
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: tmp.join(format!("g_{tag}")),
                seats_path: local_seats.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(p),
                arm: EvalArm::Off,
                table_id: 51,
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
            Ok(_) => panic!("{tag} must fail startup Andon {expected_code}"),
        };
        assert_eq!(err.code(), expected_code, "tag={tag}");
    }

    // 3. Valid vLLM serving manifest records `determinism_status == "not shown bit-exact"`,
    // mixed provenance tags, and hashes raw gpu_uuid without ever writing it to disk.
    let mut vllm_ok = base_llama_serving_json();
    vllm_ok["engine"] = json!("vllm");
    vllm_ok["server_version"] = json!("0.8.5");
    vllm_ok["cache_settings"] = json!({
        "enable_prefix_caching": false,
        "provenance": "server_reported"
    });
    let vllm_path = write_serving_manifest_json(&tmp, "vllm_ok.json", &vllm_ok);
    let vllm_game = tmp.join("vllm_game");
    let vllm_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: vllm_game.clone(),
                seats_path: clubbot_dir.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(vllm_path),
                arm: EvalArm::Off,
                table_id: 52,
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
        ),
        "valid vllm serving manifest run",
    );
    let srv = must_some(vllm_manifest.serving, "vllm serving record");
    assert_eq!(srv.determinism_status, "not shown bit-exact");
    assert_eq!(srv.engine.provenance, MetadataProvenance::ServerReported);
    assert_eq!(srv.model.provenance, MetadataProvenance::OperatorSupplied);
    assert_eq!(
        srv.machine.gpu_uuid_sha256,
        sha256_hex(b"GPU-RAW-SECRET-UUID-99887766")
    );
    let raw_manifest = must_ok(
        fs::read_to_string(vllm_game.join("manifest.json")),
        "read vllm manifest",
    );
    assert!(!raw_manifest.contains("GPU-RAW-SECRET-UUID-99887766"));

    // 4. Child-process `league-eval` binary CLI verifies `serving_env_snapshot` strips secrets
    // and replaces user home directories with `<home>`.
    let llama_ok_path =
        write_serving_manifest_json(&tmp, "llama_ok.json", &base_llama_serving_json());
    let sub_game = tmp.join("sub_env_game");
    let fake_home = tmp.join("sandbox_home");
    must_ok(fs::create_dir_all(&fake_home), "create fake home");
    let fake_home_str = fake_home.to_string_lossy().into_owned();
    let cache_dir_str = fake_home
        .join("llama_cache_dir")
        .to_string_lossy()
        .into_owned();
    let sub_game_str = sub_game.to_string_lossy().into_owned();
    let clubbot_dir_str = clubbot_dir.to_string_lossy().into_owned();
    let wrapper_path_str = wrapper_path.to_string_lossy().into_owned();
    let llama_ok_path_str = llama_ok_path.to_string_lossy().into_owned();
    let out = run_cli_via_test_seam(
        &tmp,
        "env_snapshot",
        &[
            "play",
            "--game-dir",
            &sub_game_str,
            "--seats",
            &clubbot_dir_str,
            "--wrapper",
            &wrapper_path_str,
            "--serving-manifest",
            &llama_ok_path_str,
            "--arm",
            "control",
            "--turns",
            "1",
            "--seed",
            "7",
        ],
        &[
            ("HOME", &fake_home_str),
            ("USERPROFILE", &fake_home_str),
            ("LLAMA_CACHE", &cache_dir_str),
            ("CUDA_VISIBLE_DEVICES", "0"),
            ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "0"),
            ("VLLM_WORKER_MULTIPROC_METHOD", "spawn"),
            ("VLLM_API_KEY", "sk-secret-vllm-key"),
            ("LLAMA_HF_TOKEN", "hf_secret_token"),
            ("CUDA_SECRET_KEY", "cuda-secret-val"),
            ("GGML_PASS_PHRASE", "ggml-secret-pass"),
            ("VLLM_AUTH_HEADER", "Bearer secret-auth"),
        ],
        &[
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sub_manifest_raw = must_ok(
        fs::read_to_string(sub_game.join("manifest.json")),
        "read child manifest",
    );
    let sub_manifest: EvalManifest = must_ok(
        serde_json::from_str(&sub_manifest_raw),
        "parse child manifest",
    );
    let expected_cuda_sha = sha256_hex(b"0");
    assert_eq!(
        sub_manifest
            .serving_env_snapshot
            .get("CUDA_VISIBLE_DEVICES_sha256")
            .map(String::as_str),
        Some(expected_cuda_sha.as_str())
    );
    assert!(
        !sub_manifest
            .serving_env_snapshot
            .contains_key("CUDA_VISIBLE_DEVICES")
    );
    assert_eq!(
        sub_manifest
            .serving_env_snapshot
            .get("VLLM_WORKER_MULTIPROC_METHOD")
            .map(String::as_str),
        Some("spawn")
    );
    let llama_cache_recorded = must_some(
        sub_manifest
            .serving_env_snapshot
            .get("LLAMA_CACHE")
            .map(String::as_str),
        "LLAMA_CACHE recorded",
    );
    assert!(
        llama_cache_recorded.starts_with("<home>"),
        "expected <home> replacement in '{llama_cache_recorded}'"
    );
    for secret_needle in [
        "VLLM_API_KEY",
        "LLAMA_HF_TOKEN",
        "CUDA_SECRET_KEY",
        "GGML_PASS_PHRASE",
        "VLLM_AUTH_HEADER",
        "sk-secret-vllm-key",
        "hf_secret_token",
        "cuda-secret-val",
        "ggml-secret-pass",
        "Bearer secret-auth",
        "sandbox_home",
    ] {
        assert!(
            !sub_manifest_raw.contains(secret_needle),
            "forbidden secret/home string '{secret_needle}' leaked into manifest.json: {sub_manifest_raw}"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

struct FixedReplyTransport {
    content: String,
}

impl Transport for FixedReplyTransport {
    fn count_tokens(
        &self,
        _url: &str,
        _model: &str,
        _content: &str,
        _timeout: Duration,
    ) -> Result<usize, TransportError> {
        Ok(8)
    }

    fn post_chat_completions(
        &self,
        _url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        Ok(RawHttpResponse {
            status: 200,
            body: mock_chat_with_telemetry("regent-mock", "Qwen/Qwen3-32B", &self.content, 40, 10),
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
        Ok((100_u64..140).collect())
    }
}

#[test]
fn regent_all_refused_four_turns_never_triggers_regent() {
    let tmp = fresh_tmp_dir("regent_a");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let refused_dir = tmp.join("regent_a_all_refused");
    let refused_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: refused_dir.clone(),
                seats_path,
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(serving_path),
                arm: EvalArm::Off,
                table_id: 60,
                seed: 7,
                turns: 4,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &FixedReplyTransport {
                content: "{not_valid_json".to_owned(),
            },
            None,
        ),
        "4-turn all-refused run must not fail or trigger Regent",
    );
    assert_eq!(refused_manifest.played_turns, 4);
    assert!(!refused_manifest.voided);
    assert!(!refused_manifest.seats[0].has_regent);
    // Seats 1..=3 are AiSyndicate ClubBot seats (regent: None per Club-mode table rules).
    assert!(!refused_manifest.seats[1].has_regent);
    assert!(!refused_manifest.seats[2].has_regent);
    assert!(!refused_manifest.seats[3].has_regent);
    assert_eq!(refused_manifest.seat_regents.get("Foundry"), Some(&false));

    // When Seat 0 is ClubBot instead of Model, Seat 0 keeps the Club-mode Standing Regent (`true`).
    let club_seats_path = write_seats(&tmp.join("club_check"), all_clubbot_seats());
    let club_dir = tmp.join("club_regent_check");
    let club_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: club_dir,
                seats_path: club_seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 63,
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
        ),
        "all-ClubBot play check",
    );
    assert!(club_manifest.seats[0].has_regent);
    assert_eq!(club_manifest.seat_regents.get("Foundry"), Some(&true));
    for audit in &refused_manifest.turn_regent_audit {
        assert!(!audit.converted_to_ai.contains(&0));
        assert!(!audit.regent_acted_seats.contains(&0));
    }
    let host_a = must_ok(
        EvalHost::read_game_save_and_seal(&refused_dir, TableId(60)),
        "read game A",
    );
    assert!(host_a.public_spec().seats[0].regent.is_none());
    let obs_a = must_ok(host_a.observe_seat(Seat(0)), "obs A seat 0");
    assert_eq!(
        must_ok(serde_json::to_value(obs_a.seats[0].identity), "ser id A"),
        json!("Human")
    );
    assert!(
        obs_a.seats[0].refined.iter().all(|&u| u == 0),
        "all-refused model seat must have zero converter output"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn regent_valid_passive_four_turns_never_triggers_regent() {
    let tmp = fresh_tmp_dir("regent_b");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let passive_act = json!({
        "messages": [],
        "market_actions": [{
            "action_kind": "post_offer",
            "side": "buy",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 1,
            "price_per_unit_cred": 1,
            "domain_id": null,
            "is_hybrid": false
        }],
        "bundle": { "debt_repayment_cred": 0 },
        "notes": "valid passive turn"
    })
    .to_string();

    struct TalkCleanPassiveActTransport {
        act_json: String,
    }
    impl Transport for TalkCleanPassiveActTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(8)
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let content = if body.contains(eval_wording_slot("eval_act_instruction")) {
                self.act_json.clone()
            } else {
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "clean talk" })
                    .to_string()
            };
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_with_telemetry("passive-mock", "Qwen/Qwen3-32B", &content, 40, 10),
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
            Ok((100_u64..140).collect())
        }
    }

    let passive_dir = tmp.join("regent_b_valid_passive");
    let passive_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: passive_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: Some(serving_path),
                arm: EvalArm::Off,
                table_id: 61,
                seed: 7,
                turns: 4,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &TalkCleanPassiveActTransport {
                act_json: passive_act,
            },
            None,
        ),
        "4-turn valid passive run must not trigger Regent",
    );
    assert_eq!(passive_manifest.played_turns, 4);
    assert!(!passive_manifest.voided);
    assert!(!passive_manifest.seats[0].has_regent);
    for day in 1..=4_u8 {
        let dec: EvalDecisionRecord = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(
                    passive_dir.join(format!("decisions/day-{day:02}-act-Foundry.json")),
                ),
                "read passive act dec",
            )),
            "parse passive act dec",
        );
        assert!(
            dec.cleared_non_empty_actions,
            "day {day} passive offer must clear: dec={dec:?}"
        );
        assert_eq!(dec.refused_market_actions, 0);
    }
    for audit in &passive_manifest.turn_regent_audit {
        assert!(!audit.converted_to_ai.contains(&0));
        assert!(!audit.regent_acted_seats.contains(&0));
    }
    let host_b = must_ok(
        EvalHost::read_game_save_and_seal(&passive_dir, TableId(61)),
        "read game B",
    );
    let obs_b = must_ok(host_b.observe_seat(Seat(0)), "obs B seat 0");
    assert_eq!(
        must_ok(serde_json::to_value(obs_b.seats[0].identity), "ser id B"),
        json!("Human")
    );
    assert!(obs_b.seats[0].refined.iter().all(|&u| u == 0));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn regent_andon_voids_game_and_branch_and_counts_by_arm() {
    let tmp = fresh_tmp_dir("regent_andon");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let seeded_dir = tmp.join("regent_c_andon_play");
    must_ok(fs::create_dir_all(&seeded_dir), "mkdir seeded_dir");
    let seeded_host = must_ok(
        EvalHost::from_standard_setup_with_model_seats(TableId(62), 4, 7, 12345, &[false; 4]),
        "create table with Regent on seat 0",
    );
    let _ = must_ok(
        league_eval::persist_eval_game_state(&seeded_host, &seeded_dir),
        "persist seeded host",
    );
    struct CountingRejectTransport {
        calls: AtomicUsize,
    }
    impl Transport for CountingRejectTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(8)
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_with_telemetry(
                    "regent-andon-unused",
                    "Qwen/Qwen3-32B",
                    "{not_valid_json",
                    40,
                    10,
                ),
            })
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(add_special && parse_special);
            Ok((100_u64..140).collect())
        }
    }

    let play_counter = CountingRejectTransport {
        calls: AtomicUsize::new(0),
    };
    let andon_err = match run_play_with_transport(
        &PlayOptions {
            game_dir: seeded_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Off,
            table_id: 62,
            seed: 7,
            turns: 4,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &play_counter,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("table with Regent on Model seat must refuse at startup"),
    };
    assert_eq!(andon_err.code(), "MODEL_SEAT_REGENT_INTERVENTION");
    assert_eq!(
        play_counter.calls.load(Ordering::SeqCst),
        0,
        "startup Regent refusal must make zero model or tokenizer calls"
    );

    let club_source_dir = tmp.join("regent_c_club_source");
    let club_seats_path = write_seats(&tmp.join("regent_c_club_seats"), all_clubbot_seats());
    let _club_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: club_source_dir.clone(),
                seats_path: club_seats_path,
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 64,
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
        ),
        "all-ClubBot 4-turn game to produce day-04-pre-act checkpoint with Seat 0 Regent",
    );

    let branch_void_out = tmp.join("regent_c_andon_branch");
    let branch_counter = CountingRejectTransport {
        calls: AtomicUsize::new(0),
    };
    let branch_void_err = match run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: Some(club_source_dir.join("checkpoints/day-04-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Off],
            out_dir: branch_void_out,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &branch_counter,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("branch with Regent on Model seat must refuse at startup"),
    };
    assert_eq!(branch_void_err.code(), "MODEL_SEAT_REGENT_INTERVENTION");
    assert_eq!(
        branch_counter.calls.load(Ordering::SeqCst),
        0,
        "branch startup Regent refusal must make zero model or tokenizer calls"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn missing_reply_telemetry_fields_fail_closed() {
    struct MissingFieldTransport {
        body: String,
    }
    impl Transport for MissingFieldTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(8)
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            Ok(RawHttpResponse {
                status: 200,
                body: self.body.clone(),
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

    let tmp = fresh_tmp_dir("missing_telemetry");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let clean_talk =
        json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" }).to_string();
    let top_five: Vec<Value> = (0_u64..5)
        .map(|k| {
            json!({
                "token": format!("t{k}"),
                "id": 200 + k,
                "logprob": -0.05 * ((k + 1) as f64)
            })
        })
        .collect();

    // Case 0: missing `choices[0].logprobs.content[].id` fails closed even if top-level `token_ids` exists.
    let no_reply_ids = json!({
        "id": "telem-no-reply-ids",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
            "token_ids": [11],
            "logprobs": {
                "content": [{
                    "token": "{",
                    "logprob": -0.01,
                    "top_logprobs": top_five
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
    .to_string();
    // Case 1: empty `top_logprobs`.
    let no_top_logprobs = json!({
        "id": "telem-no-top-lp",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "logprob": -0.01,
                    "top_logprobs": []
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
    .to_string();
    // Case 2: missing `usage.prompt_tokens`.
    let no_usage_prompt = json!({
        "id": "telem-no-usage-prompt",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "logprob": -0.01,
                    "top_logprobs": top_five
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": { "completion_tokens": 5, "total_tokens": 15 }
    })
    .to_string();

    for (idx, body) in [no_reply_ids, no_top_logprobs, no_usage_prompt]
        .into_iter()
        .enumerate()
    {
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: tmp.join(format!("telem_case_{idx}")),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(serving_path.clone()),
                arm: EvalArm::Off,
                table_id: 210 + idx as u64,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &MissingFieldTransport { body },
            None,
        ) {
            Err(e) => e,
            Ok(_) => panic!("case {idx} lacking reply telemetry must fail closed"),
        };
        assert_eq!(
            err.code(),
            "MISSING_REPLY_TELEMETRY",
            "case {idx} got unexpected error: {err}"
        );
    }

    // Happy path with real `llama-server` shape (`id` inside `logprobs.content[]`, no `prompt_token_ids`, no top-level `token_ids`).
    let llama_style = json!({
        "id": "telem-llama-id",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "logprob": -0.01,
                    "top_logprobs": top_five
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
    .to_string();
    let ok_game = tmp.join("telem_llama_id_ok");
    let _m = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: ok_game.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: Some(serving_path),
                arm: EvalArm::Off,
                table_id: 215,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &MissingFieldTransport { body: llama_style },
            None,
        ),
        "llama-server logprobs.content[].id style must satisfy reply_token_ids",
    );
    let rec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(ok_game.join("decisions/day-01-talk-Foundry.json")),
            "read telemetry decision",
        )),
        "parse telemetry decision",
    );
    assert_eq!(rec.reply_token_ids, vec![vec![11]]);
    assert_eq!(
        rec.prompt_token_ids_source.as_deref(),
        Some("server_tokenize")
    );
    assert_eq!(
        rec.prompt_token_ids_template_check.as_deref(),
        Some("double_apply_template_byte_equal")
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn subprocess_cuda_visible_devices_gpu_and_mig_never_verbatim() {
    fn collect_all_files(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
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

    let tmp = fresh_tmp_dir("sub_cuda_mig");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let club_seats_path = write_seats(&tmp, all_clubbot_seats());
    let llama_ok_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());
    let sub_game = tmp.join("sub_gpu_mig_game");
    let fake_home = tmp.join("sandbox_home");
    must_ok(fs::create_dir_all(&fake_home), "create fake home");

    let gpu_uuid = [
        "GPU",
        "-",
        "abcd1234",
        "-",
        "1111",
        "-",
        "2222",
        "-",
        "3333",
        "-",
        "cafe4567dead",
    ]
    .concat();
    let mig_uuid = ["MIG", "-", "9876fedc", "-", "4444", "-", "0", "-", "2"].concat();

    let fake_home_str = fake_home.to_string_lossy().into_owned();
    let sub_game_str = sub_game.to_string_lossy().into_owned();
    let club_seats_str = club_seats_path.to_string_lossy().into_owned();
    let wrapper_path_str = wrapper_path.to_string_lossy().into_owned();
    let llama_ok_path_str = llama_ok_path.to_string_lossy().into_owned();
    let out = run_cli_via_test_seam(
        &tmp,
        "gpu_mig_env",
        &[
            "play",
            "--game-dir",
            &sub_game_str,
            "--seats",
            &club_seats_str,
            "--wrapper",
            &wrapper_path_str,
            "--serving-manifest",
            &llama_ok_path_str,
            "--arm",
            "control",
            "--turns",
            "1",
            "--seed",
            "7",
        ],
        &[
            ("HOME", &fake_home_str),
            ("USERPROFILE", &fake_home_str),
            ("CUDA_VISIBLE_DEVICES", &gpu_uuid),
            ("VLLM_MIG_UUID", &mig_uuid),
        ],
        &[
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let sub_manifest: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(sub_game.join("manifest.json")),
            "read child manifest",
        )),
        "parse child manifest",
    );
    assert_eq!(
        sub_manifest
            .serving_env_snapshot
            .get("CUDA_VISIBLE_DEVICES_sha256")
            .map(String::as_str),
        Some(sha256_hex(gpu_uuid.as_bytes()).as_str())
    );
    assert_eq!(
        sub_manifest
            .serving_env_snapshot
            .get("VLLM_MIG_UUID_sha256")
            .map(String::as_str),
        Some(sha256_hex(mig_uuid.as_bytes()).as_str())
    );
    assert!(
        !sub_manifest
            .serving_env_snapshot
            .contains_key("CUDA_VISIBLE_DEVICES")
    );
    assert!(
        !sub_manifest
            .serving_env_snapshot
            .contains_key("VLLM_MIG_UUID")
    );

    let mut files = Vec::new();
    collect_all_files(&sub_game, &mut files);
    assert!(!files.is_empty());
    for file in files {
        let content = fs::read_to_string(&file).unwrap_or_default();
        assert!(
            !content.contains(&gpu_uuid) && !content.contains("cafe4567dead"),
            "raw GPU UUID leaked into {}",
            file.display()
        );
        assert!(
            !content.contains(&mig_uuid) && !content.contains("9876fedc"),
            "raw MIG ID leaked into {}",
            file.display()
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn resume_refuses_mismatched_serving_manifest() {
    let tmp = fresh_tmp_dir("resume_manifest");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let m1_path = write_serving_manifest_json(&tmp, "serving_a.json", &base_llama_serving_json());
    let mut altered = base_llama_serving_json();
    altered["model"]["sha256"] =
        json!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    let m2_path = write_serving_manifest_json(&tmp, "serving_b.json", &altered);

    struct CleanTurnTransport;
    impl Transport for CleanTurnTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(8)
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let content =
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: mock_chat_with_telemetry("s4-ok", "Qwen/Qwen3-32B", &content, 40, 10),
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
            Ok((100_u64..140).collect())
        }
    }

    let game_dir = tmp.join("s4_game");
    let _first = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(m1_path),
                arm: EvalArm::Off,
                table_id: 220,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &CleanTurnTransport,
            None,
        ),
        "turn 1 play",
    );

    let err = match run_play_with_transport(
        &PlayOptions {
            game_dir,
            seats_path,
            wrapper_path,
            wrapper_version: None,
            serving_manifest_path: Some(m2_path),
            arm: EvalArm::Off,
            table_id: 220,
            seed: 7,
            turns: 2,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &CleanTurnTransport,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("resuming with different --serving-manifest must refuse"),
    };
    assert_eq!(err.code(), "INCOMPATIBLE_EVAL_SETUP");
    assert!(
        err.to_string().contains("--serving-manifest"),
        "expected --serving-manifest in error, got: {err}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_and_env_value_scrub_redacts_secrets_and_refuses_unsafe() {
    let tmp = fresh_tmp_dir("manifest_env_scrub");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let club_seats_path = write_seats(&tmp, all_clubbot_seats());

    let sk_secret = ["sk", "-", "live", "-", "0123456789abcdef"].concat();
    let model_filename = format!("/home/probeuser/models/{sk_secret}.gguf");
    let hf_secret = ["hf", "_", "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7"].concat();
    let os_build = format!("token={hf_secret}");
    let ghp_secret = ["ghp", "_", "99887766554433221100aabbccddeeff0011"].concat();
    let gh_pat_secret = [
        "github",
        "_",
        "pat",
        "_",
        "11A2B3C4D5E6F7G8H9I0J1",
        "_",
        "KLMNOPQRSTUVWXYZ0123456789ab",
    ]
    .concat();
    let bearer_secret = ["Bea", "rer", " zyx9876543210token"].concat();
    let api_key_kv = ["api", "_", "key=", "my_raw_apikey_val_111"].concat();
    let secret_kv = ["sec", "ret=", "my_raw_secret_val_222"].concat();
    let password_kv = ["pass", "word=", "my_raw_password_val_333"].concat();
    let auth_kv = ["au", "th=", "my_raw_auth_val_444"].concat();

    let mut manifest_val = base_llama_serving_json();
    manifest_val["model"]["filename"] = json!(model_filename);
    manifest_val["machine"]["os_build"] = json!(os_build);
    manifest_val["server_version"] = json!(format!("b11433 {api_key_kv} {secret_kv}"));
    manifest_val["server_build_commit"] = json!(format!("deadbeef {password_kv}"));
    manifest_val["machine"]["gpu_name"] = json!(format!("NVIDIA {auth_kv} {gh_pat_secret}"));
    let manifest_path = write_serving_manifest_json(&tmp, "scrub_manifest.json", &manifest_val);

    let game_dir = tmp.join("s7_game");
    let fake_home = tmp.join("sandbox_home");
    must_ok(fs::create_dir_all(&fake_home), "create fake home");

    let fake_home_str = fake_home.to_string_lossy().into_owned();
    let game_dir_str = game_dir.to_string_lossy().into_owned();
    let club_seats_str = club_seats_path.to_string_lossy().into_owned();
    let wrapper_path_str = wrapper_path.to_string_lossy().into_owned();
    let manifest_path_str = manifest_path.to_string_lossy().into_owned();
    let llama_env_kv = format!("{api_key_kv} {secret_kv}");
    let out = run_cli_via_test_seam(
        &tmp,
        "manifest_scrub",
        &[
            "play",
            "--game-dir",
            &game_dir_str,
            "--seats",
            &club_seats_str,
            "--wrapper",
            &wrapper_path_str,
            "--serving-manifest",
            &manifest_path_str,
            "--arm",
            "control",
            "--turns",
            "1",
            "--seed",
            "7",
        ],
        &[
            ("HOME", &fake_home_str),
            ("USERPROFILE", &fake_home_str),
            ("VLLM_ENV_GHP", &ghp_secret),
            ("VLLM_ENV_PAT", &gh_pat_secret),
            ("LLAMA_ENV_BEARER", &bearer_secret),
            ("LLAMA_ENV_KV", &llama_env_kv),
        ],
        &[
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let manifest_raw = must_ok(
        fs::read_to_string(game_dir.join("manifest.json")),
        "read s7 manifest.json",
    );
    for secret in [
        sk_secret.as_str(),
        "0123456789abcdef",
        hf_secret.as_str(),
        "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7",
        ghp_secret.as_str(),
        "99887766554433221100aabbccddeeff0011",
        gh_pat_secret.as_str(),
        "11A2B3C4D5E6F7G8H9I0J1_KLMNOPQRSTUVWXYZ0123456789ab",
        "zyx9876543210token",
        "my_raw_apikey_val_111",
        "my_raw_secret_val_222",
        "my_raw_password_val_333",
        "my_raw_auth_val_444",
        "/home/probeuser",
    ] {
        assert!(
            !manifest_raw.contains(secret),
            "secret/home substring '{secret}' leaked into manifest.json: {manifest_raw}"
        );
    }

    let parsed: EvalManifest = must_ok(serde_json::from_str(&manifest_raw), "parse s7 manifest");
    let srv = must_some(parsed.serving, "serving record present");
    assert!(srv.model.filename.starts_with("<home>/models/"));
    assert!(srv.model.filename.contains("[REDACTED]"));
    assert!(srv.machine.os_build.contains("[REDACTED]"));
    assert!(srv.server_version.value.contains("[REDACTED]"));
    assert!(
        srv.server_build_commit
            .as_ref()
            .is_some_and(|v| v.value.contains("[REDACTED]"))
    );
    assert!(srv.machine.gpu_name.contains("[REDACTED]"));

    // Unsafe raw GPU UUID placed inside a free-text manifest field refuses with UNSAFE_MANIFEST_VALUE.
    let mut unsafe_val = base_llama_serving_json();
    let leaked_gpu = [
        "GPU",
        "-",
        "deadbeef",
        "-",
        "1234",
        "-",
        "5678",
        "-",
        "90ab",
        "-",
        "fedcba098765",
    ]
    .concat();
    unsafe_val["machine"]["os_build"] = json!(format!("host-{leaked_gpu}"));
    let unsafe_path = write_serving_manifest_json(&tmp, "unsafe_manifest.json", &unsafe_val);
    let err = match league_eval::load_and_validate_serving_manifest(&unsafe_path) {
        Err(e) => e,
        Ok(_) => panic!("raw GPU UUID in free-text field must refuse"),
    };
    assert_eq!(err.code(), "UNSAFE_MANIFEST_VALUE");
    assert!(!err.to_string().contains(&leaked_gpu));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn regent_on_model_seat_or_prior_receipt_refuses_at_startup_zero_calls() {
    struct ZeroCallTransport {
        calls: AtomicUsize,
    }
    impl Transport for ZeroCallTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(8)
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(RawHttpResponse {
                status: 500,
                body: "{\"error\":\"should not be called\"}".to_owned(),
            })
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }
    }

    let tmp = fresh_tmp_dir("regent_startup");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    // Case 1: Model seat has a Standing Regent configured at startup.
    let seeded_dir = tmp.join("s8_configured_regent");
    must_ok(
        fs::create_dir_all(&seeded_dir),
        "mkdir s8_configured_regent",
    );
    let seeded_host = must_ok(
        EvalHost::from_standard_setup_with_model_seats(TableId(230), 4, 7, 111, &[false; 4]),
        "seed host with Seat 0 Regent",
    );
    let _ = must_ok(
        league_eval::persist_eval_game_state(&seeded_host, &seeded_dir),
        "persist seeded host",
    );
    let t1 = ZeroCallTransport {
        calls: AtomicUsize::new(0),
    };
    let err1 = match run_play_with_transport(
        &PlayOptions {
            game_dir: seeded_dir,
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Off,
            table_id: 230,
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
    ) {
        Err(e) => e,
        Ok(_) => panic!("Model seat with Standing Regent must refuse at startup"),
    };
    assert_eq!(err1.code(), "MODEL_SEAT_REGENT_INTERVENTION");
    assert_eq!(
        t1.calls.load(Ordering::SeqCst),
        0,
        "Case 1 must refuse before making any model or tokenizer calls"
    );

    // Case 2: Resumed host whose earlier receipt already records a Seat-0 Regent intervention.
    let prior_dir = tmp.join("s8_prior_receipt");
    must_ok(fs::create_dir_all(&prior_dir), "mkdir s8_prior_receipt");
    let mut prior_host = must_ok(
        EvalHost::from_standard_setup_with_model_seats(TableId(231), 4, 7, 222, &[false; 4]),
        "seed host for prior Regent receipt",
    );
    for _ in 0..4 {
        let _ = must_ok(
            prior_host.apply_act_submissions(&std::collections::BTreeMap::new()),
            "apply empty turn for Regent sweep",
        );
        let _ = must_ok(prior_host.sweep_turn(), "sweep empty turn");
    }
    assert!(
        prior_host
            .public_receipts()
            .iter()
            .any(|r| r.regent_acted_seats.contains(&0) || r.converted_to_ai.contains(&0)),
        "expected prior turn receipt with seat 0 Regent hit"
    );
    let _ = must_ok(
        league_eval::persist_eval_game_state(&prior_host, &prior_dir),
        "persist host with prior Regent receipt",
    );
    let wrapper_sha = sha256_hex(&must_ok(fs::read(&wrapper_path), "read r3 wrapper bytes"));
    let seats_sha = sha256_hex(&must_ok(fs::read(&seats_path), "read seats bytes"));
    let serving_rec = must_ok(
        league_eval::load_and_validate_serving_manifest(&serving_path),
        "load validated serving manifest",
    );
    let serving_sha = sha256_hex(&must_ok(
        serde_json::to_vec(&serving_rec),
        "encode serving manifest record",
    ));
    let setup_json = json!({
        "game_id": prior_host.game_id(),
        "table_id": 231,
        "seed": 7,
        "turns": 6,
        "seat_count": 4,
        "arm": "off",
        "eval_reply_wording_arm": league_eval::EVAL_REPLY_WORDING_ARM,
        "relay_wording_arm": "v2_tool_off",
        "wrapper_version": "v2_seat_brief_2026_11_21_r3",
        "wrapper_sha256": wrapper_sha,
        "memory_mode": "snapshot",
        "seats_sha256": seats_sha,
        "response_schema_sha256": null,
        "serving_manifest_sha256": serving_sha
    });
    must_ok(
        fs::write(
            prior_dir.join("setup.json"),
            must_ok(serde_json::to_string_pretty(&setup_json), "ser setup.json"),
        ),
        "write setup.json for resume path",
    );
    let t2 = ZeroCallTransport {
        calls: AtomicUsize::new(0),
    };
    let err2 = match run_play_with_transport(
        &PlayOptions {
            game_dir: prior_dir,
            seats_path,
            wrapper_path,
            wrapper_version: None,
            serving_manifest_path: Some(serving_path),
            arm: EvalArm::Off,
            table_id: 231,
            seed: 7,
            turns: 6,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &t2,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("resumed host with prior Regent receipt must refuse at startup"),
    };
    assert_eq!(err2.code(), "MODEL_SEAT_REGENT_INTERVENTION");
    assert!(
        err2.to_string()
            .contains("startup check refused already-voided run"),
        "expected already-voided startup refusal message, got: {err2}"
    );
    assert_eq!(
        t2.calls.load(Ordering::SeqCst),
        0,
        "Case 2 must refuse before making any model or tokenizer calls"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_hash_and_json_errors_never_echo_raw_values() {
    let tmp = fresh_tmp_dir("hash_json_no_echo");

    // 1. Raw GPU UUID placed into `machine.gpu_uuid_sha256` names the field and hides the UUID.
    let raw_gpu_uuid = [
        "GPU",
        "-",
        "1234abcd",
        "-",
        "0000",
        "-",
        "1111",
        "-",
        "2222",
        "-",
        "deadbeef9999",
    ]
    .concat();
    let mut bad_gpu_hash = base_llama_serving_json();
    bad_gpu_hash["machine"]["gpu_uuid_sha256"] = json!(raw_gpu_uuid);
    let p_gpu = write_serving_manifest_json(&tmp, "bad_gpu_hash.json", &bad_gpu_hash);
    let err_gpu = match league_eval::load_and_validate_serving_manifest(&p_gpu) {
        Err(e) => e,
        Ok(_) => panic!("raw GPU UUID in gpu_uuid_sha256 must fail"),
    };
    assert_eq!(err_gpu.code(), "INVALID_GPU_UUID_SHA256");
    let msg_gpu = err_gpu.to_string();
    assert!(msg_gpu.contains("gpu_uuid_sha256"), "msg: {msg_gpu}");
    assert!(!msg_gpu.contains(&raw_gpu_uuid), "echoed UUID: {msg_gpu}");
    assert!(!msg_gpu.contains("deadbeef9999"), "echoed UUID: {msg_gpu}");

    // 2. Invalid `model.sha256` names the field and never echoes the raw invalid value.
    let bad_model_sha = ["nothex-model-", "sentinel-998877"].concat();
    let mut bad_m = base_llama_serving_json();
    bad_m["model"]["sha256"] = json!(bad_model_sha);
    let p_m = write_serving_manifest_json(&tmp, "bad_model_sha.json", &bad_m);
    let err_m = match league_eval::load_and_validate_serving_manifest(&p_m) {
        Err(e) => e,
        Ok(_) => panic!("bad model.sha256 must fail"),
    };
    assert_eq!(err_m.code(), "MISSING_MODEL_SHA256");
    let msg_m = err_m.to_string();
    assert!(msg_m.contains("model.sha256"), "msg: {msg_m}");
    assert!(!msg_m.contains(&bad_model_sha), "echoed value: {msg_m}");
    assert!(!msg_m.contains("sentinel-998877"), "echoed value: {msg_m}");

    // 3. Invalid `model.chat_template_sha256` names the field and never echoes the raw value.
    let bad_tpl_sha = ["nothex-tpl-", "sentinel-665544"].concat();
    let mut bad_t = base_llama_serving_json();
    bad_t["model"]["chat_template_sha256"] = json!(bad_tpl_sha);
    let p_t = write_serving_manifest_json(&tmp, "bad_tpl_sha.json", &bad_t);
    let err_t = match league_eval::load_and_validate_serving_manifest(&p_t) {
        Err(e) => e,
        Ok(_) => panic!("bad model.chat_template_sha256 must fail"),
    };
    assert_eq!(err_t.code(), "MISSING_SERVING_MANIFEST");
    let msg_t = err_t.to_string();
    assert!(msg_t.contains("model.chat_template_sha256"), "msg: {msg_t}");
    assert!(!msg_t.contains(&bad_tpl_sha), "echoed value: {msg_t}");
    assert!(!msg_t.contains("sentinel-665544"), "echoed value: {msg_t}");

    // 4. Malformed JSON with secret-bearing string refuses with line/column only, never echoes text.
    let secret_json_val = ["SENTINEL_BAD_JSON_", "SECRET_776655"].concat();
    let bad_json_path = tmp.join("bad_json.json");
    must_ok(
        fs::write(
            &bad_json_path,
            format!("{{\"engine\": \"{secret_json_val}\"}}"),
        ),
        "write bad json",
    );
    let err_j = match league_eval::load_and_validate_serving_manifest(&bad_json_path) {
        Err(e) => e,
        Ok(_) => panic!("unknown engine enum string must fail"),
    };
    assert_eq!(err_j.code(), "MISSING_SERVING_MANIFEST");
    let msg_j = err_j.to_string();
    assert!(msg_j.contains("line "), "msg: {msg_j}");
    assert!(
        !msg_j.contains(&secret_json_val),
        "echoed JSON string: {msg_j}"
    );
    assert!(
        !msg_j.contains("SECRET_776655"),
        "echoed JSON string: {msg_j}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

fn real_b11433_shaped_body(content: &str, prompt_tokens: u64, top_logprob_count: u64) -> String {
    let top_entries: Vec<Value> = (0_u64..top_logprob_count)
        .map(|k| {
            json!({
                "token": format!("t{k}"),
                "id": 500 + k,
                "logprob": -0.05 * ((k + 1) as f64)
            })
        })
        .collect();
    json!({
        "id": "b11433-real-1",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "logprob": -0.01,
                    "top_logprobs": top_entries
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": 5,
            "total_tokens": prompt_tokens + 5
        }
    })
    .to_string()
}

#[test]
fn real_server_shaped_reply_records_tokenize_hash_and_source() {
    struct RealServerShapedTransport {
        tpl_calls: AtomicUsize,
        tok_calls: AtomicUsize,
        chat_reqs: Mutex<Vec<Value>>,
        token_ids: Vec<u64>,
    }
    impl Transport for RealServerShapedTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            self.tpl_calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("RENDERED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            content: &str,
            add_special: bool,
            parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            self.tok_calls.fetch_add(1, Ordering::SeqCst);
            assert!(add_special && parse_special);
            assert!(content.starts_with("RENDERED:"));
            Ok(self.token_ids.clone())
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let parsed: Value = serde_json::from_str(body)
                .map_err(|e| TransportError::Network(format!("parse chat body: {e}")))?;
            assert!(parsed.get("return_tokens").is_none());
            assert!(parsed.get("return_token_ids").is_none());
            assert_eq!(parsed.get("cache_prompt"), Some(&json!(false)));
            assert_eq!(parsed.get("logprobs"), Some(&json!(true)));
            assert_eq!(parsed.get("top_logprobs"), Some(&json!(5)));
            if let Ok(mut g) = self.chat_reqs.lock() {
                g.push(parsed);
            }
            let clean =
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: real_b11433_shaped_body(&clean, self.token_ids.len() as u64, 5),
            })
        }
    }

    let tmp = fresh_tmp_dir("real_b11433_ok");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());
    let expected_ids: Vec<u64> = (1001..1013).collect();
    let expected_sha = sha256_hex(&must_ok(serde_json::to_vec(&expected_ids), "encode ids"));
    let t = RealServerShapedTransport {
        tpl_calls: AtomicUsize::new(0),
        tok_calls: AtomicUsize::new(0),
        chat_reqs: Mutex::new(Vec::new()),
        token_ids: expected_ids,
    };
    let game_dir = tmp.join("game");
    let _m = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                serving_manifest_path: Some(serving_path),
                arm: EvalArm::Off,
                table_id: 301,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &t,
            None,
        ),
        "real b11433 shaped reply must pass",
    );
    assert_eq!(t.tpl_calls.load(Ordering::SeqCst), 4);
    assert_eq!(t.tok_calls.load(Ordering::SeqCst), 2);
    let talk_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-01-talk-Foundry.json")),
            "read talk decision",
        )),
        "parse talk decision",
    );
    assert_eq!(talk_dec.prompt_token_ids_sha256s, vec![expected_sha]);
    assert_eq!(
        talk_dec.prompt_token_ids_source.as_deref(),
        Some("server_tokenize")
    );
    assert_eq!(
        talk_dec.prompt_token_ids_template_check.as_deref(),
        Some("double_apply_template_byte_equal")
    );
    assert_eq!(talk_dec.reply_token_ids, vec![vec![11]]);
    assert_eq!(talk_dec.reply_top_logprobs[0][0].top_logprobs.len(), 5);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn prompt_token_count_mismatch_refuses() {
    struct MismatchCountTransport;
    impl Transport for MismatchCountTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("RENDERED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            Ok((100_u64..111).collect())
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let clean =
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: real_b11433_shaped_body(&clean, 10, 5),
            })
        }
    }

    let tmp = fresh_tmp_dir("prompt_count_mismatch");
    let err = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("game"),
            seats_path: write_seats(&tmp, local_foundry_three_clubbot_seats()),
            wrapper_path: copy_pinned_r3_wrapper(&tmp),
            wrapper_version: None,
            serving_manifest_path: Some(write_serving_manifest_json(
                &tmp,
                "serving.json",
                &base_llama_serving_json(),
            )),
            arm: EvalArm::Off,
            table_id: 302,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &MismatchCountTransport,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("mismatched tokenize vs usage.prompt_tokens count must refuse"),
    };
    assert_eq!(err.code(), "PROMPT_TOKEN_COUNT_MISMATCH");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn missing_or_failing_template_and_tokenize_endpoints_refuse() {
    struct FailingEndpointsTransport {
        fail_template: bool,
        tok_calls: AtomicUsize,
        chat_calls: AtomicUsize,
    }
    impl Transport for FailingEndpointsTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            if self.fail_template {
                Err(TransportError::Network(
                    "POST /apply-template returned HTTP 404".to_owned(),
                ))
            } else {
                Ok(format!("RENDERED:{body}"))
            }
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            self.tok_calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network(
                "POST /tokenize returned HTTP 404".to_owned(),
            ))
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.chat_calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network(
                "chat must not be called".to_owned(),
            ))
        }
    }

    let tmp = fresh_tmp_dir("missing_endpoints");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let t_tpl = FailingEndpointsTransport {
        fail_template: true,
        tok_calls: AtomicUsize::new(0),
        chat_calls: AtomicUsize::new(0),
    };
    let err_tpl = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("fail_tpl"),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Off,
            table_id: 303,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &t_tpl,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("missing /apply-template must refuse"),
    };
    assert_eq!(err_tpl.code(), "PROMPT_TEMPLATE_FAILED");
    assert_eq!(t_tpl.tok_calls.load(Ordering::SeqCst), 0);
    assert_eq!(t_tpl.chat_calls.load(Ordering::SeqCst), 0);

    let t_tok = FailingEndpointsTransport {
        fail_template: false,
        tok_calls: AtomicUsize::new(0),
        chat_calls: AtomicUsize::new(0),
    };
    let err_tok = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("fail_tok"),
            seats_path,
            wrapper_path,
            wrapper_version: None,
            serving_manifest_path: Some(serving_path),
            arm: EvalArm::Off,
            table_id: 304,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &t_tok,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("missing /tokenize must refuse"),
    };
    assert_eq!(err_tok.code(), "PROMPT_TOKENIZE_FAILED");
    assert_eq!(t_tok.tok_calls.load(Ordering::SeqCst), 1);
    assert_eq!(t_tok.chat_calls.load(Ordering::SeqCst), 0);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn apply_template_non_identical_second_call_refuses() {
    struct DriftingTemplateTransport {
        tpl_step: AtomicUsize,
        tok_calls: AtomicUsize,
        chat_calls: AtomicUsize,
    }
    impl Transport for DriftingTemplateTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            let n = self.tpl_step.fetch_add(1, Ordering::SeqCst);
            Ok(format!("TEMPLATED_{n}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            self.tok_calls.fetch_add(1, Ordering::SeqCst);
            Ok((100_u64..110).collect())
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.chat_calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network(
                "chat must not be called".to_owned(),
            ))
        }
    }

    let tmp = fresh_tmp_dir("template_mismatch");
    let t = DriftingTemplateTransport {
        tpl_step: AtomicUsize::new(0),
        tok_calls: AtomicUsize::new(0),
        chat_calls: AtomicUsize::new(0),
    };
    let err = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("game"),
            seats_path: write_seats(&tmp, local_foundry_three_clubbot_seats()),
            wrapper_path: copy_pinned_r3_wrapper(&tmp),
            wrapper_version: None,
            serving_manifest_path: Some(write_serving_manifest_json(
                &tmp,
                "serving.json",
                &base_llama_serving_json(),
            )),
            arm: EvalArm::Off,
            table_id: 305,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &t,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("non-identical /apply-template responses must refuse"),
    };
    assert_eq!(err.code(), "PROMPT_TEMPLATE_MISMATCH");
    assert_eq!(t.tpl_step.load(Ordering::SeqCst), 2);
    assert_eq!(t.tok_calls.load(Ordering::SeqCst), 0);
    assert_eq!(t.chat_calls.load(Ordering::SeqCst), 0);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn branch_list_refuses_later_target_regent_before_any_model_call() {
    struct LocalOkTransport;
    impl Transport for LocalOkTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
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
            Ok((100_u64..110).collect())
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let clean =
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: real_b11433_shaped_body(&clean, 10, 5),
            })
        }
    }

    struct BranchZeroCallTransport {
        calls: AtomicUsize,
    }
    impl Transport for BranchZeroCallTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }
    }

    let tmp = fresh_tmp_dir("branch_list_preflight_regent");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let model_seats_path = write_seats(&tmp, local_foundry_three_clubbot_seats());
    let club_seats_path = tmp.join("club_seats.json");
    must_ok(
        fs::write(
            &club_seats_path,
            must_ok(serde_json::to_string_pretty(&all_clubbot_seats()), "ser"),
        ),
        "write club_seats",
    );
    let serving_path =
        write_serving_manifest_json(&tmp, "serving.json", &base_llama_serving_json());

    let clean_game = tmp.join("clean_target_0");
    let clean_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: clean_game.clone(),
                seats_path: model_seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(serving_path.clone()),
                arm: EvalArm::Off,
                table_id: 306,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &LocalOkTransport,
            None,
        ),
        "create clean target 0",
    );

    let regent_game = tmp.join("regent_target_1");
    let regent_manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: regent_game.clone(),
                seats_path: club_seats_path,
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(serving_path.clone()),
                arm: EvalArm::Off,
                table_id: 307,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &LocalOkTransport,
            None,
        ),
        "create regent target 1",
    );

    let sel_list = SelectionList {
        version: 1,
        selection_seed: 7,
        per_game: 1,
        strata: [1, 0, 0],
        states: vec![
            SelectedPreActState {
                game_dir: clean_game.clone(),
                game_id: clean_manifest.game_id,
                day: 1,
                stratum: TurnStratum::Early,
                seat_index: 0,
                seat_guild: "Foundry".to_owned(),
                checkpoint_prefix: clean_game.join("checkpoints/day-01-pre-act"),
            },
            SelectedPreActState {
                game_dir: regent_game.clone(),
                game_id: regent_manifest.game_id,
                day: 1,
                stratum: TurnStratum::Early,
                seat_index: 0,
                seat_guild: "Foundry".to_owned(),
                checkpoint_prefix: regent_game.join("checkpoints/day-01-pre-act"),
            },
        ],
    };
    let list_path = tmp.join("list.json");
    must_ok(
        fs::write(
            &list_path,
            must_ok(serde_json::to_string_pretty(&sel_list), "ser list"),
        ),
        "write list.json",
    );

    let zero_tx = BranchZeroCallTransport {
        calls: AtomicUsize::new(0),
    };
    let err = match run_branch_with_transport(
        &BranchOptions {
            checkpoint_prefix: None,
            list_path: Some(list_path),
            focal_seat: None,
            seats_path: model_seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Off],
            out_dir: tmp.join("branch_out"),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &zero_tx,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("later target with model-seat Regent must refuse before any model call"),
    };
    assert_eq!(err.code(), "MODEL_SEAT_REGENT_INTERVENTION");
    assert_eq!(zero_tx.calls.load(Ordering::SeqCst), 0);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn non_local_openrouter_seat_refuses_at_eval_startup_with_zero_calls() {
    struct ZeroCallTransport {
        calls: AtomicUsize,
    }
    impl Transport for ZeroCallTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(TransportError::Network("should not be called".to_owned()))
        }
    }

    let tmp = fresh_tmp_dir("non_local_openrouter_refuse");
    let openrouter_seats = write_seats(
        &tmp,
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
    let zero_tx = ZeroCallTransport {
        calls: AtomicUsize::new(0),
    };
    let err = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("game"),
            seats_path: openrouter_seats,
            wrapper_path: copy_pinned_r3_wrapper(&tmp),
            wrapper_version: None,
            serving_manifest_path: Some(write_serving_manifest_json(
                &tmp,
                "serving.json",
                &base_llama_serving_json(),
            )),
            arm: EvalArm::Off,
            table_id: 308,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &zero_tx,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("openrouter model seat in Eval mode must refuse at startup"),
    };
    assert_eq!(err.code(), "UNSUPPORTED_EVAL_MODEL_BACKEND");
    assert_eq!(zero_tx.calls.load(Ordering::SeqCst), 0);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn top_logprobs_with_four_entries_refuses() {
    struct FourTopLogprobsTransport;
    impl Transport for FourTopLogprobsTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            Ok(format!("RENDERED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            Ok((100_u64..110).collect())
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let clean =
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: real_b11433_shaped_body(&clean, 10, 4),
            })
        }
    }

    let tmp = fresh_tmp_dir("top_logprobs_four");
    let err = match run_play_with_transport(
        &PlayOptions {
            game_dir: tmp.join("game"),
            seats_path: write_seats(&tmp, local_foundry_three_clubbot_seats()),
            wrapper_path: copy_pinned_r3_wrapper(&tmp),
            wrapper_version: None,
            serving_manifest_path: Some(write_serving_manifest_json(
                &tmp,
                "serving.json",
                &base_llama_serving_json(),
            )),
            arm: EvalArm::Off,
            table_id: 309,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &FourTopLogprobsTransport,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!("4 top_logprobs entries instead of 5 must refuse"),
    };
    assert_eq!(err.code(), "MISSING_REPLY_TELEMETRY");
    let msg = err.to_string();
    assert!(
        msg.contains("top_logprobs` must have exact length 5, found 4"),
        "expected exact-5 top_logprobs refusal, got: {msg}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

struct ZeroCallTransport {
    calls: AtomicUsize,
}

impl ZeroCallTransport {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Transport for ZeroCallTransport {
    fn count_tokens(
        &self,
        _url: &str,
        _model: &str,
        _content: &str,
        _timeout: Duration,
    ) -> Result<usize, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(TransportError::Network("should not be called".to_owned()))
    }

    fn apply_template(
        &self,
        _apply_template_url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<String, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(TransportError::Network("should not be called".to_owned()))
    }

    fn tokenize_prompt(
        &self,
        _tokenize_url: &str,
        _content: &str,
        _add_special: bool,
        _parse_special: bool,
        _timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(TransportError::Network("should not be called".to_owned()))
    }

    fn post_chat_completions(
        &self,
        _url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(TransportError::Network("should not be called".to_owned()))
    }
}

#[test]
fn reasoning_budget_sent_as_reasoning_budget_tokens_when_enforceable() {
    struct BudgetCaptureTransport {
        tpl_reqs: Mutex<Vec<Value>>,
        chat_reqs: Mutex<Vec<Value>>,
    }
    impl Transport for BudgetCaptureTransport {
        fn count_tokens(
            &self,
            _url: &str,
            _model: &str,
            _content: &str,
            _timeout: Duration,
        ) -> Result<usize, TransportError> {
            Ok(4)
        }

        fn apply_template(
            &self,
            _apply_template_url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<String, TransportError> {
            let parsed: Value = serde_json::from_str(body)
                .map_err(|e| TransportError::Network(format!("parse tpl body: {e}")))?;
            if let Ok(mut g) = self.tpl_reqs.lock() {
                g.push(parsed);
            }
            Ok(format!("RENDERED:{body}"))
        }

        fn tokenize_prompt(
            &self,
            _tokenize_url: &str,
            _content: &str,
            _add_special: bool,
            _parse_special: bool,
            _timeout: Duration,
        ) -> Result<Vec<u64>, TransportError> {
            Ok((100_u64..110).collect())
        }

        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let parsed: Value = serde_json::from_str(body)
                .map_err(|e| TransportError::Network(format!("parse chat body: {e}")))?;
            if let Ok(mut g) = self.chat_reqs.lock() {
                g.push(parsed);
            }
            let clean =
                json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
                    .to_string();
            Ok(RawHttpResponse {
                status: 200,
                body: real_b11433_shaped_body(&clean, 10, 5),
            })
        }
    }

    let tmp = fresh_tmp_dir("reasoning_budget_ok");
    let mut seats = local_foundry_three_clubbot_seats();
    seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "chat_template_kwargs": { "enable_thinking": true },
        "exclude_from_transcript": false
    });
    let mut serving = base_llama_serving_json();
    serving["model"]["thinking_start_tag"] = json!("<think>");
    serving["model"]["thinking_end_tags"] = json!(["</think>"]);
    serving["server_settings"]["jinja"] = json!(true);
    serving["server_settings"]["reasoning_mode"] = json!("on");
    serving["server_settings"]["default_reasoning_budget"] = json!(-1);
    serving["server_settings"]["reasoning_budget_message"] = json!("");

    let tx = BudgetCaptureTransport {
        tpl_reqs: Mutex::new(Vec::new()),
        chat_reqs: Mutex::new(Vec::new()),
    };
    let game_dir = tmp.join("game");
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: write_seats(&tmp, seats),
                wrapper_path: copy_pinned_r3_wrapper(&tmp),
                wrapper_version: None,
                serving_manifest_path: Some(write_serving_manifest_json(
                    &tmp,
                    "serving.json",
                    &serving,
                )),
                arm: EvalArm::Off,
                table_id: 310,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &tx,
            None,
        ),
        "enforceable reasoning budget run must succeed",
    );
    let serving_rec = must_some(manifest.serving.as_ref(), "serving manifest");
    assert_eq!(
        serving_rec.model.thinking_start_tag.as_deref(),
        Some("<think>")
    );
    assert_eq!(
        serving_rec.model.thinking_end_tags,
        Some(vec!["</think>".to_owned()])
    );
    assert_eq!(serving_rec.server_settings.jinja, Some(true));
    assert_eq!(
        serving_rec.server_settings.reasoning_mode.as_deref(),
        Some("on")
    );
    assert_eq!(
        serving_rec.server_settings.default_reasoning_budget,
        Some(-1)
    );
    assert_eq!(
        serving_rec
            .server_settings
            .reasoning_budget_message
            .as_deref(),
        Some("")
    );

    let chat_reqs = must_ok(tx.chat_reqs.lock(), "chat lock");
    assert_eq!(chat_reqs.len(), 2);
    for req in chat_reqs.iter() {
        assert_eq!(req.get("reasoning_budget_tokens"), Some(&json!(256)));
        assert!(req.get("thinking_token_budget").is_none());
    }
    let tpl_reqs = must_ok(tx.tpl_reqs.lock(), "tpl lock");
    assert_eq!(tpl_reqs.len(), 4);
    for req in tpl_reqs.iter() {
        assert_eq!(req.get("reasoning_budget_tokens"), Some(&json!(256)));
        assert!(req.get("thinking_token_budget").is_none());
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn unenforceable_reasoning_budget_refuses_at_eval_startup_with_zero_calls() {
    fn valid_manifest_and_seat() -> (Value, Value) {
        let mut seats = local_foundry_three_clubbot_seats();
        seats["seats"][0]["reasoning"] = json!({
            "effort": "medium",
            "budget_tokens": 256,
            "chat_template_kwargs": { "enable_thinking": true },
            "exclude_from_transcript": false
        });
        let mut serving = base_llama_serving_json();
        serving["model"]["thinking_start_tag"] = json!("<think>");
        serving["model"]["thinking_end_tags"] = json!(["</think>"]);
        serving["server_settings"]["jinja"] = json!(true);
        serving["server_settings"]["reasoning_mode"] = json!("on");
        serving["server_settings"]["reasoning_budget_message"] = json!("");
        (seats, serving)
    }

    type CaseMutator = fn(&mut Value, &mut Value);
    let cases: [(&str, &str, CaseMutator); 9] = [
        (
            "missing_thinking_tags",
            "UNENFORCEABLE_REASONING_BUDGET",
            |_seats, srv| {
                if let Some(m) = srv["model"].as_object_mut() {
                    m.remove("thinking_start_tag");
                    m.remove("thinking_end_tags");
                }
            },
        ),
        (
            "blank_thinking_start_tag",
            "UNENFORCEABLE_REASONING_BUDGET",
            |_seats, srv| {
                srv["model"]["thinking_start_tag"] = json!("   ");
            },
        ),
        (
            "empty_end_tags",
            "UNENFORCEABLE_REASONING_BUDGET",
            |_seats, srv| {
                srv["model"]["thinking_end_tags"] = json!([]);
            },
        ),
        (
            "jinja_disabled",
            "UNENFORCEABLE_REASONING_BUDGET",
            |_seats, srv| {
                srv["server_settings"]["jinja"] = json!(false);
            },
        ),
        (
            "jinja_unspecified",
            "UNENFORCEABLE_REASONING_BUDGET",
            |_seats, srv| {
                if let Some(m) = srv["server_settings"].as_object_mut() {
                    m.remove("jinja");
                }
            },
        ),
        (
            "vllm_engine",
            "UNENFORCEABLE_REASONING_BUDGET",
            |_seats, srv| {
                srv["engine"] = json!("vllm");
                srv["cache_settings"] = json!({
                    "enable_prefix_caching": false,
                    "provenance": "server_reported"
                });
            },
        ),
        (
            "budget_over_i32_max",
            "UNENFORCEABLE_REASONING_BUDGET",
            |seats, _srv| {
                seats["seats"][0]["reasoning"]["budget_tokens"] = json!(2_147_483_648_u64);
            },
        ),
        (
            "effort_none_disables_thinking",
            "INVALID_SEAT_CONFIG",
            |seats, _srv| {
                seats["seats"][0]["reasoning"]["effort"] = json!("none");
            },
        ),
        (
            "enable_thinking_false_kwarg",
            "INVALID_SEAT_CONFIG",
            |seats, _srv| {
                seats["seats"][0]["reasoning"]["chat_template_kwargs"] =
                    json!({"enable_thinking": false});
            },
        ),
    ];

    let tmp = fresh_tmp_dir("unenforceable_budget");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    for (idx, (label, expected_code, mutate)) in cases.iter().enumerate() {
        let (mut seats, mut serving) = valid_manifest_and_seat();
        mutate(&mut seats, &mut serving);
        let case_dir = tmp.join(format!("case_{idx}_{label}"));
        let zero_tx = ZeroCallTransport::new();
        let game_dir = case_dir.join("game");
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: write_seats(&case_dir, seats),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_serving_manifest_json(
                    &case_dir,
                    "serving.json",
                    &serving,
                )),
                arm: EvalArm::Off,
                table_id: 311 + idx as u64,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &zero_tx,
            None,
        ) {
            Err(e) => e,
            Ok(_) => {
                panic!("case '{label}' must refuse at startup with {expected_code}")
            }
        };
        assert_eq!(err.code(), *expected_code, "case={label}");
        assert_eq!(err.exit_code(), 1, "case={label}");
        assert_eq!(zero_tx.call_count(), 0, "case={label}");
        assert!(!game_dir.exists(), "case={label} must not touch game_dir");
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn reasoning_budget_message_refuses_at_eval_startup_with_zero_calls() {
    let tmp = fresh_tmp_dir("reasoning_budget_msg_forbidden");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);

    let mut budgeted_seats = local_foundry_three_clubbot_seats();
    budgeted_seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "chat_template_kwargs": { "enable_thinking": true },
        "exclude_from_transcript": false
    });
    let no_budget_seats = local_foundry_three_clubbot_seats();

    let mut serving = base_llama_serving_json();
    serving["model"]["thinking_start_tag"] = json!("<think>");
    serving["model"]["thinking_end_tags"] = json!(["</think>"]);
    serving["server_settings"]["jinja"] = json!(true);
    serving["server_settings"]["reasoning_mode"] = json!("on");
    serving["server_settings"]["default_reasoning_budget"] = json!(256);
    serving["server_settings"]["reasoning_budget_message"] = json!("Time to conclude.");

    for (idx, (label, seats)) in [
        ("with_budget", budgeted_seats),
        ("without_budget", no_budget_seats),
    ]
    .into_iter()
    .enumerate()
    {
        let case_dir = tmp.join(label);
        let zero_tx = ZeroCallTransport::new();
        let game_dir = case_dir.join("game");
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: write_seats(&case_dir, seats),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_serving_manifest_json(
                    &case_dir,
                    "serving.json",
                    &serving,
                )),
                arm: EvalArm::Off,
                table_id: 330 + idx as u64,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &zero_tx,
            None,
        ) {
            Err(e) => e,
            Ok(_) => panic!("case '{label}' must refuse REASONING_BUDGET_MESSAGE_FORBIDDEN"),
        };
        assert_eq!(
            err.code(),
            "REASONING_BUDGET_MESSAGE_FORBIDDEN",
            "case={label}"
        );
        assert_eq!(err.exit_code(), 1, "case={label}");
        assert_eq!(zero_tx.call_count(), 0, "case={label}");
        assert!(!game_dir.exists(), "case={label} must not touch game_dir");
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn budget_seat_without_enable_thinking_refuses_at_eval_startup_with_zero_calls() {
    let tmp = fresh_tmp_dir("budget_without_enable_thinking");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);

    let mut omitted_kwargs_seats = local_foundry_three_clubbot_seats();
    omitted_kwargs_seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "exclude_from_transcript": false
    });
    let mut empty_kwargs_seats = local_foundry_three_clubbot_seats();
    empty_kwargs_seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "chat_template_kwargs": {},
        "exclude_from_transcript": false
    });
    let mut explicit_true_seats = local_foundry_three_clubbot_seats();
    explicit_true_seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "chat_template_kwargs": { "enable_thinking": true },
        "exclude_from_transcript": false
    });

    let mut serving_on = base_llama_serving_json();
    serving_on["model"]["thinking_start_tag"] = json!("<think>");
    serving_on["model"]["thinking_end_tags"] = json!(["</think>"]);
    serving_on["server_settings"]["jinja"] = json!(true);
    serving_on["server_settings"]["reasoning_mode"] = json!("on");
    serving_on["server_settings"]["reasoning_budget_message"] = json!("");

    let mut serving_off = serving_on.clone();
    serving_off["server_settings"]["reasoning_mode"] = json!("off");

    for (idx, (label, seats, serving)) in [
        ("omitted_kwargs", omitted_kwargs_seats, serving_on.clone()),
        ("empty_kwargs_no_flag", empty_kwargs_seats, serving_on),
        ("server_reasoning_off", explicit_true_seats, serving_off),
    ]
    .into_iter()
    .enumerate()
    {
        let case_dir = tmp.join(label);
        let zero_tx = ZeroCallTransport::new();
        let game_dir = case_dir.join("game");
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: write_seats(&case_dir, seats),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_serving_manifest_json(
                    &case_dir,
                    "serving.json",
                    &serving,
                )),
                arm: EvalArm::Off,
                table_id: 340 + idx as u64,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &zero_tx,
            None,
        ) {
            Err(e) => e,
            Ok(_) => panic!("case '{label}' must refuse UNENFORCEABLE_REASONING_BUDGET"),
        };
        assert_eq!(err.code(), "UNENFORCEABLE_REASONING_BUDGET", "case={label}");
        assert_eq!(err.exit_code(), 1, "case={label}");
        assert_eq!(zero_tx.call_count(), 0, "case={label}");
        assert!(!game_dir.exists(), "case={label} must not touch game_dir");
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn serving_manifest_records_reasoning_mode_default_budget_and_empty_message() {
    let tmp = fresh_tmp_dir("manifest_reasoning_fields");
    let mut serving = base_llama_serving_json();
    serving["model"]["thinking_start_tag"] = json!("<think>");
    serving["model"]["thinking_end_tags"] = json!(["</think>"]);
    serving["server_settings"]["jinja"] = json!(true);
    serving["server_settings"]["reasoning_mode"] = json!("auto");
    serving["server_settings"]["default_reasoning_budget"] = json!(1024);
    serving["server_settings"]["reasoning_budget_message"] = json!("");

    let game_dir = tmp.join("game");
    let _manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: write_seats(&tmp, all_clubbot_seats()),
                wrapper_path: copy_pinned_r3_wrapper(&tmp),
                wrapper_version: None,
                serving_manifest_path: Some(write_serving_manifest_json(
                    &tmp,
                    "serving.json",
                    &serving,
                )),
                arm: EvalArm::Off,
                table_id: 350,
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
        ),
        "empty reasoning_budget_message manifest run should succeed",
    );

    let manifest_raw = must_ok(
        fs::read_to_string(game_dir.join("manifest.json")),
        "read manifest.json",
    );
    let manifest_json: Value = must_ok(serde_json::from_str(&manifest_raw), "parse manifest.json");
    let ss = &manifest_json["serving"]["server_settings"];
    assert_eq!(ss.get("reasoning_mode"), Some(&json!("auto")));
    assert_eq!(ss.get("default_reasoning_budget"), Some(&json!(1024)));
    assert_eq!(ss.get("reasoning_budget_message"), Some(&json!("")));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn eval_test_seam_passes_with_untracked_file_while_real_path_refuses_dirty_tree() {
    let tmp = fresh_tmp_dir("dirty_tree_seam");
    let repo_dir = tmp.join("isolated_repo");
    must_ok(fs::create_dir_all(&repo_dir), "create isolated temp repo");
    let run_git = |args: &[&str]| {
        let out = must_ok(
            Command::new("git")
                .current_dir(&repo_dir)
                .args(args)
                .output(),
            "run git in temp repo",
        );
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.name", "Eval Test"]);
    run_git(&["config", "user.email", "eval-test@example.invalid"]);
    must_ok(
        fs::write(repo_dir.join("tracked.txt"), "clean\n"),
        "write tracked file",
    );
    run_git(&["add", "tracked.txt"]);
    run_git(&["commit", "-q", "-m", "init"]);
    let head_sha = run_git(&["rev-parse", "HEAD"]);
    must_ok(
        fs::write(repo_dir.join("untracked.tmp"), "dirty\n"),
        "write untracked file inside temp repo",
    );

    let seats_path = write_seats(&tmp, all_clubbot_seats());
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let seam_source = CrateGitShaSource::Injected {
        built_sha: &head_sha,
        live_head: Some(&head_sha),
        is_dirty: Some(false),
    };
    let live_dir_source = CrateGitShaSource::LiveDir {
        manifest_dir: &repo_dir,
        built_sha: &head_sha,
    };

    // 1. Test seam runs cleanly when the repo reports a clean injected state.
    let seam_manifest = must_ok(
        run_play_with_transport_and_git_sha(
            &PlayOptions {
                game_dir: tmp.join("seam_game"),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 319,
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
            &seam_source,
        ),
        "seam must allow dirty checkout tests",
    );
    assert_eq!(seam_manifest.played_turns, 1);

    // 2. Real git-dir check refuses the untracked file at startup with DIRTY_GIT_WORKTREE.
    let real_err = match run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: tmp.join("real_game"),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: None,
            arm: EvalArm::Off,
            table_id: 320,
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
        &live_dir_source,
    ) {
        Err(e) => e,
        Ok(_) => panic!("live git check must refuse dirty temp checkout"),
    };
    assert_eq!(real_err.code(), "DIRTY_GIT_WORKTREE");
    assert_eq!(real_err.exit_code(), 2);

    // 3. Real CLI path (`execute_cli_with_git_sha` with LiveDir) also refuses dirty tree with exit 2.
    let mut cli_out = Vec::new();
    let mut cli_err = Vec::new();
    let cli_game = tmp.join("cli_dirty_game");
    let cli_code = execute_cli_with_git_sha(
        [
            "play".into(),
            "--game-dir".into(),
            cli_game.clone().into_os_string(),
            "--seats".into(),
            seats_path.into_os_string(),
            "--wrapper".into(),
            wrapper_path.into_os_string(),
            "--arm".into(),
            "off".into(),
            "--turns".into(),
            "1".into(),
            "--seed".into(),
            "7".into(),
        ],
        &mut cli_out,
        &mut cli_err,
        &live_dir_source,
    );
    assert_eq!(cli_code, 2);
    assert!(
        String::from_utf8_lossy(&cli_err).contains("DIRTY_GIT_WORKTREE"),
        "expected DIRTY_GIT_WORKTREE on stderr, got: {}",
        String::from_utf8_lossy(&cli_err)
    );
    assert!(!cli_game.exists());

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn budget_seat_with_absent_reasoning_budget_message_refuses_while_empty_passes() {
    let tmp = fresh_tmp_dir("budget_absent_msg");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let mut budgeted_seats = local_foundry_three_clubbot_seats();
    budgeted_seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "chat_template_kwargs": { "enable_thinking": true },
        "exclude_from_transcript": false
    });
    let no_budget_seats = local_foundry_three_clubbot_seats();

    let mut serving_absent_msg = base_llama_serving_json();
    serving_absent_msg["model"]["thinking_start_tag"] = json!("<think>");
    serving_absent_msg["model"]["thinking_end_tags"] = json!(["</think>"]);
    serving_absent_msg["server_settings"]["jinja"] = json!(true);
    serving_absent_msg["server_settings"]["reasoning_mode"] = json!("on");

    // 1. Budget seat + absent server_settings.reasoning_budget_message -> refuse zero calls.
    let zero_tx = ZeroCallTransport::new();
    let refuse_dir = tmp.join("refuse_absent_msg");
    let refuse_game = refuse_dir.join("game");
    let err = match run_play_with_transport(
        &PlayOptions {
            game_dir: refuse_game.clone(),
            seats_path: write_seats(&refuse_dir, budgeted_seats.clone()),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(write_serving_manifest_json(
                &refuse_dir,
                "serving.json",
                &serving_absent_msg,
            )),
            arm: EvalArm::Off,
            table_id: 360,
            seed: 7,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &zero_tx,
        None,
    ) {
        Err(e) => e,
        Ok(_) => panic!(
            "absent server_settings.reasoning_budget_message with budget seat must refuse UNENFORCEABLE_REASONING_BUDGET"
        ),
    };
    assert_eq!(err.code(), "UNENFORCEABLE_REASONING_BUDGET");
    assert_eq!(err.exit_code(), 1);
    assert_eq!(zero_tx.call_count(), 0);
    assert!(!refuse_game.exists());

    // 2. Budget seat + explicit "" and non-budget seat + absent message both pass.
    let mut serving_empty_msg = serving_absent_msg.clone();
    serving_empty_msg["server_settings"]["reasoning_budget_message"] = json!("");
    let ok_reply = FixedReplyTransport {
        content: json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
            .to_string(),
    };
    for (idx, (label, seats, srv, expected_msg)) in [
        (
            "budget_empty_msg",
            budgeted_seats,
            serving_empty_msg,
            Some(""),
        ),
        (
            "no_budget_absent_msg",
            no_budget_seats,
            serving_absent_msg,
            None,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let case_dir = tmp.join(label);
        let manifest = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: case_dir.join("game"),
                    seats_path: write_seats(&case_dir, seats),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    serving_manifest_path: Some(write_serving_manifest_json(
                        &case_dir,
                        "serving.json",
                        &srv,
                    )),
                    arm: EvalArm::Off,
                    table_id: 361 + idx as u64,
                    seed: 7,
                    turns: 1,
                    memory_mode: league_relay::MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &ok_reply,
                None,
            ),
            &format!("case {label} should pass"),
        );
        assert_eq!(manifest.played_turns, 1, "case={label}");
        let rec = must_some(manifest.serving.as_ref(), "serving manifest");
        assert_eq!(
            rec.server_settings.reasoning_budget_message.as_deref(),
            expected_msg,
            "case={label}"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn budget_seat_with_absent_reasoning_mode_refuses_while_no_budget_seat_unchanged() {
    let tmp = fresh_tmp_dir("budget_absent_mode");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let mut budgeted_seats = local_foundry_three_clubbot_seats();
    budgeted_seats["seats"][0]["reasoning"] = json!({
        "effort": "medium",
        "budget_tokens": 256,
        "chat_template_kwargs": { "enable_thinking": true },
        "exclude_from_transcript": false
    });
    let no_budget_seats = local_foundry_three_clubbot_seats();

    let mut serving_absent_mode = base_llama_serving_json();
    serving_absent_mode["model"]["thinking_start_tag"] = json!("<think>");
    serving_absent_mode["model"]["thinking_end_tags"] = json!(["</think>"]);
    serving_absent_mode["server_settings"]["jinja"] = json!(true);
    serving_absent_mode["server_settings"]["reasoning_budget_message"] = json!("");

    let mut serving_blank_mode = serving_absent_mode.clone();
    serving_blank_mode["server_settings"]["reasoning_mode"] = json!("   ");

    // 1. Budget seat + absent/blank server_settings.reasoning_mode -> refuse zero calls.
    for (idx, (label, srv)) in [
        ("absent_mode", serving_absent_mode.clone()),
        ("blank_mode", serving_blank_mode),
    ]
    .into_iter()
    .enumerate()
    {
        let case_dir = tmp.join(label);
        let zero_tx = ZeroCallTransport::new();
        let game_dir = case_dir.join("game");
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: write_seats(&case_dir, budgeted_seats.clone()),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                serving_manifest_path: Some(write_serving_manifest_json(
                    &case_dir,
                    "serving.json",
                    &srv,
                )),
                arm: EvalArm::Off,
                table_id: 370 + idx as u64,
                seed: 7,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &zero_tx,
            None,
        ) {
            Err(e) => e,
            Ok(_) => {
                panic!("case '{label}' with budget seat must refuse UNENFORCEABLE_REASONING_BUDGET")
            }
        };
        assert_eq!(err.code(), "UNENFORCEABLE_REASONING_BUDGET", "case={label}");
        assert_eq!(err.exit_code(), 1, "case={label}");
        assert_eq!(zero_tx.call_count(), 0, "case={label}");
        assert!(!game_dir.exists(), "case={label}");
    }

    // 2. Non-budget seat + absent reasoning_mode & absent message (and "off") stay permitted.
    let mut serving_plain_absent = base_llama_serving_json();
    serving_plain_absent["server_settings"]["jinja"] = json!(true);
    let mut serving_off_absent_msg = serving_plain_absent.clone();
    serving_off_absent_msg["server_settings"]["reasoning_mode"] = json!("off");

    let ok_reply = FixedReplyTransport {
        content: json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "ok" })
            .to_string(),
    };
    for (idx, (label, srv, expected_mode)) in [
        ("no_budget_absent_all", serving_plain_absent, None),
        (
            "no_budget_off_absent_msg",
            serving_off_absent_msg,
            Some("off"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let case_dir = tmp.join(label);
        let manifest = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: case_dir.join("game"),
                    seats_path: write_seats(&case_dir, no_budget_seats.clone()),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    serving_manifest_path: Some(write_serving_manifest_json(
                        &case_dir,
                        "serving.json",
                        &srv,
                    )),
                    arm: EvalArm::Off,
                    table_id: 375 + idx as u64,
                    seed: 7,
                    turns: 1,
                    memory_mode: league_relay::MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &ok_reply,
                None,
            ),
            &format!("case {label} should remain permitted without budget seats"),
        );
        assert_eq!(manifest.played_turns, 1, "case={label}");
        let rec = must_some(manifest.serving.as_ref(), "serving manifest");
        assert_eq!(
            rec.server_settings.reasoning_mode.as_deref(),
            expected_mode,
            "case={label}"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}
