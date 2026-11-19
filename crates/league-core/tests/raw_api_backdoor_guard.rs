//! Host/seat structural isolation & raw-API back-door guard (`league-core`, Wave 2 Cards 2b & 3).
//!
//! # Feature unification (`B2`)
//! CI runs `cargo clippy`, `cargo test`, and `cargo llvm-cov` with `--workspace`, where Cargo's
//! feature unification activates `league-core/engine-internals` across the workspace (because
//! `league-core`'s own `[dev-dependencies]` and `league-standalone` enable it). Under unification,
//! the `engine_internals` module path compiles into seat-facing crates during `--workspace` builds
//! even though `TurnEngine`'s inherent raw methods stay `pub(crate)`. Therefore, this guard's
//! manifest rules, public-surface allow-lists, and seat-side identifier/signature scan are the
//! workspace-wide barrier, complemented by CI's standalone `-p league-tui` / `cargo tree` check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use league_core::engine_internals::SweepMode;

const FORBIDDEN_RAW_FNS: [&str; 8] = [
    "private_hints_for_auditor",
    "observe_seat_with_mandate",
    "preview_bundle",
    "preview_bundle_live",
    "preview_bundle_in_mode",
    "preview_bundle_with_context",
    "preview_bundle_with_context_live",
    "preview_bundle_with_context_and_mode",
];

const RAW_ENGINE_ONLY_TYPES: [&str; 10] = [
    "TurnEngine",
    "EngineState",
    "SweepMode",
    "SweepReceipt",
    "BundlePreview",
    "ScheduledShockSpec",
    "SealedEvalSeed",
    "SealedMandateDeal",
    "ClubSealPayload",
    "engine_internals",
];

const RAW_ENGINE_AND_HOST_TYPES: [&str; 13] = [
    "TurnEngine",
    "EngineState",
    "SweepMode",
    "SweepReceipt",
    "BundlePreview",
    "ScheduledShockSpec",
    "SealedEvalSeed",
    "SealedMandateDeal",
    "TableSpec",
    "ClubHost",
    "ClubSaveSnapshot",
    "ClubSealPayload",
    "engine_internals",
];

const RAW_ENGINE_TYPES_FOR_HOST_MODULES: [&str; 20] = [
    "TurnEngine",
    "EngineState",
    "SweepMode",
    "SweepReceipt",
    "BundlePreview",
    "ScheduledShockSpec",
    "ClubSealPayload",
    "engine_internals",
    "from_host_parts",
    "with_own_mandate",
    "observe_seat_with_mandate",
    "to_save_and_seal_bytes",
    "from_save_and_seal_bytes",
    "private_hints_for_auditor",
    "preview_bundle",
    "preview_bundle_live",
    "preview_bundle_in_mode",
    "preview_bundle_with_context",
    "preview_bundle_with_context_live",
    "preview_bundle_with_context_and_mode",
];

const FORBIDDEN_SEAT_IDENTIFIERS: [&str; 30] = [
    "TurnEngine",
    "EngineState",
    "engine_internals",
    "SweepMode",
    "SweepReceipt",
    "BundlePreview",
    "ScheduledShockSpec",
    "SealedEvalSeed",
    "SealedMandateDeal",
    "TableSpec",
    "ClubHost",
    "ClubSaveSnapshot",
    "ClubSealPayload",
    "mandate_for_seat",
    "from_host_parts",
    "with_own_mandate",
    "observe_seat_with_mandate",
    "to_save_and_seal_bytes",
    "from_save_and_seal_bytes",
    "write_save_and_seal",
    "read_save_and_seal",
    "wipe_seal_file",
    "public_save_snapshot",
    "private_hints_for_auditor",
    "preview_bundle",
    "preview_bundle_live",
    "preview_bundle_in_mode",
    "preview_bundle_with_context",
    "preview_bundle_with_context_live",
    "preview_bundle_with_context_and_mode",
];

const TUI_HOST_MODULES: [&str; 2] = ["src/host.rs", "src/main.rs"];

const NON_SEAT_WORKSPACE_CRATES: [&str; 7] = [
    "league-core",
    "league-standalone",
    "league-error",
    "league-ledger",
    "league-season",
    "league-gen",
    "league-tui",
];

const ALLOWED_SEAT_TYPES: &[&str] = &[
    // Seat-safe `league-core` observation, preview, predicate, and value types
    "SeatObservation",
    "PublicSweepReceipt",
    "PublicPreviewState",
    "WhatIfPreview",
    "HumanWhatIfRequest",
    "AiPreviewRequest",
    "CounterpartyAssumptions",
    "PreviewTieMode",
    "PublicTableSpec",
    "PublicCheckOrderPreview",
    "PublicCheckOutcome",
    "PublicSwapOrderPreview",
    "PublicSwapOutcome",
    "AuditorShockHint",
    "RefusedActionPreview",
    "Step0To7SweepSummary",
    "SeatSpoilagePreview",
    "SeatUpkeepPreview",
    "ConverterOrderPreview",
    "ProjectOrderPreview",
    "EscrowOrderPreview",
    "SeatStatePredicates",
    "UpkeepAndDebt",
    "SpoilageAndConversion",
    "RecipeCover",
    "GoodScarcity",
    "HoldingMark",
    "ProjectCover",
    "ProjectInputCover",
    "CoalitionReach",
    "SeasonEndAndMarks",
    "SeatSeasonMetrics",
    "StatePredicateSection",
    "TurnPredicateRecord",
    "SeatSweepOutcome",
    "FinalScoreboard",
    "SeatScoreEntry",
    "FundamentalParities",
    "SeatState",
    "SeatConfig",
    "SeatIdentity",
    "RegentDoctrine",
    "ConverterRecipe",
    "ProjectRecipe",
    "GradedMandateCard",
    "PrivateMandate",
    "MandateKind",
    "CreditTier",
    "SealedOrderBundle",
    "ConverterRunOrder",
    "ProjectDeliveryOrder",
    "CharterBidOrder",
    "CouncilVoteBallot",
    "CouncilPlatform",
    "ContractDeliveryOrder",
    "AtomicSwapProposal",
    "DualEscrowProposal",
    "EscrowFinishUnlock",
    "PreimageSecret",
    "CommitmentHash",
    "IntentPledge",
    "IntentTarget",
    "MarketAction",
    "OfferProposal",
    "OfferSide",
    "CredOffer",
    "CreditCheck",
    "CheckProposal",
    "CheckStatus",
    "TokenEscrowVault",
    "DualEscrowVault",
    "EscrowProposal",
    "EscrowStatus",
    "BondProposal",
    "BondSpec",
    "BondSettlementMode",
    "CartelDomain",
    "ActiveCharter",
    "CharterRole",
    "ActiveShock",
    "ShockKind",
    "ShockForecastEvent",
    "FillRecord",
    "FillKind",
    "BrokenPromiseRecord",
    "PendingWashFreeze",
    "WashFreezeSettlement",
    "TurnPhase",
    "AssetId",
    "Seat",
    "Turn",
    "MarketDay",
    "SeasonId",
    "TableId",
    "TableMode",
    "Units",
    "Bps",
    "ClubBot",
    "ClubBotDecision",
    "EngineError",
    // `league-error` & `league-gen` display-safe types
    "StructuredError",
    "ToStructuredError",
    "CommodityCatalog",
    "CommodityEntry",
    "CommodityDisplayEntry",
    "CommodityDisplayMap",
    "SeatCommodityDisplay",
    "CurrencyCode",
    "TableCommodityMap",
    "SeatCommodityAssignment",
    // TUI (`ratatui` / `crossterm`) UI value types
    "Action",
    "Frame",
    "Rect",
    "KeyEvent",
    "KeyCode",
    "KeyModifiers",
    "Line",
    "Span",
    "Text",
    "Paragraph",
    "Block",
    "Borders",
    "Alignment",
    "Style",
    "Color",
    "Modifier",
    "Layout",
    "Constraint",
    "Direction",
    "Table",
    "Row",
    "Cell",
    "List",
    "ListItem",
    "ListState",
    "TableState",
    "Tabs",
    "Gauge",
    "Clear",
    "Buffer",
    "Terminal",
    "TestBackend",
    // Standard library & non-engine relay/TUI crate types
    "bool",
    "char",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "str",
    "String",
    "Option",
    "Result",
    "Vec",
    "VecDeque",
    "BTreeMap",
    "BTreeSet",
    "HashMap",
    "HashSet",
    "Box",
    "Arc",
    "Rc",
    "Cow",
    "OnceLock",
    "Mutex",
    "RwLock",
    "Condvar",
    "Sender",
    "Receiver",
    "AtomicBool",
    "AtomicU8",
    "AtomicU64",
    "AtomicUsize",
    "Ordering",
    "Path",
    "PathBuf",
    "File",
    "OpenOptions",
    "TryLockError",
    "SeekFrom",
    "Duration",
    "Instant",
    "SystemTime",
    "IpAddr",
    "Ipv4Addr",
    "Ipv6Addr",
    "Formatter",
    "Error",
    "Display",
    "Debug",
    "Write",
    "Read",
    "BufRead",
    "Seek",
    "Drop",
    "Default",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Send",
    "Sync",
    "Sized",
    "AsRef",
    "Iterator",
    "IntoIterator",
    "Fn",
    "FnMut",
    "FnOnce",
    "Any",
    "AssertUnwindSafe",
    "PhantomData",
    "ExitCode",
    "Infallible",
    "Pin",
    "Value",
    "Map",
    "Number",
    "Serializer",
    "Deserializer",
    "Serialize",
    "Deserialize",
    "Agent",
    "SameFileHandle",
    "Handle",
    "Sha256",
    "Digest",
    "CodePointSetData",
];

/// Concrete allow-listed public surface types permitted in `crates/league-seat-ui`.
///
/// Explicitly excludes `dyn`, `impl Trait`, `Fn*`, generic type parameters, `Box`, `Any`,
/// `File`/`OpenOptions`/`Path`/`PathBuf`, and raw/bot/host types (`docs/club-mode-design.md` §2.3).
const ALLOWED_SEAT_UI_PUBLIC_TYPES: &[&str] = &[
    // `league-seat-types` observation, preview, predicate, value, and display types
    "SeatObservation",
    "PublicSweepReceipt",
    "PublicPreviewState",
    "WhatIfPreview",
    "HumanWhatIfRequest",
    "AiPreviewRequest",
    "CounterpartyAssumptions",
    "PreviewTieMode",
    "PublicTableSpec",
    "PublicCheckOrderPreview",
    "PublicCheckOutcome",
    "PublicSwapOrderPreview",
    "PublicSwapOutcome",
    "AuditorShockHint",
    "RefusedActionPreview",
    "Step0To7SweepSummary",
    "SeatSpoilagePreview",
    "SeatUpkeepPreview",
    "ConverterOrderPreview",
    "ProjectOrderPreview",
    "EscrowOrderPreview",
    "SeatStatePredicates",
    "UpkeepAndDebt",
    "SpoilageAndConversion",
    "RecipeCover",
    "GoodScarcity",
    "HoldingMark",
    "ProjectCover",
    "ProjectInputCover",
    "CoalitionReach",
    "SeasonEndAndMarks",
    "SeatSeasonMetrics",
    "StatePredicateSection",
    "TurnPredicateRecord",
    "SeatSweepOutcome",
    "FinalScoreboard",
    "SeatScoreEntry",
    "FundamentalParities",
    "ParitySpecSource",
    "SeatState",
    "SeatConfig",
    "SeatIdentity",
    "RegentDoctrine",
    "ConverterRecipe",
    "ProjectRecipe",
    "GradedMandateCard",
    "PrivateMandate",
    "MandateKind",
    "CreditTier",
    "SealedOrderBundle",
    "ConverterRunOrder",
    "ProjectDeliveryOrder",
    "CharterBidOrder",
    "CouncilVoteBallot",
    "CouncilPlatform",
    "ContractDeliveryOrder",
    "AtomicSwapProposal",
    "DualEscrowProposal",
    "EscrowFinishUnlock",
    "PreimageSecret",
    "CommitmentHash",
    "IntentPledge",
    "IntentTarget",
    "MarketAction",
    "OfferProposal",
    "OfferSide",
    "CredOffer",
    "CreditCheck",
    "CheckProposal",
    "CheckStatus",
    "TokenEscrowVault",
    "DualEscrowVault",
    "EscrowProposal",
    "EscrowStatus",
    "BondProposal",
    "BondSpec",
    "BondSettlementMode",
    "CartelDomain",
    "ActiveCharter",
    "CharterRole",
    "ActiveShock",
    "ShockKind",
    "ShockForecastEvent",
    "FillRecord",
    "FillKind",
    "BrokenPromiseRecord",
    "PendingWashFreeze",
    "WashFreezeSettlement",
    "TurnPhase",
    "AssetId",
    "Seat",
    "Turn",
    "MarketDay",
    "SeasonId",
    "TableId",
    "TableMode",
    "Units",
    "Bps",
    "EngineError",
    "StructuredError",
    "ToStructuredError",
    "CommodityDisplayEntry",
    "CommodityDisplayMap",
    "SeatCommodityDisplay",
    "NUMERAIRE_CODE",
    "NUMERAIRE_DISPLAY_LABEL",
    "NUMERAIRE_PLAIN_LABEL",
    // Seat-safe view-model and plain key event types in `league-seat-types::view`
    "ViewColor",
    "ViewStyle",
    "ViewAlign",
    "ViewSpan",
    "ViewRow",
    "ViewLabel",
    "ViewPanel",
    "SeatScreenView",
    "ViewportSize",
    "SeatKeyCode",
    "SeatKeyModifiers",
    "SeatKeyEvent",
    // Concrete `no_std` + `alloc` core value & collection types
    "bool",
    "char",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "str",
    "String",
    "Vec",
    "VecDeque",
    "BTreeMap",
    "BTreeSet",
    "Option",
    "Result",
    "Cow",
    "PhantomData",
    "Ordering",
    "Duration",
    "Range",
    "RangeInclusive",
    "Formatter",
    "Debug",
    "Display",
    "Error",
];

/// Strict entry-point parameter allow-list for `league-seat-ui` `pub fn` functions (`Part B` rule):
/// accepts `league-seat-types` observation/view-model/key values and seat-ui-local UI state —
/// never `Vec<KeyCode>`, slices, tuples, `StructuredError`, `EngineError`,
/// `RefusedActionPreview`, `char`, or free `&str`/`String`/`Cow`/`OsStr`/`CStr`.
const ALLOWED_SEAT_UI_ENTRY_PARAM_TYPES: &[&str] = &[
    "SeatObservation",
    "PublicSweepReceipt",
    "PublicPreviewState",
    "WhatIfPreview",
    "HumanWhatIfRequest",
    "AiPreviewRequest",
    "CounterpartyAssumptions",
    "PreviewTieMode",
    "PublicTableSpec",
    "PublicCheckOrderPreview",
    "PublicCheckOutcome",
    "PublicSwapOrderPreview",
    "PublicSwapOutcome",
    "AuditorShockHint",
    "Step0To7SweepSummary",
    "SeatSpoilagePreview",
    "SeatUpkeepPreview",
    "ConverterOrderPreview",
    "ProjectOrderPreview",
    "EscrowOrderPreview",
    "SeatStatePredicates",
    "UpkeepAndDebt",
    "SpoilageAndConversion",
    "RecipeCover",
    "GoodScarcity",
    "HoldingMark",
    "ProjectCover",
    "ProjectInputCover",
    "CoalitionReach",
    "SeasonEndAndMarks",
    "SeatSeasonMetrics",
    "StatePredicateSection",
    "TurnPredicateRecord",
    "SeatSweepOutcome",
    "FinalScoreboard",
    "SeatScoreEntry",
    "FundamentalParities",
    "SeatState",
    "SeatConfig",
    "SeatIdentity",
    "RegentDoctrine",
    "ConverterRecipe",
    "ProjectRecipe",
    "GradedMandateCard",
    "PrivateMandate",
    "MandateKind",
    "CreditTier",
    "SealedOrderBundle",
    "ConverterRunOrder",
    "ProjectDeliveryOrder",
    "CharterBidOrder",
    "CouncilVoteBallot",
    "CouncilPlatform",
    "ContractDeliveryOrder",
    "AtomicSwapProposal",
    "DualEscrowProposal",
    "EscrowFinishUnlock",
    "IntentPledge",
    "IntentTarget",
    "MarketAction",
    "OfferProposal",
    "OfferSide",
    "CredOffer",
    "CreditCheck",
    "CheckProposal",
    "CheckStatus",
    "TokenEscrowVault",
    "DualEscrowVault",
    "EscrowProposal",
    "EscrowStatus",
    "BondProposal",
    "BondSpec",
    "BondSettlementMode",
    "CartelDomain",
    "ActiveCharter",
    "CharterRole",
    "ActiveShock",
    "ShockKind",
    "ShockForecastEvent",
    "FillRecord",
    "FillKind",
    "BrokenPromiseRecord",
    "PendingWashFreeze",
    "WashFreezeSettlement",
    "TurnPhase",
    "AssetId",
    "Seat",
    "Turn",
    "MarketDay",
    "SeasonId",
    "TableId",
    "TableMode",
    "CommodityDisplayMap",
    "ViewColor",
    "ViewStyle",
    "ViewAlign",
    "ViewSpan",
    "ViewRow",
    "ViewLabel",
    "ViewPanel",
    "SeatScreenView",
    "ViewportSize",
    "SeatKeyCode",
    "SeatKeyModifiers",
    "SeatKeyEvent",
    "Action",
    "bool",
    "Option",
];

/// Additional scalar UI index fields permitted on `pub` fields of seat-ui-local state structs
/// (`UiState::selected: usize`, `AppState::seat_idx: u8`) — never admitted on `pub fn` params
/// or text carriers.
const ALLOWED_SEAT_UI_PUB_FIELD_TYPES: &[&str] = &[
    "SeatObservation",
    "PublicSweepReceipt",
    "PublicPreviewState",
    "WhatIfPreview",
    "HumanWhatIfRequest",
    "AiPreviewRequest",
    "CounterpartyAssumptions",
    "PreviewTieMode",
    "PublicTableSpec",
    "PublicCheckOrderPreview",
    "PublicCheckOutcome",
    "PublicSwapOrderPreview",
    "PublicSwapOutcome",
    "AuditorShockHint",
    "Step0To7SweepSummary",
    "SeatSpoilagePreview",
    "SeatUpkeepPreview",
    "ConverterOrderPreview",
    "ProjectOrderPreview",
    "EscrowOrderPreview",
    "SeatStatePredicates",
    "UpkeepAndDebt",
    "SpoilageAndConversion",
    "RecipeCover",
    "GoodScarcity",
    "HoldingMark",
    "ProjectCover",
    "ProjectInputCover",
    "CoalitionReach",
    "SeasonEndAndMarks",
    "SeatSeasonMetrics",
    "StatePredicateSection",
    "TurnPredicateRecord",
    "SeatSweepOutcome",
    "FinalScoreboard",
    "SeatScoreEntry",
    "FundamentalParities",
    "SeatState",
    "SeatConfig",
    "SeatIdentity",
    "RegentDoctrine",
    "ConverterRecipe",
    "ProjectRecipe",
    "GradedMandateCard",
    "PrivateMandate",
    "MandateKind",
    "CreditTier",
    "SealedOrderBundle",
    "ConverterRunOrder",
    "ProjectDeliveryOrder",
    "CharterBidOrder",
    "CouncilVoteBallot",
    "CouncilPlatform",
    "ContractDeliveryOrder",
    "AtomicSwapProposal",
    "DualEscrowProposal",
    "EscrowFinishUnlock",
    "IntentPledge",
    "IntentTarget",
    "MarketAction",
    "OfferProposal",
    "OfferSide",
    "CredOffer",
    "CreditCheck",
    "CheckProposal",
    "CheckStatus",
    "TokenEscrowVault",
    "DualEscrowVault",
    "EscrowProposal",
    "EscrowStatus",
    "BondProposal",
    "BondSpec",
    "BondSettlementMode",
    "CartelDomain",
    "ActiveCharter",
    "CharterRole",
    "ActiveShock",
    "ShockKind",
    "ShockForecastEvent",
    "FillRecord",
    "FillKind",
    "BrokenPromiseRecord",
    "PendingWashFreeze",
    "WashFreezeSettlement",
    "TurnPhase",
    "AssetId",
    "Seat",
    "Turn",
    "MarketDay",
    "SeasonId",
    "TableId",
    "TableMode",
    "CommodityDisplayMap",
    "ViewColor",
    "ViewStyle",
    "ViewAlign",
    "ViewSpan",
    "ViewRow",
    "ViewLabel",
    "ViewPanel",
    "SeatScreenView",
    "ViewportSize",
    "SeatKeyCode",
    "SeatKeyModifiers",
    "SeatKeyEvent",
    "Action",
    "bool",
    "u8",
    "usize",
    "Option",
];

/// Concrete allow-listed std/serde/error types permitted in `crates/league-seat-types` fields,
/// type aliases, and public APIs (`league-seat-types` also permits types declared in the crate).
const ALLOWED_SEAT_TYPES_SURFACE_TYPES: &[&str] = &[
    "bool",
    "char",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "str",
    "String",
    "Vec",
    "VecDeque",
    "BTreeMap",
    "BTreeSet",
    "Option",
    "Result",
    "Ordering",
    "StructuredError",
    "ToStructuredError",
    "Serializer",
    "Deserializer",
    "Serialize",
    "Deserialize",
    "Formatter",
    "Debug",
    "Display",
    "Error",
    "Sized",
    "Ok",
];

const SEAT_PRIMARY_PARAM_TYPES: [&str; 6] = [
    "SeatObservation",
    "PublicSweepReceipt",
    "PublicPreviewState",
    "WhatIfPreview",
    "HumanWhatIfRequest",
    "AiPreviewRequest",
];

const PRODUCTION_REQUIRED_MODS: [&str; 11] = [
    "bot",
    "club",
    "commit",
    "contracts",
    "engine",
    "error",
    "observation",
    "predicates",
    "replay",
    "scoring",
    "types",
];

const PRODUCTION_ALLOWED_MODS: [&str; 11] = [
    "bot",
    "club",
    "commit",
    "contracts",
    "engine",
    "error",
    "observation",
    "predicates",
    "replay",
    "scoring",
    "types",
];

const ALLOWED_TURN_ENGINE_PUB_FNS: [&str; 21] = [
    "new",
    "state",
    "public_receipts",
    "observe_seat",
    "observe_seat_with_receipts",
    "state_digest",
    "submit_mandate_commitment",
    "begin_cancel_lock",
    "apply_market_action",
    "advance_to_commit",
    "submit_order_commitment",
    "advance_to_reveal",
    "submit_order_reveal",
    "preview_regent_bundle_for_seat",
    "preview_regent_bundle_after_step0",
    "sweep_turn",
    "sweep_turn_seeded",
    "sweep_turn_with_eval_vrf",
    "sweep_turn_with_drand",
    "submit_mandate_reveal",
    "finalize_season",
];

const ALLOWED_CLUB_HOST_PUB_FNS: [&str; 29] = [
    "new",
    "from_standard_setup",
    "from_explicit_setup",
    "layout_seed",
    "bot_tie_seed",
    "table_id",
    "seat_count",
    "turn",
    "phase",
    "cancel_locked",
    "public_spec",
    "public_receipts",
    "observe_seat",
    "mandate_for_seat",
    "preview_regent_bundle_for_seat",
    "apply_market_action",
    "begin_cancel_lock",
    "advance_to_commit",
    "submit_order_commitment",
    "advance_to_reveal",
    "submit_order_reveal",
    "commit_and_reveal_all",
    "sweep_turn",
    "finalize_club_season",
    "export_replay_log",
    "public_save_snapshot",
    "write_save_and_seal",
    "read_save_and_seal",
    "wipe_seal_file",
];

const ALLOWED_OBSERVATION_PUB_FNS: [&str; 4] = [
    "preview_neutral_beacon",
    "preview_what_if_with_beacon",
    "preview_what_if",
    "preview_seat_bundle",
];

const PRODUCTION_ALLOWED_REEXPORTS: [&str; 257] = [
    // bot (landed in Part B)
    "bot::ClubBot",
    "bot::ClubBotDecision",
    // club
    "club::CLUB_SAVE_FORMAT_VERSION",
    "club::CLUB_SEAL_FORMAT_VERSION",
    "club::ClubHost",
    "club::ClubSaveSnapshot",
    "club::DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE",
    // commit
    "commit::AtomicSwapProposal",
    "commit::CharterBidOrder",
    "commit::CommitmentHash",
    "commit::ContractDeliveryOrder",
    "commit::ConverterRunOrder",
    "commit::CouncilPlatform",
    "commit::CouncilVoteBallot",
    "commit::DEFAULT_SHOCK_DRAW_TURNS",
    "commit::DRAND_QUICKNET_GENESIS_UNIX",
    "commit::DRAND_QUICKNET_PERIOD_SECS",
    "commit::DRAND_QUICKNET_SIGNATURE_BYTES",
    "commit::DrandBeaconAuthStatus",
    "commit::DrandSchedule",
    "commit::DualEscrowProposal",
    "commit::EVAL_VRF_PROOF_BYTES",
    "commit::EVAL_VRF_PUBKEY_BYTES",
    "commit::EscrowFinishUnlock",
    "commit::GradedMandateCard",
    "commit::IntentPledge",
    "commit::IntentTarget",
    "commit::MandateKind",
    "commit::MandateReveal",
    "commit::OrderReveal",
    "commit::PreimageSecret",
    "commit::PrivateMandate",
    "commit::ProjectDeliveryOrder",
    "commit::RevealSalt",
    "commit::RngDomain",
    "commit::SealedEvalSeed",
    "commit::SealedMandateDeal",
    "commit::SealedOrderBundle",
    "commit::beacon_roll_bps",
    "commit::bonus_equity_for_completion_bps",
    "commit::bonus_points_milli_for_completion_bps",
    "commit::break_tie_with_beacon",
    "commit::break_tie_with_turn_beacon",
    "commit::close_roll_bps_from_beacon",
    "commit::compute_mandate_commitment",
    "commit::compute_order_commitment",
    "commit::compute_preimage_condition",
    "commit::deal_graded_mandates",
    "commit::deal_graded_mandates_with_entropy",
    "commit::deal_mandate_deck_indices",
    "commit::default_graded_mandate_deck",
    "commit::derive_drand_beacon",
    "commit::derive_turn_beacon",
    "commit::drand_quicknet_round_for_close",
    "commit::drand_quicknet_signature_status",
    "commit::draw_shocks_for_turn",
    "commit::eval_vrf_alpha",
    "commit::expand_turn_stream",
    "commit::graded_mandate_deck_for_mean_project_value",
    "commit::standings_tie_key",
    "commit::validate_eval_vrf_pubkey",
    "commit::verify_and_extract_eval_vrf_beacon",
    "commit::verify_drand_quicknet_beacon",
    "commit::verify_eval_vrf_beacon",
    "commit::verify_mandate_reveal",
    "commit::verify_order_reveal",
    "commit::verify_preimage_condition",
    "commit::verify_sealed_mandate_deal",
    "commit::width_independent_index",
    // contracts
    "contracts::ActiveCharter",
    "contracts::BondProposal",
    "contracts::BondSettlementMode",
    "contracts::BondSpec",
    "contracts::CartelDomain",
    "contracts::CheckProposal",
    "contracts::CheckStatus",
    "contracts::CredOffer",
    "contracts::CreditCheck",
    "contracts::DualEscrowVault",
    "contracts::EscrowProposal",
    "contracts::EscrowStatus",
    "contracts::FillKind",
    "contracts::FillRecord",
    "contracts::MAX_CARTEL_DOMAINS",
    "contracts::MarketAction",
    "contracts::OfferProposal",
    "contracts::OfferSide",
    "contracts::PendingWashFreeze",
    "contracts::TokenEscrowVault",
    "contracts::WashFreezeSettlement",
    "contracts::audit_wash_freeze",
    "contracts::check_offer_domain_access",
    "contracts::effective_export_tariff_bps",
    // engine
    "engine::BrokenPromiseRecord",
    "engine::BundlePreview",
    "engine::CheckOrderPreview",
    "engine::CheckPreviewOutcome",
    "engine::ConverterOrderPreview",
    "engine::EngineState",
    "engine::EscrowOrderPreview",
    "engine::ProjectOrderPreview",
    "engine::SeatState",
    "engine::SeatUpkeepPreview",
    "engine::ShockForecastEvent",
    "engine::SwapOrderPreview",
    "engine::SwapPreviewOutcome",
    "engine::SweepReceipt",
    "engine::TurnEngine",
    "engine::TurnPhase",
    // error
    "error::StructuredError",
    "error::ToStructuredError",
    "error::scrub_secret_text",
    // observation
    "observation::AiPreviewRequest",
    "observation::AuditorShockHint",
    "observation::CounterpartyAssumptions",
    "observation::DEALT_MANDATE_COMMIT_SENTINEL",
    "observation::HYPOTHETICAL_CLOSE_MARK_CAVEAT",
    "observation::HumanWhatIfRequest",
    "observation::PREVIEW_NEUTRAL_BEACON_SEED",
    "observation::PreviewTieMode",
    "observation::PublicCheckOrderPreview",
    "observation::PublicCheckOutcome",
    "observation::PublicPreviewState",
    "observation::PublicSwapOrderPreview",
    "observation::PublicSwapOutcome",
    "observation::PublicSweepReceipt",
    "observation::PublicTableSpec",
    "observation::RefusedActionPreview",
    "observation::SeatObservation",
    "observation::SeatSpoilagePreview",
    "observation::Step0To7SweepSummary",
    "observation::WhatIfPreview",
    "observation::preview_neutral_beacon",
    "observation::preview_seat_bundle",
    "observation::preview_what_if",
    "observation::preview_what_if_with_beacon",
    // predicates
    "predicates::CoalitionReach",
    "predicates::GoodScarcity",
    "predicates::HoldingMark",
    "predicates::ProjectCover",
    "predicates::ProjectInputCover",
    "predicates::RecipeCover",
    "predicates::STATE_PREDICATE_SECTION_VERSION",
    "predicates::STATE_PREDICATES_VERSION",
    "predicates::SeasonEndAndMarks",
    "predicates::SeatSeasonMetrics",
    "predicates::SeatStatePredicates",
    "predicates::SeatSweepOutcome",
    "predicates::SpoilageAndConversion",
    "predicates::StatePredicateSection",
    "predicates::TurnPredicateRecord",
    "predicates::UpkeepAndDebt",
    "predicates::all_coalitions_reach",
    "predicates::cancel_locked",
    "predicates::coalition_reach",
    "predicates::compute_season_metrics",
    "predicates::compute_seat_state_predicates",
    "predicates::covered_with_own_offers_but_short_turns",
    "predicates::held_enough_but_short_turns",
    "predicates::scarcity_and_tariffs",
    "predicates::season_end_and_marks",
    "predicates::spoilage_and_conversion",
    "predicates::spoiled_units_over_season",
    "predicates::stranded_stock_at_close",
    "predicates::upkeep_and_debt",
    // replay
    "replay::LoggedTurn",
    "replay::REPLAY_FORMAT_VERSION",
    "replay::RejectedMarketAction",
    "replay::ReplayBeaconMode",
    "replay::ReplayLayoutSeed",
    "replay::ReplayLayoutSeedError",
    "replay::ReplayOutcome",
    "replay::ReplayRecord",
    "replay::SEALED_LAYOUT_SEED_MIN",
    "replay::SEALED_SEED_POOL_NAME",
    "replay::SealedReplaySeed",
    "replay::SeasonReplayLog",
    "replay::TurnRejectedActions",
    "replay::attach_state_predicates",
    "replay::check_replay_format_version_slice",
    "replay::layout_seed_digest",
    "replay::replay_club_season",
    "replay::replay_eval_season",
    "replay::replay_season",
    "replay::replay_season_with_state_predicates",
    // scoring
    "scoring::FinalScoreboard",
    "scoring::FundamentalParities",
    "scoring::SeatScoreEntry",
    "scoring::compute_final_marked_net_asset_equity",
    "scoring::compute_final_scoreboard",
    "scoring::compute_net_asset_equity",
    "scoring::compute_pbrs_reward",
    "scoring::compute_potential",
    "scoring::compute_table_median_pre_mandate_net_worth",
    "scoring::compute_table_median_starting_net_worth",
    "scoring::convert_mandate_points_to_equity",
    "scoring::convert_mandate_points_to_equity_with_floor",
    "scoring::convert_mandate_points_to_equity_with_median",
    "scoring::evaluate_mandate_bonus",
    "scoring::mark_good_holding_at_close",
    "scoring::table_median_net_worth",
    // types
    "types::ActiveShock",
    "types::AssetId",
    "types::BPS_SCALE",
    "types::Bps",
    "types::CLOSING_ROLL_START_TURN",
    "types::CLOSING_ROLL_WINDOW_LEN",
    "types::CharterRole",
    "types::ConverterRecipe",
    "types::CreditTier",
    "types::DEFAULT_CANCEL_LOCK_BPS",
    "types::DEFAULT_CLOSING_HAZARD_BPS",
    "types::DEFAULT_MANDATE_ALPHA_BPS",
    "types::DEFAULT_PBRS_GAMMA_BPS",
    "types::DEFAULT_TOWN_IMPORT_PREMIUM_BPS",
    "types::DEFAULT_WASH_FREEZE_BOND_CRED",
    "types::DEFAULT_WASH_FREEZE_BOUNTY_CRED",
    "types::DEFAULT_WASH_ROUND_TRIP_MIN_UNITS",
    "types::DEFAULT_WORKS_STIPEND_PER_UNIT_CRED",
    "types::DEV_LAYOUT_SEED_MAX",
    "types::EngineError",
    "types::HARD_TURN_CAP",
    "types::IDLE_MISS_TAKEOVER_TURNS",
    "types::MANDATE_STARTING_NW_FLOOR_BPS",
    "types::MAX_MANDATE_BONUS_EQUITY",
    "types::MAX_TARIFF_BPS",
    "types::MIN_MANDATE_BONUS_EQUITY",
    "types::MarketDay",
    "types::ProjectRecipe",
    "types::RESERVED_TEST_LAYOUT_SEEDS",
    "types::RegentDoctrine",
    "types::ScheduledShockSpec",
    "types::SeasonId",
    "types::Seat",
    "types::SeatConfig",
    "types::SeatIdentity",
    "types::ShockKind",
    "types::TableId",
    "types::TableMode",
    "types::TableSpec",
    "types::Turn",
    "types::UNIT_SCALE",
    "types::UNSOLD_EXCESS_PARITY_BPS",
    "types::Units",
    "types::cap_reach_probability",
    "types::ceil_bps_fee",
    "types::ceil_units_parity",
    "types::checked_ceil_units_parity",
    "types::checked_mul_units_parity",
    "types::expected_season_length_turns",
    "types::floor_bps_part",
    "types::half_up_bps",
    "types::is_reserved_layout_test_seed",
    "types::mul_units_parity",
    "types::scale_bps_signed",
    "types::town_import_cost_cred",
    "types::validate_club_dev_seed",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Pound,
    Bang,
    Semi,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Other(char),
}

const fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

const fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn tokenize_rust(source: &str) -> Vec<Tok> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comment.
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Nested block comment.
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1_usize;
            while i < len && depth > 0 {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Normal string or byte/c string (`"..."`, `b"..."`, `c"..."`).
        if b == b'"' || (matches!(b, b'b' | b'c') && i + 1 < len && bytes[i + 1] == b'"') {
            i += if b == b'"' { 1 } else { 2 };
            let start = i;
            let mut end = i;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    end = i;
                } else if bytes[i] == b'"' {
                    end = i;
                    i += 1;
                    break;
                } else {
                    i += 1;
                    end = i;
                }
            }
            tokens.push(Tok::Str(source[start..end].to_owned()));
            continue;
        }
        // Raw string (`r"..."`, `r#"..."#`, `br#"..."#`, `cr#"..."#`) vs raw ident (`r#ident`).
        let raw_start = if b == b'r' {
            Some(i + 1)
        } else if matches!(b, b'b' | b'c') && i + 1 < len && bytes[i + 1] == b'r' {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut j) = raw_start {
            let mut hashes = 0_usize;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                j += 1;
                let start = j;
                let mut end = j;
                while j < len {
                    if bytes[j] == b'"' {
                        let mut matched = 0_usize;
                        while matched < hashes
                            && j + 1 + matched < len
                            && bytes[j + 1 + matched] == b'#'
                        {
                            matched += 1;
                        }
                        if matched == hashes {
                            end = j;
                            j += 1 + hashes;
                            break;
                        }
                    }
                    j += 1;
                    end = j;
                }
                tokens.push(Tok::Str(source[start..end].to_owned()));
                i = j;
                continue;
            } else if b == b'r' && hashes == 1 && j < len && is_ident_start(bytes[j]) {
                let ident_start = j;
                while j < len && is_ident_continue(bytes[j]) {
                    j += 1;
                }
                tokens.push(Tok::Ident(source[ident_start..j].to_owned()));
                i = j;
                continue;
            }
        }
        // Byte / char literals vs lifetime (`'a`).
        if b == b'b' && i + 1 < len && bytes[i + 1] == b'\'' {
            i += 2;
            if i < len && bytes[i] == b'\\' {
                i = (i + 2).min(len);
            }
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        if b == b'\'' {
            if i + 2 < len && bytes[i + 1] == b'\\' {
                let mut j = i + 3;
                while j < len
                    && bytes[j] != b'\''
                    && !matches!(bytes[j], b'\n' | b'\r')
                    && j - i <= 32
                {
                    j += 1;
                }
                if j < len && bytes[j] == b'\'' {
                    i = j + 1;
                    continue;
                }
            } else if i + 1 < len {
                let mut chars = source[i + 1..].char_indices();
                if let Some((_, ch)) = chars.next() {
                    let after_ch = i + 1 + ch.len_utf8();
                    if after_ch < len && bytes[after_ch] == b'\'' {
                        i = after_ch + 1;
                        continue;
                    }
                }
            }
            tokens.push(Tok::Other('\''));
            i += 1;
            continue;
        }

        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            tokens.push(Tok::Ident(source[start..i].to_owned()));
            continue;
        }

        let tok = match b {
            b'#' => Tok::Pound,
            b'!' => Tok::Bang,
            b';' => Tok::Semi,
            b'{' => Tok::LBrace,
            b'}' => Tok::RBrace,
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            b'[' => Tok::LBracket,
            b']' => Tok::RBracket,
            other => Tok::Other(other as char),
        };
        tokens.push(tok);
        i += 1;
    }
    tokens
}

fn collect_rs_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        let file_type = entry
            .file_type()
            .unwrap_or_else(|err| panic!("failed to read file type {}: {err}", path.display()));
        if file_type.is_dir() {
            collect_rs_files_recursive(&path, out);
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[derive(Default, Clone)]
struct AttrFlags {
    cfg_test: bool,
    cfg_engine_internals: bool,
    doc_hidden: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum ScopeKind {
    Root {
        public: bool,
    },
    Module {
        public: bool,
        is_engine_internals: bool,
        cfg_test: bool,
    },
    InherentImpl {
        type_name: String,
        cfg_test: bool,
    },
    TraitImpl {
        mentions_host: bool,
        self_type: Option<String>,
        cfg_test: bool,
    },
    Block {
        cfg_test: bool,
    },
}

#[derive(Clone, Copy)]
enum Visibility {
    Private,
    CrateOrRestricted,
    Public,
}

fn parse_tok_attr(tokens: &[Tok], mut idx: usize, flags: &mut AttrFlags) -> usize {
    idx += 1;
    let is_inner = idx < tokens.len() && matches!(tokens[idx], Tok::Bang);
    if is_inner {
        idx += 1;
    }
    if idx >= tokens.len() || !matches!(tokens[idx], Tok::LBracket) {
        return idx;
    }
    let start = idx + 1;
    let mut depth = 1_usize;
    idx += 1;
    while idx < tokens.len() && depth > 0 {
        match tokens[idx] {
            Tok::LBracket => depth += 1,
            Tok::RBracket => depth -= 1,
            _ => {}
        }
        idx += 1;
    }
    if is_inner {
        return idx;
    }
    let end = idx.saturating_sub(1);
    if start >= end {
        return idx;
    }
    let inner = &tokens[start..end];
    if let Some(Tok::Ident(head)) = inner.first() {
        if head == "test" && inner.len() == 1 {
            flags.cfg_test = true;
        } else if head == "cfg" {
            // A1 fix: ONLY exact `#[cfg(test)]` counts as test-only — never `#[cfg(not(test))]`,
            // `#[cfg(any(test, ...))]`, `#[cfg(all(not(test)))]`, or any other predicate naming `test`.
            if inner.len() == 4
                && matches!(&inner[1], Tok::LParen)
                && matches!(&inner[2], Tok::Ident(s) if s == "test")
                && matches!(&inner[3], Tok::RParen)
            {
                flags.cfg_test = true;
            }
            if inner
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "feature"))
                && inner
                    .iter()
                    .any(|t| matches!(t, Tok::Str(s) if s == "engine-internals"))
            {
                flags.cfg_engine_internals = true;
            }
        } else if head == "doc"
            && inner
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "hidden"))
        {
            flags.doc_hidden = true;
        }
    }
    idx
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpandedUse {
    canonical_path: String,
    alias: Option<String>,
}

fn expand_use_tree_inner(
    rel_path: &str,
    tokens: &[Tok],
    prefix: &str,
    allow_glob: bool,
    out: &mut Vec<ExpandedUse>,
) -> Result<(), String> {
    let mut parts: Vec<String> = if prefix.is_empty() {
        Vec::new()
    } else {
        vec![prefix.to_owned()]
    };
    let mut pending_alias: Option<String> = None;
    let mut saw_glob = false;
    let mut i = 0_usize;
    while i < tokens.len() {
        match &tokens[i] {
            Tok::Other('*') => {
                if !allow_glob {
                    return Err(format!(
                        "{rel_path}: glob `use *` is forbidden in guarded modules"
                    ));
                }
                saw_glob = true;
                i += 1;
                continue;
            }
            Tok::Ident(kw) if kw == "as" => {
                if let Some(Tok::Ident(alias_name)) = tokens.get(i + 1) {
                    pending_alias = Some(alias_name.clone());
                }
                i += 2;
                continue;
            }
            Tok::Ident(seg) => {
                if seg == "self" {
                    // `foo::{self}` refers to `foo`.
                } else {
                    parts.push(seg.clone());
                }
                i += 1;
            }
            Tok::Other(':') => {
                i += 1;
            }
            Tok::LBrace => {
                let mut depth = 1_usize;
                let start = i + 1;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    match tokens[i] {
                        Tok::LBrace => depth += 1,
                        Tok::RBrace => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                let inner = &tokens[start..i.saturating_sub(1)];
                let group_prefix = parts.join("::");
                let mut item_start = 0_usize;
                let mut sub_depth = 0_usize;
                for (pos, tok) in inner.iter().enumerate() {
                    match tok {
                        Tok::LBrace => sub_depth += 1,
                        Tok::RBrace => sub_depth = sub_depth.saturating_sub(1),
                        Tok::Other(',') if sub_depth == 0 => {
                            if item_start < pos {
                                expand_use_tree_inner(
                                    rel_path,
                                    &inner[item_start..pos],
                                    &group_prefix,
                                    allow_glob,
                                    out,
                                )?;
                            }
                            item_start = pos + 1;
                        }
                        _ => {}
                    }
                }
                if item_start < inner.len() {
                    expand_use_tree_inner(
                        rel_path,
                        &inner[item_start..],
                        &group_prefix,
                        allow_glob,
                        out,
                    )?;
                }
                return Ok(());
            }
            Tok::Other(',') => {
                if !saw_glob && !parts.is_empty() {
                    out.push(ExpandedUse {
                        canonical_path: parts.join("::"),
                        alias: pending_alias.take(),
                    });
                }
                pending_alias = None;
                saw_glob = false;
                parts = if prefix.is_empty() {
                    Vec::new()
                } else {
                    vec![prefix.to_owned()]
                };
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    if !saw_glob && !parts.is_empty() {
        out.push(ExpandedUse {
            canonical_path: parts.join("::"),
            alias: pending_alias,
        });
    }
    Ok(())
}

fn expand_use_tree(
    rel_path: &str,
    tokens: &[Tok],
    prefix: &str,
    out: &mut Vec<ExpandedUse>,
) -> Result<(), String> {
    expand_use_tree_inner(rel_path, tokens, prefix, false, out)
}

fn expand_use_tree_allow_glob(rel_path: &str, tokens: &[Tok], out: &mut Vec<ExpandedUse>) {
    let _ = expand_use_tree_inner(rel_path, tokens, "", true, out);
}

#[derive(Default, Clone)]
struct FileAliasTable {
    map: BTreeMap<String, BTreeSet<String>>,
}

impl FileAliasTable {
    fn insert_edge(&mut self, local: String, target: String) {
        if local != target {
            self.map.entry(local).or_default().insert(target);
        }
    }

    fn resolve(&self, ident: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut stack = vec![ident.to_owned()];
        while let Some(cur) = stack.pop() {
            if out.insert(cur.clone())
                && let Some(targets) = self.map.get(&cur)
            {
                for target in targets {
                    if !out.contains(target) {
                        stack.push(target.clone());
                    }
                }
            }
        }
        out
    }

    fn merge_from(&mut self, other: &Self) {
        for (k, vs) in &other.map {
            let entry = self.map.entry(k.clone()).or_default();
            for v in vs {
                if v != k {
                    entry.insert(v.clone());
                }
            }
        }
    }
}

fn build_file_alias_table(rel_path: &str, tokens: &[Tok]) -> FileAliasTable {
    let mut table = FileAliasTable::default();
    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut i = 0_usize;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });
        let in_trait_impl = scopes
            .iter()
            .any(|s| matches!(s, ScopeKind::TraitImpl { .. }));

        match &tokens[i] {
            Tok::Pound => {
                i = parse_tok_attr(tokens, i, &mut attrs);
                continue;
            }
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        let after_vis = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                find_matching_paren(tokens, i + 1) + 1
            } else {
                i + 1
            }
        } else {
            i
        };

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let header = &tokens[after_vis + 1..j];
            let mut saw_for = false;
            let mut depth_a = 0_usize;
            let mut depth_p = 0_usize;
            let mut depth_b = 0_usize;
            for (pos, tok) in header.iter().enumerate() {
                match tok {
                    Tok::Other('<') => depth_a += 1,
                    Tok::Other('>')
                        if !matches!(
                            header.get(pos.saturating_sub(1)),
                            Some(Tok::Other('-') | Tok::Other('='))
                        ) =>
                    {
                        depth_a = depth_a.saturating_sub(1);
                    }
                    Tok::LParen => depth_p += 1,
                    Tok::RParen => depth_p = depth_p.saturating_sub(1),
                    Tok::LBracket => depth_b += 1,
                    Tok::RBracket => depth_b = depth_b.saturating_sub(1),
                    Tok::Ident(s) if depth_a == 0 && depth_p == 0 && depth_b == 0 && s == "for" => {
                        saw_for = true;
                        break;
                    }
                    Tok::Ident(s)
                        if depth_a == 0 && depth_p == 0 && depth_b == 0 && s == "where" =>
                    {
                        break;
                    }
                    _ => {}
                }
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(if saw_for {
                    ScopeKind::TraitImpl {
                        mentions_host: false,
                        self_type: None,
                        cfg_test: attrs.cfg_test,
                    }
                } else {
                    ScopeKind::InherentImpl {
                        type_name: String::new(),
                        cfg_test: attrs.cfg_test,
                    }
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                j += 1;
            }
            let mut expanded = Vec::new();
            expand_use_tree_allow_glob(rel_path, &tokens[after_vis + 1..j], &mut expanded);
            for item in expanded {
                let leaf = item
                    .canonical_path
                    .rsplit("::")
                    .next()
                    .unwrap_or(&item.canonical_path);
                let local = item.alias.as_deref().unwrap_or(leaf);
                table.insert_edge(local.to_owned(), leaf.to_owned());
                if item.canonical_path.contains("engine_internals") {
                    table.insert_edge(local.to_owned(), "engine_internals".to_owned());
                }
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if !in_trait_impl
            && matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "type")
            && let Some(Tok::Ident(alias_name)) = tokens.get(after_vis + 1)
        {
            let mut j = after_vis + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi | Tok::LBrace) {
                j += 1;
            }
            let stmt = &tokens[after_vis + 2..j];
            let eq_start = skip_angle_generics(stmt, 0);
            if let Some(rel_eq) = stmt[eq_start..]
                .iter()
                .position(|t| matches!(t, Tok::Other('=')))
            {
                let rhs_start = eq_start + rel_eq + 1;
                let mut prev_was_quote = false;
                for tok in &stmt[rhs_start..] {
                    match tok {
                        Tok::Ident(kw) if kw == "where" && !prev_was_quote => break,
                        Tok::Ident(id) => {
                            if prev_was_quote {
                                prev_was_quote = false;
                            } else if !TYPE_KEYWORDS_SKIP.contains(&id.as_str()) {
                                table.insert_edge(alias_name.clone(), id.clone());
                            }
                        }
                        Tok::Other('\'') => prev_was_quote = true,
                        _ => prev_was_quote = false,
                    }
                }
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }

    table
}

fn find_matching_brace(tokens: &[Tok], open_brace_idx: usize) -> usize {
    let mut depth = 1_usize;
    let mut k = open_brace_idx + 1;
    while k < tokens.len() && depth > 0 {
        match tokens[k] {
            Tok::LBrace => depth += 1,
            Tok::RBrace => depth -= 1,
            _ => {}
        }
        k += 1;
    }
    k.saturating_sub(1)
}

fn find_matching_paren(tokens: &[Tok], open_paren_idx: usize) -> usize {
    let mut depth = 1_usize;
    let mut k = open_paren_idx + 1;
    while k < tokens.len() && depth > 0 {
        match tokens[k] {
            Tok::LParen => depth += 1,
            Tok::RParen => depth -= 1,
            _ => {}
        }
        k += 1;
    }
    k.saturating_sub(1)
}

struct CoreFileScan {
    seen_engine_internals_mod: bool,
    pub_mods: Vec<String>,
    lib_reexports: Vec<String>,
    pub_fns_by_scope: BTreeMap<String, Vec<String>>,
}

fn check_pub_fn_body_and_return(
    rel_path: &str,
    scope_name: &str,
    fn_name: &str,
    ret_tokens: &[Tok],
    body_tokens: &[Tok],
    aliases: &FileAliasTable,
) -> Result<(), String> {
    // B4 structural body ban: no `pub fn` outside `engine_internals` may invoke `SweepMode::Live`
    // (except the four `TurnEngine::sweep_turn*` live execution entrypoints) or raw internal
    // preview/auditor helpers.
    let allowed_live_sweep = rel_path == "engine.rs"
        && scope_name == "TurnEngine"
        && matches!(
            fn_name,
            "sweep_turn"
                | "sweep_turn_seeded"
                | "sweep_turn_with_eval_vrf"
                | "sweep_turn_with_drand"
        );
    if !allowed_live_sweep {
        for w in body_tokens.windows(4) {
            if matches!(&w[0], Tok::Ident(a) if a == "SweepMode" || aliases.resolve(a).contains("SweepMode"))
                && matches!(w[1], Tok::Other(':'))
                && matches!(w[2], Tok::Other(':'))
                && matches!(&w[3], Tok::Ident(b) if b == "Live")
            {
                return Err(format!(
                    "{rel_path}: `pub fn {scope_name}::{fn_name}` references `SweepMode::Live` outside `engine_internals`"
                ));
            }
        }
    }
    for tok in body_tokens {
        if let Tok::Ident(ident) = tok {
            let resolved = aliases.resolve(ident);
            for r in &resolved {
                if r == "preview_bundle_with_context_internal"
                    || r == "sweep_turn_detailed_with_mode"
                    || (r == "observe_seat_with_mandate"
                        && !(scope_name == "ClubHost" && fn_name == "observe_seat"))
                    || (r != "observe_seat_with_mandate" && FORBIDDEN_RAW_FNS.contains(&r.as_str()))
                {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` calls restricted internal `{r}` outside `engine_internals`"
                    ));
                }
            }
        }
    }

    // B4 structural return-type ban: forbid renamed/aliased public wrappers returning raw state,
    // engine, receipt, bundle preview, or unredacted shock specs.
    let mut prev_was_quote = false;
    for tok in ret_tokens {
        if let Tok::Ident(ident) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            for resolved_ident in aliases.resolve(ident) {
                let via_note = if resolved_ident == *ident {
                    String::new()
                } else {
                    format!(" (via `{ident}`)")
                };
                if resolved_ident == "BundlePreview" {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` must not return raw `BundlePreview`{via_note} outside `engine_internals`"
                    ));
                }
                if resolved_ident == "EngineState"
                    && !(scope_name == "TurnEngine" && fn_name == "state")
                {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` must not return raw `EngineState`{via_note}"
                    ));
                }
                if resolved_ident == "TurnEngine"
                    && !(scope_name == "TurnEngine" && fn_name == "new")
                {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose `TurnEngine`{via_note}"
                    ));
                }
                if resolved_ident == "SweepReceipt"
                    && !(scope_name == "TurnEngine"
                        && matches!(
                            fn_name,
                            "sweep_turn"
                                | "sweep_turn_seeded"
                                | "sweep_turn_with_eval_vrf"
                                | "sweep_turn_with_drand"
                        ))
                {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose raw `SweepReceipt`{via_note}"
                    ));
                }
                if resolved_ident == "ScheduledShockSpec"
                    && !(rel_path == "commit.rs" && fn_name == "draw_shocks_for_turn")
                {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose unredacted `ScheduledShockSpec`{via_note}"
                    ));
                }
                if matches!(
                    resolved_ident.as_str(),
                    "SweepMode" | "ClubSealPayload" | "engine_internals"
                ) {
                    return Err(format!(
                        "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose `{resolved_ident}`{via_note}"
                    ));
                }
            }
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }

    Ok(())
}

fn check_tokens_no_raw_types_resolved(
    rel_path: &str,
    context: &str,
    tokens: &[Tok],
    banned: &[&str],
    aliases: &FileAliasTable,
) -> Result<(), String> {
    let mut prev_was_quote = false;
    for tok in tokens {
        if let Tok::Ident(ident) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            for resolved in aliases.resolve(ident) {
                if banned.contains(&resolved.as_str()) {
                    if resolved == *ident {
                        return Err(format!(
                            "{rel_path}: {context} must not reference raw engine/host type `{resolved}`"
                        ));
                    }
                    return Err(format!(
                        "{rel_path}: {context} must not reference raw engine/host type `{resolved}` (via `{ident}`)"
                    ));
                }
            }
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }
    Ok(())
}

fn skip_angle_generics(tokens: &[Tok], mut idx: usize) -> usize {
    if matches!(tokens.get(idx), Some(Tok::Other('<'))) {
        let mut depth = 1_usize;
        idx += 1;
        while idx < tokens.len() && depth > 0 {
            match tokens[idx] {
                Tok::Other('<') => depth += 1,
                Tok::Other('>')
                    if !matches!(
                        tokens.get(idx.saturating_sub(1)),
                        Some(Tok::Other('-') | Tok::Other('='))
                    ) =>
                {
                    depth -= 1;
                }
                _ => {}
            }
            idx += 1;
        }
    }
    idx
}

fn parse_impl_header(
    header: &[Tok],
    aliases: &FileAliasTable,
) -> (bool, Vec<String>, Option<String>, bool) {
    let mut i = 0_usize;
    let mut impl_generics = Vec::new();
    if matches!(header.get(i), Some(Tok::Other('<'))) {
        let start = i + 1;
        let end = skip_angle_generics(header, i).saturating_sub(1);
        impl_generics = extract_generic_param_names(&header[start..end]);
        i = end + 1;
    }
    let mut before_where: Vec<&Tok> = Vec::new();
    let mut saw_for = false;
    let mut for_pos = None;
    let mut depth_angle = 0_usize;
    let mut depth_paren = 0_usize;
    let mut depth_bracket = 0_usize;
    for (pos, tok) in header.iter().enumerate().skip(i) {
        match tok {
            Tok::Other('<') => depth_angle += 1,
            Tok::Other('>')
                if !matches!(
                    header.get(pos.saturating_sub(1)),
                    Some(Tok::Other('-') | Tok::Other('='))
                ) =>
            {
                depth_angle = depth_angle.saturating_sub(1);
            }
            Tok::LParen => depth_paren += 1,
            Tok::RParen => depth_paren = depth_paren.saturating_sub(1),
            Tok::LBracket => depth_bracket += 1,
            Tok::RBracket => depth_bracket = depth_bracket.saturating_sub(1),
            Tok::Ident(s)
                if depth_angle == 0 && depth_paren == 0 && depth_bracket == 0 && s == "for" =>
            {
                saw_for = true;
                for_pos = Some(pos);
            }
            Tok::Ident(s)
                if depth_angle == 0 && depth_paren == 0 && depth_bracket == 0 && s == "where" =>
            {
                break;
            }
            _ => {
                before_where.push(tok);
            }
        }
    }
    let _ = before_where;
    let self_slice = match for_pos {
        Some(fp) => {
            let mut end = fp + 1;
            while end < header.len() {
                if matches!(&header[end], Tok::Ident(s) if s == "where") {
                    break;
                }
                end += 1;
            }
            &header[fp + 1..end]
        }
        None => {
            let mut end = i;
            while end < header.len() {
                if matches!(&header[end], Tok::Ident(s) if s == "where") {
                    break;
                }
                end += 1;
            }
            &header[i..end]
        }
    };
    let mut self_idents = Vec::new();
    let mut depth = 0_usize;
    for (idx, tok) in self_slice.iter().enumerate() {
        match tok {
            Tok::Other('<') => depth += 1,
            Tok::Other('>')
                if !matches!(
                    self_slice.get(idx.saturating_sub(1)),
                    Some(Tok::Other('-') | Tok::Other('='))
                ) =>
            {
                depth = depth.saturating_sub(1);
            }
            Tok::Ident(s) if depth == 0 => self_idents.push(s.clone()),
            _ => {}
        }
    }
    let self_type = self_idents.last().cloned().map(|raw_name| {
        let resolved = aliases.resolve(&raw_name);
        if resolved.contains("ClubHost") {
            "ClubHost".to_owned()
        } else if resolved.contains("TurnEngine") {
            "TurnEngine".to_owned()
        } else {
            raw_name
        }
    });
    let mut prev_was_quote = false;
    let mut mentions_host = false;
    for tok in header {
        if let Tok::Ident(s) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            let resolved = aliases.resolve(s);
            if resolved.contains("ClubHost") || resolved.contains("ClubSaveSnapshot") {
                mentions_host = true;
                break;
            }
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }
    (saw_for, impl_generics, self_type, mentions_host)
}

fn check_struct_fields_not_public(
    rel_path: &str,
    struct_name: &str,
    tokens: &[Tok],
    body_start: usize,
) -> Result<(), String> {
    if !matches!(struct_name, "ClubHost" | "TurnEngine") {
        return Ok(());
    }
    let open = tokens.get(body_start);
    if matches!(open, Some(Tok::LBrace)) {
        let close = find_matching_brace(tokens, body_start);
        let body = &tokens[body_start + 1..close];
        for seg in split_top_level_comma(body) {
            let mut k = 0_usize;
            while k < seg.len() {
                if matches!(seg[k], Tok::Pound) {
                    k = parse_tok_attr(seg, k, &mut AttrFlags::default());
                } else {
                    break;
                }
            }
            if matches!(seg.get(k), Some(Tok::Ident(s)) if s == "pub") {
                let (restricted, name_start) = if matches!(seg.get(k + 1), Some(Tok::LParen)) {
                    (true, find_matching_paren(seg, k + 1) + 1)
                } else {
                    (false, k + 1)
                };
                let field_name = seg[name_start..]
                    .iter()
                    .find_map(|t| match t {
                        Tok::Ident(id) => Some(id.as_str()),
                        _ => None,
                    })
                    .unwrap_or("<field>");
                if restricted {
                    return Err(format!(
                        "{rel_path}: `{struct_name}` must not have restricted `pub(...)` field `{field_name}`"
                    ));
                }
                return Err(format!(
                    "{rel_path}: `{struct_name}` must not have pub field `{field_name}`"
                ));
            }
        }
    } else if matches!(open, Some(Tok::LParen)) {
        let close = find_matching_paren(tokens, body_start);
        let body = &tokens[body_start + 1..close];
        if body
            .iter()
            .any(|t| matches!(t, Tok::Ident(s) if s == "pub"))
        {
            return Err(format!(
                "{rel_path}: `{struct_name}` must not have pub tuple field"
            ));
        }
    }
    Ok(())
}

fn check_public_struct_no_raw_pub_fields(
    rel_path: &str,
    struct_name: &str,
    is_core_crate: bool,
    tokens: &[Tok],
    body_start: usize,
    aliases: &FileAliasTable,
) -> Result<(), String> {
    const ALWAYS_BANNED_CORE_PUB_FIELDS: [&str; 8] = [
        "TurnEngine",
        "EngineState",
        "SweepMode",
        "ClubHost",
        "ClubSaveSnapshot",
        "ClubSealPayload",
        "SealedEvalSeed",
        "engine_internals",
    ];
    const SECONDARY_CORE_PUB_FIELDS: [&str; 5] = [
        "SweepReceipt",
        "BundlePreview",
        "ScheduledShockSpec",
        "SealedMandateDeal",
        "TableSpec",
    ];
    let allow_secondary = is_core_crate
        && (matches!(
            struct_name,
            "EngineState" | "SweepReceipt" | "BundlePreview" | "TableSpec"
        ) || (rel_path == "replay.rs"
            && matches!(
                struct_name,
                "LoggedTurn" | "SeasonReplayLog" | "ReplayOutcome"
            )));

    let open = tokens.get(body_start);
    if matches!(open, Some(Tok::LBrace)) {
        let close = find_matching_brace(tokens, body_start);
        for seg in split_top_level_comma(&tokens[body_start + 1..close]) {
            let mut k = 0_usize;
            while k < seg.len() {
                if matches!(seg[k], Tok::Pound) {
                    k = parse_tok_attr(seg, k, &mut AttrFlags::default());
                } else {
                    break;
                }
            }
            // Only unrestricted `pub` fields expose raw types outside the module/crate
            // (`ClubHost`/`TurnEngine` already forbid `pub(...)` above).
            if matches!(seg.get(k), Some(Tok::Ident(s)) if s == "pub")
                && !matches!(seg.get(k + 1), Some(Tok::LParen))
            {
                let rest = &seg[k + 1..];
                let field_name = match rest.first() {
                    Some(Tok::Ident(f)) => f.as_str(),
                    _ => "<field>",
                };
                let Some(colon_pos) = rest.iter().position(|t| matches!(t, Tok::Other(':'))) else {
                    continue;
                };
                let ty_tokens = &rest[colon_pos + 1..];
                if !is_core_crate {
                    check_tokens_no_raw_types_resolved(
                        rel_path,
                        &format!("`pub struct {struct_name}` `pub` field `{field_name}`"),
                        ty_tokens,
                        &RAW_ENGINE_AND_HOST_TYPES,
                        aliases,
                    )?;
                } else {
                    check_tokens_no_raw_types_resolved(
                        rel_path,
                        &format!("`pub struct {struct_name}` `pub` field `{field_name}`"),
                        ty_tokens,
                        &ALWAYS_BANNED_CORE_PUB_FIELDS,
                        aliases,
                    )?;
                    if !allow_secondary {
                        check_tokens_no_raw_types_resolved(
                            rel_path,
                            &format!("`pub struct {struct_name}` `pub` field `{field_name}`"),
                            ty_tokens,
                            &SECONDARY_CORE_PUB_FIELDS,
                            aliases,
                        )?;
                    }
                }
            }
        }
    } else if matches!(open, Some(Tok::LParen)) {
        let close = find_matching_paren(tokens, body_start);
        for (idx, seg) in split_top_level_comma(&tokens[body_start + 1..close])
            .into_iter()
            .enumerate()
        {
            let mut k = 0_usize;
            while k < seg.len() {
                if matches!(seg[k], Tok::Pound) {
                    k = parse_tok_attr(seg, k, &mut AttrFlags::default());
                } else {
                    break;
                }
            }
            if matches!(seg.get(k), Some(Tok::Ident(s)) if s == "pub")
                && !matches!(seg.get(k + 1), Some(Tok::LParen))
            {
                let ty_tokens = &seg[k + 1..];
                if !is_core_crate {
                    check_tokens_no_raw_types_resolved(
                        rel_path,
                        &format!("`pub struct {struct_name}` `pub` tuple field .{idx}"),
                        ty_tokens,
                        &RAW_ENGINE_AND_HOST_TYPES,
                        aliases,
                    )?;
                } else {
                    check_tokens_no_raw_types_resolved(
                        rel_path,
                        &format!("`pub struct {struct_name}` `pub` tuple field .{idx}"),
                        ty_tokens,
                        &ALWAYS_BANNED_CORE_PUB_FIELDS,
                        aliases,
                    )?;
                    if !allow_secondary {
                        check_tokens_no_raw_types_resolved(
                            rel_path,
                            &format!("`pub struct {struct_name}` `pub` tuple field .{idx}"),
                            ty_tokens,
                            &SECONDARY_CORE_PUB_FIELDS,
                            aliases,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn collect_crate_restricted_aliases_from_files(files: &[(String, String)]) -> FileAliasTable {
    let mut crate_aliases = FileAliasTable::default();
    for (rel_path, source) in files {
        let tokens = tokenize_rust(source);
        let mut scopes = vec![ScopeKind::Root { public: true }];
        let mut attrs = AttrFlags::default();
        let mut i = 0_usize;
        while i < tokens.len() {
            let in_cfg_test = scopes.iter().any(|s| match s {
                ScopeKind::Root { .. } => false,
                ScopeKind::Module { cfg_test, .. }
                | ScopeKind::InherentImpl { cfg_test, .. }
                | ScopeKind::TraitImpl { cfg_test, .. }
                | ScopeKind::Block { cfg_test } => *cfg_test,
            });
            match &tokens[i] {
                Tok::Pound => {
                    i = parse_tok_attr(&tokens, i, &mut attrs);
                    continue;
                }
                Tok::RBrace => {
                    if scopes.len() > 1 {
                        scopes.pop();
                    }
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::LBrace => {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::Semi => {
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if in_cfg_test || attrs.cfg_test {
                if matches!(
                    tokens[i],
                    Tok::Ident(ref s)
                        if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
                ) {
                    let mut j = i + 1;
                    while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                        j += 1;
                    }
                    if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                        scopes.push(ScopeKind::Block { cfg_test: true });
                        attrs = AttrFlags::default();
                        i = j + 1;
                        continue;
                    }
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
                i += 1;
                continue;
            }
            let (vis, after_vis) = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                    (
                        Visibility::CrateOrRestricted,
                        find_matching_paren(&tokens, i + 1) + 1,
                    )
                } else {
                    (Visibility::Public, i + 1)
                }
            } else {
                (Visibility::Private, i)
            };
            if matches!(vis, Visibility::CrateOrRestricted | Visibility::Public) {
                if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
                    let mut j = after_vis + 1;
                    while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                        j += 1;
                    }
                    let mut expanded = Vec::new();
                    expand_use_tree_allow_glob(rel_path, &tokens[after_vis + 1..j], &mut expanded);
                    for item in expanded {
                        if let Some(alias) = &item.alias {
                            let leaf = item
                                .canonical_path
                                .rsplit("::")
                                .next()
                                .unwrap_or(&item.canonical_path);
                            crate_aliases.insert_edge(alias.clone(), leaf.to_owned());
                        }
                    }
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
                if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "type")
                    && let Some(Tok::Ident(alias_name)) = tokens.get(after_vis + 1)
                {
                    let mut j = after_vis + 2;
                    while j < tokens.len() && !matches!(tokens[j], Tok::Semi | Tok::LBrace) {
                        j += 1;
                    }
                    let stmt = &tokens[after_vis + 2..j];
                    let eq_start = skip_angle_generics(stmt, 0);
                    if let Some(rel_eq) = stmt[eq_start..]
                        .iter()
                        .position(|t| matches!(t, Tok::Other('=')))
                    {
                        let rhs_start = eq_start + rel_eq + 1;
                        let mut prev_was_quote = false;
                        for tok in &stmt[rhs_start..] {
                            match tok {
                                Tok::Ident(kw) if kw == "where" && !prev_was_quote => break,
                                Tok::Ident(id) => {
                                    if prev_was_quote {
                                        prev_was_quote = false;
                                    } else if !TYPE_KEYWORDS_SKIP.contains(&id.as_str()) {
                                        crate_aliases.insert_edge(alias_name.clone(), id.clone());
                                    }
                                }
                                Tok::Other('\'') => prev_was_quote = true,
                                _ => prev_was_quote = false,
                            }
                        }
                    }
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
            }
            if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                attrs = AttrFlags::default();
            }
            i += 1;
        }
    }
    crate_aliases
}

#[allow(dead_code)]
fn scan_file_public_api(
    rel_path: &str,
    source: &str,
    is_lib_rs: bool,
) -> Result<CoreFileScan, String> {
    scan_file_public_api_with_crate_aliases(rel_path, source, is_lib_rs, &FileAliasTable::default())
}

fn scan_file_public_api_with_crate_aliases(
    rel_path: &str,
    source: &str,
    is_lib_rs: bool,
    crate_aliases: &FileAliasTable,
) -> Result<CoreFileScan, String> {
    let tokens = tokenize_rust(source);
    let mut alias_table = build_file_alias_table(rel_path, &tokens);
    alias_table.merge_from(crate_aliases);
    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut seen_engine_internals_mod = false;
    let mut pub_mods = Vec::new();
    let mut lib_reexports = Vec::new();
    let mut pub_fns_by_scope: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut i = 0;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });
        let in_public_non_internal_mod = scopes.iter().all(|s| match s {
            ScopeKind::Root { public } => *public,
            ScopeKind::Module {
                public,
                is_engine_internals,
                ..
            } => *public && !*is_engine_internals,
            ScopeKind::InherentImpl { .. } | ScopeKind::TraitImpl { .. } => true,
            ScopeKind::Block { .. } => false,
        });
        let enclosing_impl = match scopes.last() {
            Some(ScopeKind::InherentImpl { type_name, .. }) => Some(type_name.as_str()),
            _ => None,
        };
        let in_host_trait_impl = matches!(
            scopes.last(),
            Some(ScopeKind::TraitImpl {
                mentions_host: true,
                ..
            })
        );
        let in_any_trait_impl = matches!(scopes.last(), Some(ScopeKind::TraitImpl { .. }));
        let in_engine_internals_mod = scopes.iter().any(|s| {
            matches!(
                s,
                ScopeKind::Module {
                    is_engine_internals: true,
                    ..
                }
            )
        });

        match &tokens[i] {
            Tok::Pound => {
                i = parse_tok_attr(&tokens, i, &mut attrs);
                continue;
            }
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        // Parse optional visibility (`pub` vs `pub(...)` vs private).
        let (vis, after_vis) = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                let mut j = i + 2;
                let mut depth = 1_usize;
                while j < tokens.len() && depth > 0 {
                    match tokens[j] {
                        Tok::LParen => depth += 1,
                        Tok::RParen => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                (Visibility::CrateOrRestricted, j)
            } else {
                (Visibility::Public, i + 1)
            }
        } else {
            (Visibility::Private, i)
        };

        // Track inherent `impl <Type> { ... }` and trait `impl <Trait> for <Type> { ... }`.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let header = &tokens[after_vis + 1..j];
            let (saw_for, _, self_type, mentions_host) = parse_impl_header(header, &alias_table);
            if mentions_host && saw_for {
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    "trait `impl` on `ClubHost`",
                    header,
                    &RAW_ENGINE_ONLY_TYPES,
                    &alias_table,
                )?;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(if !saw_for {
                    if let Some(tname) = self_type {
                        ScopeKind::InherentImpl {
                            type_name: tname,
                            cfg_test: attrs.cfg_test,
                        }
                    } else {
                        ScopeKind::Block {
                            cfg_test: attrs.cfg_test,
                        }
                    }
                } else {
                    ScopeKind::TraitImpl {
                        mentions_host,
                        self_type,
                        cfg_test: attrs.cfg_test,
                    }
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `mod <name> { ... }` or `mod <name>;`.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "mod")
            && let Some(Tok::Ident(mod_name)) = tokens.get(after_vis + 1)
        {
            let is_pub = matches!(vis, Visibility::Public);
            let mut j = after_vis + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let is_inline = j < tokens.len() && matches!(tokens[j], Tok::LBrace);

            if mod_name == "engine_internals" {
                if !is_lib_rs || !is_pub || !attrs.cfg_engine_internals || !attrs.doc_hidden {
                    return Err(format!(
                        "{rel_path}: `mod engine_internals` must be `#[cfg(feature = \"engine-internals\")] #[doc(hidden)] pub mod engine_internals` in lib.rs"
                    ));
                }
                seen_engine_internals_mod = true;
            } else if attrs.cfg_engine_internals {
                return Err(format!(
                    "{rel_path}: forbidden second `#[cfg(feature = \"engine-internals\")]` module `{mod_name}` (only `engine_internals` in lib.rs is allowed)"
                ));
            } else if is_lib_rs && is_pub && in_public_non_internal_mod {
                pub_mods.push(mod_name.clone());
            }

            if is_inline {
                scopes.push(ScopeKind::Module {
                    public: is_pub,
                    is_engine_internals: mod_name == "engine_internals",
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `pub use ...;`.
        if matches!(vis, Visibility::Public)
            && matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use")
        {
            let use_start = after_vis + 1;
            let mut j = use_start;
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                j += 1;
            }
            if in_public_non_internal_mod && !in_engine_internals_mod {
                let mut expanded = Vec::new();
                expand_use_tree(rel_path, &tokens[use_start..j], "", &mut expanded)?;
                for item in &expanded {
                    let path = item.canonical_path.as_str();
                    let leaf = path.rsplit("::").next().unwrap_or(path);
                    let resolved_leaf = alias_table.resolve(leaf);
                    if resolved_leaf.iter().any(|r| {
                        r == "SweepMode"
                            || r == "engine_internals"
                            || FORBIDDEN_RAW_FNS.contains(&r.as_str())
                    }) || path.contains("engine_internals")
                    {
                        return Err(format!(
                            "{rel_path}: public `pub use` re-exports restricted item `{path}` outside `engine_internals`"
                        ));
                    }
                    if let Some(alias_name) = &item.alias {
                        if let Some(raw) = resolved_leaf.iter().find(|r| {
                            RAW_ENGINE_ONLY_TYPES.contains(&r.as_str())
                                || matches!(r.as_str(), "ClubHost" | "ClubSaveSnapshot")
                        }) {
                            return Err(format!(
                                "{rel_path}: public `pub use {path} as {alias_name}` must not alias raw engine/host type `{raw}`"
                            ));
                        }
                        if is_lib_rs {
                            return Err(format!(
                                "{rel_path}: `pub use {path} as {alias_name}` aliases are forbidden at crate root"
                            ));
                        }
                    }
                    if !is_lib_rs
                        && let Some(raw) = resolved_leaf
                            .iter()
                            .find(|r| RAW_ENGINE_ONLY_TYPES.contains(&r.as_str()))
                    {
                        return Err(format!(
                            "{rel_path}: submodule `pub use` must not re-export raw engine type `{path}` (resolves to `{raw}`)"
                        ));
                    }
                    if is_lib_rs && matches!(scopes.last(), Some(ScopeKind::Root { .. })) {
                        lib_reexports.push(item.canonical_path.clone());
                    }
                }
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `[pub] [const|async|unsafe|extern "..."] fn <name>`.
        let mut fn_kw = after_vis;
        loop {
            match tokens.get(fn_kw) {
                Some(Tok::Ident(s))
                    if matches!(s.as_str(), "const" | "async" | "unsafe" | "extern") =>
                {
                    fn_kw += 1;
                    if s == "extern" && matches!(tokens.get(fn_kw), Some(Tok::Str(_))) {
                        fn_kw += 1;
                    }
                }
                _ => break,
            }
        }
        if matches!(tokens.get(fn_kw), Some(Tok::Ident(s)) if s == "fn")
            && let Some(Tok::Ident(fn_name)) = tokens.get(fn_kw + 1)
        {
            let sig_idx = skip_angle_generics(&tokens, fn_kw + 2);
            let (param_tokens, ret_start) = if matches!(tokens.get(sig_idx), Some(Tok::LParen)) {
                let close = find_matching_paren(&tokens, sig_idx);
                (&tokens[sig_idx + 1..close], close + 1)
            } else {
                (&[][..], sig_idx)
            };
            let mut j = ret_start;
            let mut arrow_pos = None;
            let mut where_pos = None;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                if arrow_pos.is_none()
                    && j + 1 < tokens.len()
                    && matches!(tokens[j], Tok::Other('-'))
                    && matches!(tokens[j + 1], Tok::Other('>'))
                {
                    arrow_pos = Some(j + 2);
                }
                if where_pos.is_none() && matches!(tokens[j], Tok::Ident(ref w) if w == "where") {
                    where_pos = Some(j);
                }
                j += 1;
            }
            let ret_slice = match arrow_pos {
                Some(rs) => {
                    let re = where_pos.unwrap_or(j).min(j);
                    if rs <= re { &tokens[rs..re] } else { &[] }
                }
                None => &[],
            };

            if in_host_trait_impl && !in_engine_internals_mod {
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    &format!("trait `impl` method `{fn_name}` parameter on `ClubHost`"),
                    param_tokens,
                    &RAW_ENGINE_ONLY_TYPES,
                    &alias_table,
                )?;
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    &format!("trait `impl` method `{fn_name}` return on `ClubHost`"),
                    ret_slice,
                    &RAW_ENGINE_ONLY_TYPES,
                    &alias_table,
                )?;
            }

            if matches!(vis, Visibility::Public) {
                if enclosing_impl == Some("TurnEngine")
                    && FORBIDDEN_RAW_FNS.contains(&fn_name.as_str())
                {
                    return Err(format!(
                        "{rel_path}: `TurnEngine::{fn_name}` must remain `pub(crate)`, never `pub` (even behind `engine-internals`, because `--workspace` feature unification would expose it)"
                    ));
                }
                if in_public_non_internal_mod && !in_engine_internals_mod {
                    let scope_key = enclosing_impl
                        .map(|t| format!("impl:{t}"))
                        .unwrap_or_else(|| format!("mod:{rel_path}"));
                    pub_fns_by_scope
                        .entry(scope_key.clone())
                        .or_default()
                        .push(fn_name.clone());
                    if FORBIDDEN_RAW_FNS.contains(&fn_name.as_str()) {
                        return Err(format!(
                            "{rel_path}: raw API `pub fn {fn_name}` must not be exposed as ordinary public API"
                        ));
                    }
                    let body_slice = if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                        let rbrace = find_matching_brace(&tokens, j);
                        &tokens[j + 1..rbrace]
                    } else {
                        &[]
                    };
                    let scope_label = enclosing_impl.unwrap_or("mod");
                    check_pub_fn_body_and_return(
                        rel_path,
                        scope_label,
                        fn_name,
                        ret_slice,
                        body_slice,
                        &alias_table,
                    )?;
                }
                if attrs.cfg_engine_internals && !in_engine_internals_mod {
                    return Err(format!(
                        "{rel_path}: `pub fn {fn_name}` uses `#[cfg(feature = \"engine-internals\")]` outside `engine_internals`"
                    ));
                }
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check associated `type <Assoc> = <Ty>;` inside a trait `impl` on `ClubHost` / `ClubSaveSnapshot`,
        // plus `pub type` aliases of raw types anywhere in `league-core`, plus `struct` pub fields.
        if let Some(Tok::Ident(item_kw)) = tokens.get(after_vis)
            && matches!(item_kw.as_str(), "enum" | "struct" | "type")
            && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
        {
            if item_kw == "type" {
                let mut j = after_vis + 2;
                while j < tokens.len() && !matches!(tokens[j], Tok::Semi | Tok::LBrace) {
                    j += 1;
                }
                let type_stmt = &tokens[after_vis + 2..j];
                let eq_start = skip_angle_generics(type_stmt, 0);
                if let Some(rel_eq) = type_stmt[eq_start..]
                    .iter()
                    .position(|t| matches!(t, Tok::Other('=')))
                {
                    let rhs = &type_stmt[eq_start + rel_eq + 1..];
                    if in_any_trait_impl && !in_engine_internals_mod {
                        check_tokens_no_raw_types_resolved(
                            rel_path,
                            &format!("trait `impl` associated `type {item_name}`"),
                            rhs,
                            &RAW_ENGINE_AND_HOST_TYPES,
                            &alias_table,
                        )?;
                    }
                    if matches!(vis, Visibility::Public) && !in_engine_internals_mod {
                        check_tokens_no_raw_types_resolved(
                            rel_path,
                            &format!("`pub type {item_name}` alias"),
                            rhs,
                            &RAW_ENGINE_AND_HOST_TYPES,
                            &alias_table,
                        )?;
                    }
                }
            }
            if matches!(vis, Visibility::Public)
                && item_name == "SweepMode"
                && in_public_non_internal_mod
                && !in_engine_internals_mod
            {
                return Err(format!(
                    "{rel_path}: `SweepMode` must not be publicly reachable outside `engine_internals`"
                ));
            }
            if item_kw == "struct" && !in_engine_internals_mod {
                let body_start = skip_angle_generics(&tokens, after_vis + 2);
                let mut k = body_start;
                while k < tokens.len()
                    && !matches!(tokens[k], Tok::LBrace | Tok::LParen | Tok::Semi)
                {
                    k += 1;
                }
                if k < tokens.len() {
                    check_struct_fields_not_public(rel_path, item_name, &tokens, k)?;
                    if matches!(vis, Visibility::Public) {
                        check_public_struct_no_raw_pub_fields(
                            rel_path,
                            item_name,
                            true,
                            &tokens,
                            k,
                            &alias_table,
                        )?;
                    }
                }
            }
            let mut j = after_vis + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }

    Ok(CoreFileScan {
        seen_engine_internals_mod,
        pub_mods,
        lib_reexports,
        pub_fns_by_scope,
    })
}

const TYPE_KEYWORDS_SKIP: [&str; 13] = [
    "mut", "const", "dyn", "impl", "fn", "for", "where", "pub", "crate", "super", "self", "Self",
    "as",
];

fn extract_generic_param_names(slice: &[Tok]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seg_start = 0_usize;
    let mut angle = 0_usize;
    let mut paren = 0_usize;
    let mut bracket = 0_usize;
    let mut segments: Vec<&[Tok]> = Vec::new();
    for (pos, tok) in slice.iter().enumerate() {
        match tok {
            Tok::Other('<') => angle += 1,
            Tok::Other('>')
                if !matches!(
                    slice.get(pos.saturating_sub(1)),
                    Some(Tok::Other('-') | Tok::Other('='))
                ) =>
            {
                angle = angle.saturating_sub(1);
            }
            Tok::LParen => paren += 1,
            Tok::RParen => paren = paren.saturating_sub(1),
            Tok::LBracket => bracket += 1,
            Tok::RBracket => bracket = bracket.saturating_sub(1),
            Tok::Other(',') if angle == 0 && paren == 0 && bracket == 0 => {
                if seg_start < pos {
                    segments.push(&slice[seg_start..pos]);
                }
                seg_start = pos + 1;
            }
            _ => {}
        }
    }
    if seg_start < slice.len() {
        segments.push(&slice[seg_start..]);
    }
    for seg in segments {
        if matches!(seg.first(), Some(Tok::Other('\''))) {
            continue;
        }
        if matches!(seg.first(), Some(Tok::Ident(s)) if s == "const") {
            continue;
        }
        if let Some(Tok::Ident(name)) = seg.first() {
            out.push(name.clone());
        }
    }
    out
}

fn collect_type_idents_from_tokens(
    tokens: &[Tok],
    out_leaf_idents: &mut Vec<String>,
    out_host_path_hit: &mut bool,
    out_forbidden_prefix: &mut Option<String>,
) {
    let mut bracket_stack: Vec<bool> = Vec::new(); // true => after `;` in `[T; N]`
    let mut prev_was_quote = false;
    let mut idx = 0_usize;
    while idx < tokens.len() {
        match &tokens[idx] {
            Tok::LBracket => {
                bracket_stack.push(false);
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            Tok::RBracket => {
                bracket_stack.pop();
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            Tok::Semi => {
                if let Some(last) = bracket_stack.last_mut() {
                    *last = true;
                }
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            _ => {}
        }
        if bracket_stack.last().copied().unwrap_or(false) {
            prev_was_quote = matches!(tokens[idx], Tok::Other('\''));
            idx += 1;
            continue;
        }
        if let Tok::Ident(ident) = &tokens[idx] {
            if prev_was_quote {
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            let is_path_prefix = matches!(
                (tokens.get(idx + 1), tokens.get(idx + 2)),
                (Some(Tok::Other(':')), Some(Tok::Other(':')))
            );
            if is_path_prefix {
                if ident == "host" {
                    *out_host_path_hit = true;
                }
                if FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str())
                    && out_forbidden_prefix.is_none()
                {
                    *out_forbidden_prefix = Some(ident.clone());
                }
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            let is_assoc_binding = matches!(tokens.get(idx + 1), Some(Tok::Other('=')))
                && !matches!(tokens.get(idx + 2), Some(Tok::Other('=')));
            if is_assoc_binding {
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            if !TYPE_KEYWORDS_SKIP.contains(&ident.as_str()) {
                out_leaf_idents.push(ident.clone());
            }
        }
        prev_was_quote = matches!(tokens[idx], Tok::Other('\''));
        idx += 1;
    }
}

fn split_top_level_comma(tokens: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut seg_start = 0_usize;
    let mut angle = 0_usize;
    let mut paren = 0_usize;
    let mut bracket = 0_usize;
    let mut brace = 0_usize;
    for (pos, tok) in tokens.iter().enumerate() {
        match tok {
            Tok::Other('<') => angle += 1,
            Tok::Other('>')
                if !matches!(
                    tokens.get(pos.saturating_sub(1)),
                    Some(Tok::Other('-') | Tok::Other('='))
                ) =>
            {
                angle = angle.saturating_sub(1);
            }
            Tok::LParen => paren += 1,
            Tok::RParen => paren = paren.saturating_sub(1),
            Tok::LBracket => bracket += 1,
            Tok::RBracket => bracket = bracket.saturating_sub(1),
            Tok::LBrace => brace += 1,
            Tok::RBrace => brace = brace.saturating_sub(1),
            Tok::Other(',') if angle == 0 && paren == 0 && bracket == 0 && brace == 0 => {
                if seg_start < pos {
                    out.push(&tokens[seg_start..pos]);
                }
                seg_start = pos + 1;
            }
            _ => {}
        }
    }
    if seg_start < tokens.len() {
        out.push(&tokens[seg_start..]);
    }
    out
}

struct SeatTypeDef {
    rel_path: String,
    name: String,
    generic_params: BTreeSet<String>,
    field_type_idents: Vec<String>,
    host_path_hit: bool,
    forbidden_prefix: Option<String>,
}

fn strip_leading_attrs_and_vis(mut tokens: &[Tok]) -> &[Tok] {
    loop {
        if matches!(tokens.first(), Some(Tok::Pound)) {
            let next = parse_tok_attr(tokens, 0, &mut AttrFlags::default());
            tokens = &tokens[next..];
            continue;
        }
        if matches!(tokens.first(), Some(Tok::Ident(s)) if s == "pub") {
            let mut next = 1_usize;
            if matches!(tokens.get(1), Some(Tok::LParen)) {
                next = find_matching_paren(tokens, 1) + 1;
            }
            tokens = &tokens[next..];
            continue;
        }
        break;
    }
    tokens
}

fn is_seat_src_type_and_fn_file(rel_path: &str, check_all_paths: bool) -> bool {
    check_all_paths
        || rel_path.starts_with("src/")
        || rel_path == "bot.rs"
        || rel_path.starts_with("bot/")
        || rel_path.starts_with("club/")
        || !rel_path.contains('/')
}

fn check_use_for_seat_code(rel_path: &str, use_tokens: &[Tok]) -> Result<(), String> {
    if use_tokens.iter().any(|t| matches!(t, Tok::Other('*'))) {
        return Err(format!(
            "{rel_path}: glob `use *` is forbidden in non-test seat code"
        ));
    }
    let mut expanded = Vec::new();
    expand_use_tree(rel_path, use_tokens, "", &mut expanded)?;
    for item in &expanded {
        if item
            .canonical_path
            .split("::")
            .any(|seg| seg == "host" || FORBIDDEN_SEAT_IDENTIFIERS.contains(&seg))
        {
            return Err(format!(
                "{rel_path}: seat code `use` references disallowed host or engine symbol `{}`",
                item.canonical_path
            ));
        }
    }
    Ok(())
}

fn collect_seat_defs_from_file(
    rel_path: &str,
    source: &str,
    check_all_paths: bool,
    defs: &mut Vec<SeatTypeDef>,
    violations: &mut Vec<String>,
) {
    let tokens = tokenize_rust(source);

    for tok in &tokens {
        if let Tok::Ident(ident) = tok
            && FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str())
        {
            violations.push(format!(
                "{rel_path}: seat module references forbidden engine-internal or host symbol `{ident}`"
            ));
        }
    }

    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut i = 0_usize;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });

        match &tokens[i] {
            Tok::Pound => {
                i = parse_tok_attr(&tokens, i, &mut attrs);
                continue;
            }
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        // Check `host::` in non-test expressions/paths even without `use`.
        if matches!(&tokens[i], Tok::Ident(s) if s == "host")
            && matches!(
                (tokens.get(i + 1), tokens.get(i + 2)),
                (Some(Tok::Other(':')), Some(Tok::Other(':')))
            )
        {
            violations.push(format!(
                "{rel_path}: seat code must not reference `host::` module path"
            ));
        }

        let after_vis = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                find_matching_paren(&tokens, i + 1) + 1
            } else {
                i + 1
            }
        } else {
            i
        };

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                j += 1;
            }
            if let Err(err) = check_use_for_seat_code(rel_path, &tokens[after_vis + 1..j]) {
                violations.push(err);
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Parse type definitions on `src/` / seat library files (and all non-host paths when enabled).
        if !is_seat_src_type_and_fn_file(rel_path, check_all_paths) {
            i += 1;
            continue;
        }

        if let Some(Tok::Ident(kw)) = tokens.get(after_vis)
            && matches!(kw.as_str(), "struct" | "enum" | "trait" | "type")
            && let Some(Tok::Ident(name)) = tokens.get(after_vis + 1)
        {
            let gen_start = after_vis + 2;
            let after_gen = skip_angle_generics(&tokens, gen_start);
            let generic_params: BTreeSet<String> = if after_gen > gen_start {
                extract_generic_param_names(&tokens[gen_start + 1..after_gen - 1])
                    .into_iter()
                    .collect()
            } else {
                BTreeSet::new()
            };

            let mut field_type_idents = Vec::new();
            let mut host_path_hit = false;
            let mut forbidden_prefix = None;

            if kw == "type" {
                let mut j = after_gen;
                while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                    j += 1;
                }
                let stmt = &tokens[after_gen..j];
                if let Some(eq_pos) = stmt.iter().position(|t| matches!(t, Tok::Other('='))) {
                    collect_type_idents_from_tokens(
                        &stmt[eq_pos + 1..],
                        &mut field_type_idents,
                        &mut host_path_hit,
                        &mut forbidden_prefix,
                    );
                }
                defs.push(SeatTypeDef {
                    rel_path: rel_path.to_owned(),
                    name: name.clone(),
                    generic_params,
                    field_type_idents,
                    host_path_hit,
                    forbidden_prefix,
                });
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if kw == "trait" {
                let mut j = after_gen;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                defs.push(SeatTypeDef {
                    rel_path: rel_path.to_owned(),
                    name: name.clone(),
                    generic_params,
                    field_type_idents,
                    host_path_hit,
                    forbidden_prefix,
                });
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if kw == "struct" {
                let mut j = after_gen;
                while j < tokens.len()
                    && !matches!(tokens[j], Tok::LBrace | Tok::LParen | Tok::Semi)
                {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    let close = find_matching_brace(&tokens, j);
                    for seg in split_top_level_comma(&tokens[j + 1..close]) {
                        let clean = strip_leading_attrs_and_vis(seg);
                        if let Some(colon_pos) =
                            clean.iter().position(|t| matches!(t, Tok::Other(':')))
                        {
                            collect_type_idents_from_tokens(
                                &clean[colon_pos + 1..],
                                &mut field_type_idents,
                                &mut host_path_hit,
                                &mut forbidden_prefix,
                            );
                        }
                    }
                    defs.push(SeatTypeDef {
                        rel_path: rel_path.to_owned(),
                        name: name.clone(),
                        generic_params,
                        field_type_idents,
                        host_path_hit,
                        forbidden_prefix,
                    });
                    attrs = AttrFlags::default();
                    i = close + 1;
                    continue;
                } else if j < tokens.len() && matches!(tokens[j], Tok::LParen) {
                    let close = find_matching_paren(&tokens, j);
                    for seg in split_top_level_comma(&tokens[j + 1..close]) {
                        let clean = strip_leading_attrs_and_vis(seg);
                        collect_type_idents_from_tokens(
                            clean,
                            &mut field_type_idents,
                            &mut host_path_hit,
                            &mut forbidden_prefix,
                        );
                    }
                    while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                        j += 1;
                    }
                    defs.push(SeatTypeDef {
                        rel_path: rel_path.to_owned(),
                        name: name.clone(),
                        generic_params,
                        field_type_idents,
                        host_path_hit,
                        forbidden_prefix,
                    });
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                } else {
                    defs.push(SeatTypeDef {
                        rel_path: rel_path.to_owned(),
                        name: name.clone(),
                        generic_params,
                        field_type_idents,
                        host_path_hit,
                        forbidden_prefix,
                    });
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
            }

            if kw == "enum" {
                let mut j = after_gen;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    let close = find_matching_brace(&tokens, j);
                    for seg in split_top_level_comma(&tokens[j + 1..close]) {
                        let clean = strip_leading_attrs_and_vis(seg);
                        if let Some(open_brace) =
                            clean.iter().position(|t| matches!(t, Tok::LBrace))
                        {
                            let v_close = find_matching_brace(clean, open_brace);
                            for fseg in split_top_level_comma(&clean[open_brace + 1..v_close]) {
                                let fclean = strip_leading_attrs_and_vis(fseg);
                                if let Some(cpos) =
                                    fclean.iter().position(|t| matches!(t, Tok::Other(':')))
                                {
                                    collect_type_idents_from_tokens(
                                        &fclean[cpos + 1..],
                                        &mut field_type_idents,
                                        &mut host_path_hit,
                                        &mut forbidden_prefix,
                                    );
                                }
                            }
                        } else if let Some(open_paren) =
                            clean.iter().position(|t| matches!(t, Tok::LParen))
                        {
                            let v_close = find_matching_paren(clean, open_paren);
                            for fseg in split_top_level_comma(&clean[open_paren + 1..v_close]) {
                                let fclean = strip_leading_attrs_and_vis(fseg);
                                collect_type_idents_from_tokens(
                                    fclean,
                                    &mut field_type_idents,
                                    &mut host_path_hit,
                                    &mut forbidden_prefix,
                                );
                            }
                        }
                    }
                    defs.push(SeatTypeDef {
                        rel_path: rel_path.to_owned(),
                        name: name.clone(),
                        generic_params,
                        field_type_idents,
                        host_path_hit,
                        forbidden_prefix,
                    });
                    attrs = AttrFlags::default();
                    i = close + 1;
                    continue;
                }
                defs.push(SeatTypeDef {
                    rel_path: rel_path.to_owned(),
                    name: name.clone(),
                    generic_params,
                    field_type_idents,
                    host_path_hit,
                    forbidden_prefix,
                });
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }
}

struct SeatFnCheck<'a> {
    rel_path: &'a str,
    enclosing_impl: Option<&'a str>,
    impl_generics: &'a BTreeSet<String>,
    fn_generics: &'a BTreeSet<String>,
    is_pub_fn: bool,
    fn_name: &'a str,
    params: &'a [Tok],
    ret_tokens: &'a [Tok],
    require_seat_obs_param_on_bot: bool,
    clean_local_types: &'a BTreeSet<String>,
    tainted_seat_types: &'a BTreeMap<String, String>,
}

fn check_seat_fn_signature(sig: &SeatFnCheck<'_>, violations: &mut Vec<String>) {
    let rel_path = sig.rel_path;
    let enclosing_impl = sig.enclosing_impl;
    let impl_generics = sig.impl_generics;
    let fn_generics = sig.fn_generics;
    let is_pub_fn = sig.is_pub_fn;
    let fn_name = sig.fn_name;
    let clean_local_types = sig.clean_local_types;
    let tainted_seat_types = sig.tainted_seat_types;

    let mut saw_primary_seat_param = false;
    for seg in split_top_level_comma(sig.params) {
        let clean = strip_leading_attrs_and_vis(seg);
        let Some(colon_pos) = clean.iter().position(|t| matches!(t, Tok::Other(':'))) else {
            // Check `&self`, `&mut self`, `self`, `mut self` receiver against `enclosing_impl`.
            let has_self_receiver = clean
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "self"));
            if has_self_receiver && let Some(impl_ty) = enclosing_impl {
                if let Some(reason) = tainted_seat_types.get(impl_ty) {
                    violations.push(format!(
                        "{rel_path}: seat `fn {impl_ty}::{fn_name}(&self)` accesses tainted type `{impl_ty}` ({reason})"
                    ));
                } else if !ALLOWED_SEAT_TYPES.contains(&impl_ty)
                    && !clean_local_types.contains(impl_ty)
                    && !impl_generics.contains(impl_ty)
                {
                    violations.push(format!(
                        "{rel_path}: seat `fn {impl_ty}::{fn_name}(&self)` accesses disallowed receiver type `{impl_ty}`"
                    ));
                }
            }
            continue;
        };

        let pat_tokens = &clean[..colon_pos];
        if pat_tokens
            .iter()
            .any(|t| matches!(t, Tok::Ident(s) if s == "self"))
            && let Some(impl_ty) = enclosing_impl
        {
            if let Some(reason) = tainted_seat_types.get(impl_ty) {
                violations.push(format!(
                    "{rel_path}: seat `fn {impl_ty}::{fn_name}(self)` accesses tainted type `{impl_ty}` ({reason})"
                ));
            } else if !ALLOWED_SEAT_TYPES.contains(&impl_ty)
                && !clean_local_types.contains(impl_ty)
                && !impl_generics.contains(impl_ty)
            {
                violations.push(format!(
                    "{rel_path}: seat `fn {impl_ty}::{fn_name}(self)` accesses disallowed receiver type `{impl_ty}`"
                ));
            }
        }

        let ty_tokens = &clean[colon_pos + 1..];
        let mut leaf_idents = Vec::new();
        let mut host_path_hit = false;
        let mut forbidden_prefix = None;
        collect_type_idents_from_tokens(
            ty_tokens,
            &mut leaf_idents,
            &mut host_path_hit,
            &mut forbidden_prefix,
        );
        if host_path_hit {
            violations.push(format!(
                "{rel_path}: seat `fn {fn_name}` parameter references forbidden `host` module"
            ));
        }
        if let Some(pref) = forbidden_prefix {
            violations.push(format!(
                "{rel_path}: seat `fn {fn_name}` parameter references forbidden symbol `{pref}`"
            ));
        }
        for ident in leaf_idents {
            if SEAT_PRIMARY_PARAM_TYPES.contains(&ident.as_str()) {
                saw_primary_seat_param = true;
            }
            if let Some(reason) = tainted_seat_types.get(&ident) {
                violations.push(format!(
                    "{rel_path}: seat `fn {fn_name}` parameter uses tainted type `{ident}` ({reason})"
                ));
            } else if !ALLOWED_SEAT_TYPES.contains(&ident.as_str())
                && !clean_local_types.contains(&ident)
                && !impl_generics.contains(&ident)
                && !fn_generics.contains(&ident)
            {
                violations.push(format!(
                    "{rel_path}: seat `fn {fn_name}` accepts disallowed parameter type `{ident}` (seat functions may take only SeatObservation, PublicSweepReceipt, or seat-safe previews)"
                ));
            }
        }
    }

    if sig.require_seat_obs_param_on_bot
        && is_pub_fn
        && enclosing_impl == Some("ClubBot")
        && !matches!(fn_name, "new" | "bot_tie_seed")
        && !saw_primary_seat_param
    {
        violations.push(format!(
            "{rel_path}: `ClubBot::{fn_name}` must take `&SeatObservation`, `PublicSweepReceipt`, or a seat-safe preview type"
        ));
    }

    let mut ret_idents = Vec::new();
    let mut ret_host_hit = false;
    let mut ret_forbidden_prefix = None;
    collect_type_idents_from_tokens(
        sig.ret_tokens,
        &mut ret_idents,
        &mut ret_host_hit,
        &mut ret_forbidden_prefix,
    );
    if ret_host_hit {
        violations.push(format!(
            "{rel_path}: seat `fn {fn_name}` return type references forbidden `host` module"
        ));
    }
    if let Some(pref) = ret_forbidden_prefix {
        violations.push(format!(
            "{rel_path}: seat `fn {fn_name}` return type references forbidden symbol `{pref}`"
        ));
    }
    for ident in ret_idents {
        if let Some(reason) = tainted_seat_types.get(&ident) {
            violations.push(format!(
                "{rel_path}: seat `fn {fn_name}` returns tainted type `{ident}` ({reason})"
            ));
        } else if !ALLOWED_SEAT_TYPES.contains(&ident.as_str())
            && !clean_local_types.contains(&ident)
            && !impl_generics.contains(&ident)
            && !fn_generics.contains(&ident)
        {
            violations.push(format!(
                "{rel_path}: seat `fn {fn_name}` returns disallowed type `{ident}`"
            ));
        }
    }
}

fn scan_seat_fns_in_file(
    rel_path: &str,
    source: &str,
    require_seat_obs_param_on_bot: bool,
    check_all_paths: bool,
    clean_local_types: &BTreeSet<String>,
    tainted_seat_types: &BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    if !is_seat_src_type_and_fn_file(rel_path, check_all_paths) {
        return;
    }
    let tokens = tokenize_rust(source);
    let mut scopes: Vec<(ScopeKind, BTreeSet<String>)> =
        vec![(ScopeKind::Root { public: true }, BTreeSet::new())];
    let mut attrs = AttrFlags::default();
    let mut i = 0_usize;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|(s, _)| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });

        match &tokens[i] {
            Tok::Pound => {
                i = parse_tok_attr(&tokens, i, &mut attrs);
                continue;
            }
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push((
                    ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    },
                    BTreeSet::new(),
                ));
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push((ScopeKind::Block { cfg_test: true }, BTreeSet::new()));
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        let after_vis = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                find_matching_paren(&tokens, i + 1) + 1
            } else {
                i + 1
            }
        } else {
            i
        };

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let header = &tokens[after_vis + 1..j];
            let (saw_for, impl_gens_vec, self_type, mentions_host) =
                parse_impl_header(header, &FileAliasTable::default());
            let impl_gens: BTreeSet<String> = impl_gens_vec.into_iter().collect();
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                let scope = match self_type {
                    Some(tname) if !saw_for => ScopeKind::InherentImpl {
                        type_name: tname,
                        cfg_test: attrs.cfg_test,
                    },
                    other => ScopeKind::TraitImpl {
                        mentions_host,
                        self_type: other,
                        cfg_test: attrs.cfg_test,
                    },
                };
                scopes.push((scope, impl_gens));
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        let mut fn_kw = after_vis;
        loop {
            match tokens.get(fn_kw) {
                Some(Tok::Ident(s))
                    if matches!(s.as_str(), "const" | "async" | "unsafe" | "extern") =>
                {
                    fn_kw += 1;
                    if s == "extern" && matches!(tokens.get(fn_kw), Some(Tok::Str(_))) {
                        fn_kw += 1;
                    }
                }
                _ => break,
            }
        }
        if matches!(tokens.get(fn_kw), Some(Tok::Ident(s)) if s == "fn")
            && let Some(Tok::Ident(fn_name)) = tokens.get(fn_kw + 1)
        {
            let gen_start = fn_kw + 2;
            let sig_idx = skip_angle_generics(&tokens, gen_start);
            let fn_generics: BTreeSet<String> = if sig_idx > gen_start {
                extract_generic_param_names(&tokens[gen_start + 1..sig_idx - 1])
                    .into_iter()
                    .collect()
            } else {
                BTreeSet::new()
            };
            let (params, after_paren) = if matches!(tokens.get(sig_idx), Some(Tok::LParen)) {
                let close = find_matching_paren(&tokens, sig_idx);
                (&tokens[sig_idx + 1..close], close + 1)
            } else {
                (&[][..], sig_idx)
            };
            let mut j = after_paren;
            let mut arrow_pos = None;
            let mut where_pos = None;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                if arrow_pos.is_none()
                    && j + 1 < tokens.len()
                    && matches!(tokens[j], Tok::Other('-'))
                    && matches!(tokens[j + 1], Tok::Other('>'))
                {
                    arrow_pos = Some(j + 2);
                }
                if where_pos.is_none() && matches!(tokens[j], Tok::Ident(ref w) if w == "where") {
                    where_pos = Some(j);
                }
                j += 1;
            }
            let ret_slice = match arrow_pos {
                Some(rs) => {
                    let re = where_pos.unwrap_or(j).min(j);
                    if rs <= re { &tokens[rs..re] } else { &[] }
                }
                None => &[],
            };

            let (enclosing_impl, enclosing_impl_generics) = scopes
                .iter()
                .rev()
                .find_map(|(s, gens)| match s {
                    ScopeKind::InherentImpl { type_name, .. } => {
                        Some((Some(type_name.as_str()), gens))
                    }
                    ScopeKind::TraitImpl {
                        self_type: Some(type_name),
                        ..
                    } => Some((Some(type_name.as_str()), gens)),
                    _ => None,
                })
                .map(|(ty, gens)| (ty, gens.clone()))
                .unwrap_or((None, BTreeSet::new()));

            let is_pub_fn = after_vis == i + 1;
            check_seat_fn_signature(
                &SeatFnCheck {
                    rel_path,
                    enclosing_impl,
                    impl_generics: &enclosing_impl_generics,
                    fn_generics: &fn_generics,
                    is_pub_fn,
                    fn_name,
                    params,
                    ret_tokens: ret_slice,
                    require_seat_obs_param_on_bot,
                    clean_local_types,
                    tainted_seat_types,
                },
                violations,
            );

            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push((
                    ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    },
                    BTreeSet::new(),
                ));
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }
}

fn scan_seat_files_two_pass(
    files: &[(String, String)],
    require_seat_obs_param_on_bot: bool,
    check_all_paths: bool,
) -> Result<(), String> {
    let mut defs = Vec::new();
    let mut violations = Vec::new();
    for (rel_path, source) in files {
        collect_seat_defs_from_file(
            rel_path,
            source,
            check_all_paths,
            &mut defs,
            &mut violations,
        );
    }

    let mut all_local_names: BTreeSet<String> = defs.iter().map(|d| d.name.clone()).collect();
    let mut tainted_seat_types: BTreeMap<String, String> = BTreeMap::new();

    for def in &defs {
        if def.host_path_hit {
            let msg = format!(
                "{}: seat struct/type `{}` references forbidden `host` module",
                def.rel_path, def.name
            );
            tainted_seat_types
                .entry(def.name.clone())
                .or_insert_with(|| msg.clone());
            violations.push(msg);
        }
        if let Some(pref) = &def.forbidden_prefix {
            let msg = format!(
                "{}: seat struct/type `{}` references forbidden symbol `{pref}`",
                def.rel_path, def.name
            );
            tainted_seat_types
                .entry(def.name.clone())
                .or_insert_with(|| msg.clone());
            violations.push(msg);
        }
        for field_ty in &def.field_type_idents {
            if !ALLOWED_SEAT_TYPES.contains(&field_ty.as_str())
                && !def.generic_params.contains(field_ty)
                && !all_local_names.contains(field_ty)
            {
                let msg = format!(
                    "{}: seat struct/type `{}` holds disallowed field/target type `{field_ty}`",
                    def.rel_path, def.name
                );
                tainted_seat_types
                    .entry(def.name.clone())
                    .or_insert_with(|| msg.clone());
                violations.push(msg);
            }
        }
    }

    // Propagate taint transitively so wrapper structs holding a tainted struct are also tainted.
    loop {
        let mut changed = false;
        for def in &defs {
            if tainted_seat_types.contains_key(&def.name) {
                continue;
            }
            for field_ty in &def.field_type_idents {
                if let Some(reason) = tainted_seat_types.get(field_ty).cloned() {
                    let msg = format!(
                        "{}: seat struct/type `{}` holds tainted field type `{field_ty}` ({reason})",
                        def.rel_path, def.name
                    );
                    tainted_seat_types.insert(def.name.clone(), msg.clone());
                    violations.push(msg);
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for tainted in tainted_seat_types.keys() {
        all_local_names.remove(tainted);
    }

    for (rel_path, source) in files {
        scan_seat_fns_in_file(
            rel_path,
            source,
            require_seat_obs_param_on_bot,
            check_all_paths,
            &all_local_names,
            &tainted_seat_types,
            &mut violations,
        );
    }

    if violations.is_empty() {
        Ok(())
    } else {
        let mut unique = Vec::new();
        for v in violations {
            if !unique.contains(&v) {
                unique.push(v);
            }
        }
        Err(unique.join("; "))
    }
}

fn scan_tui_host_module_source(rel_path: &str, source: &str) -> Result<(), String> {
    let tokens = tokenize_rust(source);
    let aliases = build_file_alias_table(rel_path, &tokens);
    let mut prev_was_quote = false;
    for tok in &tokens {
        if let Tok::Ident(ident) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            for resolved in aliases.resolve(ident) {
                if RAW_ENGINE_TYPES_FOR_HOST_MODULES.contains(&resolved.as_str()) {
                    if resolved == *ident {
                        return Err(format!(
                            "{rel_path}: TUI host module references forbidden raw engine symbol `{resolved}`"
                        ));
                    }
                    return Err(format!(
                        "{rel_path}: TUI host module references forbidden raw engine symbol `{resolved}` (via `{ident}`)"
                    ));
                }
            }
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }
    scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
        rel_path,
        false,
        source,
        true,
        &FileAliasTable::default(),
    )
}

#[allow(dead_code)]
fn scan_non_core_crate_aliases_and_reexports(
    rel_path: &str,
    is_lib_rs: bool,
    source: &str,
    forbid_host_reexports: bool,
) -> Result<(), String> {
    scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
        rel_path,
        is_lib_rs,
        source,
        forbid_host_reexports,
        &FileAliasTable::default(),
    )
}

fn scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
    rel_path: &str,
    is_lib_rs: bool,
    source: &str,
    forbid_host_reexports: bool,
    crate_aliases: &FileAliasTable,
) -> Result<(), String> {
    const NON_CORE_PUB_FN_BANNED_RETURNS: [&str; 10] = [
        "TurnEngine",
        "EngineState",
        "SweepMode",
        "SweepReceipt",
        "BundlePreview",
        "ScheduledShockSpec",
        "ClubHost",
        "ClubSaveSnapshot",
        "ClubSealPayload",
        "engine_internals",
    ];
    let tokens = tokenize_rust(source);
    let mut alias_table = build_file_alias_table(rel_path, &tokens);
    alias_table.merge_from(crate_aliases);
    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut i = 0_usize;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });
        let in_host_trait_impl = matches!(
            scopes.last(),
            Some(ScopeKind::TraitImpl {
                mentions_host: true,
                ..
            })
        );
        let in_any_trait_impl = matches!(scopes.last(), Some(ScopeKind::TraitImpl { .. }));

        match &tokens[i] {
            Tok::Pound => {
                i = parse_tok_attr(&tokens, i, &mut attrs);
                continue;
            }
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        let (vis, after_vis) = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                (
                    Visibility::CrateOrRestricted,
                    find_matching_paren(&tokens, i + 1) + 1,
                )
            } else {
                (Visibility::Public, i + 1)
            }
        } else {
            (Visibility::Private, i)
        };

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let header = &tokens[after_vis + 1..j];
            let (saw_for, _, self_type, mentions_host) = parse_impl_header(header, &alias_table);
            if mentions_host && saw_for {
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    "trait `impl` on `ClubHost`",
                    header,
                    &RAW_ENGINE_ONLY_TYPES,
                    &alias_table,
                )?;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(if !saw_for {
                    if let Some(tname) = self_type {
                        ScopeKind::InherentImpl {
                            type_name: tname,
                            cfg_test: attrs.cfg_test,
                        }
                    } else {
                        ScopeKind::Block {
                            cfg_test: attrs.cfg_test,
                        }
                    }
                } else {
                    ScopeKind::TraitImpl {
                        mentions_host,
                        self_type,
                        cfg_test: attrs.cfg_test,
                    }
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if matches!(vis, Visibility::Public)
            && matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use")
        {
            let use_start = after_vis + 1;
            let mut j = use_start;
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                j += 1;
            }
            let mut expanded = Vec::new();
            expand_use_tree(rel_path, &tokens[use_start..j], "", &mut expanded)?;
            for item in &expanded {
                let path = item.canonical_path.as_str();
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let resolved_leaf = alias_table.resolve(leaf);
                let matched_raw = resolved_leaf.iter().find(|r| {
                    let r_str = r.as_str();
                    if is_lib_rs || forbid_host_reexports {
                        RAW_ENGINE_AND_HOST_TYPES.contains(&r_str)
                            || FORBIDDEN_RAW_FNS.contains(&r_str)
                            || r_str == "engine_internals"
                    } else {
                        RAW_ENGINE_ONLY_TYPES.iter().any(|t| {
                            *t == r_str && *t != "SealedMandateDeal" && *t != "SealedEvalSeed"
                        }) || matches!(r_str, "ClubHost" | "ClubSaveSnapshot")
                            || FORBIDDEN_RAW_FNS.contains(&r_str)
                            || r_str == "engine_internals"
                    }
                });
                if let Some(raw) = matched_raw.cloned() {
                    let alias_note = item
                        .alias
                        .as_deref()
                        .map(|a| format!(" as {a}"))
                        .unwrap_or_default();
                    return Err(format!(
                        "{rel_path}: `pub use {path}{alias_note}` must not re-export raw engine or host item `{raw}`"
                    ));
                }
                if path.contains("engine_internals") {
                    return Err(format!(
                        "{rel_path}: `pub use {path}` must not re-export `engine_internals`"
                    ));
                }
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        let mut fn_kw = after_vis;
        loop {
            match tokens.get(fn_kw) {
                Some(Tok::Ident(s))
                    if matches!(s.as_str(), "const" | "async" | "unsafe" | "extern") =>
                {
                    fn_kw += 1;
                    if s == "extern" && matches!(tokens.get(fn_kw), Some(Tok::Str(_))) {
                        fn_kw += 1;
                    }
                }
                _ => break,
            }
        }
        if matches!(tokens.get(fn_kw), Some(Tok::Ident(s)) if s == "fn")
            && let Some(Tok::Ident(fn_name)) = tokens.get(fn_kw + 1)
        {
            let sig_idx = skip_angle_generics(&tokens, fn_kw + 2);
            let (param_tokens, ret_start) = if matches!(tokens.get(sig_idx), Some(Tok::LParen)) {
                let close = find_matching_paren(&tokens, sig_idx);
                (&tokens[sig_idx + 1..close], close + 1)
            } else {
                (&[][..], sig_idx)
            };
            let mut j = ret_start;
            let mut arrow_pos = None;
            let mut where_pos = None;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                if arrow_pos.is_none()
                    && j + 1 < tokens.len()
                    && matches!(tokens[j], Tok::Other('-'))
                    && matches!(tokens[j + 1], Tok::Other('>'))
                {
                    arrow_pos = Some(j + 2);
                }
                if where_pos.is_none() && matches!(tokens[j], Tok::Ident(ref w) if w == "where") {
                    where_pos = Some(j);
                }
                j += 1;
            }
            let ret_slice = match arrow_pos {
                Some(rs) => {
                    let re = where_pos.unwrap_or(j).min(j);
                    if rs <= re { &tokens[rs..re] } else { &[] }
                }
                None => &[],
            };
            if in_host_trait_impl {
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    &format!("trait `impl` method `{fn_name}` parameter on `ClubHost`"),
                    param_tokens,
                    &RAW_ENGINE_ONLY_TYPES,
                    &alias_table,
                )?;
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    &format!("trait `impl` method `{fn_name}` return on `ClubHost`"),
                    ret_slice,
                    &RAW_ENGINE_ONLY_TYPES,
                    &alias_table,
                )?;
            }
            if matches!(vis, Visibility::Public) {
                check_tokens_no_raw_types_resolved(
                    rel_path,
                    &format!("`pub fn {fn_name}` return type"),
                    ret_slice,
                    &NON_CORE_PUB_FN_BANNED_RETURNS,
                    &alias_table,
                )?;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if let Some(Tok::Ident(item_kw)) = tokens.get(after_vis)
            && matches!(item_kw.as_str(), "type" | "struct")
            && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
        {
            if item_kw == "type" {
                let mut j = after_vis + 2;
                while j < tokens.len() && !matches!(tokens[j], Tok::Semi | Tok::LBrace) {
                    j += 1;
                }
                let stmt = &tokens[after_vis + 2..j];
                let eq_start = skip_angle_generics(stmt, 0);
                if let Some(rel_eq) = stmt[eq_start..]
                    .iter()
                    .position(|t| matches!(t, Tok::Other('=')))
                {
                    let rhs = &stmt[eq_start + rel_eq + 1..];
                    if in_any_trait_impl {
                        check_tokens_no_raw_types_resolved(
                            rel_path,
                            &format!("trait `impl` associated `type {item_name}`"),
                            rhs,
                            &RAW_ENGINE_AND_HOST_TYPES,
                            &alias_table,
                        )?;
                    }
                    if matches!(vis, Visibility::Public) {
                        check_tokens_no_raw_types_resolved(
                            rel_path,
                            &format!("`pub type {item_name}` alias"),
                            rhs,
                            &RAW_ENGINE_AND_HOST_TYPES,
                            &alias_table,
                        )?;
                    }
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            // `struct`: check `pub` fields on public structs against raw types (resolved through aliases).
            let body_start = skip_angle_generics(&tokens, after_vis + 2);
            let mut k = body_start;
            while k < tokens.len() && !matches!(tokens[k], Tok::LBrace | Tok::LParen | Tok::Semi) {
                k += 1;
            }
            if k < tokens.len() && matches!(vis, Visibility::Public) {
                check_public_struct_no_raw_pub_fields(
                    rel_path,
                    item_name,
                    false,
                    &tokens,
                    k,
                    &alias_table,
                )?;
            }
            let mut j = after_vis + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                let close = find_matching_brace(&tokens, j);
                attrs = AttrFlags::default();
                i = close + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }

    Ok(())
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum SurfaceCheckMode {
    UiEntryParam,
    UiPubField,
    UiGeneral,
    SeatTypes,
}

fn check_no_non_lifetime_generics_in_crate(
    rel_path: &str,
    crate_name: &str,
    item_desc: &str,
    tokens: &[Tok],
    gen_start: usize,
    where_slice: &[Tok],
) -> Result<(), String> {
    let after_gen = skip_angle_generics(tokens, gen_start);
    if after_gen > gen_start {
        let gen_inner = &tokens[gen_start + 1..after_gen - 1];
        for seg in split_top_level_comma(gen_inner) {
            if !matches!(seg.first(), Some(Tok::Other('\''))) {
                let hint = seg
                    .iter()
                    .find_map(|t| match t {
                        Tok::Ident(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("<param>");
                return Err(format!(
                    "{rel_path}: `{item_desc}` must not declare generic type or const parameter `{hint}` in `{crate_name}` (lifetimes only)"
                ));
            }
        }
    }
    let mut prev_was_quote = false;
    for tok in where_slice {
        if let Tok::Ident(id) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            return Err(format!(
                "{rel_path}: `{item_desc}` must not declare non-lifetime `where` bound referencing `{id}` in `{crate_name}`"
            ));
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }
    Ok(())
}

fn parse_allowed_seat_types_fn_generics(
    rel_path: &str,
    item_desc: &str,
    tokens: &[Tok],
    gen_start: usize,
    where_slice: &[Tok],
) -> Result<BTreeSet<String>, String> {
    const ALLOWED_SEAT_TYPES_TRAIT_BOUNDS: [&str; 3] =
        ["Serializer", "Deserializer", "ParitySpecSource"];
    let mut allowed_fn_generics = BTreeSet::new();
    let after_gen = skip_angle_generics(tokens, gen_start);
    if after_gen > gen_start {
        let gen_inner = &tokens[gen_start + 1..after_gen - 1];
        for seg in split_top_level_comma(gen_inner) {
            if matches!(seg.first(), Some(Tok::Other('\''))) {
                continue;
            }
            if matches!(seg.first(), Some(Tok::Ident(s)) if s == "const") {
                return Err(format!(
                    "{rel_path}: `{item_desc}` must not declare const generic parameter in `league-seat-types`"
                ));
            }
            let Some(Tok::Ident(param_name)) = seg.first() else {
                return Err(format!(
                    "{rel_path}: `{item_desc}` has unsupported generic parameter syntax in `league-seat-types`"
                ));
            };
            let Some(colon_pos) = seg.iter().position(|t| matches!(t, Tok::Other(':'))) else {
                return Err(format!(
                    "{rel_path}: `{item_desc}` must not declare unbounded generic parameter `{param_name}` in `league-seat-types`"
                ));
            };
            let mut saw_allowed_bound = false;
            let mut prev_was_quote = false;
            for tok in &seg[colon_pos + 1..] {
                if let Tok::Ident(bound_id) = tok {
                    if prev_was_quote {
                        prev_was_quote = false;
                        continue;
                    }
                    if ALLOWED_SEAT_TYPES_TRAIT_BOUNDS.contains(&bound_id.as_str()) {
                        saw_allowed_bound = true;
                    } else if bound_id != "Sized" {
                        return Err(format!(
                            "{rel_path}: `{item_desc}` uses forbidden generic bound `{bound_id}` on `{param_name}` in `league-seat-types`"
                        ));
                    }
                }
                prev_was_quote = matches!(tok, Tok::Other('\''));
            }
            if !saw_allowed_bound {
                return Err(format!(
                    "{rel_path}: `{item_desc}` must not declare generic type parameter `{param_name}` without `Serializer`/`Deserializer`/`ParitySpecSource` bound in `league-seat-types`"
                ));
            }
            allowed_fn_generics.insert(param_name.clone());
        }
    }
    let mut prev_was_quote = false;
    for tok in where_slice {
        if let Tok::Ident(id) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            return Err(format!(
                "{rel_path}: `{item_desc}` must not declare `where` clause referencing `{id}` in `league-seat-types`"
            ));
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }
    Ok(allowed_fn_generics)
}

fn detect_fn_pointer_at(ty_tokens: &[Tok], idx: usize) -> Option<String> {
    let Tok::Ident(first) = ty_tokens.get(idx)? else {
        return None;
    };
    if first == "fn" {
        return Some("fn".to_owned());
    }
    if first == "unsafe" {
        match ty_tokens.get(idx + 1) {
            Some(Tok::Ident(s)) if s == "fn" => return Some("unsafe fn".to_owned()),
            Some(Tok::Ident(s)) if s == "extern" => {
                let (fn_pos, abi) = match ty_tokens.get(idx + 2) {
                    Some(Tok::Str(abi)) => (idx + 3, Some(abi.as_str())),
                    _ => (idx + 2, None),
                };
                if matches!(ty_tokens.get(fn_pos), Some(Tok::Ident(f)) if f == "fn") {
                    return Some(match abi {
                        Some(a) => format!("unsafe extern \"{a}\" fn"),
                        None => "unsafe extern fn".to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    if first == "extern" {
        let (fn_pos, abi) = match ty_tokens.get(idx + 1) {
            Some(Tok::Str(abi)) => (idx + 2, Some(abi.as_str())),
            _ => (idx + 1, None),
        };
        if matches!(ty_tokens.get(fn_pos), Some(Tok::Ident(f)) if f == "fn") {
            return Some(match abi {
                Some(a) => format!("extern \"{a}\" fn"),
                None => "extern fn".to_owned(),
            });
        }
    }
    None
}

fn is_forbidden_interior_mutability_type(ident: &str) -> bool {
    matches!(
        ident,
        "Mutex"
            | "MutexGuard"
            | "RwLock"
            | "RwLockReadGuard"
            | "RwLockWriteGuard"
            | "OnceLock"
            | "LazyLock"
            | "Once"
            | "Condvar"
            | "Barrier"
            | "Cell"
            | "RefCell"
            | "UnsafeCell"
            | "OnceCell"
            | "LazyCell"
            | "Arc"
            | "Rc"
            | "Weak"
            | "AtomicBool"
            | "AtomicI8"
            | "AtomicI16"
            | "AtomicI32"
            | "AtomicI64"
            | "AtomicI128"
            | "AtomicIsize"
            | "AtomicU8"
            | "AtomicU16"
            | "AtomicU32"
            | "AtomicU64"
            | "AtomicU128"
            | "AtomicUsize"
            | "AtomicPtr"
    )
}

struct SurfaceCheckEnv<'a> {
    crate_name: &'a str,
    local_types: &'a BTreeSet<String>,
    allowed_types: &'a [&'a str],
    allowed_fn_generics: &'a BTreeSet<String>,
}

fn check_seat_surface_type_tokens(
    rel_path: &str,
    ctx: &str,
    ty_tokens: &[Tok],
    env: &SurfaceCheckEnv<'_>,
    mode: SurfaceCheckMode,
) -> Result<(), String> {
    let crate_name = env.crate_name;
    let local_types = env.local_types;
    let allowed_types = env.allowed_types;
    let allowed_fn_generics = env.allowed_fn_generics;
    let mut bracket_stack: Vec<bool> = Vec::new();
    let mut prev_was_quote = false;
    let mut idx = 0_usize;
    while idx < ty_tokens.len() {
        match &ty_tokens[idx] {
            Tok::LBracket => {
                if matches!(
                    mode,
                    SurfaceCheckMode::UiEntryParam | SurfaceCheckMode::UiPubField
                ) {
                    return Err(format!(
                        "{rel_path}: {ctx} must not accept slice or array buffer type in `league-seat-ui` (`Part B` rule)"
                    ));
                }
                bracket_stack.push(false);
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            Tok::LParen => {
                if matches!(
                    mode,
                    SurfaceCheckMode::UiEntryParam | SurfaceCheckMode::UiPubField
                ) {
                    return Err(format!(
                        "{rel_path}: {ctx} must not accept tuple type in `league-seat-ui` (`Part B` rule)"
                    ));
                }
            }
            Tok::RBracket => {
                bracket_stack.pop();
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            Tok::Semi => {
                if let Some(last) = bracket_stack.last_mut() {
                    *last = true;
                }
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            Tok::Other('*') if !bracket_stack.last().copied().unwrap_or(false) => {
                return Err(format!(
                    "{rel_path}: {ctx} uses forbidden raw pointer type in `{crate_name}`"
                ));
            }
            _ => {}
        }
        if bracket_stack.last().copied().unwrap_or(false) {
            prev_was_quote = matches!(ty_tokens[idx], Tok::Other('\''));
            idx += 1;
            continue;
        }
        if let Tok::Ident(ident) = &ty_tokens[idx] {
            if prev_was_quote {
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            if let Some(fn_desc) = detect_fn_pointer_at(ty_tokens, idx) {
                return Err(format!(
                    "{rel_path}: {ctx} uses forbidden function pointer type `{fn_desc}` in `{crate_name}`"
                ));
            }
            if matches!(
                ident.as_str(),
                "dyn" | "impl" | "Fn" | "FnMut" | "FnOnce" | "Box" | "Any"
            ) {
                return Err(format!(
                    "{rel_path}: {ctx} uses forbidden non-concrete/trait/closure type `{ident}` in `{crate_name}`"
                ));
            }
            if is_forbidden_interior_mutability_type(ident) {
                return Err(format!(
                    "{rel_path}: {ctx} uses forbidden interior-mutability/shared-state type `{ident}` in `{crate_name}`"
                ));
            }
            if matches!(
                ident.as_str(),
                "league_core"
                    | "league_gen"
                    | "league_standalone"
                    | "league_tui"
                    | "host"
                    | "engine_internals"
                    | "fs"
                    | "env"
                    | "process"
                    | "net"
                    | "path"
                    | "cell"
                    | "atomic"
                    | "mpsc"
            ) {
                return Err(format!(
                    "{rel_path}: {ctx} references forbidden crate/module `{ident}` in `{crate_name}`"
                ));
            }
            if matches!(
                mode,
                SurfaceCheckMode::UiEntryParam | SurfaceCheckMode::UiPubField
            ) && matches!(
                ident.as_str(),
                "str" | "String" | "char" | "Cow" | "OsStr" | "OsString" | "CStr" | "CString"
            ) {
                return Err(format!(
                    "{rel_path}: {ctx} must not accept free `{ident}` from host in `league-seat-ui` (`Part B` rule: entry points take `league-seat-types` values and seat-ui-local UI state only)"
                ));
            }
            let is_path_prefix = matches!(
                (ty_tokens.get(idx + 1), ty_tokens.get(idx + 2)),
                (Some(Tok::Other(':')), Some(Tok::Other(':')))
            );
            if is_path_prefix {
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            if FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str()) {
                return Err(format!(
                    "{rel_path}: {ctx} references forbidden raw engine/host symbol `{ident}`"
                ));
            }
            let is_assoc_binding = matches!(ty_tokens.get(idx + 1), Some(Tok::Other('=')))
                && !matches!(ty_tokens.get(idx + 2), Some(Tok::Other('=')));
            if is_assoc_binding {
                prev_was_quote = false;
                idx += 1;
                continue;
            }
            if !TYPE_KEYWORDS_SKIP.contains(&ident.as_str())
                && !allowed_types.contains(&ident.as_str())
                && !local_types.contains(ident.as_str())
                && !allowed_fn_generics.contains(ident.as_str())
            {
                return Err(format!(
                    "{rel_path}: {ctx} uses non-allow-listed type `{ident}` in `{crate_name}` (concrete allow-listed types only)"
                ));
            }
        }
        prev_was_quote = matches!(ty_tokens[idx], Tok::Other('\''));
        idx += 1;
    }
    Ok(())
}

fn collect_seat_local_types(files: &[(String, String)], include_traits: bool) -> BTreeSet<String> {
    let mut local_types = BTreeSet::new();
    for (_, source) in files {
        let tokens = tokenize_rust(source);
        let mut scopes = vec![ScopeKind::Root { public: true }];
        let mut attrs = AttrFlags::default();
        let mut i = 0_usize;
        while i < tokens.len() {
            let in_cfg_test = scopes.iter().any(|s| match s {
                ScopeKind::Root { .. } => false,
                ScopeKind::Module { cfg_test, .. }
                | ScopeKind::InherentImpl { cfg_test, .. }
                | ScopeKind::TraitImpl { cfg_test, .. }
                | ScopeKind::Block { cfg_test } => *cfg_test,
            });
            match &tokens[i] {
                Tok::Pound => {
                    i = parse_tok_attr(&tokens, i, &mut attrs);
                    continue;
                }
                Tok::RBrace => {
                    if scopes.len() > 1 {
                        scopes.pop();
                    }
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::LBrace => {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::Semi => {
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if in_cfg_test || attrs.cfg_test {
                if matches!(
                    tokens[i],
                    Tok::Ident(ref s)
                        if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
                ) {
                    let mut j = i + 1;
                    while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                        j += 1;
                    }
                    if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                        scopes.push(ScopeKind::Block { cfg_test: true });
                        attrs = AttrFlags::default();
                        i = j + 1;
                        continue;
                    }
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
                i += 1;
                continue;
            }
            let after_vis = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                    find_matching_paren(&tokens, i + 1) + 1
                } else {
                    i + 1
                }
            } else {
                i
            };
            if let Some(Tok::Ident(kw)) = tokens.get(after_vis)
                && (matches!(kw.as_str(), "struct" | "enum") || (include_traits && kw == "trait"))
                && let Some(Tok::Ident(name)) = tokens.get(after_vis + 1)
            {
                local_types.insert(name.clone());
                attrs = AttrFlags::default();
                i = after_vis + 2;
                continue;
            }
            if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                attrs = AttrFlags::default();
            }
            i += 1;
        }
    }
    local_types
}

fn find_stmt_semi(tokens: &[Tok], start: usize) -> usize {
    let mut brace = 0_usize;
    let mut paren = 0_usize;
    let mut bracket = 0_usize;
    let mut j = start;
    while j < tokens.len() {
        match tokens[j] {
            Tok::LBrace => brace += 1,
            Tok::RBrace => {
                if brace == 0 {
                    break;
                }
                brace -= 1;
            }
            Tok::LParen => paren += 1,
            Tok::RParen => paren = paren.saturating_sub(1),
            Tok::LBracket => bracket += 1,
            Tok::RBracket => bracket = bracket.saturating_sub(1),
            Tok::Semi if brace == 0 && paren == 0 && bracket == 0 => break,
            _ => {}
        }
        j += 1;
    }
    j
}

fn strip_leading_attrs_only(mut seg: &[Tok]) -> &[Tok] {
    while matches!(seg.first(), Some(Tok::Pound)) {
        if matches!(seg.get(1), Some(Tok::LBracket)) {
            let mut d = 1_usize;
            let mut k = 2_usize;
            while k < seg.len() && d > 0 {
                match seg[k] {
                    Tok::LBracket => d += 1,
                    Tok::RBracket => d -= 1,
                    _ => {}
                }
                k += 1;
            }
            seg = &seg[k.min(seg.len())..];
        } else {
            break;
        }
    }
    seg
}

fn has_unrestricted_pub(seg: &[Tok]) -> bool {
    let rest = strip_leading_attrs_only(seg);
    matches!(rest.first(), Some(Tok::Ident(s)) if s == "pub")
        && !matches!(rest.get(1), Some(Tok::LParen))
}

fn check_seat_ui_public_surface(
    rel_path: &str,
    source: &str,
    local_types: &BTreeSet<String>,
) -> Result<(), String> {
    let tokens = tokenize_rust(source);
    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let empty_generics = BTreeSet::new();
    let mut i = 0_usize;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });

        if matches!(&tokens[i], Tok::Pound) {
            if tokens.get(i + 1) == Some(&Tok::LBracket) {
                let mut d = 1_usize;
                let mut j = i + 2;
                while j < tokens.len() && d > 0 {
                    match tokens[j] {
                        Tok::LBracket => d += 1,
                        Tok::RBracket => d -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let attr_inner = &tokens[i + 2..j.saturating_sub(1)];
                if attr_inner
                    .iter()
                    .any(|t| matches!(t, Tok::Ident(s) if s == "macro_export"))
                {
                    return Err(format!(
                        "{rel_path}: `#[macro_export]` is forbidden in `league-seat-ui`"
                    ));
                }
            }
            i = parse_tok_attr(&tokens, i, &mut attrs);
            continue;
        }

        match &tokens[i] {
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        if let Tok::Ident(ident) = &tokens[i] {
            if FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str()) {
                return Err(format!(
                    "{rel_path}: `league-seat-ui` references forbidden engine/host symbol `{ident}`"
                ));
            }
            // Ban ALL `type` items/aliases (`pub` or private, top-level or nested) in `league-seat-ui`.
            if ident == "type" {
                return Err(format!(
                    "{rel_path}: `type` items and aliases are forbidden in `league-seat-ui`"
                ));
            }
        }

        let (vis, after_vis) = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                (
                    Visibility::CrateOrRestricted,
                    find_matching_paren(&tokens, i + 1) + 1,
                )
            } else {
                (Visibility::Public, i + 1)
            }
        } else {
            (Visibility::Private, i)
        };

        // Ban ALL `use ... as ...` renames outside `#[cfg(test)]` in `league-seat-ui`.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
            let j = find_stmt_semi(&tokens, after_vis + 1);
            let use_tokens = &tokens[after_vis + 1..j];
            if use_tokens
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "as"))
            {
                return Err(format!(
                    "{rel_path}: `use ... as` rename is forbidden in `league-seat-ui`"
                ));
            }
            if matches!(vis, Visibility::Public) {
                let mut expanded = Vec::new();
                expand_use_tree(rel_path, use_tokens, "", &mut expanded)?;
                for item in &expanded {
                    if item.canonical_path.split("::").any(|seg| {
                        matches!(
                            seg,
                            "league_core"
                                | "league_gen"
                                | "league_standalone"
                                | "league_tui"
                                | "host"
                        )
                    }) {
                        return Err(format!(
                            "{rel_path}: `pub use {}` is forbidden in `league-seat-ui`",
                            item.canonical_path
                        ));
                    }
                }
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if matches!(vis, Visibility::Public)
            && (matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "trait")
                || (matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "unsafe")
                    && matches!(tokens.get(after_vis + 1), Some(Tok::Ident(s)) if s == "trait")))
        {
            return Err(format!(
                "{rel_path}: `pub trait` is forbidden in `league-seat-ui` public API"
            ));
        }

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let header = &tokens[after_vis + 1..j];
            let where_slice = header
                .iter()
                .position(|t| matches!(t, Tok::Ident(w) if w == "where"))
                .map(|wp| &header[wp + 1..])
                .unwrap_or(&[]);
            check_no_non_lifetime_generics_in_crate(
                rel_path,
                "league-seat-ui",
                "impl block",
                &tokens,
                after_vis + 1,
                where_slice,
            )?;
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        let mut fn_kw = after_vis;
        loop {
            match tokens.get(fn_kw) {
                Some(Tok::Ident(s))
                    if matches!(s.as_str(), "const" | "async" | "unsafe" | "extern") =>
                {
                    fn_kw += 1;
                    if s == "extern" && matches!(tokens.get(fn_kw), Some(Tok::Str(_))) {
                        fn_kw += 1;
                    }
                }
                _ => break,
            }
        }
        if matches!(tokens.get(fn_kw), Some(Tok::Ident(s)) if s == "fn")
            && let Some(Tok::Ident(fn_name)) = tokens.get(fn_kw + 1)
        {
            let gen_start = fn_kw + 2;
            let sig_idx = skip_angle_generics(&tokens, gen_start);
            let (params, ret_start) = if matches!(tokens.get(sig_idx), Some(Tok::LParen)) {
                let close = find_matching_paren(&tokens, sig_idx);
                (&tokens[sig_idx + 1..close], close + 1)
            } else {
                (&[][..], sig_idx)
            };
            let mut j = ret_start;
            let mut arrow_pos = None;
            let mut where_pos = None;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                if arrow_pos.is_none()
                    && j + 1 < tokens.len()
                    && matches!(tokens[j], Tok::Other('-'))
                    && matches!(tokens[j + 1], Tok::Other('>'))
                {
                    arrow_pos = Some(j + 2);
                }
                if where_pos.is_none() && matches!(tokens[j], Tok::Ident(ref w) if w == "where") {
                    where_pos = Some(j);
                }
                j += 1;
            }
            let where_slice = where_pos.map(|wp| &tokens[wp + 1..j]).unwrap_or(&[]);
            check_no_non_lifetime_generics_in_crate(
                rel_path,
                "league-seat-ui",
                &format!("fn {fn_name}"),
                &tokens,
                gen_start,
                where_slice,
            )?;
            let ret_slice = match arrow_pos {
                Some(rs) => {
                    let re = where_pos.unwrap_or(j).min(j);
                    if rs <= re { &tokens[rs..re] } else { &[] }
                }
                None => &[],
            };
            let (param_mode, vis_label) = if matches!(vis, Visibility::Public) {
                (SurfaceCheckMode::UiEntryParam, "pub fn")
            } else {
                (SurfaceCheckMode::UiGeneral, "fn")
            };
            let ui_env = SurfaceCheckEnv {
                crate_name: "league-seat-ui",
                local_types,
                allowed_types: if param_mode == SurfaceCheckMode::UiEntryParam {
                    ALLOWED_SEAT_UI_ENTRY_PARAM_TYPES
                } else {
                    ALLOWED_SEAT_UI_PUBLIC_TYPES
                },
                allowed_fn_generics: &empty_generics,
            };
            let ui_ret_env = SurfaceCheckEnv {
                crate_name: "league-seat-ui",
                local_types,
                allowed_types: ALLOWED_SEAT_UI_PUBLIC_TYPES,
                allowed_fn_generics: &empty_generics,
            };
            let mut key_input_param_count = 0_usize;
            for seg in split_top_level_comma(params) {
                let clean = strip_leading_attrs_and_vis(seg);
                if let Some(colon_pos) = clean.iter().position(|t| matches!(t, Tok::Other(':'))) {
                    let param_ty = &clean[colon_pos + 1..];
                    check_seat_surface_type_tokens(
                        rel_path,
                        &format!("`{vis_label} {fn_name}` parameter"),
                        param_ty,
                        &ui_env,
                        param_mode,
                    )?;
                    if param_mode == SurfaceCheckMode::UiEntryParam
                        && param_ty.iter().any(|t| {
                            matches!(
                                t,
                                Tok::Ident(s)
                                    if matches!(
                                        s.as_str(),
                                        "SeatKeyEvent" | "SeatKeyCode" | "SeatKeyModifiers"
                                    )
                            )
                        })
                    {
                        key_input_param_count += 1;
                        if key_input_param_count > 1 {
                            return Err(format!(
                                "{rel_path}: `{vis_label} {fn_name}` must accept at most one single key/mouse event parameter (`Part B` rule)"
                            ));
                        }
                    }
                }
            }
            if !ret_slice.is_empty() {
                check_seat_surface_type_tokens(
                    rel_path,
                    &format!("`{vis_label} {fn_name}` return type"),
                    ret_slice,
                    &ui_ret_env,
                    SurfaceCheckMode::UiGeneral,
                )?;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "const" || s == "static") {
            let is_static = matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "static");
            let mut k = after_vis + 1;
            if matches!(tokens.get(k), Some(Tok::Ident(s)) if s == "mut") {
                k += 1;
            }
            let item_name = match tokens.get(k) {
                Some(Tok::Ident(n)) => n.as_str(),
                _ => "<const>",
            };
            let j = find_stmt_semi(&tokens, k);
            let stmt = &tokens[k..j];
            if let Some(colon_pos) = stmt.iter().position(|t| matches!(t, Tok::Other(':'))) {
                let end_ty = stmt[colon_pos + 1..]
                    .iter()
                    .position(|t| matches!(t, Tok::Other('=')))
                    .map(|p| colon_pos + 1 + p)
                    .unwrap_or(stmt.len());
                let vis_prefix = if matches!(vis, Visibility::Public) {
                    "pub "
                } else {
                    ""
                };
                let ui_env = SurfaceCheckEnv {
                    crate_name: "league-seat-ui",
                    local_types,
                    allowed_types: ALLOWED_SEAT_UI_PUBLIC_TYPES,
                    allowed_fn_generics: &empty_generics,
                };
                check_seat_surface_type_tokens(
                    rel_path,
                    &format!(
                        "`{vis_prefix}{} {item_name}`",
                        if is_static { "static" } else { "const" }
                    ),
                    &stmt[colon_pos + 1..end_ty],
                    &ui_env,
                    SurfaceCheckMode::UiGeneral,
                )?;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if let Some(Tok::Ident(item_kw)) = tokens.get(after_vis)
            && matches!(item_kw.as_str(), "struct" | "enum")
            && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
        {
            let gen_start = after_vis + 2;
            let after_gen = skip_angle_generics(&tokens, gen_start);
            let mut body_pos = after_gen;
            let mut where_pos = None;
            while body_pos < tokens.len()
                && !matches!(tokens[body_pos], Tok::LBrace | Tok::LParen | Tok::Semi)
            {
                if where_pos.is_none()
                    && matches!(tokens[body_pos], Tok::Ident(ref w) if w == "where")
                {
                    where_pos = Some(body_pos);
                }
                body_pos += 1;
            }
            let where_slice = where_pos.map(|wp| &tokens[wp + 1..body_pos]).unwrap_or(&[]);
            check_no_non_lifetime_generics_in_crate(
                rel_path,
                "league-seat-ui",
                &format!("{item_kw} {item_name}"),
                &tokens,
                gen_start,
                where_slice,
            )?;

            let ui_env = SurfaceCheckEnv {
                crate_name: "league-seat-ui",
                local_types,
                allowed_types: ALLOWED_SEAT_UI_PUBLIC_TYPES,
                allowed_fn_generics: &empty_generics,
            };
            let ui_pub_field_env = SurfaceCheckEnv {
                crate_name: "league-seat-ui",
                local_types,
                allowed_types: ALLOWED_SEAT_UI_PUB_FIELD_TYPES,
                allowed_fn_generics: &empty_generics,
            };
            if item_kw == "struct" && body_pos < tokens.len() {
                if matches!(tokens[body_pos], Tok::LBrace) {
                    let close = find_matching_brace(&tokens, body_pos);
                    for seg in split_top_level_comma(&tokens[body_pos + 1..close]) {
                        let fmode =
                            if matches!(vis, Visibility::Public) && has_unrestricted_pub(seg) {
                                SurfaceCheckMode::UiPubField
                            } else {
                                SurfaceCheckMode::UiGeneral
                            };
                        let field_env = if fmode == SurfaceCheckMode::UiPubField {
                            &ui_pub_field_env
                        } else {
                            &ui_env
                        };
                        let clean = strip_leading_attrs_and_vis(seg);
                        if let Some(cpos) = clean.iter().position(|t| matches!(t, Tok::Other(':')))
                        {
                            check_seat_surface_type_tokens(
                                rel_path,
                                &format!("struct `{item_name}` field"),
                                &clean[cpos + 1..],
                                field_env,
                                fmode,
                            )?;
                        }
                    }
                    attrs = AttrFlags::default();
                    i = close + 1;
                    continue;
                } else if matches!(tokens[body_pos], Tok::LParen) {
                    let close = find_matching_paren(&tokens, body_pos);
                    for seg in split_top_level_comma(&tokens[body_pos + 1..close]) {
                        let fmode =
                            if matches!(vis, Visibility::Public) && has_unrestricted_pub(seg) {
                                SurfaceCheckMode::UiPubField
                            } else {
                                SurfaceCheckMode::UiGeneral
                            };
                        let field_env = if fmode == SurfaceCheckMode::UiPubField {
                            &ui_pub_field_env
                        } else {
                            &ui_env
                        };
                        let clean = strip_leading_attrs_and_vis(seg);
                        check_seat_surface_type_tokens(
                            rel_path,
                            &format!("struct `{item_name}` tuple field"),
                            clean,
                            field_env,
                            fmode,
                        )?;
                    }
                    let semi = find_stmt_semi(&tokens, close + 1);
                    attrs = AttrFlags::default();
                    i = (semi + 1).min(tokens.len());
                    continue;
                }
            }

            if item_kw == "enum"
                && body_pos < tokens.len()
                && matches!(tokens[body_pos], Tok::LBrace)
            {
                let vmode = if matches!(vis, Visibility::Public) {
                    SurfaceCheckMode::UiPubField
                } else {
                    SurfaceCheckMode::UiGeneral
                };
                let variant_env = if vmode == SurfaceCheckMode::UiPubField {
                    &ui_pub_field_env
                } else {
                    &ui_env
                };
                let close = find_matching_brace(&tokens, body_pos);
                for seg in split_top_level_comma(&tokens[body_pos + 1..close]) {
                    let clean = strip_leading_attrs_and_vis(seg);
                    if let Some(open_brace) = clean.iter().position(|t| matches!(t, Tok::LBrace)) {
                        let v_close = find_matching_brace(clean, open_brace);
                        for fseg in split_top_level_comma(&clean[open_brace + 1..v_close]) {
                            let fclean = strip_leading_attrs_and_vis(fseg);
                            if let Some(cpos) =
                                fclean.iter().position(|t| matches!(t, Tok::Other(':')))
                            {
                                check_seat_surface_type_tokens(
                                    rel_path,
                                    &format!("enum `{item_name}` variant field"),
                                    &fclean[cpos + 1..],
                                    variant_env,
                                    vmode,
                                )?;
                            }
                        }
                    } else if let Some(open_paren) =
                        clean.iter().position(|t| matches!(t, Tok::LParen))
                    {
                        let v_close = find_matching_paren(clean, open_paren);
                        for fseg in split_top_level_comma(&clean[open_paren + 1..v_close]) {
                            let fclean = strip_leading_attrs_and_vis(fseg);
                            check_seat_surface_type_tokens(
                                rel_path,
                                &format!("enum `{item_name}` tuple variant field"),
                                fclean,
                                variant_env,
                                vmode,
                            )?;
                        }
                    }
                }
                attrs = AttrFlags::default();
                i = close + 1;
                continue;
            }
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }

    Ok(())
}

struct SeatTypesAliasDef {
    rel_path: String,
    name: String,
    rhs_tokens: Vec<Tok>,
}

fn check_seat_types_surface(files: &[(String, String)]) -> Result<(), String> {
    let concrete_locals = collect_seat_local_types(files, true);
    let mut alias_defs: Vec<SeatTypesAliasDef> = Vec::new();
    let empty_generics = BTreeSet::new();

    // Pass 1: reject `use ... as`, collect every `type` item (public or private) and check its
    // generic/where parameters.
    for (rel_path, source) in files {
        let tokens = tokenize_rust(source);
        let mut scopes = vec![ScopeKind::Root { public: true }];
        let mut attrs = AttrFlags::default();
        let mut i = 0_usize;

        while i < tokens.len() {
            let in_cfg_test = scopes.iter().any(|s| match s {
                ScopeKind::Root { .. } => false,
                ScopeKind::Module { cfg_test, .. }
                | ScopeKind::InherentImpl { cfg_test, .. }
                | ScopeKind::TraitImpl { cfg_test, .. }
                | ScopeKind::Block { cfg_test } => *cfg_test,
            });
            match &tokens[i] {
                Tok::Pound => {
                    i = parse_tok_attr(&tokens, i, &mut attrs);
                    continue;
                }
                Tok::RBrace => {
                    if scopes.len() > 1 {
                        scopes.pop();
                    }
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::LBrace => {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::Semi => {
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if in_cfg_test || attrs.cfg_test {
                if matches!(
                    tokens[i],
                    Tok::Ident(ref s)
                        if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
                ) {
                    let mut j = i + 1;
                    while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                        j += 1;
                    }
                    if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                        scopes.push(ScopeKind::Block { cfg_test: true });
                        attrs = AttrFlags::default();
                        i = j + 1;
                        continue;
                    }
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
                i += 1;
                continue;
            }

            let after_vis = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                    find_matching_paren(&tokens, i + 1) + 1
                } else {
                    i + 1
                }
            } else {
                i
            };

            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
                let j = find_stmt_semi(&tokens, after_vis + 1);
                if tokens[after_vis + 1..j]
                    .iter()
                    .any(|t| matches!(t, Tok::Ident(s) if s == "as"))
                {
                    return Err(format!(
                        "{rel_path}: `use ... as` rename is forbidden in `league-seat-types`"
                    ));
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "type")
                && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
            {
                let gen_start = after_vis + 2;
                let after_gen = skip_angle_generics(&tokens, gen_start);
                let j = find_stmt_semi(&tokens, after_gen);
                let stmt = &tokens[after_gen..j];
                let where_pos = stmt
                    .iter()
                    .position(|t| matches!(t, Tok::Ident(w) if w == "where"));
                let eq_pos = stmt.iter().position(|t| matches!(t, Tok::Other('=')));
                let where_slice = match (where_pos, eq_pos) {
                    (Some(wp), Some(ep)) if wp < ep => &stmt[wp + 1..ep],
                    (Some(wp), _) => &stmt[wp + 1..],
                    _ => &[],
                };
                check_no_non_lifetime_generics_in_crate(
                    rel_path,
                    "league-seat-types",
                    &format!("type {item_name}"),
                    &tokens,
                    gen_start,
                    where_slice,
                )?;
                if let Some(ep) = eq_pos {
                    let end_rhs = where_pos.filter(|&wp| wp > ep).unwrap_or(stmt.len());
                    alias_defs.push(SeatTypesAliasDef {
                        rel_path: rel_path.clone(),
                        name: item_name.clone(),
                        rhs_tokens: stmt[ep + 1..end_rhs].to_vec(),
                    });
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                attrs = AttrFlags::default();
            }
            i += 1;
        }
    }

    // Validate the RHS of EVERY `type` item (pub or private) until fixed point so chained aliases
    // settle while any alias referencing `fn`/`dyn`/`impl`/`Fn*`/`Box`/interior-mutability/unlisted
    // type fails cleanly.
    let mut validated_aliases = BTreeSet::new();
    let mut pending: Vec<SeatTypesAliasDef> = alias_defs;
    for _pass in 0..=pending.len() {
        let mut next_pending = Vec::new();
        let mut progressed = false;
        let mut known_types = concrete_locals.clone();
        known_types.extend(validated_aliases.iter().cloned());
        let alias_env = SurfaceCheckEnv {
            crate_name: "league-seat-types",
            local_types: &known_types,
            allowed_types: ALLOWED_SEAT_TYPES_SURFACE_TYPES,
            allowed_fn_generics: &empty_generics,
        };
        for def in pending {
            if check_seat_surface_type_tokens(
                &def.rel_path,
                &format!("`type {}`", def.name),
                &def.rhs_tokens,
                &alias_env,
                SurfaceCheckMode::SeatTypes,
            )
            .is_ok()
            {
                validated_aliases.insert(def.name);
                progressed = true;
            } else {
                next_pending.push(def);
            }
        }
        pending = next_pending;
        if pending.is_empty() || !progressed {
            break;
        }
    }
    if let Some(bad) = pending.into_iter().next() {
        let mut known_types = concrete_locals.clone();
        known_types.extend(validated_aliases.iter().cloned());
        let alias_env = SurfaceCheckEnv {
            crate_name: "league-seat-types",
            local_types: &known_types,
            allowed_types: ALLOWED_SEAT_TYPES_SURFACE_TYPES,
            allowed_fn_generics: &empty_generics,
        };
        check_seat_surface_type_tokens(
            &bad.rel_path,
            &format!("`type {}`", bad.name),
            &bad.rhs_tokens,
            &alias_env,
            SurfaceCheckMode::SeatTypes,
        )?;
    }

    let mut all_local_types = concrete_locals;
    all_local_types.extend(validated_aliases);

    // Pass 2: validate structs, enums, consts, impls, and fn signatures across `league-seat-types`.
    for (rel_path, source) in files {
        let tokens = tokenize_rust(source);
        let mut scopes = vec![ScopeKind::Root { public: true }];
        let mut attrs = AttrFlags::default();
        let mut i = 0_usize;

        while i < tokens.len() {
            let in_cfg_test = scopes.iter().any(|s| match s {
                ScopeKind::Root { .. } => false,
                ScopeKind::Module { cfg_test, .. }
                | ScopeKind::InherentImpl { cfg_test, .. }
                | ScopeKind::TraitImpl { cfg_test, .. }
                | ScopeKind::Block { cfg_test } => *cfg_test,
            });
            match &tokens[i] {
                Tok::Pound => {
                    i = parse_tok_attr(&tokens, i, &mut attrs);
                    continue;
                }
                Tok::RBrace => {
                    if scopes.len() > 1 {
                        scopes.pop();
                    }
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::LBrace => {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                Tok::Semi => {
                    attrs = AttrFlags::default();
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if in_cfg_test || attrs.cfg_test {
                if matches!(
                    tokens[i],
                    Tok::Ident(ref s)
                        if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
                ) {
                    let mut j = i + 1;
                    while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                        j += 1;
                    }
                    if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                        scopes.push(ScopeKind::Block { cfg_test: true });
                        attrs = AttrFlags::default();
                        i = j + 1;
                        continue;
                    }
                    attrs = AttrFlags::default();
                    i = (j + 1).min(tokens.len());
                    continue;
                }
                i += 1;
                continue;
            }

            let (vis, after_vis) = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                    (
                        Visibility::CrateOrRestricted,
                        find_matching_paren(&tokens, i + 1) + 1,
                    )
                } else {
                    (Visibility::Public, i + 1)
                }
            } else {
                (Visibility::Private, i)
            };

            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
                let j = find_stmt_semi(&tokens, after_vis + 1);
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if rel_path == "src/view.rs"
                && matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "trait")
            {
                return Err(format!(
                    "{rel_path}: view-model module must not declare traits (`Item 4`)"
                ));
            }

            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "type") {
                let j = find_stmt_semi(&tokens, after_vis + 1);
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
                let mut j = after_vis + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                let header = &tokens[after_vis + 1..j];
                if rel_path == "src/view.rs"
                    && parse_impl_header(header, &FileAliasTable::default()).0
                {
                    return Err(format!(
                        "{rel_path}: view-model module must not implement traits beyond derives (`Item 4`)"
                    ));
                }
                let where_slice = header
                    .iter()
                    .position(|t| matches!(t, Tok::Ident(w) if w == "where"))
                    .map(|wp| &header[wp + 1..])
                    .unwrap_or(&[]);
                if rel_path == "src/view.rs"
                    && (skip_angle_generics(&tokens, after_vis + 1) > after_vis + 1
                        || !where_slice.is_empty())
                {
                    return Err(format!(
                        "{rel_path}: view-model `impl` must not declare generics or lifetimes (`Item 4`)"
                    ));
                }
                check_no_non_lifetime_generics_in_crate(
                    rel_path,
                    "league-seat-types",
                    "impl block",
                    &tokens,
                    after_vis + 1,
                    where_slice,
                )?;
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            let mut fn_kw = after_vis;
            loop {
                match tokens.get(fn_kw) {
                    Some(Tok::Ident(s))
                        if matches!(s.as_str(), "const" | "async" | "unsafe" | "extern") =>
                    {
                        fn_kw += 1;
                        if s == "extern" && matches!(tokens.get(fn_kw), Some(Tok::Str(_))) {
                            fn_kw += 1;
                        }
                    }
                    _ => break,
                }
            }
            if matches!(tokens.get(fn_kw), Some(Tok::Ident(s)) if s == "fn")
                && let Some(Tok::Ident(fn_name)) = tokens.get(fn_kw + 1)
            {
                let gen_start = fn_kw + 2;
                let sig_idx = skip_angle_generics(&tokens, gen_start);
                let (params, ret_start) = if matches!(tokens.get(sig_idx), Some(Tok::LParen)) {
                    let close = find_matching_paren(&tokens, sig_idx);
                    (&tokens[sig_idx + 1..close], close + 1)
                } else {
                    (&[][..], sig_idx)
                };
                let mut j = ret_start;
                let mut arrow_pos = None;
                let mut where_pos = None;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    if arrow_pos.is_none()
                        && j + 1 < tokens.len()
                        && matches!(tokens[j], Tok::Other('-'))
                        && matches!(tokens[j + 1], Tok::Other('>'))
                    {
                        arrow_pos = Some(j + 2);
                    }
                    if where_pos.is_none() && matches!(tokens[j], Tok::Ident(ref w) if w == "where")
                    {
                        where_pos = Some(j);
                    }
                    j += 1;
                }
                let where_slice = where_pos.map(|wp| &tokens[wp + 1..j]).unwrap_or(&[]);
                if rel_path == "src/view.rs"
                    && (skip_angle_generics(&tokens, gen_start) > gen_start
                        || !where_slice.is_empty())
                {
                    return Err(format!(
                        "{rel_path}: view-model `fn {fn_name}` must not declare generics or lifetimes (`Item 4`)"
                    ));
                }
                let allowed_fn_generics = parse_allowed_seat_types_fn_generics(
                    rel_path,
                    &format!("fn {fn_name}"),
                    &tokens,
                    gen_start,
                    where_slice,
                )?;
                let ret_slice = match arrow_pos {
                    Some(rs) => {
                        let re = where_pos.unwrap_or(j).min(j);
                        if rs <= re { &tokens[rs..re] } else { &[] }
                    }
                    None => &[],
                };
                let vis_label = if matches!(vis, Visibility::Public) {
                    "pub fn"
                } else {
                    "fn"
                };
                let fn_env = SurfaceCheckEnv {
                    crate_name: "league-seat-types",
                    local_types: &all_local_types,
                    allowed_types: ALLOWED_SEAT_TYPES_SURFACE_TYPES,
                    allowed_fn_generics: &allowed_fn_generics,
                };
                for seg in split_top_level_comma(params) {
                    let clean = strip_leading_attrs_and_vis(seg);
                    if let Some(colon_pos) = clean.iter().position(|t| matches!(t, Tok::Other(':')))
                    {
                        check_seat_surface_type_tokens(
                            rel_path,
                            &format!("`{vis_label} {fn_name}` parameter"),
                            &clean[colon_pos + 1..],
                            &fn_env,
                            SurfaceCheckMode::SeatTypes,
                        )?;
                    }
                }
                if !ret_slice.is_empty() {
                    check_seat_surface_type_tokens(
                        rel_path,
                        &format!("`{vis_label} {fn_name}` return type"),
                        ret_slice,
                        &fn_env,
                        SurfaceCheckMode::SeatTypes,
                    )?;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "const" || s == "static")
            {
                let mut k = after_vis + 1;
                if matches!(tokens.get(k), Some(Tok::Ident(s)) if s == "mut") {
                    k += 1;
                }
                let item_name = match tokens.get(k) {
                    Some(Tok::Ident(n)) => n.as_str(),
                    _ => "<const>",
                };
                let j = find_stmt_semi(&tokens, k);
                let stmt = &tokens[k..j];
                if let Some(colon_pos) = stmt.iter().position(|t| matches!(t, Tok::Other(':'))) {
                    let end_ty = stmt[colon_pos + 1..]
                        .iter()
                        .position(|t| matches!(t, Tok::Other('=')))
                        .map(|p| colon_pos + 1 + p)
                        .unwrap_or(stmt.len());
                    let const_env = SurfaceCheckEnv {
                        crate_name: "league-seat-types",
                        local_types: &all_local_types,
                        allowed_types: ALLOWED_SEAT_TYPES_SURFACE_TYPES,
                        allowed_fn_generics: &empty_generics,
                    };
                    check_seat_surface_type_tokens(
                        rel_path,
                        &format!(
                            "`{} {item_name}`",
                            if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "static")
                            {
                                "static"
                            } else {
                                "const"
                            }
                        ),
                        &stmt[colon_pos + 1..end_ty],
                        &const_env,
                        SurfaceCheckMode::SeatTypes,
                    )?;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }

            if let Some(Tok::Ident(item_kw)) = tokens.get(after_vis)
                && matches!(item_kw.as_str(), "struct" | "enum")
                && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
            {
                let gen_start = after_vis + 2;
                let after_gen = skip_angle_generics(&tokens, gen_start);
                let mut body_pos = after_gen;
                let mut where_pos = None;
                while body_pos < tokens.len()
                    && !matches!(tokens[body_pos], Tok::LBrace | Tok::LParen | Tok::Semi)
                {
                    if where_pos.is_none()
                        && matches!(tokens[body_pos], Tok::Ident(ref w) if w == "where")
                    {
                        where_pos = Some(body_pos);
                    }
                    body_pos += 1;
                }
                let where_slice = where_pos.map(|wp| &tokens[wp + 1..body_pos]).unwrap_or(&[]);
                if rel_path == "src/view.rs" && (after_gen > gen_start || !where_slice.is_empty()) {
                    return Err(format!(
                        "{rel_path}: view-model `{item_kw} {item_name}` must not declare generics or lifetimes (`Item 4`)"
                    ));
                }
                check_no_non_lifetime_generics_in_crate(
                    rel_path,
                    "league-seat-types",
                    &format!("{item_kw} {item_name}"),
                    &tokens,
                    gen_start,
                    where_slice,
                )?;

                let item_env = SurfaceCheckEnv {
                    crate_name: "league-seat-types",
                    local_types: &all_local_types,
                    allowed_types: ALLOWED_SEAT_TYPES_SURFACE_TYPES,
                    allowed_fn_generics: &empty_generics,
                };
                if item_kw == "struct" && body_pos < tokens.len() {
                    if matches!(tokens[body_pos], Tok::LBrace) {
                        let close = find_matching_brace(&tokens, body_pos);
                        for seg in split_top_level_comma(&tokens[body_pos + 1..close]) {
                            let clean = strip_leading_attrs_and_vis(seg);
                            if let Some(cpos) =
                                clean.iter().position(|t| matches!(t, Tok::Other(':')))
                            {
                                let field_ty = &clean[cpos + 1..];
                                if rel_path == "src/view.rs"
                                    && field_ty.iter().any(|t| matches!(t, Tok::Other('&')))
                                {
                                    return Err(format!(
                                        "{rel_path}: view-model struct `{item_name}` field must use owned data (references are forbidden)"
                                    ));
                                }
                                check_seat_surface_type_tokens(
                                    rel_path,
                                    &format!("struct `{item_name}` field"),
                                    field_ty,
                                    &item_env,
                                    SurfaceCheckMode::SeatTypes,
                                )?;
                            }
                        }
                        attrs = AttrFlags::default();
                        i = close + 1;
                        continue;
                    } else if matches!(tokens[body_pos], Tok::LParen) {
                        let close = find_matching_paren(&tokens, body_pos);
                        for seg in split_top_level_comma(&tokens[body_pos + 1..close]) {
                            let clean = strip_leading_attrs_and_vis(seg);
                            if rel_path == "src/view.rs"
                                && clean.iter().any(|t| matches!(t, Tok::Other('&')))
                            {
                                return Err(format!(
                                    "{rel_path}: view-model struct `{item_name}` field must use owned data (references are forbidden)"
                                ));
                            }
                            check_seat_surface_type_tokens(
                                rel_path,
                                &format!("struct `{item_name}` tuple field"),
                                clean,
                                &item_env,
                                SurfaceCheckMode::SeatTypes,
                            )?;
                        }
                        let semi = find_stmt_semi(&tokens, close + 1);
                        attrs = AttrFlags::default();
                        i = (semi + 1).min(tokens.len());
                        continue;
                    }
                }

                if item_kw == "enum"
                    && body_pos < tokens.len()
                    && matches!(tokens[body_pos], Tok::LBrace)
                {
                    let close = find_matching_brace(&tokens, body_pos);
                    for seg in split_top_level_comma(&tokens[body_pos + 1..close]) {
                        let clean = strip_leading_attrs_and_vis(seg);
                        if let Some(open_brace) =
                            clean.iter().position(|t| matches!(t, Tok::LBrace))
                        {
                            let v_close = find_matching_brace(clean, open_brace);
                            for fseg in split_top_level_comma(&clean[open_brace + 1..v_close]) {
                                let fclean = strip_leading_attrs_and_vis(fseg);
                                if let Some(cpos) =
                                    fclean.iter().position(|t| matches!(t, Tok::Other(':')))
                                {
                                    let field_ty = &fclean[cpos + 1..];
                                    if rel_path == "src/view.rs"
                                        && field_ty.iter().any(|t| matches!(t, Tok::Other('&')))
                                    {
                                        return Err(format!(
                                            "{rel_path}: view-model enum `{item_name}` field must use owned data (references are forbidden)"
                                        ));
                                    }
                                    check_seat_surface_type_tokens(
                                        rel_path,
                                        &format!("enum `{item_name}` variant field"),
                                        field_ty,
                                        &item_env,
                                        SurfaceCheckMode::SeatTypes,
                                    )?;
                                }
                            }
                        } else if let Some(open_paren) =
                            clean.iter().position(|t| matches!(t, Tok::LParen))
                        {
                            let v_close = find_matching_paren(clean, open_paren);
                            for fseg in split_top_level_comma(&clean[open_paren + 1..v_close]) {
                                let fclean = strip_leading_attrs_and_vis(fseg);
                                if rel_path == "src/view.rs"
                                    && fclean.iter().any(|t| matches!(t, Tok::Other('&')))
                                {
                                    return Err(format!(
                                        "{rel_path}: view-model enum `{item_name}` tuple field must use owned data (references are forbidden)"
                                    ));
                                }
                                check_seat_surface_type_tokens(
                                    rel_path,
                                    &format!("enum `{item_name}` tuple variant field"),
                                    fclean,
                                    &item_env,
                                    SurfaceCheckMode::SeatTypes,
                                )?;
                            }
                        }
                    }
                    attrs = AttrFlags::default();
                    i = close + 1;
                    continue;
                }
            }

            if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
                attrs = AttrFlags::default();
            }
            i += 1;
        }
    }

    Ok(())
}

fn parse_attr_bracket_end(tokens: &[Tok], lbracket_idx: usize) -> usize {
    let mut depth = 1_usize;
    let mut j = lbracket_idx + 1;
    while j < tokens.len() && depth > 0 {
        match tokens[j] {
            Tok::LBracket => depth += 1,
            Tok::RBracket => depth -= 1,
            _ => {}
        }
        j += 1;
    }
    j
}

fn is_exact_seat_no_std_inner_attr(inner: &[Tok]) -> bool {
    inner.len() == 1 && matches!(&inner[0], Tok::Ident(s) if s == "no_std")
}

fn is_exact_seat_forbid_unsafe_inner_attr(inner: &[Tok]) -> bool {
    let body = if inner.last() == Some(&Tok::Other(',')) {
        &inner[..inner.len() - 1]
    } else {
        inner
    };
    body.len() == 4
        && matches!(&body[0], Tok::Ident(s) if s == "forbid")
        && matches!(&body[1], Tok::LParen)
        && matches!(&body[2], Tok::Ident(s) if s == "unsafe_code")
        && matches!(&body[3], Tok::RParen)
}

fn is_exact_seat_forbid_inner_attr(inner: &[Tok]) -> bool {
    let body = if inner.last() == Some(&Tok::Other(',')) {
        &inner[..inner.len() - 1]
    } else {
        inner
    };
    if body.len() < 4
        || !matches!(&body[0], Tok::Ident(s) if s == "forbid")
        || !matches!(&body[1], Tok::LParen)
        || !matches!(body.last(), Some(Tok::RParen))
    {
        return false;
    }
    let lint_toks = &body[2..body.len() - 1];
    let mut lints = Vec::new();
    for seg in split_top_level_comma(lint_toks) {
        if seg.is_empty() {
            continue;
        }
        if seg.len() == 4
            && matches!(&seg[0], Tok::Ident(c) if c == "clippy")
            && matches!(seg[1], Tok::Other(':'))
            && matches!(seg[2], Tok::Other(':'))
            && let Tok::Ident(name) = &seg[3]
        {
            lints.push(name.as_str());
        } else {
            return false;
        }
    }
    lints.contains(&"disallowed_methods")
        && lints.contains(&"disallowed_types")
        && lints.contains(&"disallowed_macros")
        && lints.len() == 3
}

/// Checks attributes that `--target thumbv7em-none-eabi` cannot express on its own:
/// exact `#![no_std]` / `#![forbid(unsafe_code)]` / `#![forbid(...)]` at the top of `src/lib.rs`,
/// at most one `#[cfg(test)] mod tests { ... }`, no other `#[cfg(...)]` or `#[cfg_attr(...)]`,
/// no `#[path]` / `#[macro_export]` / `#[no_mangle]` / `#[unsafe(...)]`, and no `clippy` /
/// `unsafe_code` / `disallowed_*` suppressions.
fn check_seat_attrs_and_mark_spans(
    crate_name: &str,
    rel_path: &str,
    tokens: &[Tok],
) -> Result<Vec<bool>, String> {
    let mut is_attr_tok = vec![false; tokens.len()];
    let mut test_mod_span: Option<(usize, usize)> = None;
    let mut cfg_test_seen = false;
    let mut seen_non_attr = false;
    let mut i = 0_usize;

    while i < tokens.len() {
        if matches!(tokens[i], Tok::Pound) {
            let is_inner = matches!(tokens.get(i + 1), Some(Tok::Bang));
            let b_idx = if is_inner { i + 2 } else { i + 1 };
            if !matches!(tokens.get(b_idx), Some(Tok::LBracket)) {
                return Err(format!(
                    "{crate_name}/{rel_path}: unexpected `#` token outside attribute"
                ));
            }
            let end_excl = parse_attr_bracket_end(tokens, b_idx);
            if end_excl <= b_idx + 1 {
                return Err(format!(
                    "{crate_name}/{rel_path}: unclosed attribute bracket"
                ));
            }
            for flag in &mut is_attr_tok[i..end_excl] {
                *flag = true;
            }
            let inner = &tokens[b_idx + 1..end_excl - 1];
            if is_inner {
                if rel_path != "src/lib.rs" || seen_non_attr {
                    return Err(format!(
                        "{crate_name}/{rel_path}: inner attribute `#![...]` is only allowed at the top of `src/lib.rs`"
                    ));
                }
                if !is_exact_seat_no_std_inner_attr(inner)
                    && !is_exact_seat_forbid_unsafe_inner_attr(inner)
                    && !is_exact_seat_forbid_inner_attr(inner)
                {
                    return Err(format!(
                        "{crate_name}/{rel_path}: inner attribute is not on the seat-crate allow-list (only `#![no_std]`, `#![forbid(unsafe_code)]`, and `#![forbid(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]` are allowed)"
                    ));
                }
                i = end_excl;
                continue;
            }
            seen_non_attr = true;
            let in_test_mod = test_mod_span.is_some_and(|(s, e)| i > s && i < e);
            let head = inner.first().and_then(|t| match t {
                Tok::Ident(s) => Some(s.as_str()),
                _ => None,
            });
            match head {
                Some("cfg") => {
                    let is_exact_cfg_test = inner.len() == 4
                        && matches!(&inner[1], Tok::LParen)
                        && matches!(&inner[2], Tok::Ident(s) if s == "test")
                        && matches!(&inner[3], Tok::RParen);
                    if !is_exact_cfg_test {
                        return Err(format!(
                            "{crate_name}/{rel_path}: `#[cfg(...)]` is forbidden in seat crates (only exact `#[cfg(test)]` on one `mod tests` block is allowed)"
                        ));
                    }
                    if cfg_test_seen {
                        return Err(format!(
                            "{crate_name}/{rel_path}: at most one `#[cfg(test)] mod tests` block per file is allowed"
                        ));
                    }
                    let is_mod_tests = matches!(tokens.get(end_excl), Some(Tok::Ident(s)) if s == "mod")
                        && matches!(tokens.get(end_excl + 1), Some(Tok::Ident(s)) if s == "tests")
                        && matches!(tokens.get(end_excl + 2), Some(Tok::LBrace));
                    if !is_mod_tests {
                        return Err(format!(
                            "{crate_name}/{rel_path}: `#[cfg(test)]` is allowed only directly on `mod tests {{ ... }}`"
                        ));
                    }
                    let lbrace = end_excl + 2;
                    let rbrace = find_matching_brace(tokens, lbrace);
                    cfg_test_seen = true;
                    test_mod_span = Some((lbrace, rbrace));
                }
                Some("cfg_attr") => {
                    if inner
                        .iter()
                        .any(|t| matches!(t, Tok::Ident(s) if s == "path"))
                    {
                        return Err(format!(
                            "{crate_name}/{rel_path}: `#[cfg_attr(..., path = ...)]` is forbidden in seat crates"
                        ));
                    }
                    return Err(format!(
                        "{crate_name}/{rel_path}: `#[cfg_attr(...)]` is forbidden in seat crates"
                    ));
                }
                Some("test") if inner.len() == 1 => {
                    if !in_test_mod {
                        return Err(format!(
                            "{crate_name}/{rel_path}: `#[test]` is only allowed inside `#[cfg(test)] mod tests`"
                        ));
                    }
                }
                Some("path" | "macro_export" | "no_mangle" | "export_name" | "link_section") => {
                    let attr_name = head.unwrap_or("<unknown>");
                    return Err(format!(
                        "{crate_name}/{rel_path}: `#[{attr_name}]` attribute is forbidden in seat crates"
                    ));
                }
                Some("unsafe") => {
                    return Err(format!(
                        "{crate_name}/{rel_path}: `#[unsafe(...)]` attribute is forbidden in seat crates"
                    ));
                }
                _ => {
                    let has_suppression = inner.iter().any(|t| {
                        matches!(
                            t,
                            Tok::Ident(s) if matches!(s.as_str(), "allow" | "expect" | "warn")
                        )
                    });
                    let mentions_clippy_or_disallowed = inner.iter().any(|t| {
                        matches!(
                            t,
                            Tok::Ident(s) if s == "clippy" || s == "unsafe_code" || s.starts_with("disallowed_")
                        )
                    });
                    if has_suppression && mentions_clippy_or_disallowed {
                        return Err(format!(
                            "{crate_name}/{rel_path}: must not suppress or downgrade `clippy`/`unsafe_code`/`disallowed_*` lints via `#[allow]`/`#[expect]`"
                        ));
                    }
                }
            }
            i = end_excl;
            continue;
        }
        seen_non_attr = true;
        i += 1;
    }

    Ok(is_attr_tok)
}

fn is_forbidden_nostd_shared_state_ident(ident: &str) -> bool {
    is_forbidden_interior_mutability_type(ident)
}

fn check_seat_observation_no_deserialize(
    crate_name: &str,
    rel_path: &str,
    tokens: &[Tok],
    is_attr_tok: &[bool],
) -> Result<(), String> {
    if crate_name != "league-seat-types" {
        return Ok(());
    }
    for k in 0..tokens.len() {
        if !is_attr_tok[k]
            && matches!(&tokens[k], Tok::Ident(kw) if kw == "struct")
            && matches!(tokens.get(k + 1), Some(Tok::Ident(nm)) if nm == "SeatObservation")
        {
            let mut decl_start = k;
            if decl_start > 0 && matches!(&tokens[decl_start - 1], Tok::Ident(p) if p == "pub") {
                decl_start -= 1;
            } else if decl_start >= 3 && matches!(&tokens[decl_start - 1], Tok::RParen) {
                let mut paren_depth = 1_usize;
                let mut p = decl_start - 1;
                while p > 0 && paren_depth > 0 {
                    p -= 1;
                    match tokens[p] {
                        Tok::RParen => paren_depth += 1,
                        Tok::LParen => paren_depth -= 1,
                        _ => {}
                    }
                }
                if p > 0 && matches!(&tokens[p - 1], Tok::Ident(kw) if kw == "pub") {
                    decl_start = p - 1;
                }
            }
            let mut back = decl_start;
            while back > 0 && is_attr_tok[back - 1] {
                back -= 1;
            }
            if tokens[back..decl_start]
                .iter()
                .any(|t| matches!(t, Tok::Ident(a) if a == "Deserialize"))
            {
                return Err(format!(
                    "{crate_name}/{rel_path}: `SeatObservation` must not derive `Deserialize` (`B1` host-only mandate isolation)"
                ));
            }
        }
    }
    Ok(())
}

fn has_default_features_false(snippet: &str) -> bool {
    let mut part_start = 0_usize;
    let mut in_quote: Option<char> = None;
    let mut parts = Vec::new();
    for (idx, ch) in snippet.char_indices() {
        match in_quote {
            Some(q) if ch == q => in_quote = None,
            Some(_) => {}
            None if matches!(ch, '"' | '\'') => in_quote = Some(ch),
            None if matches!(ch, '{' | '}' | ',') => {
                parts.push(&snippet[part_start..idx]);
                part_start = idx + 1;
            }
            None => {}
        }
    }
    parts.push(&snippet[part_start..]);
    parts.into_iter().any(|part| {
        part.split_once('=')
            .is_some_and(|(lhs, rhs)| lhs.trim() == "default-features" && rhs.trim() == "false")
    })
}

fn check_seat_crate_nostd_and_channels(crate_name: &str, crate_dir: &Path) -> Result<(), String> {
    let clippy_path = crate_dir.join("clippy.toml");
    let clippy_toml = fs::read_to_string(&clippy_path)
        .map_err(|err| format!("{crate_name}: missing or unreadable clippy.toml ({err})"))?;
    for required in [
        "core::cell::RefCell",
        "core::cell::Cell",
        "core::sync::atomic::AtomicBool",
        "alloc::sync::Arc",
        "alloc::rc::Rc",
        "core::include",
        "core::include_str",
        "core::include_bytes",
        "core::env",
        "core::option_env",
    ] {
        if !clippy_toml.contains(required) {
            return Err(format!(
                "{crate_name}/clippy.toml must list `{required}` under disallowed rules"
            ));
        }
    }

    if crate_dir.join("build.rs").exists() {
        return Err(format!(
            "{crate_name}: seat crate must not define `build.rs`"
        ));
    }

    let lib_rs_path = crate_dir.join("src/lib.rs");
    let lib_src = fs::read_to_string(&lib_rs_path)
        .map_err(|err| format!("{crate_name}/src/lib.rs: failed to read ({err})"))?;
    let lib_tokens = tokenize_rust(&lib_src);
    let mut saw_no_std = false;
    let mut saw_forbid_unsafe_code = false;
    let mut forbidden_lints = BTreeSet::new();
    let mut idx = 0_usize;
    while idx + 2 < lib_tokens.len() {
        if matches!(lib_tokens[idx], Tok::Pound)
            && matches!(lib_tokens[idx + 1], Tok::Bang)
            && matches!(lib_tokens[idx + 2], Tok::LBracket)
        {
            let end = parse_attr_bracket_end(&lib_tokens, idx + 2);
            let inner = &lib_tokens[idx + 3..end.saturating_sub(1)];
            if is_exact_seat_no_std_inner_attr(inner) {
                saw_no_std = true;
            }
            if is_exact_seat_forbid_unsafe_inner_attr(inner) {
                saw_forbid_unsafe_code = true;
            }
            if matches!(inner.first(), Some(Tok::Ident(s)) if s == "deny") {
                if inner
                    .iter()
                    .any(|t| matches!(t, Tok::Ident(id) if id == "unsafe_code"))
                {
                    return Err(format!(
                        "{crate_name}/src/lib.rs must use `#![forbid(unsafe_code)]`, never `#![deny(unsafe_code)]`"
                    ));
                }
                if inner
                    .iter()
                    .any(|t| matches!(t, Tok::Ident(id) if id.starts_with("disallowed_")))
                {
                    return Err(format!(
                        "{crate_name}/src/lib.rs must use `#![forbid(...)]`, never `#![deny(...)]`, for `clippy::disallowed_*`"
                    ));
                }
            }
            if matches!(inner.first(), Some(Tok::Ident(s)) if s == "forbid") {
                for w in inner.windows(4) {
                    if matches!(&w[0], Tok::Ident(a) if a == "clippy")
                        && matches!(w[1], Tok::Other(':'))
                        && matches!(w[2], Tok::Other(':'))
                        && let Tok::Ident(lint) = &w[3]
                    {
                        forbidden_lints.insert(lint.clone());
                    }
                }
            }
            idx = end;
        } else {
            idx += 1;
        }
    }
    if !saw_no_std {
        return Err(format!("{crate_name}/src/lib.rs must declare `#![no_std]`"));
    }
    if !saw_forbid_unsafe_code {
        return Err(format!(
            "{crate_name}/src/lib.rs must declare `#![forbid(unsafe_code)]`"
        ));
    }
    for req_lint in [
        "disallowed_methods",
        "disallowed_types",
        "disallowed_macros",
    ] {
        if !forbidden_lints.contains(req_lint) {
            return Err(format!(
                "{crate_name}/src/lib.rs must enforce `#![forbid(clippy::{req_lint})]`"
            ));
        }
    }

    let subdirs: &[&str] = if crate_name == "league-error" {
        &["src"]
    } else {
        &["src", "tests"]
    };
    let mut all_rs_files = Vec::new();
    for &subdir in subdirs {
        let root = crate_dir.join(subdir);
        if !root.exists() {
            continue;
        }
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let entries = fs::read_dir(&dir)
                .map_err(|err| format!("{crate_name}: failed to read {}: {err}", dir.display()))?;
            for entry in entries {
                let entry = entry
                    .map_err(|err| format!("{crate_name}: failed to read dir entry: {err}"))?;
                let path = entry.path();
                let meta = fs::symlink_metadata(&path).map_err(|err| {
                    format!("{crate_name}: failed to stat {}: {err}", path.display())
                })?;
                if meta.file_type().is_symlink() {
                    return Err(format!(
                        "{crate_name}: symlinks are forbidden inside seat crate ({})",
                        path.display()
                    ));
                }
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                        return Err(format!(
                            "{crate_name}: non-.rs source file is forbidden inside seat crate ({})",
                            path.display()
                        ));
                    }
                    all_rs_files.push(path);
                }
            }
        }
    }
    all_rs_files.sort();

    for rs_path in all_rs_files {
        let rel = rs_path
            .strip_prefix(crate_dir)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_else(|| rs_path.display().to_string());
        let src = fs::read_to_string(&rs_path)
            .map_err(|err| format!("{crate_name}/{rel}: failed to read: {err}"))?;
        let tokens = tokenize_rust(&src);
        let is_attr_tok = check_seat_attrs_and_mark_spans(crate_name, &rel, &tokens)?;
        check_seat_observation_no_deserialize(crate_name, &rel, &tokens, &is_attr_tok)?;

        for k in 0..tokens.len() {
            if is_attr_tok[k] {
                continue;
            }
            if let Some(Tok::Ident(id)) = tokens.get(k) {
                // Ban `macro_rules!` / Macros 2.0 `macro` definitions so macros cannot hide
                // `pub fn` items from the surface checker.
                if id == "macro_rules" {
                    return Err(format!(
                        "{crate_name}/{rel}: `macro_rules` is forbidden in seat crates"
                    ));
                }
                if id == "macro"
                    && matches!(tokens.get(k + 1), Some(Tok::Ident(_)))
                    && (k == 0 || !matches!(tokens[k - 1], Tok::Other(':')))
                {
                    return Err(format!(
                        "{crate_name}/{rel}: `macro` definition is forbidden in seat crates"
                    ));
                }
                if matches!(id.as_str(), "asm" | "global_asm" | "naked_asm") {
                    return Err(format!(
                        "{crate_name}/{rel}: `{id}` inline/global assembly is forbidden in seat crates"
                    ));
                }

                // Ban `extern crate` (except `extern crate alloc;` in `src/lib.rs`) and `extern { ... }`
                // FFI blocks (leaving `extern "C" fn` pointer types to report cleanly in surface checks).
                if id == "extern" {
                    let is_allowed_extern_alloc = rel == "src/lib.rs"
                        && matches!(tokens.get(k + 1), Some(Tok::Ident(c)) if c == "crate")
                        && matches!(tokens.get(k + 2), Some(Tok::Ident(a)) if a == "alloc")
                        && matches!(tokens.get(k + 3), Some(Tok::Semi));
                    let is_extern_crate =
                        matches!(tokens.get(k + 1), Some(Tok::Ident(c)) if c == "crate");
                    let is_extern_block = matches!(tokens.get(k + 1), Some(Tok::LBrace))
                        || (matches!(tokens.get(k + 1), Some(Tok::Str(_)))
                            && matches!(tokens.get(k + 2), Some(Tok::LBrace)));
                    if !is_allowed_extern_alloc && (is_extern_crate || is_extern_block) {
                        return Err(format!(
                            "{crate_name}/{rel}: `extern` crate/FFI block is forbidden in seat crates"
                        ));
                    }
                }

                // Ban `unsafe` blocks/fns/impls/traits (leaving `unsafe fn(...)` fn pointers to
                // surface checks so fn-pointer rejection stays explicit).
                if id == "unsafe" {
                    let is_unsafe_fn_ptr_ty = matches!(tokens.get(k + 1), Some(Tok::Ident(f)) if f == "fn")
                        && matches!(tokens.get(k + 2), Some(Tok::LParen));
                    if !is_unsafe_fn_ptr_ty {
                        return Err(format!(
                            "{crate_name}/{rel}: `unsafe` is forbidden in seat crates"
                        ));
                    }
                }

                // Ban `static` items (while preserving `'static` lifetimes).
                if id == "static"
                    && !matches!(
                        k.checked_sub(1).and_then(|p| tokens.get(p)),
                        Some(Tok::Other('\''))
                    )
                {
                    return Err(format!(
                        "{crate_name}/{rel}: `static` items are forbidden in seat crates"
                    ));
                }

                // Ban compile-time host file/env macros that work even in `#![no_std]`.
                if matches!(
                    id.as_str(),
                    "include" | "include_str" | "include_bytes" | "env" | "option_env"
                ) && matches!(tokens.get(k + 1), Some(Tok::Bang))
                {
                    return Err(format!(
                        "{crate_name}/{rel}: macro `{id}!` is forbidden in seat crates"
                    ));
                }

                // Ban `core::cell::` and `core::sync::atomic::` paths, plus `core`/`alloc`
                // shared-state types (`Cell`, `RefCell`, `UnsafeCell`, `OnceCell`, `LazyCell`,
                // `Rc`, `Arc`, `Weak`, `Atomic*`).
                if matches!(id.as_str(), "cell" | "atomic")
                    && matches!(
                        (tokens.get(k + 1), tokens.get(k + 2)),
                        (Some(Tok::Other(':')), Some(Tok::Other(':')))
                    )
                {
                    return Err(format!(
                        "{crate_name}/{rel}: `{id}::` shared-state access is forbidden in seat crates"
                    ));
                }
                if is_forbidden_nostd_shared_state_ident(id) {
                    return Err(format!(
                        "{crate_name}/{rel}: forbidden shared-state / interior-mutability type `{id}` in seat crates"
                    ));
                }

                if id == "use" {
                    let end = find_stmt_semi(&tokens, k + 1);
                    let mut expanded = Vec::new();
                    expand_use_tree_allow_glob(&rel, &tokens[k + 1..end], &mut expanded);
                    for item in expanded {
                        for seg in item.canonical_path.split("::") {
                            if matches!(
                                seg,
                                "cell"
                                    | "atomic"
                                    | "asm"
                                    | "global_asm"
                                    | "naked_asm"
                                    | "include"
                                    | "include_str"
                                    | "include_bytes"
                                    | "env"
                                    | "option_env"
                            ) || is_forbidden_nostd_shared_state_ident(seg)
                            {
                                return Err(format!(
                                    "{crate_name}/{rel}: `use {}` is forbidden in seat crates",
                                    item.canonical_path
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn check_club_mod_rs_source(rel_path: &str, source: &str) -> Result<(), String> {
    let tokens = tokenize_rust(source);
    for tok in &tokens {
        if let Tok::Ident(ident) = tok {
            if RAW_ENGINE_TYPES_FOR_HOST_MODULES.contains(&ident.as_str()) {
                return Err(format!(
                    "{rel_path}: club/mod.rs must not reference raw engine symbol `{ident}`"
                ));
            }
            if matches!(
                ident.as_str(),
                "fn" | "struct" | "enum" | "impl" | "type" | "as"
            ) {
                return Err(format!(
                    "{rel_path}: club/mod.rs must only declare `pub mod host;` and re-export host surface (found `{ident}`)"
                ));
            }
        }
    }
    Ok(())
}

struct ExpectedCoreSurface<'a> {
    enforce_exact_surface: bool,
    required_pub_mods: &'a [&'a str],
    allowed_pub_mods: &'a [&'a str],
    allowed_reexports: &'a [&'a str],
    allowed_turn_engine_pub_fns: &'a [&'a str],
    allowed_club_host_pub_fns: Option<&'a [&'a str]>,
    allowed_observation_pub_fns: &'a [&'a str],
}

const PRODUCTION_SURFACE: ExpectedCoreSurface<'static> = ExpectedCoreSurface {
    enforce_exact_surface: true,
    required_pub_mods: &PRODUCTION_REQUIRED_MODS,
    allowed_pub_mods: &PRODUCTION_ALLOWED_MODS,
    allowed_reexports: &PRODUCTION_ALLOWED_REEXPORTS,
    allowed_turn_engine_pub_fns: &ALLOWED_TURN_ENGINE_PUB_FNS,
    allowed_club_host_pub_fns: Some(&ALLOWED_CLUB_HOST_PUB_FNS),
    allowed_observation_pub_fns: &ALLOWED_OBSERVATION_PUB_FNS,
};

const MINIMAL_TEST_SURFACE: ExpectedCoreSurface<'static> = ExpectedCoreSurface {
    enforce_exact_surface: false,
    required_pub_mods: &["engine", "observation"],
    allowed_pub_mods: &["bot", "club", "engine", "observation"],
    allowed_reexports: &[],
    allowed_turn_engine_pub_fns: &["observe_seat"],
    allowed_club_host_pub_fns: None,
    allowed_observation_pub_fns: &["preview_what_if", "preview_seat_bundle"],
};

fn scan_league_core_src_tree_with_surface(
    src_root: &Path,
    surface: &ExpectedCoreSurface<'_>,
) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_rs_files_recursive(src_root, &mut files);
    files.sort();

    let mut loaded_files = Vec::with_capacity(files.len());
    for path in files {
        let rel = path
            .strip_prefix(src_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_else(|| path.display().to_string());
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        loaded_files.push((rel, source));
    }
    let crate_aliases = collect_crate_restricted_aliases_from_files(&loaded_files);

    let mut visited = Vec::with_capacity(loaded_files.len());
    let mut saw_engine_internals = false;
    let mut observation_checked = false;
    let mut saw_preview_what_if = false;
    let mut saw_preview_seat_bundle = false;
    let mut bot_seat_files: Vec<(String, String)> = Vec::new();
    let mut club_seat_files: Vec<(String, String)> = Vec::new();

    for (rel, source) in loaded_files {
        let is_lib_rs = rel == "lib.rs";
        let scan =
            scan_file_public_api_with_crate_aliases(&rel, &source, is_lib_rs, &crate_aliases)?;

        if scan.seen_engine_internals_mod {
            saw_engine_internals = true;
        }

        if is_lib_rs && surface.enforce_exact_surface {
            let allowed_mods: BTreeSet<&str> = surface.allowed_pub_mods.iter().copied().collect();
            let found_mods: BTreeSet<&str> = scan.pub_mods.iter().map(String::as_str).collect();
            for m in &found_mods {
                if !allowed_mods.contains(m) {
                    return Err(format!("lib.rs exposes unlisted `pub mod {m}`"));
                }
            }
            for req in surface.required_pub_mods {
                if !found_mods.contains(req) {
                    return Err(format!("lib.rs missing required `pub mod {req}`"));
                }
            }
            let allowed_reexports: BTreeSet<&str> =
                surface.allowed_reexports.iter().copied().collect();
            for item in &scan.lib_reexports {
                if !allowed_reexports.contains(item.as_str()) {
                    return Err(format!(
                        "lib.rs re-exports unlisted `pub use` item path `{item}`"
                    ));
                }
            }
        }

        if let Some(te_fns) = scan.pub_fns_by_scope.get("impl:TurnEngine") {
            let allowed_te: BTreeSet<&str> = surface
                .allowed_turn_engine_pub_fns
                .iter()
                .copied()
                .collect();
            for f in te_fns {
                if !allowed_te.contains(f.as_str()) {
                    return Err(format!(
                        "{rel}: `impl TurnEngine` exposes unlisted `pub fn {f}`"
                    ));
                }
            }
        }

        if let Some(allowed_host) = surface.allowed_club_host_pub_fns
            && let Some(host_fns) = scan.pub_fns_by_scope.get("impl:ClubHost")
        {
            let allowed_set: BTreeSet<&str> = allowed_host.iter().copied().collect();
            for f in host_fns {
                if !allowed_set.contains(f.as_str()) {
                    return Err(format!(
                        "{rel}: `impl ClubHost` exposes unlisted `pub fn {f}`"
                    ));
                }
            }
        }

        if rel == "observation.rs" {
            observation_checked = true;
            let obs_fns = scan
                .pub_fns_by_scope
                .get("mod:observation.rs")
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            saw_preview_what_if = obs_fns.iter().any(|f| f == "preview_what_if");
            saw_preview_seat_bundle = obs_fns.iter().any(|f| f == "preview_seat_bundle");
            if surface.enforce_exact_surface {
                let allowed_obs: BTreeSet<&str> = surface
                    .allowed_observation_pub_fns
                    .iter()
                    .copied()
                    .collect();
                for f in obs_fns {
                    if !allowed_obs.contains(f.as_str()) {
                        return Err(format!("observation.rs exposes unlisted free `pub fn {f}`"));
                    }
                }
            }
            let toks = tokenize_rust(&source);
            for w in toks.windows(4) {
                if matches!(&w[0], Tok::Ident(a) if a == "SweepMode")
                    && matches!(w[1], Tok::Other(':'))
                    && matches!(w[2], Tok::Other(':'))
                    && matches!(&w[3], Tok::Ident(b) if b == "Live")
                {
                    return Err("observation.rs references SweepMode::Live".to_owned());
                }
            }
        }

        // C2: restrict host-only `SeatObservation` mandate stamping strictly to
        // `observation.rs::build_seat_observation_from_engine_state` -> `engine.rs::observe_seat_with_mandate`
        // -> `club/host.rs::ClubHost::observe_seat`.
        for tok in tokenize_rust(&source) {
            if let Tok::Ident(ident) = tok {
                match ident.as_str() {
                    "with_own_mandate" => {
                        return Err(format!(
                            "league-core/src/{rel}: deleted `with_own_mandate` builder must not reappear"
                        ));
                    }
                    "from_host_parts" if rel != "observation.rs" => {
                        return Err(format!(
                            "league-core/src/{rel}: `SeatObservation::from_host_parts` may only be called from `observation.rs`"
                        ));
                    }
                    "observe_seat_with_mandate" if rel != "engine.rs" && rel != "club/host.rs" => {
                        return Err(format!(
                            "league-core/src/{rel}: `observe_seat_with_mandate` may only be used by `engine.rs` and `club/host.rs`"
                        ));
                    }
                    "build_seat_observation_from_engine_state"
                        if rel != "observation.rs" && rel != "engine.rs" =>
                    {
                        return Err(format!(
                            "league-core/src/{rel}: `build_seat_observation_from_engine_state` may only be used by `observation.rs` and `engine.rs`"
                        ));
                    }
                    _ => {}
                }
            }
        }

        // Seat-side rule inside `league-core/src`: `bot` module and non-host `club/` files.
        if rel == "club/mod.rs" {
            check_club_mod_rs_source(&rel, &source)?;
        }
        if rel == "bot.rs" || rel.starts_with("bot/") {
            bot_seat_files.push((rel.clone(), source.clone()));
        } else if rel.starts_with("club/") && rel != "club/host.rs" && rel != "club/mod.rs" {
            club_seat_files.push((rel.clone(), source));
        }

        visited.push(rel);
    }

    if !bot_seat_files.is_empty() {
        scan_seat_files_two_pass(&bot_seat_files, true, true)?;
    }
    if !club_seat_files.is_empty() {
        scan_seat_files_two_pass(&club_seat_files, false, true)?;
    }

    if !saw_engine_internals {
        return Err("lib.rs is missing `#[cfg(feature = \"engine-internals\")] #[doc(hidden)] pub mod engine_internals`".to_owned());
    }
    if !observation_checked || !saw_preview_what_if || !saw_preview_seat_bundle {
        return Err(
            "observation.rs must keep public `preview_what_if` and `preview_seat_bundle`"
                .to_owned(),
        );
    }

    Ok(visited)
}

fn scan_league_core_src_tree(src_root: &Path) -> Result<Vec<String>, String> {
    scan_league_core_src_tree_with_surface(src_root, &PRODUCTION_SURFACE)
}

fn parse_toml_features(manifest: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut features = BTreeMap::new();
    let mut section = String::new();
    let mut pending_key: Option<String> = None;
    let mut pending_buf = String::new();

    for raw_line in manifest.lines() {
        let code = raw_line.split('#').next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        if pending_key.is_none() && code.starts_with('[') && code.ends_with(']') {
            section = code.to_owned();
            continue;
        }
        if section != "[features]" {
            continue;
        }
        if let Some(key) = pending_key.take() {
            pending_buf.push(' ');
            pending_buf.push_str(code);
            if pending_buf.contains(']') {
                features.insert(key, extract_quoted_strings(&pending_buf));
                pending_buf.clear();
            } else {
                pending_key = Some(key);
            }
            continue;
        }
        if let Some((lhs, rhs)) = code.split_once('=') {
            let key = lhs.trim().trim_matches('"').to_owned();
            let rhs_trim = rhs.trim();
            if rhs_trim.contains('[') && !rhs_trim.contains(']') {
                pending_key = Some(key);
                pending_buf = rhs_trim.to_owned();
            } else {
                features.insert(key, extract_quoted_strings(rhs_trim));
            }
        }
    }
    if let Some(unclosed) = pending_key {
        return Err(format!("unclosed `[features]` array for `{unclosed}`"));
    }
    Ok(features)
}

fn extract_quoted_strings(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.char_indices();
    while let Some((start, ch)) = chars.next() {
        if ch == '"' {
            let mut end = None;
            for (pos, c2) in chars.by_ref() {
                if c2 == '"' {
                    end = Some(pos);
                    break;
                }
            }
            if let Some(e) = end {
                out.push(s[start + 1..e].to_owned());
            }
        }
    }
    out
}

fn feature_reaches_engine_internals(
    features: &BTreeMap<String, Vec<String>>,
    start: &str,
    visited: &mut BTreeSet<String>,
) -> bool {
    let Some(items) = features.get(start) else {
        return false;
    };
    for item in items {
        let base = item.strip_prefix("dep:").unwrap_or(item);
        if base == "engine-internals" {
            return true;
        }
        if features.contains_key(base)
            && visited.insert(base.to_owned())
            && feature_reaches_engine_internals(features, base, visited)
        {
            return true;
        }
    }
    false
}

fn check_league_core_manifest(manifest: &str) -> Result<(), String> {
    let features = parse_toml_features(manifest)?;
    let Some(ei_deps) = features.get("engine-internals") else {
        return Err(
            "league-core/Cargo.toml must define non-default feature `engine-internals = []`"
                .to_owned(),
        );
    };
    if !ei_deps.is_empty() {
        return Err(
            "league-core/Cargo.toml `engine-internals` feature list must remain empty `[]`"
                .to_owned(),
        );
    }

    // B3: forbid `default` or ANY alias feature in `league-core` from reaching `engine-internals`.
    for feat_name in features.keys() {
        if feat_name == "engine-internals" {
            continue;
        }
        let mut visited = BTreeSet::new();
        visited.insert(feat_name.clone());
        if feature_reaches_engine_internals(&features, feat_name, &mut visited) {
            return Err(format!(
                "league-core/Cargo.toml alias feature `{feat_name}` must not enable `engine-internals`"
            ));
        }
        return Err(format!(
            "league-core/Cargo.toml defines unexpected feature `{feat_name}` (only `engine-internals = []` is permitted)"
        ));
    }

    let mut section = String::new();
    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_owned();
            continue;
        }
        if section != "[features]"
            && section != "[dev-dependencies]"
            && line.contains("engine-internals")
        {
            return Err(format!(
                "league-core/Cargo.toml enables `engine-internals` in unexpected section `{section}`"
            ));
        }
    }
    Ok(())
}

fn check_league_error_leaf_manifest(manifest: &str) -> Result<(), String> {
    for raw_line in manifest.lines() {
        let code_part = raw_line.split('#').next().unwrap_or("").trim();
        if code_part.contains("engine-internals") {
            return Err(
                "league-error/Cargo.toml must not enable or define `engine-internals`".to_owned(),
            );
        }
    }

    let mut section = String::new();
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut active_dep: Option<String> = None;
    let mut serde_no_default: BTreeMap<String, bool> = BTreeMap::new();

    for raw_line in manifest.lines() {
        let code_part = raw_line.split('#').next().unwrap_or("").trim();
        if code_part.is_empty() {
            continue;
        }
        let was_depth_zero = brace_depth == 0 && bracket_depth == 0;
        if was_depth_zero && code_part.starts_with('[') && code_part.ends_with(']') {
            section = code_part.to_owned();
            active_dep = None;
            if !matches!(section.as_str(), "[package]" | "[dependencies]" | "[lints]") {
                return Err(format!(
                    "league-error/Cargo.toml defines forbidden section `{section}`; only `[package]`, `[dependencies]`, and `[lints]` are permitted"
                ));
            }
            continue;
        }
        if section.is_empty() {
            return Err(format!(
                "league-error/Cargo.toml contains entry `{code_part}` before any `[section]` header"
            ));
        }
        if section == "[dependencies]" {
            if was_depth_zero && let Some((lhs, _rhs)) = code_part.split_once('=') {
                let dep_key = lhs.trim().trim_matches('"').trim_matches('\'');
                if !matches!(dep_key, "serde" | "serde_json") {
                    return Err(format!(
                        "league-error/Cargo.toml dependency `{dep_key}` is not in allow-list `[\"serde\", \"serde_json\"]`"
                    ));
                }
                active_dep = Some(dep_key.to_owned());
                serde_no_default.entry(dep_key.to_owned()).or_insert(false);
            }
            if code_part.contains("path")
                || code_part.contains("git")
                || code_part.contains("package")
            {
                return Err(
                    "league-error/Cargo.toml must only depend on registry `serde` and `serde_json` without `path`, `git`, or `package` overrides"
                        .to_owned(),
                );
            }
            if let Some(ref dep) = active_dep
                && matches!(dep.as_str(), "serde" | "serde_json")
                && has_default_features_false(code_part)
            {
                serde_no_default.insert(dep.clone(), true);
            }
            for ch in code_part.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth = brace_depth.saturating_sub(1),
                    '[' => bracket_depth += 1,
                    ']' => bracket_depth = bracket_depth.saturating_sub(1),
                    _ => {}
                }
            }
            if brace_depth == 0 && bracket_depth == 0 {
                active_dep = None;
            }
        } else if section == "[lints]" && code_part != "workspace = true" {
            return Err(format!(
                "league-error/Cargo.toml `[lints]` only permits `workspace = true`, found `{code_part}`"
            ));
        }
    }
    for (dep_name, has_no_default) in serde_no_default {
        if !has_no_default {
            return Err(format!(
                "league-error/Cargo.toml dependency `{dep_name}` must set `default-features = false` for `no_std`"
            ));
        }
    }
    Ok(())
}

fn parse_workspace_members(workspace_root: &Path) -> Result<Vec<PathBuf>, String> {
    let manifest_path = workspace_root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;

    let mut in_workspace = false;
    let mut collecting = false;
    let mut members_buf = String::new();
    let mut found_members = false;

    for raw_line in manifest.lines() {
        let code = raw_line.split('#').next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        if !collecting && code.starts_with('[') && code.ends_with(']') {
            in_workspace = code == "[workspace]";
            continue;
        }
        if !in_workspace {
            continue;
        }
        if !collecting {
            if let Some((lhs, rhs)) = code.split_once('=')
                && lhs.trim() == "members"
            {
                found_members = true;
                members_buf.push_str(rhs.trim());
                if rhs.contains(']') {
                    break;
                }
                collecting = true;
            }
        } else {
            members_buf.push(' ');
            members_buf.push_str(code);
            if code.contains(']') {
                break;
            }
        }
    }

    if !found_members {
        return Err(format!(
            "{}: missing `[workspace].members` list",
            manifest_path.display()
        ));
    }
    let rel_members = extract_quoted_strings(&members_buf);
    if rel_members.is_empty() {
        return Err(format!(
            "{}: `[workspace].members` resolved to zero members",
            manifest_path.display()
        ));
    }
    for rel in &rel_members {
        if rel.contains('*') || rel.contains('?') || rel.contains('[') {
            return Err(format!(
                "{}: glob workspace member `{rel}` is unsupported; list each workspace member explicitly",
                manifest_path.display()
            ));
        }
    }
    let mut dirs: Vec<PathBuf> = rel_members
        .into_iter()
        .map(|rel| workspace_root.join(rel))
        .collect();
    dirs.sort();
    Ok(dirs)
}

fn is_tui_host_module(rel_crate_path: &str) -> bool {
    TUI_HOST_MODULES.contains(&rel_crate_path)
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    for (idx, ch) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_double {
            if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            '#' => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn update_toml_depths(code: &str, brace_depth: &mut usize, bracket_depth: &mut usize) {
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    for ch in code.chars() {
        if escape {
            escape = false;
            continue;
        }
        if in_double {
            if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            '{' => *brace_depth += 1,
            '}' => *brace_depth = brace_depth.saturating_sub(1),
            '[' => *bracket_depth += 1,
            ']' => *bracket_depth = bracket_depth.saturating_sub(1),
            _ => {}
        }
    }
}

fn split_toml_dotted_segments(input: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    for ch in input.chars() {
        if escape {
            cur.push(ch);
            escape = false;
            continue;
        }
        if in_double {
            if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_double = false;
            } else {
                cur.push(ch);
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                cur.push(ch);
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            '.' => {
                let t = cur.trim();
                if !t.is_empty() {
                    segs.push(t.to_owned());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        segs.push(t.to_owned());
    }
    segs
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push(comp);
                }
            }
            _ => out.push(comp),
        }
    }
    out
}

fn extract_toml_key_quoted_values(snippet: &str, target_key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = snippet.as_bytes();
    let mut i = 0_usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i = (i + 1).min(bytes.len());
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'-' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
                {
                    i += 1;
                }
                let ident = &snippet[start..i];
                if ident == target_key {
                    let mut j = i;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'=' {
                        j += 1;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                            let quote = bytes[j];
                            j += 1;
                            let val_start = j;
                            while j < bytes.len() {
                                if quote == b'"' && bytes[j] == b'\\' {
                                    j += 2;
                                    continue;
                                }
                                if bytes[j] == quote {
                                    out.push(snippet[val_start..j].to_owned());
                                    j += 1;
                                    break;
                                }
                                j += 1;
                            }
                            i = j;
                        }
                    }
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn is_cargo_dep_kind(kind: &str) -> bool {
    matches!(
        kind,
        "dependencies"
            | "dev-dependencies"
            | "build-dependencies"
            | "dev_dependencies"
            | "build_dependencies"
    )
}

fn parse_workspace_dep_aliases(
    workspace_root: &Path,
    known_ws_dirs: &BTreeMap<PathBuf, String>,
) -> Result<BTreeMap<String, String>, String> {
    let manifest_path = workspace_root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    let mut aliases = BTreeMap::new();
    let mut in_ws_deps = false;
    let mut subtable_key: Option<String> = None;
    let mut brace_depth = 0_usize;
    let mut bracket_depth = 0_usize;

    for raw_line in manifest.lines() {
        let code_part = strip_toml_comment(raw_line).trim();
        if code_part.is_empty() {
            continue;
        }
        let was_depth_zero = brace_depth == 0 && bracket_depth == 0;
        if was_depth_zero && code_part.starts_with('[') && code_part.ends_with(']') {
            let inner = code_part
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(code_part);
            let segs = split_toml_dotted_segments(inner);
            in_ws_deps = false;
            subtable_key = None;
            if segs.len() == 2 && segs[0] == "workspace" && segs[1] == "dependencies" {
                in_ws_deps = true;
            } else if segs.len() == 3 && segs[0] == "workspace" && segs[1] == "dependencies" {
                in_ws_deps = true;
                subtable_key = Some(segs[2].clone());
            }
            continue;
        }
        if in_ws_deps {
            if let Some(ref alias) = subtable_key {
                if let Some(pkg) = extract_toml_key_quoted_values(code_part, "package")
                    .into_iter()
                    .next()
                {
                    aliases.insert(alias.clone(), pkg);
                }
                for rel_path in extract_toml_key_quoted_values(code_part, "path") {
                    let norm = normalize_lexical_path(&workspace_root.join(&rel_path));
                    if let Some(ws_name) = known_ws_dirs.get(&norm) {
                        aliases.insert(alias.clone(), ws_name.clone());
                    }
                }
            } else if was_depth_zero && let Some((lhs, rhs)) = code_part.split_once('=') {
                let lhs_segs = split_toml_dotted_segments(lhs);
                if let Some(alias) = lhs_segs.first() {
                    for pkg in extract_toml_key_quoted_values(rhs, "package") {
                        aliases.insert(alias.clone(), pkg);
                    }
                    for rel_path in extract_toml_key_quoted_values(rhs, "path") {
                        let norm = normalize_lexical_path(&workspace_root.join(&rel_path));
                        if let Some(ws_name) = known_ws_dirs.get(&norm) {
                            aliases.insert(alias.clone(), ws_name.clone());
                        }
                    }
                    if lhs_segs.get(1).map(String::as_str) == Some("package")
                        && let Some(pkg) = extract_quoted_strings(rhs).into_iter().next()
                    {
                        aliases.insert(alias.clone(), pkg);
                    }
                    if lhs_segs.get(1).map(String::as_str) == Some("path")
                        && let Some(rel_path) = extract_quoted_strings(rhs).into_iter().next()
                    {
                        let norm = normalize_lexical_path(&workspace_root.join(&rel_path));
                        if let Some(ws_name) = known_ws_dirs.get(&norm) {
                            aliases.insert(alias.clone(), ws_name.clone());
                        }
                    }
                }
            }
        }
        update_toml_depths(code_part, &mut brace_depth, &mut bracket_depth);
    }

    Ok(aliases)
}

fn parse_workspace_crate_edges(
    crate_name: &str,
    crate_dir: &Path,
    manifest: &str,
    ws_dep_aliases: &BTreeMap<String, String>,
    known_ws_dirs: &BTreeMap<PathBuf, String>,
) -> Result<Vec<(String, String)>, String> {
    let norm_crate_dir = normalize_lexical_path(crate_dir);
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut section = String::new();
    let mut active_dep_section: Option<String> = None;
    let mut subtable_dep_key: Option<String> = None;
    let mut in_target_root = false;
    let mut target_root_sec = String::new();
    let mut brace_depth = 0_usize;
    let mut bracket_depth = 0_usize;

    let mut push_edge = |target: String, sec: &str| {
        if target == crate_name {
            return;
        }
        let pair = (target, sec.to_owned());
        if !edges.contains(&pair) {
            edges.push(pair);
        }
    };

    for raw_line in manifest.lines() {
        let code_part = strip_toml_comment(raw_line).trim();
        if code_part.is_empty() {
            continue;
        }
        let was_depth_zero = brace_depth == 0 && bracket_depth == 0;
        if was_depth_zero && code_part.starts_with('[') && code_part.ends_with(']') {
            section = code_part.to_owned();
            active_dep_section = None;
            subtable_dep_key = None;
            in_target_root = false;
            target_root_sec.clear();
            let is_array_table = code_part.starts_with("[[") && code_part.ends_with("]]");
            if !is_array_table {
                let inner = &code_part[1..code_part.len() - 1];
                let segs = split_toml_dotted_segments(inner);
                if segs.len() == 1 && is_cargo_dep_kind(&segs[0]) {
                    active_dep_section = Some(section.clone());
                } else if segs.len() == 2 && is_cargo_dep_kind(&segs[0]) {
                    active_dep_section = Some(section.clone());
                    subtable_dep_key = Some(segs[1].clone());
                    let resolved = ws_dep_aliases
                        .get(&segs[1])
                        .cloned()
                        .unwrap_or_else(|| segs[1].clone());
                    push_edge(resolved, &section);
                } else if segs.len() == 2 && segs[0] == "target" {
                    in_target_root = true;
                    target_root_sec = section.clone();
                } else if segs.len() == 3 && segs[0] == "target" && is_cargo_dep_kind(&segs[2]) {
                    active_dep_section = Some(section.clone());
                } else if segs.len() == 4 && segs[0] == "target" && is_cargo_dep_kind(&segs[2]) {
                    active_dep_section = Some(section.clone());
                    subtable_dep_key = Some(segs[3].clone());
                    let resolved = ws_dep_aliases
                        .get(&segs[3])
                        .cloned()
                        .unwrap_or_else(|| segs[3].clone());
                    push_edge(resolved, &section);
                }
            }
            continue;
        }

        if section.is_empty() && was_depth_zero {
            return Err(format!(
                "{crate_name}/Cargo.toml contains entry `{code_part}` before any `[section]` header"
            ));
        }

        let eff_sec = if let Some(ref sec) = active_dep_section {
            Some(sec.clone())
        } else if in_target_root && was_depth_zero {
            code_part.split_once('=').and_then(|(lhs, _)| {
                let segs = split_toml_dotted_segments(lhs);
                if segs.len() >= 2 && is_cargo_dep_kind(&segs[0]) {
                    Some(target_root_sec.clone())
                } else {
                    None
                }
            })
        } else {
            None
        };

        if let Some(sec) = eff_sec {
            if let Some(ref sub_dep) = subtable_dep_key {
                if was_depth_zero && let Some((lhs, rhs)) = code_part.split_once('=') {
                    let lhs_segs = split_toml_dotted_segments(lhs);
                    if lhs_segs == ["package"] {
                        for pkg in extract_quoted_strings(rhs) {
                            push_edge(pkg, &sec);
                        }
                    } else if lhs_segs == ["path"] {
                        for rel_path in extract_quoted_strings(rhs) {
                            let norm = normalize_lexical_path(&crate_dir.join(&rel_path));
                            if norm != norm_crate_dir {
                                if let Some(ws_name) = known_ws_dirs.get(&norm) {
                                    push_edge(ws_name.clone(), &sec);
                                } else if let Some(fname) =
                                    norm.file_name().and_then(|s| s.to_str())
                                    && matches!(
                                        fname,
                                        "league-core" | "league-gen" | "league-standalone"
                                    )
                                {
                                    push_edge(fname.to_owned(), &sec);
                                }
                            }
                        }
                    } else if lhs_segs == ["workspace"] {
                        let resolved = ws_dep_aliases
                            .get(sub_dep)
                            .cloned()
                            .unwrap_or_else(|| sub_dep.clone());
                        push_edge(resolved, &sec);
                    }
                }
            } else {
                if was_depth_zero && let Some((lhs, rhs)) = code_part.split_once('=') {
                    let lhs_segs = split_toml_dotted_segments(lhs);
                    let (key_idx, extra_idx) = if in_target_root { (1, 2) } else { (0, 1) };
                    if let Some(dep_key) = lhs_segs.get(key_idx) {
                        let resolved = ws_dep_aliases
                            .get(dep_key)
                            .cloned()
                            .unwrap_or_else(|| dep_key.clone());
                        push_edge(resolved, &sec);
                        if lhs_segs.get(extra_idx).map(String::as_str) == Some("package") {
                            for pkg in extract_quoted_strings(rhs) {
                                push_edge(pkg, &sec);
                            }
                        } else if lhs_segs.get(extra_idx).map(String::as_str) == Some("path") {
                            for rel_path in extract_quoted_strings(rhs) {
                                let norm = normalize_lexical_path(&crate_dir.join(&rel_path));
                                if norm != norm_crate_dir {
                                    if let Some(ws_name) = known_ws_dirs.get(&norm) {
                                        push_edge(ws_name.clone(), &sec);
                                    } else if let Some(fname) =
                                        norm.file_name().and_then(|s| s.to_str())
                                        && matches!(
                                            fname,
                                            "league-core" | "league-gen" | "league-standalone"
                                        )
                                    {
                                        push_edge(fname.to_owned(), &sec);
                                    }
                                }
                            }
                        }
                    }
                }
                for pkg in extract_toml_key_quoted_values(code_part, "package") {
                    push_edge(pkg, &sec);
                }
                for rel_path in extract_toml_key_quoted_values(code_part, "path") {
                    let norm = normalize_lexical_path(&crate_dir.join(&rel_path));
                    if norm != norm_crate_dir {
                        if let Some(ws_name) = known_ws_dirs.get(&norm) {
                            push_edge(ws_name.clone(), &sec);
                        } else if let Some(fname) = norm.file_name().and_then(|s| s.to_str())
                            && matches!(fname, "league-core" | "league-gen" | "league-standalone")
                        {
                            push_edge(fname.to_owned(), &sec);
                        }
                    }
                }
            }
        }

        update_toml_depths(code_part, &mut brace_depth, &mut bracket_depth);
    }

    Ok(edges)
}

fn find_dep_path_to_forbidden(
    start: &str,
    edges: &BTreeMap<String, Vec<(String, String)>>,
    forbidden: &[&str],
) -> Option<(Vec<String>, String, String)> {
    let mut visited = BTreeSet::new();
    let mut queue = std::collections::VecDeque::new();
    if let Some(direct) = edges.get(start) {
        for (target, sec) in direct {
            if forbidden.contains(&target.as_str()) {
                return Some((
                    vec![start.to_owned(), target.clone()],
                    sec.clone(),
                    target.clone(),
                ));
            }
            if visited.insert(target.clone()) {
                queue.push_back((
                    target.clone(),
                    vec![start.to_owned(), target.clone()],
                    sec.clone(),
                ));
            }
        }
    }
    while let Some((node, chain, first_sec)) = queue.pop_front() {
        if let Some(nexts) = edges.get(&node) {
            for (target, sec) in nexts {
                let mut next_chain = chain.clone();
                next_chain.push(target.clone());
                if forbidden.contains(&target.as_str()) {
                    return Some((next_chain, format!("{first_sec} -> {sec}"), target.clone()));
                }
                if visited.insert(target.clone()) {
                    queue.push_back((target.clone(), next_chain, first_sec.clone()));
                }
            }
        }
    }
    None
}

fn toml_snippet_has_key(snippet: &str, target_key: &str) -> bool {
    let bytes = snippet.as_bytes();
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    let mut i = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            escape = false;
            i += 1;
            continue;
        }
        if in_double {
            if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_double = true;
                i += 1;
            }
            b'\'' => {
                in_single = true;
                i += 1;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'-' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
                {
                    i += 1;
                }
                if &snippet[start..i] == target_key {
                    let mut j = i;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && matches!(bytes[j], b'=' | b'.') {
                        return true;
                    }
                }
            }
            _ => i += 1,
        }
    }
    false
}

fn check_seat_crate_manifest(
    crate_name: &str,
    manifest: &str,
    allowed_deps: &[(&str, Option<&str>)],
) -> Result<(), String> {
    let allowed_names: Vec<&str> = allowed_deps.iter().map(|(n, _)| *n).collect();
    for raw_line in manifest.lines() {
        let code_part = strip_toml_comment(raw_line).trim();
        if code_part.contains("engine-internals") {
            return Err(format!(
                "{crate_name}/Cargo.toml must not enable or define `engine-internals`"
            ));
        }
    }

    let mut section = String::new();
    let mut subtable_dep: Option<String> = None;
    let mut active_inline_dep: Option<String> = None;
    let mut saw_lib_doctest_false = false;
    let mut serde_no_default: BTreeMap<String, bool> = BTreeMap::new();
    let mut brace_depth = 0_usize;
    let mut bracket_depth = 0_usize;

    for raw_line in manifest.lines() {
        let code_part = strip_toml_comment(raw_line).trim();
        if code_part.is_empty() {
            continue;
        }
        let was_depth_zero = brace_depth == 0 && bracket_depth == 0;
        if was_depth_zero && code_part.starts_with('[') && code_part.ends_with(']') {
            section = code_part.to_owned();
            subtable_dep = None;
            active_inline_dep = None;
            if matches!(
                section.as_str(),
                "[package]" | "[lib]" | "[dependencies]" | "[lints]"
            ) {
                continue;
            }
            let inner = &code_part[1..code_part.len() - 1];
            let segs = split_toml_dotted_segments(inner);
            if segs.len() == 2 && segs[0] == "dependencies" {
                let dep_name = segs[1].as_str();
                if !allowed_names.contains(&dep_name) {
                    return Err(format!(
                        "{crate_name}/Cargo.toml dependency `{dep_name}` is not in allow-list `{allowed_names:?}`"
                    ));
                }
                subtable_dep = Some(dep_name.to_owned());
                if matches!(dep_name, "serde" | "serde_json") {
                    serde_no_default.entry(dep_name.to_owned()).or_insert(false);
                }
                continue;
            }
            return Err(format!(
                "{crate_name}/Cargo.toml defines forbidden section `{section}`; only `[package]`, `[lib]`, `[dependencies]`, and `[lints]` are permitted"
            ));
        }
        if section.is_empty() && was_depth_zero {
            return Err(format!(
                "{crate_name}/Cargo.toml contains entry `{code_part}` before any `[section]` header"
            ));
        }
        if section == "[package]" && was_depth_zero && toml_snippet_has_key(code_part, "build") {
            return Err(format!(
                "{crate_name}/Cargo.toml must not define `build` script"
            ));
        }
        if section == "[lib]" {
            let is_exact_doctest_false = code_part
                .split_once('=')
                .is_some_and(|(lhs, rhs)| lhs.trim() == "doctest" && rhs.trim() == "false");
            if !is_exact_doctest_false {
                return Err(format!(
                    "{crate_name}/Cargo.toml `[lib]` only permits `doctest = false`, found `{code_part}`"
                ));
            }
            saw_lib_doctest_false = true;
            continue;
        }
        if section == "[dependencies]" || subtable_dep.is_some() {
            for forbidden_kw in [
                "league-core",
                "league-gen",
                "league-standalone",
                "league-tui",
            ] {
                if code_part.contains(forbidden_kw) {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not depend on `{forbidden_kw}`"
                    ));
                }
            }
            if subtable_dep.is_none() && was_depth_zero {
                active_inline_dep = code_part.split_once('=').and_then(|(lhs, _)| {
                    let segs = split_toml_dotted_segments(lhs);
                    segs.first().cloned()
                });
            }
            let current_dep = subtable_dep
                .as_ref()
                .or(active_inline_dep.as_ref())
                .cloned();
            if let Some(ref dep_key) = current_dep {
                if !allowed_names.contains(&dep_key.as_str()) {
                    return Err(format!(
                        "{crate_name}/Cargo.toml dependency `{dep_key}` is not in allow-list `{allowed_names:?}`"
                    ));
                }
                if matches!(dep_key.as_str(), "serde" | "serde_json") {
                    serde_no_default.entry(dep_key.clone()).or_insert(false);
                    if has_default_features_false(code_part) {
                        serde_no_default.insert(dep_key.clone(), true);
                    }
                }
            }
            if toml_snippet_has_key(code_part, "git")
                || toml_snippet_has_key(code_part, "package")
                || toml_snippet_has_key(code_part, "optional")
            {
                return Err(format!(
                    "{crate_name}/Cargo.toml dependency entries must not set `git`, `package`, or `optional` (`{code_part}`)"
                ));
            }
            let expected_path = current_dep.as_deref().and_then(|dk| {
                allowed_deps
                    .iter()
                    .find(|(n, _)| *n == dk)
                    .and_then(|(_, p)| *p)
            });
            for rel_path in extract_toml_key_quoted_values(code_part, "path") {
                match expected_path {
                    Some(exp) if rel_path == exp => {}
                    Some(exp) => {
                        return Err(format!(
                            "{crate_name}/Cargo.toml path dependency must point to `{exp}`, found `{rel_path}`"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "{crate_name}/Cargo.toml external dependency must not use `path` override (`{rel_path}`)"
                        ));
                    }
                }
            }
            if let Some((_, rhs)) = code_part.split_once('=')
                && was_depth_zero
                && subtable_dep.as_deref().is_some()
                && code_part.starts_with("path")
            {
                for rel_path in extract_quoted_strings(rhs) {
                    match expected_path {
                        Some(exp) if rel_path == exp => {}
                        _ => {
                            return Err(format!(
                                "{crate_name}/Cargo.toml unexpected path `{rel_path}`"
                            ));
                        }
                    }
                }
            }
            update_toml_depths(code_part, &mut brace_depth, &mut bracket_depth);
            if brace_depth == 0 && bracket_depth == 0 {
                active_inline_dep = None;
            }
        } else if section == "[lints]" && code_part != "workspace = true" {
            return Err(format!(
                "{crate_name}/Cargo.toml `[lints]` only permits `workspace = true`, found `{code_part}`"
            ));
        }
    }
    if !saw_lib_doctest_false {
        return Err(format!(
            "{crate_name}/Cargo.toml must set `[lib]` with `doctest = false`"
        ));
    }
    for (dep_name, has_no_default) in serde_no_default {
        if !has_no_default {
            return Err(format!(
                "{crate_name}/Cargo.toml dependency `{dep_name}` must set `default-features = false` for `no_std`"
            ));
        }
    }
    Ok(())
}

fn check_tui_main_no_pub_items_and_no_macros(rel_path: &str, source: &str) -> Result<(), String> {
    let tokens = tokenize_rust(source);
    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut i = 0_usize;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
            | ScopeKind::TraitImpl { cfg_test, .. }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });

        if matches!(&tokens[i], Tok::Pound) {
            if !in_cfg_test && !attrs.cfg_test && tokens.get(i + 1) == Some(&Tok::LBracket) {
                let mut d = 1_usize;
                let mut j = i + 2;
                while j < tokens.len() && d > 0 {
                    match tokens[j] {
                        Tok::LBracket => d += 1,
                        Tok::RBracket => d -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let inner = &tokens[i + 2..j.saturating_sub(1)];
                if inner
                    .iter()
                    .any(|t| matches!(t, Tok::Ident(s) if s == "macro_export"))
                {
                    return Err(format!(
                        "{rel_path}: `#[macro_export]` is forbidden in `league-tui` host binary"
                    ));
                }
            }
            i = parse_tok_attr(&tokens, i, &mut attrs);
            continue;
        }

        match &tokens[i] {
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        if matches!(&tokens[i], Tok::Ident(s) if s == "macro_rules")
            && matches!(tokens.get(i + 1), Some(Tok::Bang))
        {
            return Err(format!(
                "{rel_path}: `macro_rules!` is forbidden in `league-tui` host binary"
            ));
        }

        if rel_path == "src/main.rs"
            && matches!(&tokens[i], Tok::Ident(s) if s == "pub")
            && !matches!(tokens.get(i + 1), Some(Tok::LParen))
        {
            return Err(format!(
                "{rel_path}: `league-tui` crate root `src/main.rs` must not expose `pub` items outside `#[cfg(test)]`"
            ));
        }

        i += 1;
    }

    Ok(())
}

fn load_crate_rs_files(crate_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut rs_files = Vec::new();
    collect_rs_files_recursive(crate_dir, &mut rs_files);
    rs_files.sort();
    let mut loaded = Vec::with_capacity(rs_files.len());
    for rs_path in rs_files {
        let src = fs::read_to_string(&rs_path)
            .map_err(|err| format!("failed to read {}: {err}", rs_path.display()))?;
        let rel_in_crate = rs_path
            .strip_prefix(crate_dir)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_else(|| rs_path.display().to_string());
        loaded.push((rel_in_crate, src));
    }
    Ok(loaded)
}

fn check_workspace_manifests_and_crate_sources(
    workspace_root: &Path,
) -> Result<Vec<String>, String> {
    let member_dirs = parse_workspace_members(workspace_root)?;
    let mut known_ws_dirs = BTreeMap::new();
    let mut member_manifests = Vec::with_capacity(member_dirs.len());

    for crate_dir in member_dirs {
        let manifest_path = crate_dir.join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
        let crate_name = crate_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        known_ws_dirs.insert(normalize_lexical_path(&crate_dir), crate_name.clone());
        member_manifests.push((crate_name, crate_dir, manifest));
    }

    let ws_dep_aliases = parse_workspace_dep_aliases(workspace_root, &known_ws_dirs)?;
    let mut all_edges: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (crate_name, crate_dir, manifest) in &member_manifests {
        let edges = parse_workspace_crate_edges(
            crate_name,
            crate_dir,
            manifest,
            &ws_dep_aliases,
            &known_ws_dirs,
        )?;
        all_edges.insert(crate_name.clone(), edges);
    }

    let mut checked_crates = Vec::with_capacity(member_manifests.len());
    for (crate_name, crate_dir, manifest) in member_manifests {
        if crate_name == "league-core" {
            check_league_core_manifest(&manifest)?;
        } else if crate_name == "league-standalone" {
            if !manifest.contains("engine-internals") {
                return Err(
                    "league-standalone/Cargo.toml is expected to opt into `league-core` `engine-internals`"
                        .to_owned(),
                );
            }
            let loaded = load_crate_rs_files(&crate_dir)?;
            let crate_aliases = collect_crate_restricted_aliases_from_files(&loaded);
            for (rel_in_crate, src) in &loaded {
                scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
                    rel_in_crate,
                    rel_in_crate == "src/lib.rs",
                    src,
                    false,
                    &crate_aliases,
                )?;
            }
        } else if crate_name == "league-seat-types" {
            check_seat_crate_manifest(
                "league-seat-types",
                &manifest,
                &[
                    ("league-error", Some("../league-error")),
                    ("serde", None),
                    ("serde_json", None),
                ],
            )?;
            if let Some((chain, edge_secs, hit)) = find_dep_path_to_forbidden(
                &crate_name,
                &all_edges,
                &["league-core", "league-gen", "league-standalone"],
            ) {
                if chain.len() == 2 {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not depend on `{hit}` on any dependency edge (found in `{edge_secs}`)"
                    ));
                }
                return Err(format!(
                    "{crate_name}/Cargo.toml must not reach `{hit}` transitively via workspace dependencies ({} via `{edge_secs}`)",
                    chain.join(" -> ")
                ));
            }
            check_seat_crate_nostd_and_channels(&crate_name, &crate_dir)?;
            let loaded = load_crate_rs_files(&crate_dir)?;
            for (rel_in_crate, src) in &loaded {
                let toks = tokenize_rust(src);
                let mut from_host_parts_count = 0_usize;
                for tok in toks {
                    if let Tok::Ident(ident) = tok {
                        if ident == "from_host_parts" && rel_in_crate == "src/observation.rs" {
                            from_host_parts_count += 1;
                            if from_host_parts_count > 1 {
                                return Err(format!(
                                    "{crate_name}/{rel_in_crate}: `from_host_parts` must only be defined once and never called inside `{crate_name}`"
                                ));
                            }
                            continue;
                        }
                        if FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str()) {
                            return Err(format!(
                                "{crate_name}/{rel_in_crate}: references forbidden engine/host symbol `{ident}`"
                            ));
                        }
                    }
                }
            }
            let crate_aliases = collect_crate_restricted_aliases_from_files(&loaded);
            check_seat_types_surface(&loaded)?;
            for (rel_in_crate, src) in &loaded {
                scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
                    rel_in_crate,
                    rel_in_crate == "src/lib.rs",
                    src,
                    true,
                    &crate_aliases,
                )?;
            }
        } else if crate_name == "league-seat-ui" {
            check_seat_crate_manifest(
                "league-seat-ui",
                &manifest,
                &[("league-seat-types", Some("../league-seat-types"))],
            )?;
            if let Some((chain, edge_secs, hit)) = find_dep_path_to_forbidden(
                &crate_name,
                &all_edges,
                &["league-core", "league-gen", "league-standalone"],
            ) {
                if chain.len() == 2 {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not depend on `{hit}` on any dependency edge (found in `{edge_secs}`)"
                    ));
                }
                return Err(format!(
                    "{crate_name}/Cargo.toml must not reach `{hit}` transitively via workspace dependencies ({} via `{edge_secs}`)",
                    chain.join(" -> ")
                ));
            }
            check_seat_crate_nostd_and_channels(&crate_name, &crate_dir)?;
            let loaded = load_crate_rs_files(&crate_dir)?;
            let crate_aliases = collect_crate_restricted_aliases_from_files(&loaded);
            let local_types = collect_seat_local_types(&loaded, false);
            for (rel_in_crate, src) in &loaded {
                let toks = tokenize_rust(src);
                for tok in &toks {
                    if let Tok::Ident(ident) = tok
                        && FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str())
                    {
                        return Err(format!(
                            "{crate_name}/{rel_in_crate}: references forbidden engine/host symbol `{ident}`"
                        ));
                    }
                }
                scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
                    rel_in_crate,
                    rel_in_crate == "src/lib.rs",
                    src,
                    true,
                    &crate_aliases,
                )?;
                check_seat_ui_public_surface(rel_in_crate, src, &local_types)?;
            }
        } else {
            if crate_name == "league-error" {
                check_league_error_leaf_manifest(&manifest)?;
            }
            if crate_name == "league-tui" && crate_dir.join("src/lib.rs").exists() {
                return Err(
                    "league-tui must be a binary host crate without `src/lib.rs` (seat code lives in `league-seat-ui`)"
                        .to_owned(),
                );
            }
            let mut section = String::new();
            for raw_line in manifest.lines() {
                let code_part = strip_toml_comment(raw_line).trim();
                if code_part.starts_with('[') && code_part.ends_with(']') {
                    section = code_part.to_owned();
                }
                if code_part.contains("engine-internals") {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not enable `engine-internals`"
                    ));
                }
                if (section == "[features]" && code_part.contains("league-core/"))
                    || (code_part.contains("league-core") && code_part.contains("features"))
                {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not pass feature flags to `league-core`"
                    ));
                }
                if (crate_name == "league-ledger" || crate_name == "league-relay")
                    && section != "[package]"
                    && code_part.contains("league-core")
                {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not depend on or reference `league-core` (in `{section}`)"
                    ));
                }
                if crate_name == "league-tui" && section == "[lib]" {
                    return Err(
                        "league-tui/Cargo.toml must be binary-only (`[lib]` is forbidden)"
                            .to_owned(),
                    );
                }
                if crate_name == "league-tui" && code_part.contains("league-standalone") {
                    return Err(
                        "league-tui/Cargo.toml must not depend on `league-standalone`".to_owned(),
                    );
                }
            }

            let forbidden_for_crate: &[&str] = if crate_name == "league-error" {
                &["league-core", "league-gen", "league-standalone"]
            } else if crate_name == "league-ledger" || crate_name == "league-relay" {
                &["league-core"]
            } else {
                &[]
            };
            if !forbidden_for_crate.is_empty()
                && let Some((chain, edge_secs, hit)) =
                    find_dep_path_to_forbidden(&crate_name, &all_edges, forbidden_for_crate)
            {
                if chain.len() == 2 {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not depend on `{hit}` on any dependency edge (found in `{edge_secs}`)"
                    ));
                }
                return Err(format!(
                    "{crate_name}/Cargo.toml must not reach `{hit}` transitively via workspace dependencies ({} via `{edge_secs}`)",
                    chain.join(" -> ")
                ));
            }
            if crate_name == "league-error" {
                check_seat_crate_nostd_and_channels(&crate_name, &crate_dir)?;
            }

            let loaded = load_crate_rs_files(&crate_dir)?;
            let crate_aliases = collect_crate_restricted_aliases_from_files(&loaded);
            let is_non_seat_crate = NON_SEAT_WORKSPACE_CRATES.contains(&crate_name.as_str());
            let mut seat_files: Vec<(String, String)> = Vec::new();

            for (rel_in_crate, src) in loaded {
                scan_non_core_crate_aliases_and_reexports_with_crate_aliases(
                    &rel_in_crate,
                    rel_in_crate == "src/lib.rs",
                    &src,
                    !is_non_seat_crate || crate_name == "league-tui",
                    &crate_aliases,
                )?;

                if crate_name == "league-tui" {
                    if !is_tui_host_module(&rel_in_crate) && !rel_in_crate.starts_with("tests/") {
                        return Err(format!(
                            "league-tui/{rel_in_crate}: unexpected non-host module in `league-tui` binary host (seat logic belongs in `league-seat-ui`)"
                        ));
                    }
                    scan_tui_host_module_source(&rel_in_crate, &src)?;
                    check_tui_main_no_pub_items_and_no_macros(&rel_in_crate, &src)?;
                    for tok in tokenize_rust(&src) {
                        if let Tok::Ident(ident) = tok
                            && matches!(
                                ident.as_str(),
                                "from_host_parts"
                                    | "with_own_mandate"
                                    | "observe_seat_with_mandate"
                                    | "unmandated"
                                    | "SeatObservationPublicParts"
                                    | "serde"
                                    | "serde_json"
                                    | "Deserialize"
                                    | "from_str"
                                    | "from_slice"
                                    | "from_value"
                            )
                        {
                            return Err(format!(
                                "league-tui/{rel_in_crate}: references forbidden SeatObservation forgery/deserialization symbol `{ident}` (must construct SeatObservation solely via `ClubHost::observe_seat`)"
                            ));
                        }
                    }
                } else if is_non_seat_crate {
                    for tok in tokenize_rust(&src) {
                        if let Tok::Ident(ident) = tok
                            && (ident == "engine_internals"
                                || ident == "SweepMode"
                                || matches!(
                                    ident.as_str(),
                                    "from_host_parts"
                                        | "with_own_mandate"
                                        | "observe_seat_with_mandate"
                                )
                                || FORBIDDEN_RAW_FNS.contains(&ident.as_str()))
                        {
                            return Err(format!(
                                "{crate_name}/{rel_in_crate} references forbidden engine-internal symbol `{ident}`"
                            ));
                        }
                    }
                } else {
                    seat_files.push((rel_in_crate, src));
                }
            }

            if !seat_files.is_empty() {
                let check_all_paths = crate_name != "league-relay";
                scan_seat_files_two_pass(&seat_files, false, check_all_paths)?;
            }
        }
        checked_crates.push(crate_name);
    }

    let seen: BTreeSet<&str> = checked_crates.iter().map(String::as_str).collect();
    for expected in [
        "league-core",
        "league-error",
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
        "league-seat-types",
        "league-seat-ui",
        "league-standalone",
        "league-tui",
    ] {
        if !seen.contains(expected) {
            return Err(format!(
                "expected workspace crate `{expected}` was not scanned"
            ));
        }
    }
    Ok(checked_crates)
}

fn write_minimal_clean_core_tree(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("lib.rs"),
        r#"
pub mod engine;
pub mod observation;
#[cfg(feature = "engine-internals")]
#[doc(hidden)]
pub mod engine_internals {
    pub use crate::engine::sweep_mode::SweepMode;
    pub fn private_hints_for_auditor() {}
    pub fn preview_bundle_live() {}
}
"#,
    )?;
    fs::write(
        root.join("engine.rs"),
        r#"
pub(crate) mod sweep_mode {
    pub enum SweepMode { Live, PreviewSteps0To7 }
}
pub struct TurnEngine;
impl TurnEngine {
    pub fn observe_seat(&self) {}
    pub(crate) fn private_hints_for_auditor(&self) {}
    pub(crate) fn preview_bundle_live(&self) {}
}
"#,
    )?;
    fs::write(
        root.join("observation.rs"),
        r#"
pub fn preview_what_if() {}
pub fn preview_seat_bundle() {}
"#,
    )?;
    Ok(())
}

fn write_minimal_workspace_tree(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    let member_names = [
        "league-core",
        "league-error",
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
        "league-seat-types",
        "league-seat-ui",
        "league-standalone",
        "league-tui",
    ];
    let members_toml: String = member_names
        .iter()
        .map(|name| format!("    \"crates/{name}\",\n"))
        .collect();
    fs::write(
        root.join("Cargo.toml"),
        format!("[workspace]\nmembers = [\n{members_toml}]\n"),
    )?;
    let seat_clippy_toml = concat!(
        "disallowed-types = [\n",
        "    { path = \"core::cell::RefCell\" },\n",
        "    { path = \"core::cell::Cell\" },\n",
        "    { path = \"core::sync::atomic::AtomicBool\" },\n",
        "    { path = \"alloc::sync::Arc\" },\n",
        "    { path = \"alloc::rc::Rc\" },\n",
        "]\n",
        "disallowed-methods = []\n",
        "disallowed-macros = [\n",
        "    { path = \"core::include\" },\n",
        "    { path = \"core::include_str\" },\n",
        "    { path = \"core::include_bytes\" },\n",
        "    { path = \"core::env\" },\n",
        "    { path = \"core::option_env\" },\n",
        "]\n",
    );
    let seat_lib_stub = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
        "pub fn ok() {}\n",
    );
    for name in member_names {
        let crate_dir = root.join("crates").join(name);
        fs::create_dir_all(crate_dir.join("src"))?;
        if name == "league-tui" {
            fs::write(crate_dir.join("src/main.rs"), "fn main() {}\n")?;
        } else if matches!(
            name,
            "league-error" | "league-seat-types" | "league-seat-ui"
        ) {
            fs::write(crate_dir.join("clippy.toml"), seat_clippy_toml)?;
            fs::write(crate_dir.join("src/lib.rs"), seat_lib_stub)?;
        } else {
            fs::write(crate_dir.join("src/lib.rs"), "// clean stub\n")?;
        }
    }
    fs::write(
        root.join("crates/league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\nengine-internals = []\n[dev-dependencies]\nleague-core = { path = \".\", features = [\"engine-internals\"] }\n",
    )?;
    fs::write(
        root.join("crates/league-standalone/Cargo.toml"),
        "[package]\nname = \"league-standalone\"\n[dependencies]\nleague-core = { path = \"../league-core\", features = [\"engine-internals\"] }\n",
    )?;
    fs::write(
        root.join("crates/league-gen/Cargo.toml"),
        "[package]\nname = \"league-gen\"\n[dependencies]\nleague-core = { path = \"../league-core\" }\n",
    )?;
    fs::write(
        root.join("crates/league-seat-types/Cargo.toml"),
        "[package]\nname = \"league-seat-types\"\n[lib]\ndoctest = false\n[dependencies]\nleague-error = { path = \"../league-error\" }\n",
    )?;
    fs::write(
        root.join("crates/league-seat-ui/Cargo.toml"),
        "[package]\nname = \"league-seat-ui\"\n[lib]\ndoctest = false\n[dependencies]\nleague-seat-types = { path = \"../league-seat-types\" }\n",
    )?;
    for name in [
        "league-error",
        "league-ledger",
        "league-relay",
        "league-season",
    ] {
        fs::write(
            root.join("crates").join(name).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n"),
        )?;
    }
    fs::write(
        root.join("crates/league-tui/Cargo.toml"),
        "[package]\nname = \"league-tui\"\n[[bin]]\nname = \"league-tui\"\npath = \"src/main.rs\"\n[dependencies]\nleague-core = { path = \"../league-core\" }\nleague-seat-ui = { path = \"../league-seat-ui\" }\n",
    )?;
    Ok(())
}

fn unique_tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ml_guard_{label}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn raw_api_backdoors_stay_shut_and_seat_crates_cannot_enable_engine_internals() {
    let core_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = core_dir.join("src");
    let scanned_files = match scan_league_core_src_tree(&src_dir) {
        Ok(files) => files,
        Err(err) => panic!("raw API back-door guard failed on clean league-core/src: {err}"),
    };
    for expected in [
        "club/host.rs",
        "club/mod.rs",
        "commit.rs",
        "contracts.rs",
        "engine.rs",
        "error.rs",
        "lib.rs",
        "observation.rs",
        "predicates.rs",
        "replay.rs",
        "scoring.rs",
        "types.rs",
    ] {
        assert!(
            scanned_files.iter().any(|f| f == expected),
            "expected {expected} in scanned files: {scanned_files:?}"
        );
    }

    let workspace_root = core_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("expected workspace root directory"));
    let checked_crates = match check_workspace_manifests_and_crate_sources(workspace_root) {
        Ok(crates) => crates,
        Err(err) => panic!("workspace engine-internals isolation check failed: {err}"),
    };
    assert!(checked_crates.contains(&"league-error".to_owned()));
    assert!(checked_crates.contains(&"league-seat-types".to_owned()));
    assert!(checked_crates.contains(&"league-seat-ui".to_owned()));
    assert!(checked_crates.contains(&"league-tui".to_owned()));
    assert!(checked_crates.contains(&"league-relay".to_owned()));

    let ci_yml_path = workspace_root.join(".github/workflows/ci.yml");
    let ci_yml = fs::read_to_string(&ci_yml_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", ci_yml_path.display()));
    // Check that `.github/workflows/ci.yml` enforces the no-std target build gate (`thumbv7em-none-eabi`),
    // drops the duplicate clippy step, and keeps the `--target all` dependency checks.
    assert!(
        !ci_yml.contains("cargo clippy -p league-seat-types -p league-seat-ui"),
        ".github/workflows/ci.yml must drop the duplicate seat-crate clippy step"
    );
    assert!(
        ci_yml.contains(
            "cargo build -p league-error -p league-seat-types -p league-seat-ui --target thumbv7em-none-eabi --locked"
        ),
        ".github/workflows/ci.yml must build leaf/seat crates on thumbv7em-none-eabi"
    );

    assert_eq!(SweepMode::default(), SweepMode::PreviewSteps0To7);

    let tmp_root = unique_tmp_dir("core_controls");

    // Control A: `pub fn private_hints_for_auditor` restored on `TurnEngine` -> Err.
    let case_a = tmp_root.join("case_a");
    if let Err(err) = write_minimal_clean_core_tree(&case_a) {
        panic!("setup case_a: {err}");
    }
    assert!(scan_league_core_src_tree_with_surface(&case_a, &MINIMAL_TEST_SURFACE).is_ok());
    if let Err(err) = fs::write(
        case_a.join("engine.rs"),
        "pub struct TurnEngine;\nimpl TurnEngine {\n    pub fn private_hints_for_auditor(&self) {}\n}\n",
    ) {
        panic!("write case_a: {err}");
    }
    assert!(scan_league_core_src_tree_with_surface(&case_a, &MINIMAL_TEST_SURFACE).is_err());

    // Control B: `pub const fn preview_bundle` restored on `TurnEngine` -> Err.
    let case_b = tmp_root.join("case_b");
    if let Err(err) = write_minimal_clean_core_tree(&case_b) {
        panic!("setup case_b: {err}");
    }
    if let Err(err) = fs::write(
        case_b.join("engine.rs"),
        "pub struct TurnEngine;\nimpl TurnEngine {\n    pub const fn preview_bundle(&self) {}\n}\n",
    ) {
        panic!("write case_b: {err}");
    }
    assert!(scan_league_core_src_tree_with_surface(&case_b, &MINIMAL_TEST_SURFACE).is_err());

    // Control C: `#[cfg(feature = "engine-internals")] pub fn preview_bundle` on `TurnEngine` -> Err.
    let case_c = tmp_root.join("case_c");
    if let Err(err) = write_minimal_clean_core_tree(&case_c) {
        panic!("setup case_c: {err}");
    }
    if let Err(err) = fs::write(
        case_c.join("engine.rs"),
        "pub struct TurnEngine;\nimpl TurnEngine {\n    #[cfg(feature = \"engine-internals\")]\n    pub fn preview_bundle(&self) {}\n}\n",
    ) {
        panic!("write case_c: {err}");
    }
    assert!(scan_league_core_src_tree_with_surface(&case_c, &MINIMAL_TEST_SURFACE).is_err());

    // Control D: `pub mod engine_internals` missing `#[cfg]` or `#[doc(hidden)]` -> Err.
    let case_d = tmp_root.join("case_d");
    if let Err(err) = write_minimal_clean_core_tree(&case_d) {
        panic!("setup case_d: {err}");
    }
    if let Err(err) = fs::write(
        case_d.join("lib.rs"),
        "pub mod engine;\npub mod observation;\npub mod engine_internals {}\n",
    ) {
        panic!("write case_d: {err}");
    }
    assert!(scan_league_core_src_tree_with_surface(&case_d, &MINIMAL_TEST_SURFACE).is_err());

    // Control E: `pub use engine::sweep_mode::SweepMode;` at crate root -> Err.
    let case_e = tmp_root.join("case_e");
    if let Err(err) = write_minimal_clean_core_tree(&case_e) {
        panic!("setup case_e: {err}");
    }
    if let Err(err) = fs::write(
        case_e.join("lib.rs"),
        "pub mod engine;\npub mod observation;\npub use engine::sweep_mode::SweepMode;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
    ) {
        panic!("write case_e: {err}");
    }
    assert!(scan_league_core_src_tree_with_surface(&case_e, &MINIMAL_TEST_SURFACE).is_err());

    // Control F: `default = ["engine-internals"]` in `league-core/Cargo.toml` -> Err.
    let case_f = tmp_root.join("case_f");
    if let Err(err) = write_minimal_workspace_tree(&case_f) {
        panic!("setup case_f: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_f).is_ok());
    if let Err(err) = fs::write(
        case_f.join("crates/league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\ndefault = [\"engine-internals\"]\nengine-internals = []\n",
    ) {
        panic!("write case_f: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_f).is_err());

    // Control G: `league-tui/Cargo.toml` enables `engine-internals` or `league-relay` adds `league-core` -> Err.
    let case_g1 = tmp_root.join("case_g1");
    if let Err(err) = write_minimal_workspace_tree(&case_g1) {
        panic!("setup case_g1: {err}");
    }
    if let Err(err) = fs::write(
        case_g1.join("crates/league-tui/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\", features = [\"engine-internals\"] }\n",
    ) {
        panic!("write case_g1: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_g1).is_err());

    let case_g2 = tmp_root.join("case_g2");
    if let Err(err) = write_minimal_workspace_tree(&case_g2) {
        panic!("setup case_g2: {err}");
    }
    if let Err(err) = fs::write(
        case_g2.join("crates/league-relay/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\" }\n",
    ) {
        panic!("write case_g2: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_g2).is_err());

    // Control H: `league-seat-ui/src/lib.rs` imports `engine_internals` -> Err; comment near-miss -> Ok.
    let case_h = tmp_root.join("case_h");
    if let Err(err) = write_minimal_workspace_tree(&case_h) {
        panic!("setup case_h: {err}");
    }
    if let Err(err) = fs::write(
        case_h.join("crates/league-seat-ui/src/lib.rs"),
        "#![no_std]\n#![forbid(unsafe_code)]\n#![forbid(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]\n// Note: engine_internals and preview_bundle stay out of comments\npub fn ok() {}\n",
    ) {
        panic!("write case_h ok: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_h).is_ok());

    if let Err(err) = fs::write(
        case_h.join("crates/league-seat-ui/src/lib.rs"),
        "#![no_std]\n#![forbid(unsafe_code)]\n#![forbid(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]\nuse league_core::engine_internals;\n",
    ) {
        panic!("write case_h bad: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_h).is_err());

    let _ = fs::remove_dir_all(&tmp_root);
}

#[test]
fn planted_seat_control_1_names_turn_engine_fails() {
    let root = unique_tmp_dir("seat_turn_engine");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    if let Err(err) = fs::write(
        root.join("crates/league-seat-ui/src/ui.rs"),
        "pub fn render_bad(engine: &league_core::TurnEngine) {}\n",
    ) {
        panic!("write ui.rs: {err}");
    }
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("seat module naming TurnEngine must fail guard"));
    assert!(err.contains("TurnEngine"), "unexpected error: {err}");

    let core_dir = root.join("core_tree");
    if let Err(err) = write_minimal_clean_core_tree(&core_dir) {
        panic!("setup core_dir: {err}");
    }
    if let Err(err) = fs::write(
        core_dir.join("bot.rs"),
        "pub struct ClubBot;\nimpl ClubBot {\n    pub fn peek(&self, _e: &crate::TurnEngine) {}\n}\n",
    ) {
        panic!("write bot.rs: {err}");
    }
    let bot_err = scan_league_core_src_tree_with_surface(&core_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("bot.rs naming TurnEngine must fail guard"));
    assert!(
        bot_err.contains("TurnEngine"),
        "unexpected error: {bot_err}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_seat_control_2_names_engine_state_fails() {
    let root = unique_tmp_dir("seat_engine_state");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    let screens_dir = root.join("crates/league-seat-ui/src/screens");
    if let Err(err) = fs::create_dir_all(&screens_dir) {
        panic!("create screens_dir: {err}");
    }
    if let Err(err) = fs::write(
        screens_dir.join("board.rs"),
        "pub fn draw_board(state: &league_core::EngineState) {}\n",
    ) {
        panic!("write board.rs: {err}");
    }
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("seat module naming EngineState must fail guard"));
    assert!(err.contains("EngineState"), "unexpected error: {err}");

    let core_dir = root.join("core_tree");
    if let Err(err) = write_minimal_clean_core_tree(&core_dir) {
        panic!("setup core_dir: {err}");
    }
    if let Err(err) = fs::write(
        core_dir.join("bot.rs"),
        "pub fn helper(state: &crate::EngineState) {}\n",
    ) {
        panic!("write bot.rs: {err}");
    }
    let bot_err = scan_league_core_src_tree_with_surface(&core_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("bot.rs naming EngineState must fail guard"));
    assert!(
        bot_err.contains("EngineState"),
        "unexpected error: {bot_err}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_seat_control_3_names_engine_internals_fails() {
    let root = unique_tmp_dir("seat_engine_internals");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    if let Err(err) = fs::write(
        root.join("crates/league-seat-ui/src/render.rs"),
        "pub fn peek_hints() { let _ = league_core::engine_internals::SweepMode::Live; }\n",
    ) {
        panic!("write render.rs: {err}");
    }
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("seat module naming engine_internals must fail guard"));
    assert!(err.contains("engine_internals"), "unexpected error: {err}");

    let core_dir = root.join("core_tree");
    if let Err(err) = write_minimal_clean_core_tree(&core_dir) {
        panic!("setup core_dir: {err}");
    }
    if let Err(err) = fs::write(
        core_dir.join("bot.rs"),
        "pub fn peek() { let _ = crate::engine_internals::SweepMode::PreviewSteps0To7; }\n",
    ) {
        panic!("write bot.rs: {err}");
    }
    let bot_err = scan_league_core_src_tree_with_surface(&core_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("bot.rs naming engine_internals must fail guard"));
    assert!(
        bot_err.contains("engine_internals"),
        "unexpected error: {bot_err}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_seat_control_signature_rejects_raw_param_types_and_missing_obs_param() {
    let root = unique_tmp_dir("seat_sig");
    if let Err(err) = write_minimal_clean_core_tree(&root) {
        panic!("setup core_dir: {err}");
    }
    // Parameter taking `&TableSpec` violates the seat-side parameter rule even without naming TurnEngine.
    if let Err(err) = fs::write(
        root.join("bot.rs"),
        "pub struct ClubBot;\nimpl ClubBot {\n    pub fn decide(&self, _spec: &crate::TableSpec) {}\n}\n",
    ) {
        panic!("write bot.rs: {err}");
    }
    let err = scan_league_core_src_tree_with_surface(&root, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("seat fn accepting &TableSpec must fail guard"));
    assert!(err.contains("TableSpec"), "unexpected error: {err}");

    // `ClubBot` decision method without `&SeatObservation`, `PublicSweepReceipt`, or preview param fails.
    if let Err(err) = fs::write(
        root.join("bot.rs"),
        "pub struct ClubBot;\nimpl ClubBot {\n    pub fn decide_turn(&self, _seat: Seat) {}\n}\n",
    ) {
        panic!("write bot.rs: {err}");
    }
    let missing_err = scan_league_core_src_tree_with_surface(&root, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("ClubBot method missing SeatObservation param must fail guard"));
    assert!(
        missing_err.contains("SeatObservation"),
        "unexpected error: {missing_err}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_b3_alias_feature_and_default_transitively_enabling_engine_internals_fail() {
    let root = unique_tmp_dir("b3_alias");

    // Case 1 (M3): `seat-debug = ["engine-internals"]` in league-core + enabled in league-tui -> Err.
    let case_m3 = root.join("m3");
    if let Err(err) = write_minimal_workspace_tree(&case_m3) {
        panic!("setup m3: {err}");
    }
    if let Err(err) = fs::write(
        case_m3.join("crates/league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\nengine-internals = []\nseat-debug = [\"engine-internals\"]\n",
    ) {
        panic!("write m3 core toml: {err}");
    }
    if let Err(err) = fs::write(
        case_m3.join("crates/league-tui/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\", features = [\"seat-debug\"] }\n",
    ) {
        panic!("write m3 tui toml: {err}");
    }
    let err_m3 = check_workspace_manifests_and_crate_sources(&case_m3)
        .err()
        .unwrap_or_else(|| panic!("B3 alias feature must fail guard"));
    assert!(err_m3.contains("seat-debug"), "unexpected error: {err_m3}");

    // Case 2 (M4): `default = ["seat-debug"]` -> `seat-debug = ["engine-internals"]` -> Err.
    let case_m4 = root.join("m4");
    if let Err(err) = write_minimal_workspace_tree(&case_m4) {
        panic!("setup m4: {err}");
    }
    if let Err(err) = fs::write(
        case_m4.join("crates/league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\ndefault = [\n  \"seat-debug\",\n]\nseat-debug = [\"engine-internals\"]\nengine-internals = []\n",
    ) {
        panic!("write m4 core toml: {err}");
    }
    let err_m4 = check_workspace_manifests_and_crate_sources(&case_m4)
        .err()
        .unwrap_or_else(|| panic!("B3 transitive default alias must fail guard"));
    assert!(
        err_m4.contains("default") || err_m4.contains("seat-debug"),
        "unexpected error: {err_m4}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_b4_renamed_live_sweep_wrapper_fails() {
    let root = unique_tmp_dir("b4_live_wrapper");

    // Opus M9 reproducer: `pub fn forecast_peek` calling `preview_bundle_with_context_internal(..., SweepMode::Live)` -> Err.
    let m9_dir = root.join("m9");
    if let Err(err) = write_minimal_clean_core_tree(&m9_dir) {
        panic!("setup m9: {err}");
    }
    if let Err(err) = fs::write(
        m9_dir.join("engine.rs"),
        r#"
pub(crate) mod sweep_mode {
    pub enum SweepMode { Live, PreviewSteps0To7 }
}
pub struct TurnEngine;
impl TurnEngine {
    pub fn observe_seat(&self) {}
    pub fn forecast_peek(&self) {
        let _ = self.preview_bundle_with_context_internal(sweep_mode::SweepMode::Live);
    }
}
"#,
    ) {
        panic!("write m9 engine.rs: {err}");
    }
    let err_m9 = scan_league_core_src_tree_with_surface(&m9_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("M9 renamed live wrapper must fail guard"));
    assert!(
        err_m9.contains("SweepMode::Live") || err_m9.contains("forecast_peek"),
        "unexpected error: {err_m9}"
    );

    // Even on a brand-new module/struct outside TurnEngine, calling `SweepMode::Live` in a `pub fn` fails!
    let free_dir = root.join("free_live");
    if let Err(err) = write_minimal_clean_core_tree(&free_dir) {
        panic!("setup free_dir: {err}");
    }
    if let Err(err) = fs::write(
        free_dir.join("helper.rs"),
        "pub fn sneak_live() { let _ = crate::engine::sweep_mode::SweepMode::Live; }\n",
    ) {
        panic!("write helper.rs: {err}");
    }
    let err_free = scan_league_core_src_tree_with_surface(&free_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("free pub fn referencing SweepMode::Live must fail guard"));
    assert!(
        err_free.contains("SweepMode::Live"),
        "unexpected error: {err_free}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_b4_renamed_raw_state_wrapper_and_reexport_path_fail() {
    let root = unique_tmp_dir("b4_raw_state");

    // 1. Renamed `pub fn raw_snapshot(&self) -> &EngineState` on `ClubHost` / helper fails return-type rule.
    let state_dir = root.join("raw_ret");
    if let Err(err) = write_minimal_clean_core_tree(&state_dir) {
        panic!("setup raw_ret: {err}");
    }
    if let Err(err) = fs::write(
        state_dir.join("helper.rs"),
        "pub struct ClubHost;\nimpl ClubHost {\n    pub fn raw_snapshot(&self) -> &EngineState { loop {} }\n}\n",
    ) {
        panic!("write helper.rs: {err}");
    }
    let err_ret = scan_league_core_src_tree_with_surface(&state_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("wrapper returning EngineState must fail guard"));
    assert!(
        err_ret.contains("EngineState"),
        "unexpected error: {err_ret}"
    );

    // 2. Aliased `pub use crate::engine::sweep_mode::SweepMode as PublicMode;` fails canonical path expansion.
    let alias_dir = root.join("alias_reexport");
    if let Err(err) = write_minimal_clean_core_tree(&alias_dir) {
        panic!("setup alias_reexport: {err}");
    }
    if let Err(err) = fs::write(
        alias_dir.join("lib.rs"),
        "pub mod engine;\npub mod observation;\npub use crate::engine::sweep_mode::SweepMode as PublicMode;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
    ) {
        panic!("write lib.rs: {err}");
    }
    let err_alias = scan_league_core_src_tree_with_surface(&alias_dir, &MINIMAL_TEST_SURFACE)
        .err()
        .unwrap_or_else(|| panic!("aliased re-export of SweepMode must fail guard"));
    assert!(
        err_alias.contains("SweepMode"),
        "unexpected error: {err_alias}"
    );

    // 3. Unlisted `pub use` item path on exact production surface fails even when symbol is otherwise benign.
    let unlisted_dir = root.join("unlisted_reexport");
    if let Err(err) = write_minimal_clean_core_tree(&unlisted_dir) {
        panic!("setup unlisted_reexport: {err}");
    }
    let strict_surface = ExpectedCoreSurface {
        enforce_exact_surface: true,
        required_pub_mods: &["engine", "observation"],
        allowed_pub_mods: &["engine", "observation"],
        allowed_reexports: &["engine::TurnEngine"],
        allowed_turn_engine_pub_fns: &["observe_seat"],
        allowed_club_host_pub_fns: None,
        allowed_observation_pub_fns: &["preview_what_if", "preview_seat_bundle"],
    };
    if let Err(err) = fs::write(
        unlisted_dir.join("lib.rs"),
        "pub mod engine;\npub mod observation;\npub use engine::{TurnEngine, ExtraSecret as SafeAlias};\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
    ) {
        panic!("write lib.rs: {err}");
    }
    let err_unlisted = scan_league_core_src_tree_with_surface(&unlisted_dir, &strict_surface)
        .err()
        .unwrap_or_else(|| panic!("unlisted aliased re-export path must fail guard"));
    assert!(
        err_unlisted.contains("engine::ExtraSecret"),
        "unexpected error: {err_unlisted}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_c2_workspace_member_outside_crates_dir_fails_when_violating_guard() {
    let root = unique_tmp_dir("c2_members");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    // Add an extra workspace member outside `crates/` (`tools/extra-seat`).
    if let Err(err) = fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = [
    "crates/league-core",
    "crates/league-error",
    "crates/league-gen",
    "crates/league-ledger",
    "crates/league-relay",
    "crates/league-season",
    "crates/league-seat-types",
    "crates/league-seat-ui",
    "crates/league-standalone",
    "crates/league-tui",
    "tools/extra-seat",
]
"#,
    ) {
        panic!("write root Cargo.toml: {err}");
    }
    let extra_dir = root.join("tools/extra-seat");
    if let Err(err) = fs::create_dir_all(extra_dir.join("src")) {
        panic!("mkdir extra-seat: {err}");
    }
    if let Err(err) = fs::write(
        extra_dir.join("Cargo.toml"),
        "[package]\nname = \"extra-seat\"\n[dependencies]\nleague-core = { path = \"../../crates/league-core\" }\n",
    ) {
        panic!("write extra-seat Cargo.toml: {err}");
    }
    if let Err(err) = fs::write(extra_dir.join("src/lib.rs"), "pub fn clean() {}\n") {
        panic!("write extra-seat lib.rs: {err}");
    }
    let ok_members = check_workspace_manifests_and_crate_sources(&root)
        .unwrap_or_else(|err| panic!("clean extra-seat member should pass: {err}"));
    assert!(ok_members.contains(&"extra-seat".to_owned()));

    // Plant forbidden `engine_internals` usage in `tools/extra-seat/src/lib.rs` -> guard catches it!
    if let Err(err) = fs::write(
        extra_dir.join("src/lib.rs"),
        "pub fn bad() { let _ = league_core::engine_internals::SweepMode::Live; }\n",
    ) {
        panic!("write extra-seat bad lib.rs: {err}");
    }
    let bad_err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("extra-seat member outside crates/ must be scanned and fail"));
    assert!(
        bad_err.contains("engine_internals"),
        "unexpected error: {bad_err}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_leaf_error_control_1_engine_internals_in_manifest_fails() {
    let root = unique_tmp_dir("leaf_err_engine_internals");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    if let Err(err) = check_workspace_manifests_and_crate_sources(&root) {
        panic!("clean minimal workspace should pass guard: {err}");
    }
    if let Err(err) = fs::write(
        root.join("crates/league-error/Cargo.toml"),
        "[package]\nname = \"league-error\"\n[features]\nengine-internals = []\n",
    ) {
        panic!("write league-error Cargo.toml: {err}");
    }
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("engine-internals in league-error/Cargo.toml must fail guard"));
    assert!(
        err.contains("league-error/Cargo.toml") && err.contains("engine-internals"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_leaf_error_control_2_league_core_dep_in_manifest_fails() {
    let root = unique_tmp_dir("leaf_err_league_core_dep");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    if let Err(err) = check_workspace_manifests_and_crate_sources(&root) {
        panic!("clean minimal workspace should pass guard: {err}");
    }
    if let Err(err) = fs::write(
        root.join("crates/league-error/Cargo.toml"),
        "[package]\nname = \"league-error\"\n[dependencies]\nleague-core = { path = \"../league-core\" }\n",
    ) {
        panic!("write league-error Cargo.toml: {err}");
    }
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("league-core dep in league-error/Cargo.toml must fail guard"));
    assert!(
        err.contains("league-error/Cargo.toml") && err.contains("league-core"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_leaf_error_control_3_transitive_league_gen_dep_in_manifest_fails() {
    let root = unique_tmp_dir("leaf_err_league_gen_dep");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    if let Err(err) = check_workspace_manifests_and_crate_sources(&root) {
        panic!("clean minimal workspace should pass guard: {err}");
    }
    if let Err(err) = fs::write(
        root.join("crates/league-error/Cargo.toml"),
        "[package]\nname = \"league-error\"\n[dependencies]\nserde = \"1.0.229\"\nserde_json = \"1.0.154\"\nleague-gen = { path = \"../league-gen\" }\n",
    ) {
        panic!("write league-error Cargo.toml: {err}");
    }
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("league-gen dep in league-error/Cargo.toml must fail guard"));
    assert!(
        err.contains("league-error/Cargo.toml") && err.contains("league-gen"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_b2_relay_and_ledger_all_dep_routes_and_c4_root_key_fail() {
    let routes: [(&str, &str); 6] = [
        (
            "dev_dep",
            "[package]\nname = \"{crate}\"\n[dev-dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "build_dep",
            "[package]\nname = \"{crate}\"\n[build-dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "target_windows_dep",
            "[package]\nname = \"{crate}\"\n[target.'cfg(windows)'.dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "target_linux_dep",
            "[package]\nname = \"{crate}\"\n[target.'cfg(linux)'.dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "indirect_via_league_gen",
            "[package]\nname = \"{crate}\"\n[dependencies]\ngen_bridge = { package = \"league-gen\", path = \"../league-gen\" }\n",
        ),
        (
            "c4_root_dotted_key",
            "dependencies.league-core = { path = \"../league-core\" }\n[package]\nname = \"{crate}\"\n",
        ),
    ];

    for target_crate in ["league-relay", "league-ledger"] {
        for (label, template) in routes {
            let root = unique_tmp_dir(&format!("b2_{target_crate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            let manifest_path = root.join("crates").join(target_crate).join("Cargo.toml");
            let contents = template.replace("{crate}", target_crate);
            if let Err(err) = fs::write(&manifest_path, contents) {
                panic!("write {target_crate} manifest ({label}): {err}");
            }
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| {
                    panic!("{target_crate} planted dep route `{label}` must fail guard")
                });
            println!("PLANTED_DEP_ROUTE {target_crate}:{label} => {err}");
            assert!(
                err.contains(target_crate),
                "unexpected error for {target_crate}/{label}: {err}"
            );
            let _ = fs::remove_dir_all(&root);
        }
    }

    // Indirect route through a newly added workspace helper crate (`league-relay -> league-helper -> league-core`).
    let root = unique_tmp_dir("b2_indirect_new_crate");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    let helper_dir = root.join("crates/league-helper");
    if let Err(err) = fs::create_dir_all(helper_dir.join("src")) {
        panic!("create helper: {err}");
    }
    let _ = fs::write(helper_dir.join("src/lib.rs"), "pub fn help() {}\n");
    let _ = fs::write(
        helper_dir.join("Cargo.toml"),
        "[package]\nname = \"league-helper\"\n[dependencies]\nleague-core = { path = \"../league-core\" }\n",
    );
    let _ = fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\n    \"crates/league-core\",\n    \"crates/league-error\",\n    \"crates/league-gen\",\n    \"crates/league-helper\",\n    \"crates/league-ledger\",\n    \"crates/league-relay\",\n    \"crates/league-season\",\n    \"crates/league-seat-types\",\n    \"crates/league-seat-ui\",\n    \"crates/league-standalone\",\n    \"crates/league-tui\",\n]\n",
    );
    let _ = fs::write(
        root.join("crates/league-relay/Cargo.toml"),
        "[package]\nname = \"league-relay\"\n[dependencies]\nhelper = { package = \"league-helper\", path = \"../league-helper\" }\n",
    );
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("indirect dep via league-helper must fail guard"));
    println!("PLANTED_DEP_ROUTE league-relay:indirect_league_helper => {err}");
    assert!(
        err.contains("league-relay")
            && err.contains("league-helper")
            && err.contains("league-core"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&root);

    // PR #80 C4 root dotted key in `league-error/Cargo.toml`.
    let root = unique_tmp_dir("c4_league_error_root_key");
    if let Err(err) = write_minimal_workspace_tree(&root) {
        panic!("setup workspace: {err}");
    }
    let _ = fs::write(
        root.join("crates/league-error/Cargo.toml"),
        "dependencies.league-core = { path = \"../league-core\" }\n[package]\nname = \"league-error\"\n",
    );
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("root dotted key in league-error must fail guard"));
    println!("PLANTED_C4_ROOT_KEY league-error => {err}");
    assert!(
        err.contains("league-error/Cargo.toml"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_b1_tui_host_module_separation_and_4_club_host_routes_fail() {
    // Positive check: `league-tui` binary host (`src/host.rs` and `src/main.rs`) MAY touch
    // `ClubHost`, `ClubSaveSnapshot`, `mandate_for_seat`, and `write_save_and_seal` /
    // `read_save_and_seal` / `wipe_seal_file` while seat rendering lives cleanly in `league-seat-ui`.
    let clean_host_ws = unique_tmp_dir("tui_host_modules_ok");
    if let Err(err) = write_minimal_workspace_tree(&clean_host_ws) {
        panic!("setup workspace: {err}");
    }
    let tui_src = clean_host_ws.join("crates/league-tui/src");
    let seat_ui_src = clean_host_ws.join("crates/league-seat-ui/src");
    let _ = fs::write(
        tui_src.join("host.rs"),
        "use league_core::{ClubHost, ClubSaveSnapshot, Seat};\npub fn run_host(h: &mut ClubHost) { let _ = h.mandate_for_seat(Seat(0)); }\npub fn io_snap(s: &ClubSaveSnapshot) { let _ = s; }\n",
    );
    let _ = fs::write(
        tui_src.join("main.rs"),
        "use league_core::ClubHost;\nfn main() { let _ = std::mem::size_of::<ClubHost>(); }\n",
    );
    let _ = fs::write(
        seat_ui_src.join("app.rs"),
        "use league_seat_types::{PublicSweepReceipt, SeatObservation};\npub struct AppState { pub seat_idx: u8 }\nimpl AppState { pub fn render(&self, _obs: &SeatObservation, _r: &PublicSweepReceipt) {} }\n",
    );
    if let Err(err) = check_workspace_manifests_and_crate_sources(&clean_host_ws) {
        panic!("binary TUI host (host.rs, main.rs) + clean seat UI crate must pass: {err}");
    }

    // Even in `league-tui/src/host.rs`, raw engine types & raw seal byte helpers stay blocked.
    let _ = fs::write(
        tui_src.join("host.rs"),
        "pub fn bad_seal(h: &league_core::ClubHost) { let _ = h.to_save_and_seal_bytes(); }\n",
    );
    let host_seal_err = check_workspace_manifests_and_crate_sources(&clean_host_ws)
        .err()
        .unwrap_or_else(|| panic!("host.rs calling to_save_and_seal_bytes must fail guard"));
    println!("PLANTED_HOST_MOD_RAW_BYTES => {host_seal_err}");
    assert!(
        host_seal_err.contains("to_save_and_seal_bytes"),
        "{host_seal_err}"
    );
    let _ = fs::write(tui_src.join("host.rs"), "pub fn clean_host() {}\n");

    // Host `#[macro_export]` macro and `main.rs` `pub fn` and `src/lib.rs` in `league-tui` all fail.
    let _ = fs::write(
        tui_src.join("host.rs"),
        "#[macro_export]\nmacro_rules! rival { ($s:expr) => { $s }; }\n",
    );
    let macro_err = check_workspace_manifests_and_crate_sources(&clean_host_ws)
        .err()
        .unwrap_or_else(|| panic!("league-tui macro_export must fail guard"));
    println!("PLANTED_HOST_MOD_MACRO_EXPORT => {macro_err}");
    assert!(macro_err.contains("macro_export"), "{macro_err}");
    let _ = fs::write(tui_src.join("host.rs"), "pub fn clean_host() {}\n");

    let _ = fs::write(
        tui_src.join("main.rs"),
        "pub fn host_ref() -> &'static league_core::ClubHost { loop {} }\nfn main() {}\n",
    );
    let main_pub_err = check_workspace_manifests_and_crate_sources(&clean_host_ws)
        .err()
        .unwrap_or_else(|| panic!("league-tui/src/main.rs pub fn must fail guard"));
    println!("PLANTED_TUI_MAIN_PUB_FN => {main_pub_err}");
    assert!(main_pub_err.contains("src/main.rs"), "{main_pub_err}");
    let _ = fs::write(tui_src.join("main.rs"), "fn main() {}\n");

    let _ = fs::write(tui_src.join("lib.rs"), "pub fn sneak() {}\n");
    let lib_err = check_workspace_manifests_and_crate_sources(&clean_host_ws)
        .err()
        .unwrap_or_else(|| panic!("league-tui src/lib.rs must fail binary-only guard"));
    println!("PLANTED_TUI_BIN_ONLY_LIB_RS => {lib_err}");
    assert!(lib_err.contains("src/lib.rs"), "{lib_err}");
    let _ = fs::remove_dir_all(&clean_host_ws);

    // 4 planted `ClubHost` routes in seat crate `league-seat-ui`.
    let club_host_routes: [(&str, &str, &str); 4] = [
        (
            "route1_named_in_seat_module",
            "src/app.rs",
            "pub fn render_screen() { let _ = std::any::type_name::<league_core::ClubHost>(); }\n",
        ),
        (
            "route2_private_fn_peek",
            "src/ui/app.rs",
            "fn peek(h: &league_core::ClubHost) { let _ = h; }\n",
        ),
        (
            "route3_self_method_on_app_holding_host",
            "src/screens/board.rs",
            "pub struct App { host: league_core::ClubHost }\nimpl App { pub fn x(&self) {} }\n",
        ),
        (
            "route4_seat_struct_field_club_host",
            "src/widgets/card.rs",
            "pub struct SeatCard { host: league_core::ClubHost }\n",
        ),
    ];

    for (label, rel_file, source) in club_host_routes {
        let root = unique_tmp_dir(&format!("b1_seat_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let target = root.join("crates/league-seat-ui").join(rel_file);
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&target, source);
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("seat UI ClubHost route `{label}` must fail guard"));
        println!("PLANTED_CLUB_HOST_ROUTE {label} ({rel_file}) => {err}");
        assert!(
            err.contains("ClubHost"),
            "unexpected error for {label}: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // Two-file route 3: `App` defined in `src/state.rs` (holds `ClubHost`), `impl App` in `src/render.rs`.
    let split_root = unique_tmp_dir("b1_seat_split_self");
    if let Err(err) = write_minimal_workspace_tree(&split_root) {
        panic!("setup workspace: {err}");
    }
    let _ = fs::write(
        split_root.join("crates/league-seat-ui/src/state.rs"),
        "pub struct App { host: league_core::ClubHost }\n",
    );
    let _ = fs::write(
        split_root.join("crates/league-seat-ui/src/render.rs"),
        "use crate::state::App;\nimpl App { pub fn x(&self) {} }\n",
    );
    let split_err = check_workspace_manifests_and_crate_sources(&split_root)
        .err()
        .unwrap_or_else(|| panic!("two-file ClubHost struct in seat UI must fail guard"));
    println!("PLANTED_CLUB_HOST_ROUTE split_self_method => {split_err}");
    assert!(split_err.contains("ClubHost"), "{split_err}");
    let _ = fs::remove_dir_all(&split_root);
}

#[test]
fn planted_b1_rival_mandate_and_save_fork_routes_in_seat_code_fail() {
    let routes: [(&str, &str, &str); 5] = [
        (
            "rival_mandate_call",
            "src/ui.rs",
            "pub fn steal_rival() { let _ = mandate_for_seat; }\n",
        ),
        (
            "seal_peek_bytes",
            "src/screens/peek.rs",
            "pub fn peek_seal() { let _ = to_save_and_seal_bytes; }\n",
        ),
        (
            "fork_from_bytes",
            "src/screens/fork.rs",
            "pub fn fork_host() { let _ = from_save_and_seal_bytes; }\n",
        ),
        (
            "save_snapshot_in_seat",
            "src/views.rs",
            "pub fn read_snap(_s: &league_core::ClubSaveSnapshot) {}\n",
        ),
        (
            "read_save_and_seal_in_seat",
            "src/save_ui.rs",
            "pub fn load_file() { let _ = read_save_and_seal; }\n",
        ),
    ];

    for (label, rel_file, source) in routes {
        let root = unique_tmp_dir(&format!("b1_save_mandate_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let target = root.join("crates/league-seat-ui").join(rel_file);
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&target, source);
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("seat route `{label}` must fail guard"));
        println!("PLANTED_SAVE_MANDATE_ROUTE {label} ({rel_file}) => {err}");
        assert!(!err.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_b2_alias_trait_field_and_non_core_lib_reexport_controls_a1_to_a9_fail() {
    // A1, A1b on `crates/league-gen/src/lib.rs` (and non-aliased `pub use league_core::TurnEngine;`).
    for (label, snippet) in [
        (
            "A1_pub_use_as_Eng",
            "pub use league_core::TurnEngine as Eng;\n",
        ),
        (
            "A1_pub_use_TurnEngine",
            "pub use league_core::TurnEngine;\n",
        ),
        (
            "A1b_pub_type_Eng",
            "pub type Eng = league_core::TurnEngine;\n",
        ),
    ] {
        let root = unique_tmp_dir(&format!("b2_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(root.join("crates/league-gen/src/lib.rs"), snippet);
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("{label} must fail workspace guard"));
        println!("PLANTED_B2 {label} => {err}");
        assert!(
            err.contains("TurnEngine"),
            "unexpected error for {label}: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // A2, A3, A3b, A4, A5, A6, A7, A8, A9 inside `league-core/src`.
    let strict_surface = ExpectedCoreSurface {
        enforce_exact_surface: true,
        required_pub_mods: &["club", "engine", "observation"],
        allowed_pub_mods: &["club", "engine", "observation"],
        allowed_reexports: &[],
        allowed_turn_engine_pub_fns: &["observe_seat"],
        allowed_club_host_pub_fns: Some(&["from_standard_setup"]),
        allowed_observation_pub_fns: &["preview_what_if", "preview_seat_bundle"],
    };
    let setup_host_core = |dir: &Path| {
        if let Err(err) = write_minimal_clean_core_tree(dir) {
            panic!("setup core tree: {err}");
        }
        let _ = fs::write(
            dir.join("lib.rs"),
            "pub mod club;\npub mod engine;\npub mod observation;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
        );
        let _ = fs::create_dir_all(dir.join("club"));
        let _ = fs::write(
            dir.join("club/mod.rs"),
            "pub mod host;\npub use host::ClubHost;\n",
        );
        let _ = fs::write(
            dir.join("club/host.rs"),
            "use crate::TurnEngine;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\n",
        );
    };

    let clean_check = unique_tmp_dir("b2_clean_strict");
    setup_host_core(&clean_check);
    if let Err(err) = scan_league_core_src_tree_with_surface(&clean_check, &strict_surface) {
        panic!("clean strict test tree should pass: {err}");
    }
    let _ = fs::remove_dir_all(&clean_check);

    let core_controls: [(&str, &str, &str); 10] = [
        (
            "A2_pub_use_EngineState_as_PublicState",
            "lib.rs",
            "pub mod club;\npub mod engine;\npub mod observation;\npub use engine::EngineState as PublicState;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
        ),
        (
            "A3_pub_type_SeatView",
            "observation.rs",
            "pub type SeatView = crate::engine::EngineState;\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
        ),
        (
            "A3b_pub_type_HostView",
            "types.rs",
            "pub type HostView = crate::engine::TurnEngine;\n",
        ),
        (
            "A4_free_pub_fn_peek_state",
            "replay.rs",
            "pub fn peek_state(e: &crate::TurnEngine) -> &crate::EngineState { let _ = e; loop {} }\n",
        ),
        (
            "A5_renamed_pub_fn_forecast_peek",
            "engine.rs",
            "pub struct TurnEngine;\nimpl TurnEngine {\n    pub fn observe_seat(&self) {}\n    pub fn forecast_peek(&self) {}\n}\n",
        ),
        (
            "A6_new_pub_fn_engine_json_on_ClubHost",
            "club/host.rs",
            "use crate::TurnEngine;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost {\n    pub fn from_standard_setup() {}\n    pub fn engine_json(&self) {}\n}\n",
        ),
        (
            "A7_impl_Deref_for_ClubHost",
            "club/host.rs",
            "use crate::TurnEngine;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\nimpl std::ops::Deref for ClubHost {\n    type Target = TurnEngine;\n    fn deref(&self) -> &Self::Target { &self.engine }\n}\n",
        ),
        (
            "A8_impl_AsRef_for_ClubHost",
            "club/host.rs",
            "use crate::{EngineState, TurnEngine};\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\nimpl AsRef<EngineState> for ClubHost {\n    fn as_ref(&self) -> &EngineState { loop {} }\n}\n",
        ),
        (
            "A9_pub_engine_field_on_ClubHost",
            "club/host.rs",
            "use crate::TurnEngine;\npub struct ClubHost { pub engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\n",
        ),
        (
            "club_mod_reexport_to_save_bytes",
            "club/mod.rs",
            "pub mod host;\npub use host::{ClubHost, to_save_and_seal_bytes};\n",
        ),
    ];

    for (label, rel_file, content) in core_controls {
        let dir = unique_tmp_dir(&format!("b2_{label}"));
        setup_host_core(&dir);
        let _ = fs::write(dir.join(rel_file), content);
        let err = scan_league_core_src_tree_with_surface(&dir, &strict_surface)
            .err()
            .unwrap_or_else(|| panic!("{label} must fail guard"));
        println!("PLANTED_B2 {label} => {err}");
        assert!(!err.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn planted_c2_glob_workspace_member_gives_clear_unsupported_error() {
    let root = unique_tmp_dir("c2_glob_member");
    if let Err(err) = fs::create_dir_all(&root) {
        panic!("mkdir: {err}");
    }
    let _ = fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    );
    let err = check_workspace_manifests_and_crate_sources(&root)
        .err()
        .unwrap_or_else(|| panic!("glob workspace member must fail guard"));
    println!("PLANTED_C2_GLOB => {err}");
    assert!(
        err.contains("crates/*") && err.contains("glob") && err.contains("unsupported"),
        "unexpected error: {err}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn planted_c3_league_relay_and_future_ai_seat_crate_enforced_by_seat_rule() {
    // `league-relay` is already on the structural seat rule ahead of Card 7.
    let relay_ws = unique_tmp_dir("c3_relay_seat_rule");
    if let Err(err) = write_minimal_workspace_tree(&relay_ws) {
        panic!("setup workspace: {err}");
    }
    for (label, source) in [
        (
            "relay_names_ClubHost",
            "pub fn r(h: &ClubHost) { let _ = h; }\n",
        ),
        (
            "relay_names_TableSpec",
            "pub fn r(t: &TableSpec) { let _ = t; }\n",
        ),
        (
            "relay_disallowed_param",
            "fn helper(_x: SecretLeakStruct) {}\nstruct TurnEngine;\n",
        ),
    ] {
        let _ = fs::write(relay_ws.join("crates/league-relay/src/relay.rs"), source);
        let err = check_workspace_manifests_and_crate_sources(&relay_ws)
            .err()
            .unwrap_or_else(|| panic!("league-relay `{label}` must fail seat rule"));
        println!("PLANTED_C3 {label} => {err}");
        assert!(!err.is_empty());
    }
    let _ = fs::remove_dir_all(&relay_ws);

    // A future AI-seat crate added to workspace defaults to the seat rule.
    let seat_ws = unique_tmp_dir("c3_future_ai_seat_crate");
    if let Err(err) = write_minimal_workspace_tree(&seat_ws) {
        panic!("setup workspace: {err}");
    }
    let ai_seat_dir = seat_ws.join("crates/league-ai-seat");
    let _ = fs::create_dir_all(ai_seat_dir.join("src"));
    let _ = fs::write(
        ai_seat_dir.join("Cargo.toml"),
        "[package]\nname = \"league-ai-seat\"\n",
    );
    let _ = fs::write(
        seat_ws.join("Cargo.toml"),
        "[workspace]\nmembers = [\n    \"crates/league-ai-seat\",\n    \"crates/league-core\",\n    \"crates/league-error\",\n    \"crates/league-gen\",\n    \"crates/league-ledger\",\n    \"crates/league-relay\",\n    \"crates/league-season\",\n    \"crates/league-seat-types\",\n    \"crates/league-seat-ui\",\n    \"crates/league-standalone\",\n    \"crates/league-tui\",\n]\n",
    );
    let _ = fs::write(
        ai_seat_dir.join("src/lib.rs"),
        "pub fn decide(obs: &SeatObservation) -> &SeatObservation { obs }\n",
    );
    if let Err(err) = check_workspace_manifests_and_crate_sources(&seat_ws) {
        panic!("clean future AI seat crate should pass: {err}");
    }
    let _ = fs::write(
        ai_seat_dir.join("src/agent.rs"),
        "fn sneak(h: &league_core::ClubHost) { let _ = h; }\n",
    );
    let err = check_workspace_manifests_and_crate_sources(&seat_ws)
        .err()
        .unwrap_or_else(|| panic!("future AI seat crate referencing ClubHost must fail guard"));
    println!("PLANTED_C3 future_ai_seat_ClubHost => {err}");
    assert!(err.contains("ClubHost"), "unexpected error: {err}");
    let _ = fs::remove_dir_all(&seat_ws);
}

#[test]
fn planted_item4_seat_crates_forbidden_dep_edges_all_fail() {
    let base_deps = |seat_crate: &str| -> &'static str {
        if seat_crate == "league-seat-types" {
            "league-error = { path = \"../league-error\" }\n"
        } else {
            "league-seat-types = { path = \"../league-seat-types\" }\n"
        }
    };

    let dep_routes: [(&str, &str); 11] = [
        (
            "normal_league_core",
            "[dependencies]\n{base}league-core = { path = \"../league-core\" }\n",
        ),
        (
            "normal_league_gen",
            "[dependencies]\n{base}league-gen = { path = \"../league-gen\" }\n",
        ),
        (
            "normal_league_standalone",
            "[dependencies]\n{base}league-standalone = { path = \"../league-standalone\" }\n",
        ),
        (
            "dev_dep_league_core",
            "[dependencies]\n{base}[dev-dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "build_dep_league_core",
            "[dependencies]\n{base}[build-dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "target_windows_league_core",
            "[dependencies]\n{base}[target.'cfg(windows)'.dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "target_linux_league_core",
            "[dependencies]\n{base}[target.'cfg(linux)'.dependencies]\nleague-core = { path = \"../league-core\" }\n",
        ),
        (
            "optional_league_core",
            "[dependencies]\n{base}league-core = { path = \"../league-core\", optional = true }\n",
        ),
        (
            "optional_serde_flag",
            "[dependencies]\n{base}serde = { version = \"1.0.229\", optional = true }\n",
        ),
        (
            "renamed_package_league_core",
            "[dependencies]\n{base}lc = { package = \"league-core\", path = \"../league-core\" }\n",
        ),
        (
            "indirect_via_league_season",
            "[dependencies]\n{base}league-season = { path = \"../league-season\" }\n",
        ),
    ];

    for seat_crate in ["league-seat-types", "league-seat-ui"] {
        for (label, template) in dep_routes {
            let root = unique_tmp_dir(&format!("item4_{seat_crate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            if label == "indirect_via_league_season" {
                let _ = fs::write(
                    root.join("crates/league-season/Cargo.toml"),
                    "[package]\nname = \"league-season\"\n[dependencies]\nleague-core = { path = \"../league-core\" }\n",
                );
            }
            let manifest = format!(
                "[package]\nname = \"{seat_crate}\"\n[lib]\ndoctest = false\n{}",
                template.replace("{base}", base_deps(seat_crate))
            );
            let _ = fs::write(
                root.join("crates").join(seat_crate).join("Cargo.toml"),
                manifest,
            );
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| panic!("{seat_crate} `{label}` must fail guard"));
            println!("PLANTED_SEAT_DEP {seat_crate}:{label} => {err}");
            assert!(err.contains(seat_crate), "unexpected error: {err}");
            let _ = fs::remove_dir_all(&root);
        }
    }
}

#[test]
fn planted_item5_seat_ui_public_surface_bans_dyn_impl_fn_generics_and_box_dyn() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
        "use league_seat_types::{SeatObservation, SeatScreenView};\n",
    );

    // Lifetimes ARE allowed on both structs and public functions.
    let ok_root = unique_tmp_dir("item5_lifetimes_ok");
    if let Err(err) = write_minimal_workspace_tree(&ok_root) {
        panic!("setup workspace: {err}");
    }
    let _ = fs::write(
        ok_root.join("crates/league-seat-ui/src/lib.rs"),
        format!(
            "{hdr}pub struct SeatPanel<'a> {{ pub obs: &'a SeatObservation }}\npub fn render_panel<'a>(_p: SeatPanel<'a>) -> SeatScreenView {{ loop {{}} }}\n"
        ),
    );
    if let Err(err) = check_workspace_manifests_and_crate_sources(&ok_root) {
        panic!("lifetime-only public surface in league-seat-ui must pass: {err}");
    }
    let _ = fs::remove_dir_all(&ok_root);

    let bad_surfaces: [(&str, &str); 8] = [
        ("generic_struct", "pub struct App<T> { pub inner: T }\n"),
        ("generic_pub_fn", "pub fn render_generic<T>(_x: T) {}\n"),
        (
            "box_dyn_field",
            "pub struct DynApp { inner: Box<dyn core::fmt::Display> }\n",
        ),
        (
            "dyn_param",
            "pub fn render_dyn(_x: &dyn core::fmt::Display) {}\n",
        ),
        (
            "impl_trait_param",
            "pub fn render_impl(_x: impl core::fmt::Display) {}\n",
        ),
        (
            "closure_box_dyn_fn_field",
            "pub struct ClosureApp { f: Box<dyn Fn(u8) -> String> }\n",
        ),
        (
            "fn_mut_param",
            "pub fn render_fn_mut(_f: &mut dyn FnMut(u8)) {}\n",
        ),
        (
            "pub_trait_decl",
            "pub trait Peek { fn rival(&self, seat: u8) -> String; }\n",
        ),
    ];

    for (label, body) in bad_surfaces {
        let root = unique_tmp_dir(&format!("item5_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-ui/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("item5 `{label}` must fail league-seat-ui surface guard"));
        println!("PLANTED_ITEM5_SURFACE {label} => {err}");
        assert!(!err.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_item6_seat_crates_ban_compile_time_includes_env_build_rs_and_path_attr() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    type HostChannelPlant<'a> = (&'a str, Option<(&'a str, &'a str)>, &'a str);
    let plants: [HostChannelPlant<'_>; 8] = [
        ("include_macro", None, "include!(\"peek.inc\");\n"),
        (
            "include_str_macro",
            None,
            "pub const S: &str = include_str!(\"peek.inc\");\n",
        ),
        (
            "include_bytes_macro",
            None,
            "pub const B: &[u8] = include_bytes!(\"peek.inc\");\n",
        ),
        ("env_macro", None, "pub const E: &str = env!(\"PATH\");\n"),
        (
            "option_env_macro",
            None,
            "pub const OE: Option<&str> = option_env!(\"KEY\");\n",
        ),
        ("path_attr", None, "#[path = \"peek.inc\"]\nmod r_path;\n"),
        (
            "non_rs_file_in_src",
            Some(("src/peek.inc", "pub fn leak() {}\n")),
            "pub fn ok() {}\n",
        ),
        (
            "build_rs_script",
            Some(("build.rs", "fn main() {}\n")),
            "pub fn ok() {}\n",
        ),
    ];

    for seat_crate in ["league-seat-types", "league-seat-ui"] {
        for (label, extra_file, lib_body) in plants {
            let root = unique_tmp_dir(&format!("item6_{seat_crate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            let crate_dir = root.join("crates").join(seat_crate);
            let _ = fs::write(crate_dir.join("src/lib.rs"), format!("{hdr}{lib_body}"));
            if let Some((rel, content)) = extra_file {
                let _ = fs::write(crate_dir.join(rel), content);
            }
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| {
                    panic!("{seat_crate} `{label}` must fail include/env/build guard")
                });
            println!("PLANTED_ITEM6_HOST_ESCAPE {seat_crate}:{label} => {err}");
            assert!(err.contains(seat_crate), "unexpected error: {err}");
            let _ = fs::remove_dir_all(&root);
        }
    }
}

#[test]
fn planted_item7_a2_alias_routes_x3_x3b_x3c_y1_y2_y5_x5_x5b_and_c1_y6_all_fail() {
    let strict_surface = ExpectedCoreSurface {
        enforce_exact_surface: true,
        required_pub_mods: &["club", "engine", "observation"],
        allowed_pub_mods: &["club", "engine", "observation"],
        allowed_reexports: &[],
        allowed_turn_engine_pub_fns: &["observe_seat"],
        allowed_club_host_pub_fns: Some(&["from_standard_setup"]),
        allowed_observation_pub_fns: &["preview_what_if", "preview_seat_bundle"],
    };
    let setup_host_core = |dir: &Path| {
        if let Err(err) = write_minimal_clean_core_tree(dir) {
            panic!("setup core tree: {err}");
        }
        let _ = fs::write(
            dir.join("lib.rs"),
            "pub mod club;\npub mod engine;\npub mod observation;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
        );
        let _ = fs::create_dir_all(dir.join("club"));
        let _ = fs::write(
            dir.join("club/mod.rs"),
            "pub mod host;\npub use host::ClubHost;\n",
        );
        let _ = fs::write(
            dir.join("club/host.rs"),
            "use crate::TurnEngine;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\n",
        );
    };

    let core_a2_cases: [(&str, &str, &str, &str); 9] = [
        (
            "X3_private_use_alias_in_pub_type",
            "observation.rs",
            "use crate::engine::EngineState as Es;\npub type SeatView2 = Es;\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "EngineState",
        ),
        (
            "X3_chained_private_type_alias_in_pub_type",
            "observation.rs",
            "use crate::engine::EngineState as Es;\ntype Mid = Es;\npub type SeatView3 = Mid;\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "EngineState",
        ),
        (
            "X3c_assoc_type_in_non_host_trait_impl",
            "observation.rs",
            "pub trait Peek { type T; }\nimpl Peek for SeatObservation { type T = crate::engine::EngineState; }\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "EngineState",
        ),
        (
            "Y1_type_alias_Hh_for_ClubHost_impl_AsRef",
            "club/host.rs",
            "use crate::TurnEngine;\ntype Hh = ClubHost;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\nimpl AsRef<crate::EngineState> for Hh { fn as_ref(&self) -> &crate::EngineState { loop {} } }\n",
            "EngineState",
        ),
        (
            "Y2_private_alias_Es_in_AsRef_for_ClubHost",
            "club/host.rs",
            "use crate::TurnEngine;\nuse crate::engine::EngineState as Es;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\nimpl AsRef<Es> for ClubHost { fn as_ref(&self) -> &Es { loop {} } }\n",
            "EngineState",
        ),
        (
            "Y5_private_alias_Es2_in_Deref_Target",
            "club/host.rs",
            "use crate::TurnEngine;\nuse crate::engine::TurnEngine as Es2;\npub struct ClubHost { engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\nimpl std::ops::Deref for ClubHost { type Target = Es2; fn deref(&self) -> &Self::Target { &self.engine } }\n",
            "TurnEngine",
        ),
        (
            "X5_pub_tuple_struct_raw_field",
            "observation.rs",
            "pub struct Wrap(pub crate::engine::TurnEngine);\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "TurnEngine",
        ),
        (
            "X5_pub_named_struct_aliased_raw_field",
            "observation.rs",
            "use crate::engine::EngineState as Es;\npub struct Wrap2 { pub e: Es }\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "EngineState",
        ),
        (
            "C1_Y6_pub_crate_engine_field_message",
            "club/host.rs",
            "use crate::TurnEngine;\npub struct ClubHost { pub(crate) engine: TurnEngine }\nimpl ClubHost { pub fn from_standard_setup() {} }\n",
            "field `engine`",
        ),
    ];

    for (label, rel_file, content, expected_substr) in core_a2_cases {
        let dir = unique_tmp_dir(&format!("item7_{label}"));
        setup_host_core(&dir);
        let _ = fs::write(dir.join(rel_file), content);
        let err = scan_league_core_src_tree_with_surface(&dir, &strict_surface)
            .err()
            .unwrap_or_else(|| panic!("item7 `{label}` must fail core guard"));
        println!("PLANTED_ITEM7_A2 {label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // X3b & X5b in `league-gen` (workspace crate where `league-core` IS visible).
    for (label, snippet, expected_substr) in [
        (
            "X3b_league_gen_private_use_alias_in_pub_type",
            "use league_core::TurnEngine as Te;\npub type Eng2 = Te;\n",
            "TurnEngine",
        ),
        (
            "X5b_league_gen_pub_struct_raw_pub_field",
            "use league_core::EngineState as Es;\npub struct GenWrap { pub e: Es }\n",
            "EngineState",
        ),
    ] {
        let root = unique_tmp_dir(&format!("item7_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(root.join("crates/league-gen/src/lib.rs"), snippet);
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("item7 `{label}` must fail workspace guard"));
        println!("PLANTED_ITEM7_A2 {label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_fix2_item1_seat_ui_bans_type_use_as_and_seat_types_checks_type_rhs_and_c3_fn_ptrs() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    let seat_ui_cases: [(&str, &str, &str); 11] = [
        (
            "Cb2_private_type_fn_ptr",
            "type Cb2 = fn(u8) -> u8;\npub fn render_alias2(_f: Cb2) {}\n",
            "type",
        ),
        (
            "D_private_type_box_dyn_fn",
            "type D = Box<dyn Fn(u8) -> u8>;\npub fn render_d(_f: D) {}\n",
            "type",
        ),
        (
            "DR_private_type_ref_dyn_fn",
            "type DR<'a> = &'a dyn Fn(u8) -> u8;\npub fn render_dr(_f: DR<'_>) {}\n",
            "type",
        ),
        (
            "Sh_private_type_arc_refcell",
            "type Sh = alloc::sync::Arc<core::cell::RefCell<u8>>;\npub fn render_sh(_s: &Sh) {}\n",
            "shared-state",
        ),
        (
            "use_as_rename_Rc_Clear",
            "use alloc::rc::Rc as Clear;\npub fn render_clear(_s: &Clear<u8>) {}\n",
            "Rc",
        ),
        (
            "use_as_rename_seat_types_Clear",
            "use league_seat_types::SeatObservation as Clear;\npub fn render_clear(_s: &Clear) {}\n",
            "use ... as",
        ),
        (
            "e2e_zz_render_peek",
            "type PeekFn = Box<dyn Fn(u8) -> u8>;\npub fn zz_render_peek(_f: PeekFn, _r: u8) -> u8 { 0 }\n",
            "type",
        ),
        (
            "e2e_zz_render_ptr",
            "type PeekPtr = fn(u8) -> u8;\npub fn zz_render_ptr(_f: PeekPtr, _r: u8) -> u8 { 0 }\n",
            "type",
        ),
        (
            "C3_extern_C_fn_ptr",
            "pub fn ext_cb(_f: extern \"C\" fn(u8) -> u8) {}\n",
            "function pointer",
        ),
        (
            "C3_unsafe_fn_ptr",
            "pub fn unsafe_cb(_f: unsafe fn(u8) -> u8) {}\n",
            "function pointer",
        ),
        (
            "raw_ptr_param",
            "pub fn raw_ptr(_p: *const u8) {}\n",
            "raw pointer",
        ),
    ];

    for (label, body, expected_substr) in seat_ui_cases {
        let root = unique_tmp_dir(&format!("fix2_ui_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-ui/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("fix2 item1 `{label}` must fail seat-ui guard"));
        println!("PLANTED_FIX2_ITEM1 seat-ui:{label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // Positive check in `league-seat-types`: concrete `type` RHS is allowed.
    let ok_root = unique_tmp_dir("fix2_types_ok_alias");
    if let Err(err) = write_minimal_workspace_tree(&ok_root) {
        panic!("setup workspace: {err}");
    }
    let _ = fs::write(
        ok_root.join("crates/league-seat-types/src/lib.rs"),
        format!(
            "{hdr}extern crate alloc;\nuse alloc::vec::Vec;\npub struct SeatVal {{ pub x: u8 }}\ntype PrivAlias = Vec<SeatVal>;\npub type PubAlias = PrivAlias;\npub fn ok(_v: &PubAlias) {{}}\n"
        ),
    );
    if let Err(err) = check_workspace_manifests_and_crate_sources(&ok_root) {
        panic!("valid concrete type alias in league-seat-types must pass: {err}");
    }
    let _ = fs::remove_dir_all(&ok_root);

    let seat_types_cases: [(&str, &str, &str); 5] = [
        (
            "ZzRaw_pub_type_box_dyn_fn",
            "pub type ZzRaw = Box<dyn Fn(u8) -> String>;\n",
            "Box",
        ),
        (
            "private_type_fn_ptr_rhs",
            "type PrivFn = fn(u8) -> String;\n",
            "function pointer",
        ),
        (
            "chained_private_type_dyn_rhs",
            "type Mid<'a> = &'a dyn Fn(u8) -> u8;\n",
            "dyn",
        ),
        (
            "ZzHook_pub_struct_box_dyn",
            "pub struct ZzHook(pub Box<dyn Fn(u8) -> String>);\n",
            "Box",
        ),
        (
            "use_as_rename_in_seat_types",
            "use alloc::collections::BTreeMap as Map;\n",
            "use ... as",
        ),
    ];

    for (label, body, expected_substr) in seat_types_cases {
        let root = unique_tmp_dir(&format!("fix2_types_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-types/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("fix2 item1 `{label}` must fail seat-types guard"));
        println!("PLANTED_FIX2_ITEM1 seat-types:{label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_fix2_item2_forbid_overrides_and_cfg_attr_path_controls() {
    let forbid_hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );
    let deny_hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![deny(clippy::disallowed_methods)]\n",
        "#![deny(clippy::disallowed_types)]\n",
        "#![deny(clippy::disallowed_macros)]\n",
        "pub fn ok() {}\n",
    );

    let cases: [(&str, bool, &str, &str); 5] = [
        ("deny_instead_of_forbid", false, deny_hdr, "#![forbid("),
        (
            "allow_clippy_style",
            true,
            "#[allow(clippy::style)]\npub fn bad() {}\n",
            "suppress or downgrade",
        ),
        (
            "expect_disallowed_methods",
            true,
            "#[expect(clippy::disallowed_methods)]\npub fn bad() {}\n",
            "suppress or downgrade",
        ),
        (
            "spaced_allow_disallowed_methods",
            true,
            "#[allow( clippy::disallowed_methods )]\npub fn bad() {}\n",
            "suppress or downgrade",
        ),
        (
            "cfg_attr_path_escape",
            true,
            "#[cfg_attr(all(), path = \"peek.inc\")]\nmod leak;\n",
            "cfg_attr(..., path = ...)",
        ),
    ];

    for seat_crate in ["league-seat-types", "league-seat-ui"] {
        for (label, use_forbid, body, expected_substr) in cases {
            let root = unique_tmp_dir(&format!("fix2_io_{seat_crate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            let content = if use_forbid {
                format!("{forbid_hdr}{body}")
            } else {
                body.to_owned()
            };
            let _ = fs::write(
                root.join("crates").join(seat_crate).join("src/lib.rs"),
                content,
            );
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| panic!("{seat_crate} `{label}` must fail Item 2 guard"));
            println!("PLANTED_FIX2_ITEM2 {seat_crate}:{label} => {err}");
            assert!(
                err.contains(expected_substr),
                "expected `{expected_substr}` in `{err}` for {seat_crate}:{label}"
            );
            let _ = fs::remove_dir_all(&root);
        }
    }
}

#[test]
fn planted_fix2_item3_bans_static_nostd_shared_state_and_partb_str_entry() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    // Positive check: `&'static str` constant + private string buffer + local UI state struct +
    // `SeatObservation` concrete `pub fn` entry point in `league-seat-ui` must PASS.
    let ok_root = unique_tmp_dir("fix2_partb_ok");
    if let Err(err) = write_minimal_workspace_tree(&ok_root) {
        panic!("setup workspace: {err}");
    }
    let _ = fs::write(
        ok_root.join("crates/league-seat-ui/src/lib.rs"),
        format!(
            "{hdr}extern crate alloc;\nuse alloc::string::String;\nuse league_seat_types::{{SeatObservation, SeatScreenView}};\npub const TITLE_LINES: [&str; 1] = [\"Merchant League\"];\npub struct UiState {{ filter_buf: String, pub selected: usize }}\nfn format_priv(_s: &str) -> String {{ String::new() }}\npub fn render_ok(_obs: &SeatObservation, _ui: &UiState) -> SeatScreenView {{ loop {{}} }}\n"
        ),
    );
    if let Err(err) = check_workspace_manifests_and_crate_sources(&ok_root) {
        panic!("valid Part B UI state + `&'static str` const must pass: {err}");
    }
    let _ = fs::remove_dir_all(&ok_root);

    let channel_plants: [(&str, &str); 8] = [
        ("pub_static_slot", "pub static ZZ_SLOT: u8 = 0;\n"),
        ("private_static", "static PRIV_VAL: u8 = 0;\n"),
        (
            "core_refcell_field",
            "pub struct BadRef { inner: core::cell::RefCell<u8> }\n",
        ),
        (
            "core_cell_field",
            "pub struct BadCell { inner: core::cell::Cell<u8> }\n",
        ),
        (
            "core_oncecell_field",
            "pub struct BadOnce { inner: core::cell::OnceCell<u8> }\n",
        ),
        (
            "core_atomic_val",
            "pub struct BadAtomic { inner: core::sync::atomic::AtomicU64 }\n",
        ),
        (
            "alloc_arc_field",
            "pub struct BadArc { inner: alloc::sync::Arc<u8> }\n",
        ),
        (
            "alloc_rc_field",
            "pub struct BadRc { inner: alloc::rc::Rc<u8> }\n",
        ),
    ];

    for seat_crate in ["league-seat-types", "league-seat-ui"] {
        for (label, body) in channel_plants {
            let root = unique_tmp_dir(&format!("fix2_b1_{seat_crate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            let _ = fs::write(
                root.join("crates").join(seat_crate).join("src/lib.rs"),
                format!("{hdr}{body}"),
            );
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| panic!("{seat_crate} `{label}` must fail B1 channel guard"));
            println!("PLANTED_FIX2_ITEM3 {seat_crate}:{label} => {err}");
            assert!(!err.is_empty());
            let _ = fs::remove_dir_all(&root);
        }
    }

    // Part B entry-point rule on `league-seat-ui`: reject `&str`, `String`, and `pub` `String` field.
    for (label, body) in [
        ("pub_fn_ref_str", "pub fn render_text(_s: &str) {}\n"),
        (
            "pub_fn_owned_string",
            "pub fn render_owned(_s: String) {}\n",
        ),
        (
            "pub_struct_pub_string_field",
            "pub struct BadUiInput { pub raw_text: String }\n",
        ),
    ] {
        let root = unique_tmp_dir(&format!("fix2_partb_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-ui/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("seat-ui `{label}` must fail Part B rule"));
        println!("PLANTED_FIX2_ITEM3_PARTB {label} => {err}");
        assert!(err.contains("Part B"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_fix2_items4_5_honest_guard_routes_and_host_mandate_isolation() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    // Item 4 (`B2`): honest non-`league_core` plants for generic, `dyn`, and closure routes.
    for (label, body, expected_substr) in [
        (
            "generic_without_league_core",
            "pub fn render_generic<T: core::fmt::Debug>(_x: &T) {}\n",
            "must not declare generic",
        ),
        (
            "dyn_without_league_core",
            "pub fn render_dyn(_p: &dyn core::fmt::Display) {}\n",
            "forbidden non-concrete/trait/closure type `dyn`",
        ),
        (
            "closure_without_league_core",
            "pub fn render_closure(_f: impl Fn(u8) -> u8) {}\n",
            "forbidden non-concrete/trait/closure type `impl`",
        ),
    ] {
        let root = unique_tmp_dir(&format!("fix2_b2_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-ui/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("item4 `{label}` must fail guard"));
        println!("PLANTED_FIX2_ITEM4_B2 {label} => {err}");
        assert!(err.contains(expected_substr), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    // Item 5 (`C2`): `from_host_parts`, `with_own_mandate`, and `observe_seat_with_mandate`
    // refused outside their single allowed host construction path.
    for (crate_name, rel_file, body) in [
        (
            "league-seat-ui",
            "src/lib.rs",
            format!(
                "{hdr}pub fn forge() {{ let _ = league_seat_types::SeatObservation::from_host_parts; }}\n"
            ),
        ),
        (
            "league-seat-types",
            "src/lib.rs",
            format!(
                "{hdr}pub fn forge() {{ let _ = observation::SeatObservation::from_host_parts; }}\n"
            ),
        ),
        (
            "league-tui",
            "src/host.rs",
            "pub fn forge() { let _ = league_seat_types::SeatObservation::from_host_parts; }\n"
                .to_owned(),
        ),
        (
            "league-gen",
            "src/lib.rs",
            "pub fn forge() { let _ = league_seat_types::SeatObservation::from_host_parts; }\n"
                .to_owned(),
        ),
    ] {
        let root = unique_tmp_dir(&format!("fix2_c2_{crate_name}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(root.join("crates").join(crate_name).join(rel_file), body);
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("item5 C2 in `{crate_name}/{rel_file}` must fail guard"));
        println!("PLANTED_FIX2_ITEM5_C2 {crate_name}/{rel_file} => {err}");
        assert!(err.contains("from_host_parts"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    let core_dir = unique_tmp_dir("fix2_c2_core");
    if let Err(err) = write_minimal_clean_core_tree(&core_dir) {
        panic!("setup core tree: {err}");
    }
    let _ = fs::write(
        core_dir.join("lib.rs"),
        "pub mod bot;\npub mod engine;\npub mod observation;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
    );
    let _ = fs::write(
        core_dir.join("bot.rs"),
        "pub fn stomp(e: &crate::TurnEngine) { let _ = e.observe_seat_with_mandate; }\n",
    );
    let core_err = scan_league_core_src_tree_with_surface(
        &core_dir,
        &ExpectedCoreSurface {
            enforce_exact_surface: true,
            required_pub_mods: &["bot", "engine", "observation"],
            allowed_pub_mods: &["bot", "engine", "observation"],
            allowed_reexports: &[],
            allowed_turn_engine_pub_fns: &["observe_seat"],
            allowed_club_host_pub_fns: None,
            allowed_observation_pub_fns: &["preview_what_if", "preview_seat_bundle"],
        },
    )
    .err()
    .unwrap_or_else(|| panic!("observe_seat_with_mandate in bot.rs must fail core guard"));
    println!("PLANTED_FIX2_ITEM5_C2 league-core/bot.rs => {core_err}");
    assert!(core_err.contains("observe_seat_with_mandate"), "{core_err}");
    let _ = fs::remove_dir_all(&core_dir);
}

#[test]
fn planted_fix3_item1_cfg_escapes_and_core_v_new_all_fail() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    // Seat-crate `#[cfg(...)]` and `#[cfg_attr(...)]` controls (prevents host/target divergence).
    let seat_cfg_cases: [(&str, &str, &str, &str); 14] = [
        (
            "cfg_not_test_on_seat_ui_pub_fn",
            "league-seat-ui",
            "#[cfg(not(test))]\npub fn zz_not_test_ok() {}\n",
            "#[cfg(...)]",
        ),
        (
            "cfg_not_target_os_none_std_escape",
            "league-seat-ui",
            "#[cfg(not(target_os = \"none\"))]\nextern crate std;\n",
            "#[cfg(...)]",
        ),
        (
            "N1_cfg_not_test_type_and_ptr_fn",
            "league-seat-ui",
            "#[cfg(not(test))]\ntype PeekPtr = fn(u8) -> String;\n#[cfg(not(test))]\npub fn zz_render_ptr(_f: PeekPtr, _r: u8) -> String { String::new() }\n",
            "#[cfg(...)]",
        ),
        (
            "N1b_cfg_not_test_zz_render_peek",
            "league-seat-ui",
            "#[cfg(not(test))]\npub fn zz_render_peek(_f: Box<dyn Fn(u8) -> String>, _r: u8) -> String { String::new() }\n",
            "#[cfg(...)]",
        ),
        (
            "N1c_cfg_not_test_partb_str",
            "league-seat-ui",
            "#[cfg(not(test))]\npub fn zz_render_text(_s: &str) -> usize { 0 }\n",
            "#[cfg(...)]",
        ),
        (
            "N1d_cfg_any_test_unix",
            "league-seat-ui",
            "#[cfg(any(test, unix))]\npub fn zz_any_test() {}\n",
            "#[cfg(...)]",
        ),
        (
            "cfg_all_not_test",
            "league-seat-ui",
            "#[cfg(all(not(test)))]\npub fn zz_all_not_test() {}\n",
            "#[cfg(...)]",
        ),
        (
            "N1e_cfg_not_test_pub_mod",
            "league-seat-ui",
            "#[cfg(not(test))]\npub mod zz { pub fn p() {} }\n",
            "#[cfg(...)]",
        ),
        (
            "N2b_seat_types_cfg_not_test_ZzRaw",
            "league-seat-types",
            "#[cfg(not(test))]\npub type ZzRaw = Box<dyn Fn(u8) -> String>;\n",
            "#[cfg(...)]",
        ),
        (
            "cfg_test_stray_on_fn_not_mod_tests",
            "league-seat-ui",
            "#[cfg(test)]\npub fn stray_cfg_test() {}\n",
            "allowed only directly on",
        ),
        (
            "cfg_test_second_mod_tests_in_file",
            "league-seat-ui",
            "#[cfg(test)]\nmod tests {}\n#[cfg(test)]\nmod tests {}\n",
            "at most one `#[cfg(test)] mod tests`",
        ),
        (
            "N5b_cfg_not_clippy_std_fs_read",
            "league-seat-ui",
            "#[cfg(not(clippy))]\npub fn zz_read() -> Vec<u8> { std::fs::read(\"zz.seal\").unwrap_or_default() }\n",
            "#[cfg(...)]",
        ),
        (
            "cfg_not_clippy_on_harmless_fn",
            "league-seat-ui",
            "#[cfg(not(clippy))]\npub fn zz_unlinted() {}\n",
            "#[cfg(...)]",
        ),
        (
            "cfg_attr_all_must_use",
            "league-seat-ui",
            "#[cfg_attr(all(), must_use)]\npub fn zz_cfg_attr() {}\n",
            "#[cfg_attr(...)]",
        ),
    ];

    for (label, crate_name, body, expected_substr) in seat_cfg_cases {
        let root = unique_tmp_dir(&format!("fix3_cfg_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates").join(crate_name).join("src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("fix3 `{label}` must fail guard"));
        println!("PLANTED_FIX3_CFG {crate_name}:{label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // Also test non-`#[cfg(...)]` `parse_tok_attr` `V-new` bypass in `league-core`:
    // `#[cfg(not(test))] use ... as Es8; pub type SeatView8 = Es8;` and
    // `#[cfg(not(test))] pub struct Wrap9(pub crate::engine::TurnEngine);` MUST now fail.
    let vnew_cases: [(&str, &str, &str); 2] = [
        (
            "V_new_core_cfg_not_test_SeatView8",
            "#[cfg(not(test))]\nuse crate::engine::EngineState as Es8;\n#[cfg(not(test))]\npub type SeatView8 = Es8;\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "EngineState",
        ),
        (
            "V_new_core_cfg_not_test_Wrap9",
            "#[cfg(not(test))]\npub struct Wrap9(pub crate::engine::TurnEngine);\npub fn preview_what_if() {}\npub fn preview_seat_bundle() {}\n",
            "TurnEngine",
        ),
    ];
    let strict_surface = ExpectedCoreSurface {
        enforce_exact_surface: true,
        required_pub_mods: &["engine", "observation"],
        allowed_pub_mods: &["engine", "observation"],
        allowed_reexports: &[],
        allowed_turn_engine_pub_fns: &["observe_seat"],
        allowed_club_host_pub_fns: None,
        allowed_observation_pub_fns: &["preview_what_if", "preview_seat_bundle"],
    };
    for (label, content, expected_substr) in vnew_cases {
        let core_dir = unique_tmp_dir(&format!("fix3_{label}"));
        if let Err(err) = write_minimal_clean_core_tree(&core_dir) {
            panic!("setup core tree: {err}");
        }
        let _ = fs::write(core_dir.join("observation.rs"), content);
        let err = scan_league_core_src_tree_with_surface(&core_dir, &strict_surface)
            .err()
            .unwrap_or_else(|| panic!("fix3 core `{label}` must fail guard"));
        println!("PLANTED_FIX3_V_NEW league-core:{label} => {err}");
        assert!(err.contains(expected_substr), "{err}");
        let _ = fs::remove_dir_all(&core_dir);
    }
}

#[test]
fn planted_fix3_item1_macros_inner_attrs_extern_unsafe_and_n5_cfg_all_fail() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    let cases: [(&str, &str, &str, &str); 9] = [
        (
            "M1_macro_rules_pub_fn_ptr",
            "league-seat-ui",
            "macro_rules! zz_mk { ($n:ident, $($t:tt)*) => { pub fn $n(_f: $($t)*, _rival: u8) -> u8 { 0 } } }\nzz_mk!(zz_render_ptr, fn(u8) -> u8);\n",
            "`macro_rules` is forbidden",
        ),
        (
            "M4_macro_rules_pub_fn_str",
            "league-seat-ui",
            "macro_rules! zz_mk { ($n:ident) => { pub fn $n(_s: &str) {} } }\nzz_mk!(zz_render_text);\n",
            "`macro_rules` is forbidden",
        ),
        (
            "macros_2_0_item_decl",
            "league-seat-ui",
            "pub macro zz_m2() {}\n",
            "macro` definition",
        ),
        (
            "offlist_inner_attribute_allow",
            "league-seat-ui",
            "#![allow(dead_code)]\npub fn f() {}\n",
            "inner attribute is not on the seat-crate allow-list",
        ),
        (
            "N5_e2e_cfg_not_clippy_macro_spliced_seal_read",
            "league-seat-ui",
            "#[cfg(not(clippy))]\nmacro_rules! zz_io { ($m:ident, $f:ident) => { std::$m::$f } }\n#[cfg(not(clippy))]\npub fn zread() -> Vec<u8> { zz_io!(fs, read)(\"zz.seal\").unwrap_or_default() }\n",
            "#[cfg(...)]",
        ),
        (
            "extern_crate_std",
            "league-seat-ui",
            "extern crate std;\npub fn f() {}\n",
            "`extern` crate/FFI block",
        ),
        (
            "extern_C_block",
            "league-seat-types",
            "extern \"C\" { fn host_peek(); }\n",
            "`extern` crate/FFI block",
        ),
        (
            "no_mangle_attr",
            "league-seat-ui",
            "#[no_mangle]\npub fn zz_sym() {}\n",
            "#[no_mangle]",
        ),
        (
            "unsafe_block",
            "league-seat-ui",
            "pub fn f() { unsafe {} }\n",
            "`unsafe` is forbidden",
        ),
    ];

    for (label, crate_name, body, expected_substr) in cases {
        let root = unique_tmp_dir(&format!("fix3_retain_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates").join(crate_name).join("src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("fix3 retained rule `{label}` must fail guard"));
        println!("PLANTED_FIX3_RETAINED {crate_name}:{label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_fix3_items4_5_b1_host_only_seat_observation_and_b2_entry_text_carriers_fail() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    // Item 4 (`B1`): restoring `Deserialize` on `SeatObservation` or calling
    // `unmandated`/`serde_json`/`from_str` in `league-tui` fails the guard.
    let de_root = unique_tmp_dir("fix3_b1_obs_deserialize");
    if let Err(err) = write_minimal_workspace_tree(&de_root) {
        panic!("setup workspace: {err}");
    }
    let _ = fs::write(
        de_root.join("crates/league-seat-types/src/lib.rs"),
        format!(
            "{hdr}use serde::Serialize;\n#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct SeatObservation {{ pub seat: u8 }}\n"
        ),
    );
    let de_err = check_workspace_manifests_and_crate_sources(&de_root)
        .err()
        .unwrap_or_else(|| panic!("SeatObservation with Deserialize must fail guard"));
    println!("PLANTED_FIX3_B1 seat-types:SeatObservation_Deserialize => {de_err}");
    assert!(de_err.contains("Deserialize"), "{de_err}");
    let _ = fs::remove_dir_all(&de_root);

    for (label, code, expected_substr) in [
        (
            "tui_calls_unmandated",
            "pub fn stamp() { let _ = league_seat_types::SeatObservation::unmandated; }\n",
            "unmandated",
        ),
        (
            "tui_names_public_parts",
            "pub fn stamp(_p: league_seat_types::SeatObservationPublicParts) {}\n",
            "SeatObservationPublicParts",
        ),
        (
            "tui_uses_serde_json_from_str",
            "pub fn stamp(s: &str) { let _ = serde_json::from_str::<u8>(s); }\n",
            "serde_json",
        ),
    ] {
        let tui_root = unique_tmp_dir(&format!("fix3_b1_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&tui_root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(tui_root.join("crates/league-tui/src/host.rs"), code);
        let err = check_workspace_manifests_and_crate_sources(&tui_root)
            .err()
            .unwrap_or_else(|| panic!("B1 `{label}` in league-tui must fail guard"));
        println!("PLANTED_FIX3_B1 league-tui:{label} => {err}");
        assert!(err.contains(expected_substr), "{err}");
        let _ = fs::remove_dir_all(&tui_root);
    }

    // Item 5 (`B2`): closed host text carriers in Part B entry allow-list (using `SeatKeyCode`/`SeatKeyEvent`).
    let b2_cases: [(&str, &str, &str); 9] = [
        (
            "B_a_zz_render_keys_Vec_SeatKeyCode",
            "use league_seat_types::SeatKeyCode;\npub fn zz_render_keys(_k: Vec<SeatKeyCode>) {}\n",
            "non-allow-listed type `Vec`",
        ),
        (
            "B_a_zz_render_key_slice",
            "use league_seat_types::SeatKeyCode;\npub fn zz_render_keys(_k: &[SeatKeyCode]) {}\n",
            "slice or array buffer",
        ),
        (
            "B_a_zz_render_key_tuple",
            "use league_seat_types::SeatKeyCode;\npub fn zz_render_keys(_k: (SeatKeyCode, SeatKeyCode)) {}\n",
            "tuple type",
        ),
        (
            "B_b_zz_render_err_StructuredError",
            "use league_seat_types::StructuredError;\npub fn zz_render_err(_e: &StructuredError) {}\n",
            "non-allow-listed type `StructuredError`",
        ),
        (
            "zz_render_eng_err_EngineError",
            "use league_seat_types::EngineError;\npub fn zz_render_eng_err(_e: &EngineError) {}\n",
            "non-allow-listed type `EngineError`",
        ),
        (
            "zz_render_char",
            "pub fn zz_render_char(_c: char) {}\n",
            "must not accept free `char`",
        ),
        (
            "zz_two_key_code_params",
            "use league_seat_types::SeatKeyCode;\npub fn zz_two_keys(_a: SeatKeyCode, _b: SeatKeyCode) {}\n",
            "at most one single",
        ),
        (
            "zz_two_key_event_params",
            "use league_seat_types::SeatKeyEvent;\npub fn zz_two_events(_a: SeatKeyEvent, _b: SeatKeyEvent) {}\n",
            "at most one single",
        ),
        (
            "BadUi_pub_Vec_SeatKeyCode_field",
            "use league_seat_types::SeatKeyCode;\npub struct BadUi { pub keys: Vec<SeatKeyCode> }\n",
            "non-allow-listed type `Vec`",
        ),
    ];

    for (label, body, expected_substr) in b2_cases {
        let root = unique_tmp_dir(&format!("fix3_b2_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-ui/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("fix3 B2 `{label}` must fail guard"));
        println!("PLANTED_FIX3_B2 {label} => {err}");
        assert!(
            err.contains(expected_substr),
            "expected `{expected_substr}` in `{err}` for {label}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_fix4_nostd_manifest_doctest_view_model_and_ratatui_ban_controls() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    // 1. Missing `#![no_std]` fails early warning in both seat crates.
    for seat_crate in ["league-seat-types", "league-seat-ui"] {
        let root = unique_tmp_dir(&format!("fix4_missing_nostd_{seat_crate}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates").join(seat_crate).join("src/lib.rs"),
            "#![forbid(unsafe_code)]\n#![forbid(\n    clippy::disallowed_methods,\n    clippy::disallowed_types,\n    clippy::disallowed_macros\n)]\npub fn ok() {}\n",
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("{seat_crate} without `#![no_std]` must fail guard"));
        println!("PLANTED_FIX4_NOSTD {seat_crate}:missing_no_std => {err}");
        assert!(err.contains("#![no_std]"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    // 2. Missing `[lib] doctest = false`, `doctest = true`, and extra `[lib]` keys fail in both seat crates.
    for seat_crate in ["league-seat-types", "league-seat-ui"] {
        let dep_line = if seat_crate == "league-seat-types" {
            "league-error = { path = \"../league-error\" }\n"
        } else {
            "league-seat-types = { path = \"../league-seat-types\" }\n"
        };
        for (label, lib_sec, expected_substr) in [
            ("missing_lib_doctest_false", "", "doctest = false"),
            ("doctest_true", "[lib]\ndoctest = true\n", "doctest = false"),
            (
                "extra_lib_key_proc_macro",
                "[lib]\ndoctest = false\nproc-macro = true\n",
                "only permits `doctest = false`",
            ),
            ("package_build_script", "[lib]\ndoctest = false\n", "build"),
        ] {
            let root = unique_tmp_dir(&format!("fix4_manifest_{seat_crate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            let build_line = if label == "package_build_script" {
                "build = \"build.rs\"\n"
            } else {
                ""
            };
            let _ = fs::write(
                root.join("crates").join(seat_crate).join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{seat_crate}\"\n{build_line}{lib_sec}[dependencies]\n{dep_line}"
                ),
            );
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| panic!("{seat_crate} `{label}` must fail manifest guard"));
            println!("PLANTED_FIX4_MANIFEST {seat_crate}:{label} => {err}");
            assert!(err.contains(expected_substr), "{err}");
            let _ = fs::remove_dir_all(&root);
        }
    }

    // 3. `league-seat-ui` bans `ratatui` and `crossterm` dependencies.
    for bad_dep in ["ratatui", "crossterm"] {
        let root = unique_tmp_dir(&format!("fix4_ui_ban_{bad_dep}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-ui/Cargo.toml"),
            format!(
                "[package]\nname = \"league-seat-ui\"\n[lib]\ndoctest = false\n[dependencies]\nleague-seat-types = {{ path = \"../league-seat-types\" }}\n{bad_dep} = \"0.1.0\"\n"
            ),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("league-seat-ui depending on `{bad_dep}` must fail guard"));
        println!("PLANTED_FIX4_UI_DEP {bad_dep} => {err}");
        assert!(err.contains("allow-list"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    // 4. `serde` / `serde_json` must set `default-features = false` in `league-seat-types` and `league-error`.
    for (crate_name, manifest, expected_dep) in [
        (
            "league-seat-types",
            "[package]\nname = \"league-seat-types\"\n[lib]\ndoctest = false\n[dependencies]\nleague-error = { path = \"../league-error\" }\nserde = { version = \"1.0.229\", features = [\"derive\"] }\n",
            "serde",
        ),
        (
            "league-seat-types",
            "[package]\nname = \"league-seat-types\"\n[lib]\ndoctest = false\n[dependencies]\nleague-error = { path = \"../league-error\" }\nserde = { version = \"1.0.229\", default-features = false, features = [\"alloc\", \"derive\"] }\n[dependencies.serde_json]\nversion = \"1.0.154\"\nfeatures = [\"alloc\"]\n",
            "serde_json",
        ),
        (
            "league-error",
            "[package]\nname = \"league-error\"\n[dependencies]\nserde = \"1.0.229\"\nserde_json = { version = \"1.0.154\", default-features = false, features = [\"alloc\"] }\n",
            "serde",
        ),
    ] {
        let root = unique_tmp_dir(&format!("fix4_serde_default_{crate_name}_{expected_dep}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates").join(crate_name).join("Cargo.toml"),
            manifest,
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| {
                panic!("{crate_name} `{expected_dep}` without `default-features = false` must fail")
            });
        println!("PLANTED_FIX4_SERDE_NO_STD {crate_name}:{expected_dep} => {err}");
        assert!(err.contains("default-features = false"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    // 5. `crates/league-seat-types/src/view.rs` view-model purity controls (`Item 4`).
    for (label, view_src, expected_substr) in [
        (
            "view_trait_decl",
            "pub trait BadViewTrait { fn render(&self); }\n",
            "must not declare traits",
        ),
        (
            "view_trait_impl_beyond_derives",
            "pub struct ViewSpan { pub text: String }\nimpl core::fmt::Display for ViewSpan { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { Ok(()) } }\n",
            "must not implement traits beyond derives",
        ),
        (
            "view_generic_struct",
            "pub struct BadGenView<T> { pub inner: T }\n",
            "must not declare generics or lifetimes",
        ),
        (
            "view_lifetime_struct",
            "pub struct BadBorrowedView<'a> { pub s: &'a str }\n",
            "must not declare generics or lifetimes",
        ),
        (
            "view_ref_field",
            "pub struct BadRefView { pub s: &'static str }\n",
            "owned data (references are forbidden)",
        ),
    ] {
        let root = unique_tmp_dir(&format!("fix4_view_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-seat-types/src/lib.rs"),
            format!("{hdr}pub mod view;\n"),
        );
        let _ = fs::write(root.join("crates/league-seat-types/src/view.rs"), view_src);
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("view-model `{label}` must fail Item 4 check"));
        println!("PLANTED_FIX4_VIEW_MODEL {label} => {err}");
        assert!(err.contains(expected_substr), "{err}");
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn planted_fix5_league_error_seat_rules_forbid_unsafe_code_unsafe_attrs_and_asm_controls() {
    let hdr = concat!(
        "#![no_std]\n",
        "#![forbid(unsafe_code)]\n",
        "#![forbid(\n",
        "    clippy::disallowed_methods,\n",
        "    clippy::disallowed_types,\n",
        "    clippy::disallowed_macros\n",
        ")]\n",
    );

    // 1. Item 2(a): `static AtomicU32` and host `#[cfg(...)]` / `include_str!` / `RefCell` in `league-error`.
    for (label, body, expected_substr) in [
        (
            "static_atomic_u32",
            "pub static ZZ_ATOMIC: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);\n",
            "`static` items are forbidden",
        ),
        (
            "fn_local_atomic_u32_type",
            "pub fn zz_swap() -> u32 { let _a = core::sync::atomic::AtomicU32::new(0); 0 }\n",
            "`atomic::` shared-state access is forbidden",
        ),
        (
            "include_str_in_lib_rs",
            "pub const S: &str = include_str!(\"x.txt\");\n",
            "macro `include_str!` is forbidden",
        ),
        (
            "refcell_in_league_error",
            "pub struct BadCell { c: core::cell::RefCell<u8> }\n",
            "cell::",
        ),
    ] {
        let root = unique_tmp_dir(&format!("fix5_err_{label}"));
        if let Err(err) = write_minimal_workspace_tree(&root) {
            panic!("setup workspace: {err}");
        }
        let _ = fs::write(
            root.join("crates/league-error/src/lib.rs"),
            format!("{hdr}{body}"),
        );
        let err = check_workspace_manifests_and_crate_sources(&root)
            .err()
            .unwrap_or_else(|| panic!("league-error `{label}` must fail seat source guard"));
        println!("PLANTED_FIX5 league-error:{label} => {err}");
        assert!(err.contains(expected_substr), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    // 2. Across all three no-std crates (`league-error`, `league-seat-types`, `league-seat-ui`):
    // - `#[cfg(not(target_os = "none"))] extern crate std;`
    // - missing `#![forbid(unsafe_code)]` / `#![deny(unsafe_code)]`
    // - `#[unsafe(no_mangle)]` and `#[unsafe(export_name = ...)]`
    // - `global_asm!`, `asm!`, `naked_asm!`
    for krate in ["league-error", "league-seat-types", "league-seat-ui"] {
        for (label, use_hdr, body, expected_substr) in [
            (
                "cfg_not_target_os_none_extern_std",
                true,
                "#[cfg(not(target_os = \"none\"))]\nextern crate std;\n",
                "#[cfg(...)]",
            ),
            (
                "missing_forbid_unsafe_code",
                false,
                "#![no_std]\n#![forbid(\n    clippy::disallowed_methods,\n    clippy::disallowed_types,\n    clippy::disallowed_macros\n)]\npub fn ok() {}\n",
                "#![forbid(unsafe_code)]",
            ),
            (
                "deny_unsafe_code_instead_of_forbid",
                false,
                "#![no_std]\n#![deny(unsafe_code)]\n#![forbid(\n    clippy::disallowed_methods,\n    clippy::disallowed_types,\n    clippy::disallowed_macros\n)]\npub fn ok() {}\n",
                "#![forbid(unsafe_code)]",
            ),
            (
                "allow_unsafe_code_attr",
                true,
                "#[allow(unsafe_code)]\npub fn bad() {}\n",
                "unsafe_code",
            ),
            (
                "unsafe_no_mangle_attr",
                true,
                "#[unsafe(no_mangle)]\npub extern \"C\" fn zz_sym() {}\n",
                "#[unsafe(...)]",
            ),
            (
                "unsafe_export_name_attr",
                true,
                "#[unsafe(export_name = \"zz_sym2\")]\npub extern \"C\" fn zz_sym2() {}\n",
                "#[unsafe(...)]",
            ),
            (
                "global_asm_macro",
                true,
                "core::arch::global_asm!(\"nop\");\n",
                "global_asm",
            ),
            (
                "asm_macro",
                true,
                "pub fn a() { core::arch::asm!(\"nop\"); }\n",
                "asm",
            ),
            (
                "naked_asm_macro",
                true,
                "pub fn n() { core::arch::naked_asm!(\"nop\"); }\n",
                "naked_asm",
            ),
        ] {
            let root = unique_tmp_dir(&format!("fix5_{krate}_{label}"));
            if let Err(err) = write_minimal_workspace_tree(&root) {
                panic!("setup workspace: {err}");
            }
            let src = if use_hdr {
                format!("{hdr}{body}")
            } else {
                body.to_owned()
            };
            let _ = fs::write(root.join("crates").join(krate).join("src/lib.rs"), src);
            let err = check_workspace_manifests_and_crate_sources(&root)
                .err()
                .unwrap_or_else(|| panic!("{krate} `{label}` must fail Fix 5 guard"));
            println!("PLANTED_FIX5 {krate}:{label} => {err}");
            assert!(err.contains(expected_substr), "{err}");
            let _ = fs::remove_dir_all(&root);
        }
    }
}
