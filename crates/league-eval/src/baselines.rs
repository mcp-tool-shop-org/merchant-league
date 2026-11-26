//! Deterministic scripted `"file"` baseline seat policies for Evaluation Mode:
//! `"ClubBot"`, `"random-legal"`, `"always-defect"`, and `"tit-for-tat"`.
//!
//! Each scripted baseline is a pure function of `(seed, observation, own_to_seat_history)` (never
//! invoking `league-eval`'s secret host state or reading another seat's `decisions/`, `from-seat/`,
//! or `relay/` files) and validates every order candidate through
//! [`league_core::preview_seat_bundle`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use league_core::{
    AssetId, ClubBot, EvalActSubmission, MarketAction, OfferProposal, OfferSide, PolicyChaChaRng,
    PublicSweepReceipt, SealedOrderBundle, SeatObservation, preview_seat_bundle,
};
use league_relay::{NegotiationInputs, UntrustedWhisper, preview_wording_slot};
use league_scorer::ExtractedTalkCommitment;
use league_seat_types::{
    AiPreviewRequest, BPS_SCALE, CartelDomain, CharterBidOrder, CharterRole, CheckStatus,
    ConverterRunOrder, CouncilPlatform, CouncilVoteBallot, CredOffer, CreditCheck, CreditTier,
    EscrowFinishUnlock, EscrowStatus, FillKind, IntentTarget, PendingWashFreeze, PrivateMandate,
    ProjectDeliveryOrder, PublicTableSpec, SeatState, TokenEscrowVault, UNIT_SCALE, Units,
    WhatIfPreview,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::error::EvalError;
use crate::records::CommunicationMode;
use crate::reply::{ParsedEvalMessage, parse_eval_talk_reply};
use crate::talk_extract::extract_talk_commitments_from_messages;
use crate::wording::{EVAL_MESSAGES_MAX_ITEMS, eval_wording_slot};

/// Domain separator for [`PolicyChaChaRng`] seed derivation across baseline policies.
pub const BASELINE_RNG_DOMAIN: &[u8] = b"league-eval/baseline/v1|";

fn mul_units_parity(amount: Units, parity_per_unit: Units) -> Units {
    if amount == 0 || parity_per_unit == 0 {
        return 0;
    }
    let prod =
        i128::from(amount).saturating_mul(i128::from(parity_per_unit)) / i128::from(UNIT_SCALE);
    Units::try_from(prod).unwrap_or_else(|_| {
        if prod.is_negative() {
            Units::MIN
        } else {
            Units::MAX
        }
    })
}

// --- Named fixed candidate-grid constants for `random-legal` (`docs/eval-harness.md` `D8`) ---

/// Offer quantities (in milli-units) sampled by `"random-legal"`.
pub const RANDOM_LEGAL_OFFER_AMOUNTS: [Units; 2] = [UNIT_SCALE, 2 * UNIT_SCALE];
/// Limit price multipliers (in basis points relative to table parity) sampled by `"random-legal"`.
pub const RANDOM_LEGAL_PRICE_BPS: [u32; 3] = [8_000, 10_000, 12_000];
/// Authority `TownImport` quantity (in milli-units) sampled by `"random-legal"`.
pub const RANDOM_LEGAL_IMPORT_AMOUNT: Units = UNIT_SCALE;
/// Voluntary `CRED` auto-debt repayment fractions (in basis points) sampled by `"random-legal"`.
pub const RANDOM_LEGAL_DEBT_FRACTIONS_BPS: [u32; 3] = [0, 5_000, 10_000];
/// Syndicate Charter bid amounts (in milli-`CRED`) sampled by `"random-legal"`.
pub const RANDOM_LEGAL_CHARTER_BID_CRED: [Units; 2] = [UNIT_SCALE, 3 * UNIT_SCALE];
/// Maximum `CancelOffer` / `CancelCheck` / `CancelExpiredEscrow` actions included in the `"random-legal"` grid.
pub const RANDOM_LEGAL_MAX_CANCEL_PER_KIND: usize = 4;
/// Maximum due checks / mature escrows included in the `"random-legal"` Phase-2 grid.
pub const RANDOM_LEGAL_MAX_CLAIM_CANDIDATES: usize = 4;
/// Maximum converter batch count sampled by `"random-legal"`.
pub const RANDOM_LEGAL_MAX_CONVERTER_RUNS: u32 = 8;
/// Maximum Syndicate project batch count sampled by `"random-legal"`.
pub const RANDOM_LEGAL_MAX_PROJECT_BATCHES: u32 = 4;
/// Exact fixed Phase-1 candidate grid cap per turn for `"random-legal"`.
pub const RANDOM_LEGAL_MAX_MARKET_GRID_SIZE: usize = 121;

// --- Named fixed candidate-grid constants for `always-defect` (`docs/eval-harness.md` `D8`) ---

/// Trade quantity (in whole units) proposed by `"always-defect"` in `Round::Talk`.
pub const ALWAYS_DEFECT_TALK_WHOLE_UNITS: Units = 1;
/// Ask markup multipliers (in basis points relative to parity) in `"always-defect"`'s Phase-1 grid.
pub const ALWAYS_DEFECT_ASK_MARKUP_BPS: [u32; 2] = [10_500, 12_000];
/// Maximum resting peer offers evaluated by `"always-defect"` for crossing in Phase 1.
pub const ALWAYS_DEFECT_MAX_CROSS_OFFERS: usize = 8;
/// Syndicate Charter bid (in milli-`CRED`) in `"always-defect"`'s Turn-`6n` grid.
pub const ALWAYS_DEFECT_CHARTER_BID_CRED: Units = UNIT_SCALE;
/// Exact fixed maximum number of combined `(market_actions, bundle)` candidates evaluated per turn by `"always-defect"`.
pub const ALWAYS_DEFECT_MAX_CANDIDATES: usize = 64;

/// Maximum per-unit whole-`CRED` rounding tolerance when matching resting offers for `tit-for-tat`.
pub const TIT_FOR_TAT_PRICE_TOLERANCE_MILLI_CRED: Units = UNIT_SCALE / 2;

/// Validated `"model"` policy selector for `"file"` backend seats in `seats.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvalFileSeatModel {
    /// Existing manual/bot baseline (`"ClubBot"`), retaining `D6` Seat-`0` `RegentDoctrine`.
    ClubBot,
    /// Seeded uniform legal random policy (`"random-legal"`).
    RandomLegal,
    /// Talk-promising, act-withholding one-turn equity maximizer (`"always-defect"`).
    AlwaysDefect,
    /// Seat-only reciprocal agreement keeper (`"tit-for-tat"`, refused under `silent`).
    TitForTat,
}

impl EvalFileSeatModel {
    /// Parse `"file"` seat `"model"` string, returning [`EvalError::UnsupportedFileSeatModel`] for unknown values.
    pub fn parse_for_seat(guild: &str, model: Option<&str>) -> Result<Self, EvalError> {
        match model {
            Some("ClubBot") => Ok(Self::ClubBot),
            Some("random-legal") => Ok(Self::RandomLegal),
            Some("always-defect") => Ok(Self::AlwaysDefect),
            Some("tit-for-tat") => Ok(Self::TitForTat),
            _ => Err(EvalError::UnsupportedFileSeatModel {
                guild: guild.to_owned(),
                model: model.map(ToOwned::to_owned),
            }),
        }
    }

    /// Canonical lowercase/label string recorded in `seat_kind` and error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClubBot => "ClubBot",
            Self::RandomLegal => "random-legal",
            Self::AlwaysDefect => "always-defect",
            Self::TitForTat => "tit-for-tat",
        }
    }

    /// Returns `true` iff this `"file"` seat keeps `D6` Seat-`0` `Human` Standing Regent behavior.
    ///
    /// All three scripted evaluation baselines (`random-legal`, `always-defect`, `tit-for-tat`) set
    /// `regent: None` in every seat (`0..=3`) so Sweep Step 4 never overrides their policy.
    #[must_use]
    pub const fn uses_standing_regent(self) -> bool {
        matches!(self, Self::ClubBot)
    }

    /// Returns `true` iff this baseline requires [`CommunicationMode::Talk`].
    #[must_use]
    pub const fn requires_talk(self) -> bool {
        matches!(self, Self::TitForTat)
    }
}

/// Peer reciprocation state tracked per opponent seat by `"tit-for-tat"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStance {
    /// Honour agreements with this peer.
    Cooperate,
    /// Retaliate for an unkept agreement or on-ledger breach on the preceding turn.
    Defect,
}

#[derive(Debug, Clone, Deserialize)]
struct HistoricalPreActView {
    #[allow(dead_code)]
    turn: u8,
    seats: Vec<SeatState>,
    offers: BTreeMap<u64, CredOffer>,
    checks: BTreeMap<u64, CreditCheck>,
    escrows: BTreeMap<u64, TokenEscrowVault>,
    domains: Vec<CartelDomain>,
    #[serde(default)]
    pending_wash_freezes: Vec<PendingWashFreeze>,
    #[allow(dead_code)]
    spec: PublicTableSpec,
}

/// Strictly seat-local per-turn history loaded solely from `to-seat/*<SelfGuild>*` files for `"tit-for-tat"`.
#[derive(Debug, Clone, Default)]
pub struct OwnToSeatHistory {
    /// Historical per-day pre-act observations visible to this seat (`day < current_day`).
    pre_act_by_day: BTreeMap<u8, HistoricalPreActView>,
    /// Pre-talk `SeatObservation` balances/offers JSON recovered from `day-DD-talk-<Self>.md` when present.
    talk_views_by_day: BTreeMap<u8, HistoricalPreActView>,
    /// Whispers delivered solely to this seat in `to-seat/day-DD-act-<Self>.negotiation.json`.
    delivered_whispers_by_day: BTreeMap<u8, Vec<UntrustedWhisper>>,
}

impl OwnToSeatHistory {
    /// Load only files in `to_seat_dir` addressed to `self_guild` for turns `1..=current_day`.
    ///
    /// Never reads `decisions/`, `from-seat/`, `relay/`, `checkpoints/`, or any peer's `to-seat/` file.
    pub fn load(to_seat_dir: &Path, self_guild: &str, current_day: u8) -> Result<Self, EvalError> {
        let mut hist = Self::default();
        for day in 1..=current_day {
            if day < current_day {
                let act_path = to_seat_dir.join(format!("day-{day:02}-act-{self_guild}.md"));
                if act_path.is_file() {
                    let raw = fs::read_to_string(&act_path).map_err(|e| {
                        EvalError::Io(format!("read own act view {}: {e}", act_path.display()))
                    })?;
                    let view = parse_seat_view_json(&raw, &act_path)?;
                    hist.pre_act_by_day.insert(day, view);
                }
            }
            let talk_path = to_seat_dir.join(format!("day-{day:02}-talk-{self_guild}.md"));
            if talk_path.is_file() {
                let raw = fs::read_to_string(&talk_path).map_err(|e| {
                    EvalError::Io(format!("read own talk view {}: {e}", talk_path.display()))
                })?;
                let view = parse_seat_view_json(&raw, &talk_path)?;
                hist.talk_views_by_day.insert(day, view);
            }
            let neg_path =
                to_seat_dir.join(format!("day-{day:02}-act-{self_guild}.negotiation.json"));
            if neg_path.is_file() {
                let raw = fs::read_to_string(&neg_path).map_err(|e| {
                    EvalError::Io(format!(
                        "read own negotiation file {}: {e}",
                        neg_path.display()
                    ))
                })?;
                let neg: NegotiationInputs = serde_json::from_str(&raw).map_err(|e| {
                    EvalError::Io(format!(
                        "parse own negotiation file {}: {e}",
                        neg_path.display()
                    ))
                })?;
                hist.delivered_whispers_by_day.insert(day, neg.whispers);
            }
        }
        Ok(hist)
    }
}

