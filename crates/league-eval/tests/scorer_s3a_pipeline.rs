//! End-to-end pipeline, talk-extractor, first-draft, freeze, and join-gate tests for S3a.
//!
//! All tests use deterministic synthetic fixtures and local mock transports; no external run
//! directory or real model endpoint is ever touched.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use league_core::{
    AssetId, EvalActSubmission, EvalHost, MarketAction, OfferProposal, OfferSide,
    SealedOrderBundle, Seat, TableId, ToStructuredError as _,
};
use league_eval::{
    ArmMapStateFile, BranchManifest, BranchOptions, BuildBlindInputsOptions, CRATE_GIT_SHA,
    CrateGitShaSource, DESCRIPTIVE_ONLY_INFERENTIAL_STATUS, EVAL_REPLY_WORDING_ARM, EvalArm,
    EvalDecisionRecord, EvalError, EvalManifest, EvalSetupFile, FreezeOptions, JoinOptions,
    JoinedDecisionsFile, PerArmSummaryFile, PlayOptions, SCENARIO_RANDOMNESS_SCOPE_STATEMENT,
    ScorerFreezeFile, TalkRulesV2Table, VerifyCheckpointsOptions, eval_wording_slot,
    execute_cli_with_git_sha, extract_talk_commitments_for_state,
    extract_talk_commitments_from_messages, extract_talk_commitments_from_messages_with_rules,
    extract_talk_commitments_with_counts_for_state, resolve_eval_seat_guilds,
    run_branch_with_transport_and_git_sha, run_build_blind_inputs, run_freeze, run_join,
    run_play_with_transport_and_git_sha, run_verify_checkpoints, talk_rules_v2_digest,
    talk_rules_v2_digest_for, write_freeze_file_with_writer,
    write_freeze_file_with_writer_and_remover,
};
use league_relay::{
    CONTROL_WORDING_ARM, NegotiationInputs, PREVIEW_WORDING_ARM, RawHttpResponse, Round,
    SavedToolRoundCall, SavedToolRoundRecord, Transport, TransportError, UntrustedWhisper,
};
use league_scorer::cli::run_cli as run_scorer_cli;
use league_scorer::{
    BlindScoringRequest, CandidateScoreOutput, ScorerConfig, ScorerError, StateScoreOutput,
    TALK_EXTRACTOR_RULES_DIGEST_V2, TALK_EXTRACTOR_VERSION_V2, TalkExtractionCounts,
    TalkSourceCounts, candidate_id_for_submission, compute_blind_inputs_manifest_sha256,
    sha256_hex, to_sorted_compact_json, to_sorted_pretty_json,
};
use league_seat_types::UNIT_SCALE;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const TEST_FALLBACK_GIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ml_s3a_{label}_{}_{}", std::process::id(), seq));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
fn clear_readonly_recursive(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mode = if meta.is_dir() { 0o755 } else { 0o644 };
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                clear_readonly_recursive(&entry.path());
            }
        }
    }
}

#[cfg(not(unix))]
fn clear_readonly_recursive(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                clear_readonly_recursive(&entry.path());
            }
        }
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        clear_readonly_recursive(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn verify_ckpts_for_test(
    ckpts_dir: &Path,
    out_path: &Path,
    blind_dir: Option<&Path>,
    git_sha: &CrateGitShaSource<'_>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let _ = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir.to_path_buf(),
            out_path: out_path.to_path_buf(),
            blind_dir: blind_dir.map(Path::to_path_buf),
        },
        git_sha,
    )?;
    Ok(out_path.to_path_buf())
}

fn clean_git_sha_source() -> CrateGitShaSource<'static> {
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

fn run_eval_cli(args: &[&str], git_sha: &CrateGitShaSource<'_>) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = execute_cli_with_git_sha(
        args.iter().copied().map(OsString::from),
        &mut out,
        &mut err,
        git_sha,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn copy_pinned_wrapper(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("wrappers/v2_seat_brief_2026_11_21_r3.md");
    let dst = dir.join("wrapper.md");
    fs::write(&dst, fs::read(&src)?)?;
    Ok(dst)
}

fn write_two_model_two_bot_seats(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = dir.join("seats.json");
    let v = json!({
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
                "base_url": "http://127.0.0.1:18080/v1",
                "model": "Qwen/Qwen3-32B",
                "temperature": 0.0,
                "max_output_tokens": 512,
                "timeout_secs": 5.0,
                "max_gpu_secs_per_call": 10.0
            },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    });
    fs::write(&p, serde_json::to_string_pretty(&v)?)?;
    Ok(p)
}

fn write_serving_manifest(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = dir.join("serving.json");
    let v = json!({
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
            "gpu_name": "NVIDIA RTX Synthetic",
            "gpu_uuid": "GPU-synth-0001",
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
            "temperature": 0.0,
            "top_k": 1,
            "top_p": 1.0,
            "min_p": 0.0,
            "repeat_penalty": 1.0,
            "presence_penalty": 0.0,
            "frequency_penalty": 0.0,
            "seed": 7,
            "stop_sequences": [],
            "max_tokens": 512,
            "sampler_order": ["temperature"],
            "provenance": "operator_supplied"
        }
    });
    fs::write(&p, serde_json::to_string_pretty(&v)?)?;
    Ok(p)
}

fn mock_chat_response(content_obj: Value) -> Result<RawHttpResponse, TransportError> {
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
    let resp = json!({
        "id": "mock-s3a",
        "model": "Qwen/Qwen3-32B",
        "choices": [{
            "index": 0,
            "message": content_obj,
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

struct SyntheticPipelineTransport;

impl Transport for SyntheticPipelineTransport {
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
        let parsed: Value = serde_json::from_str(body)
            .map_err(|e| TransportError::Network(format!("bad req json: {e}")))?;
        let user_msg = parsed
            .get("messages")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or_default();
        let is_act = user_msg.contains(eval_wording_slot("eval_act_instruction"));
        let is_quarry = user_msg.contains("{\n  \"seat\": 1,");
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

        if !is_act {
            // Talk round: Quarry sends an unambiguous trade commitment to Foundry; Foundry sends none.
            let messages = if is_quarry {
                json!([
                    { "to_guild": "Foundry", "body": "I will sell 1000 raw 1 at 250 CRED." }
                ])
            } else {
                json!([])
            };
            let content = json!({
                "messages": messages,
                "market_actions": [],
                "bundle": {},
                "notes": "talk_round"
            })
            .to_string();
            return mock_chat_response(json!({ "role": "assistant", "content": content }));
        }

        // Act round: tool arms execute a first draft (control uses an illegal draft to exercise
        // rule-refused first-draft capture, then a legal second draft), followed by distinct commits.
        if !tool_name.is_empty() && tool_msgs_count == 0 {
            let is_control_tool = tool_name == "draft_turn_orders";
            let draft_args = if is_control_tool {
                // Excessive debt repayment exceeds free CRED, triggering a rule refusal on preview/check.
                json!({
                    "market_actions": [{
                        "action_kind": "post_offer",
                        "side": "sell",
                        "good": { "asset_kind": "raw", "good_index": 0 },
                        "amount": 9_999_999,
                        "price_per_unit_cred": 150
                    }],
                    "bundle": {}
                })
                .to_string()
            } else {
                json!({
                    "market_actions": [],
                    "bundle": { "debt_repayment_cred": 25 }
                })
                .to_string()
            };
            return mock_chat_response(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": draft_args
                    }
                }]
            }));
        }

        if !tool_name.is_empty() && tool_msgs_count == 1 {
            let second_args = json!({
                "market_actions": [],
                "bundle": { "debt_repayment_cred": 15 }
            })
            .to_string();
            return mock_chat_response(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": "call_2",
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": second_args
                    }
                }]
            }));
        }

        let commit_content = if tool_name == "draft_turn_orders" {
            json!({
                "messages": [],
                "market_actions": [],
                "bundle": { "debt_repayment_cred": 10 },
                "notes": "ctrl_commit"
            })
        } else if tool_name == "preview_turn_orders" {
            json!({
                "messages": [],
                "market_actions": [{
                    "action_kind": "town_import",
                    "good": { "asset_kind": "raw", "good_index": 0 },
                    "amount": 10
                }],
                "bundle": {},
                "notes": "on_commit"
            })
        } else {
            json!({
                "messages": [],
                "market_actions": [{
                    "action_kind": "town_import",
                    "good": { "asset_kind": "raw", "good_index": 0 },
                    "amount": 5
                }],
                "bundle": {},
                "notes": "off_commit"
            })
        };
        mock_chat_response(json!({
            "role": "assistant",
            "content": commit_content.to_string()
        }))
    }

    fn apply_template(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<String, TransportError> {
        Ok(format!("RENDERED:{body}"))
    }

    fn tokenize_prompt(
        &self,
        _url: &str,
        _content: &str,
        add_special: bool,
        parse_special: bool,
        _timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        assert!(add_special && parse_special);
        Ok((100_u64..220).collect())
    }
}

fn fast_test_scorer_config() -> ScorerConfig {
    ScorerConfig {
        scenarios_m: 2,
        k_synth_max: 2,
        ..ScorerConfig::default()
    }
}

fn assert_blind_bytes_clean(blind_dir: &Path, tmp_root: &Path) -> TestResult {
    let tmp_leaf = tmp_root
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("non-utf8 tmp leaf")?;
    let mut file_count = 0_usize;
    for entry in fs::read_dir(blind_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        file_count += 1;
        let text = fs::read_to_string(&path)?;
        let lower = text.to_ascii_lowercase();
        for forbidden_substr in [
            "\"off\"",
            "\"control\"",
            "\"on\"",
            "v2_control",
            "v2_preview",
            "v2_eval_reply",
            "wording_arm",
            "relay_wording_arm",
            "replicate",
            "decision_path",
            "outcome_path",
            "first_draft",
            "tool_round",
            "raw_reply",
            "reasoning",
            "decision.json",
            "outcome.json",
            &tmp_leaf.to_ascii_lowercase(),
        ] {
            assert!(
                !lower.contains(forbidden_substr),
                "blind input `{}` leaked `{forbidden_substr}`",
                path.display()
            );
        }
        assert!(
            !text.contains("\\\\") && !text.contains(":/"),
            "blind input `{}` leaked filesystem path separator",
            path.display()
        );
    }
    assert!(file_count > 0, "expected >= 1 blind_inputs json file");
    Ok(())
}

#[test]
fn e2e_synthetic_play_branch_blind_score_freeze_and_join_pipeline() -> TestResult {
    let tmp = TmpDir::new("e2e")?;
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let git_sha = clean_git_sha_source();
    let transport = SyntheticPipelineTransport;

    // 1. Build a 1-turn synthetic play run where Quarry whispers an unambiguous commitment to Foundry.
    let play_dir = tmp.path().join("play_game");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Off,
            table_id: 9101,
            seed: 17,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    assert_eq!(play_manifest.played_turns, 1);

    // 2. Branch day-01-pre-act for Foundry across off, control (r2), and on.
    let branch_dir = tmp.path().join("branch_run");
    let outcomes = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    assert_eq!(outcomes.len(), 3);
    let ctrl_manifest: EvalManifest = serde_json::from_str(&fs::read_to_string(
        branch_dir.join("control/manifest.json"),
    )?)?;
    assert_eq!(ctrl_manifest.relay_wording_arm, CONTROL_WORDING_ARM);
    let _branch_manifest: BranchManifest =
        serde_json::from_str(&fs::read_to_string(branch_dir.join("manifest.json"))?)?;

    // 3. Run build-blind-inputs.
    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("scorer_config.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let blind_dir = tmp.path().join("blind_inputs");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_cas_dir = tmp.path().join("checkpoints_cas");
    let (build_code, build_out, build_err) = run_eval_cli(
        &[
            "build-blind-inputs",
            "--run",
            play_dir.to_str().ok_or("utf8")?,
            branch_dir.to_str().ok_or("utf8")?,
            "--out-blind",
            blind_dir.to_str().ok_or("utf8")?,
            "--out-arm-map",
            arm_map_dir.to_str().ok_or("utf8")?,
            "--out-checkpoints",
            ckpts_cas_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--scorer-seed",
            "42",
        ],
        &git_sha,
    );
    assert_eq!(build_code, 0, "build-blind-inputs stderr: {build_err}");
    assert!(build_out.contains("states_built=1 arm_entries_mapped=3"));
    eprintln!("E2E[build-blind-inputs]: {}", build_out.trim());

    // Grep raw bytes of blind_inputs/ to verify zero arm, path, or wording labels.
    assert_blind_bytes_clean(&blind_dir, tmp.path())?;

    let expected_state_id = format!("{}-day-01-seat-0", play_manifest.game_id);
    let blind_raw_bytes = fs::read(blind_dir.join(format!("{expected_state_id}.json")))?;
    let expected_blind_sha = sha256_hex(&blind_raw_bytes);
    let blind_req: BlindScoringRequest = serde_json::from_slice(&blind_raw_bytes)?;
    assert_eq!(blind_req.talk_commitments.len(), 1);
    assert_eq!(blind_req.talk_commitments[0].proposer_seat, 1);
    assert_eq!(blind_req.talk_commitments[0].counterparty_seat, 0);
    assert_eq!(
        blind_req.talk_commitments[0].offered_action,
        MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(1),
                amount: 1_000 * UNIT_SCALE,
                price_per_unit_cred: 250 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        }
    );

    let arm_map: ArmMapStateFile = serde_json::from_slice(&fs::read(
        arm_map_dir.join(format!("{expected_state_id}.json")),
    )?)?;
    assert_eq!(arm_map.entries.len(), 3);
    let off_entry = arm_map
        .entries
        .iter()
        .find(|e| e.arm == EvalArm::Off)
        .ok_or("missing off entry")?;
    let ctrl_entry = arm_map
        .entries
        .iter()
        .find(|e| e.arm == EvalArm::Control)
        .ok_or("missing control entry")?;
    let on_entry = arm_map
        .entries
        .iter()
        .find(|e| e.arm == EvalArm::On)
        .ok_or("missing on entry")?;
    assert!(off_entry.first_draft_candidate_id.is_none());
    let ctrl_draft_cid = ctrl_entry
        .first_draft_candidate_id
        .as_deref()
        .ok_or("expected rule-refused first draft for control arm")?;
    let on_draft_cid = on_entry
        .first_draft_candidate_id
        .as_deref()
        .ok_or("expected first draft for on arm")?;

    // Verify control's rule-refused first draft sits in C(state) outside [v_min, v_max],
    // while committed decisions sit inside [v_min, v_max].
    let ctrl_draft_cand = blind_req
        .candidates
        .iter()
        .find(|c| c.candidate_id == ctrl_draft_cid)
        .ok_or("control first draft missing from blind candidates")?;
    assert!(!ctrl_draft_cand.in_span);
    assert!(!ctrl_draft_cand.is_arm_independent);

    let ctrl_commit_cand = blind_req
        .candidates
        .iter()
        .find(|c| c.candidate_id == ctrl_entry.committed_candidate_id)
        .ok_or("control committed missing")?;
    assert!(ctrl_commit_cand.in_span);
    let on_draft_cand = blind_req
        .candidates
        .iter()
        .find(|c| c.candidate_id == on_draft_cid)
        .ok_or("on draft missing")?;
    assert!(!on_draft_cand.in_span);

    // 4. Score twice without freeze -> byte-identical scores/*.json.
    let scores_dir_a = tmp.path().join("scores_a");
    let scores_dir_b = tmp.path().join("scores_b");
    for (tag, out_dir) in [
        ("score_run_1", &scores_dir_a),
        ("score_run_2", &scores_dir_b),
    ] {
        let mut s_out = Vec::new();
        let mut s_err = Vec::new();
        let code = run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_cas_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut s_out,
            &mut s_err,
        );
        assert_eq!(code, 0, "{tag} stderr: {}", String::from_utf8_lossy(&s_err));
        eprintln!("E2E[{tag}]: {}", String::from_utf8_lossy(&s_out).trim());
    }
    let bytes_a = fs::read(scores_dir_a.join(format!("scores/{expected_state_id}.json")))?;
    let bytes_b = fs::read(scores_dir_b.join(format!("scores/{expected_state_id}.json")))?;
    assert_eq!(bytes_a, bytes_b, "two scorer passes must be byte-identical");

    // 5. Verify checkpoints (Gate 2), write freeze file, and run scorer WITH `--freeze`.
    let vc_path = tmp.path().join("verified_checkpoints.json");
    let (vc_code, vc_out, vc_err) = run_eval_cli(
        &[
            "verify-checkpoints",
            "--checkpoints",
            ckpts_cas_dir.to_str().ok_or("utf8")?,
            "--blind",
            blind_dir.to_str().ok_or("utf8")?,
            "--out",
            vc_path.to_str().ok_or("utf8")?,
        ],
        &git_sha,
    );
    assert_eq!(vc_code, 0, "verify-checkpoints stderr: {vc_err}");
    assert!(vc_out.contains("verified_count=1"));

    let freeze_path = tmp.path().join("scorer_freeze.json");
    let (freeze_code, freeze_out, freeze_err) = run_eval_cli(
        &[
            "freeze",
            "--blind",
            blind_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--control-wording-arm",
            CONTROL_WORDING_ARM,
            "--verified-checkpoints",
            vc_path.to_str().ok_or("utf8")?,
            "--out",
            freeze_path.to_str().ok_or("utf8")?,
        ],
        &git_sha,
    );
    assert_eq!(freeze_code, 0, "freeze stderr: {freeze_err}");
    assert!(freeze_out.contains("config_sha256="));
    eprintln!("E2E[freeze]: {}", freeze_out.trim());

    let freeze: ScorerFreezeFile = serde_json::from_slice(&fs::read(&freeze_path)?)?;
    assert_eq!(freeze.control_wording_arm, CONTROL_WORDING_ARM);
    assert_eq!(freeze.talk_extractor_version, "talk_rules_v2");
    assert_eq!(freeze.verified_checkpoints_sha256.len(), 64);

    let scores_frozen = tmp.path().join("scores_frozen");
    let mut sf_out = Vec::new();
    let mut sf_err = Vec::new();
    let sf_code = run_scorer_cli(
        [
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_cas_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_path.to_str().ok_or("utf8")?,
            "--out",
            scores_frozen.to_str().ok_or("utf8")?,
        ],
        &mut sf_out,
        &mut sf_err,
    );
    assert_eq!(
        sf_code,
        0,
        "frozen score stderr: {}",
        String::from_utf8_lossy(&sf_err)
    );
    eprintln!(
        "E2E[score --freeze]: {}",
        String::from_utf8_lossy(&sf_out).trim()
    );
    let frozen_score_bytes =
        fs::read(scores_frozen.join(format!("scores/{expected_state_id}.json")))?;
    assert_eq!(frozen_score_bytes, bytes_a);
    let frozen_score_parsed: StateScoreOutput = serde_json::from_slice(&frozen_score_bytes)?;
    assert_eq!(frozen_score_parsed.blind_request_sha256, expected_blind_sha);

    // 6. Run join gate and verify each arm's committed_candidate_id reproduces accurately.
    let joined_dir = tmp.path().join("joined");
    let (join_code, join_out, join_err) = run_eval_cli(
        &[
            "join",
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--arm-map",
            arm_map_dir.to_str().ok_or("utf8")?,
            "--scores",
            scores_frozen.to_str().ok_or("utf8")?,
            "--blind",
            blind_dir.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_path.to_str().ok_or("utf8")?,
            "--out",
            joined_dir.to_str().ok_or("utf8")?,
        ],
        &git_sha,
    );
    assert_eq!(join_code, 0, "join stderr: {join_err}");
    assert!(join_out.contains("states_joined=1 decisions_joined=3"));
    eprintln!("E2E[join]: {}", join_out.trim());

    let joined_decisions: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(joined_dir.join("joined_decisions.json"))?)?;
    assert_eq!(
        joined_decisions.inferential_status,
        DESCRIPTIVE_ONLY_INFERENTIAL_STATUS
    );
    assert_eq!(
        joined_decisions.verified_checkpoints_sha256,
        freeze.verified_checkpoints_sha256
    );
    assert_eq!(
        joined_decisions.scorer_engine_build_id,
        freeze.scorer_git_sha
    );
    assert_eq!(
        joined_decisions.scenario_randomness_scope,
        SCENARIO_RANDOMNESS_SCOPE_STATEMENT
    );
    assert_eq!(joined_decisions.rows.len(), 3);
    for arm_entry in &arm_map.entries {
        let row = joined_decisions
            .rows
            .iter()
            .find(|r| r.arm == arm_entry.arm && r.replicate == arm_entry.replicate)
            .ok_or("missing arm row in joined_decisions.json")?;
        assert_eq!(row.committed_candidate_id, arm_entry.committed_candidate_id);
        assert_eq!(
            row.first_draft_candidate_id,
            arm_entry.first_draft_candidate_id
        );
        assert_eq!(row.checkpoint_engine_commit.len(), 40);
    }

    let summary_file: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(joined_dir.join("per_arm_summary.json"))?)?;
    assert_eq!(
        summary_file.inferential_status,
        DESCRIPTIVE_ONLY_INFERENTIAL_STATUS
    );
    assert_eq!(
        summary_file.verified_checkpoints_sha256,
        freeze.verified_checkpoints_sha256
    );
    assert_eq!(summary_file.scorer_engine_build_id, freeze.scorer_git_sha);
    assert_eq!(
        summary_file.scenario_randomness_scope,
        SCENARIO_RANDOMNESS_SCOPE_STATEMENT
    );
    assert_eq!(summary_file.arms.len(), 3);
    Ok(())
}

