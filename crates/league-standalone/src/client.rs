//! Checked RPC client, signer wrapper, and ledger helpers for stand-alone `xrpld`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{self, Debug};
use std::time::Duration;

use league_ledger::client::{
    Account as TestnetValidatedAccount, LedgerError, Transport, decode_hex, transaction_hash,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha512};
use strum::IntoEnumIterator;
use xrpl::core::binarycodec::encode;
use xrpl::core::keypairs::generate_seed;
use xrpl::models::transactions::Transaction;
use xrpl::models::transactions::payment::Payment;
use xrpl::models::{Amount, FlagCollection, Model, XRPAmount};
use xrpl::signing::sign;
use xrpl::wallet::Wallet;

use crate::{
    GENESIS_ACCOUNT_ADDRESS, MAINNET_MATCHED_AMENDMENTS, NotStandAlone, STANDALONE_NETWORK_ID,
    StandAloneServerInfo, StructuredError, TESTNET_ONLY_DEFAULT_YES_AMENDMENT, ToStructuredError,
    VETOED_NON_MAINNET_AMENDMENTS, require_loopback_http_url, require_standalone_info,
};

/// Minimum base transaction fee in drops for standard stand-alone transactions.
pub const BASE_FEE_DROPS: &str = "10";

/// Synchronous `ureq` transport bounded by a per-request global timeout.
#[derive(Debug, Clone)]
pub struct TimedUreq {
    agent: ureq::Agent,
}

impl TimedUreq {
    /// Construct an HTTP JSON-RPC transport with an explicit global request timeout.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }
}

impl Transport for TimedUreq {
    fn post(&self, url: &str, body: &Value) -> Result<Value, String> {
        let response = self
            .agent
            .post(url)
            .send_json(body)
            .map_err(|err| format!("http post failed: {err}"))?;
        let status = response.status();
        let parsed: Value = response
            .into_body()
            .read_json()
            .map_err(|err| format!("invalid json reply (status {status}): {err}"))?;
        Ok(parsed)
    }
}

/// Stand-alone signing account whose `Debug` output never reveals key material or seeds.
pub struct StandAloneAccount {
    wallet: Wallet,
}

impl Debug for StandAloneAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StandAloneAccount")
            .field("address", &self.wallet.classic_address)
            .field("keys", &"<redacted>")
            .finish()
    }
}

impl StandAloneAccount {
    /// Generate a fresh `ed25519` league seat validated by `league_ledger::client::Account`.
    pub fn generate_ed25519() -> Result<Self, StandAloneError> {
        let wallet = Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .map_err(|err| StandAloneError::Build(format!("wallet creation failed: {err}")))?;
        let _ = TestnetValidatedAccount::from_seed(&wallet.seed)?;
        Ok(Self { wallet })
    }

    /// Construct an `ed25519` stand-alone seat from an encoded XRPL family seed.
    pub fn from_ed25519_seed(seed: &str) -> Result<Self, StandAloneError> {
        let _ = TestnetValidatedAccount::from_seed(seed)?;
        let wallet = Wallet::new(seed, 0)
            .map_err(|_| StandAloneError::Build("failed to decode ed25519 wallet".into()))?;
        Ok(Self { wallet })
    }

    /// Derive the stand-alone genesis root account (`rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh`).
    pub(crate) fn genesis() -> Result<Self, StandAloneError> {
        let hash = Sha512::digest(b"masterpassphrase");
        let mut entropy = [0u8; 16];
        entropy.copy_from_slice(&hash[..16]);
        let seed = generate_seed(
            Some(entropy),
            Some(xrpl::constants::CryptoAlgorithm::SECP256K1),
        )
        .map_err(|err| StandAloneError::Build(format!("genesis seed derivation failed: {err}")))?;
        let wallet = Wallet::new(&seed, 0)
            .map_err(|err| StandAloneError::Build(format!("genesis wallet failed: {err}")))?;
        if wallet.classic_address != GENESIS_ACCOUNT_ADDRESS {
            return Err(StandAloneError::Build(format!(
                "derived genesis address {} does not match {GENESIS_ACCOUNT_ADDRESS}",
                wallet.classic_address
            )));
        }
        Ok(Self { wallet })
    }

    /// Classic XRPL `r...` address of this account.
    #[must_use]
    pub fn address(&self) -> &str {
        &self.wallet.classic_address
    }

    /// Compressed hexadecimal public key of this account.
    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.wallet.public_key
    }

    pub(crate) fn wallet(&self) -> &Wallet {
        &self.wallet
    }
}

/// Errors produced by `StandAlone` RPC calls, Conflict 11 checks, or transaction submissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StandAloneError {
    /// Target failed a Conflict 11 URL or `server_info` stand-alone guard check.
    NotStandAlone(NotStandAlone),
    /// Underlying `league_ledger` validation failure (such as non-ed25519 seed).
    Ledger(String),
    /// HTTP/RPC transport error.
    Transport(String),
    /// `xrpld` JSON-RPC error response.
    Rpc {
        /// RPC method name.
        method: String,
        /// `xrpld` machine error identifier.
        error: String,
        /// Optional human message from `xrpld`.
        error_message: Option<String>,
    },
    /// Unexpected or malformed JSON reply shape.
    Reply(String),
    /// Local transaction construction or signing failure.
    Build(String),
    /// Ledger transaction failed when `submit_and_accept_ok` expected `tesSUCCESS`.
    TxFailed {
        /// Transaction type name.
        tx_type: String,
        /// Engine or metadata transaction result code.
        result: String,
    },
}

impl From<NotStandAlone> for StandAloneError {
    fn from(value: NotStandAlone) -> Self {
        Self::NotStandAlone(value)
    }
}

impl From<LedgerError> for StandAloneError {
    fn from(value: LedgerError) -> Self {
        Self::Ledger(value.to_string())
    }
}

