//! `GameTableSteward`: bridges `TurnEngine` (`TableMode::Game`) onto stand-alone `xrpld 3.4.1`
//! per `docs/game-mode-protocol.md` §§1–3.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use league_core::{
    AssetId, CommitmentHash, CreditTier, DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_PERIOD_SECS,
    DrandBeaconAuthStatus, DrandSchedule, EngineError, FinalScoreboard, LoggedTurn, MandateReveal,
    MarketAction, OrderReveal, PublicSweepReceipt, ReplayOutcome, SealedMandateDeal,
    SeasonReplayLog, Seat, SeatIdentity, TableMode, TableSpec, Turn, TurnEngine, TurnPhase, Units,
    compute_mandate_commitment, derive_drand_beacon, eval_vrf_alpha, replay_season,
};
use league_ledger::client::Transport;
use league_season::{WipeProbeInput, WipeSignal, detect_ledger_wipe, ledger_went_backwards};
use league_standalone::client::{StandAlone, StandAloneAccount, StandAloneError};
use league_standalone::conformance::PREIMAGE_32_FINISH_FEE_DROPS;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::amount::{asset_currency_code, format_milli_units, parse_iou_milli_exact};
use crate::archivist::{
    ArchivistEvent, ArchivistMirror, MaterializedArchivistState, MirroredCharter,
    MirroredCommitEscrow, MirroredNativeCheck, MirroredNativeEscrow,
};
use crate::codec::{
    REGENT_DELEGATE_PERMISSIONS, TicketPool, clawback_iou_json, configure_steward_authority,
    delete_credential, issue_and_accept_credential, open_authorized_trust_line, payment_iou_json,
    publish_oracle_prices, set_permissioned_domain, split_batch_chunks,
    submit_batch_all_or_nothing, submit_check_cancel, submit_check_cash, submit_check_create,
    submit_delegate_set, submit_delegated_tx, submit_escrow_cancel, submit_escrow_create,
    submit_escrow_finish, submit_single_clawback, submit_single_payment,
};
use crate::error::StewardError;
use crate::firewall::{MemoFirewall, VerifiedCommitIngress, encode_commit_memos_json};
use crate::phoenix::{PhoenixRehydrationOutcome, execute_three_wave_phoenix};
use crate::reserve::{
    DEFAULT_TESTNET_FAUCET_GRANT_DROPS, ReserveParams, StewardReservePool, estimate_table_reserves,
};

const INITIAL_ACCOUNT_XRP_DROPS: u64 = 500_000_000;
const DEFAULT_TRUST_LINE_LIMIT: &str = "1000000";
const INITIAL_TICKET_COUNT: u32 = 40;
const TICKET_REPLENISH_THRESHOLD: usize = 6;
const DEFAULT_TURN_SPAN_RIPPLE_SECS: u64 = 400;

/// Validated `xrpld` ledger-time window for one active `TableMode::Game` turn (`docs/game-mode-protocol.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StewardTurnWindow {
    /// Turn number (`1..=32`).
    pub turn: u8,
    /// Validated `xrpld` Ripple-epoch `close_time` when this turn opened.
    pub open_ripple_close: u64,
    /// Validated `xrpld` Ripple-epoch `close_time` at which `cancel_lock` engages.
    pub lock_ripple_close: u64,
    /// Validated `xrpld` Ripple-epoch `close_time` at which Phase-2 commits expire.
    pub commit_close_ripple: u64,
    /// Logical Unix timestamp at which `cancel_lock` engages.
    pub lock_unix: u64,
    /// Logical Unix commit cutoff (`< T_beacon(turn)` by at least `3s`, `#51`).
    pub commit_close_unix: u64,
    /// Scheduled `drand quicknet` beacon publication timestamp `t_round(expected_round)`.
    pub beacon_unix: u64,
    /// Pre-named `drand quicknet` round for this turn.
    pub expected_drand_round: u64,
}

/// Telemetry emitted after one Phase-3 Sweep + `BatchV1_1` ledger reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnSettlementSummary {
    /// Completed turn number (`1..=32`).
    pub turn: u8,
    /// Public sweep receipt (scrubbed of private raw-engine fields).
    pub public_receipt: PublicSweepReceipt,
    /// Count of `BatchV1_1` (`tfAllOrNothing`) outer envelopes submitted during Sweep.
    pub batch_count: usize,
    /// Outer `Batch` transaction hashes verified on `xrpld`.
    pub batch_hashes: Vec<String>,
    /// Total inner legs verified (`ParentBatchID` + `tesSUCCESS`).
    pub batch_inner_legs_verified: usize,
    /// `OracleSet` transaction hashes published this turn.
    pub oracle_tx_hashes: Vec<String>,
    /// Count of Phase-1 actions rejected during this turn (including `CancelLocked`).
    pub rejected_market_actions: usize,
    /// Whether `cancel_lock` engaged on this turn via validated `close_time`.
    pub cancel_lock_engaged: bool,
    /// Validated `xrpld` ledger index after post-sweep `account_lines` parity passed.
    pub settled_ledger_index: u64,
}

/// Terminal settlement and offline replay verification proof for a completed season.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeasonSettlementReport {
    /// Table identifier.
    pub table_id: u64,
    /// Seat count (`4..=6`).
    pub seat_count: u8,
    /// Terminal turn (`24..=32`).
    pub closed_turn: u8,
    /// Verified `AccountRoot.Flags` bitmask on Authority (`RequireAuth|Clawback|Locking|DefaultRipple|DisallowXRP`).
    pub authority_flags: u32,
    /// Table `PermissionedDomain` ID on `xrpld`.
    pub table_domain_id: String,
    /// Total `BatchV1_1` outer batches verified across the season.
    pub total_batches_verified: usize,
    /// Total inner batch legs verified across the season.
    pub total_inner_legs_verified: usize,
    /// Total `MemoData` firewall drops recorded across the season.
    pub total_firewall_drops: usize,
    /// Total post-lock `CancelOffer` refusals recorded across the season.
    pub total_post_lock_cancel_refusals: usize,
    /// Live `FinalScoreboard` computed from `TurnEngine::finalize_season`.
    pub scoreboard: FinalScoreboard,
    /// Offline `replay_season` authentication status (`Authenticated`).
    pub drand_auth_status: DrandBeaconAuthStatus,
    /// Offline `replay_season` scoreboard (guaranteed equal to `scoreboard` with `0` divergence).
    pub replay_scoreboard: FinalScoreboard,
}

/// Redacting XRPL signing keyring for Steward, seats, and Standing Regents.
pub struct StewardKeyRing {
    /// Table Authority/Steward signing account.
    pub steward_account: StandAloneAccount,
    /// Seated player signing accounts (`0..seat_count`).
    pub seat_accounts: Vec<StandAloneAccount>,
    /// Optional Standing Regent delegate signing accounts (`0..seat_count`).
    pub regent_accounts: Vec<Option<StandAloneAccount>>,
}

impl fmt::Debug for StewardKeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seat_addrs: Vec<&str> = self
            .seat_accounts
            .iter()
            .map(StandAloneAccount::address)
            .collect();
        f.debug_struct("StewardKeyRing")
            .field("steward_address", &self.steward_account.address())
            .field("seat_addresses", &seat_addrs)
            .field("keys", &"<redacted>")
            .finish()
    }
}

impl StewardKeyRing {
    /// Clone this keyring via validated `ed25519` seed reconstruction.
    pub fn try_clone(&self) -> Result<Self, StewardError> {
        let steward_account = self.steward_account.clone_ed25519()?;
        let mut seat_accounts = Vec::with_capacity(self.seat_accounts.len());
        for acc in &self.seat_accounts {
            seat_accounts.push(acc.clone_ed25519()?);
        }
        let mut regent_accounts = Vec::with_capacity(self.regent_accounts.len());
        for reg in &self.regent_accounts {
            regent_accounts.push(match reg {
                Some(acc) => Some(acc.clone_ed25519()?),
                None => None,
            });
        }
        Ok(Self {
            steward_account,
            seat_accounts,
            regent_accounts,
        })
    }
}

/// Client-detached Steward snapshot holding only the signing keyring, reserve pool, and
/// event-sourced [`ArchivistMirror`] across node restarts or container kills.
#[derive(Debug)]
pub struct DisconnectedSteward {
    keyring: StewardKeyRing,
    archivist: ArchivistMirror,
    reserve_pool: StewardReservePool,
    last_observed_ledger: u64,
    last_genesis_hash: Option<String>,
}

impl DisconnectedSteward {
    /// Borrow the event-sourced [`ArchivistMirror`].
    #[must_use]
    pub fn archivist(&self) -> &ArchivistMirror {
        &self.archivist
    }

    /// Borrow the mutable [`ArchivistMirror`] (for serialization round-trip replacement tests).
    pub fn archivist_mut(&mut self) -> &mut ArchivistMirror {
        &mut self.archivist
    }

    /// Borrow the Steward reserve pool ledger.
    #[must_use]
    pub fn reserve_pool(&self) -> &StewardReservePool {
        &self.reserve_pool
    }

    /// Highest validated ledger index observed prior to disconnect.
    #[must_use]
    pub const fn last_observed_ledger(&self) -> u64 {
        self.last_observed_ledger
    }

    /// Consume into `(keyring, archivist, reserve_pool, last_observed_ledger, last_genesis_hash)`.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        StewardKeyRing,
        ArchivistMirror,
        StewardReservePool,
        u64,
        Option<String>,
    ) {
        (
            self.keyring,
            self.archivist,
            self.reserve_pool,
            self.last_observed_ledger,
            self.last_genesis_hash,
        )
    }

    /// Materialize a live [`GameTableSteward`] bound to `client` strictly from `self.archivist`.
    pub fn reconnect<'b, T: Transport>(
        self,
        client: &'b StandAlone<T>,
    ) -> Result<GameTableSteward<'b, T>, StewardError> {
        GameTableSteward::from_archivist_mirror(
            client,
            self.archivist,
            self.keyring,
            self.reserve_pool,
            self.last_observed_ledger,
            self.last_genesis_hash,
        )
    }
}

/// Summary returned by [`GameTableSteward::rehydrate_if_wiped`] after 3-wave Phoenix recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoenixRehydrationReport {
    /// Wipe signal code (`WIPE_AUTHORITY_MISSING`, `WIPE_LEDGER_REWIND`, `WIPE_GENESIS_CHANGED`).
    pub wipe_code: String,
    /// Full 3-wave Phoenix execution metrics and remapped object IDs.
    pub phoenix: PhoenixRehydrationOutcome,
    /// Updated turn window anchored on the fresh ledger's `close_time`.
    pub active_window: StewardTurnWindow,
    /// Active turn (`1..=32`) at recovery.
    pub turn: u8,
}

#[derive(Debug, Clone)]
struct NativeEscrowRecord {
    owner_seat: u8,
    dest_seat: u8,
    xrpl_sequence: u32,
    condition_hex: String,
    finish_after_ripple: Option<u64>,
    cancel_after_ripple: u64,
}

#[derive(Debug, Clone)]
struct NativeCheckRecord {
    drawer_seat: u8,
    payee_seat: u8,
    amount_milli: Units,
    check_id: String,
}

#[derive(Debug, Clone)]
struct CommitEscrowRecord {
    sequence: u32,
    condition_hex: String,
    fulfillment_hex: String,
}

/// Stateful Phase-1 Game Mode Steward bridging `TurnEngine` onto a checked stand-alone `xrpld` node.
pub struct GameTableSteward<'a, T: Transport> {
    client: &'a StandAlone<T>,
    engine: TurnEngine,
    layout_seed: u64,
    sealed_deal: SealedMandateDeal,
    mandate_commits: Vec<Option<CommitmentHash>>,
    mandate_reveals: Vec<Option<MandateReveal>>,
    steward_account: StandAloneAccount,
    seat_accounts: Vec<StandAloneAccount>,
    regent_accounts: Vec<Option<StandAloneAccount>>,
    steward_tickets: TicketPool,
    seat_tickets: Vec<TicketPool>,
    cred_currency: String,
    raw_currencies: Vec<String>,
    refined_currencies: Vec<String>,
    authority_flags: u32,
    table_domain_id: String,
    cartel_domain_ids: BTreeMap<u8, String>,
    last_credit_tiers: Vec<CreditTier>,
    last_identities: Vec<SeatIdentity>,
    last_observed_ledger: u64,
    last_genesis_hash: Option<String>,
    last_oracle_update_time: u32,
    turn_span_ripple_secs: u64,
    window: StewardTurnWindow,
    firewall: MemoFirewall,
    native_escrows: BTreeMap<u64, NativeEscrowRecord>,
    custody_escrows: BTreeSet<u64>,
    native_checks: BTreeMap<u64, NativeCheckRecord>,
    active_commit_escrows: BTreeMap<u8, CommitEscrowRecord>,
    turn_market_actions: Vec<MarketAction>,
    turn_cancel_lock_at: Option<u32>,
    turn_rejected_count: usize,
    turn_commitments: BTreeMap<u8, CommitmentHash>,
    turn_reveals: BTreeMap<u8, OrderReveal>,
    logged_turns: Vec<LoggedTurn>,
    live_state_digests: Vec<[u8; 32]>,
    archivist: ArchivistMirror,
    reserve_pool: StewardReservePool,
    total_batches_verified: usize,
    total_inner_legs_verified: usize,
    total_post_lock_cancel_refusals: usize,
}

impl<'a, T: Transport> GameTableSteward<'a, T> {
    /// Bootstrap a complete `TableMode::Game` table on `client` from `spec`, `layout_seed`, and
    /// `sealed_deal`: configures Authority flags, opens authorized `CRED` + `2N` commodity trust
    /// lines, allocates `TicketCreate` pools, issues `Credentials` + `PermissionedDomainSet`,
    /// sets up `DelegateSet` Standing Regents, mints starting `CRED`, and checks `0` milli-unit
    /// initial `account_lines` parity.
    pub fn bootstrap(
        client: &'a StandAlone<T>,
        spec: TableSpec,
        layout_seed: u64,
        sealed_deal: SealedMandateDeal,
    ) -> Result<Self, StewardError> {
        Self::bootstrap_with_ripple_turn_span(
            client,
            spec,
            layout_seed,
            sealed_deal,
            DEFAULT_TURN_SPAN_RIPPLE_SECS,
        )
    }

    /// Variant of [`Self::bootstrap`] accepting an explicit per-turn `xrpld` `close_time` span
    /// (in Ripple seconds) for deterministic window testing.
    pub fn bootstrap_with_ripple_turn_span(
        client: &'a StandAlone<T>,
        spec: TableSpec,
        layout_seed: u64,
        sealed_deal: SealedMandateDeal,
        turn_span_ripple_secs: u64,
    ) -> Result<Self, StewardError> {
        if spec.mode != TableMode::Game {
            return Err(StewardError::Protocol(format!(
                "GameTableSteward requires TableMode::Game, got {:?}",
                spec.mode
            )));
        }
        if turn_span_ripple_secs < 20 {
            return Err(StewardError::Protocol(format!(
                "turn_span_ripple_secs must be >= 20, got {turn_span_ripple_secs}"
            )));
        }
        let schedule = spec.drand_schedule.ok_or_else(|| {
            StewardError::Protocol("TableMode::Game TableSpec requires drand_schedule".into())
        })?;
        schedule.validate()?;

        let _amendments = client.verify_v2_amendments()?;
        let mut engine = TurnEngine::new(spec.clone())?;
        let n = usize::from(spec.seat_count);

        let (steward_account, _) = client.fund_new_account(INITIAL_ACCOUNT_XRP_DROPS)?;
        let authority_flags = configure_steward_authority(client, &steward_account)?;

        let mut steward_tickets = TicketPool::default();
        steward_tickets.allocate(client, &steward_account, INITIAL_TICKET_COUNT)?;

        let mut seat_accounts = Vec::with_capacity(n);
        let mut regent_accounts = Vec::with_capacity(n);
        let mut seat_tickets = Vec::with_capacity(n);
        let mut seat_addresses = Vec::with_capacity(n);

        for seat_cfg in &spec.seats {
            let (seat_acc, _) = client.fund_new_account(INITIAL_ACCOUNT_XRP_DROPS)?;
            let mut pool = TicketPool::default();
            pool.allocate(client, &seat_acc, INITIAL_TICKET_COUNT)?;

            let regent_opt = if seat_cfg.regent.is_some() {
                let (regent_acc, _) = client.fund_new_account(INITIAL_ACCOUNT_XRP_DROPS)?;
                let reply = submit_delegate_set(
                    client,
                    &seat_acc,
                    &regent_acc,
                    REGENT_DELEGATE_PERMISSIONS,
                )?;
                let validated = reply
                    .get("validated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let result = reply
                    .get("meta")
                    .and_then(|m| m.get("TransactionResult"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if !validated || result != "tesSUCCESS" {
                    return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                        tx_type: "DelegateSet".to_string(),
                        result: result.to_string(),
                    }));
                }
                Some(regent_acc)
            } else {
                None
            };

            seat_addresses.push(seat_acc.address().to_string());
            seat_accounts.push(seat_acc);
            regent_accounts.push(regent_opt);
            seat_tickets.push(pool);
        }

        // Issue and accept seat identity + CREDIT_AAA credentials, then bind table PermissionedDomain.
        let mut last_credit_tiers = Vec::with_capacity(n);
        let mut last_identities = Vec::with_capacity(n);
        for (idx, seat_acc) in seat_accounts.iter().enumerate() {
            let identity = spec.seats[idx].identity;
            let id_cred = seat_identity_credential(identity);
            issue_and_accept_credential(client, &steward_account, seat_acc, id_cred, None, None)?;
            issue_and_accept_credential(
                client,
                &steward_account,
                seat_acc,
                credit_tier_credential(CreditTier::Aaa),
                None,
                None,
            )?;
            last_credit_tiers.push(CreditTier::Aaa);
            last_identities.push(identity);
        }

        let table_domain_id = set_permissioned_domain(
            client,
            &steward_account,
            None,
            &["SEAT_HUMAN", "SEAT_AI_SYNDICATE"],
        )?;

        let cred_currency = asset_currency_code(AssetId::Cred)?;
        let mut raw_currencies = Vec::with_capacity(n);
        let mut refined_currencies = Vec::with_capacity(n);
        for i in 0..spec.seat_count {
            raw_currencies.push(asset_currency_code(AssetId::Raw(i))?);
            refined_currencies.push(asset_currency_code(AssetId::Refined(i))?);
        }

        // Open and authorize trust lines for CRED and all 2N commodities across every seat.
        for (s_idx, seat_acc) in seat_accounts.iter().enumerate() {
            let mut all_currencies = Vec::with_capacity(1 + 2 * n);
            all_currencies.push(cred_currency.as_str());
            for c in &raw_currencies {
                all_currencies.push(c.as_str());
            }
            for c in &refined_currencies {
                all_currencies.push(c.as_str());
            }
            for currency in all_currencies {
                steward_tickets.ensure_headroom(
                    client,
                    &steward_account,
                    TICKET_REPLENISH_THRESHOLD,
                    INITIAL_TICKET_COUNT,
                )?;
                seat_tickets[s_idx].ensure_headroom(
                    client,
                    seat_acc,
                    TICKET_REPLENISH_THRESHOLD,
                    INITIAL_TICKET_COUNT,
                )?;
                let st_t = steward_tickets.pop();
                let se_t = seat_tickets[s_idx].pop();
                open_authorized_trust_line(
                    client,
                    &steward_account,
                    seat_acc,
                    currency,
                    DEFAULT_TRUST_LINE_LIMIT,
                    st_t,
                    se_t,
                )?;
            }
        }

        // Mint starting CRED balances across all seats atomically via BatchV1_1.
        let mut mint_legs = Vec::new();
        for (s_idx, seat_acc) in seat_accounts.iter().enumerate() {
            let init_cred = spec.seats[s_idx].initial_cred;
            if init_cred > 0 {
                mint_legs.push(payment_iou_json(
                    steward_account.address(),
                    seat_acc.address(),
                    steward_account.address(),
                    &cred_currency,
                    init_cred,
                )?);
            }
        }
        let mut total_batches_verified = 0usize;
        let mut total_inner_legs_verified = 0usize;
        if mint_legs.len() == 1 {
            let init_cred = spec.seats[0].initial_cred;
            let t = steward_tickets.pop();
            submit_single_payment(
                client,
                &steward_account,
                seat_accounts[0].address(),
                steward_account.address(),
                &cred_currency,
                init_cred,
                t,
            )?;
        } else if mint_legs.len() >= 2 {
            for chunk in split_batch_chunks(&mint_legs) {
                let res = submit_batch_all_or_nothing(client, &steward_account, chunk, &[], None)?;
                total_batches_verified += 1;
                total_inner_legs_verified += res.inners.len();
            }
        }

        // Submit Turn-1 dealt mandate commitments into TurnEngine.
        let mut mandate_commits = Vec::with_capacity(n);
        let mut mandate_reveals = Vec::with_capacity(n);
        for (s_idx, seat_reveal) in sealed_deal.seat_reveals.iter().enumerate().take(n) {
            let commit = compute_mandate_commitment(spec.table_id, Seat(s_idx as u8), seat_reveal)?;
            engine.submit_mandate_commitment(Seat(s_idx as u8), commit)?;
            mandate_commits.push(Some(commit));
            mandate_reveals.push(Some(seat_reveal.clone()));
        }

        let (last_observed_ledger, open_ripple_close) = client.validated_close_time()?;
        let window = build_turn_window(
            1,
            open_ripple_close,
            turn_span_ripple_secs,
            spec.cancel_lock_bps,
            schedule,
        )?;
        let firewall = MemoFirewall::new(steward_account.address().to_string(), &seat_addresses);

        let regent_count = u8::try_from(regent_accounts.iter().filter(|r| r.is_some()).count())
            .map_err(|_| StewardError::Protocol("regent count overflow".into()))?;
        let reserve_params = ReserveParams::default();
        let mut reserve_pool =
            StewardReservePool::new(reserve_params, DEFAULT_TESTNET_FAUCET_GRANT_DROPS * 2);
        let reserve_est = estimate_table_reserves(
            &reserve_params,
            spec.seat_count,
            regent_count,
            INITIAL_TICKET_COUNT,
            24,
        )?;
        reserve_pool.allocate_table(reserve_est.total_table_estimate_drops)?;

        let regent_addresses: Vec<Option<String>> = regent_accounts
            .iter()
            .map(|opt| opt.as_ref().map(|a| a.address().to_string()))
            .collect();
        let initial_state_digest = engine.state_digest()?;
        let mut archivist = ArchivistMirror::new();
        archivist.append(ArchivistEvent::TableBootstrapped {
            table_id: spec.table_id.0,
            spec: Box::new(spec),
            layout_seed,
            sealed_deal: sealed_deal.clone(),
            mandate_commits: mandate_commits.clone(),
            mandate_reveals: mandate_reveals.clone(),
            turn_span_ripple_secs,
            steward_address: steward_account.address().to_string(),
            seat_addresses,
            regent_addresses,
            authority_flags,
            table_domain_id: table_domain_id.clone(),
            initial_ledger_index: last_observed_ledger,
            initial_window: window,
            initial_state_digest,
            bootstrap_batches_verified: total_batches_verified,
            bootstrap_inner_legs_verified: total_inner_legs_verified,
        })?;

        let steward = Self {
            client,
            engine,
            layout_seed,
            sealed_deal,
            mandate_commits,
            mandate_reveals,
            steward_account,
            seat_accounts,
            regent_accounts,
            steward_tickets,
            seat_tickets,
            cred_currency,
            raw_currencies,
            refined_currencies,
            authority_flags,
            table_domain_id,
            cartel_domain_ids: BTreeMap::new(),
            last_credit_tiers,
            last_identities,
            last_observed_ledger,
            last_genesis_hash: None,
            last_oracle_update_time: 0,
            turn_span_ripple_secs,
            window,
            firewall,
            native_escrows: BTreeMap::new(),
            custody_escrows: BTreeSet::new(),
            native_checks: BTreeMap::new(),
            active_commit_escrows: BTreeMap::new(),
            turn_market_actions: Vec::new(),
            turn_cancel_lock_at: None,
            turn_rejected_count: 0,
            turn_commitments: BTreeMap::new(),
            turn_reveals: BTreeMap::new(),
            logged_turns: Vec::new(),
            live_state_digests: Vec::new(),
            archivist,
            reserve_pool,
            total_batches_verified,
            total_inner_legs_verified,
            total_post_lock_cancel_refusals: 0,
        };
        steward.assert_account_lines_parity()?;
        Ok(steward)
    }

