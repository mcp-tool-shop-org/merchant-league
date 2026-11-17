---
title: The game
description: How a Merchant League table works, from the ring economy and the three phases of a turn to politics, the season's random close and scoring.
sidebar:
  order: 2
---

This page describes the rules as designed in [`docs/design-v2.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/design-v2.md). The rules engine implements them in pure state; the game client and the on-ledger play come later. Numbers marked provisional wait on playtests.

## The table

A table has four, five or six seats. Every seat can be a person or an AI player, and they play under the same rules.

The table's coin is `CRED`, and every market prices in it. Goods and coin are tokens issued on the XRPL Testnet by the table's own steward account, and only the table's seats may hold them.

## A ring of merchants

The seats sit in a ring, and every seat depends on its neighbours.

1. **Raw goods.** Each turn, each seat receives a yield of its own raw good, which no other seat produces.
2. **Workshops.** Each seat runs workshops that turn *other seats'* raw goods into its own refined good. Workshops have capacity caps, so a seat cannot simply convert everything it buys.
3. **Projects.** Shared projects consume bundles of refined goods from several seats and award permanent project equity, plus a small `CRED` stipend.
4. **Upkeep.** At the end of every turn, each seat owes upkeep in the refined good made by its neighbour on one side. A shortfall becomes `CRED` debt at an interest rate set by the seat's credit tier.

Raw and refined goods are perishable: a share of what a seat holds spoils each turn (20% as a baseline). A seat that is short can always buy from the town at a steep premium (2.25 times the fair price, provisional), so a withholding neighbour can hurt a seat but cannot freeze it out.

The ring is built so that one or two seats can never cover their own upkeep, three seats can cover at most one member's, and, at a five- or six-seat table, four can cover at most two members'. Trade across the whole table is the only way to prosper.

## A turn, in three phases

All seats act at the same time. Nobody waits for anyone else, and nobody gets to move last.

### 1. Talk and market

Seats negotiate. Every message carries a typed header (who, what, how much, by when) and a short free-text body for persuasion. The text itself is never written to the ledger; at most a salted hash of an agreed deal is.

At the same time, seats post and cancel offers on the open market, write credit notes, and lock coin into escrow for deals. Goods are never locked in escrow, because they spoil and the steward must be able to claw spoilage back.

### 2. Commit

Before the deadline, each seat commits one sealed bundle of its secret actions: workshop runs, project deliveries, charter bids and votes. Nobody can see another seat's bundle until it is revealed. A seat that commits but fails to reveal loses its secret actions for that turn. There is never a re-roll.

### 3. Sweep

At the cutoff, the engine resolves the turn in a fixed order, the same for everyone:

1. clear the market's resting offers in one batch, at one price per good;
2. check every reveal against its commitment;
3. settle swaps, escrows, deliveries and workshop runs;
4. cash maturing credit notes. A note whose writer can no longer cover it bounces, and the writer's credit tier drops;
5. credit project deliveries and mint the new raw yields;
6. collect upkeep and price any shortfall as debt;
7. apply spoilage;
8. run the political calendar;
9. advance the supply shocks and, late in the season, roll for the close.

## Politics

- **Councils**, every 4 turns, elect a **Trade Minister**, who sets export tariffs of 0% to 35% on each good, and **Cartel Chairs**, who found and run cartels. A cartel is a private market for its members, and a chair can expel a member at once.
- **Charters**, auctioned every 6 turns by sealed bid, hand out three temporary roles. The **Clearinghouse Broker** may trade between cartel markets and the open market. The **Customs Inspector** may freeze suspicious trades. The **Oracle Auditor** learns the next supply shock early and may sell that knowledge, or bluff.

## Supply shocks

Shocks are telegraphed. The Oracle Auditor gets a private hint first; the next turn a public forecast narrows the range; the turn after, the shock lands: a supply freeze, a tariff holiday, a demand surge or a spoilage spike.

## The season and its random close

A season lasts 24 to 32 turns. From turn 24, every turn rolls for the close (35% a turn, provisional), and turn 32 always ends it. The expected length is about 26 turns. Nobody knows which turn is last, so nobody can plan a perfect final move.

## Scoring

At the close, each seat's score is the sum of three parts:

- **Project equity**, earned by delivering to projects during the season.
- **Net assets**: `CRED` and receivables, less debts and open credit notes, plus leftover goods. Leftover goods are marked at the standing bids for them, never below half their base value, and anything left over is marked at half its base value. Traded prices do not count, so a last-turn trade cannot inflate a holding.
- **A mandate bonus.** Each seat is dealt a hidden objective card at the start. It is revealed at the close and scores if it was met.

Because every unit of score counts, a seat that is behind still has reasons to trade well until the last turn, and punishing a betrayal is a choice with a price.

## What one seat may know about another

- **Public to every seat:** every seat's balances and credit tier, and every resting offer, including the offers in a cartel's market.
- **Private until the rules reveal it:** a seat's mandate card, its negotiation text, its committed but unrevealed orders, randomness not yet published, and its private notes.

Balances and offers sit on a public ledger, where anyone's software can read them, so the rules make them public rather than pretend otherwise.

## Seats that stop playing

A person may set up a stand-in before the season, limited by three settings they choose: appetite for risk, a minimum reserve and how much credit to extend. If the seat goes idle, the stand-in keeps paying upkeep and running workshops so its neighbours are not starved. This rule is still being tuned.
