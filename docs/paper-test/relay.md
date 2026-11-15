# Paper-test seat relay (`league-relay`)

`league-relay` sends one round (`talk` or `act`) of a paper-test market day to automated OpenRouter seats, records local transcripts and call metadata, and writes each seat's reply verbatim for the referee. Manual seats (such as Claude Code subagents) use the `file` backend and are reported without making any call.

No game directories or prompts live in this repository. Pass `--game-dir <dir>` pointing at an external folder for each game.

## Running one turn

Every live call requires three things together:

1. `--live` on the command line;
2. `--cap-credits <n>` setting the cumulative credit ceiling for the game;
3. `OPENROUTER_API_KEY` set in the process environment (never in a file or flag).

Without `--live`, `league-relay` refuses to touch the network even when `OPENROUTER_API_KEY` is set.

### First turn (requires `--wrapper`)

On a seat's first turn (when `<game-dir>/relay/seats/<Guild>.jsonl` has no prior turns), pass `--wrapper <path>`. The relay prepends that file's text ahead of `<game-dir>/to-seat/day-DD-<round>-<Guild>.md` with one blank line between them, sends the request with no system prompt (`messages` holds only `user` and `assistant` turns), and appends both sides to `<game-dir>/relay/seats/<Guild>.jsonl`.

```sh
OPENROUTER_API_KEY=... cargo run -p league-relay -- turn \
  --game-dir /path/to/game-03 \
  --day 01 \
  --round talk \
  --seats /path/to/game-03/seats.json \
  --wrapper /path/to/wrapper.md \
  --live \
  --cap-credits 2.50
```

### Later turns

Once `<game-dir>/relay/seats/<Guild>.jsonl` holds earlier turns, `league-relay` replays every stored `user` and `assistant` message in order followed by the new `to-seat` block. Even if `--wrapper` is still passed on the command line, the wrapper is only prepended on a seat's first turn.

```sh
OPENROUTER_API_KEY=... cargo run -p league-relay -- turn \
  --game-dir /path/to/game-03 \
  --day 01 \
  --round act \
  --seats /path/to/game-03/seats.json \
  --live \
  --cap-credits 2.50
```

### Offline mock runs

For dry runs and harness checks without a key or network socket, pass `--mock-dir <dir>` containing `responses.json`:

```sh
cargo run -p league-relay -- turn \
  --game-dir /path/to/mock-game \
  --day 01 \
  --round talk \
  --seats /path/to/mock-game/seats.json \
  --wrapper /path/to/wrapper.md \
  --mock-dir /path/to/mock-fixtures \
  --cap-credits 1.00
```

## Directory layout and verbatim writes

Within `<game-dir>`:

- `to-seat/day-DD-<round>-<Guild>.md` — input block prepared for that seat and round.
- `from-seat/day-DD-<round>-<Guild>.md` — assistant reply written **byte-for-byte** (`reply.as_bytes()`). Trailing newlines and line endings are neither added nor stripped, so the file's SHA-256 always equals `reply_sha256` in `relay/calls.jsonl`.
- `relay/seats/<Guild>.jsonl` — alternating `user` and `assistant` JSON lines for that seat. Updated only when a turn delivers a non-empty reply.
- `relay/calls.jsonl` — one JSON line per call or pre-call refusal: `time_utc`, `day`, `round`, `guild`, `attempt`, `trigger`, `model_requested`, `model_returned`, `request_body_sha256`, `reply_sha256`, `prompt_tokens`, `completion_tokens`, `total_tokens`, `cost` (what OpenRouter reported, or `null`), `charged_credits` (the amount counted against `--cap-credits`), `latency_ms`, `outcome`, and optional redacted `error`.

All `openrouter` seats in a turn run concurrently. An error, timeout, or missing input file on one seat never stops another seat from finishing. `league-relay` exits `0` when every seat succeeds (or is `file`), `2` when any seat fails to deliver a reply, and `1` when CLI arguments, seat configuration, or the live gate are invalid.

## Seat file format

The seat file is JSON (`{"seats": [...]}` or a bare `[...]` array), with one object per seated guild:

```json
{
  "seats": [
    {
      "guild": "Saltworks",
      "backend": "openrouter",
      "model": "google/gemini-2.5-pro-06-05",
      "temperature": 0.7,
      "max_output_tokens": 1024,
      "timeout_secs": 120,
      "max_cost_per_call": 0.05
    },
    {
      "guild": "Weavers",
      "backend": "openrouter",
      "model": "openai/gpt-oss-120b",
      "temperature": 0.7,
      "max_output_tokens": 1024,
      "timeout_secs": 120,
      "max_cost_per_call": 0.05
    },
    {
      "guild": "Smiths",
      "backend": "file",
      "model": "claude-sonnet-5-5"
    }
  ]
}
```

Validation rules:

