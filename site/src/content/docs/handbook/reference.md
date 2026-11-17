---
title: Reference
description: The Merchant League design documents, the paper-test records, the receipts, and the paper-test relay's command line.
sidebar:
  order: 6
---

## Design documents

| Document | What it holds |
| --- | --- |
| [`docs/design-v2.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/design-v2.md) | The current design: the economy, the turn engine, contracts and reputation, scoring, politics, shocks and the season, the two modes, and the seed pools |
| [`docs/design.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/design.md) | The first design, kept for its history. `design-v2.md` supersedes it |
| [`docs/standalone-xrpld.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/standalone-xrpld.md) | The stand-alone `xrpld` harness: the pinned image, its limits and its isolation checks |
| [`docs/paper-test.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/paper-test.md) | How the paper playtests were run |
| [`docs/paper-test/`](https://github.com/mcp-tool-shop-org/merchant-league/tree/main/docs/paper-test) | The playtests' setups, logs, scores and results |
| [`docs/receipts/`](https://github.com/mcp-tool-shop-org/merchant-league/tree/main/docs/receipts) | Live checks taken on the Testnet |

The Testnet is wiped from time to time (next on 2026-11-25), which deletes every account and transaction on it. After a wipe, the receipts' transaction hashes stop resolving. What they recorded, and the test vectors in `crates/league-ledger/tests/vectors/`, stay the evidence.

## `league-relay`

The relay ran the paper playtests: it sends one AI seat its turn, records the reply and keeps a transcript per seat.

```text
league-relay turn --game-dir <dir> --day <DD> --round <talk|act> --seats <file> [options]
league-relay prebook-check --seats <file> --wrapper <path> [--response-schema <path>] [options]
```

- **`turn`** runs one round of one day for every seat in the seats file.
- **`prebook-check`** runs a whole turn offline, against canned responses in a fresh temporary folder, so a setup can be checked before any paid call.

| Option | Meaning |
| --- | --- |
| `--wrapper <path>` | The system wrapper sent as the system message; required on a seat's first turn |
| `--memory-mode <mode>` | What a seat remembers: `snapshot` (the default), `emc` or `full` |
| `--response-schema <path>` | A JSON Schema the reply must follow |
| `--scratchpad` / `--no-scratchpad` | Keep, or drop, a seat's persistent notes |
| `--mixed-scratchpad-arm` | Allow a deliberate mix of seats with and without notes |
| `--live` | Allow live OpenRouter calls; needs `OPENROUTER_API_KEY` and `--cap-credits` |
| `--cap-credits <n>` | The total credit cap, checked against the call log before each call |
| `--cap-gpu-secs <n>` | The total GPU-seconds cap for local, OpenAI-compatible seats |
| `--allow-vm-host <host>` | One more local server address a local seat may use |
| `--mock-dir <dir>` | Run against canned responses in `<dir>/responses.json` |
| `-h`, `--help` | Print the help |

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Done; every seat's turn was clean |
| 1 | An error: bad arguments, a bad seats file, a refused key placement, input or output |
| 2 | Done, but at least one seat's turn was not clean |
| 3 | A call was made but could not be recorded; its reservation is kept for the next run |
| 4 | Another `turn` holds the game folder's lock, or the lock was changed mid-turn |

## Project files

- [CHANGELOG](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/CHANGELOG.md)
- [SECURITY.md](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/SECURITY.md)
- [LICENSE](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/LICENSE) (MIT)
