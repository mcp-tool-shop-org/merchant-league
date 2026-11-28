//! Steward Testnet XRP reserve, fee-budget, and faucet pool accounting
//! (`docs/game-mode-protocol.md` §5.1).
//!
//! Seat isolation invariant: raw drop balances, owner reserves, fee floors, and
//! faucet grants remain strictly inside host/Steward accounting and never enter
//! `SeatObservation` or LLM prompts.

use league_ledger::client::{Testnet, Transport};
use league_standalone::client::StandAlone;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::StewardError;

/// One XRP expressed in integer drops (`10^6`).
pub const DROPS_PER_XRP: u64 = 1_000_000;
/// Default XRPL `AccountRoot` base reserve (`1.0` XRP on `xrpld 3.4.1`).
pub const DEFAULT_BASE_RESERVE_DROPS: u64 = 1_000_000;
/// Default XRPL per-object owner reserve increment (`0.2` XRP on `xrpld 3.4.1`).
pub const DEFAULT_OWNER_RESERVE_DROPS: u64 = 200_000;
/// Default Testnet open-ledger base transaction fee floor in drops.
pub const DEFAULT_BASE_FEE_DROPS: u64 = 12;
/// Default `EscrowFinish` preimage fulfillment fee floor in drops (`league-ledger`).
pub const DEFAULT_ESCROW_FINISH_FEE_DROPS: u64 = 500;
/// Standard Testnet faucet grant per call (`1,000` Testnet XRP).
pub const DEFAULT_TESTNET_FAUCET_GRANT_DROPS: u64 = 1_000 * DROPS_PER_XRP;
/// Default safety multiplier (`15_000 bps` = `1.50x`) triggering faucet replenishment.
pub const DEFAULT_RESERVE_SAFETY_BPS: u32 = 15_000;

/// Integer-drop ledger reserve and fee parameters for Steward pool sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveParams {
    /// Base XRP reserve per funded account in drops.
    pub base_reserve_drops: u64,
    /// Owner reserve increment per owned ledger object in drops.
    pub owner_reserve_drops: u64,
    /// Standard single-tx fee floor in drops.
    pub base_fee_drops: u64,
    /// Preimage `EscrowFinish` fee floor in drops (`>= 350`, Testnet `500`).
    pub escrow_finish_fee_drops: u64,
    /// Expected XRP grant per Testnet faucet call in drops.
    pub faucet_grant_drops: u64,
    /// Safety factor in basis points (`>= 10_000`, default `15_000` = `1.50x`).
    pub safety_factor_bps: u32,
}

impl Default for ReserveParams {
    fn default() -> Self {
        Self {
            base_reserve_drops: DEFAULT_BASE_RESERVE_DROPS,
            owner_reserve_drops: DEFAULT_OWNER_RESERVE_DROPS,
            base_fee_drops: DEFAULT_BASE_FEE_DROPS,
            escrow_finish_fee_drops: DEFAULT_ESCROW_FINISH_FEE_DROPS,
            faucet_grant_drops: DEFAULT_TESTNET_FAUCET_GRANT_DROPS,
            safety_factor_bps: DEFAULT_RESERVE_SAFETY_BPS,
        }
    }
}

impl ReserveParams {
    /// Parse `reserve_base_xrp` / `reserve_inc_xrp` (or drop equivalents) from an XRPL
    /// `server_info` JSON payload, falling back to [`Self::default`] when omitted.
    #[must_use]
    pub fn from_server_info_json(info_root: &Value) -> Self {
        let mut params = Self::default();
        let validated = info_root
            .get("info")
            .and_then(|i| i.get("validated_ledger"))
            .or_else(|| info_root.get("validated_ledger"));
        let Some(vl) = validated else {
            return params;
        };
        if let Some(base_drops) = parse_xrp_or_drops(vl, "reserve_base_xrp", "reserve_base") {
            params.base_reserve_drops = base_drops;
        }
        if let Some(inc_drops) = parse_xrp_or_drops(vl, "reserve_inc_xrp", "reserve_inc") {
            params.owner_reserve_drops = inc_drops;
        }
        params
    }
}

