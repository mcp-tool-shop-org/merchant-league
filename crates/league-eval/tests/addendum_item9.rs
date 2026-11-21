//! Item 9 Addendum Section 3 tests (`1` through `7`) and Director wording checks (`2026-11-15`).
//!
//! Every test runs offline with mock transports only and zero API keys or GPU calls.

use std::collections::VecDeque;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use league_core::{
    AssetId, EvalActSubmission, EvalHost, MarketAction, OfferProposal, OfferSide, Seat, TableId,
};
use league_eval::{
    BranchOptions, BranchSummary, EVAL_MESSAGES_MAX_ITEMS, EVAL_REPLY_WORDING,
    EVAL_REPLY_WORDING_ARM, EVAL_SEAT_VIEW_SERIALIZER_NAME, EvalArm, EvalDecisionRecord, EvalError,
    EvalManifest, EvalSetupFile, PASS_B_LIMIT_STATEMENT, PlayOptions, ReplyRefusalCode,
    WrongPartCounter, build_act_negotiation_inputs, build_eval_response_format_config,
    eval_internal_slot_names, eval_wording_slot, execute_cli, parse_eval_act_reply,
    parse_eval_talk_reply, persist_eval_game_state, render_seat_view_markdown,
    run_branch_with_transport, run_play_with_transport, validate_eval_seats_file,
};
use league_relay::{
    CallLogEntry, MemoryMode, NegotiationInputs, PREVIEW_WORDING_ARM, RawHttpResponse, Round,
    SeatFile, Transport, TransportError, build_preview_turn_orders_tool_definition,
    preview_wording_slot, sha256_hex, wording_slot,
};
use serde_json::json;

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
    let dir = std::env::temp_dir().join(format!("ml_eval_a9_{label}_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    must_ok(fs::create_dir_all(&dir), "create temp dir");
    dir
}

struct MockTransport {
    replies: Mutex<VecDeque<String>>,
    recorded_bodies: Mutex<Vec<String>>,
}

impl MockTransport {
    fn from_contents(items: Vec<String>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(items)),
            recorded_bodies: Mutex::new(Vec::new()),
        }
    }

    fn recorded_requests(&self) -> Vec<String> {
        let guard = match self.recorded_bodies.lock() {
            Ok(g) => g,
            Err(e) => panic!("recorded_bodies lock poisoned: {e}"),
        };
        guard.clone()
    }
}

impl Transport for MockTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        {
            let mut rec = self
                .recorded_bodies
                .lock()
                .map_err(|_| TransportError::Network("lock poisoned".to_owned()))?;
            rec.push(body.to_owned());
        }
        let content = {
            let mut q = self
                .replies
                .lock()
                .map_err(|_| TransportError::Network("lock poisoned".to_owned()))?;
            q.pop_front()
                .ok_or_else(|| TransportError::Network("mock reply queue empty".to_owned()))?
        };
        let resp = json!({
            "id": "mock-chat-1",
            "model": "qwen/qwen3-32b",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 20,
                "total_tokens": 70,
                "cost": 0.001
            }
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

fn write_wrapper(dir: &Path) -> (PathBuf, Vec<u8>) {
    let p = dir.join("wrapper.md");
    let content = b"# Eval Brief v2\nFollow Evaluation Mode instructions strictly.\n".to_vec();
    must_ok(fs::write(&p, &content), "write wrapper");
    (p, content)
}

fn write_seats_file(dir: &Path, spec: serde_json::Value) -> PathBuf {
    let p = dir.join("seats.json");
    let text = must_ok(serde_json::to_string_pretty(&spec), "serialize seats.json");
    must_ok(fs::write(&p, text), "write seats.json");
    p
}

fn one_openrouter_three_clubbot_seats() -> serde_json::Value {
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
    })
}

fn one_local_three_clubbot_seats() -> serde_json::Value {
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

#[test]
fn test_1_act_parse() {
    let valid_act = json!({
        "messages": [],
        "market_actions": [
            {
                "action_kind": "post_offer",
                "side": "buy",
                "good": { "asset_kind": "raw", "good_index": 0 },
                "amount": 100,
                "price_per_unit_cred": 150,
                "domain_id": null,
                "is_hybrid": false
            },
            {
                "action_kind": "town_import",
                "good": { "asset_kind": "raw", "good_index": 0 },
                "amount": 50
            },
            {
                "action_kind": "create_escrow",
                "destination": 1,
                "asset": { "asset_kind": "cred" },
                "amount": 25,
                "cancel_after_turn": 3
            }
        ],
        "bundle": {
            "converter_runs": [{ "recipe_idx": 0, "runs": 1 }],
            "debt_repayment_cred": 25,
            "charter_bids": [{ "role": "oracle_auditor", "bid_cred": 10 }]
        }
    });

    // Non-note-keeping seat: `notes_present` must be `false`.
    let parsed_no_notes = parse_eval_act_reply(&valid_act.to_string(), 0, false);
    assert_eq!(parsed_no_notes.refusal_code, None);
    assert!(!parsed_no_notes.notes_present);
    assert!(!parsed_no_notes.is_wrong_part);
    assert!(parsed_no_notes.submission.pass_b_actions.is_empty());
    assert_eq!(parsed_no_notes.submission.market_actions.len(), 3);
    assert_eq!(
        parsed_no_notes.submission.market_actions[0],
        MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: 100,
                price_per_unit_cred: 150,
                domain_id: None,
                is_hybrid: false,
            }
        }
    );
    assert_eq!(parsed_no_notes.submission.bundle.converter_runs.len(), 1);
    assert_eq!(parsed_no_notes.submission.bundle.debt_repayment_cred, 25);
    assert_eq!(parsed_no_notes.submission.bundle.charter_bids.len(), 1);

    // Note-keeping seat (`keeps_notes = true`): `notes_present` is `true` iff string `notes` exists.
    let mut with_notes = valid_act.clone();
    if let Some(obj) = with_notes.as_object_mut() {
        obj.insert("notes".to_owned(), json!("Keep salt for turn 3"));
    }
    let parsed_with_notes = parse_eval_act_reply(&with_notes.to_string(), 0, true);
    assert_eq!(parsed_with_notes.refusal_code, None);
    assert!(parsed_with_notes.notes_present);
    assert!(parsed_with_notes.submission.pass_b_actions.is_empty());
    assert_eq!(parsed_with_notes.submission.market_actions.len(), 3);

    // Providing `"notes"` on a non-note-keeping seat is refused and leaves `notes_present == false`.
    let unexpected_notes = parse_eval_act_reply(&with_notes.to_string(), 0, false);
    assert_eq!(
        unexpected_notes.refusal_code,
        Some(ReplyRefusalCode::ActReplyInvalid)
    );
    assert!(!unexpected_notes.notes_present);
    assert_eq!(unexpected_notes.submission, EvalActSubmission::default());

    // One refused case per failure reason, including a `<think>` block and a markdown code fence.
    let refusal_cases: [(&str, bool, &str); 12] = [
        (
            "think_block",
            false,
            "<think>plan</think>{\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
        ),
        (
            "code_fence",
            false,
            "```json\n{\"messages\":[],\"market_actions\":[],\"bundle\":{}}\n```",
        ),
        (
            "trailing_token",
            false,
            "{\"messages\":[],\"market_actions\":[],\"bundle\":{}} trailing",
        ),
        (
            "duplicate_key",
            false,
            "{\"messages\":[],\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
        ),
        ("non_object_root", false, "[]"),
        (
            "unexpected_top_key",
            false,
            "{\"messages\":[],\"market_actions\":[],\"bundle\":{},\"extra\":1}",
        ),
        (
            "missing_notes_when_required",
            true,
            "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}",
        ),
        (
            "non_string_notes",
            true,
            "{\"messages\":[],\"market_actions\":[],\"bundle\":{},\"notes\":123}",
        ),
        (
            "missing_market_actions",
            false,
            "{\"messages\":[],\"bundle\":{}}",
        ),
        (
            "invalid_market_action_item",
            false,
            "{\"messages\":[],\"market_actions\":[{\"action_kind\":\"bogus\"}],\"bundle\":{}}",
        ),
        (
            "missing_bundle",
            false,
            "{\"messages\":[],\"market_actions\":[]}",
        ),
        (
            "invalid_bundle_field",
            false,
            "{\"messages\":[],\"market_actions\":[],\"bundle\":{\"unknown_field\":1}}",
        ),
    ];

    for (label, keeps_notes, raw_reply) in refusal_cases {
        let out = parse_eval_act_reply(raw_reply, 0, keeps_notes);
        assert_eq!(
            out.refusal_code,
            Some(ReplyRefusalCode::ActReplyInvalid),
            "case `{label}` must return ACT_REPLY_INVALID"
        );
        assert!(!out.is_wrong_part);
        assert!(
            out.refusal_reason.as_ref().is_some_and(|r| !r.is_empty()),
            "case `{label}` must record refusal_reason"
        );
        assert_eq!(
            out.submission,
            EvalActSubmission::default(),
            "case `{label}` must return empty EvalActSubmission"
        );
        assert!(out.submission.pass_b_actions.is_empty());
        assert!(!out.notes_present || (keeps_notes && label == "notes"));
    }

    // B1: in `act`, any non-empty `messages` (even malformed `[42]` or missing `body`) wins with ACT_REPLY_HAS_MESSAGES;
    // in `talk`, malformed envelopes/messages with empty orders get `TALK_REPLY_INVALID`, while malformed orders win with `TALK_REPLY_HAS_ORDERS`.
    let seated = vec![
        "Foundry".to_owned(),
        "Quarry".to_owned(),
        "Harbor".to_owned(),
        "Mint".to_owned(),
    ];
    for (label, raw_reply) in [
        (
            "act_messages_exceeds_max_items",
            "{\"messages\":[{\"to_guild\":\"Quarry\",\"body\":\"1\"},{\"to_guild\":\"Quarry\",\"body\":\"2\"},{\"to_guild\":\"Quarry\",\"body\":\"3\"},{\"to_guild\":\"Quarry\",\"body\":\"4\"}],\"market_actions\":[],\"bundle\":{}}",
        ),
        (
            "act_messages_missing_body",
            "{\"messages\":[{\"to_guild\":\"Quarry\"}],\"market_actions\":[],\"bundle\":{}}",
        ),
        (
            "act_messages_non_object_item",
            "{\"messages\":[42],\"market_actions\":[],\"bundle\":{}}",
        ),
    ] {
        let out = parse_eval_act_reply(raw_reply, 0, false);
        assert_eq!(
            out.refusal_code,
            Some(ReplyRefusalCode::ActReplyHasMessages),
            "case `{label}` must return ACT_REPLY_HAS_MESSAGES"
        );
        assert!(out.is_wrong_part);
        let talk_out = parse_eval_talk_reply(raw_reply, 0, "Foundry", &seated, false);
        assert_eq!(
            talk_out.refusal_code,
            Some(ReplyRefusalCode::TalkReplyInvalid),
            "case `{label}` in talk must return TALK_REPLY_INVALID"
        );
        assert!(!talk_out.is_wrong_part);
    }

    for (label, raw_talk) in [
        (
            "talk_malformed_market_action_bogus",
            "{\"messages\":[],\"market_actions\":[{\"action_kind\":\"bogus\"}],\"bundle\":{}}",
        ),
        (
            "talk_incomplete_post_offer",
            "{\"messages\":[],\"market_actions\":[{\"action_kind\":\"post_offer\"}],\"bundle\":{}}",
        ),
        (
            "talk_unknown_bundle_field",
            "{\"messages\":[],\"market_actions\":[],\"bundle\":{\"unknown_field\":1}}",
        ),
    ] {
        let talk_out = parse_eval_talk_reply(raw_talk, 0, "Foundry", &seated, false);
        assert_eq!(
            talk_out.refusal_code,
            Some(ReplyRefusalCode::TalkReplyHasOrders),
            "case `{label}` must return TALK_REPLY_HAS_ORDERS"
        );
        assert!(talk_out.is_wrong_part);
    }

    // Double-check `parse_eval_talk_reply` `notes_present` behavior too.
    let valid_talk_with_notes = json!({
        "messages": [{ "to_guild": "Quarry", "body": "hello" }],
        "market_actions": [],
        "bundle": {},
        "notes": "talk note"
    });
    let t_notes = parse_eval_talk_reply(
        &valid_talk_with_notes.to_string(),
        0,
        "Foundry",
        &[
            "Foundry".to_owned(),
            "Quarry".to_owned(),
            "Harbor".to_owned(),
            "Mint".to_owned(),
        ],
        true,
    );
    assert_eq!(t_notes.refusal_code, None);
    assert!(t_notes.notes_present);
}

