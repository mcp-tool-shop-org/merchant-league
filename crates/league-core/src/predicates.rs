//! Deterministic state predicates: plain numbers describing one seat's position at one turn,
//! for the replay log and the paper-test metrics.
//!
//! Every predicate is a pure function of the engine state's public parts plus the asking seat's own
//! state. Each number comes from the engine's own rule code (the Sweep helpers in
//! [`crate::engine`] and the close mark in [`crate::scoring`]), never from a second copy of a rule.
//!
//! **What is public, by rule** (`docs/design-v2.md` §2, "What One Seat May Know About Another"): every seat's free
//! balances (raw, refined, `CRED`, auto-debt) and credit tier, and every resting offer, cartel-domain
//! books included. Private: mandate cards and their commitments until revealed, negotiation text,
//! committed-but-unrevealed orders, unpublished randomness and private notes.
//!
//! **What a predicate reads, and never reads.** It reads the table rules (`TableSpec`: the ring,
//! recipes, projects, closing hazards), the fundamental parities, realized active shocks, tariffs,
//! cartel domains, charters, resting offers, seats' balances and credit tiers. It never reads a
//! mandate card or mandate commitment, any seat's sealed or committed-but-unrevealed turn orders,
//! drawn shocks not yet realized, or beacons and seeds.
//!
//! **No predicate recommends anything.** The outputs are state descriptions only: no move, price,
//! quantity to trade or partner. Field names are code identifiers and are never shown to a seat.

pub use league_seat_types::{
    CoalitionReach, GoodScarcity, HoldingMark, ProjectCover, ProjectInputCover, RecipeCover,
    STATE_PREDICATE_SECTION_VERSION, STATE_PREDICATES_VERSION, SeasonEndAndMarks,
    SeatSeasonMetrics, SeatStatePredicates, SeatSweepOutcome, SpoilageAndConversion,
    StatePredicateSection, TurnPredicateRecord, UpkeepAndDebt,
};

use crate::contracts::OfferSide;
use crate::engine::{
    EngineState, TurnPhase, batches_covered_by_inputs, effective_transfer_tariff_bps,
    raw_yield_at_sweep, settle_upkeep, spoilage_bps_at_sweep,
};
use crate::scoring::{close_mark_breakdown, seat_close_stock};
use crate::types::{
    AssetId, Bps, CLOSING_ROLL_START_TURN, EngineError, HARD_TURN_CAP, Seat, Units, floor_bps_part,
};

/// Basis points at which a closing hazard makes the close certain (`roll < 10_000` always holds).
const CERTAIN_CLOSE_HAZARD_BPS: Bps = 10_000;

/// Public Phase-1 cancel-lock predicate (`true` once the cancel lock is engaged for this turn).
#[must_use]
pub const fn cancel_locked(state: &EngineState) -> bool {
    state.cancel_locked
}

fn seat_index(state: &EngineState, seat: Seat) -> Result<usize, EngineError> {
    let idx = usize::from(seat.0);
    if idx >= state.seats.len() || idx >= state.spec.seats.len() {
        return Err(EngineError::InvalidSeat(seat.0));
    }
    Ok(idx)
}

/// Predicate 1: upkeep owed, held, the shortfall, the debt it adds at this turn's Sweep and the
/// interest rate at the seat's credit tier, via the Sweep's own `settle_upkeep`. Reads only the
/// asking seat's own balances, offers and tier, and the table rules (all public by rule,
/// `docs/design-v2.md` §2, "What One Seat May Know About Another").
pub fn upkeep_and_debt(state: &EngineState, seat: Seat) -> Result<UpkeepAndDebt, EngineError> {
    let idx = seat_index(state, seat)?;
    let cfg = &state.spec.seats[idx];
    let own = &state.seats[idx];
    let upkeep_g = usize::from(cfg.upkeep_good);
    let held = own.refined.get(upkeep_g).copied().unwrap_or(0);
    let parity = state
        .parities
        .refined_per_unit
        .get(upkeep_g)
        .copied()
        .unwrap_or(0);
    let settlement = settle_upkeep(held, cfg.upkeep_amount, || parity);
    let (_, refined_stock) = seat_close_stock(state, seat.0);
    let on_sells = refined_stock
        .get(upkeep_g)
        .copied()
        .unwrap_or(0)
        .saturating_sub(held)
        .max(0);
    Ok(UpkeepAndDebt {
        upkeep_good: cfg.upkeep_good,
        owed_units: cfg.upkeep_amount,
        held_units: held,
        held_on_own_resting_sells: on_sells,
        shortfall_units: settlement.shortfall,
        debt_added_at_sweep: settlement.new_shortfall_debt,
        interest_bps: own.credit_tier.interest_bps(),
    })
}

