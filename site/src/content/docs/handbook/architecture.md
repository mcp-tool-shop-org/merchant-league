---
title: Architecture
description: The Merchant League Rust workspace, its crates, its two modes and how a turn moves through the code.
sidebar:
  order: 4
---

Merchant League is a Rust workspace (edition 2024, toolchain 1.100.0). Unsafe code is forbidden across the workspace, and any Clippy warning fails CI.

## The crates

| Crate | Role |
| --- | --- |
| `league-core` | The rules engine. Pure state, with no input or output: the turn phases, contracts, scoring, the season close, replay and the state predicates |
| `league-gen` | The table generator: the linear program, the fairness checks, the mandate deck and the seed pools |
| `league-dice` | Commit-and-reveal dice mixed with a public beacon |
| `league-ledger` | The XRPL Testnet client, built on `xrpl-rust`. It refuses any network that is not the Testnet |
| `league-standalone` | The stand-alone `xrpld` harness: a pinned container, its limits and a conformance suite |
| `league-season` | Season boundaries, from the Testnet's announced and observed resets |
| `league-relay` | The paper-test relay: one AI seat's turn at a time, with transcripts and a credit cap |
| `league-tui` | The terminal client, built on ratatui. A title screen for now |

`vendor/xrpl-rust` holds `xrpl-rust` 1.3.0 with upstream's own fix for a security advisory that no release carries yet. It is patched in through `[patch.crates-io]` and is not a workspace member.

## Two modes, one engine

```text
               league-gen  -->  a checked table
                                     |
                                     v
 seats' orders -->  league-core (pure state, deterministic)  -->  replay log
                         |                       |
            Game mode    v                       v   Training / evaluation mode
   league-ledger: the public Testnet      league-standalone: a private xrpld
   (network id 1 only)                     in Docker (loopback only)
```

The rules live once, in `league-core`. A mode changes only where the ledger side of a turn happens: on the public Testnet for people, or on a private stand-alone node that closes ledgers in milliseconds for AI training and evaluation.

## A turn through the code

1. **Talk and market.** Seats' messages and offers arrive. An offer locks its worst-case cost at once, but nothing crosses until the sweep.
2. **Commit.** Each seat's sealed bundle is stored as a commitment, a salted SHA-256 digest.
3. **Sweep.** `league-core` runs the fixed sequence: batch-clear the market, check reveals, settle, cash credit notes, mint yields, collect upkeep, apply spoilage, run politics, advance shocks and roll for the close.
4. **Record.** The turn's inputs and the beacon output join the replay log. Replaying the log reproduces every state exactly.

## The state predicates

`league-core::predicates` computes facts about one seat's position from public state and that seat's own holdings: upkeep and debt, spoilage and conversion, scarcity and tariffs, what groups of seats could cover, and how the close would mark its holdings. They never read another seat's private information. Today they feed the paper tests' metrics; later they feed the planned hint system.

## Quality gates

- `cargo fmt`, Clippy with warnings as errors, and the full test suite
- a 90% line-coverage floor
- `cargo audit`: advisories are fixed by upgrading, never ignored
- two tripwires, one for Testnet only and one against income language, each with its own planted controls
- an Atlas map of the repository (`@dogfood-lab/atlas`), checked in CI