#[test]
fn first_draft_call_index_ordering_no_reply_refusals_and_or_merge() -> TestResult {
    let tmp = TmpDir::new("draft_edges")?;
    let ckpt_dir = tmp.path().join("src_game");
    let ckpts = ckpt_dir.join("checkpoints");
    fs::create_dir_all(&ckpts)?;
    let mut host = EvalHost::from_standard_setup(TableId(9102), 4, 1, 21)?
        .with_engine_commit(TEST_FALLBACK_GIT_SHA)?;
    let obs = host.observe_seat(Seat(0))?;
    let (save_path, seal_path) = host.write_pre_act_checkpoint(&ckpts)?;
    assert!(save_path.is_file() && seal_path.is_file());
    fs::rename(
        ckpts.join("day-01-pre-act.save.json"),
        ckpts.join("day-01-pre-act.save.json"),
    )?;

    // Advance one turn so we can confirm season setup + manifest exist for resolve_eval_seat_guilds.
    let source_setup = EvalSetupFile {
        game_id: host.game_id().to_owned(),
        table_id: 9102,
        seed: 21,
        turns: 1,
        seat_count: 4,
        arm: EvalArm::Off,
        eval_reply_wording_arm: EVAL_REPLY_WORDING_ARM.to_owned(),
        relay_wording_arm: PREVIEW_WORDING_ARM.to_owned(),
        wrapper_version: "v2_seat_brief_2026_11_21_r3".to_owned(),
        wrapper_sha256: "a".repeat(64),
        memory_mode: "snapshot".to_owned(),
        seats_sha256: "b".repeat(64),
        response_schema_sha256: None,
        serving_manifest_sha256: None,
    };
    fs::write(
        ckpt_dir.join("setup.json"),
        serde_json::to_string_pretty(&source_setup)?,
    )?;

    // Advance host via ClubBot + generate dummy receipt for BranchArmOutcome fixture.
    let bot_subs: BTreeMap<u8, EvalActSubmission> = (0_u8..4)
        .map(|s| Ok((s, EvalActSubmission::default())))
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;
    host.apply_act_submissions(&bot_subs)?;
    let receipt = host.sweep_turn()?;

    let make_decision =
        |arm: EvalArm, no_reply: bool, raw_reply: Option<&str>| EvalDecisionRecord {
            game_id: host.game_id().to_owned(),
            table_id: 9102,
            day: 1,
            round: Round::Act,
            seat: 0,
            guild: "Foundry".to_owned(),
            arm,
            seat_kind: "Model".to_owned(),
            keeps_notes: true,
            notes_present: false,
            raw_reply: raw_reply.map(str::to_owned),
            discarded_reply_path: None,
            discarded_reply_sha256: None,
            attempts: 1,
            failed_attempts: Vec::new(),
            prompt_tokens: 10,
            completion_tokens: 5,
            wall_secs: Some(0.1),
            tool_calls_used: 0,
            cleared_non_empty_actions: false,
            prompt_token_ids_sha256s: Vec::new(),
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
            reply_token_ids: Vec::new(),
            reply_top_logprobs: Vec::new(),
            intermediate_bundle_sha256s: Vec::new(),
            refused_market_actions: 0,
            refused_bundle_legs: 0,
            parse_failure: no_reply,
            refusal_code: None,
            refusal_reason: None,
            loaded_history_sha256: BTreeMap::new(),
            invalid_recipients: Vec::new(),
            no_reply,
            apply_report: None,
            refused_bundle: None,
            club_bot_baseline: None,
        };

    // Decision JSON also supplies seat guilds for `resolve_eval_seat_guilds` when manifest.json absent.
    let decisions_dir = ckpt_dir.join("decisions");
    fs::create_dir_all(&decisions_dir)?;
    for (seat, guild) in [(0_u8, "Foundry"), (1, "Quarry"), (2, "Harbor"), (3, "Mint")] {
        let mut d = make_decision(EvalArm::Off, true, None);
        d.seat = seat;
        d.guild = guild.to_owned();
        d.round = Round::Talk;
        fs::write(
            decisions_dir.join(format!("day-01-talk-{guild}.json")),
            serde_json::to_string_pretty(&d)?,
        )?;
    }

    let branch_dir = tmp.path().join("branch_edges");
    // Arm 1: `off` has no_reply == true -> committed defaults to passive.
    let off_dir = branch_dir.join("off");
    fs::create_dir_all(&off_dir)?;
    let off_dec = make_decision(EvalArm::Off, true, None);
    let off_out = league_eval::BranchArmOutcome {
        source_game_id: host.game_id().to_owned(),
        table_id: 9102,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        arm: EvalArm::Off,
        decision: off_dec.clone(),
        receipt: receipt.clone(),
    };
    fs::write(
        off_dir.join("decision.json"),
        serde_json::to_string_pretty(&off_dec)?,
    )?;
    fs::write(
        off_dir.join("outcome.json"),
        serde_json::to_string_pretty(&off_out)?,
    )?;

    // Arm 2: `control` has a whole-reply refusal (wrong-part messages non-empty) AND an out-of-order
    // tool-round file verifying `call.call_index` is read (selecting call_index 1 after skipping
    // call_index 0 CALL_LIMIT_REACHED and call_index 2 MALFORMED_TOOL_CALL while preserving rule refusal).
    let ctrl_dir = branch_dir.join("control");
    let ctrl_seats = ctrl_dir.join("relay/seats");
    fs::create_dir_all(&ctrl_seats)?;
    let wrong_part_raw = json!({
        "messages": [{ "to_guild": "Quarry", "body": "oops" }],
        "market_actions": [],
        "bundle": {}
    })
    .to_string();
    let ctrl_dec = make_decision(EvalArm::Control, false, Some(&wrong_part_raw));
    let ctrl_out = league_eval::BranchArmOutcome {
        source_game_id: host.game_id().to_owned(),
        table_id: 9102,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        arm: EvalArm::Control,
        decision: ctrl_dec.clone(),
        receipt: receipt.clone(),
    };
    fs::write(
        ctrl_dir.join("decision.json"),
        serde_json::to_string_pretty(&ctrl_dec)?,
    )?;
    fs::write(
        ctrl_dir.join("outcome.json"),
        serde_json::to_string_pretty(&ctrl_out)?,
    )?;

    // Out-of-order Vec: array[0] has call_index=3, array[1] has call_index=1 (wanted rule-refused),
    // array[2] has call_index=0 (CALL_LIMIT_REACHED), array[3] has call_index=2 (MALFORMED_TOOL_CALL).
    let wanted_args = json!({
        "market_actions": [{
            "action_kind": "town_import",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 7
        }],
        "bundle": {}
    })
    .to_string();
    let other_args = json!({
        "market_actions": [],
        "bundle": { "debt_repayment_cred": 99 }
    })
    .to_string();
    let mk_call =
        |call_index: usize, sent: bool, wire: Option<&str>, args: &str| SavedToolRoundCall {
            call_index,
            id: format!("id_{call_index}"),
            kind: "function".to_owned(),
            name: "draft_turn_orders".to_owned(),
            arguments: args.to_owned(),
            raw_received_call: json!({}),
            call_sha256: "a".repeat(64),
            arguments_sha256: "b".repeat(64),
            result_wire_json: wire.map(str::to_owned),
            result_sha256: wire.map(|_| "c".repeat(64)),
            tool_result_sent: sent,
        };
    let tool_rec = SavedToolRoundRecord {
        day: "01".to_owned(),
        round: Round::Act,
        guild: "Foundry".to_owned(),
        run_id: "r1".to_owned(),
        reservation_id: "resv1".to_owned(),
        request_body_sha256: "c".repeat(64),
        round_index: 0,
        attempt: 1,
        outcome: "TOOL_CALL".to_owned(),
        assistant_content: None,
        assistant_content_sha256: None,
        reasoning: None,
        assistant_reply_sha256: "d".repeat(64),
        body_redacted: false,
        calls: vec![
            mk_call(3, true, Some(r#"{"status":"ok"}"#), &other_args),
            mk_call(
                1,
                true,
                Some(r#"{"status":"refused","error_code":"INSUFFICIENT_BALANCE"}"#),
                &wanted_args,
            ),
            mk_call(
                0,
                true,
                Some(r#"{"error_code":"CALL_LIMIT_REACHED"}"#),
                &other_args,
            ),
            mk_call(
                2,
                true,
                Some(r#"{"error_code":"MALFORMED_TOOL_CALL"}"#),
                "{bad",
            ),
        ],
    };
    fs::write(
        ctrl_seats.join("Foundry.day-01-act.tool-round-0.json"),
        serde_json::to_string_pretty(&tool_rec)?,
    )?;

    // Arm 3: `on` commits exact same submission as its first draft (OR-merging `in_span = false` + `true` => `true`)!
    let on_dir = branch_dir.join("on");
    let on_seats = on_dir.join("relay/seats");
    fs::create_dir_all(&on_seats)?;
    let on_commit_raw = json!({
        "messages": [],
        "market_actions": [{
            "action_kind": "town_import",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 12
        }],
        "bundle": {},
        "notes": "on_commit_notes"
    })
    .to_string();
    let on_draft_args = json!({
        "market_actions": [{
            "action_kind": "town_import",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 12
        }],
        "bundle": {}
    })
    .to_string();
    let on_dec = make_decision(EvalArm::On, false, Some(&on_commit_raw));
    let on_out = league_eval::BranchArmOutcome {
        source_game_id: host.game_id().to_owned(),
        table_id: 9102,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        arm: EvalArm::On,
        decision: on_dec.clone(),
        receipt,
    };
    fs::write(
        on_dir.join("decision.json"),
        serde_json::to_string_pretty(&on_dec)?,
    )?;
    fs::write(
        on_dir.join("outcome.json"),
        serde_json::to_string_pretty(&on_out)?,
    )?;
    let on_tool_rec = SavedToolRoundRecord {
        day: "01".to_owned(),
        round: Round::Act,
        guild: "Foundry".to_owned(),
        run_id: "r2".to_owned(),
        reservation_id: "resv2".to_owned(),
        request_body_sha256: "e".repeat(64),
        round_index: 0,
        attempt: 1,
        outcome: "TOOL_CALL".to_owned(),
        assistant_content: None,
        assistant_content_sha256: None,
        reasoning: None,
        assistant_reply_sha256: "f".repeat(64),
        body_redacted: false,
        calls: vec![
            mk_call(0, false, None, &other_args),
            mk_call(1, true, Some(r#"{"status":"ok"}"#), &on_draft_args),
        ],
    };
    fs::write(
        on_seats.join("Foundry.day-01-act.tool-round-0.json"),
        serde_json::to_string_pretty(&on_tool_rec)?,
    )?;

    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let report = run_build_blind_inputs(&league_eval::BuildBlindInputsOptions {
        run_dirs: vec![ckpt_dir.clone(), branch_dir],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: None,
        config_path: cfg_path,
        scorer_seed: 7,
    })?;
    assert_eq!(report.states_built, 1);
    assert_eq!(report.arm_entries_mapped, 3);

    let sid = format!("{}-day-01-seat-0", host.game_id());
    let arm_map: ArmMapStateFile =
        serde_json::from_slice(&fs::read(arm_map_dir.join(format!("{sid}.json")))?)?;
    let passive_cid = candidate_id_for_submission(&EvalActSubmission::default())?;
    let expected_ctrl_draft_cid = candidate_id_for_submission(&EvalActSubmission {
        market_actions: vec![MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(0),
            amount: 7,
        }],
        pass_b_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    })?;
    let expected_on_cid = candidate_id_for_submission(&EvalActSubmission {
        market_actions: vec![MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(0),
            amount: 12,
        }],
        pass_b_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    })?;

    for e in &arm_map.entries {
        match e.arm {
            EvalArm::Off => assert_eq!(e.committed_candidate_id, passive_cid),
            EvalArm::Control => {
                assert_eq!(e.committed_candidate_id, passive_cid);
                assert_eq!(
                    e.first_draft_candidate_id.as_deref(),
                    Some(expected_ctrl_draft_cid.as_str())
                );
            }
            EvalArm::On => {
                assert_eq!(e.committed_candidate_id, expected_on_cid);
                assert_eq!(
                    e.first_draft_candidate_id.as_deref(),
                    Some(expected_on_cid.as_str())
                );
            }
        }
    }

    let req: BlindScoringRequest =
        serde_json::from_slice(&fs::read(blind_dir.join(format!("{sid}.json")))?)?;
    let on_cand = req
        .candidates
        .iter()
        .find(|c| c.candidate_id == expected_on_cid)
        .ok_or("missing on candidate")?;
    assert!(
        on_cand.in_span && !on_cand.is_arm_independent,
        "matching first_draft + committed must OR-merge in_span to true"
    );
    let ctrl_draft_cand = req
        .candidates
        .iter()
        .find(|c| c.candidate_id == expected_ctrl_draft_cid)
        .ok_or("missing ctrl draft")?;
    assert!(!ctrl_draft_cand.in_span);
    let _ = obs;
    Ok(())
}

#[test]
fn talk_extractor_files_cover_clear_ambiguous_retraction_and_repeat_determinism() -> TestResult {
    let tmp = TmpDir::new("talk_files")?;
    let host = EvalHost::from_standard_setup(TableId(9103), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Mint".to_owned(),
    ];

    let write_whisper_case = |dir: &Path, whispers: Vec<UntrustedWhisper>| -> TestResult {
        let to_seat = dir.join("to-seat");
        fs::create_dir_all(&to_seat)?;
        let neg = NegotiationInputs {
            batna: "batna".to_owned(),
            whispers,
        };
        fs::write(
            to_seat.join("day-01-act-Foundry.negotiation.json"),
            serde_json::to_string_pretty(&neg)?,
        )?;
        Ok(())
    };

    // 1. Clear offer via negotiation.json gives 1 commitment.
    let case_clear = tmp.path().join("clear");
    write_whisper_case(
        &case_clear,
        vec![UntrustedWhisper {
            from_guild: "Quarry".to_owned(),
            intent_header: None,
            body: "I commit to buy 500 refined Foundry at 400 CRED.".to_owned(),
        }],
    )?;
    let c1 = extract_talk_commitments_for_state(&case_clear, 1, 0, &seat_guilds, &obs)?;
    let c2 = extract_talk_commitments_for_state(&case_clear, 1, 0, &seat_guilds, &obs)?;
    assert_eq!(c1.len(), 1);
    assert_eq!(
        c1[0].offered_action,
        MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Refined(0),
                amount: 500 * UNIT_SCALE,
                price_per_unit_cred: 400 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        }
    );
    assert_eq!(to_sorted_pretty_json(&c1)?, to_sorted_pretty_json(&c2)?);

    // 2. Ambiguous offer in decisions/day-01-talk-Quarry.json gives 0 commitments.
    let case_ambig = tmp.path().join("ambig");
    let dec_dir = case_ambig.join("decisions");
    fs::create_dir_all(&dec_dir)?;
    let ambig_reply = json!({
        "messages": [{ "to_guild": "Harbor", "body": "Maybe I will buy 500 refined Foundry for 400 CRED." }],
        "market_actions": [],
        "bundle": {}
    })
    .to_string();
    let talk_dec = EvalDecisionRecord {
        game_id: host.game_id().to_owned(),
        table_id: 9103,
        day: 1,
        round: Round::Talk,
        seat: 1,
        guild: "Quarry".to_owned(),
        arm: EvalArm::Off,
        seat_kind: "Model".to_owned(),
        keeps_notes: false,
        notes_present: false,
        raw_reply: Some(ambig_reply),
        discarded_reply_path: None,
        discarded_reply_sha256: None,
        attempts: 1,
        failed_attempts: Vec::new(),
        prompt_tokens: 1,
        completion_tokens: 1,
        wall_secs: None,
        tool_calls_used: 0,
        cleared_non_empty_actions: false,
        prompt_token_ids_sha256s: Vec::new(),
        prompt_token_ids_source: None,
        prompt_token_ids_template_check: None,
        reply_token_ids: Vec::new(),
        reply_top_logprobs: Vec::new(),
        intermediate_bundle_sha256s: Vec::new(),
        refused_market_actions: 0,
        refused_bundle_legs: 0,
        parse_failure: false,
        refusal_code: None,
        refusal_reason: None,
        loaded_history_sha256: BTreeMap::new(),
        invalid_recipients: Vec::new(),
        no_reply: false,
        apply_report: None,
        refused_bundle: None,
        club_bot_baseline: None,
    };
    fs::write(
        dec_dir.join("day-01-talk-Quarry.json"),
        serde_json::to_string_pretty(&talk_dec)?,
    )?;
    let ambig_out = extract_talk_commitments_for_state(&case_ambig, 1, 0, &seat_guilds, &obs)?;
    assert!(ambig_out.is_empty());

    // Resolve guilds fallback via decisions dir.
    let mut dummy_f = talk_dec.clone();
    dummy_f.seat = 0;
    dummy_f.guild = "Foundry".to_owned();
    let mut dummy_h = talk_dec.clone();
    dummy_h.seat = 2;
    dummy_h.guild = "Harbor".to_owned();
    let mut dummy_m = talk_dec.clone();
    dummy_m.seat = 3;
    dummy_m.guild = "Mint".to_owned();
    fs::write(
        dec_dir.join("day-01-talk-Foundry.json"),
        serde_json::to_string_pretty(&dummy_f)?,
    )?;
    fs::write(
        dec_dir.join("day-01-talk-Harbor.json"),
        serde_json::to_string_pretty(&dummy_h)?,
    )?;
    fs::write(
        dec_dir.join("day-01-talk-Mint.json"),
        serde_json::to_string_pretty(&dummy_m)?,
    )?;
    assert_eq!(resolve_eval_seat_guilds(&case_ambig, 4)?, seat_guilds);

    // 3. Retraction whisper removes the offer cleanly.
    let case_retract = tmp.path().join("retract");
    write_whisper_case(
        &case_retract,
        vec![
            UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "We will buy 500 refined Foundry at 400 CRED.".to_owned(),
            },
            UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "Cancel that: no deal.".to_owned(),
            },
        ],
    )?;
    let ret_out = extract_talk_commitments_for_state(&case_retract, 1, 0, &seat_guilds, &obs)?;
    assert!(ret_out.is_empty());
    Ok(())
}

#[test]
fn freeze_and_join_gates_refuse_every_violation_with_named_codes() -> TestResult {
    let tmp = TmpDir::new("gates")?;
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let git_sha = clean_git_sha_source();
    let transport = SyntheticPipelineTransport;

    let play_dir = tmp.path().join("play_ctrl");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9104,
            seed: 5,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let branch_dir = tmp.path().join("branch_ctrl");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&league_eval::BuildBlindInputsOptions {
        run_dirs: vec![play_dir.clone(), branch_dir.clone()],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 11,
    })?;

    let scores_dir = tmp.path().join("scores");
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        ),
        0
    );

    let joined_out = tmp.path().join("joined_fail");
    let missing_freeze_path = tmp.path().join("missing_freeze.json");
    let vc_path = verify_ckpts_for_test(
        &ckpts_dir,
        &tmp.path().join("verified_checkpoints.json"),
        Some(&blind_dir),
        &git_sha,
    )?;

    // 1. Join without a freeze file MUST be refused (`MISSING_FREEZE_FILE`, §8.2 step 2).
    let no_freeze_err = run_join(&league_eval::JoinOptions {
        freeze_path: missing_freeze_path,
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: joined_out.clone(),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected MISSING_FREEZE_FILE")?;
    assert_eq!(no_freeze_err.code(), "MISSING_FREEZE_FILE");

    // 2. Freeze refuses dirty git worktree (`DIRTY_GIT_WORKTREE`).
    let freeze_path = tmp.path().join("scorer_freeze.json");
    let dirty_sha = CrateGitShaSource::Injected {
        built_sha: TEST_FALLBACK_GIT_SHA,
        live_head: Some(TEST_FALLBACK_GIT_SHA),
        is_dirty: Some(true),
    };
    let dirty_err = run_freeze(
        &league_eval::FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &dirty_sha,
    )
    .err()
    .ok_or("expected DIRTY_GIT_WORKTREE")?;
    assert_eq!(dirty_err.code(), "DIRTY_GIT_WORKTREE");

    // Write valid freeze once.
    let valid_freeze = run_freeze(
        &league_eval::FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;

    // 3. Freeze refuses overwrite (`FREEZE_FILE_EXISTS`).
    let overwrite_err = run_freeze(
        &league_eval::FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )
    .err()
    .ok_or("expected FREEZE_FILE_EXISTS")?;
    assert_eq!(overwrite_err.code(), "FREEZE_FILE_EXISTS");

    // 4. Join refuses `FREEZE_MANIFEST_MISMATCH`.
    let mut bad_manifest_freeze = valid_freeze.clone();
    bad_manifest_freeze.blind_inputs_manifest_sha256 = "0".repeat(64);
    let bad_manifest_freeze_path = tmp.path().join("bad_manifest_freeze.json");
    fs::write(
        &bad_manifest_freeze_path,
        to_sorted_pretty_json(&bad_manifest_freeze)?,
    )?;
    let manifest_err = run_join(&league_eval::JoinOptions {
        freeze_path: bad_manifest_freeze_path,
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: joined_out.clone(),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected FREEZE_MANIFEST_MISMATCH")?;
    assert_eq!(manifest_err.code(), "FREEZE_MANIFEST_MISMATCH");

    // 5. Join refuses `FREEZE_CONFIG_MISMATCH`.
    let mut bad_cfg_freeze = valid_freeze.clone();
    bad_cfg_freeze.scorer_config_sha256 = "1".repeat(64);
    let bad_cfg_freeze_path = tmp.path().join("bad_cfg_freeze.json");
    fs::write(
        &bad_cfg_freeze_path,
        to_sorted_pretty_json(&bad_cfg_freeze)?,
    )?;
    let cfg_err = run_join(&league_eval::JoinOptions {
        freeze_path: bad_cfg_freeze_path,
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: joined_out.clone(),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected FREEZE_CONFIG_MISMATCH")?;
    assert_eq!(cfg_err.code(), "FREEZE_CONFIG_MISMATCH");

    // 6. Control run's `setup.json` `relay_wording_arm` differs -> `CONTROL_WORDING_ARM_MISMATCH`.
    let play_setup_path = play_dir.join("setup.json");
    let mut play_setup: EvalSetupFile =
        serde_json::from_str(&fs::read_to_string(&play_setup_path)?)?;
    assert_eq!(play_setup.arm, EvalArm::Control);
    play_setup.relay_wording_arm = "v2_control_2026_11_21_r1".to_owned();
    fs::write(&play_setup_path, serde_json::to_string_pretty(&play_setup)?)?;
    let wording_err = run_join(&league_eval::JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: joined_out.clone(),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: Some(blind_dir.clone()),
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected CONTROL_WORDING_ARM_MISMATCH when setup.json relay_wording_arm differs")?;
    assert_eq!(wording_err.code(), "CONTROL_WORDING_ARM_MISMATCH");

    // Restore setup.json, test `league-scorer score --freeze` against tampered tree dir (`FREEZE_TREE_MISMATCH`).
    play_setup.relay_wording_arm = CONTROL_WORDING_ARM.to_owned();
    fs::write(&play_setup_path, serde_json::to_string_pretty(&play_setup)?)?;

    let fake_crate_dir = tmp.path().join("fake_scorer_crate");
    fs::create_dir_all(fake_crate_dir.join("src"))?;
    fs::write(fake_crate_dir.join("src/lib.rs"), b"// modified tree")?;
    let mut err_tree = Vec::new();
    let tree_code = run_scorer_cli(
        [
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_path.to_str().ok_or("utf8")?,
            "--scorer-crate-dir",
            fake_crate_dir.to_str().ok_or("utf8")?,
            "--out",
            tmp.path().join("scores_tree_fail").to_str().ok_or("utf8")?,
        ],
        &mut Vec::new(),
        &mut err_tree,
    );
    assert_eq!(tree_code, 1);
    assert!(String::from_utf8_lossy(&err_tree).contains("FREEZE_TREE_MISMATCH"));

    // Restore scores directly after confirming join now succeeds cleanly once setup.json matches.
    let sid = format!("{}-day-01-seat-0", play_manifest.game_id);
    let score_json: StateScoreOutput =
        serde_json::from_slice(&fs::read(scores_dir.join(format!("scores/{sid}.json")))?)?;
    assert_eq!(score_json.state_id, sid);

    let join_ok = run_join(&league_eval::JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: joined_out,
        verified_checkpoints_path: Some(vc_path),
        blind_dir: Some(blind_dir),
        source_game_dirs: Vec::new(),
    })?;
    assert_eq!(join_ok.states_joined, 1);
    assert_eq!(join_ok.decisions_joined, 1);
    Ok(())
}

#[test]
fn cli_flag_validation_and_error_variants_for_s3a() -> TestResult {
    let git_sha = clean_git_sha_source();
    let tmp = TmpDir::new("cli_s3a")?;

    let (help_code, help_out, _) = run_eval_cli(&["--help"], &git_sha);
    assert_eq!(help_code, 0);
    for cmd in ["build-blind-inputs", "freeze", "join"] {
        assert!(help_out.contains(cmd), "--help missing `{cmd}`");
    }

    for (args, expected_code_substr) in [
        (
            vec![
                "build-blind-inputs",
                "--out-blind",
                "a",
                "--out-arm-map",
                "b",
                "--config",
                "c",
                "--scorer-seed",
                "1",
            ],
            "INVALID_USAGE",
        ),
        (
            vec![
                "build-blind-inputs",
                "--run",
                "r",
                "--out-blind",
                "same",
                "--out-arm-map",
                "same",
                "--config",
                "c",
                "--scorer-seed",
                "1",
            ],
            "INVALID_USAGE",
        ),
        (
            vec![
                "freeze",
                "--blind",
                "b",
                "--config",
                "c",
                "--control-wording-arm",
                "r2",
            ],
            "INVALID_USAGE",
        ),
        (
            vec!["join", "--freeze", "f", "--arm-map", "a", "--scores", "s"],
            "INVALID_USAGE",
        ),
    ] {
        let mut cli = vec!["--error-format", "json"];
        cli.extend(args);
        let (code, _, err) = run_eval_cli(&cli, &git_sha);
        assert_eq!(code, 1, "expected exit 1, stderr: {err}");
        let v: Value = serde_json::from_str(err.trim())?;
        assert_eq!(v["code"], expected_code_substr);
    }

    for (err, expected_code) in [
        (
            EvalError::FreezeFileExists("exists".into()),
            "FREEZE_FILE_EXISTS",
        ),
        (
            EvalError::MissingFreezeFile("missing".into()),
            "MISSING_FREEZE_FILE",
        ),
        (
            EvalError::FreezeManifestMismatch("manifest".into()),
            "FREEZE_MANIFEST_MISMATCH",
        ),
        (
            EvalError::FreezeConfigMismatch("config".into()),
            "FREEZE_CONFIG_MISMATCH",
        ),
        (
            EvalError::ControlWordingArmMismatch("arm".into()),
            "CONTROL_WORDING_ARM_MISMATCH",
        ),
        (
            EvalError::DuplicateStateId("dup".into()),
            "DUPLICATE_STATE_ID",
        ),
    ] {
        assert_eq!(err.code(), expected_code);
        assert!(err.to_string().starts_with(expected_code));
        assert_eq!(err.exit_code(), 1);
        let _ = err.to_structured_error();
    }

    let wrapped_scorer_err: EvalError =
        ScorerError::FreezeTreeMismatch("tree mismatch".to_owned()).into();
    assert_eq!(wrapped_scorer_err.code(), "FREEZE_TREE_MISMATCH");
    assert_eq!(wrapped_scorer_err.exit_code(), 1);
    assert!(
        wrapped_scorer_err
            .to_string()
            .contains("FREEZE_TREE_MISMATCH")
    );
    let _ = wrapped_scorer_err.to_structured_error();

    let _ = tmp;
    Ok(())
}

#[test]
fn talk_extractor_grammar_variants_ignored_clauses_and_errors() -> TestResult {
    let tmp = TmpDir::new("talk_grammar")?;
    let host = EvalHost::from_standard_setup(TableId(9105), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Mint".to_owned(),
    ];

    assert_eq!(
        resolve_eval_seat_guilds(&tmp.path().join("no_game_dir"), 4)
            .err()
            .ok_or("expected MISSING_SOURCE_SETUP")?
            .code(),
        "MISSING_SOURCE_SETUP"
    );

    let bad_guild_len = extract_talk_commitments_from_messages(0, &seat_guilds[..3], &[], &obs);
    assert_eq!(
        bad_guild_len
            .err()
            .ok_or("expected bad guild len error")?
            .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    let bad_focal = extract_talk_commitments_from_messages(9, &seat_guilds, &[], &obs);
    assert_eq!(
        bad_focal
            .err()
            .ok_or("expected bad focal seat error")?
            .code(),
        "UNKNOWN_FOCAL_SEAT"
    );

    // Exercise compound good forms (`raw0`, `refined:0`, `Foundry raw`) under closed templates
    // alongside neutral greeting clauses.
    let messages = vec![
        (
            1_u8,
            0_u8,
            "I commit to sell 200 raw0 at 350 CRED.".to_owned(),
        ),
        (
            2_u8,
            0_u8,
            "I commit to buy 300 refined:0 at 420 CRED.".to_owned(),
        ),
        (
            3_u8,
            0_u8,
            "Hi! I will buy 150 Foundry raw at 310 CRED.".to_owned(),
        ),
    ];
    let extracted = extract_talk_commitments_from_messages(0, &seat_guilds, &messages, &obs)?;
    assert_eq!(extracted.len(), 3);

    // Unlisted clauses, conditionals, hedges, conflicting/malformed orders all fail closed.
    for fail_closed_msg in [
        "@Foundry! I commit to sell 200 raw0 at 350 CRED.",
        "I commit to buy 300 refined:0 for 420 CRED; If you accept, I will buy 100 raw0 at 200.",
        "I will buy 150 Foundry raw for 310 CRED. ... Maybe I commit to buy 100 raw0 at 100.",
        "Hello Foundry! Buying selling 100 Foundry refined for 200 CRED.",
        "I commit to sell at 100 for 200 raw0 300 CRED.",
        "I commit to sell 100 raw0 50 CRED foo at 200.",
        "I commit to sell raw0 at 200 CRED.",
        "I commit to sell 100 raw0 at CRED.",
        "I commit to sell 100 raw0 at roughly 200 CRED.",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1_u8, 0_u8, fail_closed_msg.to_owned())],
                &obs,
            )?
            .is_empty(),
            "unlisted/malformed clause must fail closed: `{fail_closed_msg}`"
        );
    }

    // Exercise decisions dir with missing seat, no_reply, refusal, and matching negotiation+talk.
    let game_dir = tmp.path().join("mixed_talk_game");
    let dec_dir = game_dir.join("decisions");
    let to_seat = game_dir.join("to-seat");
    fs::create_dir_all(&dec_dir)?;
    fs::create_dir_all(&to_seat)?;

    let neg = NegotiationInputs {
        batna: "b".to_owned(),
        whispers: vec![UntrustedWhisper {
            from_guild: "Quarry".to_owned(),
            intent_header: None,
            body: "I commit to buy 500 Foundry refined at 400 CRED.".to_owned(),
        }],
    };
    fs::write(
        to_seat.join("day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&neg)?,
    )?;

    let make_dec =
        |seat: u8, guild: &str, raw_reply: Option<String>, no_reply: bool| EvalDecisionRecord {
            game_id: host.game_id().to_owned(),
            table_id: 9105,
            day: 1,
            round: Round::Talk,
            seat,
            guild: guild.to_owned(),
            arm: EvalArm::Off,
            seat_kind: "Model".to_owned(),
            keeps_notes: false,
            notes_present: false,
            raw_reply,
            discarded_reply_path: None,
            discarded_reply_sha256: None,
            attempts: 1,
            failed_attempts: Vec::new(),
            prompt_tokens: 1,
            completion_tokens: 1,
            wall_secs: None,
            tool_calls_used: 0,
            cleared_non_empty_actions: false,
            prompt_token_ids_sha256s: Vec::new(),
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
            reply_token_ids: Vec::new(),
            reply_top_logprobs: Vec::new(),
            intermediate_bundle_sha256s: Vec::new(),
            refused_market_actions: 0,
            refused_bundle_legs: 0,
            parse_failure: false,
            refusal_code: None,
            refusal_reason: None,
            loaded_history_sha256: BTreeMap::new(),
            invalid_recipients: Vec::new(),
            no_reply,
            apply_report: None,
            refused_bundle: None,
            club_bot_baseline: None,
        };
    let differing_talk = json!({
        "messages": [{ "to_guild": "Foundry", "body": "I commit to buy 450 raw1 at 390 CRED." }],
        "market_actions": [],
        "bundle": {}
    })
    .to_string();
    let refused_talk = "not valid json talk";
    fs::write(
        dec_dir.join("day-01-talk-Quarry.json"),
        serde_json::to_string_pretty(&make_dec(1, "Quarry", Some(differing_talk), false))?,
    )?;
    fs::write(
        dec_dir.join("day-01-talk-Harbor.json"),
        serde_json::to_string_pretty(&make_dec(2, "Harbor", None, true))?,
    )?;
    fs::write(
        dec_dir.join("day-01-talk-Mint.json"),
        serde_json::to_string_pretty(&make_dec(3, "Mint", Some(refused_talk.to_owned()), false))?,
    )?;
    assert_eq!(
        resolve_eval_seat_guilds(&game_dir, 4)
            .err()
            .ok_or("expected missing seat 0 guild")?
            .code(),
        "MISSING_SOURCE_SETUP"
    );
    let combined = extract_talk_commitments_for_state(&game_dir, 1, 0, &seat_guilds, &obs)?;
    assert_eq!(combined.len(), 2);
    Ok(())
}

#[test]
fn cli_flag_errors_and_builder_freeze_join_refusal_matrix() -> TestResult {
    let git_sha = clean_git_sha_source();
    let tmp = TmpDir::new("matrix")?;

    // Exercise CLI usage & flag-validation errors for build-blind-inputs, freeze, join.
    for args in [
        vec![
            "build-blind-inputs",
            "--error-format",
            "json",
            "--run",
            "a,b",
            "--bogus",
        ],
        vec!["build-blind-inputs", "--run", "a"],
        vec!["build-blind-inputs", "--run", "a", "--out-blind", "b"],
        vec![
            "build-blind-inputs",
            "--run",
            "a",
            "--out-blind",
            "b",
            "--out-arm-map",
            "c",
        ],
        vec![
            "build-blind-inputs",
            "--run",
            "a",
            "--out-blind",
            "b",
            "--out-arm-map",
            "c",
            "--config",
            "cfg",
            "--scorer-seed",
            "not_a_number",
        ],
        vec![
            "build-blind-inputs",
            "--run",
            "a",
            "--out-blind",
            "b",
            "--out-arm-map",
            "c",
            "--config",
            "cfg",
        ],
        vec!["freeze", "--error-format", "json", "--bogus"],
        vec!["freeze"],
        vec!["freeze", "--blind", "b"],
        vec!["freeze", "--blind", "b", "--config", "c"],
        vec![
            "freeze",
            "--blind",
            "b",
            "--config",
            "c",
            "--control-wording-arm",
            "r2",
            "--scorer-crate-dir",
            "crate_dir",
        ],
        vec!["join", "--error-format", "json", "--bogus"],
        vec!["join"],
        vec!["join", "--freeze", "f"],
        vec!["join", "--freeze", "f", "--arm-map", "a"],
    ] {
        let (code, _, err) = run_eval_cli(&args, &git_sha);
        assert_eq!(code, 1, "expected usage error for {args:?}, stderr={err}");
        assert!(err.contains("INVALID_USAGE") || err.contains("code"));
    }

    // Config and run-dir validation errors in `run_build_blind_inputs` and `run_freeze`.
    let mut bad_ver_cfg = fast_test_scorer_config();
    bad_ver_cfg.talk_extractor_version = "talk_rules_v99".to_owned();
    let bad_ver_cfg_path = tmp.path().join("bad_ver_cfg.json");
    fs::write(&bad_ver_cfg_path, to_sorted_pretty_json(&bad_ver_cfg)?)?;

    let mut bad_dig_cfg = fast_test_scorer_config();
    bad_dig_cfg.talk_extractor_rules_digest = "f".repeat(64);
    let bad_dig_cfg_path = tmp.path().join("bad_dig_cfg.json");
    fs::write(&bad_dig_cfg_path, to_sorted_pretty_json(&bad_dig_cfg)?)?;

    let empty_dir = tmp.path().join("empty_runs");
    fs::create_dir_all(&empty_dir)?;
    let valid_cfg = fast_test_scorer_config();
    let valid_cfg_path = tmp.path().join("valid_cfg.json");
    fs::write(&valid_cfg_path, to_sorted_pretty_json(&valid_cfg)?)?;

    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![empty_dir.clone()],
            out_blind_dir: tmp.path().join("ob1"),
            out_arm_map_dir: tmp.path().join("oa1"),
            out_checkpoints_dir: None,
            config_path: bad_ver_cfg_path.clone(),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected bad talk version")?
        .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![empty_dir.clone()],
            out_blind_dir: tmp.path().join("ob2"),
            out_arm_map_dir: tmp.path().join("oa2"),
            out_checkpoints_dir: None,
            config_path: bad_dig_cfg_path.clone(),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected bad talk digest")?
        .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![tmp.path().join("does_not_exist")],
            out_blind_dir: tmp.path().join("ob3"),
            out_arm_map_dir: tmp.path().join("oa3"),
            out_checkpoints_dir: None,
            config_path: valid_cfg_path.clone(),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected missing --run dir")?
        .code(),
        "MISSING_SOURCE_SETUP"
    );
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![empty_dir.clone()],
            out_blind_dir: tmp.path().join("ob4"),
            out_arm_map_dir: tmp.path().join("oa4"),
            out_checkpoints_dir: None,
            config_path: valid_cfg_path.clone(),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected empty --run dir")?
        .code(),
        "MISSING_SOURCE_SETUP"
    );

    let unused_vc = tmp.path().join("unused_vc.json");
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: empty_dir.clone(),
                config_path: valid_cfg_path.clone(),
                control_wording_arm: "   ".to_owned(),
                out_path: tmp.path().join("f0.json"),
                verified_checkpoints_path: unused_vc.clone(),
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected empty control_wording_arm usage error")?
        .code(),
        "INVALID_USAGE"
    );
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: empty_dir.clone(),
                config_path: bad_ver_cfg_path,
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("f1.json"),
                verified_checkpoints_path: unused_vc.clone(),
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected bad talk version in run_freeze")?
        .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: empty_dir.clone(),
                config_path: bad_dig_cfg_path,
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("f2.json"),
                verified_checkpoints_path: unused_vc,
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected bad talk digest in run_freeze")?
        .code(),
        "SCHEMA_VALIDATION_FAILED"
    );

    // Build a valid control play+branch fixture for exhaustive freeze/join failure gates.
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let transport = SyntheticPipelineTransport;

    let play_dir = tmp.path().join("play_m");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9106,
            seed: 9,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    // Copy control branch outcome without its play dir -> missing source game & missing checkpoint checks.
    let orphan_outcome_dir = tmp.path().join("orphan_outcome");
    let orphan_arm_dir = orphan_outcome_dir.join("control");
    fs::create_dir_all(&orphan_arm_dir)?;
    fs::copy(
        play_dir.join("decisions/day-01-act-Foundry.json"),
        orphan_arm_dir.join("decision.json"),
    )?;
    // Write a synthetic outcome matching BranchArmOutcome fields.
    let branch_dir = tmp.path().join("branch_m");
    let outcomes = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    fs::copy(
        branch_dir.join("control/outcome.json"),
        orphan_arm_dir.join("outcome.json"),
    )?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![orphan_outcome_dir],
            out_blind_dir: tmp.path().join("ob_orphan"),
            out_arm_map_dir: tmp.path().join("oa_orphan"),
            out_checkpoints_dir: None,
            config_path: valid_cfg_path.clone(),
            scorer_seed: 5,
        })
        .err()
        .ok_or("expected missing source game dir")?
        .code(),
        "MISSING_SOURCE_SETUP"
    );

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir.clone(), branch_dir.clone()],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: valid_cfg_path.clone(),
        scorer_seed: 5,
    })?;

    let scores_dir = tmp.path().join("scores");
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                valid_cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        ),
        0
    );

    let vc_path = verify_ckpts_for_test(
        &ckpts_dir,
        &tmp.path().join("verified_checkpoints.json"),
        Some(&blind_dir),
        &git_sha,
    )?;
    let freeze_path = tmp.path().join("freeze.json");
    let freeze = run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: valid_cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;

    // Check `ScorerFreezeFile::validate()` refusal variants.
    let mut bad_git_freeze = freeze.clone();
    bad_git_freeze.scorer_git_sha = "not40hex".to_owned();
    assert_eq!(
        bad_git_freeze
            .validate()
            .err()
            .ok_or("expected bad git sha")?
            .code(),
        "INVALID_CRATE_GIT_SHA"
    );
    let mut bad_tree_freeze = freeze.clone();
    bad_tree_freeze.scorer_tree_sha256 = "short".to_owned();
    assert_eq!(
        bad_tree_freeze
            .validate()
            .err()
            .ok_or("expected bad tree hex")?
            .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    let mut missing_vc_freeze = freeze.clone();
    missing_vc_freeze.verified_checkpoints_sha256 = "".to_owned();
    assert_eq!(
        missing_vc_freeze
            .validate()
            .err()
            .ok_or("expected missing verified_checkpoints_sha256")?
            .code(),
        "MISSING_VERIFIED_CHECKPOINTS"
    );
    let mut bad_vc_freeze = freeze.clone();
    bad_vc_freeze.verified_checkpoints_sha256 = "short".to_owned();
    assert_eq!(
        bad_vc_freeze
            .validate()
            .err()
            .ok_or("expected bad verified_checkpoints_sha256 hex")?
            .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    let mut empty_arm_freeze = freeze.clone();
    empty_arm_freeze.control_wording_arm = "".to_owned();
    assert_eq!(
        empty_arm_freeze
            .validate()
            .err()
            .ok_or("expected empty control_wording_arm")?
            .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    let mut empty_talk_freeze = freeze.clone();
    empty_talk_freeze.talk_extractor_version = " ".to_owned();
    assert_eq!(
        empty_talk_freeze
            .validate()
            .err()
            .ok_or("expected empty talk_extractor_version")?
            .code(),
        "SCHEMA_VALIDATION_FAILED"
    );
    assert_eq!(freeze.talk_extractor_version, TALK_EXTRACTOR_VERSION_V2);
    assert!(!TALK_EXTRACTOR_RULES_DIGEST_V2.is_empty());

    let joined_fail = tmp.path().join("joined_fail");

    // Empty --arm-map -> FREEZE_MANIFEST_MISMATCH
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: empty_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected empty arm-map error")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    // Empty --blind in run_freeze -> FREEZE_MANIFEST_MISMATCH; in run_join -> NO_BLIND_INPUTS
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: empty_dir.clone(),
                config_path: valid_cfg_path.clone(),
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("freeze_empty_blind.json"),
                verified_checkpoints_path: vc_path.clone(),
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected freeze empty blind error")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: Some(empty_dir.clone()),
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected empty blind error")?
        .code(),
        "NO_BLIND_INPUTS"
    );

    // Tampered --blind manifest vs freeze & stem != state_id
    let sid = format!("{}-day-01-seat-0", play_manifest.game_id);
    let bad_blind_dir = tmp.path().join("bad_blind");
    fs::create_dir_all(&bad_blind_dir)?;
    fs::copy(
        blind_dir.join(format!("{sid}.json")),
        bad_blind_dir.join("wrong_stem.json"),
    )?;
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: bad_blind_dir.clone(),
                config_path: valid_cfg_path.clone(),
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("freeze_wrong_stem.json"),
                verified_checkpoints_path: vc_path.clone(),
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected freeze blind stem mismatch")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: Some(bad_blind_dir.clone()),
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected blind stem mismatch")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );
    let mut blind_req: BlindScoringRequest =
        serde_json::from_slice(&fs::read(blind_dir.join(format!("{sid}.json")))?)?;
    blind_req.scorer_seed = 999;
    fs::remove_file(bad_blind_dir.join("wrong_stem.json"))?;
    fs::write(
        bad_blind_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&blind_req)?,
    )?;
    assert_ne!(
        compute_blind_inputs_manifest_sha256(&bad_blind_dir)?,
        freeze.blind_inputs_manifest_sha256
    );
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: Some(bad_blind_dir),
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected blind manifest mismatch")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    // Empty --scores -> FREEZE_MANIFEST_MISMATCH
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: empty_dir,
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected empty scores error")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    // Score state_id mismatch, metadata mismatch, candidate set mismatch, missing candidate id.
    let mut score_val: StateScoreOutput =
        serde_json::from_slice(&fs::read(scores_dir.join(format!("scores/{sid}.json")))?)?;
    let direct_scores_dir = tmp.path().join("direct_scores");
    fs::create_dir_all(&direct_scores_dir)?;

    let mut wrong_sid_score = score_val.clone();
    wrong_sid_score.state_id = "other-day-01-seat-0".to_owned();
    fs::write(
        direct_scores_dir.join("other-day-01-seat-0.json"),
        to_sorted_pretty_json(&wrong_sid_score)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: direct_scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected state_id set mismatch")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );
    fs::remove_file(direct_scores_dir.join("other-day-01-seat-0.json"))?;

    let mut wrong_meta_score = score_val.clone();
    wrong_meta_score.scorer_seed = 777;
    fs::write(
        direct_scores_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&wrong_meta_score)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: direct_scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected metadata mismatch")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    let mut wrong_cands_score = score_val.clone();
    if let Some(first_c) = wrong_cands_score.candidates.first_mut() {
        *first_c = CandidateScoreOutput {
            candidate_id: "cand_wrong_0000".to_owned(),
            ..first_c.clone()
        };
    }
    fs::write(
        direct_scores_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&wrong_cands_score)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: direct_scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected candidate set mismatch")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    // Restore valid score directly inside `direct_scores_dir` and test control wording edge branches.
    fs::write(
        direct_scores_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&score_val)?,
    )?;
    score_val.scorer_config_sha256 = "f".repeat(64);
    let diff_cfg_scores = tmp.path().join("diff_cfg_scores");
    fs::create_dir_all(&diff_cfg_scores)?;
    fs::write(
        diff_cfg_scores.join(format!("{sid}.json")),
        to_sorted_pretty_json(&score_val)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: diff_cfg_scores,
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected score config mismatch")?
        .code(),
        "FREEZE_CONFIG_MISMATCH"
    );

    // Control setup.json wrong arm, wrong manifest wording, wrong relay/setup.json, and missing proof.
    let setup_path = play_dir.join("setup.json");
    let orig_setup: EvalSetupFile = serde_json::from_str(&fs::read_to_string(&setup_path)?)?;
    let mut off_setup = orig_setup.clone();
    off_setup.arm = EvalArm::Off;
    fs::write(&setup_path, serde_json::to_string_pretty(&off_setup)?)?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: direct_scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected wrong setup.json arm")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::write(&setup_path, serde_json::to_string_pretty(&orig_setup)?)?;

    let ctrl_manifest_path = branch_dir.join("control/manifest.json");
    let orig_manifest: EvalManifest =
        serde_json::from_str(&fs::read_to_string(&ctrl_manifest_path)?)?;
    let mut bad_manifest = orig_manifest.clone();
    bad_manifest.relay_wording_arm = "v2_control_2026_11_21_r1".to_owned();
    fs::write(
        &ctrl_manifest_path,
        serde_json::to_string_pretty(&bad_manifest)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: direct_scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected bad manifest relay_wording_arm")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::write(
        &ctrl_manifest_path,
        serde_json::to_string_pretty(&orig_manifest)?,
    )?;

    let relay_setup_path = branch_dir.join("control/relay/setup.json");
    let relay_orig = fs::read_to_string(&relay_setup_path)?;
    fs::write(
        &relay_setup_path,
        json!({ "wording_arm": "v2_control_2026_11_21_r1" }).to_string(),
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: direct_scores_dir.clone(),
            out_dir: joined_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected bad relay/setup.json wording_arm")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::write(&relay_setup_path, relay_orig)?;

    // Edit arm_map copy where control entry lacks setup_path, manifest_path, and relay/setup.json.
    let no_proof_arm_map_dir = tmp.path().join("no_proof_arm_map");
    fs::create_dir_all(&no_proof_arm_map_dir)?;
    let mut am: ArmMapStateFile =
        serde_json::from_slice(&fs::read(arm_map_dir.join(format!("{sid}.json")))?)?;
    let isolated_dec = tmp.path().join("isolated/decision.json");
    if let Some(parent) = isolated_dec.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&isolated_dec, b"{}")?;
    for entry in &mut am.entries {
        if entry.arm == EvalArm::Control {
            entry.setup_path = None;
            entry.manifest_path = None;
            entry.decision_path = isolated_dec.clone();
        }
    }
    fs::write(
        no_proof_arm_map_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&am)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path,
            arm_map_dir: no_proof_arm_map_dir,
            scores_dir: direct_scores_dir,
            out_dir: joined_fail,
            verified_checkpoints_path: Some(vc_path),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected missing wording proof")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );

    let _ = outcomes;
    Ok(())
}

