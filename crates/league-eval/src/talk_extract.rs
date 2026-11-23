//! Frozen deterministic talk-phase trade commitment extractor (`§4.3`, `§5.2`).
//!
//! Reads `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs::whispers`) and
//! `decisions/day-DD-talk-<Guild>.json` (`parse_eval_talk_reply`) under `TALK_EXTRACTOR_VERSION_V2`
//! (`"talk_rules_v2"`) and extracts unambiguous `(proposer_seat, counterparty_seat, offered_action)`
//! spot-offer commitments scaled to `league_seat_types::UNIT_SCALE` milli-units/milli-`CRED`,
//! dropping any commitment retracted or contradicted in talk.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use league_core::{AssetId, MarketAction, OfferProposal, OfferSide, SeatObservation};
use league_relay::NegotiationInputs;
use league_scorer::{
    ExtractedTalkCommitment, TALK_EXTRACTOR_VERSION_V2, canonicalize_talk_commitments, sha256_hex,
    to_sorted_compact_json, validate_talk_commitment,
};
use league_seat_types::UNIT_SCALE;
use serde::Serialize;

use crate::error::EvalError;
use crate::records::{EvalDecisionRecord, EvalManifest};
use crate::reply::parse_eval_talk_reply;

/// Canonical frozen rule table for `talk_rules_v2` (`§4.3`, `§5.2`).
///
/// Field names appear in strict alphabetical order so compact JSON serialization and
/// `talk_extractor_rules_digest` remain deterministic. Every extraction rule lives either here as
/// data or in this source file hashed into [`talk_rules_v2_digest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TalkRulesV2Table {
    /// Cues indicating uncertainty, hedging, or questions (discard clause).
    pub ambiguity_cues: &'static [&'static str],
    /// Tokens designating a buy offer (`OfferSide::Buy`).
    pub buy_tokens: &'static [&'static str],
    /// ASCII clause delimiter characters.
    pub clause_break_chars: &'static [&'static str],
    /// Whether `": "` breaks clauses (keeping compact `raw:0` intact).
    pub colon_space_breaks_clause: bool,
    /// Phrases/tokens signaling an explicit commitment to post an offer.
    pub commitment_cues: &'static [&'static str],
    /// Whether conflicting active offers for the same `(pair, asset)` latch to contradicted.
    pub contradiction_latches_pair_asset: bool,
    /// Optional unit suffixes following price figures (`CRED`).
    pub cred_tokens: &'static [&'static str],
    /// Precedence order when classifying each clause.
    pub cue_precedence: &'static [&'static str],
    /// Whether `.` followed immediately by an ASCII digit stays inside the current clause.
    pub decimal_dot_keeps_clause: bool,
    /// Require exactly one positive whole-number amount before the price cue.
    pub exactly_one_amount_before_price: bool,
    /// Require exactly one asset specification before the price cue.
    pub exactly_one_good_before_price: bool,
    /// Require exactly one positive whole-number price after the price cue.
    pub exactly_one_price_after_cue: bool,
    /// Require exactly one price cue per commitment clause.
    pub exactly_one_price_cue: bool,
    /// Explicit cancellation cues that retract any prior matching commitment cleanly.
    pub explicit_retraction_cues: &'static [&'static str],
    /// Cross-source merge policy across `to-seat/*.negotiation.json` and `decisions/*-talk-*.json`.
    pub merge_policy: &'static str,
    /// Negation cues that trigger retraction only when governing a trade verb nearby.
    pub negated_offer_cues: &'static [&'static str],
    /// Maximum token distance after a negated cue within which a trade verb triggers retraction.
    pub negated_offer_max_gap_tokens: usize,
    /// Trade verbs governed by `negated_offer_cues` to distinguish offers from filler phrases.
    pub negated_offer_verbs: &'static [&'static str],
    /// Fixed-point unit multiplier (`league_seat_types::UNIT_SCALE = 1_000`).
    pub number_unit_scale: i64,
    /// Prepositions/symbols separating order amount/good from limit unit price.
    pub price_cues: &'static [&'static str],
    /// Prefixes identifying `AssetId::Raw(i)`.
    pub raw_prefixes: &'static [&'static str],
    /// Prefixes identifying `AssetId::Refined(i)`.
    pub refined_prefixes: &'static [&'static str],
    /// Retraction scoping policy name.
    pub retraction_scope_policy: &'static str,
    /// Supported seat/good reference grammar forms.
    pub seat_ref_forms: &'static [&'static str],
    /// Tokens designating a sell offer (`OfferSide::Sell`).
    pub sell_tokens: &'static [&'static str],
    /// Non-alphanumeric characters kept inside compound tokens.
    pub token_join_chars: &'static [&'static str],
    /// Leading/trailing punctuation characters trimmed from finished tokens.
    pub token_trim_chars: &'static [&'static str],
    /// Conditional or out-of-scope settlement cues (discard clause rather than guessing).
    pub unsupported_condition_cues: &'static [&'static str],
    /// Version identifier pinned in `ScorerConfig::talk_extractor_version`.
    pub version: &'static str,
}

