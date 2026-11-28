//! Wave 2 Card 2 acceptance & regression suite for `league-core`:
//! - Must item 1 & 2: `TableMode::Club`, `SeatObservation`, `PublicSweepReceipt`, `PublicPreviewState`,
//!   `SweepMode::{Live, PreviewSteps0To7}`, `TurnEngine::preview_regent_bundle_for_seat`,
//!   `preview_what_if*`, `preview_seat_bundle`, shock magnitude visibility from `realize_turn`, and
//!   `league_core::error::{StructuredError, ToStructuredError}`.
//! - Must item 3 (B-N2): Planted `check_seat_view(&view_json, &engine_state) -> Result<(), Leak>`
//!   redaction guard with four controls (clean full-season control + three planted single-leak
//!   controls: `newly_drawn_shocks` before forecast, shock magnitude before realize, and
//!   `post_state_digest`).
//! - §2.5 Steps 0–7 Equivalence `proptest` (all-committed and Regent-modeled human turns) and
//!   Secret Non-Interference + `Commit`/`Reveal` Isolation `proptest`.
//! - Must item 5: Versioned replay compatibility (`current` reads, `newer` refused by name via
//!   `EngineError::UnsupportedReplayVersion`, `pre-version` reads).
//! - Club vs Eval mutual refusal & `SeedPool::Dev` + reserved-seed rejection tests.

use std::collections::{BTreeMap, BTreeSet};

use league_core::{
    AiPreviewRequest, AssetId, AtomicSwapProposal, CharterBidOrder, CharterRole, CheckProposal,
    ContractDeliveryOrder, ConverterRunOrder, CounterpartyAssumptions, DEFAULT_SHOCK_DRAW_TURNS,
    DEV_LAYOUT_SEED_MAX, DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_SIGNATURE_BYTES,
    DrandSchedule, DualEscrowProposal, EngineError, EngineState, EscrowFinishUnlock,
    EscrowProposal, FillKind, HARD_TURN_CAP, HumanWhatIfRequest, IntentTarget, LoggedTurn,
    MarketAction, OfferProposal, OfferSide, OrderReveal, PREVIEW_NEUTRAL_BEACON_SEED,
    PreviewTieMode, ProjectDeliveryOrder, PublicCheckOrderPreview, PublicCheckOutcome,
    PublicSwapOrderPreview, PublicSwapOutcome, REPLAY_FORMAT_VERSION, RESERVED_TEST_LAYOUT_SEEDS,
    RefusedActionPreview, RegentDoctrine, ReplayBeaconMode, ReplayLayoutSeedError, ReplayOutcome,
    ReplayRecord, RevealSalt, ScheduledShockSpec, SealedEvalSeed, SealedOrderBundle,
    SeasonReplayLog, Seat, SeatIdentity, SeatObservation, ShockKind, StructuredError, SweepReceipt,
    TableId, TableMode, TableSpec, ToStructuredError, Turn, TurnEngine, TurnPhase, UNIT_SCALE,
    WhatIfPreview, attach_state_predicates, check_replay_format_version_slice,
    compute_mandate_commitment, compute_order_commitment, derive_turn_beacon, engine_internals,
    is_reserved_layout_test_seed, preview_neutral_beacon, preview_seat_bundle, preview_what_if,
    preview_what_if_with_beacon, replay_club_season, replay_eval_season, replay_season,
    replay_season_with_state_predicates, scrub_secret_text, validate_club_dev_seed,
};
use proptest::prelude::*;
use sha2::{Digest as _, Sha256};

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn must_err<T, E>(res: Result<T, E>, ctx: &str) -> E {
    match res {
        Ok(_) => panic!("{ctx}: expected error"),
        Err(err) => err,
    }
}

fn non_reserved_dev_seed(raw: u64) -> u64 {
    let mut candidate = raw % (DEV_LAYOUT_SEED_MAX + 1);
    while is_reserved_layout_test_seed(candidate) {
        candidate = (candidate + 1) % (DEV_LAYOUT_SEED_MAX + 1);
    }
    candidate
}

fn eval_seed_from(tag: u8) -> SealedEvalSeed {
    must_ok(
        SealedEvalSeed::try_from_bytes([tag.max(1); 32]),
        "eval_seed_from",
    )
}

fn seat_salts(n: u8, tag: u8) -> Vec<RevealSalt> {
    (0..n)
        .map(|s| RevealSalt([tag.max(1).wrapping_add(s).max(1); 32]))
        .collect()
}

fn commit_and_reveal_subset(
    engine: &mut TurnEngine,
    table_id: TableId,
    turn: u8,
    bundles_by_seat: &BTreeMap<u8, SealedOrderBundle>,
) -> (
    BTreeMap<u8, league_core::CommitmentHash>,
    BTreeMap<u8, OrderReveal>,
) {
    must_ok(engine.advance_to_commit(), "advance_to_commit");
    let mut commits = BTreeMap::new();
    let mut reveals = BTreeMap::new();
    for (&s, bundle) in bundles_by_seat {
        let rev = OrderReveal {
            bundle: bundle.clone(),
            salt: RevealSalt([0x31_u8.wrapping_add(s).wrapping_add(turn).max(1); 32]),
        };
        let commit = must_ok(
            compute_order_commitment(table_id, Turn(turn), Seat(s), &rev),
            "compute_order_commitment",
        );
        must_ok(
            engine.submit_order_commitment(Seat(s), commit),
            "submit_order_commitment",
        );
        commits.insert(s, commit);
        reveals.insert(s, rev);
    }
    must_ok(engine.advance_to_reveal(), "advance_to_reveal");
    for (&s, rev) in &reveals {
        must_ok(
            engine.submit_order_reveal(Seat(s), rev.clone()),
            "submit_order_reveal",
        );
    }
    (commits, reveals)
}

// =================================================================================================
// 1. B-N2 Planted `SeatObservation` Redaction Guard (`check_seat_view`) & Four Controls
// =================================================================================================

/// Shock record shape inferred purely from JSON scalar values (never inspecting field names or
/// searching for `"magnitude"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShockRecordType {
    /// Drawn (`ScheduledShockSpec`) or private `OracleAuditor` hint carrying all three stage turns.
    DrawnOrAuditorHint,
    /// Narrowed public `ShockForecastEvent`.
    Forecast,
    /// Realized `ActiveShock` observed before its `realize_turn` or in excess count.
    PrematureActiveShock,
}

/// Specific leak classified by [`check_seat_view`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leak {
    /// Unforecast shock data appeared in a seat view before `forecast_turn` completed.
    NewlyDrawnShockBeforeForecast {
        /// Affected seat index.
        target_seat: u8,
        /// Shock kind string/variant.
        kind: ShockKind,
        /// Turn at which the public forecast should first appear.
        forecast_turn: u8,
    },
    /// Shock `magnitude_bps` appeared on a drawn/forecast/hint record or before `realize_turn`.
    MagnitudeBeforeRealize {
        /// Classified record shape that exposed `magnitude_bps`.
        record_type: ShockRecordType,
        /// Affected seat index.
        target_seat: u8,
        /// Shock kind variant.
        kind: ShockKind,
        /// Exposed magnitude in basis points.
        magnitude_bps: u16,
    },
    /// Canonical `EngineState` post-sweep SHA-256 digest appeared in serialized seat view.
    PostStateDigest,
    /// Serialized view JSON was malformed or missing observer seat index.
    InvalidViewJson(String),
}

#[derive(Default)]
struct ObjectScalars {
    strings: Vec<String>,
    ints: Vec<u64>,
}

