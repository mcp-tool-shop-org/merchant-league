use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::engine_types::TurnPhase;
use crate::types::{AssetId, Bps, TableId, Units};

/// Version of [`SeatStatePredicates`]; bumped whenever a field's meaning or layout changes.
pub const STATE_PREDICATES_VERSION: u32 = 3;

/// Version of [`StatePredicateSection`]; bumped whenever a field's meaning or layout changes.
pub const STATE_PREDICATE_SECTION_VERSION: u32 = 3;

/// Predicate 1: upkeep and debt at this turn's Sweep (Step 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpkeepAndDebt {
    /// Refined good index owed as upkeep.
    pub upkeep_good: u8,
    /// Units of the upkeep good owed this turn.
    pub owed_units: Units,
    /// Units of the upkeep good the seat holds free now.
    pub held_units: Units,
    /// Units of the upkeep good on the seat's own resting `Sell` offers (not free until cancelled).
    pub held_on_own_resting_sells: Units,
    /// Units owed and not covered by `held_units` (`0` when covered).
    pub shortfall_units: Units,
    /// `CRED` auto-debt the shortfall would add at this turn's Sweep.
    pub debt_added_at_sweep: Units,
    /// Per-turn auto-debt interest rate (bps) at the seat's current credit tier.
    pub interest_bps: Bps,
}

/// One of the seat's converter recipes, against its current raw holdings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeCover {
    /// Index into the seat's `converters`.
    pub recipe_idx: usize,
    /// Refined good the recipe makes.
    pub output_good: u8,
    /// Refined units one batch makes.
    pub output_units_per_batch: Units,
    /// Runs left this turn under the recipe's cap (runs are only used at the Sweep).
    pub capacity_runs_left: u32,
    /// Full batches the seat's held inputs cover, ignoring the cap.
    pub batches_covered_by_inputs: u32,
}

/// Predicate 2: spoilage and conversion at this turn's Sweep (Steps 2, 4b and 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpoilageAndConversion {
    /// The seat's own raw good index.
    pub own_raw_good: u8,
    /// Raw units of its own good minted to the seat at this turn's Sweep (Step 4b).
    pub own_raw_yield_at_sweep: Units,
    /// Step 6 decay rate (bps) on each raw good this turn, indexed by good.
    pub spoilage_bps_by_good: Vec<Bps>,
    /// Free raw units of each good that spoil at this turn's Sweep if nothing changes, indexed by
    /// good (the own good includes this turn's yield).
    pub spoil_units_held_by_good: Vec<Units>,
    /// Raw units on the seat's own resting raw `Sell` offers that spoil at this turn's Sweep.
    pub spoil_units_on_resting_offers: Units,
    /// All raw units the seat loses to spoilage at this turn's Sweep if nothing changes.
    pub spoil_units_total: Units,
    /// Sum of `capacity_runs_left` over the seat's recipes.
    pub workshop_capacity_runs_left: u32,
    /// Each of the seat's recipes against its held inputs.
    pub recipes: Vec<RecipeCover>,
}

/// Predicate 3, for one good: what is public about its scarcity, plus the seat's own holding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodScarcity {
    /// The good.
    pub good: AssetId,
    /// The only seat that produces it (the ring: `Raw(i)` is extracted, `Refined(i)` converted, by seat `i`).
    pub producer_seat: u8,
    /// Units the asking seat holds free.
    pub own_units: Units,
    /// Units all seats hold of it: free balances plus units on their resting `Sell` offers (the
    /// stock the close rule marks), summed over the table.
    pub table_units_held: Units,
    /// Units on resting open-book `Sell` offers, all makers.
    pub open_book_units_offered: Units,
    /// Units on resting open-book `Buy` offers, all makers.
    pub open_book_units_bid: Units,
    /// Units on resting `Sell` offers in cartel-domain books, all domains and makers.
    pub domain_book_units_offered: Units,
    /// Units on resting `Buy` offers in cartel-domain books, all domains and makers.
    pub domain_book_units_bid: Units,
    /// Seats whose converter recipes take it (raw goods) or whose upkeep is it (refined goods).
    pub seats_needing: u8,
    /// Export tariff (bps) in force on an open-book transfer of it this turn.
    pub open_book_tariff_bps: Bps,
}

/// Predicate 4, for one named set of seats: which of its members' upkeep goods the set can make
/// among its members, from the ring and the recipes alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoalitionReach {
    /// Members, ascending (`2..=N-1` seats, the asking seat included).
    pub members: Vec<u8>,
    /// Upkeep goods of members that a member's recipe makes from raw goods members extract.
    pub upkeep_goods_makeable: Vec<u8>,
    /// Upkeep goods of members that no member's recipe makes from members' raw goods alone.
    pub upkeep_goods_not_makeable: Vec<u8>,
}