impl fmt::Display for StandAloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStandAlone(err) => write!(f, "{err}"),
            Self::Ledger(msg) => write!(f, "account validation error: {msg}"),
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::Rpc {
                method,
                error,
                error_message,
            } => match error_message {
                Some(detail) => write!(f, "rpc {method} failed ({error}): {detail}"),
                None => write!(f, "rpc {method} failed ({error})"),
            },
            Self::Reply(msg) => write!(f, "unexpected rpc reply: {msg}"),
            Self::Build(msg) => write!(f, "transaction build error: {msg}"),
            Self::TxFailed { tx_type, result } => {
                write!(f, "transaction {tx_type} failed with {result}")
            }
        }
    }
}

impl std::error::Error for StandAloneError {}

impl StandAloneError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotStandAlone(e) => e.code(),
            Self::Ledger(_) => "STANDALONE_ACCOUNT",
            Self::Transport(_) => "STANDALONE_TRANSPORT",
            Self::Rpc { .. } => "STANDALONE_RPC",
            Self::Reply(_) => "STANDALONE_BAD_REPLY",
            Self::Build(_) => "STANDALONE_TX_BUILD",
            Self::TxFailed { .. } => "STANDALONE_TX_FAILED",
        }
    }
}

impl ToStructuredError for StandAloneError {
    fn to_structured_error(&self) -> StructuredError {
        match self {
            Self::NotStandAlone(e) => e.to_structured_error(),
            Self::Ledger(msg) => StructuredError::new(
                self.code(),
                self.to_string(),
                "supply a valid ed25519 XRPL seed (sEd...) when constructing a stand-alone seat account",
                false,
            )
            .with_cause(msg),
            Self::Transport(msg) => StructuredError::new(
                self.code(),
                self.to_string(),
                "verify the local stand-alone xrpld container is running and accepting loopback HTTP JSON-RPC requests",
                true,
            )
            .with_cause(msg),
            Self::Rpc {
                error,
                error_message,
                ..
            } => {
                let retryable = matches!(error.as_str(), "tooBusy" | "slowDown" | "noCurrent");
                let cause_raw = error_message.as_deref().unwrap_or(error.as_str());
                StructuredError::new(
                    self.code(),
                    self.to_string(),
                    "inspect the stand-alone xrpld JSON-RPC request parameters and reported error identifier",
                    retryable,
                )
                .with_cause(cause_raw)
            }
            Self::Reply(msg) => StructuredError::new(
                self.code(),
                self.to_string(),
                "check that the loopback endpoint is an xrpld 3.4.1 JSON-RPC port returning standard result payloads",
                false,
            )
            .with_cause(msg),
            Self::Build(msg) => StructuredError::new(
                self.code(),
                self.to_string(),
                "verify transaction fields, currency codes, and signer keys before building or signing",
                false,
            )
            .with_cause(msg),
            Self::TxFailed { result, .. } => StructuredError::new(
                self.code(),
                self.to_string(),
                "inspect the validated engine result code, account balances, sequence, and enabled v2 amendments",
                false,
            )
            .with_cause(result),
        }
    }
}

/// Verified summary of the v2 amendment feature set and full feature map on a stand-alone ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentReport {
    /// Required `MAINNET_MATCHED_AMENDMENTS` observed with `enabled == true`.
    pub verified_enabled: Vec<String>,
    /// Forbidden non-Mainnet amendments confirmed `enabled == false` (or absent).
    pub rejected_non_mainnet: Vec<String>,
    /// Observed status of `fixBatchV1_2` (`DefaultYes` on `xrpld 3.4.1` and Testnet).
    pub testnet_only_default_yes_enabled: Option<bool>,
    /// Full `feature` RPC map (`name`, or hash fallback -> `enabled`) for CI logging/diffing.
    pub all_features: BTreeMap<String, bool>,
}

/// Trust line entry returned by `account_lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustLineBalance {
    /// Counterparty / issuer classic address.
    pub account: String,
    /// Currency code (`3`-char ASCII or 40-char uppercase hex).
    pub currency: String,
    /// Balance string relative to the queried account.
    pub balance: String,
    /// Configured credit limit string.
    pub limit: String,
}

/// Open DEX offer returned by `account_offers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerOffer {
    /// Offer `Sequence` number.
    pub seq: u32,
    /// Offer flags bitmask.
    pub flags: u32,
    /// Raw `taker_gets` JSON amount.
    pub taker_gets: Value,
    /// Raw `taker_pays` JSON amount.
    pub taker_pays: Value,
}

/// Result of submitting a signed transaction and closing the ledger via `ledger_accept`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTx {
    /// Local/ledger transaction hash (uppercase 64-hex).
    pub hash: String,
    /// Assigned account `Sequence` of this transaction (`OfferSequence` for offers/escrows).
    pub sequence: u32,
    /// Preliminary `submit` `engine_result`.
    pub preliminary: String,
    /// Validated `meta.TransactionResult` after `ledger_accept`.
    pub final_result: String,
    /// Whether `tx` RPC reported `validated == true`.
    pub validated: bool,
    /// Validated ledger sequence holding this transaction.
    pub ledger_index: u64,
    /// Full validated `meta` JSON object.
    pub meta: Value,
}

/// Private capability witness: only `StandAlone::with_transport` creates this token.
struct VerifiedStandAlone;

/// Checked JSON-RPC client restricted strictly to a loopback stand-alone `xrpld` node.
pub struct StandAlone<T: Transport> {
    url: String,
    transport: T,
    verified: VerifiedStandAlone,
}

impl<T: Transport> Debug for StandAlone<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StandAlone")
            .field("url", &self.url)
            .field("verified", &true)
            .finish()
    }
}

impl StandAlone<TimedUreq> {
    /// Connect to a loopback `http://127.0.0.1:<port>` or `http://localhost:<port>` endpoint
    /// and verify `server_info` satisfies Conflict 11 before returning.
    pub fn connect_loopback(url: &str, timeout: Duration) -> Result<Self, StandAloneError> {
        Self::with_transport(url, TimedUreq::new(timeout))
    }
}