fn collect_object_nodes(value: &serde_json::Value, out: &mut Vec<ObjectScalars>) {
    match value {
        serde_json::Value::Object(map) => {
            let mut scalars = ObjectScalars::default();
            for child in map.values() {
                match child {
                    serde_json::Value::String(s) => scalars.strings.push(s.clone()),
                    serde_json::Value::Number(n) => {
                        if let Some(u) = n.as_u64() {
                            scalars.ints.push(u);
                        }
                    }
                    _ => {}
                }
            }
            out.push(scalars);
            for child in map.values() {
                collect_object_nodes(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_object_nodes(child, out);
            }
        }
        _ => {}
    }
}

fn canonical_state_digest(engine_state: &EngineState) -> Result<[u8; 32], Leak> {
    let encoded =
        serde_json::to_vec(engine_state).map_err(|err| Leak::InvalidViewJson(err.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/state/v2");
    hasher.update(encoded);
    Ok(hasher.finalize().into())
}

fn push_candidate_digest(
    out: &mut Vec<([u8; 32], String)>,
    state: &EngineState,
) -> Result<(), Leak> {
    let digest = canonical_state_digest(state)?;
    if out.iter().all(|(existing, _)| existing != &digest) {
        let digest_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        out.push((digest, digest_hex));
    }
    Ok(())
}

/// Computes canonical `EngineState` digests across every phase state a seat could compare against
/// during a turn (`B3`): the current state, turn-start `TalkAndMarket`, cancel-locked
/// Computes canonical `EngineState` digests across every phase state a seat could compare against
/// during a turn (`B3`, `B5`): the current state, pre-`TownImport` unwound states for current-turn
/// Phase-1 imports, turn-start `TalkAndMarket`, cancel-locked `TalkAndMarket`, `Commit` entry and
/// commitment-prefix states, `Reveal` entry and reveal-prefix states, and post-Sweep close phases
/// (`AwaitingMandateReveal`, `Completed`).
fn push_phase_digests_for_base(
    out: &mut Vec<([u8; 32], String)>,
    base_state: &EngineState,
) -> Result<(), Leak> {
    push_candidate_digest(out, base_state)?;

    // Turn-start `TalkAndMarket` and post-Pass-A cancel-locked `TalkAndMarket`.
    for cancel_locked in [false, true] {
        let mut s = base_state.clone();
        s.phase = TurnPhase::TalkAndMarket;
        s.cancel_locked = cancel_locked;
        s.turn_commits.clear();
        s.turn_reveals.clear();
        push_candidate_digest(out, &s)?;
    }

    // `Commit` entry and cumulative submission prefixes of `turn_commits`.
    {
        let mut s = base_state.clone();
        s.phase = TurnPhase::Commit;
        s.cancel_locked = false;
        s.turn_commits.clear();
        s.turn_reveals.clear();
        push_candidate_digest(out, &s)?;
        for (&seat_idx, &commit_hash) in &base_state.turn_commits {
            s.turn_commits.insert(seat_idx, commit_hash);
            push_candidate_digest(out, &s)?;
        }
    }

    // `Reveal` entry (both with empty and full `turn_commits`) and cumulative `turn_reveals`.
    for keep_commits in [true, false] {
        let mut s = base_state.clone();
        s.phase = TurnPhase::Reveal;
        s.cancel_locked = false;
        if !keep_commits {
            s.turn_commits.clear();
        }
        s.turn_reveals.clear();
        push_candidate_digest(out, &s)?;
        for (&seat_idx, rev) in &base_state.turn_reveals {
            s.turn_reveals.insert(seat_idx, rev.clone());
            push_candidate_digest(out, &s)?;
        }
    }

    // Post-Sweep close phases (`AwaitingMandateReveal` and `Completed`).
    {
        let n = base_state.seats.len();
        let mut s = base_state.clone();
        s.phase = TurnPhase::AwaitingMandateReveal;
        s.cancel_locked = false;
        s.turn_commits.clear();
        s.turn_reveals.clear();
        push_candidate_digest(out, &s)?;

        s.mandate_reveals = vec![None; n];
        push_candidate_digest(out, &s)?;

        s.phase = TurnPhase::Completed;
        s.mandate_reveals = base_state.mandate_reveals.clone();
        push_candidate_digest(out, &s)?;
    }

    Ok(())
}

fn candidate_phase_digests(engine_state: &EngineState) -> Result<Vec<([u8; 32], String)>, Leak> {
    let mut out = Vec::new();
    push_phase_digests_for_base(&mut out, engine_state)?;

    // Unwind current-turn `TownImport` fill-log entries stepwise so start-of-turn and mid-turn
    // pre-action `post_state_digest`s are also caught deterministically after market actions (`B5`).
    let mut unwound = engine_state.clone();
    while let Some(pos) = unwound
        .fill_log
        .iter()
        .rposition(|r| r.turn == engine_state.turn && r.kind == FillKind::TownImport)
    {
        let rec = unwound.fill_log.remove(pos);
        if let Some(seat_state) = unwound.seats.get_mut(usize::from(rec.to)) {
            seat_state.cred = seat_state.cred.saturating_add(rec.cred_amount);
            match rec.good {
                Some(AssetId::Raw(g)) => {
                    if let Some(slot) = seat_state.raw.get_mut(usize::from(g)) {
                        *slot = slot.saturating_sub(rec.net_units);
                    }
                }
                Some(AssetId::Refined(m)) => {
                    if let Some(slot) = seat_state.refined.get_mut(usize::from(m)) {
                        *slot = slot.saturating_sub(rec.net_units);
                    }
                }
                _ => {}
            }
        }
        unwound.pending_town_import_cred_spent = unwound
            .pending_town_import_cred_spent
            .saturating_sub(rec.cred_amount);
        push_phase_digests_for_base(&mut out, &unwound)?;
    }

    Ok(out)
}

fn allowed_public_32byte_values(engine_state: &EngineState) -> BTreeSet<[u8; 32]> {
    let mut allowed = BTreeSet::new();
    if let Some(pk) = engine_state.spec.eval_vrf_pubkey {
        allowed.insert(pk);
    }
    for escrow in engine_state.escrows.values() {
        if let Some(hash) = escrow.condition_hash {
            allowed.insert(hash.0);
        }
    }
    for pledge in engine_state.pledges.values() {
        allowed.insert(pledge.memo_digest);
        if let IntentTarget::RevealPreimage { condition, .. } = pledge.target {
            allowed.insert(condition.0);
        }
    }
    allowed
}

fn decode_hex_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (idx, chunk) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        out[idx] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

fn contains_unlisted_32byte_digest(
    value: &serde_json::Value,
    allowed: &BTreeSet<[u8; 32]>,
) -> bool {
    match value {
        serde_json::Value::String(s) => {
            decode_hex_32(s).is_some_and(|digest| !allowed.contains(&digest))
        }
        serde_json::Value::Array(items) => {
            if items.len() == 32
                && let Some(bytes_vec) = items
                    .iter()
                    .map(|v| v.as_u64().and_then(|u| u8::try_from(u).ok()))
                    .collect::<Option<Vec<u8>>>()
            {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes_vec);
                let strictly_increasing = items.windows(2).all(|w| w[0].as_u64() < w[1].as_u64());
                if !strictly_increasing && !allowed.contains(&arr) {
                    return true;
                }
            }
            items
                .iter()
                .any(|child| contains_unlisted_32byte_digest(child, allowed))
        }
        serde_json::Value::Object(map) => map
            .values()
            .any(|child| contains_unlisted_32byte_digest(child, allowed)),
        _ => false,
    }
}

fn contains_digest_value(value: &serde_json::Value, digest: &[u8; 32], digest_hex: &str) -> bool {
    match value {
        serde_json::Value::String(s) => s.eq_ignore_ascii_case(digest_hex),
        serde_json::Value::Array(items) => {
            if items.len() == 32
                && items
                    .iter()
                    .zip(digest.iter())
                    .all(|(v, &b)| v.as_u64() == Some(u64::from(b)))
            {
                return true;
            }
            items
                .iter()
                .any(|child| contains_digest_value(child, digest, digest_hex))
        }
        serde_json::Value::Object(map) => map
            .values()
            .any(|child| contains_digest_value(child, digest, digest_hex)),
        _ => false,
    }
}

/// Pure redaction guard verifying a serialized seat view against ground-truth `engine_state`
/// (`B-N2`, `B3`, `B5`).
pub fn check_seat_view(view_json: &str, engine_state: &EngineState) -> Result<(), Leak> {
    check_seat_view_with_history(view_json, engine_state, &[])
}

/// Pure redaction guard verifying a serialized seat view against ground-truth `engine_state` and
/// optional prior turn/phase digests (`B-N2`, `B3`, `B5`).
pub fn check_seat_view_with_history(
    view_json: &str,
    engine_state: &EngineState,
    historical_post_state_digests: &[[u8; 32]],
) -> Result<(), Leak> {
    let root: serde_json::Value =
        serde_json::from_str(view_json).map_err(|err| Leak::InvalidViewJson(err.to_string()))?;
    let observer_seat = root
        .get("seat")
        .and_then(serde_json::Value::as_u64)
        .and_then(|s| u8::try_from(s).ok())
        .ok_or_else(|| Leak::InvalidViewJson("missing u8 observer seat".to_owned()))?;

    // Check (iii) across recorded historical digests, phase/market-action-unwound candidate
    // digests, AND any 32-byte/64-hex value outside public `eval_vrf_pubkey`/escrow/pledge hashes.
    for digest in historical_post_state_digests {
        let digest_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if contains_digest_value(&root, digest, &digest_hex) {
            return Err(Leak::PostStateDigest);
        }
    }
    for (digest, digest_hex) in candidate_phase_digests(engine_state)? {
        if contains_digest_value(&root, &digest, &digest_hex) {
            return Err(Leak::PostStateDigest);
        }
    }
    let allowed_32 = allowed_public_32byte_values(engine_state);
    if contains_unlisted_32byte_digest(&root, &allowed_32) {
        return Err(Leak::PostStateDigest);
    }

    let is_active_turn = matches!(
        engine_state.phase,
        TurnPhase::TalkAndMarket | TurnPhase::Commit | TurnPhase::Reveal
    );
    let last_swept_turn = if is_active_turn {
        engine_state.turn.saturating_sub(1)
    } else {
        engine_state.turn
    };
    let has_active_auditor = engine_state.charters.iter().any(|c| {
        c.role == CharterRole::OracleAuditor
            && c.holder == observer_seat
            && c.expires_at_turn >= engine_state.turn
    });

    let mut nodes = Vec::new();
    collect_object_nodes(&root, &mut nodes);

    let shocks: Vec<&ScheduledShockSpec> = engine_state
        .spec
        .shocks
        .iter()
        .chain(engine_state.drawn_shocks.iter())
        .collect();

    // Pass 1 — Check (i): unforecast `newly_drawn_shocks` before `forecast_turn` completes.
    for shock in &shocks {
        if last_swept_turn >= shock.forecast_turn {
            continue;
        }
        let kind_str = serde_json::to_value(shock.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| Leak::InvalidViewJson("non-string ShockKind".to_owned()))?;
        let target_u64 = u64::from(shock.target_seat);
        let hint_u64 = u64::from(shock.hint_turn);
        let forecast_u64 = u64::from(shock.forecast_turn);
        let realize_u64 = u64::from(shock.realize_turn);
        let duration = match shock.kind {
            ShockKind::DemandSurge => 2_u8,
            ShockKind::SupplyFreeze | ShockKind::TariffHoliday | ShockKind::SpoilageSpike => 1_u8,
        };
        let expires_u64 = u64::from(shock.realize_turn + duration - 1);

        let mut pre_forecast_hits = 0_usize;
        for node in &nodes {
            let matches_turn_stage = node.ints.contains(&hint_u64)
                || node.ints.contains(&forecast_u64)
                || node.ints.contains(&realize_u64)
                || node.ints.contains(&expires_u64);
            let matches_identity = node.strings.iter().any(|s| s == &kind_str)
                && node.ints.contains(&target_u64)
                && matches_turn_stage;
            if matches_identity {
                pre_forecast_hits += 1;
            }
        }
        let allowed_pre_forecast = usize::from(
            is_active_turn
                && has_active_auditor
                && shock.hint_turn <= engine_state.turn
                && engine_state.turn < shock.forecast_turn,
        );
        if pre_forecast_hits > allowed_pre_forecast {
            return Err(Leak::NewlyDrawnShockBeforeForecast {
                target_seat: shock.target_seat,
                kind: shock.kind,
                forecast_turn: shock.forecast_turn,
            });
        }
    }

    // Pass 2 — Check (ii): `magnitude_bps` inspected purely via shock value tuples & record shape.
    for shock in &shocks {
        let kind_str = serde_json::to_value(shock.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| Leak::InvalidViewJson("non-string ShockKind".to_owned()))?;
        let target_u64 = u64::from(shock.target_seat);
        let hint_u64 = u64::from(shock.hint_turn);
        let forecast_u64 = u64::from(shock.forecast_turn);
        let realize_u64 = u64::from(shock.realize_turn);
        let mag_u64 = u64::from(shock.magnitude_bps);
        let duration = match shock.kind {
            ShockKind::DemandSurge => 2_u8,
            ShockKind::SupplyFreeze | ShockKind::TariffHoliday | ShockKind::SpoilageSpike => 1_u8,
        };
        let expires_u64 = u64::from(shock.realize_turn + duration - 1);

        let realized_now = if is_active_turn {
            engine_state.turn >= shock.realize_turn
        } else {
            last_swept_turn >= shock.realize_turn
        };
        let active_in_live_state = usize::from(
            is_active_turn
                && engine_state.turn >= shock.realize_turn
                && engine_state.turn < shock.realize_turn + duration,
        );
        let active_in_past_receipt = usize::from(last_swept_turn >= shock.realize_turn);
        let allowed_active_records = active_in_live_state + active_in_past_receipt;

        let mut realized_mag_hits = 0_usize;

        for node in &nodes {
            let matches_turn_stage = node.ints.contains(&hint_u64)
                || node.ints.contains(&forecast_u64)
                || node.ints.contains(&realize_u64)
                || node.ints.contains(&expires_u64);
            let matches_identity = node.strings.iter().any(|s| s == &kind_str)
                && node.ints.contains(&target_u64)
                && matches_turn_stage;
            if !matches_identity {
                continue;
            }

            if node.ints.contains(&mag_u64) {
                let is_drawn_or_hint_shape = node.ints.contains(&hint_u64)
                    && node.ints.contains(&forecast_u64)
                    && node.ints.contains(&realize_u64);
                if is_drawn_or_hint_shape {
                    return Err(Leak::MagnitudeBeforeRealize {
                        record_type: ShockRecordType::DrawnOrAuditorHint,
                        target_seat: shock.target_seat,
                        kind: shock.kind,
                        magnitude_bps: shock.magnitude_bps,
                    });
                }

                if !realized_now {
                    let record_type = if node.ints.contains(&realize_u64) {
                        ShockRecordType::Forecast
                    } else {
                        ShockRecordType::PrematureActiveShock
                    };
                    return Err(Leak::MagnitudeBeforeRealize {
                        record_type,
                        target_seat: shock.target_seat,
                        kind: shock.kind,
                        magnitude_bps: shock.magnitude_bps,
                    });
                }

                if expires_u64 != realize_u64
                    && node.ints.contains(&realize_u64)
                    && !node.ints.contains(&expires_u64)
                {
                    return Err(Leak::MagnitudeBeforeRealize {
                        record_type: ShockRecordType::Forecast,
                        target_seat: shock.target_seat,
                        kind: shock.kind,
                        magnitude_bps: shock.magnitude_bps,
                    });
                }

                realized_mag_hits += 1;
                if realized_mag_hits > allowed_active_records {
                    return Err(Leak::MagnitudeBeforeRealize {
                        record_type: ShockRecordType::Forecast,
                        target_seat: shock.target_seat,
                        kind: shock.kind,
                        magnitude_bps: shock.magnitude_bps,
                    });
                }
            }
        }
    }

    Ok(())
}

// --- Test-only leaky view wrappers for B-N2 negative controls (never present in non-test code) ---

#[derive(serde::Serialize)]
struct LeakedShockForecastWithMag {
    target_seat: u8,
    kind: ShockKind,
    realize_turn: u8,
    magnitude_bps: u16,
}

#[derive(serde::Serialize)]
struct LeakedAuditorHintWithMag {
    hint_turn: u8,
    forecast_turn: u8,
    realize_turn: u8,
    target_seat: u8,
    kind: ShockKind,
    magnitude_bps: u16,
}

#[derive(serde::Serialize)]
struct LeakedReceiptWithDrawnAndDigest {
    turn: u8,
    newly_drawn_shocks: Vec<ScheduledShockSpec>,
    post_state_digest: Option<[u8; 32]>,
}

#[derive(serde::Serialize)]
struct LeakySeatView {
    seat: u8,
    observation: SeatObservation,
    leaked_receipts: Vec<LeakedReceiptWithDrawnAndDigest>,
    leaked_forecasts: Vec<LeakedShockForecastWithMag>,
    leaked_auditor_hints: Vec<LeakedAuditorHintWithMag>,
}

fn observe_seat_clean_json(engine: &TurnEngine, seat: Seat) -> String {
    let obs = must_ok(engine.observe_seat(seat), "observe_seat");
    must_ok(serde_json::to_string(&obs), "serialize SeatObservation")
}

fn observe_seat_leak_newly_drawn_json(
    engine: &TurnEngine,
    seat: Seat,
    raw_receipts: &[SweepReceipt],
) -> String {
    let obs = must_ok(engine.observe_seat(seat), "observe_seat");
    let leaked_receipts = raw_receipts
        .iter()
        .map(|r| LeakedReceiptWithDrawnAndDigest {
            turn: r.turn,
            newly_drawn_shocks: r.newly_drawn_shocks.clone(),
            post_state_digest: None,
        })
        .collect();
    must_ok(
        serde_json::to_string(&LeakySeatView {
            seat: seat.0,
            observation: obs,
            leaked_receipts,
            leaked_forecasts: Vec::new(),
            leaked_auditor_hints: Vec::new(),
        }),
        "serialize leaky view 1",
    )
}

fn observe_seat_leak_magnitude_before_realize_json(engine: &TurnEngine, seat: Seat) -> String {
    let mut obs = must_ok(engine.observe_seat(seat), "observe_seat");
    let state = engine.state();
    let is_active_turn = matches!(
        state.phase,
        TurnPhase::TalkAndMarket | TurnPhase::Commit | TurnPhase::Reveal
    );
    let last_swept_turn = if is_active_turn {
        state.turn.saturating_sub(1)
    } else {
        state.turn
    };
    let leaked_forecasts = state
        .spec
        .shocks
        .iter()
        .chain(state.drawn_shocks.iter())
        .filter(|s| s.forecast_turn <= last_swept_turn && state.turn <= s.realize_turn)
        .map(|s| LeakedShockForecastWithMag {
            target_seat: s.target_seat,
            kind: s.kind,
            realize_turn: s.realize_turn,
            magnitude_bps: s.magnitude_bps,
        })
        .collect();
    let leaked_auditor_hints = if !obs.auditor_hints.is_empty() {
        // Replace the clean hint slot rather than duplicating it so only leak #2 is injected.
        obs.auditor_hints.clear();
        state
            .spec
            .shocks
            .iter()
            .chain(state.drawn_shocks.iter())
            .filter(|s| s.hint_turn <= state.turn && state.turn < s.forecast_turn)
            .map(|s| LeakedAuditorHintWithMag {
                hint_turn: s.hint_turn,
                forecast_turn: s.forecast_turn,
                realize_turn: s.realize_turn,
                target_seat: s.target_seat,
                kind: s.kind,
                magnitude_bps: s.magnitude_bps,
            })
            .collect()
    } else {
        Vec::new()
    };
    must_ok(
        serde_json::to_string(&LeakySeatView {
            seat: seat.0,
            observation: obs,
            leaked_receipts: Vec::new(),
            leaked_forecasts,
            leaked_auditor_hints,
        }),
        "serialize leaky view 2",
    )
}

fn observe_seat_leak_post_state_digest_json(
    engine: &TurnEngine,
    seat: Seat,
    raw_receipts: &[SweepReceipt],
) -> String {
    let obs = must_ok(engine.observe_seat(seat), "observe_seat");
    let leaked_receipts = raw_receipts
        .iter()
        .map(|r| LeakedReceiptWithDrawnAndDigest {
            turn: r.turn,
            newly_drawn_shocks: Vec::new(),
            post_state_digest: Some(r.post_state_digest),
        })
        .collect();
    must_ok(
        serde_json::to_string(&LeakySeatView {
            seat: seat.0,
            observation: obs,
            leaked_receipts,
            leaked_forecasts: Vec::new(),
            leaked_auditor_hints: Vec::new(),
        }),
        "serialize leaky view 3",
    )
}

struct GuardSeasonSummary {
    turns_played: u8,
    turn8_auditor_hints_count: usize,
    realized_shock_turns_seen: usize,
    leak_hits: Vec<(u8, u8, Leak)>,
}

fn run_planted_guard_season(
    mode: fn(&TurnEngine, Seat, &[SweepReceipt]) -> Result<(), Leak>,
) -> GuardSeasonSummary {
    let tid = TableId(202_611);
    let n = 4_u8;
    let seed = 1_u64;
    let eval_seed = eval_seed_from(0x5A);
    let (spec, deal, eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            tid,
            n,
            seed,
            [0x44; 32],
            &seat_salts(n, 0x11),
            eval_seed,
        ),
        "standard_club_chiral_ring_with_explicit_setup",
    );
    let mut engine = must_ok(TurnEngine::new(spec), "TurnEngine::new");
    for s in 0..n {
        let rev = &deal.seat_reveals[usize::from(s)];
        let mc = must_ok(
            compute_mandate_commitment(tid, Seat(s), rev),
            "compute_mandate_commitment",
        );
        must_ok(
            engine.submit_mandate_commitment(Seat(s), mc),
            "submit_mandate_commitment",
        );
    }

    let mut raw_receipts = Vec::new();
    let mut turn8_auditor_hints_count = 0_usize;
    let mut realized_shock_turns_seen = 0_usize;
    let mut leak_hits = Vec::new();
    let mut turns_played = 0_u8;

    for t in 1..=HARD_TURN_CAP {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        turns_played = t;

        for s in 0..n {
            if let Err(leak) = mode(&engine, Seat(s), &raw_receipts) {
                leak_hits.push((t, s, leak));
            }
            if t == 8 && s == 1 {
                let obs = must_ok(engine.observe_seat(Seat(s)), "observe turn 8 auditor");
                turn8_auditor_hints_count = obs.auditor_hints.len();
                assert!(
                    obs.auditor_hints
                        .iter()
                        .all(|h| h.hint_turn == 8 && h.forecast_turn == 9 && h.realize_turn == 10),
                    "Turn 8 Auditor hint must reference Turn 7 shock draw (8 -> 9 -> 10)"
                );
            }
            if !engine.state().active_shocks.is_empty() && s == 0 {
                let obs = must_ok(engine.observe_seat(Seat(0)), "observe active shock");
                assert_eq!(obs.active_shocks, engine.state().active_shocks);
                realized_shock_turns_seen += 1;
            }
        }

        let mut bundles = BTreeMap::new();
        for s in 0..n {
            let mut b = SealedOrderBundle::default();
            b.converter_runs.push(ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            });
            if t == 6 && s == 1 {
                b.charter_bids.push(CharterBidOrder {
                    role: CharterRole::OracleAuditor,
                    bid_cred: 500,
                });
            }
            bundles.insert(s, b);
        }
        commit_and_reveal_subset(&mut engine, tid, t, &bundles);
        let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove_turn");
        let rec = must_ok(engine.sweep_turn_with_eval_vrf(&proof), "sweep_turn");
        let closed = rec.season_closed;
        raw_receipts.push(rec);
        if closed {
            break;
        }
    }

    // Observe every seat after close (`AwaitingMandateReveal`) as well.
    for s in 0..n {
        if let Err(leak) = mode(&engine, Seat(s), &raw_receipts) {
            leak_hits.push((turns_played, s, leak));
        }
    }

    assert_eq!(
        engine.state().drawn_shocks.len(),
        DEFAULT_SHOCK_DRAW_TURNS.len(),
        "standard_chiral_ring season must draw all 4 Horizon Shocks on turns 2, 7, 12, 17"
    );
    assert!(
        turn8_auditor_hints_count >= 1,
        "Seat 1 must hold active OracleAuditor won on Turn 6 and observe Turn-8 hint of Turn-7 draw"
    );
    assert!(
        realized_shock_turns_seen >= 4,
        "All 4 realized shocks must become active during season so post-realize magnitude passes"
    );

    GuardSeasonSummary {
        turns_played,
        turn8_auditor_hints_count,
        realized_shock_turns_seen,
        leak_hits,
    }
}

