<p align="center">
  <a href="README.md">English</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh.md">中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

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

**Status: prototype, under active design.** Offline **Club Mode** in `league-tui` is playable from source on one shared terminal (4–6 human and `ClubBot` seats from Lobby through end-of-season scoreboard and replay export; private whispers are deferred past `v0.1.0`). Public-Testnet Game mode and the container image are not built yet, and there is **no release yet**. Testnet only: no real money, and no path to Mainnet.

## The game

- **A ring of merchants.** Each seat produces its own raw good and needs goods from the other seats to pay its upkeep, run its workshops and deliver to shared projects. Nobody can prosper alone, and no small group can either.
- **Simultaneous turns.** Every turn has three phases. Seats talk and trade in the open market; then each seat commits its orders in secret; then every seat's orders resolve at once, by the same rules for everyone.
- **Politics.** Councils set tariffs, cartels can open private markets, and charters rotate between seats through the season.
- **A season of 24 to 32 turns.** Shocks (a supply freeze, a tariff holiday, a demand surge or a spoilage spike) are forecast publicly a turn before they land, and the last turn is rolled, so nobody can plan a perfect final move.
- **Scoring.** A seat scores its net worth at the close, plus a hidden objective card dealt at the start. Every seat keeps a reason to play until the last turn.
- **Checked tables.** Every table is generated from a seed and checked before it is dealt. Each seat's starting value must sit within 3% of the table's average, and within 7% of the average when a seat is valued by what it adds to every group of seats.

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
| `league-ledger` | The XRPL Testnet client. It refuses any network that is not the Testnet |
| `league-standalone` | The stand-alone `xrpld` harness for training mode, in a pinned, resource-capped container |
| `league-season` | The Testnet's announced reset dates, and a check that spots an unannounced reset |
| `league-relay` | The tool that ran the paper playtests with AI seats |
| `league-tui` | The offline Club Mode terminal host: Lobby setup, four-zone hot-seat turns, Privacy Curtain with optional PINs, timed passes, mid-turn save/resume, end-of-season scoreboard, and replay export |

## Play a season