/// Predicate 2: raw units that spoil at this turn's Sweep if nothing changes (no converter runs,
/// trades, fills, deliveries or Regent action), workshop capacity left, and full batches each
/// recipe's held inputs cover. Uses the Sweep's own yield, decay-rate and input-clamp helpers.
/// Reads only the asking seat's own balances and offers, the table rules and realized shocks (all
/// public by rule, `docs/design-v2.md` §2, "What One Seat May Know About Another").
pub fn spoilage_and_conversion(
    state: &EngineState,
    seat: Seat,
) -> Result<SpoilageAndConversion, EngineError> {
    let idx = seat_index(state, seat)?;
    let turn = state.turn;
    let n = state.spec.seats.len();
    let own = &state.seats[idx];
    let yield_now = raw_yield_at_sweep(state, idx, turn);
    let rates: Vec<Bps> = (0..n)
        .map(|g| spoilage_bps_at_sweep(state, g, turn))
        .collect();

    let mut held_spoil = Vec::with_capacity(n);
    for (g, &rate) in rates.iter().enumerate() {
        let mut free = own.raw.get(g).copied().unwrap_or(0);
        if g == idx {
            free = free.saturating_add(yield_now);
        }
        held_spoil.push(floor_bps_part(free, rate));
    }
    let mut on_offers: Units = 0;
    for offer in state.offers.values() {
        if offer.maker == seat.0
            && offer.side == OfferSide::Sell
            && let AssetId::Raw(g) = offer.good
            && let Some(&rate) = rates.get(usize::from(g))
        {
            on_offers = on_offers.saturating_add(floor_bps_part(offer.remaining_units, rate));
        }
    }
    let total = held_spoil
        .iter()
        .fold(on_offers, |acc, &v| acc.saturating_add(v));

    let recipes: Vec<RecipeCover> = state.spec.seats[idx]
        .converters
        .iter()
        .enumerate()
        .map(|(recipe_idx, recipe)| RecipeCover {
            recipe_idx,
            output_good: recipe.output_good,
            output_units_per_batch: recipe.output_amount,
            capacity_runs_left: recipe.max_runs_per_turn,
            batches_covered_by_inputs: batches_covered_by_inputs(recipe, &own.raw),
        })
        .collect();
    let capacity = recipes
        .iter()
        .fold(0_u32, |acc, r| acc.saturating_add(r.capacity_runs_left));

    Ok(SpoilageAndConversion {
        own_raw_good: seat.0,
        own_raw_yield_at_sweep: yield_now,
        spoilage_bps_by_good: rates,
        spoil_units_held_by_good: held_spoil,
        spoil_units_on_resting_offers: on_offers,
        spoil_units_total: total,
        workshop_capacity_runs_left: capacity,
        recipes,
    })
}

