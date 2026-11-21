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
| [`docs/club-mode-design.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/club-mode-design.md) | Offline Club Mode (`v0.1.0`) specification: host/seat `no_std` boundary, hot-seat Privacy Curtain, timed passes, What-If Sandbox, save/resume security, and `SHIP_GATE` closure |
| [`docs/design.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/design.md) | The first design, kept for its history. `design-v2.md` supersedes it |
| [`docs/standalone-xrpld.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/standalone-xrpld.md) | The stand-alone `xrpld` harness: the pinned image, its limits and its isolation checks |
| [`docs/paper-test.md`](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/docs/paper-test.md) | How the paper playtests were run |
| [`docs/paper-test/`](https://github.com/mcp-tool-shop-org/merchant-league/tree/main/docs/paper-test) | The playtests' setups, logs, scores and results |
| [`docs/receipts/`](https://github.com/mcp-tool-shop-org/merchant-league/tree/main/docs/receipts) | Live checks taken on the Testnet |

The Testnet is wiped from time to time (next on 2026-11-25), which deletes every account and transaction on it. After a wipe, the receipts' transaction hashes stop resolving. What they recorded, and the test vectors in `crates/league-ledger/tests/vectors/`, stay the evidence.

## `league-tui`

The offline hot-seat host binary runs full Club Mode seasons in a terminal without network calls or Testnet credentials (prototype — not released yet).

```text
league-tui [options]
```

| Option | Meaning |
| --- | --- |
| `--save-dir <DIR>` | Directory for atomic `club-<id>.save.json`, private `club-<id>.seal`, and exported `club-<id>.replay.json` |
| `--resume <PATH>` | Resume from a `club-<id>.save.json` file or directory containing one (in-season resumes always open behind a Curtain; post-season resumes open on the public Session Standby reveal view or Completed End-of-Season tabs) |
| `--turn-clock <180..=300>` | Override whole-turn clock budget in seconds (`75%` Sub-Pass A / `25%` Sub-Pass B across `H` humans; `210` default) |
| `--untimed` | Disable timed turn auto-locks (overrides Lobby and `.seal` clock settings; conflicts with `--turn-clock`) |
| `--pins` | Enable 4-digit table-local Curtain PINs entered interactively behind the Curtain (passing `--pin` on CLI fails with `PIN_CLI_FORBIDDEN`) |
| `--error-format <text\|json>` | Error output encoding on `stderr`: `text` (default `error: [CODE] ...; hint: ...`) or compact `StructuredError` JSON |
| `--log-level <level>` | Diagnostic verbosity: `silent`, `normal` (default), `verbose`, or `debug` (or `LEAGUE_LOG_LEVEL` env var) |
| `-q`, `--silent` | Shorthand for `--log-level silent` |
| `-v`, `--verbose` | Shorthand for `--log-level verbose` (turn/pass state transitions and resumed metadata after terminal restore) |
| `--debug` | Shorthand for `--log-level debug` (Curtain/drain events and scrubbed panic backtraces after terminal restore) |
| `--log-file <PATH>` | Append scrubbed diagnostic lines directly to `<PATH>` |
| `-h`, `--help` | Print usage, CLI override precedence, and exit codes (succeeds without a TTY) |
| `-V`, `--version` | Print `league-tui` version (succeeds without a TTY) |

**CLI precedence:** `--turn-clock`, `--untimed`, and `--pins` override conflicting Lobby UI choices and resumed `.seal` settings; `--log-level`, `-q`, `-v`, and `--debug` override `LEAGUE_LOG_LEVEL`.

### `league-tui` logging and panic safety

While the alternate screen is active, zero diagnostic lines are written to the terminal screen stream; buffered diagnostics flush to `stderr` only after cooked terminal mode is restored (`disable_raw_mode()` + `LeaveAlternateScreen`). Credentials, API keys, XRPL seeds, and sealed JSON keys are redacted across all four log levels, while PINs, private Vault notes, dealt mandates, staged orders, and commit salts are never passed to logger or panic lines. An uncaught panic restores cooked terminal mode before writing one redacted line to `stderr` (`stack backtrace:` appears solely under `--debug`).

### `league-tui` exit codes

| Code | Meaning |
| --- | --- |
| 0 | Clean completion (`--help`, `--version`, quit from Lobby or active/completed session cleanly) |
| 1 | User or configuration error (bad or conflicting flags, invalid `LEAGUE_LOG_LEVEL`, `--pin` on CLI [`PIN_CLI_FORBIDDEN`], missing `--resume` target [`INVALID_RESUME_TARGET`], conflicting or unreadable `--save-dir` target [`UNFINISHED_SEASON_EXISTS` / `SEASON_SAVE_EXISTS` / `SAVE_DIR_UNREADABLE`], or non-TTY launch [`TTY_REQUIRED`], checked before raw mode is entered) |
| 2 | Runtime error (corrupt `.save.json` / `.seal` checkpoint refusal, autosave failure [`TUI_AUTOSAVE_FAILED`], wedged pass error, or terminal I/O failure) |
| 3 | Partial success (season finalized cleanly through `TurnPhase::Completed`, but `club-<id>.replay.json` write, target check, or `.seal` wipe failed [`TUI_REPLAY_EXPORT_FAILED` / `REPLAY_TARGET_EXISTS`]; `.seal` stays intact for retry) |

An uncaught main-thread panic exits with Rust's standard code `101` after restoring cooked terminal mode.

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
| `--preview-tool <off\|on>` | Enable own-orders `preview_turn_orders` tool on `act` turns (`off` default; locks wording arm `v2_preview_2026_11_20_r2`, while `v2_preview_2026_11_17_r1` stays sealed; probes `relay/seats/` writability at startup, requires `to-seat/day-DD-act-<Guild>.preview-state.json` with `seat` equal to the `0`-based index in `seats.json`; when the tool is `on`, every chat-completions HTTP reply body is kept as received before the relay reads it — `/tokenize` replies are not kept — in `relay/seats/<Guild>.day-DD-<round>.http-reply-round-<n>-attempt-<a>.json` linked by `http_reply_sha256`, credential-redacted only with game values kept as written by the model, while raw files hold `body_sha256` and join by `reservation_id`: a fresh `"RESERVED"` line starts with `null`, and on the next run orphan migration scans regular `*.http-reply-round-*` files in that seat's `relay/seats/` directory and links the matching file's SHA-256 when `1` matches, keeps `null` when `0` match, or keeps `null` with one `[normal]` diagnostic naming `reservation_id` and match count when `>=2` match — including non-200, late, non-JSON, provider-error, and thinking-only / token-exhausted replies even though no answer was delivered — and credential-redacted (keeping game values as written) `tool-round-<n>.json` and `discarded-round-<n>-attempt-<a>.json` sidecars linked by `reply_sha256` are built from it without ever replaying raw or sidecar files to any model or showing them to seats; refuses odd `tool_calls` via `MALFORMED_TOOL_CALL` on `act` while not executing `tool_calls` when `off` or `talk`. A game folder holds sealed game values (in `relay/seats/` and the `.seal`), so it must not be committed or published) |
| `--error-format <text|json>` | Error encoding on `stderr`: `text` (default) or compact `StructuredError` JSON |
| `--log-level <level>` | Diagnostic verbosity: `silent`, `normal` (default), `verbose`, or `debug` (or `LEAGUE_LOG_LEVEL`) |
| `-q`, `--silent` | Shorthand for `--log-level silent` |
| `-v`, `--verbose` | Shorthand for `--log-level verbose` (timings and cap accounting) |
| `--debug` | Shorthand for `--log-level debug` (redacted transport, hash, lock metadata, and panic backtraces) |
| `-h`, `--help` | Print the help |

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Done; every seat's turn was clean (`--help` and passing `prebook-check` also exit `0`) |
| 1 | User or configuration error (bad arguments or `LEAGUE_LOG_LEVEL`, bad seats file, refused key placement, live/local gate refusal, missing `--wrapper` on turn 1, incompatible setup, or missing/malformed/wrong-day/wrong-seat `.preview-state.json` [`PREVIEW_STATE_*`] with `--preview-tool on` on `act`) |
| 2 | Runtime error (`TURN_LOCKED`, `UNRECORDED_CHARGE`, `TOOL_ROUND_STORE_WRITE_FAILED`, file I/O or missing `--game-dir`, corrupt log/setup, failed `prebook-check`, or a `turn` with `0` clean seats) |
| 3 | Partial success (`turn` recorded cleanly on disk with at least `1` clean seat and at least `1` failed seat) |

An uncaught panic on the main thread exits with Rust's standard code `101` (with secrets redacted on `stderr`, and a stack backtrace emitted only under `--debug`).

**Breaking change (`B7`):** Exit `4` is retired, `TurnLocked` and `UnrecordedCharge` now return `2`, `RelayError::Io` / `CorruptLog` return `2`, `3` now means partial success, and default `--error-format text` `stderr` output changed from `error: <msg>` to `error: [CODE] <msg>; hint: <hint>`. Scripts that matched exit `3` to catch unrecorded spend or scraped `error: <msg>` text must check exit `2` and pass `--error-format json` (`code == "UNRECORDED_CHARGE"`).

## Project files

- [CHANGELOG](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/CHANGELOG.md)
- [SECURITY.md](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/SECURITY.md)
- [LICENSE](https://github.com/mcp-tool-shop-org/merchant-league/blob/main/LICENSE) (MIT)