/// One holding valued by the close-marking rule as if the season closed now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldingMark {
    /// The good.
    pub good: AssetId,
    /// Units the close rule would mark (free plus the seat's own resting `Sell` offers).
    pub units: Units,
    /// The close mark (`mark_good_holding_at_close`).
    pub marked_value: Units,
    /// Units marked by standing `>= UNIT_SCALE` (`1.000`-unit) bids above the `50%` floor.
    pub units_marked_by_bids: Units,
    /// Units marked at the pooled `50%` floor.
    pub units_at_floor: Units,
    /// The floor's value for `units_at_floor`.
    pub value_at_floor: Units,
}

/// One project input against the seat's holdings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInputCover {
    /// Refined good index.
    pub good: u8,
    /// Units one batch takes.
    pub units_per_batch: Units,
    /// Units the seat holds free.
    pub held_units: Units,
}

/// One project bundle against the seat's refined holdings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCover {
    /// Project identifier.
    pub project_id: u16,
    /// Each input with the seat's holding of it.
    pub inputs: Vec<ProjectInputCover>,
    /// Full batches the seat's holdings cover.
    pub batches_covered: u32,
}

/// Predicate 5: season end and marks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonEndAndMarks {
    /// `true` once the season has closed (`AwaitingMandateReveal` or `Completed`).
    pub season_closed: bool,
    /// Sweeps after this turn's before the earliest possible close (`0`: this turn's Sweep can close it).
    pub turns_until_earliest_close: u8,
    /// Sweeps after this turn's before the latest possible close (`0`: this turn's Sweep closes it).
    pub turns_until_latest_close: u8,
    /// Every raw and refined good, marked as if the season closed now.
    pub holdings: Vec<HoldingMark>,
    /// Every project bundle (projects never close) against the seat's refined holdings.
    pub projects: Vec<ProjectCover>,
}

/// The five predicates' outputs for one seat at one turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatStatePredicates {
    /// [`STATE_PREDICATES_VERSION`] at the time of writing.
    pub version: u32,
    /// Table identifier.
    pub table_id: TableId,
    /// Turn the state is at.
    pub turn: u8,
    /// Phase the state is in.
    pub phase: TurnPhase,
    /// The asking seat.
    pub seat: u8,
    /// Predicate 1.
    pub upkeep: UpkeepAndDebt,
    /// Predicate 2.
    pub spoilage: SpoilageAndConversion,
    /// Predicate 3, one entry per good (`Raw(0..N)` then `Refined(0..N)`).
    pub scarcity: Vec<GoodScarcity>,
    /// Predicate 4, for every set of `2..=N-1` seats that includes the asking seat.
    pub coalitions: Vec<CoalitionReach>,
    /// Predicate 5.
    pub season_end: SeasonEndAndMarks,
    /// Public Phase-1 cancel-lock status (`C4` amended `2026-11-17`): `true` once `begin_cancel_lock`
    /// has engaged during `TalkAndMarket`, cleared when Phase 1 ends.
    /// Always `false` in recorded predicates: they are taken at turn start, before any lock.
    pub cancel_locked: bool,
}

/// What one seat's Sweep actually did, as the engine's own Sweep reported it (not a prediction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatSweepOutcome {
    /// Seat index.
    pub seat: u8,
    /// Upkeep units owed and not paid at Step 5.
    pub upkeep_shortfall_units: Units,
    /// `CRED` auto-debt Step 5 added for that shortfall.
    pub shortfall_debt_added: Units,
    /// Raw units Step 6 removed from the seat (free balances plus its resting raw `Sell` offers).
    pub spoiled_units: Units,
}

/// One turn of the section: every seat's predicates at the start of the turn (before any Phase-1
/// action), and what the turn's Sweep then did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnPredicateRecord {
    /// Turn number.
    pub turn: u8,
    /// Predicates per seat, ascending seat order, at the start of the turn.
    pub at_turn_start: Vec<SeatStatePredicates>,
    /// The Sweep's outcome per seat, ascending seat order.
    pub sweep: Vec<SeatSweepOutcome>,
}

/// The optional replay-record section: state predicates for every seat and turn of a replayed
/// season, and every seat's predicates once the season has closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatePredicateSection {
    /// [`STATE_PREDICATE_SECTION_VERSION`] at the time of writing.
    pub version: u32,
    /// Table identifier.
    pub table_id: TableId,
    /// One record per played turn, in order.
    pub turns: Vec<TurnPredicateRecord>,
    /// Predicates per seat in the closed state (after the closing Sweep, before mandate reveals).
    pub at_close: Vec<SeatStatePredicates>,
}

/// The three paper-test metrics for one seat over a finished season.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatSeasonMetrics {
    /// Seat index.
    pub seat: u8,
    /// Turns short at the Sweep although enough was held free (`held_enough_but_short_turns`).
    pub held_enough_but_short_turns: Vec<u8>,
    /// Turns short at the Sweep although free plus own resting sells covered it
    /// (`covered_with_own_offers_but_short_turns`).
    pub covered_with_own_offers_but_short_turns: Vec<u8>,
    /// Raw units lost to spoilage over the season (`spoiled_units_over_season`).
    pub spoiled_units: Units,
    /// Units held at the close that the close rule marked at the `50%` floor (`stranded_stock_at_close`).
    pub stranded_units_at_close: Units,
    /// The floor value of those units.
    pub stranded_value_at_close: Units,
}