#[test]
fn planted_guard_season_clean_control_every_seat_every_turn_ok() {
    let summary = run_planted_guard_season(|eng, seat, _| {
        let json = observe_seat_clean_json(eng, seat);
        check_seat_view(&json, eng.state())
    });
    assert!(summary.turns_played >= 24);
    assert_eq!(summary.turn8_auditor_hints_count, 1);
    assert!(summary.realized_shock_turns_seen >= 4);
    assert!(
        summary.leak_hits.is_empty(),
        "clean SeatObservation must pass check_seat_view across every seat and turn: {:?}",
        summary.leak_hits
    );
}

#[test]
fn planted_guard_control_1_newly_drawn_shocks_before_forecast_err() {
    let summary = run_planted_guard_season(|eng, seat, receipts| {
        let json = observe_seat_leak_newly_drawn_json(eng, seat, receipts);
        check_seat_view(&json, eng.state())
    });
    assert!(
        !summary.leak_hits.is_empty(),
        "planted newly_drawn_shocks leak must be rejected"
    );
    assert!(summary.leak_hits.iter().any(|(t, _, leak)| {
        *t == 3
            && matches!(
                leak,
                Leak::NewlyDrawnShockBeforeForecast {
                    forecast_turn: 4,
                    ..
                }
            )
    }));
    assert!(summary.leak_hits.iter().any(|(t, s, leak)| {
        *t == 8
            && *s == 1
            && matches!(
                leak,
                Leak::NewlyDrawnShockBeforeForecast {
                    forecast_turn: 9,
                    ..
                }
            )
    }));
}

#[test]
fn planted_guard_control_2_magnitude_before_realize_err() {
    let summary = run_planted_guard_season(|eng, seat, _| {
        let json = observe_seat_leak_magnitude_before_realize_json(eng, seat);
        check_seat_view(&json, eng.state())
    });
    assert!(
        !summary.leak_hits.is_empty(),
        "planted magnitude before realize must be rejected"
    );
    assert!(summary.leak_hits.iter().any(|(t, _, leak)| {
        *t == 5
            && matches!(
                leak,
                Leak::MagnitudeBeforeRealize {
                    record_type: ShockRecordType::Forecast,
                    ..
                }
            )
    }));
    assert!(summary.leak_hits.iter().any(|(t, s, leak)| {
        *t == 8
            && *s == 1
            && matches!(
                leak,
                Leak::MagnitudeBeforeRealize {
                    record_type: ShockRecordType::DrawnOrAuditorHint,
                    ..
                }
            )
    }));

    // Also explicitly verify a pre-realize ShockForecastEvent (`turn = 3 < realize_turn = 5`) with
    // `magnitude_bps` attached is rejected before realize with `MagnitudeBeforeRealize::Forecast`.
    let tid = TableId(202_699);
    let eval_seed = eval_seed_from(0x5B);
    let (mut spec, _, eval_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            tid,
            4,
            1,
            [0x45; 32],
            &seat_salts(4, 0x12),
            eval_seed,
        ),
        "spec pre-realize forecast check",
    );
    spec.shocks = vec![ScheduledShockSpec {
        hint_turn: 1,
        forecast_turn: 2,
        realize_turn: 5,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_800,
    }];
    let mut eng = must_ok(TurnEngine::new(spec), "eng");
    for t in 1..=2_u8 {
        commit_and_reveal_subset(&mut eng, tid, t, &BTreeMap::new());
        let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
        must_ok(eng.sweep_turn_with_eval_vrf(&proof), "sweep");
    }
    let clean_t3 = observe_seat_clean_json(&eng, Seat(0));
    assert_eq!(check_seat_view(&clean_t3, eng.state()), Ok(()));
    let leaky_t3 = observe_seat_leak_magnitude_before_realize_json(&eng, Seat(0));
    assert!(matches!(
        check_seat_view(&leaky_t3, eng.state()),
        Err(Leak::MagnitudeBeforeRealize {
            record_type: ShockRecordType::Forecast,
            target_seat: 0,
            kind: ShockKind::SupplyFreeze,
            magnitude_bps: 2_800,
        })
    ));
}

#[test]
fn planted_guard_control_3_post_state_digest_err() {
    let summary = run_planted_guard_season(|eng, seat, receipts| {
        let json = observe_seat_leak_post_state_digest_json(eng, seat, receipts);
        check_seat_view(&json, eng.state())
    });
    assert!(
        !summary.leak_hits.is_empty(),
        "planted post_state_digest leak must be rejected"
    );
    assert!(
        summary
            .leak_hits
            .iter()
            .all(|(t, _, leak)| { *t >= 2 && matches!(leak, Leak::PostStateDigest) })
    );
}

#[test]
fn planted_guard_control_4_post_state_digest_in_commit_phase_err() {
    let tid = TableId(202_622);
    let n = 4_u8;
    let eval_seed = eval_seed_from(0x5C);
    let (spec, deal, eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            tid,
            n,
            1,
            [0x46; 32],
            &seat_salts(n, 0x13),
            eval_seed,
        ),
        "club spec",
    );
    let mut engine = must_ok(TurnEngine::new(spec), "TurnEngine::new");
    for s in 0..n {
        let rev = &deal.seat_reveals[usize::from(s)];
        let mc = must_ok(compute_mandate_commitment(tid, Seat(s), rev), "mc");
        must_ok(engine.submit_mandate_commitment(Seat(s), mc), "submit mc");
    }

    let mut raw_receipts = Vec::new();
    for t in 1..=2_u8 {
        commit_and_reveal_subset(&mut engine, tid, t, &BTreeMap::new());
        let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove_turn");
        raw_receipts.push(must_ok(engine.sweep_turn_with_eval_vrf(&proof), "sweep"));
    }

    // Turn 3: advance to `TurnPhase::Commit` — the leaked Turn-2 `post_state_digest` (whose phase
    // was TalkAndMarket at Turn 3 start) MUST return `Err(Leak::PostStateDigest)`!
    must_ok(engine.advance_to_commit(), "advance_to_commit");
    assert_eq!(engine.state().turn, 3);
    assert_eq!(engine.state().phase, TurnPhase::Commit);
    let commit_entry_digest = must_ok(engine.state_digest(), "commit_entry_digest");

    let clean_commit = observe_seat_clean_json(&engine, Seat(0));
    assert_eq!(check_seat_view(&clean_commit, engine.state()), Ok(()));

    let leaky_commit = observe_seat_leak_post_state_digest_json(&engine, Seat(0), &raw_receipts);
    assert_eq!(
        check_seat_view(&leaky_commit, engine.state()),
        Err(Leak::PostStateDigest),
        "leaked previous-sweep post_state_digest during Commit phase must be rejected"
    );

    // Also reject hex-encoded leaked `post_state_digest` during `Commit`.
    let r2_digest_hex: String = raw_receipts[1]
        .post_state_digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let hex_leak_commit = serde_json::json!({
        "seat": 0,
        "observation": must_ok(engine.observe_seat(Seat(0)), "obs"),
        "leaked_hex_digest": r2_digest_hex,
    })
    .to_string();
    assert_eq!(
        check_seat_view(&hex_leak_commit, engine.state()),
        Err(Leak::PostStateDigest)
    );

    // Submit a peer commitment in `Commit`: clean view passes; leaking prior receipt digest or
    // `Commit`-entry digest or post-peer-commit digest MUST all return `Err(Leak::PostStateDigest)`.
    let peer_rev = OrderReveal {
        bundle: SealedOrderBundle::default(),
        salt: RevealSalt([0x7A; 32]),
    };
    let peer_commit = must_ok(
        compute_order_commitment(tid, Turn(3), Seat(1), &peer_rev),
        "peer commit",
    );
    must_ok(
        engine.submit_order_commitment(Seat(1), peer_commit),
        "submit peer commit",
    );
    let clean_after_peer_commit = observe_seat_clean_json(&engine, Seat(0));
    assert_eq!(
        check_seat_view(&clean_after_peer_commit, engine.state()),
        Ok(())
    );
    let leaky_after_peer_commit =
        observe_seat_leak_post_state_digest_json(&engine, Seat(0), &raw_receipts);
    assert_eq!(
        check_seat_view(&leaky_after_peer_commit, engine.state()),
        Err(Leak::PostStateDigest)
    );
    let leaky_commit_entry = serde_json::json!({
        "seat": 0,
        "observation": must_ok(engine.observe_seat(Seat(0)), "obs"),
        "leaked_digest": commit_entry_digest,
    })
    .to_string();
    assert_eq!(
        check_seat_view(&leaky_commit_entry, engine.state()),
        Err(Leak::PostStateDigest)
    );

    // Advance to `TurnPhase::Reveal` and submit peer reveal: leaking `r2.post_state_digest` still fails.
    must_ok(engine.advance_to_reveal(), "advance_to_reveal");
    must_ok(engine.submit_order_reveal(Seat(1), peer_rev), "peer rev");
    let clean_reveal = observe_seat_clean_json(&engine, Seat(0));
    assert_eq!(check_seat_view(&clean_reveal, engine.state()), Ok(()));
    let leaky_reveal = observe_seat_leak_post_state_digest_json(&engine, Seat(0), &raw_receipts);
    assert_eq!(
        check_seat_view(&leaky_reveal, engine.state()),
        Err(Leak::PostStateDigest)
    );
}