/// Deterministic reserve and fee estimate for one `TableMode::Game` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableReserveEstimate {
    /// Seat count (`4..=6`).
    pub seat_count: u8,
    /// Standing Regent delegate account count (`0..=seat_count`).
    pub regent_count: u8,
    /// Total XRPL accounts provisioned (`1` Steward + seats + regents).
    pub account_count: u32,
    /// Authorized IOU trust lines (`seat_count * (1 + 2 * seat_count)`).
    pub trust_line_objects: u32,
    /// `Credentials`, `PermissionedDomains`, `DelegateSet`, and `OracleSet` owner objects.
    pub credential_domain_delegate_objects: u32,
    /// Pre-allocated `TicketCreate` objects across Steward and seats.
    pub ticket_objects: u32,
    /// Worst-case transient `TokenEscrow` and `CheckCreate` ledger objects (`0` open `Offer`
    /// objects because `PostOffer` locks via Steward `Clawback` custody).
    pub transient_contract_objects: u32,
    /// Sum of owned ledger objects across the table.
    pub total_owner_objects: u32,
    /// Base reserve across `account_count` in drops.
    pub base_reserve_drops: u64,
    /// Owner reserve across `total_owner_objects` in drops.
    pub owner_reserve_drops: u64,
    /// 32-turn `BatchV1_1`, `EscrowFinish` (`500` drops), and 3-wave Phoenix fee budget in drops.
    pub season_fee_budget_drops: u64,
    /// Unscaled table reserve + fee estimate in drops.
    pub total_table_estimate_drops: u64,
    /// Scaled replenishment threshold `ceil(total_table_estimate_drops * safety_factor_bps / 10_000)`.
    pub replenish_threshold_drops: u64,
}

/// Compute the deterministic reserve + fee footprint for one `TableMode::Game` table
/// (`docs/game-mode-protocol.md` §5.1).
pub fn estimate_table_reserves(
    params: &ReserveParams,
    seat_count: u8,
    regent_count: u8,
    tickets_per_account: u32,
    max_transient_contracts: u32,
) -> Result<TableReserveEstimate, StewardError> {
    if !(4..=6).contains(&seat_count) {
        return Err(StewardError::Protocol(format!(
            "table reserve estimate requires seat_count in 4..=6, got {seat_count}"
        )));
    }
    if regent_count > seat_count {
        return Err(StewardError::Protocol(format!(
            "regent_count {regent_count} exceeds seat_count {seat_count}"
        )));
    }

    let n = u32::from(seat_count);
    let r = u32::from(regent_count);
    let account_count = 1 + n + r;
    // Steward custody (`Clawback`) leaves 0 native DEX Offer objects on seat owner directories.
    let trust_line_objects = n * (1 + 2 * n);
    let credential_domain_delegate_objects = n * 4 + 4 + r + 2;
    let ticket_objects = (1 + n).saturating_mul(tickets_per_account);
    let transient_contract_objects = max_transient_contracts;
    let total_owner_objects = trust_line_objects
        .saturating_add(credential_domain_delegate_objects)
        .saturating_add(ticket_objects)
        .saturating_add(transient_contract_objects);

    let base_reserve_drops = u64::from(account_count).saturating_mul(params.base_reserve_drops);
    let owner_reserve_drops =
        u64::from(total_owner_objects).saturating_mul(params.owner_reserve_drops);

    // 32 turns: up to 4 BatchV1_1 envelopes/turn (~120 drops each) + N EscrowFinishes/turn +
    // one complete 3-wave Phoenix rehydration budget (~140 tx equivalents).
    let per_turn_batch_fees = 4_u64.saturating_mul(params.base_fee_drops.saturating_mul(10));
    let per_turn_escrow_fees = u64::from(n).saturating_mul(params.escrow_finish_fee_drops);
    let phoenix_fee_allowance = 140_u64.saturating_mul(params.base_fee_drops.max(20));
    let season_fee_budget_drops = 32_u64
        .saturating_mul(per_turn_batch_fees.saturating_add(per_turn_escrow_fees))
        .saturating_add(phoenix_fee_allowance);

    let total_table_estimate_drops = base_reserve_drops
        .saturating_add(owner_reserve_drops)
        .saturating_add(season_fee_budget_drops);
    let safety = u128::from(params.safety_factor_bps.max(10_000));
    let threshold_u128 = (u128::from(total_table_estimate_drops) * safety).div_ceil(10_000);
    let replenish_threshold_drops = u64::try_from(threshold_u128).unwrap_or(u64::MAX);

    Ok(TableReserveEstimate {
        seat_count,
        regent_count,
        account_count,
        trust_line_objects,
        credential_domain_delegate_objects,
        ticket_objects,
        transient_contract_objects,
        total_owner_objects,
        base_reserve_drops,
        owner_reserve_drops,
        season_fee_budget_drops,
        total_table_estimate_drops,
        replenish_threshold_drops,
    })
}

