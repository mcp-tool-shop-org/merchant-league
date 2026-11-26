//! Pinned model-facing wording entries for Evaluation Mode (`v2_eval_reply_2026_11_21_r1`) and
//! pinned wrapper verification.
//!
//! Every slot name in [`EVAL_REPLY_WORDING`] is an internal identifier and never shown to a seated
//! model. Shown strings are on single lines with single spaces.

use league_relay::{WordingEntry, sha256_hex};

use crate::error::EvalError;
use crate::records::CommunicationMode;

/// Named registered Evaluation Mode reply wording arm.
pub const EVAL_REPLY_WORDING_ARM: &str = "v2_eval_reply_2026_11_21_r1";

/// Named registered Evaluation Mode silent-market reply wording arm.
pub const EVAL_SILENT_REPLY_WORDING_ARM: &str = "v2_eval_silent_reply_2026_11_26_r1";

/// Pinned Evaluation Mode wrapper allow-list `(version, sha256)` — strictly one entry (`r3`).
pub const PINNED_WRAPPERS: &[(&str, &str)] = &[(
    "v2_seat_brief_2026_11_21_r3",
    "66d5665950b2ef9bad177dbcfbe132912fa99f09c328a40866502dbf16d6a06d",
)];

/// Pinned Evaluation Mode silent-market wrapper allow-list `(version, sha256)`.
///
/// Derived from `v2_seat_brief_2026_11_21_r3.md` via deletions of disabled communication channels,
/// one added sentence under `## Each turn` (`"There is no talk round, and no messages are sent
/// between seats."`), and one minimal escrow-rule conjunction fix retaining `r3`'s `"or"` when
/// removing `` or `condition_hash` `` (`"Needs `bond` or `finish_after_turn`;"`).
pub const PINNED_SILENT_WRAPPERS: &[(&str, &str)] = &[(
    "v2_seat_brief_silent_2026_11_26_r1",
    "d83afe6c02691beb2c1dfac0ce98956f57989bbcb6ea5d84934302f62ca9c7e3",
)];

/// Superseded historical wrapper hashes that must never be accepted.
pub const REJECTED_WRAPPER_HASHES: &[(&str, &str)] = &[
    (
        "v2_seat_brief_2026_11_17_r1",
        "53124129dc2bf06dc97c624ab2509e062b5882b058a9d56502fc28f729975f4b",
    ),
    (
        "v2_seat_brief_2026_11_20_r2",
        "1fecf981f1df39dfc552cef90c24430aff4faa0f8990d77cfafee62a7ef74268",
    ),
];

/// Check whether a lowercase hex SHA-256 wrapper `digest` is allowed by [`PINNED_WRAPPERS`] and
/// agrees with any supplied `explicit_version`, returning the canonical pinned wrapper version.
pub fn check_wrapper_digest(
    digest: &str,
    explicit_version: Option<&str>,
) -> Result<String, EvalError> {
    if let Some((old_ver, _)) = REJECTED_WRAPPER_HASHES
        .iter()
        .find(|(_, hash)| *hash == digest)
    {
        return Err(EvalError::UnpinnedWrapper(format!(
            "refused superseded wrapper '{old_ver}' (sha256 {digest}); expected '{}' ({})",
            PINNED_WRAPPERS[0].0, PINNED_WRAPPERS[0].1
        )));
    }
    let Some((pinned_version, _)) = PINNED_WRAPPERS.iter().find(|(_, hash)| *hash == digest) else {
        return Err(EvalError::UnpinnedWrapper(format!(
            "wrapper sha256 '{digest}' is not in PINNED_WRAPPERS; expected '{}' ({})",
            PINNED_WRAPPERS[0].0, PINNED_WRAPPERS[0].1
        )));
    };
    if let Some(v) = explicit_version {
        let trimmed = v.trim();
        if !trimmed.is_empty() && trimmed != *pinned_version {
            return Err(EvalError::UnpinnedWrapper(format!(
                "--wrapper-version '{trimmed}' does not match pinned wrapper version '{pinned_version}'"
            )));
        }
    }
    Ok((*pinned_version).to_owned())
}

