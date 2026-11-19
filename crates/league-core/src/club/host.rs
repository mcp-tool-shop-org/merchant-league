//! Exclusive Club Mode host wrapper owning [`TurnEngine`] and `.save.json` / `.seal` I/O.
//!
//! Why `ClubHost` lives in `league_core::club::host`:
//! 1. Both headless `ClubBot` season runners (`league-core` / `league-gen` tests) and the
//!    interactive `league-tui` binary share one canonical host boundary without inverting crate
//!    dependencies or pulling terminal UI crates into pure-rules crates.
//! 2. Keeping `engine: TurnEngine` strictly private inside `ClubHost` lets `league-tui` never name
//!    `TurnEngine`, `EngineState`, `SweepReceipt`, or `engine_internals` anywhere in its crate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::Path;

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
    LoggedTurn, SeasonReplayLog, check_deal_seed_independence, check_eval_vrf_independence,
    replay_season_with_state_predicates,
};
use crate::scoring::{FinalScoreboard, FundamentalParities};
use crate::types::{
    ActiveShock, AssetId, EngineError, HARD_TURN_CAP, RegentDoctrine, Seat, TableId, TableMode,
    TableSpec, Turn, UNIT_SCALE, Units, validate_club_dev_seed,
};

/// Current schema version for public `.save.json` snapshots written by [`ClubHost`].
pub const CLUB_SAVE_FORMAT_VERSION: u32 = 1;

/// Current schema version for private `.seal` sidecar payloads written by [`ClubHost`].
///
/// Bumped to `2` in Fix 1 (`C3`) now that `live_state_digests` is required rather than `#[serde(default)]`.
pub const CLUB_SEAL_FORMAT_VERSION: u32 = 2;

/// Default Standing Regent fallback initialized on `Human` Club seats (`docs/club-mode-design.md` §1.2).
pub const DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE: RegentDoctrine = RegentDoctrine {
    risk_tolerance_bps: 5_000,
    min_raw_reserve: UNIT_SCALE,
    auto_cash_due_claims: true,
};

/// Public, spectator-safe Club Mode save projection matching [`SeatObservation`] without any
/// hidden secrets, `TurnEngine`, `EngineState`, unannounced `drawn_shocks`, VRF proofs, or
/// duplicate receipt histories (`docs/club-mode-design.md` §1.6 `B4`, `C2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClubSaveSnapshot {
    /// Save projection format version (`CLUB_SAVE_FORMAT_VERSION`).
    pub format_version: u32,
    /// Clear `SeedPool::Dev` layout seed (`0..=9_999`).
    pub layout_seed: u64,
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

/// Sub-pass and Privacy Curtain resume cursor persisted solely inside `.seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClubPauseCursor {
    /// `true` when paused inside `SubPassB`; `false` when paused inside `SubPassA`.
    pub sub_pass_b: bool,
    /// Active seat offset (`0..seat_count`) inside the rotating pass order.
    pub pass_offset: usize,
    /// Next human seat behind the Privacy Curtain if a Curtain hand-off screen is open.
    pub curtain_next_seat: Option<u8>,
}

/// Private hot-seat session checkpoint stored exclusively inside the `.seal` sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClubSessionCheckpoint {
    /// Saved mid-turn pause cursor (`None` when outside an interactive TUI pause).
    pub pause_cursor: Option<ClubPauseCursor>,
    /// Per-seat uncommitted Phase-2 bundles queued prior to commit/reveal.
    pub pending_bundles: Vec<SealedOrderBundle>,
    /// Per-seat `SubPassB` explicit commit flags (distinguishes locked vs timed-out idle seats).
    pub sub_pass_b_committed: Vec<bool>,
    /// Per-seat private scratch notes, held strictly out of `.save.json`.
    pub seat_notes: Vec<String>,
    /// Optional per-seat 4-digit table-local Curtain PINs, held strictly out of `.save.json`.
    pub seat_pins: Vec<Option<String>>,
    /// Whether optional table-local 4-digit Curtain PINs are enabled for this table.
    pub pins_enabled: bool,
}

impl ClubSessionCheckpoint {
    /// Returns the `.seal`-derived Phase-2 commit salt for `seat` on the host's active turn.
    pub fn turn_reveal_salt(host: &ClubHost, seat: Seat) -> Result<RevealSalt, EngineError> {
        host.current_turn_salts
            .get(usize::from(seat.0))
            .cloned()
            .ok_or(EngineError::InvalidSeat(seat.0))
    }

    /// Validates and stages `checkpoint` on `host` so subsequent `write_save_and_seal` writes it
    /// into `.seal` (and never `.save.json`).
    pub fn store_on_host(host: &mut ClubHost, checkpoint: Self) -> Result<(), EngineError> {
        let n = usize::from(host.seat_count());
        validate_checkpoint_shapes(n, &checkpoint)?;
        host.session_pause_cursor = checkpoint.pause_cursor;
        host.session_pending_bundles = normalize_vec_len(checkpoint.pending_bundles, n);
        host.session_sub_pass_b_committed = normalize_vec_len(checkpoint.sub_pass_b_committed, n);
        host.session_seat_notes = normalize_vec_len(checkpoint.seat_notes, n);
        host.session_seat_pins = normalize_vec_len(checkpoint.seat_pins, n);
        host.session_pins_enabled = checkpoint.pins_enabled;
        Ok(())
    }