/// Predicate 3: for each good, the asking seat's own holding, the table-wide holdings of all seats,
/// the units on resting offers (open book and cartel-domain books counted apart), how many seats'
/// recipes or upkeep need it, and the open-book tariff in force.
///
/// Every input is public by rule (`docs/design-v2.md` §2, "What One Seat May Know About Another"): seats' free balances
/// and resting offers, domain books included, plus the ring, the recipes and the tariffs.
pub fn scarcity_and_tariffs(
    state: &EngineState,
    seat: Seat,
) -> Result<Vec<GoodScarcity>, EngineError> {
    let idx = seat_index(state, seat)?;
    let n = state.spec.seats.len();
    let own = &state.seats[idx];
    let goods = (0..n)
        .map(|g| AssetId::Raw(g as u8))
        .chain((0..n).map(|g| AssetId::Refined(g as u8)));
    let stocks: Vec<(Vec<Units>, Vec<Units>)> = (0..state.seats.len())
        .map(|s| seat_close_stock(state, s as u8))
        .collect();
    let mut out = Vec::with_capacity(2 * n);
    for good in goods {
        let (producer, own_units, seats_needing) = match good {
            AssetId::Raw(g) => (
                g,
                own.raw.get(usize::from(g)).copied().unwrap_or(0),
                state
                    .spec
                    .seats
                    .iter()
                    .filter(|cfg| {
                        cfg.converters
                            .iter()
                            .any(|c| c.inputs.iter().any(|&(raw_g, _)| raw_g == g))
                    })
                    .count(),
            ),
            AssetId::Refined(m) => (
                m,
                own.refined.get(usize::from(m)).copied().unwrap_or(0),
                state
                    .spec
                    .seats
                    .iter()
                    .filter(|cfg| cfg.upkeep_good == m)
                    .count(),
            ),
            AssetId::Cred => continue,
        };
        let (mut offered, mut bid, mut domain_offered, mut domain_bid): (
            Units,
            Units,
            Units,
            Units,
        ) = (0, 0, 0, 0);
        for offer in state.offers.values() {
            if offer.good != good || offer.remaining_units <= 0 {
                continue;
            }
            let slot = match (offer.side, offer.domain_id.is_some()) {
                (OfferSide::Sell, false) => &mut offered,
                (OfferSide::Buy, false) => &mut bid,
                (OfferSide::Sell, true) => &mut domain_offered,
                (OfferSide::Buy, true) => &mut domain_bid,
            };
            *slot = slot.saturating_add(offer.remaining_units);
        }
        out.push(GoodScarcity {
            good,
            producer_seat: producer,
            own_units,
            table_units_held: stocks.iter().fold(0, |acc: Units, (raw, refined)| {
                let held = match good {
                    AssetId::Raw(g) => raw.get(usize::from(g)),
                    AssetId::Refined(g) => refined.get(usize::from(g)),
                    AssetId::Cred => None,
                };
                acc.saturating_add(held.copied().unwrap_or(0))
            }),
            open_book_units_offered: offered,
            open_book_units_bid: bid,
            domain_book_units_offered: domain_offered,
            domain_book_units_bid: domain_bid,
            seats_needing: u8::try_from(seats_needing).unwrap_or(u8::MAX),
            open_book_tariff_bps: effective_transfer_tariff_bps(
                state, good, None, producer, seat.0,
            ),
        });
    }
    Ok(out)
}

/// Predicate 4: for the named set `members` (`2..=N-1` distinct seats including `seat`), which of
/// its members' upkeep goods some member's recipe makes from raw goods members extract, and which
/// not. Reads only the ring and the recipes (public table rules, `docs/design-v2.md` §2, "What One Seat May Know About Another");
/// it says what the rules allow, not what seats will do.
pub fn coalition_reach(
    state: &EngineState,
    seat: Seat,
    members: &[u8],
) -> Result<CoalitionReach, EngineError> {
    seat_index(state, seat)?;
    let n = state.spec.seats.len();
    let mut set: Vec<u8> = members.to_vec();
    set.sort_unstable();
    set.dedup();
    if set.len() != members.len() || set.len() < 2 || set.len() + 1 > n {
        return Err(EngineError::InvalidOrder(format!(
            "coalition must name 2..={} distinct seats",
            n.saturating_sub(1)
        )));
    }
    if set.iter().any(|&m| usize::from(m) >= n) {
        return Err(EngineError::InvalidOrder(
            "coalition names a seat outside the table".to_owned(),
        ));
    }
    if !set.contains(&seat.0) {
        return Err(EngineError::InvalidOrder(
            "coalition must include the asking seat".to_owned(),
        ));
    }
    let mut upkeep_goods: Vec<u8> = set
        .iter()
        .map(|&m| state.spec.seats[usize::from(m)].upkeep_good)
        .collect();
    upkeep_goods.sort_unstable();
    upkeep_goods.dedup();
    let (makeable, not_makeable): (Vec<u8>, Vec<u8>) =
        upkeep_goods.into_iter().partition(|&good| {
            set.contains(&good)
                && state.spec.seats[usize::from(good)]
                    .converters
                    .iter()
                    .any(|c| {
                        c.output_good == good
                            && c.inputs.iter().all(|&(raw_g, _)| set.contains(&raw_g))
                    })
        });
    Ok(CoalitionReach {
        members: set,
        upkeep_goods_makeable: makeable,
        upkeep_goods_not_makeable: not_makeable,
    })
}

