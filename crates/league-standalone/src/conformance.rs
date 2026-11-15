//! Seed-27 Section B item B3 conformance tests comparing `league-core` (`ab06dd39`) rules
//! against a live stand-alone `xrpld` container without modifying `crates/league-core`.

use std::fmt::Write as _;

use league_ledger::client::Transport;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use xrpl::models::transactions::account_set::AccountSet;
use xrpl::models::transactions::escrow_cancel::EscrowCancel;
use xrpl::models::transactions::escrow_create::EscrowCreate;
use xrpl::models::transactions::escrow_finish::EscrowFinish;
use xrpl::models::transactions::offer_create::OfferCreate;
use xrpl::models::transactions::payment::Payment;
use xrpl::models::transactions::trust_set::TrustSet;

use crate::client::{StandAlone, StandAloneAccount, StandAloneError};
use crate::encode_currency_code;

/// AccountSet flag enabling rippling across issued trust lines.
pub const ASF_DEFAULT_RIPPLE: u32 = 8;
/// AccountSet flag enabling issuer clawback on trust lines.
pub const ASF_ALLOW_TRUSTLINE_CLAWBACK: u32 = 16;
/// AccountSet flag enabling TokenEscrow locking on issued trust lines.
pub const ASF_ALLOW_TRUSTLINE_LOCKING: u32 = 17;

/// Fixed point milli-unit scale used by `league-core` (`ab06dd39` `types.rs:13`).
pub const CORE_UNIT_SCALE: i64 = 1_000;
/// Basis-point denominator (`10_000 = 100%`) used by `league-core`.
pub const CORE_BPS_SCALE: i128 = 10_000;
/// XRPL transfer-rate identity factor (`1_000_000_000` = `0%` fee).
pub const XRPL_TRANSFER_RATE_IDENTITY: u32 = 1_000_000_000;
/// Drops fee floor verified for a 32-byte `PREIMAGE-SHA-256` `EscrowFinish`.
pub const PREIMAGE_32_FINISH_FEE_DROPS: &str = "350";

const INITIAL_SEAT_XRP_DROPS: u64 = 500_000_000;
const INITIAL_CRED_ALLOCATION: &str = "1000";

/// Bootstrapped single-table 4-seat stand-alone fixture (Steward + 4 `ed25519` seats).
#[derive(Debug)]
pub struct FourSeatTable {
    /// Table Steward acting as `CRED` issuer (with DefaultRipple, Clawback, and Locking enabled).
    pub steward: StandAloneAccount,
    /// Four funded `ed25519` league seats.
    pub seats: [StandAloneAccount; 4],
    /// Hex-encoded 160-bit `CRED` numeraire code (`4352454400000000000000000000000000000000`).
    pub cred_currency: String,
    /// Validated ledger index after bootstrap completed.
    pub bootstrapped_ledger: u64,
}

/// Result of conformance check B3.1 (offer upfront lock vs. spend-down behind open offers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferSpendDownReport {
    /// Whether `league-core` (`engine.rs:825-842,909-919`) permits spending behind an open offer.
    pub league_core_spend_behind_offer_allowed: bool,
    /// Live `xrpld` transaction result when spending `60` of `100` behind an `80` offer.
    pub xrpld_spend_behind_offer_result: String,
    /// Seller's funded commodity balance remaining right after spend-down.
    pub seller_balance_after_spend: String,
    /// Seller's open offer count before crossing.
    pub open_offers_before_cross: usize,
    /// Buyer's commodity received when crossing for `80` against the under-funded offer.
    pub buyer_received_on_cross: String,
    /// Seller's open offer count after crossing (`0` when `xrpld` deletes unfunded remainder).
    pub open_offers_after_cross: usize,
}

/// One partial-fill + tariff comparison case under conformance check B3.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TariffRoundingCase {
    /// Descriptive case name.
    pub case_name: String,
    /// Commodity code used on ledger.
    pub currency: String,
    /// Issuer export tariff in basis points.
    pub tariff_bps: u16,
    /// Maker's `league-core` unit trade credit (`mul_units_parity`).
    pub core_trade_cred_milli: i64,
    /// `league-core` `ceil_bps_fee` tariff deduction in milli-units.
    pub core_tariff_milli: i64,
    /// Buyer's net commodity credit in `league-core` (`fill - ceil_bps_fee`).
    pub core_buyer_net_milli: i64,
    /// Seller's observed `xrpld` commodity balance delta (negative string).
    pub xrpld_seller_commodity_delta: String,
    /// Buyer's observed `xrpld` commodity balance after fill.
    pub xrpld_buyer_commodity_received: String,
    /// Buyer's observed `xrpld` `CRED` spent.
    pub xrpld_buyer_cred_delta: String,
    /// Seller's observed `xrpld` `CRED` received.
    pub xrpld_seller_cred_received: String,
    /// Concise summary of where `xrpld` differs from `league-core`.
    pub difference_summary: String,
}

