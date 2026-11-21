//! Exclusive Evaluation Mode host wrapper owning [`TurnEngine`], `.save.json` / `.seal`
//! persistence, and pre-act / act-recorded checkpoints.
//!
//! Modelled on [`crate::club::host::ClubHost`] strictly for [`TableMode::Eval`] (`TableMode::Club`
//! and [`TableMode::Game`] are refused). Pre-derives all `32` `EvalVrf` proofs on initialization
//! and drops the `SealedEvalSeed` in memory; neither `.save.json` nor `.seal` stores a raw seed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commit::{
    CommitmentHash, CouncilPlatform, EVAL_VRF_PROOF_BYTES, IntentPledge, MandateReveal,
    OrderReveal, PrivateMandate, RevealSalt, SealedEvalSeed, SealedMandateDeal, SealedOrderBundle,
    compute_mandate_commitment, compute_order_commitment, draw_nonzero_256_bits,
    verify_and_extract_eval_vrf_beacon, verify_sealed_mandate_deal,
};
use crate::contracts::{
    ActiveCharter, CartelDomain, CredOffer, CreditCheck, DualEscrowVault, FillRecord, MarketAction,
    PendingWashFreeze, TokenEscrowVault,
};
use crate::engine::{SeatState, ShockForecastEvent, TurnEngine, TurnPhase};
use crate::observation::{
    PublicSweepReceipt, PublicTableSpec, SeatObservation, compute_public_forecasts,
    public_sweep_receipt_from_raw, public_table_spec_from_raw,
};
use crate::predicates::{SeatSeasonMetrics, StatePredicateSection, compute_season_metrics};
use crate::replay::{
    LoggedTurn, ReplayLayoutSeed, SEALED_LAYOUT_SEED_MIN, SeasonReplayLog,
    check_deal_seed_independence, check_eval_vrf_independence, replay_eval_season,
    replay_season_with_state_predicates,
};
use crate::scoring::{FinalScoreboard, FundamentalParities};
use crate::types::{
    ActiveShock, AssetId, EngineError, HARD_TURN_CAP, Seat, TableId, TableMode, TableSpec, Turn,
    Units,
};

/// Current schema version for public `.save.json` snapshots written by [`EvalHost`].
pub const EVAL_SAVE_FORMAT_VERSION: u32 = 1;

/// Current schema version for private `.seal` sidecar payloads written by [`EvalHost`].
pub const EVAL_SEAL_FORMAT_VERSION: u32 = 1;

/// Public, spectator-safe Evaluation Mode save projection matching [`SeatObservation`] without any
/// hidden secrets, `TurnEngine`, `EngineState`, unannounced `drawn_shocks`, VRF proofs, or
/// duplicate receipt histories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalSaveSnapshot {
    /// Save projection format version (`EVAL_SAVE_FORMAT_VERSION`).
    pub format_version: u32,
    /// Public clear layout seed (`< 2^63`) or redacted `sealed`-pool digest (`>= 2^63`).
    pub layout_seed: ReplayLayoutSeed,
    /// Table identifier.
    pub table_id: TableId,
    /// Current turn index (`1..=32`).
    pub turn: u8,
    /// Current turn phase or terminal close state.
    pub phase: TurnPhase,
    /// Whether Phase-1 cancel lock is currently engaged.
    pub cancel_locked: bool,
    /// Current Trade Minister seat index.
    pub trade_minister: u8,
    /// Current Cartel Chair seat index (`None` if vacant).
    pub cartel_chair: Option<u8>,
    /// Public table rule parameters.
    pub spec: PublicTableSpec,
    /// Precomputed fundamental shadow parities.
    pub parities: FundamentalParities,
    /// Per-seat public balances and reputations.
    pub seats: Vec<SeatState>,
    /// Resting `CRED`-priced DEX limit orders keyed by canonical `offer_id`.
    pub offers: BTreeMap<u64, CredOffer>,
    /// Track-2 deferred checks keyed by canonical `check_id`.
    pub checks: BTreeMap<u64, CreditCheck>,
    /// Track-1 `TokenEscrow` vaults keyed by canonical `escrow_id`.
    pub escrows: BTreeMap<u64, TokenEscrowVault>,
    /// Co-signed dual delivery-versus-payment escrows keyed by shared `escrow_id`.
    pub dual_escrows: BTreeMap<u64, DualEscrowVault>,
    /// Recorded Track-2 structured diplomatic pledges keyed by `pledge_id`.
    pub pledges: BTreeMap<u64, IntentPledge>,
    /// Pledge IDs already resolved (honored or flagged broken).
    pub audited_pledges: BTreeSet<u64>,
    /// Permissioned cartel trade domains.
    pub domains: Vec<CartelDomain>,
    /// Public council platforms filed ahead of Turn-`4n` elections.
    pub council_platforms: BTreeMap<u8, CouncilPlatform>,
    /// Active rotating Syndicate Charters.
    pub charters: Vec<ActiveCharter>,
    /// Realized active Horizon Shocks (`turn >= realize_turn`; magnitude is public from realize).
    pub active_shocks: Vec<ActiveShock>,
    /// Publicly announced Horizon Shock forecasts (`forecast_turn <= last_swept_turn`, zero magnitude).
    pub public_forecasts: Vec<ShockForecastEvent>,
    /// Monotonic next offer ID.
    pub next_offer_id: u64,
    /// Monotonic next check ID.
    pub next_check_id: u64,
    /// Monotonic next escrow ID.
    pub next_escrow_id: u64,
    /// Cumulative delivered volume `(sender, receiver, asset)`.
    #[serde(with = "crate::engine::delivery_ledger_serde")]
    pub delivery_ledger: BTreeMap<(u8, u8, AssetId), Units>,
    /// Previous step PBRS potentials `Psi_i(s_t)`.
    pub last_potentials: Vec<Units>,
    /// Accumulator of `CRED` spent on Phase-1 `TownImport` orders during the current turn.
    pub pending_town_import_cred_spent: Units,
    /// Append-only per-turn settlement log across all turns.
    pub fill_log: Vec<FillRecord>,
    /// Unsettled bonded `FlagWashFreeze` challenges for the current turn.
    pub pending_wash_freezes: Vec<PendingWashFreeze>,
    /// Optional `ristretto255` VRF public key (`PK_eval`) committed on the table.
    #[serde(default, with = "crate::types::opt_hex32")]
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Single seat-safe 30-field historical Sweep receipt sequence.
    pub public_receipts: Vec<PublicSweepReceipt>,
}

/// One seat's recorded act-round submission for the current turn (`market_actions`, optional
/// post-cancel-lock `pass_b_actions`, and Phase-2 [`SealedOrderBundle`]).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalActSubmission {
    /// Phase-1 Pass A market actions submitted before `cancel_lock`.
    pub market_actions: Vec<MarketAction>,
    /// Optional Phase-1 Pass B market actions submitted after `cancel_lock`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pass_b_actions: Vec<MarketAction>,
    /// Phase-2 sealed order bundle committed and revealed for Sweep.
    pub bundle: SealedOrderBundle,
}

impl fmt::Debug for EvalActSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvalActSubmission")
            .field("market_actions_len", &self.market_actions.len())
            .field("pass_b_actions_len", &self.pass_b_actions.len())
            .field("bundle", &"<hidden>")
            .finish()
    }
}

/// Refusal report for one seat when applying [`EvalActSubmission`]s to [`EvalHost`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalSeatApplyReport {
    /// `(action_index, error_code)` pairs refused during Pass A `market_actions`.
    pub refused_market_actions: Vec<(usize, String)>,
    /// `(action_index, error_code)` pairs refused during Pass B `pass_b_actions`.
    pub refused_pass_b_actions: Vec<(usize, String)>,
    /// Error code if the seat's [`SealedOrderBundle`] failed reveal validation and fell back to an
    /// empty bundle.
    pub refused_bundle_code: Option<String>,
}

/// Per-seat refusal summary returned by [`EvalHost::apply_act_submissions`] and
/// [`EvalHost::apply_recorded_act_submissions_from_seal`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalActApplyOutcome {
    /// Per-seat application reports keyed by seat index `0..seat_count`.
    pub seats: BTreeMap<u8, EvalSeatApplyReport>,
}

#[derive(Serialize, Deserialize)]
struct EvalSealPayload {
    format_version: u32,
    game_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pre_act_checkpoint_sha256: Option<String>,
    layout_seed: u64,
    vrf_proofs: Vec<Vec<u8>>,
    engine: TurnEngine,
    sealed_mandate_deal: SealedMandateDeal,
    mandate_commits: Vec<Option<CommitmentHash>>,
    mandate_reveals: Vec<Option<MandateReveal>>,
    logged_turns: Vec<LoggedTurn>,
    current_market_actions: Vec<MarketAction>,
    current_cancel_lock_at: Option<u32>,
    current_commitments: BTreeMap<u8, CommitmentHash>,
    current_reveals: BTreeMap<u8, OrderReveal>,
    bot_tie_seed: u64,
    live_state_digests: Vec<[u8; 32]>,
    turn_salts: Vec<Vec<RevealSalt>>,
    #[serde(default)]
    recorded_act_submissions: BTreeMap<u8, EvalActSubmission>,
}

/// Exclusive Evaluation Mode host wrapper owning [`TurnEngine`], pre-derived VRF proofs, sealed
/// Turn-1 mandates, replay turn capture, per-turn order salts, and `.save.json` / `.seal`
/// persistence.
///
/// No Evaluation Mode seat module touches `TurnEngine`, `EngineState`, `SweepReceipt`, or
/// `SealedEvalSeed` directly: every seat reads only [`SeatObservation`] via [`Self::observe_seat`]
/// and [`PublicSweepReceipt`] via [`Self::sweep_turn`] / [`Self::public_receipts`].
pub struct EvalHost {
    game_id: String,
    layout_seed: u64,
    bot_tie_seed: u64,
    vrf_proofs: Vec<Vec<u8>>,
    engine: TurnEngine,
    sealed_mandate_deal: SealedMandateDeal,
    mandate_commits: Vec<Option<CommitmentHash>>,
    mandate_reveals: Vec<Option<MandateReveal>>,
    logged_turns: Vec<LoggedTurn>,
    live_state_digests: Vec<[u8; 32]>,
    current_market_actions: Vec<MarketAction>,
    current_cancel_lock_at: Option<u32>,
    current_commitments: BTreeMap<u8, CommitmentHash>,
    current_reveals: BTreeMap<u8, OrderReveal>,
    turn_salts: Vec<Vec<RevealSalt>>,
    recorded_act_submissions: BTreeMap<u8, EvalActSubmission>,
    active_pre_act_checkpoint_sha256: Option<String>,
}