#[test]
fn test_2_wrong_part() {
    let tmp = fresh_tmp_dir("wrong_part");
    let game_dir = tmp.join("game");
    let (wrapper_path, _) = write_wrapper(&tmp);
    let seats_path = write_seats_file(&tmp, one_openrouter_three_clubbot_seats());

    // Talk reply contains an order (`TALK_REPLY_HAS_ORDERS`); act reply contains a message (`ACT_REPLY_HAS_MESSAGES`).
    let talk_with_order = json!({
        "messages": [{ "to_guild": "Quarry", "body": "should NOT be delivered!" }],
        "market_actions": [{
            "action_kind": "town_import",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 50
        }],
        "bundle": {}
    });
    let act_with_msg = json!({
        "messages": [{ "to_guild": "Quarry", "body": "too late" }],
        "market_actions": [{
            "action_kind": "town_import",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 50
        }],
        "bundle": { "debt_repayment_cred": 10 }
    });

    let transport =
        MockTransport::from_contents(vec![talk_with_order.to_string(), act_with_msg.to_string()]);
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 2,
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
        "run_play_with_transport wrong_part",
    );

    // 1. Talk messages in the refused reply were NOT delivered to Quarry.
    let neg_raw = must_ok(
        fs::read_to_string(game_dir.join("to-seat/day-01-act-Quarry.negotiation.json")),
        "read Quarry negotiation.json",
    );
    let neg: NegotiationInputs =
        must_ok(serde_json::from_str(&neg_raw), "parse Quarry negotiation");
    assert!(
        neg.whispers.is_empty(),
        "whispers from TALK_REPLY_HAS_ORDERS reply must NOT be delivered"
    );

    // 2. Talk and act decision records recorded exact refusal codes; wrong-part act left empty orders.
    let talk_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-01-talk-Foundry.json")),
            "read talk decision",
        )),
        "parse talk decision",
    );
    assert_eq!(
        talk_dec.refusal_code.as_deref(),
        Some("TALK_REPLY_HAS_ORDERS")
    );

    let act_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-01-act-Foundry.json")),
            "read act decision",
        )),
        "parse act decision",
    );
    assert_eq!(
        act_dec.refusal_code.as_deref(),
        Some("ACT_REPLY_HAS_MESSAGES")
    );
    assert_eq!(act_dec.refused_bundle, None);

    // 3. Item 1.8 WrongPartCounter incremented in both cases & triggers 5% stop line (both denominators).
    assert_eq!(manifest.wrong_part_refusals.talk_reply_has_orders, 1);
    assert_eq!(manifest.wrong_part_refusals.act_reply_has_messages, 1);
    assert_eq!(manifest.wrong_part_refusals.total_wrong_part_refusals, 2);
    assert_eq!(manifest.wrong_part_refusals.delivered_model_replies, 2);
    assert_eq!(manifest.wrong_part_refusals.total_model_rounds, 2);
    assert!(
        manifest
            .wrong_part_refusals
            .exceeds_five_percent_preflight_stop_line_delivered
    );
    assert!(
        manifest
            .wrong_part_refusals
            .exceeds_five_percent_preflight_stop_line_all_rounds
    );

    // Also verify boundary behavior around 5% (`20` replies w/ 1 refusal == 5.0% <= threshold; `19` replies == 5.26% > threshold)
    // AND divergence between delivered_model_replies and total_model_rounds when `record_no_reply()` is included (`B1`).
    let mut counter = WrongPartCounter::default();
    for _ in 0..18 {
        counter.record_delivered_reply(None);
    }
    counter.record_delivered_reply(Some(ReplyRefusalCode::TalkReplyHasOrders));
    counter.record_no_reply();
    assert_eq!(counter.delivered_model_replies, 19);
    assert_eq!(counter.total_model_rounds, 20);
    assert!(counter.exceeds_five_percent_preflight_stop_line_delivered);
    assert!(!counter.exceeds_five_percent_preflight_stop_line_all_rounds);

    let mut over_counter = WrongPartCounter::default();
    for _ in 0..18 {
        over_counter.record_delivered_reply(None);
    }
    over_counter.record_delivered_reply(Some(ReplyRefusalCode::ActReplyHasMessages));
    assert!(over_counter.exceeds_five_percent_preflight_stop_line_delivered);
    assert!(over_counter.exceeds_five_percent_preflight_stop_line_all_rounds);

    // Check bundle-only refusal on talk also yields TALK_REPLY_HAS_ORDERS.
    let bundle_only_talk = json!({
        "messages": [],
        "market_actions": [],
        "bundle": { "debt_repayment_cred": 5 }
    });
    let parsed_bundle_talk = parse_eval_talk_reply(
        &bundle_only_talk.to_string(),
        0,
        "Foundry",
        &["Foundry".to_owned(), "Quarry".to_owned()],
        false,
    );
    assert_eq!(
        parsed_bundle_talk.refusal_code,
        Some(ReplyRefusalCode::TalkReplyHasOrders)
    );
    assert!(parsed_bundle_talk.is_wrong_part);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_3_messages() {
    let tmp = fresh_tmp_dir("messages");
    let game_dir = tmp.join("game");
    let (wrapper_path, _) = write_wrapper(&tmp);
    let seats_path = write_seats_file(&tmp, one_openrouter_three_clubbot_seats());

    let talk_reply = json!({
        "messages": [
            { "to_guild": "UnknownGuild", "body": "who are you?" },
            { "to_guild": "Foundry", "body": "talking to myself" },
            { "to_guild": "Harbor", "body": "Trade Raw 0 at 150 CRED." }
        ],
        "market_actions": [],
        "bundle": {}
    });
    let valid_empty_act = json!({
        "messages": [],
        "market_actions": [],
        "bundle": {}
    });

    let transport =
        MockTransport::from_contents(vec![talk_reply.to_string(), valid_empty_act.to_string()]);
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 3,
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
        "run_play_with_transport messages",
    );

    // 1. Valid sibling message arrived as whisper in Harbor's `negotiation.json`.
    let harbor_neg: NegotiationInputs = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("to-seat/day-01-act-Harbor.negotiation.json")),
            "read Harbor negotiation",
        )),
        "parse Harbor negotiation",
    );
    assert_eq!(harbor_neg.whispers.len(), 1);
    assert_eq!(harbor_neg.whispers[0].from_guild, "Foundry");
    assert_eq!(harbor_neg.whispers[0].body, "Trade Raw 0 at 150 CRED.");

    // 2. Both invalid recipients got `MESSAGE_RECIPIENT_INVALID`, while Harbor still got its message.
    let talk_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-01-talk-Foundry.json")),
            "read Foundry talk decision",
        )),
        "parse talk decision",
    );
    assert_eq!(talk_dec.refusal_code, None);
    assert_eq!(talk_dec.invalid_recipients.len(), 2);
    for entry in &talk_dec.invalid_recipients {
        assert!(
            entry.contains("MESSAGE_RECIPIENT_INVALID"),
            "invalid_recipient entry must include MESSAGE_RECIPIENT_INVALID (`B5`), got: {entry}"
        );
    }
    assert_eq!(
        manifest.refusal_counts_by_seat["Foundry"].message_recipient_invalid,
        2
    );

    // 3. EVERY seat's act-round negotiation file carries the exact pinned `eval_batna` sentence.
    let expected_batna = eval_wording_slot("eval_batna");
    for guild in ["Foundry", "Quarry", "Harbor", "Mint"] {
        let neg: NegotiationInputs = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(
                    game_dir.join(format!("to-seat/day-01-act-{guild}.negotiation.json")),
                ),
                "read seat negotiation",
            )),
            "parse seat negotiation",
        );
        assert_eq!(
            neg.batna, expected_batna,
            "seat `{guild}` act negotiation file must carry exact `eval_batna`"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_4_schema() {
    const EXPECTED_EVAL_SCHEMA_SHA256: &str =
        "052a89fd5bd4624c9a7c7a599772bf22b8cb4fad27eda0e006ecf2579ee8b544";

    let cfg = must_ok(build_eval_response_format_config(), "build schema");
    let expected_sha = cfg.canonical_sha256();
    assert_eq!(
        expected_sha, EXPECTED_EVAL_SCHEMA_SHA256,
        "STOP_RULE: unified Eval schema canonical SHA-256 drifted from pinned literal"
    );
    let tool_def = build_preview_turn_orders_tool_definition();
    let tool_props = &tool_def["function"]["parameters"]["properties"];
    let k_ma = preview_wording_slot("key_market_actions");
    let k_bundle = preview_wording_slot("key_bundle");
    let k_messages = eval_wording_slot("eval_messages_key");
    let k_notes = wording_slot("scratchpad_schema_key");

    // `r2` subtrees byte-for-byte identical.
    assert_eq!(cfg.schema["properties"][&k_ma], tool_props[&k_ma]);
    assert_eq!(cfg.schema["properties"][&k_bundle], tool_props[&k_bundle]);
    assert_eq!(
        cfg.schema["properties"][k_messages]["maxItems"],
        json!(EVAL_MESSAGES_MAX_ITEMS)
    );
    assert!(cfg.schema["properties"].get(&k_notes).is_some());

    // Verify relay's setup validation accepts the schema across both `local` (`notes` kept)
    // and `openrouter` (`notes` stripped automatically), under `off` and `on`, and locks sha256 in setup.json.
    for (label, arm, seats_json, act_reply) in [
        (
            "off_openrouter",
            EvalArm::Off,
            one_openrouter_three_clubbot_seats(),
            json!({ "messages": [], "market_actions": [], "bundle": {} }),
        ),
        (
            "on_local_notes",
            EvalArm::On,
            one_local_three_clubbot_seats(),
            json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "local note" }),
        ),
    ] {
        let tmp = fresh_tmp_dir(&format!("schema_{label}"));
        let game_dir = tmp.join("game");
        let (wrapper_path, _) = write_wrapper(&tmp);
        let seats_path = write_seats_file(&tmp, seats_json);
        let talk_reply = if label == "on_local_notes" {
            json!({ "messages": [], "market_actions": [], "bundle": {}, "notes": "talk note" })
        } else {
            json!({ "messages": [], "market_actions": [], "bundle": {} })
        };
        let transport =
            MockTransport::from_contents(vec![talk_reply.to_string(), act_reply.to_string()]);
        let _manifest = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: game_dir.clone(),
                    seats_path,
                    wrapper_path,
                    wrapper_version: None,
                    arm,
                    table_id: 4,
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
            &format!("run_play_with_transport schema {label}"),
        );

        let setup_raw = must_ok(
            fs::read_to_string(game_dir.join("setup.json")),
            "read setup.json",
        );
        let setup: EvalSetupFile = must_ok(serde_json::from_str(&setup_raw), "parse setup.json");
        assert_eq!(setup.eval_reply_wording_arm, EVAL_REPLY_WORDING_ARM);
        assert_eq!(
            setup.response_schema_sha256.as_deref(),
            Some(expected_sha.as_str())
        );

        let relay_setup_raw = must_ok(
            fs::read_to_string(game_dir.join("relay/setup.json")),
            "read relay/setup.json",
        );
        let relay_setup: league_relay::GameSetupRecord = must_ok(
            serde_json::from_str(&relay_setup_raw),
            "parse relay/setup.json",
        );
        let expected_relay_arm = match arm {
            EvalArm::Off => league_relay::ACTIVE_WORDING_ARM,
            EvalArm::On => PREVIEW_WORDING_ARM,
            EvalArm::Control => unreachable!(),
        };
        assert_eq!(relay_setup.wording_arm, expected_relay_arm);
        assert_eq!(
            relay_setup.response_schema_sha256.as_deref(),
            Some(expected_sha.as_str()),
            "relay/setup.json must lock response_schema_sha256 (`C13`)"
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}

