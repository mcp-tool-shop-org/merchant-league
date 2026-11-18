//! Seed-27 Section B item B3 conformance tests comparing the real [`league_core::TurnEngine`]
//! against a live stand-alone `xrpld` container without modifying `crates/league-core`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use league_core::{
    AssetId, AtomicSwapProposal, Bps, EngineError, EscrowFinishUnlock, EscrowProposal,
    EscrowStatus, MarketAction, OfferProposal, OfferSide, OrderReveal, PreimageSecret, RevealSalt,
    SealedOrderBundle, Seat, SwapPreviewOutcome, TableId, TableSpec, Turn, TurnEngine, UNIT_SCALE,
    Units, ceil_bps_fee, checked_mul_units_parity, compute_order_commitment,
    compute_preimage_condition, derive_turn_beacon,
};
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

/// XRPL transfer-rate identity factor (`1_000_000_000` = `0%` fee).
pub const XRPL_TRANSFER_RATE_IDENTITY: u32 = 1_000_000_000;
/// Drops fee floor verified for a 32-byte `PREIMAGE-SHA-256` `EscrowFinish`.
pub const PREIMAGE_32_FINISH_FEE_DROPS: &str = "350";

const INITIAL_SEAT_XRP_DROPS: u64 = 500_000_000;
const INITIAL_CRED_ALLOCATION: &str = "1000";
const CONFORMANCE_TABLE_SEED: u64 = 27;
const CONFORMANCE_BEACON_SEED: u64 = 27_001;

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
    /// Whether `league-core`'s [`TurnEngine`] permits spending behind an open offer.
    pub league_core_spend_behind_offer_allowed: bool,
    /// Seller's remaining free balance in [`TurnEngine`] right after locking an `80` offer of `100`.
    pub core_seller_free_after_offer_milli: Units,
    /// Buyer's commodity balance credited by [`TurnEngine`] when crossing against the `80` offer.
    pub core_buyer_received_on_cross_milli: Units,
    /// Live `xrpld` transaction result when spending `60` of `100` behind an `80` offer.
    pub xrpld_spend_behind_offer_result: String,
    /// Seller's observed `xrpld` commodity balance remaining right after spend-down.
    pub seller_balance_after_spend: String,
    /// Seller's open offer count on `xrpld` before crossing.
    pub open_offers_before_cross: usize,
    /// Buyer's observed `xrpld` commodity balance after crossing for `80` against the under-funded offer.
    pub buyer_received_on_cross: String,
    /// Seller's open offer count on `xrpld` after crossing (`0` when `xrpld` deletes unfunded remainder).
    pub open_offers_after_cross: usize,
    /// Buyer's open offer count on `xrpld` after crossing (`1` when unfilled `40` bid remains resting).
    pub buyer_open_offers_after_cross: usize,
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
    /// Seller's `CRED` increase observed from [`TurnEngine`] after the cross (`checked_mul_units_parity`).
    pub core_trade_cred_milli: Units,
    /// Tariff withheld in [`TurnEngine`] on the fill (`ceil_bps_fee`).
    pub core_tariff_milli: Units,
    /// Buyer's net commodity credit observed from [`TurnEngine`] (`fill - core_tariff_milli`).
    pub core_buyer_net_milli: Units,
    /// Seller's observed `xrpld` commodity balance transition (`"<before>-><after>"` from `account_lines`).
    pub xrpld_seller_commodity_delta: String,
    /// Buyer's observed `xrpld` commodity balance read from `account_lines` after fill.
    pub xrpld_buyer_commodity_received: String,
    /// Buyer's observed `xrpld` `CRED` balance transition (`"<before>-><after>"` from `account_lines`).
    pub xrpld_buyer_cred_delta: String,
    /// Seller's observed `xrpld` `CRED` balance read from `account_lines` after fill.
    pub xrpld_seller_cred_received: String,
    /// Concise summary of where `xrpld` differs from `league-core`.
    pub difference_summary: String,
}

/// Aggregate result of conformance check B3.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartialFillTariffReport {
    /// Measured comparison cases covering `Quality` and `TransferRate` vs [`TurnEngine`].
    pub cases: Vec<TariffRoundingCase>,
}

/// Observed [`TurnEngine`] preimage escrow checks returned by [`run_core_preimage_escrow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorePreimageEscrowSummary {
    /// `CreateEscrow` deducted exact collateral with zero drop fees (`cred` change `== -amount`).
    pub zero_drop_fee_verified: bool,
    /// Whether `preview_bundle` unlocked Escrow 1 before `finish_after_turn`.
    pub finish_before_after_unlocked: bool,
    /// Whether `preview_bundle` unlocked Escrow 1 with the wrong preimage after `finish_after_turn`.
    pub finish_wrong_secret_unlocked: bool,
    /// Whether sweeping with the valid preimage at `turn == finish_after_turn` unlocked Escrow 1.
    pub finish_at_boundary_unlocked: bool,
    /// Whether `CancelExpiredEscrow` succeeded before `cancel_after_turn`.
    pub cancel_before_expiry_allowed: bool,
    /// Whether `preview_bundle` unlocked Escrow 2 at `turn == cancel_after_turn`.
    pub finish_at_or_after_expiry_unlocked: bool,
    /// Whether `CancelExpiredEscrow` refunded Escrow 2 at `turn == cancel_after_turn`.
    pub cancel_at_expiry_allowed: bool,
}

