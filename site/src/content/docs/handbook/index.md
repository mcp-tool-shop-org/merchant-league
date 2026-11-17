---
title: Merchant League Handbook
description: The guide to Merchant League, a slow-turn trading and politics game on the XRPL Testnet for four to six seats, people or AI players.
sidebar:
  order: 0
---

Merchant League is a slow-turn trading and politics game for four to six seats, played on the XRP Ledger **Testnet**. Each seat makes what its neighbours need, so every deal matters and nobody wins alone. People and AI players sit at the same table, under the same rules.

:::caution[Prototype, design stage]
The rules engine, the table generator and the replay are built and tested. The game itself is **not playable yet**: there is no game client and no release. Testnet only, with no real money and no path to Mainnet.
:::

## What is in this handbook

| Page | What it covers |
| --- | --- |
| [Getting started](./getting-started/) | Building the workspace, running the tests and the checks CI runs |
| [The game](./the-game/) | The ring economy, the three phases of a turn, politics, the season and scoring |
| [Fairness and randomness](./fairness/) | How tables are checked before they are dealt, seed pools, randomness and exact replay |
| [Architecture](./architecture/) | The crates, the two modes and how a turn moves through the code |
| [Security](./security/) | The Testnet-only guarantee, keys and secrets, the harness's limits, and reporting |
| [Reference](./reference/) | The design documents, the playtest records and the relay's command line |

## Two ways to play

Both modes share one rules engine, and a game replays exactly from its seed and its log.

- **Game mode** runs on the public XRPL Testnet, at about two turns a day, for people alone or with AI seats.
- **Training and evaluation mode** runs on a private stand-alone `xrpld` node in a local Docker container, closing ledgers in milliseconds, for AI training, benchmarks and replays.

## Where the project is going

The next large piece is the game client, on top of the rules engine that exists today. The first tag, v0.1.0, waits until a table can actually be played. Progress is recorded in the [CHANGELOG](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/CHANGELOG.md).
