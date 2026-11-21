//! Seat-view and negotiation file rendering from [`SeatObservation`] (`Item 9` addendum 1.5, 1.6).
//!
//! Serializer & indent (`1.6`): `serde_json::to_string_pretty` (2-space ASCII indentation, `\n`
//! line endings) using [`SeatObservation`]'s existing serde field names.

use league_core::SeatObservation;
use league_relay::{NegotiationInputs, Round, UntrustedWhisper};

use crate::error::EvalError;
use crate::wording::eval_wording_slot;

/// Deterministic pretty JSON serializer label recorded in tests and docs (`1.6`).
pub const EVAL_SEAT_VIEW_SERIALIZER_NAME: &str = "serde_json::to_string_pretty";

/// Indentation used by [`EVAL_SEAT_VIEW_SERIALIZER_NAME`].
pub const EVAL_SEAT_VIEW_JSON_INDENT_SPACES: usize = 2;

/// Render `<game-dir>/to-seat/day-DD-<round>-<Guild>.md` from a seat-safe [`SeatObservation`].
///
/// Format (`1.6`, exact order):
/// 1. the round's pinned instruction (`eval_talk_instruction` or `eval_act_instruction`);
/// 2. the pinned line before the state (`eval_state_intro`);
/// 3. [`SeatObservation`] serialized via `serde_json::to_string_pretty` (`2`-space indent).
pub fn render_seat_view_markdown(round: Round, obs: &SeatObservation) -> Result<String, EvalError> {
    let instruction = match round {
        Round::Talk => eval_wording_slot("eval_talk_instruction"),
        Round::Act => eval_wording_slot("eval_act_instruction"),
    };
    let intro = eval_wording_slot("eval_state_intro");
    let state_json = serde_json::to_string_pretty(obs)
        .map_err(|e| EvalError::Io(format!("failed to serialize SeatObservation: {e}")))?;
    Ok(format!("{instruction}\n\n{intro}\n\n{state_json}\n"))
}

/// Render `<game-dir>/to-seat/day-DD-act-<Guild>.preview-state.json` using the exact canonical
/// `PublicPreviewState` representation consumed by `league-relay --preview-tool on`.
pub fn render_preview_state_json(obs: &SeatObservation) -> Result<String, EvalError> {
    let preview_state = obs.to_public_preview_state();
    serde_json::to_string_pretty(&preview_state)
        .map(|s| format!("{s}\n"))
        .map_err(|e| EvalError::Io(format!("failed to serialize PublicPreviewState: {e}")))
}

/// Construct `<game-dir>/to-seat/day-DD-act-<Guild>.negotiation.json` for every seat on every
/// `act` round (`1.5`), using the pinned `eval_batna` sentence so every seat sees the same
/// prompt sections.
#[must_use]
pub fn build_act_negotiation_inputs(whispers: Vec<UntrustedWhisper>) -> NegotiationInputs {
    NegotiationInputs {
        batna: eval_wording_slot("eval_batna").to_owned(),
        whispers,
    }
}