impl<T: Transport> StandAlone<T> {
    /// Verify `url` and a live `server_info` call via `transport`, refusing any non-stand-alone
    /// target (including Mainnet `network_id == 0`, Testnet `1`, or missing id) after 1 RPC.
    pub fn with_transport(url: &str, transport: T) -> Result<Self, StandAloneError> {
        let normalized_url = require_loopback_http_url(url)?;
        let raw_info = raw_rpc_call(&transport, &normalized_url, "server_info", json!({}))?;
        let parsed_info = parse_server_info(&raw_info)?;
        require_standalone_info(&parsed_info)?;
        Ok(Self {
            url: normalized_url,
            transport,
            verified: VerifiedStandAlone,
        })
    }

    /// Normalized loopback RPC URL bound to this client.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Execute a raw `xrpld` JSON-RPC method against this verified stand-alone node.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, StandAloneError> {
        let _ = &self.verified;
        raw_rpc_call(&self.transport, &self.url, method, params)
    }

    /// Query and parse `server_info`.
    pub fn server_info(&self) -> Result<StandAloneServerInfo, StandAloneError> {
        let raw = self.call("server_info", json!({}))?;
        parse_server_info(&raw)
    }

    /// Advance the stand-alone ledger via the admin `ledger_accept` RPC method.
    pub fn ledger_accept(&self) -> Result<u64, StandAloneError> {
        let res = self.call("ledger_accept", json!({}))?;
        parse_u64_field(&res, "ledger_current_index")
            .ok_or_else(|| StandAloneError::Reply("missing ledger_current_index".into()))
    }

    /// Return `(validated_ledger_index, close_time)` in XRPL Ripple-epoch seconds.
    pub fn validated_close_time(&self) -> Result<(u64, u64), StandAloneError> {
        let res = self.call(
            "ledger",
            json!({
                "ledger_index": "validated",
                "transactions": false,
                "expand": false
            }),
        )?;
        let ledger = res
            .get("ledger")
            .ok_or_else(|| StandAloneError::Reply("missing ledger object".into()))?;
        let seq = parse_u64_field(ledger, "ledger_index")
            .or_else(|| parse_u64_field(&res, "ledger_index"))
            .ok_or_else(|| StandAloneError::Reply("missing validated ledger_index".into()))?;
        let close_time = parse_u64_field(ledger, "close_time")
            .ok_or_else(|| StandAloneError::Reply("missing close_time".into()))?;
        Ok((seq, close_time))
    }

    /// Verify that required Mainnet-matched amendments are enabled and forbidden non-Mainnet
    /// amendments (`ConfidentialTransfer`, `MPTokensV2`, `DynamicMPT`) are disabled.
    pub fn verify_v2_amendments(&self) -> Result<AmendmentReport, StandAloneError> {
        let res = self.call("feature", json!({}))?;
        let Some(map) = res
            .get("features")
            .and_then(Value::as_object)
            .or_else(|| res.as_object())
        else {
            return Err(StandAloneError::Reply(
                "feature response did not contain a JSON feature map".into(),
            ));
        };

        let mut by_name: BTreeMap<String, bool> = BTreeMap::new();
        for (key, entry) in map {
            if key == "status" || !entry.is_object() {
                continue;
            }
            let enabled = entry
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                by_name.insert(name.to_string(), enabled);
            } else {
                by_name.insert(key.clone(), enabled);
            }
        }

        let mut verified_enabled = Vec::new();
        let mut missing_or_disabled = Vec::new();
        for name in MAINNET_MATCHED_AMENDMENTS {
            if by_name.get(*name).copied() == Some(true) {
                verified_enabled.push((*name).to_string());
            } else {
                missing_or_disabled.push((*name).to_string());
            }
        }
        if !missing_or_disabled.is_empty() {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                "Mainnet-matched amendments not enabled: {}",
                missing_or_disabled.join(", ")
            ))));
        }

        let mut rejected_non_mainnet = Vec::new();
        for vetoed in VETOED_NON_MAINNET_AMENDMENTS {
            match by_name.get(*vetoed).copied() {
                Some(true) => {
                    return Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                        "forbidden non-Mainnet amendment {vetoed} is enabled"
                    ))));
                }
                _ => rejected_non_mainnet.push((*vetoed).to_string()),
            }
        }

        let testnet_only_default_yes_enabled =
            by_name.get(TESTNET_ONLY_DEFAULT_YES_AMENDMENT).copied();

        Ok(AmendmentReport {
            verified_enabled,
            rejected_non_mainnet,
            testnet_only_default_yes_enabled,
            all_features: by_name,
        })
    }

    /// Fetch the open-ledger next `Sequence` for `address`.
    pub fn account_sequence(&self, address: &str) -> Result<u32, StandAloneError> {
        let res = self.call(
            "account_info",
            json!({
                "account": address,
                "ledger_index": "current"
            }),
        )?;
        let seq = res
            .get("account_data")
            .and_then(|d| parse_u64_field(d, "Sequence"))
            .ok_or_else(|| StandAloneError::Reply("missing account_data.Sequence".into()))?;
        u32::try_from(seq)
            .map_err(|_| StandAloneError::Reply(format!("Sequence {seq} overflows u32")))
    }

    /// Fetch the validated XRP balance of `address` in drops.
    pub fn account_xrp_drops(&self, address: &str) -> Result<u64, StandAloneError> {
        let res = self.call(
            "account_info",
            json!({
                "account": address,
                "ledger_index": "validated"
            }),
        )?;
        let balance = res
            .get("account_data")
            .and_then(|d| d.get("Balance"))
            .and_then(Value::as_str)
            .ok_or_else(|| StandAloneError::Reply("missing account_data.Balance".into()))?;
        balance
            .parse::<u64>()
            .map_err(|err| StandAloneError::Reply(format!("invalid drops {balance:?}: {err}")))
    }

    /// Fetch validated trust lines for `address`.
    pub fn account_lines(&self, address: &str) -> Result<Vec<TrustLineBalance>, StandAloneError> {
        let res = self.call(
            "account_lines",
            json!({
                "account": address,
                "ledger_index": "validated"
            }),
        )?;
        let Some(lines) = res.get("lines").and_then(Value::as_array) else {
            return Err(StandAloneError::Reply("missing lines array".into()));
        };
        let mut out = Vec::with_capacity(lines.len());
        for item in lines {
            let account = item
                .get("account")
                .and_then(Value::as_str)
                .ok_or_else(|| StandAloneError::Reply("trust line missing account".into()))?;
            let currency = item
                .get("currency")
                .and_then(Value::as_str)
                .ok_or_else(|| StandAloneError::Reply("trust line missing currency".into()))?;
            let balance = item
                .get("balance")
                .and_then(Value::as_str)
                .ok_or_else(|| StandAloneError::Reply("trust line missing balance".into()))?;
            let limit = item
                .get("limit")
                .and_then(Value::as_str)
                .ok_or_else(|| StandAloneError::Reply("trust line missing limit".into()))?;
            out.push(TrustLineBalance {
                account: account.to_string(),
                currency: currency.to_string(),
                balance: balance.to_string(),
                limit: limit.to_string(),
            });
        }
        Ok(out)
    }

    /// Fetch open validated offers for `address`.
    pub fn account_offers(&self, address: &str) -> Result<Vec<LedgerOffer>, StandAloneError> {
        let res = self.call(
            "account_offers",
            json!({
                "account": address,
                "ledger_index": "validated"
            }),
        )?;
        let Some(offers) = res.get("offers").and_then(Value::as_array) else {
            return Err(StandAloneError::Reply("missing offers array".into()));
        };
        let mut out = Vec::with_capacity(offers.len());
        for item in offers {
            let seq_u64 = parse_u64_field(item, "seq")
                .ok_or_else(|| StandAloneError::Reply("offer missing seq".into()))?;
            let seq = u32::try_from(seq_u64)
                .map_err(|_| StandAloneError::Reply(format!("offer seq {seq_u64} > u32")))?;
            let flags_u64 = parse_u64_field(item, "flags").unwrap_or(0);
            let flags = u32::try_from(flags_u64)
                .map_err(|_| StandAloneError::Reply(format!("offer flags {flags_u64} > u32")))?;
            let taker_gets = item
                .get("taker_gets")
                .cloned()
                .ok_or_else(|| StandAloneError::Reply("offer missing taker_gets".into()))?;
            let taker_pays = item
                .get("taker_pays")
                .cloned()
                .ok_or_else(|| StandAloneError::Reply("offer missing taker_pays".into()))?;
            out.push(LedgerOffer {
                seq,
                flags,
                taker_gets,
                taker_pays,
            });
        }
        Ok(out)
    }

    /// Query the node's current `fee` RPC minimum base fee in drops.
    pub fn fee_drops(&self) -> Result<u64, StandAloneError> {
        let res = self.call("fee", json!({}))?;
        let base = res
            .get("drops")
            .and_then(|d| d.get("base_fee"))
            .and_then(Value::as_str)
            .ok_or_else(|| StandAloneError::Reply("missing drops.base_fee".into()))?;
        base.parse::<u64>()
            .map_err(|err| StandAloneError::Reply(format!("invalid base_fee {base:?}: {err}")))
    }

    /// Populate common fields (`NetworkID`, `Sequence`, `Fee`, `LastLedgerSequence`), sign
    /// with `signer`, verify local hash, and call `submit` without closing the ledger.
    pub fn submit_signed<'a, Tx, F>(
        &self,
        tx: &mut Tx,
        signer: &StandAloneAccount,
        fee_drops: Option<&str>,
    ) -> Result<(String, u32, Value), StandAloneError>
    where
        F: IntoEnumIterator + Serialize + Debug + PartialEq,
        Tx: Transaction<'a, F> + Model + Serialize + DeserializeOwned + Clone + Debug,
    {
        let sequence = match tx.get_common_fields().sequence {
            Some(seq) => seq,
            None => self.account_sequence(signer.address())?,
        };
        let (validated_seq, _) = self.validated_close_time()?;
        let last_ledger = u32::try_from(validated_seq.saturating_add(20))
            .map_err(|_| StandAloneError::Build("ledger sequence exceeds u32".into()))?;
        {
            let common = tx.get_mut_common_fields();
            common.network_id = Some(STANDALONE_NETWORK_ID);
            if common.sequence.is_none() {
                common.sequence = Some(sequence);
            }
            if common.fee.is_none() {
                common.fee = Some(XRPAmount::from(
                    fee_drops.unwrap_or(BASE_FEE_DROPS).to_string(),
                ));
            }
            if common.last_ledger_sequence.is_none() {
                common.last_ledger_sequence = Some(last_ledger);
            }
        }
        sign(tx, signer.wallet(), false)
            .map_err(|err| StandAloneError::Build(format!("xrpl signing failed: {err}")))?;
        let blob_hex = encode(tx)
            .map_err(|err| StandAloneError::Build(format!("binary encoding failed: {err}")))?;
        let blob_bytes = decode_hex(&blob_hex).map_err(StandAloneError::Build)?;
        let local_hash = transaction_hash(&blob_bytes);
        let reply = self.call("submit", json!({ "tx_blob": blob_hex }))?;
        Ok((local_hash, sequence, reply))
    }

    /// Submit `tx`, accept the ledger once when accepted into the open ledger, and inspect
    /// validated `tx` metadata (returning `validated: false` cleanly when rejected at submit).
    pub fn submit_and_accept<'a, Tx, F>(
        &self,
        tx: &mut Tx,
        signer: &StandAloneAccount,
        fee_drops: Option<&str>,
    ) -> Result<ValidatedTx, StandAloneError>
    where
        F: IntoEnumIterator + Serialize + Debug + PartialEq,
        Tx: Transaction<'a, F> + Model + Serialize + DeserializeOwned + Clone + Debug,
    {
        let (local_hash, sequence, submit_reply) = self.submit_signed(tx, signer, fee_drops)?;
        let preliminary = submit_reply
            .get("engine_result")
            .and_then(Value::as_str)
            .ok_or_else(|| StandAloneError::Reply("submit reply missing engine_result".into()))?
            .to_string();
        let accepted = submit_reply
            .get("accepted")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !accepted {
            return Ok(ValidatedTx {
                hash: local_hash,
                sequence,
                preliminary: preliminary.clone(),
                final_result: preliminary,
                validated: false,
                ledger_index: 0,
                meta: Value::Object(serde_json::Map::new()),
            });
        }

        let _ = self.ledger_accept()?;
        let tx_reply = match self.call("tx", json!({ "transaction": local_hash })) {
            Ok(reply) => reply,
            Err(StandAloneError::Rpc { error, .. }) if error == "txnNotFound" => {
                return Ok(ValidatedTx {
                    hash: local_hash,
                    sequence,
                    preliminary: preliminary.clone(),
                    final_result: preliminary,
                    validated: false,
                    ledger_index: 0,
                    meta: Value::Object(serde_json::Map::new()),
                });
            }
            Err(err) => return Err(err),
        };
        let validated = tx_reply
            .get("validated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let ledger_index = parse_u64_field(&tx_reply, "ledger_index").unwrap_or(0);
        let meta = tx_reply
            .get("meta")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let final_result = meta
            .get("TransactionResult")
            .and_then(Value::as_str)
            .unwrap_or(&preliminary)
            .to_string();

        Ok(ValidatedTx {
            hash: local_hash,
            sequence,
            preliminary,
            final_result,
            validated,
            ledger_index,
            meta,
        })
    }

    /// Submit `tx`, close the ledger, and require `validated == true` with `tesSUCCESS`.
    pub fn submit_and_accept_ok<'a, Tx, F>(
        &self,
        tx: &mut Tx,
        signer: &StandAloneAccount,
        fee_drops: Option<&str>,
    ) -> Result<ValidatedTx, StandAloneError>
    where
        F: IntoEnumIterator + Serialize + Debug + PartialEq,
        Tx: Transaction<'a, F> + Model + Serialize + DeserializeOwned + Clone + Debug,
    {
        let tx_type = tx.get_transaction_type().to_string();
        let receipt = self.submit_and_accept(tx, signer, fee_drops)?;
        if !receipt.validated || receipt.final_result != "tesSUCCESS" {
            return Err(StandAloneError::TxFailed {
                tx_type,
                result: receipt.final_result,
            });
        }
        Ok(receipt)
    }

    /// Fund `destination` with `amount_drops` XRP from the stand-alone genesis account.
    pub fn fund_account(
        &self,
        destination: &str,
        amount_drops: u64,
    ) -> Result<ValidatedTx, StandAloneError> {
        let genesis = StandAloneAccount::genesis()?;
        let mut payment = Payment::new(
            Cow::Owned(genesis.address().to_string()),
            None,
            None,
            Some(FlagCollection::from(Vec::new())),
            None,
            None,
            None,
            None,
            None,
            None,
            Amount::XRPAmount(XRPAmount::from(amount_drops.to_string())),
            Cow::Owned(destination.to_string()),
            None,
            None,
            None,
            None,
            None,
        );
        self.submit_and_accept_ok(&mut payment, &genesis, None)
    }

    /// Generate a fresh `ed25519` account and fund it from the stand-alone genesis root.
    pub fn fund_new_account(
        &self,
        amount_drops: u64,
    ) -> Result<(StandAloneAccount, ValidatedTx), StandAloneError> {
        let account = StandAloneAccount::generate_ed25519()?;
        let receipt = self.fund_account(account.address(), amount_drops)?;
        Ok((account, receipt))
    }
}

