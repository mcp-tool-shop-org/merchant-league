//! Dual-track contract primitives (`BatchV1_1` atomic swaps, `CRED`-priced DEX books,
//! `TokenEscrow` collateral vaults and Steward bonds, breakable `CheckCreate` notes,
//! cartel `PermissionedDomains`, and 6-turn Syndicate Charters).

use std::collections::BTreeMap;

pub use league_seat_types::{
    ActiveCharter, BondProposal, BondSettlementMode, BondSpec, CartelDomain, CheckProposal,
    CheckStatus, CredOffer, CreditCheck, DualEscrowVault, EscrowProposal, EscrowStatus, FillKind,
    FillRecord, MAX_CARTEL_DOMAINS, MarketAction, OfferProposal, OfferSide, PendingWashFreeze,
    TokenEscrowVault, WashFreezeSettlement,
};

use crate::scoring::FundamentalParities;
use crate::types::{
    AssetId, Bps, CreditTier, EngineError, UNIT_SCALE, Units, town_import_cost_cred,
};

/// Validates and computes required tariff rate (`bps`) for a commodity transfer between two seats.
#[must_use]
pub fn effective_export_tariff_bps(
    base_tariff_bps: Bps,
    domain_id: Option<u8>,
    domains: &[CartelDomain],
    sender: u8,
    receiver: u8,
    holiday_bps: Option<Bps>,
) -> Bps {
    if let Some(d_id) = domain_id
        && let Some(dom) = domains.iter().find(|d| d.domain_id == d_id)
        && dom.members.contains(&sender)
        && dom.members.contains(&receiver)
    {
        return 0;
    }
    match holiday_bps {
        Some(reduction_bps) => {
            let remaining = 10_000_i32 - i32::from(reduction_bps.min(10_000));
            ((i32::from(base_tariff_bps) * remaining) / 10_000) as Bps
        }
        None => base_tariff_bps,
    }
}

/// Validates domain access credentials (`PermissionedDomain` or `ClearinghouseBroker` `tfHybrid`).
pub fn check_offer_domain_access(
    seat: u8,
    domain_id: Option<u8>,
    is_hybrid: bool,
    credit_tier: CreditTier,
    domains: &[CartelDomain],
    has_broker_charter: bool,
) -> Result<(), EngineError> {
    if is_hybrid && !has_broker_charter {
        return Err(EngineError::Unauthorized(format!(
            "seat {seat} attempted tfHybrid without active ClearinghouseBroker charter"
        )));
    }
    let Some(d_id) = domain_id else {
        return Ok(());
    };
    let Some(dom) = domains.iter().find(|d| d.domain_id == d_id) else {
        return Err(EngineError::UnknownObject(format!("cartel domain {d_id}")));
    };
    if !credit_tier.may_access_prime_cartel() {
        return Err(EngineError::DomainAccessDenied(format!(
            "seat {seat} with tier {credit_tier:?} cannot access cartel domain {d_id}"
        )));
    }
    if !dom.members.contains(&seat) {
        if is_hybrid && has_broker_charter {
            return Ok(());
        }
        return Err(EngineError::DomainAccessDenied(format!(
            "seat {seat} lacks credential for cartel domain {d_id}"
        )));
    }
    Ok(())
}

/// Evaluates the pure `§5.2` (amended `2026-11-17`) Customs Inspector wash-freeze audit predicate
/// over `prev_turn_fills` for `target_seat`.
///
/// Confirmed (`true`) if either condition holds across the previous turn's settled fills:
/// - **(a) Round trip:** for some commodity `g` (`Raw(_)` or `Refined(_)`) and some peer seat
///   `Y != target_seat`, total gross units sent `target_seat -> Y` in `g` and total gross units
///   received `Y -> target_seat` in `g` are both `>= wash_round_trip_min_units`.
/// - **(b) Above town-import price:** `target_seat` participated as buyer or seller in a peer fill
///   (`from.is_some()`, excluding Authority `TownImport`) on commodity `g` whose
///   `unit_price_cred` exceeds `town_import_cost_cred(UNIT_SCALE, parities.asset_parity(g),
///   town_import_premium_bps)`.
#[must_use]
pub fn audit_wash_freeze(
    prev_turn_fills: &[FillRecord],
    target_seat: u8,
    parities: &FundamentalParities,
    town_import_premium_bps: Bps,
    wash_round_trip_min_units: Units,
) -> bool {
    if wash_round_trip_min_units <= 0 {
        return false;
    }

    let mut flows: BTreeMap<(u8, AssetId), (Units, Units)> = BTreeMap::new();
    for fill in prev_turn_fills {
        let Some(good) = fill.good else {
            continue;
        };
        if !matches!(good, AssetId::Raw(_) | AssetId::Refined(_)) {
            continue;
        }
        let Some(from) = fill.from else {
            // Authority TownImport has `from: None` and neither starts a peer round-trip nor counts
            // as an overpriced peer trade.
            continue;
        };
        let involves_target = from == target_seat || fill.to == target_seat;
        if !involves_target {
            continue;
        }
        if let Some(price) = fill.unit_price_cred
            && let Ok(import_unit_price) = town_import_cost_cred(
                UNIT_SCALE,
                parities.asset_parity(good),
                town_import_premium_bps,
            )
            && price > import_unit_price
        {
            return true;
        }
        if fill.units > 0 && from != fill.to {
            let peer = if from == target_seat { fill.to } else { from };
            let entry = flows.entry((peer, good)).or_insert((0, 0));
            if from == target_seat {
                entry.0 = entry.0.saturating_add(fill.units);
            } else {
                entry.1 = entry.1.saturating_add(fill.units);
            }
        }
    }

    flows
        .values()
        .any(|&(sent, recv)| sent >= wash_round_trip_min_units && recv >= wash_round_trip_min_units)
}