#[test]
fn builder_and_talk_error_paths_and_edge_branches() -> TestResult {
    let tmp = TmpDir::new("builder_talk_errs")?;
    let git_sha = clean_git_sha_source();
    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    // 1. `talk_extract`: additional ClauseEffect::Ignore patterns & file parse errors.
    let host = EvalHost::from_standard_setup(TableId(9107), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Mint".to_owned(),
    ];
    let ignore_msgs = vec![(
        1_u8,
        0_u8,
        ". ; I commit to buy 100 raw0 and 200 raw1 for 300 CRED. \
         I commit to buy 100 at 200 raw0. \
         I commit to buy 100 raw0 at 200 roughly."
            .to_owned(),
    )];
    let ignored_res = extract_talk_commitments_from_messages(0, &seat_guilds, &ignore_msgs, &obs)?;
    assert!(ignored_res.is_empty());

    let bad_manifest_dir = tmp.path().join("bad_manifest_game");
    fs::create_dir_all(&bad_manifest_dir)?;
    fs::write(bad_manifest_dir.join("manifest.json"), b"{bad")?;
    assert_eq!(
        resolve_eval_seat_guilds(&bad_manifest_dir, 4)
            .err()
            .ok_or("expected malformed manifest error")?
            .code(),
        "EVAL_IO"
    );

    let bad_neg_dir = tmp.path().join("bad_neg_game");
    fs::create_dir_all(bad_neg_dir.join("to-seat"))?;
    fs::write(
        bad_neg_dir.join("to-seat/day-01-act-Foundry.negotiation.json"),
        b"{bad",
    )?;
    assert_eq!(
        extract_talk_commitments_for_state(&bad_neg_dir, 1, 0, &seat_guilds, &obs)
            .err()
            .ok_or("expected malformed negotiation.json error")?
            .code(),
        "EVAL_IO"
    );

    let bad_talk_dir = tmp.path().join("bad_talk_game");
    fs::create_dir_all(bad_talk_dir.join("decisions"))?;
    fs::write(
        bad_talk_dir.join("decisions/day-01-talk-Quarry.json"),
        b"{bad",
    )?;
    assert_eq!(
        extract_talk_commitments_for_state(&bad_talk_dir, 1, 0, &seat_guilds, &obs)
            .err()
            .ok_or("expected malformed talk decision.json error")?
            .code(),
        "EVAL_IO"
    );

    // 2. `run_build_blind_inputs`: config I/O + parse errors.
    let dummy_run = tmp.path().join("dummy_run");
    fs::create_dir_all(&dummy_run)?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![dummy_run.clone()],
            out_blind_dir: tmp.path().join("ob_mcfg"),
            out_arm_map_dir: tmp.path().join("oa_mcfg"),
            out_checkpoints_dir: None,
            config_path: tmp.path().join("no_cfg.json"),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected missing config EVAL_IO")?
        .code(),
        "EVAL_IO"
    );
    let bad_cfg_path = tmp.path().join("bad_cfg.json");
    fs::write(&bad_cfg_path, b"{bad")?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![dummy_run.clone()],
            out_blind_dir: tmp.path().join("ob_bcfg"),
            out_arm_map_dir: tmp.path().join("oa_bcfg"),
            out_checkpoints_dir: None,
            config_path: bad_cfg_path,
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected malformed config EVAL_IO")?
        .code(),
        "EVAL_IO"
    );

    // Build real synthetic fixtures for deeper builder edge branches.
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let transport = SyntheticPipelineTransport;

    let play_dir = tmp.path().join("play_b");
    let _ = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Off,
            table_id: 9108,
            seed: 13,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let branch_dir = tmp.path().join("branch_b");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Off, EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    // Malformed outcome.json & malformed decision.json.
    let bad_out_dir = tmp.path().join("bad_outcome_run");
    fs::create_dir_all(bad_out_dir.join("off"))?;
    fs::write(bad_out_dir.join("off/outcome.json"), b"{bad")?;
    fs::copy(
        branch_dir.join("off/decision.json"),
        bad_out_dir.join("off/decision.json"),
    )?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![bad_out_dir],
            out_blind_dir: tmp.path().join("ob_bo"),
            out_arm_map_dir: tmp.path().join("oa_bo"),
            out_checkpoints_dir: None,
            config_path: cfg_path.clone(),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected malformed outcome EVAL_IO")?
        .code(),
        "EVAL_IO"
    );

    let bad_dec_dir = tmp.path().join("bad_dec_run");
    fs::create_dir_all(bad_dec_dir.join("off"))?;
    fs::copy(
        branch_dir.join("off/outcome.json"),
        bad_dec_dir.join("off/outcome.json"),
    )?;
    fs::write(bad_dec_dir.join("off/decision.json"), b"{bad")?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![bad_dec_dir],
            out_blind_dir: tmp.path().join("ob_bd"),
            out_arm_map_dir: tmp.path().join("oa_bd"),
            out_checkpoints_dir: None,
            config_path: cfg_path.clone(),
            scorer_seed: 1,
        })
        .err()
        .ok_or("expected malformed decision EVAL_IO")?
        .code(),
        "EVAL_IO"
    );

    // Exercise `no_reply: false, raw_reply: None`, parent/direct `setup.json` matching `arm`,
    // `relay/seats/` ignored subdir + wrong round/guild + `calls.jsonl` blank line & reservation filter.
    let off_dec_path = branch_dir.join("off/decision.json");
    let orig_off_dec = fs::read_to_string(&off_dec_path)?;
    let mut no_raw_dec: EvalDecisionRecord = serde_json::from_str(&orig_off_dec)?;
    no_raw_dec.no_reply = false;
    no_raw_dec.raw_reply = None;
    fs::write(&off_dec_path, serde_json::to_string_pretty(&no_raw_dec)?)?;

    // Copy `play_dir/setup.json` into `branch_dir/setup.json` (which has `arm: Off`), matching parent setup!
    fs::copy(play_dir.join("setup.json"), branch_dir.join("setup.json"))?;
    // Copy `setup.json` directly into `branch_dir/off/setup.json` too? Let's test `off` via parent setup and
    // `control` via direct setup copy!
    let mut ctrl_direct_setup: EvalSetupFile =
        serde_json::from_str(&fs::read_to_string(play_dir.join("setup.json"))?)?;
    ctrl_direct_setup.arm = EvalArm::Control;
    ctrl_direct_setup.relay_wording_arm = CONTROL_WORDING_ARM.to_owned();
    fs::write(
        branch_dir.join("control/setup.json"),
        serde_json::to_string_pretty(&ctrl_direct_setup)?,
    )?;

    let ctrl_seats = branch_dir.join("control/relay/seats");
    fs::create_dir_all(ctrl_seats.join("ignored_subdir"))?;
    fs::write(
        ctrl_seats.join("Foundry.day-01-act.tool-round-ignore.txt"),
        b"txt",
    )?;
    // Copy existing tool-round JSON into an unrelated Talk-round record with same prefix pattern.
    for entry in fs::read_dir(&ctrl_seats)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("Foundry.day-01-act.tool-round-") && name.ends_with(".json") {
            let mut wrong_rec: SavedToolRoundRecord =
                serde_json::from_slice(&fs::read(entry.path())?)?;
            wrong_rec.round = Round::Talk;
            fs::write(
                ctrl_seats.join("Foundry.day-01-act.tool-round-999.json"),
                serde_json::to_string_pretty(&wrong_rec)?,
            )?;
            let mut wrong_resv: SavedToolRoundRecord =
                serde_json::from_slice(&fs::read(entry.path())?)?;
            wrong_resv.reservation_id = "other_unmatched_resv".to_owned();
            fs::write(
                ctrl_seats.join("Foundry.day-01-act.tool-round-998.json"),
                serde_json::to_string_pretty(&wrong_resv)?,
            )?;
            break;
        }
    }
    let ctrl_calls = branch_dir.join("control/relay/calls.jsonl");
    let calls_orig = fs::read_to_string(&ctrl_calls)?;
    fs::write(&ctrl_calls, format!("\n{calls_orig}\n"))?;

    let edge_blind = tmp.path().join("edge_blind");
    let edge_arm_map = tmp.path().join("edge_arm_map");
    let edge_report = run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir.clone(), branch_dir.clone()],
        out_blind_dir: edge_blind,
        out_arm_map_dir: edge_arm_map.clone(),
        out_checkpoints_dir: None,
        config_path: cfg_path.clone(),
        scorer_seed: 7,
    })?;
    assert_eq!(edge_report.states_built, 1);

    // Malformed `calls.jsonl` line & malformed tool-round JSON error paths.
    fs::write(&ctrl_calls, "{bad json line\n")?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![play_dir.clone(), branch_dir.clone()],
            out_blind_dir: tmp.path().join("ob_bc"),
            out_arm_map_dir: tmp.path().join("oa_bc"),
            out_checkpoints_dir: None,
            config_path: cfg_path.clone(),
            scorer_seed: 7,
        })
        .err()
        .ok_or("expected malformed calls.jsonl EVAL_IO")?
        .code(),
        "EVAL_IO"
    );
    fs::write(&ctrl_calls, calls_orig)?;

    fs::write(
        ctrl_seats.join("Foundry.day-01-act.tool-round-999.json"),
        b"{bad",
    )?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![play_dir.clone(), branch_dir.clone()],
            out_blind_dir: tmp.path().join("ob_btr"),
            out_arm_map_dir: tmp.path().join("oa_btr"),
            out_checkpoints_dir: None,
            config_path: cfg_path.clone(),
            scorer_seed: 7,
        })
        .err()
        .ok_or("expected malformed tool-round EVAL_IO")?
        .code(),
        "EVAL_IO"
    );
    fs::remove_file(ctrl_seats.join("Foundry.day-01-act.tool-round-999.json"))?;
    fs::remove_file(ctrl_seats.join("Foundry.day-01-act.tool-round-998.json"))?;

    // Missing pre-act checkpoint seal triggers `CHECKPOINT_NOT_FOUND`.
    let seal_file = play_dir.join("checkpoints/day-01-pre-act.seal");
    fs::remove_file(&seal_file)?;
    assert_eq!(
        run_build_blind_inputs(&BuildBlindInputsOptions {
            run_dirs: vec![play_dir, branch_dir],
            out_blind_dir: tmp.path().join("ob_mckpt"),
            out_arm_map_dir: tmp.path().join("oa_mckpt"),
            out_checkpoints_dir: None,
            config_path: cfg_path,
            scorer_seed: 7,
        })
        .err()
        .ok_or("expected missing pre-act checkpoint CHECKPOINT_NOT_FOUND")?
        .code(),
        "CHECKPOINT_NOT_FOUND"
    );

    let _ = edge_arm_map;
    Ok(())
}