fn parse_seat_view_json(raw: &str, path: &Path) -> Result<HistoricalPreActView, EvalError> {
    let idx = raw.find('{').ok_or_else(|| {
        EvalError::Io(format!(
            "own view {} missing JSON SeatObservation object",
            path.display()
        ))
    })?;
    serde_json::from_str(&raw[idx..]).map_err(|e| {
        EvalError::Io(format!(
            "parse own SeatObservation JSON in {}: {e}",
            path.display()
        ))
    })
}

/// Derive the seeded deterministic [`PolicyChaChaRng`] for a baseline seat decision.
pub fn derive_baseline_rng(
    layout_seed: u64,
    obs: &SeatObservation,
    round_tag: u8,
    policy_tag: u8,
) -> Result<PolicyChaChaRng, EvalError> {
    let obs_bytes = serde_json::to_vec(obs)
        .map_err(|e| EvalError::Io(format!("encode SeatObservation for RNG seed: {e}")))?;
    let obs_digest: [u8; 32] = Sha256::digest(&obs_bytes).into();
    let mut hasher = Sha256::new();
    hasher.update(BASELINE_RNG_DOMAIN);
    hasher.update(layout_seed.to_le_bytes());
    hasher.update(obs.table_id.0.to_le_bytes());
    hasher.update([obs.turn, obs.seat, round_tag, policy_tag]);
    hasher.update(obs_digest);
    let seed: [u8; 32] = hasher.finalize().into();
    Ok(PolicyChaChaRng::from_seed_bytes(seed))
}

// --- Canonical Wire Reply Encoder ---

fn encode_asset(asset: AssetId) -> Value {
    let k_kind = preview_wording_slot("key_asset_kind");
    let k_idx = preview_wording_slot("key_good_index");
    match asset {
        AssetId::Cred => json!({
            k_kind: preview_wording_slot("val_asset_cred"),
            k_idx: Value::Null,
        }),
        AssetId::Raw(i) => json!({
            k_kind: preview_wording_slot("val_asset_raw"),
            k_idx: i,
        }),
        AssetId::Refined(i) => json!({
            k_kind: preview_wording_slot("val_asset_refined"),
            k_idx: i,
        }),
    }
}

fn encode_market_action(action: &MarketAction, communication: CommunicationMode) -> Value {
    let k_kind = preview_wording_slot("key_action_kind");
    let mut obj = Map::new();
    match action {
        MarketAction::PostOffer { order, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_post_offer")),
            );
            obj.insert(
                preview_wording_slot("key_side"),
                Value::String(match order.side {
                    OfferSide::Buy => preview_wording_slot("val_side_buy"),
                    OfferSide::Sell => preview_wording_slot("val_side_sell"),
                }),
            );
            obj.insert(preview_wording_slot("key_good"), encode_asset(order.good));
            obj.insert(preview_wording_slot("key_amount"), json!(order.amount));
            obj.insert(
                preview_wording_slot("key_price_per_unit_cred"),
                json!(order.price_per_unit_cred),
            );
            obj.insert(
                preview_wording_slot("key_domain_id"),
                order.domain_id.map_or(Value::Null, |d| json!(d)),
            );
            obj.insert(
                preview_wording_slot("key_is_hybrid"),
                json!(order.is_hybrid),
            );
        }
        MarketAction::CancelOffer { offer_id, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_cancel_offer")),
            );
            obj.insert(preview_wording_slot("key_offer_id"), json!(offer_id));
        }
        MarketAction::CreateCheck { check, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_create_check")),
            );
            obj.insert(preview_wording_slot("key_payee"), json!(check.payee));
            obj.insert(preview_wording_slot("key_asset"), encode_asset(check.asset));
            obj.insert(preview_wording_slot("key_send_max"), json!(check.send_max));
            obj.insert(
                preview_wording_slot("key_deliver_min"),
                json!(check.deliver_min),
            );
            obj.insert(preview_wording_slot("key_due_turn"), json!(check.due_turn));
            obj.insert(
                preview_wording_slot("key_expiration_turn"),
                json!(check.expiration_turn),
            );
        }
        MarketAction::CancelCheck { check_id, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_cancel_check")),
            );
            obj.insert(preview_wording_slot("key_check_id"), json!(check_id));
        }
        MarketAction::CreateEscrow { escrow, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_create_escrow")),
            );
            obj.insert(
                preview_wording_slot("key_destination"),
                json!(escrow.destination),
            );
            obj.insert(
                preview_wording_slot("key_asset"),
                encode_asset(escrow.asset),
            );
            obj.insert(preview_wording_slot("key_amount"), json!(escrow.amount));
            obj.insert(
                preview_wording_slot("key_finish_after_turn"),
                escrow.finish_after_turn.map_or(Value::Null, |t| json!(t)),
            );
            obj.insert(
                preview_wording_slot("key_cancel_after_turn"),
                json!(escrow.cancel_after_turn),
            );
            if communication == CommunicationMode::Talk {
                obj.insert(
                    preview_wording_slot("key_condition_hash"),
                    escrow
                        .condition_hash
                        .as_ref()
                        .map_or(Value::Null, |c| Value::String(hex::encode(c.0))),
                );
            }
            obj.insert(preview_wording_slot("key_bond"), Value::Null);
        }
        MarketAction::CancelExpiredEscrow { escrow_id, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_cancel_expired_escrow")),
            );
            obj.insert(preview_wording_slot("key_escrow_id"), json!(escrow_id));
        }
        MarketAction::RecordIntentPledge { pledge, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_record_intent_pledge")),
            );
            obj.insert(
                preview_wording_slot("key_pledge_id"),
                json!(pledge.pledge_id),
            );
            obj.insert(
                preview_wording_slot("key_recipient_seat"),
                json!(pledge.counterparty),
            );
            obj.insert(
                preview_wording_slot("key_target"),
                encode_intent_target(&pledge.target),
            );
            obj.insert(
                preview_wording_slot("key_honor_weight"),
                json!(pledge.honor_weight),
            );
            obj.insert(
                preview_wording_slot("key_memo_digest"),
                Value::String(hex::encode(pledge.memo_digest)),
            );
        }
        MarketAction::FlagWashFreeze { target_seat, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_flag_wash_freeze")),
            );
            obj.insert(preview_wording_slot("key_target_seat"), json!(target_seat));
        }
        MarketAction::StandForOffice { platform, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_stand_for_office")),
            );
            obj.insert(
                preview_wording_slot("key_proposed_tariffs_bps"),
                encode_tariffs(&platform.proposed_tariffs_bps),
            );
            obj.insert(
                preview_wording_slot("key_cartel_admissions"),
                encode_domain_pairs(&platform.cartel_admissions),
            );
            obj.insert(
                preview_wording_slot("key_cartel_revocations"),
                encode_domain_pairs(&platform.cartel_revocations),
            );
        }
        MarketAction::TownImport { good, amount, .. } => {
            obj.insert(
                k_kind,
                Value::String(preview_wording_slot("val_action_town_import")),
            );
            obj.insert(preview_wording_slot("key_good"), encode_asset(*good));
            obj.insert(preview_wording_slot("key_amount"), json!(amount));
        }
    }
    Value::Object(obj)
}

fn encode_intent_target(target: &IntentTarget) -> Value {
    let k_kind = preview_wording_slot("key_target_kind");
    match target {
        IntentTarget::VoteForSeat {
            office_turn,
            candidate,
        } => json!({
            k_kind: preview_wording_slot("val_target_vote_for_seat"),
            preview_wording_slot("key_office_turn"): office_turn,
            preview_wording_slot("key_candidate"): candidate,
        }),
        IntentTarget::PreserveCartelMembership {
            office_turn,
            domain_id,
            beneficiary,
        } => json!({
            k_kind: preview_wording_slot("val_target_preserve_cartel"),
            preview_wording_slot("key_office_turn"): office_turn,
            preview_wording_slot("key_domain_id"): domain_id,
            preview_wording_slot("key_beneficiary"): beneficiary,
        }),
        IntentTarget::RevealPreimage {
            escrow_id,
            condition,
        } => json!({
            k_kind: preview_wording_slot("val_target_reveal_preimage"),
            preview_wording_slot("key_escrow_id"): escrow_id,
            preview_wording_slot("key_condition"): hex::encode(condition.0),
        }),
    }
}

fn encode_tariffs(items: &[(u8, (u16, u16))]) -> Value {
    let k_seat = preview_wording_slot("wire_key_seat");
    let k_raw = preview_wording_slot("key_raw_bps");
    let k_ref = preview_wording_slot("key_refined_bps");
    Value::Array(
        items
            .iter()
            .map(|&(seat, (raw_bps, refined_bps))| {
                json!({
                    k_seat.as_str(): seat,
                    k_raw.as_str(): raw_bps,
                    k_ref.as_str(): refined_bps,
                })
            })
            .collect(),
    )
}

fn encode_domain_pairs(pairs: &[(u8, u8)]) -> Value {
    let k_dom = preview_wording_slot("key_domain_id");
    let k_seat = preview_wording_slot("wire_key_seat");
    Value::Array(
        pairs
            .iter()
            .map(|&(domain_id, seat)| {
                json!({
                    k_dom.as_str(): domain_id,
                    k_seat.as_str(): seat,
                })
            })
            .collect(),
    )
}