/// Aggregate result of conformance check B3.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartialFillTariffReport {
    /// Measured comparison cases covering `Quality` and `TransferRate` vs `ceil_bps_fee`.
    pub cases: Vec<TariffRoundingCase>,
}

/// Result of conformance check B3.3 (32-byte `PREIMAGE-SHA-256` fee floor and time bounds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreimageEscrowReport {
    /// Result of `EscrowFinish` with standard `10`-drop fee (`telINSUF_FEE_P` expected).
    pub finish_fee_10_result: String,
    /// Result of `EscrowFinish` with `349`-drop fee (`telINSUF_FEE_P` expected).
    pub finish_fee_349_result: String,
    /// Result of `EscrowFinish` with `350`-drop fee before `FinishAfter` (`tecNO_PERMISSION`).
    pub finish_before_after_result: String,
    /// Validated `close_time` observed right after `FinishAfter` became reachable.
    pub close_time_at_finish: u64,
    /// `FinishAfter` value configured on Escrow 1.
    pub finish_after: u64,
    /// Result of `EscrowFinish` with wrong 32-byte preimage (`tecCRYPTOCONDITION_ERROR`).
    pub finish_wrong_secret_result: String,
    /// Result of `EscrowFinish` with valid preimage and `350`-drop fee after `FinishAfter`.
    pub finish_valid_result: String,
    /// Result of `EscrowCancel` before `CancelAfter` (`tecNO_PERMISSION`).
    pub cancel_before_expiry_result: String,
    /// Result of `EscrowFinish` after `CancelAfter` (`tecNO_PERMISSION`).
    pub finish_after_expiry_result: String,
    /// Result of `EscrowCancel` after `CancelAfter` (`tesSUCCESS`).
    pub cancel_after_expiry_result: String,
    /// Discrete `league-core` turn comparison summary (`>= finish_after_turn`, `0`-drop fee).
    pub league_core_comparison: String,
}

/// Complete live conformance report printed cleanly by CI (`B3_REPORT:`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct B3SuiteReport {
    /// Pinned OCI image reference tested.
    pub image: String,
    /// Confirmed stand-alone network ID (`21337`).
    pub network_id: u32,
    /// Number of Mainnet-matched amendments confirmed enabled.
    pub verified_amendment_count: usize,
    /// Bootstrapped ledger index.
    pub bootstrapped_ledger: u64,
    /// B3.1 report.
    pub offer_spend_down: OfferSpendDownReport,
    /// B3.2 report.
    pub partial_fill_tariff: PartialFillTariffReport,
    /// B3.3 report.
    pub preimage_escrow: PreimageEscrowReport,
}

/// Pure `league-core` (`ab06dd39` `types.rs:32-38`) `ceil(amount * bps / 10_000)` helper.
#[must_use]
pub const fn core_ceil_bps_fee(amount: i64, bps: u16) -> i64 {
    if amount <= 0 || bps == 0 {
        return 0;
    }
    let num = (amount as i128) * (bps as i128) + (CORE_BPS_SCALE - 1);
    (num / CORE_BPS_SCALE) as i64
}

/// Pure `league-core` (`ab06dd39` `types.rs:61-66`) `floor(amount * parity_per_unit / 1_000)`.
#[must_use]
pub const fn core_mul_units_parity(amount: i64, parity_per_unit: i64) -> i64 {
    if amount <= 0 || parity_per_unit <= 0 {
        return 0;
    }
    (((amount as i128) * (parity_per_unit as i128)) / (CORE_UNIT_SCALE as i128)) as i64
}

/// Pure `league-core` (`ab06dd39` `engine.rs:825-842,909-919`) upfront-lock spend rule.
#[must_use]
pub const fn core_locked_spend_allows(initial_balance: i64, locked_offer: i64, spend: i64) -> bool {
    initial_balance - locked_offer >= spend
}

/// Compute `TransferRate` integer value (`1_000_000_000 + bps * 100_000`).
#[must_use]
pub const fn transfer_rate_from_bps(bps: u16) -> u32 {
    XRPL_TRANSFER_RATE_IDENTITY + (bps as u32) * 100_000
}