/// Predicate 4 for every set of `2..=N-1` seats that includes `seat`, in ascending bitmask order.
pub fn all_coalitions_reach(
    state: &EngineState,
    seat: Seat,
) -> Result<Vec<CoalitionReach>, EngineError> {
    seat_index(state, seat)?;
    let n = state.spec.seats.len();
    let mut out = Vec::new();
    for mask in 0_u32..(1_u32 << n) {
        if mask & (1 << seat.0) == 0 {
            continue;
        }
        let members: Vec<u8> = (0..n as u8).filter(|&s| mask & (1 << s) != 0).collect();
        if members.len() < 2 || members.len() + 1 > n {
            continue;
        }
        out.push(coalition_reach(state, seat, &members)?);
    }
    Ok(out)
}

/// Turns from `turn` to the first close turn that `pred(hazard)` admits (`HARD_TURN_CAP` always does).
fn turns_until_close(state: &EngineState, pred: impl Fn(Bps) -> bool) -> u8 {
    let turn = state.turn;
    let close_at = (turn..HARD_TURN_CAP)
        .find(|&t| {
            t >= CLOSING_ROLL_START_TURN
                && state
                    .spec
                    .closing_hazard_bps
                    .get(usize::from(t - CLOSING_ROLL_START_TURN))
                    .is_some_and(|&hz| pred(hz))
        })
        .unwrap_or(HARD_TURN_CAP);
    close_at.saturating_sub(turn)
}

