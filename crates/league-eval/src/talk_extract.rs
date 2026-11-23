//! Frozen deterministic closed-grammar talk-phase trade commitment extractor (`§4.3`, `§5.2`).
//!
//! Reads `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs::whispers`) and
//! `decisions/day-DD-talk-<Guild>.json` (`parse_eval_talk_reply`) under `TALK_EXTRACTOR_VERSION_V2`
//! (`"talk_rules_v2"`) and extracts unambiguous `(proposer_seat, counterparty_seat, offered_action)`
//! spot-offer commitments scaled to `league_seat_types::UNIT_SCALE` milli-units/milli-`CRED`,
//! failing closed per message whenever any character, number, or clause falls outside the closed
//! grammar table.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use league_core::{AssetId, MarketAction, OfferProposal, OfferSide, SeatObservation};
use league_relay::NegotiationInputs;
use league_scorer::{
    ExtractedTalkCommitment, TALK_EXTRACTOR_VERSION_V2, TalkExtractionCounts, TalkSourceCounts,
    canonicalize_talk_commitments, sha256_hex, to_sorted_compact_json, validate_talk_commitment,
};
use league_seat_types::UNIT_SCALE;
use serde::Serialize;

use crate::error::EvalError;
use crate::records::{EvalDecisionRecord, EvalManifest};
use crate::reply::parse_eval_talk_reply;

/// Canonical frozen closed-grammar rule table for `talk_rules_v2` (`§4.3`, `§5.2`).
///
/// Field names appear in strict alphabetical order so compact JSON serialization and
/// `talk_extractor_rules_digest` remain deterministic. `talk_extractor_rules_digest` pins this
/// rule table together with LF-normalized `crates/league-eval/src/talk_extract.rs` source bytes
/// (covering per-message allowed-ASCII validation, fail-closed ambiguity latching, clause
/// splitting, closed-grammar whole-clause templates, thousands comma grouping, and cross-source
/// merging). Upstream reply parsing (`crates/league-eval/src/reply.rs`), decision records
/// (`crates/league-eval/src/records.rs`), and `NegotiationInputs` (`league-relay`) are pinned
/// by `scorer_git_sha` and `blind_inputs_manifest_sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TalkRulesV2Table {
    /// Whether `<GUILD> raw` / `<GUILD> refined` two-token goods are accepted.
    pub allow_guild_then_prefix_goods: bool,
    /// Whether exact `1,000`-style comma thousands groups are parsed inside numeric tokens.
    pub allow_thousands_comma_groups: bool,
    /// Complete set of allowed ASCII bytes after `quote_normalizations`; any message holding another ASCII byte or any non-ASCII char fails closed as ambiguous for its pair.
    pub allowed_ascii_chars: &'static str,
    /// Lowercase tokens matching `<SIDE>` as `OfferSide::Buy`.
    pub buy_tokens: &'static [&'static str],
    /// ASCII clause delimiter characters.
    pub clause_break_chars: &'static [&'static str],
    /// Whether `": "` breaks clauses (keeping compact `raw:0` intact).
    pub colon_space_breaks_clause: bool,
    /// Closed list of neutral lead-in clause templates permitted before a colon-space break.
    pub colon_space_lead_templates: &'static [&'static [&'static str]],
    /// Closed list of first-person spot-offer commitment templates matching complete clauses.
    pub commitment_templates: &'static [&'static [&'static str]],
    /// Single leading separator prefixes stripped between a good prefix and seat reference in compound tokens.
    pub compound_good_separators: &'static [&'static str],
    /// Whether conflicting active offers for the same `(pair, asset)` latch to contradicted.
    pub contradiction_latches_pair_asset: bool,
    /// Optional trailing currency unit tokens consumed by `<CRED?>`.
    pub cred_tokens: &'static [&'static str],
    /// Whether `.` followed immediately by an ASCII digit stays inside the current clause.
    pub decimal_dot_keeps_clause: bool,
    /// Negation prefixes expanded by `<NEG>` inside `retraction_templates`.
    pub negation_prefixes: &'static [&'static [&'static str]],
    /// Closed list of neutral non-commitment templates (plus empty clauses) that neither propose nor retract.
    pub neutral_templates: &'static [&'static [&'static str]],
    /// Fixed-point unit multiplier (`league_seat_types::UNIT_SCALE = 1_000`).
    pub number_unit_scale: i64,
    /// Prepositions/symbols matching `<PRICE_CUE>`.
    pub price_cues: &'static [&'static str],
    /// Typographic apostrophe `(from, to)` normalizations applied before whole-message validation.
    pub quote_normalizations: &'static [(&'static str, &'static str)],
    /// Prefixes identifying `AssetId::Raw(i)`.
    pub raw_prefixes: &'static [&'static str],
    /// Prefixes identifying `AssetId::Refined(i)`.
    pub refined_prefixes: &'static [&'static str],
    /// Optional single leading token stripped before matching `retraction_templates`.
    pub retraction_optional_lead_tokens: &'static [&'static str],
    /// Closed list of explicit retraction and first-person refusal templates matching complete clauses.
    pub retraction_templates: &'static [&'static [&'static str]],
    /// Lowercase tokens matching `<SIDE>` as `OfferSide::Sell`.
    pub sell_tokens: &'static [&'static str],
    /// Symbols emitted as standalone tokens even when adjacent to alphanumerics.
    pub standalone_tokens: &'static [&'static str],
    /// Non-alphanumeric characters kept inside compound tokens.
    pub token_join_chars: &'static [&'static str],
    /// Version identifier pinned in `ScorerConfig::talk_extractor_version`.
    pub version: &'static str,
}

