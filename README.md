<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

# Merchant League

A slow-turn trading and politics game for four to six seats, played on the XRP Ledger **Testnet**. Seats can be people or AI players, at the same table and under the same rules.

**Status: prototype, design stage.** The rules engine, the table generator and the replay are built and tested. The game itself is **not playable yet**: there is no game client and no release. Testnet only: no real money, and no path to Mainnet.

## The game

- **A ring of merchants.** Each seat produces its own raw good and needs goods from the other seats to pay its upkeep, run its workshops and deliver to shared projects. Nobody can prosper alone, and no small group can either.
- **Simultaneous turns.** Every turn has three phases. Seats talk and trade in the open market; then each seat commits its orders in secret; then every seat's orders resolve at once, by the same rules for everyone.
- **Politics.** Councils set tariffs, cartels can open private markets, and charters rotate between seats through the season.
- **A season of 24 to 32 turns.** Shocks to supply are announced two turns ahead, and the last turn is rolled, so nobody can plan a perfect final move.
- **Scoring.** A seat scores its net worth at the close, plus a hidden objective card dealt at the start. Every seat keeps a reason to play until the last turn.
- **Checked tables.** Every table is generated from a seed and checked before it is dealt. Each seat's starting value must sit within 3% of the others', and within 7% when a seat is valued by what it adds to every group of seats.

The full design is in [`docs/design-v2.md`](docs/design-v2.md).

## Two ways to play

| Mode | Where it runs | Who it's for |
| --- | --- | --- |
| **Game mode** | the public XRPL Testnet, at about two turns a day | people, alone or with AI seats |
| **Training and evaluation mode** | a private stand-alone `xrpld` node in a local Docker container, closing ledgers in milliseconds | AI seats: training, benchmarks and replays |

Both modes share one rules engine. A game replays exactly from its seed and its log.

## What is built

| Crate | What it does |
| --- | --- |
| `league-core` | The rules engine: the turn phases, contracts, scoring, the season close, replay, and the state predicates behind the planned hint system |
| `league-gen` | The table generator, with its fairness checks and its seed pools |
| `league-dice` | Dice that no single player can predict or steer: commit and reveal, mixed with a public randomness beacon |
| `league-ledger` | The XRPL Testnet client. It refuses any network that is not the Testnet |
| `league-standalone` | The stand-alone `xrpld` harness for training mode, in a pinned, resource-capped container |
| `league-season` | Season boundaries, which follow the Testnet's resets |
| `league-relay` | The tool that ran the paper playtests with AI seats |
| `league-tui` | The terminal client. A title screen only, for now |

## Build from source

You need [Rust](https://rustup.rs/). The pinned toolchain (1.100.0) installs itself from `rust-toolchain.toml` on the first build.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

CI builds and tests on Linux, and the workspace is developed on Windows. Other platforms are untested.

Docker is needed only for the stand-alone `xrpld` harness, and its tests are skipped unless you ask for them:

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`scripts/verify.sh` runs the checks CI runs, except the Docker harness and the Atlas map check.

## Security and data

- **What it touches.** In Game mode (not built yet), a player's Testnet keys stay on their own machine, in the git-ignored `.league/` folder, and are never logged. The paper-test relay reads its OpenRouter key only from the `OPENROUTER_API_KEY` environment variable, makes no network call without `--live` and a credit cap, and keeps its transcripts in the game folder you give it.
- **What it does not touch.** No real money, no Mainnet, and nothing outside the folders above. There is **no telemetry**: nothing is sent anywhere except the calls a mode needs (the Testnet, a public randomness beacon and, for the relay, OpenRouter).
- **Permissions.** Network access to the Testnet in Game mode. Docker, with its port bound to `127.0.0.1` only, for the stand-alone harness.
- **Guards.** Every Testnet connection must report the Testnet's network id, CI fails on any Mainnet hostname, and a second tripwire fails on any copy that frames the game as a source of income.

See [`SECURITY.md`](SECURITY.md) for the threat model and how to report a vulnerability.

## Documentation

- [Handbook](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Design v2](docs/design-v2.md): the rules, the economy and the two modes
- [Paper test](docs/paper-test.md): the first playtests, played on paper with AI seats
- [Stand-alone `xrpld` harness](docs/standalone-xrpld.md)
- [Receipts](docs/receipts/): live Testnet checks. The Testnet is wiped from time to time (next on 2026-11-25), so their transaction hashes stop resolving afterwards; what they recorded, and the test vectors, stay the evidence.

## Support

This is a prototype under active design. Issues and ideas are welcome on the [issue tracker](https://github.com/mcp-tool-shop-org/merchant-league/issues). There are no releases yet and no support guarantees. See the [CHANGELOG](CHANGELOG.md) for progress.

## License

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
