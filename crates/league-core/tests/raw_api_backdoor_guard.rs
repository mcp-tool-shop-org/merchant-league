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

const FORBIDDEN_RAW_FNS: [&str; 7] = [
    "private_hints_for_auditor",
    "preview_bundle",
    "preview_bundle_live",
    "preview_bundle_in_mode",
    "preview_bundle_with_context",
    "preview_bundle_with_context_live",
    "preview_bundle_with_context_and_mode",
];

const FORBIDDEN_SEAT_IDENTIFIERS: [&str; 16] = [
    "TurnEngine",
    "EngineState",
    "engine_internals",
    "SweepMode",
    "SweepReceipt",
    "BundlePreview",
    "ScheduledShockSpec",
    "SealedEvalSeed",
    "SealedMandateDeal",
    "private_hints_for_auditor",
    "preview_bundle",
    "preview_bundle_live",
    "preview_bundle_in_mode",
    "preview_bundle_with_context",
    "preview_bundle_with_context_live",
    "preview_bundle_with_context_and_mode",
];

const ALLOWED_SEAT_PARAM_TYPES: [&str; 53] = [
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
    "SeatStatePredicates",
    "SeatState",
    "PrivateMandate",
    "SealedOrderBundle",
    "MarketAction",
    "TurnPhase",
    "AssetId",
    "Seat",
    "Turn",
    "TableId",
    "Units",
    "Bps",
    "ClubBot",
    "ClubBotDecision",
    "Action",
    "Frame",
    "Rect",
    "KeyEvent",
    "bool",
    "u8",
    "u16",
    "u32",
    "u64",
    "usize",
    "i64",
    "str",
    "String",
    "Option",
    "Result",
    "Vec",
    "BTreeMap",
    "BTreeSet",
    "Box",
    "mut",
    "dyn",
    "impl",
    "Self",
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

const ALLOWED_CLUB_HOST_PUB_FNS: [&str; 31] = [
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
    "own_mandate",
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
    "to_save_and_seal_bytes",
    "from_save_and_seal_bytes",
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
                while j < len && bytes[j] != b'\'' && j - i <= 8 {
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
        if head == "cfg" {
            if inner
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "test"))
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

fn expand_use_tree(
    rel_path: &str,
    tokens: &[Tok],
    prefix: &str,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let mut parts: Vec<String> = if prefix.is_empty() {
        Vec::new()
    } else {
        vec![prefix.to_owned()]
    };
    let mut i = 0_usize;
    while i < tokens.len() {
        match &tokens[i] {
            Tok::Other('*') => {
                return Err(format!(
                    "{rel_path}: glob `use *` is forbidden in guarded modules"
                ));
            }
            Tok::Ident(kw) if kw == "as" => {
                // Skip alias target (`A as B` expands only to canonical path `...::A`).
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
                                expand_use_tree(
                                    rel_path,
                                    &inner[item_start..pos],
                                    &group_prefix,
                                    out,
                                )?;
                            }
                            item_start = pos + 1;
                        }
                        _ => {}
                    }
                }
                if item_start < inner.len() {
                    expand_use_tree(rel_path, &inner[item_start..], &group_prefix, out)?;
                }
                return Ok(());
            }
            Tok::Other(',') => {
                if !parts.is_empty() {
                    out.push(parts.join("::"));
                }
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
    if !parts.is_empty() {
        out.push(parts.join("::"));
    }
    Ok(())
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
            if matches!(&w[0], Tok::Ident(a) if a == "SweepMode")
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
        if let Tok::Ident(ident) = tok
            && (ident == "preview_bundle_with_context_internal"
                || ident == "sweep_turn_detailed_with_mode"
                || FORBIDDEN_RAW_FNS.contains(&ident.as_str()))
        {
            return Err(format!(
                "{rel_path}: `pub fn {scope_name}::{fn_name}` calls restricted internal `{ident}` outside `engine_internals`"
            ));
        }
    }

    // B4 structural return-type ban: forbid renamed public wrappers returning raw state, engine,
    // receipt, bundle preview, or unredacted shock specs.
    let mut prev_was_quote = false;
    for tok in ret_tokens {
        if let Tok::Ident(ident) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            if ident == "BundlePreview" {
                return Err(format!(
                    "{rel_path}: `pub fn {scope_name}::{fn_name}` must not return raw `BundlePreview` outside `engine_internals`"
                ));
            }
            if ident == "EngineState" && !(scope_name == "TurnEngine" && fn_name == "state") {
                return Err(format!(
                    "{rel_path}: `pub fn {scope_name}::{fn_name}` must not return raw `EngineState`"
                ));
            }
            if ident == "TurnEngine"
                && !((scope_name == "TurnEngine" && fn_name == "new")
                    || (scope_name == "PublicPreviewState" && fn_name == "to_engine"))
            {
                return Err(format!(
                    "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose `TurnEngine`"
                ));
            }
            if ident == "SweepReceipt"
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
                    "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose raw `SweepReceipt`"
                ));
            }
            if ident == "ScheduledShockSpec"
                && !(rel_path == "commit.rs" && fn_name == "draw_shocks_for_turn")
            {
                return Err(format!(
                    "{rel_path}: `pub fn {scope_name}::{fn_name}` must not expose unredacted `ScheduledShockSpec`"
                ));
            }
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }

    Ok(())
}

