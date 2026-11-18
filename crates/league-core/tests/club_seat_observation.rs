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

use std::collections::BTreeMap;

use league_core::{
    AiPreviewRequest, AssetId, AtomicSwapProposal, CharterBidOrder, CharterRole, CheckProposal,
    ContractDeliveryOrder, ConverterRunOrder, CounterpartyAssumptions, DEFAULT_SHOCK_DRAW_TURNS,
    DEV_LAYOUT_SEED_MAX, DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_SIGNATURE_BYTES,
    DrandSchedule, DualEscrowProposal, EngineError, EngineState, EscrowFinishUnlock,
    EscrowProposal, FillKind, HARD_TURN_CAP, HumanWhatIfRequest, LoggedTurn, MarketAction,
    OfferProposal, OfferSide, OrderReveal, PREVIEW_NEUTRAL_BEACON_SEED, PreviewTieMode,
    ProjectDeliveryOrder, PublicCheckOutcome, PublicSwapOutcome, REPLAY_FORMAT_VERSION,
    RESERVED_TEST_LAYOUT_SEEDS, RegentDoctrine, ReplayBeaconMode, ReplayLayoutSeedError,
    ReplayOutcome, ReplayRecord, RevealSalt, ScheduledShockSpec, SealedEvalSeed, SealedOrderBundle,
    SeasonReplayLog, Seat, SeatIdentity, SeatObservation, ShockKind, StructuredError, SweepReceipt,
    TableId, TableMode, TableSpec, ToStructuredError, Turn, TurnEngine, TurnPhase, UNIT_SCALE,
    WhatIfPreview, attach_state_predicates, check_replay_format_version_slice,
    compute_mandate_commitment, compute_order_commitment, derive_turn_beacon,
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

/// Pure redaction guard verifying a serialized seat view against ground-truth `engine_state` (`B-N2`).
///
/// Inspects serialized JSON **values** only (never field names, and never searching `"magnitude"`):
/// - `(i)` forbids any shock record before `forecast_turn` completes (permitting at most one clean
///   `AuditorShockHint` strictly during `[hint_turn, forecast_turn)` when the observer holds active
///   `OracleAuditor`);
/// - `(ii)` classifies any record carrying `magnitude_bps` by record shape/turn values, forbidding
///   drawn, forecast, or Auditor-hint magnitudes anywhere and forbidding any magnitude before
///   `realize_turn` completes (while allowing realized `ActiveShock` records from `realize_turn` on);
/// - `(iii)` forbids `post_state_digest` anywhere in the serialized view as either `[u8; 32]` or hex.
pub fn check_seat_view(view_json: &str, engine_state: &EngineState) -> Result<(), Leak> {
    let root: serde_json::Value =
        serde_json::from_str(view_json).map_err(|err| Leak::InvalidViewJson(err.to_string()))?;
    let observer_seat = root
        .get("seat")
        .and_then(serde_json::Value::as_u64)
        .and_then(|s| u8::try_from(s).ok())
        .ok_or_else(|| Leak::InvalidViewJson("missing u8 observer seat".to_owned()))?;

    // Check (iii) first using canonical EngineState digest bytes/hex anywhere in JSON values.
    let digest = canonical_state_digest(engine_state)?;
    let digest_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if contains_digest_value(&root, &digest, &digest_hex) {
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

// =================================================================================================
// 2. §2.5 Steps 0–7 Equivalence `proptest` (All-Committed and Regent-Modeled Turns)
// =================================================================================================

fn assert_step0_to_7_matches_live(
    preview: &WhatIfPreview,
    live_engine: &TurnEngine,
    live_receipt: &SweepReceipt,
    seat: u8,
) {
    assert_eq!(preview.seat, seat);
    assert_eq!(preview.turn, live_receipt.turn);
    assert_eq!(
        preview.post_sweep_seat_state,
        live_engine.state().seats[usize::from(seat)],
        "full SeatState mismatch at turn {} seat {}",
        live_receipt.turn,
        seat
    );
    let expected_summary = league_core::Step0To7SweepSummary::from_sweep_receipt(live_receipt);
    assert_eq!(
        preview.step0_to_7_receipt, expected_summary,
        "Step 0-7 summary mismatch at turn {} seat {}",
        live_receipt.turn, seat
    );
    assert_eq!(preview.public_fills, live_receipt.fills);
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
        // Advance through Turn 4 (election) and Turn 6 (charter auction + post-Turn-2 shock draw)
        // checking equivalence on Turns 1..=6.
        for t in 1..=6_u8 {
            if engine.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
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
            let peer_action = MarketAction::TownImport {
                seat: (actor + 2) % n,
                good: AssetId::Raw((actor + 2) % n),
                amount: 200,
            };

            let mut actor_bundle = SealedOrderBundle::default();
            actor_bundle.converter_runs.push(ConverterRunOrder {
                recipe_idx: 0,
                runs: 1,
            });
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
                if regent_missing_peer && p == peer_absent {
                    // Case (b): uncommitted Human peer modeled via Standing Regent helper!
                    let preview_regent = must_ok(
                        engine.preview_regent_bundle_after_step0(
                            Turn(t),
                            Seat(p),
                            &[action.clone(), peer_action.clone()],
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
                    committed_bundles.insert(p, pb.clone());
                    assumed_peer_bundles.insert(p, pb);
                }
            }

            let req = HumanWhatIfRequest {
                seat: actor,
                market_actions: vec![action.clone()],
                cancel_lock_at: Some(1),
                bundle: actor_bundle.clone(),
                tie_mode: PreviewTieMode::NeutralSeed,
                assumptions: CounterpartyAssumptions {
                    assumed_peer_market_actions: vec![peer_action.clone()],
                    assumed_peer_bundles: assumed_peer_bundles.clone(),
                    ..CounterpartyAssumptions::default()
                },
            };
            let preview = must_ok(
                preview_what_if_with_beacon(&pub_state, &req, turn_beacon),
                "preview_what_if_with_beacon",
            );

            // Apply identical Phase-1 actions, cancel lock, and Phase-2 commit/reveal subset on live engine.
            must_ok(engine.apply_market_action(action), "live action");
            must_ok(engine.begin_cancel_lock(), "live cancel_lock");
            must_ok(engine.apply_market_action(peer_action), "live peer_action");
            commit_and_reveal_subset(&mut engine, tid, t, &committed_bundles);
            let live_receipt = must_ok(engine.sweep_turn_with_eval_vrf(&proof), "live sweep");

            if !regent_missing_peer {
                assert!(live_receipt.non_reveal_seats.is_empty());
                assert_step0_to_7_matches_live(&preview, &engine, &live_receipt, actor);
            } else {
                // In Case (b) where peer_absent omitted reveal and its Regent acted, seat state
                // and Step 0-7 economic/political counters match identically!
                prop_assert_eq!(
                    &preview.post_sweep_seat_state,
                    &engine.state().seats[usize::from(actor)]
                );
                prop_assert_eq!(preview.public_fills, live_receipt.fills);
                prop_assert_eq!(
                    preview.step0_to_7_receipt.batch_trades_cleared,
                    live_receipt.batch_trades_cleared
                );
                prop_assert_eq!(
                    preview.step0_to_7_receipt.converter_batches_run,
                    live_receipt.converter_batches_run
                );
                prop_assert_eq!(
                    preview.step0_to_7_receipt.total_upkeep_shortfall_debt,
                    live_receipt.total_upkeep_shortfall_debt
                );
                prop_assert_eq!(
                    preview.step0_to_7_receipt.total_spoilage_clawed_back,
                    live_receipt.total_spoilage_clawed_back
                );
                prop_assert_eq!(
                    preview.step0_to_7_receipt.elected_officers,
                    live_receipt.elected_officers
                );
                prop_assert_eq!(
                    preview.step0_to_7_receipt.charters_awarded,
                    live_receipt.charters_awarded
                );
                prop_assert_eq!(
                    preview.step0_to_7_receipt.town_import_cred_spent,
                    live_receipt.town_import_cred_spent
                );
            }
        }
    }
}

// =================================================================================================
// 3. Secret Non-Interference & `Commit`/`Reveal` Isolation `proptest` (§2.5 item 2, C4, B1)
// =================================================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn proptest_secret_non_interference_and_commit_reveal_isolation(
        raw_seed in 0_u64..10_000,
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

        // Unforecast pre-seeded shocks (`forecast_turn >= turn = 1`) must also remain non-interfering
        // on Turn 1 (even for an active OracleAuditor if hint_turn > turn).
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

        // Submit differing mandate commits for peers 1..n.
        for s in 0..n {
            let rev1 = &deal1.seat_reveals[usize::from(s)];
            let c1 = must_ok(compute_mandate_commitment(tid, Seat(s), rev1), "c1");
            must_ok(e1.submit_mandate_commitment(Seat(s), c1), "e1 mc");
            let rev2 = &deal2.seat_reveals[usize::from(s)];
            let c2 = must_ok(compute_mandate_commitment(tid, Seat(s), rev2), "c2");
            must_ok(e2.submit_mandate_commitment(Seat(s), c2), "e2 mc");
        }

        // Across TalkAndMarket, Commit (only peer 1 committed in e1, none in e2), and
        // Reveal (peer 1 reveals a converter+project bundle in e1 vs different bundle in e2),
        // Seat 0's SeatObservation, PublicPreviewState, preview_what_if, and preview_seat_bundle
        // MUST stay byte-identical!
        if phase_pick >= 1 {
            must_ok(e1.advance_to_commit(), "e1 commit");
            must_ok(e2.advance_to_commit(), "e2 commit");
            let b_peer1 = SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder { recipe_idx: 0, runs: 2 }],
                ..SealedOrderBundle::default()
            };
            let rev_p1 = OrderReveal {
                bundle: b_peer1.clone(),
                salt: RevealSalt([0x77; 32]),
            };
            let c_p1 = must_ok(compute_order_commitment(tid, Turn(1), Seat(1), &rev_p1), "c_p1");
            must_ok(e1.submit_order_commitment(Seat(1), c_p1), "e1 submit p1");

            let b_peer2 = SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder { recipe_idx: 0, runs: 1 }],
                ..SealedOrderBundle::default()
            };
            let rev_p2 = OrderReveal {
                bundle: b_peer2,
                salt: RevealSalt([alt_byte; 32]),
            };
            let c_p2 = must_ok(compute_order_commitment(tid, Turn(1), Seat(2), &rev_p2), "c_p2");
            must_ok(e2.submit_order_commitment(Seat(2), c_p2), "e2 submit p2");

            if phase_pick == 2 {
                must_ok(e1.advance_to_reveal(), "e1 reveal");
                must_ok(e2.advance_to_reveal(), "e2 reveal");
                must_ok(e1.submit_order_reveal(Seat(1), rev_p1), "e1 rev p1");
                must_ok(e2.submit_order_reveal(Seat(2), rev_p2), "e2 rev p2");
            }
        }

        let obs1 = must_ok(e1.observe_seat(Seat(0)), "obs1");
        let obs2 = must_ok(e2.observe_seat(Seat(0)), "obs2");
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
            seat: 0,
            market_actions: Vec::new(),
            bundle: SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder { recipe_idx: 0, runs: 1 }],
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
            seat: 0,
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
    assert!(check_replay_format_version_slice(br#"{"format_version":0}"#).is_err());
    assert!(check_replay_format_version_slice(br#"{"format_version":"2"}"#).is_err());
    assert!(serde_json::from_slice::<SeasonReplayLog>(&future_log_bytes).is_err());
    assert!(serde_json::from_slice::<ReplayOutcome>(&future_out_bytes).is_err());
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
    assert!(bad_pub.to_engine().is_err());
}
