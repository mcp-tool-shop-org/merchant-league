# Merchant League rules

## Goal and scoring
Your score is Total Equity at season close: Project Equity + Net Asset Equity + mandate bonus. Seats are ranked by it; exact ties are broken by a random draw.
- Project Equity: permanent points from project deliveries.
- Net Asset Equity: `cred` (including `cred` you have locked), plus goods, minus auto-debt, minus the `send_max` of your open checks (goods at base value).
- Goods at close (including units on your sell offers): trade prices do not count. Units that another seat's resting buy offer would take count at that offer's price if it exceeds half the good's base value (`parities`). All other units count at half of base value.
- Mandate: a card dealt before turn 1, shown only to you. If its condition holds at close it adds 3 to 8 points; one point is `mandate_alpha_bps` (2.5% by default) of the larger of the table's median net worth before mandates and half its median starting net worth.
- Credit tier, `bond_breaches`, `broken_promises` and `honor_penalties` are visible records, not subtracted from Total Equity.

## How to play
This is a competitive game, and you are playing to win. Your score is the only thing that counts at the end; the other guilds are your rivals, even when you trade with them. Make deals when they help you, and refuse or end them when they don't. You may bluff, keep your plans to yourself, and break a promise that isn't backed by a bond, as long as you stay within the rules; a broken bond still costs you, as the rules say. Don't give away advantages out of politeness, and don't make a deal just to be agreeable. Being trusted is worth something, so spend that trust only when it buys you more than it costs.

## Each turn
There is no talk round, and no messages are sent between seats.
- Act: you submit `market_actions` (a list) and `bundle` (sealed orders). No seat sees another's orders until they take effect.

Resolution order:
1. Every seat's `market_actions`, seat 0 first, each list in order. A failed action is skipped with an error code; the rest still apply.
2. A cancel lock starts: a later `cancel_offer` is refused with `CANCEL_LOCKED`. Some seats also submit market actions after the cancel lock; you submit yours once, before it.
3. Bundles are revealed. One `atomic_swaps` entry with an unknown asset, or one invalid `contract_deliveries` or `dual_escrows` entry, drops the whole bundle; other invalid entries are skipped.
4. Sweep: 0 offers cross; 1 bundles checked; 2 escrows, swaps, deliveries, bond deadlines, `debt_repayment_cred`, `converter_runs`; 3 `check_cashes`, wash-freeze audits; 4 `project_deliveries`, then raw yields credited; 5 interest, upkeep; 6 spoilage; 7 elections (turns divisible by 4), charter auctions (divisible by 6); 8 shocks, closing roll.

An entry that cannot be filled at sweep time (too few goods, no matching swap) does nothing.

## Goods and money
- Amounts are integers in thousandths: 1000 is one unit. Prices are milli-`cred` per 1000 units.
- `cred` is the coin. Seat i makes its own raw good and refined good. An `asset` is an object with `asset_kind` (`cred`, `raw`, `refined`) and `good_index` (null for `cred`); a `good` is the same but never `cred`.
- Each turn you receive raw units of your own raw good. Converter recipes turn foreign raw goods into your refined good. The table spec lists yields, recipes, projects and upkeep.
- Free units are those not locked in an offer, escrow or bond. Only `cred` can be escrowed.
- Goods moved between seats lose an export tariff from the units received; it is zero when both seats belong to the `domain_id` given.
- The Authority is the game itself. Balances, credit tiers and resting offers are visible to every seat. Your view may spell names its own way (for example `ClearinghouseBroker`); order keys and values are exactly as written here.

## `market_actions`
Each item has `action_kind`; only the listed keys apply.
- `post_offer`: `side` (`buy`/`sell`), `good`, `amount`, `price_per_unit_cred`, `domain_id`, `is_hybrid`. A buy locks `cred`, a sell locks the goods. At sweep step 0, per good and book, buys (highest limit first) and sells (lowest first) cross, ties by a public random draw, at one price (the midpoint of the highest crossed sell limit and lowest crossed buy limit). Excess locked `cred` is refunded; unfilled offers keep resting; a seat's own offers never cross. A `domain_id` offer needs membership in that cartel (never for D-tier seats) and trades only in that cartel book. `is_hybrid` needs the broker charter and lets an offer cross between books; a hybrid offer needs no membership.
- `cancel_offer`: `offer_id`. Unlocks what it held.
- `create_check`: `payee`, `asset`, `send_max`, `deliver_min`, `due_turn`, `expiration_turn`. Locks nothing.
- `cancel_check`: `check_id`.
- `create_escrow`: `destination`, `asset` (must be `cred`), `amount`, `finish_after_turn`, `cancel_after_turn`, `bond`. Locks `amount` now. Needs `bond`, `finish_after_turn`; `cancel_after_turn` is later than today and than `finish_after_turn`. `bond` holds `good`, `required_units`, `deadline_turn` (before `cancel_after_turn`), `mode`.
- `cancel_expired_escrow`: `escrow_id`. From `cancel_after_turn` on, the `cred` returns to the sender.
- `flag_wash_freeze`: `target_seat`; inspector charter holder only. Debits `wash_freeze_bond_cred` from free `cred`. That turn the target's offers, swaps, deliveries and dual escrows are blocked. Step 3 audits the previous turn's fills: confirmed by a same-good round trip with one peer of at least `wash_round_trip_min_units` each way, or a fill above the good's town-import price. Confirmed: you recover the bond plus up to `wash_freeze_bounty_cred` from the target's free `cred`. Otherwise the bond goes to the target.
- `stand_for_office`: `proposed_tariffs_bps` (items of `seat`, `raw_bps`, `refined_bps`), `cartel_admissions`, `cartel_revocations` (items of `domain_id`, `seat`). Files your platform; see Politics.
- `town_import`: `good`, `amount`. Buys from the Authority at once, paid in `cred`, at `town_import_premium_bps` / 10,000 times base value.

