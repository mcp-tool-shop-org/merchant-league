//! Pinned model-facing wording entries for Evaluation Mode seat replies (`v2_eval_reply_2026_11_21_r1`).
//!
//! Every slot name in [`EVAL_REPLY_WORDING`] is an internal identifier and never shown to a seated
//! model. Shown strings are copied byte-for-byte from the Director's approved text (2026-11-21)
//! on single lines with single spaces.

use league_relay::WordingEntry;

/// Named registered Evaluation Mode reply wording arm (`Item 9` addendum, Section 2).
pub const EVAL_REPLY_WORDING_ARM: &str = "v2_eval_reply_2026_11_21_r1";

/// Pinned `maxItems` limit for the top-level `messages` array in talk and act replies.
pub const EVAL_MESSAGES_MAX_ITEMS: usize = 3;

/// Complete table of model-facing strings approved by the Director on `2026-11-21`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_wording_table_and_internal_slot_names_resolve_cleanly() {
        assert_eq!(eval_wording_slot("eval_messages_key"), "messages");
        assert_eq!(eval_internal_slot_names().len(), EVAL_REPLY_WORDING.len());
    }

    #[test]
    #[should_panic(expected = "unknown EVAL_REPLY_WORDING slot")]
    fn unknown_wording_slot_panics() {
        let _ = eval_wording_slot("does_not_exist");
    }
}
