//! Strict reply parsing for Evaluation Mode `talk` and `act` rounds (`Item 9` addendum 1.3–1.5).
//!
//! Every `market_actions` and `bundle` payload delegates to
//! [`league_relay::parse_preview_tool_arguments`]. Think blocks, code fences, trailing tokens, and
//! duplicate JSON keys are strictly refused without stripping or repair (`1.4`). Only a top-level
//! `"notes"` string on note-keeping seats is tolerated (`1.4`).

use std::fmt;

use league_core::{EvalActSubmission, MarketAction, SealedOrderBundle};
use league_relay::{
    Round, UntrustedWhisper, parse_preview_tool_arguments, preview_wording_slot, wording_slot,
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::records::CommunicationMode;
use crate::wording::{EVAL_MESSAGES_MAX_ITEMS, eval_wording_slot};

/// Named refusal codes defined by the Item 9 reply contract (`1.3`, `A1`, `B1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ReplyRefusalCode {
    /// An `act` reply is not one schema-valid unique-key JSON object, or its
    /// `market_actions` / `bundle` failed parsing.
    #[serde(rename = "ACT_REPLY_INVALID")]
    ActReplyInvalid,
    /// A `talk` reply with no orders is not one schema-valid unique-key JSON object, or its
    /// `messages` / `market_actions` / `bundle` shape failed parsing (`B1`).
    #[serde(rename = "TALK_REPLY_INVALID")]
    TalkReplyInvalid,
    /// `league-relay` returned `NoReply` after exhausting schema / JSON retries (`A1.2`).
    #[serde(rename = "REPLY_SCHEMA_INVALID")]
    ReplySchemaInvalid,
    /// A `talk` reply contained non-empty `market_actions` or a non-empty `bundle`.
    #[serde(rename = "TALK_REPLY_HAS_ORDERS")]
    TalkReplyHasOrders,
    /// An `act` reply contained non-empty `messages`.
    #[serde(rename = "ACT_REPLY_HAS_MESSAGES")]
    ActReplyHasMessages,
    /// One message in an accepted `talk` reply addressed an unknown guild or the sender itself.
    #[serde(rename = "MESSAGE_RECIPIENT_INVALID")]
    MessageRecipientInvalid,
}

impl ReplyRefusalCode {
    /// Stable `SCREAMING_SNAKE_CASE` code string (`1.3`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActReplyInvalid => "ACT_REPLY_INVALID",
            Self::TalkReplyInvalid => "TALK_REPLY_INVALID",
            Self::ReplySchemaInvalid => "REPLY_SCHEMA_INVALID",
            Self::TalkReplyHasOrders => "TALK_REPLY_HAS_ORDERS",
            Self::ActReplyHasMessages => "ACT_REPLY_HAS_MESSAGES",
            Self::MessageRecipientInvalid => "MESSAGE_RECIPIENT_INVALID",
        }
    }

    /// Parse a serialized `SCREAMING_SNAKE_CASE` refusal code (`B5` recount / branch summary).
    #[must_use]
    pub fn from_code_str(raw: &str) -> Option<Self> {
        match raw {
            "ACT_REPLY_INVALID" => Some(Self::ActReplyInvalid),
            "TALK_REPLY_INVALID" => Some(Self::TalkReplyInvalid),
            "REPLY_SCHEMA_INVALID" => Some(Self::ReplySchemaInvalid),
            "TALK_REPLY_HAS_ORDERS" => Some(Self::TalkReplyHasOrders),
            "ACT_REPLY_HAS_MESSAGES" => Some(Self::ActReplyHasMessages),
            "MESSAGE_RECIPIENT_INVALID" => Some(Self::MessageRecipientInvalid),
            _ => None,
        }
    }
}

/// One validated `{to_guild, body}` message parsed from a reply's `messages` array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedEvalMessage {
    /// Target guild name.
    pub to_guild: String,
    /// Message text body.
    pub body: String,
}

/// Diagnostic for a single message rejected inside a `talk` reply (`MESSAGE_RECIPIENT_INVALID`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidRecipientDiagnostic {
    /// Zero-based index inside the reply's `messages` array.
    pub message_index: usize,
    /// Recipient string supplied by the seat.
    pub to_guild: String,
    /// Refusal code (`MESSAGE_RECIPIENT_INVALID`).
    pub code: ReplyRefusalCode,
    /// Human-readable reason.
    pub reason: String,
}

/// Outcome of strictly parsing a seat's `act` reply (`from-seat/day-DD-act-<Guild>.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedActReply {
    /// Final submission passed to `EvalHost::apply_act_submissions` (`default()` on refusal).
    pub submission: EvalActSubmission,
    /// Optional primary refusal code (`ACT_REPLY_INVALID` or `ACT_REPLY_HAS_MESSAGES`).
    pub refusal_code: Option<ReplyRefusalCode>,
    /// Detailed explanation when refused.
    pub refusal_reason: Option<String>,
    /// `true` iff `keeps_notes == true` and a string `"notes"` key was present (`1.4`).
    pub notes_present: bool,
    /// `true` iff the refusal was `ACT_REPLY_HAS_MESSAGES` (`1.8`).
    pub is_wrong_part: bool,
}