#[test]
fn freeze_and_join_parse_and_candidate_error_paths() -> TestResult {
    let tmp = TmpDir::new("freeze_join_parse")?;
    let git_sha = clean_git_sha_source();
    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    // `run_freeze` error paths: missing config, malformed config, nonexistent --blind dir.
    let unused_vc = tmp.path().join("unused_vc.json");
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: tmp.path().join("missing_blind"),
                config_path: tmp.path().join("no_cfg.json"),
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("nf.json"),
                verified_checkpoints_path: unused_vc.clone(),
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected freeze missing config EVAL_IO")?
        .code(),
        "EVAL_IO"
    );
    let bad_cfg = tmp.path().join("bad_cfg.json");
    fs::write(&bad_cfg, b"{bad")?;
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: tmp.path().join("missing_blind"),
                config_path: bad_cfg,
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("nf2.json"),
                verified_checkpoints_path: unused_vc.clone(),
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected freeze bad config EVAL_IO")?
        .code(),
        "EVAL_IO"
    );
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: tmp.path().join("missing_blind"),
                config_path: cfg_path.clone(),
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("nf3.json"),
                verified_checkpoints_path: unused_vc,
                scorer_crate_dir: None,
            },
            &git_sha,
        )
        .err()
        .ok_or("expected freeze missing --blind FREEZE_MANIFEST_MISMATCH")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    // Invalid `--error-format` on each CLI subcommand.
    for cmd in ["build-blind-inputs", "verify-checkpoints", "freeze", "join"] {
        let (code, _, err) = run_eval_cli(&[cmd, "--error-format", "xml"], &git_sha);
        assert_eq!(code, 1, "expected invalid error-format for {cmd}: {err}");
        assert!(err.contains("INVALID_USAGE"));
    }

    // Build minimal synthetic fixture with control arm to exercise remaining `run_join` branches.
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let transport = SyntheticPipelineTransport;

    let play_dir = tmp.path().join("play_j");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9109,
            seed: 19,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let branch_dir = tmp.path().join("branch_j");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir.clone(), branch_dir.clone()],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 23,
    })?;

    let scores_dir = tmp.path().join("scores");
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        ),
        0
    );

    // Freeze written into a newly created nested subdirectory.
    let vc_path = verify_ckpts_for_test(
        &ckpts_dir,
        &tmp.path().join("verified_checkpoints.json"),
        Some(&blind_dir),
        &git_sha,
    )?;
    let nested_freeze_path = tmp.path().join("nested/dir/scorer_freeze.json");
    let _freeze = run_freeze(
        &FreezeOptions {
            blind_dir,
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: nested_freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;

    let sid = format!("{}-day-01-seat-0", play_manifest.game_id);
    let out_fail = tmp.path().join("out_fail");

    // Malformed freeze JSON, nonexistent --arm-map, malformed arm_map JSON, malformed score JSON.
    let bad_freeze_json = tmp.path().join("bad_freeze.json");
    fs::write(&bad_freeze_json, b"{bad")?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: bad_freeze_json,
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected bad freeze json EVAL_IO")?
        .code(),
        "EVAL_IO"
    );
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: tmp.path().join("missing_arm_map"),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected missing arm_map EVAL_IO")?
        .code(),
        "EVAL_IO"
    );

    let bad_am_dir = tmp.path().join("bad_am");
    fs::create_dir_all(&bad_am_dir)?;
    fs::write(bad_am_dir.join(format!("{sid}.json")), b"{bad")?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: bad_am_dir,
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected bad arm_map EVAL_IO")?
        .code(),
        "EVAL_IO"
    );

    let bad_scores_dir = tmp.path().join("bad_scores");
    fs::create_dir_all(&bad_scores_dir)?;
    fs::write(bad_scores_dir.join(format!("{sid}.json")), b"{bad")?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: bad_scores_dir,
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected bad score EVAL_IO")?
        .code(),
        "EVAL_IO"
    );

    // Control setup.json malformed & missing; manifest.json malformed & missing; relay/setup.json malformed.
    let setup_path = play_dir.join("setup.json");
    let setup_orig = fs::read_to_string(&setup_path)?;
    fs::write(&setup_path, b"{bad")?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected malformed setup.json")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::remove_file(&setup_path)?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected missing setup.json")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::write(&setup_path, setup_orig)?;

    let ctrl_manifest_path = branch_dir.join("control/manifest.json");
    let manifest_orig = fs::read_to_string(&ctrl_manifest_path)?;
    fs::write(&ctrl_manifest_path, b"{bad")?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected malformed manifest.json")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::remove_file(&ctrl_manifest_path)?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected missing manifest.json")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::write(&ctrl_manifest_path, manifest_orig)?;

    let relay_setup_path = branch_dir.join("control/relay/setup.json");
    let relay_setup_orig = fs::read_to_string(&relay_setup_path)?;
    fs::write(&relay_setup_path, b"{bad")?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: arm_map_dir.clone(),
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected malformed relay/setup.json")?
        .code(),
        "CONTROL_WORDING_ARM_MISMATCH"
    );
    fs::write(&relay_setup_path, relay_setup_orig)?;

    // Exercise `find_candidate_score`: missing committed_candidate_id and missing first_draft_candidate_id
    // while `blind_candidate_ids` still matches `score.candidates`.
    let mut am_missing_commit: ArmMapStateFile =
        serde_json::from_slice(&fs::read(arm_map_dir.join(format!("{sid}.json")))?)?;
    let real_commit_id = am_missing_commit.entries[0].committed_candidate_id.clone();
    am_missing_commit.entries[0].committed_candidate_id = "absent_commit_cid".to_owned();
    let am_mc_dir = tmp.path().join("am_mc");
    fs::create_dir_all(&am_mc_dir)?;
    fs::write(
        am_mc_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&am_missing_commit)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path.clone(),
            arm_map_dir: am_mc_dir,
            scores_dir: scores_dir.clone(),
            out_dir: out_fail.clone(),
            verified_checkpoints_path: Some(vc_path.clone()),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected missing committed candidate in score.candidates")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    am_missing_commit.entries[0].committed_candidate_id = real_commit_id;
    am_missing_commit.entries[0].first_draft_candidate_id = Some("absent_draft_cid".to_owned());
    let am_md_dir = tmp.path().join("am_md");
    fs::create_dir_all(&am_md_dir)?;
    fs::write(
        am_md_dir.join(format!("{sid}.json")),
        to_sorted_pretty_json(&am_missing_commit)?,
    )?;
    assert_eq!(
        run_join(&JoinOptions {
            freeze_path: nested_freeze_path,
            arm_map_dir: am_md_dir,
            scores_dir,
            out_dir: out_fail,
            verified_checkpoints_path: Some(vc_path),
            blind_dir: None,
            source_game_dirs: Vec::new(),
        })
        .err()
        .ok_or("expected missing first_draft candidate in score.candidates")?
        .code(),
        "FREEZE_MANIFEST_MISMATCH"
    );

    Ok(())
}