fn raw_rpc_call<T: Transport>(
    transport: &T,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, StandAloneError> {
    let body = json!({
        "method": method,
        "params": [params]
    });
    let reply = transport
        .post(url, &body)
        .map_err(StandAloneError::Transport)?;
    let result = reply
        .get("result")
        .cloned()
        .ok_or_else(|| StandAloneError::Reply(format!("{method} reply missing result object")))?;
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        let error_message = result
            .get("error_message")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        return Err(StandAloneError::Rpc {
            method: method.to_string(),
            error: error.to_string(),
            error_message,
        });
    }
    if result.get("status").and_then(Value::as_str) == Some("error") {
        return Err(StandAloneError::Rpc {
            method: method.to_string(),
            error: "error".into(),
            error_message: result
                .get("error_message")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        });
    }
    Ok(result)
}

fn parse_server_info(raw: &Value) -> Result<StandAloneServerInfo, StandAloneError> {
    let info = raw
        .get("info")
        .ok_or_else(|| StandAloneError::Reply("server_info missing info object".into()))?;
    let network_id = parse_u64_field(info, "network_id").and_then(|v| u32::try_from(v).ok());
    let validation_quorum =
        parse_u64_field(info, "validation_quorum").and_then(|v| u32::try_from(v).ok());
    let peers = parse_u64_field(info, "peers");
    let server_state = info
        .get("server_state")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let validated_ledger_seq = info
        .get("validated_ledger")
        .and_then(|v| parse_u64_field(v, "seq"));
    let complete_ledgers = info
        .get("complete_ledgers")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let build_version = info
        .get("build_version")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    Ok(StandAloneServerInfo {
        network_id,
        validation_quorum,
        peers,
        server_state,
        validated_ledger_seq,
        complete_ledgers,
        build_version,
    })
}