fn encode_bundle(bundle: &SealedOrderBundle, communication: CommunicationMode) -> Value {
    let converter_runs: Vec<Value> = bundle
        .converter_runs
        .iter()
        .map(|c| {
            json!({
                preview_wording_slot("key_recipe_idx"): c.recipe_idx,
                preview_wording_slot("key_runs"): c.runs,
            })
        })
        .collect();
    let project_deliveries: Vec<Value> = bundle
        .project_deliveries
        .iter()
        .map(|p| {
            json!({
                preview_wording_slot("key_project_id"): p.project_id,
                preview_wording_slot("key_batches"): p.batches,
            })
        })
        .collect();
    let atomic_swaps: Vec<Value> = bundle
        .atomic_swaps
        .iter()
        .map(|s| {
            json!({
                preview_wording_slot("key_swap_id"): s.swap_id,
                preview_wording_slot("key_maker_seat"): s.maker,
                preview_wording_slot("key_taker_seat"): s.taker,
                preview_wording_slot("key_maker_asset"): encode_asset(s.maker_asset),
                preview_wording_slot("key_maker_amount"): s.maker_amount,
                preview_wording_slot("key_taker_asset"): encode_asset(s.taker_asset),
                preview_wording_slot("key_taker_amount"): s.taker_amount,
                preview_wording_slot("key_domain_id"): s.domain_id,
            })
        })
        .collect();
    let contract_deliveries: Vec<Value> = bundle
        .contract_deliveries
        .iter()
        .map(|d| {
            json!({
                preview_wording_slot("key_recipient_seat"): d.counterparty,
                preview_wording_slot("key_good"): encode_asset(d.good),
                preview_wording_slot("key_gross_amount"): d.gross_amount,
                preview_wording_slot("key_domain_id"): d.domain_id,
                preview_wording_slot("key_dual_escrow_id"): d.dual_escrow_id,
            })
        })
        .collect();
    let dual_escrows: Vec<Value> = bundle
        .dual_escrows
        .iter()
        .map(|d| {
            json!({
                preview_wording_slot("key_proposal_id"): d.proposal_id,
                preview_wording_slot("key_buyer_seat"): d.buyer,
                preview_wording_slot("key_seller_seat"): d.seller,
                preview_wording_slot("key_good"): encode_asset(d.good),
                preview_wording_slot("key_required_units"): d.required_units,
                preview_wording_slot("key_payment_cred"): d.payment_cred,
                preview_wording_slot("key_buyer_bond_cred"): d.buyer_bond_cred,
                preview_wording_slot("key_seller_bond_cred"): d.seller_bond_cred,
                preview_wording_slot("key_deadline_turn"): d.deadline_turn,
                preview_wording_slot("key_domain_id"): d.domain_id,
            })
        })
        .collect();
    let escrow_finishes: Vec<Value> = bundle
        .escrow_finishes
        .iter()
        .map(|f| {
            let mut m = Map::new();
            m.insert(preview_wording_slot("key_escrow_id"), json!(f.escrow_id));
            if communication == CommunicationMode::Talk {
                m.insert(
                    preview_wording_slot("key_preimage"),
                    f.preimage
                        .as_ref()
                        .map_or(Value::Null, |p| Value::String(hex::encode(p.0))),
                );
            }
            Value::Object(m)
        })
        .collect();
    let council_vote = match bundle.council_vote {
        None => Value::Null,
        Some(ref v) => json!({
            preview_wording_slot("key_trade_minister"): v.trade_minister,
            preview_wording_slot("key_cartel_chair"): v.cartel_chair,
        }),
    };
    let charter_bids: Vec<Value> = bundle
        .charter_bids
        .iter()
        .map(|b| {
            let role_str = match b.role {
                CharterRole::ClearinghouseBroker => preview_wording_slot("val_charter_broker"),
                CharterRole::CustomsInspector => preview_wording_slot("val_charter_inspector"),
                CharterRole::OracleAuditor => preview_wording_slot("val_charter_auditor"),
            };
            json!({
                preview_wording_slot("key_role"): role_str,
                preview_wording_slot("key_bid_cred"): b.bid_cred,
            })
        })
        .collect();

    let mut bmap = Map::new();
    bmap.insert(
        preview_wording_slot("key_atomic_swaps"),
        Value::Array(atomic_swaps),
    );
    bmap.insert(
        preview_wording_slot("key_charter_bids"),
        Value::Array(charter_bids),
    );
    bmap.insert(
        preview_wording_slot("key_check_cashes"),
        json!(bundle.check_cashes),
    );
    bmap.insert(
        preview_wording_slot("key_contract_deliveries"),
        Value::Array(contract_deliveries),
    );
    bmap.insert(
        preview_wording_slot("key_converter_runs"),
        Value::Array(converter_runs),
    );
    bmap.insert(preview_wording_slot("key_council_vote"), council_vote);
    bmap.insert(
        preview_wording_slot("key_debt_repayment_cred"),
        json!(bundle.debt_repayment_cred),
    );
    bmap.insert(
        preview_wording_slot("key_dual_escrows"),
        Value::Array(dual_escrows),
    );
    bmap.insert(
        preview_wording_slot("key_escrow_finishes"),
        Value::Array(escrow_finishes),
    );
    bmap.insert(
        preview_wording_slot("key_project_deliveries"),
        Value::Array(project_deliveries),
    );
    Value::Object(bmap)
}

/// Serialize messages, `market_actions`, and `bundle` as a strict canonical Evaluation Mode reply
/// string parseable by `parse_eval_talk_reply`, `parse_eval_act_reply`, and
/// `parse_eval_silent_act_reply`.
pub fn encode_canonical_eval_reply(
    communication: CommunicationMode,
    messages: &[ParsedEvalMessage],
    submission: &EvalActSubmission,
) -> Result<String, EvalError> {
    let mut root = Map::new();
    root.insert(
        preview_wording_slot("key_bundle"),
        encode_bundle(&submission.bundle, communication),
    );
    let ma_vals: Vec<Value> = submission
        .market_actions
        .iter()
        .map(|a| encode_market_action(a, communication))
        .collect();
    root.insert(
        preview_wording_slot("key_market_actions"),
        Value::Array(ma_vals),
    );
    if communication == CommunicationMode::Talk {
        let k_msgs = eval_wording_slot("eval_messages_key");
        let k_to = eval_wording_slot("eval_to_guild_key");
        let k_body = eval_wording_slot("eval_body_key");
        let msgs_arr: Vec<Value> = messages
            .iter()
            .map(|m| json!({ k_to: m.to_guild, k_body: m.body }))
            .collect();
        root.insert(k_msgs.to_owned(), Value::Array(msgs_arr));
    }
    serde_json::to_string(&Value::Object(root))
        .map_err(|e| EvalError::Io(format!("encode canonical eval reply JSON: {e}")))
}

// --- Shared Preview Helpers ---

fn preview_clean(
    obs: &SeatObservation,
    market_actions: &[MarketAction],
    bundle: &SealedOrderBundle,
) -> Option<WhatIfPreview> {
    let pub_state = obs.to_public_preview_state();
    let req = AiPreviewRequest {
        seat: obs.seat,
        market_actions: market_actions.to_vec(),
        bundle: bundle.clone(),
    };
    let preview = preview_seat_bundle(&pub_state, &req).ok()?;
    preview.refused_market_actions.is_empty().then_some(preview)
}

fn clubbot_pass_a_only(
    bot: &ClubBot,
    obs: &SeatObservation,
    own_mandate: &PrivateMandate,
) -> EvalActSubmission {
    let dec = bot.decide_full_turn(obs, own_mandate);
    let mut actions = dec.pass_a_actions;
    for extra in dec.pass_b_actions {
        let mut candidate = actions.clone();
        candidate.push(extra.clone());
        if preview_clean(obs, &candidate, &dec.bundle).is_some() {
            actions = candidate;
        }
    }
    if preview_clean(obs, &actions, &dec.bundle).is_some() {
        EvalActSubmission {
            market_actions: actions,
            pass_b_actions: Vec::new(),
            bundle: dec.bundle,
        }
    } else {
        EvalActSubmission::default()
    }
}

fn due_solvent_check_ids(obs: &SeatObservation, free_override: Option<&[SeatState]>) -> Vec<u64> {
    let seats = free_override.unwrap_or(&obs.seats);
    let mut ids = Vec::new();
    for (&check_id, check) in &obs.checks {
        if check.payee == obs.seat
            && check.status == CheckStatus::Open
            && obs.turn >= check.due_turn
            && obs.turn < check.expiration_turn
        {
            let drawer_free =
                seats
                    .get(usize::from(check.drawer))
                    .map_or(0, |s| match check.asset {
                        AssetId::Cred => s.cred,
                        AssetId::Raw(g) => s.raw.get(usize::from(g)).copied().unwrap_or(0),
                        AssetId::Refined(r) => s.refined.get(usize::from(r)).copied().unwrap_or(0),
                    });
            if drawer_free >= check.deliver_min {
                ids.push(check_id);
            }
        }
    }
    ids
}

fn mature_plain_escrow_ids(obs: &SeatObservation) -> Vec<u64> {
    obs.escrows
        .iter()
        .filter_map(|(&id, esc)| {
            let participant = esc.sender == obs.seat || esc.destination == obs.seat;
            let finish_ok = esc
                .finish_after_turn
                .is_some_and(|ft| obs.turn >= ft && obs.turn < esc.cancel_after_turn);
            (esc.status == EscrowStatus::Locked
                && esc.condition_hash.is_none()
                && esc.bond.is_none()
                && participant
                && finish_ok)
                .then_some(id)
        })
        .collect()
}

fn max_feasible_converter_runs(
    recipe_idx: usize,
    obs: &SeatObservation,
    free_raw: &[Units],
    cap: u32,
) -> u32 {
    let Some(seat_cfg) = obs.spec.seats.get(usize::from(obs.seat)) else {
        return 0;
    };
    let Some(recipe) = seat_cfg.converters.get(recipe_idx) else {
        return 0;
    };
    let mut max_runs = recipe.max_runs_per_turn.min(cap);
    for &(in_g, per_run) in &recipe.inputs {
        if per_run <= 0 {
            return 0;
        }
        let held = free_raw.get(usize::from(in_g)).copied().unwrap_or(0);
        let cover = u32::try_from((held / per_run).max(0)).unwrap_or(u32::MAX);
        max_runs = max_runs.min(cover);
    }
    max_runs
}

fn max_feasible_project_batches(
    project_idx: usize,
    obs: &SeatObservation,
    free_refined: &[Units],
    spare_upkeep: bool,
    cap: u32,
) -> u32 {
    let Some(proj) = obs.spec.projects.get(project_idx) else {
        return 0;
    };
    let Some(seat_cfg) = obs.spec.seats.get(usize::from(obs.seat)) else {
        return 0;
    };
    let upkeep_idx = usize::from(seat_cfg.upkeep_good);
    let mut max_batches = cap;
    for &(in_g, per_batch) in &proj.inputs {
        if per_batch <= 0 {
            return 0;
        }
        let mut held = free_refined.get(usize::from(in_g)).copied().unwrap_or(0);
        if spare_upkeep && usize::from(in_g) == upkeep_idx {
            held = held.saturating_sub(seat_cfg.upkeep_amount);
        }
        let cover = u32::try_from((held / per_batch).max(0)).unwrap_or(u32::MAX);
        max_batches = max_batches.min(cover);
    }
    max_batches
}

fn all_tradeable_goods(seat_count: u8) -> Vec<AssetId> {
    let mut goods = Vec::with_capacity(usize::from(seat_count) * 2);
    for g in 0..seat_count {
        goods.push(AssetId::Raw(g));
    }
    for g in 0..seat_count {
        goods.push(AssetId::Refined(g));
    }
    goods
}

fn scaled_bps_price(parity: Units, bps: u32) -> Units {
    let p = (i128::from(parity.max(1)) * i128::from(bps)) / BPS_SCALE;
    i64::try_from(p.max(1)).unwrap_or(Units::MAX)
}

// --- Policy (a): `random-legal` ---