#[test]
fn talk_rules_v2_digest_pins_every_table_field_and_source_bytes() -> TestResult {
    let base = TalkRulesV2Table::frozen();
    let sample_src = b"fn sample() -> i64 { 1000 }\r\n";
    let base_digest = talk_rules_v2_digest_for(&base, sample_src)?;

    // CRLF and LF source bytes normalize identically across platforms.
    assert_eq!(
        base_digest,
        talk_rules_v2_digest_for(&base, b"fn sample() -> i64 { 1000 }\n")?
    );

    // Changing any byte of source changes the digest.
    let mut_src_digest = talk_rules_v2_digest_for(&base, b"fn sample() -> i64 { 1001 }\n")?;
    assert_ne!(mut_src_digest, base_digest);

    // Mutating any single field of `TalkRulesV2Table` changes the digest.
    let mut t = base.clone();
    t.allow_guild_then_prefix_goods = false;
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.allow_thousands_comma_groups = false;
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.allowed_ascii_chars = "abc";
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.buy_tokens = &["bid"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.clause_break_chars = &[";"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.colon_space_breaks_clause = false;
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.colon_space_lead_templates = &[&["hi"]];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.commitment_templates = &[&["i", "sell", "<AMOUNT>", "<GOOD>", "<PRICE_CUE>", "<PRICE>"]];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.compound_good_separators = &["_"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.contradiction_latches_pair_asset = false;
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.cred_tokens = &["cred"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.decimal_dot_keeps_clause = false;
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.negation_prefixes = &[&["won't"]];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.neutral_templates = &[&["hi"]];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.number_unit_scale = 1;
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.price_cues = &["at"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.quote_normalizations = &[("\u{2019}", "'")];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.raw_prefixes = &["r"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.refined_prefixes = &["refined"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.retraction_optional_lead_tokens = &["now"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.retraction_templates = &[&["cancel"]];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.sell_tokens = &["ask"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.standalone_tokens = &[];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base.clone();
    t.token_join_chars = &["_"];
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    let mut t = base;
    t.version = "talk_rules_v99";
    assert_ne!(talk_rules_v2_digest_for(&t, sample_src)?, base_digest);

    // Pinned live digest must equal scorer constant.
    assert_eq!(talk_rules_v2_digest()?, TALK_EXTRACTOR_RULES_DIGEST_V2);
    Ok(())
}

#[test]
fn talk_v2_scaling_decimals_overflow_merge_and_retraction_scoping() -> TestResult {
    let tmp = TmpDir::new("talk_v2")?;
    let host = EvalHost::from_standard_setup(TableId(9110), 4, 1, 0)?;
    let obs = host.observe_seat(league_core::Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    // Clear scaled offer and byte-identical repeat determinism.
    let clear = extract_talk_commitments_from_messages(
        0,
        &seat_guilds,
        &[(1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned())],
        &obs,
    )?;
    assert_eq!(clear.len(), 1);
    assert_eq!(
        clear[0].offered_action,
        MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(1),
                amount: 1_000 * UNIT_SCALE,
                price_per_unit_cred: 250 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        }
    );
    let clear_repeat = extract_talk_commitments_from_messages(
        0,
        &seat_guilds,
        &[(1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned())],
        &obs,
    )?;
    assert_eq!(
        to_sorted_compact_json(&clear)?,
        to_sorted_compact_json(&clear_repeat)?
    );

    // Ambiguous, decimal, zero, and overflow quantities/prices are all refused cleanly.
    for rejected in [
        "Maybe I will sell 1000 raw 1 at 250 CRED.",
        "Will sell 1000 raw 1 at 250 or 300 CRED.",
        "If you agree, I will sell 1000 raw 1 at 250 CRED?",
        "I offer 1000 raw 1 at 250 CRED.",
        "I will sell 1.5 raw 1 at 250 CRED.",
        "I will sell 1000 raw 1 at 25.5 CRED.",
        "I will sell 0 raw 1 at 250 CRED.",
        "I will sell 99999999999999999 raw 1 at 250 CRED.",
        "I will sell 10 raw 1 at 999999999999999999 CRED.",
        "I will sell 99999999999999999999 raw 1 at 10 CRED.",
    ] {
        let out = extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[(1, 0, rejected.to_owned())],
            &obs,
        )?;
        assert!(
            out.is_empty(),
            "expected empty for `{rejected}`, got {out:?}"
        );
    }

    // Later explicit retraction and contradiction drop the commitment.
    assert!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[
                (1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned()),
                (1, 0, "Actually, I retract my offer on raw 1.".to_owned()),
            ],
            &obs,
        )?
        .is_empty()
    );
    assert!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[
                (1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned()),
                (1, 0, "I will sell 500 raw 1 at 300 CRED.".to_owned()),
            ],
            &obs,
        )?
        .is_empty()
    );

    // Retraction scoping: listed neutral greeting keeps the clean offer; unlisted filler or
    // verb-governed negation retracts/fails closed cleanly.
    let expected_five = vec![league_scorer::ExtractedTalkCommitment {
        proposer_seat: 1,
        counterparty_seat: 0,
        offered_action: MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 5 * UNIT_SCALE,
                price_per_unit_cred: 10 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        },
    }];
    assert_eq!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[(1, 0, "Hi: I will sell 5 raw 0 at 10".to_owned())],
            &obs,
        )?,
        expected_five
    );
    // Unlisted filler clauses fail closed even if paired with a valid offer clause.
    for unlisted_msg in [
        "Don't worry: I will sell 5 raw 0 at 10",
        "Don't worry, I will sell 5 raw 0 at 10",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, unlisted_msg.to_owned())],
                &obs,
            )?
            .is_empty()
        );
    }
    assert!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[
                (1, 0, "I will sell 5 raw 0 at 10".to_owned()),
                (1, 0, "I won't sell raw 0".to_owned()),
            ],
            &obs,
        )?
        .is_empty()
    );

    // Cross-source merge policy across `to-seat/*.negotiation.json` and `decisions/day-01-talk-*.json`:
    // Retraction in one source must NEVER be resurrected by a copy of the offer in the other.
    let make_talk_rec = |seat: u8, guild: &str, messages: Value| EvalDecisionRecord {
        game_id: host.game_id().to_owned(),
        table_id: 9110,
        day: 1,
        round: Round::Talk,
        seat,
        guild: guild.to_owned(),
        arm: EvalArm::Off,
        seat_kind: "Model".to_owned(),
        keeps_notes: false,
        notes_present: false,
        raw_reply: Some(
            json!({
                "messages": messages,
                "market_actions": [],
                "bundle": {}
            })
            .to_string(),
        ),
        discarded_reply_path: None,
        discarded_reply_sha256: None,
        attempts: 1,
        failed_attempts: Vec::new(),
        prompt_tokens: 1,
        completion_tokens: 1,
        wall_secs: None,
        tool_calls_used: 0,
        cleared_non_empty_actions: false,
        prompt_token_ids_sha256s: Vec::new(),
        prompt_token_ids_source: None,
        prompt_token_ids_template_check: None,
        reply_token_ids: Vec::new(),
        reply_top_logprobs: Vec::new(),
        intermediate_bundle_sha256s: Vec::new(),
        refused_market_actions: 0,
        refused_bundle_legs: 0,
        parse_failure: false,
        refusal_code: None,
        refusal_reason: None,
        loaded_history_sha256: BTreeMap::new(),
        invalid_recipients: Vec::new(),
        no_reply: false,
        apply_report: None,
        refused_bundle: None,
        club_bot_baseline: None,
    };

    // Case A: Retracted in negotiation.json while talk reply still repeats the offer -> empty.
    let case_neg_retract = tmp.path().join("neg_retract");
    fs::create_dir_all(case_neg_retract.join("to-seat"))?;
    fs::create_dir_all(case_neg_retract.join("decisions"))?;
    fs::write(
        case_neg_retract.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![
                UntrustedWhisper {
                    from_guild: "Quarry".to_owned(),
                    intent_header: None,
                    body: "I will sell 5 raw 0 at 10.".to_owned(),
                },
                UntrustedWhisper {
                    from_guild: "Quarry".to_owned(),
                    intent_header: None,
                    body: "Actually I retract raw 0.".to_owned(),
                },
            ],
        })?,
    )?;
    fs::write(
        case_neg_retract.join("decisions/day-01-talk-Quarry.json"),
        serde_json::to_string_pretty(&make_talk_rec(
            1,
            "Quarry",
            json!([{ "to_guild": "Foundry", "body": "I will sell 5 raw 0 at 10." }]),
        ))?,
    )?;
    assert!(
        extract_talk_commitments_for_state(&case_neg_retract, 1, 0, &seat_guilds, &obs)?.is_empty()
    );

    // Case B: Offered in negotiation.json while retracted (named & global) in talk reply -> empty.
    let case_talk_retract = tmp.path().join("talk_retract");
    fs::create_dir_all(case_talk_retract.join("to-seat"))?;
    fs::create_dir_all(case_talk_retract.join("decisions"))?;
    fs::write(
        case_talk_retract.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "I will sell 5 raw 0 at 10.".to_owned(),
            }],
        })?,
    )?;
    fs::write(
        case_talk_retract.join("decisions/day-01-talk-Quarry.json"),
        serde_json::to_string_pretty(&make_talk_rec(
            1,
            "Quarry",
            json!([{ "to_guild": "Foundry", "body": "Cancel that, no deal." }]),
        ))?,
    )?;
    assert!(
        extract_talk_commitments_for_state(&case_talk_retract, 1, 0, &seat_guilds, &obs)?
            .is_empty()
    );

    // Case C: Distinct non-retracted offers across negotiation + talk survive; conflicting prices drop.
    let case_union = tmp.path().join("union_ok");
    fs::create_dir_all(case_union.join("to-seat"))?;
    fs::create_dir_all(case_union.join("decisions"))?;
    fs::write(
        case_union.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![
                UntrustedWhisper {
                    from_guild: "Quarry".to_owned(),
                    intent_header: None,
                    body: "I will sell 5 raw 0 at 10.".to_owned(),
                },
                UntrustedWhisper {
                    from_guild: "Harbor".to_owned(),
                    intent_header: None,
                    body: "I will buy 4 raw 1 at 20.".to_owned(),
                },
            ],
        })?,
    )?;
    fs::write(
        case_union.join("decisions/day-01-talk-Quarry.json"),
        serde_json::to_string_pretty(&make_talk_rec(
            1,
            "Quarry",
            json!([{ "to_guild": "Foundry", "body": "I will buy 3 refined 0 at 50." }]),
        ))?,
    )?;
    fs::write(
        case_union.join("decisions/day-01-talk-Harbor.json"),
        serde_json::to_string_pretty(&make_talk_rec(
            2,
            "Harbor",
            json!([{ "to_guild": "Foundry", "body": "I will buy 4 raw 1 at 30." }]),
        ))?,
    )?;
    let union_out = extract_talk_commitments_for_state(&case_union, 1, 0, &seat_guilds, &obs)?;
    assert_eq!(
        union_out,
        vec![
            league_scorer::ExtractedTalkCommitment {
                proposer_seat: 1,
                counterparty_seat: 0,
                offered_action: MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Refined(0),
                        amount: 3 * UNIT_SCALE,
                        price_per_unit_cred: 50 * UNIT_SCALE,
                        domain_id: None,
                        is_hybrid: false,
                    },
                },
            },
            league_scorer::ExtractedTalkCommitment {
                proposer_seat: 1,
                counterparty_seat: 0,
                offered_action: MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: AssetId::Raw(0),
                        amount: 5 * UNIT_SCALE,
                        price_per_unit_cred: 10 * UNIT_SCALE,
                        domain_id: None,
                        is_hybrid: false,
                    },
                },
            },
        ]
    );

    // Small whole-unit amount (`2 raw 1`) must parse as amount=2*UNIT_SCALE on Raw(1), not `2 raw`.
    let small_unit_out = extract_talk_commitments_from_messages(
        0,
        &seat_guilds,
        &[(1, 0, "I will sell 2 raw 1 at 250 CRED.".to_owned())],
        &obs,
    )?;
    assert_eq!(
        small_unit_out,
        vec![league_scorer::ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 0,
            offered_action: MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    amount: 2 * UNIT_SCALE,
                    price_per_unit_cred: 250 * UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
            },
        }]
    );

    Ok(())
}