#[test]
fn test_5_seat_view() {
    // 1. Director's 2026-11-15 rule: single-line, single spaces, exact count, no slot names leaked.
    assert_eq!(EVAL_REPLY_WORDING.len(), 11);
    for entry in EVAL_REPLY_WORDING {
        assert!(
            !entry.shown_text.contains('\n') && !entry.shown_text.contains('\r'),
            "slot `{}` must be single-line",
            entry.slot
        );
        assert!(
            !entry.shown_text.contains("  "),
            "slot `{}` must use single spaces",
            entry.slot
        );
        assert_eq!(
            entry.shown_text.trim(),
            entry.shown_text,
            "slot `{}` must have no leading/trailing whitespace",
            entry.slot
        );
    }

    let host = must_ok(
        EvalHost::from_standard_setup(TableId(5), 4, 7, 11),
        "from_standard_setup",
    );
    let obs = must_ok(host.observe_seat(Seat(0)), "observe_seat");
    let pretty_obs = must_ok(
        serde_json::to_string_pretty(&obs),
        "serde_json::to_string_pretty",
    );
    assert_eq!(
        EVAL_SEAT_VIEW_SERIALIZER_NAME,
        "serde_json::to_string_pretty"
    );

    let talk_md = must_ok(render_seat_view_markdown(Round::Talk, &obs), "render talk");
    let expected_talk = format!(
        "{}\n\n{}\n\n{}\n",
        eval_wording_slot("eval_talk_instruction"),
        eval_wording_slot("eval_state_intro"),
        pretty_obs
    );
    assert_eq!(talk_md, expected_talk);

    let act_md = must_ok(render_seat_view_markdown(Round::Act, &obs), "render act");
    let expected_act = format!(
        "{}\n\n{}\n\n{}\n",
        eval_wording_slot("eval_act_instruction"),
        eval_wording_slot("eval_state_intro"),
        pretty_obs
    );
    assert_eq!(act_md, expected_act);

    // Instruction must come strictly first.
    assert!(talk_md.starts_with(eval_wording_slot("eval_talk_instruction")));
    assert!(act_md.starts_with(eval_wording_slot("eval_act_instruction")));

    // Check none of the internal slot names leak into any rendered prompt, negotiation file,
    // schema JSON, or actual wire HTTP request body.
    let cfg = must_ok(build_eval_response_format_config(), "build schema");
    let schema_str = cfg.schema.to_string();

    let tmp = fresh_tmp_dir("seat_view");
    let game_dir = tmp.join("game");
    must_ok(
        persist_eval_game_state(&host, &game_dir),
        "seed turn-0 game_dir",
    );
    let (wrapper_path, _) = write_wrapper(&tmp);
    let seats_path = write_seats_file(&tmp, one_openrouter_three_clubbot_seats());
    let empty_reply = json!({ "messages": [], "market_actions": [], "bundle": {} }).to_string();
    let transport = MockTransport::from_contents(vec![empty_reply.clone(), empty_reply]);
    let _ = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 5,
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
        "run_play seat_view",
    );

    let written_talk = must_ok(
        fs::read_to_string(game_dir.join("to-seat/day-01-talk-Foundry.md")),
        "read written talk",
    );
    assert_eq!(written_talk, talk_md);
    let written_act = must_ok(
        fs::read_to_string(game_dir.join("to-seat/day-01-act-Foundry.md")),
        "read written act",
    );
    assert_eq!(written_act, act_md);
    let neg_str = must_ok(
        fs::read_to_string(game_dir.join("to-seat/day-01-act-Foundry.negotiation.json")),
        "read neg",
    );
    let reqs = transport.recorded_requests();
    assert_eq!(reqs.len(), 2);

    for slot_name in eval_internal_slot_names() {
        assert!(!talk_md.contains(slot_name), "talk_md leaked `{slot_name}`");
        assert!(!act_md.contains(slot_name), "act_md leaked `{slot_name}`");
        assert!(!neg_str.contains(slot_name), "neg_str leaked `{slot_name}`");
        assert!(
            !schema_str.contains(slot_name),
            "schema leaked `{slot_name}`"
        );
        for req in &reqs {
            assert!(
                !req.contains(slot_name),
                "wire HTTP body leaked `{slot_name}`"
            );
        }
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_6_seats() {
    let tmp = fresh_tmp_dir("seats");
    let (wrapper_path, _) = write_wrapper(&tmp);

    // 1. All-`"file"`, `"model": "ClubBot"` table runs ClubBot offline cleanly via NoopTransport.
    let clubbot_game = tmp.join("clubbot_game");
    let clubbot_seats = write_seats_file(
        &tmp,
        json!({
            "seats": [
                { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: clubbot_game.clone(),
                seats_path: clubbot_seats,
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 6,
                seed: 9,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &league_eval::NoopTransport,
            None,
        ),
        "all-ClubBot play",
    );
    assert_eq!(manifest.played_turns, 2);
    for s in &manifest.seats {
        assert_eq!(s.backend, "file");
        assert_eq!(s.model, "ClubBot");
        assert!(!s.keeps_notes);
    }
    let act_dec: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(clubbot_game.join("decisions/day-01-act-Foundry.json")),
            "read ClubBot decision",
        )),
        "parse ClubBot decision",
    );
    assert_eq!(act_dec.seat_kind, "ClubBot");
    assert!(act_dec.club_bot_baseline.is_some());

    // 2. Any other `file` seat (`model: None` or `model: Some("Other")`) is refused with
    //    `UNSUPPORTED_FILE_SEAT_MODEL` (exit 1).
    for (tag, model_field) in [("omitted_model", None), ("wrong_model", Some("HumanAgent"))] {
        let mut seat0 = json!({ "guild": "Foundry", "backend": "file" });
        if let Some(m) = model_field
            && let Some(obj) = seat0.as_object_mut()
        {
            obj.insert("model".to_owned(), json!(m));
        }
        let bad_seats = tmp.join(format!("bad_seats_{tag}.json"));
        must_ok(
            fs::write(
                &bad_seats,
                must_ok(
                    serde_json::to_string_pretty(&json!({
                        "seats": [
                            seat0,
                            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
                        ]
                    })),
                    "serialize bad seats",
                ),
            ),
            "write bad seats",
        );
        let err = match run_play_with_transport(
            &PlayOptions {
                game_dir: tmp.join(format!("bad_game_{tag}")),
                seats_path: bad_seats.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 60,
                seed: 9,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &league_eval::NoopTransport,
            None,
        ) {
            Err(e) => e,
            Ok(_) => panic!("non-ClubBot file seat `{tag}` must be refused"),
        };
        assert_eq!(err.code(), "UNSUPPORTED_FILE_SEAT_MODEL");
        assert!(matches!(err, EvalError::UnsupportedFileSeatModel { .. }));
        assert_eq!(err.exit_code(), 1);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = execute_cli(
            [
                "play".into(),
                "--game-dir".into(),
                tmp.join(format!("bad_cli_{tag}")).into_os_string(),
                "--seats".into(),
                bad_seats.into_os_string(),
                "--wrapper".into(),
                wrapper_path.clone().into_os_string(),
                "--arm".into(),
                "off".into(),
                "--seed".into(),
                "9".into(),
            ],
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(exit, 1);
        let err_text = String::from_utf8_lossy(&stderr);
        assert!(
            err_text.contains("UNSUPPORTED_FILE_SEAT_MODEL"),
            "CLI stderr missing code: {err_text}"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_7_records() {
    let tmp = fresh_tmp_dir("records");
    let game_dir = tmp.join("game");
    let (wrapper_path, wrapper_bytes) = write_wrapper(&tmp);
    let expected_wrapper_sha = sha256_hex(&wrapper_bytes);

    // Seat 0 (`Foundry`, openrouter): triggers TALK_REPLY_HAS_ORDERS on turn 1 talk and ACT_REPLY_INVALID on turn 1 act,
    //   then valid recipient + invalid recipient (`MESSAGE_RECIPIENT_INVALID`) on turn 2 talk and ACT_REPLY_HAS_MESSAGES on turn 2 act.
    // Seat 1 (`Quarry`, openrouter): sends 1 invalid recipient on turn 1 talk, valid empty act on turn 1, valid empty replies on turn 2.
    let seats_path = write_seats_file(
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
                {
                    "guild": "Quarry",
                    "backend": "openrouter",
                    "model": "qwen/qwen3-32b",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_cost_per_call": 0.05
                },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        }),
    );

    let talk_order_wrong = json!({
        "messages": [],
        "market_actions": [{
            "action_kind": "town_import",
            "good": { "asset_kind": "raw", "good_index": 0 },
            "amount": 10
        }],
        "bundle": {}
    })
    .to_string();
    let talk_bad_rcpt = json!({
        "messages": [{ "to_guild": "Phantom", "body": "bad" }],
        "market_actions": [],
        "bundle": {}
    })
    .to_string();
    let act_invalid_fence =
        "```json\n{\"messages\":[],\"market_actions\":[],\"bundle\":{}}\n```".to_owned();
    let act_empty_ok = json!({ "messages": [], "market_actions": [], "bundle": {} }).to_string();
    let talk_clean = json!({ "messages": [], "market_actions": [], "bundle": {} }).to_string();
    let act_msg_wrong = json!({
        "messages": [{ "to_guild": "Quarry", "body": "oops" }],
        "market_actions": [],
        "bundle": {}
    })
    .to_string();

    // Seat calls happen per round across seats Foundry and Quarry. Notice seats may run concurrently
    // on threads, so let's make our two-seat deterministic test use 1 automated seat in order if concurrency
    // would interleave queue order — OR route dynamically by checking which guild prompt `body` names!
    // Routing by user message guild keeps seat 0 vs seat 1 100% deterministic!
    struct PerGuildTransport {
        turn_idx: AtomicUsize,
    }
    impl Transport for PerGuildTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            let v: serde_json::Value =
                serde_json::from_str(body).map_err(|e| TransportError::Network(e.to_string()))?;
            let msg_arr = v["messages"]
                .as_array()
                .ok_or_else(|| TransportError::Network("missing messages array".to_owned()))?;
            let user_msg = msg_arr
                .last()
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default();
            let header = user_msg.split("\"phase\"").next().unwrap_or(user_msg);
            let is_talk = header.contains("This is the talk round.");
            let is_turn_1 = header.contains("\"turn\": 1,");
            let is_foundry = header.contains("\"seat\": 0,");

            let _ = self.turn_idx.fetch_add(1, Ordering::SeqCst);
            let content = match (is_foundry, is_turn_1, is_talk) {
                (true, true, true) => json!({
                    "messages": [],
                    "market_actions": [{
                        "action_kind": "town_import",
                        "good": { "asset_kind": "raw", "good_index": 0 },
                        "amount": 10
                    }],
                    "bundle": {}
                })
                .to_string(),
                (true, true, false) => {
                    // Duplicate key passes generic JSON Schema in relay, then fails strict unique-key check (`ACT_REPLY_INVALID`).
                    "{\"messages\":[],\"messages\":[],\"market_actions\":[],\"bundle\":{}}"
                        .to_owned()
                }
                (true, false, true) => json!({
                    "messages": [
                        { "to_guild": "Phantom", "body": "ghost" },
                        { "to_guild": "Quarry", "body": "real" }
                    ],
                    "market_actions": [],
                    "bundle": {}
                })
                .to_string(),
                (true, false, false) => json!({
                    "messages": [{ "to_guild": "Quarry", "body": "wrong part" }],
                    "market_actions": [],
                    "bundle": {}
                })
                .to_string(),
                (false, true, true) => json!({
                    "messages": [{ "to_guild": "Quarry", "body": "self msg" }],
                    "market_actions": [],
                    "bundle": {}
                })
                .to_string(),
                _ => json!({
                    "messages": [],
                    "market_actions": [],
                    "bundle": {}
                })
                .to_string(),
            };

            let resp = json!({
                "id": "mock-per-guild",
                "model": "qwen/qwen3-32b",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": content },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 50, "completion_tokens": 20, "total_tokens": 70, "cost": 0.001 }
            });
            Ok(RawHttpResponse {
                status: 200,
                body: resp.to_string(),
            })
        }
    }

    let _unused = (
        talk_order_wrong,
        talk_bad_rcpt,
        act_invalid_fence,
        act_empty_ok,
        talk_clean,
        act_msg_wrong,
    );

    let transport = PerGuildTransport {
        turn_idx: AtomicUsize::new(0),
    };
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path,
                wrapper_path,
                wrapper_version: Some("wrapper-v2".to_owned()),
                arm: EvalArm::On,
                table_id: 7,
                seed: 7,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &transport,
            None,
        ),
        "run_play records",
    );

    let saved_manifest: EvalManifest = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("manifest.json")),
            "read manifest.json",
        )),
        "parse manifest.json",
    );
    assert_eq!(saved_manifest, manifest);
    assert_eq!(manifest.pass_b_limit, PASS_B_LIMIT_STATEMENT);
    assert_eq!(manifest.wrapper_sha256, expected_wrapper_sha);
    assert_eq!(manifest.wrapper_version, "wrapper-v2");

    let foundry_counts = must_some(
        manifest.refusal_counts_by_seat.get("Foundry"),
        "Foundry refusal counts",
    );
    assert_eq!(foundry_counts.talk_reply_has_orders, 1);
    assert_eq!(foundry_counts.act_reply_invalid, 1);
    assert_eq!(foundry_counts.message_recipient_invalid, 1);
    assert_eq!(foundry_counts.act_reply_has_messages, 1);

    let quarry_counts = must_some(
        manifest.refusal_counts_by_seat.get("Quarry"),
        "Quarry refusal counts",
    );
    assert_eq!(quarry_counts.message_recipient_invalid, 1);

    let on_arm = must_some(
        manifest.refusal_counts_by_arm.get("on"),
        "arm `on` refusal counts",
    );
    assert_eq!(on_arm.talk_reply_has_orders, 1);
    assert_eq!(on_arm.act_reply_invalid, 1);
    assert_eq!(on_arm.act_reply_has_messages, 1);
    assert_eq!(on_arm.message_recipient_invalid, 2);

    let _ = fs::remove_dir_all(&tmp);
}

