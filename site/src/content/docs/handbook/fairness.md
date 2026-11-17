---
title: Fairness and randomness
description: How Merchant League checks every table before it is dealt, keeps its seed pools apart, rolls dice nobody can steer, and replays every game exactly.
sidebar:
  order: 3
---

## Checked tables

Every table is generated from a seed by `league-gen` and checked before it is dealt. A table that fails a check is thrown away, and the generator tries again.

- **Starting value.** The generator solves a linear program over the table and prices every seat's starting position at its shadow prices. Every seat must sit within **3%** of the others.
- **What a seat adds to every group.** A second measure values each seat by what it adds to every possible group of seats (its Shapley value). Every seat must sit within **7%** of the average by that measure too. This catches a seat that holds a bottleneck for a small group, which the first measure can miss.
- **Group limits.** One or two seats can never cover their own upkeep. Three seats can cover at most one member's, and four can cover at most two members'.
- **Real choices.** In more than 90% of tables, seats rank their best trades differently, so play does not collapse into one obvious move.

The generator's version is recorded with every table, and the tests pin a digest of a known table, so a change to the generator cannot pass unnoticed.

## Seed pools

Seeds are split into pools that never overlap, so a table used for training can never turn up in an evaluation:

| Pool | Seeds | Used for |
| --- | --- | --- |
| dev | 0 to 9,999 | development and playtests |
| train | 10,000 to 999,999 | training |
| probe | 1,000,000 to 1,099,999 | probes |
| held-out | 2,000,000 to 2,099,999 | held-out evaluation |
| sealed | 2^63 and above | sealed evaluation |

Every other seed is unassigned and refused. A table's stated purpose must match its seed's pool, or setup refuses it. Sealed seeds never appear in the clear in a replay log: the log records only the pool and a digest.

## Dice nobody can steer

`league-dice` makes rolls that no single player, the studio or the randomness beacon's operators can predict or bend:

1. Each seat commits to a secret before the reveal deadline.
2. The secrets are revealed and mixed with a public [drand](https://drand.love/) beacon round published **after** that deadline.
3. A seat that does not reveal loses the day. There is never a re-roll, so refusing to reveal cannot buy a better roll.

The last seat to reveal cannot see the outcome first, because the beacon round does not exist yet when it commits.

## Exact replay

A game replays exactly from its seed and its ordered log of commitments, reveals and beacon outputs. Replays are bit-for-bit: the tests run full seasons at four, five and six seats and check that the replay and its JSON form match. Anyone who holds the log can check the scores, the tie-breaks and every turn's state.
