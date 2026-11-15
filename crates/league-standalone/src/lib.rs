//! Stand-alone `xrpld` harness and Seed-27 Section B item B3 conformance tests.
//!
//! Enforces the `docs/design-v2.md` section 9 Conflict 11 boundary:
//! `league-ledger` remains strictly bound to XRPL Testnet (`network_id == 1`),
//! while this crate talks only to a loopback `http://127.0.0.1:<port>` or
//! `http://localhost:<port>` stand-alone `xrpld` process reporting
//! `network_id == 21337`, `validation_quorum == 0`, and `peers == 0`.

use std::fmt::{self, Write as _};

use sha2::{Digest as _, Sha512};

pub mod client;
pub mod conformance;
pub mod docker;

/// Fixed XRPL `[network_id]` required by the stand-alone harness.
pub const STANDALONE_NETWORK_ID: u32 = 21_337;

/// Official `xrpld` Docker image repository pinned by this harness.
pub const OFFICIAL_XRPLD_IMAGE_REPO: &str = "docker.io/xrplf/xrpld";

/// Official `xrpld` image tag pinned by this harness (matches Testnet 3.4.1).
pub const OFFICIAL_XRPLD_IMAGE_TAG: &str = "3.4.1";

/// Pinned SHA-256 manifest digest for `docker.io/xrplf/xrpld:3.4.1`.
pub const OFFICIAL_XRPLD_IMAGE_DIGEST: &str =
    "sha256:457f095d1948a354119187df962b60504a4791521f8225a28ba2345c34a6b0ce";

/// Full immutable OCI image reference passed to `docker pull` and `docker run`.
pub const OFFICIAL_XRPLD_PINNED_REF: &str = "docker.io/xrplf/xrpld:3.4.1@sha256:457f095d1948a354119187df962b60504a4791521f8225a28ba2345c34a6b0ce";

/// Container label applied to every launched container so cleanup checks can find leaks.
pub const STANDALONE_HARNESS_LABEL: &str = "merchant-league.harness=standalone";

/// Standard XRPL stand-alone genesis root classic address (`masterpassphrase`).
pub const GENESIS_ACCOUNT_ADDRESS: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";

/// Amendments enabled on Mainnet (`docs/design-v2.md` section 8) that default to `DefaultNo`
/// in `xrpld 3.4.1` and therefore must be listed under `[amendments]` at genesis.
pub const MAINNET_MATCHED_AMENDMENTS: &[&str] = &[
    "BatchV1_1",
    "TokenEscrow",
    "fixTokenEscrowV1",
    "PermissionedDomains",
    "PermissionedDEX",
    "Credentials",
    "PermissionDelegationV1_1",
    "PriceOracle",
    "fixPriceOracleOrder",
    "DeepFreeze",
    "AMMClawback",
    "fixAMMClawbackRounding",
];

/// Forbidden or non-Mainnet `DefaultNo` amendments verified to remain `enabled == false`
/// on a stand-alone ledger (`docs/design-v2.md` section 8).
pub const VETOED_NON_MAINNET_AMENDMENTS: &[&str] =
    &["ConfidentialTransfer", "MPTokensV2", "DynamicMPT"];

/// Amendment enabled on both public Testnet (`3.4.1`) and stand-alone `xrpld 3.4.1` via
/// `DefaultYes` while still `enabled=false` on Mainnet (`docs/design-v2.md` section 8).
pub const TESTNET_ONLY_DEFAULT_YES_AMENDMENT: &str = "fixBatchV1_2";

/// Error returned when an endpoint URL or `server_info` snapshot violates Conflict 11.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotStandAlone {
    /// Human-readable reason explaining why the target was refused.
    pub reason: String,
}

impl NotStandAlone {
    /// Construct a guard error with a fixed explanation.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for NotStandAlone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a local stand-alone xrpld node: {}", self.reason)
    }
}

impl std::error::Error for NotStandAlone {}

/// Parsed `info` subset from `server_info` checked by `require_standalone_info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandAloneServerInfo {
    /// Reported `info.network_id` (`21337` required).
    pub network_id: Option<u32>,
    /// Reported `info.validation_quorum` (`0` required in stand-alone mode).
    pub validation_quorum: Option<u32>,
    /// Reported `info.peers` (`0` or absent -> `0` required in single-node mode).
    pub peers: Option<u64>,
    /// Reported `info.server_state` (must be present and non-empty).
    pub server_state: Option<String>,
    /// Latest validated ledger sequence if present.
    pub validated_ledger_seq: Option<u64>,
    /// Complete ledgers range string if present.
    pub complete_ledgers: Option<String>,
    /// Build version string reported by `xrpld`.
    pub build_version: Option<String>,
}