/// Validate wrapper bytes against [`PINNED_WRAPPERS`] and verify any explicit `--wrapper-version`.
pub fn resolve_pinned_wrapper(
    raw_bytes: &[u8],
    explicit_version: Option<&str>,
) -> Result<(String, String), EvalError> {
    resolve_pinned_wrapper_for_communication(raw_bytes, explicit_version, CommunicationMode::Talk)
}

/// Validate wrapper bytes against the allow-list matching `communication` and verify any
/// explicit `--wrapper-version`.
pub fn resolve_pinned_wrapper_for_communication(
    raw_bytes: &[u8],
    explicit_version: Option<&str>,
    communication: CommunicationMode,
) -> Result<(String, String), EvalError> {
    let digest = sha256_hex(raw_bytes);
    let version = match communication {
        CommunicationMode::Talk => check_wrapper_digest(&digest, explicit_version)?,
        CommunicationMode::Silent => check_digest_in_allowlist(
            &digest,
            explicit_version,
            PINNED_SILENT_WRAPPERS,
            "PINNED_SILENT_WRAPPERS",
        )?,
    };
    Ok((version, digest))
}

fn check_digest_in_allowlist(
    digest: &str,
    explicit_version: Option<&str>,
    allowlist: &[(&str, &str)],
    list_name: &str,
) -> Result<String, EvalError> {
    let Some((pinned_version, _)) = allowlist.iter().find(|(_, hash)| *hash == digest) else {
        return Err(EvalError::UnpinnedWrapper(format!(
            "wrapper sha256 '{digest}' is not in {list_name}; expected '{}' ({})",
            allowlist[0].0, allowlist[0].1
        )));
    };
    if let Some(v) = explicit_version {
        let trimmed = v.trim();
        if !trimmed.is_empty() && trimmed != *pinned_version {
            return Err(EvalError::UnpinnedWrapper(format!(
                "--wrapper-version '{trimmed}' does not match pinned wrapper version '{pinned_version}'"
            )));
        }
    }
    Ok((*pinned_version).to_owned())
}

/// Pinned `maxItems` limit for the top-level `messages` array in talk and act replies.
pub const EVAL_MESSAGES_MAX_ITEMS: usize = 3;

/// Complete table of model-facing strings for Evaluation Mode structured replies.
///
/// Order-side (`market_actions` and `bundle`) keys, descriptions, and enums continue to come
/// solely from [`league_relay::PREVIEW_ARM_WORDING`] (`r2`).
pub const EVAL_REPLY_WORDING: &[WordingEntry] = &[
    WordingEntry {
        slot: "eval_messages_key",
        shown_text: "messages",
    },
    WordingEntry {
        slot: "eval_messages_description",
        shown_text: "Messages you send to other seats in the talk round, at most 3. In the act round, leave this list empty.",
    },
    WordingEntry {
        slot: "eval_messages_max_items",
        shown_text: "3",
    },
    WordingEntry {
        slot: "eval_to_guild_key",
        shown_text: "to_guild",
    },
    WordingEntry {
        slot: "eval_to_guild_description",
        shown_text: "The guild of the seat that receives this message. It must be another seat at this table.",
    },
    WordingEntry {
        slot: "eval_body_key",
        shown_text: "body",
    },
    WordingEntry {
        slot: "eval_body_description",
        shown_text: "The text of your message, at most 120 words and 1,800 characters. Longer text is cut short, and the receiver is told it was cut.",
    },
    WordingEntry {
        slot: "eval_talk_instruction",
        shown_text: "This is the talk round. You may send up to 3 messages to other seats in `messages`. Each message reaches only the seat you address, at the start of the act round. Leave `market_actions` and `bundle` empty: a reply that contains orders in this round is refused, and its messages are not sent.",
    },
    WordingEntry {
        slot: "eval_act_instruction",
        shown_text: "This is the act round. Put your orders for this turn in `market_actions` and `bundle`. Leave `messages` empty: a reply that contains messages in this round is refused, and its orders are not applied.",
    },
    WordingEntry {
        slot: "eval_state_intro",
        shown_text: "The game state you can see is below, as JSON.",
    },
    WordingEntry {
        slot: "eval_batna",
        shown_text: "Your own orders and the commitments you already hold are still applied under the rules, as is everything the rules apply every turn.",
    },
];

