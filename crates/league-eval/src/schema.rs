//! Unified game-wide structured output JSON Schema for Evaluation Mode (`Item 9` addendum 1.2).
//!
//! Constructed in code from:
//! 1. The exact `market_actions` and `bundle` parameter subtrees built by
//!    [`league_relay::build_preview_turn_orders_tool_definition`] (`r2`);
//! 2. The pinned Section 2 entries in [`crate::wording::EVAL_REPLY_WORDING`] (`messages`,
//!    `to_guild`, `body`, `maxItems = 3`);
//! 3. `league-relay`'s existing pinned `"notes"` field (`scratchpad_schema_key`).
//!
//! Passed to [`ResponseFormatConfig::from_json_str`] so `relay/setup.json` locks its digest and
//! `league-relay` handles note-keeping vs non-note-keeping seats via
//! `without_scratchpad_property()`.

use league_relay::{
    ResponseFormatConfig, build_preview_turn_orders_tool_definition, preview_wording_slot,
    wording_slot,
};

use crate::error::EvalError;
use crate::wording::{EVAL_MESSAGES_MAX_ITEMS, eval_wording_slot};

/// Build the raw `response_format` JSON value before passing to `ResponseFormatConfig::from_json_str`.
pub fn build_eval_response_format_value() -> Result<serde_json::Value, EvalError> {
    let tool_def = build_preview_turn_orders_tool_definition();
    let fn_key = preview_wording_slot("schema_kw_function");
    let params_key = preview_wording_slot("schema_kw_parameters");
    let props_key = preview_wording_slot("schema_kw_properties");
    let req_key = preview_wording_slot("schema_kw_required");
    let add_props_key = preview_wording_slot("schema_kw_additional_properties");
    let type_key = preview_wording_slot("schema_kw_type");
    let desc_key = preview_wording_slot("schema_kw_description");
    let items_key = preview_wording_slot("schema_kw_items");

    let ty_obj = preview_wording_slot("schema_ty_object");
    let ty_arr = preview_wording_slot("schema_ty_array");
    let ty_str = preview_wording_slot("schema_ty_string");

    let tool_props = tool_def
        .get(&fn_key)
        .and_then(|v| v.get(&params_key))
        .and_then(|v| v.get(&props_key))
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            EvalError::SchemaValidation(
                "preview tool definition missing function.parameters.properties".to_owned(),
            )
        })?;

    let k_market_actions = preview_wording_slot("key_market_actions");
    let k_bundle = preview_wording_slot("key_bundle");
    let market_actions_subtree = extract_tool_subtree(tool_props, &k_market_actions)?;
    let bundle_subtree = extract_tool_subtree(tool_props, &k_bundle)?;

    let k_messages = eval_wording_slot("eval_messages_key");
    let k_to_guild = eval_wording_slot("eval_to_guild_key");
    let k_body = eval_wording_slot("eval_body_key");
    let k_notes = wording_slot("scratchpad_schema_key");
    let notes_desc = wording_slot("scratchpad_schema_description");

    let message_item = serde_json::json!({
        type_key.as_str(): ty_obj.as_str(),
        props_key.as_str(): {
            k_to_guild: {
                type_key.as_str(): ty_str.as_str(),
                desc_key.as_str(): eval_wording_slot("eval_to_guild_description"),
            },
            k_body: {
                type_key.as_str(): ty_str.as_str(),
                desc_key.as_str(): eval_wording_slot("eval_body_description"),
            }
        },
        req_key.as_str(): [k_to_guild, k_body],
        add_props_key.as_str(): false,
    });

    let max_items_slot = eval_wording_slot("eval_messages_max_items");
    let max_items_parsed = max_items_slot.parse::<usize>().map_err(|e| {
        EvalError::SchemaValidation(format!(
            "eval_messages_max_items slot '{max_items_slot}' is not a valid integer: {e}"
        ))
    })?;
    if max_items_parsed != EVAL_MESSAGES_MAX_ITEMS {
        return Err(EvalError::SchemaValidation(format!(
            "eval_messages_max_items slot ({max_items_parsed}) diverges from EVAL_MESSAGES_MAX_ITEMS ({EVAL_MESSAGES_MAX_ITEMS})"
        )));
    }

    let messages_subtree = serde_json::json!({
        type_key.as_str(): ty_arr.as_str(),
        desc_key.as_str(): eval_wording_slot("eval_messages_description"),
        "maxItems": max_items_parsed,
        items_key.as_str(): message_item,
    });

    let notes_subtree = serde_json::json!({
        type_key.as_str(): ty_str.as_str(),
        desc_key.as_str(): notes_desc,
    });

    Ok(serde_json::json!({
        type_key.as_str(): ty_obj.as_str(),
        props_key.as_str(): {
            k_messages: messages_subtree,
            k_market_actions.clone(): market_actions_subtree,
            k_bundle.clone(): bundle_subtree,
            k_notes.clone(): notes_subtree,
        },
        req_key.as_str(): [k_messages, k_market_actions, k_bundle, k_notes],
        add_props_key.as_str(): false,
    }))
}