/// Normalize and verify that `url` targets a plain-HTTP loopback `ip:port` endpoint.
pub fn require_loopback_http_url(url: &str) -> Result<String, NotStandAlone> {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.bytes().any(|b| b.is_ascii_control() || b == b' ') {
        return Err(NotStandAlone::new(
            "endpoint URL must not be empty or contain spaces/control characters",
        ));
    }
    let Some(rest) = trimmed.strip_prefix("http://") else {
        return Err(NotStandAlone::new(
            "stand-alone endpoint must use plain http:// on loopback",
        ));
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains(['/', '?', '#', '@']) {
        return Err(NotStandAlone::new(
            "stand-alone endpoint must not carry userinfo, paths, queries, or fragments",
        ));
    }
    let Some((host, port_text)) = rest.rsplit_once(':') else {
        return Err(NotStandAlone::new(
            "stand-alone endpoint must include an explicit host and port",
        ));
    };
    if host != "127.0.0.1" && host != "localhost" {
        return Err(NotStandAlone::new(format!(
            "refusing non-loopback host {host:?}; only 127.0.0.1 or localhost is permitted"
        )));
    }
    let port: u16 = port_text
        .parse()
        .map_err(|_| NotStandAlone::new(format!("invalid port {port_text:?}")))?;
    if port == 0 {
        return Err(NotStandAlone::new("port must be non-zero"));
    }
    Ok(format!("http://{host}:{port}/"))
}

/// Check Conflict 11 invariants on a `server_info` response before any write call proceeds.
pub fn require_standalone_info(info: &StandAloneServerInfo) -> Result<(), NotStandAlone> {
    match info.network_id {
        Some(STANDALONE_NETWORK_ID) => {}
        Some(0) => {
            return Err(NotStandAlone::new("refusing network_id 0 (Mainnet)"));
        }
        Some(1) => {
            return Err(NotStandAlone::new(
                "refusing network_id 1 (Testnet); use league-ledger instead",
            ));
        }
        Some(other) => {
            return Err(NotStandAlone::new(format!(
                "refusing unexpected network_id {other}; expected {STANDALONE_NETWORK_ID}"
            )));
        }
        None => {
            return Err(NotStandAlone::new(
                "missing network_id in server_info; refusing unconfigured node",
            ));
        }
    }

    if info.validation_quorum != Some(0) {
        return Err(NotStandAlone::new(format!(
            "stand-alone mode requires validation_quorum == 0, got {:?}",
            info.validation_quorum
        )));
    }
    if info.peers.unwrap_or(0) != 0 {
        return Err(NotStandAlone::new(format!(
            "stand-alone mode requires peers == 0, got {:?}",
            info.peers
        )));
    }
    match info.server_state.as_deref() {
        Some(state) if !state.trim().is_empty() => Ok(()),
        _ => Err(NotStandAlone::new(
            "missing server_state in stand-alone server_info",
        )),
    }
}

/// Compute the uppercase 64-character SHA-512Half amendment ID used in `xrpld.cfg`.
#[must_use]
pub fn amendment_id_hex(name: &str) -> String {
    let digest = Sha512::digest(name.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in &digest[..32] {
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// Build the isolated `xrpld.cfg` written into `/tmp/xrpld/xrpld.cfg` inside the container.
pub fn stand_alone_xrpld_cfg(network_id: u32) -> Result<String, NotStandAlone> {
    if network_id != STANDALONE_NETWORK_ID {
        return Err(NotStandAlone::new(format!(
            "stand-alone xrpld.cfg requires network_id {STANDALONE_NETWORK_ID}, got {network_id}"
        )));
    }
    let mut cfg = String::new();
    cfg.push_str(
        "[server]\n\
         port_rpc_admin\n\n\
         [port_rpc_admin]\n\
         port = 5005\n\
         ip = 0.0.0.0\n\
         admin = 0.0.0.0\n\
         protocol = http\n\n\
         [node_size]\n\
         tiny\n\n\
         [node_db]\n\
         type = Memory\n\
         path = /tmp/xrpld/db\n\n\
         [database_path]\n\
         /tmp/xrpld/db\n\n\
         [debug_logfile]\n\
         /tmp/xrpld/debug.log\n\n",
    );
    let _ = writeln!(cfg, "[network_id]\n{network_id}\n");
    cfg.push_str(
        "[rpc_startup]\n\
         { \"command\": \"log_level\", \"severity\": \"warning\" }\n\n\
         [amendments]\n",
    );
    for name in MAINNET_MATCHED_AMENDMENTS {
        let _ = writeln!(cfg, "{} {name}", amendment_id_hex(name));
    }
    Ok(cfg)
}

/// Encode an XRPL issued currency ticker: 3-char standard ASCII or 40-char padded uppercase hex.
///
/// Four-character codes such as `CRED` (`docs/design-v2.md` section 1.2) require XRPL's
/// 160-bit hexadecimal encoding (`4352454400000000000000000000000000000000`).
pub fn encode_currency_code(code: &str) -> Result<String, NotStandAlone> {
    if code.is_empty() || code == "XRP" {
        return Err(NotStandAlone::new(format!(
            "currency code {code:?} is not a valid issued currency"
        )));
    }
    if code.len() == 3
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"?!@#$%^&*<>(){}[]|".contains(&b))
    {
        return Ok(code.to_string());
    }
    if code.len() <= 20
        && code
            .bytes()
            .all(|b| b.is_ascii() && !b.is_ascii_control() && b != b' ')
        && code.as_bytes()[0] != 0
    {
        let mut bytes = [0u8; 20];
        bytes[..code.len()].copy_from_slice(code.as_bytes());
        let mut hex = String::with_capacity(40);
        for b in bytes {
            let _ = write!(hex, "{b:02X}");
        }
        return Ok(hex);
    }
    Err(NotStandAlone::new(format!(
        "unsupported issued currency symbol {code:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_info() -> StandAloneServerInfo {
        StandAloneServerInfo {
            network_id: Some(STANDALONE_NETWORK_ID),
            validation_quorum: Some(0),
            peers: Some(0),
            server_state: Some("full".to_string()),
            validated_ledger_seq: Some(2),
            complete_ledgers: Some("1-2".to_string()),
            build_version: Some("3.4.1".to_string()),
        }
    }

    #[test]
    fn loopback_url_accepts_only_explicit_loopback_http() {
        assert_eq!(
            require_loopback_http_url("http://127.0.0.1:5005"),
            Ok("http://127.0.0.1:5005/".to_string())
        );
        assert_eq!(
            require_loopback_http_url("http://localhost:32768/"),
            Ok("http://localhost:32768/".to_string())
        );
        for bad in [
            "",
            "https://127.0.0.1:5005",
            "http://0.0.0.0:5005",
            "http://192.168.1.5:5005",
            "http://example.invalid:5005",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "http://127.0.0.1:5005/rpc",
            "http://user@127.0.0.1:5005",
            "http://127.0.0.1:notaport",
        ] {
            assert!(
                require_loopback_http_url(bad).is_err(),
                "expected refusal for {bad:?}"
            );
        }
    }

    #[test]
    fn require_standalone_info_refuses_mainnet_testnet_missing_and_networked_nodes() {
        assert_eq!(require_standalone_info(&valid_info()), Ok(()));

        for bad_id in [None, Some(0), Some(1), Some(2), Some(u32::MAX)] {
            let mut info = valid_info();
            info.network_id = bad_id;
            assert!(
                require_standalone_info(&info).is_err(),
                "expected network_id {bad_id:?} refusal"
            );
        }

        let mut bad_quorum = valid_info();
        bad_quorum.validation_quorum = Some(1);
        assert!(require_standalone_info(&bad_quorum).is_err());

        let mut missing_quorum = valid_info();
        missing_quorum.validation_quorum = None;
        assert!(require_standalone_info(&missing_quorum).is_err());

        let mut has_peers = valid_info();
        has_peers.peers = Some(2);
        assert!(require_standalone_info(&has_peers).is_err());

        let mut empty_state = valid_info();
        empty_state.server_state = Some("   ".to_string());
        assert!(require_standalone_info(&empty_state).is_err());
    }

    #[test]
    fn amendment_hashes_and_cfg_match_expected_format() {
        let cred_hash = amendment_id_hex("Credentials");
        assert_eq!(cred_hash.len(), 64);
        assert!(cred_hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(
            amendment_id_hex("BatchV1_1"),
            amendment_id_hex("ConfidentialTransfer")
        );

        assert!(stand_alone_xrpld_cfg(0).is_err());
        assert!(stand_alone_xrpld_cfg(1).is_err());

        let cfg = match stand_alone_xrpld_cfg(STANDALONE_NETWORK_ID) {
            Ok(text) => text,
            Err(err) => panic!("unexpected cfg error: {err}"),
        };
        assert!(cfg.contains("[network_id]\n21337"));
        assert!(cfg.contains("BatchV1_1"));
        assert!(!cfg.contains("ConfidentialTransfer"));
        assert!(!cfg.contains("MPTokensV2"));
    }

    #[test]
    fn currency_code_encodes_cred_as_160_bit_hex() {
        assert_eq!(encode_currency_code("RAW"), Ok("RAW".to_string()));
        assert_eq!(
            encode_currency_code("CRED"),
            Ok("4352454400000000000000000000000000000000".to_string())
        );
        assert!(encode_currency_code("XRP").is_err());
        assert!(encode_currency_code("").is_err());
        assert!(encode_currency_code("TOO_LONG_CURRENCY_SYMBOL_21").is_err());
    }
}
