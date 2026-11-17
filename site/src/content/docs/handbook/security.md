---
title: Security
description: The Testnet-only guarantee, how Merchant League keeps keys and secrets, the stand-alone harness's limits, and how to report a vulnerability.
sidebar:
  order: 5
---

Merchant League plays only on the XRPL **Testnet**. Test XRP and the game's tokens have no value, and there is no real money anywhere in the game. That still leaves things worth protecting: players' keys, the dice's secrets, the fairness of a roll, and the Testnet-only guarantee itself.

## The Testnet-only guarantee

- `league-ledger` names only Testnet endpoints, and every connection must report the Testnet's network id (1) before it is used.
- CI fails on any Mainnet or Devnet hostname anywhere in the code or the docs.
- Any path that could reach Mainnet is treated as a security bug.

## Keys and secrets

- **Players' Testnet keys** (Game mode, not built yet) stay on the player's machine, in the git-ignored `.league/` folder, and are never logged or printed.
- **Dice secrets** stay hidden until they are revealed, and secret types redact themselves in debug output.
- **The relay's OpenRouter key** is read only from the `OPENROUTER_API_KEY` environment variable. Passing it as an argument or in a file is refused, and errors and panics pass through redaction, so the key cannot leak through a message.
- The relay makes no network call without `--live`, and `--live` requires a credit cap, checked against the call log before every call.

## The stand-alone harness

The training-mode harness runs one pinned `xrpld` image in Docker, inside fixed limits:

- the exact image tag and manifest digest, and nothing else;
- at most one container per machine;
- 2 CPUs, 2 GB of memory and no swap;
- a 512 MB scratch disk in memory, with no host folders mounted;
- its port bound to `127.0.0.1` only;
- a self-expiry after at most 10 minutes, and removal after every run, including every failure path.

Before it talks to the node, the harness checks that the node reports the stand-alone network id, and refuses the Testnet's and Mainnet's.

## No telemetry

Nothing is sent anywhere except the calls a mode needs: the Testnet, the public drand beacon and, for the relay, OpenRouter.

## Threats and defences

| Threat | Defence |
| --- | --- |
| The client is pointed at Mainnet | Only Testnet endpoints exist in the code; every connection must report network id 1; CI fails on Mainnet hostnames |
| A player biases the dice by not revealing | A missing reveal loses the day; there is never a re-roll |
| The last player to reveal sees the outcome first | The roll also mixes in a drand round published after the reveal deadline |
| A player makes many free accounts | Earnings are capped per account and per day; ranked play needs an invitation |
| A seed or secret leaks into logs | Secret types redact themselves in debug output; seeds stay in `.league/` |

## Reporting a vulnerability

Report it privately through GitHub's security advisories for the repository ("Report a vulnerability" on the [Security tab](https://github.com/mcp-tool-shop-org/merchant-league/security)), not in a public issue. Include what you found, how to reproduce it, and what it could let someone do. We aim to acknowledge reports within a week.