    /// Reads the persisted checkpoint state from a freshly loaded [`ClubHost`].
    #[must_use]
    pub fn load_from_host(host: &ClubHost) -> Self {
        Self {
            pause_cursor: host.session_pause_cursor.clone(),
            pending_bundles: host.session_pending_bundles.clone(),
            sub_pass_b_committed: host.session_sub_pass_b_committed.clone(),
            seat_notes: host.session_seat_notes.clone(),
            seat_pins: host.session_seat_pins.clone(),
            pins_enabled: host.session_pins_enabled,
        }
    }
}

fn validate_pin_format(pin: &str) -> bool {
    pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())
}

fn normalize_vec_len<T: Default>(mut v: Vec<T>, n: usize) -> Vec<T> {
    if v.is_empty() {
        v.resize_with(n, T::default);
    }
    v
}

fn validate_checkpoint_shapes(
    seat_count: usize,
    checkpoint: &ClubSessionCheckpoint,
) -> Result<(), EngineError> {
    if let Some(cursor) = &checkpoint.pause_cursor {
        if cursor.pass_offset > seat_count {
            return Err(EngineError::InvalidConfig(format!(
                "Club checkpoint pass_offset {} exceeds seat_count {seat_count}",
                cursor.pass_offset
            )));
        }
        if let Some(next_seat) = cursor.curtain_next_seat
            && usize::from(next_seat) >= seat_count
        {
            return Err(EngineError::InvalidSeat(next_seat));
        }
    }
    for (name, len) in [
        ("pending_bundles", checkpoint.pending_bundles.len()),
        ("sub_pass_b_committed", checkpoint.sub_pass_b_committed.len()),
        ("seat_notes", checkpoint.seat_notes.len()),
        ("seat_pins", checkpoint.seat_pins.len()),
    ] {
        if len != 0 && len != seat_count {
            return Err(EngineError::InvalidConfig(format!(
                "Club checkpoint {name} length {len} does not match seat_count {seat_count}"
            )));
        }
    }
    for pin_opt in &checkpoint.seat_pins {
        if let Some(pin) = pin_opt
            && !validate_pin_format(pin)
        {
            return Err(EngineError::InvalidConfig(
                "Club checkpoint PIN must be exactly 4 ASCII digits".to_owned(),
            ));
        }
    }
    Ok(())
}

fn derive_turn_commit_salts(
    entropy: &RevealSalt,
    table_id: TableId,
    turn: u8,
    seat_count: u8,
) -> Vec<RevealSalt> {
    let mut salts = Vec::with_capacity(usize::from(seat_count));
    for s in 0..seat_count {
        let mut hasher = Sha256::new();
        hasher.update(b"league-club-order-salt-v1");
        hasher.update(entropy.0);
        hasher.update(table_id.0.to_le_bytes());
        hasher.update([turn, s]);
        let mut bytes: [u8; 32] = hasher.finalize().into();
        if bytes == [0u8; 32] {
            bytes[0] = 1;
        }
        salts.push(RevealSalt(bytes));
    }
    salts
}

#[derive(Serialize, Deserialize)]
struct ClubSealPayload {
    format_version: u32,
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
    #[serde(default)]
    commit_salt_entropy: Option<RevealSalt>,
    #[serde(default)]
    current_turn_salts: Vec<RevealSalt>,
    #[serde(default)]
    session_pause_cursor: Option<ClubPauseCursor>,
    #[serde(default)]
    session_pending_bundles: Vec<SealedOrderBundle>,
    #[serde(default)]
    session_sub_pass_b_committed: Vec<bool>,
    #[serde(default)]
    session_seat_notes: Vec<String>,
    #[serde(default)]
    session_seat_pins: Vec<Option<String>>,
    #[serde(default)]
    session_pins_enabled: bool,
}

/// Exclusive Club Mode host wrapper owning [`TurnEngine`], pre-derived VRF proofs, sealed Turn-1
/// mandates, replay turn capture, and `.save.json` / `.seal` persistence.
///
/// No Club Mode seat or UI module touches `TurnEngine`, `EngineState`, `SweepReceipt`, or
/// `SealedEvalSeed` directly: every seat reads only [`SeatObservation`] via [`Self::observe_seat`]
/// and [`PublicSweepReceipt`] via [`Self::sweep_turn`] / [`Self::public_receipts`].
pub struct ClubHost {
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
    commit_salt_entropy: RevealSalt,
    current_turn_salts: Vec<RevealSalt>,
    session_pause_cursor: Option<ClubPauseCursor>,
    session_pending_bundles: Vec<SealedOrderBundle>,
    session_sub_pass_b_committed: Vec<bool>,
    session_seat_notes: Vec<String>,
    session_seat_pins: Vec<Option<String>>,
    session_pins_enabled: bool,
}