fn scan_file_public_api(
    rel_path: &str,
    source: &str,
    is_lib_rs: bool,
) -> Result<CoreFileScan, String> {
    let tokens = tokenize_rust(source);
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
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });
        let in_public_non_internal_mod = scopes.iter().all(|s| match s {
            ScopeKind::Root { public } => *public,
            ScopeKind::Module {
                public,
                is_engine_internals,
                ..
            } => *public && !*is_engine_internals,
            ScopeKind::InherentImpl { .. } => true,
            ScopeKind::Block { .. } => false,
        });
        let enclosing_impl = match scopes.last() {
            Some(ScopeKind::InherentImpl { type_name, .. }) => Some(type_name.as_str()),
            _ => None,
        };
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

        // Track inherent `impl <Type> { ... }`.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            let mut before_where_or_generics = Vec::new();
            let mut saw_for = false;
            let mut depth_angle = 0_usize;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                match &tokens[j] {
                    Tok::Other('<') => depth_angle += 1,
                    Tok::Other('>') => depth_angle = depth_angle.saturating_sub(1),
                    Tok::Ident(s) if depth_angle == 0 => {
                        if s == "for" {
                            saw_for = true;
                        } else if s == "where" {
                            break;
                        } else {
                            before_where_or_generics.push(s.as_str());
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let type_name = before_where_or_generics.last().copied().unwrap_or("");
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(if !saw_for && !type_name.is_empty() {
                    ScopeKind::InherentImpl {
                        type_name: type_name.to_owned(),
                        cfg_test: attrs.cfg_test,
                    }
                } else {
                    ScopeKind::Block {
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
                for path in &expanded {
                    let leaf = path.rsplit("::").next().unwrap_or(path.as_str());
                    if leaf == "SweepMode"
                        || leaf == "engine_internals"
                        || path.contains("engine_internals")
                        || FORBIDDEN_RAW_FNS.contains(&leaf)
                    {
                        return Err(format!(
                            "{rel_path}: public `pub use` re-exports restricted item `{path}` outside `engine_internals`"
                        ));
                    }
                    if is_lib_rs && matches!(scopes.last(), Some(ScopeKind::Root { .. })) {
                        lib_reexports.push(path.clone());
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
            let mut sig_idx = fn_kw + 2;
            // Skip optional fn generic `<...>` parameters before `(`.
            if matches!(tokens.get(sig_idx), Some(Tok::Other('<'))) {
                let mut angle = 1_usize;
                sig_idx += 1;
                while sig_idx < tokens.len() && angle > 0 {
                    match tokens[sig_idx] {
                        Tok::Other('<') => angle += 1,
                        Tok::Other('>') => angle = angle.saturating_sub(1),
                        _ => {}
                    }
                    sig_idx += 1;
                }
            }
            let mut ret_start = sig_idx;
            if matches!(tokens.get(sig_idx), Some(Tok::LParen)) {
                ret_start = find_matching_paren(&tokens, sig_idx) + 1;
            }
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

        // Check `pub enum SweepMode` / `pub struct SweepMode` / `pub type SweepMode`.
        if matches!(vis, Visibility::Public)
            && matches!(
                tokens.get(after_vis),
                Some(Tok::Ident(s)) if matches!(s.as_str(), "enum" | "struct" | "type")
            )
            && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
        {
            if item_name == "SweepMode" && in_public_non_internal_mod && !in_engine_internals_mod {
                return Err(format!(
                    "{rel_path}: `SweepMode` must not be publicly reachable outside `engine_internals`"
                ));
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

fn check_seat_pub_fn_signature(
    rel_path: &str,
    enclosing_impl: Option<&str>,
    fn_name: &str,
    params: &[Tok],
    ret_tokens: &[Tok],
    require_seat_obs_param_on_bot: bool,
) -> Result<(), String> {
    let mut param_segments: Vec<&[Tok]> = Vec::new();
    let mut seg_start = 0_usize;
    let mut depth_paren = 0_usize;
    let mut depth_angle = 0_usize;
    let mut depth_bracket = 0_usize;
    for (pos, tok) in params.iter().enumerate() {
        match tok {
            Tok::LParen => depth_paren += 1,
            Tok::RParen => depth_paren = depth_paren.saturating_sub(1),
            Tok::Other('<') => depth_angle += 1,
            Tok::Other('>') => depth_angle = depth_angle.saturating_sub(1),
            Tok::LBracket => depth_bracket += 1,
            Tok::RBracket => depth_bracket = depth_bracket.saturating_sub(1),
            Tok::Other(',') if depth_paren == 0 && depth_angle == 0 && depth_bracket == 0 => {
                if seg_start < pos {
                    param_segments.push(&params[seg_start..pos]);
                }
                seg_start = pos + 1;
            }
            _ => {}
        }
    }
    if seg_start < params.len() {
        param_segments.push(&params[seg_start..]);
    }

    let mut saw_primary_seat_param = false;
    for seg in param_segments {
        let Some(colon_pos) = seg.iter().position(|t| matches!(t, Tok::Other(':'))) else {
            continue;
        };
        let ty_tokens = &seg[colon_pos + 1..];
        let mut prev_was_quote = false;
        for (idx, tok) in ty_tokens.iter().enumerate() {
            if let Tok::Ident(ident) = tok {
                if prev_was_quote {
                    prev_was_quote = false;
                    continue;
                }
                let is_path_prefix = matches!(
                    (ty_tokens.get(idx + 1), ty_tokens.get(idx + 2)),
                    (Some(Tok::Other(':')), Some(Tok::Other(':')))
                );
                if is_path_prefix {
                    if FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str()) {
                        return Err(format!(
                            "{rel_path}: seat `pub fn {fn_name}` parameter references forbidden symbol `{ident}`"
                        ));
                    }
                    prev_was_quote = false;
                    continue;
                }
                if SEAT_PRIMARY_PARAM_TYPES.contains(&ident.as_str()) {
                    saw_primary_seat_param = true;
                }
                if !ALLOWED_SEAT_PARAM_TYPES.contains(&ident.as_str()) {
                    return Err(format!(
                        "{rel_path}: seat `pub fn {fn_name}` accepts disallowed parameter type `{ident}` (seat functions may take only SeatObservation, PublicSweepReceipt, or seat-safe previews)"
                    ));
                }
            }
            prev_was_quote = matches!(tok, Tok::Other('\''));
        }
    }

    if require_seat_obs_param_on_bot
        && enclosing_impl == Some("ClubBot")
        && !matches!(fn_name, "new" | "bot_tie_seed")
        && !saw_primary_seat_param
    {
        return Err(format!(
            "{rel_path}: `ClubBot::{fn_name}` must take `&SeatObservation`, `PublicSweepReceipt`, or a seat-safe preview type"
        ));
    }

    let mut prev_was_quote = false;
    for tok in ret_tokens {
        if let Tok::Ident(ident) = tok {
            if prev_was_quote {
                prev_was_quote = false;
                continue;
            }
            if FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str()) {
                return Err(format!(
                    "{rel_path}: seat `pub fn {fn_name}` return type references forbidden symbol `{ident}`"
                ));
            }
        }
        prev_was_quote = matches!(tok, Tok::Other('\''));
    }

    Ok(())
}

fn scan_seat_module_source(
    rel_path: &str,
    source: &str,
    enforce_param_allowlist: bool,
    require_seat_obs_param_on_bot: bool,
) -> Result<(), String> {
    let tokens = tokenize_rust(source);

    // Full-file token check (including tests inside seat crates/modules): seat modules must never
    // name `TurnEngine`, `EngineState`, `engine_internals`, or raw engine internals anywhere.
    for tok in &tokens {
        if let Tok::Ident(ident) = tok
            && FORBIDDEN_SEAT_IDENTIFIERS.contains(&ident.as_str())
        {
            return Err(format!(
                "{rel_path}: seat module references forbidden engine-internal symbol `{ident}`"
            ));
        }
    }

    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut i = 0;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::InherentImpl { cfg_test, .. }
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

        // Ban glob imports (`use ...::*;`) in non-test seat code so aliased internals cannot hide.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use") {
            let mut j = after_vis + 1;
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                if matches!(tokens[j], Tok::Other('*')) {
                    return Err(format!(
                        "{rel_path}: glob `use *` is forbidden in non-test seat code"
                    ));
                }
                j += 1;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            let mut type_idents = Vec::new();
            let mut saw_for = false;
            let mut depth_angle = 0_usize;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                match &tokens[j] {
                    Tok::Other('<') => depth_angle += 1,
                    Tok::Other('>') => depth_angle = depth_angle.saturating_sub(1),
                    Tok::Ident(s) if depth_angle == 0 => {
                        if s == "for" {
                            saw_for = true;
                        } else if s == "where" {
                            break;
                        } else {
                            type_idents.push(s.as_str());
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let type_name = type_idents.last().copied().unwrap_or("");
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(if !saw_for && !type_name.is_empty() {
                    ScopeKind::InherentImpl {
                        type_name: type_name.to_owned(),
                        cfg_test: attrs.cfg_test,
                    }
                } else {
                    ScopeKind::Block {
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
            let mut sig_idx = fn_kw + 2;
            if matches!(tokens.get(sig_idx), Some(Tok::Other('<'))) {
                let mut angle = 1_usize;
                sig_idx += 1;
                while sig_idx < tokens.len() && angle > 0 {
                    match tokens[sig_idx] {
                        Tok::Other('<') => angle += 1,
                        Tok::Other('>') => angle = angle.saturating_sub(1),
                        _ => {}
                    }
                    sig_idx += 1;
                }
            }
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

            if enforce_param_allowlist && matches!(vis, Visibility::Public) {
                let enclosing_impl = match scopes.last() {
                    Some(ScopeKind::InherentImpl { type_name, .. }) => Some(type_name.as_str()),
                    _ => None,
                };
                check_seat_pub_fn_signature(
                    rel_path,
                    enclosing_impl,
                    fn_name,
                    params,
                    ret_slice,
                    require_seat_obs_param_on_bot,
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

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
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

    let mut visited = Vec::with_capacity(files.len());
    let mut saw_engine_internals = false;
    let mut observation_checked = false;
    let mut saw_preview_what_if = false;
    let mut saw_preview_seat_bundle = false;

    for path in files {
        let rel = path
            .strip_prefix(src_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_else(|| path.display().to_string());
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let is_lib_rs = rel == "lib.rs";
        let scan = scan_file_public_api(&rel, &source, is_lib_rs)?;

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

        // Seat-side rule inside `league-core/src`: `bot` module and non-host `club/` files.
        if rel == "bot.rs" || rel.starts_with("bot/") {
            scan_seat_module_source(&rel, &source, true, true)?;
        }
        if rel.starts_with("club/") && rel != "club/host.rs" {
            scan_seat_module_source(&rel, &source, false, false)?;
        }

        visited.push(rel);
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
    let mut dirs: Vec<PathBuf> = rel_members
        .into_iter()
        .map(|rel| workspace_root.join(rel))
        .collect();
    dirs.sort();
    Ok(dirs)
}

fn is_tui_seat_render_path(rel_crate_path: &str) -> bool {
    rel_crate_path == "src/lib.rs"
        || rel_crate_path == "src/ui.rs"
        || rel_crate_path.starts_with("src/ui/")
        || rel_crate_path == "src/render.rs"
        || rel_crate_path.starts_with("src/render/")
        || rel_crate_path == "src/screens.rs"
        || rel_crate_path.starts_with("src/screens/")
        || rel_crate_path == "src/views.rs"
        || rel_crate_path.starts_with("src/views/")
}

fn check_workspace_manifests_and_crate_sources(
    workspace_root: &Path,
) -> Result<Vec<String>, String> {
    let member_dirs = parse_workspace_members(workspace_root)?;
    let mut checked_crates = Vec::new();

    for crate_dir in member_dirs {
        let manifest_path = crate_dir.join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
        let crate_name = crate_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();

        if crate_name == "league-core" {
            check_league_core_manifest(&manifest)?;
        } else if crate_name == "league-standalone" {
            if !manifest.contains("engine-internals") {
                return Err(
                    "league-standalone/Cargo.toml is expected to opt into `league-core` `engine-internals`".to_owned(),
                );
            }
        } else {
            let mut in_features = false;
            for raw_line in manifest.lines() {
                let code_part = raw_line.split('#').next().unwrap_or("").trim();
                if code_part.starts_with('[') && code_part.ends_with(']') {
                    in_features = code_part == "[features]";
                }
                if code_part.contains("engine-internals") {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not enable `engine-internals`"
                    ));
                }
                if (in_features && code_part.contains("league-core/"))
                    || (code_part.contains("league-core") && code_part.contains("features"))
                {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not pass feature flags to `league-core`"
                    ));
                }
                // C1: Card 7 plans to add `league-core` to `league-relay` and must explicitly
                // transition `league-relay` onto the structural seat-side rule in the same PR.
                if crate_name == "league-relay" && code_part.contains("league-core") {
                    return Err(
                        "league-relay/Cargo.toml must not depend on `league-core`".to_owned()
                    );
                }
                if crate_name == "league-tui" && code_part.contains("league-standalone") {
                    return Err(
                        "league-tui/Cargo.toml must not depend on `league-standalone`".to_owned(),
                    );
                }
            }

            let mut rs_files = Vec::new();
            collect_rs_files_recursive(&crate_dir, &mut rs_files);
            rs_files.sort();
            for rs_path in rs_files {
                let src = fs::read_to_string(&rs_path)
                    .map_err(|err| format!("failed to read {}: {err}", rs_path.display()))?;
                let rel_in_crate = rs_path
                    .strip_prefix(&crate_dir)
                    .ok()
                    .and_then(|p| p.to_str())
                    .map(|s| s.replace('\\', "/"))
                    .unwrap_or_else(|| rs_path.display().to_string());

                if crate_name == "league-tui" {
                    let enforce_params = is_tui_seat_render_path(&rel_in_crate);
                    scan_seat_module_source(&rel_in_crate, &src, enforce_params, false)?;
                } else {
                    for tok in tokenize_rust(&src) {
                        if let Tok::Ident(ident) = tok
                            && (ident == "engine_internals"
                                || ident == "SweepMode"
                                || FORBIDDEN_RAW_FNS.contains(&ident.as_str()))
                        {
                            return Err(format!(
                                "{} references forbidden engine-internal symbol `{ident}`",
                                rs_path.display()
                            ));
                        }
                    }
                }
            }
        }
        checked_crates.push(crate_name);
    }

    let seen: BTreeSet<&str> = checked_crates.iter().map(String::as_str).collect();
    for expected in [
        "league-core",
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
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
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
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
    for name in member_names {
        let crate_dir = root.join("crates").join(name);
        fs::create_dir_all(crate_dir.join("src"))?;
        fs::write(crate_dir.join("src/lib.rs"), "// clean stub\n")?;
    }
    fs::write(
        root.join("crates/league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\nengine-internals = []\n[dev-dependencies]\nleague-core = { path = \".\", features = [\"engine-internals\"] }\n",
    )?;
    fs::write(
        root.join("crates/league-standalone/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\", features = [\"engine-internals\"] }\n",
    )?;
    for name in [
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
        "league-tui",
    ] {
        fs::write(
            root.join("crates").join(name).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n"),
        )?;
    }
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
    assert!(checked_crates.contains(&"league-tui".to_owned()));
    assert!(checked_crates.contains(&"league-relay".to_owned()));

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

    // Control H: `league-tui/src/lib.rs` imports `engine_internals` -> Err; comment near-miss -> Ok.
    let case_h = tmp_root.join("case_h");
    if let Err(err) = write_minimal_workspace_tree(&case_h) {
        panic!("setup case_h: {err}");
    }
    if let Err(err) = fs::write(
        case_h.join("crates/league-tui/src/lib.rs"),
        "// Note: engine_internals and preview_bundle stay out of comments\npub fn ok() {}\n",
    ) {
        panic!("write case_h ok: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_h).is_ok());

    if let Err(err) = fs::write(
        case_h.join("crates/league-tui/src/lib.rs"),
        "use league_core::engine_internals;\n",
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
        root.join("crates/league-tui/src/ui.rs"),
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
    let screens_dir = root.join("crates/league-tui/src/screens");
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
        root.join("crates/league-tui/src/render.rs"),
        "pub fn peek_hints() { let _ = std::any::type_name::<fn()>(); let _ = league_core::engine_internals::SweepMode::Live; }\n",
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
    "crates/league-gen",
    "crates/league-ledger",
    "crates/league-relay",
    "crates/league-season",
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