/// Decide one `Round::Act` submission for `"random-legal"` using the fixed grid (`D8`) and
/// deterministic [`PolicyChaChaRng`] seeded from `(layout_seed, obs)`.
pub fn decide_random_legal_act(
    layout_seed: u64,
    obs: &SeatObservation,
) -> Result<EvalActSubmission, EvalError> {
    let mut rng = derive_baseline_rng(layout_seed, obs, 1, 1)?;
    let empty_bundle = SealedOrderBundle::default();

    // 1. Fixed finite Phase-1 candidate grid bounded by `RANDOM_LEGAL_MAX_MARKET_GRID_SIZE`.
    let mut raw_ma_candidates: Vec<Vec<MarketAction>> =
        Vec::with_capacity(RANDOM_LEGAL_MAX_MARKET_GRID_SIZE);
    raw_ma_candidates.push(Vec::new());

    for &offer_id in obs
        .offers
        .iter()
        .filter_map(|(id, o)| (o.maker == obs.seat).then_some(id))
        .take(RANDOM_LEGAL_MAX_CANCEL_PER_KIND)
    {
        raw_ma_candidates.push(vec![MarketAction::CancelOffer {
            seat: obs.seat,
            offer_id,
        }]);
    }
    for &check_id in obs
        .checks
        .iter()
        .filter_map(|(id, c)| (c.drawer == obs.seat && c.status == CheckStatus::Open).then_some(id))
        .take(RANDOM_LEGAL_MAX_CANCEL_PER_KIND)
    {
        raw_ma_candidates.push(vec![MarketAction::CancelCheck {
            seat: obs.seat,
            check_id,
        }]);
    }
    for &escrow_id in obs
        .escrows
        .iter()
        .filter_map(|(id, e)| {
            (e.sender == obs.seat
                && e.status == EscrowStatus::Locked
                && obs.turn >= e.cancel_after_turn)
                .then_some(id)
        })
        .take(RANDOM_LEGAL_MAX_CANCEL_PER_KIND)
    {
        raw_ma_candidates.push(vec![MarketAction::CancelExpiredEscrow {
            seat: obs.seat,
            escrow_id,
        }]);
    }
    if obs.charters.iter().any(|c| {
        c.holder == obs.seat
            && c.role == CharterRole::CustomsInspector
            && c.expires_at_turn >= obs.turn
    }) {
        for target in 0..obs.spec.seat_count {
            if target != obs.seat {
                raw_ma_candidates.push(vec![MarketAction::FlagWashFreeze {
                    inspector: obs.seat,
                    target_seat: target,
                }]);
            }
        }
    }
    if obs.turn.is_multiple_of(4) {
        raw_ma_candidates.push(vec![MarketAction::StandForOffice {
            seat: obs.seat,
            platform: CouncilPlatform::default(),
        }]);
    }

    let goods = all_tradeable_goods(obs.spec.seat_count);
    for side in [OfferSide::Buy, OfferSide::Sell] {
        for &good in &goods {
            let parity = obs.parities.asset_parity(good).max(1);
            for &amount in &RANDOM_LEGAL_OFFER_AMOUNTS {
                for &bps in &RANDOM_LEGAL_PRICE_BPS {
                    if raw_ma_candidates.len() >= RANDOM_LEGAL_MAX_MARKET_GRID_SIZE {
                        break;
                    }
                    raw_ma_candidates.push(vec![MarketAction::PostOffer {
                        seat: obs.seat,
                        order: OfferProposal {
                            side,
                            good,
                            amount,
                            price_per_unit_cred: scaled_bps_price(parity, bps),
                            domain_id: None,
                            is_hybrid: false,
                        },
                    }]);
                }
            }
        }
    }
    for &good in &goods {
        if raw_ma_candidates.len() >= RANDOM_LEGAL_MAX_MARKET_GRID_SIZE {
            break;
        }
        raw_ma_candidates.push(vec![MarketAction::TownImport {
            seat: obs.seat,
            good,
            amount: RANDOM_LEGAL_IMPORT_AMOUNT,
        }]);
    }
    raw_ma_candidates.truncate(RANDOM_LEGAL_MAX_MARKET_GRID_SIZE);

    let mut legal_ma: Vec<(Vec<MarketAction>, WhatIfPreview)> = Vec::new();
    for ma in raw_ma_candidates {
        if let Some(preview) = preview_clean(obs, &ma, &empty_bundle) {
            legal_ma.push((ma, preview));
        }
    }
    if legal_ma.is_empty() {
        return Ok(EvalActSubmission::default());
    }
    let chosen_idx = rng.gen_index(legal_ma.len());
    let (chosen_ma, ma_preview) = legal_ma.swap_remove(chosen_idx);

    // 2. Sequential uniform legal draws across Phase-2 bundle coordinates conditioned on Pass-A preview.
    let mut bundle = SealedOrderBundle::default();
    let post_market = &ma_preview.post_market_seat_state;

    // Debt repayment grid
    if post_market.cred_debt > 0 && post_market.cred > 0 {
        let max_repay = post_market.cred.min(post_market.cred_debt);
        let mut repay_opts = vec![0_i64];
        for &bps in &RANDOM_LEGAL_DEBT_FRACTIONS_BPS {
            let val = ((i128::from(max_repay) * i128::from(bps)) / BPS_SCALE) as Units;
            if val > 0 && !repay_opts.contains(&val) {
                repay_opts.push(val);
            }
        }
        let pick = rng.gen_index(repay_opts.len());
        let mut cand = bundle.clone();
        cand.debt_repayment_cred = repay_opts[pick];
        if preview_clean(obs, &chosen_ma, &cand).is_some() {
            bundle = cand;
        }
    }

    // Converter runs grid (one chosen recipe index if any recipe has feasible runs)
    let n_recipes = obs
        .spec
        .seats
        .get(usize::from(obs.seat))
        .map_or(0, |s| s.converters.len());
    let mut avail_raw = post_market.raw.clone();
    if n_recipes > 0 {
        let mut conv_opts: Vec<Vec<ConverterRunOrder>> = vec![Vec::new()];
        for r_idx in 0..n_recipes {
            let max_r = max_feasible_converter_runs(
                r_idx,
                obs,
                &avail_raw,
                RANDOM_LEGAL_MAX_CONVERTER_RUNS,
            );
            for runs in 1..=max_r {
                conv_opts.push(vec![ConverterRunOrder {
                    recipe_idx: r_idx,
                    runs,
                }]);
            }
        }
        let legal_conv: Vec<Vec<ConverterRunOrder>> = conv_opts
            .into_iter()
            .filter(|cr| {
                let mut cand = bundle.clone();
                cand.converter_runs = cr.clone();
                preview_clean(obs, &chosen_ma, &cand).is_some()
            })
            .collect();
        if !legal_conv.is_empty() {
            let pick = rng.gen_index(legal_conv.len());
            bundle.converter_runs = legal_conv[pick].clone();
            if let Some(seat_cfg) = obs.spec.seats.get(usize::from(obs.seat)) {
                for cr in &bundle.converter_runs {
                    if let Some(rec) = seat_cfg.converters.get(cr.recipe_idx) {
                        for &(g, amt) in &rec.inputs {
                            if let Some(slot) = avail_raw.get_mut(usize::from(g)) {
                                *slot = slot.saturating_sub(amt.saturating_mul(i64::from(cr.runs)));
                            }
                        }
                    }
                }
            }
        }
    }

    // Project deliveries grid using post-converter simulated refined inventory
    let after_conv_ref = preview_clean(obs, &chosen_ma, &bundle).map_or_else(
        || post_market.refined.clone(),
        |p| p.post_sweep_seat_state.refined,
    );
    if !obs.spec.projects.is_empty() {
        let mut proj_opts: Vec<Vec<ProjectDeliveryOrder>> = vec![Vec::new()];
        for (p_idx, proj) in obs.spec.projects.iter().enumerate() {
            let max_b = max_feasible_project_batches(
                p_idx,
                obs,
                &after_conv_ref,
                false,
                RANDOM_LEGAL_MAX_PROJECT_BATCHES,
            );
            for batches in 1..=max_b {
                proj_opts.push(vec![ProjectDeliveryOrder {
                    project_id: proj.id,
                    batches,
                }]);
            }
        }
        let legal_proj: Vec<Vec<ProjectDeliveryOrder>> = proj_opts
            .into_iter()
            .filter(|pd| {
                let mut cand = bundle.clone();
                cand.project_deliveries = pd.clone();
                preview_clean(obs, &chosen_ma, &cand).is_some()
            })
            .collect();
        if !legal_proj.is_empty() {
            let pick = rng.gen_index(legal_proj.len());
            bundle.project_deliveries = legal_proj[pick].clone();
        }
    }

    // Due check cashes & mature plain escrows
    let due_checks: Vec<u64> = due_solvent_check_ids(obs, None)
        .into_iter()
        .take(RANDOM_LEGAL_MAX_CLAIM_CANDIDATES)
        .collect();
    if !due_checks.is_empty() {
        let mut opts = vec![Vec::new()];
        for cid in due_checks {
            opts.push(vec![cid]);
        }
        let pick = rng.gen_index(opts.len());
        let mut cand = bundle.clone();
        cand.check_cashes = opts[pick].clone();
        if preview_clean(obs, &chosen_ma, &cand).is_some() {
            bundle = cand;
        }
    }

    let plain_escrows: Vec<u64> = mature_plain_escrow_ids(obs)
        .into_iter()
        .take(RANDOM_LEGAL_MAX_CLAIM_CANDIDATES)
        .collect();
    if !plain_escrows.is_empty() {
        let mut opts = vec![Vec::new()];
        for eid in plain_escrows {
            opts.push(vec![EscrowFinishUnlock {
                escrow_id: eid,
                preimage: None,
            }]);
        }
        let pick = rng.gen_index(opts.len());
        let mut cand = bundle.clone();
        cand.escrow_finishes = opts[pick].clone();
        if preview_clean(obs, &chosen_ma, &cand).is_some() {
            bundle = cand;
        }
    }

    // Council vote on Turn 4n
    if obs.turn.is_multiple_of(4) && obs.spec.seat_count > 0 {
        let mut vote_opts: Vec<Option<CouncilVoteBallot>> = vec![None];
        for tm in 0..obs.spec.seat_count {
            for cc in 0..obs.spec.seat_count {
                vote_opts.push(Some(CouncilVoteBallot {
                    trade_minister: tm,
                    cartel_chair: cc,
                }));
            }
        }
        let pick = rng.gen_index(vote_opts.len());
        let mut cand = bundle.clone();
        cand.council_vote = vote_opts[pick].clone();
        if preview_clean(obs, &chosen_ma, &cand).is_some() {
            bundle = cand;
        }
    }

    // Charter bids on Turn 6n
    if obs.turn.is_multiple_of(6) {
        let mut bid_opts: Vec<Vec<CharterBidOrder>> = vec![Vec::new()];
        let roles = [
            CharterRole::ClearinghouseBroker,
            CharterRole::CustomsInspector,
            CharterRole::OracleAuditor,
        ];
        let avail_cred = post_market.cred.saturating_sub(bundle.debt_repayment_cred);
        for role in roles {
            for &bid_cred in &RANDOM_LEGAL_CHARTER_BID_CRED {
                if avail_cred >= bid_cred {
                    bid_opts.push(vec![CharterBidOrder { role, bid_cred }]);
                }
            }
        }
        let pick = rng.gen_index(bid_opts.len());
        let mut cand = bundle.clone();
        cand.charter_bids = bid_opts[pick].clone();
        if preview_clean(obs, &chosen_ma, &cand).is_some() {
            bundle = cand;
        }
    }

    if preview_clean(obs, &chosen_ma, &bundle).is_some() {
        Ok(EvalActSubmission {
            market_actions: chosen_ma,
            pass_b_actions: Vec::new(),
            bundle,
        })
    } else {
        Ok(EvalActSubmission::default())
    }
}

// --- Shared `talk_rules_v2` Commitment Helpers ---

fn whole_cred_near_parity(obs: &SeatObservation, good: AssetId, bps: u32) -> Units {
    let parity = obs.parities.asset_parity(good).max(UNIT_SCALE);
    let target = ((i128::from(parity) * i128::from(bps)) / BPS_SCALE).max(i128::from(UNIT_SCALE));
    let rounded = ((target + i128::from(UNIT_SCALE / 2)) / i128::from(UNIT_SCALE)).max(1);
    i64::try_from(rounded).unwrap_or(1)
}

fn format_talk_commitment_body(
    side: OfferSide,
    whole_units: Units,
    good: AssetId,
    whole_price: Units,
) -> Option<String> {
    let side_word = match side {
        OfferSide::Buy => "buy",
        OfferSide::Sell => "sell",
    };
    let good_word = match good {
        AssetId::Raw(idx) => format!("raw_{idx}"),
        AssetId::Refined(idx) => format!("refined_{idx}"),
        AssetId::Cred => return None,
    };
    Some(format!(
        "i agree to {side_word} {whole_units} {good_word} at {whole_price} cred."
    ))
}

// --- Policy (b): `always-defect` ---

/// Planned `Round::Talk` messages and promised `(is_sell, good)` keys for `"always-defect"`.
pub type AlwaysDefectTalkPlan = (Vec<ParsedEvalMessage>, BTreeSet<(bool, AssetId)>);