/// Result of conformance check B3.3 (32-byte `PREIMAGE-SHA-256` fee floor and time bounds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreimageEscrowReport {
    /// Result of `EscrowFinish` on `xrpld` with standard `10`-drop fee (`telINSUF_FEE_P`).
    pub finish_fee_10_result: String,
    /// Result of `EscrowFinish` on `xrpld` with `349`-drop fee (`telINSUF_FEE_P`).
    pub finish_fee_349_result: String,
    /// Result of `EscrowFinish` on `xrpld` with `350`-drop fee before `FinishAfter` (`tecNO_PERMISSION`).
    pub finish_before_after_result: String,
    /// Validated `close_time` on `xrpld` observed right after `FinishAfter` became reachable.
    pub close_time_at_finish: u64,
    /// `FinishAfter` value configured on Escrow 1.
    pub finish_after: u64,
    /// Result of `EscrowFinish` on `xrpld` with wrong 32-byte preimage (`tecCRYPTOCONDITION_ERROR`).
    pub finish_wrong_secret_result: String,
    /// Result of `EscrowFinish` on `xrpld` with valid preimage and `350`-drop fee after `FinishAfter`.
    pub finish_valid_result: String,
    /// Result of `EscrowCancel` on `xrpld` before `CancelAfter` (`tecNO_PERMISSION`).
    pub cancel_before_expiry_result: String,
    /// Result of `EscrowFinish` on `xrpld` after `CancelAfter` (`tecNO_PERMISSION`).
    pub finish_after_expiry_result: String,
    /// Result of `EscrowCancel` on `xrpld` after `CancelAfter` (`tesSUCCESS`).
    pub cancel_after_expiry_result: String,
    /// Whether [`TurnEngine`] deducted exact collateral with zero drop fee.
    pub core_zero_drop_fee_verified: bool,
    /// Whether [`TurnEngine`] unlocked Escrow 1 before `finish_after_turn`.
    pub core_finish_before_after_unlocked: bool,
    /// Whether [`TurnEngine`] unlocked Escrow 1 with a wrong preimage.
    pub core_finish_wrong_secret_unlocked: bool,
    /// Whether [`TurnEngine`] unlocked Escrow 1 at `turn == finish_after_turn`.
    pub core_finish_at_boundary_unlocked: bool,
    /// Whether [`TurnEngine`] permitted `CancelExpiredEscrow` before `cancel_after_turn`.
    pub core_cancel_before_expiry_allowed: bool,
    /// Whether [`TurnEngine`] unlocked Escrow 2 at `turn == cancel_after_turn`.
    pub core_finish_at_or_after_expiry_unlocked: bool,
    /// Whether [`TurnEngine`] refunded Escrow 2 at `turn == cancel_after_turn`.
    pub core_cancel_at_expiry_allowed: bool,
    /// Discrete `league-core` vs `xrpld` fee and close-time comparison summary.
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

fn map_core_err(err: EngineError) -> StandAloneError {
    StandAloneError::Build(format!("league-core error: {err}"))
}

fn build_conformance_engine(
    seat0_raw_yield: Units,
    seat0_raw_tariff_bps: Bps,
) -> Result<TurnEngine, StandAloneError> {
    let salts = [
        RevealSalt([0x11; 32]),
        RevealSalt([0x22; 32]),
        RevealSalt([0x33; 32]),
        RevealSalt([0x44; 32]),
    ];
    let (mut spec, _) = TableSpec::standard_chiral_ring_with_explicit_deal(
        TableId(CONFORMANCE_TABLE_SEED),
        4,
        CONFORMANCE_TABLE_SEED,
        [0x27; 32],
        &salts,
    )
    .map_err(map_core_err)?;
    spec.shocks.clear();
    spec.works_stipend_per_unit_cred = 0;
    for seat in &mut spec.seats {
        seat.initial_cred = 100 * UNIT_SCALE;
        seat.perishable_decay_bps = 0;
        seat.regent = None;
    }
    spec.seats[0].raw_yield_per_turn = seat0_raw_yield;
    spec.seats[0].initial_raw_tariff_bps = seat0_raw_tariff_bps;
    TurnEngine::new(spec).map_err(map_core_err)
}