fn find_and_mutate_first_json_string_token(
    value: &mut serde_json::Value,
    needle: &str,
    replacement: &str,
) -> bool {
    match value {
        serde_json::Value::String(s) => {
            if s == needle {
                *s = replacement.to_owned();
                return true;
            }
            false
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if find_and_mutate_first_json_string_token(item, needle, replacement) {
                    return true;
                }
            }
            false
        }
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if k == needle
                    && let Some(v) = map.remove(&k)
                {
                    map.insert(replacement.to_owned(), v);
                    return true;
                }
                if let Some(v) = map.get_mut(&k)
                    && find_and_mutate_first_json_string_token(v, needle, replacement)
                {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn mutate_last_char(text: &str) -> String {
    let mut ch = if text.ends_with('X') { 'Y' } else { 'X' };
    let last_len = text.chars().last().map_or(0, char::len_utf8);
    let candidate = format!("{}{ch}", &text[..text.len() - last_len]);
    if candidate != text {
        candidate
    } else {
        ch = 'Z';
        format!("{text}{ch}")
    }
}

#[test]
fn b2_b3_golden_seat_view_pinned_words_and_one_char_mutation_guard() {
    // 1. Literal pin of all 11 approved strings (`v2_eval_reply_2026_11_21_r1`).
    const PINNED_APPROVED_WORDING: [(&str, &str); 11] = [
        ("eval_messages_key", "messages"),
        (
            "eval_messages_description",
            "Messages you send to other seats in the talk round, at most 3. In the act round, leave this list empty.",
        ),
        ("eval_messages_max_items", "3"),
        ("eval_to_guild_key", "to_guild"),
        (
            "eval_to_guild_description",
            "The guild of the seat that receives this message. It must be another seat at this table.",
        ),
        ("eval_body_key", "body"),
        (
            "eval_body_description",
            "The text of your message, at most 120 words and 1,800 characters. Longer text is cut short, and the receiver is told it was cut.",
        ),
        (
            "eval_talk_instruction",
            "This is the talk round. You may send up to 3 messages to other seats in `messages`. Each message reaches only the seat you address, at the start of the act round. Leave `market_actions` and `bundle` empty: a reply that contains orders in this round is refused, and its messages are not sent.",
        ),
        (
            "eval_act_instruction",
            "This is the act round. Put your orders for this turn in `market_actions` and `bundle`. Leave `messages` empty: a reply that contains messages in this round is refused, and its orders are not applied.",
        ),
        (
            "eval_state_intro",
            "The game state you can see is below, as JSON.",
        ),
        (
            "eval_batna",
            "Your own orders and the commitments you already hold are still applied under the rules, as is everything the rules apply every turn.",
        ),
    ];

    assert_eq!(EVAL_REPLY_WORDING.len(), PINNED_APPROVED_WORDING.len());
    for (entry, (expected_slot, expected_text)) in EVAL_REPLY_WORDING
        .iter()
        .zip(PINNED_APPROVED_WORDING.iter())
    {
        assert_eq!(entry.slot, *expected_slot);
        assert_eq!(
            entry.shown_text, *expected_text,
            "pinned text drifted for slot `{expected_slot}`"
        );
    }
    assert_eq!(
        EVAL_MESSAGES_MAX_ITEMS.to_string(),
        eval_wording_slot("eval_messages_max_items"),
        "EVAL_MESSAGES_MAX_ITEMS constant must match eval_messages_max_items slot"
    );

    // 2. B3 fixed-entropy golden seat view comparison.
    let fixed_host = must_ok(
        EvalHost::from_explicit_entropy(
            TableId(96_001),
            4,
            7,
            [0x41; 32],
            &[[0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32]],
            [0x55; 32],
            0x66,
        ),
        "from_explicit_entropy",
    );
    let obs = must_ok(fixed_host.observe_seat(Seat(0)), "observe fixed seat 0");
    let talk_md = must_ok(
        render_seat_view_markdown(Round::Talk, &obs),
        "render fixed talk md",
    );
    let golden_talk =
        include_str!("fixtures/day-01-talk-Foundry.expected.md").replace("\r\n", "\n");
    assert_eq!(
        talk_md, golden_talk,
        "fixed-entropy Talk SeatObservation view must match committed golden fixture byte-for-byte"
    );

    // 3. Card 7 whole-token one-character mutation check across all 4 rendered paths.
    let cfg = must_ok(build_eval_response_format_config(), "build schema");
    let act_md = must_ok(
        render_seat_view_markdown(Round::Act, &obs),
        "render fixed act md",
    );
    let neg_str = must_ok(
        serde_json::to_string_pretty(&build_act_negotiation_inputs(vec![])),
        "serialize empty negotiation",
    );

    const SEALED_SCHEMA_SHA: &str =
        "052a89fd5bd4624c9a7c7a599772bf22b8cb4fad27eda0e006ecf2579ee8b544";
    const SEALED_TALK_VIEW_SHA: &str =
        "5727faa205f05a87e05309cb878e0dbf16137e4a73be4271e6b732482f7fd3f4";
    const SEALED_ACT_VIEW_SHA: &str =
        "e10bb49bb5cc9444205621aad5875207aed20dd79ba172c7b7827d9d2106b6fa";
    const SEALED_NEGOTIATION_SHA: &str =
        "15e4e56ea02879494f499510660fdb7fb799de5c573f71c23d573a8c218e9e1f";

    let schema_sha = cfg.canonical_sha256();
    let talk_view_sha = sha256_hex(talk_md.as_bytes());
    let act_view_sha = sha256_hex(act_md.as_bytes());
    let neg_sha = sha256_hex(neg_str.as_bytes());
    assert_eq!(schema_sha, SEALED_SCHEMA_SHA);
    assert_eq!(talk_view_sha, SEALED_TALK_VIEW_SHA);
    assert_eq!(act_view_sha, SEALED_ACT_VIEW_SHA);
    assert_eq!(neg_sha, SEALED_NEGOTIATION_SHA);

    let mutated_schema_sha = |schema_val: serde_json::Value| -> String {
        let mut altered_cfg = cfg.clone();
        altered_cfg.schema = schema_val;
        altered_cfg.canonical_sha256()
    };

    // Negative control: substring `"tok"` must never match whole JSON string tokens.
    let mut neg_ctrl_val = cfg.schema.clone();
    assert!(!find_and_mutate_first_json_string_token(
        &mut neg_ctrl_val,
        "tok",
        "Xok"
    ));

    // Every entry in `EVAL_REPLY_WORDING` must break its sealed render-path digest on a 1-char edit.
    for entry in EVAL_REPLY_WORDING {
        let shown = eval_wording_slot(entry.slot);
        let mutated = mutate_last_char(shown);
        match entry.slot {
            "eval_messages_key"
            | "eval_messages_description"
            | "eval_to_guild_key"
            | "eval_to_guild_description"
            | "eval_body_key"
            | "eval_body_description" => {
                let mut v = cfg.schema.clone();
                assert!(
                    find_and_mutate_first_json_string_token(&mut v, shown, &mutated),
                    "slot `{}` missing from eval schema as whole token",
                    entry.slot
                );
                assert_ne!(mutated_schema_sha(v), SEALED_SCHEMA_SHA);
            }
            "eval_messages_max_items" => {
                let mut v = cfg.schema.clone();
                let current = v["properties"]["messages"]["maxItems"].as_u64();
                assert_eq!(current, Some(EVAL_MESSAGES_MAX_ITEMS as u64));
                v["properties"]["messages"]["maxItems"] = json!(EVAL_MESSAGES_MAX_ITEMS + 1);
                assert_ne!(mutated_schema_sha(v), SEALED_SCHEMA_SHA);
            }
            "eval_talk_instruction" => {
                assert!(talk_md.split("\n\n").any(|p| p == shown));
                let altered = talk_md.replacen(shown, &mutated, 1);
                assert_ne!(sha256_hex(altered.as_bytes()), SEALED_TALK_VIEW_SHA);
            }
            "eval_act_instruction" => {
                assert!(act_md.split("\n\n").any(|p| p == shown));
                let altered = act_md.replacen(shown, &mutated, 1);
                assert_ne!(sha256_hex(altered.as_bytes()), SEALED_ACT_VIEW_SHA);
            }
            "eval_state_intro" => {
                assert!(talk_md.split("\n\n").any(|p| p == shown));
                assert!(act_md.split("\n\n").any(|p| p == shown));
                let altered_talk = talk_md.replacen(shown, &mutated, 1);
                let altered_act = act_md.replacen(shown, &mutated, 1);
                assert_ne!(sha256_hex(altered_talk.as_bytes()), SEALED_TALK_VIEW_SHA);
                assert_ne!(sha256_hex(altered_act.as_bytes()), SEALED_ACT_VIEW_SHA);
            }
            "eval_batna" => {
                let mut v: serde_json::Value = must_ok(serde_json::from_str(&neg_str), "parse neg");
                assert!(find_and_mutate_first_json_string_token(
                    &mut v, shown, &mutated
                ));
                let altered = must_ok(serde_json::to_string_pretty(&v), "ser altered neg");
                assert_ne!(sha256_hex(altered.as_bytes()), SEALED_NEGOTIATION_SHA);
            }
            other => panic!("unhandled slot `{other}` in mutation check"),
        }
    }
}

fn collect_relay_seat_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn http_reply_file_has_content(path: &Path, expected_content: &str) -> bool {
    let Ok(txt) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(rec) = serde_json::from_str::<league_relay::SavedHttpReplyRecord>(&txt) else {
        return false;
    };
    let Ok(body_val) = serde_json::from_str::<serde_json::Value>(&rec.body) else {
        return false;
    };
    body_val["choices"][0]["message"]["content"].as_str() == Some(expected_content)
}

fn discarded_reply_file_has_content(path: &Path, expected_content: &str) -> bool {
    let Ok(txt) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(rec) = serde_json::from_str::<league_relay::SavedDiscardedReplyRecord>(&txt) else {
        return false;
    };
    rec.assistant_content.as_deref() == Some(expected_content)
}

#[test]
fn a1_e2e_fenced_think_and_trailing_replies_across_play_and_branch() {
    // 1. A1.3: `validate_eval_seats_file` refuses `reasoning.exclude_from_transcript: true` for both `local` and `openrouter`.
    for bad_seat in [
        json!({
            "guild": "Foundry",
            "backend": "local",
            "base_url": "http://127.0.0.1:18080/v1",
            "model": "Qwen/Qwen3-32B",
            "temperature": 0.0,
            "max_output_tokens": 512,
            "timeout_secs": 5.0,
            "max_gpu_secs_per_call": 10.0,
            "reasoning": { "effort": "low", "exclude_from_transcript": true }
        }),
        json!({
            "guild": "Foundry",
            "backend": "openrouter",
            "model": "qwen/qwen3-32b",
            "temperature": 0.0,
            "max_output_tokens": 512,
            "timeout_secs": 5.0,
            "max_cost_per_call": 0.05,
            "reasoning": { "effort": "medium", "exclude_from_transcript": true }
        }),
    ] {
        let sf_json = json!({
            "seats": [
                bad_seat,
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        });
        let sf = must_ok(
            SeatFile::from_json_str(&sf_json.to_string(), None),
            "parse seat file",
        );
        let err = match validate_eval_seats_file(&sf) {
            Err(e) => e,
            Ok(()) => panic!("must refuse reasoning.exclude_from_transcript=true"),
        };
        assert!(matches!(err, EvalError::EvalThinkStrippingForbidden(_)));
        assert_eq!(err.code(), "EVAL_THINK_STRIPPING_FORBIDDEN");
    }

    // 2. End-to-end across BOTH `play` and `branch`, under BOTH `off` and `on`:
    //    Because `validate_eval_seats_file` forbids `exclude_from_transcript: true` (`A1.3`) and
    //    `league-eval` enables `eval_mode` in `league-relay`, `league-relay` leaves `<think>`
    //    unstripped, runs attempt 1 only (no re-send), returns `NO_REPLY`, and persists both raw
    //    HTTP sidecars and discarded-reply sidecars in BOTH `off` and `on` arms.
    let valid_clean = "{\"messages\":[],\"market_actions\":[],\"bundle\":{}}".to_owned();
    let fenced_reply = format!("```json\n{valid_clean}\n```");
    let think_reply = format!("<think>internal plan</think>{valid_clean}");
    let trailing_reply = format!("{valid_clean} trailing_tokens");

    let cases = [
        (
            "fenced",
            fenced_reply,
            1_usize,
            true,
            Some("REPLY_SCHEMA_INVALID"),
        ),
        (
            "think",
            think_reply,
            1_usize,
            true,
            Some("REPLY_SCHEMA_INVALID"),
        ),
        (
            "trailing",
            trailing_reply,
            1_usize,
            true,
            Some("REPLY_SCHEMA_INVALID"),
        ),
    ];

    for (shape_name, raw_reply, expect_calls, expect_no_reply, expect_code) in cases {
        for arm in [EvalArm::Off, EvalArm::On] {
            let tmp = fresh_tmp_dir(&format!("a1_{shape_name}_{}", arm.as_str()));
            let play_dir = tmp.join("play_game");
            let (wrapper_path, _) = write_wrapper(&tmp);
            let seats_path = write_seats_file(&tmp, one_openrouter_three_clubbot_seats());

            // Queue an extra clean reply behind the malformed reply to prove `eval_mode` never
            // triggers a second HTTP call for that round.
            let play_queue = vec![valid_clean.clone(), raw_reply.clone(), valid_clean.clone()];
            let play_transport = MockTransport::from_contents(play_queue);
            let manifest = must_ok(
                run_play_with_transport(
                    &PlayOptions {
                        game_dir: play_dir.clone(),
                        seats_path: seats_path.clone(),
                        wrapper_path: wrapper_path.clone(),
                        wrapper_version: None,
                        arm,
                        table_id: 90,
                        seed: 7,
                        turns: 1,
                        memory_mode: MemoryMode::Snapshot,
                        live: false,
                        cap_credits: None,
                        cap_gpu_secs: None,
                        allowed_vm_hosts: Vec::new(),
                    },
                    &play_transport,
                    None,
                ),
                &format!("play a1 {shape_name} {}", arm.as_str()),
            );
            assert_eq!(
                play_transport.recorded_requests().len(),
                1 + expect_calls,
                "mock HTTP call count mismatch in play {shape_name}/{}",
                arm.as_str()
            );

            let act_dec: EvalDecisionRecord = must_ok(
                serde_json::from_str(&must_ok(
                    fs::read_to_string(play_dir.join("decisions/day-01-act-Foundry.json")),
                    "read play act decision",
                )),
                "parse play act decision",
            );
            assert_eq!(act_dec.attempts, expect_calls as u32);
            assert_eq!(act_dec.no_reply, expect_no_reply);
            assert_eq!(act_dec.refusal_code.as_deref(), expect_code);
            let play_calls_raw = must_ok(
                fs::read_to_string(play_dir.join("relay/calls.jsonl")),
                "read calls",
            );
            let play_act_call: CallLogEntry = must_some(
                play_calls_raw
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(|l| serde_json::from_str::<CallLogEntry>(l).ok())
                    .find(|e| e.guild == "Foundry" && e.round == Round::Act),
                "Foundry Day 1 Act call log row",
            );
            assert_eq!(play_act_call.attempt, 1);
            assert_eq!(play_act_call.outcome, "NO_REPLY");
            if expect_no_reply {
                assert_eq!(act_dec.failed_attempts.len(), expect_calls);
                assert_eq!(act_dec.failed_attempts[0].attempt, 1);
                assert_eq!(act_dec.failed_attempts[0].outcome, "NO_REPLY");
                let disc_rel = must_some(
                    act_dec.discarded_reply_path.as_deref(),
                    "play discarded_reply_path",
                );
                let disc_sha = must_some(
                    act_dec.discarded_reply_sha256.as_deref(),
                    "play discarded_reply_sha256",
                );
                assert_eq!(
                    disc_rel,
                    "relay/seats/Foundry.day-01-act.discarded-round-0-attempt-1.json"
                );
                let disc_bytes = must_ok(fs::read(play_dir.join(disc_rel)), "read play discarded");
                assert_eq!(sha256_hex(&disc_bytes), disc_sha);
                assert_eq!(play_act_call.reply_sha256.as_deref(), Some(disc_sha));
                assert!(play_act_call.http_reply_sha256.is_some());
                assert_eq!(
                    manifest.refusal_counts_by_seat["Foundry"].reply_schema_invalid,
                    1
                );
                assert_eq!(
                    manifest.refusal_counts_by_arm[arm.as_str()].reply_schema_invalid,
                    1
                );
            } else {
                assert!(act_dec.failed_attempts.is_empty());
                assert_eq!(act_dec.raw_reply.as_deref(), Some(valid_clean.as_str()));
                assert_eq!(
                    manifest.refusal_counts_by_seat["Foundry"].reply_schema_invalid,
                    0
                );
            }

            let play_relay_files = collect_relay_seat_files(&play_dir.join("relay/seats"));
            let has_act_http_raw = play_relay_files.iter().any(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                n.contains("day-01-act.http-reply-round-")
                    && http_reply_file_has_content(p, &raw_reply)
            });
            let has_discarded_raw = play_relay_files.iter().any(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                n.contains("day-01-act.discarded-round-")
                    && discarded_reply_file_has_content(p, &raw_reply)
            });
            assert!(
                has_act_http_raw,
                "play --arm {} must persist raw HTTP reply for `{shape_name}`",
                arm.as_str()
            );
            if expect_no_reply {
                assert!(
                    has_discarded_raw,
                    "play --arm {} must persist discarded sidecar for `{shape_name}`",
                    arm.as_str()
                );
            }

            // Now prepare a clean source game and branch on `day-01-pre-act` with `raw_reply`.
            let src_dir = tmp.join("src_game");
            let src_transport =
                MockTransport::from_contents(vec![valid_clean.clone(), valid_clean.clone()]);
            let _ = must_ok(
                run_play_with_transport(
                    &PlayOptions {
                        game_dir: src_dir.clone(),
                        seats_path: seats_path.clone(),
                        wrapper_path: wrapper_path.clone(),
                        wrapper_version: None,
                        arm,
                        table_id: 91,
                        seed: 7,
                        turns: 1,
                        memory_mode: MemoryMode::Snapshot,
                        live: false,
                        cap_credits: None,
                        cap_gpu_secs: None,
                        allowed_vm_hosts: Vec::new(),
                    },
                    &src_transport,
                    None,
                ),
                "clean source game",
            );

            let branch_out = tmp.join("branch_out");
            let branch_queue = vec![raw_reply.clone(), valid_clean.clone()];
            let branch_transport = MockTransport::from_contents(branch_queue);
            let outcomes = must_ok(
                run_branch_with_transport(
                    &BranchOptions {
                        checkpoint_prefix: Some(src_dir.join("checkpoints/day-01-pre-act")),
                        list_path: None,
                        focal_seat: Some("Foundry".to_owned()),
                        seats_path: seats_path.clone(),
                        wrapper_path: wrapper_path.clone(),
                        arms: vec![arm],
                        out_dir: branch_out.clone(),
                        memory_mode: MemoryMode::Snapshot,
                        live: false,
                        cap_credits: None,
                        cap_gpu_secs: None,
                        allowed_vm_hosts: Vec::new(),
                    },
                    &branch_transport,
                    None,
                ),
                &format!("branch a1 {shape_name} {}", arm.as_str()),
            );
            assert_eq!(branch_transport.recorded_requests().len(), expect_calls);
            assert_eq!(outcomes.len(), 1);
            let b_dec = &outcomes[0].decision;
            assert_eq!(b_dec.attempts, expect_calls as u32);
            assert_eq!(b_dec.no_reply, expect_no_reply);
            assert_eq!(b_dec.refusal_code.as_deref(), expect_code);
            let b_calls_raw = must_ok(
                fs::read_to_string(branch_out.join(arm.as_str()).join("relay/calls.jsonl")),
                "read branch calls",
            );
            let b_call: CallLogEntry = must_some(
                b_calls_raw
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(|l| serde_json::from_str::<CallLogEntry>(l).ok())
                    .next(),
                "branch call log row",
            );
            assert_eq!(b_call.attempt, 1);
            assert_eq!(b_call.outcome, "NO_REPLY");
            if expect_no_reply {
                let b_disc_rel = must_some(
                    b_dec.discarded_reply_path.as_deref(),
                    "branch discarded_reply_path",
                );
                let b_disc_sha = must_some(
                    b_dec.discarded_reply_sha256.as_deref(),
                    "branch discarded_reply_sha256",
                );
                let b_disc_bytes = must_ok(
                    fs::read(branch_out.join(arm.as_str()).join(b_disc_rel)),
                    "read branch discarded",
                );
                assert_eq!(sha256_hex(&b_disc_bytes), b_disc_sha);
                assert_eq!(b_call.reply_sha256.as_deref(), Some(b_disc_sha));
            }

            let summary: BranchSummary = must_ok(
                serde_json::from_str(&must_ok(
                    fs::read_to_string(branch_out.join("branch_summary.json")),
                    "read branch_summary.json",
                )),
                "parse branch_summary.json",
            );
            assert_eq!(summary.evaluated_arms, 1);
            assert_eq!(summary.wrong_part_refusals.total_model_rounds, 1);
            if expect_no_reply {
                assert_eq!(
                    summary.refusal_counts_by_seat["Foundry"].reply_schema_invalid,
                    1
                );
                assert_eq!(
                    summary.refusal_counts_by_arm[arm.as_str()].reply_schema_invalid,
                    1
                );
                assert_eq!(summary.wrong_part_refusals.delivered_model_replies, 0);
            } else {
                assert_eq!(summary.wrong_part_refusals.delivered_model_replies, 1);
            }

            let branch_relay_files =
                collect_relay_seat_files(&branch_out.join(arm.as_str()).join("relay/seats"));
            let b_has_http = branch_relay_files.iter().any(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                n.contains("http-reply-round-") && http_reply_file_has_content(p, &raw_reply)
            });
            let b_has_disc = branch_relay_files.iter().any(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                n.contains("discarded-round-") && discarded_reply_file_has_content(p, &raw_reply)
            });
            assert!(
                b_has_http,
                "branch --arm {} must persist raw HTTP reply",
                arm.as_str()
            );
            if expect_no_reply {
                assert!(
                    b_has_disc,
                    "branch --arm {} must persist discarded sidecars",
                    arm.as_str()
                );
            }

            let _ = fs::remove_dir_all(&tmp);
        }
    }
}

