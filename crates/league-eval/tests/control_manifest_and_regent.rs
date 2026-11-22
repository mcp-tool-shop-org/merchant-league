//! Tests for the inert control arm, pinned wrapper, serving manifests, telemetry, and model-seat Regent rules.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use league_core::{EvalHost, Seat, TableId};
use league_eval::{
    BranchManifest, BranchOptions, EvalArm, EvalDecisionRecord, EvalError, EvalManifest,
    MetadataProvenance, NoopTransport, PINNED_WRAPPERS, PlayOptions, REJECTED_WRAPPER_HASHES,
    WordingArmDigests, check_wrapper_digest, eval_wording_slot, run_branch_with_transport,
    run_play_with_transport,
};
use league_relay::{RawHttpResponse, Transport, TransportError, sha256_hex};
use serde_json::{Value, json};

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
        "prompt_token_ids": [101, 200, 300],
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "token_ids": [11, 12],
            "logprobs": {
                "content": [{
                    "token": "{",
                    "id": 11,
                    "token_id": 11,
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

        let step = self.script_step.fetch_add(1, Ordering::SeqCst);
        let prompt_ids = vec![100_u64 + step as u64, 200, 300];
        let reply_ids = vec![11_u64, 12, 13];
        let logprob_cands: Vec<Value> = (0..7)
            .map(|k| {
                json!({
                    "token": format!("t{k}"),
                    "token_id": 500 + k,
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
            "id": format!("mock-resp-{step}"),
            "model": "Qwen/Qwen3-32B",
            "prompt_token_ids": prompt_ids,
            "choices": [{
                "index": 0,
                "message": message_obj,
                "token_ids": reply_ids,
                "logprobs": {
                    "content": [{
                        "token": "{",
                        "token_id": 11,
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
    assert!(!act_dec.reply_token_ids.is_empty());
    assert!(!act_dec.reply_top_logprobs.is_empty());
    // Capped at top-5 even though mock returned 7 candidates!
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
        assert_eq!(c, "Draft recorded. Nothing was checked or submitted.");
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
    assert_eq!(digests.control_arm, "v2_control_2026_11_21_r1");
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
        "v2_control_2026_11_21_r1"
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
    let bin = env!("CARGO_BIN_EXE_league-eval");
    let out = must_ok(
        Command::new(bin)
            .args([
                "play",
                "--game-dir",
                &sub_game.to_string_lossy(),
                "--seats",
                &clubbot_dir.to_string_lossy(),
                "--wrapper",
                &wrapper_path.to_string_lossy(),
                "--serving-manifest",
                &llama_ok_path.to_string_lossy(),
                "--arm",
                "control",
                "--turns",
                "1",
                "--seed",
                "7",
            ])
            .env("HOME", &fake_home_str)
            .env("USERPROFILE", &fake_home_str)
            .env("LLAMA_CACHE", &cache_dir_str)
            .env("CUDA_VISIBLE_DEVICES", "0")
            .env("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "0")
            .env("VLLM_WORKER_MULTIPROC_METHOD", "spawn")
            .env("VLLM_API_KEY", "sk-secret-vllm-key")
            .env("LLAMA_HF_TOKEN", "hf_secret_token")
            .env("CUDA_SECRET_KEY", "cuda-secret-val")
            .env("GGML_PASS_PHRASE", "ggml-secret-pass")
            .env("VLLM_AUTH_HEADER", "Bearer secret-auth")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .output(),
        "run child league-eval env snapshot",
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
fn fix2_s1_missing_telemetry_fields_fail_closed() {
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
    }

    let tmp = fresh_tmp_dir("fix2_s1_missing_telemetry");
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

    // Case 1: missing `prompt_token_ids` fails closed with `MISSING_REPLY_TELEMETRY`.
    let no_prompt_ids = json!({
        "id": "s1-no-prompt-ids",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
            "token_ids": [11],
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
    // Case 2: missing both `token_ids` and `logprobs.content[].id`/`token_id`.
    let no_reply_ids = json!({
        "id": "s1-no-reply-ids",
        "model": "Qwen/Qwen3-32B",
        "prompt_token_ids": [101, 102],
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
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
    // Case 3: missing `top_logprobs`.
    let no_top_logprobs = json!({
        "id": "s1-no-top-lp",
        "model": "Qwen/Qwen3-32B",
        "prompt_token_ids": [101, 102],
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": clean_talk },
            "token_ids": [11],
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

    for (idx, body) in [no_prompt_ids, no_reply_ids, no_top_logprobs]
        .into_iter()
        .enumerate()
    {
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: tmp.join(format!("s1_case_{idx}")),
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

    // Happy path with `llama-server` `id` inside `logprobs.content[]` (no top-level `token_ids`).
    let llama_style = json!({
        "id": "s1-llama-id",
        "model": "Qwen/Qwen3-32B",
        "prompt_token_ids": [101, 102, 103],
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
    let ok_game = tmp.join("s1_llama_id_ok");
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
            "read s1 decision",
        )),
        "parse s1 decision",
    );
    assert_eq!(rec.reply_token_ids, vec![vec![11]]);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn fix2_s2_subprocess_cuda_visible_devices_gpu_and_mig_never_verbatim() {
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

    let tmp = fresh_tmp_dir("fix2_s2_sub");
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

    let bin = env!("CARGO_BIN_EXE_league-eval");
    let out = must_ok(
        Command::new(bin)
            .args([
                "play",
                "--game-dir",
                &sub_game.to_string_lossy(),
                "--seats",
                &club_seats_path.to_string_lossy(),
                "--wrapper",
                &wrapper_path.to_string_lossy(),
                "--serving-manifest",
                &llama_ok_path.to_string_lossy(),
                "--arm",
                "control",
                "--turns",
                "1",
                "--seed",
                "7",
            ])
            .env("HOME", &fake_home)
            .env("USERPROFILE", &fake_home)
            .env("CUDA_VISIBLE_DEVICES", &gpu_uuid)
            .env("VLLM_MIG_UUID", &mig_uuid)
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .output(),
        "run child league-eval with GPU/MIG env vars",
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
fn fix2_s4_resume_refuses_mismatched_serving_manifest() {
    let tmp = fresh_tmp_dir("fix2_s4_resume_manifest");
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
fn fix2_s7_manifest_and_env_value_scrub_redacts_secrets_and_refuses_unsafe() {
    let tmp = fresh_tmp_dir("fix2_s7_scrub");
    let wrapper_path = copy_pinned_r3_wrapper(&tmp);
    let club_seats_path = write_seats(&tmp, all_clubbot_seats());

    let sk_secret = ["sk", "-", "live", "-", "0123456789abcdef"].concat();
    let model_filename = format!("/home/probeuser/models/{sk_secret}.gguf");
    let hf_secret = ["hf", "_", "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5"].concat();
    let os_build = format!("token={hf_secret}");
    let ghp_secret = ["ghp", "_", "99887766554433221100aabbccddeeff"].concat();
    let gh_pat_secret = ["github", "_", "pat", "_", "11223344556677889900aabbccdd"].concat();
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

    let bin = env!("CARGO_BIN_EXE_league-eval");
    let out = must_ok(
        Command::new(bin)
            .args([
                "play",
                "--game-dir",
                &game_dir.to_string_lossy(),
                "--seats",
                &club_seats_path.to_string_lossy(),
                "--wrapper",
                &wrapper_path.to_string_lossy(),
                "--serving-manifest",
                &manifest_path.to_string_lossy(),
                "--arm",
                "control",
                "--turns",
                "1",
                "--seed",
                "7",
            ])
            .env("HOME", &fake_home)
            .env("USERPROFILE", &fake_home)
            .env("VLLM_ENV_GHP", &ghp_secret)
            .env("VLLM_ENV_PAT", &gh_pat_secret)
            .env("LLAMA_ENV_BEARER", &bearer_secret)
            .env("LLAMA_ENV_KV", format!("{api_key_kv} {secret_kv}"))
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .output(),
        "run child league-eval for manifest and env scrubbing",
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
        "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5",
        ghp_secret.as_str(),
        "99887766554433221100aabbccddeeff",
        gh_pat_secret.as_str(),
        "11223344556677889900aabbccdd",
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
fn fix2_s8_regent_on_model_seat_or_prior_receipt_refuses_at_startup_zero_calls() {
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
    }

    let tmp = fresh_tmp_dir("fix2_s8_startup");
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
fn fix2_item9_hash_and_json_errors_never_echo_raw_values() {
    let tmp = fresh_tmp_dir("fix2_item9_no_echo");

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