fn sweep_engine_turn(
    engine: &mut TurnEngine,
    bundles: &[SealedOrderBundle; 4],
    beacon_seed: u64,
) -> Result<league_core::SweepReceipt, StandAloneError> {
    let table_id = engine.state().table_id;
    let turn = Turn(engine.state().turn);
    engine.advance_to_commit().map_err(map_core_err)?;
    let mut reveals = Vec::with_capacity(4);
    for (idx, bundle) in bundles.iter().enumerate() {
        let s = idx as u8;
        let salt = RevealSalt([0xA0 + s; 32]);
        let reveal = OrderReveal {
            bundle: bundle.clone(),
            salt,
        };
        let commit =
            compute_order_commitment(table_id, turn, Seat(s), &reveal).map_err(map_core_err)?;
        engine
            .submit_order_commitment(Seat(s), commit)
            .map_err(map_core_err)?;
        reveals.push((Seat(s), reveal));
    }
    engine.advance_to_reveal().map_err(map_core_err)?;
    for (seat, reveal) in reveals {
        engine
            .submit_order_reveal(seat, reveal)
            .map_err(map_core_err)?;
    }
    engine.sweep_turn_seeded(beacon_seed).map_err(map_core_err)
}

/// Drive the B3.1 offer upfront-lock vs spend-behind check directly on [`TurnEngine`].
///
/// Returns `(spend_behind_offer_allowed, seller_free_after_offer_milli, buyer_received_on_cross_milli)`.
pub fn run_core_offer_spend_down() -> Result<(bool, Units, Units), StandAloneError> {
    let mut engine = build_conformance_engine(100 * UNIT_SCALE, 0)?;
    let empty = [
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
    ];
    let _ = sweep_engine_turn(&mut engine, &empty, CONFORMANCE_BEACON_SEED)?;

    // Seat 0 holds 100_000 Raw(0) on Turn 2; posting an 80_000 sell offer locks upfront.
    engine
        .apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 80 * UNIT_SCALE,
                price_per_unit_cred: UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        })
        .map_err(map_core_err)?;
    let seller_free_after_offer = engine.state().seats[0].raw[0];

    // Attempt to spend 60_000 Raw(0) behind the resting 80_000 offer:
    // (1) via a second PostOffer, and (2) via a co-signed AtomicSwapProposal in preview_bundle.
    let second_offer_allowed = engine
        .apply_market_action(MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: 60 * UNIT_SCALE,
                price_per_unit_cred: UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        })
        .is_ok();

    let swap = AtomicSwapProposal {
        swap_id: 901,
        maker: 0,
        taker: 2,
        maker_asset: AssetId::Raw(0),
        maker_amount: 60 * UNIT_SCALE,
        taker_asset: AssetId::Cred,
        taker_amount: 60 * UNIT_SCALE,
        domain_id: None,
    };
    let mut bundle_seat0 = SealedOrderBundle::default();
    bundle_seat0.atomic_swaps.push(swap.clone());
    let mut bundle_seat2 = SealedOrderBundle::default();
    bundle_seat2.atomic_swaps.push(swap);
    let mut peer_bundles = BTreeMap::new();
    peer_bundles.insert(2, bundle_seat2);
    let preview = league_core::engine_internals::preview_bundle_with_context_live(
        &engine,
        Seat(0),
        &[],
        &bundle_seat0,
        &peer_bundles,
        derive_turn_beacon(
            CONFORMANCE_BEACON_SEED + 1,
            engine.state().table_id,
            Turn(2),
        ),
    )
    .map_err(map_core_err)?;
    let swap_behind_allowed = preview
        .swap_orders
        .iter()
        .any(|o| o.outcome == SwapPreviewOutcome::Settled);

    // Seat 1 crosses for 80_000 Raw(0) at Sweep Step 0 batch clearing and receives 80_000 at 0 bps tariff.
    let buyer_before = engine.state().seats[1].raw[0];
    engine
        .apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: 80 * UNIT_SCALE,
                price_per_unit_cred: UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        })
        .map_err(map_core_err)?;
    let _ = sweep_engine_turn(&mut engine, &empty, CONFORMANCE_BEACON_SEED + 2)?;
    let buyer_received_on_cross = engine.state().seats[1].raw[0] - buyer_before;

    Ok((
        second_offer_allowed || swap_behind_allowed,
        seller_free_after_offer,
        buyer_received_on_cross,
    ))
}