impl fmt::Debug for EvalHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let st = self.engine.state();
        let layout_view = replay_layout_seed_for_raw(self.layout_seed);
        f.debug_struct("EvalHost")
            .field("game_id", &self.game_id)
            .field("table_id", &st.table_id)
            .field("layout_seed", &layout_view)
            .field("turn", &st.turn)
            .field("phase", &st.phase)
            .field("seat_count", &st.spec.seat_count)
            .field("logged_turns_len", &self.logged_turns.len())
            .field("vrf_proofs", &"<hidden>")
            .field("sealed_mandate_deal", &"<hidden>")
            .field("turn_salts", &"<hidden>")
            .field("recorded_act_submissions", &"<hidden>")
            .finish()
    }
}

impl EvalHost {
    /// Initializes an [`EvalHost`] over a validated `TableMode::Eval` [`TableSpec`], pre-deriving
    /// all `32` `EvalVrf` proofs (`1..=HARD_TURN_CAP`), dropping `eval_seed` in memory, pre-deriving
    /// all `32` turns of Phase-2 reveal salts from OS entropy, and recording Turn-1 Private Mandate
    /// commitments. Refuses [`TableMode::Club`] and [`TableMode::Game`].
    pub fn new(
        spec: TableSpec,
        layout_seed: u64,
        sealed_mandate_deal: SealedMandateDeal,
        eval_seed: SealedEvalSeed,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        if spec.mode != TableMode::Eval {
            return Err(EngineError::InvalidConfig(format!(
                "INVALID_TABLE_MODE: EvalHost requires TableMode::Eval, got {:?}",
                spec.mode
            )));
        }
        if spec.drand_schedule.is_some() {
            return Err(EngineError::InvalidConfig(
                "INVALID_EVAL_DRAND_SCHEDULE: EvalHost forbids drand_schedule".to_owned(),
            ));
        }
        spec.validate()?;

        let eval_pk = spec.eval_vrf_pubkey.ok_or_else(|| {
            EngineError::InvalidConfig(
                "MISSING_EVAL_VRF_PUBKEY: EvalHost requires TableSpec::eval_vrf_pubkey".to_owned(),
            )
        })?;
        let actual_pk = eval_seed.public_key_bytes()?;
        if actual_pk != eval_pk {
            return Err(EngineError::InvalidConfig(
                "EVAL_VRF_PUBKEY_MISMATCH: SealedEvalSeed public key does not match TableSpec::eval_vrf_pubkey"
                    .to_owned(),
            ));
        }

        let sealed_context = layout_seed >= SEALED_LAYOUT_SEED_MIN;
        let candidates =
            check_deal_seed_independence(layout_seed, &sealed_mandate_deal, sealed_context)?;
        check_eval_vrf_independence(candidates, eval_pk, sealed_context)?;

        if !verify_sealed_mandate_deal(
            spec.table_id,
            spec.seat_count,
            &spec.mandate_deck,
            &spec.dealt_mandate_commits,
            &sealed_mandate_deal,
        )? {
            return Err(EngineError::InvalidConfig(
                "INVALID_SEALED_MANDATE_DEAL: sealed_mandate_deal does not match TableSpec::dealt_mandate_commits and deck"
                    .to_owned(),
            ));
        }

        let game_id = generate_eval_game_id()?;
        let vrf_proofs = derive_all_vrf_proofs(eval_seed, spec.table_id)?;
        let n = usize::from(spec.seat_count);
        let seat_count = spec.seat_count;
        let mut engine = TurnEngine::new(spec)?;
        let table_id = engine.state().table_id;
        let turn_salts = derive_all_turn_commit_salts(table_id, seat_count)?;
        let mut mandate_commits = Vec::with_capacity(n);
        for s in 0..n as u8 {
            let rev = &sealed_mandate_deal.seat_reveals[usize::from(s)];
            let mc = compute_mandate_commitment(table_id, Seat(s), rev)?;
            engine.submit_mandate_commitment(Seat(s), mc)?;
            mandate_commits.push(Some(mc));
        }

        Ok(Self {
            game_id,
            layout_seed,
            bot_tie_seed,
            vrf_proofs,
            engine,
            sealed_mandate_deal,
            mandate_commits,
            mandate_reveals: vec![None; n],
            logged_turns: Vec::new(),
            live_state_digests: Vec::new(),
            current_market_actions: Vec::new(),
            current_cancel_lock_at: None,
            current_commitments: BTreeMap::new(),
            current_reveals: BTreeMap::new(),
            turn_salts,
            recorded_act_submissions: BTreeMap::new(),
            active_pre_act_checkpoint_sha256: None,
        })
    }

