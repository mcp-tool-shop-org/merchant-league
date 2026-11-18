//! Deterministic rules-only Club Mode opponent policy ([`ClubBot`]).
//!
//! Strictly consumes [`SeatObservation`], [`PrivateMandate`], and [`preview_seat_bundle`] — never
//! any raw engine types, hidden state, secrets, or counterparty private information.

use std::collections::BTreeSet;

use crate::commit::{
    CharterBidOrder, ConverterRunOrder, CouncilVoteBallot, MandateKind, PrivateMandate,
    ProjectDeliveryOrder, SealedOrderBundle,
};
use crate::contracts::{CheckStatus, MarketAction, OfferProposal, OfferSide};
use crate::engine::TurnPhase;
use crate::observation::{AiPreviewRequest, SeatObservation, preview_seat_bundle};
use crate::types::{
    AssetId, BPS_SCALE, CharterRole, UNIT_SCALE, Units, checked_ceil_units_parity,
    mul_units_parity, town_import_cost_cred,
};

/// Deterministic decision plan produced for one turn by [`ClubBot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubBotDecision {
    /// Phase-1 Pass A actions executed before `cancel_lock`.
    pub pass_a_actions: Vec<MarketAction>,
    /// Phase-1 Pass B actions executed after `cancel_lock`.
    pub pass_b_actions: Vec<MarketAction>,
    /// Phase-2 sealed order bundle committed and revealed for Sweep.
    pub bundle: SealedOrderBundle,
}

/// Pure-Rust deterministic rules-only opponent policy for offline Club Mode (`docs/club-mode-design.md` §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClubBot {
    bot_tie_seed: u64,
}

impl ClubBot {
    /// Constructs a deterministic [`ClubBot`] keyed by `bot_tie_seed` for reproducible tie-breaking.
    #[must_use]
    pub const fn new(bot_tie_seed: u64) -> Self {
        Self { bot_tie_seed }
    }

    /// Returns the deterministic tie-break seed configured on this [`ClubBot`].
    #[must_use]
    pub const fn bot_tie_seed(&self) -> u64 {
        self.bot_tie_seed
    }

    /// Plans Phase-1 Pass A market actions from `obs` and `own_mandate` (`!obs.cancel_locked`).
    #[must_use]
    pub fn plan_pass_a(
        &self,
        obs: &SeatObservation,
        own_mandate: &PrivateMandate,
    ) -> Vec<MarketAction> {
        let Some(ctx) = BotTurnContext::new(obs) else {
            return Vec::new();
        };
        if obs.phase != TurnPhase::TalkAndMarket || obs.cancel_locked {
            return Vec::new();
        }
        let actions = self.build_pass_a_candidates(&ctx, own_mandate);
        Self::prune_refused_actions(obs, actions, &SealedOrderBundle::default())
    }

    /// Plans Phase-1 Pass B actions (`TownImport` fallback) and the Phase-2 [`SealedOrderBundle`].
    #[must_use]
    pub fn plan_pass_b_and_bundle(
        &self,
        obs: &SeatObservation,
        own_mandate: &PrivateMandate,
    ) -> (Vec<MarketAction>, SealedOrderBundle) {
        let Some(ctx) = BotTurnContext::new(obs) else {
            return (Vec::new(), SealedOrderBundle::default());
        };
        let mut bundle = self.build_bundle_candidates(&ctx, own_mandate);
        let pass_b_actions = if obs.phase == TurnPhase::TalkAndMarket {
            let candidates = self.build_pass_b_candidates(&ctx, &bundle);
            Self::prune_refused_actions(obs, candidates, &bundle)
        } else {
            Vec::new()
        };

        // Safeguard upkeep metric (`held_enough_but_short_turns == []`): if baseline held enough
        // upkeep free and our simulated Sweep would nonetheless end with upkeep shortfall debt,
        // drop discretionary project deliveries so Step 4 never starves Step 5.
        let baseline_upkeep = &obs.baseline_predicates.upkeep;
        if baseline_upkeep.held_units >= baseline_upkeep.owed_units
            && !bundle.project_deliveries.is_empty()
            && let Some(preview) = Self::run_preview(obs, &pass_b_actions, &bundle)
            && preview.upkeep.new_shortfall_debt > 0
        {
            bundle.project_deliveries.clear();
        }

        (pass_b_actions, bundle)
    }