/// Drive a single B3.2 partial-fill + tariff scenario directly through [`TurnEngine::apply_market_action`].
///
/// Returns `(core_trade_cred_milli, core_tariff_milli, core_buyer_net_milli)`.
pub fn run_core_tariff_case(
    tariff_bps: Bps,
    third_party_transfer_gross: Option<Units>,
    maker_offer_units: Units,
    maker_price_cred: Units,
    taker_buy_units: Units,
    taker_limit_price_cred: Units,
) -> Result<(Units, Units, Units), StandAloneError> {
    let mut engine = build_conformance_engine(25 * UNIT_SCALE, tariff_bps)?;
    let empty = [
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
    ];
    let _ = sweep_engine_turn(&mut engine, &empty, CONFORMANCE_BEACON_SEED)?;

    let seller_seat: u8 = if let Some(gross) = third_party_transfer_gross {
        // Seat 0 sells `gross` Raw(0) to Seat 2 so Seat 2 holds the third-party test inventory
        // obtained under real TurnEngine tariff deduction (`ceil_bps_fee`).
        engine
            .apply_market_action(MarketAction::PostOffer {
                seat: 0,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(0),
                    amount: gross,
                    price_per_unit_cred: UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
            })
            .map_err(map_core_err)?;
        engine
            .apply_market_action(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: gross,
                    price_per_unit_cred: UNIT_SCALE,
                    domain_id: None,
                    is_hybrid: false,
                },
            })
            .map_err(map_core_err)?;
        let _ = sweep_engine_turn(&mut engine, &empty, CONFORMANCE_BEACON_SEED + 1)?;
        2
    } else {
        0
    };

    let seller_cred_before = engine.state().seats[usize::from(seller_seat)].cred;
    let buyer_raw_before = engine.state().seats[1].raw[0];

    engine
        .apply_market_action(MarketAction::PostOffer {
            seat: seller_seat,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: maker_offer_units,
                price_per_unit_cred: maker_price_cred,
                domain_id: None,
                is_hybrid: false,
            },
        })
        .map_err(map_core_err)?;
    engine
        .apply_market_action(MarketAction::PostOffer {
            seat: 1,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(0),
                amount: taker_buy_units,
                price_per_unit_cred: taker_limit_price_cred,
                domain_id: None,
                is_hybrid: false,
            },
        })
        .map_err(map_core_err)?;
    let _ = sweep_engine_turn(&mut engine, &empty, CONFORMANCE_BEACON_SEED + 2)?;

    let trade_cred = engine.state().seats[usize::from(seller_seat)].cred - seller_cred_before;
    let buyer_net = engine.state().seats[1].raw[0] - buyer_raw_before;
    let tariff_withheld = taker_buy_units - buyer_net;

    let batch_price_cred =
        ((i128::from(maker_price_cred) + i128::from(taker_limit_price_cred)) / 2) as Units;
    let expected_trade =
        checked_mul_units_parity(taker_buy_units, batch_price_cred).map_err(map_core_err)?;
    let expected_tariff = ceil_bps_fee(taker_buy_units, tariff_bps);
    if trade_cred != expected_trade || tariff_withheld != expected_tariff {
        return Err(StandAloneError::Build(format!(
            "TurnEngine tariff outcome mismatch: got trade={trade_cred} tariff={tariff_withheld}, \
             expected trade={expected_trade} tariff={expected_tariff}"
        )));
    }
    Ok((trade_cred, tariff_withheld, buyer_net))
}

/// Drive the B3.3 preimage escrow fee and turn-bound checks directly on [`TurnEngine`].
pub fn run_core_preimage_escrow(
    secret: &[u8; 32],
    wrong_secret: &[u8; 32],
) -> Result<CorePreimageEscrowSummary, StandAloneError> {
    let mut engine = build_conformance_engine(10 * UNIT_SCALE, 0)?;
    let secret_obj = PreimageSecret(*secret);
    let cond = compute_preimage_condition(&secret_obj).map_err(map_core_err)?;

    let cred_before = engine.state().seats[0].cred;
    let escrow_amount = 5 * UNIT_SCALE;
    engine
        .apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: escrow_amount,
                finish_after_turn: Some(2),
                cancel_after_turn: 4,
                condition_hash: Some(cond),
                bond: None,
            },
        })
        .map_err(map_core_err)?;
    engine
        .apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Cred,
                amount: escrow_amount,
                finish_after_turn: None,
                cancel_after_turn: 3,
                condition_hash: Some(cond),
                bond: None,
            },
        })
        .map_err(map_core_err)?;
    let zero_drop_fee_verified = cred_before - engine.state().seats[0].cred == 2 * escrow_amount;

    // Turn 1 (`turn < finish_after_turn = 2`): valid preimage unlock must NOT release Escrow 1.
    let mut valid_escrow1_bundle = SealedOrderBundle::default();
    valid_escrow1_bundle
        .escrow_finishes
        .push(EscrowFinishUnlock {
            escrow_id: 1,
            preimage: Some(secret_obj),
        });
    let early_preview = league_core::engine_internals::preview_bundle_live(
        &engine,
        Seat(1),
        &[],
        &valid_escrow1_bundle,
    )
    .map_err(map_core_err)?;
    let finish_before_after_unlocked = early_preview
        .escrow_orders
        .first()
        .is_some_and(|e| e.unlocked);

    // Turn 1 (`turn < cancel_after_turn = 3`): CancelExpiredEscrow on Escrow 2 must be refused.
    let cancel_before_expiry_allowed = engine
        .apply_market_action(MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 2,
        })
        .is_ok();

    let empty = [
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
    ];
    let _ = sweep_engine_turn(&mut engine, &empty, CONFORMANCE_BEACON_SEED)?;

    // Turn 2 (`turn == finish_after_turn = 2`): wrong preimage is refused; valid preimage unlocks.
    let mut wrong_escrow1_bundle = SealedOrderBundle::default();
    wrong_escrow1_bundle
        .escrow_finishes
        .push(EscrowFinishUnlock {
            escrow_id: 1,
            preimage: Some(PreimageSecret(*wrong_secret)),
        });
    let wrong_preview = league_core::engine_internals::preview_bundle_live(
        &engine,
        Seat(1),
        &[],
        &wrong_escrow1_bundle,
    )
    .map_err(map_core_err)?;
    let finish_wrong_secret_unlocked = wrong_preview
        .escrow_orders
        .first()
        .is_some_and(|e| e.unlocked);

    let turn2_bundles = [
        SealedOrderBundle::default(),
        valid_escrow1_bundle,
        SealedOrderBundle::default(),
        SealedOrderBundle::default(),
    ];
    let turn2_receipt =
        sweep_engine_turn(&mut engine, &turn2_bundles, CONFORMANCE_BEACON_SEED + 1)?;
    let finish_at_boundary_unlocked = turn2_receipt.escrows_finished.contains(&1)
        && engine
            .state()
            .escrows
            .get(&1)
            .is_some_and(|e| e.status == EscrowStatus::Finished);

    // Turn 3 (`turn == cancel_after_turn = 3` on Escrow 2): finish fails, cancel refunds.
    let mut valid_escrow2_bundle = SealedOrderBundle::default();
    valid_escrow2_bundle
        .escrow_finishes
        .push(EscrowFinishUnlock {
            escrow_id: 2,
            preimage: Some(PreimageSecret(*secret)),
        });
    let expired_preview = league_core::engine_internals::preview_bundle_live(
        &engine,
        Seat(1),
        &[],
        &valid_escrow2_bundle,
    )
    .map_err(map_core_err)?;
    let finish_at_or_after_expiry_unlocked = expired_preview
        .escrow_orders
        .first()
        .is_some_and(|e| e.unlocked);

    let cancel_at_expiry_allowed = engine
        .apply_market_action(MarketAction::CancelExpiredEscrow {
            seat: 0,
            escrow_id: 2,
        })
        .is_ok()
        && engine
            .state()
            .escrows
            .get(&2)
            .is_some_and(|e| e.status == EscrowStatus::Cancelled);

    Ok(CorePreimageEscrowSummary {
        zero_drop_fee_verified,
        finish_before_after_unlocked,
        finish_wrong_secret_unlocked,
        finish_at_boundary_unlocked,
        cancel_before_expiry_allowed,
        finish_at_or_after_expiry_unlocked,
        cancel_at_expiry_allowed,
    })
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