impl TalkRulesV2Table {
    /// Returns the immutable frozen rule table for `talk_rules_v2`.
    #[must_use]
    pub const fn frozen() -> Self {
        Self {
            ambiguity_cues: &[
                "?",
                "maybe",
                "might",
                "perhaps",
                "possibly",
                "probably",
                "could",
                "would consider",
                "consider",
                "thinking",
                "uncertain",
                "not sure",
                "unsure",
                "tentative",
                "roughly",
                "around",
                "about",
                "approx",
                "approximately",
                "either",
                "or",
                "whether",
                "depends",
            ],
            buy_tokens: &["buy", "buying", "bid"],
            clause_break_chars: &["\n", "\r", ";", "!", "?", "."],
            colon_space_breaks_clause: true,
            commitment_cues: &[
                "will sell",
                "will buy",
                "i'll sell",
                "i'll buy",
                "we will sell",
                "we will buy",
                "i commit to",
                "commit to",
                "agree to",
                "offer to",
                "post sell",
                "post buy",
                "i offer",
                "offer",
                "selling",
                "buying",
                "sell",
                "buy",
            ],
            contradiction_latches_pair_asset: true,
            cred_tokens: &["cred", "creds"],
            cue_precedence: &[
                "retraction",
                "ambiguity",
                "unsupported_condition",
                "commitment",
            ],
            decimal_dot_keeps_clause: true,
            exactly_one_amount_before_price: true,
            exactly_one_good_before_price: true,
            exactly_one_price_after_cue: true,
            exactly_one_price_cue: true,
            explicit_retraction_cues: &[
                "cancel",
                "retract",
                "withdraw",
                "never mind",
                "nevermind",
                "call off",
                "no deal",
                "rescind",
                "walk back",
            ],
            merge_policy: "per_source_then_union_with_retraction_dominance",
            negated_offer_cues: &["won't", "will not", "cannot", "can't", "don't", "do not"],
            negated_offer_max_gap_tokens: 2,
            negated_offer_verbs: &[
                "sell", "selling", "buy", "buying", "bid", "offer", "post", "commit", "trade",
                "deal",
            ],
            number_unit_scale: UNIT_SCALE,
            price_cues: &["@", "at", "for"],
            raw_prefixes: &["raw"],
            refined_prefixes: &["refined", "ref"],
            retraction_scope_policy: "governs_offer_or_standalone_retraction",
            seat_ref_forms: &[
                "seat_index",
                "guild_name_case_insensitive",
                "compound_prefix_ref",
                "prefix_then_ref",
                "guild_then_prefix",
            ],
            sell_tokens: &["sell", "selling", "ask"],
            token_join_chars: &["_", "'", ":", "."],
            token_trim_chars: &[".", ":", "_"],
            unsupported_condition_cues: &[
                "if", "unless", "when", "until", "hybrid", "domain", "swap", "check", "escrow",
                "pledge",
            ],
            version: TALK_EXTRACTOR_VERSION_V2,
        }
    }
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
    retracted_all: bool,
    assets: BTreeMap<AssetId, SourceAssetOutcome>,
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'\''
}