impl TalkRulesV2Table {
    /// Returns the immutable frozen rule table for `talk_rules_v2`.
    #[must_use]
    pub const fn frozen() -> Self {
        Self {
            allow_guild_then_prefix_goods: true,
            allow_thousands_comma_groups: true,
            allowed_ascii_chars: " \t\n\r!',.:;@0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz",
            buy_tokens: &["buy"],
            clause_break_chars: &["\n", "\r", ";", "!", "."],
            colon_space_breaks_clause: true,
            colon_space_lead_templates: &[&["hi"], &["hello"], &["greetings"]],
            commitment_templates: &[
                &[
                    "i",
                    "will",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i'll",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "will",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we'll",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i",
                    "commit",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "commit",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i",
                    "offer",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "offer",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i",
                    "agree",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "agree",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i",
                    "will",
                    "post",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i'll",
                    "post",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
            ],
            compound_good_separators: &[":", "_"],
            contradiction_latches_pair_asset: true,
            cred_tokens: &["cred", "creds"],
            decimal_dot_keeps_clause: true,
            negation_prefixes: &[
                &["won't"],
                &["will", "not"],
                &["cannot"],
                &["can't"],
                &["don't"],
                &["do", "not"],
            ],
            neutral_templates: &[
                &["hi"],
                &["hello"],
                &["greetings"],
                &["thanks"],
                &["thank", "you"],
                &["ok"],
                &["okay"],
                &["noted"],
                &["understood"],
            ],
            number_unit_scale: UNIT_SCALE,
            price_cues: &["@", "at"],
            quote_normalizations: &[
                ("\u{2018}", "'"),
                ("\u{2019}", "'"),
                ("\u{02bc}", "'"),
                ("\u{ff07}", "'"),
            ],
            raw_prefixes: &["raw"],
            refined_prefixes: &["refined", "ref"],
            retraction_optional_lead_tokens: &["actually"],
            retraction_templates: &[
                &["cancel"],
                &["cancel", "that"],
                &["cancel", "all"],
                &["cancel", "all", "offers"],
                &["cancel", "my", "offer"],
                &["cancel", "that", "no", "deal"],
                &["i", "cancel"],
                &["we", "cancel"],
                &["i", "cancel", "that"],
                &["i", "cancel", "all", "offers"],
                &["retract"],
                &["retract", "that"],
                &["retract", "all"],
                &["retract", "all", "offers"],
                &["retract", "my", "offer"],
                &["i", "retract"],
                &["i", "retract", "that"],
                &["i", "retract", "my", "offer"],
                &["we", "retract"],
                &["withdraw"],
                &["withdraw", "that"],
                &["withdraw", "my", "offer"],
                &["i", "withdraw"],
                &["we", "withdraw"],
                &["never", "mind"],
                &["nevermind"],
                &["call", "off"],
                &["call", "that", "off"],
                &["no", "deal"],
                &["rescind"],
                &["walk", "back"],
                &["cancel", "<GOOD>"],
                &["cancel", "on", "<GOOD>"],
                &["cancel", "for", "<GOOD>"],
                &["cancel", "my", "offer", "on", "<GOOD>"],
                &["i", "cancel", "<GOOD>"],
                &["i", "cancel", "my", "offer", "on", "<GOOD>"],
                &["retract", "<GOOD>"],
                &["retract", "on", "<GOOD>"],
                &["retract", "for", "<GOOD>"],
                &["retract", "my", "offer", "on", "<GOOD>"],
                &["i", "retract", "<GOOD>"],
                &["i", "retract", "my", "offer", "on", "<GOOD>"],
                &["we", "retract", "<GOOD>"],
                &["withdraw", "<GOOD>"],
                &["withdraw", "my", "offer", "on", "<GOOD>"],
                &["i", "withdraw", "<GOOD>"],
                &["i", "withdraw", "my", "offer", "on", "<GOOD>"],
                &["no", "deal", "on", "<GOOD>"],
                &["no", "deal", "for", "<GOOD>"],
                &["rescind", "<GOOD>"],
                &["walk", "back", "<GOOD>"],
                &[
                    "i",
                    "<NEG>",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "<NEG>",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i",
                    "<NEG>",
                    "commit",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "<NEG>",
                    "commit",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "i",
                    "<NEG>",
                    "offer",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &[
                    "we",
                    "<NEG>",
                    "offer",
                    "to",
                    "<SIDE>",
                    "<AMOUNT>",
                    "<GOOD>",
                    "<PRICE_CUE>",
                    "<PRICE>",
                    "<CRED?>",
                ],
                &["i", "<NEG>", "<SIDE>", "<GOOD>"],
                &["we", "<NEG>", "<SIDE>", "<GOOD>"],
                &["i", "<NEG>", "commit", "to", "<SIDE>", "<GOOD>"],
                &["we", "<NEG>", "commit", "to", "<SIDE>", "<GOOD>"],
                &["i", "<NEG>", "offer", "to", "<SIDE>", "<GOOD>"],
                &["we", "<NEG>", "offer", "to", "<SIDE>", "<GOOD>"],
                &["i", "<NEG>", "trade", "<GOOD>"],
                &["we", "<NEG>", "trade", "<GOOD>"],
                &["i", "<NEG>", "deal", "on", "<GOOD>"],
                &["we", "<NEG>", "deal", "on", "<GOOD>"],
                &["i", "<NEG>", "<SIDE>"],
                &["we", "<NEG>", "<SIDE>"],
                &["i", "<NEG>", "commit"],
                &["we", "<NEG>", "commit"],
                &["i", "<NEG>", "trade"],
                &["we", "<NEG>", "trade"],
                &["i", "<NEG>", "deal"],
                &["we", "<NEG>", "deal"],
            ],
            sell_tokens: &["sell"],
            standalone_tokens: &["@"],
            token_join_chars: &["_", "'", ":", "."],
            version: TALK_EXTRACTOR_VERSION_V2,
        }
    }
}

/// Result returned by [`extract_talk_commitments_with_counts_for_state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkExtractionResult {
    /// Canonical extracted peer trade commitments for the state.
    pub commitments: Vec<ExtractedTalkCommitment>,
    /// Per-source and total message classification counts.
    pub counts: TalkExtractionCounts,
}

fn normalize_crlf_to_lf(source_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source_bytes.len());
    let mut i = 0_usize;
    while i < source_bytes.len() {
        if source_bytes[i] == b'\r' && source_bytes.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(source_bytes[i]);
            i += 1;
        }
    }
    out
}