#[test]
fn planted_guard_control_5_post_state_digest_after_market_action_err() {
    let tid = TableId(202_623);
    let n = 4_u8;
    let eval_seed = eval_seed_from(0x5D);
    let (spec, deal, eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            tid,
            n,
            1,
            [0x47; 32],
            &seat_salts(n, 0x14),
            eval_seed,
        ),
        "club spec",
    );
    let mut engine = must_ok(TurnEngine::new(spec), "TurnEngine::new");
    for s in 0..n {
        let rev = &deal.seat_reveals[usize::from(s)];
        let mc = must_ok(compute_mandate_commitment(tid, Seat(s), rev), "mc");
        must_ok(engine.submit_mandate_commitment(Seat(s), mc), "submit mc");
    }

    let mut raw_receipts = Vec::new();
    for t in 1..=2_u8 {
        commit_and_reveal_subset(&mut engine, tid, t, &BTreeMap::new());
        let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove_turn");
        raw_receipts.push(must_ok(engine.sweep_turn_with_eval_vrf(&proof), "sweep"));
    }

    // Turn 3: apply a `TownImport` market action and then a `PostOffer` market action before
    // observing (`TalkAndMarket`, and then `Commit`); Turn-2 `post_state_digest` must be rejected!
    must_ok(
        engine.apply_market_action(MarketAction::TownImport {
            seat: 1,
            good: AssetId::Raw(1),
            amount: 200,
        }),
        "town import",
    );
    let clean_after_import = observe_seat_clean_json(&engine, Seat(0));
    assert_eq!(check_seat_view(&clean_after_import, engine.state()), Ok(()));
    let leaky_after_import =
        observe_seat_leak_post_state_digest_json(&engine, Seat(0), &raw_receipts[1..]);
    assert_eq!(
        check_seat_view(&leaky_after_import, engine.state()),
        Err(Leak::PostStateDigest),
        "Turn-2 post_state_digest leaked after Turn-3 TownImport must fail check_seat_view"
    );

    let buy_good = AssetId::Raw(2);
    let bid_price = engine.state().parities.asset_parity(buy_good) + 100;
    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: buy_good,
                amount: 300,
                price_per_unit_cred: bid_price,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "post offer",
    );
    let clean_after_offer = observe_seat_clean_json(&engine, Seat(0));
    assert_eq!(check_seat_view(&clean_after_offer, engine.state()), Ok(()));
    let leaky_after_offer =
        observe_seat_leak_post_state_digest_json(&engine, Seat(0), &raw_receipts[1..]);
    assert_eq!(
        check_seat_view(&leaky_after_offer, engine.state()),
        Err(Leak::PostStateDigest),
        "Turn-2 post_state_digest leaked after Turn-3 PostOffer must fail check_seat_view"
    );

    must_ok(engine.advance_to_commit(), "advance_to_commit");
    let clean_commit_after_actions = observe_seat_clean_json(&engine, Seat(0));
    assert_eq!(
        check_seat_view(&clean_commit_after_actions, engine.state()),
        Ok(())
    );
    let t2_hex: String = raw_receipts[1]
        .post_state_digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let hex_leak_after_actions = serde_json::json!({
        "seat": 0,
        "observation": must_ok(engine.observe_seat(Seat(0)), "obs"),
        "leaked_hex_digest": t2_hex,
    })
    .to_string();
    assert_eq!(
        check_seat_view(&hex_leak_after_actions, engine.state()),
        Err(Leak::PostStateDigest),
        "Turn-2 hex digest leaked in Commit after Phase-1 actions must fail check_seat_view"
    );
}

#[test]
fn planted_guard_control_6_older_turn_post_state_digest_err() {
    let tid = TableId(202_624);
    let n = 4_u8;
    let eval_seed = eval_seed_from(0x5E);
    let (spec, deal, eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_explicit_setup(
            tid,
            n,
            1,
            [0x48; 32],
            &seat_salts(n, 0x15),
            eval_seed,
        ),
        "club spec",
    );
    let mut engine = must_ok(TurnEngine::new(spec), "TurnEngine::new");
    for s in 0..n {
        let rev = &deal.seat_reveals[usize::from(s)];
        let mc = must_ok(compute_mandate_commitment(tid, Seat(s), rev), "mc");
        must_ok(engine.submit_mandate_commitment(Seat(s), mc), "submit mc");
    }

    let mut raw_receipts = Vec::new();
    for t in 1..=4_u8 {
        commit_and_reveal_subset(&mut engine, tid, t, &BTreeMap::new());
        let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove_turn");
        raw_receipts.push(must_ok(engine.sweep_turn_with_eval_vrf(&proof), "sweep"));
    }

    // Turn 5: apply a market action so state diverges from all prior turns, then test leaking
    // Turn-1 (`raw_receipts[0]`) and Turn-2 (`raw_receipts[1]`) digests in both `[u8; 32]` and hex.
    must_ok(
        engine.apply_market_action(MarketAction::TownImport {
            seat: 2,
            good: AssetId::Raw(2),
            amount: 150,
        }),
        "turn 5 market action",
    );
    let clean_t5 = observe_seat_clean_json(&engine, Seat(0));
    let past_digests: Vec<[u8; 32]> = raw_receipts.iter().map(|r| r.post_state_digest).collect();
    assert_eq!(
        check_seat_view_with_history(&clean_t5, engine.state(), &past_digests),
        Ok(())
    );
    assert_eq!(check_seat_view(&clean_t5, engine.state()), Ok(()));

    for older_idx in [0_usize, 1_usize] {
        let older_slice = &raw_receipts[older_idx..=older_idx];
        let leaky_bytes = observe_seat_leak_post_state_digest_json(&engine, Seat(0), older_slice);
        assert_eq!(
            check_seat_view(&leaky_bytes, engine.state()),
            Err(Leak::PostStateDigest),
            "older turn {} byte-array digest must fail check_seat_view on Turn 5",
            older_idx + 1
        );
        assert_eq!(
            check_seat_view_with_history(&leaky_bytes, engine.state(), &past_digests),
            Err(Leak::PostStateDigest)
        );

        let older_hex: String = older_slice[0]
            .post_state_digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let leaky_hex = serde_json::json!({
            "seat": 0,
            "observation": must_ok(engine.observe_seat(Seat(0)), "obs"),
            "leaked_older_digest": older_hex,
        })
        .to_string();
        assert_eq!(
            check_seat_view(&leaky_hex, engine.state()),
            Err(Leak::PostStateDigest),
            "older turn {} hex digest must fail check_seat_view on Turn 5",
            older_idx + 1
        );
    }
}

// =================================================================================================
// 2. §2.5 Steps 0–7 Equivalence `proptest` (All-Committed and Regent-Modeled Turns)
// =================================================================================================

struct LiveStep0To7Ref<'a> {
    obs: &'a SeatObservation,
    expected_ref: &'a league_core::BundlePreview,
    expected_refused: &'a [RefusedActionPreview],
    pre_live_seat_state: &'a league_core::SeatState,
    post_market_live_seat_state: &'a league_core::SeatState,
    live_engine: &'a TurnEngine,
    live_receipt: &'a SweepReceipt,
    seat: u8,
    regent_absent_peer: Option<(u8, bool)>,
}