fn contains_cue(lower: &str, cue: &str) -> bool {
    if cue == "?" || cue == "@" {
        return lower.contains(cue);
    }
    let hay = lower.as_bytes();
    let mut start_from = 0_usize;
    while let Some(rel) = lower[start_from..].find(cue) {
        let abs_start = start_from + rel;
        let abs_end = abs_start + cue.len();
        let left_ok = abs_start == 0 || !is_word_byte(hay[abs_start - 1]);
        let right_ok = abs_end == hay.len() || !is_word_byte(hay[abs_end]);
        if left_ok && right_ok {
            return true;
        }
        start_from = abs_start + 1;
    }
    false
}

fn has_negated_offer_retraction(tokens: &[String], rules: &TalkRulesV2Table) -> bool {
    for &cue in rules.negated_offer_cues {
        let cue_words: Vec<&str> = cue.split_ascii_whitespace().collect();
        if cue_words.is_empty() || cue_words.len() > tokens.len() {
            continue;
        }
        let max_start = tokens.len() - cue_words.len();
        for start in 0..=max_start {
            let matches_cue = cue_words
                .iter()
                .enumerate()
                .all(|(off, w)| tokens[start + off] == *w);
            if !matches_cue {
                continue;
            }
            let after = start + cue_words.len();
            let look_end = (after + rules.negated_offer_max_gap_tokens).min(tokens.len());
            if tokens[after..look_end]
                .iter()
                .any(|tok| rules.negated_offer_verbs.contains(&tok.as_str()))
            {
                return true;
            }
        }
    }
    false
}

fn split_clauses<'a>(body: &'a str, rules: &TalkRulesV2Table) -> Vec<&'a str> {
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
        if !is_break
            && rules.colon_space_breaks_clause
            && b == b':'
            && bytes.get(i + 1).is_some_and(u8::is_ascii_whitespace)
        {
            is_break = true;
        }
        if is_break {
            let end = if b == b'?' { i + 1 } else { i };
            let slice = body[start..end].trim();
            if !slice.is_empty() {
                out.push(slice);
            }
            start = i + 1;
        }
        i += 1;
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn char_in_rule_list(ch: char, list: &[&str]) -> bool {
    list.iter()
        .any(|s| s.starts_with(ch) && s.len() == ch.len_utf8())
}

fn is_join_char(ch: char, rules: &TalkRulesV2Table) -> bool {
    char_in_rule_list(ch, rules.token_join_chars)
}

fn is_trim_char(ch: char, rules: &TalkRulesV2Table) -> bool {
    char_in_rule_list(ch, rules.token_trim_chars)
}