- `guild`: case-insensitively unique across the seat file, matching `[A-Za-z][A-Za-z0-9_-]*`.
- `backend`: `"openrouter"` or `"file"`. A `"file"` seat makes no call and prints `outcome=FILE_MANUAL`.
- `model` (required for `"openrouter"`): exact `provider/model` OpenRouter slug. Floating aliases (`openrouter/auto`, `*/auto`, `latest`, `*-latest`) and `:` routing suffixes (`:free`, `:nitro`, `:floor`, `:online`) are rejected.
- `temperature` (required for `"openrouter"`): finite number in `0.0..=2.0`.
- `max_output_tokens` (required for `"openrouter"`): positive integer sent as `max_tokens`.
- `timeout_secs` (required for `"openrouter"`): positive finite wall-clock limit per call attempt in seconds.
- `max_cost_per_call` (required for `"openrouter"`): positive finite number in credits reserved against `--cap-credits` before each call.
- Keys or tokens inside the seat file are rejected.

## Delivery failures and retries

A delivery failure is any of:

- transport or TLS/connection error;
- non-200 HTTP status or provider error object;
- empty or whitespace-only reply content;
- no complete reply within the seat's `timeout_secs`.

On the first delivery failure, `league-relay` logs the failed call in `relay/calls.jsonl` with `outcome: "harness"` and sends **one** identical retry (`attempt: 2`, `trigger: "harness"`, byte-identical request body and `request_body_sha256`). If a reply arrives after `timeout_secs` has elapsed, it is discarded rather than delivered, recorded in `relay/calls.jsonl` with `outcome: "LATE_DISCARDED"` (including its `cost` and token counts), and counted against `--cap-credits` before the `harness` retry starts. If the second attempt also fails, `league-relay` logs `outcome: "NO_REPLY"` (or `"LATE_DISCARDED"` if the second attempt also arrived after `timeout_secs`), writes no `from-seat` file, and leaves `relay/seats/<Guild>.jsonl` unchanged.

A non-empty reply in the wrong game form is **not** retried. It is written verbatim to `from-seat/` so the referee rules on it under the turn protocol.

## Credit cap, reservation, and fail-closed rules

Before every call (including a `harness` retry), `league-relay` takes one lock on the call ledger, sums `charged_credits` across `<game-dir>/relay/calls.jsonl` (plus any unrecorded in-memory charge from this run), and checks whether `charged + reserved_in_flight + max_cost_per_call <= cap`. Every sent attempt records `cost` as what OpenRouter reported and `charged_credits` as `cost` when a valid `usage.cost` is present or that seat's `max_cost_per_call` when no valid cost was recorded (including a late HTTP 200 without `usage`, an on-time empty HTTP 200 without `usage`, or a client timeout before a `harness` re-send); pre-call refusals record `charged_credits: 0.0`. Per OpenRouter's usage-accounting documentation (<https://openrouter.ai/docs/use-cases/usage-accounting>, read 2026-11-15), usage including cost is returned automatically on every response (`usage: {include: true}` is deprecated and has no effect), and any response missing `usage.cost` is charged at `max_cost_per_call`. When a call is allowed to start, that seat's `max_cost_per_call` stays in `reserved_in_flight` until the call finishes and its ledger line is appended under the same lock, so concurrent seats cannot pass the same cap check together:

- **Cap reached (`CAP_REACHED`):** if `charged + reserved_in_flight + max_cost_per_call` would pass `--cap-credits <n>`, the call is not sent, `outcome: "CAP_REACHED"` is appended to `relay/calls.jsonl`, and nothing is written to `from-seat/` or `relay/seats/<Guild>.jsonl`.
- **Ledger write failure (`LEDGER_ERROR`):** if appending to `relay/calls.jsonl` fails after a sent call, the call's charge stays held in memory against `--cap-credits` for the rest of the run and no further calls start in that turn (fail closed).
- **Cost over estimate (`COST_OVER_ESTIMATE`):** if a returned `usage.cost` exceeds that seat's `max_cost_per_call`, `league-relay` records the returned cost in `relay/calls.jsonl` with `outcome: "COST_OVER_ESTIMATE"` and refuses any further calls in that turn (fail closed). Seats already in flight are not stopped, so the cap can be passed by their actual cost minus their estimate; the worst case per turn is the sum over in-flight seats (for example, at cap `0.02`, two concurrent seats with `max_cost_per_call: 0.01` that both return `0.05` record `0.10` before the turn stops).
- **Unknown cost (`COST_UNKNOWN`):** if an HTTP 200 completion response carries no `usage.cost` field, `league-relay` logs its token counts (`prompt_tokens`, `completion_tokens`, `total_tokens`) with `cost: null` and `charged_credits: max_cost_per_call`, and refuses any further calls in that turn with `outcome: "COST_UNKNOWN"` (fail closed).

## Secrets

`OPENROUTER_API_KEY` is read from the environment only. Passing `--api-key` (or any key flag) on the command line or placing a key field in `seats.json` is rejected without echoing the value. `ApiKey`, `LiveGate`, and `UreqTransport` redact the key in `Debug` output; error and log paths cap untrusted input at `4 KiB` (`4096` bytes), scrub the full key, any `8+` character substring of `OPENROUTER_API_KEY`, `Bearer ...`, and `sk-or-...` tokens, and only then truncate to the display limit; and the `league-relay` binary installs `install_redacting_panic_hook` at startup so panic messages written to `stderr` apply the same redaction (library callers can install it with `install_redacting_panic_hook` as well).