    /// Plans Pass A actions, Pass B actions, and Phase-2 bundle deterministically for a turn.
    #[must_use]
    pub fn decide_full_turn(
        &self,
        obs: &SeatObservation,
        own_mandate: &PrivateMandate,
    ) -> ClubBotDecision {
        let pass_a_actions = self.plan_pass_a(obs, own_mandate);
        let (pass_b_actions, bundle) = self.plan_pass_b_and_bundle(obs, own_mandate);
        ClubBotDecision {
            pass_a_actions,
            pass_b_actions,
            bundle,
        }
    }

    fn build_pass_a_candidates(
        &self,
        ctx: &BotTurnContext<'_>,
        own_mandate: &PrivateMandate,
    ) -> Vec<MarketAction> {
        let obs = ctx.obs;
        let seat = ctx.seat;
        let seat_idx = ctx.seat_idx;
        let seat_cfg = ctx.seat_cfg;
        let mut actions = Vec::new();

        let mut free_cred = obs.seats[seat_idx].cred;
        let mut free_raw = obs.seats[seat_idx].raw.clone();
        let mut free_refined = obs.seats[seat_idx].refined.clone();

        // 1. Pull stale resting own offers from earlier turns plus any resting Sell on our own
        // `upkeep_good`, unlocking trapped CRED/goods before quoting fresh prices.
        for offer in obs.offers.values() {
            if offer.maker != seat {
                continue;
            }
            let is_own_upkeep_sell = offer.side == OfferSide::Sell
                && offer.good == AssetId::Refined(seat_cfg.upkeep_good);
            if offer.created_turn < obs.turn || is_own_upkeep_sell {
                actions.push(MarketAction::CancelOffer {
                    seat,
                    offer_id: offer.offer_id,
                });
                match offer.side {
                    OfferSide::Buy => {
                        free_cred = free_cred.saturating_add(offer.locked_cred);
                    }
                    OfferSide::Sell => match offer.good {
                        AssetId::Raw(g) => {
                            if let Some(slot) = free_raw.get_mut(usize::from(g)) {
                                *slot = slot.saturating_add(offer.remaining_units);
                            }
                        }
                        AssetId::Refined(m) => {
                            if let Some(slot) = free_refined.get_mut(usize::from(m)) {
                                *slot = slot.saturating_add(offer.remaining_units);
                            }
                        }
                        AssetId::Cred => {}
                    },
                }
            }
        }

        // Wash-freeze blocks `PostOffer` during `turn <= wash_frozen_until_turn`.
        if obs.seats[seat_idx].wash_frozen_until_turn >= obs.turn {
            return actions;
        }

        let upkeep_g = seat_cfg.upkeep_good;
        let upkeep_good = AssetId::Refined(upkeep_g);
        let upkeep_parity = obs.parities.asset_parity(upkeep_good).max(1);
        let one_turn_upkeep_reserve =
            mul_units_parity(seat_cfg.upkeep_amount, upkeep_parity).saturating_mul(3) / 2;

        // 2. Bid for `upkeep_good` up to `2x` `upkeep_amount` reserve at a price strictly below
        // `town_import_premium_bps` (`2.25x` default).
        let target_upkeep_stock = seat_cfg.upkeep_amount.saturating_mul(2);
        let current_upkeep = free_refined
            .get(usize::from(upkeep_g))
            .copied()
            .unwrap_or(0);
        let resting_upkeep_bid = Self::own_open_bid_units(obs, seat, upkeep_good);
        let missing_upkeep = target_upkeep_stock
            .saturating_sub(current_upkeep)
            .saturating_sub(resting_upkeep_bid);
        if missing_upkeep > 0 && free_cred > 0 {
            let max_prem = i64::from(obs.spec.town_import_premium_bps).saturating_sub(2_000);
            let bid_mult_bps = if current_upkeep < seat_cfg.upkeep_amount {
                15_500_i64.min(max_prem)
            } else {
                12_500_i64.min(max_prem)
            };
            let bid_price = ((i128::from(upkeep_parity) * i128::from(bid_mult_bps)) / BPS_SCALE)
                .max(1) as Units;
            if let Some(amount) = Self::affordable_bid_units(missing_upkeep, free_cred, bid_price)
                && amount >= 200
                && let Ok(lock) = checked_ceil_units_parity(amount, bid_price)
            {
                free_cred = free_cred.saturating_sub(lock);
                actions.push(MarketAction::PostOffer {
                    seat,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: upkeep_good,
                        amount,
                        price_per_unit_cred: bid_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                });
            }
        }

        // 3. Bid for foreign `Raw` inputs needed by profitable converter recipes, keeping at least
        // `1.5x` next-turn upkeep CRED reserve when feasible.
        let cred_budget = free_cred.saturating_sub(one_turn_upkeep_reserve / 2);
        let mut remaining_input_budget = cred_budget.max(0);
        let ranked_recipes = Self::ranked_converter_recipes(ctx);
        for &(recipe_idx, _margin) in &ranked_recipes {
            let recipe = &seat_cfg.converters[recipe_idx];
            for &(in_g, per_run) in &recipe.inputs {
                if in_g == seat {
                    continue;
                }
                let raw_asset = AssetId::Raw(in_g);
                let held = free_raw.get(usize::from(in_g)).copied().unwrap_or(0);
                let open_bid = Self::own_open_bid_units(obs, seat, raw_asset);
                let target_input = per_run.saturating_mul(2);
                let needed = target_input.saturating_sub(held).saturating_sub(open_bid);
                if needed < 200 || remaining_input_budget <= 0 {
                    continue;
                }
                let parity = obs.parities.asset_parity(raw_asset).max(1);
                let bid_price = ((i128::from(parity) * 11_200_i128) / BPS_SCALE).max(1) as Units;
                if let Some(amount) =
                    Self::affordable_bid_units(needed, remaining_input_budget, bid_price)
                    && amount >= 200
                    && let Ok(lock) = checked_ceil_units_parity(amount, bid_price)
                {
                    remaining_input_budget = remaining_input_budget.saturating_sub(lock);
                    free_cred = free_cred.saturating_sub(lock);
                    actions.push(MarketAction::PostOffer {
                        seat,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good: raw_asset,
                            amount,
                            price_per_unit_cred: bid_price,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    });
                }
            }
        }

        // 4. Quote perishable `Raw(seat)` surplus so idle raw stock converts into CRED instead of
        // spoiling on Step 6 `Clawback` (`turns_until_latest_close` tightens quote near season end).
        let near_close = obs.baseline_predicates.season_end.turns_until_latest_close <= 4;
        let own_raw_free = free_raw.get(seat_idx).copied().unwrap_or(0);
        let raw_reserve_needed = Self::own_raw_used_by_local_recipes(seat_cfg, seat);
        let sellable_raw = own_raw_free.saturating_sub(raw_reserve_needed);
        if sellable_raw >= 500 {
            let own_raw_asset = AssetId::Raw(seat);
            let parity = obs.parities.asset_parity(own_raw_asset).max(1);
            let ask_bps = if near_close { 9_400_i128 } else { 10_000_i128 };
            let ask_price = ((i128::from(parity) * ask_bps) / BPS_SCALE).max(1) as Units;
            let quote_units = ((sellable_raw * 9) / 10).max(500).min(sellable_raw);
            actions.push(MarketAction::PostOffer {
                seat,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: own_raw_asset,
                    amount: quote_units,
                    price_per_unit_cred: ask_price,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }

        // 5. Quote excess `Refined(seat)` (never selling `upkeep_good`!), sparing project &
        // `MinRefinedVariety` mandate buffers.
        if seat != upkeep_g {
            let own_ref_asset = AssetId::Refined(seat);
            let own_ref_free = free_refined.get(seat_idx).copied().unwrap_or(0);
            let min_keep = match own_mandate.kind {
                MandateKind::MinRefinedVariety { min_each, .. } => min_each,
                _ => 500,
            };
            let sellable_ref = own_ref_free.saturating_sub(min_keep);
            if sellable_ref >= 500 {
                let parity = obs.parities.asset_parity(own_ref_asset).max(1);
                let ask_bps = if near_close { 10_500_i128 } else { 11_200_i128 };
                let ask_price = ((i128::from(parity) * ask_bps) / BPS_SCALE).max(1) as Units;
                let quote_units = ((sellable_ref * 3) / 4).max(500).min(sellable_ref);
                actions.push(MarketAction::PostOffer {
                    seat,
                    order: OfferProposal {
                        side: OfferSide::Sell,
                        good: own_ref_asset,
                        amount: quote_units,
                        price_per_unit_cred: ask_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                });
            }
        }

        actions
    }

    fn build_pass_b_candidates(
        &self,
        ctx: &BotTurnContext<'_>,
        bundle: &SealedOrderBundle,
    ) -> Vec<MarketAction> {
        let obs = ctx.obs;
        let seat = ctx.seat;
        let seat_cfg = ctx.seat_cfg;
        let seat_state = &obs.seats[ctx.seat_idx];
        let upkeep_g = seat_cfg.upkeep_good;
        let upkeep_good = AssetId::Refined(upkeep_g);

        // Check post-Step-0 simulated shortfall before spending 2.25x on `TownImport`:
        // only use `TownImport` if a shortfall still remains after crossing offers AND multi-turn
        // debt interest penalty (`rate_bps * remaining_horizon`) outweighs the TownImport premium.
        let mut sim_shortfall = obs.baseline_predicates.upkeep.shortfall_units;
        if let Some(preview) = Self::run_preview(obs, &[], bundle) {
            if preview.upkeep.new_shortfall_debt <= 0 {
                sim_shortfall = 0;
            } else {
                sim_shortfall = obs
                    .baseline_predicates
                    .upkeep
                    .owed_units
                    .saturating_sub(preview.upkeep.paid_with_refined);
            }
        }
        if sim_shortfall <= 0 {
            return Vec::new();
        }

        let rem_turns = i128::from(
            obs.baseline_predicates
                .season_end
                .turns_until_latest_close
                .min(10),
        );
        let rate_bps = i128::from(seat_state.credit_tier.interest_bps());
        let premium_over_parity_bps =
            i128::from(obs.spec.town_import_premium_bps).saturating_sub(BPS_SCALE);
        let debt_penalty_exceeds_import =
            rate_bps.saturating_mul(rem_turns) > premium_over_parity_bps;
        if !debt_penalty_exceeds_import {
            return Vec::new();
        }

        let parity = obs.parities.asset_parity(upkeep_good);
        let Ok(full_cost) =
            town_import_cost_cred(sim_shortfall, parity, obs.spec.town_import_premium_bps)
        else {
            return Vec::new();
        };
        let available_cred = seat_state.cred.saturating_sub(bundle.debt_repayment_cred);
        if available_cred >= full_cost && full_cost > 0 {
            vec![MarketAction::TownImport {
                seat,
                good: upkeep_good,
                amount: sim_shortfall,
            }]
        } else {
            Vec::new()
        }
    }

    fn build_bundle_candidates(
        &self,
        ctx: &BotTurnContext<'_>,
        own_mandate: &PrivateMandate,
    ) -> SealedOrderBundle {
        let obs = ctx.obs;
        let seat = ctx.seat;
        let seat_idx = ctx.seat_idx;
        let seat_cfg = ctx.seat_cfg;
        let seat_state = &obs.seats[seat_idx];
        let mut bundle = SealedOrderBundle::default();

        // 1. Cash all open checks due to `seat` whose drawer currently holds sufficient balance so
        // healthy settlements collect without triggering needless hard-credit bounces.
        for (&check_id, check) in &obs.checks {
            if check.payee == seat
                && check.status == CheckStatus::Open
                && obs.turn >= check.due_turn
                && obs.turn < check.expiration_turn
            {
                let drawer_free = Self::seat_asset_balance(obs, check.drawer, check.asset);
                if drawer_free >= check.deliver_min {
                    bundle.check_cashes.push(check_id);
                }
            }
        }

        // 2. Repay `CRED` auto-debt when `free_cred` exceeds `1.5x` next-turn upkeep parity reserve.
        let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
        let upkeep_parity = obs.parities.asset_parity(upkeep_good).max(1);
        let upkeep_cred_reserve =
            (mul_units_parity(seat_cfg.upkeep_amount, upkeep_parity).saturating_mul(3) / 2)
                .max(1_000);
        if seat_state.cred_debt > 0 {
            let excess_cred = seat_state.cred.saturating_sub(upkeep_cred_reserve);
            let repay = excess_cred.min(seat_state.cred_debt);
            if repay > 0 {
                bundle.debt_repayment_cred = repay;
            }
        }

        // 3. Schedule Tier-2 converter runs (`SpoilageAndConversion`) in descending margin order,
        // tracking shared raw inventories across multiple recipes so runs never exceed holdings.
        let mut avail_raw = seat_state.raw.clone();
        let mut produced_refined = vec![0_i64; obs.spec.seats.len()];
        for &(recipe_idx, margin) in &Self::ranked_converter_recipes(ctx) {
            if margin < 0 {
                continue;
            }
            let recipe = &seat_cfg.converters[recipe_idx];
            let mut max_possible = recipe.max_runs_per_turn;
            for &(in_g, per_run) in &recipe.inputs {
                if per_run <= 0 {
                    max_possible = 0;
                    break;
                }
                let held = avail_raw.get(usize::from(in_g)).copied().unwrap_or(0);
                let by_input = u32::try_from((held / per_run).max(0)).unwrap_or(u32::MAX);
                max_possible = max_possible.min(by_input);
            }
            if max_possible > 0 {
                for &(in_g, per_run) in &recipe.inputs {
                    if let Some(slot) = avail_raw.get_mut(usize::from(in_g)) {
                        *slot =
                            slot.saturating_sub(per_run.saturating_mul(i64::from(max_possible)));
                    }
                }
                if let Some(slot) = produced_refined.get_mut(usize::from(recipe.output_good)) {
                    *slot = slot.saturating_add(
                        recipe.output_amount.saturating_mul(i64::from(max_possible)),
                    );
                }
                bundle.converter_runs.push(ConverterRunOrder {
                    recipe_idx,
                    runs: max_possible,
                });
            }
        }

        // 4. Schedule Tier-3 Syndicate Project deliveries, strictly sparing `owed_units` of
        // `upkeep_good` (`held_enough_but_short_turns == []`!) and `MinRefinedVariety` buffers near close.
        let mut spendable_refined: Vec<Units> = seat_state
            .refined
            .iter()
            .zip(produced_refined.iter())
            .map(|(&free, &prod)| free.saturating_add(prod))
            .collect();
        let upkeep_idx = usize::from(seat_cfg.upkeep_good);
        if let Some(upkeep_slot) = spendable_refined.get_mut(upkeep_idx) {
            *upkeep_slot = upkeep_slot.saturating_sub(seat_cfg.upkeep_amount);
        }
        if let MandateKind::MinRefinedVariety {
            distinct_goods,
            min_each,
        } = own_mandate.kind
            && obs.baseline_predicates.season_end.turns_until_latest_close <= 5
        {
            let mut protected = usize::from(distinct_goods);
            for (g_idx, slot) in spendable_refined.iter_mut().enumerate() {
                if g_idx == upkeep_idx {
                    continue;
                }
                if protected > 0 && *slot >= min_each {
                    *slot = slot.saturating_sub(min_each);
                    protected -= 1;
                }
            }
        }

        let mut ranked_projects: Vec<(usize, Units)> = obs
            .spec
            .projects
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let weight = match own_mandate.kind {
                    MandateKind::MinProjectEquity { .. } => p.equity_points.saturating_mul(2),
                    _ => p.equity_points,
                };
                (idx, weight)
            })
            .collect();
        ranked_projects.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        for (proj_idx, _) in ranked_projects {
            let proj = &obs.spec.projects[proj_idx];
            let mut batches = u32::MAX;
            for &(good_idx, per_batch) in &proj.inputs {
                if per_batch <= 0 {
                    batches = 0;
                    break;
                }
                let held = spendable_refined
                    .get(usize::from(good_idx))
                    .copied()
                    .unwrap_or(0);
                let cover = u32::try_from((held / per_batch).max(0)).unwrap_or(u32::MAX);
                batches = batches.min(cover);
            }
            if batches > 0 && batches < u32::MAX {
                for &(good_idx, per_batch) in &proj.inputs {
                    if let Some(slot) = spendable_refined.get_mut(usize::from(good_idx)) {
                        *slot = slot.saturating_sub(per_batch.saturating_mul(i64::from(batches)));
                    }
                }
                bundle.project_deliveries.push(ProjectDeliveryOrder {
                    project_id: proj.id,
                    batches,
                });
            }
        }

        // 5. Cast deterministic Turn-`4n` council ballots and modest Turn-`6n` charter bids.
        let n_u8 = obs.spec.seat_count;
        if obs.turn.is_multiple_of(4) && n_u8 >= 2 {
            let tm = seat;
            let alt =
                (seat + 1 + ((self.bot_tie_seed ^ u64::from(obs.turn)) as u8 % (n_u8 - 1))) % n_u8;
            bundle.council_vote = Some(CouncilVoteBallot {
                trade_minister: tm,
                cartel_chair: alt,
            });
        }
        if obs.turn.is_multiple_of(6) {
            let free_after_repay = seat_state.cred.saturating_sub(bundle.debt_repayment_cred);
            if free_after_repay > upkeep_cred_reserve.saturating_mul(2) {
                let roles = [
                    CharterRole::ClearinghouseBroker,
                    CharterRole::OracleAuditor,
                    CharterRole::CustomsInspector,
                ];
                let pick =
                    roles[usize::from(seat).wrapping_add(usize::from(obs.turn / 6)) % roles.len()];
                bundle.charter_bids.push(CharterBidOrder {
                    role: pick,
                    bid_cred: 150,
                });
            }
        }

        bundle
    }

    fn ranked_converter_recipes(ctx: &BotTurnContext<'_>) -> Vec<(usize, Units)> {
        let obs = ctx.obs;
        let mut ranked: Vec<(usize, Units)> = ctx
            .seat_cfg
            .converters
            .iter()
            .enumerate()
            .map(|(idx, recipe)| {
                let out_asset = AssetId::Refined(recipe.output_good);
                let out_val =
                    mul_units_parity(recipe.output_amount, obs.parities.asset_parity(out_asset));
                let in_val: Units = recipe
                    .inputs
                    .iter()
                    .map(|&(g, units)| {
                        let raw_asset = AssetId::Raw(g);
                        let tariff_bps = obs
                            .baseline_predicates
                            .scarcity
                            .iter()
                            .find(|s| s.good == raw_asset)
                            .map_or(0, |s| s.open_book_tariff_bps);
                        let base = mul_units_parity(units, obs.parities.asset_parity(raw_asset));
                        let tariff_cost =
                            ((i128::from(base) * i128::from(tariff_bps)) / BPS_SCALE) as Units;
                        base.saturating_add(tariff_cost)
                    })
                    .sum();
                (idx, out_val.saturating_sub(in_val))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked
    }

    fn own_raw_used_by_local_recipes(seat_cfg: &crate::types::SeatConfig, seat: u8) -> Units {
        seat_cfg
            .converters
            .iter()
            .filter_map(|r| {
                r.inputs
                    .iter()
                    .find(|&&(g, _)| g == seat)
                    .map(|&(_, amt)| amt.saturating_mul(i64::from(r.max_runs_per_turn)))
            })
            .sum()
    }

    fn own_open_bid_units(obs: &SeatObservation, seat: u8, good: AssetId) -> Units {
        obs.offers
            .values()
            .filter(|o| o.maker == seat && o.side == OfferSide::Buy && o.good == good)
            .map(|o| o.remaining_units)
            .sum()
    }

    fn affordable_bid_units(
        desired: Units,
        cred_budget: Units,
        price_per_unit_cred: Units,
    ) -> Option<Units> {
        if desired <= 0 || cred_budget <= 0 || price_per_unit_cred <= 0 {
            return None;
        }
        let max_affordable = ((i128::from(cred_budget) * i128::from(UNIT_SCALE))
            / i128::from(price_per_unit_cred))
        .min(i128::from(desired));
        let mut units = Units::try_from(max_affordable.max(0)).ok()?;
        while units > 0 {
            if let Ok(lock) = checked_ceil_units_parity(units, price_per_unit_cred)
                && lock <= cred_budget
            {
                return Some(units);
            }
            units = units.saturating_sub(1);
        }
        None
    }

    fn seat_asset_balance(obs: &SeatObservation, seat: u8, asset: AssetId) -> Units {
        let Some(state) = obs.seats.get(usize::from(seat)) else {
            return 0;
        };
        match asset {
            AssetId::Cred => state.cred,
            AssetId::Raw(g) => state.raw.get(usize::from(g)).copied().unwrap_or(0),
            AssetId::Refined(m) => state.refined.get(usize::from(m)).copied().unwrap_or(0),
        }
    }

    fn run_preview(
        obs: &SeatObservation,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
    ) -> Option<crate::observation::WhatIfPreview> {
        let pub_state = obs.to_public_preview_state();
        let req = AiPreviewRequest {
            seat: obs.seat,
            market_actions: market_actions.to_vec(),
            bundle: bundle.clone(),
        };
        preview_seat_bundle(&pub_state, &req).ok()
    }

    fn prune_refused_actions(
        obs: &SeatObservation,
        mut actions: Vec<MarketAction>,
        bundle: &SealedOrderBundle,
    ) -> Vec<MarketAction> {
        // Iteratively remove refused actions surfaced by `preview_seat_bundle` (`0` refused guarantee).
        for _ in 0..=actions.len() {
            if actions.is_empty() {
                break;
            }
            let Some(preview) = Self::run_preview(obs, &actions, bundle) else {
                return Vec::new();
            };
            if preview.refused_market_actions.is_empty() {
                return actions;
            }
            let refused: BTreeSet<usize> = preview
                .refused_market_actions
                .iter()
                .map(|r| r.action_index)
                .collect();
            let next: Vec<MarketAction> = actions
                .into_iter()
                .enumerate()
                .filter_map(|(idx, a)| (!refused.contains(&idx)).then_some(a))
                .collect();
            actions = next;
        }
        Vec::new()
    }
}

struct BotTurnContext<'a> {
    obs: &'a SeatObservation,
    seat: u8,
    seat_idx: usize,
    seat_cfg: &'a crate::types::SeatConfig,
}

impl<'a> BotTurnContext<'a> {
    fn new(obs: &'a SeatObservation) -> Option<Self> {
        let seat_idx = usize::from(obs.seat);
        let seat_cfg = obs.spec.seats.get(seat_idx)?;
        let _ = obs.seats.get(seat_idx)?;
        if seat_idx >= obs.seats.len() || obs.spec.seats.len() != obs.seats.len() {
            return None;
        }
        if matches!(
            obs.phase,
            TurnPhase::AwaitingMandateReveal | TurnPhase::Completed
        ) {
            return None;
        }
        Some(Self {
            obs,
            seat: obs.seat,
            seat_idx,
            seat_cfg,
        })
    }
}