#[test]
fn b1_e2e_wrong_round_wins_over_parse_and_branch_counts_both_denominators() {
    let tmp = fresh_tmp_dir("b1_e2e");
    let game_dir = tmp.join("game");
    let (wrapper_path, _) = write_wrapper(&tmp);
    let seats_path = write_seats_file(&tmp, one_openrouter_three_clubbot_seats());

    // Turn 1 talk: `post_offer` missing required fields (`action_kind` only -> passes r2 relay schema!)
    //   -> `TALK_REPLY_HAS_ORDERS` wins over order-parse failure (`is_wrong_part=true`).
    // Turn 1 act: clean empty reply.
    // Turn 2 talk: duplicate JSON key (empty orders -> `TALK_REPLY_INVALID`).
    // Turn 2 act: malformed fenced JSON (1 failed relay attempt in eval_mode -> NO_REPLY -> `REPLY_SCHEMA_INVALID`).
    let incomplete_post_offer_talk = json!({
        "messages": [],
        "market_actions": [{ "action_kind": "post_offer" }],
        "bundle": {}
    })
    .to_string();
    let clean = json!({ "messages": [], "market_actions": [], "bundle": {} }).to_string();
    let dup_key_talk =
        "{\"messages\":[],\"messages\":[],\"market_actions\":[],\"bundle\":{}}".to_owned();
    let bad_fenced_act = format!("```json\n{clean}\n```");

    let transport = MockTransport::from_contents(vec![
        incomplete_post_offer_talk,
        clean,
        dup_key_talk,
        bad_fenced_act.clone(),
    ]);
    let manifest = must_ok(
        run_play_with_transport(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper_path.clone(),
                wrapper_version: None,
                arm: EvalArm::Off,
                table_id: 92,
                seed: 7,
                turns: 2,
                memory_mode: MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &transport,
            None,
        ),
        "b1 e2e play",
    );

    let d1_talk: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-01-talk-Foundry.json")),
            "read d1 talk",
        )),
        "parse d1 talk",
    );
    assert_eq!(
        d1_talk.refusal_code.as_deref(),
        Some("TALK_REPLY_HAS_ORDERS")
    );

    let d2_talk: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-02-talk-Foundry.json")),
            "read d2 talk",
        )),
        "parse d2 talk",
    );
    assert_eq!(d2_talk.refusal_code.as_deref(), Some("TALK_REPLY_INVALID"));

    let d2_act: EvalDecisionRecord = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(game_dir.join("decisions/day-02-act-Foundry.json")),
            "read d2 act",
        )),
        "parse d2 act",
    );
    assert!(d2_act.no_reply);
    assert_eq!(d2_act.attempts, 1);
    assert_eq!(d2_act.refusal_code.as_deref(), Some("REPLY_SCHEMA_INVALID"));
    assert!(d2_act.discarded_reply_path.is_some());
    assert!(d2_act.discarded_reply_sha256.is_some());

    // 4 evaluated rounds, 3 delivered, 1 no_reply, 1 wrong-part refusal:
    assert_eq!(manifest.wrong_part_refusals.talk_reply_has_orders, 1);
    assert_eq!(manifest.wrong_part_refusals.delivered_model_replies, 3);
    assert_eq!(manifest.wrong_part_refusals.total_model_rounds, 4);
    assert!((manifest.wrong_part_refusals.wrong_part_rate_delivered - (1.0 / 3.0)).abs() < 1e-12);
    assert!((manifest.wrong_part_refusals.wrong_part_rate_all_rounds - 0.25).abs() < 1e-12);
    assert_eq!(
        manifest.refusal_counts_by_seat["Foundry"].talk_reply_has_orders,
        1
    );
    assert_eq!(
        manifest.refusal_counts_by_seat["Foundry"].talk_reply_invalid,
        1
    );
    assert_eq!(
        manifest.refusal_counts_by_seat["Foundry"].reply_schema_invalid,
        1
    );

    // Branch across [Off, On]: Off gets ACT_REPLY_HAS_MESSAGES (`post_offer` missing fields + message),
    // On gets NO_REPLY (`REPLY_SCHEMA_INVALID` after 1 attempt in eval_mode) -> `branch_summary.json` tracks BOTH!
    let wrong_act_with_bad_orders = json!({
        "messages": [{ "to_guild": "Quarry", "body": "in act" }],
        "market_actions": [{ "action_kind": "post_offer" }],
        "bundle": {}
    })
    .to_string();
    let branch_out = tmp.join("branch_out");
    let branch_transport =
        MockTransport::from_contents(vec![wrong_act_with_bad_orders, bad_fenced_act]);
    let outcomes = must_ok(
        run_branch_with_transport(
            &BranchOptions {
                checkpoint_prefix: Some(game_dir.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path,
                wrapper_path,
                arms: vec![EvalArm::Off, EvalArm::On],
                out_dir: branch_out.clone(),
                memory_mode: MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            &branch_transport,
            None,
        ),
        "b1 branch",
    );
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes[0].decision.refusal_code.as_deref(),
        Some("ACT_REPLY_HAS_MESSAGES")
    );
    assert_eq!(outcomes[1].decision.attempts, 1);
    assert_eq!(
        outcomes[1].decision.refusal_code.as_deref(),
        Some("REPLY_SCHEMA_INVALID")
    );
    assert!(outcomes[1].decision.discarded_reply_path.is_some());
    assert!(outcomes[1].decision.discarded_reply_sha256.is_some());

    let summary: BranchSummary = must_ok(
        serde_json::from_str(&must_ok(
            fs::read_to_string(branch_out.join("branch_summary.json")),
            "read branch summary",
        )),
        "parse branch summary",
    );
    assert_eq!(summary.evaluated_arms, 2);
    assert_eq!(summary.wrong_part_refusals.act_reply_has_messages, 1);
    assert_eq!(summary.wrong_part_refusals.delivered_model_replies, 1);
    assert_eq!(summary.wrong_part_refusals.total_model_rounds, 2);
    assert_eq!(
        summary.refusal_counts_by_arm["off"].act_reply_has_messages,
        1
    );
    assert_eq!(summary.refusal_counts_by_arm["on"].reply_schema_invalid, 1);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn relay1_1a_e2e_wrong_round_precedes_schema_in_both_arms_and_single_call() {
    let clean = json!({ "messages": [], "market_actions": [], "bundle": {} }).to_string();
    let talk_orders_with_unknown_bundle_key = json!({
        "messages": [],
        "market_actions": [],
        "bundle": { "debt_repayment_cred": 5, "unknown_field": 1 }
    })
    .to_string();
    let act_messages_with_unknown_key = json!({
        "messages": [{ "to_guild": "Quarry", "body": "hi" }],
        "market_actions": [],
        "bundle": {},
        "unknown_key": 1
    })
    .to_string();
    let non_json = "not valid json at all".to_owned();

    for arm in [EvalArm::Off, EvalArm::On] {
        let tmp = fresh_tmp_dir(&format!("relay1_1a_{}", arm.as_str()));
        let (wrapper_path, _) = write_wrapper(&tmp);
        let seats_path = write_seats_file(&tmp, one_openrouter_three_clubbot_seats());

        // 1. Talk reply with orders + unknown bundle key -> TALK_REPLY_HAS_ORDERS (1 call),
        //    counts toward 5% stop-line, keeps discarded sidecar & sha256.
        let play_talk_dir = tmp.join("play_talk");
        let talk_transport = MockTransport::from_contents(vec![
            talk_orders_with_unknown_bundle_key.clone(),
            clean.clone(),
            clean.clone(),
        ]);
        let talk_manifest = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: play_talk_dir.clone(),
                    seats_path: seats_path.clone(),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    arm,
                    table_id: 93,
                    seed: 7,
                    turns: 1,
                    memory_mode: MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &talk_transport,
                None,
            ),
            "1a talk play",
        );
        // 1 call for Talk (single attempt, unconsumed spare stays unused) + 1 call for Act = 2.
        assert_eq!(talk_transport.recorded_requests().len(), 2);

        let d1_talk: EvalDecisionRecord = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(play_talk_dir.join("decisions/day-01-talk-Foundry.json")),
                "read d1 talk",
            )),
            "parse d1 talk",
        );
        assert!(d1_talk.no_reply);
        assert_eq!(d1_talk.attempts, 1);
        assert_eq!(
            d1_talk.refusal_code.as_deref(),
            Some("TALK_REPLY_HAS_ORDERS")
        );
        assert_eq!(
            d1_talk.raw_reply.as_deref(),
            Some(talk_orders_with_unknown_bundle_key.as_str())
        );
        let talk_disc_rel = must_some(
            d1_talk.discarded_reply_path.as_deref(),
            "talk discarded_reply_path",
        );
        let talk_disc_sha = must_some(
            d1_talk.discarded_reply_sha256.as_deref(),
            "talk discarded_reply_sha256",
        );
        assert_eq!(
            sha256_hex(&must_ok(
                fs::read(play_talk_dir.join(talk_disc_rel)),
                "read talk discarded file"
            )),
            talk_disc_sha
        );
        assert_eq!(talk_manifest.wrong_part_refusals.talk_reply_has_orders, 1);
        assert_eq!(
            talk_manifest.wrong_part_refusals.total_wrong_part_refusals,
            1
        );
        assert_eq!(talk_manifest.wrong_part_refusals.delivered_model_replies, 1);
        assert_eq!(talk_manifest.wrong_part_refusals.total_model_rounds, 2);
        assert!(
            talk_manifest
                .wrong_part_refusals
                .exceeds_five_percent_preflight_stop_line_delivered
        );
        assert!(
            talk_manifest
                .wrong_part_refusals
                .exceeds_five_percent_preflight_stop_line_all_rounds
        );
        assert_eq!(
            talk_manifest.refusal_counts_by_seat["Foundry"].talk_reply_has_orders,
            1
        );
        assert_eq!(
            talk_manifest.refusal_counts_by_seat["Foundry"].reply_schema_invalid,
            0
        );

        // 2. Act reply with messages + unknown key -> ACT_REPLY_HAS_MESSAGES (1 call), in play & branch.
        let play_act_dir = tmp.join("play_act");
        let act_transport = MockTransport::from_contents(vec![
            clean.clone(),
            act_messages_with_unknown_key.clone(),
            clean.clone(),
        ]);
        let act_manifest = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: play_act_dir.clone(),
                    seats_path: seats_path.clone(),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    arm,
                    table_id: 94,
                    seed: 7,
                    turns: 1,
                    memory_mode: MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &act_transport,
                None,
            ),
            "1a act play",
        );
        assert_eq!(act_transport.recorded_requests().len(), 2);

        let d1_act: EvalDecisionRecord = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(play_act_dir.join("decisions/day-01-act-Foundry.json")),
                "read d1 act",
            )),
            "parse d1 act",
        );
        assert!(d1_act.no_reply);
        assert_eq!(d1_act.attempts, 1);
        assert_eq!(
            d1_act.refusal_code.as_deref(),
            Some("ACT_REPLY_HAS_MESSAGES")
        );
        assert!(d1_act.discarded_reply_path.is_some());
        assert!(d1_act.discarded_reply_sha256.is_some());
        assert_eq!(act_manifest.wrong_part_refusals.act_reply_has_messages, 1);
        assert!(
            act_manifest
                .wrong_part_refusals
                .exceeds_five_percent_preflight_stop_line_delivered
        );
        assert!(
            act_manifest
                .wrong_part_refusals
                .exceeds_five_percent_preflight_stop_line_all_rounds
        );

        let branch_act_out = tmp.join("branch_act");
        let branch_act_transport = MockTransport::from_contents(vec![
            act_messages_with_unknown_key.clone(),
            clean.clone(),
        ]);
        let branch_act_outcomes = must_ok(
            run_branch_with_transport(
                &BranchOptions {
                    checkpoint_prefix: Some(play_act_dir.join("checkpoints/day-01-pre-act")),
                    list_path: None,
                    focal_seat: Some("Foundry".to_owned()),
                    seats_path: seats_path.clone(),
                    wrapper_path: wrapper_path.clone(),
                    arms: vec![arm],
                    out_dir: branch_act_out.clone(),
                    memory_mode: MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &branch_act_transport,
                None,
            ),
            "1a act branch",
        );
        assert_eq!(branch_act_transport.recorded_requests().len(), 1);
        assert_eq!(
            branch_act_outcomes[0].decision.refusal_code.as_deref(),
            Some("ACT_REPLY_HAS_MESSAGES")
        );
        assert_eq!(branch_act_outcomes[0].decision.attempts, 1);
        let branch_act_summary: BranchSummary = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(branch_act_out.join("branch_summary.json")),
                "read branch act summary",
            )),
            "parse branch act summary",
        );
        assert_eq!(
            branch_act_summary
                .wrong_part_refusals
                .act_reply_has_messages,
            1
        );
        assert!(
            branch_act_summary
                .wrong_part_refusals
                .exceeds_five_percent_preflight_stop_line_all_rounds
        );

        // 3. Non-JSON reply -> REPLY_SCHEMA_INVALID (1 call, not wrong-round), in play & branch.
        let play_nonjson_dir = tmp.join("play_nonjson");
        let nonjson_transport =
            MockTransport::from_contents(vec![clean.clone(), non_json.clone(), clean.clone()]);
        let nonjson_manifest = must_ok(
            run_play_with_transport(
                &PlayOptions {
                    game_dir: play_nonjson_dir.clone(),
                    seats_path: seats_path.clone(),
                    wrapper_path: wrapper_path.clone(),
                    wrapper_version: None,
                    arm,
                    table_id: 95,
                    seed: 7,
                    turns: 1,
                    memory_mode: MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &nonjson_transport,
                None,
            ),
            "1a non-json play",
        );
        assert_eq!(nonjson_transport.recorded_requests().len(), 2);
        let d1_nonjson: EvalDecisionRecord = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(play_nonjson_dir.join("decisions/day-01-act-Foundry.json")),
                "read non-json decision",
            )),
            "parse non-json decision",
        );
        assert!(d1_nonjson.no_reply);
        assert_eq!(d1_nonjson.attempts, 1);
        assert_eq!(
            d1_nonjson.refusal_code.as_deref(),
            Some("REPLY_SCHEMA_INVALID")
        );
        assert!(d1_nonjson.discarded_reply_path.is_some());
        assert!(d1_nonjson.discarded_reply_sha256.is_some());
        assert_eq!(
            nonjson_manifest
                .wrong_part_refusals
                .total_wrong_part_refusals,
            0
        );

        let branch_nonjson_out = tmp.join("branch_nonjson");
        let branch_nonjson_transport =
            MockTransport::from_contents(vec![non_json.clone(), clean.clone()]);
        let branch_nonjson_outcomes = must_ok(
            run_branch_with_transport(
                &BranchOptions {
                    checkpoint_prefix: Some(play_nonjson_dir.join("checkpoints/day-01-pre-act")),
                    list_path: None,
                    focal_seat: Some("Foundry".to_owned()),
                    seats_path,
                    wrapper_path,
                    arms: vec![arm],
                    out_dir: branch_nonjson_out.clone(),
                    memory_mode: MemoryMode::Snapshot,
                    live: false,
                    cap_credits: None,
                    cap_gpu_secs: None,
                    allowed_vm_hosts: Vec::new(),
                },
                &branch_nonjson_transport,
                None,
            ),
            "1a non-json branch",
        );
        assert_eq!(branch_nonjson_transport.recorded_requests().len(), 1);
        assert_eq!(
            branch_nonjson_outcomes[0].decision.refusal_code.as_deref(),
            Some("REPLY_SCHEMA_INVALID")
        );
        assert_eq!(branch_nonjson_outcomes[0].decision.attempts, 1);
        let branch_nonjson_summary: BranchSummary = must_ok(
            serde_json::from_str(&must_ok(
                fs::read_to_string(branch_nonjson_out.join("branch_summary.json")),
                "read branch non-json summary",
            )),
            "parse branch non-json summary",
        );
        assert_eq!(
            branch_nonjson_summary
                .wrong_part_refusals
                .total_wrong_part_refusals,
            0
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
