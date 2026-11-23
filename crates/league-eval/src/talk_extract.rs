//! Frozen deterministic talk-phase trade commitment extractor (`§4.3`, `§5.2`).
//!
//! Reads `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs::whispers`) and
//! `decisions/day-DD-talk-<Guild>.json` (`parse_eval_talk_reply`) under `TALK_EXTRACTOR_VERSION_V1`
//! (`"talk_rules_v1"`) and extracts unambiguous `(proposer_seat, counterparty_seat, offered_action)`
//! spot-offer commitments, dropping any commitment retracted or contradicted later in talk.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use league_core::{AssetId, MarketAction, OfferProposal, OfferSide, SeatObservation};
use league_relay::NegotiationInputs;
use league_scorer::{
    ExtractedTalkCommitment, TALK_EXTRACTOR_VERSION_V1, canonicalize_talk_commitments, sha256_hex,
    to_sorted_compact_json, validate_talk_commitment,
};
use serde::Serialize;

use crate::error::EvalError;
use crate::records::{EvalDecisionRecord, EvalManifest};
use crate::reply::parse_eval_talk_reply;

/// Canonical frozen rule table for `talk_rules_v1` (`§4.3`, `§5.2`).
///
/// Field names appear in strict alphabetical order so compact JSON serialization and
/// `talk_extractor_rules_digest` remain deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TalkRulesV1Table {
    /// Cues indicating uncertainty, hedging, or questions (discard clause).
    pub ambiguity_cues: &'static [&'static str],
    /// Tokens designating a buy offer (`OfferSide::Buy`).
    pub buy_tokens: &'static [&'static str],
    /// Phrases/tokens signaling an explicit commitment to post an offer.
    pub commitment_cues: &'static [&'static str],
    /// Optional unit suffixes following price figures (`CRED`).
    pub cred_tokens: &'static [&'static str],
    /// Prepositions/symbols separating order amount/good from limit unit price.
    pub price_cues: &'static [&'static str],
    /// Prefixes identifying `AssetId::Raw(i)`.
    pub raw_prefixes: &'static [&'static str],
    /// Prefixes identifying `AssetId::Refined(i)`.
    pub refined_prefixes: &'static [&'static str],
    /// Cues canceling or retracting an earlier commitment in the same pair stream.
    pub retraction_cues: &'static [&'static str],
    /// Tokens designating a sell offer (`OfferSide::Sell`).
    pub sell_tokens: &'static [&'static str],
    /// Conditional or out-of-scope settlement cues (discard clause rather than guessing).
    pub unsupported_condition_cues: &'static [&'static str],
    /// Version identifier pinned in `ScorerConfig::talk_extractor_version`.
    pub version: &'static str,
}

impl TalkRulesV1Table {
    /// Returns the immutable frozen rule table for `talk_rules_v1`.
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
            cred_tokens: &["cred", "creds"],
            price_cues: &["@", "at", "for"],
            raw_prefixes: &["raw"],
            refined_prefixes: &["refined", "ref"],
            retraction_cues: &[
                "cancel",
                "retract",
                "withdraw",
                "never mind",
                "nevermind",
                "call off",
                "no deal",
                "rescind",
                "walk back",
                "won't",
                "will not",
                "cannot",
                "can't",
                "don't",
                "do not",
            ],
            sell_tokens: &["sell", "selling", "ask"],
            unsupported_condition_cues: &[
                "if", "unless", "when", "until", "hybrid", "domain", "swap", "check", "escrow",
                "pledge",
            ],
            version: TALK_EXTRACTOR_VERSION_V1,
        }
    }
}