#[test]
fn join_refuses_scores_from_rebuilt_blind_inputs_with_different_talk_commitments() -> TestResult {
    let tmp = TmpDir::new("talk_tamper")?;
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let git_sha = clean_git_sha_source();
    let transport = SyntheticPipelineTransport;

    // 1. Build run A with initial talk commitments and freeze A.
    let play_dir = tmp.path().join("play_a");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9111,
            seed: 31,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    let branch_dir = tmp.path().join("branch_a");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let blind_a = tmp.path().join("blind_a");
    let arm_map_a = tmp.path().join("arm_map_a");
    let ckpts_a = tmp.path().join("ckpts_a");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir.clone(), branch_dir.clone()],
        out_blind_dir: blind_a.clone(),
        out_arm_map_dir: arm_map_a.clone(),
        out_checkpoints_dir: Some(ckpts_a.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 42,
    })?;

    let vc_a = verify_ckpts_for_test(
        &ckpts_a,
        &tmp.path().join("vc_a.json"),
        Some(&blind_a),
        &git_sha,
    )?;
    let freeze_a = tmp.path().join("freeze_a.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_a.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_a.clone(),
            verified_checkpoints_path: vc_a.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;

    // 2. Modify talk commitments in `play_a`'s recorded talk inputs, rebuild B with identical
    // `candidate_id`s, score B, then attempt `join` against A's freeze + A's `arm_map/` (without `--blind`).
    let neg_path = play_dir.join("to-seat/day-01-act-Foundry.negotiation.json");
    fs::write(
        &neg_path,
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: eval_wording_slot("eval_batna").to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "I commit to buy 200 raw 1 at 300 CRED.".to_owned(),
            }],
        })?,
    )?;
    let talk_dec_path = play_dir.join("decisions/day-01-talk-Quarry.json");
    let mut talk_dec: EvalDecisionRecord = serde_json::from_slice(&fs::read(&talk_dec_path)?)?;
    talk_dec.raw_reply = Some(
        json!({
            "messages": [{ "to_guild": "Foundry", "body": "I commit to buy 200 raw 1 at 300 CRED." }],
            "market_actions": [],
            "bundle": {}
        })
        .to_string(),
    );
    fs::write(&talk_dec_path, serde_json::to_string_pretty(&talk_dec)?)?;

    let blind_b = tmp.path().join("blind_b");
    let arm_map_b = tmp.path().join("arm_map_b");
    let ckpts_b = tmp.path().join("ckpts_b");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir, branch_dir],
        out_blind_dir: blind_b.clone(),
        out_arm_map_dir: arm_map_b,
        out_checkpoints_dir: Some(ckpts_b.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 42,
    })?;

    let sid = format!("{}-day-01-seat-0", play_manifest.game_id);
    let req_a: BlindScoringRequest =
        serde_json::from_slice(&fs::read(blind_a.join(format!("{sid}.json")))?)?;
    let req_b: BlindScoringRequest =
        serde_json::from_slice(&fs::read(blind_b.join(format!("{sid}.json")))?)?;
    let cids_a: Vec<String> = req_a
        .candidates
        .iter()
        .map(|c| c.candidate_id.clone())
        .collect();
    let cids_b: Vec<String> = req_b
        .candidates
        .iter()
        .map(|c| c.candidate_id.clone())
        .collect();
    assert_eq!(
        cids_a, cids_b,
        "candidate IDs must remain identical across A and B"
    );
    assert_ne!(
        req_a.talk_commitments, req_b.talk_commitments,
        "talk_commitments must differ between A and B"
    );

    let scores_b = tmp.path().join("scores_b");
    let mut sb_out = Vec::new();
    let mut sb_err = Vec::new();
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_b.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_b.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_b.to_str().ok_or("utf8")?,
            ],
            &mut sb_out,
            &mut sb_err,
        ),
        0,
        "score B failed: {}",
        String::from_utf8_lossy(&sb_err)
    );

    let joined_tampered = tmp.path().join("joined_tampered");
    let (join_code, _join_out, join_err) = run_eval_cli(
        &[
            "join",
            "--freeze",
            freeze_a.to_str().ok_or("utf8")?,
            "--arm-map",
            arm_map_a.to_str().ok_or("utf8")?,
            "--scores",
            scores_b.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_a.to_str().ok_or("utf8")?,
            "--out",
            joined_tampered.to_str().ok_or("utf8")?,
        ],
        &git_sha,
    );
    assert_eq!(join_code, 1);
    assert!(
        join_err.contains("FREEZE_MANIFEST_MISMATCH"),
        "expected FREEZE_MANIFEST_MISMATCH, got: {join_err}"
    );
    eprintln!(
        "REBUILT_TALK_JOIN_REJECTED: exit={join_code} {}",
        join_err.trim()
    );
    Ok(())
}

#[test]
fn relay_setup_without_wording_arm_key_refused_with_control_wording_arm_mismatch() -> TestResult {
    let tmp = TmpDir::new("relay_setup_missing_key")?;
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let git_sha = clean_git_sha_source();
    let transport = SyntheticPipelineTransport;

    let play_dir = tmp.path().join("play_r");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9112,
            seed: 41,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    let branch_dir = tmp.path().join("branch_r");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir, branch_dir.clone()],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 17,
    })?;
    let scores_dir = tmp.path().join("scores");
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        ),
        0
    );
    let vc_path = verify_ckpts_for_test(
        &ckpts_dir,
        &tmp.path().join("vc_r.json"),
        Some(&blind_dir),
        &git_sha,
    )?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir,
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;

    // Overwrite `control/relay/setup.json` with `{}` (missing `"wording_arm"` key) -> refused!
    let sid = format!("{}-day-01-seat-0", play_manifest.game_id);
    let relay_setup_path = branch_dir.join("control/relay/setup.json");
    fs::write(&relay_setup_path, b"{}")?;
    let err_missing_key = run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.path().join("joined_fail"),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected CONTROL_WORDING_ARM_MISMATCH when relay/setup.json lacks wording_arm")?;
    assert_eq!(err_missing_key.code(), "CONTROL_WORDING_ARM_MISMATCH");

    // Non-string `"wording_arm"` is also refused (`CONTROL_WORDING_ARM_MISMATCH`).
    fs::write(&relay_setup_path, b"{\"wording_arm\":123}")?;
    let err_non_str = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: tmp.path().join("joined_fail2"),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected CONTROL_WORDING_ARM_MISMATCH on non-string wording_arm")?;
    assert_eq!(err_non_str.code(), "CONTROL_WORDING_ARM_MISMATCH");
    let _ = sid;
    Ok(())
}

#[test]
fn talk_v2_curly_apostrophe_and_non_ascii_rejection() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9113), 4, 1, 0)?;
    let obs = host.observe_seat(league_core::Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    // Typographic apostrophe refusals (U+2019, U+2018, U+02BC, U+FF07) normalize to ASCII `'`,
    // never invent an offer, and retract a prior matching offer.
    for msg in [
        "I won\u{2019}t sell 5 raw 0 at 10",
        "I won\u{2018}t sell 5 raw 0 at 10",
        "I can\u{02bc}t sell 5 raw 0 at 10",
        "I don\u{ff07}t commit to sell 5 raw 0 at 10",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, msg.to_owned())],
                &obs,
            )?
            .is_empty(),
            "curly-apostrophe refusal must yield empty: `{msg}`"
        );
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[
                    (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                    (1, 0, msg.to_owned()),
                ],
                &obs,
            )?
            .is_empty(),
            "curly-apostrophe refusal must retract prior offer: `{msg}`"
        );
    }

    // Listed neutral clause preserves valid ASCII offers; ASCII and curly double quotes fail closed.
    let expected_five = vec![league_scorer::ExtractedTalkCommitment {
        proposer_seat: 1,
        counterparty_seat: 0,
        offered_action: MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 5 * UNIT_SCALE,
                price_per_unit_cred: 10 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        },
    }];
    assert_eq!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[(1, 0, "Hello: I will sell 5 raw 0 at 10".to_owned())],
            &obs,
        )?,
        expected_five
    );
    for quoted in [
        "\"I will sell 5 raw 0 at 10\"",
        "\u{201c}I will sell 5 raw 0 at 10\u{201d}",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, quoted.to_owned())],
                &obs,
            )?
            .is_empty(),
            "quoted message must fail closed: `{quoted}`"
        );
    }

    // Ambiguous whole messages (unlisted em-dash, non-ASCII text/scripts, unmapped prime,
    // backtick, `~`, `<`, `(`, or fail-closed negation + trade verb) yield nothing and latch
    // their `(proposer, counterparty)` pair so earlier or later offers for that pair are cancelled.
    for non_ascii in [
        "Не буду sell 5 raw 0 at 10",
        "Не продаю: I won\u{2019}t sell 5 raw 0 at 10; Не буду sell 5 raw 0 at 10",
        "私は sell 5 raw 0 at 10",
        "I will sell 5 raw 0 at 10 caf\u{00e9}",
        "I won\u{2032}t sell 5 raw 0 at 10",
        "I will sell 5 raw 0 at 10. Actually\u{2014}cancel that.",
        "I will sell 5 raw 0 at 10. Отменяю.",
        "I will sell 5 raw 0 at 10. Не буду sell 4 raw 1 at 20.",
        "I will sell 5 raw 0 at 10. I won\u{2032}t sell raw 0.",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, non_ascii.to_owned())],
                &obs,
            )?
            .is_empty(),
            "ambiguous non-ASCII/unlisted message must fail closed: `{non_ascii}`"
        );
    }
    Ok(())
}

#[test]
fn talk_v2_per_message_fail_closed_ambiguity_latches_pair_and_negation() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9115), 4, 1, 0)?;
    let obs = host.observe_seat(league_core::Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    let expected_five = vec![league_scorer::ExtractedTalkCommitment {
        proposer_seat: 1,
        counterparty_seat: 0,
        offered_action: MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 5 * UNIT_SCALE,
                price_per_unit_cred: 10 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        },
    }];

    // Single-message and cross-message ambiguity (`\u{2014}`) cancels earlier and later offers on
    // that pair, without disturbing clean offers from another pair.
    assert!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[
                (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                (1, 0, "Actually\u{2014}cancel that.".to_owned()),
                (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
            ],
            &obs,
        )?
        .is_empty()
    );
    assert_eq!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[
                (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                (2, 0, "Actually\u{2014}cancel that.".to_owned()),
            ],
            &obs,
        )?,
        expected_five
    );

    // Disallowed ASCII characters (`~`, `<`, `(`, `` ` ``) and non-refusal negation + verb cues
    // all fail closed and cancel any prior offer on that pair.
    for msg in [
        "I won`t sell 5 raw 0 at 10",
        "I will sell 5 raw 0 at ~10",
        "I will sell 5 raw 0 at <10",
        "I will sell 5 raw 0 at (10)",
        "I am not selling 5 raw 0 at 10",
        "I never sell 5 raw 0 at 10",
        "I wont sell 5 raw 0 at 10",
        "Neither buy nor sell 5 raw 0 at 10",
        "I have no offer to sell 5 raw 0 at 10",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, msg.to_owned())],
                &obs,
            )?
            .is_empty(),
            "fail-closed message must yield empty: `{msg}`"
        );
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[
                    (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                    (1, 0, msg.to_owned()),
                    (1, 0, "I will sell 6 raw 0 at 12.".to_owned()),
                ],
                &obs,
            )?
            .is_empty(),
            "fail-closed message must latch pair empty: `{msg}`"
        );
    }
    Ok(())
}