/// Receipt recorded when Steward replenishes its reserve pool from the Testnet faucet
/// (never stores secret seed material).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaucetReplenishReceipt {
    /// Funded classic XRPL `r...` address returned by the faucet.
    pub funded_address: String,
    /// Drops credited to the reserve pool.
    pub drops_added: u64,
    /// Pool available balance after crediting this grant.
    pub pool_available_drops_after: u64,
}

/// Host-only Steward XRP fee and reserve pool ledger (`docs/game-mode-protocol.md` §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StewardReservePool {
    /// Active ledger reserve and fee parameters.
    pub params: ReserveParams,
    /// Unallocated pool reserves currently available in drops.
    pub available_drops: u64,
    /// Cumulative drops provisioned for table operations.
    pub allocated_drops: u64,
    /// Cumulative drops consumed by 3-wave Phoenix rehydrations.
    pub rehydration_drops_spent: u64,
    /// Number of Testnet faucet top-ups executed.
    pub faucet_top_ups: u32,
    /// Total drops received from faucet top-ups.
    pub faucet_drops_received: u64,
}

impl StewardReservePool {
    /// Construct a reserve pool with `initial_available_drops`.
    #[must_use]
    pub fn new(params: ReserveParams, initial_available_drops: u64) -> Self {
        Self {
            params,
            available_drops: initial_available_drops,
            allocated_drops: 0,
            rehydration_drops_spent: 0,
            faucet_top_ups: 0,
            faucet_drops_received: 0,
        }
    }

    /// Return `true` when available pool drops fall below `estimate.replenish_threshold_drops`.
    #[must_use]
    pub const fn needs_replenishment(&self, estimate: &TableReserveEstimate) -> bool {
        self.available_drops < estimate.replenish_threshold_drops
    }

    /// Provision `amount_drops` from `available_drops` for a table allocation.
    pub fn allocate_table(&mut self, amount_drops: u64) -> Result<(), StewardError> {
        if self.available_drops < amount_drops {
            return Err(StewardError::Protocol(format!(
                "steward reserve pool holds {} drops, below requested {} drops",
                self.available_drops, amount_drops
            )));
        }
        self.available_drops -= amount_drops;
        self.allocated_drops = self.allocated_drops.saturating_add(amount_drops);
        Ok(())
    }

    /// Record drops spent provisioning accounts and transaction fees during Phoenix rehydration.
    pub fn record_phoenix_rehydration_funding(
        &mut self,
        amount_drops: u64,
    ) -> Result<(), StewardError> {
        if self.available_drops < amount_drops {
            self.available_drops = 0;
        } else {
            self.available_drops -= amount_drops;
        }
        self.rehydration_drops_spent = self.rehydration_drops_spent.saturating_add(amount_drops);
        Ok(())
    }