/// Compute the deterministic `Round::Talk` messages and underlying promised [`OfferProposal`]s for `"always-defect"`.
pub fn plan_always_defect_talk(
    obs: &SeatObservation,
    seated_guilds: &[String],
) -> Result<AlwaysDefectTalkPlan, EvalError> {
    let mut messages = Vec::new();
    let mut promised_pairs = BTreeSet::new();
    let mut sim_actions: Vec<MarketAction> = Vec::new();
    let empty_bundle = SealedOrderBundle::default();
    let goods = all_tradeable_goods(obs.spec.seat_count);

    for (peer_idx, peer_guild) in seated_guilds.iter().enumerate() {
        if messages.len() >= EVAL_MESSAGES_MAX_ITEMS {
            break;
        }
        let Ok(peer_u8) = u8::try_from(peer_idx) else {
            continue;
        };
        if peer_u8 == obs.seat {
            continue;
        }
        // Prefer promising to sell an owned raw good first, then any held tradeable good, then buying `Raw(peer)`.
        let preferred_sell = AssetId::Raw(obs.seat);
        let sell_candidates = std::iter::once(preferred_sell)
            .chain(goods.iter().copied().filter(|&g| g != preferred_sell));

        let mut picked: Option<(OfferProposal, String)> = None;
        for good in sell_candidates {
            if promised_pairs.contains(&(true, good)) {
                continue;
            }
            let whole_price = whole_cred_near_parity(obs, good, 10_000);
            let order = OfferProposal {
                side: OfferSide::Sell,
                good,
                amount: ALWAYS_DEFECT_TALK_WHOLE_UNITS * UNIT_SCALE,
                price_per_unit_cred: whole_price * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            };
            let mut trial = sim_actions.clone();
            trial.push(MarketAction::PostOffer {
                seat: obs.seat,
                order: order.clone(),
            });
            if preview_clean(obs, &trial, &empty_bundle).is_some()
                && let Some(body) = format_talk_commitment_body(
                    OfferSide::Sell,
                    ALWAYS_DEFECT_TALK_WHOLE_UNITS,
                    good,
                    whole_price,
                )
            {
                picked = Some((order, body));
                break;
            }
        }

        if picked.is_none() {
            let buy_good = AssetId::Raw(peer_u8);
            if !promised_pairs.contains(&(false, buy_good)) {
                let whole_price = whole_cred_near_parity(obs, buy_good, 10_000);
                let order = OfferProposal {
                    side: OfferSide::Buy,
                    good: buy_good,
                    amount: ALWAYS_DEFECT_TALK_WHOLE_UNITS * UNIT_SCALE,
                    price_per_unit_cred: whole_price * UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                };
                let mut trial = sim_actions.clone();
                trial.push(MarketAction::PostOffer {
                    seat: obs.seat,
                    order: order.clone(),
                });
                if preview_clean(obs, &trial, &empty_bundle).is_some()
                    && let Some(body) = format_talk_commitment_body(
                        OfferSide::Buy,
                        ALWAYS_DEFECT_TALK_WHOLE_UNITS,
                        buy_good,
                        whole_price,
                    )
                {
                    picked = Some((order, body));
                }
            }
        }

        if let Some((order, body)) = picked {
            let check = extract_talk_commitments_from_messages(
                peer_u8,
                seated_guilds,
                &[(obs.seat, peer_u8, body.clone())],
                obs,
            )?;
            if !check.is_empty() {
                sim_actions.push(MarketAction::PostOffer {
                    seat: obs.seat,
                    order: order.clone(),
                });
                promised_pairs.insert((order.side == OfferSide::Sell, order.good));
                messages.push(ParsedEvalMessage {
                    to_guild: peer_guild.clone(),
                    body,
                });
            }
        }
    }

    Ok((messages, promised_pairs))
}

fn action_matches_promised(action: &MarketAction, promised: &BTreeSet<(bool, AssetId)>) -> bool {
    matches!(
        action,
        MarketAction::PostOffer { order, .. }
            if promised.contains(&(order.side == OfferSide::Sell, order.good))
    )
}

fn score_one_turn_value(obs: &SeatObservation, preview: &WhatIfPreview) -> i128 {
    let post = &preview.post_sweep_seat_state;
    let mut equity =
        i128::from(post.project_equity) + i128::from(post.cred) - i128::from(post.cred_debt);
    for (idx, &units) in post.raw.iter().enumerate() {
        if let Ok(idx_u8) = u8::try_from(idx) {
            let parity = obs.parities.asset_parity(AssetId::Raw(idx_u8));
            equity += i128::from(mul_units_parity(units, parity));
        }
    }
    for (idx, &units) in post.refined.iter().enumerate() {
        if let Ok(idx_u8) = u8::try_from(idx) {
            let parity = obs.parities.asset_parity(AssetId::Refined(idx_u8));
            equity += i128::from(mul_units_parity(units, parity));
        }
    }
    // Subtract upkeep shortfall penalty explicitly when any new debt accrued.
    equity -= i128::from(preview.upkeep.new_shortfall_debt);
    equity
}