    /// Bootstraps an [`EvalHost`] using [`TableSpec::standard_eval_chiral_ring_with_setup`].
    pub fn from_standard_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let (spec, deal, eval_seed) =
            TableSpec::standard_eval_chiral_ring_with_setup(table_id, seat_count, layout_seed)?;
        Self::new(spec, layout_seed, deal, eval_seed, bot_tie_seed)
    }

    /// Bootstraps a reproducible [`EvalHost`] using
    /// [`TableSpec::standard_eval_chiral_ring_with_explicit_setup`].
    #[allow(clippy::too_many_arguments)]
    pub fn from_explicit_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        deal_seed: [u8; 32],
        seat_salts: &[RevealSalt],
        eval_seed: SealedEvalSeed,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let (spec, deal, eval_seed) = TableSpec::standard_eval_chiral_ring_with_explicit_setup(
            table_id,
            seat_count,
            layout_seed,
            deal_seed,
            seat_salts,
            eval_seed,
        )?;
        Self::new(spec, layout_seed, deal, eval_seed, bot_tie_seed)
    }

    /// Returns the per-game identifier assigned when this game directory/host was initialized
    /// (different for every D2 duplicate).
    #[must_use]
    pub fn game_id(&self) -> &str {
        &self.game_id
    }

    /// Returns the clear layout seed for this table (`ReplayLayoutSeed` handles redaction when
    /// exported for `>= 2^63` seeds).
    #[must_use]
    pub const fn layout_seed(&self) -> u64 {
        self.layout_seed
    }

    /// Returns the deterministic tie-break seed assigned to [`crate::ClubBot`] seats.
    #[must_use]
    pub const fn bot_tie_seed(&self) -> u64 {
        self.bot_tie_seed
    }

    /// Returns the table identifier.
    #[must_use]
    pub const fn table_id(&self) -> TableId {
        self.engine.state().table_id
    }

    /// Returns the number of seats on this table (`4..=6`).
    #[must_use]
    pub const fn seat_count(&self) -> u8 {
        self.engine.state().spec.seat_count
    }

    /// Returns the current turn index (`1..=32`).
    #[must_use]
    pub const fn turn(&self) -> u8 {
        self.engine.state().turn
    }

    /// Returns the current [`TurnPhase`].
    #[must_use]
    pub const fn phase(&self) -> TurnPhase {
        self.engine.state().phase
    }

    /// Returns whether Phase-1 cancel lock is currently engaged.
    #[must_use]
    pub const fn cancel_locked(&self) -> bool {
        self.engine.state().cancel_locked
    }

    /// Projects the public [`PublicTableSpec`] without hidden `shocks` or `dealt_mandate_commits`.
    #[must_use]
    pub fn public_spec(&self) -> PublicTableSpec {
        public_table_spec_from_raw(&self.engine.state().spec)
    }

    /// Returns the seat-safe 30-field [`PublicSweepReceipt`] history from live sweeps.
    #[must_use]
    pub fn public_receipts(&self) -> &[PublicSweepReceipt] {
        self.engine.public_receipts()
    }

    /// Projects the sole seat-safe read boundary [`SeatObservation`] for `seat`, carrying only
    /// `seat`'s own dealt [`PrivateMandate`] (never reveal salts or rival seats' mandates).
    ///
    /// Host-only persistence byte helpers (`to_save_and_seal_bytes`, `from_save_and_seal_bytes`)
    /// are `pub(crate)`:
    ///
    /// ```compile_fail,E0624
    /// use league_core::{EvalHost, TableId};
    /// let host = EvalHost::from_standard_setup(TableId(1), 4, 1, 0).unwrap();
    /// let _ = host.to_save_and_seal_bytes();
    /// ```
    ///
    /// ```compile_fail,E0624
    /// use league_core::EvalHost;
    /// let _ = EvalHost::from_save_and_seal_bytes(&[], &[]);
    /// ```
    pub fn observe_seat(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        let mandate = *self.mandate_for_seat(seat)?;
        self.engine.observe_seat_with_mandate(seat, mandate)
    }

    /// Host-only lookup returning `seat`'s dealt [`PrivateMandate`] (without its reveal salt).
    ///
    /// Forbidden in seat-side modules (`raw_api_backdoor_guard`). Seats must read only their own
    /// mandate embedded in [`SeatObservation::own_mandate`] from [`Self::observe_seat`].
    pub fn mandate_for_seat(&self, seat: Seat) -> Result<&PrivateMandate, EngineError> {
        let n = self.seat_count();
        if seat.0 >= n {
            return Err(EngineError::InvalidSeat(seat.0));
        }
        Ok(&self.sealed_mandate_deal.seat_reveals[usize::from(seat.0)].mandate)
    }

    /// Computes the deterministic Standing Regent fallback bundle for `seat` on the current turn.
    pub fn preview_regent_bundle_for_seat(
        &self,
        seat: Seat,
    ) -> Result<Option<SealedOrderBundle>, EngineError> {
        let turn = Turn(self.engine.state().turn);
        self.engine.preview_regent_bundle_for_seat(turn, seat)
    }

    /// Private Phase-2 commit salt helper for `seat` on the host's active turn.
    fn turn_reveal_salt(&self, seat: Seat) -> Result<RevealSalt, EngineError> {
        let turn_idx = self.turn().checked_sub(1).map(usize::from).ok_or_else(|| {
            EngineError::InvalidPhase("turn_reveal_salt requires turn >= 1".to_owned())
        })?;
        self.turn_salts
            .get(turn_idx)
            .and_then(|salts| salts.get(usize::from(seat.0)))
            .cloned()
            .ok_or(EngineError::InvalidSeat(seat.0))
    }

    /// Private helper returning recorded per-seat act submissions held in memory for the turn.
    #[cfg(test)]
    const fn recorded_act_submissions(&self) -> &BTreeMap<u8, EvalActSubmission> {
        &self.recorded_act_submissions
    }

    /// Applies one Phase-1 (`TalkAndMarket`) [`MarketAction`] atomically and records valid actions
    /// for replay log export.
    pub fn apply_market_action(&mut self, action: MarketAction) -> Result<(), EngineError> {
        self.engine.apply_market_action(action.clone())?;
        self.current_market_actions.push(action);
        Ok(())
    }

    /// Engages Phase-1 cancel lock (`TalkAndMarket` only) and captures its index for replay.
    pub fn begin_cancel_lock(&mut self) -> Result<(), EngineError> {
        self.engine.begin_cancel_lock()?;
        if self.engine.state().cancel_locked && self.current_cancel_lock_at.is_none() {
            self.current_cancel_lock_at = Some(self.current_market_actions.len() as u32);
        }
        Ok(())
    }

    /// Transitions the current turn from `TalkAndMarket` -> `Commit`.
    pub fn advance_to_commit(&mut self) -> Result<(), EngineError> {
        self.engine.advance_to_commit()
    }

    /// Records a seat's sealed SHA-256 Phase-2 commitment during `Commit`.
    pub fn submit_order_commitment(
        &mut self,
        seat: Seat,
        commitment: CommitmentHash,
    ) -> Result<(), EngineError> {
        self.engine.submit_order_commitment(seat, commitment)?;
        self.current_commitments.insert(seat.0, commitment);
        Ok(())
    }

    /// Transitions the current turn from `Commit` -> `Reveal`.
    pub fn advance_to_reveal(&mut self) -> Result<(), EngineError> {
        self.engine.advance_to_reveal()
    }

    /// Verifies and stores a seat's Phase-2 [`OrderReveal`] during `Reveal`.
    pub fn submit_order_reveal(
        &mut self,
        seat: Seat,
        reveal: OrderReveal,
    ) -> Result<(), EngineError> {
        self.engine.submit_order_reveal(seat, reveal.clone())?;
        self.current_reveals.insert(seat.0, reveal);
        Ok(())
    }

    /// Executes `TalkAndMarket -> Commit -> Reveal` across all `reveals` atomically in order.
    pub fn commit_and_reveal_all(
        &mut self,
        reveals: &BTreeMap<u8, OrderReveal>,
    ) -> Result<(), EngineError> {
        let table_id = self.table_id();
        let turn = Turn(self.turn());
        self.advance_to_commit()?;
        for (&seat_idx, rev) in reveals {
            let commit = compute_order_commitment(table_id, turn, Seat(seat_idx), rev)?;
            self.submit_order_commitment(Seat(seat_idx), commit)?;
        }
        self.advance_to_reveal()?;
        for (&seat_idx, rev) in reveals {
            self.submit_order_reveal(Seat(seat_idx), rev.clone())?;
        }
        Ok(())
    }

    /// Applies all seats' act-round submissions (`market_actions`, `cancel_lock`, `pass_b_actions`,
    /// and Phase-2 [`SealedOrderBundle`]s) in deterministic seat order `0..seat_count` and stores
    /// the normalized submissions for `checkpoints/day-DD-act-recorded.seal`.
    ///
    /// Invalid actions or bundles on any seat are recorded by error code in [`EvalActApplyOutcome`]
    /// and never halt the turn: cross-seat actions record `"UNAUTHORIZED"`, invalid market actions
    /// are skipped, and an invalid Phase-2 bundle falls back to [`SealedOrderBundle::default()`].
    pub fn apply_act_submissions(
        &mut self,
        submissions: &BTreeMap<u8, EvalActSubmission>,
    ) -> Result<EvalActApplyOutcome, EngineError> {
        self.ensure_pre_act_state("apply_act_submissions")?;
        let seat_count = self.seat_count();
        for &seat_idx in submissions.keys() {
            if seat_idx >= seat_count {
                return Err(EngineError::InvalidSeat(seat_idx));
            }
        }

        let mut normalized = BTreeMap::new();
        let mut reports = BTreeMap::new();
        for s in 0..seat_count {
            normalized.insert(s, submissions.get(&s).cloned().unwrap_or_default());
            reports.insert(s, EvalSeatApplyReport::default());
        }

        for s in 0..seat_count {
            if let Some(sub) = normalized.get(&s) {
                for (idx, action) in sub.market_actions.iter().enumerate() {
                    if action.actor_seat() != s {
                        if let Some(report) = reports.get_mut(&s) {
                            report
                                .refused_market_actions
                                .push((idx, "UNAUTHORIZED".to_owned()));
                        }
                    } else if let Err(err) = self.apply_market_action(action.clone())
                        && let Some(report) = reports.get_mut(&s)
                    {
                        report
                            .refused_market_actions
                            .push((idx, err.code().to_owned()));
                    }
                }
            }
        }

        self.begin_cancel_lock()?;

        for s in 0..seat_count {
            if let Some(sub) = normalized.get(&s) {
                for (idx, action) in sub.pass_b_actions.iter().enumerate() {
                    if action.actor_seat() != s {
                        if let Some(report) = reports.get_mut(&s) {
                            report
                                .refused_pass_b_actions
                                .push((idx, "UNAUTHORIZED".to_owned()));
                        }
                    } else if let Err(err) = self.apply_market_action(action.clone())
                        && let Some(report) = reports.get_mut(&s)
                    {
                        report
                            .refused_pass_b_actions
                            .push((idx, err.code().to_owned()));
                    }
                }
            }
        }

        let table_id = self.table_id();
        let turn = Turn(self.turn());
        let mut probe = self.engine.clone();
        probe.advance_to_commit()?;
        let mut candidate_reveals = BTreeMap::new();
        for s in 0..seat_count {
            let salt = self.turn_reveal_salt(Seat(s))?;
            let bundle = normalized
                .get(&s)
                .map(|sub| sub.bundle.clone())
                .unwrap_or_default();
            let reveal = OrderReveal { bundle, salt };
            let commit = compute_order_commitment(table_id, turn, Seat(s), &reveal)?;
            probe.submit_order_commitment(Seat(s), commit)?;
            candidate_reveals.insert(s, reveal);
        }
        probe.advance_to_reveal()?;
        let mut effective_reveals = BTreeMap::new();
        for s in 0..seat_count {
            let salt = self.turn_reveal_salt(Seat(s))?;
            let candidate = candidate_reveals
                .get(&s)
                .cloned()
                .ok_or(EngineError::InvalidSeat(s))?;
            let final_reveal = match probe.submit_order_reveal(Seat(s), candidate.clone()) {
                Ok(()) => candidate,
                Err(err) => {
                    if let Some(report) = reports.get_mut(&s) {
                        report.refused_bundle_code = Some(err.code().to_owned());
                    }
                    OrderReveal {
                        bundle: SealedOrderBundle::default(),
                        salt,
                    }
                }
            };
            effective_reveals.insert(s, final_reveal);
        }

        self.commit_and_reveal_all(&effective_reveals)?;
        self.recorded_act_submissions = normalized;
        Ok(EvalActApplyOutcome { seats: reports })
    }

    /// Runs the 8-step live `EvalVrf` Turn Sweep using the pre-derived VRF proof for the current
    /// turn, logs the completed [`LoggedTurn`] privately, clears current-turn act submissions, and
    /// returns only [`PublicSweepReceipt`].
    pub fn sweep_turn(&mut self) -> Result<PublicSweepReceipt, EngineError> {
        let turn = self.engine.state().turn;
        let proof_idx = usize::from(turn.checked_sub(1).ok_or_else(|| {
            EngineError::InvalidPhase("turn index must be >= 1 to sweep".to_owned())
        })?);
        let proof = self
            .vrf_proofs
            .get(proof_idx)
            .ok_or_else(|| {
                EngineError::InvalidConfig(format!("missing pre-derived VRF proof for turn {turn}"))
            })?
            .clone();
        let eval_pk = self.engine.state().spec.eval_vrf_pubkey.ok_or_else(|| {
            EngineError::InvalidConfig("EvalHost requires TableSpec::eval_vrf_pubkey".to_owned())
        })?;
        let beacon = verify_and_extract_eval_vrf_beacon(
            eval_pk,
            self.engine.state().table_id,
            Turn(turn),
            &proof,
        )?;
        let raw_receipt = self.engine.sweep_turn_with_eval_vrf(&proof)?;
        let public_receipt = public_sweep_receipt_from_raw(&raw_receipt);
        self.live_state_digests.push(raw_receipt.post_state_digest);
        self.logged_turns.push(LoggedTurn {
            turn,
            market_actions: std::mem::take(&mut self.current_market_actions),
            cancel_lock_at: self.current_cancel_lock_at.take(),
            commitments: std::mem::take(&mut self.current_commitments),
            reveals: std::mem::take(&mut self.current_reveals),
            turn_beacon: beacon,
            drand_round: None,
            drand_signature: None,
            eval_vrf_proof: Some(proof),
            drawn_shocks: raw_receipt.newly_drawn_shocks,
        });
        self.recorded_act_submissions.clear();
        self.active_pre_act_checkpoint_sha256 = None;
        Ok(public_receipt)
    }

    /// Unseals all Turn-1 Private Charter Mandates after Season Close, finalizes the
    /// [`FinalScoreboard`], exports and verifies the deterministic Eval [`SeasonReplayLog`], and
    /// returns the derived [`StatePredicateSection`] and [`SeatSeasonMetrics`].
    pub fn finalize_eval_season(
        &mut self,
    ) -> Result<
        (
            FinalScoreboard,
            SeasonReplayLog,
            StatePredicateSection,
            Vec<SeatSeasonMetrics>,
        ),
        EngineError,
    > {
        if self.phase() != TurnPhase::AwaitingMandateReveal {
            return Err(EngineError::InvalidPhase(
                "finalize_eval_season requires AwaitingMandateReveal".to_owned(),
            ));
        }
        let n = usize::from(self.seat_count());
        let mut reveals = Vec::with_capacity(n);
        for s in 0..n as u8 {
            let rev = self.sealed_mandate_deal.seat_reveals[usize::from(s)].clone();
            if self.engine.state().mandate_reveals[usize::from(s)].is_none() {
                self.engine.submit_mandate_reveal(Seat(s), rev.clone())?;
            }
            reveals.push(Some(rev));
        }
        let scoreboard = self.engine.finalize_season()?;
        self.mandate_reveals = reveals;
        let log = self.export_replay_log()?;
        let verified = replay_eval_season(&log)?;
        let (replay_out, section) = replay_season_with_state_predicates(&log)?;
        if replay_out.scoreboard != scoreboard || verified.scoreboard != scoreboard {
            return Err(EngineError::SeasonEnded(
                "replayed scoreboard diverged from live EvalHost scoreboard".to_owned(),
            ));
        }
        if replay_out.state_digests.len() != self.live_state_digests.len() {
            return Err(EngineError::SeasonEnded(format!(
                "replayed state digest count {} diverged from live EvalHost turn count {}",
                replay_out.state_digests.len(),
                self.live_state_digests.len()
            )));
        }
        for (idx, (replayed_digest, live_digest)) in replay_out
            .state_digests
            .iter()
            .zip(self.live_state_digests.iter())
            .enumerate()
        {
            if replayed_digest != live_digest {
                return Err(EngineError::SeasonEnded(format!(
                    "replayed post_state_digest diverged from live EvalHost digest at turn {}",
                    idx + 1
                )));
            }
        }
        let metrics = compute_season_metrics(&section);
        Ok((scoreboard, log, section, metrics))
    }

    /// Exports and round-trip verifies the portable Eval [`SeasonReplayLog`] through
    /// [`SeasonReplayLog::from_eval_json_slice`] once the season reaches [`TurnPhase::Completed`].
    pub fn export_replay_log(&self) -> Result<SeasonReplayLog, EngineError> {
        if self.phase() != TurnPhase::Completed {
            return Err(EngineError::InvalidPhase(
                "export_replay_log requires Completed phase".to_owned(),
            ));
        }
        let log = SeasonReplayLog::from_rig_seed(
            self.engine.state().spec.clone(),
            self.layout_seed,
            self.sealed_mandate_deal.clone(),
            self.mandate_commits.clone(),
            self.logged_turns.clone(),
            self.mandate_reveals.clone(),
        )?;
        let json_bytes = log.to_json_pretty_bytes()?;
        SeasonReplayLog::from_eval_json_slice(&json_bytes)
    }

    /// Builds the public, spectator-safe [`EvalSaveSnapshot`] matching [`SeatObservation`].
    pub fn public_save_snapshot(&self) -> Result<EvalSaveSnapshot, EngineError> {
        let state = self.engine.state();
        let (last_swept_turn, _, public_forecasts) = compute_public_forecasts(state);
        let public_receipts = self
            .engine
            .public_receipts()
            .iter()
            .filter(|r| r.turn <= last_swept_turn)
            .cloned()
            .collect();
        Ok(EvalSaveSnapshot {
            format_version: EVAL_SAVE_FORMAT_VERSION,
            layout_seed: replay_layout_seed_for_raw(self.layout_seed)?,
            table_id: state.table_id,
            turn: state.turn,
            phase: state.phase,
            cancel_locked: state.cancel_locked,
            trade_minister: state.trade_minister,
            cartel_chair: state.cartel_chair,
            spec: public_table_spec_from_raw(&state.spec),
            parities: state.parities.clone(),
            seats: state.seats.clone(),
            offers: state.offers.clone(),
            checks: state.checks.clone(),
            escrows: state.escrows.clone(),
            dual_escrows: state.dual_escrows.clone(),
            pledges: state.pledges.clone(),
            audited_pledges: state.audited_pledges.clone(),
            domains: state.domains.clone(),
            council_platforms: state.council_platforms.clone(),
            charters: state.charters.clone(),
            active_shocks: state.active_shocks.clone(),
            public_forecasts,
            next_offer_id: state.next_offer_id,
            next_check_id: state.next_check_id,
            next_escrow_id: state.next_escrow_id,
            delivery_ledger: state.delivery_ledger.clone(),
            last_potentials: state.last_potentials.clone(),
            pending_town_import_cred_spent: state.pending_town_import_cred_spent,
            fill_log: state.fill_log.clone(),
            pending_wash_freezes: state.pending_wash_freezes.clone(),
            eval_vrf_pubkey: state.spec.eval_vrf_pubkey,
            public_receipts,
        })
    }

    /// Serializes the public `.save.json` projection and private `.seal` sidecar bytes
    /// (host-only `pub(crate)` helper behind [`Self::write_save_and_seal`]).
    pub(crate) fn to_save_and_seal_bytes(&self) -> Result<(Vec<u8>, Vec<u8>), EngineError> {
        let save = self.public_save_snapshot()?;
        let save_bytes = serde_json::to_vec_pretty(&save).map_err(|err| {
            EngineError::InvalidConfig(format!("failed to serialize EvalSaveSnapshot: {err}"))
        })?;
        let seal_bytes = self.to_seal_bytes()?;
        Ok((save_bytes, seal_bytes))
    }

    pub(crate) fn to_seal_bytes(&self) -> Result<Vec<u8>, EngineError> {
        self.to_seal_bytes_with_checkpoint_sha(None)
    }

    fn to_seal_bytes_with_checkpoint_sha(
        &self,
        pre_act_checkpoint_sha256: Option<String>,
    ) -> Result<Vec<u8>, EngineError> {
        let seal = EvalSealPayload {
            format_version: EVAL_SEAL_FORMAT_VERSION,
            game_id: self.game_id.clone(),
            pre_act_checkpoint_sha256,
            layout_seed: self.layout_seed,
            vrf_proofs: self.vrf_proofs.clone(),
            engine: self.engine.clone(),
            sealed_mandate_deal: self.sealed_mandate_deal.clone(),
            mandate_commits: self.mandate_commits.clone(),
            mandate_reveals: self.mandate_reveals.clone(),
            logged_turns: self.logged_turns.clone(),
            current_market_actions: self.current_market_actions.clone(),
            current_cancel_lock_at: self.current_cancel_lock_at,
            current_commitments: self.current_commitments.clone(),
            current_reveals: self.current_reveals.clone(),
            bot_tie_seed: self.bot_tie_seed,
            live_state_digests: self.live_state_digests.clone(),
            turn_salts: self.turn_salts.clone(),
            recorded_act_submissions: self.recorded_act_submissions.clone(),
        };
        serde_json::to_vec(&seal).map_err(|err| {
            EngineError::InvalidConfig(format!("failed to serialize Eval seal sidecar: {err}"))
        })
    }

    /// Reconstructs and validates an [`EvalHost`] from `.save.json` and `.seal` sidecar bytes
    /// (host-only `pub(crate)` helper behind [`Self::read_save_and_seal`]).
    pub(crate) fn from_save_and_seal_bytes(
        save_bytes: &[u8],
        seal_bytes: &[u8],
    ) -> Result<Self, EngineError> {
        #[derive(serde::Deserialize)]
        struct FormatVersionHeader {
            format_version: u32,
        }
        if let Ok(hdr) = serde_json::from_slice::<FormatVersionHeader>(save_bytes)
            && hdr.format_version != EVAL_SAVE_FORMAT_VERSION
        {
            return Err(EngineError::UnsupportedReplayVersion {
                found: hdr.format_version,
                newest_known: EVAL_SAVE_FORMAT_VERSION,
            });
        }
        let save: EvalSaveSnapshot = serde_json::from_slice(save_bytes).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "MALFORMED_SAVE_JSON: Eval save JSON does not parse: {err}"
            ))
        })?;
        if save.format_version != EVAL_SAVE_FORMAT_VERSION {
            return Err(EngineError::UnsupportedReplayVersion {
                found: save.format_version,
                newest_known: EVAL_SAVE_FORMAT_VERSION,
            });
        }

        let seal = validate_eval_seal_bytes(seal_bytes)?;
        let expected_save_seed = replay_layout_seed_for_raw(seal.layout_seed)?;
        if save.layout_seed != expected_save_seed {
            return Err(EngineError::InvalidConfig(
                "SEAL_LAYOUT_SEED_MISMATCH: Eval save layout_seed does not match seal layout_seed"
                    .to_owned(),
            ));
        }

        let host = Self {
            game_id: seal.game_id,
            layout_seed: seal.layout_seed,
            bot_tie_seed: seal.bot_tie_seed,
            vrf_proofs: seal.vrf_proofs,
            engine: seal.engine,
            sealed_mandate_deal: seal.sealed_mandate_deal,
            mandate_commits: seal.mandate_commits,
            mandate_reveals: seal.mandate_reveals,
            logged_turns: seal.logged_turns,
            live_state_digests: seal.live_state_digests,
            current_market_actions: seal.current_market_actions,
            current_cancel_lock_at: seal.current_cancel_lock_at,
            current_commitments: seal.current_commitments,
            current_reveals: seal.current_reveals,
            turn_salts: seal.turn_salts,
            recorded_act_submissions: seal.recorded_act_submissions,
            active_pre_act_checkpoint_sha256: None,
        };
        if host.public_save_snapshot()? != save {
            return Err(EngineError::InvalidConfig(
                "SAVE_SEAL_SNAPSHOT_MISMATCH: Eval save snapshot does not match verified seal state"
                    .to_owned(),
            ));
        }
        Ok(host)
    }

    /// Writes `.save.json` and `.seal` atomically (`tmp` + `flush` + `sync_all` + `rename`).
    pub fn write_save_and_seal(
        &self,
        save_path: &Path,
        seal_path: &Path,
    ) -> Result<(), EngineError> {
        let (save_bytes, seal_bytes) = self.to_save_and_seal_bytes()?;
        write_atomic_bytes(seal_path, &seal_bytes)?;
        write_atomic_bytes(save_path, &save_bytes)?;
        Ok(())
    }

    /// Reads and validates a saved Evaluation Mode game from `.save.json` and `.seal`.
    pub fn read_save_and_seal(save_path: &Path, seal_path: &Path) -> Result<Self, EngineError> {
        let save_bytes = fs::read(save_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to read Eval save {}: {err}",
                save_path.display()
            ))
        })?;
        let seal_bytes = fs::read(seal_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to read Eval seal {}: {err}",
                seal_path.display()
            ))
        })?;
        Self::from_save_and_seal_bytes(&save_bytes, &seal_bytes)
    }

    /// Removes a private `.seal` sidecar file. Succeeds if the file is already absent.
    pub fn wipe_seal_file(seal_path: &Path) -> Result<(), EngineError> {
        match fs::remove_file(seal_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EngineError::InvalidConfig(format!(
                "failed to remove Eval seal {}: {err}",
                seal_path.display()
            ))),
        }
    }

    /// Writes `eval-<id>.save.json` and `eval-<id>.seal` inside `game_dir` from a single `(dir, id)`
    /// pair so callers cannot swap public and private paths (`R8`).
    pub fn write_game_save_and_seal(
        &self,
        game_dir: &Path,
    ) -> Result<(PathBuf, PathBuf), EngineError> {
        let (save_path, seal_path) = eval_game_save_and_seal_paths(game_dir, self.table_id());
        self.write_save_and_seal(&save_path, &seal_path)?;
        Ok((save_path, seal_path))
    }

    /// Reads and validates `eval-<id>.save.json` and `eval-<id>.seal` from `game_dir` (`R8`).
    pub fn read_game_save_and_seal(
        game_dir: &Path,
        table_id: TableId,
    ) -> Result<Self, EngineError> {
        let (save_path, seal_path) = eval_game_save_and_seal_paths(game_dir, table_id);
        Self::read_save_and_seal(&save_path, &seal_path)
    }

    /// Forks a D2 duplicate table from `eval-<id>.save.json` and `eval-<id>.seal` in `game_dir`,
    /// assigning a fresh per-game identifier distinct from the source table (`R5`, `R8`).
    pub fn fork_duplicate_from_game_dir(
        game_dir: &Path,
        table_id: TableId,
    ) -> Result<Self, EngineError> {
        let (save_path, seal_path) = eval_game_save_and_seal_paths(game_dir, table_id);
        Self::fork_duplicate_from_turn_0(&save_path, &seal_path)
    }

    /// Writes `checkpoints/day-DD-pre-act.save.json` and `checkpoints/day-DD-pre-act.seal`
    /// atomically at the start of an act round before any seat's act reply is read, binding
    /// this host's active pre-act checkpoint SHA-256 for [`Self::write_act_recorded_seal`].
    pub fn write_pre_act_checkpoint(
        &mut self,
        checkpoints_dir: &Path,
    ) -> Result<(PathBuf, PathBuf), EngineError> {
        self.ensure_pre_act_state("write_pre_act_checkpoint")?;
        let day = self.turn();
        let save_path = checkpoints_dir.join(format!("day-{day:02}-pre-act.save.json"));
        let seal_path = checkpoints_dir.join(format!("day-{day:02}-pre-act.seal"));
        let (save_bytes, seal_bytes) = self.to_save_and_seal_bytes()?;
        write_atomic_bytes(&seal_path, &seal_bytes)?;
        write_atomic_bytes(&save_path, &save_bytes)?;
        self.active_pre_act_checkpoint_sha256 =
            Some(compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes));
        Ok((save_path, seal_path))
    }

    /// Writes `checkpoints/day-DD-act-recorded.seal` atomically after all act replies are applied
    /// and before [`Self::sweep_turn`], binding `game_id` and the active pre-act checkpoint pair's
    /// SHA-256.
    pub fn write_act_recorded_seal(&self, checkpoints_dir: &Path) -> Result<PathBuf, EngineError> {
        if self.phase() != TurnPhase::Reveal {
            return Err(EngineError::InvalidPhase(
                "write_act_recorded_seal requires Reveal phase after apply_act_submissions"
                    .to_owned(),
            ));
        }
        let n = usize::from(self.seat_count());
        if self.recorded_act_submissions.len() != n {
            return Err(EngineError::InvalidPhase(format!(
                "MISSING_RECORDED_ACT_SUBMISSIONS: expected {n} seat submissions, got {}",
                self.recorded_act_submissions.len()
            )));
        }
        let expected_ckpt_sha = self
            .active_pre_act_checkpoint_sha256
            .as_deref()
            .ok_or_else(|| {
                EngineError::InvalidPhase(
                    "MISSING_PRE_ACT_CHECKPOINT: write_pre_act_checkpoint must precede write_act_recorded_seal"
                        .to_owned(),
                )
            })?;
        let day = self.turn();
        let pre_save_path = checkpoints_dir.join(format!("day-{day:02}-pre-act.save.json"));
        let pre_seal_path = checkpoints_dir.join(format!("day-{day:02}-pre-act.seal"));
        let pre_save_bytes = fs::read(&pre_save_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "PRE_ACT_CHECKPOINT_MISMATCH: missing pre-act save {}: {err}",
                pre_save_path.display()
            ))
        })?;
        let pre_seal_bytes = fs::read(&pre_seal_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "PRE_ACT_CHECKPOINT_MISMATCH: missing pre-act seal {}: {err}",
                pre_seal_path.display()
            ))
        })?;
        let disk_sha = compute_pre_act_pair_sha256_hex(&pre_save_bytes, &pre_seal_bytes);
        if disk_sha != expected_ckpt_sha {
            return Err(EngineError::InvalidConfig(
                "PRE_ACT_CHECKPOINT_MISMATCH: on-disk day-DD-pre-act checkpoint does not match host pre-act digest"
                    .to_owned(),
            ));
        }
        let seal_path = checkpoints_dir.join(format!("day-{day:02}-act-recorded.seal"));
        let seal_bytes =
            self.to_seal_bytes_with_checkpoint_sha(Some(expected_ckpt_sha.to_owned()))?;
        write_atomic_bytes(&seal_path, &seal_bytes)?;
        Ok(seal_path)
    }

    /// Reads and validates the per-seat act submissions recorded inside
    /// `checkpoints/day-DD-act-recorded.seal`, verifying that the recorded seal belongs to this
    /// host's `game_id`, pre-act checkpoint SHA-256, table, turn, and origin (`R5`).
    pub fn read_recorded_act_submissions(
        &self,
        recorded_seal_path: &Path,
    ) -> Result<BTreeMap<u8, EvalActSubmission>, EngineError> {
        let seal = self.read_and_verify_recorded_seal_for_host(
            "read_recorded_act_submissions",
            recorded_seal_path,
        )?;
        Ok(seal.recorded_act_submissions)
    }

    /// Loads `checkpoints/day-DD-act-recorded.seal`, verifies it matches this host's `game_id`,
    /// pre-act checkpoint SHA-256, table, turn, layout seed, VRF proofs, and mandate deal, and
    /// applies the recorded submissions.
    pub fn apply_recorded_act_submissions_from_seal(
        &mut self,
        recorded_seal_path: &Path,
    ) -> Result<EvalActApplyOutcome, EngineError> {
        let seal = self.read_and_verify_recorded_seal_for_host(
            "apply_recorded_act_submissions_from_seal",
            recorded_seal_path,
        )?;
        self.apply_act_submissions(&seal.recorded_act_submissions)
    }

    /// Forks an independent [`EvalHost`] from a verified turn-0 or pre-act checkpoint pair,
    /// retaining `game_id` and binding `active_pre_act_checkpoint_sha256` to that pair.
    pub fn fork_from_checkpoint(save_path: &Path, seal_path: &Path) -> Result<Self, EngineError> {
        let save_bytes = fs::read(save_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to read Eval checkpoint save {}: {err}",
                save_path.display()
            ))
        })?;
        let seal_bytes = fs::read(seal_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to read Eval checkpoint seal {}: {err}",
                seal_path.display()
            ))
        })?;
        let mut host = Self::from_save_and_seal_bytes(&save_bytes, &seal_bytes)?;
        host.ensure_pre_act_state("fork_from_checkpoint")?;
        host.active_pre_act_checkpoint_sha256 =
            Some(compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes));
        Ok(host)
    }

    /// Forks a fresh D2 duplicate [`EvalHost`] from a turn-0 checkpoint pair, assigning a distinct
    /// OS-CSPRNG `game_id` so recorded seals from one duplicate are refused on another (`R5`).
    pub fn fork_duplicate_from_turn_0(
        save_path: &Path,
        seal_path: &Path,
    ) -> Result<Self, EngineError> {
        let mut host = Self::read_save_and_seal(save_path, seal_path)?;
        if host.turn() != 1 || !host.logged_turns.is_empty() {
            return Err(EngineError::InvalidPhase(format!(
                "INVALID_TURN_0_CHECKPOINT: expected turn 1 with 0 logged turns, got turn {} ({} logged)",
                host.turn(),
                host.logged_turns.len()
            )));
        }
        host.ensure_pre_act_state("fork_duplicate_from_turn_0")?;
        let src_game_id = host.game_id.clone();
        let mut next_game_id = generate_eval_game_id()?;
        while next_game_id == src_game_id {
            next_game_id = generate_eval_game_id()?;
        }
        host.game_id = next_game_id;
        host.active_pre_act_checkpoint_sha256 = None;
        Ok(host)
    }

    fn read_and_verify_recorded_seal_for_host(
        &self,
        op: &str,
        recorded_seal_path: &Path,
    ) -> Result<EvalSealPayload, EngineError> {
        self.ensure_pre_act_state(op)?;
        let expected_ckpt_sha = self
            .active_pre_act_checkpoint_sha256
            .as_deref()
            .ok_or_else(|| {
                EngineError::InvalidPhase(format!(
                    "MISSING_PRE_ACT_CHECKPOINT: {op} requires an active pre-act checkpoint from fork_from_checkpoint or write_pre_act_checkpoint"
                ))
            })?;
        let seal = read_verified_act_recorded_seal(recorded_seal_path)?;
        let st = seal.engine.state();
        if st.table_id != self.table_id() {
            return Err(EngineError::InvalidConfig(format!(
                "RECORDED_SEAL_TABLE_MISMATCH: expected table {}, got {}",
                self.table_id().0,
                st.table_id.0
            )));
        }
        if st.turn != self.turn() {
            return Err(EngineError::InvalidConfig(format!(
                "RECORDED_SEAL_TURN_MISMATCH: expected turn {}, got {}",
                self.turn(),
                st.turn
            )));
        }
        if seal.layout_seed != self.layout_seed
            || seal.vrf_proofs != self.vrf_proofs
            || seal.sealed_mandate_deal != self.sealed_mandate_deal
        {
            return Err(EngineError::InvalidConfig(
                "RECORDED_SEAL_ORIGIN_MISMATCH: recorded act seal does not match forked host"
                    .to_owned(),
            ));
        }
        if seal.game_id != self.game_id {
            return Err(EngineError::InvalidConfig(format!(
                "RECORDED_SEAL_GAME_ID_MISMATCH: expected game_id {}, got {}",
                self.game_id, seal.game_id
            )));
        }
        if seal.pre_act_checkpoint_sha256.as_deref() != Some(expected_ckpt_sha) {
            return Err(EngineError::InvalidConfig(
                "RECORDED_SEAL_CHECKPOINT_MISMATCH: recorded act seal checkpoint SHA-256 does not match this host's pre-act save+seal pair"
                    .to_owned(),
            ));
        }
        Ok(seal)
    }

    fn ensure_pre_act_state(&self, op: &str) -> Result<(), EngineError> {
        if self.phase() != TurnPhase::TalkAndMarket {
            return Err(EngineError::InvalidPhase(format!(
                "INVALID_CHECKPOINT_PHASE: {op} requires TalkAndMarket, got {:?}",
                self.phase()
            )));
        }
        if self.cancel_locked() {
            return Err(EngineError::InvalidPhase(format!(
                "INVALID_PRE_ACT_STATE: {op} requires cancel_locked == false"
            )));
        }
        if !self.current_market_actions.is_empty() {
            return Err(EngineError::InvalidPhase(format!(
                "INVALID_PRE_ACT_STATE: {op} requires zero applied market actions on current turn"
            )));
        }
        if !self.recorded_act_submissions.is_empty() {
            return Err(EngineError::InvalidPhase(format!(
                "INVALID_PRE_ACT_STATE: {op} requires empty recorded_act_submissions"
            )));
        }
        Ok(())
    }
}