fn assert_step0_to_7_matches_live(preview: &WhatIfPreview, live: &LiveStep0To7Ref<'_>) {
    let expected_swap_orders: Vec<PublicSwapOrderPreview> = live
        .expected_ref
        .swap_orders
        .iter()
        .map(|s| PublicSwapOrderPreview {
            swap_id: s.swap_id,
            maker: s.maker,
            taker: s.taker,
            outcome: match s.outcome {
                league_core::SwapPreviewOutcome::Settled => PublicSwapOutcome::Settled,
                league_core::SwapPreviewOutcome::UnmatchedOrInsufficientBalance => {
                    PublicSwapOutcome::UnmatchedOrInsufficientBalance
                }
            },
        })
        .collect();
    let expected_check_orders: Vec<PublicCheckOrderPreview> = live
        .expected_ref
        .check_orders
        .iter()
        .map(|c| PublicCheckOrderPreview {
            check_id: c.check_id,
            outcome: match c.outcome {
                league_core::CheckPreviewOutcome::Cashed => PublicCheckOutcome::Cashed,
                league_core::CheckPreviewOutcome::Bounced => PublicCheckOutcome::Bounced,
                league_core::CheckPreviewOutcome::NotEligibleOrIgnored => {
                    PublicCheckOutcome::IneligibleOrIgnored
                }
            },
        })
        .collect();

    // `PreviewSteps0To7` omits Step-8 `activate_shocks_for_turn(turn + 1)` while running identical
    // deterministic expiry (`expires_after_turn > turn`) at Step 8 cleanup. Reconstruct that exact
    // post-Steps-0-7 active shock slice so `post_sweep_predicates` compares bit-for-bit (`B6`).
    let mut expected_post_state = live.live_engine.state().clone();
    expected_post_state.active_shocks = live
        .obs
        .active_shocks
        .iter()
        .copied()
        .filter(|s| s.expires_after_turn > live.live_receipt.turn)
        .collect();
    let expected_post_sweep_predicates = must_ok(
        league_core::compute_seat_state_predicates(&expected_post_state, Seat(live.seat)),
        "expected post_sweep_predicates",
    );

    let mut expected_summary = league_core::Step0To7SweepSummary::from(live.live_receipt);
    if let Some((absent_seat, absent_was_human_at_turn_start)) = live.regent_absent_peer {
        assert!(
            preview.step0_to_7_receipt.non_reveal_seats.is_empty(),
            "preview normalizes absent peers into synthetic valid reveals"
        );
        assert_eq!(
            live.live_receipt.non_reveal_seats,
            vec![absent_seat],
            "live turn must record absent Regent peer in non_reveal_seats"
        );
        if absent_was_human_at_turn_start {
            assert!(
                live.live_receipt.regent_acted_seats.contains(&absent_seat),
                "live turn must record absent Human Regent peer in regent_acted_seats"
            );
        }
        // Only Step-1 `non_reveal_seats` and `regent_acted_seats` for `absent_seat` differ by
        // construction in Case (b) (Step-4 C1a takeover entries on any seat, including other
        // seats, are preserved identically across both); all 24 other `Step0To7SweepSummary`
        // fields MUST match bit-for-bit!
        expected_summary
            .non_reveal_seats
            .retain(|&s| s != absent_seat);
        if !preview
            .step0_to_7_receipt
            .regent_acted_seats
            .contains(&absent_seat)
        {
            expected_summary
                .regent_acted_seats
                .retain(|&s| s != absent_seat);
        }
    }

    let expected_what_if = WhatIfPreview {
        seat: live.seat,
        turn: live.live_receipt.turn,
        baseline_predicates: live.obs.baseline_predicates.clone(),
        pre_seat_state: live.pre_live_seat_state.clone(),
        post_market_seat_state: live.post_market_live_seat_state.clone(),
        post_sweep_seat_state: live.live_engine.state().seats[usize::from(live.seat)].clone(),
        post_sweep_predicates: expected_post_sweep_predicates,
        close_mark_caveat: league_core::HYPOTHETICAL_CLOSE_MARK_CAVEAT.to_owned(),
        refused_market_actions: live.expected_refused.to_vec(),
        converter_orders: live.expected_ref.converter_orders.clone(),
        project_orders: live.expected_ref.project_orders.clone(),
        swap_orders: expected_swap_orders,
        check_orders: expected_check_orders,
        escrow_orders: live.expected_ref.escrow_orders.clone(),
        upkeep: live.expected_ref.upkeep,
        spoilage: league_core::SeatSpoilagePreview {
            perishable_spoiled_units: live.expected_ref.seat_spoilage_clawed_back,
        },
        public_fills: live.live_receipt.fills.clone(),
        step0_to_7_receipt: expected_summary,
    };
    assert_eq!(
        preview, &expected_what_if,
        "full 18-field WhatIfPreview mismatch at turn {} seat {}",
        live.live_receipt.turn, live.seat
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn proptest_steps_0_to_7_preview_matches_live_sweep_all_committed_and_regent(
        raw_seed in 0_u64..20_000,
        use_club in any::<bool>(),
        seat_offset in 0_u8..6,
        bid_mult in 1_i64..4,
        regent_missing_peer in any::<bool>(),
    ) {
        let seed = non_reserved_dev_seed(raw_seed);
        let n = 4 + (seed % 3) as u8;
        let actor = seat_offset % n;
        let peer_absent = (actor + 1) % n;
        let peer2 = (actor + 2) % n;
        let peer3 = (actor + 3) % n;
        let eval_seed = eval_seed_from(((seed % 200) + 1) as u8);
        let tid = TableId(50_000 + seed);

        let (mut spec, _, eval_seed) = if use_club {
            must_ok(
                TableSpec::standard_club_chiral_ring_with_explicit_setup(
                    tid,
                    n,
                    seed,
                    [0x19; 32],
                    &seat_salts(n, 0x22),
                    eval_seed,
                ),
                "club spec",
            )
        } else {
            let mut eval_pk_spec = must_ok(
                TableSpec::standard_eval_chiral_ring_with_explicit_setup(
                    tid,
                    n,
                    seed,
                    [0x19; 32],
                    &seat_salts(n, 0x22),
                    eval_seed,
                ),
                "eval spec",
            );
            // Make peer_absent Human with RegentDoctrine so both Club & Eval exercise Regent modeling.
            eval_pk_spec.0.seats[usize::from(peer_absent)].identity = SeatIdentity::Human;
            eval_pk_spec.0.seats[usize::from(peer_absent)].regent = Some(RegentDoctrine {
                risk_tolerance_bps: 5_000,
                min_raw_reserve: UNIT_SCALE,
                auto_cash_due_claims: true,
            });
            eval_pk_spec
        };
        if use_club {
            spec.seats[usize::from(peer_absent)].identity = SeatIdentity::Human;
            spec.seats[usize::from(peer_absent)].regent = Some(RegentDoctrine {
                risk_tolerance_bps: 5_000,
                min_raw_reserve: UNIT_SCALE,
                auto_cash_due_claims: true,
            });
        }

        let mut engine = must_ok(TurnEngine::new(spec), "TurnEngine::new");
        let mut pending_due_check_id: Option<u64> = None;

        // Advance through Turn 4 (election) and Turn 6 (charter auction + post-Turn-2 shock draw)
        // checking full 18-field `WhatIfPreview` equivalence on Turns 1..=6.
        for t in 1..=6_u8 {
            if engine.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            let regent_absent_this_turn = regent_missing_peer && ((t + (seed as u8 & 1)) % 2 == 1);
            let (turn_beacon, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove_turn");
            let obs = must_ok(engine.observe_seat(Seat(actor)), "observe_seat");
            let pub_state = obs.to_public_preview_state();

            let action = if t == 1 {
                let buy_good = AssetId::Raw((actor + 1) % n);
                let price = engine.state().parities.asset_parity(buy_good) + 100 * bid_mult;
                MarketAction::PostOffer {
                    seat: actor,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: buy_good,
                        amount: 500,
                        price_per_unit_cred: price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }
            } else {
                let good = AssetId::Raw(actor);
                let price = engine.state().parities.asset_parity(good) + 100 * bid_mult;
                MarketAction::PostOffer {
                    seat: actor,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good,
                        amount: 500,
                        price_per_unit_cred: price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }
            };
            let mut actor_market_actions = vec![action.clone()];
            if t == 3 {
                // Deliberately include a refused Phase-1 action so `refused_market_actions` is checked.
                actor_market_actions.push(MarketAction::CancelOffer {
                    seat: actor,
                    offer_id: 999_999,
                });
            }

            let peer_action = MarketAction::TownImport {
                seat: peer2,
                good: AssetId::Raw(peer2),
                amount: 200,
            };
            let mut peer_market_actions = vec![peer_action.clone()];
            let mut regent_phase1_context = vec![action.clone(), peer_action.clone()];
            let mut created_check_this_turn = false;
            if t == 2 {
                let check_action = MarketAction::CreateCheck {
                    seat: peer2,
                    check: CheckProposal {
                        payee: actor,
                        asset: AssetId::Cred,
                        send_max: 200,
                        deliver_min: 200,
                        due_turn: 3,
                        expiration_turn: 5,
                    },
                };
                peer_market_actions.push(check_action.clone());
                regent_phase1_context.push(check_action);
                created_check_this_turn = true;
            }

            let mut actor_bundle = SealedOrderBundle::default();
            actor_bundle.converter_runs.push(ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            });
            let mut assumed_cosigned_swaps = Vec::new();
            if t == 3 {
                if let Some(cid) = pending_due_check_id.take() {
                    actor_bundle.check_cashes.push(cid);
                }
                actor_bundle.check_cashes.push(999_999);
                let swap_settled = AtomicSwapProposal {
                    swap_id: 301,
                    maker: actor,
                    taker: peer2,
                    maker_asset: AssetId::Cred,
                    maker_amount: 100,
                    taker_asset: AssetId::Raw(peer2),
                    taker_amount: 100,
                    domain_id: None,
                };
                let swap_unmatched = AtomicSwapProposal {
                    swap_id: 302,
                    maker: actor,
                    taker: peer3,
                    maker_asset: AssetId::Cred,
                    maker_amount: 50,
                    taker_asset: AssetId::Raw(peer3),
                    taker_amount: 50,
                    domain_id: None,
                };
                actor_bundle.atomic_swaps.push(swap_settled.clone());
                actor_bundle.atomic_swaps.push(swap_unmatched);
                assumed_cosigned_swaps.push(swap_settled);
            }
            if t % 4 == 0 {
                actor_bundle.council_vote = Some(league_core::CouncilVoteBallot {
                    trade_minister: actor,
                    cartel_chair: (actor + 1) % n,
                });
            }
            if t % 6 == 0 {
                actor_bundle.charter_bids.push(CharterBidOrder {
                    role: CharterRole::OracleAuditor,
                    bid_cred: 250,
                });
            }

            let mut committed_bundles = BTreeMap::new();
            committed_bundles.insert(actor, actor_bundle.clone());

            let mut assumed_peer_bundles = BTreeMap::new();
            for p in 0..n {
                if p == actor {
                    continue;
                }
                if regent_absent_this_turn && p == peer_absent {
                    // Case (b): uncommitted Human peer modeled via Standing Regent helper!
                    let preview_regent = must_ok(
                        engine.preview_regent_bundle_after_step0(
                            Turn(t),
                            Seat(p),
                            &regent_phase1_context,
                            turn_beacon,
                        ),
                        "preview_regent_bundle_after_step0",
                    );
                    if let Some(rb) = preview_regent {
                        assumed_peer_bundles.insert(p, rb);
                    }
                } else {
                    let mut pb = SealedOrderBundle::default();
                    pb.converter_runs.push(ConverterRunOrder {
                        recipe_idx: 0,
                        runs: 1,
                    });
                    assumed_peer_bundles.insert(p, pb.clone());
                    if t == 3 && p == peer2 {
                        pb.atomic_swaps.extend(assumed_cosigned_swaps.iter().cloned());
                    }
                    committed_bundles.insert(p, pb);
                }
            }
            let mut ref_peer_bundles = assumed_peer_bundles.clone();
            if t == 3 {
                ref_peer_bundles
                    .entry(peer2)
                    .or_default()
                    .atomic_swaps
                    .extend(assumed_cosigned_swaps.iter().cloned());
            }

            let req = HumanWhatIfRequest {
                seat: actor,
                market_actions: actor_market_actions.clone(),
                cancel_lock_at: Some(actor_market_actions.len()),
                bundle: actor_bundle.clone(),
                tie_mode: PreviewTieMode::NeutralSeed,
                assumptions: CounterpartyAssumptions {
                    assumed_peer_market_actions: peer_market_actions.clone(),
                    assumed_peer_bundles: assumed_peer_bundles.clone(),
                    assumed_cosigned_swaps: assumed_cosigned_swaps.clone(),
                    ..CounterpartyAssumptions::default()
                },
            };
            let preview = must_ok(
                preview_what_if_with_beacon(&pub_state, &req, turn_beacon),
                "preview_what_if_with_beacon",
            );
            if t == 3 {
                prop_assert!(!preview.refused_market_actions.is_empty());
                prop_assert_eq!(preview.check_orders.len(), 2);
                prop_assert_eq!(preview.swap_orders.len(), 2);
            }

            // Apply identical Phase-1 actions, cancel lock, and reference Steps 0–7 preview before
            // live commit/reveal & sweep.
            let absent_was_human_at_turn_start =
                engine.state().seats[usize::from(peer_absent)].identity == SeatIdentity::Human;
            let pre_live_seat_state = engine.state().seats[usize::from(actor)].clone();
            let mut live_refused = Vec::new();
            for (idx, act) in actor_market_actions.iter().enumerate() {
                if let Err(err) = engine.apply_market_action(act.clone()) {
                    let msg = err.to_string();
                    live_refused.push(RefusedActionPreview {
                        action_index: idx,
                        error_code: err.code().to_owned(),
                        message: scrub_secret_text(&msg),
                    });
                }
            }
            must_ok(engine.begin_cancel_lock(), "live cancel_lock");
            let mut created_check_id_now = None;
            for peer_act in &peer_market_actions {
                if matches!(peer_act, MarketAction::CreateCheck { .. }) {
                    created_check_id_now = Some(engine.state().next_check_id);
                }
                must_ok(engine.apply_market_action(peer_act.clone()), "live peer_action");
            }
            if created_check_this_turn {
                pending_due_check_id = created_check_id_now;
            }
            let post_market_live_seat_state = engine.state().seats[usize::from(actor)].clone();

            let expected_ref = must_ok(
                engine_internals::preview_bundle_with_context_and_mode(
                    &engine,
                    Seat(actor),
                    &[],
                    &actor_bundle,
                    &ref_peer_bundles,
                    turn_beacon,
                    engine_internals::SweepMode::PreviewSteps0To7,
                ),
                "ref steps 0-7 preview",
            );

            commit_and_reveal_subset(&mut engine, tid, t, &committed_bundles);
            let live_receipt = must_ok(engine.sweep_turn_with_eval_vrf(&proof), "live sweep");

            if !regent_absent_this_turn {
                prop_assert!(live_receipt.non_reveal_seats.is_empty());
                assert_step0_to_7_matches_live(
                    &preview,
                    &LiveStep0To7Ref {
                        obs: &obs,
                        expected_ref: &expected_ref,
                        expected_refused: &live_refused,
                        pre_live_seat_state: &pre_live_seat_state,
                        post_market_live_seat_state: &post_market_live_seat_state,
                        live_engine: &engine,
                        live_receipt: &live_receipt,
                        seat: actor,
                        regent_absent_peer: None,
                    },
                );
            } else {
                // Case (b): uncommitted peer_absent is Regent-modeled; full SeatState, full
                // 18-field WhatIfPreview, and all Step 0-7 receipt fields (except Step-1
                // `non_reveal_seats` and `regent_acted_seats` for `peer_absent`, which differ by
                // construction) match bit-for-bit!
                assert_step0_to_7_matches_live(
                    &preview,
                    &LiveStep0To7Ref {
                        obs: &obs,
                        expected_ref: &expected_ref,
                        expected_refused: &live_refused,
                        pre_live_seat_state: &pre_live_seat_state,
                        post_market_live_seat_state: &post_market_live_seat_state,
                        live_engine: &engine,
                        live_receipt: &live_receipt,
                        seat: actor,
                        regent_absent_peer: Some((peer_absent, absent_was_human_at_turn_start)),
                    },
                );
            }
        }
    }
}

// =================================================================================================
// 3. Secret Non-Interference & `Commit`/`Reveal` Isolation `proptest` (§2.5 item 2, C4, B1, B5)
// =================================================================================================

fn mutate_drawn_shocks_in_engine(
    engine: &TurnEngine,
    mutate: impl FnOnce(&mut Vec<ScheduledShockSpec>),
) -> TurnEngine {
    let mut val = must_ok(serde_json::to_value(engine), "engine to value");
    let shocks_val = val
        .get_mut("state")
        .and_then(|st| st.get_mut("drawn_shocks"))
        .unwrap_or_else(|| panic!("missing state.drawn_shocks"));
    let mut drawn: Vec<ScheduledShockSpec> = must_ok(
        serde_json::from_value(shocks_val.clone()),
        "parse drawn_shocks",
    );
    mutate(&mut drawn);
    *shocks_val = must_ok(
        serde_json::to_value(&drawn),
        "serialize mutated drawn_shocks",
    );
    must_ok(serde_json::from_value(val), "reconstruct TurnEngine")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn proptest_secret_non_interference_and_commit_reveal_isolation(
        raw_seed in 0_u64..10_000,
        use_club in any::<bool>(),
        club_stage in 0_u8..3,
        phase_pick in 0_u8..3,
        alt_byte in 1_u8..=250,
    ) {
        let seed = non_reserved_dev_seed(raw_seed);
        let tid = TableId(80_000 + seed);
        let n = 4_u8;
        let eval_seed = eval_seed_from(0x33);

        let (mut spec1, deal1, _) = must_ok(
            TableSpec::standard_club_chiral_ring_with_explicit_setup(
                tid, n, seed, [0x11; 32], &seat_salts(n, 0x05), eval_seed.clone(),
            ),
            "spec1",
        );
        let (mut spec2, deal2, _) = must_ok(
            TableSpec::standard_club_chiral_ring_with_explicit_setup(
                tid, n, seed, [alt_byte; 32], &seat_salts(n, alt_byte), eval_seed.clone(),
            ),
            "spec2",
        );

        let (mut e1, mut e2, observer_seat, target_turn) = if !use_club {
            // Eval path (Turn 1): vary pre-seeded `spec.shocks` (`hint_turn > turn = 1`),
            // `dealt_mandate_commits`, and rival `mandate_commits`.
            spec1.mode = TableMode::Eval;
            spec2.mode = TableMode::Eval;
            spec1.shocks = vec![ScheduledShockSpec {
                hint_turn: 2,
                forecast_turn: 3,
                realize_turn: 4,
                target_seat: 2,
                kind: ShockKind::SupplyFreeze,
                magnitude_bps: 2_500,
            }];
            spec2.shocks = vec![ScheduledShockSpec {
                hint_turn: 2,
                forecast_turn: 3,
                realize_turn: 4,
                target_seat: 3,
                kind: ShockKind::TariffHoliday,
                magnitude_bps: 7_500,
            }];
            let mut e1 = must_ok(TurnEngine::new(spec1), "e1");
            let mut e2 = must_ok(TurnEngine::new(spec2), "e2");
            for s in 0..n {
                let c1 = must_ok(
                    compute_mandate_commitment(tid, Seat(s), &deal1.seat_reveals[usize::from(s)]),
                    "c1",
                );
                must_ok(e1.submit_mandate_commitment(Seat(s), c1), "e1 mc");
                let c2 = must_ok(
                    compute_mandate_commitment(tid, Seat(s), &deal2.seat_reveals[usize::from(s)]),
                    "c2",
                );
                must_ok(e2.submit_mandate_commitment(Seat(s), c2), "e2 mc");
            }
            (e1, e2, 0_u8, 1_u8)
        } else {
            // Club path (`TableMode::Club`, empty `spec.shocks`): vary Turn-1 `dealt_mandate_commits`
            // and `mandate_commits`, advance through real VRF Sweeps to Turn 3 (`club_stage == 0`,
            // after Turn-2 draw) or Turn 8 (`club_stage` 1 & 2, after Turn-6 OracleAuditor win and
            // Turn-7 draw), and vary `state.drawn_shocks` between `e1` and `e2`.
            prop_assert_eq!(spec1.mode, TableMode::Club);
            prop_assert!(spec1.shocks.is_empty());
            let mut e1 = must_ok(TurnEngine::new(spec1), "club e1");
            let mut e2 = must_ok(TurnEngine::new(spec2), "club e2");
            for s in 0..n {
                let c1 = must_ok(
                    compute_mandate_commitment(tid, Seat(s), &deal1.seat_reveals[usize::from(s)]),
                    "c1",
                );
                must_ok(e1.submit_mandate_commitment(Seat(s), c1), "e1 mc");
                let c2 = must_ok(
                    compute_mandate_commitment(tid, Seat(s), &deal2.seat_reveals[usize::from(s)]),
                    "c2",
                );
                must_ok(e2.submit_mandate_commitment(Seat(s), c2), "e2 mc");
            }
            let (target_turn, observer_seat) = match club_stage {
                0 => (3_u8, 0_u8),
                1 => (8_u8, 0_u8),
                _ => (8_u8, 1_u8),
            };
            for t in 1..target_turn {
                let mut bundles = BTreeMap::new();
                if t == 6 {
                    let mut b1 = SealedOrderBundle::default();
                    b1.charter_bids.push(CharterBidOrder {
                        role: CharterRole::OracleAuditor,
                        bid_cred: 300,
                    });
                    bundles.insert(1_u8, b1);
                }
                commit_and_reveal_subset(&mut e1, tid, t, &bundles);
                commit_and_reveal_subset(&mut e2, tid, t, &bundles);
                let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
                must_ok(e1.sweep_turn_with_eval_vrf(&proof), "e1 sweep");
                must_ok(e2.sweep_turn_with_eval_vrf(&proof), "e2 sweep");
            }

            let mutated_e2 = mutate_drawn_shocks_in_engine(&e2, |drawn| {
                let alt_kind = match alt_byte % 4 {
                    0 => ShockKind::DemandSurge,
                    1 => ShockKind::SupplyFreeze,
                    2 => ShockKind::TariffHoliday,
                    _ => ShockKind::SpoilageSpike,
                };
                if club_stage == 0 {
                    // Turn 3 after Turn-2 draw (hint 3, forecast 4, realize 5): mutate unforecast
                    // shock completely while observing non-Auditor Seat 0.
                    if let Some(last) = drawn.last_mut() {
                        last.target_seat = (last.target_seat + 1 + (alt_byte % 3)) % n;
                        last.kind = alt_kind;
                        last.magnitude_bps = 1_000 + u16::from(alt_byte) * 20;
                    }
                } else if club_stage == 1 {
                    // Turn 8 after Turn-7 draw (hint 8, forecast 9, realize 10): mutate Turn-7
                    // drawn shock and append a future hint_turn = 9 shock; observe non-Auditor Seat 0.
                    if let Some(last) = drawn.last_mut() {
                        last.target_seat = (last.target_seat + 1 + (alt_byte % 3)) % n;
                        last.kind = alt_kind;
                        last.magnitude_bps = 1_000 + u16::from(alt_byte) * 20;
                    }
                    drawn.push(ScheduledShockSpec {
                        hint_turn: 9,
                        forecast_turn: 10,
                        realize_turn: 11,
                        target_seat: alt_byte % n,
                        kind: alt_kind,
                        magnitude_bps: 3_333,
                    });
                } else {
                    // Turn 8 observing active `OracleAuditor` Seat 1: vary `magnitude_bps` on the
                    // active Turn-7 shock (`hint_turn = 8, forecast_turn = 9`) AND vary a future
                    // `hint_turn = 9 > 8` shock in `drawn_shocks`!
                    if let Some(last) = drawn.last_mut() {
                        last.magnitude_bps = 1_200 + u16::from(alt_byte) * 25;
                    }
                    drawn.push(ScheduledShockSpec {
                        hint_turn: 9,
                        forecast_turn: 10,
                        realize_turn: 11,
                        target_seat: alt_byte % n,
                        kind: alt_kind,
                        magnitude_bps: 4_444,
                    });
                }
            });
            (e1, mutated_e2, observer_seat, target_turn)
        };

        // Across TalkAndMarket, Commit (differing peer commitments), and Reveal (differing peer
        // reveals), `observer_seat`'s `SeatObservation`, `PublicPreviewState`, `preview_what_if`,
        // and `preview_seat_bundle` MUST stay byte-identical!
        let peer_a = (observer_seat + 1) % n;
        let peer_b = (observer_seat + 2) % n;
        if phase_pick >= 1 {
            must_ok(e1.advance_to_commit(), "e1 commit");
            must_ok(e2.advance_to_commit(), "e2 commit");
            let b_peer_a = SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 2,
                }],
                ..SealedOrderBundle::default()
            };
            let rev_pa = OrderReveal {
                bundle: b_peer_a,
                salt: RevealSalt([0x77; 32]),
            };
            let c_pa = must_ok(
                compute_order_commitment(tid, Turn(target_turn), Seat(peer_a), &rev_pa),
                "c_pa",
            );
            must_ok(e1.submit_order_commitment(Seat(peer_a), c_pa), "e1 submit pa");

            let b_peer_b = SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 1,
                }],
                ..SealedOrderBundle::default()
            };
            let rev_pb = OrderReveal {
                bundle: b_peer_b,
                salt: RevealSalt([alt_byte; 32]),
            };
            let c_pb = must_ok(
                compute_order_commitment(tid, Turn(target_turn), Seat(peer_b), &rev_pb),
                "c_pb",
            );
            must_ok(e2.submit_order_commitment(Seat(peer_b), c_pb), "e2 submit pb");

            if phase_pick == 2 {
                must_ok(e1.advance_to_reveal(), "e1 reveal");
                must_ok(e2.advance_to_reveal(), "e2 reveal");
                must_ok(e1.submit_order_reveal(Seat(peer_a), rev_pa), "e1 rev pa");
                must_ok(e2.submit_order_reveal(Seat(peer_b), rev_pb), "e2 rev pb");
            }
        }

        let obs1 = must_ok(e1.observe_seat(Seat(observer_seat)), "obs1");
        let obs2 = must_ok(e2.observe_seat(Seat(observer_seat)), "obs2");
        if use_club && club_stage == 2 {
            prop_assert!(!obs1.auditor_hints().is_empty());
        }
        prop_assert_eq!(obs1.auditor_hints(), obs2.auditor_hints());
        prop_assert_eq!(
            must_ok(serde_json::to_vec(&obs1), "ser obs1"),
            must_ok(serde_json::to_vec(&obs2), "ser obs2")
        );

        let pub1 = obs1.to_public_preview_state();
        let pub2 = obs2.to_public_preview_state();
        prop_assert_eq!(
            must_ok(serde_json::to_vec(&pub1), "ser pub1"),
            must_ok(serde_json::to_vec(&pub2), "ser pub2")
        );

        let ai_req = AiPreviewRequest {
            seat: observer_seat,
            market_actions: Vec::new(),
            bundle: SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 1,
                }],
                ..SealedOrderBundle::default()
            },
        };
        let ai1 = must_ok(preview_seat_bundle(&pub1, &ai_req), "ai1");
        let ai2 = must_ok(preview_seat_bundle(&pub2, &ai_req), "ai2");
        prop_assert_eq!(
            must_ok(serde_json::to_vec(&ai1), "ser ai1"),
            must_ok(serde_json::to_vec(&ai2), "ser ai2")
        );

        let human_req = HumanWhatIfRequest {
            seat: observer_seat,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            bundle: ai_req.bundle,
            tie_mode: PreviewTieMode::NeutralSeed,
            assumptions: CounterpartyAssumptions::default(),
        };
        let hw1 = must_ok(preview_what_if(&pub1, &human_req), "hw1");
        let hw2 = must_ok(preview_what_if(&pub2, &human_req), "hw2");
        prop_assert_eq!(hw1, hw2);
    }
}