fn tokenize_clause(lower: &str, rules: &TalkRulesV2Table) -> Vec<String> {
    let bytes = lower.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'@' {
            tokens.push("@".to_owned());
            i += 1;
            continue;
        }
        let ch = char::from(b);
        let is_tok = b.is_ascii_alphanumeric() || is_join_char(ch, rules);
        if !is_tok {
            i += 1;
            continue;
        }
        let mut buf = String::new();
        while i < bytes.len() {
            let c = bytes[i];
            if c == b',' {
                let prev_digit = !buf.is_empty() && buf.as_bytes()[buf.len() - 1].is_ascii_digit();
                let next_digit = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
                if prev_digit && next_digit {
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
        let trimmed = buf.trim_matches(|c| is_trim_char(c, rules));
        if !trimmed.is_empty() {
            tokens.push(trimmed.to_owned());
        }
    }
    tokens
}

enum ParsedScaledNum {
    NotNumeric,
    InvalidOrOverflow,
    Valid(i64),
}

fn parse_pos_scaled_units(tok: &str, scale: i64) -> ParsedScaledNum {
    let cleaned = tok.replace('_', "");
    if cleaned.is_empty() {
        return ParsedScaledNum::NotNumeric;
    }
    if cleaned.contains('.') {
        let without_dots = cleaned.replace('.', "");
        if !without_dots.is_empty() && without_dots.bytes().all(|b| b.is_ascii_digit()) {
            return ParsedScaledNum::InvalidOrOverflow;
        }
        return ParsedScaledNum::NotNumeric;
    }
    if !cleaned.bytes().all(|b| b.is_ascii_digit()) {
        return ParsedScaledNum::NotNumeric;
    }
    let Ok(val) = cleaned.parse::<i64>() else {
        return ParsedScaledNum::InvalidOrOverflow;
    };
    if val <= 0 {
        return ParsedScaledNum::InvalidOrOverflow;
    }
    match val.checked_mul(scale) {
        Some(scaled) if scaled > 0 => ParsedScaledNum::Valid(scaled),
        _ => ParsedScaledNum::InvalidOrOverflow,
    }
}

fn resolve_guild_seat_ref(
    tok: &str,
    seat_guilds: &[String],
    rules: &TalkRulesV2Table,
) -> Option<u8> {
    let stripped = tok.trim_matches(|c| is_trim_char(c, rules));
    for (idx, guild) in seat_guilds.iter().enumerate() {
        if stripped == guild.to_ascii_lowercase() {
            return u8::try_from(idx).ok();
        }
    }
    None
}

fn resolve_seat_ref(tok: &str, seat_guilds: &[String], rules: &TalkRulesV2Table) -> Option<u8> {
    let stripped = tok.trim_matches(|c| is_trim_char(c, rules));
    if stripped.bytes().all(|b| b.is_ascii_digit())
        && !stripped.is_empty()
        && let Ok(idx) = stripped.parse::<usize>()
        && idx < seat_guilds.len()
    {
        return u8::try_from(idx).ok();
    }
    resolve_guild_seat_ref(tok, seat_guilds, rules)
}

fn parse_good_span(
    tokens: &[String],
    rules: &TalkRulesV2Table,
    seat_guilds: &[String],
) -> Vec<(AssetId, usize, usize)> {
    let mut matches = Vec::new();
    let mut i = 0_usize;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        let mut compound_hit = None;
        for &pref in rules.raw_prefixes {
            if let Some(rest) = t.strip_prefix(pref)
                && !rest.is_empty()
                && let Some(seat) = resolve_seat_ref(rest, seat_guilds, rules)
            {
                compound_hit = Some(AssetId::Raw(seat));
                break;
            }
        }
        if compound_hit.is_none() {
            for &pref in rules.refined_prefixes {
                if let Some(rest) = t.strip_prefix(pref)
                    && !rest.is_empty()
                    && let Some(seat) = resolve_seat_ref(rest, seat_guilds, rules)
                {
                    compound_hit = Some(AssetId::Refined(seat));
                    break;
                }
            }
        }
        if let Some(asset) = compound_hit {
            matches.push((asset, i, i + 1));
            i += 1;
            continue;
        }

        if i + 1 < tokens.len() {
            let next = tokens[i + 1].as_str();
            if rules.raw_prefixes.contains(&t)
                && let Some(seat) = resolve_seat_ref(next, seat_guilds, rules)
            {
                matches.push((AssetId::Raw(seat), i, i + 2));
                i += 2;
                continue;
            }
            if rules.refined_prefixes.contains(&t)
                && let Some(seat) = resolve_seat_ref(next, seat_guilds, rules)
            {
                matches.push((AssetId::Refined(seat), i, i + 2));
                i += 2;
                continue;
            }
            // Only named guild tokens match `guild_then_prefix` (e.g. `Foundry raw`) so bare
            // whole-unit amounts like `2 raw 1` or `0 raw 1` never capture the quantity as a seat.
            if let Some(seat) = resolve_guild_seat_ref(t, seat_guilds, rules) {
                if rules.raw_prefixes.contains(&next) {
                    matches.push((AssetId::Raw(seat), i, i + 2));
                    i += 2;
                    continue;
                }
                if rules.refined_prefixes.contains(&next) {
                    matches.push((AssetId::Refined(seat), i, i + 2));
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    matches
}

enum ClauseEffect {
    Ignore,
    Retract(Option<AssetId>),
    Offer(OfferProposal),
}

fn interpret_clause(
    clause: &str,
    rules: &TalkRulesV2Table,
    seat_guilds: &[String],
) -> ClauseEffect {
    let lower = clause.to_ascii_lowercase();
    let tokens = tokenize_clause(&lower, rules);
    if tokens.is_empty() {
        return ClauseEffect::Ignore;
    }

    let is_explicit_retract = rules
        .explicit_retraction_cues
        .iter()
        .any(|cue| contains_cue(&lower, cue));
    let is_negated_retract = has_negated_offer_retraction(&tokens, rules);
    if is_explicit_retract || is_negated_retract {
        let goods = parse_good_span(&tokens, rules, seat_guilds);
        let targeted = if goods.len() == 1 {
            Some(goods[0].0)
        } else {
            None
        };
        return ClauseEffect::Retract(targeted);
    }

    if rules
        .ambiguity_cues
        .iter()
        .any(|cue| contains_cue(&lower, cue))
    {
        return ClauseEffect::Ignore;
    }
    if rules
        .unsupported_condition_cues
        .iter()
        .any(|cue| contains_cue(&lower, cue))
    {
        return ClauseEffect::Ignore;
    }

    let has_commit_cue = rules
        .commitment_cues
        .iter()
        .any(|cue| contains_cue(&lower, cue));
    if !has_commit_cue {
        return ClauseEffect::Ignore;
    }

    let mut has_sell = false;
    let mut has_buy = false;
    let mut price_cue_positions = Vec::new();
    for (idx, tok) in tokens.iter().enumerate() {
        let t = tok.as_str();
        if rules.sell_tokens.contains(&t) {
            has_sell = true;
        }
        if rules.buy_tokens.contains(&t) {
            has_buy = true;
        }
        if rules.price_cues.contains(&t) {
            price_cue_positions.push(idx);
        }
    }
    let side = match (has_sell, has_buy) {
        (true, false) => OfferSide::Sell,
        (false, true) => OfferSide::Buy,
        _ => return ClauseEffect::Ignore,
    };

    if price_cue_positions.is_empty()
        || (rules.exactly_one_price_cue && price_cue_positions.len() != 1)
    {
        return ClauseEffect::Ignore;
    }
    let price_idx = price_cue_positions[0];

    let goods = parse_good_span(&tokens, rules, seat_guilds);
    if goods.is_empty() || (rules.exactly_one_good_before_price && goods.len() != 1) {
        return ClauseEffect::Ignore;
    }
    let (good, good_start, good_end) = goods[0];
    if good_end > price_idx {
        return ClauseEffect::Ignore;
    }

    let mut amounts = Vec::new();
    for (idx, tok) in tokens[..price_idx].iter().enumerate() {
        if idx >= good_start && idx < good_end {
            continue;
        }
        match parse_pos_scaled_units(tok, rules.number_unit_scale) {
            ParsedScaledNum::Valid(val) => amounts.push(val),
            ParsedScaledNum::InvalidOrOverflow => return ClauseEffect::Ignore,
            ParsedScaledNum::NotNumeric => {}
        }
    }
    if amounts.is_empty() || (rules.exactly_one_amount_before_price && amounts.len() != 1) {
        return ClauseEffect::Ignore;
    }
    let amount = amounts[0];

    let mut prices = Vec::new();
    for tok in &tokens[(price_idx + 1)..] {
        if rules.cred_tokens.contains(&tok.as_str()) {
            continue;
        }
        match parse_pos_scaled_units(tok, rules.number_unit_scale) {
            ParsedScaledNum::Valid(val) => prices.push(val),
            ParsedScaledNum::InvalidOrOverflow | ParsedScaledNum::NotNumeric => {
                return ClauseEffect::Ignore;
            }
        }
    }
    if prices.is_empty() || (rules.exactly_one_price_after_cue && prices.len() != 1) {
        return ClauseEffect::Ignore;
    }
    let price_per_unit_cred = prices[0];

    ClauseEffect::Offer(OfferProposal {
        side,
        good,
        amount,
        price_per_unit_cred,
        domain_id: None,
        is_hybrid: false,
    })
}

fn process_source_messages(
    focal_seat: u8,
    seat_guilds: &[String],
    messages: &[(u8, u8, String)],
    rules: &TalkRulesV2Table,
) -> BTreeMap<(u8, u8), SourcePairOutcome> {
    let mut by_pair: BTreeMap<(u8, u8), SourcePairOutcome> = BTreeMap::new();
    let seat_count = seat_guilds.len();
    for (proposer_seat, counterparty_seat, body) in messages {
        if *proposer_seat == focal_seat
            || usize::from(*proposer_seat) >= seat_count
            || *counterparty_seat != focal_seat
        {
            continue;
        }
        let pair = by_pair
            .entry((*proposer_seat, *counterparty_seat))
            .or_default();
        for clause in split_clauses(body, rules) {
            match interpret_clause(clause, rules, seat_guilds) {
                ClauseEffect::Ignore => {}
                ClauseEffect::Retract(Some(good)) => {
                    pair.assets.insert(good, SourceAssetOutcome::Retracted);
                }
                ClauseEffect::Retract(None) => {
                    pair.retracted_all = true;
                    for val in pair.assets.values_mut() {
                        *val = SourceAssetOutcome::Retracted;
                    }
                }
                ClauseEffect::Offer(proposal) => {
                    let asset = proposal.good;
                    match pair.assets.get_mut(&asset) {
                        None => {
                            pair.assets
                                .insert(asset, SourceAssetOutcome::Active(proposal));
                        }
                        Some(SourceAssetOutcome::Retracted) => {
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
    by_pair
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
    let mut all_assets = BTreeMap::<AssetId, ()>::new();
    for &g in neg.assets.keys() {
        all_assets.insert(g, ());
    }
    for &g in talk.assets.keys() {
        all_assets.insert(g, ());
    }

    let mut merged = SourcePairOutcome {
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

/// Pure deterministic talk commitment extraction over ordered `(proposer_seat, counterparty_seat, body)`
/// messages (`talk_rules_v2`, `§4.3`).
pub fn extract_talk_commitments_from_messages(
    focal_seat: u8,
    seat_guilds: &[String],
    messages: &[(u8, u8, String)],
    obs: &SeatObservation,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
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

    let pair_map = process_source_messages(focal_seat, seat_guilds, messages, &rules);
    finalize_pair_assets(pair_map, focal_seat, seat_count)
}

/// Extracts frozen deterministic `talk_rules_v2` commitments for `day` from
/// `<source_game_dir>/to-seat/day-DD-act-<Guild>.negotiation.json` and
/// `<source_game_dir>/decisions/day-DD-talk-<Guild>.json` (`§4.3`, `§8.1`).
///
/// Because neither file carries cross-source message timestamps, each source is processed
/// independently and merged per `(proposer, counterparty, asset)` with retraction dominance so a
/// retracted commitment in one source is never resurrected by a stale copy in the other.
pub fn extract_talk_commitments_for_state(
    source_game_dir: &Path,
    day: u8,
    focal_seat: u8,
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
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

    let neg_outcomes = process_source_messages(focal_seat, seat_guilds, &neg_msgs, &rules);
    let talk_outcomes = process_source_messages(focal_seat, seat_guilds, &talk_msgs, &rules);

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

    finalize_pair_assets(merged, focal_seat, seat_count)
}