/// Outcome of strictly parsing a seat's `talk` reply (`from-seat/day-DD-talk-<Guild>.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTalkReply {
    /// Accepted whispers ready to route to `(recipient_guild, whisper)`.
    pub accepted_whispers: Vec<(String, UntrustedWhisper)>,
    /// Whole-reply refusal code (`TALK_REPLY_HAS_ORDERS` or `TALK_REPLY_INVALID` on malformed JSON).
    pub refusal_code: Option<ReplyRefusalCode>,
    /// Per-message invalid-recipient diagnostics (`MESSAGE_RECIPIENT_INVALID`, `1.3`).
    pub invalid_recipients: Vec<InvalidRecipientDiagnostic>,
    /// Detailed explanation when refused at reply level.
    pub refusal_reason: Option<String>,
    /// `true` iff `keeps_notes == true` and a string `"notes"` key was present (`1.4`).
    pub notes_present: bool,
    /// `true` iff the refusal was `TALK_REPLY_HAS_ORDERS` (`1.8`).
    pub is_wrong_part: bool,
}

struct StrictUniqueValue;

impl<'de> DeserializeSeed<'de> for StrictUniqueValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictUniqueVisitor)
    }
}

struct StrictUniqueVisitor;

impl<'de> Visitor<'de> for StrictUniqueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any valid JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| de::Error::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut arr = Vec::new();
        while let Some(elem) = seq.next_element_seed(StrictUniqueValue)? {
            arr.push(elem);
        }
        Ok(Value::Array(arr))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut obj = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if obj.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate JSON key '{key}'")));
            }
            let val = map.next_value_seed(StrictUniqueValue)?;
            obj.insert(key, val);
        }
        Ok(Value::Object(obj))
    }
}

fn parse_strict_json_object(raw: &str) -> Result<Map<String, Value>, String> {
    if raw.contains("<think>") || raw.contains("</think>") {
        return Err("think block is not permitted in Eval seat replies".to_owned());
    }
    if raw.contains("```") {
        return Err("markdown code fence is not permitted in Eval seat replies".to_owned());
    }
    let mut de = serde_json::Deserializer::from_str(raw.trim());
    let val = StrictUniqueValue
        .deserialize(&mut de)
        .map_err(|e| format!("invalid JSON: {e}"))?;
    de.end()
        .map_err(|e| format!("trailing content after JSON object: {e}"))?;
    match val {
        Value::Object(obj) => Ok(obj),
        _ => Err("reply root must be a JSON object".to_owned()),
    }
}

struct CommonReplyEnvelope {
    messages: Vec<ParsedEvalMessage>,
    market_actions: Vec<league_core::MarketAction>,
    bundle: SealedOrderBundle,
    notes_present: bool,
}

