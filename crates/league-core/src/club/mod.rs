//! Offline Club Mode host ownership boundary (`ClubHost`).
//!
//! Only [`host`] inside this module owns [`crate::TurnEngine`] for Club Mode, calls
//! `TurnEngine::state()`, executes sweeps, serializes/deserializes `TurnEngine` inside `.seal`,
//! and performs atomic `.save.json` / `.seal` persistence. All seat-side modules and UI screens
//! consume solely [`crate::SeatObservation`], [`crate::PublicSweepReceipt`], and seat-safe
//! previews.

pub mod host;

pub use host::{
    CLUB_SAVE_FORMAT_VERSION, CLUB_SEAL_FORMAT_VERSION, ClubHost, ClubSaveSnapshot,
    DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE,
};