fn replay_layout_seed_for_raw(layout_seed: u64) -> Result<ReplayLayoutSeed, EngineError> {
    if layout_seed >= SEALED_LAYOUT_SEED_MIN {
        ReplayLayoutSeed::sealed_checked(layout_seed)
            .map_err(|err| EngineError::InvalidConfig(err.to_string()))
    } else {
        ReplayLayoutSeed::try_clear(layout_seed)
            .map_err(|err| EngineError::InvalidConfig(err.to_string()))
    }
}

fn read_verified_act_recorded_seal(
    recorded_seal_path: &Path,
) -> Result<EvalSealPayload, EngineError> {
    let seal_bytes = fs::read(recorded_seal_path).map_err(|err| {
        EngineError::InvalidConfig(format!(
            "failed to read Eval recorded-act seal {}: {err}",
            recorded_seal_path.display()
        ))
    })?;
    let seal = validate_eval_seal_bytes(&seal_bytes)?;
    if seal.engine.state().phase != TurnPhase::Reveal {
        return Err(EngineError::InvalidPhase(format!(
            "INVALID_ACT_RECORDED_SEAL_PHASE: expected Reveal phase, got {:?}",
            seal.engine.state().phase
        )));
    }
    let n = usize::from(seal.engine.state().spec.seat_count);
    if seal.recorded_act_submissions.len() != n {
        return Err(EngineError::InvalidConfig(format!(
            "INVALID_ACT_RECORDED_SUBMISSIONS: expected {n} seat submissions, got {}",
            seal.recorded_act_submissions.len()
        )));
    }
    if seal.pre_act_checkpoint_sha256.is_none() {
        return Err(EngineError::InvalidConfig(
            "MISSING_ACT_RECORDED_CHECKPOINT_SHA256: recorded act seal must carry pre_act_checkpoint_sha256"
                .to_owned(),
        ));
    }
    Ok(seal)
}