/// Construct and validate the unified Evaluation Mode [`ResponseFormatConfig`] through
/// `league-relay`'s own setup validator (`ResponseFormatConfig::from_json_str`).
pub fn build_eval_response_format_config() -> Result<ResponseFormatConfig, EvalError> {
    let val = build_eval_response_format_value()?;
    let raw = serde_json::to_string(&val)
        .map_err(|e| EvalError::SchemaValidation(format!("serialize schema JSON: {e}")))?;
    ResponseFormatConfig::from_json_str(&raw, None)
        .map_err(|e| EvalError::SchemaValidation(e.to_string()))
}

/// Build the raw `response_format` JSON value for the silent-market condition by deleting
/// `messages`, `record_intent_pledge` / memo keys, and `condition_hash` / `preimage` keys from
/// [`build_eval_response_format_value`] with zero new phrasing.
pub fn build_eval_silent_response_format_value() -> Result<serde_json::Value, EvalError> {
    let mut val = build_eval_response_format_value()?;
    let props_key = preview_wording_slot("schema_kw_properties");
    let req_key = preview_wording_slot("schema_kw_required");
    let items_key = preview_wording_slot("schema_kw_items");
    let k_messages = eval_wording_slot("eval_messages_key");
    let k_ma = preview_wording_slot("key_market_actions");
    let k_bundle = preview_wording_slot("key_bundle");
    let k_action_kind = preview_wording_slot("key_action_kind");
    let v_pledge = preview_wording_slot("val_action_record_intent_pledge");
    let k_escrow_finishes = preview_wording_slot("key_escrow_finishes");
    let deleted_ma_props = [
        preview_wording_slot("key_condition_hash"),
        preview_wording_slot("key_pledge_id"),
        preview_wording_slot("key_recipient_seat"),
        preview_wording_slot("key_target"),
        preview_wording_slot("key_honor_weight"),
        preview_wording_slot("key_memo_digest"),
    ];
    let k_preimage = preview_wording_slot("key_preimage");

    let root = val.as_object_mut().ok_or_else(|| {
        EvalError::SchemaValidation("eval response schema root must be object".to_owned())
    })?;
    if let Some(props) = root.get_mut(&props_key).and_then(|v| v.as_object_mut()) {
        props.remove(k_messages);
        if let Some(ma_props) = props
            .get_mut(&k_ma)
            .and_then(|v| v.get_mut(&items_key))
            .and_then(|v| v.get_mut(&props_key))
            .and_then(|v| v.as_object_mut())
        {
            if let Some(kind_enum) = ma_props
                .get_mut(&k_action_kind)
                .and_then(|v| v.get_mut("enum"))
                .and_then(|v| v.as_array_mut())
            {
                kind_enum.retain(|item| item.as_str() != Some(v_pledge.as_str()));
            }
            for del_k in &deleted_ma_props {
                ma_props.remove(del_k.as_str());
            }
        }
        if let Some(ef_props) = props
            .get_mut(&k_bundle)
            .and_then(|v| v.get_mut(&props_key))
            .and_then(|v| v.get_mut(&k_escrow_finishes))
            .and_then(|v| v.get_mut(&items_key))
            .and_then(|v| v.get_mut(&props_key))
            .and_then(|v| v.as_object_mut())
        {
            ef_props.remove(&k_preimage);
        }
    }
    if let Some(req) = root.get_mut(&req_key).and_then(|v| v.as_array_mut()) {
        req.retain(|item| item.as_str() != Some(k_messages));
    }
    Ok(val)
}

/// Construct and validate the silent-market [`ResponseFormatConfig`] through `league-relay`.
pub fn build_eval_silent_response_format_config() -> Result<ResponseFormatConfig, EvalError> {
    let val = build_eval_silent_response_format_value()?;
    let raw = serde_json::to_string(&val)
        .map_err(|e| EvalError::SchemaValidation(format!("serialize silent schema JSON: {e}")))?;
    ResponseFormatConfig::from_json_str(&raw, None)
        .map_err(|e| EvalError::SchemaValidation(e.to_string()))
}

fn extract_tool_subtree(
    tool_props: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<serde_json::Value, EvalError> {
    tool_props.get(key).cloned().ok_or_else(|| {
        EvalError::SchemaValidation(format!("preview tool definition missing `{key}` subtree"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_validator_accepts_built_eval_schema_and_r2_subtrees_match_byte_for_byte() {
        let cfg = match build_eval_response_format_config() {
            Ok(cfg) => cfg,
            Err(err) => panic!("STOP_RULE: relay refused pinned Eval schema: {err}"),
        };
        assert_eq!(cfg.name, wording_slot("default_schema_name"));
        assert!(cfg.schema_has_scratchpad_property());

        let tool_def = build_preview_turn_orders_tool_definition();
        let params = &tool_def["function"]["parameters"]["properties"];
        let k_ma = preview_wording_slot("key_market_actions");
        let k_bundle = preview_wording_slot("key_bundle");
        assert_eq!(cfg.schema["properties"][&k_ma], params[&k_ma]);
        assert_eq!(cfg.schema["properties"][&k_bundle], params[&k_bundle]);

        let empty_map = serde_json::Map::new();
        assert!(matches!(
            extract_tool_subtree(&empty_map, "missing_key"),
            Err(EvalError::SchemaValidation(_))
        ));
    }
}