/// Computes the canonical lowercase 64-hex SHA-256 digest of [`TalkRulesV1Table::frozen`].
pub fn talk_rules_v1_digest() -> Result<String, EvalError> {
    let bytes = to_sorted_compact_json(&TalkRulesV1Table::frozen())
        .map_err(|e| EvalError::SchemaValidation(format!("serialize talk_rules_v1 table: {e}")))?;
    Ok(sha256_hex(&bytes))
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
enum PairAssetState {
    Active(OfferProposal),
    Contradicted,
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    byte_start: usize,
    byte_end: usize,
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

fn split_clauses(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut start = 0_usize;
    let mut i = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        let is_break = match b {
            b'\n' | b'\r' | b';' | b'!' | b'?' => true,
            b'.' => {
                let next_is_digit = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
                !next_is_digit
            }
            _ => false,
        };
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

fn tokenize_clause(lower: &str) -> Vec<Token> {
    let bytes = lower.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'@' {
            tokens.push(Token {
                text: "@".to_owned(),
                byte_start: i,
                byte_end: i + 1,
            });
            i += 1;
            continue;
        }
        let is_tok = b.is_ascii_alphanumeric() || b == b'_' || b == b'\'' || b == b'.';
        if !is_tok {
            i += 1;
            continue;
        }
        let start = i;
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
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'\'' || c == b':' || c == b'.' {
                buf.push(char::from(c));
                i += 1;
            } else {
                break;
            }
        }
        let trimmed = buf.trim_matches(|ch| ch == '.' || ch == ':' || ch == '_');
        if !trimmed.is_empty() {
            tokens.push(Token {
                text: trimmed.to_owned(),
                byte_start: start,
                byte_end: i,
            });
        }
    }
    tokens
}

fn parse_pos_int(tok: &str) -> Option<i64> {
    let cleaned = tok.replace('_', "");
    if cleaned.is_empty() || !cleaned.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let val: i64 = cleaned.parse().ok()?;
    (val > 0).then_some(val)
}

fn resolve_seat_ref(tok: &str, seat_guilds: &[String]) -> Option<u8> {
    let stripped = tok.trim_matches(|c| c == ':' || c == '_');
    if stripped.bytes().all(|b| b.is_ascii_digit())
        && !stripped.is_empty()
        && let Ok(idx) = stripped.parse::<usize>()
        && idx < seat_guilds.len()
    {
        return u8::try_from(idx).ok();
    }
    for (idx, guild) in seat_guilds.iter().enumerate() {
        if stripped == guild.to_ascii_lowercase() {
            return u8::try_from(idx).ok();
        }
    }
    None
}

fn parse_good_span(
    tokens: &[Token],
    rules: &TalkRulesV1Table,
    seat_guilds: &[String],
) -> Vec<(AssetId, usize, usize)> {
    let mut matches = Vec::new();
    let mut i = 0_usize;
    while i < tokens.len() {
        let t = tokens[i].text.as_str();
        // Compound forms like `raw0`, `raw:1`, `refined_0`, `ref:foundry`.
        let mut compound_hit = None;
        for &pref in rules.raw_prefixes {
            if let Some(rest) = t.strip_prefix(pref)
                && !rest.is_empty()
                && let Some(seat) = resolve_seat_ref(rest, seat_guilds)
            {
                compound_hit = Some(AssetId::Raw(seat));
                break;
            }
        }
        if compound_hit.is_none() {
            for &pref in rules.refined_prefixes {
                if let Some(rest) = t.strip_prefix(pref)
                    && !rest.is_empty()
                    && let Some(seat) = resolve_seat_ref(rest, seat_guilds)
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

        // Two-token forms: `raw <seat|guild>`, `refined <seat|guild>`, `<guild> raw`, `<guild> refined`.
        if i + 1 < tokens.len() {
            let next = tokens[i + 1].text.as_str();
            if rules.raw_prefixes.contains(&t)
                && let Some(seat) = resolve_seat_ref(next, seat_guilds)
            {
                matches.push((AssetId::Raw(seat), i, i + 2));
                i += 2;
                continue;
            }
            if rules.refined_prefixes.contains(&t)
                && let Some(seat) = resolve_seat_ref(next, seat_guilds)
            {
                matches.push((AssetId::Refined(seat), i, i + 2));
                i += 2;
                continue;
            }
            if let Some(seat) = resolve_seat_ref(t, seat_guilds) {
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
    rules: &TalkRulesV1Table,
    seat_guilds: &[String],
) -> ClauseEffect {
    let lower = clause.to_ascii_lowercase();
    let tokens = tokenize_clause(&lower);
    if tokens.is_empty() {
        return ClauseEffect::Ignore;
    }

    if rules
        .retraction_cues
        .iter()
        .any(|cue| contains_cue(&lower, cue))
    {
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
        let t = tok.text.as_str();
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

    if price_cue_positions.len() != 1 {
        return ClauseEffect::Ignore;
    }
    let price_idx = price_cue_positions[0];

    let goods = parse_good_span(&tokens, rules, seat_guilds);
    if goods.len() != 1 {
        return ClauseEffect::Ignore;
    }
    let (good, good_start, good_end) = goods[0];
    if good_end > price_idx {
        // Expect unambiguous order amount + good specification before the price cue.
        return ClauseEffect::Ignore;
    }

    let mut amounts = Vec::new();
    for (idx, tok) in tokens[..price_idx].iter().enumerate() {
        if idx >= good_start && idx < good_end {
            continue;
        }
        if let Some(val) = parse_pos_int(&tok.text) {
            amounts.push(val);
        }
    }
    if amounts.len() != 1 {
        return ClauseEffect::Ignore;
    }
    let amount = amounts[0];

    let mut prices = Vec::new();
    for tok in &tokens[(price_idx + 1)..] {
        if rules.cred_tokens.contains(&tok.text.as_str()) {
            continue;
        }
        if let Some(val) = parse_pos_int(&tok.text) {
            prices.push(val);
        } else {
            return ClauseEffect::Ignore;
        }
    }
    if prices.len() != 1 {
        return ClauseEffect::Ignore;
    }
    let price_per_unit_cred = prices[0];

    let _unused_span_guard = (&tokens[0].byte_start, &tokens[tokens.len() - 1].byte_end);

    ClauseEffect::Offer(OfferProposal {
        side,
        good,
        amount,
        price_per_unit_cred,
        domain_id: None,
        is_hybrid: false,
    })
}

/// Pure deterministic talk commitment extraction over ordered `(proposer_seat, counterparty_seat, body)`
/// messages (`talk_rules_v1`, `§4.3`).
pub fn extract_talk_commitments_from_messages(
    focal_seat: u8,
    seat_guilds: &[String],
    messages: &[(u8, u8, String)],
    obs: &SeatObservation,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
    let rules = TalkRulesV1Table::frozen();
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

    let mut pair_state: BTreeMap<(u8, u8, AssetId), PairAssetState> = BTreeMap::new();

    for (proposer_seat, counterparty_seat, body) in messages {
        if *proposer_seat == focal_seat
            || *proposer_seat == *counterparty_seat
            || *proposer_seat >= seat_count
            || *counterparty_seat >= seat_count
        {
            continue;
        }
        for clause in split_clauses(body) {
            match interpret_clause(clause, &rules, seat_guilds) {
                ClauseEffect::Ignore => {}
                ClauseEffect::Retract(Some(good)) => {
                    pair_state.remove(&(*proposer_seat, *counterparty_seat, good));
                }
                ClauseEffect::Retract(None) => {
                    pair_state.retain(|(p, cp, _), _| {
                        !(*p == *proposer_seat && *cp == *counterparty_seat)
                    });
                }
                ClauseEffect::Offer(proposal) => {
                    let key = (*proposer_seat, *counterparty_seat, proposal.good);
                    match pair_state.get_mut(&key) {
                        None => {
                            pair_state.insert(key, PairAssetState::Active(proposal));
                        }
                        Some(PairAssetState::Active(existing)) if *existing == proposal => {}
                        Some(state) => {
                            *state = PairAssetState::Contradicted;
                        }
                    }
                }
            }
        }
    }

    let mut raw_commitments = Vec::new();
    for ((proposer_seat, counterparty_seat, _), state) in pair_state {
        if let PairAssetState::Active(order) = state {
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

    canonicalize_talk_commitments(&raw_commitments)
        .map_err(|e| EvalError::SchemaValidation(format!("canonicalize talk commitments: {e}")))
}

/// Extracts frozen deterministic `talk_rules_v1` commitments for `day` from
/// `<source_game_dir>/to-seat/day-DD-act-<Guild>.negotiation.json` and
/// `<source_game_dir>/decisions/day-DD-talk-<Guild>.json` (`§4.3`, `§8.1`).
pub fn extract_talk_commitments_for_state(
    source_game_dir: &Path,
    day: u8,
    focal_seat: u8,
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<Vec<ExtractedTalkCommitment>, EvalError> {
    let day_str = format!("{day:02}");
    let guild_to_seat: BTreeMap<String, u8> = seat_guilds
        .iter()
        .enumerate()
        .filter_map(|(i, g)| u8::try_from(i).ok().map(|s| (g.clone(), s)))
        .collect();

    let mut neg_by_pair: BTreeMap<(u8, u8), Vec<String>> = BTreeMap::new();
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
                neg_by_pair
                    .entry((proposer_seat, counterparty_seat))
                    .or_default()
                    .push(whisper.body);
            }
        }
    }

    let mut talk_by_pair: BTreeMap<(u8, u8), Vec<String>> = BTreeMap::new();
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
                talk_by_pair
                    .entry((proposer_seat, counterparty_seat))
                    .or_default()
                    .push(whisper.body);
            }
        }
    }

    let mut all_pairs = BTreeMap::<(u8, u8), ()>::new();
    for &k in neg_by_pair.keys() {
        all_pairs.insert(k, ());
    }
    for &k in talk_by_pair.keys() {
        all_pairs.insert(k, ());
    }

    let mut messages = Vec::new();
    for (proposer_seat, counterparty_seat) in all_pairs.keys().copied() {
        let neg_msgs = neg_by_pair.get(&(proposer_seat, counterparty_seat));
        let talk_msgs = talk_by_pair.get(&(proposer_seat, counterparty_seat));
        match (neg_msgs, talk_msgs) {
            (Some(nm), Some(tm)) if nm == tm => {
                for body in nm {
                    messages.push((proposer_seat, counterparty_seat, body.clone()));
                }
            }
            (Some(nm), Some(tm)) => {
                for body in nm.iter().chain(tm.iter()) {
                    messages.push((proposer_seat, counterparty_seat, body.clone()));
                }
            }
            (Some(nm), None) => {
                for body in nm {
                    messages.push((proposer_seat, counterparty_seat, body.clone()));
                }
            }
            (None, Some(tm)) => {
                for body in tm {
                    messages.push((proposer_seat, counterparty_seat, body.clone()));
                }
            }
            (None, None) => {}
        }
    }

    extract_talk_commitments_from_messages(focal_seat, seat_guilds, &messages, obs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_core::{EvalHost, TableId};
    use league_scorer::TALK_EXTRACTOR_RULES_DIGEST_V1;

    fn sample_obs(focal: u8) -> Result<SeatObservation, Box<dyn std::error::Error>> {
        let host = EvalHost::from_standard_setup(TableId(97_101), 4, 1, 0)?;
        Ok(host.observe_seat(league_core::Seat(focal))?)
    }

    fn guilds() -> Vec<String> {
        vec![
            "Foundry".to_owned(),
            "Quarry".to_owned(),
            "Harbor".to_owned(),
            "Glassworks".to_owned(),
        ]
    }

    #[test]
    fn talk_rules_v1_digest_matches_pinned_scorer_constant()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(talk_rules_v1_digest()?, TALK_EXTRACTOR_RULES_DIGEST_V1);
        Ok(())
    }

    #[test]
    fn clear_ambiguous_retraction_and_contradiction_rules_are_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let obs = sample_obs(0)?;
        let g = guilds();

        // Clear unambiguous offer gives one commitment.
        let clear = extract_talk_commitments_from_messages(
            0,
            &g,
            &[(1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned())],
            &obs,
        )?;
        assert_eq!(clear.len(), 1);
        assert_eq!(clear[0].proposer_seat, 1);
        assert_eq!(clear[0].counterparty_seat, 0);
        assert_eq!(
            clear[0].offered_action,
            MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    amount: 1000,
                    price_per_unit_cred: 250,
                    domain_id: None,
                    is_hybrid: false,
                },
            }
        );

        // Two runs give byte-identical output.
        let clear_repeat = extract_talk_commitments_from_messages(
            0,
            &g,
            &[(1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned())],
            &obs,
        )?;
        assert_eq!(
            to_sorted_compact_json(&clear)?,
            to_sorted_compact_json(&clear_repeat)?
        );

        // Ambiguous phrasing yields zero commitments.
        for ambiguous in [
            "Maybe I will sell 1000 raw 1 at 250 CRED.",
            "Will sell 1000 raw 1 at 250 or 300 CRED.",
            "If you agree, I will sell 1000 raw 1 at 250 CRED?",
            "I offer 1000 raw 1 at 250 CRED.",
        ] {
            let out = extract_talk_commitments_from_messages(
                0,
                &g,
                &[(1, 0, ambiguous.to_owned())],
                &obs,
            )?;
            assert!(
                out.is_empty(),
                "expected empty for `{ambiguous}`, got {out:?}"
            );
        }

        // Later retraction removes the commitment.
        let retracted = extract_talk_commitments_from_messages(
            0,
            &g,
            &[
                (1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned()),
                (1, 0, "Actually, I retract my offer on raw 1.".to_owned()),
            ],
            &obs,
        )?;
        assert!(retracted.is_empty());

        // Later contradicting offer drops the commitment instead of picking one half.
        let contradicted = extract_talk_commitments_from_messages(
            0,
            &g,
            &[
                (1, 0, "I will sell 1000 raw 1 at 250 CRED.".to_owned()),
                (
                    1,
                    0,
                    "Correction: I will sell 500 raw 1 at 300 CRED.".to_owned(),
                ),
            ],
            &obs,
        )?;
        assert!(contradicted.is_empty());
        Ok(())
    }
}