/// Look up the pinned shown text for an internal [`EVAL_REPLY_WORDING`] slot.
#[must_use]
pub fn eval_wording_slot(slot: &str) -> &'static str {
    for entry in EVAL_REPLY_WORDING {
        if entry.slot == slot {
            return entry.shown_text;
        }
    }
    unreachable!("unknown EVAL_REPLY_WORDING slot: {slot}")
}

/// Return all internal slot names in [`EVAL_REPLY_WORDING`] for prompt-leak tests.
#[must_use]
pub fn eval_internal_slot_names() -> Vec<&'static str> {
    EVAL_REPLY_WORDING.iter().map(|e| e.slot).collect()
}

/// Complete table of model-facing strings for the silent-market condition (`v2_eval_silent_reply_2026_11_26_r1`).
///
/// Constructed from [`EVAL_REPLY_WORDING`] with talk and `messages` slots/sentences deleted and no new phrasing.
pub const EVAL_SILENT_REPLY_WORDING: &[WordingEntry] = &[
    WordingEntry {
        slot: "eval_silent_act_instruction",
        shown_text: "This is the act round. Put your orders for this turn in `market_actions` and `bundle`.",
    },
    WordingEntry {
        slot: "eval_state_intro",
        shown_text: "The game state you can see is below, as JSON.",
    },
];

/// Look up the pinned shown text for an internal [`EVAL_SILENT_REPLY_WORDING`] slot.
#[must_use]
pub fn eval_silent_wording_slot(slot: &str) -> &'static str {
    for entry in EVAL_SILENT_REPLY_WORDING {
        if entry.slot == slot {
            return entry.shown_text;
        }
    }
    unreachable!("unknown EVAL_SILENT_REPLY_WORDING slot: {slot}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_wording_table_and_internal_slot_names_resolve_cleanly() {
        assert_eq!(eval_wording_slot("eval_messages_key"), "messages");
        assert_eq!(
            eval_wording_slot("eval_messages_max_items"),
            EVAL_MESSAGES_MAX_ITEMS.to_string()
        );
        assert_eq!(eval_internal_slot_names().len(), EVAL_REPLY_WORDING.len());
    }

    #[test]
    #[should_panic(expected = "unknown EVAL_REPLY_WORDING slot")]
    fn unknown_wording_slot_panics() {
        let _ = eval_wording_slot("does_not_exist");
    }

    #[test]
    fn silent_wording_and_wrapper_resolution_enforce_pins() {
        assert_eq!(
            eval_silent_wording_slot("eval_state_intro"),
            eval_wording_slot("eval_state_intro")
        );
        assert!(
            eval_silent_wording_slot("eval_silent_act_instruction")
                .contains("This is the act round.")
        );
        let silent_entry = &PINNED_SILENT_WRAPPERS[0];
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("wrappers/v2_seat_brief_silent_2026_11_26_r1.md");
        let Ok(bytes) = std::fs::read(&path) else {
            panic!("read silent wrapper");
        };
        let Ok((ver, sha)) =
            resolve_pinned_wrapper_for_communication(&bytes, None, CommunicationMode::Silent)
        else {
            panic!("resolve silent wrapper");
        };
        assert_eq!(ver, silent_entry.0);
        assert_eq!(sha, silent_entry.1);

        let err_bad_ver = resolve_pinned_wrapper_for_communication(
            &bytes,
            Some("v2_seat_brief_2026_11_21_r3"),
            CommunicationMode::Silent,
        );
        assert!(err_bad_ver.is_err());
        let Ok((ver_ws, _)) = resolve_pinned_wrapper_for_communication(
            &bytes,
            Some("   "),
            CommunicationMode::Silent,
        ) else {
            panic!("empty/whitespace explicit version should use pinned digest match");
        };
        assert_eq!(ver_ws, silent_entry.0);
        let err_wrong_digest =
            resolve_pinned_wrapper_for_communication(b"tampered", None, CommunicationMode::Silent);
        assert!(err_wrong_digest.is_err());
    }
}