    /// Automatically replenish from `TESTNET_FAUCET` via [`Testnet::fund_new_account`] whenever
    /// available headroom is below `estimate.replenish_threshold_drops`.
    pub fn replenish_from_testnet_faucet_if_low<T: Transport>(
        &mut self,
        testnet: &Testnet<T>,
        estimate: &TableReserveEstimate,
    ) -> Result<Vec<FaucetReplenishReceipt>, StewardError> {
        let mut receipts = Vec::new();
        let max_calls = 8_u32;
        for _ in 0..max_calls {
            if !self.needs_replenishment(estimate) {
                break;
            }
            let funded = testnet
                .fund_new_account()
                .map_err(|err| StewardError::Protocol(format!("testnet faucet failed: {err}")))?;
            let addr = funded.account.address().to_string();
            // Drop `funded` immediately so its secret seed never persists in Steward memory.
            drop(funded);

            let granted = match testnet.call(
                "account_info",
                json!({ "account": addr, "ledger_index": "validated" }),
            ) {
                Ok(info) => info
                    .get("account_data")
                    .and_then(|d| d.get("Balance"))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(self.params.faucet_grant_drops),
                Err(_) => self.params.faucet_grant_drops,
            };
            self.available_drops = self.available_drops.saturating_add(granted);
            self.faucet_top_ups = self.faucet_top_ups.saturating_add(1);
            self.faucet_drops_received = self.faucet_drops_received.saturating_add(granted);
            receipts.push(FaucetReplenishReceipt {
                funded_address: addr,
                drops_added: granted,
                pool_available_drops_after: self.available_drops,
            });
        }
        if self.needs_replenishment(estimate) {
            return Err(StewardError::Protocol(format!(
                "steward reserve pool {} drops remained below threshold {} drops after faucet top-ups",
                self.available_drops, estimate.replenish_threshold_drops
            )));
        }
        Ok(receipts)
    }

    /// Top up `pool_address` from the stand-alone genesis root when pool headroom is low.
    pub fn replenish_from_standalone_genesis_if_low<T: Transport>(
        &mut self,
        client: &StandAlone<T>,
        pool_address: &str,
        estimate: &TableReserveEstimate,
    ) -> Result<u64, StewardError> {
        if !self.needs_replenishment(estimate) {
            return Ok(0);
        }
        let target = estimate
            .replenish_threshold_drops
            .max(self.params.faucet_grant_drops);
        let top_up = target.saturating_sub(self.available_drops);
        if top_up > 0 {
            let _ = client.fund_account(pool_address, top_up)?;
            self.available_drops = self.available_drops.saturating_add(top_up);
            self.faucet_top_ups = self.faucet_top_ups.saturating_add(1);
            self.faucet_drops_received = self.faucet_drops_received.saturating_add(top_up);
        }
        Ok(top_up)
    }
}