/// Compute DER-encoded `PREIMAGE-SHA-256` `(condition_hex, fulfillment_hex)` for a 32-byte secret.
#[must_use]
pub fn preimage_sha256_condition_and_fulfillment(secret: &[u8; 32]) -> (String, String) {
    let digest = Sha256::digest(secret);
    (
        format!("A0258020{}810120", to_upper_hex(&digest)),
        format!("A0228020{}", to_upper_hex(secret)),
    )
}

/// Bootstrap one 4-seat single table (1 Steward + 4 `ed25519` seats) on a verified stand-alone node.
pub fn bootstrap_four_seat_table<T: Transport>(
    client: &StandAlone<T>,
) -> Result<FourSeatTable, StandAloneError> {
    let cred_currency = encode_currency_code("CRED")?;
    let (steward, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    configure_issuer_account(client, &steward, true, None)?;

    let s0 = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?.0;
    let s1 = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?.0;
    let s2 = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?.0;
    let s3 = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?.0;

    for seat in [&s0, &s1, &s2, &s3] {
        set_trust_line(client, seat, steward.address(), &cred_currency, "1000000")?;
        pay_iou(
            client,
            &steward,
            steward.address(),
            seat.address(),
            &cred_currency,
            INITIAL_CRED_ALLOCATION,
        )?;
    }

    let (bootstrapped_ledger, _) = client.validated_close_time()?;
    Ok(FourSeatTable {
        steward,
        seats: [s0, s1, s2, s3],
        cred_currency,
        bootstrapped_ledger,
    })
}

/// Conformance B3.1: `league-core` locks funds upfront vs. `xrpld` allowing spend-down.
pub fn run_offer_spend_down_conformance<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<OfferSpendDownReport, StandAloneError> {
    let (issuer, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    configure_issuer_account(client, &issuer, false, None)?;

    let seller = &table.seats[0];
    let buyer = &table.seats[1];
    let bystander = &table.seats[2];
    let code = "SDW";

    for seat in [seller, buyer, bystander] {
        set_trust_line(client, seat, issuer.address(), code, "10000")?;
    }
    pay_iou(
        client,
        &issuer,
        issuer.address(),
        seller.address(),
        code,
        "100",
    )?;

    // Seller places an offer to sell 80 SDW for 80 CRED.
    let mut seller_offer: OfferCreate<'static> = build_tx(
        "OfferCreate",
        seller,
        json!({
            "TakerGets": iou(code, issuer.address(), "80"),
            "TakerPays": iou(&table.cred_currency, table.steward.address(), "80")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut seller_offer, seller, None)?;
    let open_offers_before_cross = client.account_offers(seller.address())?.len();

    // Compare against league-core upfront lock (`engine.rs:825-842,909-919`):
    // with 100_000 owned and 80_000 locked, spending 60_000 fails upfront in league-core.
    let league_core_spend_behind_offer_allowed = core_locked_spend_allows(100_000, 80_000, 60_000);

    // On live xrpld, spending 60 SDW out of 100 behind the resting 80 offer succeeds!
    let mut spend_tx: Payment<'static> = build_tx(
        "Payment",
        seller,
        json!({
            "Destination": bystander.address(),
            "Amount": iou(code, issuer.address(), "60")
        }),
    )?;
    let spend_receipt = client.submit_and_accept(&mut spend_tx, seller, None)?;
    let seller_balance_after_spend =
        find_line_balance(client, seller.address(), issuer.address(), code)?;

    // Buyer crosses to buy up to 80 SDW for 80 CRED: xrpld fills the 40 remaining and deletes
    // Seller's now-unfunded remainder from the directory.
    let mut buyer_offer: OfferCreate<'static> = build_tx(
        "OfferCreate",
        buyer,
        json!({
            "TakerPays": iou(code, issuer.address(), "80"),
            "TakerGets": iou(&table.cred_currency, table.steward.address(), "80")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut buyer_offer, buyer, None)?;

    let buyer_received_on_cross =
        find_line_balance(client, buyer.address(), issuer.address(), code)?;
    let open_offers_after_cross = client.account_offers(seller.address())?.len();

    Ok(OfferSpendDownReport {
        league_core_spend_behind_offer_allowed,
        xrpld_spend_behind_offer_result: spend_receipt.final_result,
        seller_balance_after_spend,
        open_offers_before_cross,
        buyer_received_on_cross,
        open_offers_after_cross,
    })
}

/// Conformance B3.2: partial fills, `Quality`, and `TransferRate` vs. `league-core` `ceil_bps_fee`.
pub fn run_partial_fill_tariff_conformance<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<PartialFillTariffReport, StandAloneError> {
    let cases = vec![
        run_case_direct_issuer_export(client, table)?,
        run_case_third_party_headroom(client, table)?,
        run_case_third_party_exact_balance(client, table)?,
    ];
    Ok(PartialFillTariffReport { cases })
}

fn run_case_direct_issuer_export<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<TariffRoundingCase, StandAloneError> {
    let (issuer_seller, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    configure_issuer_account(client, &issuer_seller, false, Some(800))?;
    let buyer = &table.seats[1];
    let code = "R0A";

    set_trust_line(
        client,
        &issuer_seller,
        table.steward.address(),
        &table.cred_currency,
        "1000000",
    )?;
    set_trust_line(client, buyer, issuer_seller.address(), code, "10000")?;

    let buyer_cred_before = find_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let mut maker: OfferCreate<'static> = build_tx(
        "OfferCreate",
        &issuer_seller,
        json!({
            "TakerGets": iou(code, issuer_seller.address(), "1"),
            "TakerPays": iou(&table.cred_currency, table.steward.address(), "1")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut maker, &issuer_seller, None)?;

    let mut taker: OfferCreate<'static> = build_tx(
        "OfferCreate",
        buyer,
        json!({
            "TakerPays": iou(code, issuer_seller.address(), "0.333"),
            "TakerGets": iou(&table.cred_currency, table.steward.address(), "0.5")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut taker, buyer, None)?;

    let buyer_r0a = find_line_balance(client, buyer.address(), issuer_seller.address(), code)?;
    let buyer_cred_after = find_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;
    let seller_cred = find_line_balance(
        client,
        issuer_seller.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let tariff_bps = 800;
    let core_trade = core_mul_units_parity(333, 1_000);
    let core_fee = core_ceil_bps_fee(333, tariff_bps);
    Ok(TariffRoundingCase {
        case_name: "direct_issuer_export_exempt".into(),
        currency: code.into(),
        tariff_bps,
        core_trade_cred_milli: core_trade,
        core_tariff_milli: core_fee,
        core_buyer_net_milli: 333 - core_fee,
        xrpld_seller_commodity_delta: format!("-{buyer_r0a}"),
        xrpld_buyer_commodity_received: buyer_r0a,
        xrpld_buyer_cred_delta: format!("{buyer_cred_before}->{buyer_cred_after}"),
        xrpld_seller_cred_received: seller_cred,
        difference_summary: "XRPL exempts issuer->holder primary issuance from TransferRate (0 fee vs league-core ceil_bps_fee=27 milli)".into(),
    })
}

fn run_case_third_party_headroom<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<TariffRoundingCase, StandAloneError> {
    let (issuer, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    configure_issuer_account(client, &issuer, false, Some(750))?;
    let (seller, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    let (buyer, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    let code = "R0S";

    for acc in [&seller, &buyer] {
        set_trust_line(client, acc, issuer.address(), code, "10000")?;
        set_trust_line(
            client,
            acc,
            table.steward.address(),
            &table.cred_currency,
            "1000000",
        )?;
    }
    pay_iou(
        client,
        &issuer,
        issuer.address(),
        seller.address(),
        code,
        "10",
    )?;
    pay_iou(
        client,
        &table.steward,
        table.steward.address(),
        buyer.address(),
        &table.cred_currency,
        "100",
    )?;

    let mut maker: OfferCreate<'static> = build_tx(
        "OfferCreate",
        &seller,
        json!({
            "TakerGets": iou(code, issuer.address(), "1"),
            "TakerPays": iou(&table.cred_currency, table.steward.address(), "1.255")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut maker, &seller, None)?;

    let mut taker: OfferCreate<'static> = build_tx(
        "OfferCreate",
        &buyer,
        json!({
            "TakerPays": iou(code, issuer.address(), "0.333"),
            "TakerGets": iou(&table.cred_currency, table.steward.address(), "0.5")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut taker, &buyer, None)?;

    let seller_after = find_line_balance(client, seller.address(), issuer.address(), code)?;
    let buyer_r0s = find_line_balance(client, buyer.address(), issuer.address(), code)?;
    let buyer_cred_after = find_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;
    let seller_cred_after = find_line_balance(
        client,
        seller.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let tariff_bps = 750;
    let core_trade = core_mul_units_parity(333, 1_255);
    let core_fee = core_ceil_bps_fee(333, tariff_bps);
    Ok(TariffRoundingCase {
        case_name: "third_party_headroom_sender_surcharge".into(),
        currency: code.into(),
        tariff_bps,
        core_trade_cred_milli: core_trade,
        core_tariff_milli: core_fee,
        core_buyer_net_milli: 333 - core_fee,
        xrpld_seller_commodity_delta: format!("10->{seller_after}"),
        xrpld_buyer_commodity_received: buyer_r0s,
        xrpld_buyer_cred_delta: format!("100->{buyer_cred_after}"),
        xrpld_seller_cred_received: seller_cred_after,
        difference_summary: "XRPL charges TransferRate upward on seller (1.075x) while keeping rational CRED quality (sub-milli) vs league-core buyer-deducted ceil_bps_fee and floor mul_units_parity".into(),
    })
}

fn run_case_third_party_exact_balance<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<TariffRoundingCase, StandAloneError> {
    let (issuer, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    configure_issuer_account(client, &issuer, false, Some(833))?;
    let (seller, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    let (buyer, _) = client.fund_new_account(INITIAL_SEAT_XRP_DROPS)?;
    let code = "R0T";

    for acc in [&seller, &buyer] {
        set_trust_line(client, acc, issuer.address(), code, "10000")?;
        set_trust_line(
            client,
            acc,
            table.steward.address(),
            &table.cred_currency,
            "1000000",
        )?;
    }
    pay_iou(
        client,
        &issuer,
        issuer.address(),
        seller.address(),
        code,
        "1",
    )?;
    pay_iou(
        client,
        &table.steward,
        table.steward.address(),
        buyer.address(),
        &table.cred_currency,
        "100",
    )?;

    let mut maker: OfferCreate<'static> = build_tx(
        "OfferCreate",
        &seller,
        json!({
            "TakerGets": iou(code, issuer.address(), "1"),
            "TakerPays": iou(&table.cred_currency, table.steward.address(), "1")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut maker, &seller, None)?;

    // Buyer allows up to 1.2 CRED for 1 R0T so the TransferRate-adjusted quality crosses cleanly.
    let mut taker: OfferCreate<'static> = build_tx(
        "OfferCreate",
        &buyer,
        json!({
            "TakerPays": iou(code, issuer.address(), "1"),
            "TakerGets": iou(&table.cred_currency, table.steward.address(), "1.2")
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut taker, &buyer, None)?;

    let seller_after = find_line_balance(client, seller.address(), issuer.address(), code)?;
    let buyer_r0t = find_line_balance(client, buyer.address(), issuer.address(), code)?;
    let buyer_cred_after = find_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;
    let seller_cred_after = find_line_balance(
        client,
        seller.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let tariff_bps = 833;
    let core_trade = core_mul_units_parity(1_000, 1_000);
    let core_fee = core_ceil_bps_fee(1_000, tariff_bps);
    Ok(TariffRoundingCase {
        case_name: "third_party_exact_balance_iou_mantissa".into(),
        currency: code.into(),
        tariff_bps,
        core_trade_cred_milli: core_trade,
        core_tariff_milli: core_fee,
        core_buyer_net_milli: 1_000 - core_fee,
        xrpld_seller_commodity_delta: format!("1->{seller_after}"),
        xrpld_buyer_commodity_received: buyer_r0t,
        xrpld_buyer_cred_delta: format!("100->{buyer_cred_after}"),
        xrpld_seller_cred_received: seller_cred_after,
        difference_summary: "With exact 1 unit available, XRPL divides 1/1.0833 to 16 significant digits (~0.923105...) vs league-core 1000-ceil(83.3)=916 milli-units (0.916)".into(),
    })
}

/// Conformance B3.3: 32-byte `PREIMAGE-SHA-256` fee floor and `FinishAfter`/`CancelAfter` bounds.
pub fn run_preimage_escrow_conformance<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<PreimageEscrowReport, StandAloneError> {
    let owner = &table.seats[0];
    let destination = &table.seats[1];
    let secret = [0x42u8; 32];
    let wrong_secret = [0x99u8; 32];
    let (cond, fulfil) = preimage_sha256_condition_and_fulfillment(&secret);
    let (_, wrong_fulfil) = preimage_sha256_condition_and_fulfillment(&wrong_secret);

    let (_, base_close) = client.validated_close_time()?;
    let finish_after = base_close + 25;
    let cancel_after = base_close + 300;
    let finish_after_u32 = u32::try_from(finish_after)
        .map_err(|_| StandAloneError::Build("FinishAfter exceeds u32".into()))?;
    let cancel_after_u32 = u32::try_from(cancel_after)
        .map_err(|_| StandAloneError::Build("CancelAfter exceeds u32".into()))?;

    let mut escrow1: EscrowCreate<'static> = build_tx(
        "EscrowCreate",
        owner,
        json!({
            "Amount": iou(&table.cred_currency, table.steward.address(), "5"),
            "Destination": destination.address(),
            "Condition": cond,
            "FinishAfter": finish_after_u32,
            "CancelAfter": cancel_after_u32
        }),
    )?;
    let escrow1_tx = client.submit_and_accept_ok(&mut escrow1, owner, None)?;

    let finish_fee_10_result = submit_escrow_finish(
        client,
        destination,
        owner.address(),
        escrow1_tx.sequence,
        &cond,
        &fulfil,
        "10",
    )?
    .final_result;
    let finish_fee_349_result = submit_escrow_finish(
        client,
        destination,
        owner.address(),
        escrow1_tx.sequence,
        &cond,
        &fulfil,
        "349",
    )?
    .final_result;
    let finish_before_after_result = submit_escrow_finish(
        client,
        destination,
        owner.address(),
        escrow1_tx.sequence,
        &cond,
        &fulfil,
        PREIMAGE_32_FINISH_FEE_DROPS,
    )?
    .final_result;

    let close_time_at_finish = advance_past_close_time(client, finish_after, 40)?;
    let finish_wrong_secret_result = submit_escrow_finish(
        client,
        destination,
        owner.address(),
        escrow1_tx.sequence,
        &cond,
        &wrong_fulfil,
        PREIMAGE_32_FINISH_FEE_DROPS,
    )?
    .final_result;
    let finish_valid_result = submit_escrow_finish(
        client,
        destination,
        owner.address(),
        escrow1_tx.sequence,
        &cond,
        &fulfil,
        PREIMAGE_32_FINISH_FEE_DROPS,
    )?
    .final_result;

    // Escrow 2 tests CancelAfter enforcement cleanly.
    let (_, base_close_2) = client.validated_close_time()?;
    let cancel_after_2 = base_close_2 + 25;
    let cancel_after_2_u32 = u32::try_from(cancel_after_2)
        .map_err(|_| StandAloneError::Build("CancelAfter2 exceeds u32".into()))?;
    let mut escrow2: EscrowCreate<'static> = build_tx(
        "EscrowCreate",
        owner,
        json!({
            "Amount": iou(&table.cred_currency, table.steward.address(), "5"),
            "Destination": destination.address(),
            "Condition": cond,
            "CancelAfter": cancel_after_2_u32
        }),
    )?;
    let escrow2_tx = client.submit_and_accept_ok(&mut escrow2, owner, None)?;

    let cancel_before_expiry_result =
        submit_escrow_cancel(client, owner, owner.address(), escrow2_tx.sequence)?.final_result;

    let _ = advance_past_close_time(client, cancel_after_2, 40)?;
    let finish_after_expiry_result = submit_escrow_finish(
        client,
        destination,
        owner.address(),
        escrow2_tx.sequence,
        &cond,
        &fulfil,
        PREIMAGE_32_FINISH_FEE_DROPS,
    )?
    .final_result;
    let cancel_after_expiry_result =
        submit_escrow_cancel(client, owner, owner.address(), escrow2_tx.sequence)?.final_result;

    Ok(PreimageEscrowReport {
        finish_fee_10_result,
        finish_fee_349_result,
        finish_before_after_result,
        close_time_at_finish,
        finish_after,
        finish_wrong_secret_result,
        finish_valid_result,
        cancel_before_expiry_result,
        finish_after_expiry_result,
        cancel_after_expiry_result,
        league_core_comparison: "league-core charges 0 drops and checks discrete turn >= finish_after_turn.unwrap_or(1) and turn >= cancel_after_turn (`engine.rs:1035-1048,1240-1255`); xrpld requires >=350 drops for 32-byte fulfillment and enforces parent_close_time > FinishAfter and >= CancelAfter".into(),
    })
}

/// Deserialize a typed `xrpl-rust` transaction model from JSON fields and signer address.
pub fn build_tx<Tx: DeserializeOwned>(
    kind: &str,
    account: &StandAloneAccount,
    mut fields: Value,
) -> Result<Tx, StandAloneError> {
    fields["TransactionType"] = json!(kind);
    fields["Account"] = json!(account.address());
    serde_json::from_value(fields)
        .map_err(|err| StandAloneError::Build(format!("invalid {kind} fields: {err}")))
}

fn configure_issuer_account<T: Transport>(
    client: &StandAlone<T>,
    issuer: &StandAloneAccount,
    allow_clawback_and_locking: bool,
    transfer_rate_bps: Option<u16>,
) -> Result<(), StandAloneError> {
    if allow_clawback_and_locking {
        let mut clawback: AccountSet<'static> = build_tx(
            "AccountSet",
            issuer,
            json!({ "SetFlag": ASF_ALLOW_TRUSTLINE_CLAWBACK }),
        )?;
        let _ = client.submit_and_accept_ok(&mut clawback, issuer, None)?;

        let mut locking: AccountSet<'static> = build_tx(
            "AccountSet",
            issuer,
            json!({ "SetFlag": ASF_ALLOW_TRUSTLINE_LOCKING }),
        )?;
        let _ = client.submit_and_accept_ok(&mut locking, issuer, None)?;
    }

    let mut ripple: AccountSet<'static> = build_tx(
        "AccountSet",
        issuer,
        json!({ "SetFlag": ASF_DEFAULT_RIPPLE }),
    )?;
    let _ = client.submit_and_accept_ok(&mut ripple, issuer, None)?;

    if let Some(bps) = transfer_rate_bps {
        let mut rate: AccountSet<'static> = build_tx(
            "AccountSet",
            issuer,
            json!({ "TransferRate": transfer_rate_from_bps(bps) }),
        )?;
        let _ = client.submit_and_accept_ok(&mut rate, issuer, None)?;
    }
    Ok(())
}

fn set_trust_line<T: Transport>(
    client: &StandAlone<T>,
    holder: &StandAloneAccount,
    issuer_addr: &str,
    currency: &str,
    limit: &str,
) -> Result<(), StandAloneError> {
    let mut tx: TrustSet<'static> = build_tx(
        "TrustSet",
        holder,
        json!({
            "LimitAmount": {
                "currency": currency,
                "issuer": issuer_addr,
                "value": limit
            }
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut tx, holder, None)?;
    Ok(())
}

fn pay_iou<T: Transport>(
    client: &StandAlone<T>,
    sender: &StandAloneAccount,
    issuer_addr: &str,
    destination: &str,
    currency: &str,
    value: &str,
) -> Result<(), StandAloneError> {
    let mut tx: Payment<'static> = build_tx(
        "Payment",
        sender,
        json!({
            "Destination": destination,
            "Amount": iou(currency, issuer_addr, value)
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut tx, sender, None)?;
    Ok(())
}

fn submit_escrow_finish<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    owner_addr: &str,
    offer_seq: u32,
    cond: &str,
    fulfil: &str,
    fee_drops: &str,
) -> Result<crate::client::ValidatedTx, StandAloneError> {
    let mut tx: EscrowFinish<'static> = build_tx(
        "EscrowFinish",
        signer,
        json!({
            "Owner": owner_addr,
            "OfferSequence": offer_seq,
            "Condition": cond,
            "Fulfillment": fulfil
        }),
    )?;
    client.submit_and_accept(&mut tx, signer, Some(fee_drops))
}

fn submit_escrow_cancel<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    owner_addr: &str,
    offer_seq: u32,
) -> Result<crate::client::ValidatedTx, StandAloneError> {
    let mut tx: EscrowCancel<'static> = build_tx(
        "EscrowCancel",
        signer,
        json!({
            "Owner": owner_addr,
            "OfferSequence": offer_seq
        }),
    )?;
    client.submit_and_accept(&mut tx, signer, None)
}

fn advance_past_close_time<T: Transport>(
    client: &StandAlone<T>,
    target: u64,
    max_accepts: usize,
) -> Result<u64, StandAloneError> {
    let (_, mut close_time) = client.validated_close_time()?;
    for _ in 0..max_accepts {
        if close_time > target {
            return Ok(close_time);
        }
        let _ = client.ledger_accept()?;
        (_, close_time) = client.validated_close_time()?;
    }
    if close_time > target {
        return Ok(close_time);
    }
    Err(StandAloneError::Reply(format!(
        "close_time {close_time} did not pass target {target} after {max_accepts} ledger_accept calls"
    )))
}

fn find_line_balance<T: Transport>(
    client: &StandAlone<T>,
    account: &str,
    issuer: &str,
    currency: &str,
) -> Result<String, StandAloneError> {
    let lines = client.account_lines(account)?;
    Ok(lines
        .into_iter()
        .find(|l| l.account == issuer && l.currency == currency)
        .map_or_else(|| "0".to_string(), |l| l.balance))
}

fn iou(currency: &str, issuer: &str, value: &str) -> Value {
    json!({
        "currency": currency,
        "issuer": issuer,
        "value": value
    })
}

fn to_upper_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02X}");
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::{Value, json};

    use super::*;
    use crate::STANDALONE_NETWORK_ID;

    struct ScriptedNode {
        step: RefCell<u64>,
        close_time: RefCell<u64>,
        seq: RefCell<u32>,
    }

    impl ScriptedNode {
        fn new() -> Self {
            Self {
                step: RefCell::new(0),
                close_time: RefCell::new(815_000_000),
                seq: RefCell::new(1),
            }
        }
    }

    impl Transport for ScriptedNode {
        fn post(&self, _url: &str, body: &Value) -> Result<Value, String> {
            let method = body
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match method {
                "server_info" => Ok(json!({
                    "result": {
                        "status": "success",
                        "info": {
                            "network_id": STANDALONE_NETWORK_ID,
                            "validation_quorum": 0,
                            "peers": 0,
                            "server_state": "full"
                        }
                    }
                })),
                "ledger" => Ok(json!({
                    "result": {
                        "status": "success",
                        "ledger": {
                            "ledger_index": 2 + *self.step.borrow(),
                            "close_time": *self.close_time.borrow()
                        }
                    }
                })),
                "ledger_accept" => {
                    *self.step.borrow_mut() += 1;
                    *self.close_time.borrow_mut() += 10;
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "ledger_current_index": 3 + *self.step.borrow()
                        }
                    }))
                }
                "account_info" => {
                    let s = *self.seq.borrow();
                    *self.seq.borrow_mut() = s + 1;
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "account_data": { "Sequence": s, "Balance": "500000000" }
                        }
                    }))
                }
                "submit" => Ok(json!({
                    "result": {
                        "status": "success",
                        "accepted": true,
                        "engine_result": "tesSUCCESS"
                    }
                })),
                "tx" => Ok(json!({
                    "result": {
                        "status": "success",
                        "validated": true,
                        "ledger_index": 2 + *self.step.borrow(),
                        "meta": { "TransactionResult": "tesSUCCESS" }
                    }
                })),
                "account_lines" => Ok(json!({
                    "result": {
                        "status": "success",
                        "lines": [{
                            "account": "rStubIssuer",
                            "currency": "RAW",
                            "balance": "40",
                            "limit": "1000"
                        }]
                    }
                })),
                "account_offers" => Ok(json!({
                    "result": {
                        "status": "success",
                        "offers": []
                    }
                })),
                other => Err(format!("unexpected scripted rpc {other}")),
            }
        }
    }

    #[test]
    fn core_math_and_condition_helpers_match_design_v2_and_part3_rules() {
        assert_eq!(core_ceil_bps_fee(0, 800), 0);
        assert_eq!(core_ceil_bps_fee(333, 800), 27);
        assert_eq!(core_ceil_bps_fee(333, 750), 25);
        assert_eq!(core_ceil_bps_fee(1_000, 833), 84);
        assert_eq!(core_mul_units_parity(333, 1_255), 417);
        assert!(!core_locked_spend_allows(100_000, 80_000, 60_000));
        assert!(core_locked_spend_allows(100_000, 80_000, 20_000));
        assert_eq!(transfer_rate_from_bps(750), 1_075_000_000);

        let (cond, fulfil) = preimage_sha256_condition_and_fulfillment(&[0x42u8; 32]);
        assert!(cond.starts_with("A0258020") && cond.ends_with("810120") && cond.len() == 78);
        assert!(fulfil.starts_with("A0228020") && fulfil.len() == 72);
    }

    #[test]
    fn bootstrap_and_conformance_helpers_run_against_scripted_node() {
        let client = match StandAlone::with_transport("http://127.0.0.1:5005", ScriptedNode::new())
        {
            Ok(c) => c,
            Err(err) => panic!("client failed: {err}"),
        };
        let table = match bootstrap_four_seat_table(&client) {
            Ok(t) => t,
            Err(err) => panic!("bootstrap failed: {err}"),
        };
        assert_eq!(
            table.cred_currency,
            "4352454400000000000000000000000000000000"
        );
        assert!(run_offer_spend_down_conformance(&client, &table).is_ok());
        assert!(run_partial_fill_tariff_conformance(&client, &table).is_ok());
        assert!(run_preimage_escrow_conformance(&client, &table).is_ok());
    }
}