/// Decide one `Round::Act` submission for `"always-defect"`: withholds every promised trade and
/// picks the highest one-turn mark-to-market equity candidate from a fixed bounded grid
/// (`ALWAYS_DEFECT_MAX_CANDIDATES = 64`), tie-broken by canonical reply JSON.
pub fn decide_always_defect_act(
    layout_seed: u64,
    obs: &SeatObservation,
    own_mandate: &PrivateMandate,
    seated_guilds: &[String],
    communication: CommunicationMode,
    talk_view: Option<&SeatObservation>,
) -> Result<EvalActSubmission, EvalError> {
    let promised_pairs = if communication == CommunicationMode::Talk {
        let source_obs = talk_view.unwrap_or(obs);
        plan_always_defect_talk(source_obs, seated_guilds)?.1
    } else {
        BTreeSet::new()
    };

    // Mandatory cancellations of any resting offer matching a promised `(side, good)`.
    let mut base_cancels: Vec<MarketAction> = Vec::new();
    if !obs.cancel_locked {
        for (&offer_id, offer) in &obs.offers {
            if offer.maker == obs.seat
                && promised_pairs.contains(&(offer.side == OfferSide::Sell, offer.good))
            {
                base_cancels.push(MarketAction::CancelOffer {
                    seat: obs.seat,
                    offer_id,
                });
            }
        }
    }

    let bot = ClubBot::new(layout_seed);
    let mut bot_sub = clubbot_pass_a_only(&bot, obs, own_mandate);
    bot_sub
        .market_actions
        .retain(|a| !action_matches_promised(a, &promised_pairs));

    // Crossing candidates from resting peer offers that do NOT satisfy any own promise.
    let mut cross_actions = base_cancels.clone();
    for offer in obs
        .offers
        .values()
        .filter(|o| {
            o.maker != obs.seat && !promised_pairs.contains(&(o.side == OfferSide::Buy, o.good))
        })
        .take(ALWAYS_DEFECT_MAX_CROSS_OFFERS)
    {
        let opposite = match offer.side {
            OfferSide::Buy => OfferSide::Sell,
            OfferSide::Sell => OfferSide::Buy,
        };
        cross_actions.push(MarketAction::PostOffer {
            seat: obs.seat,
            order: OfferProposal {
                side: opposite,
                good: offer.good,
                amount: offer.remaining_units,
                price_per_unit_cred: offer.price_per_unit_cred,
                domain_id: offer.domain_id,
                is_hybrid: false,
            },
        });
    }

    let mut ma_variants: Vec<Vec<MarketAction>> = vec![base_cancels.clone()];
    if !bot_sub.market_actions.is_empty() {
        let mut combined = base_cancels.clone();
        combined.extend(bot_sub.market_actions.clone());
        ma_variants.push(combined);
    }
    if cross_actions.len() > base_cancels.len() {
        ma_variants.push(cross_actions);
    }

    // High-markup surplus asks for unpromised own goods
    let own_state = &obs.seats[usize::from(obs.seat)];
    for &bps in &ALWAYS_DEFECT_ASK_MARKUP_BPS {
        let good = AssetId::Raw(obs.seat);
        let free = own_state
            .raw
            .get(usize::from(obs.seat))
            .copied()
            .unwrap_or(0);
        if free >= UNIT_SCALE && !promised_pairs.contains(&(true, good)) {
            let mut variant = base_cancels.clone();
            let parity = obs.parities.asset_parity(good).max(1);
            variant.push(MarketAction::PostOffer {
                seat: obs.seat,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good,
                    amount: free,
                    price_per_unit_cred: scaled_bps_price(parity, bps),
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            ma_variants.push(variant);
        }
    }

    // Build fixed bundle variants
    let due_checks = due_solvent_check_ids(obs, None);
    let plain_escrows: Vec<EscrowFinishUnlock> = mature_plain_escrow_ids(obs)
        .into_iter()
        .map(|id| EscrowFinishUnlock {
            escrow_id: id,
            preimage: None,
        })
        .collect();
    let max_debt = own_state.cred.min(own_state.cred_debt);
    let vote = obs.turn.is_multiple_of(4).then_some(CouncilVoteBallot {
        trade_minister: obs.seat,
        cartel_chair: obs.seat,
    });
    let charter_bids =
        if obs.turn.is_multiple_of(6) && own_state.cred >= ALWAYS_DEFECT_CHARTER_BID_CRED {
            vec![CharterBidOrder {
                role: CharterRole::ClearinghouseBroker,
                bid_cred: ALWAYS_DEFECT_CHARTER_BID_CRED,
            }]
        } else {
            Vec::new()
        };

    let empty_domestic = SealedOrderBundle {
        check_cashes: due_checks.clone(),
        escrow_finishes: plain_escrows.clone(),
        council_vote: vote.clone(),
        ..SealedOrderBundle::default()
    };
    let mut greedy_bundle = bot_sub.bundle.clone();
    greedy_bundle.debt_repayment_cred = max_debt;
    greedy_bundle.council_vote = vote;
    greedy_bundle.charter_bids = charter_bids.clone();
    greedy_bundle.check_cashes = due_checks;
    greedy_bundle.escrow_finishes = plain_escrows;

    let mut no_proj_bundle = greedy_bundle.clone();
    no_proj_bundle.project_deliveries.clear();

    let bundle_variants = vec![
        SealedOrderBundle::default(),
        empty_domestic,
        bot_sub.bundle,
        no_proj_bundle,
        greedy_bundle,
    ];

    let mut grid: Vec<EvalActSubmission> = Vec::with_capacity(ALWAYS_DEFECT_MAX_CANDIDATES);
    for ma in &ma_variants {
        for b in &bundle_variants {
            if grid.len() >= ALWAYS_DEFECT_MAX_CANDIDATES {
                break;
            }
            grid.push(EvalActSubmission {
                market_actions: ma.clone(),
                pass_b_actions: Vec::new(),
                bundle: b.clone(),
            });
        }
    }

    let mut best: Option<(i128, String, EvalActSubmission)> = None;
    for cand in grid {
        let Some(preview) = preview_clean(obs, &cand.market_actions, &cand.bundle) else {
            continue;
        };
        let score = score_one_turn_value(obs, &preview);
        let canon = encode_canonical_eval_reply(communication, &[], &cand)?;
        let replace = match &best {
            None => true,
            Some((best_score, best_canon, _)) => {
                score > *best_score || (score == *best_score && canon < *best_canon)
            }
        };
        if replace {
            best = Some((score, canon, cand));
        }
    }

    Ok(best.map_or_else(EvalActSubmission::default, |(_, _, sub)| sub))
}

// --- Policy (c): `tit-for-tat` ---

fn seat_holds_free_for_offer(state: &SeatState, order: &OfferProposal) -> bool {
    match order.side {
        OfferSide::Sell => match order.good {
            AssetId::Raw(g) => state.raw.get(usize::from(g)).copied().unwrap_or(0) >= order.amount,
            AssetId::Refined(r) => {
                state.refined.get(usize::from(r)).copied().unwrap_or(0) >= order.amount
            }
            AssetId::Cred => false,
        },
        OfferSide::Buy => state.cred >= mul_units_parity(order.amount, order.price_per_unit_cred),
    }
}

fn commitment_applicable_in_view(
    view: &HistoricalPreActView,
    proposer: u8,
    order: &OfferProposal,
) -> bool {
    let Some(state) = view.seats.get(usize::from(proposer)) else {
        return false;
    };
    if state.wash_frozen_until_turn >= view.turn {
        return false;
    }
    if view
        .pending_wash_freezes
        .iter()
        .any(|f| f.target_seat == proposer && f.turn == view.turn)
    {
        return false;
    }
    if let Some(domain_id) = order.domain_id {
        if state.credit_tier == CreditTier::D {
            return false;
        }
        let in_domain = view
            .domains
            .iter()
            .any(|d| d.domain_id == domain_id && d.members.contains(&proposer));
        if !in_domain {
            return false;
        }
    }
    seat_holds_free_for_offer(state, order)
}

fn price_satisfies_promise(side: OfferSide, actual_price: Units, promised_price: Units) -> bool {
    let tol = TIT_FOR_TAT_PRICE_TOLERANCE_MILLI_CRED;
    match side {
        // Promised to sell at `promised_price`: at-or-lower ask is at-or-better for buyer.
        OfferSide::Sell => actual_price <= promised_price.saturating_add(tol),
        // Promised to buy at `promised_price`: at-or-higher bid is at-or-better for seller.
        OfferSide::Buy => actual_price.saturating_add(tol) >= promised_price,
    }
}

fn commitment_satisfied_by_fills_or_resting_offer(
    commitment: &ExtractedTalkCommitment,
    turn: u8,
    self_seat: u8,
    receipt: &PublicSweepReceipt,
    next_offers: &BTreeMap<u64, CredOffer>,
) -> bool {
    let MarketAction::PostOffer { ref order, .. } = commitment.offered_action else {
        return false;
    };
    let peer = commitment.proposer_seat;
    let filled_units: Units = receipt
        .fills
        .iter()
        .filter(|f| {
            let pair_ok = match order.side {
                OfferSide::Sell => f.from == Some(peer) && f.to == self_seat,
                OfferSide::Buy => f.from == Some(self_seat) && f.to == peer,
            };
            let good_ok = f.good == Some(order.good);
            let kind_ok = matches!(
                f.kind,
                FillKind::BatchFill | FillKind::AtomicSwap | FillKind::ContractDelivery
            );
            let price_ok = f
                .unit_price_cred
                .is_some_and(|p| price_satisfies_promise(order.side, p, order.price_per_unit_cred));
            pair_ok && good_ok && kind_ok && price_ok
        })
        .map(|f| f.units)
        .sum();
    if filled_units >= order.amount {
        return true;
    }
    next_offers.values().any(|o| {
        o.maker == peer
            && o.created_turn == turn
            && o.side == order.side
            && o.good == order.good
            && o.remaining_units >= order.amount
            && price_satisfies_promise(order.side, o.price_per_unit_cred, order.price_per_unit_cred)
    })
}

/// Deterministically judge per-peer cooperation stances on `current_day` for `"tit-for-tat"` from
/// solely `history` (this seat's own `to-seat/` inputs) and `current_obs` (`SeatObservation` + `public_receipts`).
pub fn compute_tit_for_tat_stances(
    history: &OwnToSeatHistory,
    current_obs: &SeatObservation,
    seated_guilds: &[String],
) -> Result<BTreeMap<u8, PeerStance>, EvalError> {
    let self_seat = current_obs.seat;
    let seat_count = current_obs.spec.seat_count;
    let mut stances = BTreeMap::new();
    for p in 0..seat_count {
        if p != self_seat {
            stances.insert(p, PeerStance::Cooperate);
        }
    }

    let receipts_by_turn: BTreeMap<u8, &PublicSweepReceipt> = current_obs
        .public_receipts
        .iter()
        .map(|r| (r.turn, r))
        .collect();

    for k in 1..current_obs.turn {
        let Some(receipt_k) = receipts_by_turn.get(&k) else {
            continue;
        };
        let Some(view_k) = history.pre_act_by_day.get(&k) else {
            continue;
        };
        let next_view_opt = if k + 1 < current_obs.turn {
            history.pre_act_by_day.get(&(k + 1))
        } else {
            None
        };
        let next_offers: &BTreeMap<u64, CredOffer> = next_view_opt
            .map(|v| &v.offers)
            .unwrap_or(&current_obs.offers);
        let next_seats: &[SeatState] = next_view_opt
            .map(|v| v.seats.as_slice())
            .unwrap_or(&current_obs.seats);

        let delivered = history
            .delivered_whispers_by_day
            .get(&k)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        for p in 0..seat_count {
            if p == self_seat {
                continue;
            }
            let Some(peer_guild) = seated_guilds.get(usize::from(p)) else {
                continue;
            };
            let peer_msgs: Vec<(u8, u8, String)> = delivered
                .iter()
                .filter(|w| w.from_guild == *peer_guild)
                .map(|w| (p, self_seat, w.body.clone()))
                .collect();
            let extracted = extract_talk_commitments_from_messages(
                self_seat,
                seated_guilds,
                &peer_msgs,
                current_obs,
            )?;
            let applicable: Vec<ExtractedTalkCommitment> = extracted
                .into_iter()
                .filter(|c| match &c.offered_action {
                    MarketAction::PostOffer { order, .. } => {
                        commitment_applicable_in_view(view_k, p, order)
                    }
                    _ => false,
                })
                .collect();

            let pledge_broke = receipt_k
                .broken_promises
                .iter()
                .any(|bp| bp.promisor == p && bp.counterparty == self_seat);
            let check_bounced = receipt_k.checks_bounced.iter().any(|cid| {
                view_k
                    .checks
                    .get(cid)
                    .is_some_and(|c| c.drawer == p && c.payee == self_seat)
            });
            let bond_breached = next_seats.get(usize::from(p)).is_some_and(|ns| {
                let prev = view_k
                    .seats
                    .get(usize::from(p))
                    .map_or(0, |ps| ps.bond_breaches);
                ns.bond_breaches > prev
                    && view_k.escrows.values().any(|e| {
                        (e.sender == p && e.destination == self_seat)
                            || (e.destination == p && e.sender == self_seat)
                    })
            });

            if pledge_broke || check_bounced || bond_breached {
                stances.insert(p, PeerStance::Defect);
                continue;
            }
            if !applicable.is_empty() {
                let all_kept = applicable.iter().all(|c| {
                    commitment_satisfied_by_fills_or_resting_offer(
                        c,
                        k,
                        self_seat,
                        receipt_k,
                        next_offers,
                    )
                });
                stances.insert(
                    p,
                    if all_kept {
                        PeerStance::Cooperate
                    } else {
                        PeerStance::Defect
                    },
                );
            }
        }
    }

    Ok(stances)
}

#[derive(Debug, Clone)]
struct TitForTatPlannedPromise {
    peer_seat: u8,
    peer_guild: String,
    order: OfferProposal,
    body: String,
}

fn plan_tit_for_tat_promises_for_state(
    seats: &[SeatState],
    obs: &SeatObservation,
    seated_guilds: &[String],
) -> Result<Vec<TitForTatPlannedPromise>, EvalError> {
    let self_seat = obs.seat;
    let seat_count = obs.spec.seat_count;
    let Some(own_state) = seats.get(usize::from(self_seat)) else {
        return Ok(Vec::new());
    };
    let mut rem_cred = own_state.cred;
    let mut rem_own_raw = own_state
        .raw
        .get(usize::from(self_seat))
        .copied()
        .unwrap_or(0);
    let mut out = Vec::new();

    for p in 0..seat_count {
        if out.len() >= EVAL_MESSAGES_MAX_ITEMS {
            break;
        }
        if p == self_seat {
            continue;
        }
        let Some(peer_guild) = seated_guilds.get(usize::from(p)) else {
            continue;
        };
        let peer_raw = AssetId::Raw(p);
        let buy_whole_price = whole_cred_near_parity(obs, peer_raw, 10_000);
        let buy_cost = buy_whole_price.saturating_mul(UNIT_SCALE);
        let chosen = if rem_cred >= buy_cost {
            rem_cred = rem_cred.saturating_sub(buy_cost);
            Some((
                OfferProposal {
                    side: OfferSide::Buy,
                    good: peer_raw,
                    amount: UNIT_SCALE,
                    price_per_unit_cred: buy_whole_price * UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
                format_talk_commitment_body(OfferSide::Buy, 1, peer_raw, buy_whole_price),
            ))
        } else if rem_own_raw >= UNIT_SCALE {
            let own_raw = AssetId::Raw(self_seat);
            let sell_whole_price = whole_cred_near_parity(obs, own_raw, 10_000);
            rem_own_raw = rem_own_raw.saturating_sub(UNIT_SCALE);
            Some((
                OfferProposal {
                    side: OfferSide::Sell,
                    good: own_raw,
                    amount: UNIT_SCALE,
                    price_per_unit_cred: sell_whole_price * UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
                format_talk_commitment_body(OfferSide::Sell, 1, own_raw, sell_whole_price),
            ))
        } else {
            None
        };

        if let Some((order, Some(body))) = chosen {
            let extracted = extract_talk_commitments_from_messages(
                p,
                seated_guilds,
                &[(self_seat, p, body.clone())],
                obs,
            )?;
            if !extracted.is_empty() {
                out.push(TitForTatPlannedPromise {
                    peer_seat: p,
                    peer_guild: peer_guild.clone(),
                    order,
                    body,
                });
            }
        }
    }

    Ok(out)
}

/// Plan `Round::Talk` messages for `"tit-for-tat"` strictly from `obs` and `seated_guilds`.
pub fn decide_tit_for_tat_talk(
    obs: &SeatObservation,
    seated_guilds: &[String],
) -> Result<Vec<ParsedEvalMessage>, EvalError> {
    let promises = plan_tit_for_tat_promises_for_state(&obs.seats, obs, seated_guilds)?;
    Ok(promises
        .into_iter()
        .map(|p| ParsedEvalMessage {
            to_guild: p.peer_guild,
            body: p.body,
        })
        .collect())
}

/// Decide `Round::Act` orders for `"tit-for-tat"` strictly from `(history, obs, own_mandate)`.
///
/// Honours agreements with peers in [`PeerStance::Cooperate`] (and crosses incoming applicable
/// offers from cooperating peers), while withholding promised orders and cancelling own stale
/// matching offers against any peer in [`PeerStance::Defect`].
pub fn decide_tit_for_tat_act(
    layout_seed: u64,
    history: &OwnToSeatHistory,
    obs: &SeatObservation,
    own_mandate: &PrivateMandate,
    seated_guilds: &[String],
) -> Result<EvalActSubmission, EvalError> {
    let stances = compute_tit_for_tat_stances(history, obs, seated_guilds)?;
    let talk_seats = history
        .talk_views_by_day
        .get(&obs.turn)
        .map_or(obs.seats.as_slice(), |v| v.seats.as_slice());
    let planned_promises = plan_tit_for_tat_promises_for_state(talk_seats, obs, seated_guilds)?;

    let mut actions: Vec<MarketAction> = Vec::new();
    let empty_bundle = SealedOrderBundle::default();

    // 1. Retaliatory cancellations against defecting peers on matching goods (when not cancel-locked).
    let mut blocked_goods_by_defector: BTreeSet<(bool, AssetId)> = BTreeSet::new();
    for p in &planned_promises {
        if stances.get(&p.peer_seat).copied() == Some(PeerStance::Defect) {
            blocked_goods_by_defector.insert((p.order.side == OfferSide::Sell, p.order.good));
            if !obs.cancel_locked {
                for (&offer_id, offer) in &obs.offers {
                    if offer.maker == obs.seat
                        && offer.side == p.order.side
                        && offer.good == p.order.good
                    {
                        actions.push(MarketAction::CancelOffer {
                            seat: obs.seat,
                            offer_id,
                        });
                    }
                }
            }
        }
    }

    // 2. Keep promised offers toward cooperating peers (`PeerStance::Cooperate`).
    let mut covered_incoming_pairs: BTreeSet<(u8, bool, AssetId)> = BTreeSet::new();
    for p in &planned_promises {
        if stances.get(&p.peer_seat).copied() == Some(PeerStance::Cooperate)
            && !blocked_goods_by_defector.contains(&(p.order.side == OfferSide::Sell, p.order.good))
        {
            let mut trial = actions.clone();
            trial.push(MarketAction::PostOffer {
                seat: obs.seat,
                order: p.order.clone(),
            });
            if preview_clean(obs, &trial, &empty_bundle).is_some() {
                actions = trial;
                covered_incoming_pairs.insert((
                    p.peer_seat,
                    p.order.side == OfferSide::Sell,
                    p.order.good,
                ));
            }
        }
    }

    // 3. Cross incoming applicable `talk_rules_v2` commitments delivered in today's `negotiation.json`
    //    from cooperating peers when parity-aligned and legal.
    if let Some(delivered) = history.delivered_whispers_by_day.get(&obs.turn) {
        let mut today_msgs = Vec::new();
        for w in delivered {
            if let Some(peer_idx) = seated_guilds.iter().position(|g| g == &w.from_guild)
                && let Ok(peer_u8) = u8::try_from(peer_idx)
                && peer_u8 != obs.seat
            {
                today_msgs.push((peer_u8, obs.seat, w.body.clone()));
            }
        }
        let incoming =
            extract_talk_commitments_from_messages(obs.seat, seated_guilds, &today_msgs, obs)?;
        for comm in incoming {
            if stances.get(&comm.proposer_seat).copied() != Some(PeerStance::Cooperate) {
                continue;
            }
            let MarketAction::PostOffer { order, .. } = comm.offered_action else {
                continue;
            };
            let opp_side = match order.side {
                OfferSide::Buy => OfferSide::Sell,
                OfferSide::Sell => OfferSide::Buy,
            };
            let opp_is_sell = opp_side == OfferSide::Sell;
            if blocked_goods_by_defector.contains(&(opp_is_sell, order.good)) {
                continue;
            }
            if covered_incoming_pairs.contains(&(comm.proposer_seat, opp_is_sell, order.good)) {
                continue;
            }
            let parity = obs.parities.asset_parity(order.good).max(1);
            let within_band = match opp_side {
                OfferSide::Buy => order.price_per_unit_cred <= scaled_bps_price(parity, 15_000),
                OfferSide::Sell => order.price_per_unit_cred >= scaled_bps_price(parity, 7_000),
            };
            if !within_band {
                continue;
            }
            let counter = MarketAction::PostOffer {
                seat: obs.seat,
                order: OfferProposal {
                    side: opp_side,
                    good: order.good,
                    amount: order.amount,
                    price_per_unit_cred: order.price_per_unit_cred,
                    domain_id: order.domain_id,
                    is_hybrid: false,
                },
            };
            let mut trial = actions.clone();
            trial.push(counter);
            if preview_clean(obs, &trial, &empty_bundle).is_some() {
                actions = trial;
                covered_incoming_pairs.insert((comm.proposer_seat, opp_is_sell, order.good));
            }
        }
    }

    // 4. Add domestic `ClubBot` bundle (`converter_runs`, `project_deliveries`, `debt_repayment_cred`,
    //    `check_cashes`, `council_vote`, `charter_bids`), verified against `actions` via preview.
    let bot = ClubBot::new(layout_seed);
    let (_, bot_bundle) = bot.plan_pass_b_and_bundle(obs, own_mandate);
    let final_bundle = if preview_clean(obs, &actions, &bot_bundle).is_some() {
        bot_bundle
    } else {
        let mut safe_bundle = SealedOrderBundle {
            check_cashes: bot_bundle.check_cashes,
            debt_repayment_cred: bot_bundle.debt_repayment_cred,
            council_vote: bot_bundle.council_vote,
            ..SealedOrderBundle::default()
        };
        let mut with_conv = safe_bundle.clone();
        with_conv.converter_runs = bot_bundle.converter_runs;
        if preview_clean(obs, &actions, &with_conv).is_some() {
            safe_bundle = with_conv;
        }
        let mut with_proj = safe_bundle.clone();
        with_proj.project_deliveries = bot_bundle.project_deliveries;
        if preview_clean(obs, &actions, &with_proj).is_some() {
            safe_bundle = with_proj;
        }
        safe_bundle
    };

    if preview_clean(obs, &actions, &final_bundle).is_some() {
        Ok(EvalActSubmission {
            market_actions: actions,
            pass_b_actions: Vec::new(),
            bundle: final_bundle,
        })
    } else {
        Ok(EvalActSubmission::default())
    }
}

/// Output of scripted baseline `Round::Talk` generation: `(messages, delivered_whispers, canonical_raw_reply)`.
pub type ScriptedBaselineTalkOutput = (
    Vec<ParsedEvalMessage>,
    Vec<(String, UntrustedWhisper)>,
    String,
);

/// Dispatch `Round::Talk` reply generation for a non-`ClubBot` scripted `"file"` seat.
pub fn decide_scripted_baseline_talk(
    policy: EvalFileSeatModel,
    obs: &SeatObservation,
    seated_guilds: &[String],
) -> Result<ScriptedBaselineTalkOutput, EvalError> {
    let guild = seated_guilds
        .get(usize::from(obs.seat))
        .cloned()
        .unwrap_or_default();
    let messages = match policy {
        EvalFileSeatModel::ClubBot => Vec::new(),
        EvalFileSeatModel::RandomLegal => Vec::new(),
        EvalFileSeatModel::AlwaysDefect => plan_always_defect_talk(obs, seated_guilds)?.0,
        EvalFileSeatModel::TitForTat => decide_tit_for_tat_talk(obs, seated_guilds)?,
    };
    let raw_reply = encode_canonical_eval_reply(
        CommunicationMode::Talk,
        &messages,
        &EvalActSubmission::default(),
    )?;
    let parsed = parse_eval_talk_reply(&raw_reply, obs.seat, &guild, seated_guilds, false);
    if let Some(code) = parsed.refusal_code {
        return Err(EvalError::SchemaValidation(format!(
            "scripted baseline '{}' emitted invalid talk reply ({}): {}",
            policy.as_str(),
            code.as_str(),
            parsed.refusal_reason.unwrap_or_default()
        )));
    }
    Ok((messages, parsed.accepted_whispers, raw_reply))
}

/// Dispatch `Round::Act` decision + canonical `raw_reply` generation for non-`ClubBot` scripted `"file"` seats.
#[allow(clippy::too_many_arguments)]
pub fn decide_scripted_baseline_act(
    policy: EvalFileSeatModel,
    communication: CommunicationMode,
    layout_seed: u64,
    obs: &SeatObservation,
    own_mandate: &PrivateMandate,
    seated_guilds: &[String],
    to_seat_dir: Option<&Path>,
    talk_view: Option<&SeatObservation>,
) -> Result<(EvalActSubmission, String), EvalError> {
    let sub = match policy {
        EvalFileSeatModel::ClubBot => {
            let bot = ClubBot::new(layout_seed);
            let dec = bot.decide_full_turn(obs, own_mandate);
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            }
        }
        EvalFileSeatModel::RandomLegal => decide_random_legal_act(layout_seed, obs)?,
        EvalFileSeatModel::AlwaysDefect => decide_always_defect_act(
            layout_seed,
            obs,
            own_mandate,
            seated_guilds,
            communication,
            talk_view,
        )?,
        EvalFileSeatModel::TitForTat => {
            if communication != CommunicationMode::Talk {
                let guild = seated_guilds
                    .get(usize::from(obs.seat))
                    .map_or("unknown", String::as_str);
                return Err(EvalError::BaselineRequiresTalk {
                    guild: guild.to_owned(),
                    model: policy.as_str().to_owned(),
                });
            }
            let guild = seated_guilds
                .get(usize::from(obs.seat))
                .ok_or_else(|| EvalError::UnknownFocalSeat(format!("seat {}", obs.seat)))?;
            let hist = match to_seat_dir {
                Some(dir) => OwnToSeatHistory::load(dir, guild, obs.turn)?,
                None => OwnToSeatHistory::default(),
            };
            decide_tit_for_tat_act(layout_seed, &hist, obs, own_mandate, seated_guilds)?
        }
    };
    let raw_reply = encode_canonical_eval_reply(communication, &[], &sub)?;
    Ok((sub, raw_reply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_core::{EvalHost, Seat, TableId};
    use league_seat_types::{
        ActiveCharter, AtomicSwapProposal, CheckProposal, CommitmentHash, ContractDeliveryOrder,
        DualEscrowProposal, EscrowProposal, IntentPledge, PreimageSecret,
    };

    #[test]
    fn encode_canonical_eval_reply_covers_all_actions_and_bundle_legs() {
        let sub = EvalActSubmission {
            market_actions: vec![
                MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(1),
                        amount: UNIT_SCALE,
                        price_per_unit_cred: 2 * UNIT_SCALE,
                        domain_id: Some(1),
                        is_hybrid: true,
                    },
                },
                MarketAction::CancelOffer {
                    seat: 0,
                    offer_id: 9,
                },
                MarketAction::CreateCheck {
                    seat: 0,
                    check: CheckProposal {
                        payee: 1,
                        asset: AssetId::Cred,
                        send_max: 2 * UNIT_SCALE,
                        deliver_min: UNIT_SCALE,
                        due_turn: 2,
                        expiration_turn: 5,
                    },
                },
                MarketAction::CancelCheck {
                    seat: 0,
                    check_id: 4,
                },
                MarketAction::CreateEscrow {
                    seat: 0,
                    escrow: EscrowProposal {
                        destination: 1,
                        asset: AssetId::Cred,
                        amount: UNIT_SCALE,
                        finish_after_turn: Some(2),
                        cancel_after_turn: 5,
                        condition_hash: Some(CommitmentHash([0x11; 32])),
                        bond: None,
                    },
                },
                MarketAction::CancelExpiredEscrow {
                    seat: 0,
                    escrow_id: 5,
                },
                MarketAction::RecordIntentPledge {
                    seat: 0,
                    pledge: IntentPledge {
                        pledge_id: 7,
                        promisor: 0,
                        counterparty: 1,
                        target: IntentTarget::VoteForSeat {
                            office_turn: 4,
                            candidate: 1,
                        },
                        honor_weight: 50,
                        memo_digest: [0x22; 32],
                    },
                },
                MarketAction::RecordIntentPledge {
                    seat: 0,
                    pledge: IntentPledge {
                        pledge_id: 8,
                        promisor: 0,
                        counterparty: 1,
                        target: IntentTarget::PreserveCartelMembership {
                            office_turn: 4,
                            domain_id: 1,
                            beneficiary: 1,
                        },
                        honor_weight: 50,
                        memo_digest: [0x23; 32],
                    },
                },
                MarketAction::RecordIntentPledge {
                    seat: 0,
                    pledge: IntentPledge {
                        pledge_id: 9,
                        promisor: 0,
                        counterparty: 1,
                        target: IntentTarget::RevealPreimage {
                            escrow_id: 6,
                            condition: CommitmentHash([0x24; 32]),
                        },
                        honor_weight: 50,
                        memo_digest: [0x25; 32],
                    },
                },
                MarketAction::FlagWashFreeze {
                    inspector: 0,
                    target_seat: 2,
                },
                MarketAction::StandForOffice {
                    seat: 0,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: vec![(1, (100, 200))],
                        cartel_admissions: vec![(1, 2)],
                        cartel_revocations: vec![(1, 3)],
                    },
                },
                MarketAction::TownImport {
                    seat: 0,
                    good: AssetId::Raw(0),
                    amount: UNIT_SCALE,
                },
            ],
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                converter_runs: vec![ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 1,
                }],
                project_deliveries: vec![ProjectDeliveryOrder {
                    project_id: 0,
                    batches: 1,
                }],
                atomic_swaps: vec![AtomicSwapProposal {
                    swap_id: 1,
                    maker: 0,
                    taker: 1,
                    maker_asset: AssetId::Raw(0),
                    maker_amount: UNIT_SCALE,
                    taker_asset: AssetId::Refined(1),
                    taker_amount: UNIT_SCALE,
                    domain_id: Some(1),
                }],
                contract_deliveries: vec![ContractDeliveryOrder {
                    counterparty: 1,
                    good: AssetId::Refined(0),
                    gross_amount: UNIT_SCALE,
                    domain_id: None,
                    dual_escrow_id: Some(3),
                }],
                dual_escrows: vec![DualEscrowProposal {
                    proposal_id: 2,
                    buyer: 0,
                    seller: 1,
                    good: AssetId::Raw(1),
                    required_units: UNIT_SCALE,
                    payment_cred: 2 * UNIT_SCALE,
                    buyer_bond_cred: UNIT_SCALE,
                    seller_bond_cred: UNIT_SCALE,
                    deadline_turn: 4,
                    domain_id: None,
                }],
                debt_repayment_cred: 500,
                escrow_finishes: vec![EscrowFinishUnlock {
                    escrow_id: 6,
                    preimage: Some(PreimageSecret([0x33; 32])),
                }],
                check_cashes: vec![4],
                council_vote: Some(CouncilVoteBallot {
                    trade_minister: 0,
                    cartel_chair: 1,
                }),
                charter_bids: vec![
                    CharterBidOrder {
                        role: CharterRole::ClearinghouseBroker,
                        bid_cred: UNIT_SCALE,
                    },
                    CharterBidOrder {
                        role: CharterRole::CustomsInspector,
                        bid_cred: UNIT_SCALE,
                    },
                    CharterBidOrder {
                        role: CharterRole::OracleAuditor,
                        bid_cred: UNIT_SCALE,
                    },
                ],
            },
        };

        let talk_json = match encode_canonical_eval_reply(CommunicationMode::Talk, &[], &sub) {
            Ok(s) => s,
            Err(e) => panic!("encode talk reply: {e}"),
        };
        let parsed = crate::reply::parse_eval_act_reply(&talk_json, 0, false);
        assert!(parsed.refusal_code.is_none(), "{:?}", parsed.refusal_reason);
        assert_eq!(
            parsed.submission.market_actions.len(),
            sub.market_actions.len()
        );
        assert_eq!(parsed.submission.bundle.charter_bids.len(), 3);

        // Silent mode omits preimage/condition_hash cleanly on plain orders.
        let silent_sub = EvalActSubmission {
            market_actions: vec![MarketAction::CreateEscrow {
                seat: 0,
                escrow: EscrowProposal {
                    destination: 1,
                    asset: AssetId::Cred,
                    amount: UNIT_SCALE,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 5,
                    condition_hash: None,
                    bond: None,
                },
            }],
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                escrow_finishes: vec![EscrowFinishUnlock {
                    escrow_id: 6,
                    preimage: None,
                }],
                ..SealedOrderBundle::default()
            },
        };
        let silent_json =
            match encode_canonical_eval_reply(CommunicationMode::Silent, &[], &silent_sub) {
                Ok(s) => s,
                Err(e) => panic!("encode silent reply: {e}"),
            };
        let parsed_s = crate::reply::parse_eval_silent_act_reply(&silent_json, 0, false);
        assert!(
            parsed_s.refusal_code.is_none(),
            "{:?}",
            parsed_s.refusal_reason
        );
    }

    #[test]
    fn random_legal_always_defect_and_tit_for_tat_edge_branches() {
        let Ok(host) = EvalHost::from_standard_setup_with_model_seats(
            TableId(88),
            4,
            9001,
            111,
            &[true, true, true, true],
        ) else {
            panic!("init host");
        };
        let Ok(mut obs) = host.observe_seat(Seat(0)) else {
            panic!("observe seat 0");
        };
        let Ok(mandate) = host.mandate_for_seat(Seat(0)) else {
            panic!("mandate 0");
        };
        let guilds = vec![
            "Foundry".to_owned(),
            "Quarry".to_owned(),
            "Harbor".to_owned(),
            "Mint".to_owned(),
        ];

        // Inject synthetic checks, expired plain escrows, CustomsInspector charter, turn = 12 (divisible by 4 and 6),
        // resting offers, and debt so random_legal & always_defect hit every candidate generator path.
        obs.turn = 12;
        obs.seats[0].cred_debt = 2 * UNIT_SCALE;
        obs.seats[1].cred = 50 * UNIT_SCALE;
        obs.charters.push(ActiveCharter {
            role: CharterRole::CustomsInspector,
            holder: 0,
            expires_at_turn: 18,
        });
        obs.checks.insert(
            91,
            CreditCheck {
                check_id: 91,
                drawer: 0,
                payee: 1,
                asset: AssetId::Cred,
                send_max: UNIT_SCALE,
                deliver_min: UNIT_SCALE,
                due_turn: 12,
                expiration_turn: 15,
                status: CheckStatus::Open,
            },
        );
        obs.checks.insert(
            92,
            CreditCheck {
                check_id: 92,
                drawer: 1,
                payee: 0,
                asset: AssetId::Cred,
                send_max: UNIT_SCALE,
                deliver_min: UNIT_SCALE,
                due_turn: 10,
                expiration_turn: 16,
                status: CheckStatus::Open,
            },
        );
        obs.escrows.insert(
            71,
            TokenEscrowVault {
                escrow_id: 71,
                sender: 0,
                destination: 1,
                amount_cred: UNIT_SCALE,
                finish_after_turn: Some(2),
                cancel_after_turn: 10,
                condition_hash: None,
                bond: None,
                status: EscrowStatus::Locked,
            },
        );
        obs.escrows.insert(
            72,
            TokenEscrowVault {
                escrow_id: 72,
                sender: 1,
                destination: 0,
                amount_cred: UNIT_SCALE,
                finish_after_turn: Some(8),
                cancel_after_turn: 16,
                condition_hash: None,
                bond: None,
                status: EscrowStatus::Locked,
            },
        );
        obs.offers.insert(
            51,
            CredOffer {
                offer_id: 51,
                maker: 1,
                side: OfferSide::Sell,
                good: AssetId::Raw(1),
                remaining_units: UNIT_SCALE,
                price_per_unit_cred: 2 * UNIT_SCALE,
                locked_cred: 0,
                domain_id: None,
                is_hybrid: false,
                created_turn: 11,
            },
        );
        obs.offers.insert(
            52,
            CredOffer {
                offer_id: 52,
                maker: 0,
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                remaining_units: UNIT_SCALE,
                price_per_unit_cred: 2 * UNIT_SCALE,
                locked_cred: 2 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
                created_turn: 11,
            },
        );

        let Ok(rl) = decide_random_legal_act(9001, &obs) else {
            panic!("random_legal edge branches");
        };
        assert!(preview_clean(&obs, &rl.market_actions, &rl.bundle).is_some());

        let Ok(ad) = decide_always_defect_act(
            9001,
            &obs,
            mandate,
            &guilds,
            CommunicationMode::Talk,
            Some(&obs),
        ) else {
            panic!("always_defect edge branches");
        };
        assert!(preview_clean(&obs, &ad.market_actions, &ad.bundle).is_some());

        for policy in [
            EvalFileSeatModel::ClubBot,
            EvalFileSeatModel::RandomLegal,
            EvalFileSeatModel::AlwaysDefect,
            EvalFileSeatModel::TitForTat,
        ] {
            let Ok((_, _, raw)) = decide_scripted_baseline_talk(policy, &obs, &guilds) else {
                panic!("talk dispatch failed for {policy:?}");
            };
            assert!(raw.contains("\"messages\""));
            let res = decide_scripted_baseline_act(
                policy,
                CommunicationMode::Talk,
                9001,
                &obs,
                mandate,
                &guilds,
                None,
                None,
            );
            assert!(
                res.is_ok(),
                "act dispatch failed for {policy:?}: {:?}",
                res.err()
            );
        }

        let Err(tft_err) = decide_scripted_baseline_act(
            EvalFileSeatModel::TitForTat,
            CommunicationMode::Silent,
            9001,
            &obs,
            mandate,
            &guilds,
            None,
            None,
        ) else {
            panic!("tit-for-tat must error under Silent");
        };
        assert_eq!(tft_err.code(), "BASELINE_REQUIRES_TALK");

        // Test pledge_broke, check_bounced, and bond_breached triggers in compute_tit_for_tat_stances.
        let mut rec_host = host;
        let Ok(_outcome) = rec_host.apply_act_submissions(&BTreeMap::new()) else {
            panic!("apply empty subs");
        };
        let Ok(mut sweep_rec) = rec_host.sweep_turn() else {
            panic!("sweep turn 1");
        };
        sweep_rec.checks_bounced = vec![92];
        sweep_rec.broken_promises = vec![league_seat_types::BrokenPromiseRecord {
            pledge_id: 1,
            promisor: 2,
            counterparty: 0,
            weight: 50,
        }];
        let mut obs2 = obs.clone();
        obs2.turn = 2;
        obs2.public_receipts = vec![sweep_rec];
        obs2.seats[3].bond_breaches = 1;

        let mut prev_view = HistoricalPreActView {
            turn: 1,
            spec: obs.spec.clone(),
            seats: obs.seats.clone(),
            checks: BTreeMap::new(),
            escrows: BTreeMap::new(),
            offers: BTreeMap::new(),
            domains: obs.domains.clone(),
            pending_wash_freezes: Vec::new(),
        };
        prev_view.checks.insert(
            92,
            CreditCheck {
                check_id: 92,
                drawer: 1,
                payee: 0,
                asset: AssetId::Cred,
                send_max: UNIT_SCALE,
                deliver_min: UNIT_SCALE,
                due_turn: 1,
                expiration_turn: 3,
                status: CheckStatus::Open,
            },
        );
        prev_view.escrows.insert(
            81,
            TokenEscrowVault {
                escrow_id: 81,
                sender: 3,
                destination: 0,
                amount_cred: UNIT_SCALE,
                finish_after_turn: Some(1),
                cancel_after_turn: 3,
                condition_hash: None,
                bond: None,
                status: EscrowStatus::Locked,
            },
        );
        prev_view.seats[3].bond_breaches = 0;

        let mut hist = OwnToSeatHistory::default();
        hist.pre_act_by_day.insert(1, prev_view);
        let Ok(stances) = compute_tit_for_tat_stances(&hist, &obs2, &guilds) else {
            panic!("stances");
        };
        assert_eq!(stances.get(&1), Some(&PeerStance::Defect));
        assert_eq!(stances.get(&2), Some(&PeerStance::Defect));
        assert_eq!(stances.get(&3), Some(&PeerStance::Defect));
    }
}