Offline **Club Mode** runs a full `24..=32`-turn hot-seat season in your terminal (`4–6` human or `ClubBot` seats on one machine):

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Lobby setup:** Choose seat count (`4–6`, `Left`/`Right`), toggle each seat between `Human` and `ClubBot` (`Space`), enter a clear `SeedPool::Dev` seed in `0..=9999` (reserved test seeds and other pools are refused by name), choose a turn budget (`180–300 s`, `210 s` default, or `Untimed` with `Space`), and toggle 4-digit table-local Curtain PINs. Press `Enter` on **Start Club Mode Table**. Starting while an unfinished `club-*.seal` exists in `--save-dir` is refused (`UNFINISHED_SEASON_EXISTS`); once exported, the next table receives the next free table ID without overwriting prior saves. Command-line flags (`--turn-clock`, `--untimed`, `--pins`) override conflicting Lobby and saved `.seal` choices (`league-tui --help` documents precedence; passing PIN digits via `--pin` on the CLI is refused).
- **Turn keys:** `1`/`2`/`3`/`Tab`/`BackTab` switch enabled composer tabs (`4: Diplomatic Desk (Disabled)` stays disabled); `F6` cycles focus across the Public Ledger, Composer Tabs, and Private Vault; `[`/`]` cycles action kinds or bundle sections; `Up`/`Down`/`PageUp`/`PageDown`/`Home` selects or scrolls; `Left`/`Right`/`+`/`-`/`Space` adjusts fields; `e` edits free-text overrides; `n` edits personal Private Vault notes; `Enter` submits an action, stages a bundle section, or runs the `p` What-If Sandbox; `Del`/`Backspace`/`X` clears staged orders; `c` toggles guild good names vs 3-letter codes (`[CRED]`, `[TMB]`); `r`/`F2` opens the Sweep Chronicle modal; `Ctrl+l`/`Ctrl+L` locks your pass and opens the Privacy Curtain; `q`/`Esc`/`Ctrl+c` quits cleanly.
- **Privacy Curtain, dead time & PINs:** Every hand-off, timeout, and in-season resume opens a public Privacy Curtain (clearing the alternate screen in place with `CSI 2 J` and never `CSI 3 J` scrollback purge), drains queued input, and enforces a `400 ms` input dead time both when opening and when lifting (lift dead time does not count against the arriving seat's clock). Press `Space` then `Enter` to lift; if PINs are enabled, type your masked 4-digit PIN (entered only behind the Curtain — the 1st turn sets it, later hand-offs verify it, wrong tries build zero seat observations, and 3 misses reset back to the unarmed Curtain).
- **Clock:** Timed turns split each budget `75%` Sub-Pass A / `25%` Sub-Pass B across all `H` human seats (`floor(0.75 * B / H)` s and `floor(0.25 * B / H)` s). Locking early advances immediately; idle Sub-Pass B timeouts resolve via the Standing Regent fallback doctrine.
- **Save & resume:** With `--save-dir <DIR>`, the host autosaves atomically at every Curtain, initial PIN commit, and Sweep leaving `TalkAndMarket`. Public state saves to `club-<id>.save.json`; hidden mandates, salts, pre-derived VRF proofs, notes, uncommitted bundles, and PINs stay solely in `club-<id>.seal`. Resume any time with `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (or `--resume ./saves`, selecting the active `.seal` table): in-season resumes always land behind the active human seat's Curtain with a fresh pass clock (advancing any leading `ClubBot` cursors first), while post-season resumes land directly on the public `Session Standby` reveal screen (`AwaitingMandateReveal`) or `Completed` End-of-Season tabs so you can retry replay export. Passing `--resume` together with a different `--save-dir` that already contains another season refuses to overwrite it (`UNFINISHED_SEASON_EXISTS` or `SEASON_SAVE_EXISTS`).
- **End of season & replay export:** After the Closing Bell closes `TalkAndMarket`, step through the public standby mandate-reveal stage (`Enter`/`Space` per seat, showing zero mandates beforehand) to finalize into the four-tab End-of-Season screen (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). On Tab 4, press `e` or `Enter` to write and verify `club-<id>.replay.json` and wipe the private `.seal` file (if export fails, `.seal` stays on disk so you can retry).

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

`bash scripts/verify.sh` runs the checks CI runs, except the Docker harness and the Atlas map check. Coverage, `cargo audit` and the site build run only when `cargo-llvm-cov`, `cargo-audit` and `npm` are installed, and the script names any check it skipped.

## Security and data

- **What it touches.** In offline Club Mode (`league-tui`), `--save-dir` writes public `club-<id>.save.json`, private hot-seat `club-<id>.seal` (deleted upon successful replay export), verified `club-<id>.replay.json`, and optional `--log-file` metadata diagnostics (never logging PINs, Vault notes, mandates, staged orders, or commit salts, and redacting API/XRPL credentials and sealed JSON keys across all four log levels) on your own machine with zero network calls. In Game mode (not built yet), a player's Testnet keys stay on their own machine, in the git-ignored `.league/` folder, and are never logged. The paper-test relay reads its OpenRouter key only from the `OPENROUTER_API_KEY` environment variable and makes no OpenRouter call without `--live` and a credit cap. Its local seats refuse any API key, reach only a loopback address or a VM address you allow with `--allow-vm-host`, and need a GPU-seconds cap. It keeps its transcripts in the game folder you give it.
- **What it does not touch.** No real money, no Mainnet, and nothing outside the folders above. There is **no telemetry**: nothing is sent anywhere except the calls a mode needs (the Testnet, a public randomness beacon and, for the relay, OpenRouter or the local model servers you allow).
- **Permissions.** Network access to the Testnet in Game mode. Docker, with its port bound to `127.0.0.1` only, for the stand-alone harness.
- **Guards.** Every Testnet connection must report the Testnet's network id. A CI tripwire fails on any Mainnet hostname written in the repository outside vendored code, and a second one fails on copy that frames the game as a source of income. Review covers what a word list can't see.

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