fn verify_talk_rules_sec52_doc(doc_text: &str) -> Result<(), String> {
    let sec_start = doc_text
        .find("#### 5.2.1 Frozen Talk Extractor Rule Table")
        .ok_or_else(|| "missing `#### 5.2.1` section in docs/decision-scorer.md".to_owned())?;
    let sec_end = doc_text[sec_start..]
        .find("## 6. Horizon Valuation")
        .map(|off| sec_start + off)
        .ok_or_else(|| "missing `## 6. Horizon Valuation` after `#### 5.2.1`".to_owned())?;
    let section = &doc_text[sec_start..sec_end];

    let start_marker = "<!-- talk_rules_v2_table:start -->";
    let end_marker = "<!-- talk_rules_v2_table:end -->";
    let block_start = section
        .find(start_marker)
        .ok_or_else(|| format!("missing `{start_marker}` marker"))?
        + start_marker.len();
    let block_end = section[block_start..]
        .find(end_marker)
        .map(|off| block_start + off)
        .ok_or_else(|| format!("missing `{end_marker}` marker"))?;
    let prose_part = &section[..block_start - start_marker.len()];
    let fenced = section[block_start..block_end].trim();
    let doc_json = fenced
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .ok_or_else(|| {
            "expected fenced ```json block inside talk_rules_v2_table markers".to_owned()
        })?;

    let doc_json_lf = doc_json.replace("\r\n", "\n");
    let expected_json = to_sorted_pretty_json(&TalkRulesV2Table::frozen())
        .map_err(|e| format!("serialize frozen table: {e}"))?;
    if doc_json_lf != expected_json.trim() {
        return Err(format!(
            "docs/decision-scorer.md §5.2.1 talk_rules_v2 JSON differs from TalkRulesV2Table::frozen():\ndoc:\n{doc_json_lf}\nexpected:\n{expected_json}"
        ));
    }

    let doc_val: Value =
        serde_json::from_str(&doc_json_lf).map_err(|e| format!("parse doc table JSON: {e}"))?;
    let obj = doc_val
        .as_object()
        .ok_or_else(|| "talk_rules_v2 JSON must be an object".to_owned())?;
    let json_keys: BTreeSet<String> = obj.keys().cloned().collect();

    let mut bullet_keys = BTreeSet::new();
    for line in prose_part.lines() {
        if let Some(after) = line.trim_start().strip_prefix("- **`")
            && let Some((name, _rest)) = after.split_once("`:**")
            && !bullet_keys.insert(name.to_owned())
        {
            return Err(format!("duplicate bullet for `{name}` in §5.2.1"));
        }
    }
    if bullet_keys != json_keys {
        return Err(format!(
            "§5.2.1 bullet keys {bullet_keys:?} disagree with TalkRulesV2Table fields {json_keys:?}"
        ));
    }
    Ok(())
}

#[test]
fn talk_rules_v2_sec52_doc_matches_frozen_table() -> TestResult {
    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/decision-scorer.md");
    let doc_text = fs::read_to_string(&doc_path)?;
    verify_talk_rules_sec52_doc(&doc_text).map_err(std::io::Error::other)?;

    // Negative controls proving doc drift fails immediately.
    let bad_val = doc_text.replacen(
        "\"allow_thousands_comma_groups\": true",
        "\"allow_thousands_comma_groups\": false",
        1,
    );
    assert_ne!(bad_val, doc_text);
    assert!(verify_talk_rules_sec52_doc(&bad_val).is_err());

    let missing_bullet = doc_text.replacen("- **`quote_normalizations`:**", "- **`other`:**", 1);
    assert_ne!(missing_bullet, doc_text);
    assert!(verify_talk_rules_sec52_doc(&missing_bullet).is_err());
    Ok(())
}

#[test]
fn colon_space_lead_templates_are_subset_of_neutral_templates() -> TestResult {
    let rules = TalkRulesV2Table::frozen();
    let neutral_set: BTreeSet<&[&str]> = rules.neutral_templates.iter().copied().collect();
    for lead in rules.colon_space_lead_templates {
        assert!(
            neutral_set.contains(lead),
            "colon_space_lead_templates entry {lead:?} missing from neutral_templates"
        );
    }
    assert!(!rules.colon_space_lead_templates.is_empty());
    Ok(())
}

#[test]
fn write_freeze_file_removes_partial_file_on_write_error_allowing_retry() -> TestResult {
    let tmp = TmpDir::new("freeze_partial_cleanup")?;
    let freeze_path = tmp.path().join("scorer_freeze.json");

    let err = write_freeze_file_with_writer(&freeze_path, |file| {
        file.write_all(b"{\"partial\":")?;
        Err(std::io::Error::other("simulated write failure"))
    })
    .err()
    .ok_or("expected write_freeze_file_with_writer to return Io error")?;
    assert_eq!(err.code(), "EVAL_IO");
    assert!(
        !freeze_path.exists(),
        "partial freeze file must be deleted on write failure"
    );

    // Retry succeeds cleanly instead of failing with FREEZE_FILE_EXISTS.
    write_freeze_file_with_writer(&freeze_path, |file| file.write_all(b"{\"complete\":true}"))?;
    assert_eq!(fs::read(&freeze_path)?, b"{\"complete\":true}");

    // Existing completed freeze file still refuses overwrite with FREEZE_FILE_EXISTS.
    let exists_err = write_freeze_file_with_writer(&freeze_path, |file| file.write_all(b"again"))
        .err()
        .ok_or("expected FREEZE_FILE_EXISTS on completed file")?;
    assert_eq!(exists_err.code(), "FREEZE_FILE_EXISTS");
    Ok(())
}

#[test]
fn join_refuses_duplicate_state_id_in_scores_and_arm_map() -> TestResult {
    let tmp = TmpDir::new("join_dup_state_id")?;
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let git_sha = clean_git_sha_source();
    let transport = SyntheticPipelineTransport;

    let play_dir = tmp.path().join("play_d");
    let play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9114,
            seed: 51,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    let branch_dir = tmp.path().join("branch_d");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Control],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir, branch_dir],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 19,
    })?;
    let scores_dir = tmp.path().join("scores");
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        ),
        0
    );
    let vc_path = verify_ckpts_for_test(
        &ckpts_dir,
        &tmp.path().join("vc_dup.json"),
        Some(&blind_dir),
        &git_sha,
    )?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir,
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;

    let sid = format!("{}-day-01-seat-0", play_manifest.game_id);

    // 1. Duplicate `state_id` across score files must be refused with DUPLICATE_STATE_ID.
    let dup_score_path = scores_dir.join("scores").join(format!("{sid}-dup.json"));
    fs::copy(
        scores_dir.join("scores").join(format!("{sid}.json")),
        &dup_score_path,
    )?;
    let err_dup_score = run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.path().join("out_dup_score"),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected DUPLICATE_STATE_ID on duplicate score state_id")?;
    assert_eq!(err_dup_score.code(), "DUPLICATE_STATE_ID");
    assert!(
        err_dup_score.to_string().contains("duplicate state_id"),
        "unexpected error: {err_dup_score}"
    );
    fs::remove_file(&dup_score_path)?;

    // 2. Duplicate `state_id` across arm_map/ files must be refused with DUPLICATE_STATE_ID.
    let dup_am_path = arm_map_dir.join(format!("{sid}-dup.json"));
    fs::copy(arm_map_dir.join(format!("{sid}.json")), &dup_am_path)?;
    let err_dup_am = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: tmp.path().join("out_dup_am"),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected DUPLICATE_STATE_ID on duplicate arm_map state_id")?;
    assert_eq!(err_dup_am.code(), "DUPLICATE_STATE_ID");
    assert!(
        err_dup_am.to_string().contains("duplicate state_id"),
        "unexpected error: {err_dup_am}"
    );
    fs::remove_file(&dup_am_path)?;
    Ok(())
}

#[test]
fn talk_v2_closed_grammar_rejects_unlisted_clauses_and_latches_pair() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9116), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    for bad_msg in [
        "I will sell 5 raw 0 at 10. No.",
        "I will sell 5 raw 0 at 10. Not.",
        "I will sell 5 raw 0 at 10. Never!",
        "I will sell 5 raw 0 at 10. Maybe not.",
        "I will sell 5 raw 0 at 10. Maybe.",
        "I will sell 5 raw 0 at 10. If you pay first.",
        "I will sell 5 raw 0 at 10. Unless Foundry objects.",
        "I wouldn't sell 5 raw 0 at 10",
        "I couldn't sell 5 raw 0 at 10",
        "I didn't sell 5 raw 0 at 10",
        "I refuse to sell 5 raw 0 at 10",
        "I will_not sell 5 raw 0 at 10",
        "I will:not sell 5 raw 0 at 10",
        "Quarry posted: I will sell 5 raw 0 at 10",
        "Quarry wrote \"I will sell 5 raw 0 at 10\"",
        "Foundry will sell 5 raw 0 at 10",
        "You will sell 5 raw 0 at 10",
        "Will you sell 5 raw 0 at 10",
        "Sell me 5 raw 0 at 10",
        "I will sell 5 raw 0 at 10,5",
        "I will sell 5,5 raw 0 at 10",
        "I will sell 5 raw 0 for 50 creds.",
        "I commit to buy 500 refined Foundry for 400 CRED.",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, bad_msg.to_owned())],
                &obs,
            )?
            .is_empty(),
            "unlisted message must fail closed alone: `{bad_msg}`"
        );
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[
                    (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                    (1, 0, bad_msg.to_owned()),
                ],
                &obs,
            )?
            .is_empty(),
            "unlisted message must latch prior offer empty: `{bad_msg}`"
        );
    }
    Ok(())
}

#[test]
fn talk_v2_insert_word_sweep_across_commitment_templates_emits_zero_offers() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9117), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];
    let rules = TalkRulesV2Table::frozen();
    let sweep_words = [
        "not", "no", "never", "maybe", "if", "unless", "would", "wouldn't", "refuse", "he",
        "wrote", "you", "me", "foundry", "please", "probably", "for",
    ];

    let mut total_cases = 0_usize;
    for template in rules.commitment_templates {
        let mut tokens: Vec<&str> = Vec::new();
        for &slot in *template {
            match slot {
                "<SIDE>" => tokens.push("sell"),
                "<AMOUNT>" => tokens.push("5"),
                "<GOOD>" => {
                    tokens.push("raw");
                    tokens.push("0");
                }
                "<PRICE_CUE>" => tokens.push("at"),
                "<PRICE>" => tokens.push("10"),
                "<CRED?>" => tokens.push("cred"),
                lit => tokens.push(lit),
            }
        }
        let clean = format!("{}.", tokens.join(" "));
        assert_eq!(
            extract_talk_commitments_from_messages(0, &seat_guilds, &[(1, 0, clean)], &obs)?.len(),
            1,
            "base template must extract 1 offer"
        );

        for word in sweep_words {
            for pos in 0..=tokens.len() {
                let mut mutated = Vec::with_capacity(tokens.len() + 1);
                mutated.extend_from_slice(&tokens[..pos]);
                mutated.push(word);
                mutated.extend_from_slice(&tokens[pos..]);
                let body = format!("{}.", mutated.join(" "));
                let out = extract_talk_commitments_from_messages(
                    0,
                    &seat_guilds,
                    &[(1, 0, body.clone())],
                    &obs,
                )?;
                assert!(
                    out.is_empty(),
                    "insert-word sweep invented commitment for `{body}`: {out:?}"
                );
                total_cases += 1;
            }
        }
    }
    assert_eq!(total_cases, 2125);
    eprintln!("INSERT_WORD_SWEEP_CASES={total_cases}");
    Ok(())
}

#[test]
fn talk_v2_closed_grammar_valid_templates_retractions_and_thousands_commas() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9118), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    for (msg, expected_side, expected_good, expected_amt, expected_price) in [
        (
            "Hi! I will sell 5 raw 0 at 10.",
            OfferSide::Sell,
            AssetId::Raw(0),
            5 * UNIT_SCALE,
            10 * UNIT_SCALE,
        ),
        (
            "Hello! I'll buy 1,000 refined 0 at 250 CRED.",
            OfferSide::Buy,
            AssetId::Refined(0),
            1_000 * UNIT_SCALE,
            250 * UNIT_SCALE,
        ),
        (
            "Greetings; we will sell 12,500 Foundry raw @ 30 creds.",
            OfferSide::Sell,
            AssetId::Raw(0),
            12_500 * UNIT_SCALE,
            30 * UNIT_SCALE,
        ),
        (
            "Thanks. We'll buy 7 ref1 at 15.",
            OfferSide::Buy,
            AssetId::Refined(1),
            7 * UNIT_SCALE,
            15 * UNIT_SCALE,
        ),
        (
            "Thank you! I commit to sell 9 raw:0 at 40.",
            OfferSide::Sell,
            AssetId::Raw(0),
            9 * UNIT_SCALE,
            40 * UNIT_SCALE,
        ),
        (
            "Ok. We commit to buy 3 raw_1 at 22.",
            OfferSide::Buy,
            AssetId::Raw(1),
            3 * UNIT_SCALE,
            22 * UNIT_SCALE,
        ),
        (
            "Okay! I offer to sell 4 Quarry refined at 55 CRED.",
            OfferSide::Sell,
            AssetId::Refined(1),
            4 * UNIT_SCALE,
            55 * UNIT_SCALE,
        ),
        (
            "Noted. We offer to buy 6 raw 0 at 12.",
            OfferSide::Buy,
            AssetId::Raw(0),
            6 * UNIT_SCALE,
            12 * UNIT_SCALE,
        ),
        (
            "Understood. I agree to sell 8 refined Foundry @ 90.",
            OfferSide::Sell,
            AssetId::Refined(0),
            8 * UNIT_SCALE,
            90 * UNIT_SCALE,
        ),
        (
            "We agree to buy 2 raw 0 at 18 CRED.",
            OfferSide::Buy,
            AssetId::Raw(0),
            2 * UNIT_SCALE,
            18 * UNIT_SCALE,
        ),
        (
            "I will post sell 15 raw 1 at 25.",
            OfferSide::Sell,
            AssetId::Raw(1),
            15 * UNIT_SCALE,
            25 * UNIT_SCALE,
        ),
        (
            "I'll post buy 11 refined 1 at 65.",
            OfferSide::Buy,
            AssetId::Refined(1),
            11 * UNIT_SCALE,
            65 * UNIT_SCALE,
        ),
    ] {
        let out = extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[(1, 0, msg.to_owned())],
            &obs,
        )?;
        assert_eq!(
            out,
            vec![league_scorer::ExtractedTalkCommitment {
                proposer_seat: 1,
                counterparty_seat: 0,
                offered_action: MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: expected_side,
                        good: expected_good,
                        amount: expected_amt,
                        price_per_unit_cred: expected_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                },
            }],
            "failed valid template `{msg}`"
        );
    }

    // Retractions and closed-grammar refusals retract a prior active offer.
    for retract_msg in [
        "Cancel.",
        "Actually I retract raw 0.",
        "Actually, I retract my offer on raw 0.",
        "We withdraw.",
        "Never mind.",
        "Walk back raw 0.",
        "I can't trade raw 0.",
        "We don't commit to sell raw 0.",
        "I will not sell 5 raw 0 at 10.",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[
                    (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                    (1, 0, retract_msg.to_owned()),
                ],
                &obs,
            )?
            .is_empty(),
            "retraction must drop prior offer: `{retract_msg}`"
        );
    }

    // Asset-scoped retraction only drops the retracted good while leaving another asset intact.
    let scoped = extract_talk_commitments_from_messages(
        0,
        &seat_guilds,
        &[
            (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
            (1, 0, "I will buy 3 raw 1 at 20.".to_owned()),
            (1, 0, "Actually retract raw 0.".to_owned()),
        ],
        &obs,
    )?;
    assert_eq!(scoped.len(), 1);
    assert_eq!(
        scoped[0].offered_action,
        MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 3 * UNIT_SCALE,
                price_per_unit_cred: 20 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        }
    );

    // Malformed comma placement fails closed and latches the pair empty.
    for bad_comma in [
        "I will sell 5 raw 0 at 10,5.",
        "I will sell 5,5 raw 0 at 10.",
        "I will sell 1,00 raw 0 at 10.",
        "I will sell 1000, raw 0 at 10.",
        "I will sell ,1000 raw 0 at 10.",
        "I will sell 1,,000 raw 0 at 10.",
        "I will sell 0,000 raw 0 at 10.",
        "I will sell 1_000 raw 0 at 10.",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[
                    (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                    (1, 0, bad_comma.to_owned()),
                ],
                &obs,
            )?
            .is_empty(),
            "malformed numeric comma must latch pair empty: `{bad_comma}`"
        );
    }
    Ok(())
}