fn raw_bundle_map_is_empty(b: &Map<String, Value>) -> bool {
    const EMPTY_ARRAY_KEYS: [&str; 8] = [
        "converter_runs",
        "project_deliveries",
        "atomic_swaps",
        "contract_deliveries",
        "dual_escrows",
        "escrow_finishes",
        "check_cashes",
        "charter_bids",
    ];
    for (k, v) in b {
        if k == "debt_repayment_cred" {
            if v.as_u64() != Some(0) {
                return false;
            }
        } else if k == "council_vote" {
            if !v.is_null() {
                return false;
            }
        } else if EMPTY_ARRAY_KEYS.contains(&k.as_str()) {
            if !v.as_array().is_some_and(Vec::is_empty) {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

fn detect_raw_wrong_round_obj(
    round: Round,
    obj: &Map<String, Value>,
) -> Option<(ReplyRefusalCode, String)> {
    match round {
        Round::Talk => {
            let k_ma = preview_wording_slot("key_market_actions");
            let k_bundle = preview_wording_slot("key_bundle");
            let has_non_empty_ma = obj
                .get(&k_ma)
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
            let has_non_empty_bundle = obj
                .get(&k_bundle)
                .and_then(Value::as_object)
                .is_some_and(|b| !raw_bundle_map_is_empty(b));
            if has_non_empty_ma || has_non_empty_bundle {
                Some((
                    ReplyRefusalCode::TalkReplyHasOrders,
                    format!("talk reply must leave '{k_ma}' and '{k_bundle}' empty"),
                ))
            } else {
                None
            }
        }
        Round::Act => {
            let k_messages = eval_wording_slot("eval_messages_key");
            if let Some(msg_arr) = obj.get(k_messages).and_then(Value::as_array)
                && !msg_arr.is_empty()
            {
                Some((
                    ReplyRefusalCode::ActReplyHasMessages,
                    format!(
                        "act reply must leave '{k_messages}' empty; got {} message(s)",
                        msg_arr.len()
                    ),
                ))
            } else {
                None
            }
        }
    }
}

/// Inspect raw text returned from a discarded/schema-failed Eval round for wrong-round orders or
/// messages before coding `REPLY_SCHEMA_INVALID` (`§1a` item 2). Non-JSON text returns `(None, false)`.
#[must_use]
pub(crate) fn classify_wrong_round_from_raw_text(
    round: Round,
    raw: &str,
    keeps_notes: bool,
) -> (Option<(ReplyRefusalCode, String)>, bool) {
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw.trim()) else {
        return (None, false);
    };
    let k_notes = wording_slot("scratchpad_schema_key");
    let notes_present = keeps_notes && obj.get(&k_notes).is_some_and(Value::is_string);
    (detect_raw_wrong_round_obj(round, &obj), notes_present)
}

fn validate_common_reply_envelope_obj(
    obj: &Map<String, Value>,
    asking_seat: u8,
    keeps_notes: bool,
) -> (Option<CommonReplyEnvelope>, Option<String>, bool) {
    validate_reply_envelope_obj(obj, asking_seat, keeps_notes, true)
}

fn validate_reply_envelope_obj(
    obj: &Map<String, Value>,
    asking_seat: u8,
    keeps_notes: bool,
    allow_messages: bool,
) -> (Option<CommonReplyEnvelope>, Option<String>, bool) {
    let k_notes = wording_slot("scratchpad_schema_key");
    let k_messages = eval_wording_slot("eval_messages_key");
    let k_to_guild = eval_wording_slot("eval_to_guild_key");
    let k_body = eval_wording_slot("eval_body_key");
    let k_ma = preview_wording_slot("key_market_actions");
    let k_bundle = preview_wording_slot("key_bundle");

    let notes_present = keeps_notes && obj.get(&k_notes).is_some_and(Value::is_string);

    for k in obj.keys() {
        let allowed = (allow_messages && k == k_messages)
            || k == k_ma.as_str()
            || k == k_bundle.as_str()
            || (keeps_notes && k == &k_notes);
        if !allowed {
            let reason = if k == &k_notes && !keeps_notes {
                format!("unexpected '{k_notes}' property on non-note-keeping seat")
            } else {
                format!("unexpected top-level property '{k}'")
            };
            return (None, Some(reason), notes_present);
        }
    }

    if keeps_notes && !notes_present {
        return (
            None,
            Some(format!(
                "missing or non-string required '{k_notes}' property on note-keeping seat"
            )),
            false,
        );
    }

    let messages = if allow_messages {
        let Some(msg_arr) = obj.get(k_messages).and_then(Value::as_array) else {
            return (
                None,
                Some(format!("missing or non-array '{k_messages}' property")),
                notes_present,
            );
        };
        if msg_arr.len() > EVAL_MESSAGES_MAX_ITEMS {
            return (
                None,
                Some(format!(
                    "'{k_messages}' has {} items, exceeding maxItems={EVAL_MESSAGES_MAX_ITEMS}",
                    msg_arr.len()
                )),
                notes_present,
            );
        }

        let mut parsed_msgs = Vec::with_capacity(msg_arr.len());
        for (idx, item) in msg_arr.iter().enumerate() {
            let Some(m_obj) = item.as_object() else {
                return (
                    None,
                    Some(format!("'{k_messages}[{idx}]' must be a JSON object")),
                    notes_present,
                );
            };
            for mk in m_obj.keys() {
                if mk != k_to_guild && mk != k_body {
                    return (
                        None,
                        Some(format!(
                            "'{k_messages}[{idx}]' contains unexpected key '{mk}'"
                        )),
                        notes_present,
                    );
                }
            }
            let Some(to_guild) = m_obj.get(k_to_guild).and_then(Value::as_str) else {
                return (
                    None,
                    Some(format!(
                        "'{k_messages}[{idx}]' missing string '{k_to_guild}'"
                    )),
                    notes_present,
                );
            };
            let Some(body) = m_obj.get(k_body).and_then(Value::as_str) else {
                return (
                    None,
                    Some(format!("'{k_messages}[{idx}]' missing string '{k_body}'")),
                    notes_present,
                );
            };
            parsed_msgs.push(ParsedEvalMessage {
                to_guild: to_guild.to_owned(),
                body: body.to_owned(),
            });
        }
        parsed_msgs
    } else {
        Vec::new()
    };

    let Some(ma_val) = obj.get(&k_ma) else {
        return (
            None,
            Some(format!("missing required '{k_ma}' property")),
            notes_present,
        );
    };
    let Some(bundle_val) = obj.get(&k_bundle) else {
        return (
            None,
            Some(format!("missing required '{k_bundle}' property")),
            notes_present,
        );
    };

    let order_payload = serde_json::json!({
        k_ma.as_str(): ma_val,
        k_bundle.as_str(): bundle_val,
    });
    let order_json = match serde_json::to_string(&order_payload) {
        Ok(s) => s,
        Err(e) => {
            return (
                None,
                Some(format!("failed to serialize orders for parser: {e}")),
                notes_present,
            );
        }
    };
    let Some(parsed_req) = parse_preview_tool_arguments(&order_json, asking_seat) else {
        let reason =
            diagnose_order_parse_failure(&k_ma, &k_bundle, ma_val, bundle_val, asking_seat);
        return (None, Some(reason), notes_present);
    };

    (
        Some(CommonReplyEnvelope {
            messages,
            market_actions: parsed_req.market_actions,
            bundle: parsed_req.bundle,
            notes_present,
        }),
        None,
        notes_present,
    )
}

fn diagnose_order_parse_failure(
    k_ma: &str,
    k_bundle: &str,
    ma_val: &Value,
    bundle_val: &Value,
    asking_seat: u8,
) -> String {
    let Some(ma_arr) = ma_val.as_array() else {
        return format!("'{k_ma}' must be a JSON array");
    };
    if !bundle_val.is_object() {
        return format!("'{k_bundle}' must be a JSON object");
    }
    for (idx, item) in ma_arr.iter().enumerate() {
        let probe = serde_json::json!({
            k_ma: [item],
            k_bundle: {},
        });
        if probe
            .to_string()
            .pipe(|s| parse_preview_tool_arguments(&s, asking_seat).is_none())
        {
            return format!("invalid market action at '{k_ma}[{idx}]'");
        }
    }
    let bundle_probe = serde_json::json!({
        k_ma: [],
        k_bundle: bundle_val,
    });
    if parse_preview_tool_arguments(&bundle_probe.to_string(), asking_seat).is_none() {
        return format!("invalid order bundle in '{k_bundle}'");
    }
    format!("invalid '{k_ma}' or '{k_bundle}'")
}

trait PipeExt: Sized {
    fn pipe<U>(self, f: impl FnOnce(Self) -> U) -> U {
        f(self)
    }
}
impl<T> PipeExt for T {}

/// Strictly parse a delivered `act` reply (`from-seat/day-DD-act-<Guild>.md`, `1.3`, `1.4`, `B1`).
///
/// Wrong-round detection (`ACT_REPLY_HAS_MESSAGES`) runs on the raw JSON object before order or
/// message validation (`B1`). Refused act replies always yield [`EvalActSubmission::default()`]
/// with empty `pass_b_actions` (`1.1`, `1.3`). Valid model-seat act replies always leave
/// `pass_b_actions` empty (`1.1`).
#[must_use]
pub fn parse_eval_act_reply(raw: &str, asking_seat: u8, keeps_notes: bool) -> ParsedActReply {
    let obj = match parse_strict_json_object(raw) {
        Ok(o) => o,
        Err(reason) => {
            return ParsedActReply {
                submission: EvalActSubmission::default(),
                refusal_code: Some(ReplyRefusalCode::ActReplyInvalid),
                refusal_reason: Some(reason),
                notes_present: false,
                is_wrong_part: false,
            };
        }
    };
    let k_notes = wording_slot("scratchpad_schema_key");
    let notes_present = keeps_notes && obj.get(&k_notes).is_some_and(Value::is_string);
    if let Some((code, reason)) = detect_raw_wrong_round_obj(Round::Act, &obj) {
        return ParsedActReply {
            submission: EvalActSubmission::default(),
            refusal_code: Some(code),
            refusal_reason: Some(reason),
            notes_present,
            is_wrong_part: true,
        };
    }

    let (env_opt, err_opt, env_notes_present) =
        validate_common_reply_envelope_obj(&obj, asking_seat, keeps_notes);
    let Some(env) = env_opt else {
        return ParsedActReply {
            submission: EvalActSubmission::default(),
            refusal_code: Some(ReplyRefusalCode::ActReplyInvalid),
            refusal_reason: err_opt,
            notes_present: env_notes_present,
            is_wrong_part: false,
        };
    };

    ParsedActReply {
        submission: EvalActSubmission {
            market_actions: env.market_actions,
            pass_b_actions: Vec::new(),
            bundle: env.bundle,
        },
        refusal_code: None,
        refusal_reason: None,
        notes_present: env.notes_present,
        is_wrong_part: false,
    }
}

fn validate_silent_raw_forbidden_keys(obj: &Map<String, Value>) -> Option<String> {
    let k_ma = preview_wording_slot("key_market_actions");
    let k_bundle = preview_wording_slot("key_bundle");
    let k_escrow_finishes = preview_wording_slot("key_escrow_finishes");
    let k_preimage = preview_wording_slot("key_preimage");
    let forbidden_ma_keys = [
        preview_wording_slot("key_condition_hash"),
        preview_wording_slot("key_pledge_id"),
        preview_wording_slot("key_recipient_seat"),
        preview_wording_slot("key_target"),
        preview_wording_slot("key_honor_weight"),
        preview_wording_slot("key_memo_digest"),
    ];
    if let Some(ma_arr) = obj.get(&k_ma).and_then(Value::as_array) {
        for (idx, item) in ma_arr.iter().enumerate() {
            if let Some(m) = item.as_object() {
                for fk in &forbidden_ma_keys {
                    if m.contains_key(fk.as_str()) {
                        return Some(format!(
                            "silent condition forbids '{fk}' inside '{k_ma}[{idx}]'"
                        ));
                    }
                }
            }
        }
    }
    if let Some(ef_arr) = obj
        .get(&k_bundle)
        .and_then(|v| v.get(&k_escrow_finishes))
        .and_then(Value::as_array)
    {
        for (idx, item) in ef_arr.iter().enumerate() {
            if item
                .as_object()
                .is_some_and(|m| m.contains_key(&k_preimage))
            {
                return Some(format!(
                    "silent condition forbids '{k_preimage}' inside '{k_bundle}.{k_escrow_finishes}[{idx}]'"
                ));
            }
        }
    }
    None
}

fn validate_silent_orders_no_communication(
    market_actions: &[MarketAction],
    bundle: &SealedOrderBundle,
) -> Option<String> {
    for (idx, act) in market_actions.iter().enumerate() {
        match act {
            MarketAction::RecordIntentPledge { .. } => {
                return Some(format!(
                    "silent condition forbids record_intent_pledge at market_actions[{idx}]"
                ));
            }
            MarketAction::CreateEscrow { escrow, .. } if escrow.condition_hash.is_some() => {
                return Some(format!(
                    "silent condition forbids condition_hash in create_escrow at market_actions[{idx}]"
                ));
            }
            _ => {}
        }
    }
    for (idx, fin) in bundle.escrow_finishes.iter().enumerate() {
        if fin.preimage.is_some() {
            return Some(format!(
                "silent condition forbids preimage in escrow_finishes[{idx}]"
            ));
        }
    }
    None
}

/// Strictly parse a delivered `act` reply in the silent-market condition.
///
/// Rejects any `"messages"` property (`ActReplyHasMessages` when non-empty, `ActReplyInvalid` when
/// empty), `record_intent_pledge` / memo keys, `condition_hash`, and `preimage`.
#[must_use]
pub fn parse_eval_silent_act_reply(
    raw: &str,
    asking_seat: u8,
    keeps_notes: bool,
) -> ParsedActReply {
    let obj = match parse_strict_json_object(raw) {
        Ok(o) => o,
        Err(reason) => {
            return ParsedActReply {
                submission: EvalActSubmission::default(),
                refusal_code: Some(ReplyRefusalCode::ActReplyInvalid),
                refusal_reason: Some(reason),
                notes_present: false,
                is_wrong_part: false,
            };
        }
    };
    let k_notes = wording_slot("scratchpad_schema_key");
    let notes_present = keeps_notes && obj.get(&k_notes).is_some_and(Value::is_string);
    if let Some((code, reason)) = detect_raw_wrong_round_obj(Round::Act, &obj) {
        return ParsedActReply {
            submission: EvalActSubmission::default(),
            refusal_code: Some(code),
            refusal_reason: Some(reason),
            notes_present,
            is_wrong_part: true,
        };
    }

    let (env_opt, err_opt, env_notes_present) =
        validate_reply_envelope_obj(&obj, asking_seat, keeps_notes, false);
    let Some(env) = env_opt else {
        return ParsedActReply {
            submission: EvalActSubmission::default(),
            refusal_code: Some(ReplyRefusalCode::ActReplyInvalid),
            refusal_reason: err_opt,
            notes_present: env_notes_present,
            is_wrong_part: false,
        };
    };

    if let Some(reason) = validate_silent_raw_forbidden_keys(&obj)
        .or_else(|| validate_silent_orders_no_communication(&env.market_actions, &env.bundle))
    {
        return ParsedActReply {
            submission: EvalActSubmission::default(),
            refusal_code: Some(ReplyRefusalCode::ActReplyInvalid),
            refusal_reason: Some(reason),
            notes_present: env.notes_present,
            is_wrong_part: false,
        };
    }

    ParsedActReply {
        submission: EvalActSubmission {
            market_actions: env.market_actions,
            pass_b_actions: Vec::new(),
            bundle: env.bundle,
        },
        refusal_code: None,
        refusal_reason: None,
        notes_present: env.notes_present,
        is_wrong_part: false,
    }
}

/// Dispatch `act` reply parsing according to [`CommunicationMode`].
#[must_use]
pub fn parse_eval_act_reply_for_communication(
    communication: CommunicationMode,
    raw: &str,
    asking_seat: u8,
    keeps_notes: bool,
) -> ParsedActReply {
    match communication {
        CommunicationMode::Talk => parse_eval_act_reply(raw, asking_seat, keeps_notes),
        CommunicationMode::Silent => parse_eval_silent_act_reply(raw, asking_seat, keeps_notes),
    }
}

/// Strictly parse a delivered `talk` reply (`from-seat/day-DD-talk-<Guild>.md`, `1.3`–`1.5`, `B1`).
///
/// Wrong-round detection (`TALK_REPLY_HAS_ORDERS`) runs on the raw JSON object before order or
/// message validation (`B1`). Refuses the entire `talk` reply when `market_actions` or `bundle`
/// is non-empty (`TALK_REPLY_HAS_ORDERS`, `0` messages sent), or when JSON/schema validation fails
/// with no wrong-round orders (`TALK_REPLY_INVALID`, `0` messages sent). When one message addresses
/// an unknown guild or the sender itself, records `MESSAGE_RECIPIENT_INVALID` for that message alone
/// and delivers the seat's other valid messages.
#[must_use]
pub fn parse_eval_talk_reply(
    raw: &str,
    sender_seat: u8,
    sender_guild: &str,
    seated_guilds: &[String],
    keeps_notes: bool,
) -> ParsedTalkReply {
    let obj = match parse_strict_json_object(raw) {
        Ok(o) => o,
        Err(reason) => {
            return ParsedTalkReply {
                accepted_whispers: Vec::new(),
                refusal_code: Some(ReplyRefusalCode::TalkReplyInvalid),
                invalid_recipients: Vec::new(),
                refusal_reason: Some(reason),
                notes_present: false,
                is_wrong_part: false,
            };
        }
    };
    let k_notes = wording_slot("scratchpad_schema_key");
    let notes_present = keeps_notes && obj.get(&k_notes).is_some_and(Value::is_string);
    let k_ma = preview_wording_slot("key_market_actions");
    let k_bundle = preview_wording_slot("key_bundle");
    if let Some((code, reason)) = detect_raw_wrong_round_obj(Round::Talk, &obj) {
        return ParsedTalkReply {
            accepted_whispers: Vec::new(),
            refusal_code: Some(code),
            invalid_recipients: Vec::new(),
            refusal_reason: Some(reason),
            notes_present,
            is_wrong_part: true,
        };
    }

    let (env_opt, err_opt, env_notes_present) =
        validate_common_reply_envelope_obj(&obj, sender_seat, keeps_notes);
    let Some(env) = env_opt else {
        return ParsedTalkReply {
            accepted_whispers: Vec::new(),
            refusal_code: Some(ReplyRefusalCode::TalkReplyInvalid),
            invalid_recipients: Vec::new(),
            refusal_reason: err_opt,
            notes_present: env_notes_present,
            is_wrong_part: false,
        };
    };

    if !env.market_actions.is_empty() || env.bundle != SealedOrderBundle::default() {
        return ParsedTalkReply {
            accepted_whispers: Vec::new(),
            refusal_code: Some(ReplyRefusalCode::TalkReplyHasOrders),
            invalid_recipients: Vec::new(),
            refusal_reason: Some(format!(
                "talk reply must leave '{k_ma}' and '{k_bundle}' empty"
            )),
            notes_present: env.notes_present,
            is_wrong_part: true,
        };
    }

    let mut accepted_whispers = Vec::with_capacity(env.messages.len());
    let mut invalid_recipients = Vec::new();
    for (idx, msg) in env.messages.into_iter().enumerate() {
        if msg.to_guild == sender_guild {
            invalid_recipients.push(InvalidRecipientDiagnostic {
                message_index: idx,
                to_guild: msg.to_guild,
                code: ReplyRefusalCode::MessageRecipientInvalid,
                reason: "message cannot be addressed to the sender itself".to_owned(),
            });
            continue;
        }
        if !seated_guilds.iter().any(|g| g == &msg.to_guild) {
            invalid_recipients.push(InvalidRecipientDiagnostic {
                message_index: idx,
                to_guild: msg.to_guild,
                code: ReplyRefusalCode::MessageRecipientInvalid,
                reason: "message addressed to unknown guild not seated at this table".to_owned(),
            });
            continue;
        }
        accepted_whispers.push((
            msg.to_guild,
            UntrustedWhisper {
                from_guild: sender_guild.to_owned(),
                intent_header: None,
                body: msg.body,
            },
        ));
    }

    ParsedTalkReply {
        accepted_whispers,
        refusal_code: None,
        invalid_recipients,
        refusal_reason: None,
        notes_present: env.notes_present,
        is_wrong_part: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_unique_visitor_and_error_branches_are_covered() {
        // Exercise bool, negative i64, u64, finite f64, null (`visit_unit`), and nested duplicate keys.
        let scalar_json = r#"{"a":true,"b":-5,"c":7,"d":1.25,"e":null,"f":[false]}"#;
        let mut de = serde_json::Deserializer::from_str(scalar_json);
        assert!(StrictUniqueValue.deserialize(&mut de).is_ok());

        let nested_dup =
            r#"{"messages":[],"market_actions":[],"bundle":{"x":1,"nested":{"k":1,"k":2}}}"#;
        assert_eq!(
            parse_eval_act_reply(nested_dup, 0, false).refusal_code,
            Some(ReplyRefusalCode::ActReplyInvalid)
        );
        assert_eq!(
            parse_eval_talk_reply("<think>x</think>{}", 0, "Foundry", &[], false).refusal_code,
            Some(ReplyRefusalCode::TalkReplyInvalid)
        );

        let seated = vec![
            "Foundry".to_owned(),
            "Quarry".to_owned(),
            "Harbor".to_owned(),
            "Mint".to_owned(),
        ];
        // Pure malformed envelopes with no wrong-round content (`ACT_REPLY_INVALID` vs `TALK_REPLY_INVALID`):
        for bad in [
            r#"{bad"#,
            r#"{"messages":"not-arr","market_actions":[],"bundle":{}}"#,
            r#"{"messages":[],"market_actions":"not-arr","bundle":{}}"#,
            r#"{"messages":[],"market_actions":[],"bundle":[]}"#,
        ] {
            let act = parse_eval_act_reply(bad, 0, false);
            assert_eq!(act.refusal_code, Some(ReplyRefusalCode::ActReplyInvalid));
            assert!(!act.is_wrong_part);
            let talk = parse_eval_talk_reply(bad, 0, "Foundry", &seated, false);
            assert_eq!(talk.refusal_code, Some(ReplyRefusalCode::TalkReplyInvalid));
            assert!(!talk.is_wrong_part);
        }

        // Malformed message items in `talk` -> `TALK_REPLY_INVALID`:
        for malformed_talk_msg in [
            r#"{"messages":[42],"market_actions":[],"bundle":{}}"#,
            r#"{"messages":[{"to_guild":"Quarry","body":"hi","extra":1}],"market_actions":[],"bundle":{}}"#,
            r#"{"messages":[{"body":"missing guild"}],"market_actions":[],"bundle":{}}"#,
            r#"{"messages":[{"to_guild":"Quarry"}],"market_actions":[],"bundle":{}}"#,
        ] {
            let talk = parse_eval_talk_reply(malformed_talk_msg, 0, "Foundry", &seated, false);
            assert_eq!(talk.refusal_code, Some(ReplyRefusalCode::TalkReplyInvalid));
            assert!(!talk.is_wrong_part);
        }

        // B1 wrong-round priority over malformed orders / messages:
        for wrong_talk in [
            r#"{"messages":[],"market_actions":[{"action_kind":"bogus"}],"bundle":{}}"#,
            r#"{"messages":[],"market_actions":[{"action_kind":"post_offer"}],"bundle":{}}"#,
            r#"{"messages":[],"market_actions":[],"bundle":{"unknown_field":1}}"#,
            r#"{"messages":[],"market_actions":[],"bundle":{"debt_repayment_cred":10}}"#,
            r#"{"messages":[],"market_actions":[],"bundle":{"converter_runs":["bad"]}}"#,
        ] {
            let res = parse_eval_talk_reply(wrong_talk, 0, "Foundry", &seated, false);
            assert_eq!(res.refusal_code, Some(ReplyRefusalCode::TalkReplyHasOrders));
            assert!(res.is_wrong_part);
            assert!(res.accepted_whispers.is_empty());
        }
        for wrong_act in [
            r#"{"messages":[42],"market_actions":[],"bundle":{}}"#,
            r#"{"messages":[{"to_guild":"Quarry"}],"market_actions":[],"bundle":{}}"#,
            r#"{"messages":[{"to_guild":"Quarry","body":"hi","extra":1}],"market_actions":[],"bundle":{}}"#,
        ] {
            let res = parse_eval_act_reply(wrong_act, 0, false);
            assert_eq!(
                res.refusal_code,
                Some(ReplyRefusalCode::ActReplyHasMessages)
            );
            assert!(res.is_wrong_part);
            assert_eq!(res.submission, EvalActSubmission::default());
        }

        // Explicit all-empty r2 bundle fields in talk pass as non-wrong-round:
        let ok_talk = parse_eval_talk_reply(
            r#"{"messages":[],"market_actions":[],"bundle":{"converter_runs":[],"project_deliveries":[],"atomic_swaps":[],"contract_deliveries":[],"dual_escrows":[],"debt_repayment_cred":0,"escrow_finishes":[],"check_cashes":[],"council_vote":null,"charter_bids":[]}}"#,
            0,
            "Foundry",
            &seated,
            false,
        );
        assert_eq!(ok_talk.refusal_code, None);

        // Talk exceeding EVAL_MESSAGES_MAX_ITEMS and non-object message item -> `TALK_REPLY_INVALID`:
        let too_many_msgs = r#"{"messages":[{"to_guild":"Quarry","body":"1"},{"to_guild":"Harbor","body":"2"},{"to_guild":"Mint","body":"3"},{"to_guild":"Quarry","body":"4"}],"market_actions":[],"bundle":{}}"#;
        let err_many = parse_eval_talk_reply(too_many_msgs, 0, "Foundry", &seated, false);
        assert_eq!(
            err_many.refusal_code,
            Some(ReplyRefusalCode::TalkReplyInvalid)
        );

        // Silent mode forbids raw `condition_hash` and `preimage` keys as well as direct order helper checks:
        let strip_messages_from_talk = |sub: &EvalActSubmission| -> String {
            let Ok(raw) = crate::baselines::encode_canonical_eval_reply(
                crate::records::CommunicationMode::Talk,
                &[],
                sub,
            ) else {
                panic!("encode talk reply for silent forbidden key test");
            };
            let Ok(mut map) = serde_json::from_str::<Map<String, Value>>(&raw) else {
                panic!("parse encoded talk map");
            };
            map.remove("messages");
            serde_json::to_string(&map).unwrap_or_default()
        };
        let silent_cond_hash = strip_messages_from_talk(&EvalActSubmission {
            market_actions: vec![MarketAction::CreateEscrow {
                seat: 0,
                escrow: league_seat_types::EscrowProposal {
                    destination: 1,
                    asset: league_core::AssetId::Cred,
                    amount: 1000,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 4,
                    condition_hash: Some(league_seat_types::CommitmentHash([0x11; 32])),
                    bond: None,
                },
            }],
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle::default(),
        });
        let err_ch = parse_eval_silent_act_reply(&silent_cond_hash, 0, false);
        assert_eq!(err_ch.refusal_code, Some(ReplyRefusalCode::ActReplyInvalid));
        assert!(
            err_ch
                .refusal_reason
                .as_deref()
                .is_some_and(|r| r.contains("condition_hash")),
            "{:?}",
            err_ch.refusal_reason
        );

        let silent_preimage = strip_messages_from_talk(&EvalActSubmission {
            market_actions: Vec::new(),
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                escrow_finishes: vec![league_seat_types::EscrowFinishUnlock {
                    escrow_id: 1,
                    preimage: Some(league_seat_types::PreimageSecret([0x22; 32])),
                }],
                ..SealedOrderBundle::default()
            },
        });
        let err_pre = parse_eval_silent_act_reply(&silent_preimage, 0, false);
        assert_eq!(
            err_pre.refusal_code,
            Some(ReplyRefusalCode::ActReplyInvalid)
        );
        assert!(
            err_pre
                .refusal_reason
                .as_deref()
                .is_some_and(|r| r.contains("preimage")),
            "{:?}",
            err_pre.refusal_reason
        );

        assert!(
            validate_silent_orders_no_communication(&[], &SealedOrderBundle::default()).is_none()
        );
        assert!(
            validate_silent_orders_no_communication(
                &[MarketAction::RecordIntentPledge {
                    seat: 0,
                    pledge: league_seat_types::IntentPledge {
                        pledge_id: 1,
                        promisor: 0,
                        counterparty: 1,
                        target: league_seat_types::IntentTarget::VoteForSeat {
                            office_turn: 4,
                            candidate: 1,
                        },
                        honor_weight: 10,
                        memo_digest: [0x11; 32],
                    },
                }],
                &SealedOrderBundle::default(),
            )
            .is_some_and(|s| s.contains("record_intent_pledge"))
        );
        assert!(
            validate_silent_orders_no_communication(
                &[MarketAction::CreateEscrow {
                    seat: 0,
                    escrow: league_seat_types::EscrowProposal {
                        destination: 1,
                        asset: league_core::AssetId::Cred,
                        amount: 1000,
                        finish_after_turn: Some(2),
                        cancel_after_turn: 4,
                        condition_hash: Some(league_seat_types::CommitmentHash([0x22; 32])),
                        bond: None,
                    },
                }],
                &SealedOrderBundle::default(),
            )
            .is_some_and(|s| s.contains("condition_hash"))
        );
        assert!(
            validate_silent_orders_no_communication(
                &[],
                &SealedOrderBundle {
                    escrow_finishes: vec![league_seat_types::EscrowFinishUnlock {
                        escrow_id: 1,
                        preimage: Some(league_seat_types::PreimageSecret([0x33; 32])),
                    }],
                    ..SealedOrderBundle::default()
                },
            )
            .is_some_and(|s| s.contains("preimage"))
        );
    }
}
