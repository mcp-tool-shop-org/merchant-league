---
title: Security
description: The Testnet-only guarantee, how Merchant League keeps keys and secrets, the stand-alone harness's limits, and how to report a vulnerability.
sidebar:
  order: 5
---

Merchant League plays only on the XRPL **Testnet**. Test XRP and the game's tokens have no value, and there is no real money anywhere in the game. That still leaves things worth protecting: players' keys, committed orders, the fairness of the game's randomness, and the Testnet-only guarantee itself.

## The Testnet-only guarantee

- `league-ledger` names only Testnet endpoints, and every connection must report the Testnet's network id (1) before it is used.
- A CI tripwire fails on any Mainnet or Devnet hostname written in the repository outside vendored code. It can't see a host assembled at build time, so review is the rest of the check.
- Any path that could reach Mainnet is treated as a security bug.

## Keys and secrets

- **Players' Testnet keys** (Game mode, not built yet) stay on the player's machine, in the git-ignored `.league/` folder, and are never logged or printed.
- **Committed orders and secrets** stay hidden until they are revealed, and secret types redact themselves in debug output. The engine checks commitments in pure state; the reveal on the ledger is not built yet.
- **The relay's OpenRouter key** is read only from the `OPENROUTER_API_KEY` environment variable. The relay refuses key-like flags (`--key`, `--api-key`, `--token` and similar), arguments that look like keys, and keys in the seat file, and its errors and panics pass through redaction. It does not scan the `--wrapper` or `--response-schema` files, so keep keys out of them.
- The relay makes no OpenRouter call without `--live`, and `--live` requires a credit cap, checked against the call log before every call.
- The relay's local seats refuse any API key, reach only a loopback address or a private-network address allowed with `--allow-vm-host`, and need a GPU-seconds cap (`--cap-gpu-secs`).

## The stand-alone harness

The training-mode harness runs one pinned `xrpld` image in Docker, inside fixed limits:

- the exact image tag and manifest digest, and nothing else;
- at most one container per machine;
- 2 CPUs, 2 GB of memory and no swap;
- a scratch disk in memory (512 MB by default, and never more than about 5 GB), with no host folders mounted;
- its port bound to `127.0.0.1` only;
- a self-expiry after at most 10 minutes, and removal after every run, on every failure path the harness handles; a run that is killed outright can leave a container behind, and the next run sweeps it away before it starts.

Before it talks to the node, the harness checks that the node reports the stand-alone network id, and refuses the Testnet's and Mainnet's.

## No telemetry

Nothing is sent anywhere except the calls a mode needs: the Testnet, the public drand beacon and, for the relay, OpenRouter or the local model servers you allow.

## Threats and defences

| Threat | Defence |
| --- | --- |
| The client is pointed at Mainnet | The game's ledger client names only Testnet endpoints, and every connection must report network id 1; the stand-alone harness accepts only its private node (network id 21337, on loopback); a CI tripwire fails on Mainnet hostnames written in the repository |
| A player biases a roll by not revealing | A seat that commits but does not reveal loses its secret actions for the turn; there is never a re-roll |
| The last player to reveal sees the outcome first | No seat's reveal feeds the randomness. Game mode (not built yet) is designed to take each turn's beacon from a public drand round; checking the round's signature is open (#29). Evaluation runs use a sealed verifiable random function |
| A player makes many free accounts | Planned for Game mode, not built yet: ranked queues need verified matchmaking credentials, hide handles behind seasonal aliases, and flag pairs of seats that funnel wealth to each other |
| A seed or secret leaks into logs | Secret types redact themselves in debug output; in Game mode (not built yet), seeds will stay in `.league/` |

## Reporting a vulnerability

Report it privately through GitHub's security advisories for the repository ("Report a vulnerability" on the [Security tab](https://github.com/mcp-tool-shop-org/merchant-league/security)), not in a public issue. Include what you found, how to reproduce it, and what it could let someone do. We aim to acknowledge reports within a week.