// =================================================================================================
// 4. `TableMode::Club` vs `TableMode::Eval` Refusal Tests & Replay Verification
// =================================================================================================

#[test]
fn club_and_eval_tables_refuse_each_others_inputs_and_bad_seeds() {
    // Seed validation: accept valid Dev seeds, refuse reserved and out-of-pool seeds.
    assert!(validate_club_dev_seed(0).is_ok());
    assert!(validate_club_dev_seed(1).is_ok());
    for &reserved in &RESERVED_TEST_LAYOUT_SEEDS {
        let err = must_err(validate_club_dev_seed(reserved), "reserved seed");
        assert_eq!(err.code(), "INVALID_CONFIG");
        assert!(err.to_string().contains("RESERVED_SEED_FORBIDDEN"));
    }
    for non_dev in [
        DEV_LAYOUT_SEED_MAX + 1,
        10_000,
        500_000,
        1_500_000,
        (1_u64 << 63) | 7,
    ] {
        let err = must_err(validate_club_dev_seed(non_dev), "non-dev seed");
        assert_eq!(err.code(), "INVALID_CONFIG");
        assert!(err.to_string().contains("SEED_POOL_MISMATCH"));
    }

    // TableSpec validation for TableMode::Club.
    let eval_seed = eval_seed_from(0x41);
    let (club_spec, club_deal, club_eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_setup(TableId(7_001), 4, 1),
        "club setup",
    );
    assert_eq!(club_spec.mode, TableMode::Club);
    assert!(club_spec.eval_vrf_pubkey.is_some());
    assert!(club_spec.drand_schedule.is_none());
    assert!(club_spec.shocks.is_empty());

    let mut bad = club_spec.clone();
    bad.drand_schedule = Some(DrandSchedule {
        turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 600,
        turn_period_secs: 3,
    });
    assert!(bad.validate().is_err());

    let mut bad = club_spec.clone();
    bad.shocks.push(ScheduledShockSpec {
        hint_turn: 2,
        forecast_turn: 3,
        realize_turn: 4,
        target_seat: 0,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_000,
    });
    assert!(bad.validate().is_err());

    let mut bad = club_spec.clone();
    bad.eval_vrf_pubkey = None;
    assert!(bad.validate().is_err());

    // TurnEngine sweep mode mutual refusal on Club tables.
    let mut club_eng = must_ok(TurnEngine::new(club_spec.clone()), "club engine");
    commit_and_reveal_subset(&mut club_eng, club_spec.table_id, 1, &BTreeMap::new());
    let dummy_beacon = derive_turn_beacon(0, club_spec.table_id, Turn(1));
    assert!(club_eng.sweep_turn(dummy_beacon).is_err());
    assert!(club_eng.sweep_turn_seeded(0).is_err());
    assert!(
        club_eng
            .sweep_turn_with_drand(1, &[0x12; DRAND_QUICKNET_SIGNATURE_BYTES])
            .is_err()
    );

    // Run a valid full Club season and replay via both replay_club_season and replay_season.
    let mut club_eng = must_ok(TurnEngine::new(club_spec.clone()), "club engine clean");
    let mut mandate_commits = Vec::new();
    for s in 0..4_u8 {
        let rev = &club_deal.seat_reveals[usize::from(s)];
        let mc = must_ok(
            compute_mandate_commitment(club_spec.table_id, Seat(s), rev),
            "mc",
        );
        must_ok(club_eng.submit_mandate_commitment(Seat(s), mc), "submit mc");
        mandate_commits.push(Some(mc));
    }
    let mut logged_turns = Vec::new();
    let mut live_digests = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if club_eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let mut bundles = BTreeMap::new();
        for s in 0..4_u8 {
            bundles.insert(
                s,
                SealedOrderBundle {
                    converter_runs: vec![ConverterRunOrder {
                        recipe_idx: 0,
                        runs: 1,
                    }],
                    ..SealedOrderBundle::default()
                },
            );
        }
        let (commits, reveals) =
            commit_and_reveal_subset(&mut club_eng, club_spec.table_id, t, &bundles);
        let (beacon, proof) = must_ok(
            club_eval_seed.prove_turn(club_spec.table_id, Turn(t)),
            "prove",
        );
        let rec = must_ok(club_eng.sweep_turn_with_eval_vrf(&proof), "club vrf sweep");
        live_digests.push(rec.post_state_digest);
        logged_turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commit_deadline_unix: None,
            commitments: commits,
            reveals,
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(proof.to_vec()),
            drawn_shocks: rec.newly_drawn_shocks.clone(),
        });
        if rec.season_closed {
            break;
        }
    }
    let mut mandate_reveals = Vec::new();
    for s in 0..4_u8 {
        let rev = club_deal.seat_reveals[usize::from(s)].clone();
        must_ok(club_eng.submit_mandate_reveal(Seat(s), rev.clone()), "mr");
        mandate_reveals.push(Some(rev));
    }
    let club_board = must_ok(club_eng.finalize_season(), "finalize");

    let club_log = must_ok(
        SeasonReplayLog::from_rig_seed(
            club_spec.clone(),
            1,
            club_deal.clone(),
            mandate_commits.clone(),
            logged_turns.clone(),
            mandate_reveals.clone(),
        ),
        "from_rig_seed club",
    );
    assert_eq!(club_log.mode, TableMode::Club);
    assert_eq!(club_log.beacon_mode, ReplayBeaconMode::EvalVrf);
    let club_out = must_ok(replay_club_season(&club_log), "replay_club_season");
    assert_eq!(club_out.state_digests, live_digests);
    assert_eq!(club_out.scoreboard, club_board);
    assert_eq!(club_out.drand_beacon_auth_status, None);
    let club_record = must_ok(attach_state_predicates(&club_log), "attach predicates");
    assert!(club_record.state_predicates.is_some());
    let (club_out_2, _) = must_ok(
        replay_season_with_state_predicates(&club_log),
        "replay_season_with_state_predicates",
    );
    assert_eq!(club_out_2, club_out);

    // Club vs Eval mutual refusal on ReplayRecord deserializers and replay entry points:
    let club_json = must_ok(serde_json::to_vec(&club_log), "ser club log");
    assert!(ReplayRecord::from_club_json_slice(&club_json).is_ok());
    let eval_rejects_club = must_err(
        ReplayRecord::from_eval_json_slice(&club_json),
        "eval rejects club",
    );
    assert_eq!(eval_rejects_club.code(), "INVALID_CONFIG");
    assert!(replay_eval_season(&club_log).is_err());

    // Build an Eval log and confirm Club refuses it cleanly.
    let (eval_spec, eval_deal, ev_seed) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            TableId(7_002),
            4,
            2,
            [0x61; 32],
            &seat_salts(4, 0x31),
            eval_seed,
        ),
        "eval setup",
    );
    let mut eval_eng = must_ok(TurnEngine::new(eval_spec.clone()), "eval eng");
    let mut ev_turns = Vec::new();
    for t in 1..=HARD_TURN_CAP {
        if eval_eng.state().phase != TurnPhase::TalkAndMarket {
            break;
        }
        let (c, r) =
            commit_and_reveal_subset(&mut eval_eng, eval_spec.table_id, t, &BTreeMap::new());
        let (b, p) = must_ok(ev_seed.prove_turn(eval_spec.table_id, Turn(t)), "ev prove");
        let rec = must_ok(eval_eng.sweep_turn_with_eval_vrf(&p), "ev sweep");
        ev_turns.push(LoggedTurn {
            turn: t,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            commit_deadline_unix: None,
            commitments: c,
            reveals: r,
            turn_beacon: b,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(p.to_vec()),
            drawn_shocks: rec.newly_drawn_shocks.clone(),
        });
        if rec.season_closed {
            break;
        }
    }
    let eval_log = must_ok(
        SeasonReplayLog::from_rig_seed(
            eval_spec.clone(),
            2,
            eval_deal,
            vec![None; 4],
            ev_turns,
            vec![None; 4],
        ),
        "eval log",
    );
    let eval_json = must_ok(serde_json::to_vec(&eval_log), "ser eval log");
    assert!(ReplayRecord::from_eval_json_slice(&eval_json).is_ok());
    let club_rejects_eval = must_err(
        ReplayRecord::from_club_json_slice(&eval_json),
        "club rejects eval",
    );
    assert_eq!(club_rejects_eval.code(), "INVALID_CONFIG");
    assert!(replay_club_season(&eval_log).is_err());

    // Refuse invalid Club SeasonReplayLog overrides: sealed seed, non-dev seed, mode mismatch, SeedDerived.
    let sealed_err = must_err(
        SeasonReplayLog::from_rig_seed(
            club_spec.clone(),
            1 << 63,
            club_deal.clone(),
            mandate_commits.clone(),
            logged_turns.clone(),
            mandate_reveals.clone(),
        ),
        "club rejects sealed seed",
    );
    assert!(sealed_err.to_string().contains("SEED_POOL_MISMATCH"));
    assert!(
        SeasonReplayLog::from_rig_seed(
            club_spec,
            10_000,
            club_deal,
            mandate_commits,
            logged_turns,
            mandate_reveals,
        )
        .is_err()
    );
    let mut bad_mode_log = club_log.clone();
    bad_mode_log.mode = TableMode::Eval;
    assert!(replay_season(&bad_mode_log).is_err());
    let mut bad_beacon_log = club_log;
    bad_beacon_log.beacon_mode = ReplayBeaconMode::SeedDerived;
    assert!(replay_season(&bad_beacon_log).is_err());

    // Verify table_id spec setter preserves `drand_schedule` / `shocks` failure checks inside replay.
    let _ = &eval_spec;
}