/// Predicate 5: Sweeps until the earliest and the latest possible close, every good the seat holds
/// marked by the close-marking rule as if the season closed now (via the rule's own code), and each
/// project bundle against the seat's refined holdings.
///
/// Every input is public by rule (`docs/design-v2.md` §2, "What One Seat May Know About Another"): the seat's own
/// stock, and other seats' resting bids, credit tiers, wash freezes, domains and charters.
///
/// **Which bids the marks read.** Exactly the resting bids the close rule's own filter
/// (`bid_executable_by_seller` + `remaining_units >= UNIT_SCALE` in `scoring.rs`) lets the
/// asking seat sell into: a resting `Buy` on the good with at least one whole unit (`>= UNIT_SCALE`)
/// left, made by another seat whose wash freeze has ended; on the open book any such bid; in a
/// cartel domain only when the maker's credit tier may use prime domains and either the asking seat
/// is a domain member with such a tier (and the maker is a member or the bid is a hybrid bid from
/// a maker holding an active Clearinghouse Broker charter), or the asking seat holds an active
/// Clearinghouse Broker charter, or the bid is a hybrid bid from a maker holding one. A domain bid
/// outside those cases is never read and changes nothing here. Since cartel-domain books are
/// public by the same rule, reading the executable domain bids is simply the close rule applied.
pub fn season_end_and_marks(
    state: &EngineState,
    seat: Seat,
) -> Result<SeasonEndAndMarks, EngineError> {
    let idx = seat_index(state, seat)?;
    let n = state.spec.seats.len();
    let season_closed = matches!(
        state.phase,
        TurnPhase::AwaitingMandateReveal | TurnPhase::Completed
    );
    let (earliest, latest) = if season_closed {
        (0, 0)
    } else {
        (
            turns_until_close(state, |hz| hz > 0),
            turns_until_close(state, |hz| hz >= CERTAIN_CLOSE_HAZARD_BPS),
        )
    };

    let (raw_stock, refined_stock) = seat_close_stock(state, seat.0);
    let goods = (0..n)
        .map(|g| {
            (
                AssetId::Raw(g as u8),
                raw_stock.get(g).copied().unwrap_or(0),
            )
        })
        .chain((0..n).map(|g| {
            (
                AssetId::Refined(g as u8),
                refined_stock.get(g).copied().unwrap_or(0),
            )
        }));
    let holdings = goods
        .map(|(good, units)| {
            let mark = close_mark_breakdown(state, seat.0, good, units);
            HoldingMark {
                good,
                units,
                marked_value: mark.marked_value,
                units_marked_by_bids: mark.units_marked_by_bids,
                units_at_floor: mark.units_at_floor,
                value_at_floor: mark.value_at_floor,
            }
        })
        .collect();

    let own = &state.seats[idx];
    let projects = state
        .spec
        .projects
        .iter()
        .map(|proj| {
            let inputs: Vec<ProjectInputCover> = proj
                .inputs
                .iter()
                .map(|&(good, per_batch)| ProjectInputCover {
                    good,
                    units_per_batch: per_batch,
                    held_units: own.refined.get(usize::from(good)).copied().unwrap_or(0),
                })
                .collect();
            let batches_covered = inputs
                .iter()
                .map(|inp| {
                    if inp.units_per_batch <= 0 {
                        0
                    } else {
                        u32::try_from(inp.held_units.max(0) / inp.units_per_batch)
                            .unwrap_or(u32::MAX)
                    }
                })
                .min()
                .unwrap_or(0);
            ProjectCover {
                project_id: proj.id,
                inputs,
                batches_covered,
            }
        })
        .collect();

    Ok(SeasonEndAndMarks {
        season_closed,
        turns_until_earliest_close: earliest,
        turns_until_latest_close: latest,
        holdings,
        projects,
    })
}

/// All five predicates for `seat` at the state's current turn. Reads `state` only; changes nothing.
pub fn compute_seat_state_predicates(
    state: &EngineState,
    seat: Seat,
) -> Result<SeatStatePredicates, EngineError> {
    Ok(SeatStatePredicates {
        version: STATE_PREDICATES_VERSION,
        table_id: state.table_id,
        turn: state.turn,
        phase: state.phase,
        seat: seat.0,
        upkeep: upkeep_and_debt(state, seat)?,
        spoilage: spoilage_and_conversion(state, seat)?,
        scarcity: scarcity_and_tariffs(state, seat)?,
        coalitions: all_coalitions_reach(state, seat)?,
        season_end: season_end_and_marks(state, seat)?,
        cancel_locked: cancel_locked(state),
    })
}

fn seat_record(records: &[SeatStatePredicates], seat: u8) -> Option<&SeatStatePredicates> {
    records.iter().find(|p| p.seat == seat)
}

/// Upkeep units the seat could have had at the Sweep from the start-of-turn predicates alone: what
/// it held free (predicate 1) plus the most any single one of its recipes that makes the upkeep good
/// could add (predicate 2: `min(batches covered by held inputs, runs left) * units per batch`).
///
/// The recipe term is unreachable on a validated table: `TableSpec::validate` requires every
/// recipe's output to be its owner's own refined good, and a seat's upkeep good is never its own,
/// so no recipe of the seat makes its upkeep good. It is kept so the count stays correct if a
/// table type ever allows it.
fn coverable_upkeep_units(p: &SeatStatePredicates) -> Units {
    let convertible = p
        .spoilage
        .recipes
        .iter()
        .filter(|r| r.output_good == p.upkeep.upkeep_good)
        .map(|r| {
            let runs = r.batches_covered_by_inputs.min(r.capacity_runs_left);
            r.output_units_per_batch.saturating_mul(i64::from(runs))
        })
        .max()
        .unwrap_or(0);
    p.upkeep.held_units.saturating_add(convertible)
}