#[test]
fn talk_v2_extract_state_merge_latch_and_state_isolation() -> TestResult {
    let tmp = TmpDir::new("state_merge_latch")?;
    let host = EvalHost::from_standard_setup(TableId(9119), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    let write_talk = |dir: &Path, body: &str| -> TestResult {
        let dec_dir = dir.join("decisions");
        fs::create_dir_all(&dec_dir)?;
        let rec = EvalDecisionRecord {
            game_id: host.game_id().to_owned(),
            table_id: 9119,
            day: 1,
            round: Round::Talk,
            seat: 1,
            guild: "Quarry".to_owned(),
            arm: EvalArm::Off,
            seat_kind: "Model".to_owned(),
            keeps_notes: false,
            notes_present: false,
            raw_reply: Some(
                json!({
                    "messages": [{ "to_guild": "Foundry", "body": body }],
                    "market_actions": [],
                    "bundle": {}
                })
                .to_string(),
            ),
            discarded_reply_path: None,
            discarded_reply_sha256: None,
            attempts: 1,
            failed_attempts: Vec::new(),
            prompt_tokens: 1,
            completion_tokens: 1,
            wall_secs: None,
            tool_calls_used: 0,
            cleared_non_empty_actions: false,
            prompt_token_ids_sha256s: Vec::new(),
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
            reply_token_ids: Vec::new(),
            reply_top_logprobs: Vec::new(),
            intermediate_bundle_sha256s: Vec::new(),
            refused_market_actions: 0,
            refused_bundle_legs: 0,
            parse_failure: false,
            refusal_code: None,
            refusal_reason: None,
            loaded_history_sha256: BTreeMap::new(),
            invalid_recipients: Vec::new(),
            no_reply: false,
            apply_report: None,
            refused_bundle: None,
            club_bot_baseline: None,
        };
        fs::write(
            dec_dir.join("day-01-talk-Quarry.json"),
            serde_json::to_string_pretty(&rec)?,
        )?;
        Ok(())
    };

    // State 1: negotiation latched ambiguous while talk holds a clean offer -> yields nothing.
    let s1 = tmp.path().join("state1_neg_ambig");
    fs::create_dir_all(s1.join("to-seat"))?;
    fs::write(
        s1.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "Actually\u{2014}cancel that.".to_owned(),
            }],
        })?,
    )?;
    write_talk(&s1, "I will sell 5 raw 0 at 10.")?;
    assert!(extract_talk_commitments_for_state(&s1, 1, 0, &seat_guilds, &obs)?.is_empty());

    // Inverse direction: talk ambiguous while negotiation holds a clean offer -> yields nothing.
    let s_talk_ambig = tmp.path().join("state_talk_ambig");
    fs::create_dir_all(s_talk_ambig.join("to-seat"))?;
    fs::write(
        s_talk_ambig.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "I will sell 5 raw 0 at 10.".to_owned(),
            }],
        })?,
    )?;
    write_talk(&s_talk_ambig, "Maybe.")?;
    assert!(
        extract_talk_commitments_for_state(&s_talk_ambig, 1, 0, &seat_guilds, &obs)?.is_empty()
    );

    // State 2 with clean messages is completely isolated from State 1's ambiguous latch.
    let s2 = tmp.path().join("state2_clean");
    fs::create_dir_all(s2.join("to-seat"))?;
    fs::write(
        s2.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "I will sell 5 raw 0 at 10.".to_owned(),
            }],
        })?,
    )?;
    write_talk(&s2, "I will sell 5 raw 0 at 10.")?;
    let clean_out = extract_talk_commitments_for_state(&s2, 1, 0, &seat_guilds, &obs)?;
    assert_eq!(clean_out.len(), 1);

    // Same game directory across two days: day 1 ambiguous latch leaves day 2's clean offer intact.
    let same_game = tmp.path().join("same_game_two_days");
    fs::create_dir_all(same_game.join("to-seat"))?;
    fs::create_dir_all(same_game.join("decisions"))?;
    fs::write(
        same_game.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "Maybe I will sell 5 raw 0 at 10.".to_owned(),
            }],
        })?,
    )?;
    fs::write(
        same_game.join("to-seat/day-02-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "I will sell 5 raw 0 at 10.".to_owned(),
            }],
        })?,
    )?;
    assert!(extract_talk_commitments_for_state(&same_game, 1, 0, &seat_guilds, &obs)?.is_empty());
    assert_eq!(
        extract_talk_commitments_for_state(&same_game, 2, 0, &seat_guilds, &obs)?,
        clean_out
    );
    Ok(())
}

#[test]
fn we_will_sell_message_lands_in_matched_counts() -> TestResult {
    let tmp = TmpDir::new("we_will_sell_matched")?;
    let host = EvalHost::from_standard_setup(TableId(9119), 4, 1, 77)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Hearth".to_owned(),
        "Wharf".to_owned(),
    ];
    let game_dir = tmp.path().join("game");
    fs::create_dir_all(game_dir.join("to-seat"))?;
    fs::create_dir_all(game_dir.join("decisions"))?;
    fs::write(
        game_dir.join("to-seat/day-01-act-Foundry.negotiation.json"),
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Quarry".to_owned(),
                intent_header: None,
                body: "We will sell 5 raw 0 at 10.".to_owned(),
            }],
        })?,
    )?;
    let ext = extract_talk_commitments_with_counts_for_state(&game_dir, 1, 0, &seat_guilds, &obs)?;
    assert_eq!(ext.commitments.len(), 1);
    assert_eq!(
        ext.counts,
        TalkExtractionCounts {
            negotiation: TalkSourceCounts {
                ambiguous: 0,
                matched: 1,
                seen: 1,
            },
            talk: TalkSourceCounts::default(),
            total: TalkSourceCounts {
                ambiguous: 0,
                matched: 1,
                seen: 1,
            },
        }
    );
    Ok(())
}

#[test]
fn talk_extraction_counts_blind_sha_and_join_arm_summary() -> TestResult {
    let tmp = TmpDir::new("counts_pipeline")?;
    let wrapper_path = copy_pinned_wrapper(tmp.path())?;
    let seats_path = write_two_model_two_bot_seats(tmp.path())?;
    let serving_path = write_serving_manifest(tmp.path())?;
    let git_sha = clean_git_sha_source();
    let transport = SyntheticPipelineTransport;

    let play_1 = tmp.path().join("play_c1");
    let m1 = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_1.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9120,
            seed: 61,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    let branch_1 = tmp.path().join("branch_c1");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_1.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            serving_manifest_path: Some(serving_path.clone()),
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: branch_1.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    // Inject mixed matched + ambiguous messages into State 1 (same pair `Quarry -> Foundry`, so
    // the ambiguous message latches commitments to empty while recording 2 seen / 1 matched / 1 ambiguous
    // in negotiation, and 2 seen / 1 matched / 1 ambiguous in talk).
    let neg_1_path = play_1.join("to-seat/day-01-act-Foundry.negotiation.json");
    let neg_1_orig = fs::read_to_string(&neg_1_path)?;
    fs::write(
        &neg_1_path,
        serde_json::to_string_pretty(&NegotiationInputs {
            batna: eval_wording_slot("eval_batna").to_owned(),
            whispers: vec![
                UntrustedWhisper {
                    from_guild: "Quarry".to_owned(),
                    intent_header: None,
                    body: "Thanks.".to_owned(),
                },
                UntrustedWhisper {
                    from_guild: "Quarry".to_owned(),
                    intent_header: None,
                    body: "No.".to_owned(),
                },
            ],
        })?,
    )?;
    let talk_1_path = play_1.join("decisions/day-01-talk-Quarry.json");
    let mut talk_1_rec: EvalDecisionRecord = serde_json::from_slice(&fs::read(&talk_1_path)?)?;
    talk_1_rec.raw_reply = Some(
        json!({
            "messages": [
                { "to_guild": "Foundry", "body": "Hi." },
                { "to_guild": "Foundry", "body": "I will sell 5 raw 0 at 10,5." }
            ],
            "market_actions": [],
            "bundle": {},
            "notes": "mixed_talk"
        })
        .to_string(),
    );
    fs::write(&talk_1_path, serde_json::to_string_pretty(&talk_1_rec)?)?;

    // Check direct state extraction counts first.
    let (save1, seal1) = (
        play_1.join("checkpoints/day-01-pre-act.save.json"),
        play_1.join("checkpoints/day-01-pre-act.seal"),
    );
    let host1 = EvalHost::fork_from_checkpoint(&save1, &seal1)?;
    let obs1 = host1.observe_seat(Seat(0))?;
    let guilds1 = resolve_eval_seat_guilds(&play_1, obs1.spec.seat_count)?;
    let ext1 = extract_talk_commitments_with_counts_for_state(&play_1, 1, 0, &guilds1, &obs1)?;
    assert!(ext1.commitments.is_empty());
    assert_eq!(
        ext1.counts,
        TalkExtractionCounts {
            negotiation: TalkSourceCounts {
                ambiguous: 1,
                matched: 1,
                seen: 2,
            },
            talk: TalkSourceCounts {
                ambiguous: 1,
                matched: 1,
                seen: 2,
            },
            total: TalkSourceCounts {
                ambiguous: 2,
                matched: 2,
                seen: 4,
            },
        }
    );

    // State 2 with purely clean messages (1 matched in talk, 0 in negotiation).
    let play_2 = tmp.path().join("play_c2");
    let m2 = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_2.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: Some(serving_path.clone()),
            arm: EvalArm::Control,
            table_id: 9121,
            seed: 62,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;
    let branch_2 = tmp.path().join("branch_c2");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_2.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: Some(serving_path),
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: branch_2.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &git_sha,
    )?;

    let cfg = fast_test_scorer_config();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let blind_a = tmp.path().join("blind_a");
    let arm_map_a = tmp.path().join("arm_map_a");
    let ckpts_a = tmp.path().join("ckpts_a");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_1.clone(), branch_1.clone(), play_2, branch_2],
        out_blind_dir: blind_a.clone(),
        out_arm_map_dir: arm_map_a.clone(),
        out_checkpoints_dir: Some(ckpts_a.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 71,
    })?;

    let sid1 = format!("{}-day-01-seat-0", m1.game_id);
    let sid2 = format!("{}-day-01-seat-0", m2.game_id);
    let blind1_bytes = fs::read(blind_a.join(format!("{sid1}.json")))?;
    let blind1_req: BlindScoringRequest = serde_json::from_slice(&blind1_bytes)?;
    assert_eq!(blind1_req.talk_extraction_counts, ext1.counts);
    let blind1_sha_before = sha256_hex(&blind1_bytes);

    // Changing only counts (replacing ambiguous `No.` with neutral `Ok.` while keeping commitments empty)
    // must change blind_request_sha256.
    let mut alt_neg = serde_json::from_str::<NegotiationInputs>(&neg_1_orig)?;
    alt_neg.whispers = vec![
        UntrustedWhisper {
            from_guild: "Quarry".to_owned(),
            intent_header: None,
            body: "Thanks.".to_owned(),
        },
        UntrustedWhisper {
            from_guild: "Quarry".to_owned(),
            intent_header: None,
            body: "Ok.".to_owned(),
        },
    ];
    fs::write(&neg_1_path, serde_json::to_string_pretty(&alt_neg)?)?;
    let blind_alt = tmp.path().join("blind_alt");
    let arm_map_alt = tmp.path().join("arm_map_alt");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_1, branch_1],
        out_blind_dir: blind_alt.clone(),
        out_arm_map_dir: arm_map_alt,
        out_checkpoints_dir: None,
        config_path: cfg_path.clone(),
        scorer_seed: 71,
    })?;
    let blind1_alt_bytes = fs::read(blind_alt.join(format!("{sid1}.json")))?;
    let blind1_alt_req: BlindScoringRequest = serde_json::from_slice(&blind1_alt_bytes)?;
    assert_eq!(blind1_alt_req.talk_commitments, blind1_req.talk_commitments);
    assert_ne!(
        blind1_alt_req.talk_extraction_counts,
        blind1_req.talk_extraction_counts
    );
    assert_ne!(sha256_hex(&blind1_alt_bytes), blind1_sha_before);

    // Score, freeze, and join both states; verify counts appear per arm in summary report.
    let scores_dir = tmp.path().join("scores");
    assert_eq!(
        run_scorer_cli(
            [
                "score",
                "--blind-inputs",
                blind_a.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_a.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                scores_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        ),
        0
    );
    let score2: StateScoreOutput = serde_json::from_slice(&fs::read(
        scores_dir.join("scores").join(format!("{sid2}.json")),
    )?)?;
    let vc_path = verify_ckpts_for_test(
        &ckpts_a,
        &tmp.path().join("vc_counts.json"),
        Some(&blind_a),
        &git_sha,
    )?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_a.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;
    let joined_dir = tmp.path().join("joined");
    let report = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir: arm_map_a,
        scores_dir,
        out_dir: joined_dir.clone(),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: Some(blind_a),
        source_game_dirs: Vec::new(),
    })?;
    assert_eq!(report.states_joined, 2);

    let summary: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(joined_dir.join("per_arm_summary.json"))?)?;
    assert_eq!(summary.arms.len(), 3);
    // State 1 has seen=4, matched=2, ambiguous=2; State 2 has the single synthetic Talk whisper (seen=1, matched=1, ambiguous=0).
    let expected_s2_counts = score2.talk_extraction_counts;
    for arm_row in &summary.arms {
        assert_eq!(arm_row.talk_counts.n_states, 2);
        assert_eq!(
            arm_row.talk_counts.totals.total.seen,
            4 + expected_s2_counts.total.seen
        );
        assert_eq!(
            arm_row.talk_counts.totals.total.matched,
            2 + expected_s2_counts.total.matched
        );
        assert_eq!(
            arm_row.talk_counts.totals.total.ambiguous,
            2 + expected_s2_counts.total.ambiguous
        );
        assert_eq!(arm_row.talk_counts.per_state_ambiguous.min, 0);
        assert_eq!(arm_row.talk_counts.per_state_ambiguous.max, 2);
        assert_eq!(arm_row.talk_counts.per_state_ambiguous.median, Some(1.0));
        assert_eq!(arm_row.talk_counts.per_state_seen.max, 4);
    }
    Ok(())
}

#[test]
fn write_freeze_file_reports_partial_cleanup_removal_failure() -> TestResult {
    let tmp = TmpDir::new("freeze_rm_err")?;
    let freeze_path = tmp.path().join("scorer_freeze.json");
    let err = write_freeze_file_with_writer_and_remover(
        &freeze_path,
        |_file| Err(std::io::Error::other("simulated write failure")),
        |_path| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "simulated remove failure",
            ))
        },
    )
    .err()
    .ok_or("expected EVAL_IO when both write and partial cleanup fail")?;
    assert_eq!(err.code(), "EVAL_IO");
    let msg = err.to_string();
    assert!(
        msg.contains("simulated write failure")
            && msg.contains("also failed to remove partial file")
            && msg.contains("simulated remove failure"),
        "unexpected cleanup failure error: {msg}"
    );
    let _ = fs::remove_file(&freeze_path);
    Ok(())
}

#[test]
fn talk_v2_colon_space_lead_ins_allow_greetings_only_and_refuse_others() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9121), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];

    for allowed in [
        "Hi: I will sell 5 raw 0 at 10.",
        "Hello: I will sell 5 raw 0 at 10.",
        "Greetings: I will sell 5 raw 0 at 10.",
        "Hi: Ok. I will sell 5 raw 0 at 10.",
    ] {
        let out = extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[(1, 0, allowed.to_owned())],
            &obs,
        )?;
        assert_eq!(
            out.len(),
            1,
            "expected allowed colon-space lead `{allowed}`"
        );
    }

    for refused in [
        "Ok: I will sell 5 raw 0 at 10.",
        "Okay: I will sell 5 raw 0 at 10.",
        "Noted: I will sell 5 raw 0 at 10.",
        "Understood: I will sell 5 raw 0 at 10.",
        "Thanks: I will sell 5 raw 0 at 10.",
        "Thank you: I will sell 5 raw 0 at 10.",
        "Cancel that: I will sell 5 raw 0 at 10.",
        "I will sell 5 raw 0 at 10: no deal.",
        ": I will sell 5 raw 0 at 10.",
        "Noted: ",
    ] {
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[(1, 0, refused.to_owned())],
                &obs,
            )?
            .is_empty(),
            "colon-space lead must fail closed alone: `{refused}`"
        );
        assert!(
            extract_talk_commitments_from_messages(
                0,
                &seat_guilds,
                &[
                    (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                    (1, 0, refused.to_owned()),
                ],
                &obs,
            )?
            .is_empty(),
            "colon-space lead must latch prior offer empty: `{refused}`"
        );
    }

    // Two retractions joined by `: ` without any offer continue to retract cleanly.
    assert!(
        extract_talk_commitments_from_messages(
            0,
            &seat_guilds,
            &[
                (1, 0, "I will sell 5 raw 0 at 10.".to_owned()),
                (1, 0, "Cancel that: no deal.".to_owned()),
            ],
            &obs,
        )?
        .is_empty()
    );
    Ok(())
}

#[test]
fn talk_v2_table_driven_standalone_compound_and_guild_prefix_rules() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(9122), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(0))?;
    let seat_guilds = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Glassworks".to_owned(),
    ];
    let frozen = TalkRulesV2Table::frozen();

    // 1. `standalone_tokens` governs `@` splitting (`raw 0@10` and `raw 0 @ 10`).
    for msg in ["I will sell 5 raw 0@10.", "I will sell 5 raw 0 @ 10."] {
        assert_eq!(
            extract_talk_commitments_from_messages_with_rules(
                0,
                &seat_guilds,
                &[(1, 0, msg.to_owned())],
                &obs,
                &frozen,
            )?
            .len(),
            1
        );
        let mut no_standalone = frozen.clone();
        no_standalone.standalone_tokens = &[];
        assert!(
            extract_talk_commitments_from_messages_with_rules(
                0,
                &seat_guilds,
                &[(1, 0, msg.to_owned())],
                &obs,
                &no_standalone,
            )?
            .is_empty(),
            "removing standalone_tokens must disable `{msg}`"
        );
    }

    // 2. `compound_good_separators` governs `:` / `_` in compound goods (`raw:0`, `raw_0`).
    assert_eq!(
        extract_talk_commitments_from_messages_with_rules(
            0,
            &seat_guilds,
            &[(1, 0, "I will sell 5 raw:0 at 10.".to_owned())],
            &obs,
            &frozen,
        )?
        .len(),
        1
    );
    let mut only_underscore = frozen.clone();
    only_underscore.compound_good_separators = &["_"];
    assert!(
        extract_talk_commitments_from_messages_with_rules(
            0,
            &seat_guilds,
            &[(1, 0, "I will sell 5 raw:0 at 10.".to_owned())],
            &obs,
            &only_underscore,
        )?
        .is_empty()
    );
    assert_eq!(
        extract_talk_commitments_from_messages_with_rules(
            0,
            &seat_guilds,
            &[(1, 0, "I will sell 5 raw_0 at 10.".to_owned())],
            &obs,
            &only_underscore,
        )?
        .len(),
        1
    );

    // 3. `allow_guild_then_prefix_goods` governs `<GUILD> raw` and `<GUILD> refined`.
    for msg in [
        "I will sell 5 Foundry raw at 10.",
        "I will sell 5 Foundry refined at 10.",
    ] {
        assert_eq!(
            extract_talk_commitments_from_messages_with_rules(
                0,
                &seat_guilds,
                &[(1, 0, msg.to_owned())],
                &obs,
                &frozen,
            )?
            .len(),
            1
        );
        let mut no_guild_prefix = frozen.clone();
        no_guild_prefix.allow_guild_then_prefix_goods = false;
        assert!(
            extract_talk_commitments_from_messages_with_rules(
                0,
                &seat_guilds,
                &[(1, 0, msg.to_owned())],
                &obs,
                &no_guild_prefix,
            )?
            .is_empty(),
            "disabling allow_guild_then_prefix_goods must reject `{msg}`"
        );
    }

    // 4. `colon_space_lead_templates` governs whether `Hi:` is accepted.
    assert_eq!(
        extract_talk_commitments_from_messages_with_rules(
            0,
            &seat_guilds,
            &[(1, 0, "Hi: I will sell 5 raw 0 at 10.".to_owned())],
            &obs,
            &frozen,
        )?
        .len(),
        1
    );
    let mut no_colon_leads = frozen;
    no_colon_leads.colon_space_lead_templates = &[];
    assert!(
        extract_talk_commitments_from_messages_with_rules(
            0,
            &seat_guilds,
            &[(1, 0, "Hi: I will sell 5 raw 0 at 10.".to_owned())],
            &obs,
            &no_colon_leads,
        )?
        .is_empty()
    );
    Ok(())
}