// =================================================================================================
// 5. Replay Format Version Compatibility Tests (Must Item 5: Current / Newer / Pre-Version)
// =================================================================================================

#[test]
fn replay_format_version_current_reads_newer_refused_by_name_and_pre_version_reads() {
    // 1. Current format_version == REPLAY_FORMAT_VERSION (1) reads cleanly.
    const PRE_CARD1_FIXTURE: &str = include_str!("fixtures/pre_card1_94c73626_replay_bundle.json");
    let mut val: serde_json::Value =
        must_ok(serde_json::from_str(PRE_CARD1_FIXTURE), "parse fixture");
    let pre_log_bytes = must_ok(serde_json::to_vec(&val["log"]), "pre log bytes");
    let pre_outcome_bytes = must_ok(serde_json::to_vec(&val["outcome"]), "pre outcome bytes");

    // Pre-version (no `format_version` key) reads cleanly.
    let pre_log = must_ok(SeasonReplayLog::from_json_slice(&pre_log_bytes), "pre log");
    let pre_rec = must_ok(ReplayRecord::from_json_slice(&pre_log_bytes), "pre rec");
    let pre_out = must_ok(
        ReplayOutcome::from_json_slice(&pre_outcome_bytes),
        "pre out",
    );
    assert_eq!(pre_rec.log, pre_log);

    // Current version serialized from Rust carries `format_version: REPLAY_FORMAT_VERSION` and reads cleanly.
    let cur_log_bytes = must_ok(serde_json::to_vec(&pre_log), "cur log bytes");
    let cur_val: serde_json::Value = must_ok(serde_json::from_slice(&cur_log_bytes), "cur val");
    assert_eq!(
        cur_val["format_version"],
        serde_json::json!(REPLAY_FORMAT_VERSION)
    );
    assert_eq!(
        must_ok(SeasonReplayLog::from_json_slice(&cur_log_bytes), "cur log"),
        pre_log
    );
    assert_eq!(
        must_ok(ReplayRecord::from_json_slice(&cur_log_bytes), "cur rec").log,
        pre_log
    );
    let cur_out_bytes = must_ok(serde_json::to_vec(&pre_out), "cur out bytes");
    assert_eq!(
        must_ok(ReplayOutcome::from_json_slice(&cur_out_bytes), "cur out"),
        pre_out
    );

    // 2. Newer version (`2`, `99`) — EVEN WITH AN UNKNOWN FUTURE FILL TAG inside the body —
    //    is refused cleanly by name (`UnsupportedReplayVersion { found, newest_known }`),
    //    NOT an opaque serde variant error!
    val["log"]["format_version"] = serde_json::json!(2);
    val["log"]["spec"]["future_tag"] = serde_json::json!("unknown_v2_enum_variant");
    let future_log_bytes = must_ok(serde_json::to_vec(&val["log"]), "future log");
    let err = must_err(
        SeasonReplayLog::from_json_slice(&future_log_bytes),
        "future log refused",
    );
    assert_eq!(err.code(), "UNSUPPORTED_REPLAY_VERSION");
    assert_eq!(
        err,
        EngineError::UnsupportedReplayVersion {
            found: 2,
            newest_known: REPLAY_FORMAT_VERSION,
        }
    );
    assert!(err.to_string().contains("version 2"));
    assert!(
        err.to_string()
            .contains(&format!("newest known version is {REPLAY_FORMAT_VERSION}"))
    );

    let err_rec = must_err(
        ReplayRecord::from_json_slice(&future_log_bytes),
        "future record refused",
    );
    assert_eq!(err_rec, err);

    val["outcome"]["format_version"] = serde_json::json!(99);
    val["outcome"]["receipts"][0]["fills"] = serde_json::json!([
        {
            "turn": 1,
            "kind": "future_fill_kind_v99",
            "good": "Cred",
            "from": 0,
            "to": 1,
            "units": 100,
            "net_units": 100,
            "cred_amount": 100
        }
    ]);
    let future_out_bytes = must_ok(serde_json::to_vec(&val["outcome"]), "future out");
    let err_out = must_err(
        ReplayOutcome::from_json_slice(&future_out_bytes),
        "future outcome refused",
    );
    assert_eq!(
        err_out,
        EngineError::UnsupportedReplayVersion {
            found: 99,
            newest_known: REPLAY_FORMAT_VERSION,
        }
    );

    // Version 0 and non-numeric format_version / serde deserialize rejections also surface clearly.
    assert_eq!(
        must_err(
            check_replay_format_version_slice(br#"{"format_version":0}"#),
            "version 0 header"
        ),
        EngineError::UnsupportedReplayVersion {
            found: 0,
            newest_known: REPLAY_FORMAT_VERSION,
        }
    );
    assert!(check_replay_format_version_slice(br#"{"format_version":"2"}"#).is_err());
    assert!(serde_json::from_slice::<SeasonReplayLog>(&future_log_bytes).is_err());
    assert!(serde_json::from_slice::<ReplayOutcome>(&future_out_bytes).is_err());

    // SeasonReplayLog::from_eval_json_slice & SeasonReplayLog::from_club_json_slice reject version 0
    // and mismatched modes (`C5`).
    val["log"]["format_version"] = serde_json::json!(0);
    let v0_log_bytes = must_ok(serde_json::to_vec(&val["log"]), "v0 log bytes");
    assert_eq!(
        must_err(SeasonReplayLog::from_json_slice(&v0_log_bytes), "v0 log"),
        EngineError::UnsupportedReplayVersion {
            found: 0,
            newest_known: REPLAY_FORMAT_VERSION,
        }
    );
    assert_eq!(
        must_err(
            SeasonReplayLog::from_eval_json_slice(&v0_log_bytes),
            "v0 eval log"
        ),
        EngineError::UnsupportedReplayVersion {
            found: 0,
            newest_known: REPLAY_FORMAT_VERSION,
        }
    );
    assert!(SeasonReplayLog::from_eval_json_slice(&cur_log_bytes).is_ok());
    assert_eq!(
        must_err(
            SeasonReplayLog::from_club_json_slice(&cur_log_bytes),
            "club rejects eval log"
        )
        .code(),
        "INVALID_CONFIG"
    );
}

// =================================================================================================
// 6. `StructuredError`, `ToStructuredError`, and Preview Edge-Case Coverage
// =================================================================================================

#[test]
fn structured_error_shapes_codes_and_secret_scrubbing() {
    let engine_variants = [
        EngineError::InvalidConfig("bad config OPENROUTER_API_KEY=sk-or-secret123".to_owned()),
        EngineError::InvalidPhase("wrong phase".to_owned()),
        EngineError::InvalidSeat(9),
        EngineError::InvalidOrder("Bearer my-secret-token-value bad order".to_owned()),
        EngineError::UnknownObject("missing".to_owned()),
        EngineError::Unauthorized("unauth".to_owned()),
        EngineError::PerishableEscrowForbidden,
        EngineError::InsufficientBalance("short".to_owned()),
        EngineError::AlreadySubmitted(0),
        EngineError::CommitmentMismatch(1),
        EngineError::DomainAccessDenied("denied".to_owned()),
        EngineError::CancelLocked,
        EngineError::SeasonEnded("ended".to_owned()),
        EngineError::UnsupportedReplayVersion {
            found: 5,
            newest_known: REPLAY_FORMAT_VERSION,
        },
    ];
    for err in &engine_variants {
        let se = err.to_structured_error();
        assert_eq!(se.code, err.code());
        assert!(!se.hint.is_empty());
        assert!(!se.retryable);
        assert!(!se.message.contains("sk-or-secret123"));
        assert!(!se.message.contains("my-secret-token-value"));
        let json = must_ok(se.to_json_string(), "to_json_string");
        assert!(json.contains(err.code()));
    }

    let seed_errs = [
        ReplayLayoutSeedError::SealedSeedInClear {
            seed_digest: "00".repeat(32),
        },
        ReplayLayoutSeedError::InvalidSealedPool {
            pool: "dev".to_owned(),
        },
        ReplayLayoutSeedError::InvalidSealedDigest,
        ReplayLayoutSeedError::NonSealedSeedInSealed { seed: 42 },
    ];
    for err in &seed_errs {
        let se = err.to_structured_error();
        assert_eq!(se.code, err.code());
        assert!(!se.hint.is_empty());
    }

    let raw_xrpl = "failed signing with sEd7f3A9kL2mN4pQ6rT8vW1xY3zB5cD";
    let with_cause = StructuredError::new("SIGN_ERR", raw_xrpl, "rotate seed", true)
        .with_cause("GEMINI_API_KEY=AIzaSySecret12345 caused retryable failure");
    let text = with_cause.format_text();
    assert!(!text.contains("sEd7f3A9kL2mN4pQ6rT8vW1xY3zB5cD"));
    assert!(!text.contains("AIzaSySecret12345"));
    assert!(text.contains("[REDACTED]"));
    assert!(text.contains("[retryable]"));
    assert_eq!(with_cause.to_string(), text);

    // Non-secret substrings ("bearish", short "sn123", trailing space after Bearer) remain intact.
    assert_eq!(
        scrub_secret_text("bearish market sn123"),
        "bearish market sn123"
    );
    assert_eq!(
        scrub_secret_text("Bearer   trailing"),
        "Bearer   [REDACTED]"
    );
}

#[test]
fn what_if_counterparty_assumptions_pessimistic_ties_and_ai_preview_note_ban() {
    let tid = TableId(90_001);
    let mut engine = must_ok(
        TurnEngine::new(must_ok(TableSpec::standard_chiral_ring(tid, 4, 1), "spec")),
        "eng",
    );
    // Sweep Turn 1 so Step 4 extracts initial raw stock for Turn 2 offers/swaps/deliveries.
    commit_and_reveal_subset(&mut engine, tid, 1, &BTreeMap::new());
    let live_r1 = must_ok(engine.sweep_turn_seeded(1), "sweep turn 1");

    // Create a resting offer from Seat 1, an open Check from Seat 2 -> Seat 0 (due_turn = 2),
    // and an unconditioned Escrow from Seat 2 -> Seat 0.
    let offer_id = engine.state().next_offer_id;
    must_ok(
        engine.apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(1),
                amount: 1_000,
                price_per_unit_cred: 500,
                domain_id: None,
                is_hybrid: false,
            },
        }),
        "post offer",
    );
    let check_due_id = engine.state().next_check_id;
    must_ok(
        engine.apply_market_action(MarketAction::CreateCheck {
            seat: 2,
            check: CheckProposal {
                payee: 0,
                asset: AssetId::Cred,
                send_max: 500,
                deliver_min: 100,
                due_turn: 2,
                expiration_turn: 4,
            },
        }),
        "create check due",
    );
    let check_bounce_id = engine.state().next_check_id;
    must_ok(
        engine.apply_market_action(MarketAction::CreateCheck {
            seat: 3,
            check: CheckProposal {
                payee: 0,
                asset: AssetId::Cred,
                send_max: 500,
                deliver_min: 100,
                due_turn: 2,
                expiration_turn: 4,
            },
        }),
        "create check bounce",
    );
    let escrow_id = engine.state().next_escrow_id;
    must_ok(
        engine.apply_market_action(MarketAction::CreateEscrow {
            seat: 2,
            escrow: EscrowProposal {
                destination: 0,
                asset: AssetId::Cred,
                amount: 300,
                finish_after_turn: Some(2),
                cancel_after_turn: 4,
                condition_hash: None,
                bond: None,
            },
        }),
        "create escrow",
    );

    let obs0 = must_ok(engine.observe_seat(Seat(0)), "obs0");
    let pub0 = obs0.to_public_preview_state();

    let swap = AtomicSwapProposal {
        swap_id: 1,
        maker: 0,
        taker: 1,
        maker_asset: AssetId::Cred,
        maker_amount: 500,
        taker_asset: AssetId::Raw(1),
        taker_amount: 500,
        domain_id: None,
    };
    let dual = DualEscrowProposal {
        proposal_id: 2,
        buyer: 0,
        seller: 1,
        good: AssetId::Raw(1),
        required_units: 500,
        payment_cred: 600,
        buyer_bond_cred: 300,
        seller_bond_cred: 300,
        deadline_turn: 2,
        domain_id: None,
    };

    let req = HumanWhatIfRequest {
        seat: 0,
        market_actions: vec![
            // Valid buy
            MarketAction::TownImport {
                seat: 0,
                good: AssetId::Raw(3),
                amount: 500,
            },
            // Refused action (Seat 0 cannot cancel Seat 1's offer)
            MarketAction::CancelOffer { seat: 0, offer_id },
        ],
        cancel_lock_at: None,
        bundle: SealedOrderBundle {
            converter_runs: vec![ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            }],
            project_deliveries: vec![ProjectDeliveryOrder {
                project_id: 1,
                batches: 1,
            }],
            atomic_swaps: vec![swap.clone()],
            dual_escrows: vec![dual.clone()],
            check_cashes: vec![check_due_id, check_bounce_id, 999_999],
            ..SealedOrderBundle::default()
        },
        tie_mode: PreviewTieMode::PessimisticForSeat,
        assumptions: CounterpartyAssumptions {
            assumed_cancelled_offer_ids: vec![offer_id],
            assumed_peer_market_actions: vec![],
            assumed_cosigned_swaps: vec![swap],
            assumed_cosigned_dual_escrows: vec![dual],
            assumed_peer_deliveries: vec![(
                1,
                ContractDeliveryOrder {
                    counterparty: 0,
                    good: AssetId::Raw(1),
                    gross_amount: 600,
                    domain_id: None,
                    dual_escrow_id: None,
                },
            )],
            assumed_peer_escrow_finishes: vec![(
                2,
                EscrowFinishUnlock {
                    escrow_id,
                    preimage: None,
                },
            )],
            assumed_bounced_check_ids: vec![check_bounce_id],
            assumed_peer_bundles: BTreeMap::new(),
        },
    };

    let res = must_ok(preview_what_if(&pub0, &req), "preview_what_if");
    assert_eq!(res.refused_market_actions.len(), 1);
    assert_eq!(res.refused_market_actions[0].action_index, 1);
    assert_eq!(res.refused_market_actions[0].error_code, "UNKNOWN_OBJECT");
    assert_eq!(res.swap_orders.len(), 1);
    assert_eq!(res.swap_orders[0].outcome, PublicSwapOutcome::Settled);
    assert_eq!(res.check_orders.len(), 3);
    assert_eq!(res.check_orders[0].outcome, PublicCheckOutcome::Cashed);
    assert_eq!(res.check_orders[1].outcome, PublicCheckOutcome::Bounced);
    assert_eq!(
        res.check_orders[2].outcome,
        PublicCheckOutcome::IneligibleOrIgnored
    );
    assert!(
        res.public_fills
            .iter()
            .any(|f| f.kind == FillKind::AtomicSwap)
    );

    let res_json = must_ok(serde_json::to_string(&res), "serialize WhatIfPreview");
    assert!(!res_json.to_ascii_lowercase().contains("note"));
    assert!(!res_json.contains("post_state_digest"));
    assert!(!res_json.contains("newly_drawn_shocks"));

    // Verify neutral beacon helper matches seed 0.
    assert_eq!(
        preview_neutral_beacon(tid, Turn(1)),
        derive_turn_beacon(PREVIEW_NEUTRAL_BEACON_SEED, tid, Turn(1))
    );

    // AI preview rejects another seat's Phase-1 action and honors empty-peer semantics.
    let bad_ai = AiPreviewRequest {
        seat: 0,
        market_actions: vec![MarketAction::CancelOffer { seat: 1, offer_id }],
        bundle: SealedOrderBundle::default(),
    };
    assert_eq!(
        must_err(preview_seat_bundle(&pub0, &bad_ai), "bad ai").code(),
        "UNAUTHORIZED"
    );

    // After cancel_locked, `assumed_cancelled_offer_ids` is ignored; observe_seat_with_receipts
    // projects raw receipts without leaking and matches observe_seat identically.
    must_ok(engine.begin_cancel_lock(), "begin_cancel_lock");
    let obs_auto = must_ok(engine.observe_seat(Seat(0)), "obs locked auto");
    let obs_locked = must_ok(
        engine.observe_seat_with_receipts(Seat(0), &[live_r1]),
        "obs locked",
    );
    assert!(obs_locked.cancel_locked);
    assert_eq!(obs_locked, obs_auto);
    assert_eq!(engine.public_receipts().len(), 1);

    // Error paths on `preview_regent_bundle_for_seat`, `PublicPreviewState::to_engine`, and phases.
    assert!(
        engine
            .preview_regent_bundle_for_seat(Turn(0), Seat(0))
            .is_err()
    );
    assert!(
        engine
            .preview_regent_bundle_for_seat(Turn(99), Seat(0))
            .is_err()
    );
    assert!(
        engine
            .preview_regent_bundle_for_seat(Turn(1), Seat(9))
            .is_err()
    );
    assert_eq!(
        must_ok(
            engine.preview_regent_bundle_for_seat(Turn(1), Seat(1)),
            "ai regent"
        ),
        None
    );
    let mut bad_pub = obs0.to_public_preview_state();
    bad_pub.seats.pop();
    assert!(
        preview_seat_bundle(
            &bad_pub,
            &AiPreviewRequest {
                seat: 0,
                market_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            },
        )
        .is_err()
    );
}

