//! Evaluation Mode host ownership boundary (`EvalHost`).
//!
//! Only [`host`] inside this module owns [`crate::TurnEngine`] for Evaluation Mode, calls
//! `TurnEngine::state()`, executes sweeps, serializes/deserializes `TurnEngine` inside `.seal`,
//! and performs atomic `.save.json` / `.seal` and checkpoint persistence. All seat-side modules
//! consume solely [`crate::SeatObservation`], [`crate::PublicSweepReceipt`], and seat-safe
//! previews.

pub mod host;

pub use host::{
    EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EvalActApplyOutcome, EvalActSubmission,
    EvalHost, EvalSaveSnapshot, EvalSeatApplyReport,
};