fn short_turns_where(
    section: &StatePredicateSection,
    seat: u8,
    covered: impl Fn(&SeatStatePredicates) -> bool,
) -> Vec<u8> {
    section
        .turns
        .iter()
        .filter(|rec| {
            let had_shortfall = rec
                .sweep
                .iter()
                .any(|o| o.seat == seat && o.upkeep_shortfall_units > 0);
            had_shortfall && seat_record(&rec.at_turn_start, seat).is_some_and(&covered)
        })
        .map(|rec| rec.turn)
        .collect()
}

/// Metric 1a: the turns on which `seat`'s Sweep left an upkeep shortfall although, at the start of
/// that turn, it held enough of the upkeep good **free** to cover it (predicate 1), or could have
/// converted enough (predicate 2; unreachable on a validated table, see `coverable_upkeep_units`).
/// On a validated table this is exactly "held enough free at the start, still short at the Sweep".
#[must_use]
pub fn held_enough_but_short_turns(section: &StatePredicateSection, seat: u8) -> Vec<u8> {
    short_turns_where(section, seat, |p| {
        coverable_upkeep_units(p) >= p.upkeep.owed_units
    })
}

/// Metric 1b: the turns on which, at the start of the turn, `seat`'s free units of its upkeep good
/// plus the units on its own resting `Sell` offers of it were at least the upkeep owed (predicate
/// 1), and the Sweep still left an upkeep shortfall. It records those two facts only: it does not
/// say the shortfall was avoidable, since another seat can fill the resting sell (or cash a due
/// check on that good) in the same turn before the seat could cancel. Includes every turn of
/// [`held_enough_but_short_turns`] that the free units alone cover.
#[must_use]
pub fn covered_with_own_offers_but_short_turns(
    section: &StatePredicateSection,
    seat: u8,
) -> Vec<u8> {
    short_turns_where(section, seat, |p| {
        p.upkeep
            .held_units
            .saturating_add(p.upkeep.held_on_own_resting_sells)
            >= p.upkeep.owed_units
    })
}

/// Metric 2, spoiled units: the raw units `seat` lost to spoilage over the season, summed from the
/// engine's own Step 6 outcome on every played turn.
#[must_use]
pub fn spoiled_units_over_season(section: &StatePredicateSection, seat: u8) -> Units {
    section
        .turns
        .iter()
        .flat_map(|rec| rec.sweep.iter())
        .filter(|o| o.seat == seat)
        .fold(0, |acc: Units, o| acc.saturating_add(o.spoiled_units))
}

/// Metric 3, stranded stock at the close: `(units, value)` of `seat`'s holdings in the closed state
/// that the close-marking rule valued at the pooled `50%` floor, summed over goods.
#[must_use]
pub fn stranded_stock_at_close(section: &StatePredicateSection, seat: u8) -> (Units, Units) {
    seat_record(&section.at_close, seat).map_or((0, 0), |p| {
        p.season_end
            .holdings
            .iter()
            .fold((0, 0), |(u, v): (Units, Units), h| {
                (
                    u.saturating_add(h.units_at_floor),
                    v.saturating_add(h.value_at_floor),
                )
            })
    })
}

/// The three metrics for every seat that appears in the section's close records.
#[must_use]
pub fn compute_season_metrics(section: &StatePredicateSection) -> Vec<SeatSeasonMetrics> {
    section
        .at_close
        .iter()
        .map(|p| {
            let (stranded_units_at_close, stranded_value_at_close) =
                stranded_stock_at_close(section, p.seat);
            SeatSeasonMetrics {
                seat: p.seat,
                held_enough_but_short_turns: held_enough_but_short_turns(section, p.seat),
                covered_with_own_offers_but_short_turns: covered_with_own_offers_but_short_turns(
                    section, p.seat,
                ),
                spoiled_units: spoiled_units_over_season(section, p.seat),
                stranded_units_at_close,
                stranded_value_at_close,
            }
        })
        .collect()
}