#[test]
fn card2b_c2_c3_c5_and_auditor_accessor_regressions() {
    // 1. C2: TurnEngine omits `public_receipts` before first sweep, serializes it afterward while
    // preserving `state_digest()`, and still deserializes pre-Card-2 JSON without `public_receipts`.
    let tid = TableId(90_002);
    let (club_spec, deal, eval_seed) = must_ok(
        TableSpec::standard_club_chiral_ring_with_setup(tid, 4, 2),
        "club setup",
    );
    let mut eng = must_ok(TurnEngine::new(club_spec), "new");
    let pre_sweep_val = must_ok(serde_json::to_value(&eng), "pre-sweep engine json");
    assert!(pre_sweep_val.get("public_receipts").is_none());

    for s in 0..4_u8 {
        let mc = must_ok(
            compute_mandate_commitment(tid, Seat(s), &deal.seat_reveals[usize::from(s)]),
            "mc",
        );
        must_ok(eng.submit_mandate_commitment(Seat(s), mc), "submit mc");
    }

    for t in 1..=2_u8 {
        let mut bundles = BTreeMap::new();
        if t == 2 {
            // Let Seat 1 win OracleAuditor later? Not needed here; test Sweep 1 & 2 receipts.
            bundles.insert(0, SealedOrderBundle::default());
        }
        commit_and_reveal_subset(&mut eng, tid, t, &bundles);
        let (_, proof) = must_ok(eval_seed.prove_turn(tid, Turn(t)), "prove");
        must_ok(eng.sweep_turn_with_eval_vrf(&proof), "sweep");
    }
    assert_eq!(eng.public_receipts().len(), 2);
    let digest_live = must_ok(eng.state_digest(), "digest_live");
    let mut post_val = must_ok(serde_json::to_value(&eng), "post-sweep engine json");
    assert!(post_val.get("public_receipts").is_some());
    let roundtrip: TurnEngine = must_ok(serde_json::from_value(post_val.clone()), "rt engine");
    assert_eq!(roundtrip, eng);
    assert_eq!(roundtrip.public_receipts(), eng.public_receipts());
    assert_eq!(must_ok(roundtrip.state_digest(), "rt digest"), digest_live);

    if let Some(obj) = post_val.as_object_mut() {
        obj.remove("public_receipts");
    }
    let pre_card2_compat: TurnEngine = must_ok(
        serde_json::from_value(post_val),
        "pre-card2 TurnEngine json",
    );
    assert!(pre_card2_compat.public_receipts().is_empty());
    assert_eq!(
        must_ok(pre_card2_compat.state_digest(), "compat digest"),
        digest_live
    );

    // 2. C5: `SeasonReplayLog::from_club_json_slice` accepts Club logs and rejects Eval logs.
    let club_log = must_ok(
        SeasonReplayLog::from_rig_seed(
            eng.state().spec.clone(),
            2,
            deal,
            vec![None; 4],
            Vec::new(),
            vec![None; 4],
        ),
        "club log",
    );
    let club_bytes = must_ok(serde_json::to_vec(&club_log), "club bytes");
    assert!(SeasonReplayLog::from_club_json_slice(&club_bytes).is_ok());
    assert_eq!(
        must_err(
            SeasonReplayLog::from_eval_json_slice(&club_bytes),
            "eval rejects club log"
        )
        .code(),
        "INVALID_CONFIG"
    );

    // 3. `SeatObservation::auditor_hints` + `engine_internals::preview_bundle_in_mode` B2 forecast
    // check (`PreviewSteps0To7` omits the premature Turn-8 forecast while `Live` shows it).
    let (mut eval_spec, _, eval_seed2) = must_ok(
        TableSpec::standard_eval_chiral_ring_with_setup(TableId(90_003), 4, 3),
        "eval spec",
    );
    eval_spec.shocks = vec![ScheduledShockSpec {
        hint_turn: 7,
        forecast_turn: 8,
        realize_turn: 9,
        target_seat: 2,
        kind: ShockKind::SupplyFreeze,
        magnitude_bps: 2_200,
    }];
    let mut e_sh = must_ok(TurnEngine::new(eval_spec), "e_sh");
    for t in 1..=6_u8 {
        let mut b = BTreeMap::new();
        if t == 6 {
            let mut b1 = SealedOrderBundle::default();
            b1.charter_bids.push(CharterBidOrder {
                role: CharterRole::OracleAuditor,
                bid_cred: 300,
            });
            b.insert(1, b1);
        }
        commit_and_reveal_subset(&mut e_sh, TableId(90_003), t, &b);
        let (_, proof) = must_ok(eval_seed2.prove_turn(TableId(90_003), Turn(t)), "prove sh");
        must_ok(e_sh.sweep_turn_with_eval_vrf(&proof), "sweep sh");
    }
    // Turn 7 (`hint_turn == 7 < forecast_turn == 8`): Seat 1 holds OracleAuditor and sees 1
    // redacted hint (no magnitude), while Seat 0 sees empty.
    let obs_t7_s1 = must_ok(e_sh.observe_seat(Seat(1)), "obs_t7_s1");
    let obs_t7_s0 = must_ok(e_sh.observe_seat(Seat(0)), "obs_t7_s0");
    assert_eq!(obs_t7_s1.auditor_hints().len(), 1);
    assert_eq!(obs_t7_s1.auditor_hints()[0].hint_turn, 7);
    assert_eq!(obs_t7_s1.auditor_hints()[0].forecast_turn, 8);
    assert_eq!(obs_t7_s1.auditor_hints()[0].realize_turn, 9);
    assert!(obs_t7_s0.auditor_hints().is_empty());

    // Sweep Turn 7 -> Turn 8 (`forecast_turn == 8`): Turn-7 Auditor window has closed, and Turn-8
    // forecast must NOT appear prematurely in `SeatObservation` or `PreviewSteps0To7`!
    commit_and_reveal_subset(&mut e_sh, TableId(90_003), 7, &BTreeMap::new());
    let (_, proof7) = must_ok(eval_seed2.prove_turn(TableId(90_003), Turn(7)), "prove 7");
    must_ok(e_sh.sweep_turn_with_eval_vrf(&proof7), "sweep 7");

    let obs_t8 = must_ok(e_sh.observe_seat(Seat(1)), "obs_t8");
    assert!(obs_t8.auditor_hints().is_empty());
    assert!(obs_t8.public_forecasts.is_empty());
    let prev_safe = must_ok(
        engine_internals::preview_bundle_in_mode(
            &e_sh,
            Seat(1),
            &[],
            &SealedOrderBundle::default(),
            engine_internals::SweepMode::PreviewSteps0To7,
        ),
        "preview_bundle_in_mode safe",
    );
    assert!(prev_safe.sweep_receipt.public_forecasts.is_empty());
    let prev_live = must_ok(
        engine_internals::preview_bundle_in_mode(
            &e_sh,
            Seat(1),
            &[],
            &SealedOrderBundle::default(),
            engine_internals::SweepMode::Live,
        ),
        "preview_bundle_in_mode live",
    );
    assert_eq!(prev_live.sweep_receipt.public_forecasts.len(), 1);

    // 4. C3: Zero-unit `EscrowRefunded` fills are skipped at both DualEscrow default and expired
    // TokenEscrow refund sites (via synthetic zero-bond vault in deserialized TurnEngine state).
    let base_spec = must_ok(
        TableSpec::standard_chiral_ring(TableId(90_004), 4, 1),
        "spec",
    );
    let zero_eng = must_ok(TurnEngine::new(base_spec), "zero_eng");
    let mut eng_val = must_ok(serde_json::to_value(&zero_eng), "eng_val");
    eng_val["state"]["dual_escrows"] = serde_json::json!({
        "1": {
            "escrow_id": 1,
            "buyer": 0,
            "seller": 1,
            "good": { "Raw": 1 },
            "required_units": 500,
            "payment_cred": 500,
            "buyer_bond_cred": 0,
            "seller_bond_cred": 250,
            "baseline_delivered": 0,
            "deadline_turn": 1,
            "domain_id": null,
            "status": "Locked"
        }
    });
    eng_val["state"]["escrows"] = serde_json::json!({
        "2": {
            "escrow_id": 2,
            "sender": 2,
            "destination": 3,
            "amount_cred": 0,
            "finish_after_turn": null,
            "cancel_after_turn": 1,
            "condition_hash": null,
            "bond": null,
            "status": "Locked"
        }
    });
    let mut patched_eng: TurnEngine = must_ok(serde_json::from_value(eng_val), "patched_eng");
    commit_and_reveal_subset(&mut patched_eng, TableId(90_004), 1, &BTreeMap::new());
    let zero_rec = must_ok(
        patched_eng.sweep_turn(derive_turn_beacon(1, TableId(90_004), Turn(1))),
        "zero sweep",
    );
    assert!(
        zero_rec
            .fills
            .iter()
            .any(|f| f.kind == FillKind::EscrowForfeited && f.units == 250)
    );
    assert!(
        !zero_rec
            .fills
            .iter()
            .any(|f| f.kind == FillKind::EscrowRefunded && f.units == 0),
        "zero-unit EscrowRefunded fill must be skipped (C3): {:?}",
        zero_rec.fills
    );
}