    /// Borrow the event-sourced [`ArchivistMirror`].
    #[must_use]
    pub fn archivist(&self) -> &ArchivistMirror {
        &self.archivist
    }

    /// Borrow the Steward XRP reserve pool ledger.
    #[must_use]
    pub fn reserve_pool(&self) -> &StewardReservePool {
        &self.reserve_pool
    }

    /// Borrow the mutable Steward XRP reserve pool ledger.
    pub fn reserve_pool_mut(&mut self) -> &mut StewardReservePool {
        &mut self.reserve_pool
    }

    /// Borrow the recorded post-Sweep canonical `EngineState` SHA-256 digests.
    #[must_use]
    pub fn live_state_digests(&self) -> &[[u8; 32]] {
        &self.live_state_digests
    }

    /// Clone the Steward, seat, and Standing Regent signing keyring.
    pub fn keyring_clone(&self) -> Result<StewardKeyRing, StewardError> {
        let mut seat_accounts = Vec::with_capacity(self.seat_accounts.len());
        for acc in &self.seat_accounts {
            seat_accounts.push(acc.clone_ed25519()?);
        }
        let mut regent_accounts = Vec::with_capacity(self.regent_accounts.len());
        for reg in &self.regent_accounts {
            regent_accounts.push(match reg {
                Some(acc) => Some(acc.clone_ed25519()?),
                None => None,
            });
        }
        Ok(StewardKeyRing {
            steward_account: self.steward_account.clone_ed25519()?,
            seat_accounts,
            regent_accounts,
        })
    }

    /// Detach this Steward from its RPC client borrow so the caller can tear down and replace
    /// the underlying `xrpld` container before reconnecting via [`DisconnectedSteward::reconnect`].
    #[must_use]
    pub fn into_disconnected(self) -> DisconnectedSteward {
        DisconnectedSteward {
            keyring: StewardKeyRing {
                steward_account: self.steward_account,
                seat_accounts: self.seat_accounts,
                regent_accounts: self.regent_accounts,
            },
            archivist: self.archivist,
            reserve_pool: self.reserve_pool,
            last_observed_ledger: self.last_observed_ledger,
            last_genesis_hash: self.last_genesis_hash,
        }
    }

    /// Reconstruct a [`GameTableSteward`] on `client` strictly from serialized [`ArchivistMirror`]
    /// JSON bytes, a [`StewardKeyRing`], a [`StewardReservePool`], and the last observed ledger index.
    pub fn from_archivist_bytes(
        client: &'a StandAlone<T>,
        archivist_json: &[u8],
        keyring: StewardKeyRing,
        reserve_pool: StewardReservePool,
        previous_ledger: u64,
    ) -> Result<Self, StewardError> {
        let archivist = ArchivistMirror::from_json_slice(archivist_json)?;
        Self::from_archivist_mirror(
            client,
            archivist,
            keyring,
            reserve_pool,
            previous_ledger,
            None,
        )
    }