fn parse_u64_field(obj: &Value, key: &str) -> Option<u64> {
    let val = obj.get(key)?;
    val.as_u64()
        .or_else(|| val.as_str().and_then(|s| s.parse::<u64>().ok()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::amendment_id_hex;

    struct ScriptedTransport {
        replies: RefCell<VecDeque<Result<Value, String>>>,
        requests: RefCell<Vec<(String, Value)>>,
    }

    impl ScriptedTransport {
        fn new(replies: Vec<Result<Value, String>>) -> Self {
            Self {
                replies: RefCell::new(VecDeque::from(replies)),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.borrow().len()
        }
    }

    impl Transport for ScriptedTransport {
        fn post(&self, url: &str, body: &Value) -> Result<Value, String> {
            self.requests
                .borrow_mut()
                .push((url.to_string(), body.clone()));
            self.replies
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("no scripted reply remaining".into()))
        }
    }

    fn standalone_server_info_reply(
        network_id: Option<u32>,
        quorum: Option<u32>,
        peers: Option<u64>,
        state: Option<&str>,
    ) -> Value {
        let mut info = serde_json::Map::new();
        if let Some(id) = network_id {
            info.insert("network_id".into(), json!(id));
        }
        if let Some(q) = quorum {
            info.insert("validation_quorum".into(), json!(q));
        }
        if let Some(p) = peers {
            info.insert("peers".into(), json!(p));
        }
        if let Some(s) = state {
            info.insert("server_state".into(), json!(s));
        }
        info.insert(
            "validated_ledger".into(),
            json!({ "seq": 2, "complete_ledgers": "1-2" }),
        );
        info.insert("complete_ledgers".into(), json!("1-2"));
        info.insert("build_version".into(), json!("3.4.1"));
        json!({
            "result": {
                "status": "success",
                "info": Value::Object(info)
            }
        })
    }

    #[test]
    fn genesis_and_ed25519_accounts_derive_and_redact_seeds() {
        let genesis = match StandAloneAccount::genesis() {
            Ok(acc) => acc,
            Err(err) => panic!("genesis failed: {err}"),
        };
        assert_eq!(genesis.address(), GENESIS_ACCOUNT_ADDRESS);
        assert!(genesis.public_key().starts_with("03"));

        let acc = match StandAloneAccount::generate_ed25519() {
            Ok(acc) => acc,
            Err(err) => panic!("ed25519 gen failed: {err}"),
        };
        assert!(acc.address().starts_with('r'));
        let secret_prefix = &acc.wallet().seed[..6];
        let debug_text = format!("{acc:?}");
        assert!(debug_text.contains(acc.address()));
        assert!(!debug_text.contains(secret_prefix));

        let roundtrip = StandAloneAccount::from_ed25519_seed(&acc.wallet().seed);
        assert!(roundtrip.is_ok());
        assert!(StandAloneAccount::from_ed25519_seed(&genesis.wallet().seed).is_err());
        assert!(StandAloneAccount::from_ed25519_seed("not_a_seed_secret_bytes").is_err());
    }

    #[test]
    fn with_transport_refuses_wrong_network_id_after_single_rpc() {
        for bad_id in [None, Some(0), Some(1), Some(42)] {
            let transport = ScriptedTransport::new(vec![Ok(standalone_server_info_reply(
                bad_id,
                Some(0),
                Some(0),
                Some("full"),
            ))]);
            let res = StandAlone::with_transport("http://127.0.0.1:5005", transport);
            assert!(res.is_err(), "must reject network_id {bad_id:?}");
        }

        let transport_non_loopback = ScriptedTransport::new(vec![]);
        let res = StandAlone::with_transport("https://127.0.0.1:5005", transport_non_loopback);
        assert!(res.is_err());

        let transport_quorum = ScriptedTransport::new(vec![Ok(standalone_server_info_reply(
            Some(STANDALONE_NETWORK_ID),
            Some(2),
            Some(0),
            Some("full"),
        ))]);
        assert!(StandAlone::with_transport("http://127.0.0.1:5005", transport_quorum).is_err());

        let transport_peers = ScriptedTransport::new(vec![Ok(standalone_server_info_reply(
            Some(STANDALONE_NETWORK_ID),
            Some(0),
            Some(3),
            Some("full"),
        ))]);
        assert!(StandAlone::with_transport("http://127.0.0.1:5005", transport_peers).is_err());
    }

    #[test]
    fn connect_loopback_enforces_guard_via_real_http_socket() {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(err) => panic!("loopback bind failed: {err}"),
        };
        let addr = match listener.local_addr() {
            Ok(a) => a,
            Err(err) => panic!("local_addr failed: {err}"),
        };
        let url = format!("http://{addr}");

        let server = thread::spawn(move || {
            for response_json in [
                standalone_server_info_reply(Some(0), Some(0), Some(0), Some("full")),
                standalone_server_info_reply(
                    Some(STANDALONE_NETWORK_ID),
                    Some(0),
                    Some(0),
                    Some("full"),
                ),
            ] {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut req_bytes = Vec::new();
                    let mut chunk = [0u8; 1024];
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                req_bytes.extend_from_slice(&chunk[..n]);
                                if let Some(hdr_end) =
                                    req_bytes.windows(4).position(|w| w == b"\r\n\r\n")
                                {
                                    let headers = String::from_utf8_lossy(&req_bytes[..hdr_end]);
                                    let content_len = headers
                                        .lines()
                                        .find_map(|line| {
                                            let (k, v) = line.split_once(':')?;
                                            k.trim()
                                                .eq_ignore_ascii_case("content-length")
                                                .then(|| v.trim().parse::<usize>().ok())?
                                        })
                                        .unwrap_or(0);
                                    if req_bytes.len() >= hdr_end + 4 + content_len {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    let body = response_json.to_string();
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(body.as_bytes());
                    let _ = stream.flush();
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                }
            }
        });

        let rejected = StandAlone::connect_loopback(&url, Duration::from_secs(2));
        assert!(rejected.is_err());

        let accepted = match StandAlone::connect_loopback(&url, Duration::from_secs(2)) {
            Ok(client) => client,
            Err(err) => panic!("expected loopback connection to succeed: {err}"),
        };
        assert_eq!(accepted.url(), format!("{url}/"));
        let _ = server.join();
    }

    #[test]
    fn verify_v2_amendments_and_queries_work_with_scripted_transport() {
        let mut features = serde_json::Map::new();
        for name in MAINNET_MATCHED_AMENDMENTS {
            features.insert(
                amendment_id_hex(name),
                json!({ "name": name, "enabled": true, "supported": true }),
            );
        }
        features.insert(
            amendment_id_hex("ConfidentialTransfer"),
            json!({ "name": "ConfidentialTransfer", "enabled": false, "supported": true }),
        );
        features.insert(
            amendment_id_hex("fixBatchV1_2"),
            json!({ "name": "fixBatchV1_2", "enabled": true, "supported": true }),
        );

        let tx_hash = "A".repeat(64);
        let transport = ScriptedTransport::new(vec![
            Ok(standalone_server_info_reply(
                Some(STANDALONE_NETWORK_ID),
                Some(0),
                Some(0),
                Some("full"),
            )),
            Ok(json!({ "result": { "status": "success", "features": Value::Object(features) } })),
            Ok(json!({ "result": { "status": "success", "ledger_current_index": 4 } })),
            Ok(json!({
                "result": {
                    "status": "success",
                    "ledger": { "ledger_index": "4", "close_time": 815_000_000 }
                }
            })),
            Ok(json!({
                "result": {
                    "status": "success",
                    "drops": { "base_fee": "10" }
                }
            })),
            Ok(json!({
                "result": {
                    "status": "success",
                    "account_data": { "Sequence": 1, "Balance": "100000000" }
                }
            })),
            Ok(json!({
                "result": {
                    "status": "success",
                    "lines": [{ "account": GENESIS_ACCOUNT_ADDRESS, "currency": "RAW", "balance": "250", "limit": "100000" }]
                }
            })),
            Ok(json!({
                "result": {
                    "status": "success",
                    "offers": [{ "seq": 5, "flags": 0, "taker_gets": "1000", "taker_pays": "2000" }]
                }
            })),
            // fund_new_account flow: sequence -> validated_close_time -> submit -> ledger_accept -> tx
            Ok(json!({ "result": { "status": "success", "account_data": { "Sequence": 1 } } })),
            Ok(
                json!({ "result": { "status": "success", "ledger": { "ledger_index": 4, "close_time": 815_000_001 } } }),
            ),
            Ok(
                json!({ "result": { "status": "success", "accepted": true, "engine_result": "tesSUCCESS" } }),
            ),
            Ok(json!({ "result": { "status": "success", "ledger_current_index": 5 } })),
            Ok(json!({
                "result": {
                    "status": "success",
                    "validated": true,
                    "ledger_index": 4,
                    "hash": tx_hash,
                    "meta": { "TransactionResult": "tesSUCCESS" }
                }
            })),
        ]);

        let client = match StandAlone::with_transport("http://localhost:5005", transport) {
            Ok(c) => c,
            Err(err) => panic!("client init failed: {err}"),
        };
        let report = match client.verify_v2_amendments() {
            Ok(r) => r,
            Err(err) => panic!("amendments failed: {err}"),
        };
        assert_eq!(
            report.verified_enabled.len(),
            MAINNET_MATCHED_AMENDMENTS.len()
        );
        assert_eq!(
            report.all_features.len(),
            MAINNET_MATCHED_AMENDMENTS.len() + 2
        );
        assert_eq!(
            report.all_features.get("ConfidentialTransfer"),
            Some(&false)
        );
        assert_eq!(
            report.all_features.get(TESTNET_ONLY_DEFAULT_YES_AMENDMENT),
            Some(&true)
        );
        assert_eq!(client.ledger_accept(), Ok(4));
        assert_eq!(client.validated_close_time(), Ok((4, 815_000_000)));
        assert_eq!(client.fee_drops(), Ok(10));
        assert_eq!(
            client.account_xrp_drops(GENESIS_ACCOUNT_ADDRESS),
            Ok(100_000_000)
        );
        assert_eq!(
            client
                .account_lines(GENESIS_ACCOUNT_ADDRESS)
                .map(|v| v.len()),
            Ok(1)
        );
        assert_eq!(
            client
                .account_offers(GENESIS_ACCOUNT_ADDRESS)
                .map(|v| v.len()),
            Ok(1)
        );

        let (acct, receipt) = match client.fund_new_account(200_000_000) {
            Ok(pair) => pair,
            Err(err) => panic!("fund_new_account failed: {err}"),
        };
        assert_eq!(receipt.sequence, 1);
        assert!(receipt.validated);
        assert_eq!(receipt.final_result, "tesSUCCESS");
        assert!(acct.address().starts_with('r'));
        assert_eq!(client.transport.request_count(), 13);
    }

    #[test]
    fn submit_and_accept_handles_rejected_tel_fee_and_ok_fails() {
        let genesis = match StandAloneAccount::genesis() {
            Ok(g) => g,
            Err(err) => panic!("genesis failed: {err}"),
        };
        let dest = match StandAloneAccount::generate_ed25519() {
            Ok(d) => d,
            Err(err) => panic!("gen failed: {err}"),
        };
        let transport = ScriptedTransport::new(vec![
            Ok(standalone_server_info_reply(
                Some(STANDALONE_NETWORK_ID),
                Some(0),
                Some(0),
                Some("full"),
            )),
            // submit_and_accept: sequence -> validated_close_time -> submit (accepted: false)
            Ok(json!({ "result": { "status": "success", "account_data": { "Sequence": 7 } } })),
            Ok(
                json!({ "result": { "status": "success", "ledger": { "ledger_index": 3, "close_time": 815_000_001 } } }),
            ),
            Ok(
                json!({ "result": { "status": "success", "accepted": false, "engine_result": "telINSUF_FEE_P" } }),
            ),
            // submit_and_accept_ok with telINSUF_FEE_P must return TxFailed error
            Ok(json!({ "result": { "status": "success", "account_data": { "Sequence": 8 } } })),
            Ok(
                json!({ "result": { "status": "success", "ledger": { "ledger_index": 3, "close_time": 815_000_001 } } }),
            ),
            Ok(
                json!({ "result": { "status": "success", "accepted": false, "engine_result": "telINSUF_FEE_P" } }),
            ),
        ]);
        let client = match StandAlone::with_transport("http://127.0.0.1:5005", transport) {
            Ok(c) => c,
            Err(err) => panic!("init failed: {err}"),
        };
        let mut p1 = Payment::new(
            Cow::Owned(genesis.address().to_string()),
            None,
            None,
            Some(FlagCollection::from(Vec::new())),
            None,
            None,
            None,
            None,
            None,
            None,
            Amount::XRPAmount(XRPAmount::from("1000000".to_string())),
            Cow::Owned(dest.address().to_string()),
            None,
            None,
            None,
            None,
            None,
        );
        let rejected = match client.submit_and_accept(&mut p1, &genesis, Some("10")) {
            Ok(v) => v,
            Err(err) => panic!("expected ValidatedTx wrapper for telINSUF_FEE_P: {err}"),
        };
        assert_eq!(rejected.sequence, 7);
        assert!(!rejected.validated);
        assert_eq!(rejected.final_result, "telINSUF_FEE_P");

        let mut p2 = Payment::new(
            Cow::Owned(genesis.address().to_string()),
            None,
            None,
            Some(FlagCollection::from(Vec::new())),
            None,
            None,
            None,
            None,
            None,
            None,
            Amount::XRPAmount(XRPAmount::from("1000000".to_string())),
            Cow::Owned(dest.address().to_string()),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(matches!(
            client.submit_and_accept_ok(&mut p2, &genesis, Some("10")),
            Err(StandAloneError::TxFailed { result, .. }) if result == "telINSUF_FEE_P"
        ));
    }

    #[test]
    fn verify_v2_amendments_rejects_vetoed_or_missing_feature() {
        let mut missing_map = serde_json::Map::new();
        missing_map.insert("status".into(), json!("success"));
        let transport_missing = ScriptedTransport::new(vec![
            Ok(standalone_server_info_reply(
                Some(STANDALONE_NETWORK_ID),
                Some(0),
                Some(0),
                Some("full"),
            )),
            Ok(json!({ "result": Value::Object(missing_map) })),
        ]);
        let client_missing =
            match StandAlone::with_transport("http://127.0.0.1:5005", transport_missing) {
                Ok(c) => c,
                Err(err) => panic!("init failed: {err}"),
            };
        assert!(client_missing.verify_v2_amendments().is_err());

        let mut veto_map = serde_json::Map::new();
        veto_map.insert("status".into(), json!("success"));
        for name in MAINNET_MATCHED_AMENDMENTS {
            veto_map.insert(
                amendment_id_hex(name),
                json!({ "name": name, "enabled": true, "supported": true }),
            );
        }
        veto_map.insert(
            amendment_id_hex("ConfidentialTransfer"),
            json!({ "name": "ConfidentialTransfer", "enabled": true, "supported": true }),
        );
        let transport_veto = ScriptedTransport::new(vec![
            Ok(standalone_server_info_reply(
                Some(STANDALONE_NETWORK_ID),
                Some(0),
                Some(0),
                Some("full"),
            )),
            Ok(json!({ "result": Value::Object(veto_map) })),
        ]);
        let client_veto = match StandAlone::with_transport("http://127.0.0.1:5005", transport_veto)
        {
            Ok(c) => c,
            Err(err) => panic!("init failed: {err}"),
        };
        assert!(client_veto.verify_v2_amendments().is_err());
    }

    #[test]
    fn standalone_error_structured_shape_and_secret_scrubbing() {
        let ed_seed = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let secp_seed = "sp5fghtJtpUorTwvof1NpDXAzNwf5";
        let cases = [
            (
                StandAloneError::NotStandAlone(NotStandAlone::new("wrong network_id")),
                "NOT_STANDALONE",
                false,
            ),
            (
                StandAloneError::Ledger(format!("bad seed {ed_seed}")),
                "STANDALONE_ACCOUNT",
                false,
            ),
            (
                StandAloneError::Transport(format!("Bearer tok_123 {secp_seed}")),
                "STANDALONE_TRANSPORT",
                true,
            ),
            (
                StandAloneError::Rpc {
                    method: "server_info".into(),
                    error: "tooBusy".into(),
                    error_message: Some("try again".into()),
                },
                "STANDALONE_RPC",
                true,
            ),
            (
                StandAloneError::Rpc {
                    method: "tx".into(),
                    error: "txnNotFound".into(),
                    error_message: None,
                },
                "STANDALONE_RPC",
                false,
            ),
            (
                StandAloneError::Reply("OPENROUTER_API_KEY=sk-or-secret999".into()),
                "STANDALONE_BAD_REPLY",
                false,
            ),
            (
                StandAloneError::Build(format!("sign failed {ed_seed}")),
                "STANDALONE_TX_BUILD",
                false,
            ),
            (
                StandAloneError::TxFailed {
                    tx_type: "Payment".into(),
                    result: "tecUNFUNDED_PAYMENT".into(),
                },
                "STANDALONE_TX_FAILED",
                false,
            ),
        ];

        for (err, expected_code, expect_retryable) in cases {
            assert_eq!(err.code(), expected_code);
            let se = err.to_structured_error();
            assert_eq!(se.code, expected_code);
            assert!(!se.hint.is_empty());
            assert!(se.cause.is_some());
            assert_eq!(se.retryable, expect_retryable);
            let json = se
                .to_json_string()
                .unwrap_or_else(|e| panic!("json serialize: {e}"));
            let text = se.format_text();
            for rendered in [&json, &text, &se.message] {
                assert!(!rendered.contains(ed_seed), "leaked ed seed: {rendered}");
                assert!(
                    !rendered.contains(secp_seed),
                    "leaked secp seed: {rendered}"
                );
                assert!(!rendered.contains("tok_123"), "leaked bearer: {rendered}");
                assert!(
                    !rendered.contains("sk-or-secret999"),
                    "leaked api key: {rendered}"
                );
            }
        }
    }
}