fn parse_xrp_or_drops(vl: &Value, xrp_key: &str, drops_key: &str) -> Option<u64> {
    if let Some(xrp_val) = vl.get(xrp_key) {
        if let Some(whole) = xrp_val.as_u64() {
            return whole.checked_mul(DROPS_PER_XRP);
        }
        if let Some(f) = xrp_val.as_f64()
            && f.is_finite()
            && f >= 0.0
        {
            return Some((f * (DROPS_PER_XRP as f64)).round() as u64);
        }
    }
    vl.get(drops_key).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use league_standalone::client::StandAloneAccount;
    use xrpl::wallet::Wallet;

    use super::*;

    struct ScriptedFaucetTransport {
        replies: RefCell<VecDeque<Result<Value, String>>>,
    }

    impl ScriptedFaucetTransport {
        fn new(replies: Vec<Result<Value, String>>) -> Self {
            Self {
                replies: RefCell::new(VecDeque::from(replies)),
            }
        }
    }

    impl Transport for ScriptedFaucetTransport {
        fn post(&self, _url: &str, _body: &Value) -> Result<Value, String> {
            self.replies
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("no scripted reply".into()))
        }
    }

    #[test]
    fn estimate_table_reserves_covers_4_to_6_seats_and_rejects_invalid_counts() {
        let params = ReserveParams::from_server_info_json(&json!({
            "info": {
                "validated_ledger": {
                    "reserve_base_xrp": 1,
                    "reserve_inc_xrp": 0.2
                }
            }
        }));
        assert_eq!(params.base_reserve_drops, 1_000_000);
        assert_eq!(params.owner_reserve_drops, 200_000);

        let est6 = estimate_table_reserves(&params, 6, 6, 40, 24)
            .unwrap_or_else(|e| panic!("6-seat estimate: {e}"));
        assert_eq!(est6.account_count, 13);
        assert_eq!(est6.trust_line_objects, 78);
        assert!(est6.total_table_estimate_drops > 30 * DROPS_PER_XRP);
        assert!(est6.replenish_threshold_drops > est6.total_table_estimate_drops);

        let est4 = estimate_table_reserves(&params, 4, 2, 20, 12)
            .unwrap_or_else(|e| panic!("4-seat estimate: {e}"));
        assert_eq!(est4.trust_line_objects, 36);
        assert!(est4.total_table_estimate_drops < est6.total_table_estimate_drops);

        assert!(estimate_table_reserves(&params, 3, 0, 10, 4).is_err());
        assert!(estimate_table_reserves(&params, 4, 5, 10, 4).is_err());
    }

    #[test]
    fn reserve_pool_auto_replenishes_from_testnet_faucet_when_low() {
        let wallet = Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .unwrap_or_else(|e| panic!("wallet: {e}"));
        let params = ReserveParams::default();
        let est = estimate_table_reserves(&params, 4, 4, 20, 10)
            .unwrap_or_else(|e| panic!("estimate: {e}"));

        let transport = ScriptedFaucetTransport::new(vec![
            Ok(json!({ "result": { "info": { "network_id": 1 } } })),
            Ok(json!({
                "seed": wallet.seed,
                "account": { "classicAddress": wallet.classic_address }
            })),
            Ok(json!({
                "result": {
                    "account_data": {
                        "Balance": "1000000000"
                    }
                }
            })),
        ]);
        let testnet = Testnet::with_transport(transport, "http://rpc", "http://faucet")
            .unwrap_or_else(|e| panic!("testnet: {e}"));

        let mut pool = StewardReservePool::new(params, 5_000_000);
        assert!(pool.needs_replenishment(&est));
        assert!(pool.allocate_table(10_000_000).is_err());

        let receipts = pool
            .replenish_from_testnet_faucet_if_low(&testnet, &est)
            .unwrap_or_else(|e| panic!("faucet replenish: {e}"));
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].funded_address, wallet.classic_address);
        assert_eq!(receipts[0].drops_added, 1_000_000_000);
        assert!(!pool.needs_replenishment(&est));
        assert_eq!(pool.faucet_top_ups, 1);

        // Second check above threshold performs zero faucet calls.
        let noop = pool
            .replenish_from_testnet_faucet_if_low(&testnet, &est)
            .unwrap_or_else(|e| panic!("noop: {e}"));
        assert!(noop.is_empty());

        assert!(pool.allocate_table(est.total_table_estimate_drops).is_ok());
        assert!(pool.record_phoenix_rehydration_funding(10_000_000).is_ok());
        assert_eq!(pool.rehydration_drops_spent, 10_000_000);
    }

    #[test]
    fn reserve_params_and_pool_edge_branches() {
        let direct = ReserveParams::from_server_info_json(&json!({
            "validated_ledger": {
                "reserve_base": 2_000_000,
                "reserve_inc": 500_000
            }
        }));
        assert_eq!(direct.base_reserve_drops, 2_000_000);
        assert_eq!(direct.owner_reserve_drops, 500_000);

        let fallback = ReserveParams::from_server_info_json(&json!({}));
        assert_eq!(fallback, ReserveParams::default());

        let mut pool = StewardReservePool::new(fallback, 100_000_000);
        assert_eq!(pool.params, ReserveParams::default());
        assert_eq!(pool.available_drops, 100_000_000);
        assert_eq!(pool.rehydration_drops_spent, 0);
        assert_eq!(pool.faucet_top_ups, 0);
        assert_eq!(pool.faucet_drops_received, 0);

        // Funding larger than available clamps available_drops to 0 without underflow.
        assert!(pool.record_phoenix_rehydration_funding(150_000_000).is_ok());
        assert_eq!(pool.available_drops, 0);
        assert_eq!(pool.rehydration_drops_spent, 150_000_000);

        let est = estimate_table_reserves(&ReserveParams::default(), 4, 4, 20, 10)
            .unwrap_or_else(|e| panic!("est: {e}"));

        // Stand-alone genesis replenish branch: no-op when above threshold, funds when below.
        struct SaMockTransport;
        impl Transport for SaMockTransport {
            fn post(&self, _url: &str, body: &Value) -> Result<Value, String> {
                let method = body
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match method {
                    "server_info" => Ok(json!({
                        "result": {
                            "status": "success",
                            "info": { "network_id": 21337, "validation_quorum": 0, "peers": 0, "server_state": "full" }
                        }
                    })),
                    "ledger" => Ok(json!({
                        "result": { "status": "success", "ledger": { "ledger_index": 2, "close_time": 10 } }
                    })),
                    "fee" => Ok(
                        json!({ "result": { "status": "success", "drops": { "base_fee": "10" } } }),
                    ),
                    "account_info" => Ok(json!({
                        "result": { "status": "success", "account_data": { "Sequence": 1, "Balance": "1000000000" } }
                    })),
                    "submit" => Ok(json!({
                        "result": { "status": "success", "accepted": true, "engine_result": "tesSUCCESS" }
                    })),
                    "ledger_accept" => {
                        Ok(json!({ "result": { "status": "success", "ledger_current_index": 3 } }))
                    }
                    "tx" => Ok(json!({
                        "result": { "status": "success", "validated": true, "meta": { "TransactionResult": "tesSUCCESS" } }
                    })),
                    other => Err(format!("unexpected sa method {other}")),
                }
            }
        }
        let pool_acc =
            StandAloneAccount::generate_ed25519().unwrap_or_else(|e| panic!("pool_acc: {e}"));
        let sa_client = StandAlone::with_transport("http://127.0.0.1:5005", SaMockTransport)
            .unwrap_or_else(|e| panic!("sa: {e}"));
        let top_up = pool
            .replenish_from_standalone_genesis_if_low(&sa_client, pool_acc.address(), &est)
            .unwrap_or_else(|e| panic!("standalone top-up: {e}"));
        assert!(top_up > 0);
        assert!(pool.faucet_top_ups >= 1);
        assert!(pool.faucet_drops_received >= top_up);
        let noop_sa = pool
            .replenish_from_standalone_genesis_if_low(&sa_client, pool_acc.address(), &est)
            .unwrap_or_else(|e| panic!("noop sa: {e}"));
        assert_eq!(noop_sa, 0);

        // Testnet faucet exhaustion error after 8 small grants that never reach threshold.
        let wallet = Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .unwrap_or_else(|e| panic!("wallet: {e}"));
        let mut exhaust_replies = vec![Ok(json!({ "result": { "info": { "network_id": 1 } } }))];
        for _ in 0..8 {
            exhaust_replies.push(Ok(json!({
                "seed": wallet.seed,
                "account": { "classicAddress": wallet.classic_address }
            })));
            // Force account_info fallback arm (Err -> faucet_grant_drops).
            exhaust_replies.push(Err("rpc timeout".into()));
        }
        let tiny_params = ReserveParams {
            faucet_grant_drops: 1_000,
            ..ReserveParams::default()
        };
        let mut tiny_pool = StewardReservePool::new(tiny_params, 0);
        let tiny_testnet = Testnet::with_transport(
            ScriptedFaucetTransport::new(exhaust_replies),
            "http://rpc",
            "http://faucet",
        )
        .unwrap_or_else(|e| panic!("testnet: {e}"));
        assert!(matches!(
            tiny_pool.replenish_from_testnet_faucet_if_low(&tiny_testnet, &est),
            Err(StewardError::Protocol(_))
        ));
    }
}
