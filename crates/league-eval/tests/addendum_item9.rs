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
    EVAL_MESSAGES_MAX_ITEMS, EVAL_REPLY_WORDING, EVAL_REPLY_WORDING_ARM,
    EVAL_SEAT_VIEW_SERIALIZER_NAME, EvalArm, EvalDecisionRecord, EvalError, EvalManifest,
    EvalSetupFile, PASS_B_LIMIT_STATEMENT, PlayOptions, ReplyRefusalCode, WrongPartCounter,
    build_eval_response_format_config, eval_internal_slot_names, eval_wording_slot, execute_cli,
    parse_eval_act_reply, parse_eval_talk_reply, persist_eval_game_state,
    render_seat_view_markdown, run_play_with_transport,
};
use league_relay::{
    NegotiationInputs, PREVIEW_WORDING_ARM, RawHttpResponse, Round, Transport, TransportError,
    build_preview_turn_orders_tool_definition, preview_wording_slot, sha256_hex, wording_slot,
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
    let refusal_cases: [(&str, bool, &str); 14] = [
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
            "messages_exceeds_max_items",
            false,
            "{\"messages\":[{\"to_guild\":\"Quarry\",\"body\":\"1\"},{\"to_guild\":\"Quarry\",\"body\":\"2\"},{\"to_guild\":\"Quarry\",\"body\":\"3\"},{\"to_guild\":\"Quarry\",\"body\":\"4\"}],\"market_actions\":[],\"bundle\":{}}",
        ),
        (
            "messages_missing_body",
            false,
            "{\"messages\":[{\"to_guild\":\"Quarry\"}],\"market_actions\":[],\"bundle\":{}}",
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

    // 3. Item 1.8 WrongPartCounter incremented in both cases & triggers 5% stop line.
    assert_eq!(manifest.wrong_part_refusals.talk_reply_has_orders, 1);
    assert_eq!(manifest.wrong_part_refusals.act_reply_has_messages, 1);
    assert_eq!(manifest.wrong_part_refusals.total_wrong_part_refusals, 2);
    assert_eq!(manifest.wrong_part_refusals.total_model_replies, 2);
    assert!(
        manifest
            .wrong_part_refusals
            .exceeds_five_percent_preflight_stop_line
    );

    // Also verify boundary behavior around 5% (`20` replies w/ 1 refusal == 5.0% <= threshold; `19` replies == 5.26% > threshold).
    let mut counter = WrongPartCounter::default();
    for _ in 0..19 {
        counter.record_reply(None);
    }
    counter.record_reply(Some(ReplyRefusalCode::TalkReplyHasOrders));
    assert_eq!(counter.total_model_replies, 20);
    assert!(!counter.exceeds_five_percent_preflight_stop_line);

    let mut over_counter = WrongPartCounter::default();
    for _ in 0..18 {
        over_counter.record_reply(None);
    }
    over_counter.record_reply(Some(ReplyRefusalCode::ActReplyHasMessages));
    assert!(over_counter.exceeds_five_percent_preflight_stop_line);

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
    let cfg = must_ok(build_eval_response_format_config(), "build schema");
    let expected_sha = cfg.canonical_sha256();
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
