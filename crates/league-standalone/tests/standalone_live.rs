//! Gated integration test driving one live stand-alone `xrpld` container.
//!
//! - Unset or empty `LEAGUE_XRPLD_LIVE`: returns immediately (workspace tests / llvm-cov).
//! - `LEAGUE_XRPLD_LIVE=smoke`: launches 1 bounded container, checks `server_info`,
//!   verifies Mainnet-matched amendments, bootstraps 1 4-seat table, and tears down.
//! - `LEAGUE_XRPLD_LIVE=full`: runs smoke plus all Seed-27 Section B item B3 conformance tests.

use serde_json::json;

use league_standalone::conformance::{
    B3SuiteReport, bootstrap_four_seat_table, run_offer_spend_down_conformance,
    run_partial_fill_tariff_conformance, run_preimage_escrow_conformance,
};
use league_standalone::docker::{DockerSpec, StandAloneContainer};
use league_standalone::{OFFICIAL_XRPLD_PINNED_REF, STANDALONE_NETWORK_ID};

#[test]
fn standalone_xrpld_live_harness() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::var("LEAGUE_XRPLD_LIVE").unwrap_or_default();
    if mode.is_empty() || mode == "off" {
        return Ok(());
    }
    if mode != "smoke" && mode != "full" {
        return Err(format!("unsupported LEAGUE_XRPLD_LIVE mode {mode:?}").into());
    }

    let container = StandAloneContainer::start(DockerSpec::default())?;
    let client = container.client();

    let info = client.server_info()?;
    if info.network_id != Some(STANDALONE_NETWORK_ID) || info.validation_quorum != Some(0) {
        return Err(format!("unexpected server_info {info:?}").into());
    }
    let amendments = client.verify_v2_amendments()?;
    let table = bootstrap_four_seat_table(client)?;
    let next_ledger = client.ledger_accept()?;

    println!(
        "SMOKE_REPORT: {}",
        json!({
            "image": OFFICIAL_XRPLD_PINNED_REF,
            "network_id": info.network_id,
            "validation_quorum": info.validation_quorum,
            "peers": info.peers,
            "server_state": info.server_state,
            "build_version": info.build_version,
            "verified_amendment_count": amendments.verified_enabled.len(),
            "rejected_non_mainnet": amendments.rejected_non_mainnet,
            "testnet_only_default_yes_enabled": amendments.testnet_only_default_yes_enabled,
            "bootstrapped_ledger": table.bootstrapped_ledger,
            "next_ledger": next_ledger,
        })
    );

    if mode == "full" {
        let offer_spend_down = run_offer_spend_down_conformance(client, &table)?;
        let partial_fill_tariff = run_partial_fill_tariff_conformance(client, &table)?;
        let preimage_escrow = run_preimage_escrow_conformance(client, &table)?;
        let report = B3SuiteReport {
            image: OFFICIAL_XRPLD_PINNED_REF.to_string(),
            network_id: STANDALONE_NETWORK_ID,
            verified_amendment_count: amendments.verified_enabled.len(),
            bootstrapped_ledger: table.bootstrapped_ledger,
            offer_spend_down,
            partial_fill_tariff,
            preimage_escrow,
        };
        println!("B3_REPORT: {}", serde_json::to_string(&report)?);

        assert!(
            !report
                .offer_spend_down
                .league_core_spend_behind_offer_allowed
        );
        assert_eq!(
            report.offer_spend_down.xrpld_spend_behind_offer_result,
            "tesSUCCESS"
        );
        assert_eq!(report.offer_spend_down.open_offers_after_cross, 0);

        assert_eq!(
            report.preimage_escrow.finish_fee_10_result,
            "telINSUF_FEE_P"
        );
        assert_eq!(
            report.preimage_escrow.finish_fee_349_result,
            "telINSUF_FEE_P"
        );
        assert_eq!(
            report.preimage_escrow.finish_before_after_result,
            "tecNO_PERMISSION"
        );
        assert_eq!(
            report.preimage_escrow.finish_wrong_secret_result,
            "tecCRYPTOCONDITION_ERROR"
        );
        assert_eq!(report.preimage_escrow.finish_valid_result, "tesSUCCESS");
        assert_eq!(
            report.preimage_escrow.cancel_before_expiry_result,
            "tecNO_PERMISSION"
        );
        assert_eq!(
            report.preimage_escrow.finish_after_expiry_result,
            "tecNO_PERMISSION"
        );
        assert_eq!(
            report.preimage_escrow.cancel_after_expiry_result,
            "tesSUCCESS"
        );
    }

    container.teardown()?;
    Ok(())
}