fn eval_game_save_and_seal_paths(game_dir: &Path, table_id: TableId) -> (PathBuf, PathBuf) {
    let id = table_id.0;
    (
        game_dir.join(format!("eval-{id}.save.json")),
        game_dir.join(format!("eval-{id}.seal")),
    )
}

fn is_lower_ascii_hex_len(raw: &str, expected_len: usize) -> bool {
    raw.len() == expected_len
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn generate_eval_game_id() -> Result<String, EngineError> {
    let entropy = draw_nonzero_256_bits()?;
    Ok(hex::encode(&entropy[..16]))
}

fn compute_pre_act_pair_sha256_hex(save_bytes: &[u8], seal_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/eval-pre-act-pair/v1|");
    hasher.update((save_bytes.len() as u64).to_le_bytes());
    hasher.update(save_bytes);
    hasher.update((seal_bytes.len() as u64).to_le_bytes());
    hasher.update(seal_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    hex::encode(digest)
}

fn validate_eval_seal_bytes(seal_bytes: &[u8]) -> Result<EvalSealPayload, EngineError> {
    #[derive(serde::Deserialize)]
    struct FormatVersionHeader {
        format_version: u32,
    }
    if let Ok(hdr) = serde_json::from_slice::<FormatVersionHeader>(seal_bytes)
        && hdr.format_version != EVAL_SEAL_FORMAT_VERSION
    {
        return Err(EngineError::UnsupportedReplayVersion {
            found: hdr.format_version,
            newest_known: EVAL_SEAL_FORMAT_VERSION,
        });
    }
    let seal: EvalSealPayload = serde_json::from_slice(seal_bytes).map_err(|err| {
        EngineError::InvalidConfig(format!(
            "MALFORMED_SEAL_JSON: Eval seal JSON does not parse at line {}, column {} ({:?})",
            err.line(),
            err.column(),
            err.classify()
        ))
    })?;
    if seal.format_version != EVAL_SEAL_FORMAT_VERSION {
        return Err(EngineError::UnsupportedReplayVersion {
            found: seal.format_version,
            newest_known: EVAL_SEAL_FORMAT_VERSION,
        });
    }
    if !is_lower_ascii_hex_len(&seal.game_id, 32) {
        return Err(EngineError::InvalidConfig(
            "INVALID_SEAL_GAME_ID: EvalHost seal requires a 32-hex game_id".to_owned(),
        ));
    }
    if let Some(ref ckpt_sha) = seal.pre_act_checkpoint_sha256
        && !is_lower_ascii_hex_len(ckpt_sha, 64)
    {
        return Err(EngineError::InvalidConfig(
            "INVALID_ACT_RECORDED_CHECKPOINT_SHA256: pre_act_checkpoint_sha256 must be 64 lowercase hex characters"
                .to_owned(),
        ));
    }

    let spec = &seal.engine.state().spec;
    if spec.mode != TableMode::Eval {
        return Err(EngineError::InvalidConfig(format!(
            "INVALID_SEAL_TABLE_MODE: EvalHost seal requires TableMode::Eval, got {:?}",
            spec.mode
        )));
    }
    if spec.drand_schedule.is_some() {
        return Err(EngineError::InvalidConfig(
            "INVALID_SEAL_DRAND_SCHEDULE: EvalHost seal forbids drand_schedule".to_owned(),
        ));
    }
    let eval_pk = spec.eval_vrf_pubkey.ok_or_else(|| {
        EngineError::InvalidConfig(
            "MISSING_SEAL_EVAL_VRF_PUBKEY: EvalHost seal requires TableSpec::eval_vrf_pubkey"
                .to_owned(),
        )
    })?;
    spec.validate()?;

    if seal.vrf_proofs.len() != usize::from(HARD_TURN_CAP) {
        return Err(EngineError::InvalidConfig(format!(
            "INVALID_SEAL_VRF_PROOF_COUNT: EvalHost seal expected {HARD_TURN_CAP} VRF proofs, got {}",
            seal.vrf_proofs.len()
        )));
    }
    let table_id = seal.engine.state().table_id;
    for (idx, proof) in seal.vrf_proofs.iter().enumerate() {
        if proof.len() != EVAL_VRF_PROOF_BYTES {
            return Err(EngineError::InvalidConfig(format!(
                "INVALID_SEAL_VRF_PROOF_LEN: EvalHost seal proof {} has length {} (expected {EVAL_VRF_PROOF_BYTES})",
                idx + 1,
                proof.len()
            )));
        }
        verify_and_extract_eval_vrf_beacon(eval_pk, table_id, Turn((idx + 1) as u8), proof)?;
    }

    let sealed_context = seal.layout_seed >= SEALED_LAYOUT_SEED_MIN;
    let candidates =
        check_deal_seed_independence(seal.layout_seed, &seal.sealed_mandate_deal, sealed_context)?;
    check_eval_vrf_independence(candidates, eval_pk, sealed_context)?;
    if !verify_sealed_mandate_deal(
        table_id,
        spec.seat_count,
        &spec.mandate_deck,
        &spec.dealt_mandate_commits,
        &seal.sealed_mandate_deal,
    )? {
        return Err(EngineError::InvalidConfig(
            "INVALID_SEALED_MANDATE_DEAL: EvalHost seal sealed_mandate_deal failed verification"
                .to_owned(),
        ));
    }
    let n = usize::from(spec.seat_count);
    if seal.mandate_commits.len() != n || seal.mandate_reveals.len() != n {
        return Err(EngineError::InvalidConfig(
            "INVALID_SEAL_MANDATE_LEN: EvalHost seal mandate commit/reveal lengths do not match seat_count"
                .to_owned(),
        ));
    }
    if seal.live_state_digests.len() != seal.logged_turns.len() {
        return Err(EngineError::InvalidConfig(format!(
            "INVALID_SEAL_DIGEST_LEN: EvalHost seal live_state_digests len {} does not match logged_turns len {}",
            seal.live_state_digests.len(),
            seal.logged_turns.len()
        )));
    }
    if seal.turn_salts.len() != usize::from(HARD_TURN_CAP)
        || seal
            .turn_salts
            .iter()
            .any(|per_turn| per_turn.len() != n || per_turn.iter().any(|salt| salt.is_degenerate()))
    {
        return Err(EngineError::InvalidConfig(
            "INVALID_SEAL_TURN_SALTS: EvalHost seal turn_salts invalid or wrong shape".to_owned(),
        ));
    }
    for &seat_idx in seal.recorded_act_submissions.keys() {
        if seat_idx >= spec.seat_count {
            return Err(EngineError::InvalidSeat(seat_idx));
        }
    }
    if !seal.recorded_act_submissions.is_empty()
        && seal.engine.state().phase == TurnPhase::Reveal
        && seal.recorded_act_submissions.len() != n
    {
        return Err(EngineError::InvalidConfig(format!(
            "INVALID_RECORDED_SUBMISSIONS_LEN: expected {n} recorded submissions in Reveal phase, got {}",
            seal.recorded_act_submissions.len()
        )));
    }

    Ok(seal)
}

fn derive_all_vrf_proofs(
    eval_seed: SealedEvalSeed,
    table_id: TableId,
) -> Result<Vec<Vec<u8>>, EngineError> {
    let mut vrf_proofs = Vec::with_capacity(usize::from(HARD_TURN_CAP));
    for t in 1..=HARD_TURN_CAP {
        let (_, proof) = eval_seed.prove_turn(table_id, Turn(t))?;
        vrf_proofs.push(proof.to_vec());
    }
    Ok(vrf_proofs)
}

fn derive_all_turn_commit_salts(
    table_id: TableId,
    seat_count: u8,
) -> Result<Vec<Vec<RevealSalt>>, EngineError> {
    let root_entropy = draw_nonzero_256_bits()?;
    let mut all_turns = Vec::with_capacity(usize::from(HARD_TURN_CAP));
    for turn in 1..=HARD_TURN_CAP {
        let mut salts = Vec::with_capacity(usize::from(seat_count));
        for s in 0..seat_count {
            let mut hasher = Sha256::new();
            hasher.update(b"league-eval-order-salt-v1");
            hasher.update(root_entropy);
            hasher.update(table_id.0.to_le_bytes());
            hasher.update([turn, s]);
            let mut bytes: [u8; 32] = hasher.finalize().into();
            if bytes == [0u8; 32] {
                bytes[0] = 1;
            }
            salts.push(RevealSalt(bytes));
        }
        all_turns.push(salts);
    }
    Ok(all_turns)
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), EngineError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to create directory {}: {err}",
                parent.display()
            ))
        })?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("eval");
    let tmp_path = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let mut file = fs::File::create(&tmp_path).map_err(|err| {
        EngineError::InvalidConfig(format!(
            "failed to create temp file {}: {err}",
            tmp_path.display()
        ))
    })?;
    if let Err(err) = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&tmp_path);
        return Err(EngineError::InvalidConfig(format!(
            "failed to write temp file {}: {err}",
            tmp_path.display()
        )));
    }
    drop(file);
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(EngineError::InvalidConfig(format!(
            "failed to rename {} -> {}: {err}",
            tmp_path.display(),
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::DrandSchedule;
    use crate::contracts::{OfferProposal, OfferSide};

    fn must_ok<T, E: fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
        match res {
            Ok(v) => v,
            Err(err) => panic!("{ctx}: {err}"),
        }
    }

    fn explicit_host(tid: TableId, layout_seed: u64, byte_tag: u8) -> EvalHost {
        let eval_seed = must_ok(
            SealedEvalSeed::try_from_bytes([byte_tag; 32]),
            "SealedEvalSeed::try_from_bytes",
        );
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([byte_tag + 1 + i; 32])).collect();
        must_ok(
            EvalHost::from_explicit_setup(
                tid,
                4,
                layout_seed,
                [byte_tag + 9; 32],
                &salts,
                eval_seed,
                77,
            ),
            "EvalHost::from_explicit_setup",
        )
    }

    #[test]
    fn eval_host_debug_redacts_secrets_and_new_rejects_bad_configs() {
        let mut host = explicit_host(TableId(910_001), 7, 0x30);
        let dbg = format!("{host:?} {host:#?}");
        assert!(dbg.contains("EvalHost"));
        assert!(dbg.contains("<hidden>"));
        assert!(!dbg.contains("30303030"));

        // R6: verify Debug omits all secrets the host actually holds (deal seed + all 4 salts)
        // across hex, Rust decimal byte list, and JSON number array encodings.
        let held_secrets: [[u8; 32]; 5] = [
            [0x39; 32], // deal_seed in explicit_host(.., 0x30)
            [0x31; 32], // seat 0 salt
            [0x32; 32], // seat 1 salt
            [0x33; 32], // seat 2 salt
            [0x34; 32], // seat 3 salt
        ];
        for secret_bytes in &held_secrets {
            let hex_enc = hex::encode(secret_bytes);
            let dec_enc = format!("{secret_bytes:?}");
            let json_enc = must_ok(
                serde_json::to_string(secret_bytes.as_slice()),
                "ser secret_bytes",
            );
            for enc in [&hex_enc, &dec_enc, &json_enc] {
                assert!(
                    !dbg.contains(enc),
                    "EvalHost Debug leaked held secret encoding `{enc}`"
                );
            }
        }

        let sub = EvalActSubmission::default();
        let sub_dbg = format!("{sub:?}");
        assert!(sub_dbg.contains("EvalActSubmission"));
        assert!(sub_dbg.contains("<hidden>"));

        let (mut spec, deal, eval_seed) = must_ok(
            TableSpec::standard_eval_chiral_ring_with_setup(TableId(910_002), 4, 7),
            "standard_eval_chiral_ring_with_setup",
        );
        spec.drand_schedule = Some(DrandSchedule {
            turn_1_close_unix: 1_700_000_000,
            turn_period_secs: 30,
        });
        assert!(
            EvalHost::new(spec, 7, deal, eval_seed, 1)
                .err()
                .is_some_and(|e| e.to_string().contains("INVALID_EVAL_DRAND_SCHEDULE"))
        );

        let (mut spec2, deal2, eval_seed2) = must_ok(
            TableSpec::standard_eval_chiral_ring_with_setup(TableId(910_003), 4, 7),
            "standard_eval_chiral_ring_with_setup",
        );
        spec2.eval_vrf_pubkey = None;
        assert!(
            EvalHost::new(spec2, 7, deal2, eval_seed2, 1)
                .err()
                .is_some_and(|e| e.to_string().contains("MISSING_EVAL_VRF_PUBKEY"))
        );

        let (spec3, deal3, _) = must_ok(
            TableSpec::standard_eval_chiral_ring_with_setup(TableId(910_004), 4, 7),
            "standard_eval_chiral_ring_with_setup",
        );
        let wrong_seed = must_ok(SealedEvalSeed::try_from_bytes([0x55; 32]), "wrong_seed");
        assert!(
            EvalHost::new(spec3, 7, deal3, wrong_seed, 1)
                .err()
                .is_some_and(|e| e.to_string().contains("EVAL_VRF_PUBKEY_MISMATCH"))
        );

        let err_seat = host.mandate_for_seat(Seat(9)).err();
        assert!(err_seat.is_some());
        assert!(host.preview_regent_bundle_for_seat(Seat(0)).is_ok());
        assert!(host.turn_reveal_salt(Seat(0)).is_ok());
        assert!(host.recorded_act_submissions().is_empty());
        assert!(host.export_replay_log().is_err());
        assert!(host.finalize_eval_season().is_err());
    }

    #[test]
    fn eval_host_refuses_tampered_save_and_seal_variants() {
        let mut host = explicit_host(TableId(910_010), 7, 0x40);
        let subs = BTreeMap::from([(
            0,
            EvalActSubmission {
                market_actions: vec![MarketAction::PostOffer {
                    seat: 1, // cross-seat unauthorized!
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(1),
                        amount: 100,
                        price_per_unit_cred: 1_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }],
                pass_b_actions: vec![],
                bundle: SealedOrderBundle::default(),
            },
        )]);
        let outcome = must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
        assert_eq!(
            outcome
                .seats
                .get(&0)
                .map(|r| r.refused_market_actions.clone()),
            Some(vec![(0, "UNAUTHORIZED".to_owned())])
        );

        let (save_bytes, seal_bytes) =
            must_ok(host.to_save_and_seal_bytes(), "to_save_and_seal_bytes");

        // Save version mismatch
        let mut bad_ver_save: serde_json::Value =
            must_ok(serde_json::from_slice(&save_bytes), "parse save");
        bad_ver_save["format_version"] = serde_json::json!(99);
        let bad_ver_bytes = must_ok(serde_json::to_vec(&bad_ver_save), "ser bad save");
        assert!(matches!(
            EvalHost::from_save_and_seal_bytes(&bad_ver_bytes, &seal_bytes),
            Err(EngineError::UnsupportedReplayVersion {
                found: 99,
                newest_known: 1
            })
        ));

        // Seal version mismatch
        let mut bad_ver_seal: serde_json::Value =
            must_ok(serde_json::from_slice(&seal_bytes), "parse seal");
        bad_ver_seal["format_version"] = serde_json::json!(99);
        let bad_seal_bytes = must_ok(serde_json::to_vec(&bad_ver_seal), "ser bad seal");
        assert!(matches!(
            EvalHost::from_save_and_seal_bytes(&save_bytes, &bad_seal_bytes),
            Err(EngineError::UnsupportedReplayVersion {
                found: 99,
                newest_known: 1
            })
        ));

        // Tampered seal VRF proof count
        let mut bad_vrf_seal: serde_json::Value =
            must_ok(serde_json::from_slice(&seal_bytes), "parse seal");
        bad_vrf_seal["vrf_proofs"] = serde_json::json!([]);
        let bad_vrf_bytes = must_ok(serde_json::to_vec(&bad_vrf_seal), "ser bad vrf seal");
        assert!(
            EvalHost::from_save_and_seal_bytes(&save_bytes, &bad_vrf_bytes)
                .err()
                .is_some_and(|e| e.to_string().contains("INVALID_SEAL_VRF_PROOF_COUNT"))
        );

        // Tampered seal turn_salts
        let mut bad_salts_seal: serde_json::Value =
            must_ok(serde_json::from_slice(&seal_bytes), "parse seal");
        bad_salts_seal["turn_salts"] = serde_json::json!([]);
        let bad_salts_bytes = must_ok(serde_json::to_vec(&bad_salts_seal), "ser bad salts");
        assert!(
            EvalHost::from_save_and_seal_bytes(&save_bytes, &bad_salts_bytes)
                .err()
                .is_some_and(|e| e.to_string().contains("INVALID_SEAL_TURN_SALTS"))
        );

        // Tampered save layout_seed
        let mut bad_seed_save: serde_json::Value =
            must_ok(serde_json::from_slice(&save_bytes), "parse save");
        bad_seed_save["layout_seed"] = serde_json::json!(888);
        let bad_seed_bytes = must_ok(serde_json::to_vec(&bad_seed_save), "ser bad seed save");
        assert!(
            EvalHost::from_save_and_seal_bytes(&bad_seed_bytes, &seal_bytes)
                .err()
                .is_some_and(|e| e.to_string().contains("SEAL_LAYOUT_SEED_MISMATCH"))
        );

        // Tampered save snapshot state -> SAVE_SEAL_SNAPSHOT_MISMATCH
        let mut bad_state_save: serde_json::Value =
            must_ok(serde_json::from_slice(&save_bytes), "parse save");
        bad_state_save["turn"] = serde_json::json!(9);
        let bad_state_bytes = must_ok(serde_json::to_vec(&bad_state_save), "ser bad state save");
        assert!(
            EvalHost::from_save_and_seal_bytes(&bad_state_bytes, &seal_bytes)
                .err()
                .is_some_and(|e| e.to_string().contains("SAVE_SEAL_SNAPSHOT_MISMATCH"))
        );
    }

    #[test]
    fn eval_host_sealed_pool_seed_and_bundle_fallback_work_cleanly() {
        let sealed_seed = SEALED_LAYOUT_SEED_MIN + 42;
        let mut host = explicit_host(TableId(910_020), sealed_seed, 0x60);
        let snap = must_ok(host.public_save_snapshot(), "public_save_snapshot");
        assert!(snap.layout_seed.is_sealed());
        assert_eq!(snap.layout_seed.clear_seed(), None);

        let mut bad_bundle = SealedOrderBundle::default();
        bad_bundle
            .atomic_swaps
            .push(crate::commit::AtomicSwapProposal {
                swap_id: 1,
                maker: 0,
                taker: 1,
                maker_asset: AssetId::Raw(99), // out-of-range asset triggers bundle fallback
                maker_amount: 100,
                taker_asset: AssetId::Cred,
                taker_amount: 100,
                domain_id: None,
            });
        let subs = BTreeMap::from([(
            0,
            EvalActSubmission {
                market_actions: vec![],
                pass_b_actions: vec![],
                bundle: bad_bundle,
            },
        )]);
        let outcome = must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
        assert!(
            outcome
                .seats
                .get(&0)
                .and_then(|r| r.refused_bundle_code.as_ref())
                .is_some()
        );
        let receipt = must_ok(host.sweep_turn(), "sweep_turn");
        assert_eq!(receipt.turn, 1);

        let (save_bytes, seal_bytes) =
            must_ok(host.to_save_and_seal_bytes(), "to_save_and_seal_bytes");
        let rt = must_ok(
            EvalHost::from_save_and_seal_bytes(&save_bytes, &seal_bytes),
            "round-trip",
        );
        let (save_rt, seal_rt) = must_ok(rt.to_save_and_seal_bytes(), "to_save_and_seal_bytes rt");
        assert_eq!(save_bytes, save_rt);
        assert_eq!(seal_bytes, seal_rt);
    }

    #[test]
    fn eval_host_planted_mandate_struct_canary_stays_in_seal_only_and_preconditions_hold() {
        let mut host = explicit_host(TableId(910_030), 7, 0x70);
        // Plant unique PrivateMandate field values directly in sealed_mandate_deal (distinct from
        // PublicTableSpec.mandate_deck) plus unique 32-byte ASCII RevealSalt bytes.
        host.sealed_mandate_deal.seat_reveals[0].mandate = PrivateMandate {
            kind: crate::commit::MandateKind::MinProjectEquity {
                target: 73_918_264_501,
            },
            bonus_equity: 6_543_219_876,
        };
        host.sealed_mandate_deal.seat_reveals[0].salt =
            RevealSalt(*b"CANARY_MANDATE_SALT_CARD10_SEAT0");

        let tmp = std::env::temp_dir().join(format!("ml_eval_unit_canary_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let ckpt_dir = tmp.join("checkpoints");
        let (save_p, seal_p) = must_ok(
            host.write_pre_act_checkpoint(&ckpt_dir),
            "write_pre_act_checkpoint",
        );
        let save_text = must_ok(fs::read_to_string(&save_p), "read pre-act save");
        let seal_text = must_ok(fs::read_to_string(&seal_p), "read pre-act seal");

        let expected_salt_hex = "43414e4152595f4d414e444154455f53414c545f4341524431305f5345415430";
        for canary in ["73918264501", "6543219876", expected_salt_hex] {
            assert!(
                seal_text.contains(canary),
                "mandate canary `{canary}` missing from .seal"
            );
            assert!(
                !save_text.contains(canary),
                "mandate canary `{canary}` leaked into .save.json"
            );
        }

        // Exercise precondition error paths and wipe_seal_file.
        assert!(host.write_act_recorded_seal(&ckpt_dir).is_err());
        must_ok(host.begin_cancel_lock(), "begin_cancel_lock");
        assert!(host.write_pre_act_checkpoint(&ckpt_dir).is_err());
        must_ok(EvalHost::wipe_seal_file(&seal_p), "wipe_seal_file");
        must_ok(
            EvalHost::wipe_seal_file(&seal_p),
            "wipe_seal_file idempotent",
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn eval_host_internal_guards_cover_private_fields_and_tamper_variants()
    -> Result<(), Box<dyn std::error::Error>> {
        // Keep tests inside `src/eval/host.rs` strictly scoped to `pub(crate)` / private fields so
        // `tests/eval_host.rs` exercises the public API cleanly without inflating `src/` test lines.
        let mut h = explicit_host(TableId(910_040), 7, 0x20);
        let reveals: BTreeMap<u8, OrderReveal> = (0..4_u8)
            .map(|s| {
                Ok((
                    s,
                    OrderReveal {
                        bundle: SealedOrderBundle::default(),
                        salt: h.turn_reveal_salt(Seat(s))?,
                    },
                ))
            })
            .collect::<Result<_, EngineError>>()?;
        h.commit_and_reveal_all(&reveals)?;
        let tmp = std::env::temp_dir().join(format!("ml_eval_priv_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        assert!(h.write_act_recorded_seal(&tmp.join("ckpts")).is_err());

        let (reveal_save, reveal_seal) = h.to_save_and_seal_bytes()?;
        assert!(EvalHost::from_save_and_seal_bytes(b"{not-json", &reveal_seal).is_err());
        assert!(EvalHost::from_save_and_seal_bytes(&reveal_save, b"{not-json").is_err());

        let base_seal_val: serde_json::Value = serde_json::from_slice(&reveal_seal)?;
        type SealMutation = (&'static str, fn(&mut serde_json::Value));
        let seal_mutations: [SealMutation; 5] = [
            ("INVALID_SEAL_GAME_ID", |v| {
                v["game_id"] = serde_json::json!("not-hex")
            }),
            ("INVALID_ACT_RECORDED_CHECKPOINT_SHA256", |v| {
                v["pre_act_checkpoint_sha256"] = serde_json::json!("short")
            }),
            ("INVALID_SEAL_DRAND_SCHEDULE", |v| {
                v["engine"]["state"]["spec"]["drand_schedule"] = serde_json::json!({
                    "turn_1_close_unix": 1700000000_u64,
                    "turn_period_secs": 30_u64
                });
            }),
            ("MISSING_SEAL_EVAL_VRF_PUBKEY", |v| {
                v["engine"]["state"]["spec"]["eval_vrf_pubkey"] = serde_json::Value::Null;
            }),
            ("INVALID_SEAL_TURN_SALTS", |v| {
                v["turn_salts"][0][0] = serde_json::json!("00".repeat(32));
            }),
        ];
        for (expected_code, mutator) in seal_mutations {
            let mut mutated = base_seal_val.clone();
            mutator(&mut mutated);
            let bytes = serde_json::to_vec(&mutated)?;
            let err = EvalHost::from_save_and_seal_bytes(&reveal_save, &bytes)
                .err()
                .ok_or(expected_code)?;
            assert!(err.to_string().contains(expected_code));
        }

        let mut h_dirty_subs = explicit_host(TableId(910_044), 7, 0x20);
        h_dirty_subs
            .recorded_act_submissions
            .insert(0, EvalActSubmission::default());
        assert!(
            h_dirty_subs
                .write_pre_act_checkpoint(&tmp.join("dirty_ckpts"))
                .is_err()
        );
        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }
}