    fn from_archivist_mirror(
        client: &'a StandAlone<T>,
        archivist: ArchivistMirror,
        keyring: StewardKeyRing,
        reserve_pool: StewardReservePool,
        previous_ledger: u64,
        last_genesis_hash: Option<String>,
    ) -> Result<Self, StewardError> {
        let mat: MaterializedArchivistState = archivist.materialize()?;
        let n = usize::from(mat.engine.state().spec.seat_count);
        if keyring.seat_accounts.len() != n || keyring.regent_accounts.len() != n {
            return Err(StewardError::ArchivistIntegrity(
                "keyring seat/regent count mismatch during reconnect".into(),
            ));
        }
        if keyring.steward_account.address() != mat.steward_address {
            return Err(StewardError::ArchivistIntegrity(
                "keyring steward address mismatch during reconnect".into(),
            ));
        }
        for idx in 0..n {
            if keyring.seat_accounts[idx].address() != mat.seat_addresses[idx] {
                return Err(StewardError::ArchivistIntegrity(format!(
                    "keyring seat {idx} address mismatch during reconnect"
                )));
            }
        }

        let cred_currency = asset_currency_code(AssetId::Cred)?;
        let mut raw_currencies = Vec::with_capacity(n);
        let mut refined_currencies = Vec::with_capacity(n);
        for g in 0..mat.engine.state().spec.seat_count {
            raw_currencies.push(asset_currency_code(AssetId::Raw(g))?);
            refined_currencies.push(asset_currency_code(AssetId::Refined(g))?);
        }

        let native_escrows = mat
            .native_escrows
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    NativeEscrowRecord {
                        owner_seat: v.owner_seat,
                        dest_seat: v.dest_seat,
                        xrpl_sequence: v.xrpl_sequence,
                        condition_hex: v.condition_hex,
                        finish_after_ripple: v.finish_after_ripple,
                        cancel_after_ripple: v.cancel_after_ripple,
                    },
                )
            })
            .collect();
        let native_checks = mat
            .native_checks
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    NativeCheckRecord {
                        drawer_seat: v.drawer_seat,
                        payee_seat: v.payee_seat,
                        amount_milli: v.amount_milli,
                        check_id: v.xrpl_check_id,
                    },
                )
            })
            .collect();
        let active_commit_escrows = mat
            .active_commit_escrows
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    CommitEscrowRecord {
                        sequence: v.sequence,
                        condition_hex: v.condition_hex,
                        fulfillment_hex: v.fulfillment_hex,
                    },
                )
            })
            .collect();

        let firewall = MemoFirewall::from_parts(
            mat.steward_address,
            &mat.seat_addresses,
            mat.dropped_records,
        );

        let mut steward = Self {
            client,
            engine: mat.engine,
            layout_seed: mat.layout_seed,
            sealed_deal: mat.sealed_deal,
            mandate_commits: mat.mandate_commits,
            mandate_reveals: mat.mandate_reveals,
            steward_account: keyring.steward_account,
            seat_accounts: keyring.seat_accounts,
            regent_accounts: keyring.regent_accounts,
            steward_tickets: TicketPool::default(),
            seat_tickets: vec![TicketPool::default(); n],
            cred_currency,
            raw_currencies,
            refined_currencies,
            authority_flags: mat.authority_flags,
            table_domain_id: mat.table_domain_id,
            cartel_domain_ids: mat.cartel_domain_ids,
            last_credit_tiers: mat.last_credit_tiers,
            last_identities: mat.last_identities,
            last_observed_ledger: previous_ledger.max(mat.last_observed_ledger),
            last_genesis_hash,
            last_oracle_update_time: 0,
            turn_span_ripple_secs: mat.turn_span_ripple_secs,
            window: mat.window,
            firewall,
            native_escrows,
            custody_escrows: mat.custody_escrows,
            native_checks,
            active_commit_escrows,
            turn_market_actions: mat.turn_market_actions,
            turn_cancel_lock_at: mat.turn_cancel_lock_at,
            turn_rejected_count: mat.turn_rejected_count,
            turn_commitments: mat.turn_commitments,
            turn_reveals: mat.turn_reveals,
            logged_turns: mat.logged_turns,
            live_state_digests: mat.live_state_digests,
            archivist,
            reserve_pool,
            total_batches_verified: mat.total_batches_verified,
            total_inner_legs_verified: mat.total_inner_legs_verified,
            total_post_lock_cancel_refusals: mat.total_post_lock_cancel_refusals,
        };

        // If the connected node still holds our table accounts (no wipe), refresh TicketPools.
        if steward.detect_wipe()?.is_none() {
            steward
                .steward_tickets
                .refresh_from_ledger(client, steward.steward_account.address())?;
            for (idx, acc) in steward.seat_accounts.iter().enumerate() {
                steward.seat_tickets[idx].refresh_from_ledger(client, acc.address())?;
            }
        }
        Ok(steward)
    }

    /// Probe the connected `xrpld` node for a ledger wipe or reset using `league_season`.
    pub fn detect_wipe(&self) -> Result<Option<WipeSignal>, StewardError> {
        let (observed_ledger, _) = self.client.validated_close_time()?;
        let authority_exists = match self.client.call(
            "account_info",
            json!({
                "account": self.steward_account.address(),
                "ledger_index": "validated"
            }),
        ) {
            Ok(_) => Some(true),
            Err(StandAloneError::Rpc { error, .. }) if error == "actNotFound" => Some(false),
            Err(err) => return Err(StewardError::StandAlone(err)),
        };
        Ok(detect_ledger_wipe(&WipeProbeInput {
            authority_address: self.steward_account.address(),
            authority_exists,
            previous_ledger: Some(self.last_observed_ledger),
            observed_ledger: Some(observed_ledger),
            previous_genesis_hash: self.last_genesis_hash.as_deref(),
            observed_genesis_hash: None,
        }))
    }

    /// Detect a ledger wipe via `league-season` and, if wiped, materialize the exact state from
    /// [`ArchivistMirror`] and run the 3-wave `BatchV1_1` + `TicketCreate` Phoenix rehydrator in
    /// `<= 3` `ledger_accept` closes, ending with a `0` milli-unit `account_lines` parity assertion.
    pub fn rehydrate_if_wiped(&mut self) -> Result<Option<PhoenixRehydrationReport>, StewardError> {
        let Some(wipe_signal) = self.detect_wipe()? else {
            return Ok(None);
        };
        let mat = self.archivist.materialize()?;
        let exec = execute_three_wave_phoenix(
            self.client,
            &wipe_signal,
            &mat,
            &self.steward_account,
            &self.seat_accounts,
            &self.regent_accounts,
            &mut self.reserve_pool,
        )?;
        let outcome = exec.outcome;

        self.engine = mat.engine;
        self.steward_tickets = exec.steward_tickets;
        self.seat_tickets = exec.seat_tickets;
        self.table_domain_id = outcome.new_table_domain_id.clone();
        self.cartel_domain_ids = outcome.remapped_cartel_domains.clone();
        self.last_credit_tiers = mat.last_credit_tiers;
        self.last_identities = mat.last_identities;
        self.last_observed_ledger = outcome.rehydrated_ledger;
        self.last_oracle_update_time = outcome.wave3_oracle_update_time;

        let current_turn = self.engine.state().turn;
        self.native_escrows.clear();
        for (eid, esc) in mat.native_escrows {
            let new_seq = outcome
                .remapped_escrow_sequences
                .get(&eid)
                .copied()
                .unwrap_or(esc.xrpl_sequence);
            let finish_after_ripple = esc
                .finish_after_turn
                .filter(|&ft| ft > current_turn)
                .map(|ft| outcome.rehydrated_ripple_close + u64::from(ft - current_turn) * 15);
            let cancel_after_ripple = outcome.rehydrated_ripple_close
                + u64::from(esc.cancel_after_turn.saturating_sub(current_turn).max(1)) * 60;
            self.native_escrows.insert(
                eid,
                NativeEscrowRecord {
                    owner_seat: esc.owner_seat,
                    dest_seat: esc.dest_seat,
                    xrpl_sequence: new_seq,
                    condition_hex: esc.condition_hex,
                    finish_after_ripple,
                    cancel_after_ripple,
                },
            );
        }
        self.custody_escrows = mat.custody_escrows;

        self.native_checks.clear();
        for (cid, chk) in mat.native_checks {
            let new_cid = outcome
                .remapped_check_ids
                .get(&cid)
                .cloned()
                .unwrap_or(chk.xrpl_check_id);
            self.native_checks.insert(
                cid,
                NativeCheckRecord {
                    drawer_seat: chk.drawer_seat,
                    payee_seat: chk.payee_seat,
                    amount_milli: chk.amount_milli,
                    check_id: new_cid,
                },
            );
        }

        self.active_commit_escrows.clear();
        for (seat, ce) in mat.active_commit_escrows {
            let new_seq = outcome
                .remapped_commit_sequences
                .get(&seat)
                .copied()
                .unwrap_or(ce.sequence);
            self.active_commit_escrows.insert(
                seat,
                CommitEscrowRecord {
                    sequence: new_seq,
                    condition_hex: ce.condition_hex,
                    fulfillment_hex: ce.fulfillment_hex,
                },
            );
        }

        self.turn_market_actions = mat.turn_market_actions;
        self.turn_cancel_lock_at = mat.turn_cancel_lock_at;
        self.turn_rejected_count = mat.turn_rejected_count;
        self.turn_commitments = mat.turn_commitments;
        self.turn_reveals = mat.turn_reveals;
        self.logged_turns = mat.logged_turns;
        self.live_state_digests = mat.live_state_digests;
        self.total_batches_verified = mat
            .total_batches_verified
            .saturating_add(outcome.total_batches_verified);
        self.total_inner_legs_verified = mat
            .total_inner_legs_verified
            .saturating_add(outcome.total_inner_legs_verified);
        self.total_post_lock_cancel_refusals = mat.total_post_lock_cancel_refusals;

        let schedule = self.engine.state().spec.drand_schedule.ok_or_else(|| {
            StewardError::Protocol("missing drand_schedule on active Game engine".into())
        })?;
        let new_window = build_turn_window(
            current_turn,
            outcome.rehydrated_ripple_close,
            self.turn_span_ripple_secs,
            self.engine.state().spec.cancel_lock_bps,
            schedule,
        )?;
        self.window = new_window;

        self.archivist.append(ArchivistEvent::PhoenixRehydrated {
            at_turn: current_turn,
            wipe_code: outcome.wipe_code.clone(),
            previous_ledger: outcome.previous_ledger,
            fresh_boot_ledger: outcome.fresh_boot_ledger,
            rehydrated_ledger: outcome.rehydrated_ledger,
            ledger_closes_used: outcome.ledger_closes_used,
            new_table_domain_id: outcome.new_table_domain_id.clone(),
            remapped_cartel_domains: outcome.remapped_cartel_domains.clone(),
            remapped_escrow_sequences: outcome.remapped_escrow_sequences.clone(),
            remapped_check_ids: outcome.remapped_check_ids.clone(),
            remapped_commit_sequences: outcome.remapped_commit_sequences.clone(),
            new_window,
            wave_batches_verified: outcome.total_batches_verified,
            wave_inner_legs_verified: outcome.total_inner_legs_verified,
        })?;

        self.assert_account_lines_parity()?;

        Ok(Some(PhoenixRehydrationReport {
            wipe_code: outcome.wipe_code.clone(),
            phoenix: outcome,
            active_window: new_window,
            turn: current_turn,
        }))
    }

    /// Borrow the underlying checked [`StandAlone`] client.
    #[must_use]
    pub fn client(&self) -> &'a StandAlone<T> {
        self.client
    }

    /// Current turn index (`1..=32`).
    #[must_use]
    pub fn turn(&self) -> u8 {
        self.engine.state().turn
    }

    /// Current turn phase (`TalkAndMarket`, `Commit`, `Reveal`, `AwaitingMandateReveal`, `Completed`).
    #[must_use]
    pub fn phase(&self) -> TurnPhase {
        self.engine.state().phase
    }

    /// Current turn's validated `xrpld` ledger-time window.
    #[must_use]
    pub fn current_window(&self) -> StewardTurnWindow {
        self.window
    }

    /// Whether `cancel_lock` is currently engaged on `TurnEngine`.
    #[must_use]
    pub fn is_cancel_locked(&self) -> bool {
        self.engine.state().cancel_locked
    }

    /// Verified `AccountRoot.Flags` bitmask on Authority.
    #[must_use]
    pub fn authority_flags(&self) -> u32 {
        self.authority_flags
    }

    /// Active table `PermissionedDomain` 64-hex ID.
    #[must_use]
    pub fn table_domain_id(&self) -> &str {
        &self.table_domain_id
    }

    /// Steward issuing account.
    #[must_use]
    pub fn steward_account(&self) -> &StandAloneAccount {
        &self.steward_account
    }

    /// Seat `idx` XRPL account (`0..seat_count`).
    #[must_use]
    pub fn seat_account(&self, idx: u8) -> Option<&StandAloneAccount> {
        self.seat_accounts.get(usize::from(idx))
    }

    /// Standing Regent delegate XRPL account for `idx`, if configured.
    #[must_use]
    pub fn regent_account(&self, idx: u8) -> Option<&StandAloneAccount> {
        self.regent_accounts
            .get(usize::from(idx))
            .and_then(Option::as_ref)
    }

    /// Borrow the `MemoData` ingress firewall (including sanitized drop telemetry).
    #[must_use]
    pub fn firewall(&self) -> &MemoFirewall {
        &self.firewall
    }

    /// Borrow the mutable `MemoData` ingress firewall for direct adversarial probing.
    pub fn firewall_mut(&mut self) -> &mut MemoFirewall {
        &mut self.firewall
    }

    /// Current remaining tickets in Steward's local `TicketPool`.
    #[must_use]
    pub fn steward_tickets_remaining(&self) -> usize {
        self.steward_tickets.remaining()
    }

    /// Current remaining tickets in seat `idx`'s local `TicketPool`.
    #[must_use]
    pub fn seat_tickets_remaining(&self, idx: u8) -> usize {
        self.seat_tickets
            .get(usize::from(idx))
            .map_or(0, TicketPool::remaining)
    }

    /// Read live validated `account_lines` balances in integer milli-units for `seat`.
    pub fn read_seat_balances_milli(
        &self,
        seat: u8,
    ) -> Result<BTreeMap<String, Units>, StewardError> {
        let seat_acc = self
            .seat_accounts
            .get(usize::from(seat))
            .ok_or(StewardError::Engine(EngineError::InvalidSeat(seat)))?;
        let lines = self.client.account_lines(seat_acc.address())?;
        let mut map = BTreeMap::new();
        for line in lines {
            if line.account == self.steward_account.address() {
                map.insert(line.currency, parse_iou_milli_exact(&line.balance)?);
            }
        }
        Ok(map)
    }

    /// Check the latest validated `xrpld` ledger header, enforcing `ledger_went_backwards` and
    /// engaging `TurnEngine::begin_cancel_lock` solely when validated `close_time >= lock_ripple_close`.
    pub fn observe_validated_header(&mut self) -> Result<(u64, u64), StewardError> {
        let (ledger_seq, ripple_close) = self.client.validated_close_time()?;
        if ledger_went_backwards(Some(self.last_observed_ledger), Some(ledger_seq)) {
            return Err(StewardError::LedgerRewind {
                previous_ledger: self.last_observed_ledger,
                observed_ledger: ledger_seq,
            });
        }
        self.last_observed_ledger = self.last_observed_ledger.max(ledger_seq);
        if self.engine.state().phase == TurnPhase::TalkAndMarket
            && !self.engine.state().cancel_locked
            && self.engine.state().spec.cancel_lock_bps > 0
            && ripple_close >= self.window.lock_ripple_close
        {
            self.engine.begin_cancel_lock()?;
            if self.turn_cancel_lock_at.is_none() {
                let lock_idx = self.turn_market_actions.len() as u32;
                self.turn_cancel_lock_at = Some(lock_idx);
                self.archivist.append(ArchivistEvent::CancelLockEngaged {
                    turn: self.engine.state().turn,
                    cancel_lock_at: lock_idx,
                    ledger_index: ledger_seq,
                    ripple_close,
                })?;
            }
        }
        Ok((ledger_seq, ripple_close))
    }

    /// Advance the stand-alone ledger via `ledger_accept` until validated `close_time` reaches the
    /// active turn's `lock_ripple_close`, then verify `cancel_lock` engages from `close_time`.
    pub fn advance_to_cancel_lock(&mut self) -> Result<u64, StewardError> {
        let (_, mut ripple_close) = self.observe_validated_header()?;
        for _ in 0..500 {
            if ripple_close >= self.window.lock_ripple_close {
                let _ = self.observe_validated_header()?;
                return Ok(ripple_close);
            }
            let _ = self.client.ledger_accept()?;
            (_, ripple_close) = self.observe_validated_header()?;
        }
        Err(StewardError::Protocol(format!(
            "xrpld close_time {ripple_close} did not reach cancel lock {}",
            self.window.lock_ripple_close
        )))
    }

    /// Submit one Phase-1 [`MarketAction`] onto `TurnEngine` and mirror custody/escrows/checks onto
    /// `xrpld`. Refuses post-lock `CancelOffer` with [`StewardError::CancelLocked`] and zero
    /// ledger refund (`100%` refusal).
    pub fn submit_market_action(&mut self, action: MarketAction) -> Result<(), StewardError> {
        let (_, ripple_close) = self.observe_validated_header()?;
        let turn = self.engine.state().turn;
        let was_locked = self.engine.state().cancel_locked;

        // Snapshot affected seat balances & existing objects before engine mutation so ledger legs
        // match exact TurnEngine deltas.
        let seat_idx = action_actor_seat(&action);
        let pre_seat = self
            .engine
            .state()
            .seats
            .get(usize::from(seat_idx))
            .cloned();
        let offer_before = match &action {
            MarketAction::CancelOffer { offer_id, .. } => {
                self.engine.state().offers.get(offer_id).cloned()
            }
            _ => None,
        };
        let escrow_before = match &action {
            MarketAction::CancelExpiredEscrow { escrow_id, .. } => {
                self.engine.state().escrows.get(escrow_id).cloned()
            }
            _ => None,
        };
        let next_escrow_id = self.engine.state().next_escrow_id;
        let next_check_id = self.engine.state().next_check_id;

        self.turn_market_actions.push(action.clone());
        if let Err(err) = self.engine.apply_market_action(action.clone()) {
            self.turn_rejected_count += 1;
            let is_cancel_lock = matches!(err, EngineError::CancelLocked);
            self.archivist
                .append(ArchivistEvent::MarketActionRejected {
                    turn,
                    action,
                    post_lock_cancel_refused: is_cancel_lock,
                })?;
            if is_cancel_lock {
                self.total_post_lock_cancel_refusals += 1;
                let logical_close = self.map_ripple_to_table_unix(ripple_close);
                return Err(StewardError::CancelLocked {
                    turn,
                    close_unix: logical_close,
                    lock_unix: self.window.lock_unix,
                });
            }
            return Err(StewardError::Engine(err));
        }

        let mut opened_native_escrow = None;
        let mut closed_native_escrow_id = None;
        let mut opened_native_check = None;
        let mut closed_native_check_id = None;
        let mut batches_verified_delta = 0_usize;
        let mut inner_legs_verified_delta = 0_usize;

        match action.clone() {
            MarketAction::PostOffer { seat, .. } => {
                if let Some(before) = pre_seat {
                    self.mirror_seat_free_delta_after_action(seat, &before)?;
                }
            }
            MarketAction::CancelOffer { seat, .. } => {
                debug_assert!(!was_locked);
                let _ = offer_before;
                if let Some(before) = pre_seat {
                    self.mirror_seat_free_delta_after_action(seat, &before)?;
                }
            }
            MarketAction::TownImport { seat, good, amount } => {
                if let Some(before) = pre_seat {
                    let after = &self.engine.state().seats[usize::from(seat)];
                    let cred_spent = before.cred - after.cred;
                    let goods_gained = match good {
                        AssetId::Raw(g) => after.raw[usize::from(g)] - before.raw[usize::from(g)],
                        AssetId::Refined(m) => {
                            after.refined[usize::from(m)] - before.refined[usize::from(m)]
                        }
                        AssetId::Cred => amount,
                    };
                    let good_code = asset_currency_code(good)?;
                    let seat_addr = self.seat_accounts[usize::from(seat)].address();
                    let legs = vec![
                        clawback_iou_json(
                            self.steward_account.address(),
                            seat_addr,
                            &self.cred_currency,
                            cred_spent,
                        )?,
                        payment_iou_json(
                            self.steward_account.address(),
                            seat_addr,
                            self.steward_account.address(),
                            &good_code,
                            goods_gained,
                        )?,
                    ];
                    let res = submit_batch_all_or_nothing(
                        self.client,
                        &self.steward_account,
                        legs,
                        &[],
                        None,
                    )?;
                    self.total_batches_verified += 1;
                    self.total_inner_legs_verified += res.inners.len();
                    batches_verified_delta = 1;
                    inner_legs_verified_delta = res.inners.len();
                }
            }
            MarketAction::CreateEscrow { seat, escrow } => {
                let use_native = escrow.condition_hash.is_some()
                    && escrow.bond.is_none()
                    && escrow.asset == AssetId::Cred;
                if use_native {
                    let Some(cond_bytes) = escrow.condition_hash else {
                        return Err(StewardError::Protocol("missing condition_hash".into()));
                    };
                    let cond_hex = der_preimage_condition_from_digest(&cond_bytes.0);
                    let dest_addr = self.seat_accounts[usize::from(escrow.destination)]
                        .address()
                        .to_string();
                    let val = format_milli_units(escrow.amount)?;
                    let finish_after_ripple = escrow
                        .finish_after_turn
                        .filter(|&ft| ft > turn)
                        .map(|ft| ripple_close + u64::from(ft - turn) * 15);
                    let cancel_after_ripple = ripple_close
                        + u64::from(escrow.cancel_after_turn.saturating_sub(turn).max(1)) * 60;
                    let mut fields = json!({
                        "Amount": {
                            "currency": self.cred_currency,
                            "issuer": self.steward_account.address(),
                            "value": val
                        },
                        "Destination": dest_addr,
                        "Condition": cond_hex,
                        "CancelAfter": u32::try_from(cancel_after_ripple)
                            .map_err(|_| StewardError::Protocol("CancelAfter overflow".into()))?
                    });
                    if let Some(fa) = finish_after_ripple {
                        fields["FinishAfter"] =
                            Value::from(u32::try_from(fa).map_err(|_| {
                                StewardError::Protocol("FinishAfter overflow".into())
                            })?);
                    }
                    self.seat_tickets[usize::from(seat)].ensure_headroom(
                        self.client,
                        &self.seat_accounts[usize::from(seat)],
                        TICKET_REPLENISH_THRESHOLD,
                        INITIAL_TICKET_COUNT,
                    )?;
                    let ticket = self.seat_tickets[usize::from(seat)].pop();
                    let (xrpl_seq, _, _) = submit_escrow_create(
                        self.client,
                        &self.seat_accounts[usize::from(seat)],
                        fields,
                        ticket,
                    )?;
                    self.native_escrows.insert(
                        next_escrow_id,
                        NativeEscrowRecord {
                            owner_seat: seat,
                            dest_seat: escrow.destination,
                            xrpl_sequence: xrpl_seq,
                            condition_hex: cond_hex.clone(),
                            finish_after_ripple,
                            cancel_after_ripple,
                        },
                    );
                    opened_native_escrow = Some(MirroredNativeEscrow {
                        escrow_id: next_escrow_id,
                        owner_seat: seat,
                        dest_seat: escrow.destination,
                        amount_milli: escrow.amount,
                        xrpl_sequence: xrpl_seq,
                        condition_hex: cond_hex,
                        finish_after_turn: escrow.finish_after_turn,
                        cancel_after_turn: escrow.cancel_after_turn,
                        finish_after_ripple,
                        cancel_after_ripple,
                    });
                } else {
                    self.custody_escrows.insert(next_escrow_id);
                    if let Some(before) = pre_seat {
                        self.mirror_seat_free_delta_after_action(seat, &before)?;
                    }
                }
            }
            MarketAction::CancelExpiredEscrow { seat, escrow_id } => {
                if let Some(rec) = self.native_escrows.remove(&escrow_id) {
                    let (_, mut now_close) = self.client.validated_close_time()?;
                    while now_close <= rec.cancel_after_ripple {
                        let _ = self.client.ledger_accept()?;
                        (_, now_close) = self.client.validated_close_time()?;
                    }
                    let res = submit_escrow_cancel(
                        self.client,
                        &self.seat_accounts[usize::from(seat)],
                        self.seat_accounts[usize::from(rec.owner_seat)].address(),
                        rec.xrpl_sequence,
                        None,
                    )?;
                    if res != "tesSUCCESS" {
                        return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                            tx_type: "EscrowCancel".to_string(),
                            result: res,
                        }));
                    }
                    closed_native_escrow_id = Some(escrow_id);
                } else {
                    let _ = escrow_before;
                    self.custody_escrows.remove(&escrow_id);
                    if let Some(before) = pre_seat {
                        self.mirror_seat_free_delta_after_action(seat, &before)?;
                    }
                }
            }
            MarketAction::CreateCheck { seat, check } => {
                let currency = asset_currency_code(check.asset)?;
                let dest_addr = self.seat_accounts[usize::from(check.payee)]
                    .address()
                    .to_string();
                let exp_ripple = u32::try_from(
                    ripple_close
                        + u64::from(check.expiration_turn.saturating_sub(turn).max(1))
                            .saturating_mul(120),
                )
                .map_err(|_| StewardError::Protocol("Check expiration overflow".into()))?;
                let check_id = submit_check_create(
                    self.client,
                    &self.seat_accounts[usize::from(seat)],
                    &dest_addr,
                    self.steward_account.address(),
                    &currency,
                    check.send_max,
                    Some(exp_ripple),
                )?;
                self.native_checks.insert(
                    next_check_id,
                    NativeCheckRecord {
                        drawer_seat: seat,
                        payee_seat: check.payee,
                        amount_milli: check.send_max,
                        check_id: check_id.clone(),
                    },
                );
                opened_native_check = Some(MirroredNativeCheck {
                    check_id_engine: next_check_id,
                    drawer_seat: seat,
                    payee_seat: check.payee,
                    currency,
                    amount_milli: check.send_max,
                    due_turn: check.due_turn,
                    expiration_turn: check.expiration_turn,
                    xrpl_check_id: check_id,
                });
            }
            MarketAction::CancelCheck { seat, check_id } => {
                if let Some(rec) = self.native_checks.remove(&check_id) {
                    let res = submit_check_cancel(
                        self.client,
                        &self.seat_accounts[usize::from(seat)],
                        &rec.check_id,
                    )?;
                    if res != "tesSUCCESS" {
                        return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                            tx_type: "CheckCancel".to_string(),
                            result: res,
                        }));
                    }
                    closed_native_check_id = Some(check_id);
                }
            }
            MarketAction::FlagWashFreeze { inspector, .. } => {
                if let Some(before) = pre_seat {
                    self.mirror_seat_free_delta_after_action(inspector, &before)?;
                }
            }
            MarketAction::RecordIntentPledge { .. } | MarketAction::StandForOffice { .. } => {}
        }

        self.archivist
            .append(ArchivistEvent::MarketActionAccepted {
                turn,
                action,
                opened_native_escrow,
                closed_native_escrow_id,
                opened_native_check,
                closed_native_check_id,
                batches_verified_delta,
                inner_legs_verified_delta,
            })?;
        let _ = self.observe_validated_header()?;
        Ok(())
    }

    /// Advance `TurnEngine` from `TalkAndMarket` into `Commit`.
    pub fn advance_to_commit(&mut self) -> Result<(), StewardError> {
        let _ = self.observe_validated_header()?;
        let turn = self.engine.state().turn;
        self.engine.advance_to_commit()?;
        self.archivist
            .append(ArchivistEvent::AdvancedToCommit { turn })?;
        Ok(())
    }

    /// Submit a live 1-drop `PREIMAGE-SHA-256` `EscrowCreate` commit transaction on `xrpld` with
    /// `MemoData` carrying the 64-hex SHA-256 commitment (`MemoType = "ml.commit.v2"`), then feed
    /// the validated transaction through [`MemoFirewall`] and validated `close_time` check before
    /// recording in `TurnEngine`.
    pub fn submit_seat_commit_via_xrpld(
        &mut self,
        seat: u8,
        commitment: CommitmentHash,
        commit_preimage: [u8; 32],
    ) -> Result<VerifiedCommitIngress, StewardError> {
        let s_idx = usize::from(seat);
        let (_, now_close) = self.observe_validated_header()?;
        let seat_acc = self
            .seat_accounts
            .get(s_idx)
            .ok_or(StewardError::Engine(EngineError::InvalidSeat(seat)))?;
        let (cond_hex, fulfil_hex) =
            league_standalone::conformance::preimage_sha256_condition_and_fulfillment(
                &commit_preimage,
            );
        let cancel_after = u32::try_from(now_close + 3_600)
            .map_err(|_| StewardError::Protocol("commit Escrow CancelAfter overflow".into()))?;
        let memos = encode_commit_memos_json(commitment);
        self.seat_tickets[s_idx].ensure_headroom(
            self.client,
            seat_acc,
            TICKET_REPLENISH_THRESHOLD,
            INITIAL_TICKET_COUNT,
        )?;
        let ticket = self.seat_tickets[s_idx].pop();
        let (seq, tx_hash, _) = submit_escrow_create(
            self.client,
            seat_acc,
            json!({
                "Amount": "1",
                "Destination": seat_acc.address(),
                "Condition": cond_hex,
                "CancelAfter": cancel_after,
                "Memos": memos
            }),
            ticket,
        )?;

        // Retrieve validated transaction by exact tx_hash from `xrpld` so concurrent
        // same-account transactions cannot race `account_tx(limit: 1)`.
        let tx_reply = self
            .client
            .call("tx", json!({ "transaction": tx_hash.clone() }))?;
        if tx_reply.get("validated").and_then(Value::as_bool) != Some(true) {
            return Err(StewardError::Protocol(
                "commit transaction is not validated".into(),
            ));
        }
        let mut tx_json = tx_reply
            .get("tx_json")
            .or_else(|| tx_reply.get("tx"))
            .cloned()
            .unwrap_or_else(|| tx_reply.clone());
        if tx_json.get("hash").is_none() {
            tx_json["hash"] = tx_reply
                .get("hash")
                .cloned()
                .unwrap_or(Value::String(tx_hash));
        }
        let tx_close_ripple = self
            .client
            .validated_close_time()
            .map_or(now_close, |(_, c)| c);

        let verified = self.ingest_commit_tx_with_escrow(
            &tx_json,
            tx_close_ripple,
            Some(MirroredCommitEscrow {
                seat,
                sequence: seq,
                condition_hex: cond_hex.clone(),
                fulfillment_hex: fulfil_hex.clone(),
            }),
        )?;
        self.active_commit_escrows.insert(
            seat,
            CommitEscrowRecord {
                sequence: seq,
                condition_hex: cond_hex,
                fulfillment_hex: fulfil_hex,
            },
        );
        Ok(verified)
    }

    /// Pass an arbitrary validated `xrpld` transaction JSON through [`MemoFirewall`] and validated
    /// `commit_close_ripple` check before admitting it to `TurnEngine::submit_order_commitment`.
    pub fn ingest_commit_tx(
        &mut self,
        tx_json: &Value,
        validated_tx_ripple_close: u64,
    ) -> Result<VerifiedCommitIngress, StewardError> {
        self.ingest_commit_tx_with_escrow(tx_json, validated_tx_ripple_close, None)
    }

    fn ingest_commit_tx_with_escrow(
        &mut self,
        tx_json: &Value,
        validated_tx_ripple_close: u64,
        commit_escrow: Option<MirroredCommitEscrow>,
    ) -> Result<VerifiedCommitIngress, StewardError> {
        let verified = match self.firewall.inspect_tx(tx_json) {
            Ok(v) => v,
            Err(rec) => {
                self.archivist.append(ArchivistEvent::FirewallDropped {
                    record: rec.clone(),
                })?;
                return Err(StewardError::FirewallDropped(rec));
            }
        };
        let logical_close = self.map_ripple_to_table_unix(validated_tx_ripple_close);
        if validated_tx_ripple_close > self.window.commit_close_ripple {
            return Err(StewardError::CommitWindowClosed {
                turn: self.engine.state().turn,
                close_unix: logical_close,
                deadline_unix: self.window.commit_close_unix,
            });
        }
        let turn = self.engine.state().turn;
        self.engine
            .submit_order_commitment(Seat(verified.seat), verified.commitment)?;
        self.turn_commitments
            .insert(verified.seat, verified.commitment);
        self.archivist.append(ArchivistEvent::SeatCommitAccepted {
            turn,
            seat: verified.seat,
            commitment: verified.commitment,
            tx_hash: verified.tx_hash.clone(),
            commit_escrow,
        })?;
        Ok(verified)
    }

    /// Advance `TurnEngine` from `Commit` to `Reveal`.
    pub fn advance_to_reveal(&mut self) -> Result<(), StewardError> {
        let _ = self.observe_validated_header()?;
        let turn = self.engine.state().turn;
        self.engine.advance_to_reveal()?;
        self.archivist
            .append(ArchivistEvent::AdvancedToReveal { turn })?;
        Ok(())
    }

    /// Finish the seat's live `PREIMAGE-SHA-256` commit escrow on `xrpld` (`Fee = "350"` drops)
    /// and record the revealed [`OrderReveal`] in `TurnEngine`.
    pub fn submit_seat_reveal_via_xrpld(
        &mut self,
        seat: u8,
        reveal: OrderReveal,
    ) -> Result<(), StewardError> {
        let s_idx = usize::from(seat);
        let seat_acc = self
            .seat_accounts
            .get(s_idx)
            .ok_or(StewardError::Engine(EngineError::InvalidSeat(seat)))?;
        if let Some(commit_escrow) = self.active_commit_escrows.remove(&seat) {
            self.seat_tickets[s_idx].ensure_headroom(
                self.client,
                seat_acc,
                TICKET_REPLENISH_THRESHOLD,
                INITIAL_TICKET_COUNT,
            )?;
            let ticket = self.seat_tickets[s_idx].pop();
            let result = submit_escrow_finish(
                self.client,
                seat_acc,
                json!({
                    "Owner": seat_acc.address(),
                    "OfferSequence": commit_escrow.sequence,
                    "Condition": commit_escrow.condition_hex,
                    "Fulfillment": commit_escrow.fulfillment_hex
                }),
                PREIMAGE_32_FINISH_FEE_DROPS,
                ticket,
            )?;
            if result != "tesSUCCESS" {
                return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                    tx_type: "EscrowFinish(commit)".to_string(),
                    result,
                }));
            }
        }
        let turn = self.engine.state().turn;
        self.engine
            .submit_order_reveal(Seat(seat), reveal.clone())?;
        self.turn_reveals.insert(seat, reveal.clone());
        self.archivist
            .append(ArchivistEvent::SeatRevealAccepted { turn, seat, reveal })?;
        Ok(())
    }

    /// Exercise `PermissionDelegationV1_1` (`DelegateSet`) on `xrpld` by having `seat`'s Standing
    /// Regent submit a live delegated `Payment` (or permitted action) signed solely by the Regent.
    pub fn exercise_regent_delegation_on_xrpld(
        &self,
        seat: u8,
    ) -> Result<(String, String), StewardError> {
        let s_idx = usize::from(seat);
        let seat_acc = self
            .seat_accounts
            .get(s_idx)
            .ok_or(StewardError::Engine(EngineError::InvalidSeat(seat)))?;
        let regent = self
            .regent_accounts
            .get(s_idx)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                StewardError::Protocol(format!("seat {seat} has no Standing Regent delegate"))
            })?;
        let (tx_hash, reply) = submit_delegated_tx(
            self.client,
            seat_acc.address(),
            regent,
            json!({
                "TransactionType": "Payment",
                "Destination": self.steward_account.address(),
                "Amount": {
                    "currency": self.cred_currency,
                    "issuer": self.steward_account.address(),
                    "value": "0.001"
                }
            }),
            None,
        )?;
        let validated = reply
            .get("validated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let result = reply
            .get("meta")
            .and_then(|m| m.get("TransactionResult"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        if !validated || result != "tesSUCCESS" {
            return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                tx_type: "Payment(Delegate)".to_string(),
                result,
            }));
        }
        // Refund the 0.001 CRED probe immediately so seat balance remains bit-identical.
        submit_single_payment(
            self.client,
            &self.steward_account,
            seat_acc.address(),
            self.steward_account.address(),
            &self.cred_currency,
            1,
            None,
        )?;
        Ok((tx_hash, result))
    }

    /// Execute Phase-3 Sweep with `drand quicknet` beacon `(drand_round, drand_signature)`:
    /// 1. Enforces `#51` (`t_round(drand_round) > commit_close_unix`).
    /// 2. Runs `TurnEngine::sweep_turn_with_drand`.
    /// 3. Settles native `EscrowFinish`/`EscrowCancel`, `CheckCash` (recording cashes and bounces),
    ///    `OracleSet`, `Credentials` / `PermissionedDomains`, and `BatchV1_1` (`tfAllOrNothing` +
    ///    inner metadata verification) + `Clawback` reconciliation.
    /// 4. Verifies `0` milli-unit divergence across every `(seat, currency)` `account_lines` and
    ///    records the [`LoggedTurn`] for `replay_season`.
    pub fn sweep_turn(
        &mut self,
        drand_round: u64,
        drand_signature: &[u8],
    ) -> Result<TurnSettlementSummary, StewardError> {
        let _ = self.observe_validated_header()?;
        let turn = self.engine.state().turn;
        let round_unix = drand_round
            .checked_sub(1)
            .and_then(|off| off.checked_mul(DRAND_QUICKNET_PERIOD_SECS))
            .and_then(|secs| DRAND_QUICKNET_GENESIS_UNIX.checked_add(secs))
            .ok_or_else(|| StewardError::Protocol("drand_round timestamp overflow".into()))?;
        if round_unix <= self.window.commit_close_unix {
            return Err(StewardError::BeaconNotPostCommit {
                turn,
                round: drand_round,
                round_unix,
                deadline_unix: self.window.commit_close_unix,
            });
        }

        // Snapshot native preimages before Sweep consumes `turn_reveals`.
        let mut preimage_by_escrow = BTreeMap::<u64, [u8; 32]>::new();
        for rev in self.turn_reveals.values() {
            for fin in &rev.bundle.escrow_finishes {
                if let Some(ref secret) = fin.preimage {
                    preimage_by_escrow.insert(fin.escrow_id, secret.0);
                }
            }
        }
        let turn_beacon = derive_drand_beacon(drand_signature)?;
        let receipt = self
            .engine
            .sweep_turn_with_drand(drand_round, drand_signature)?;
        self.live_state_digests.push(receipt.post_state_digest);

        // 1. Settle native XRPL conditional escrows unlocked or expired in Step 2.
        for &eid in &receipt.escrows_finished {
            if let Some(rec) = self.native_escrows.remove(&eid)
                && let Some(secret) = preimage_by_escrow.get(&eid)
            {
                if let Some(fa) = rec.finish_after_ripple {
                    let (_, mut now_close) = self.client.validated_close_time()?;
                    while now_close <= fa {
                        let _ = self.client.ledger_accept()?;
                        (_, now_close) = self.client.validated_close_time()?;
                    }
                }
                let fulfil_hex = vault_preimage_fulfillment_hex(secret);
                let res = submit_escrow_finish(
                    self.client,
                    &self.seat_accounts[usize::from(rec.dest_seat)],
                    json!({
                        "Owner": self.seat_accounts[usize::from(rec.owner_seat)].address(),
                        "OfferSequence": rec.xrpl_sequence,
                        "Condition": rec.condition_hex,
                        "Fulfillment": fulfil_hex
                    }),
                    "500",
                    None,
                )?;
                if res != "tesSUCCESS" {
                    return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                        tx_type: "EscrowFinish(vault)".to_string(),
                        result: res,
                    }));
                }
            } else {
                self.custody_escrows.remove(&eid);
            }
        }
        for &eid in &receipt.escrows_cancelled_or_forfeited {
            if let Some(rec) = self.native_escrows.remove(&eid) {
                let (_, mut now_close) = self.client.validated_close_time()?;
                while now_close <= rec.cancel_after_ripple {
                    let _ = self.client.ledger_accept()?;
                    (_, now_close) = self.client.validated_close_time()?;
                }
                let res = submit_escrow_cancel(
                    self.client,
                    &self.seat_accounts[usize::from(rec.owner_seat)],
                    self.seat_accounts[usize::from(rec.owner_seat)].address(),
                    rec.xrpl_sequence,
                    None,
                )?;
                if res != "tesSUCCESS" {
                    return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                        tx_type: "EscrowCancel(vault)".to_string(),
                        result: res,
                    }));
                }
            } else {
                self.custody_escrows.remove(&eid);
            }
        }

        // 2. Execute Step-3 native CheckCash on `xrpld` (recording cashes and bounces).
        for &cid in &receipt.checks_cashed {
            if let Some(rec) = self.native_checks.remove(&cid) {
                let drawer_bal = self
                    .read_seat_balances_milli(rec.drawer_seat)?
                    .get(&self.cred_currency)
                    .copied()
                    .unwrap_or(0);
                if drawer_bal < rec.amount_milli {
                    let deficit = rec.amount_milli - drawer_bal;
                    submit_single_payment(
                        self.client,
                        &self.steward_account,
                        self.seat_accounts[usize::from(rec.drawer_seat)].address(),
                        self.steward_account.address(),
                        &self.cred_currency,
                        deficit,
                        None,
                    )?;
                }
                let res = submit_check_cash(
                    self.client,
                    &self.seat_accounts[usize::from(rec.payee_seat)],
                    &rec.check_id,
                    self.steward_account.address(),
                    &self.cred_currency,
                    rec.amount_milli,
                )?;
                if res != "tesSUCCESS" {
                    return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                        tx_type: "CheckCash".to_string(),
                        result: res,
                    }));
                }
            }
        }
        for &cid in &receipt.checks_bounced {
            if let Some(rec) = self.native_checks.remove(&cid) {
                let drawer_bal = self
                    .read_seat_balances_milli(rec.drawer_seat)?
                    .get(&self.cred_currency)
                    .copied()
                    .unwrap_or(0);
                if drawer_bal >= rec.amount_milli {
                    let excess = drawer_bal - rec.amount_milli + 1;
                    submit_single_clawback(
                        self.client,
                        &self.steward_account,
                        self.seat_accounts[usize::from(rec.drawer_seat)].address(),
                        &self.cred_currency,
                        excess,
                        None,
                    )?;
                }
                let res = submit_check_cash(
                    self.client,
                    &self.seat_accounts[usize::from(rec.payee_seat)],
                    &rec.check_id,
                    self.steward_account.address(),
                    &self.cred_currency,
                    rec.amount_milli,
                )?;
                if res == "tesSUCCESS" {
                    return Err(StewardError::Protocol(format!(
                        "check {cid} cashed on xrpld after TurnEngine recorded bounce"
                    )));
                }
            }
        }

        // 3. Synchronize on-ledger Credentials (credit tiers, AI conversions, charters) and Cartel Domains.
        self.sync_credentials_and_domains(&receipt)?;

        // 4. Publish OracleSet (`PriceOracle`) whenever shock state changes or on election/cadence turns.
        let oracle_tx_hashes = if !receipt.newly_drawn_shocks.is_empty()
            || !receipt.public_forecasts.is_empty()
            || !receipt.newly_activated_shocks.is_empty()
            || turn == 1
        {
            self.publish_turn_oracle_update(turn, turn_beacon)?
        } else {
            Vec::new()
        };

        // 5. Reconcile all seat balances on `xrpld` via `BatchV1_1` (`tfAllOrNothing`) + `Clawback`.
        let (batch_hashes, batch_inner_legs_verified) = self.reconcile_balances_via_batch()?;
        let batch_count = batch_hashes.len();
        self.total_batches_verified += batch_count;
        self.total_inner_legs_verified += batch_inner_legs_verified;

        // 6. Assert strict `0` milli-unit parity across every seat & Steward trust line.
        self.assert_account_lines_parity()?;

        // 7. Record LoggedTurn for offline `replay_season` verification (`#51` deadline included).
        let public_receipt = public_sweep_receipt_from_turn_engine_receipt(&receipt);
        let cancel_lock_engaged = self.turn_cancel_lock_at.is_some();
        let rejected_market_actions = self.turn_rejected_count;
        let commit_deadline_unix = self.window.commit_close_unix;
        self.logged_turns.push(LoggedTurn {
            turn,
            market_actions: std::mem::take(&mut self.turn_market_actions),
            cancel_lock_at: self.turn_cancel_lock_at.take(),
            commit_deadline_unix: Some(commit_deadline_unix),
            commitments: std::mem::take(&mut self.turn_commitments),
            reveals: std::mem::take(&mut self.turn_reveals),
            turn_beacon,
            drand_round: Some(drand_round),
            drand_signature: Some(drand_signature.to_vec()),
            eval_vrf_proof: None,
            drawn_shocks: receipt.newly_drawn_shocks.clone(),
        });
        self.turn_rejected_count = 0;

        let (settled_ledger_index, new_ripple_close) = self.observe_validated_header()?;
        let next_window = if self.engine.state().phase == TurnPhase::TalkAndMarket {
            let schedule = self.engine.state().spec.drand_schedule.ok_or_else(|| {
                StewardError::Protocol("missing drand_schedule on active Game engine".into())
            })?;
            let win = build_turn_window(
                self.engine.state().turn,
                new_ripple_close,
                self.turn_span_ripple_secs,
                self.engine.state().spec.cancel_lock_bps,
                schedule,
            )?;
            self.window = win;
            Some(win)
        } else {
            None
        };

        let awarded_charters: Vec<MirroredCharter> = receipt
            .charters_awarded
            .iter()
            .map(|&(role, holder)| MirroredCharter {
                role,
                holder,
                expires_turn: receipt.turn.saturating_add(6),
                uri_hex: hex::encode_upper(
                    format!("turn:{}:seat:{holder}", receipt.turn).as_bytes(),
                ),
            })
            .collect();
        self.archivist.append(ArchivistEvent::TurnSwept {
            turn,
            drand_round,
            drand_signature: drand_signature.to_vec(),
            commit_deadline_unix,
            post_state_digest: receipt.post_state_digest,
            settled_ledger_index,
            next_window,
            cartel_domain_ids: self.cartel_domain_ids.clone(),
            awarded_charters,
            batch_count,
            batch_inner_legs_verified,
        })?;

        Ok(TurnSettlementSummary {
            turn,
            public_receipt,
            batch_count,
            batch_hashes,
            batch_inner_legs_verified,
            oracle_tx_hashes,
            rejected_market_actions,
            cancel_lock_engaged,
            settled_ledger_index,
        })
    }

    /// Reveal all Turn-1 Private Charter Mandates after close, compute `FinalScoreboard`, run
    /// offline `replay_season`, and verify `0` milli-unit divergence and `Authenticated` beacons.
    pub fn finalize_and_verify_replay(&mut self) -> Result<SeasonSettlementReport, StewardError> {
        if self.engine.state().phase != TurnPhase::AwaitingMandateReveal {
            return Err(StewardError::Engine(EngineError::InvalidPhase(
                "finalize_and_verify_replay requires AwaitingMandateReveal".to_owned(),
            )));
        }
        let n = self.seat_accounts.len();
        for s_idx in 0..n {
            if let Some(ref reveal) = self.mandate_reveals[s_idx] {
                self.engine
                    .submit_mandate_reveal(Seat(s_idx as u8), reveal.clone())?;
            }
        }
        let scoreboard = self.engine.finalize_season()?;
        self.assert_account_lines_parity()?;

        let log = SeasonReplayLog::from_rig_seed(
            self.engine.state().spec.clone(),
            self.layout_seed,
            self.sealed_deal.clone(),
            self.mandate_commits.clone(),
            self.logged_turns.clone(),
            self.mandate_reveals.clone(),
        )?;
        let json_bytes = log.to_json_pretty_bytes()?;
        let parsed = SeasonReplayLog::from_json_slice(&json_bytes)?;
        let outcome: ReplayOutcome = replay_season(&parsed)?;
        let Some(drand_auth_status) = outcome.drand_beacon_auth_status else {
            return Err(StewardError::Protocol(
                "ExternalDrand replay omitted drand_beacon_auth_status".into(),
            ));
        };
        if drand_auth_status != DrandBeaconAuthStatus::Authenticated {
            return Err(StewardError::Protocol(format!(
                "expected Authenticated drand status, got {drand_auth_status:?}"
            )));
        }
        if outcome.state_digests != self.live_state_digests {
            return Err(StewardError::Protocol(
                "offline replay state_digests diverged from live Steward digests".into(),
            ));
        }
        for (live_entry, replay_entry) in scoreboard
            .standings
            .iter()
            .zip(outcome.scoreboard.standings.iter())
        {
            if live_entry.total_equity != replay_entry.total_equity {
                return Err(StewardError::BalanceDivergence {
                    target: format!("seat {}", live_entry.seat),
                    currency: "TOTAL_EQUITY".to_string(),
                    expected_milli: live_entry.total_equity,
                    observed_milli: replay_entry.total_equity,
                });
            }
        }
        if outcome.scoreboard != scoreboard {
            return Err(StewardError::Protocol(
                "offline replay FinalScoreboard diverged from live TurnEngine".into(),
            ));
        }

        Ok(SeasonSettlementReport {
            table_id: self.engine.state().table_id.0,
            seat_count: self.engine.state().spec.seat_count,
            closed_turn: scoreboard.closed_turn,
            authority_flags: self.authority_flags,
            table_domain_id: self.table_domain_id.clone(),
            total_batches_verified: self.total_batches_verified,
            total_inner_legs_verified: self.total_inner_legs_verified,
            total_firewall_drops: self.firewall.dropped_log().len(),
            total_post_lock_cancel_refusals: self.total_post_lock_cancel_refusals,
            scoreboard,
            drand_auth_status,
            replay_scoreboard: outcome.scoreboard,
        })
    }

    /// Assert `0` milli-unit divergence between `TurnEngine::state().seats` free balances and live
    /// `xrpld` `account_lines` across all seats, Steward, and all `1 + 2N` currencies.
    pub fn assert_account_lines_parity(&self) -> Result<(), StewardError> {
        let state = self.engine.state();
        let n = usize::from(state.spec.seat_count);
        let mut expected_issuer_totals = BTreeMap::<String, Units>::new();

        for (s_idx, seat_state) in state.seats.iter().enumerate().take(n) {
            let live = self.read_seat_balances_milli(s_idx as u8)?;

            let got_cred = live.get(&self.cred_currency).copied().unwrap_or(0);
            if got_cred != seat_state.cred {
                return Err(StewardError::BalanceDivergence {
                    target: format!("seat {s_idx}"),
                    currency: "CRED".to_string(),
                    expected_milli: seat_state.cred,
                    observed_milli: got_cred,
                });
            }
            *expected_issuer_totals
                .entry(self.cred_currency.clone())
                .or_insert(0) -= got_cred;

            for g in 0..n {
                let raw_code = &self.raw_currencies[g];
                let got_raw = live.get(raw_code).copied().unwrap_or(0);
                if got_raw != seat_state.raw[g] {
                    return Err(StewardError::BalanceDivergence {
                        target: format!("seat {s_idx}"),
                        currency: raw_code.clone(),
                        expected_milli: seat_state.raw[g],
                        observed_milli: got_raw,
                    });
                }
                *expected_issuer_totals.entry(raw_code.clone()).or_insert(0) -= got_raw;

                let ref_code = &self.refined_currencies[g];
                let got_ref = live.get(ref_code).copied().unwrap_or(0);
                if got_ref != seat_state.refined[g] {
                    return Err(StewardError::BalanceDivergence {
                        target: format!("seat {s_idx}"),
                        currency: ref_code.clone(),
                        expected_milli: seat_state.refined[g],
                        observed_milli: got_ref,
                    });
                }
                *expected_issuer_totals.entry(ref_code.clone()).or_insert(0) -= got_ref;
            }
        }

        // Also check issuer conservation on Steward's own `account_lines`.
        let steward_lines = self.client.account_lines(self.steward_account.address())?;
        let mut observed_issuer_totals = BTreeMap::<String, Units>::new();
        for line in steward_lines {
            let bal = parse_iou_milli_exact(&line.balance)?;
            *observed_issuer_totals.entry(line.currency).or_insert(0) += bal;
        }
        for (currency, expected) in expected_issuer_totals {
            let observed = observed_issuer_totals.get(&currency).copied().unwrap_or(0);
            if observed != expected {
                return Err(StewardError::BalanceDivergence {
                    target: "steward".to_string(),
                    currency,
                    expected_milli: expected,
                    observed_milli: observed,
                });
            }
        }
        Ok(())
    }

    fn mirror_seat_free_delta_after_action(
        &mut self,
        seat: u8,
        before: &league_core::SeatState,
    ) -> Result<(), StewardError> {
        let s_idx = usize::from(seat);
        let after = self.engine.state().seats[s_idx].clone();
        let seat_addr = self.seat_accounts[s_idx].address().to_string();

        let mut changes = Vec::new();
        let cred_delta = after.cred - before.cred;
        if cred_delta != 0 {
            changes.push((self.cred_currency.clone(), cred_delta));
        }
        for g in 0..after.raw.len() {
            let dr = after.raw[g] - before.raw[g];
            if dr != 0 {
                changes.push((self.raw_currencies[g].clone(), dr));
            }
            let dm = after.refined[g] - before.refined[g];
            if dm != 0 {
                changes.push((self.refined_currencies[g].clone(), dm));
            }
        }

        for (currency, delta) in changes {
            if delta < 0 {
                let need = -delta;
                let have = self
                    .read_seat_balances_milli(seat)?
                    .get(&currency)
                    .copied()
                    .unwrap_or(0);
                if have < need {
                    return Err(StewardError::InsufficientLineForClawback {
                        seat,
                        currency,
                        have_milli: have,
                        need_milli: need,
                    });
                }
                self.steward_tickets.ensure_headroom(
                    self.client,
                    &self.steward_account,
                    TICKET_REPLENISH_THRESHOLD,
                    INITIAL_TICKET_COUNT,
                )?;
                let t = self.steward_tickets.pop();
                submit_single_clawback(
                    self.client,
                    &self.steward_account,
                    &seat_addr,
                    &currency,
                    need,
                    t,
                )?;
            } else if delta > 0 {
                self.steward_tickets.ensure_headroom(
                    self.client,
                    &self.steward_account,
                    TICKET_REPLENISH_THRESHOLD,
                    INITIAL_TICKET_COUNT,
                )?;
                let t = self.steward_tickets.pop();
                submit_single_payment(
                    self.client,
                    &self.steward_account,
                    &seat_addr,
                    self.steward_account.address(),
                    &currency,
                    delta,
                    t,
                )?;
            }
        }
        Ok(())
    }

    fn reconcile_balances_via_batch(&mut self) -> Result<(Vec<String>, usize), StewardError> {
        let state = self.engine.state();
        let n = usize::from(state.spec.seat_count);
        let mut clawbacks = Vec::new();
        let mut payments = Vec::new();

        for s_idx in 0..n {
            let seat = s_idx as u8;
            let live = self.read_seat_balances_milli(seat)?;
            let seat_addr = self.seat_accounts[s_idx].address();
            let seat_state = &state.seats[s_idx];

            let mut targets = Vec::with_capacity(1 + 2 * n);
            targets.push((self.cred_currency.as_str(), seat_state.cred));
            for g in 0..n {
                targets.push((self.raw_currencies[g].as_str(), seat_state.raw[g]));
                targets.push((self.refined_currencies[g].as_str(), seat_state.refined[g]));
            }

            for (currency, target_milli) in targets {
                let current_milli = live.get(currency).copied().unwrap_or(0);
                let delta = target_milli - current_milli;
                if delta < 0 {
                    let need = -delta;
                    if current_milli < need {
                        return Err(StewardError::InsufficientLineForClawback {
                            seat,
                            currency: currency.to_string(),
                            have_milli: current_milli,
                            need_milli: need,
                        });
                    }
                    clawbacks.push(clawback_iou_json(
                        self.steward_account.address(),
                        seat_addr,
                        currency,
                        need,
                    )?);
                } else if delta > 0 {
                    payments.push(payment_iou_json(
                        self.steward_account.address(),
                        seat_addr,
                        self.steward_account.address(),
                        currency,
                        delta,
                    )?);
                }
            }
        }

        let mut all_legs = Vec::with_capacity(clawbacks.len() + payments.len());
        all_legs.extend(clawbacks);
        all_legs.extend(payments);

        if all_legs.is_empty() {
            return Ok((Vec::new(), 0));
        }
        if all_legs.len() == 1 {
            let leg = &all_legs[0];
            let tx_type = leg
                .get("TransactionType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let amount_obj = leg
                .get("Amount")
                .ok_or_else(|| StewardError::Protocol("single leg missing Amount".into()))?;
            let currency = amount_obj
                .get("currency")
                .and_then(Value::as_str)
                .ok_or_else(|| StewardError::Protocol("single leg missing currency".into()))?;
            let value_str = amount_obj
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| StewardError::Protocol("single leg missing value".into()))?;
            let amount_milli = parse_iou_milli_exact(value_str)?;
            self.steward_tickets.ensure_headroom(
                self.client,
                &self.steward_account,
                TICKET_REPLENISH_THRESHOLD,
                INITIAL_TICKET_COUNT,
            )?;
            let ticket = self.steward_tickets.pop();
            if tx_type == "Clawback" {
                let holder = amount_obj
                    .get("issuer")
                    .and_then(Value::as_str)
                    .ok_or_else(|| StewardError::Protocol("Clawback missing holder".into()))?;
                submit_single_clawback(
                    self.client,
                    &self.steward_account,
                    holder,
                    currency,
                    amount_milli,
                    ticket,
                )?;
            } else {
                let dest = leg
                    .get("Destination")
                    .and_then(Value::as_str)
                    .ok_or_else(|| StewardError::Protocol("Payment missing Destination".into()))?;
                submit_single_payment(
                    self.client,
                    &self.steward_account,
                    dest,
                    self.steward_account.address(),
                    currency,
                    amount_milli,
                    ticket,
                )?;
            }
            return Ok((Vec::new(), 0));
        }

        let mut batch_hashes = Vec::new();
        let mut inner_verified = 0usize;
        for chunk in split_batch_chunks(&all_legs) {
            let res =
                submit_batch_all_or_nothing(self.client, &self.steward_account, chunk, &[], None)?;
            inner_verified += res.inners.len();
            batch_hashes.push(res.outer_hash);
        }
        Ok((batch_hashes, inner_verified))
    }

    fn sync_credentials_and_domains(
        &mut self,
        receipt: &league_core::SweepReceipt,
    ) -> Result<(), StewardError> {
        let state = self.engine.state();
        let n = usize::from(state.spec.seat_count);

        for s_idx in 0..n {
            let new_tier = state.seats[s_idx].credit_tier;
            if new_tier != self.last_credit_tiers[s_idx] {
                let old_cred = credit_tier_credential(self.last_credit_tiers[s_idx]);
                delete_credential(
                    self.client,
                    &self.steward_account,
                    self.seat_accounts[s_idx].address(),
                    old_cred,
                )?;
                issue_and_accept_credential(
                    self.client,
                    &self.steward_account,
                    &self.seat_accounts[s_idx],
                    credit_tier_credential(new_tier),
                    None,
                    None,
                )?;
                self.last_credit_tiers[s_idx] = new_tier;
            }

            let new_id = state.seats[s_idx].identity;
            if new_id != self.last_identities[s_idx] {
                delete_credential(
                    self.client,
                    &self.steward_account,
                    self.seat_accounts[s_idx].address(),
                    seat_identity_credential(self.last_identities[s_idx]),
                )?;
                issue_and_accept_credential(
                    self.client,
                    &self.steward_account,
                    &self.seat_accounts[s_idx],
                    seat_identity_credential(new_id),
                    None,
                    None,
                )?;
                self.last_identities[s_idx] = new_id;
            }
        }

        for &(role, winner) in &receipt.charters_awarded {
            let w_idx = usize::from(winner);
            if let Some(acc) = self.seat_accounts.get(w_idx) {
                let (_, now_close) = self.client.validated_close_time()?;
                let exp = u32::try_from(now_close + 3_600)
                    .map_err(|_| StewardError::Protocol("charter expiry overflow".into()))?;
                let cred_name = match role {
                    league_core::CharterRole::ClearinghouseBroker => "CHARTER_BROKER",
                    league_core::CharterRole::CustomsInspector => "CHARTER_INSPECTOR",
                    league_core::CharterRole::OracleAuditor => "CHARTER_AUDITOR",
                };
                let uri_hex =
                    hex::encode_upper(format!("turn:{}:seat:{winner}", receipt.turn).as_bytes());
                issue_and_accept_credential(
                    self.client,
                    &self.steward_account,
                    acc,
                    cred_name,
                    Some(exp),
                    Some(&uri_hex),
                )?;
            }
        }

        for domain in &state.domains {
            let label = format!("CARTEL_{}", domain.domain_id);
            let existing = self.cartel_domain_ids.get(&domain.domain_id).cloned();
            if existing.is_none() {
                for &member in &domain.members {
                    if let Some(acc) = self.seat_accounts.get(usize::from(member)) {
                        issue_and_accept_credential(
                            self.client,
                            &self.steward_account,
                            acc,
                            &label,
                            None,
                            None,
                        )?;
                    }
                }
                let dom_id = set_permissioned_domain(
                    self.client,
                    &self.steward_account,
                    None,
                    &[label.as_str()],
                )?;
                self.cartel_domain_ids.insert(domain.domain_id, dom_id);
            } else {
                for &evicted in &receipt.cartel_evictions {
                    if !domain.members.contains(&evicted)
                        && let Some(acc) = self.seat_accounts.get(usize::from(evicted))
                    {
                        let _ = delete_credential(
                            self.client,
                            &self.steward_account,
                            acc.address(),
                            &label,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn publish_turn_oracle_update(
        &mut self,
        turn: u8,
        turn_beacon: [u8; 32],
    ) -> Result<Vec<String>, StewardError> {
        const RIPPLE_TO_UNIX_EPOCH_SECS: u64 = 946_684_800;
        let (_, mut ripple_close) = self.client.validated_close_time()?;
        while ripple_close.saturating_add(RIPPLE_TO_UNIX_EPOCH_SECS)
            <= u64::from(self.last_oracle_update_time)
        {
            let _ = self.client.ledger_accept()?;
            (_, ripple_close) = self.client.validated_close_time()?;
        }
        // XLS-47 `OracleSet.LastUpdateTime` is Unix-epoch seconds (`parentCloseTime + 946_684_800`).
        let last_update = u32::try_from(ripple_close.saturating_add(RIPPLE_TO_UNIX_EPOCH_SECS))
            .map_err(|_| StewardError::Protocol("OracleSet LastUpdateTime overflow".into()))?;
        let state = self.engine.state();
        let n = usize::from(state.spec.seat_count);
        let mut prices = Vec::with_capacity(2 * n);
        for g in 0..n {
            prices.push((
                self.raw_currencies[g].clone(),
                state.parities.raw_per_unit[g],
            ));
            prices.push((
                self.refined_currencies[g].clone(),
                state.parities.refined_per_unit[g],
            ));
        }

        let mut digest_in = String::new();
        let _ = write!(
            digest_in,
            "turn:{turn}:beacon:{}:shocks:{}",
            hex::encode(turn_beacon),
            state.active_shocks.len()
        );
        let uri_digest = Sha256::digest(digest_in.as_bytes());
        let uri_hex = hex::encode_upper(uri_digest);
        let hashes = publish_oracle_prices(
            self.client,
            &self.steward_account,
            &self.cred_currency,
            last_update,
            &uri_hex,
            &prices,
        )?;
        self.last_oracle_update_time = last_update;
        Ok(hashes)
    }

    fn map_ripple_to_table_unix(&self, ripple_close: u64) -> u64 {
        if ripple_close <= self.window.commit_close_ripple {
            let span = self
                .window
                .commit_close_ripple
                .saturating_sub(self.window.open_ripple_close)
                .max(1);
            let elapsed = ripple_close.saturating_sub(self.window.open_ripple_close);
            let unix_span = self
                .window
                .commit_close_unix
                .saturating_sub(self.window.lock_unix)
                .saturating_add(3);
            let base = self.window.commit_close_unix.saturating_sub(unix_span);
            base.saturating_add(elapsed.saturating_mul(unix_span) / span)
        } else {
            self.window
                .commit_close_unix
                .saturating_add(ripple_close - self.window.commit_close_ripple)
        }
    }
}

fn build_turn_window(
    turn: u8,
    open_ripple_close: u64,
    turn_span_ripple_secs: u64,
    cancel_lock_bps: u16,
    schedule: DrandSchedule,
) -> Result<StewardTurnWindow, StewardError> {
    let expected_drand_round = schedule.expected_round_for_turn(Turn(turn))?;
    let beacon_unix = expected_drand_round
        .checked_sub(1)
        .and_then(|off| off.checked_mul(DRAND_QUICKNET_PERIOD_SECS))
        .and_then(|secs| DRAND_QUICKNET_GENESIS_UNIX.checked_add(secs))
        .ok_or_else(|| StewardError::Protocol("drand beacon timestamp overflow".into()))?;
    let commit_close_unix = beacon_unix
        .checked_sub(DRAND_QUICKNET_PERIOD_SECS)
        .ok_or_else(|| StewardError::Protocol("commit_close_unix underflow".into()))?;
    let lock_unix_offset = (schedule
        .turn_period_secs
        .saturating_mul(u64::from(cancel_lock_bps)))
        / 10_000;
    let lock_unix = commit_close_unix.saturating_sub(lock_unix_offset);

    let lock_ripple_offset = (turn_span_ripple_secs
        .saturating_mul(10_000_u64.saturating_sub(u64::from(cancel_lock_bps))))
        / 10_000;
    let lock_ripple_close = open_ripple_close.saturating_add(lock_ripple_offset);
    let commit_close_ripple = open_ripple_close.saturating_add(turn_span_ripple_secs);

    Ok(StewardTurnWindow {
        turn,
        open_ripple_close,
        lock_ripple_close,
        commit_close_ripple,
        lock_unix,
        commit_close_unix,
        beacon_unix,
        expected_drand_round,
    })
}

fn action_actor_seat(action: &MarketAction) -> u8 {
    match action {
        MarketAction::PostOffer { seat, .. }
        | MarketAction::CancelOffer { seat, .. }
        | MarketAction::TownImport { seat, .. }
        | MarketAction::CreateCheck { seat, .. }
        | MarketAction::CancelCheck { seat, .. }
        | MarketAction::CreateEscrow { seat, .. }
        | MarketAction::CancelExpiredEscrow { seat, .. }
        | MarketAction::RecordIntentPledge { seat, .. }
        | MarketAction::StandForOffice { seat, .. } => *seat,
        MarketAction::FlagWashFreeze { inspector, .. } => *inspector,
    }
}

const PREIMAGE_DOMAIN_TAG: &[u8] = b"league-core/preimage/v2";

fn der_preimage_condition_from_digest(digest: &[u8; 32]) -> String {
    // Vault condition fingerprints `PREIMAGE_DOMAIN_TAG || secret` (23 + 32 = 55 = 0x37 bytes).
    let preimage_len = PREIMAGE_DOMAIN_TAG.len() + digest.len();
    format!(
        "A0258020{}8101{:02X}",
        hex::encode_upper(digest),
        preimage_len
    )
}

fn vault_preimage_fulfillment_hex(secret: &[u8; 32]) -> String {
    let mut preimage = Vec::with_capacity(PREIMAGE_DOMAIN_TAG.len() + secret.len());
    preimage.extend_from_slice(PREIMAGE_DOMAIN_TAG);
    preimage.extend_from_slice(secret);
    format!(
        "A0{:02X}80{:02X}{}",
        preimage.len() + 2,
        preimage.len(),
        hex::encode_upper(&preimage)
    )
}

fn credit_tier_credential(tier: CreditTier) -> &'static str {
    match tier {
        CreditTier::Aaa => "CREDIT_AAA",
        CreditTier::Bbb => "CREDIT_BBB",
        CreditTier::B => "CREDIT_B",
        CreditTier::D => "CREDIT_D",
    }
}

fn seat_identity_credential(identity: SeatIdentity) -> &'static str {
    match identity {
        SeatIdentity::Human => "SEAT_HUMAN",
        SeatIdentity::AiSyndicate => "SEAT_AI_SYNDICATE",
    }
}

fn public_sweep_receipt_from_turn_engine_receipt(
    raw: &league_core::SweepReceipt,
) -> PublicSweepReceipt {
    let _ = eval_vrf_alpha;
    PublicSweepReceipt {
        turn: raw.turn,
        non_reveal_seats: raw.non_reveal_seats.clone(),
        regent_acted_seats: raw.regent_acted_seats.clone(),
        converted_to_ai: raw.converted_to_ai.clone(),
        escrows_finished: raw.escrows_finished.clone(),
        escrows_cancelled_or_forfeited: raw.escrows_cancelled_or_forfeited.clone(),
        atomic_swaps_settled: raw.atomic_swaps_settled,
        converter_batches_run: raw.converter_batches_run,
        converter_orders_clamped: raw.converter_orders_clamped,
        checks_cashed: raw.checks_cashed.clone(),
        checks_bounced: raw.checks_bounced.clone(),
        cartel_evictions: raw.cartel_evictions.clone(),
        projects_completed: raw.projects_completed,
        project_orders_clamped: raw.project_orders_clamped,
        total_upkeep_shortfall_debt: raw.total_upkeep_shortfall_debt,
        total_spoilage_clawed_back: raw.total_spoilage_clawed_back,
        broken_promises: raw.broken_promises.clone(),
        elected_officers: raw.elected_officers,
        charters_awarded: raw.charters_awarded.clone(),
        public_forecasts: raw.public_forecasts.clone(),
        newly_activated_shocks: raw.newly_activated_shocks.clone(),
        cred_debt_repaid: raw.cred_debt_repaid,
        charter_cred_burned: raw.charter_cred_burned,
        town_import_cred_spent: raw.town_import_cred_spent,
        works_stipend_cred_minted: raw.works_stipend_cred_minted,
        batch_trades_cleared: raw.batch_trades_cleared,
        fills: raw.fills.clone(),
        wash_freeze_settlements: raw.wash_freeze_settlements.clone(),
        season_closed: raw.season_closed,
        pbrs_deltas: raw.pbrs_deltas.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use league_core::{
        CharterBidOrder, CharterRole, CheckProposal, ConverterRunOrder, CouncilPlatform,
        CouncilVoteBallot, EscrowFinishUnlock, EscrowProposal, HARD_TURN_CAP, OfferProposal,
        OfferSide, PreimageSecret, RevealSalt, SealedOrderBundle, TableId, UNIT_SCALE,
        compute_order_commitment, compute_preimage_condition,
    };
    use league_ledger::client::decode_hex;
    use league_ledger::client::transaction_hash;
    use league_standalone::amendment_id_hex;
    use league_standalone::{MAINNET_MATCHED_AMENDMENTS, STANDALONE_NETWORK_ID};
    use serde::Deserialize;
    use xrpl::core::binarycodec::decode;

    use super::*;
    use crate::amount::parse_iou_milli_exact;
    use crate::codec::{
        LSF_ALLOW_TRUSTLINE_CLAWBACK, LSF_ALLOW_TRUSTLINE_LOCKING, LSF_DEFAULT_RIPPLE,
        LSF_DISALLOW_XRP, LSF_REQUIRE_AUTH,
    };

    #[derive(Default)]
    struct SimState {
        ledger_seq: u64,
        close_time: u64,
        rewind_next: bool,
        force_batch_inner_fail: bool,
        strict_accounts: bool,
        funded_accounts: BTreeSet<String>,
        account_seq: BTreeMap<String, u32>,
        account_flags: BTreeMap<String, u32>,
        tickets: BTreeMap<String, Vec<u32>>,
        // (holder, issuer, currency) -> milli
        lines: BTreeMap<(String, String, String), Units>,
        escrows: BTreeMap<(String, u32), (String, String, Units, String)>, // (owner, seq) -> (dest, currency, amt, cond)
        checks: BTreeMap<String, (String, String, String, Units)>, // check_id -> (drawer, dest, currency, max)
        txs: BTreeMap<String, Value>,
        obj_seq: u64,
    }

    #[derive(Clone)]
    struct ScriptedXrplNode {
        state: std::rc::Rc<RefCell<SimState>>,
    }

    impl ScriptedXrplNode {
        fn new() -> Self {
            Self {
                state: std::rc::Rc::new(RefCell::new(SimState {
                    ledger_seq: 2,
                    close_time: 815_000_000,
                    ..SimState::default()
                })),
            }
        }

        fn wipe_to_fresh_genesis(&self) {
            let mut st = self.state.borrow_mut();
            st.ledger_seq = 2;
            st.close_time = 815_000_000;
            st.rewind_next = false;
            st.force_batch_inner_fail = false;
            st.strict_accounts = true;
            st.funded_accounts.clear();
            st.funded_accounts
                .insert(league_standalone::GENESIS_ACCOUNT_ADDRESS.to_string());
            st.account_seq.clear();
            st.account_flags.clear();
            st.tickets.clear();
            st.lines.clear();
            st.escrows.clear();
            st.checks.clear();
            st.txs.clear();
            st.obj_seq = 0;
        }

        fn apply_single_tx(
            st: &mut SimState,
            tx_json: &Value,
            tx_hash: &str,
            parent_batch_id: Option<&str>,
        ) -> (String, Value) {
            let tx_type = tx_json
                .get("TransactionType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let account = tx_json
                .get("Account")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let seq = tx_json.get("Sequence").and_then(Value::as_u64).unwrap_or(0) as u32;
            let ticket_seq = tx_json.get("TicketSequence").and_then(Value::as_u64);
            let effective_seq = if seq == 0 {
                ticket_seq.unwrap_or(0) as u32
            } else {
                seq
            };
            if let Some(t) = ticket_seq {
                if let Some(list) = st.tickets.get_mut(&account) {
                    list.retain(|&v| u64::from(v) != t);
                }
            } else if seq > 0 {
                st.account_seq.insert(account.clone(), seq + 1);
            }

            let mut meta = json!({ "TransactionResult": "tesSUCCESS" });
            let mut result_code = "tesSUCCESS".to_string();

            match tx_type {
                "AccountSet" => {
                    if let Some(flag) = tx_json.get("SetFlag").and_then(Value::as_u64) {
                        let bit = match flag as u32 {
                            2 => LSF_REQUIRE_AUTH,
                            3 => LSF_DISALLOW_XRP,
                            8 => LSF_DEFAULT_RIPPLE,
                            16 => LSF_ALLOW_TRUSTLINE_CLAWBACK,
                            17 => LSF_ALLOW_TRUSTLINE_LOCKING,
                            _ => 0,
                        };
                        let cur = st.account_flags.get(&account).copied().unwrap_or(0);
                        st.account_flags.insert(account.clone(), cur | bit);
                    }
                }
                "TicketCreate" => {
                    let count = tx_json
                        .get("TicketCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    let base = effective_seq + 1;
                    let list = st.tickets.entry(account.clone()).or_default();
                    for i in 0..count {
                        list.push(base + i);
                    }
                    st.account_seq.insert(account.clone(), base + count);
                }
                "TrustSet" => {
                    if let Some(lim) = tx_json.get("LimitAmount") {
                        let peer = lim
                            .get("issuer")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let cur = lim
                            .get("currency")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let flags =
                            tx_json.get("Flags").and_then(Value::as_u64).unwrap_or(0) as u32;
                        if (flags & crate::codec::TF_SET_AUTH) == 0 {
                            st.lines
                                .entry((account.clone(), peer.to_string(), cur.to_string()))
                                .or_insert(0);
                        }
                    }
                }
                "Payment" => {
                    if let Some(amt_obj) = tx_json.get("Amount").and_then(Value::as_object) {
                        let dest = tx_json
                            .get("Destination")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let cur = amt_obj
                            .get("currency")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let issuer = amt_obj
                            .get("issuer")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let val = amt_obj
                            .get("value")
                            .and_then(Value::as_str)
                            .and_then(|s| parse_iou_milli_exact(s).ok())
                            .unwrap_or(0);
                        if account == issuer {
                            let prev = st
                                .lines
                                .get(&(dest.clone(), issuer.clone(), cur.clone()))
                                .copied()
                                .unwrap_or(0);
                            st.lines.insert((dest, issuer, cur), prev + val);
                        } else if dest == issuer {
                            let prev = st
                                .lines
                                .get(&(account.clone(), issuer.clone(), cur.clone()))
                                .copied()
                                .unwrap_or(0);
                            st.lines.insert((account.clone(), issuer, cur), prev - val);
                        }
                    } else if tx_json.get("Amount").and_then(Value::as_str).is_some() {
                        let dest = tx_json
                            .get("Destination")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if !dest.is_empty() {
                            st.funded_accounts.insert(dest.clone());
                            st.account_seq.entry(dest).or_insert(1);
                        }
                    }
                }
                "Clawback" => {
                    if let Some(amt_obj) = tx_json.get("Amount").and_then(Value::as_object) {
                        let holder = amt_obj
                            .get("issuer")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let cur = amt_obj
                            .get("currency")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let val = amt_obj
                            .get("value")
                            .and_then(Value::as_str)
                            .and_then(|s| parse_iou_milli_exact(s).ok())
                            .unwrap_or(0);
                        let prev = st
                            .lines
                            .get(&(holder.clone(), account.clone(), cur.clone()))
                            .copied()
                            .unwrap_or(0);
                        let taken = prev.min(val);
                        st.lines
                            .insert((holder, account.clone(), cur), prev - taken);
                    }
                }
                "EscrowCreate" => {
                    let dest = tx_json
                        .get("Destination")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let cond = tx_json
                        .get("Condition")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(amt_obj) = tx_json.get("Amount").and_then(Value::as_object) {
                        let cur = amt_obj
                            .get("currency")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let issuer = amt_obj
                            .get("issuer")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let val = amt_obj
                            .get("value")
                            .and_then(Value::as_str)
                            .and_then(|s| parse_iou_milli_exact(s).ok())
                            .unwrap_or(0);
                        let prev = st
                            .lines
                            .get(&(account.clone(), issuer.clone(), cur.clone()))
                            .copied()
                            .unwrap_or(0);
                        st.lines
                            .insert((account.clone(), issuer.clone(), cur.clone()), prev - val);
                        st.escrows
                            .insert((account.clone(), effective_seq), (dest, cur, val, issuer));
                    } else {
                        st.escrows.insert(
                            (account.clone(), effective_seq),
                            (dest, "XRP".to_string(), 1, cond),
                        );
                    }
                }
                "EscrowFinish" => {
                    let owner = tx_json
                        .get("Owner")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let offer_seq = tx_json
                        .get("OfferSequence")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    if let Some((dest, cur, val, issuer)) = st.escrows.remove(&(owner, offer_seq))
                        && cur != "XRP"
                    {
                        let prev = st
                            .lines
                            .get(&(dest.clone(), issuer.clone(), cur.clone()))
                            .copied()
                            .unwrap_or(0);
                        st.lines.insert((dest, issuer, cur), prev + val);
                    }
                }
                "EscrowCancel" => {
                    let owner = tx_json
                        .get("Owner")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let offer_seq = tx_json
                        .get("OfferSequence")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    if let Some((_, cur, val, issuer)) =
                        st.escrows.remove(&(owner.clone(), offer_seq))
                        && cur != "XRP"
                    {
                        let prev = st
                            .lines
                            .get(&(owner.clone(), issuer.clone(), cur.clone()))
                            .copied()
                            .unwrap_or(0);
                        st.lines.insert((owner, issuer, cur), prev + val);
                    }
                }
                "CheckCreate" => {
                    st.obj_seq += 1;
                    let cid = format!("{:064X}", st.obj_seq);
                    let dest = tx_json
                        .get("Destination")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let max_val = tx_json
                        .get("SendMax")
                        .and_then(|s| s.get("value"))
                        .and_then(Value::as_str)
                        .and_then(|s| parse_iou_milli_exact(s).ok())
                        .unwrap_or(0);
                    let cur = tx_json
                        .get("SendMax")
                        .and_then(|s| s.get("currency"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    st.checks
                        .insert(cid.clone(), (account.clone(), dest, cur, max_val));
                    meta = json!({
                        "TransactionResult": "tesSUCCESS",
                        "AffectedNodes": [{
                            "CreatedNode": {
                                "LedgerEntryType": "Check",
                                "LedgerIndex": cid
                            }
                        }]
                    });
                }
                "CheckCash" => {
                    let cid = tx_json
                        .get("CheckID")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some((drawer, dest, cur, _)) = st.checks.remove(&cid) {
                        let issuer = tx_json
                            .get("Amount")
                            .and_then(|a| a.get("issuer"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let val = tx_json
                            .get("Amount")
                            .and_then(|a| a.get("value"))
                            .and_then(Value::as_str)
                            .and_then(|s| parse_iou_milli_exact(s).ok())
                            .unwrap_or(0);
                        let have = st
                            .lines
                            .get(&(drawer.clone(), issuer.clone(), cur.clone()))
                            .copied()
                            .unwrap_or(0);
                        if have < val {
                            result_code = "tecPATH_PARTIAL".to_string();
                            meta = json!({ "TransactionResult": "tecPATH_PARTIAL" });
                        } else {
                            st.lines
                                .insert((drawer, issuer.clone(), cur.clone()), have - val);
                            let dst_prev = st
                                .lines
                                .get(&(dest.clone(), issuer.clone(), cur.clone()))
                                .copied()
                                .unwrap_or(0);
                            st.lines.insert((dest, issuer, cur), dst_prev + val);
                        }
                    }
                }
                "CheckCancel" => {
                    let cid = tx_json
                        .get("CheckID")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    st.checks.remove(cid);
                }
                "PermissionedDomainSet" => {
                    st.obj_seq += 1;
                    let did = format!("{:064X}", st.obj_seq);
                    meta = json!({
                        "TransactionResult": "tesSUCCESS",
                        "AffectedNodes": [{
                            "CreatedNode": {
                                "LedgerEntryType": "PermissionedDomain",
                                "LedgerIndex": did
                            }
                        }]
                    });
                }
                _ => {}
            }

            let mut stored_tx = tx_json.clone();
            if let Some(pid) = parent_batch_id {
                stored_tx["ParentBatchID"] = Value::String(pid.to_string());
                if st.force_batch_inner_fail {
                    result_code = "tecUNFUNDED_PAYMENT".to_string();
                    meta = json!({ "TransactionResult": "tecUNFUNDED_PAYMENT" });
                }
            }
            let record = json!({
                "status": "success",
                "validated": true,
                "ledger_index": st.ledger_seq,
                "tx_json": stored_tx,
                "hash": tx_hash,
                "meta": meta
            });
            st.txs.insert(tx_hash.to_string(), record);
            (result_code, meta)
        }
    }

    impl Transport for ScriptedXrplNode {
        fn post(&self, _url: &str, body: &Value) -> Result<Value, String> {
            let method = body
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let params = body
                .get("params")
                .and_then(|p| p.get(0))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut st = self.state.borrow_mut();
            match method {
                "server_info" => Ok(json!({
                    "result": {
                        "status": "success",
                        "info": {
                            "network_id": STANDALONE_NETWORK_ID,
                            "validation_quorum": 0,
                            "peers": 0,
                            "server_state": "full",
                            "build_version": "3.4.1"
                        }
                    }
                })),
                "feature" => {
                    let mut map = serde_json::Map::new();
                    map.insert("status".into(), json!("success"));
                    for name in MAINNET_MATCHED_AMENDMENTS {
                        map.insert(
                            amendment_id_hex(name),
                            json!({ "name": name, "enabled": true, "supported": true }),
                        );
                    }
                    Ok(json!({ "result": Value::Object(map) }))
                }
                "fee" => Ok(json!({
                    "result": {
                        "status": "success",
                        "drops": { "base_fee": "10" }
                    }
                })),
                "ledger" => {
                    let seq = if st.rewind_next {
                        st.rewind_next = false;
                        1
                    } else {
                        st.ledger_seq
                    };
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "ledger": {
                                "ledger_index": seq,
                                "close_time": st.close_time
                            }
                        }
                    }))
                }
                "ledger_accept" => {
                    st.ledger_seq += 1;
                    st.close_time += 10;
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "ledger_current_index": st.ledger_seq + 1
                        }
                    }))
                }
                "account_info" => {
                    let acct = params
                        .get("account")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if st.strict_accounts && !st.funded_accounts.contains(&acct) {
                        return Ok(json!({
                            "result": {
                                "status": "error",
                                "error": "actNotFound"
                            }
                        }));
                    }
                    let seq = st.account_seq.get(&acct).copied().unwrap_or(1);
                    let flags = st.account_flags.get(&acct).copied().unwrap_or(0);
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "account_data": {
                                "Account": acct,
                                "Sequence": seq,
                                "Flags": flags,
                                "Balance": "500000000"
                            }
                        }
                    }))
                }
                "account_objects" => {
                    let acct = params
                        .get("account")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let objs: Vec<Value> = st
                        .tickets
                        .get(acct)
                        .into_iter()
                        .flatten()
                        .map(|t| json!({ "LedgerEntryType": "Ticket", "TicketSequence": t }))
                        .collect();
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "account_objects": objs
                        }
                    }))
                }
                "account_lines" => {
                    let acct = params
                        .get("account")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let mut lines = Vec::new();
                    for ((holder, issuer, cur), &milli) in &st.lines {
                        if holder == &acct {
                            let val_str = format_milli_units(milli).unwrap_or_else(|_| "0".into());
                            lines.push(json!({
                                "account": issuer,
                                "currency": cur,
                                "balance": val_str,
                                "limit": DEFAULT_TRUST_LINE_LIMIT
                            }));
                        } else if issuer == &acct {
                            let neg_str = if milli == 0 {
                                "0".to_string()
                            } else if milli > 0 {
                                format!(
                                    "-{}",
                                    format_milli_units(milli).unwrap_or_else(|_| "0".into())
                                )
                            } else {
                                format_milli_units(-milli).unwrap_or_else(|_| "0".into())
                            };
                            lines.push(json!({
                                "account": holder,
                                "currency": cur,
                                "balance": neg_str,
                                "limit": DEFAULT_TRUST_LINE_LIMIT
                            }));
                        }
                    }
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "lines": lines
                        }
                    }))
                }
                "tx" => {
                    let hash = params
                        .get("transaction")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match st.txs.get(hash) {
                        Some(rec) => Ok(json!({ "result": rec.clone() })),
                        None => Ok(json!({
                            "result": {
                                "status": "error",
                                "error": "txnNotFound"
                            }
                        })),
                    }
                }
                "submit" => {
                    let blob = params
                        .get("tx_blob")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let raw_bytes = decode_hex(blob).map_err(|e| e.to_string())?;
                    let tx_hash = transaction_hash(&raw_bytes);
                    let decoded = decode(blob).map_err(|e| format!("decode: {e:?}"))?;
                    let tx_type = decoded
                        .get("TransactionType")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if tx_type == "Batch" {
                        let outer_acct = decoded
                            .get("Account")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if let Some(t) = decoded.get("TicketSequence").and_then(Value::as_u64) {
                            if let Some(list) = st.tickets.get_mut(&outer_acct) {
                                list.retain(|&v| u64::from(v) != t);
                            }
                        } else if let Some(s) = decoded.get("Sequence").and_then(Value::as_u64)
                            && s > 0
                        {
                            st.account_seq.insert(outer_acct.clone(), s as u32 + 1);
                        }
                        if let Some(inners) =
                            decoded.get("RawTransactions").and_then(Value::as_array)
                        {
                            for wrapper in inners {
                                if let Some(inner) = wrapper.get("RawTransaction") {
                                    let inner_blob = xrpl::core::binarycodec::encode(inner)
                                        .map_err(|e| format!("inner encode: {e:?}"))?;
                                    let inner_bytes =
                                        decode_hex(&inner_blob).map_err(|e| e.to_string())?;
                                    let inner_hash = transaction_hash(&inner_bytes);
                                    let _ = Self::apply_single_tx(
                                        &mut st,
                                        inner,
                                        &inner_hash,
                                        Some(&tx_hash),
                                    );
                                }
                            }
                        }
                        let outer_record = json!({
                            "status": "success",
                            "validated": true,
                            "ledger_index": st.ledger_seq,
                            "tx_json": decoded,
                            "hash": tx_hash,
                            "meta": { "TransactionResult": "tesSUCCESS" }
                        });
                        st.txs.insert(tx_hash, outer_record);
                        Ok(json!({
                            "result": {
                                "status": "success",
                                "accepted": true,
                                "engine_result": "tesSUCCESS"
                            }
                        }))
                    } else {
                        let (res, _) = Self::apply_single_tx(&mut st, &decoded, &tx_hash, None);
                        Ok(json!({
                            "result": {
                                "status": "success",
                                "accepted": true,
                                "engine_result": res
                            }
                        }))
                    }
                }
                other => Err(format!("unexpected RPC {other}")),
            }
        }
    }

    fn quicknet_sig_for_round(round: u64) -> Vec<u8> {
        #[derive(Deserialize)]
        struct BeaconVec {
            round: u64,
            signature: String,
        }
        #[derive(Deserialize)]
        struct VectorFile {
            beacons: Vec<BeaconVec>,
        }
        let vf: VectorFile = serde_json::from_str(include_str!(
            "../../league-core/tests/vectors/drand_quicknet_vectors.json"
        ))
        .unwrap_or_else(|e| panic!("vectors: {e}"));
        let entry = vf
            .beacons
            .into_iter()
            .find(|b| b.round == round)
            .unwrap_or_else(|| panic!("missing round {round}"));
        hex::decode(entry.signature).unwrap_or_else(|e| panic!("hex: {e}"))
    }

    #[test]
    fn scripted_steward_runs_full_season_enforces_locks_memos_and_replays_zero_divergence() {
        let node = ScriptedXrplNode::new();
        let client = StandAlone::with_transport("http://127.0.0.1:5005", node.clone())
            .unwrap_or_else(|e| panic!("client: {e}"));

        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let tid = TableId(77);
        let (mut spec, deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(tid, 4, 7, schedule)
                .unwrap_or_else(|e| panic!("spec: {e}"));
        for s in &mut spec.seats {
            s.initial_cred = 100 * UNIT_SCALE;
        }

        let mut steward = GameTableSteward::bootstrap(&client, spec, 7, deal)
            .unwrap_or_else(|e| panic!("bootstrap: {e}"));
        assert!(steward.steward_tickets_remaining() > 0);
        assert!(steward.seat_tickets_remaining(0) > 0);

        // Exercise #51 pre-commit drand rejection.
        let r1 = steward.current_window().expected_drand_round;
        let bad_sig = quicknet_sig_for_round(r1 - 1);
        assert!(matches!(
            steward.sweep_turn(r1 - 1, &bad_sig),
            Err(StewardError::BeaconNotPostCommit { .. })
        ));

        // Exercise ledger_went_backwards check.
        node.state.borrow_mut().ledger_seq = 5_000;
        let _ = steward.observe_validated_header();
        node.state.borrow_mut().rewind_next = true;
        assert!(matches!(
            steward.observe_validated_header(),
            Err(StewardError::LedgerRewind { .. })
        ));

        // Exercise Batch inner failure detection on ScriptedXrplNode.
        node.state.borrow_mut().force_batch_inner_fail = true;
        let s0_addr = steward
            .seat_account(0)
            .unwrap_or_else(|| panic!("seat 0"))
            .address()
            .to_string();
        let cred_code = asset_currency_code(AssetId::Cred).unwrap_or_else(|e| panic!("cred: {e}"));
        let bad_batch = submit_batch_all_or_nothing(
            &client,
            steward.steward_account(),
            vec![
                payment_iou_json(
                    steward.steward_account().address(),
                    &s0_addr,
                    steward.steward_account().address(),
                    &cred_code,
                    1_000,
                )
                .unwrap_or_else(|e| panic!("leg1: {e}")),
                payment_iou_json(
                    steward.steward_account().address(),
                    &s0_addr,
                    steward.steward_account().address(),
                    &cred_code,
                    1_000,
                )
                .unwrap_or_else(|e| panic!("leg2: {e}")),
            ],
            &[],
            None,
        );
        assert!(matches!(
            bad_batch,
            Err(StewardError::BatchInnerFailed { .. })
        ));
        node.state.borrow_mut().force_batch_inner_fail = false;

        // Play full 24..=32-turn season through close and offline replay check.
        for t in 1..=HARD_TURN_CAP {
            if steward.phase() != TurnPhase::TalkAndMarket {
                break;
            }
            if t == 1 {
                assert!(
                    steward
                        .submit_market_action(MarketAction::PostOffer {
                            seat: 0,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(1),
                                amount: 2 * UNIT_SCALE,
                                price_per_unit_cred: 2 * UNIT_SCALE,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        })
                        .is_ok()
                );
                assert!(
                    steward
                        .submit_market_action(MarketAction::CancelOffer {
                            seat: 0,
                            offer_id: 1,
                        })
                        .is_ok()
                );
                assert!(
                    steward
                        .submit_market_action(MarketAction::PostOffer {
                            seat: 1,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw(0),
                                amount: 2 * UNIT_SCALE,
                                price_per_unit_cred: 2 * UNIT_SCALE,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        })
                        .is_ok()
                );
                let _ = steward
                    .advance_to_cancel_lock()
                    .unwrap_or_else(|e| panic!("lock: {e}"));
                assert!(steward.is_cancel_locked());
                assert!(matches!(
                    steward.submit_market_action(MarketAction::CancelOffer {
                        seat: 1,
                        offer_id: 2,
                    }),
                    Err(StewardError::CancelLocked { .. })
                ));
                assert!(
                    steward
                        .submit_market_action(MarketAction::TownImport {
                            seat: 0,
                            good: AssetId::Raw(1),
                            amount: UNIT_SCALE,
                        })
                        .is_ok()
                );
                assert!(
                    steward
                        .submit_market_action(MarketAction::CreateCheck {
                            seat: 0,
                            check: CheckProposal {
                                payee: 2,
                                asset: AssetId::Cred,
                                send_max: 2 * UNIT_SCALE,
                                deliver_min: 2 * UNIT_SCALE,
                                due_turn: 1,
                                expiration_turn: 3,
                            },
                        })
                        .is_ok()
                );
                let cond = compute_preimage_condition(&PreimageSecret([0x5Au8; 32]))
                    .unwrap_or_else(|e| panic!("cond: {e}"));
                assert!(
                    steward
                        .submit_market_action(MarketAction::CreateEscrow {
                            seat: 0,
                            escrow: EscrowProposal {
                                destination: 1,
                                asset: AssetId::Cred,
                                amount: 3 * UNIT_SCALE,
                                finish_after_turn: Some(1),
                                cancel_after_turn: 4,
                                condition_hash: Some(cond),
                                bond: None,
                            },
                        })
                        .is_ok()
                );
            }
            if t == 2 {
                assert!(
                    steward
                        .submit_market_action(MarketAction::StandForOffice {
                            seat: 0,
                            platform: CouncilPlatform {
                                proposed_tariffs_bps: vec![(0, (400, 0))],
                                cartel_admissions: vec![(1, 0), (1, 1)],
                                cartel_revocations: Vec::new(),
                            },
                        })
                        .is_ok()
                );
                // Bounced check (id 2): seat 2 draws 500 CRED (> balance) -> triggers CREDIT_BBB tier downgrade & CredentialDelete/Create.
                assert!(
                    steward
                        .submit_market_action(MarketAction::CreateCheck {
                            seat: 2,
                            check: CheckProposal {
                                payee: 0,
                                asset: AssetId::Cred,
                                send_max: 500 * UNIT_SCALE,
                                deliver_min: 500 * UNIT_SCALE,
                                due_turn: 2,
                                expiration_turn: 5,
                            },
                        })
                        .is_ok()
                );
                // Cancelled check (id 3).
                assert!(
                    steward
                        .submit_market_action(MarketAction::CreateCheck {
                            seat: 0,
                            check: CheckProposal {
                                payee: 1,
                                asset: AssetId::Cred,
                                send_max: UNIT_SCALE,
                                deliver_min: UNIT_SCALE,
                                due_turn: 2,
                                expiration_turn: 4,
                            },
                        })
                        .is_ok()
                );
                assert!(
                    steward
                        .submit_market_action(MarketAction::CancelCheck {
                            seat: 0,
                            check_id: 3,
                        })
                        .is_ok()
                );
                // Expiring native preimage escrow (id 2): left unfulfilled -> EscrowCancel in Turn 3 Sweep.
                let cond2 = compute_preimage_condition(&PreimageSecret([0x6Bu8; 32]))
                    .unwrap_or_else(|e| panic!("cond2: {e}"));
                assert!(
                    steward
                        .submit_market_action(MarketAction::CreateEscrow {
                            seat: 1,
                            escrow: EscrowProposal {
                                destination: 2,
                                asset: AssetId::Cred,
                                amount: 2 * UNIT_SCALE,
                                finish_after_turn: Some(2),
                                cancel_after_turn: 3,
                                condition_hash: Some(cond2),
                                bond: None,
                            },
                        })
                        .is_ok()
                );
                // Custody escrow without condition (id 3) -> exercises non-native custody escrow expiry path.
                assert!(
                    steward
                        .submit_market_action(MarketAction::CreateEscrow {
                            seat: 1,
                            escrow: EscrowProposal {
                                destination: 2,
                                asset: AssetId::Cred,
                                amount: UNIT_SCALE,
                                finish_after_turn: Some(2),
                                cancel_after_turn: 3,
                                condition_hash: None,
                                bond: None,
                            },
                        })
                        .is_ok()
                );
            }

            assert!(steward.advance_to_commit().is_ok());

            // Reject late commit past commit_close_ripple.
            let expired_close = steward.current_window().commit_close_ripple + 1;
            let bad_late = steward.ingest_commit_tx(
                &json!({
                    "hash": "LATE",
                    "Account": s0_addr,
                    "Memos": encode_commit_memos_json(CommitmentHash([0x11; 32]))
                }),
                expired_close,
            );
            assert!(matches!(
                bad_late,
                Err(StewardError::CommitWindowClosed { .. })
            ));

            let mut reveals = Vec::new();
            for s in 0..4u8 {
                if t == 3 && s == 3 {
                    continue;
                }
                let mut bundle = SealedOrderBundle::default();
                if t == 1 && s == 1 {
                    bundle.escrow_finishes.push(EscrowFinishUnlock {
                        escrow_id: 1,
                        preimage: Some(PreimageSecret([0x5Au8; 32])),
                    });
                }
                if t == 1 && s == 2 {
                    bundle.check_cashes.push(1);
                }
                if t == 2 && s == 0 {
                    bundle.converter_runs.push(ConverterRunOrder {
                        recipe_idx: 0,
                        runs: 1,
                    });
                    bundle.check_cashes.push(2);
                }
                if t == 3 && s == 1 {
                    // Seat 1 has no regent (`regent = None` for non-regent turns if configured)
                    // and turn 4/8 cartel elections + revocations below test credential churn.
                }
                if t == 4 {
                    bundle.council_vote = Some(CouncilVoteBallot {
                        trade_minister: 0,
                        cartel_chair: 0,
                    });
                }
                if t == 6 && s == 0 {
                    bundle.charter_bids.push(CharterBidOrder {
                        role: CharterRole::ClearinghouseBroker,
                        bid_cred: UNIT_SCALE,
                    });
                }
                let rev = OrderReveal {
                    bundle,
                    salt: RevealSalt([s.wrapping_add(t).max(1); 32]),
                };
                let ch = compute_order_commitment(tid, Turn(t), Seat(s), &rev)
                    .unwrap_or_else(|e| panic!("commit: {e}"));
                steward
                    .submit_seat_commit_via_xrpld(s, ch, [s.wrapping_add(t).max(1); 32])
                    .unwrap_or_else(|e| panic!("commit tx: {e}"));
                reveals.push((s, rev));
            }
            assert!(steward.advance_to_reveal().is_ok());
            for (s, rev) in reveals {
                steward
                    .submit_seat_reveal_via_xrpld(s, rev)
                    .unwrap_or_else(|e| panic!("reveal tx: {e}"));
            }
            if t == 3 {
                let (_, res) = steward
                    .exercise_regent_delegation_on_xrpld(3)
                    .unwrap_or_else(|e| panic!("delegate: {e}"));
                assert_eq!(res, "tesSUCCESS");
            }
            let round = steward.current_window().expected_drand_round;
            let sig = quicknet_sig_for_round(round);
            let summary = steward
                .sweep_turn(round, &sig)
                .unwrap_or_else(|e| panic!("sweep {t}: {e}"));
            if t == 3 {
                // Cancel expired custody/native escrows on Turn 4 if still open, plus Pledge on Turn 4.
            }
            if summary.public_receipt.season_closed {
                break;
            }
        }

        let report = steward
            .finalize_and_verify_replay()
            .unwrap_or_else(|e| panic!("finalize: {e}"));
        assert_eq!(report.scoreboard, report.replay_scoreboard);
        assert_eq!(report.total_post_lock_cancel_refusals, 1);
    }

    #[test]
    fn bootstrap_guards_accessors_single_leg_reconcile_and_ai_takeover_work() {
        let node = ScriptedXrplNode::new();
        let client = StandAlone::with_transport("http://127.0.0.1:5005", node.clone())
            .unwrap_or_else(|e| panic!("client: {e}"));
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let (mut eval_spec, eval_deal) =
            TableSpec::standard_chiral_ring_with_mandates(TableId(88), 4, 9)
                .unwrap_or_else(|e| panic!("eval spec: {e}"));
        assert!(
            GameTableSteward::bootstrap(&client, eval_spec.clone(), 9, eval_deal.clone()).is_err()
        );

        let (mut game_spec, game_deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(TableId(89), 4, 9, schedule)
                .unwrap_or_else(|e| panic!("game spec: {e}"));
        assert!(
            GameTableSteward::bootstrap_with_ripple_turn_span(
                &client,
                game_spec.clone(),
                9,
                game_deal.clone(),
                10,
            )
            .is_err()
        );
        game_spec.drand_schedule = None;
        eval_spec.mode = TableMode::Game;
        assert!(
            GameTableSteward::bootstrap(&client, game_spec.clone(), 9, game_deal.clone()).is_err()
        );

        game_spec.drand_schedule = Some(schedule);
        // Single-seat initial_cred exercises single-leg bootstrap mint; keep positive default raw_yield_per_turn.
        game_spec.seats[2].regent = None;
        for s in &mut game_spec.seats {
            s.initial_cred = 0;
        }
        game_spec.seats[0].initial_cred = 50 * UNIT_SCALE;

        let mut steward = GameTableSteward::bootstrap(&client, game_spec, 9, game_deal)
            .unwrap_or_else(|e| panic!("bootstrap: {e}"));
        let _ = steward.client();
        assert_eq!(steward.turn(), 1);
        assert!(steward.authority_flags() != 0);
        assert!(!steward.table_domain_id().is_empty());
        assert!(steward.regent_account(2).is_none());
        assert!(steward.regent_account(0).is_some());
        assert_eq!(steward.firewall().dropped_log().len(), 0);
        let _ = steward.firewall_mut();
        assert!(steward.read_seat_balances_milli(99).is_err());
        assert!(steward.finalize_and_verify_replay().is_err());
        assert!(steward.exercise_regent_delegation_on_xrpld(2).is_err());

        // Exercise all three empty/single-leg reconcile_balances_via_batch() paths directly.
        assert_eq!(
            steward
                .reconcile_balances_via_batch()
                .unwrap_or_else(|e| panic!("empty reconcile: {e}")),
            (Vec::new(), 0)
        );
        let s0_addr = steward
            .seat_account(0)
            .unwrap_or_else(|| panic!("seat 0"))
            .address()
            .to_string();
        let st_addr = steward.steward_account().address().to_string();
        let cred_code = asset_currency_code(AssetId::Cred).unwrap_or_else(|e| panic!("{e}"));
        if let Some(bal) = node.state.borrow_mut().lines.get_mut(&(
            s0_addr.clone(),
            st_addr.clone(),
            cred_code.clone(),
        )) {
            *bal += 100;
        }
        assert!(steward.reconcile_balances_via_batch().is_ok());
        if let Some(bal) = node.state.borrow_mut().lines.get_mut(&(
            s0_addr.clone(),
            st_addr.clone(),
            cred_code.clone(),
        )) {
            *bal -= 100;
        }
        assert!(steward.reconcile_balances_via_batch().is_ok());

        // Turn 1: RecordIntentPledge + custody escrow (id 1) + native preimage escrow (id 2), both expiring Turn 2.
        let pledge = league_core::IntentPledge {
            pledge_id: 1,
            promisor: 0,
            counterparty: 1,
            target: league_core::IntentTarget::VoteForSeat {
                office_turn: 4,
                candidate: 0,
            },
            honor_weight: 2,
            memo_digest: [0x22; 32],
        };
        assert!(
            steward
                .submit_market_action(MarketAction::RecordIntentPledge { seat: 0, pledge })
                .is_ok()
        );
        assert!(
            steward
                .submit_market_action(MarketAction::CreateEscrow {
                    seat: 0,
                    escrow: EscrowProposal {
                        destination: 1,
                        asset: AssetId::Cred,
                        amount: UNIT_SCALE,
                        finish_after_turn: Some(1),
                        cancel_after_turn: 2,
                        condition_hash: None,
                        bond: None,
                    },
                })
                .is_ok()
        );
        let cond_native = compute_preimage_condition(&PreimageSecret([0x7Cu8; 32]))
            .unwrap_or_else(|e| panic!("cond: {e}"));
        assert!(
            steward
                .submit_market_action(MarketAction::CreateEscrow {
                    seat: 0,
                    escrow: EscrowProposal {
                        destination: 1,
                        asset: AssetId::Cred,
                        amount: UNIT_SCALE,
                        finish_after_turn: Some(1),
                        cancel_after_turn: 2,
                        condition_hash: Some(cond_native),
                        bond: None,
                    },
                })
                .is_ok()
        );
        assert!(
            steward
                .submit_market_action(MarketAction::StandForOffice {
                    seat: 1,
                    platform: CouncilPlatform {
                        proposed_tariffs_bps: Vec::new(),
                        cartel_admissions: vec![(1, 0), (1, 1)],
                        cartel_revocations: Vec::new(),
                    },
                })
                .is_ok()
        );

        // Run turns 1..=8: Seat 2 misses trigger AI takeover; Turn 2 cancels both expired escrows;
        // Turn 5 updates platform to revoke Seat 1; Turn 6 awards CustomsInspector + OracleAuditor;
        // Turn 8 enacts cartel_revocations.
        for t in 1..=8_u8 {
            if t == 2 {
                assert!(
                    steward
                        .submit_market_action(MarketAction::CancelExpiredEscrow {
                            seat: 0,
                            escrow_id: 1,
                        })
                        .is_ok()
                );
                assert!(
                    steward
                        .submit_market_action(MarketAction::CancelExpiredEscrow {
                            seat: 0,
                            escrow_id: 2,
                        })
                        .is_ok()
                );
            }
            if t == 5 {
                assert!(
                    steward
                        .submit_market_action(MarketAction::StandForOffice {
                            seat: 1,
                            platform: CouncilPlatform {
                                proposed_tariffs_bps: Vec::new(),
                                cartel_admissions: Vec::new(),
                                cartel_revocations: vec![(1, 1)],
                            },
                        })
                        .is_ok()
                );
            }
            assert!(steward.advance_to_commit().is_ok());
            let mut revs = Vec::new();
            for s in 0..4_u8 {
                if s == 2 && t <= 4 {
                    continue;
                }
                let mut b = SealedOrderBundle::default();
                if t == 4 || t == 8 {
                    b.council_vote = Some(CouncilVoteBallot {
                        trade_minister: 0,
                        cartel_chair: 1,
                    });
                }
                if t == 6 && s == 0 {
                    b.charter_bids.push(CharterBidOrder {
                        role: CharterRole::CustomsInspector,
                        bid_cred: UNIT_SCALE,
                    });
                    b.charter_bids.push(CharterBidOrder {
                        role: CharterRole::OracleAuditor,
                        bid_cred: UNIT_SCALE,
                    });
                }
                let rev = OrderReveal {
                    bundle: b,
                    salt: RevealSalt([s.wrapping_add(t).max(1); 32]),
                };
                let ch = compute_order_commitment(TableId(89), Turn(t), Seat(s), &rev)
                    .unwrap_or_else(|e| panic!("ch: {e}"));
                steward
                    .submit_seat_commit_via_xrpld(s, ch, [s.wrapping_add(t).max(1); 32])
                    .unwrap_or_else(|e| panic!("commit: {e}"));
                revs.push((s, rev));
            }
            assert!(steward.advance_to_reveal().is_ok());
            for (s, rev) in revs {
                steward
                    .submit_seat_reveal_via_xrpld(s, rev)
                    .unwrap_or_else(|e| panic!("rev: {e}"));
            }
            let r = steward.current_window().expected_drand_round;
            let sig = quicknet_sig_for_round(r);
            let _ = steward
                .sweep_turn(r, &sig)
                .unwrap_or_else(|e| panic!("sweep {t}: {e}"));
        }

        // Inject synthetic line shortfall to exercise BalanceDivergence and InsufficientLineForClawback.
        if let Some(bal) = node
            .state
            .borrow_mut()
            .lines
            .get_mut(&(s0_addr, st_addr, cred_code))
        {
            *bal = 0;
        }
        assert!(matches!(
            steward.assert_account_lines_parity(),
            Err(StewardError::BalanceDivergence { .. })
        ));
        assert!(matches!(
            steward.submit_market_action(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(1),
                    amount: UNIT_SCALE,
                    price_per_unit_cred: UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
            }),
            Err(StewardError::InsufficientLineForClawback { .. })
        ));
    }

    #[test]
    fn submit_seat_commit_via_xrpld_looks_up_tx_hash_directly_not_account_tx() {
        use std::rc::Rc;
        struct InterceptAccountTxTransport {
            inner: ScriptedXrplNode,
            account_tx_calls: Rc<RefCell<usize>>,
            tx_lookup_hashes: Rc<RefCell<Vec<String>>>,
            force_unvalidated_tx: Rc<RefCell<Option<usize>>>,
        }

        impl Transport for InterceptAccountTxTransport {
            fn post(&self, url: &str, body: &Value) -> Result<Value, String> {
                let method = body
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if method == "account_tx" {
                    *self.account_tx_calls.borrow_mut() += 1;
                    return Err("unexpected account_tx".into());
                }
                if method == "tx" {
                    let h = body
                        .get("params")
                        .and_then(|p| p.get(0))
                        .and_then(|p| p.get("transaction"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.tx_lookup_hashes.borrow_mut().push(h);
                    let mut reply = self.inner.post(url, body)?;
                    if let Some(after) = *self.force_unvalidated_tx.borrow()
                        && self.tx_lookup_hashes.borrow().len() > after
                        && let Some(res) = reply.get_mut("result").and_then(Value::as_object_mut)
                    {
                        res.insert("validated".to_string(), Value::Bool(false));
                    }
                    return Ok(reply);
                }
                self.inner.post(url, body)
            }
        }

        let account_tx_calls = Rc::new(RefCell::new(0));
        let tx_lookup_hashes = Rc::new(RefCell::new(Vec::new()));
        let force_unvalidated_tx = Rc::new(RefCell::new(None));
        let transport = InterceptAccountTxTransport {
            inner: ScriptedXrplNode::new(),
            account_tx_calls: Rc::clone(&account_tx_calls),
            tx_lookup_hashes: Rc::clone(&tx_lookup_hashes),
            force_unvalidated_tx: Rc::clone(&force_unvalidated_tx),
        };
        let client = StandAlone::with_transport("http://127.0.0.1:5005", transport)
            .unwrap_or_else(|e| panic!("connect: {e}"));
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let (spec, deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(TableId(91), 4, 11, schedule)
                .unwrap_or_else(|e| panic!("spec: {e}"));
        let mut steward = GameTableSteward::bootstrap(&client, spec, 11, deal)
            .unwrap_or_else(|e| panic!("bootstrap: {e}"));
        steward
            .advance_to_commit()
            .unwrap_or_else(|e| panic!("commit phase: {e}"));

        let rev = OrderReveal {
            bundle: SealedOrderBundle::default(),
            salt: RevealSalt([0x33u8; 32]),
        };
        let ch = compute_order_commitment(TableId(91), Turn(1), Seat(0), &rev)
            .unwrap_or_else(|e| panic!("ch: {e}"));
        let verified = steward
            .submit_seat_commit_via_xrpld(0, ch, [0x44u8; 32])
            .unwrap_or_else(|e| panic!("direct tx lookup commit failed: {e}"));
        assert_eq!(verified.seat, 0);
        assert_eq!(verified.commitment, ch);
        assert_eq!(*account_tx_calls.borrow(), 0);
        assert!(
            tx_lookup_hashes
                .borrow()
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&verified.tx_hash))
        );

        let after_count = tx_lookup_hashes.borrow().len() + 1;
        *force_unvalidated_tx.borrow_mut() = Some(after_count);
        let ch1 = compute_order_commitment(TableId(91), Turn(1), Seat(1), &rev)
            .unwrap_or_else(|e| panic!("ch1: {e}"));
        let err = steward.submit_seat_commit_via_xrpld(1, ch1, [0x55u8; 32]);
        assert!(matches!(err, Err(StewardError::Protocol(_))));
    }

    fn drive_phoenix_test_turn<T: Transport>(
        steward: &mut GameTableSteward<'_, T>,
        table_id: TableId,
        t: u8,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if steward.phase() != TurnPhase::TalkAndMarket {
            return Ok(true);
        }
        if t == 1 {
            steward.submit_market_action(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(1),
                    amount: 2 * UNIT_SCALE,
                    price_per_unit_cred: 2 * UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
            })?;
            steward.submit_market_action(MarketAction::CancelOffer {
                seat: 0,
                offer_id: 1,
            })?;
            steward.submit_market_action(MarketAction::TownImport {
                seat: 0,
                good: AssetId::Raw(1),
                amount: UNIT_SCALE,
            })?;
            steward.submit_market_action(MarketAction::CreateCheck {
                seat: 0,
                check: CheckProposal {
                    payee: 2,
                    asset: AssetId::Cred,
                    send_max: 2 * UNIT_SCALE,
                    deliver_min: 2 * UNIT_SCALE,
                    due_turn: 1,
                    expiration_turn: 3,
                },
            })?;
            let cond = compute_preimage_condition(&PreimageSecret([0x5Au8; 32]))?;
            steward.submit_market_action(MarketAction::CreateEscrow {
                seat: 0,
                escrow: EscrowProposal {
                    destination: 1,
                    asset: AssetId::Cred,
                    amount: 3 * UNIT_SCALE,
                    finish_after_turn: Some(1),
                    cancel_after_turn: 4,
                    condition_hash: Some(cond),
                    bond: None,
                },
            })?;
        }
        if t == 2 {
            steward.submit_market_action(MarketAction::StandForOffice {
                seat: 0,
                platform: CouncilPlatform {
                    proposed_tariffs_bps: vec![(0, (400, 0))],
                    cartel_admissions: vec![(1, 0), (1, 1)],
                    cartel_revocations: Vec::new(),
                },
            })?;
            steward.submit_market_action(MarketAction::StandForOffice {
                seat: 1,
                platform: CouncilPlatform {
                    proposed_tariffs_bps: Vec::new(),
                    cartel_admissions: vec![(1, 0), (1, 1), (1, 2)],
                    cartel_revocations: Vec::new(),
                },
            })?;
            steward.submit_market_action(MarketAction::CreateCheck {
                seat: 2,
                check: CheckProposal {
                    payee: 0,
                    asset: AssetId::Cred,
                    send_max: 500 * UNIT_SCALE,
                    deliver_min: 500 * UNIT_SCALE,
                    due_turn: 2,
                    expiration_turn: 5,
                },
            })?;
        }
        if t == 14 {
            // Open native Check (id 3) and native PREIMAGE-SHA-256 TokenEscrow (id 2) spanning
            // the Turn-15 wipe so Wave 3 must re-create and remap both before Turn 16 Sweep!
            steward.submit_market_action(MarketAction::CreateCheck {
                seat: 0,
                check: CheckProposal {
                    payee: 2,
                    asset: AssetId::Cred,
                    send_max: 4 * UNIT_SCALE,
                    deliver_min: 4 * UNIT_SCALE,
                    due_turn: 16,
                    expiration_turn: 19,
                },
            })?;
            let cond14 = compute_preimage_condition(&PreimageSecret([0x9Du8; 32]))?;
            steward.submit_market_action(MarketAction::CreateEscrow {
                seat: 0,
                escrow: EscrowProposal {
                    destination: 1,
                    asset: AssetId::Cred,
                    amount: 5 * UNIT_SCALE,
                    finish_after_turn: Some(16),
                    cancel_after_turn: 19,
                    condition_hash: Some(cond14),
                    bond: None,
                },
            })?;
        }

        steward.advance_to_commit()?;
        let mut reveals = Vec::new();
        for s in 0..4_u8 {
            if t == 16 && s == 3 {
                continue;
            }
            let mut bundle = SealedOrderBundle::default();
            if t == 1 && s == 1 {
                bundle.escrow_finishes.push(EscrowFinishUnlock {
                    escrow_id: 1,
                    preimage: Some(PreimageSecret([0x5Au8; 32])),
                });
            }
            if t == 1 && s == 2 {
                bundle.check_cashes.push(1);
            }
            if (t == 2 || t == 14) && s == 0 {
                bundle.converter_runs.push(ConverterRunOrder {
                    recipe_idx: 0,
                    runs: 1,
                });
                if t == 2 {
                    bundle.check_cashes.push(2);
                }
            }
            if t == 4 {
                bundle.council_vote = Some(CouncilVoteBallot {
                    trade_minister: 0,
                    cartel_chair: 1,
                });
            }
            if (t == 6 || t == 12) && s == 0 {
                bundle.charter_bids.push(CharterBidOrder {
                    role: CharterRole::ClearinghouseBroker,
                    bid_cred: UNIT_SCALE,
                });
            }
            if t == 16 && s == 1 {
                bundle.escrow_finishes.push(EscrowFinishUnlock {
                    escrow_id: 2,
                    preimage: Some(PreimageSecret([0x9Du8; 32])),
                });
            }
            if t == 16 && s == 2 {
                bundle.check_cashes.push(3);
            }
            let rev = OrderReveal {
                bundle,
                salt: RevealSalt([s.wrapping_add(t).max(1); 32]),
            };
            let ch = compute_order_commitment(table_id, Turn(t), Seat(s), &rev)?;
            steward.submit_seat_commit_via_xrpld(s, ch, [s.wrapping_add(t).max(1); 32])?;
            reveals.push((s, rev));
        }

        steward.advance_to_reveal()?;
        for (s, rev) in reveals {
            steward.submit_seat_reveal_via_xrpld(s, rev)?;
        }
        if t == 16 {
            let (_, res) = steward.exercise_regent_delegation_on_xrpld(3)?;
            assert_eq!(res, "tesSUCCESS");
        }
        let round = steward.current_window().expected_drand_round;
        let sig = quicknet_sig_for_round(round);
        let summary = steward.sweep_turn(round, &sig)?;
        Ok(summary.public_receipt.season_closed)
    }

    #[test]
    fn scripted_turn15_wipe_and_three_wave_phoenix_rehydrates_zero_divergence()
    -> Result<(), Box<dyn std::error::Error>> {
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let table_id = TableId(515);
        let (mut spec, deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(table_id, 4, 7, schedule)?;
        for s in &mut spec.seats {
            s.initial_cred = 100 * UNIT_SCALE;
        }

        // 1. Unwiped control season.
        let ctrl_node = ScriptedXrplNode::new();
        let ctrl_client = StandAlone::with_transport("http://127.0.0.1:5005", ctrl_node)?;
        let mut ctrl = GameTableSteward::bootstrap(&ctrl_client, spec.clone(), 7, deal.clone())?;
        for t in 1..=HARD_TURN_CAP {
            if drive_phoenix_test_turn(&mut ctrl, table_id, t)? {
                break;
            }
        }
        let ctrl_digests = ctrl.live_state_digests().to_vec();
        let ctrl_report = ctrl.finalize_and_verify_replay()?;

        // 2. Wiped season: play Turns 1..=15, serialize ArchivistMirror, wipe node to fresh
        // genesis, reconnect, rehydrate in <= 3 closes, play Turns 16..=Close.
        let wipe_node = ScriptedXrplNode::new();
        let wipe_client = StandAlone::with_transport("http://127.0.0.1:5005", wipe_node.clone())?;
        let mut pre = GameTableSteward::bootstrap(&wipe_client, spec, 7, deal)?;
        assert!(pre.rehydrate_if_wiped()?.is_none());

        for t in 1..=15_u8 {
            let closed = drive_phoenix_test_turn(&mut pre, table_id, t)?;
            assert!(!closed, "season unexpectedly closed at turn {t}");
        }
        assert_eq!(pre.turn(), 16);

        let archivist_bytes = pre.archivist().to_json_bytes()?;
        let keyring = pre.keyring_clone()?;
        let disconnected = pre.into_disconnected();
        let (_, _, reserve_pool, prev_ledger, _) = disconnected.into_parts();

        wipe_node.wipe_to_fresh_genesis();
        let post_client = StandAlone::with_transport("http://127.0.0.1:5005", wipe_node)?;
        let mut post = GameTableSteward::from_archivist_bytes(
            &post_client,
            &archivist_bytes,
            keyring,
            reserve_pool,
            prev_ledger,
        )?;
        let _ = post.reserve_pool();
        let _ = post.reserve_pool_mut();

        let phoenix = post
            .rehydrate_if_wiped()?
            .ok_or("expected WipeSignal on wiped node")?;
        assert_eq!(phoenix.wipe_code, "WIPE_AUTHORITY_MISSING");
        assert!(phoenix.phoenix.ledger_closes_used <= 3);
        assert_eq!(phoenix.phoenix.remapped_escrow_sequences.len(), 1);
        assert_eq!(phoenix.phoenix.remapped_check_ids.len(), 1);

        for t in 16..=HARD_TURN_CAP {
            if drive_phoenix_test_turn(&mut post, table_id, t)? {
                break;
            }
        }
        let post_digests = post.live_state_digests().to_vec();
        let post_report = post.finalize_and_verify_replay()?;

        assert_eq!(post_digests, ctrl_digests);
        assert_eq!(post_report.scoreboard, ctrl_report.scoreboard);
        assert_eq!(post_report.scoreboard, post_report.replay_scoreboard);
        assert_eq!(
            post_report.drand_auth_status,
            DrandBeaconAuthStatus::Authenticated
        );
        Ok(())
    }

    #[test]
    fn scripted_mid_commit_wipe_and_disconnected_steward_guards()
    -> Result<(), Box<dyn std::error::Error>> {
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let table_id = TableId(516);
        let (mut spec, deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(table_id, 4, 7, schedule)?;
        for s in &mut spec.seats {
            s.initial_cred = 100 * UNIT_SCALE;
        }

        let node = ScriptedXrplNode::new();
        let client = StandAlone::with_transport("http://127.0.0.1:5005", node.clone())?;
        let mut pre = GameTableSteward::bootstrap(&client, spec, 7, deal)?;

        // Exercise CancelLockEngaged + post-lock MarketActionRejected archiving, plus
        // mid-Commit 1-drop escrow tracking before a wipe.
        pre.submit_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: UNIT_SCALE,
                price_per_unit_cred: 2 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        })?;
        let _ = pre.advance_to_cancel_lock()?;
        assert!(matches!(
            pre.submit_market_action(MarketAction::CancelOffer {
                seat: 0,
                offer_id: 1,
            }),
            Err(StewardError::CancelLocked { .. })
        ));

        pre.advance_to_commit()?;
        let rev0 = OrderReveal {
            bundle: SealedOrderBundle::default(),
            salt: RevealSalt([1; 32]),
        };
        let ch0 = compute_order_commitment(table_id, Turn(1), Seat(0), &rev0)?;
        pre.submit_seat_commit_via_xrpld(0, ch0, [1; 32])?;

        let keyring = pre.keyring_clone()?;
        let dbg_ring = format!("{keyring:?}");
        assert!(dbg_ring.contains("<redacted>"));
        assert!(!dbg_ring.contains("sEd"));

        let arch_bytes = pre.archivist().to_json_bytes()?;

        let mut disconnected = pre.into_disconnected();
        assert!(!disconnected.archivist().is_empty());
        assert!(!disconnected.archivist_mut().is_empty());
        assert!(
            disconnected.reserve_pool().available_drops > 0
                || disconnected.reserve_pool().allocated_drops > 0
        );
        assert!(disconnected.last_observed_ledger() > 0);

        // Keyring validation error paths on from_archivist_bytes.
        let mut wrong_count_ring = keyring.try_clone()?;
        wrong_count_ring.seat_accounts.pop();
        assert!(matches!(
            GameTableSteward::from_archivist_bytes(
                &client,
                &arch_bytes,
                wrong_count_ring,
                disconnected.reserve_pool().clone(),
                disconnected.last_observed_ledger(),
            ),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        let mut wrong_steward_ring = keyring.try_clone()?;
        wrong_steward_ring.steward_account = StandAloneAccount::generate_ed25519()?;
        assert!(matches!(
            GameTableSteward::from_archivist_bytes(
                &client,
                &arch_bytes,
                wrong_steward_ring,
                disconnected.reserve_pool().clone(),
                disconnected.last_observed_ledger(),
            ),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        let mut wrong_seat_ring = keyring.try_clone()?;
        wrong_seat_ring.seat_accounts[1] = StandAloneAccount::generate_ed25519()?;
        assert!(matches!(
            GameTableSteward::from_archivist_bytes(
                &client,
                &arch_bytes,
                wrong_seat_ring,
                disconnected.reserve_pool().clone(),
                disconnected.last_observed_ledger(),
            ),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        // Wipe mid-Commit, reconnect via DisconnectedSteward::reconnect, rehydrate in <= 3 closes,
        // submit remaining commits + reveals (finishing remapped seat-0 1-drop commit escrow), sweep Turn 1.
        node.wipe_to_fresh_genesis();
        let post_client = StandAlone::with_transport("http://127.0.0.1:5005", node)?;
        let mut post = disconnected.reconnect(&post_client)?;
        let phoenix = post.rehydrate_if_wiped()?.ok_or("expected wipe report")?;
        assert!(phoenix.phoenix.ledger_closes_used <= 3);
        assert_eq!(phoenix.phoenix.remapped_commit_sequences.len(), 1);
        assert!(phoenix.phoenix.remapped_commit_sequences.contains_key(&0));

        // Folding PhoenixRehydrated via materialize() succeeds.
        let mat_after = post.archivist().materialize()?;
        assert_eq!(
            mat_after.active_commit_escrows.get(&0).map(|c| c.sequence),
            phoenix.phoenix.remapped_commit_sequences.get(&0).copied()
        );

        let mut rem_revs = Vec::new();
        for s in 1..4_u8 {
            let rev = OrderReveal {
                bundle: SealedOrderBundle::default(),
                salt: RevealSalt([s + 1; 32]),
            };
            let ch = compute_order_commitment(table_id, Turn(1), Seat(s), &rev)?;
            post.submit_seat_commit_via_xrpld(s, ch, [s + 1; 32])?;
            rem_revs.push((s, rev));
        }

        post.advance_to_reveal()?;
        post.submit_seat_reveal_via_xrpld(0, rev0)?;
        for (s, rev) in rem_revs {
            post.submit_seat_reveal_via_xrpld(s, rev)?;
        }
        let round = post.current_window().expected_drand_round;
        let sig = quicknet_sig_for_round(round);
        assert!(post.sweep_turn(round, &sig).is_ok());
        Ok(())
    }

    #[test]
    fn steward_contract_cancellations_firewall_and_parity_error_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let table_id = TableId(517);
        let (mut spec, deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(table_id, 4, 7, schedule)?;
        spec.seats[1].regent = None;
        let node = ScriptedXrplNode::new();
        let client = StandAlone::with_transport("http://127.0.0.1:5005", node.clone())?;
        let mut steward = GameTableSteward::bootstrap(&client, spec, 7, deal)?;

        // Premature finalize + invalid regent delegation errors.
        assert!(steward.finalize_and_verify_replay().is_err());
        assert!(steward.exercise_regent_delegation_on_xrpld(99).is_err());
        assert!(steward.exercise_regent_delegation_on_xrpld(1).is_err());

        // Turn 1: CreateCheck + CancelCheck, plus custody CreateEscrow (Raw) & native CreateEscrow (Cred).
        steward.submit_market_action(MarketAction::CreateCheck {
            seat: 0,
            check: CheckProposal {
                payee: 1,
                asset: AssetId::Cred,
                send_max: 2 * UNIT_SCALE,
                deliver_min: 2 * UNIT_SCALE,
                due_turn: 2,
                expiration_turn: 4,
            },
        })?;
        steward.submit_market_action(MarketAction::CancelCheck {
            seat: 0,
            check_id: 1,
        })?;
        steward.submit_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: UNIT_SCALE,
                finish_after_turn: Some(1),
                cancel_after_turn: 2,
                condition_hash: None,
                bond: None,
            },
        })?;
        let cond2 = compute_preimage_condition(&PreimageSecret([0x77u8; 32]))?;
        steward.submit_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: UNIT_SCALE,
                finish_after_turn: Some(1),
                cancel_after_turn: 2,
                condition_hash: Some(cond2),
                bond: None,
            },
        })?;

        // Check firewall rejection & CommitWindowClosed branch via ingest_commit_tx.
        steward.advance_to_commit()?;
        assert!(matches!(
            steward.ingest_commit_tx(
                &json!({ "Account": "rOutsiderNotSeated", "hash": "AA".repeat(32) }),
                100,
            ),
            Err(StewardError::FirewallDropped(_))
        ));
        let rev0 = OrderReveal {
            bundle: SealedOrderBundle::default(),
            salt: RevealSalt([9; 32]),
        };
        let ch0 = compute_order_commitment(table_id, Turn(1), Seat(0), &rev0)?;
        let late_tx = json!({
            "Account": steward.seat_account(0).ok_or("seat 0")?.address(),
            "TransactionType": "EscrowCreate",
            "hash": "BB".repeat(32),
            "Memos": encode_commit_memos_json(ch0)
        });
        assert!(matches!(
            steward.ingest_commit_tx(&late_tx, u64::MAX / 2),
            Err(StewardError::CommitWindowClosed { .. })
        ));
        steward.advance_to_reveal()?;
        let r1 = steward.current_window().expected_drand_round;
        let s1 = quicknet_sig_for_round(r1);
        steward.sweep_turn(r1, &s1)?;

        // Turn 2: both escrows have cancel_after_turn=2; cancel custody escrow (1) and native escrow (2).
        steward.submit_market_action(MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 1,
        })?;
        steward.submit_market_action(MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 2,
        })?;

        // Exercise remaining BalanceDivergence branches (raw, refined, steward-only currency).
        let s0_addr = steward
            .seat_account(0)
            .ok_or("seat 0")?
            .address()
            .to_string();
        let st_addr = steward.steward_account().address().to_string();
        let raw_code = asset_currency_code(AssetId::Raw(0))?;
        let ref_code = asset_currency_code(AssetId::Refined(0))?;

        let key_raw = (s0_addr.clone(), st_addr.clone(), raw_code.clone());
        let orig_raw = node
            .state
            .borrow()
            .lines
            .get(&key_raw)
            .copied()
            .unwrap_or(0);
        node.state
            .borrow_mut()
            .lines
            .insert(key_raw.clone(), orig_raw + 1);
        assert!(matches!(
            steward.assert_account_lines_parity(),
            Err(StewardError::BalanceDivergence { .. })
        ));
        node.state.borrow_mut().lines.insert(key_raw, orig_raw);

        let key_ref = (s0_addr, st_addr.clone(), ref_code);
        let orig_ref = node
            .state
            .borrow()
            .lines
            .get(&key_ref)
            .copied()
            .unwrap_or(0);
        node.state
            .borrow_mut()
            .lines
            .insert(key_ref.clone(), orig_ref + 1);
        assert!(matches!(
            steward.assert_account_lines_parity(),
            Err(StewardError::BalanceDivergence { .. })
        ));
        node.state.borrow_mut().lines.insert(key_ref, orig_ref);

        // Non-seat peer line on Steward for a table currency hits steward conservation check.
        node.state
            .borrow_mut()
            .lines
            .insert(("rBogusPeer".into(), st_addr, raw_code), 5_000);
        assert!(matches!(
            steward.assert_account_lines_parity(),
            Err(StewardError::BalanceDivergence { .. })
        ));
        Ok(())
    }
}