/// Computes the canonical lowercase 64-hex SHA-256 digest covering both `table` and
/// LF-normalized extractor implementation source bytes (`§5.2`, `§8.2`).
pub fn talk_rules_v2_digest_for(
    table: &TalkRulesV2Table,
    source_bytes: &[u8],
) -> Result<String, EvalError> {
    let table_bytes = to_sorted_compact_json(table)
        .map_err(|e| EvalError::SchemaValidation(format!("serialize talk_rules_v2 table: {e}")))?;
    let normalized_source = normalize_crlf_to_lf(source_bytes);
    let mut payload = Vec::with_capacity(38 + table_bytes.len() + normalized_source.len());
    payload.extend_from_slice(b"merchant-league/talk-extractor/v2\n");
    payload.extend_from_slice(&table_bytes);
    payload.push(b'\n');
    payload.extend_from_slice(&normalized_source);
    Ok(sha256_hex(&payload))
}

/// Computes the pinned lowercase 64-hex SHA-256 digest of [`TalkRulesV2Table::frozen`] paired with
/// LF-normalized `crates/league-eval/src/talk_extract.rs` source bytes.
pub fn talk_rules_v2_digest() -> Result<String, EvalError> {
    talk_rules_v2_digest_for(
        &TalkRulesV2Table::frozen(),
        include_bytes!("talk_extract.rs"),
    )
}

/// Resolves seat guilds `0..seat_count` from `<game_dir>/manifest.json` or `decisions/`.
pub fn resolve_eval_seat_guilds(game_dir: &Path, seat_count: u8) -> Result<Vec<String>, EvalError> {
    let manifest_path = game_dir.join("manifest.json");
    if manifest_path.is_file() {
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", manifest_path.display())))?;
        let manifest: EvalManifest = serde_json::from_str(&raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", manifest_path.display())))?;
        let mut by_seat = BTreeMap::new();
        for seat in manifest.seats {
            by_seat.insert(seat.seat, seat.guild);
        }
        return collect_contiguous_guilds(&by_seat, seat_count, game_dir);
    }

    let decisions_dir = game_dir.join("decisions");
    if decisions_dir.is_dir() {
        let mut by_seat = BTreeMap::new();
        let entries = fs::read_dir(&decisions_dir)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", decisions_dir.display())))?;
        let mut paths = Vec::new();
        for entry_res in entries {
            let entry =
                entry_res.map_err(|e| EvalError::Io(format!("read decisions entry: {e}")))?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                paths.push(entry.path());
            }
        }
        paths.sort();
        for p in paths {
            let raw = fs::read_to_string(&p)
                .map_err(|e| EvalError::Io(format!("read {}: {e}", p.display())))?;
            if let Ok(rec) = serde_json::from_str::<EvalDecisionRecord>(&raw) {
                by_seat.entry(rec.seat).or_insert(rec.guild);
            }
        }
        return collect_contiguous_guilds(&by_seat, seat_count, game_dir);
    }

    Err(EvalError::MissingSourceSetup(format!(
        "cannot determine seat guilds in {} (no manifest.json or decisions/)",
        game_dir.display()
    )))
}