## `bundle`
All keys are optional.
- `converter_runs`: `recipe_idx` (index into your recipes), `runs`. Each run uses the recipe's raw inputs and makes its refined output. Runs executed are the least of: requested, `max_runs_per_turn` (shared by entries for one recipe), what your free raw goods cover.
- `project_deliveries`: `project_id`, `batches`. Each batch destroys the project's refined inputs from free units, adds its `equity_points` to Project Equity, and pays `works_stipend_per_unit_cred` of `cred` per 1000 units destroyed. Limited by free refined units.
- `atomic_swaps`: `swap_id`, `maker_seat`, `taker_seat`, `maker_asset`, `maker_amount`, `taker_asset`, `taker_amount`, `domain_id`. Settles only if both named seats submit an identical entry and hold the amounts free. Both legs happen or neither.
- `contract_deliveries`: `recipient_seat`, `good`, `gross_amount`, `domain_id`, `dual_escrow_id`. Sends free goods to `recipient_seat` with no payment, unless `dual_escrow_id` names a dual escrow where you are the seller.
- `dual_escrows`: `proposal_id`, `buyer_seat`, `seller_seat`, `good`, `required_units`, `payment_cred`, `buyer_bond_cred`, `seller_bond_cred`, `deadline_turn`, `domain_id`. Opens only if buyer and seller submit identical entries. Each bond is above 0, together at most `payment_cred`; `deadline_turn` is from now to 32. Both bonds lock at once.
- `debt_repayment_cred`: repays auto-debt from free `cred`, up to your debt.
- `escrow_finishes`: `escrow_id`. The sender or destination of a plain escrow releases it to the destination from `finish_after_turn` until before `cancel_after_turn`.
- `check_cashes`: list of `check_id`. Only the payee may present, from `due_turn` until before `expiration_turn`.
- `council_vote`: `trade_minister`, `cartel_chair` (seat indexes). Counts only on turns divisible by 4.
- `charter_bids`: `role` (`clearinghouse_broker`, `customs_inspector`, `oracle_auditor`), `bid_cred`. Counts only on turns divisible by 6.

## Credit, bonds, reputation
- Check cash: if the drawer's free `asset` is at least `deliver_min`, the payee receives the lesser of that balance and `send_max`, less the export tariff if it is a good. Otherwise it bounces: the payee gets nothing, `hard_credit_defaults` rises and the drawer drops one tier (AAA, BBB, B, D). D leaves all cartels. Tiers never rise.
- Delivery counts are cumulative net deliveries of `good` (after tariff) since creation, by any route. Bond `mode` `seller_performance_bond` (sender is seller): by `deadline_turn`, if deliveries from sender to destination reach `required_units` the `cred` returns to the sender; else it goes to the destination and `bond_breaches` rises. `buyer_dvp_payment` (sender is buyer): if deliveries from destination to sender reach `required_units`, the `cred` goes to the destination; else it returns to the sender and `bond_breaches` rises.
- Dual escrow: if deliveries from seller to buyer reach `required_units` by `deadline_turn`, the buyer pays `payment_cred` in total (bond plus top-up) and the seller recovers its bond. A seller that misses forfeits its bond to the buyer. A buyer unable to pay the top-up forfeits its bond to the seller, who keeps the goods. Either forfeit adds to `bond_breaches`.

## Upkeep, debt, spoilage
- Each sweep you owe `upkeep_amount` of refined good `upkeep_good` from free units. A shortfall becomes `cred` debt at that good's base value.
- Interest on debt owed before that sweep's shortfall: AAA 4%, BBB 10%, B 18%, D 30% per turn.
- Raw goods lose a fraction each sweep, rounded down, at each raw good's `perishable_decay_bps`; units on resting sell offers too. Refined goods and `cred` do not spoil.

## Politics
- Council (turns divisible by 4): each `council_vote` is one vote for Trade Minister and one for Cartel Chair. Most votes wins, ties by random draw, no votes keeps the incumbent. A seat holds one office: the Trade Minister is excluded from the chair count, and the chair is vacant if the incumbent chair became Trade Minister and no other seat has a chair vote.
- A winner enacts its filed platform. Trade Minister: sets each listed seat's raw and refined export tariffs (at most 3500 bps). Cartel Chair: admits seats to cartel domains (a new `domain_id` creates one; D-tier refused) and revokes seats, cancelling their offers there. Platforms clear after each election. Cartel members trade with each other at zero export tariff.
- Charters (turns divisible by 6): per role, the highest `bid_cred` the bidder can pay wins, ties to the lower seat; the winner pays the bid, which is destroyed. A charter lasts 6 turns. Broker: `is_hybrid`. Inspector: `flag_wash_freeze`. Auditor: sees private shock hints.

## Horizon shocks
A shock has a target seat, kind and size. The auditor sees a private hint first. A public forecast then gives target, kind and turn, without size; size shows once active. `SupplyFreeze` cuts the target's raw yield; `TariffHoliday` cuts export tariffs on its goods; `DemandSurge` raises points for projects using its refined good (2 turns); `SpoilageSpike` raises decay on its raw good. Others last 1 turn.

## Season length
No season ends before turn 24; none runs past turn 32. After each sweep on turns 24 to 31, a closing roll ends the season with the probability in `closing_hazard_bps` (35% per turn by default). It is not shown in advance.
