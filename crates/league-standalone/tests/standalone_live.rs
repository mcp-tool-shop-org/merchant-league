//! Gated integration test driving one live stand-alone `xrpld` container.
//!
//! - Unset or empty `LEAGUE_XRPLD_LIVE`: returns immediately (workspace tests / llvm-cov).
//! - `LEAGUE_XRPLD_LIVE=smoke`: launches 1 bounded container, checks `server_info`,
//!   verifies the Mainnet-matched v2 feature set, bootstraps 1 4-seat table, and tears down.
//! - `LEAGUE_XRPLD_LIVE=full`: runs smoke plus all Seed-27 Section B item B3 conformance tests
//!   against the real `league-core` [`TurnEngine`], logging `FEATURE_MAP:` once per run.

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
            "feature_count": amendments.all_features.len(),
            "bootstrapped_ledger": table.bootstrapped_ledger,
            "next_ledger": next_ledger,
        })
    );

    if mode == "full" {
        println!(
            "FEATURE_MAP: {}",
            serde_json::to_string(&amendments.all_features)?
        );

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

        // B3.1 assertions (real TurnEngine + live xrpld).
        assert!(
            !report
                .offer_spend_down
                .league_core_spend_behind_offer_allowed
        );
        assert_eq!(
            report.offer_spend_down.core_seller_free_after_offer_milli,
            20_000
        );
        assert_eq!(
            report.offer_spend_down.core_buyer_received_on_cross_milli,
            80_000
        );
        assert_eq!(
            report.offer_spend_down.xrpld_spend_behind_offer_result,
            "tesSUCCESS"
        );
        assert_eq!(report.offer_spend_down.seller_balance_after_spend, "40");
        assert_eq!(report.offer_spend_down.open_offers_before_cross, 1);
        assert_eq!(report.offer_spend_down.buyer_received_on_cross, "40");
        assert_eq!(report.offer_spend_down.open_offers_after_cross, 0);
        assert_eq!(report.offer_spend_down.buyer_open_offers_after_cross, 1);

        // B3.2 assertions pinning TurnEngine + live xrpld ledger values.
        assert_eq!(report.partial_fill_tariff.cases.len(), 3);
        let c0 = &report.partial_fill_tariff.cases[0];
        assert_eq!(c0.currency, "R0A");
        assert_eq!(c0.tariff_bps, 800);
        assert_eq!(c0.core_trade_cred_milli, 416);
        assert_eq!(c0.core_tariff_milli, 27);
        assert_eq!(c0.core_buyer_net_milli, 306);
        assert_eq!(c0.xrpld_seller_commodity_delta, "0->-0.333");
        assert_eq!(c0.xrpld_buyer_commodity_received, "0.333");
        assert_eq!(c0.xrpld_buyer_cred_delta, "960->959.667");
        assert_eq!(c0.xrpld_seller_cred_received, "0.333");

        let c1 = &report.partial_fill_tariff.cases[1];
        assert_eq!(c1.currency, "R0S");
        assert_eq!(c1.tariff_bps, 750);
        assert_eq!(c1.core_trade_cred_milli, 458);
        assert_eq!(c1.core_tariff_milli, 25);
        assert_eq!(c1.core_buyer_net_milli, 308);
        assert_eq!(c1.xrpld_seller_commodity_delta, "10->9.642025");
        assert_eq!(c1.xrpld_buyer_commodity_received, "0.333");
        assert_eq!(c1.xrpld_buyer_cred_delta, "100->99.582085");
        assert_eq!(c1.xrpld_seller_cred_received, "0.417915");

        let c2 = &report.partial_fill_tariff.cases[2];
        assert_eq!(c2.currency, "R0T");
        assert_eq!(c2.tariff_bps, 833);
        assert_eq!(c2.core_trade_cred_milli, 1_100);
        assert_eq!(c2.core_tariff_milli, 84);
        assert_eq!(c2.core_buyer_net_milli, 916);
        assert_eq!(c2.xrpld_seller_commodity_delta, "1->0");
        assert_eq!(c2.xrpld_buyer_commodity_received, "0.9231053263177329");
        assert_eq!(c2.xrpld_buyer_cred_delta, "100->99.07689467368227");
        assert_eq!(c2.xrpld_seller_cred_received, "0.9231053263177329");

        // B3.3 assertions (real TurnEngine + live xrpld).
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
        assert!(report.preimage_escrow.close_time_at_finish > report.preimage_escrow.finish_after);
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
        assert!(report.preimage_escrow.core_zero_drop_fee_verified);
        assert!(!report.preimage_escrow.core_finish_before_after_unlocked);
        assert!(!report.preimage_escrow.core_finish_wrong_secret_unlocked);
        assert!(report.preimage_escrow.core_finish_at_boundary_unlocked);
        assert!(!report.preimage_escrow.core_cancel_before_expiry_allowed);
        assert!(
            !report
                .preimage_escrow
                .core_finish_at_or_after_expiry_unlocked
        );
        assert!(report.preimage_escrow.core_cancel_at_expiry_allowed);
    }

    container.teardown()?;
    Ok(())
}