fn collect_contiguous_guilds(
    by_seat: &BTreeMap<u8, String>,
    seat_count: u8,
    game_dir: &Path,
) -> Result<Vec<String>, EvalError> {
    let mut out = Vec::with_capacity(usize::from(seat_count));
    for seat in 0..seat_count {
        let guild = by_seat.get(&seat).cloned().ok_or_else(|| {
            EvalError::MissingSourceSetup(format!(
                "missing guild mapping for seat {seat} in {}",
                game_dir.display()
            ))
        })?;
        out.push(guild);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceAssetOutcome {
    Active(OfferProposal),
    Retracted,
    Contradicted,
}

#[derive(Debug, Clone, Default)]
struct SourcePairOutcome {
    ambiguous_latched: bool,
    retracted_all: bool,
    assets: BTreeMap<AssetId, SourceAssetOutcome>,
}

impl SourcePairOutcome {
    fn latch_ambiguous(&mut self) {
        self.ambiguous_latched = true;
        self.retracted_all = true;
        self.assets.clear();
    }
}

fn split_clauses<'a>(body: &'a str, rules: &TalkRulesV2Table) -> Vec<(&'a str, bool)> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut start = 0_usize;
    let mut i = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        let mut is_break = rules
            .clause_break_chars
            .iter()
            .any(|delim| delim.as_bytes() == [b]);
        if is_break && b == b'.' && rules.decimal_dot_keeps_clause {
            let next_is_digit = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
            if next_is_digit {
                is_break = false;
            }
        }
        let mut ended_at_colon_space = false;
        if !is_break
            && rules.colon_space_breaks_clause
            && b == b':'
            && bytes.get(i + 1).is_some_and(u8::is_ascii_whitespace)
        {
            is_break = true;
            ended_at_colon_space = true;
        }
        if is_break {
            let slice = body[start..i].trim();
            if !slice.is_empty() || ended_at_colon_space {
                out.push((slice, ended_at_colon_space));
            }
            start = i + 1;
        }
        i += 1;
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        out.push((tail, false));
    }
    out
}

fn is_join_char(ch: char, rules: &TalkRulesV2Table) -> bool {
    rules
        .token_join_chars
        .iter()
        .any(|s| s.starts_with(ch) && s.len() == ch.len_utf8())
}

fn matching_standalone_prefix(slice: &str, rules: &TalkRulesV2Table) -> Option<&'static str> {
    rules
        .standalone_tokens
        .iter()
        .copied()
        .find(|tok| !tok.is_empty() && slice.starts_with(tok))
}

fn tokenize_clause(lower: &str, rules: &TalkRulesV2Table) -> Vec<String> {
    let bytes = lower.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0_usize;
    while i < bytes.len() {
        if let Some(standalone) = matching_standalone_prefix(&lower[i..], rules) {
            tokens.push(standalone.to_owned());
            i += standalone.len();
            continue;
        }
        let b = bytes[i];
        let ch = char::from(b);
        let starts_comma_num = b == b',' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
        let is_tok = b.is_ascii_alphanumeric() || is_join_char(ch, rules) || starts_comma_num;
        if !is_tok {
            i += 1;
            continue;
        }
        let mut buf = String::new();
        while i < bytes.len() {
            if matching_standalone_prefix(&lower[i..], rules).is_some() {
                break;
            }
            let c = bytes[i];
            if c == b',' {
                let prev_digit = !buf.is_empty() && buf.as_bytes()[buf.len() - 1].is_ascii_digit();
                let next_digit = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
                if prev_digit || next_digit {
                    buf.push(',');
                    i += 1;
                    continue;
                }
                break;
            }
            let c_ch = char::from(c);
            if c.is_ascii_alphanumeric() || is_join_char(c_ch, rules) {
                buf.push(c_ch);
                i += 1;
            } else {
                break;
            }
        }
        if !buf.is_empty() {
            tokens.push(buf);
        }
    }
    tokens
}

fn parse_pos_scaled_units(tok: &str, rules: &TalkRulesV2Table) -> Option<i64> {
    if tok.is_empty() {
        return None;
    }
    let digits = if tok.contains(',') {
        if !rules.allow_thousands_comma_groups {
            return None;
        }
        let mut groups = tok.split(',');
        let first = groups.next()?;
        if first.is_empty()
            || first.len() > 3
            || first.starts_with('0')
            || !first.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mut combined = String::from(first);
        let mut group_count = 1_usize;
        for group in groups {
            if group.len() != 3 || !group.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            combined.push_str(group);
            group_count += 1;
        }
        if group_count < 2 {
            return None;
        }
        combined
    } else {
        if !tok.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        tok.to_owned()
    };

    let val = digits.parse::<i64>().ok()?;
    if val <= 0 {
        return None;
    }
    val.checked_mul(rules.number_unit_scale)
}

fn resolve_guild_seat_ref(tok: &str, seat_guilds: &[String]) -> Option<u8> {
    for (idx, guild) in seat_guilds.iter().enumerate() {
        if tok == guild.to_ascii_lowercase() {
            return u8::try_from(idx).ok();
        }
    }
    None
}

fn resolve_seat_ref(tok: &str, seat_guilds: &[String]) -> Option<u8> {
    if !tok.is_empty()
        && tok.bytes().all(|b| b.is_ascii_digit())
        && let Ok(idx) = tok.parse::<usize>()
        && idx < seat_guilds.len()
    {
        return u8::try_from(idx).ok();
    }
    resolve_guild_seat_ref(tok, seat_guilds)
}

fn strip_single_compound_sep<'a>(rest: &'a str, rules: &TalkRulesV2Table) -> &'a str {
    for &sep in rules.compound_good_separators {
        if !sep.is_empty()
            && let Some(stripped) = rest.strip_prefix(sep)
        {
            return stripped;
        }
    }
    rest
}