/// Conformance B3.1: [`TurnEngine`] locks funds upfront vs. `xrpld` allowing spend-down.
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

    let (
        league_core_spend_behind_offer_allowed,
        core_seller_free_after_offer_milli,
        core_buyer_received_on_cross_milli,
    ) = run_core_offer_spend_down()?;

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
        require_line_balance(client, seller.address(), issuer.address(), code)?;

    // Buyer crosses to buy up to 80 SDW for 80 CRED: xrpld fills the 40 remaining, deletes
    // Seller's now-unfunded remainder, and leaves Buyer's unfilled 40-SDW offer resting.
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
        require_line_balance(client, buyer.address(), issuer.address(), code)?;
    let open_offers_after_cross = client.account_offers(seller.address())?.len();
    let buyer_open_offers_after_cross = client.account_offers(buyer.address())?.len();

    Ok(OfferSpendDownReport {
        league_core_spend_behind_offer_allowed,
        core_seller_free_after_offer_milli,
        core_buyer_received_on_cross_milli,
        xrpld_spend_behind_offer_result: spend_receipt.final_result,
        seller_balance_after_spend,
        open_offers_before_cross,
        buyer_received_on_cross,
        open_offers_after_cross,
        buyer_open_offers_after_cross,
    })
}

/// Conformance B3.2: partial fills, `Quality`, and `TransferRate` vs. [`TurnEngine`].
pub fn run_partial_fill_tariff_conformance<T: Transport>(
    client: &StandAlone<T>,
    table: &FourSeatTable,
) -> Result<PartialFillTariffReport, StandAloneError> {
    let cases = vec![
        run_case_direct_issuer_export(client, table)?,
        run_case_third_party_headroom(client, table)?,
        run_case_third_party_balance_capped(client, table)?,
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

    let seller_before =
        require_line_balance(client, issuer_seller.address(), buyer.address(), code)?;
    let buyer_cred_before = require_line_balance(
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

    let seller_after =
        require_line_balance(client, issuer_seller.address(), buyer.address(), code)?;
    let buyer_r0a = require_line_balance(client, buyer.address(), issuer_seller.address(), code)?;
    let buyer_cred_after = require_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;
    let seller_cred = require_line_balance(
        client,
        issuer_seller.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let tariff_bps = 800;
    let (core_trade_cred_milli, core_tariff_milli, core_buyer_net_milli) =
        run_core_tariff_case(tariff_bps, None, 1_000, 1_000, 333, 1_500)?;
    let difference_summary = format!(
        "XRPL exempts issuer->holder primary issuance from TransferRate (observed seller cred={seller_cred}, buyer recv={buyer_r0a}) vs league-core TurnEngine ceil_bps_fee={core_tariff_milli} milli, net={core_buyer_net_milli} milli, uniform batch cred={core_trade_cred_milli} milli"
    );
    Ok(TariffRoundingCase {
        case_name: "direct_issuer_export_exempt".into(),
        currency: code.into(),
        tariff_bps,
        core_trade_cred_milli,
        core_tariff_milli,
        core_buyer_net_milli,
        xrpld_seller_commodity_delta: format!("{seller_before}->{seller_after}"),
        xrpld_buyer_commodity_received: buyer_r0a,
        xrpld_buyer_cred_delta: format!("{buyer_cred_before}->{buyer_cred_after}"),
        xrpld_seller_cred_received: seller_cred,
        difference_summary,
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

    let seller_before = require_line_balance(client, seller.address(), issuer.address(), code)?;
    let buyer_cred_before = require_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
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

    let seller_after = require_line_balance(client, seller.address(), issuer.address(), code)?;
    let buyer_r0s = require_line_balance(client, buyer.address(), issuer.address(), code)?;
    let buyer_cred_after = require_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;
    let seller_cred_after = require_line_balance(
        client,
        seller.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let tariff_bps = 750;
    let (core_trade_cred_milli, core_tariff_milli, core_buyer_net_milli) =
        run_core_tariff_case(tariff_bps, Some(10_811), 1_000, 1_255, 333, 1_502)?;
    let difference_summary = format!(
        "XRPL charges TransferRate upward on seller (observed buyer recv={buyer_r0s}, seller cred={seller_cred_after}) while keeping rational CRED quality vs league-core TurnEngine buyer-deducted tariff={core_tariff_milli} milli, net={core_buyer_net_milli} milli, uniform batch cred={core_trade_cred_milli} milli"
    );
    Ok(TariffRoundingCase {
        case_name: "third_party_headroom_sender_surcharge".into(),
        currency: code.into(),
        tariff_bps,
        core_trade_cred_milli,
        core_tariff_milli,
        core_buyer_net_milli,
        xrpld_seller_commodity_delta: format!("{seller_before}->{seller_after}"),
        xrpld_buyer_commodity_received: buyer_r0s,
        xrpld_buyer_cred_delta: format!("{buyer_cred_before}->{buyer_cred_after}"),
        xrpld_seller_cred_received: seller_cred_after,
        difference_summary,
    })
}

fn run_case_third_party_balance_capped<T: Transport>(
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

    let seller_before = require_line_balance(client, seller.address(), issuer.address(), code)?;
    let buyer_cred_before = require_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
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

    let seller_after = require_line_balance(client, seller.address(), issuer.address(), code)?;
    let buyer_r0t = require_line_balance(client, buyer.address(), issuer.address(), code)?;
    let buyer_cred_after = require_line_balance(
        client,
        buyer.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;
    let seller_cred_after = require_line_balance(
        client,
        seller.address(),
        table.steward.address(),
        &table.cred_currency,
    )?;

    let tariff_bps = 833;
    let (core_trade_cred_milli, core_tariff_milli, core_buyer_net_milli) =
        run_core_tariff_case(tariff_bps, Some(1_091), 1_000, 1_000, 1_000, 1_200)?;
    let difference_summary = format!(
        "With balance-capped seller ({seller_before}->{seller_after}), XRPL divides across TransferRate (observed buyer recv={buyer_r0t}, seller cred={seller_cred_after}) vs league-core TurnEngine tariff={core_tariff_milli} milli, net={core_buyer_net_milli} milli, uniform batch cred={core_trade_cred_milli} milli; most of the commodity gap is fee direction plus ceil_bps_fee rounding with ~1e-16 from 16-digit mantissa"
    );
    Ok(TariffRoundingCase {
        case_name: "third_party_balance_capped_sender_surcharge".into(),
        currency: code.into(),
        tariff_bps,
        core_trade_cred_milli,
        core_tariff_milli,
        core_buyer_net_milli,
        xrpld_seller_commodity_delta: format!("{seller_before}->{seller_after}"),
        xrpld_buyer_commodity_received: buyer_r0t,
        xrpld_buyer_cred_delta: format!("{buyer_cred_before}->{buyer_cred_after}"),
        xrpld_seller_cred_received: seller_cred_after,
        difference_summary,
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

    let core = run_core_preimage_escrow(&secret, &wrong_secret)?;

    let league_core_comparison = format!(
        "league-core TurnEngine zero_drop_fee={} finish_boundary={} cancel_expiry={}; xrpld observed finish_fee_349={}, finish_valid={}, close_time={} > finish_after={} (the > vs >= boundary of CancelAfter is not measured at the boundary in this run)",
        core.zero_drop_fee_verified,
        core.finish_at_boundary_unlocked,
        core.cancel_at_expiry_allowed,
        finish_fee_349_result,
        finish_valid_result,
        close_time_at_finish,
        finish_after,
    );
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
        core_zero_drop_fee_verified: core.zero_drop_fee_verified,
        core_finish_before_after_unlocked: core.finish_before_after_unlocked,
        core_finish_wrong_secret_unlocked: core.finish_wrong_secret_unlocked,
        core_finish_at_boundary_unlocked: core.finish_at_boundary_unlocked,
        core_cancel_before_expiry_allowed: core.cancel_before_expiry_allowed,
        core_finish_at_or_after_expiry_unlocked: core.finish_at_or_after_expiry_unlocked,
        core_cancel_at_expiry_allowed: core.cancel_at_expiry_allowed,
        league_core_comparison,
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

fn require_line_balance<T: Transport>(
    client: &StandAlone<T>,
    account: &str,
    issuer: &str,
    currency: &str,
) -> Result<String, StandAloneError> {
    let lines = client.account_lines(account)?;
    lines
        .into_iter()
        .find(|l| l.account == issuer && l.currency == currency)
        .map(|l| l.balance)
        .ok_or_else(|| {
            StandAloneError::Reply(format!(
                "missing expected trust line for account={account} peer={issuer} currency={currency}"
            ))
        })
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
    use std::collections::BTreeSet;
    use std::rc::Rc;

    use serde_json::{Value, json};
    use xrpl::core::binarycodec;

    use super::*;
    use crate::STANDALONE_NETWORK_ID;

    #[derive(Default)]
    struct RecordedNodeState {
        step: u64,
        close_time: u64,
        seq: u32,
        read_counter: u64,
        trust_lines: BTreeSet<(String, String, String)>,
        observed_balances: Vec<(String, String, String, String)>,
    }

    struct ScriptedNode {
        state: Rc<RefCell<RecordedNodeState>>,
    }

    impl ScriptedNode {
        fn new() -> Self {
            Self {
                state: Rc::new(RefCell::new(RecordedNodeState {
                    step: 0,
                    close_time: 815_000_000,
                    seq: 1,
                    read_counter: 0,
                    trust_lines: BTreeSet::new(),
                    observed_balances: Vec::new(),
                })),
            }
        }
    }

    impl Transport for ScriptedNode {
        fn post(&self, _url: &str, body: &Value) -> Result<Value, String> {
            let method = body
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut st = self.state.borrow_mut();
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
                            "ledger_index": 2 + st.step,
                            "close_time": st.close_time
                        }
                    }
                })),
                "ledger_accept" => {
                    st.step += 1;
                    st.close_time += 10;
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "ledger_current_index": 3 + st.step
                        }
                    }))
                }
                "account_info" => {
                    let s = st.seq;
                    st.seq = s + 1;
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "account_data": { "Sequence": s, "Balance": "500000000" }
                        }
                    }))
                }
                "submit" => {
                    if let Some(blob) = body
                        .get("params")
                        .and_then(|p| p.get(0))
                        .and_then(|p| p.get("tx_blob"))
                        .and_then(Value::as_str)
                        && let Ok(tx) = binarycodec::decode(blob)
                        && tx.get("TransactionType").and_then(Value::as_str) == Some("TrustSet")
                        && let Some(holder) = tx.get("Account").and_then(Value::as_str)
                        && let Some(limit) = tx.get("LimitAmount")
                        && let Some(issuer) = limit.get("issuer").and_then(Value::as_str)
                        && let Some(currency) = limit.get("currency").and_then(Value::as_str)
                    {
                        st.trust_lines.insert((
                            holder.to_string(),
                            issuer.to_string(),
                            currency.to_string(),
                        ));
                        st.trust_lines.insert((
                            issuer.to_string(),
                            holder.to_string(),
                            currency.to_string(),
                        ));
                    }
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "accepted": true,
                            "engine_result": "tesSUCCESS"
                        }
                    }))
                }
                "tx" => Ok(json!({
                    "result": {
                        "status": "success",
                        "validated": true,
                        "ledger_index": 2 + st.step,
                        "meta": { "TransactionResult": "tesSUCCESS" }
                    }
                })),
                "account_lines" => {
                    let account = body
                        .get("params")
                        .and_then(|p| p.get(0))
                        .and_then(|p| p.get("account"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    st.read_counter += 1;
                    let batch_id = st.read_counter;
                    let matching: Vec<(String, String)> = st
                        .trust_lines
                        .iter()
                        .filter(|(a, _, _)| a == &account)
                        .map(|(_, peer, cur)| (peer.clone(), cur.clone()))
                        .collect();
                    let mut lines = Vec::with_capacity(matching.len());
                    for (idx, (peer, currency)) in matching.into_iter().enumerate() {
                        let bal = format!("obs-{batch_id}-{idx}");
                        st.observed_balances.push((
                            account.clone(),
                            peer.clone(),
                            currency.clone(),
                            bal.clone(),
                        ));
                        lines.push(json!({
                            "account": peer,
                            "currency": currency,
                            "balance": bal,
                            "limit": "1000000"
                        }));
                    }
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "lines": lines
                        }
                    }))
                }
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
    fn turn_engine_conformance_models_match_live_suite_cases() {
        let (spend_allowed, seller_free, buyer_got) = match run_core_offer_spend_down() {
            Ok(v) => v,
            Err(err) => panic!("run_core_offer_spend_down failed: {err}"),
        };
        assert!(!spend_allowed);
        assert_eq!(seller_free, 20_000);
        assert_eq!(buyer_got, 80_000);

        assert_eq!(
            run_core_tariff_case(800, None, 1_000, 1_000, 333, 1_500),
            Ok((416, 27, 306))
        );
        assert_eq!(
            run_core_tariff_case(750, Some(10_811), 1_000, 1_255, 333, 1_502),
            Ok((458, 25, 308))
        );
        assert_eq!(
            run_core_tariff_case(833, Some(1_091), 1_000, 1_000, 1_000, 1_200),
            Ok((1_100, 84, 916))
        );

        let escrow = match run_core_preimage_escrow(&[0x42u8; 32], &[0x99u8; 32]) {
            Ok(v) => v,
            Err(err) => panic!("run_core_preimage_escrow failed: {err}"),
        };
        assert!(escrow.zero_drop_fee_verified);
        assert!(!escrow.finish_before_after_unlocked);
        assert!(!escrow.finish_wrong_secret_unlocked);
        assert!(escrow.finish_at_boundary_unlocked);
        assert!(!escrow.cancel_before_expiry_allowed);
        assert!(!escrow.finish_at_or_after_expiry_unlocked);
        assert!(escrow.cancel_at_expiry_allowed);

        assert_eq!(transfer_rate_from_bps(750), 1_075_000_000);
        let (cond, fulfil) = preimage_sha256_condition_and_fulfillment(&[0x42u8; 32]);
        assert!(cond.starts_with("A0258020") && cond.ends_with("810120") && cond.len() == 78);
        assert!(fulfil.starts_with("A0228020") && fulfil.len() == 72);
    }

    #[test]
    fn observed_report_fields_come_from_ledger_reads_and_missing_line_errors() {
        let node = ScriptedNode::new();
        let log = Rc::clone(&node.state);
        let client = match StandAlone::with_transport("http://127.0.0.1:5005", node) {
            Ok(c) => c,
            Err(err) => panic!("client failed: {err}"),
        };

        assert!(
            require_line_balance(&client, "rMissing", "rIssuer", "R0A").is_err(),
            "missing trust line must return an error instead of synthesizing 0"
        );

        let table = match bootstrap_four_seat_table(&client) {
            Ok(t) => t,
            Err(err) => panic!("bootstrap failed: {err}"),
        };
        assert_eq!(
            table.cred_currency,
            "4352454400000000000000000000000000000000"
        );

        let spend = match run_offer_spend_down_conformance(&client, &table) {
            Ok(r) => r,
            Err(err) => panic!("B3.1 failed: {err}"),
        };
        let tariffs = match run_partial_fill_tariff_conformance(&client, &table) {
            Ok(r) => r,
            Err(err) => panic!("B3.2 failed: {err}"),
        };
        assert!(run_preimage_escrow_conformance(&client, &table).is_ok());

        let observed_set: BTreeSet<String> = log
            .borrow()
            .observed_balances
            .iter()
            .map(|(_, _, _, bal)| bal.clone())
            .collect();
        assert!(observed_set.contains(&spend.seller_balance_after_spend));
        assert!(observed_set.contains(&spend.buyer_received_on_cross));

        assert_eq!(tariffs.cases.len(), 3);
        for case in &tariffs.cases {
            let (s_before, s_after) = match case.xrpld_seller_commodity_delta.split_once("->") {
                Some(pair) => pair,
                None => panic!("expected before->after seller delta in {}", case.case_name),
            };
            assert!(
                observed_set.contains(s_before) && observed_set.contains(s_after),
                "case {} seller delta {} must come from account_lines",
                case.case_name,
                case.xrpld_seller_commodity_delta
            );
            assert!(
                observed_set.contains(&case.xrpld_buyer_commodity_received),
                "case {} buyer commodity {} must come from account_lines",
                case.case_name,
                case.xrpld_buyer_commodity_received
            );
            let (b_before, b_after) = match case.xrpld_buyer_cred_delta.split_once("->") {
                Some(pair) => pair,
                None => panic!(
                    "expected before->after buyer cred delta in {}",
                    case.case_name
                ),
            };
            assert!(
                observed_set.contains(b_before) && observed_set.contains(b_after),
                "case {} buyer cred delta {} must come from account_lines",
                case.case_name,
                case.xrpld_buyer_cred_delta
            );
            assert!(
                observed_set.contains(&case.xrpld_seller_cred_received),
                "case {} seller cred {} must come from account_lines",
                case.case_name,
                case.xrpld_seller_cred_received
            );
            assert!(
                case.difference_summary
                    .contains(&case.xrpld_seller_cred_received),
                "r2 N2: difference_summary must format live xrpld cred read {}, got: {}",
                case.xrpld_seller_cred_received,
                case.difference_summary
            );
        }
        let escrow_rep = match run_preimage_escrow_conformance(&client, &table) {
            Ok(v) => v,
            Err(err) => panic!("B3.3 failed: {err}"),
        };
        assert!(
            escrow_rep
                .league_core_comparison
                .contains(&escrow_rep.finish_valid_result),
            "r2 N2: league_core_comparison must format live result"
        );
    }
}