impl fmt::Debug for ClubHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let st = self.engine.state();
        f.debug_struct("ClubHost")
            .field("table_id", &st.table_id)
            .field("layout_seed", &self.layout_seed)
            .field("turn", &st.turn)
            .field("phase", &st.phase)
            .field("seat_count", &st.spec.seat_count)
            .field("logged_turns_len", &self.logged_turns.len())
            .field("vrf_proofs", &"<hidden>")
            .field("sealed_mandate_deal", &"<hidden>")
            .finish()
    }
}

impl ClubHost {
    /// Initializes a [`ClubHost`] over a validated `TableMode::Club` [`TableSpec`], pre-deriving
    /// all `32` `EvalVrf` proofs (`1..=HARD_TURN_CAP`), dropping `eval_seed` in memory (`B4`), and
    /// recording Turn-1 Private Mandate commitments.
    pub fn new(
        spec: TableSpec,
        layout_seed: u64,
        sealed_mandate_deal: SealedMandateDeal,
        eval_seed: SealedEvalSeed,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        if spec.mode != TableMode::Club {
            return Err(EngineError::InvalidConfig(format!(
                "ClubHost requires TableMode::Club, got {:?}",
                spec.mode
            )));
        }
        validate_club_dev_seed(layout_seed)?;
        spec.validate()?;

        let eval_pk = spec.eval_vrf_pubkey.ok_or_else(|| {
            EngineError::InvalidConfig("ClubHost requires TableSpec::eval_vrf_pubkey".to_owned())
        })?;
        let actual_pk = eval_seed.public_key_bytes()?;
        if actual_pk != eval_pk {
            return Err(EngineError::InvalidConfig(
                "SealedEvalSeed public key does not match TableSpec::eval_vrf_pubkey".to_owned(),
            ));
        }

        let candidates = check_deal_seed_independence(layout_seed, &sealed_mandate_deal, false)?;
        check_eval_vrf_independence(candidates, eval_pk, false)?;

        if !verify_sealed_mandate_deal(
            spec.table_id,
            spec.seat_count,
            &spec.mandate_deck,
            &spec.dealt_mandate_commits,
            &sealed_mandate_deal,
        )? {
            return Err(EngineError::InvalidConfig(
                "sealed_mandate_deal does not match TableSpec::dealt_mandate_commits and deck"
                    .to_owned(),
            ));
        }

        let vrf_proofs = derive_all_vrf_proofs(eval_seed, spec.table_id)?;

        let n = usize::from(spec.seat_count);
        let seat_count = spec.seat_count;
        let mut engine = TurnEngine::new(spec)?;
        let table_id = engine.state().table_id;
        let initial_turn = engine.state().turn;
        let commit_salt_entropy = RevealSalt(draw_nonzero_256_bits()?);
        let current_turn_salts =
            derive_turn_commit_salts(&commit_salt_entropy, table_id, initial_turn, seat_count);
        let mut mandate_commits = Vec::with_capacity(n);
        for s in 0..n as u8 {
            let rev = &sealed_mandate_deal.seat_reveals[usize::from(s)];
            let mc = compute_mandate_commitment(table_id, Seat(s), rev)?;
            engine.submit_mandate_commitment(Seat(s), mc)?;
            mandate_commits.push(Some(mc));
        }

        Ok(Self {
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
            commit_salt_entropy,
            current_turn_salts,
            session_pause_cursor: None,
            session_pending_bundles: vec![SealedOrderBundle::default(); n],
            session_sub_pass_b_committed: vec![false; n],
            session_seat_notes: vec![String::new(); n],
            session_seat_pins: vec![None; n],
            session_pins_enabled: false,
        })
    }

