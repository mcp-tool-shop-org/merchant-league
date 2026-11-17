---
title: Fairness and randomness
description: How Merchant League checks every table before it is dealt, keeps its seed pools apart, draws randomness nobody can steer, and replays every game exactly.
sidebar:
  order: 3
---

## Checked tables

Every table is generated from a seed by `league-gen` and checked before it is dealt. A table that fails one of these checks is thrown away, and the generator tries again:

- **Starting value.** The generator solves a linear program over the table and prices every seat's starting position at its shadow prices. Every seat must sit within **3%** of the table's average.
- **What a seat adds to every group.** A second measure values each seat by what it adds to every possible group of seats (its Shapley value). Every seat must sit within **7%** of the average by that measure too. This catches a seat that holds a bottleneck for a small group, which the first measure can miss.
- **No self-sufficient pairs.** One or two seats can never cover their own upkeep.

Two more properties are proved by the generator's tests over hundreds of seeds per table size, rather than checked on each table: three seats cover at most one member's upkeep, and at a five- or six-seat table four seats cover at most two members'. The tests also report, without requiring it, that in more than 90% of tables the seats rank their best trades differently, so play does not collapse into one obvious move.

The generator's version is recorded with every table, and the tests pin a digest of a known table, so a change to the generator cannot pass unnoticed.

## Seed pools

Seeds are split into pools that never overlap, and setup refuses a table whose stated purpose doesn't match its seed's pool, so training tables and evaluation tables come from separate ranges:

| Pool | Seeds | Used for |
| --- | --- | --- |
| dev | 0 to 9,999 | development and playtests |
| train | 10,000 to 999,999 | training |
| probe | 1,000,000 to 1,099,999 | probes |
| held-out | 2,000,000 to 2,099,999 | held-out evaluation |
| sealed | 2^63 and above | sealed evaluation |

Every other seed is unassigned and refused. A table's stated purpose must match its seed's pool, or setup refuses it. Sealed seeds never appear in the clear in a replay log: the log records only the pool and a digest.

## Randomness nobody can steer

The game's randomness (shocks, the closing roll and tie-breaks) comes from a beacon that no player can predict or bend. It is built in `league-core`:

- **Game mode** takes each turn's beacon from a public [drand](https://drand.love/) round (the quicknet chain) published **after** the turn's commit deadline, so neither a seat nor the studio can steer a roll that was fixed before the round existed. Checking the round's signature during replay is still open ([#29](https://github.com/mcp-tool-shop-org/merchant-league/issues/29)).
- **Evaluation runs** (`Eval` mode) use a verifiable random function instead. Its seed is sealed by the studio's orchestrator, away from every seat, and anyone can check its output against the published key afterwards.

Orders follow the same rule: each seat commits a salted hash of its sealed orders, then reveals them. A seat that commits but does not reveal loses its secret actions for that turn, and there is never a re-roll.

`league-dice`, the crate for per-merchant dice secrets settled on the ledger, is stubs only for now.

## Exact replay

A game replays exactly from its seed and its ordered log of commitments, reveals and beacon outputs. Replays are bit-for-bit: the tests run full seasons at four, five and six seats and check that the replay and its JSON form match. Anyone who holds the log can check the scores, the tie-breaks and every turn's state.