fn parse_good_at(
    tokens: &[String],
    pos: usize,
    rules: &TalkRulesV2Table,
    seat_guilds: &[String],
) -> Option<(AssetId, usize)> {
    let t = tokens.get(pos)?.as_str();
    for &pref in rules.raw_prefixes {
        if let Some(rest) = t.strip_prefix(pref) {
            let trimmed = strip_single_compound_sep(rest, rules);
            if !trimmed.is_empty()
                && let Some(seat) = resolve_seat_ref(trimmed, seat_guilds)
            {
                return Some((AssetId::Raw(seat), 1));
            }
        }
    }
    for &pref in rules.refined_prefixes {
        if let Some(rest) = t.strip_prefix(pref) {
            let trimmed = strip_single_compound_sep(rest, rules);
            if !trimmed.is_empty()
                && let Some(seat) = resolve_seat_ref(trimmed, seat_guilds)
            {
                return Some((AssetId::Refined(seat), 1));
            }
        }
    }

    let next = tokens.get(pos + 1)?.as_str();
    if rules.raw_prefixes.contains(&t)
        && let Some(seat) = resolve_seat_ref(next, seat_guilds)
    {
        return Some((AssetId::Raw(seat), 2));
    }
    if rules.refined_prefixes.contains(&t)
        && let Some(seat) = resolve_seat_ref(next, seat_guilds)
    {
        return Some((AssetId::Refined(seat), 2));
    }
    // Only named guild tokens match `guild_then_prefix` (e.g. `Foundry raw`) so bare
    // whole-unit amounts like `2 raw 1` never capture the quantity as a seat.
    if rules.allow_guild_then_prefix_goods
        && let Some(seat) = resolve_guild_seat_ref(t, seat_guilds)
    {
        if rules.raw_prefixes.contains(&next) {
            return Some((AssetId::Raw(seat), 2));
        }
        if rules.refined_prefixes.contains(&next) {
            return Some((AssetId::Refined(seat), 2));
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
struct TemplateCapture {
    side: Option<OfferSide>,
    amount: Option<i64>,
    good: Option<AssetId>,
    price: Option<i64>,
}

fn match_template_from(
    tokens: &[String],
    t_idx: usize,
    pattern: &[&str],
    p_idx: usize,
    rules: &TalkRulesV2Table,
    seat_guilds: &[String],
    cap: TemplateCapture,
) -> Option<TemplateCapture> {
    if p_idx == pattern.len() {
        return (t_idx == tokens.len()).then_some(cap);
    }
    let pat = pattern[p_idx];
    if pat == "<CRED?>" {
        if t_idx < tokens.len()
            && rules.cred_tokens.contains(&tokens[t_idx].as_str())
            && let Some(matched) = match_template_from(
                tokens,
                t_idx + 1,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                cap.clone(),
            )
        {
            return Some(matched);
        }
        return match_template_from(tokens, t_idx, pattern, p_idx + 1, rules, seat_guilds, cap);
    }
    if t_idx >= tokens.len() {
        return None;
    }

    match pat {
        "<SIDE>" => {
            let tok = tokens[t_idx].as_str();
            let side = if rules.sell_tokens.contains(&tok) {
                OfferSide::Sell
            } else if rules.buy_tokens.contains(&tok) {
                OfferSide::Buy
            } else {
                return None;
            };
            let mut next_cap = cap;
            next_cap.side = Some(side);
            match_template_from(
                tokens,
                t_idx + 1,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                next_cap,
            )
        }
        "<AMOUNT>" => {
            let val = parse_pos_scaled_units(&tokens[t_idx], rules)?;
            let mut next_cap = cap;
            next_cap.amount = Some(val);
            match_template_from(
                tokens,
                t_idx + 1,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                next_cap,
            )
        }
        "<PRICE>" => {
            let val = parse_pos_scaled_units(&tokens[t_idx], rules)?;
            let mut next_cap = cap;
            next_cap.price = Some(val);
            match_template_from(
                tokens,
                t_idx + 1,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                next_cap,
            )
        }
        "<PRICE_CUE>" => {
            if !rules.price_cues.contains(&tokens[t_idx].as_str()) {
                return None;
            }
            match_template_from(
                tokens,
                t_idx + 1,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                cap,
            )
        }
        "<GOOD>" => {
            let (good, used) = parse_good_at(tokens, t_idx, rules, seat_guilds)?;
            let mut next_cap = cap;
            next_cap.good = Some(good);
            match_template_from(
                tokens,
                t_idx + used,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                next_cap,
            )
        }
        "<NEG>" => {
            for &neg in rules.negation_prefixes {
                if tokens[t_idx..].len() >= neg.len()
                    && neg
                        .iter()
                        .enumerate()
                        .all(|(off, w)| tokens[t_idx + off] == *w)
                    && let Some(matched) = match_template_from(
                        tokens,
                        t_idx + neg.len(),
                        pattern,
                        p_idx + 1,
                        rules,
                        seat_guilds,
                        cap.clone(),
                    )
                {
                    return Some(matched);
                }
            }
            None
        }
        literal => {
            if tokens[t_idx] != literal {
                return None;
            }
            match_template_from(
                tokens,
                t_idx + 1,
                pattern,
                p_idx + 1,
                rules,
                seat_guilds,
                cap,
            )
        }
    }
}

fn match_any_template(
    tokens: &[String],
    templates: &[&[&str]],
    rules: &TalkRulesV2Table,
    seat_guilds: &[String],
) -> Option<TemplateCapture> {
    for &template in templates {
        if let Some(cap) = match_template_from(
            tokens,
            0,
            template,
            0,
            rules,
            seat_guilds,
            TemplateCapture::default(),
        ) {
            return Some(cap);
        }
    }
    None
}

#[derive(Debug, Clone)]
enum MatchedClause {
    Neutral,
    Retract(Option<AssetId>),
    Offer(OfferProposal),
}

fn normalize_message_text(body: &str, rules: &TalkRulesV2Table) -> String {
    let mut normalized = body.to_owned();
    for &(from, to) in rules.quote_normalizations {
        if normalized.contains(from) {
            normalized = normalized.replace(from, to);
        }
    }
    normalized
}

fn message_has_only_allowed_ascii(text: &str, rules: &TalkRulesV2Table) -> bool {
    let allowed = rules.allowed_ascii_chars.as_bytes();
    text.bytes().all(|b| b.is_ascii() && allowed.contains(&b))
}

fn interpret_clause(
    clause: &str,
    rules: &TalkRulesV2Table,
    seat_guilds: &[String],
) -> Option<(MatchedClause, bool)> {
    let lower = clause.to_ascii_lowercase();
    let tokens = tokenize_clause(&lower, rules);
    if tokens.is_empty() {
        return Some((MatchedClause::Neutral, false));
    }

    if match_any_template(
        &tokens,
        rules.colon_space_lead_templates,
        rules,
        seat_guilds,
    )
    .is_some()
    {
        return Some((MatchedClause::Neutral, true));
    }

    if match_any_template(&tokens, rules.neutral_templates, rules, seat_guilds).is_some() {
        return Some((MatchedClause::Neutral, false));
    }

    if let Some(cap) = match_any_template(&tokens, rules.retraction_templates, rules, seat_guilds) {
        return Some((MatchedClause::Retract(cap.good), false));
    }
    if tokens.len() > 1
        && rules
            .retraction_optional_lead_tokens
            .contains(&tokens[0].as_str())
        && let Some(cap) =
            match_any_template(&tokens[1..], rules.retraction_templates, rules, seat_guilds)
    {
        return Some((MatchedClause::Retract(cap.good), false));
    }

    if let Some(cap) = match_any_template(&tokens, rules.commitment_templates, rules, seat_guilds) {
        let proposal = OfferProposal {
            side: cap.side?,
            good: cap.good?,
            amount: cap.amount?,
            price_per_unit_cred: cap.price?,
            domain_id: None,
            is_hybrid: false,
        };
        return Some((MatchedClause::Offer(proposal), false));
    }

    None
}

fn process_source_messages(
    focal_seat: u8,
    seat_guilds: &[String],
    messages: &[(u8, u8, String)],
    rules: &TalkRulesV2Table,
) -> (BTreeMap<(u8, u8), SourcePairOutcome>, TalkSourceCounts) {
    let mut by_pair: BTreeMap<(u8, u8), SourcePairOutcome> = BTreeMap::new();
    let mut counts = TalkSourceCounts::default();
    let seat_count = seat_guilds.len();
    for (proposer_seat, counterparty_seat, body) in messages {
        if *proposer_seat == focal_seat
            || usize::from(*proposer_seat) >= seat_count
            || *counterparty_seat != focal_seat
        {
            continue;
        }
        counts.seen = counts.seen.saturating_add(1);
        let pair = by_pair
            .entry((*proposer_seat, *counterparty_seat))
            .or_default();
        let normalized = normalize_message_text(body, rules);
        if !message_has_only_allowed_ascii(&normalized, rules) {
            counts.ambiguous = counts.ambiguous.saturating_add(1);
            pair.latch_ambiguous();
            continue;
        }
        let mut parsed_clauses = Vec::new();
        let mut any_retract_before_colon_space = false;
        let mut has_offer = false;
        let mut message_ambiguous = false;
        for (clause, ended_at_colon_space) in split_clauses(&normalized, rules) {
            match interpret_clause(clause, rules, seat_guilds) {
                Some((effect, is_colon_lead)) => {
                    if ended_at_colon_space {
                        match &effect {
                            MatchedClause::Neutral if is_colon_lead => {}
                            MatchedClause::Retract(_) => {
                                any_retract_before_colon_space = true;
                            }
                            _ => {
                                message_ambiguous = true;
                                break;
                            }
                        }
                    }
                    if matches!(effect, MatchedClause::Offer(_)) {
                        has_offer = true;
                    }
                    parsed_clauses.push(effect);
                }
                None => {
                    message_ambiguous = true;
                    break;
                }
            }
        }
        if any_retract_before_colon_space && has_offer {
            message_ambiguous = true;
        }
        if message_ambiguous {
            counts.ambiguous = counts.ambiguous.saturating_add(1);
            pair.latch_ambiguous();
            continue;
        }
        counts.matched = counts.matched.saturating_add(1);
        if pair.ambiguous_latched {
            continue;
        }
        for effect in parsed_clauses {
            match effect {
                MatchedClause::Neutral => {}
                MatchedClause::Retract(Some(good)) => {
                    pair.assets.insert(good, SourceAssetOutcome::Retracted);
                }
                MatchedClause::Retract(None) => {
                    pair.retracted_all = true;
                    for val in pair.assets.values_mut() {
                        *val = SourceAssetOutcome::Retracted;
                    }
                }
                MatchedClause::Offer(proposal) => {
                    let asset = proposal.good;
                    match pair.assets.get_mut(&asset) {
                        None | Some(SourceAssetOutcome::Retracted) => {
                            pair.assets
                                .insert(asset, SourceAssetOutcome::Active(proposal));
                        }
                        Some(SourceAssetOutcome::Active(existing)) if *existing == proposal => {}
                        Some(existing) => {
                            if rules.contradiction_latches_pair_asset {
                                *existing = SourceAssetOutcome::Contradicted;
                            } else {
                                *existing = SourceAssetOutcome::Active(proposal);
                            }
                        }
                    }
                }
            }
        }
    }
    (by_pair, counts)
}

fn finalize_pair_assets(
    pair_map: BTreeMap<(u8, u8), SourcePairOutcome>,
    focal_seat: u8,
    seat_count: u8,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
    let mut raw_commitments = Vec::new();
    for ((proposer_seat, counterparty_seat), pair) in pair_map {
        for (_, state) in pair.assets {
            if let SourceAssetOutcome::Active(order) = state {
                let commitment = ExtractedTalkCommitment {
                    proposer_seat,
                    counterparty_seat,
                    offered_action: MarketAction::PostOffer {
                        seat: proposer_seat,
                        order,
                    },
                };
                validate_talk_commitment(&commitment, focal_seat, seat_count).map_err(|e| {
                    EvalError::SchemaValidation(format!("invalid extracted talk commitment: {e}"))
                })?;
                raw_commitments.push(commitment);
            }
        }
    }
    canonicalize_talk_commitments(&raw_commitments)
        .map_err(|e| EvalError::SchemaValidation(format!("canonicalize talk commitments: {e}")))
}

fn merge_two_pair_sources(
    neg: &SourcePairOutcome,
    talk: &SourcePairOutcome,
    rules: &TalkRulesV2Table,
) -> SourcePairOutcome {
    if neg.ambiguous_latched || talk.ambiguous_latched {
        return SourcePairOutcome {
            ambiguous_latched: true,
            retracted_all: true,
            assets: BTreeMap::new(),
        };
    }
    let mut all_assets = BTreeMap::<AssetId, ()>::new();
    for &g in neg.assets.keys() {
        all_assets.insert(g, ());
    }
    for &g in talk.assets.keys() {
        all_assets.insert(g, ());
    }

    let mut merged = SourcePairOutcome {
        ambiguous_latched: false,
        retracted_all: neg.retracted_all || talk.retracted_all,
        assets: BTreeMap::new(),
    };
    for asset in all_assets.keys().copied() {
        let neg_state = neg.assets.get(&asset);
        let talk_state = talk.assets.get(&asset);
        let combined = match (neg_state, talk_state) {
            (Some(SourceAssetOutcome::Retracted), _) | (_, Some(SourceAssetOutcome::Retracted)) => {
                SourceAssetOutcome::Retracted
            }
            (Some(SourceAssetOutcome::Contradicted), _)
            | (_, Some(SourceAssetOutcome::Contradicted)) => SourceAssetOutcome::Contradicted,
            (None, Some(_)) if neg.retracted_all => SourceAssetOutcome::Retracted,
            (Some(_), None) if talk.retracted_all => SourceAssetOutcome::Retracted,
            (Some(SourceAssetOutcome::Active(a)), Some(SourceAssetOutcome::Active(b))) => {
                if a == b {
                    SourceAssetOutcome::Active(a.clone())
                } else if rules.contradiction_latches_pair_asset {
                    SourceAssetOutcome::Contradicted
                } else {
                    SourceAssetOutcome::Active(b.clone())
                }
            }
            (Some(SourceAssetOutcome::Active(a)), None) => SourceAssetOutcome::Active(a.clone()),
            (None, Some(SourceAssetOutcome::Active(b))) => SourceAssetOutcome::Active(b.clone()),
            (None, None) => SourceAssetOutcome::Retracted,
        };
        merged.assets.insert(asset, combined);
    }
    merged
}

/// Test helper evaluating an explicit [`TalkRulesV2Table`] against ordered
/// `(proposer_seat, counterparty_seat, body)` messages.
#[doc(hidden)]
pub fn extract_talk_commitments_from_messages_with_rules(
    focal_seat: u8,
    seat_guilds: &[String],
    messages: &[(u8, u8, String)],
    obs: &SeatObservation,
    rules: &TalkRulesV2Table,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
    let seat_count = obs.spec.seat_count;
    if usize::from(seat_count) != seat_guilds.len() {
        return Err(EvalError::SchemaValidation(format!(
            "seat_guilds length {} != spec.seat_count {seat_count}",
            seat_guilds.len()
        )));
    }
    if focal_seat >= seat_count {
        return Err(EvalError::UnknownFocalSeat(format!(
            "focal_seat {focal_seat} out of range 0..{seat_count}"
        )));
    }

    let (pair_map, _counts) = process_source_messages(focal_seat, seat_guilds, messages, rules);
    finalize_pair_assets(pair_map, focal_seat, seat_count)
}

/// Pure deterministic talk commitment extraction over ordered `(proposer_seat, counterparty_seat, body)`
/// messages (`talk_rules_v2`, `§4.3`).
pub fn extract_talk_commitments_from_messages(
    focal_seat: u8,
    seat_guilds: &[String],
    messages: &[(u8, u8, String)],
    obs: &SeatObservation,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
    let rules = TalkRulesV2Table::frozen();
    extract_talk_commitments_from_messages_with_rules(
        focal_seat,
        seat_guilds,
        messages,
        obs,
        &rules,
    )
}

/// Extracts frozen deterministic `talk_rules_v2` commitments and message classification counts
/// for `day` from `<source_game_dir>/to-seat/day-DD-act-<Guild>.negotiation.json` and
/// `<source_game_dir>/decisions/day-DD-talk-<Guild>.json` (`§4.3`, `§8.1`).
///
/// Because neither file carries cross-source message timestamps, each source is processed
/// independently and merged per `(proposer, counterparty, asset)` with retraction and ambiguity
/// latch dominance so a retracted or latched-empty commitment in one source is never resurrected
/// by a stale copy in the other.
pub fn extract_talk_commitments_with_counts_for_state(
    source_game_dir: &Path,
    day: u8,
    focal_seat: u8,
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<TalkExtractionResult, EvalError> {
    let rules = TalkRulesV2Table::frozen();
    let seat_count = obs.spec.seat_count;
    if usize::from(seat_count) != seat_guilds.len() {
        return Err(EvalError::SchemaValidation(format!(
            "seat_guilds length {} != spec.seat_count {seat_count}",
            seat_guilds.len()
        )));
    }
    if focal_seat >= seat_count {
        return Err(EvalError::UnknownFocalSeat(format!(
            "focal_seat {focal_seat} out of range 0..{seat_count}"
        )));
    }

    let day_str = format!("{day:02}");
    let guild_to_seat: BTreeMap<String, u8> = seat_guilds
        .iter()
        .enumerate()
        .filter_map(|(i, g)| u8::try_from(i).ok().map(|s| (g.clone(), s)))
        .collect();

    let mut neg_msgs = Vec::new();
    for (recipient_idx, recipient_guild) in seat_guilds.iter().enumerate() {
        let Ok(counterparty_seat) = u8::try_from(recipient_idx) else {
            continue;
        };
        let neg_path = source_game_dir.join("to-seat").join(format!(
            "day-{day_str}-act-{recipient_guild}.negotiation.json"
        ));
        if !neg_path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&neg_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", neg_path.display())))?;
        let neg: NegotiationInputs = serde_json::from_str(&raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", neg_path.display())))?;
        for whisper in neg.whispers {
            if let Some(&proposer_seat) = guild_to_seat.get(&whisper.from_guild) {
                neg_msgs.push((proposer_seat, counterparty_seat, whisper.body));
            }
        }
    }

    let mut talk_msgs = Vec::new();
    for (proposer_idx, proposer_guild) in seat_guilds.iter().enumerate() {
        let Ok(proposer_seat) = u8::try_from(proposer_idx) else {
            continue;
        };
        let dec_path = source_game_dir
            .join("decisions")
            .join(format!("day-{day_str}-talk-{proposer_guild}.json"));
        if !dec_path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&dec_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", dec_path.display())))?;
        let rec: EvalDecisionRecord = serde_json::from_str(&raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", dec_path.display())))?;
        if rec.no_reply {
            continue;
        }
        let Some(raw_reply) = rec.raw_reply.as_deref() else {
            continue;
        };
        let parsed = parse_eval_talk_reply(
            raw_reply,
            proposer_seat,
            proposer_guild,
            seat_guilds,
            rec.keeps_notes,
        );
        if parsed.refusal_code.is_some() {
            continue;
        }
        for (recipient_guild, whisper) in parsed.accepted_whispers {
            if let Some(&counterparty_seat) = guild_to_seat.get(&recipient_guild) {
                talk_msgs.push((proposer_seat, counterparty_seat, whisper.body));
            }
        }
    }

    let (neg_outcomes, neg_counts) =
        process_source_messages(focal_seat, seat_guilds, &neg_msgs, &rules);
    let (talk_outcomes, talk_counts) =
        process_source_messages(focal_seat, seat_guilds, &talk_msgs, &rules);

    let mut all_pairs = BTreeMap::<(u8, u8), ()>::new();
    for &pair in neg_outcomes.keys() {
        all_pairs.insert(pair, ());
    }
    for &pair in talk_outcomes.keys() {
        all_pairs.insert(pair, ());
    }

    let empty = SourcePairOutcome::default();
    let mut merged = BTreeMap::new();
    for pair in all_pairs.keys().copied() {
        let n = neg_outcomes.get(&pair).unwrap_or(&empty);
        let t = talk_outcomes.get(&pair).unwrap_or(&empty);
        merged.insert(pair, merge_two_pair_sources(n, t, &rules));
    }

    let commitments = finalize_pair_assets(merged, focal_seat, seat_count)?;
    let counts = TalkExtractionCounts::from_sources(neg_counts, talk_counts)
        .map_err(|e| EvalError::SchemaValidation(format!("talk extraction counts: {e}")))?;
    Ok(TalkExtractionResult {
        commitments,
        counts,
    })
}

/// Convenience wrapper returning only extracted commitments from
/// [`extract_talk_commitments_with_counts_for_state`].
pub fn extract_talk_commitments_for_state(
    source_game_dir: &Path,
    day: u8,
    focal_seat: u8,
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
    Ok(extract_talk_commitments_with_counts_for_state(
        source_game_dir,
        day,
        focal_seat,
        seat_guilds,
        obs,
    )?
    .commitments)
}