    /// Bootstraps a [`ClubHost`] using [`TableSpec::standard_club_chiral_ring_with_setup`].
    pub fn from_standard_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let (spec, deal, eval_seed) =
            TableSpec::standard_club_chiral_ring_with_setup(table_id, seat_count, layout_seed)?;
        Self::new(spec, layout_seed, deal, eval_seed, bot_tie_seed)
    }

    /// Bootstraps a reproducible [`ClubHost`] using
    /// [`TableSpec::standard_club_chiral_ring_with_explicit_setup`].
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
        let (spec, deal, eval_seed) = TableSpec::standard_club_chiral_ring_with_explicit_setup(
            table_id,
            seat_count,
            layout_seed,
            deal_seed,
            seat_salts,
            eval_seed,
        )?;
        Self::new(spec, layout_seed, deal, eval_seed, bot_tie_seed)
    }

    /// Returns the clear `SeedPool::Dev` layout seed for this table.
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
    /// are `pub(crate)`, and `own_mandate` is removed from [`ClubHost`]:
    ///
    /// ```compile_fail,E0624
    /// use league_core::{ClubHost, TableId};
    /// let host = ClubHost::from_standard_setup(TableId(1), 4, 1, 0).unwrap();
    /// let _ = host.to_save_and_seal_bytes();
    /// ```
    ///
    /// ```compile_fail,E0624
    /// use league_core::ClubHost;
    /// let _ = ClubHost::from_save_and_seal_bytes(&[], &[]);
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use league_core::{ClubHost, Seat, TableId};
    /// let host = ClubHost::from_standard_setup(TableId(1), 4, 1, 0).unwrap();
    /// let _ = host.own_mandate(Seat(0));
    /// ```
    ///
    /// `SeatObservation::own_mandate` cannot be overwritten or forged outside host construction:
    ///
    /// ```compile_fail,E0616
    /// use league_core::{ClubHost, Seat, TableId};
    /// let host = ClubHost::from_standard_setup(TableId(1), 4, 1, 0).unwrap();
    /// let mut obs = host.observe_seat(Seat(0)).unwrap();
    /// obs.own_mandate = None;
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use league_core::{ClubHost, Seat, TableId};
    /// let host = ClubHost::from_standard_setup(TableId(1), 4, 1, 0).unwrap();
    /// let obs = host.observe_seat(Seat(0)).unwrap();
    /// let m = obs.own_mandate().copied().unwrap();
    /// let _ = obs.with_own_mandate(m);
    /// ```
    ///
    /// `SeatObservation` intentionally omits `Deserialize` (`B1`) so neither save files,
    /// replay logs, nor host wrappers can forge `own_mandate` via a serde round-trip:
    ///
    /// ```compile_fail,E0277
    /// use league_core::SeatObservation;
    /// let _: Result<SeatObservation, _> = serde_json::from_str("{}");
    /// ```
    ///
    /// `TurnEngine::observe_seat_with_mandate` is `pub(crate)` and unreachable outside `league-core`:
    ///
    /// ```compile_fail,E0624
    /// use league_core::{
    ///     CreditTier, MandateKind, PrivateMandate, Seat, TableId, TableSpec, TurnEngine,
    /// };
    /// let spec = TableSpec::standard_chiral_ring(TableId(1), 4, 1).unwrap();
    /// let engine = TurnEngine::new(spec).unwrap();
    /// let mandate = PrivateMandate {
    ///     kind: MandateKind::MaintainCleanCredit {
    ///         min_tier: CreditTier::Aaa,
    ///     },
    ///     bonus_equity: 3_000,
    /// };
    /// let _ = engine.observe_seat_with_mandate(Seat(0), mandate);
    /// ```
    pub fn observe_seat(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        let mandate = *self.mandate_for_seat(seat)?;
        self.engine.observe_seat_with_mandate(seat, mandate)
    }

    /// Host-only lookup returning `seat`'s dealt [`PrivateMandate`] (without its reveal salt).
    ///
    /// This method returns **any** seat's mandate and is forbidden in seat-side modules and UI
    /// screens (`raw_api_backdoor_guard`). Seats must read only their own mandate embedded in
    /// [`SeatObservation::own_mandate`] from [`Self::observe_seat`].
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

    /// Runs the 8-step live `EvalVrf` Turn Sweep using the pre-derived VRF proof for the current
    /// turn, logs the completed [`LoggedTurn`] privately, and returns only [`PublicSweepReceipt`].
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
            EngineError::InvalidConfig("ClubHost requires TableSpec::eval_vrf_pubkey".to_owned())
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
        if self.phase() == TurnPhase::TalkAndMarket {
            self.current_turn_salts = derive_turn_commit_salts(
                &self.commit_salt_entropy,
                self.table_id(),
                self.turn(),
                self.seat_count(),
            );
        }
        Ok(public_receipt)
    }

    /// Unseals all Turn-1 Private Charter Mandates after Season Close, finalizes the
    /// [`FinalScoreboard`], exports and verifies the deterministic [`SeasonReplayLog`], and returns
    /// the derived [`StatePredicateSection`] and [`SeatSeasonMetrics`].
    pub fn finalize_club_season(
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
                "finalize_club_season requires AwaitingMandateReveal".to_owned(),
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
        let (replay_out, section) = replay_season_with_state_predicates(&log)?;
        if replay_out.scoreboard != scoreboard {
            return Err(EngineError::SeasonEnded(
                "replayed scoreboard diverged from live ClubHost scoreboard".to_owned(),
            ));
        }
        if replay_out.state_digests.len() != self.live_state_digests.len() {
            return Err(EngineError::SeasonEnded(format!(
                "replayed state digest count {} diverged from live ClubHost turn count {}",
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
                    "replayed post_state_digest diverged from live ClubHost digest at turn {}",
                    idx + 1
                )));
            }
        }
        let metrics = compute_season_metrics(&section);
        Ok((scoreboard, log, section, metrics))
    }

    /// Exports the portable [`SeasonReplayLog`] once the season has reached [`TurnPhase::Completed`].
    pub fn export_replay_log(&self) -> Result<SeasonReplayLog, EngineError> {
        if self.phase() != TurnPhase::Completed {
            return Err(EngineError::InvalidPhase(
                "export_replay_log requires Completed phase".to_owned(),
            ));
        }
        SeasonReplayLog::from_rig_seed(
            self.engine.state().spec.clone(),
            self.layout_seed,
            self.sealed_mandate_deal.clone(),
            self.mandate_commits.clone(),
            self.logged_turns.clone(),
            self.mandate_reveals.clone(),
        )
    }

    /// Builds the public, spectator-safe [`ClubSaveSnapshot`] matching [`SeatObservation`].
    pub fn public_save_snapshot(&self) -> Result<ClubSaveSnapshot, EngineError> {
        let state = self.engine.state();
        let (last_swept_turn, _, public_forecasts) = compute_public_forecasts(state);
        let public_receipts = self
            .engine
            .public_receipts()
            .iter()
            .filter(|r| r.turn <= last_swept_turn)
            .cloned()
            .collect();
        Ok(ClubSaveSnapshot {
            format_version: CLUB_SAVE_FORMAT_VERSION,
            layout_seed: self.layout_seed,
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
            EngineError::InvalidConfig(format!("failed to serialize ClubSaveSnapshot: {err}"))
        })?;
        let seal = ClubSealPayload {
            format_version: CLUB_SEAL_FORMAT_VERSION,
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
            commit_salt_entropy: Some(self.commit_salt_entropy.clone()),
            current_turn_salts: self.current_turn_salts.clone(),
            session_pause_cursor: self.session_pause_cursor.clone(),
            session_pending_bundles: self.session_pending_bundles.clone(),
            session_sub_pass_b_committed: self.session_sub_pass_b_committed.clone(),
            session_seat_notes: self.session_seat_notes.clone(),
            session_seat_pins: self.session_seat_pins.clone(),
            session_pins_enabled: self.session_pins_enabled,
        };
        let seal_bytes = serde_json::to_vec(&seal).map_err(|err| {
            EngineError::InvalidConfig(format!("failed to serialize Club seal sidecar: {err}"))
        })?;
        Ok((save_bytes, seal_bytes))
    }

    /// Reconstructs and validates a [`ClubHost`] from `.save.json` and `.seal` sidecar bytes
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
            && hdr.format_version != CLUB_SAVE_FORMAT_VERSION
        {
            return Err(EngineError::UnsupportedReplayVersion {
                found: hdr.format_version,
                newest_known: CLUB_SAVE_FORMAT_VERSION,
            });
        }
        let save: ClubSaveSnapshot = serde_json::from_slice(save_bytes).map_err(|err| {
            EngineError::InvalidConfig(format!("Club save JSON does not parse: {err}"))
        })?;
        if save.format_version != CLUB_SAVE_FORMAT_VERSION {
            return Err(EngineError::UnsupportedReplayVersion {
                found: save.format_version,
                newest_known: CLUB_SAVE_FORMAT_VERSION,
            });
        }
        if let Ok(hdr) = serde_json::from_slice::<FormatVersionHeader>(seal_bytes)
            && hdr.format_version != CLUB_SEAL_FORMAT_VERSION
        {
            return Err(EngineError::UnsupportedReplayVersion {
                found: hdr.format_version,
                newest_known: CLUB_SEAL_FORMAT_VERSION,
            });
        }
        let seal: ClubSealPayload = serde_json::from_slice(seal_bytes).map_err(|err| {
            EngineError::InvalidConfig(format!("Club seal JSON does not parse: {err}"))
        })?;
        if seal.format_version != CLUB_SEAL_FORMAT_VERSION {
            return Err(EngineError::UnsupportedReplayVersion {
                found: seal.format_version,
                newest_known: CLUB_SEAL_FORMAT_VERSION,
            });
        }
        validate_club_dev_seed(seal.layout_seed)?;
        if seal.layout_seed != save.layout_seed {
            return Err(EngineError::InvalidConfig(
                "Club save layout_seed does not match seal layout_seed".to_owned(),
            ));
        }

        let spec = &seal.engine.state().spec;
        if spec.mode != TableMode::Club {
            return Err(EngineError::InvalidConfig(format!(
                "ClubHost seal requires TableMode::Club, got {:?}",
                spec.mode
            )));
        }
        spec.validate()?;
        let eval_pk = spec.eval_vrf_pubkey.ok_or_else(|| {
            EngineError::InvalidConfig(
                "ClubHost seal requires TableSpec::eval_vrf_pubkey".to_owned(),
            )
        })?;
        if seal.vrf_proofs.len() != usize::from(HARD_TURN_CAP) {
            return Err(EngineError::InvalidConfig(format!(
                "ClubHost seal expected {HARD_TURN_CAP} VRF proofs, got {}",
                seal.vrf_proofs.len()
            )));
        }
        let table_id = seal.engine.state().table_id;
        for (idx, proof) in seal.vrf_proofs.iter().enumerate() {
            if proof.len() != EVAL_VRF_PROOF_BYTES {
                return Err(EngineError::InvalidConfig(format!(
                    "ClubHost seal proof {} has length {} (expected {EVAL_VRF_PROOF_BYTES})",
                    idx + 1,
                    proof.len()
                )));
            }
            verify_and_extract_eval_vrf_beacon(eval_pk, table_id, Turn((idx + 1) as u8), proof)?;
        }

        let candidates =
            check_deal_seed_independence(seal.layout_seed, &seal.sealed_mandate_deal, false)?;
        check_eval_vrf_independence(candidates, eval_pk, false)?;
        if !verify_sealed_mandate_deal(
            table_id,
            spec.seat_count,
            &spec.mandate_deck,
            &spec.dealt_mandate_commits,
            &seal.sealed_mandate_deal,
        )? {
            return Err(EngineError::InvalidConfig(
                "ClubHost seal sealed_mandate_deal failed verification".to_owned(),
            ));
        }
        let n = usize::from(spec.seat_count);
        if seal.mandate_commits.len() != n || seal.mandate_reveals.len() != n {
            return Err(EngineError::InvalidConfig(
                "ClubHost seal mandate commit/reveal lengths do not match seat_count".to_owned(),
            ));
        }
        if seal.live_state_digests.len() != seal.logged_turns.len() {
            return Err(EngineError::InvalidConfig(format!(
                "ClubHost seal live_state_digests len {} does not match logged_turns len {}",
                seal.live_state_digests.len(),
                seal.logged_turns.len()
            )));
        }

        let commit_salt_entropy = match seal.commit_salt_entropy {
            Some(entropy) => {
                if entropy.0 == [0u8; 32] {
                    return Err(EngineError::InvalidConfig(
                        "ClubHost seal commit_salt_entropy must be non-zero".to_owned(),
                    ));
                }
                entropy
            }
            None => RevealSalt(draw_nonzero_256_bits()?),
        };
        let current_turn_salts = if seal.current_turn_salts.is_empty() {
            derive_turn_commit_salts(
                &commit_salt_entropy,
                table_id,
                seal.engine.state().turn,
                spec.seat_count,
            )
        } else {
            if seal.current_turn_salts.len() != n
                || seal.current_turn_salts.iter().any(|salt| salt.0 == [0u8; 32])
            {
                return Err(EngineError::InvalidConfig(
                    "ClubHost seal current_turn_salts invalid or wrong length".to_owned(),
                ));
            }
            seal.current_turn_salts
        };
        let ckpt = ClubSessionCheckpoint {
            pause_cursor: seal.session_pause_cursor,
            pending_bundles: seal.session_pending_bundles,
            sub_pass_b_committed: seal.session_sub_pass_b_committed,
            seat_notes: seal.session_seat_notes,
            seat_pins: seal.session_seat_pins,
            pins_enabled: seal.session_pins_enabled,
        };
        validate_checkpoint_shapes(n, &ckpt)?;

        let host = Self {
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
            commit_salt_entropy,
            current_turn_salts,
            session_pause_cursor: ckpt.pause_cursor,
            session_pending_bundles: normalize_vec_len(ckpt.pending_bundles, n),
            session_sub_pass_b_committed: normalize_vec_len(ckpt.sub_pass_b_committed, n),
            session_seat_notes: normalize_vec_len(ckpt.seat_notes, n),
            session_seat_pins: normalize_vec_len(ckpt.seat_pins, n),
            session_pins_enabled: ckpt.pins_enabled,
        };
        if host.public_save_snapshot()? != save {
            return Err(EngineError::InvalidConfig(
                "Club save snapshot does not match verified seal state".to_owned(),
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

    /// Reads and validates a saved Club Mode game from `.save.json` and `.seal`.
    pub fn read_save_and_seal(save_path: &Path, seal_path: &Path) -> Result<Self, EngineError> {
        let save_bytes = fs::read(save_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to read Club save {}: {err}",
                save_path.display()
            ))
        })?;
        let seal_bytes = fs::read(seal_path).map_err(|err| {
            EngineError::InvalidConfig(format!(
                "failed to read Club seal {}: {err}",
                seal_path.display()
            ))
        })?;
        Self::from_save_and_seal_bytes(&save_bytes, &seal_bytes)
    }

    /// Removes the private `.seal` sidecar file once a season finishes and its replay log is
    /// exported (`docs/club-mode-design.md` §1.6). Succeeds if the file is already absent.
    pub fn wipe_seal_file(seal_path: &Path) -> Result<(), EngineError> {
        match fs::remove_file(seal_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EngineError::InvalidConfig(format!(
                "failed to remove Club seal {}: {err}",
                seal_path.display()
            ))),
        }
    }
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

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), EngineError> {
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
        .unwrap_or("club");
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

    fn must_ok_host<T, E: fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
        match res {
            Ok(v) => v,
            Err(err) => panic!("{ctx}: {err}"),
        }
    }

    #[test]
    fn planted_b3_perturbed_live_state_digest_makes_finalize_club_season_refuse() {
        let eval_seed = must_ok_host(
            SealedEvalSeed::try_from_bytes([0x6F; 32]),
            "SealedEvalSeed::try_from_bytes",
        );
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x41 + i; 32])).collect();
        let mut host = must_ok_host(
            ClubHost::from_explicit_setup(
                TableId(905_001),
                4,
                7,
                [0x4B; 32],
                &salts,
                eval_seed,
                77,
            ),
            "ClubHost::from_explicit_setup",
        );
        for _ in 1..=HARD_TURN_CAP {
            if host.phase() != TurnPhase::TalkAndMarket {
                break;
            }
            must_ok_host(
                host.commit_and_reveal_all(&BTreeMap::new()),
                "commit_and_reveal_all",
            );
            let _ = must_ok_host(host.sweep_turn(), "sweep_turn");
        }
        assert_eq!(host.phase(), TurnPhase::AwaitingMandateReveal);
        assert_eq!(host.live_state_digests.len(), host.logged_turns.len());

        let (save_bytes, seal_bytes) = must_ok_host(host.to_save_and_seal_bytes(), "save bytes");
        let mut host_clean = must_ok_host(
            ClubHost::from_save_and_seal_bytes(&save_bytes, &seal_bytes),
            "clean clone via bytes",
        );
        let _ = must_ok_host(host_clean.finalize_club_season(), "clean finalize passes");

        let mut host_perturbed = must_ok_host(
            ClubHost::from_save_and_seal_bytes(&save_bytes, &seal_bytes),
            "perturbed clone via bytes",
        );
        host_perturbed.live_state_digests[2][0] ^= 0x01;
        let err = match host_perturbed.finalize_club_season() {
            Ok(_) => panic!("perturbed live post_state_digest must fail finalize_club_season"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(
            matches!(err, EngineError::SeasonEnded(_)) && msg.contains("turn 3"),
            "unexpected digest mismatch error: {err}"
        );
        println!("PLANTED_B3_DIGEST_PERTURBATION_RESULT: {err}");
    }

    #[test]
    fn to_and_from_save_and_seal_bytes_reject_live_state_digests_count_mismatch() {
        let eval_seed = must_ok_host(
            SealedEvalSeed::try_from_bytes([0x6E; 32]),
            "SealedEvalSeed::try_from_bytes",
        );
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x51 + i; 32])).collect();
        let mut host = must_ok_host(
            ClubHost::from_explicit_setup(
                TableId(905_002),
                4,
                7,
                [0x4C; 32],
                &salts,
                eval_seed,
                88,
            ),
            "ClubHost::from_explicit_setup",
        );
        must_ok_host(
            host.commit_and_reveal_all(&BTreeMap::new()),
            "commit_and_reveal_all",
        );
        let _ = must_ok_host(host.sweep_turn(), "sweep_turn");
        let (save_bytes, seal_bytes) = must_ok_host(host.to_save_and_seal_bytes(), "save bytes");
        let mut seal_json: serde_json::Value =
            must_ok_host(serde_json::from_slice(&seal_bytes), "parse seal");
        seal_json["live_state_digests"] = serde_json::json!([]);
        let bad_seal = must_ok_host(serde_json::to_vec(&seal_json), "serialize bad seal");
        let err = match ClubHost::from_save_and_seal_bytes(&save_bytes, &bad_seal) {
            Ok(_) => panic!("mismatched live_state_digests len must be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("live_state_digests"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn club_host_new_validation_accessors_debug_and_premature_phase_errors() {
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x61 + i; 32])).collect();
        let tid = TableId(905_101);
        let host = must_ok_host(
            ClubHost::from_explicit_setup(
                tid,
                4,
                7,
                [0x4D; 32],
                &salts,
                must_ok_host(SealedEvalSeed::try_from_bytes([0x7C; 32]), "eval"),
                88,
            ),
            "host setup",
        );

        assert_eq!(host.layout_seed(), 7);
        assert_eq!(host.bot_tie_seed(), 88);
        assert_eq!(host.table_id(), tid);
        assert!(!host.cancel_locked());
        assert_eq!(host.public_spec().table_id, tid);
        assert!(host.public_receipts().is_empty());
        assert!(format!("{host:?}").contains("<hidden>"));
        assert!(host.mandate_for_seat(Seat(99)).is_err());
        assert!(host.observe_seat(Seat(99)).is_err());
        assert!(host.preview_regent_bundle_for_seat(Seat(0)).is_ok());

        let mut host_mut = host;
        assert!(matches!(
            host_mut.finalize_club_season(),
            Err(EngineError::InvalidPhase(_))
        ));
        assert!(matches!(
            host_mut.export_replay_log(),
            Err(EngineError::InvalidPhase(_))
        ));

        let (base_spec, base_deal, base_eval) = must_ok_host(
            TableSpec::standard_club_chiral_ring_with_explicit_setup(
                TableId(905_102),
                4,
                7,
                [0x4E; 32],
                &salts,
                must_ok_host(SealedEvalSeed::try_from_bytes([0x7D; 32]), "eval2"),
            ),
            "base spec",
        );

        let mut wrong_mode = base_spec.clone();
        wrong_mode.mode = TableMode::Eval;
        assert!(
            ClubHost::new(
                wrong_mode,
                7,
                base_deal.clone(),
                must_ok_host(SealedEvalSeed::try_from_bytes([0x7D; 32]), "eval"),
                1,
            )
            .is_err()
        );

        assert!(
            ClubHost::new(
                base_spec.clone(),
                1_u64 << 63,
                base_deal.clone(),
                must_ok_host(SealedEvalSeed::try_from_bytes([0x7D; 32]), "eval"),
                1,
            )
            .is_err()
        );

        let mut no_pk = base_spec.clone();
        no_pk.eval_vrf_pubkey = None;
        assert!(
            ClubHost::new(
                no_pk,
                7,
                base_deal.clone(),
                must_ok_host(SealedEvalSeed::try_from_bytes([0x7D; 32]), "eval"),
                1,
            )
            .is_err()
        );

        let mismatched_eval = must_ok_host(SealedEvalSeed::try_from_bytes([0x7E; 32]), "eval3");
        assert!(
            ClubHost::new(base_spec.clone(), 7, base_deal.clone(), mismatched_eval, 1).is_err()
        );

        let mut bad_commits = base_spec;
        bad_commits.dealt_mandate_commits[0].0[0] ^= 0x01;
        assert!(ClubHost::new(bad_commits, 7, base_deal, base_eval, 1).is_err());
    }

    #[test]
    fn from_save_and_seal_bytes_rejects_corrupted_headers_versions_proofs_and_mismatches() {
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x71 + i; 32])).collect();
        let mut host = must_ok_host(
            ClubHost::from_explicit_setup(
                TableId(905_103),
                4,
                7,
                [0x5E; 32],
                &salts,
                must_ok_host(SealedEvalSeed::try_from_bytes([0x7F; 32]), "eval"),
                99,
            ),
            "host setup",
        );
        must_ok_host(
            host.commit_and_reveal_all(&BTreeMap::new()),
            "commit_and_reveal_all",
        );
        let _ = must_ok_host(host.sweep_turn(), "sweep_turn");
        let (save_bytes, seal_bytes) = must_ok_host(host.to_save_and_seal_bytes(), "save_seal");

        let save_val: serde_json::Value =
            must_ok_host(serde_json::from_slice(&save_bytes), "parse save");
        let seal_val: serde_json::Value =
            must_ok_host(serde_json::from_slice(&seal_bytes), "parse seal");

        let mut bad_save_ver = save_val.clone();
        bad_save_ver["format_version"] = serde_json::json!(99);
        assert!(matches!(
            ClubHost::from_save_and_seal_bytes(
                &must_ok_host(serde_json::to_vec(&bad_save_ver), "s"),
                &seal_bytes,
            ),
            Err(EngineError::UnsupportedReplayVersion { found: 99, .. })
        ));
        assert!(ClubHost::from_save_and_seal_bytes(b"{not-json", &seal_bytes).is_err());

        let mut bad_seal_ver = seal_val.clone();
        bad_seal_ver["format_version"] = serde_json::json!(99);
        assert!(matches!(
            ClubHost::from_save_and_seal_bytes(
                &save_bytes,
                &must_ok_host(serde_json::to_vec(&bad_seal_ver), "s"),
            ),
            Err(EngineError::UnsupportedReplayVersion { found: 99, .. })
        ));
        assert!(ClubHost::from_save_and_seal_bytes(&save_bytes, b"{not-json").is_err());

        let mut mismatch_seed = seal_val.clone();
        mismatch_seed["layout_seed"] = serde_json::json!(9);
        assert!(
            ClubHost::from_save_and_seal_bytes(
                &save_bytes,
                &must_ok_host(serde_json::to_vec(&mismatch_seed), "s"),
            )
            .is_err()
        );

        let mut short_proofs = seal_val.clone();
        if let Some(arr) = short_proofs["vrf_proofs"].as_array_mut() {
            arr.pop();
        }
        assert!(
            ClubHost::from_save_and_seal_bytes(
                &save_bytes,
                &must_ok_host(serde_json::to_vec(&short_proofs), "s"),
            )
            .is_err()
        );

        let mut wrong_len_proof = seal_val.clone();
        wrong_len_proof["vrf_proofs"][0] = serde_json::json!(vec![0_u8; 4]);
        assert!(
            ClubHost::from_save_and_seal_bytes(
                &save_bytes,
                &must_ok_host(serde_json::to_vec(&wrong_len_proof), "s"),
            )
            .is_err()
        );

        let mut bad_proof_byte = seal_val.clone();
        if let Some(first) = bad_proof_byte["vrf_proofs"][0]
            .as_array_mut()
            .and_then(|a| a.first_mut())
            && let Some(b) = first.as_u64()
        {
            *first = serde_json::json!(b ^ 1);
        }
        assert!(
            ClubHost::from_save_and_seal_bytes(
                &save_bytes,
                &must_ok_host(serde_json::to_vec(&bad_proof_byte), "s"),
            )
            .is_err()
        );

        let mut bad_commits_len = seal_val;
        if let Some(arr) = bad_commits_len["mandate_commits"].as_array_mut() {
            arr.pop();
        }
        assert!(
            ClubHost::from_save_and_seal_bytes(
                &save_bytes,
                &must_ok_host(serde_json::to_vec(&bad_commits_len), "s"),
            )
            .is_err()
        );

        let mut mismatched_turn_save = save_val;
        mismatched_turn_save["turn"] = serde_json::json!(99);
        assert!(
            ClubHost::from_save_and_seal_bytes(
                &must_ok_host(serde_json::to_vec(&mismatched_turn_save), "s"),
                &seal_bytes,
            )
            .is_err()
        );
    }
}
