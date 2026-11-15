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

On a seat's first turn (when `<game-dir>/relay/<Guild>.jsonl` has no prior turns), pass `--wrapper <path>`. The relay prepends that file's text ahead of `<game-dir>/to-seat/day-DD-<round>-<Guild>.md` with one blank line between them, sends the request with no system prompt (`messages` holds only `user` and `assistant` turns), and appends both sides to `<game-dir>/relay/<Guild>.jsonl`.

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

Once `<game-dir>/relay/<Guild>.jsonl` holds earlier turns, `league-relay` replays every stored `user` and `assistant` message in order followed by the new `to-seat` block. Even if `--wrapper` is still passed on the command line, the wrapper is only prepended on a seat's first turn.

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
- `relay/<Guild>.jsonl` — alternating `user` and `assistant` JSON lines for that seat. Updated only when a turn delivers a non-empty reply.
- `relay/calls.jsonl` — one JSON line per call or pre-call refusal: `time_utc`, `day`, `round`, `guild`, `attempt`, `trigger`, `model_requested`, `model_returned`, `request_body_sha256`, `reply_sha256`, `prompt_tokens`, `completion_tokens`, `total_tokens`, `cost`, `latency_ms`, `outcome`, and optional redacted `error`.

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
      "timeout_secs": 120
    },
    {
      "guild": "Weavers",
      "backend": "openrouter",
      "model": "openai/gpt-oss-120b",
      "temperature": 0.7,
      "max_output_tokens": 1024,
      "timeout_secs": 120
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

- `guild`: unique, non-empty ASCII letters, digits, `-`, or `_`.
- `backend`: `"openrouter"` or `"file"`. A `"file"` seat makes no call and prints `outcome=FILE_MANUAL`.
- `model` (required for `"openrouter"`): exact `provider/model` OpenRouter slug. Floating aliases (`openrouter/auto`, `*/auto`, `latest`, `*-latest`) and `:` routing suffixes (`:free`, `:nitro`, `:floor`, `:online`) are rejected.
- `temperature` (required for `"openrouter"`): finite number in `0.0..=2.0`.
- `max_output_tokens` (required for `"openrouter"`): positive integer sent as `max_tokens`.
- `timeout_secs` (required for `"openrouter"`): positive finite wall-clock limit per call attempt in seconds.
- Keys or tokens inside the seat file are rejected.

## Delivery failures and retries

A delivery failure is any of:

- transport or TLS/connection error;
- non-200 HTTP status or provider error object;
- empty or whitespace-only reply content;
- no complete reply within the seat's `timeout_secs`.

On the first delivery failure, `league-relay` logs the failed call in `relay/calls.jsonl` with `outcome: "harness"` and sends **one** identical retry (`attempt: 2`, `trigger: "harness"`, byte-identical request body and `request_body_sha256`). If the second attempt also fails, `league-relay` logs `outcome: "NO_REPLY"`, writes no `from-seat` file, and leaves `<Guild>.jsonl` unchanged.

A non-empty reply in the wrong game form is **not** retried. It is written verbatim to `from-seat/` so the referee rules on it under the turn protocol.

## Credit cap and unknown-cost rule

Before every call (including a `harness` retry), `league-relay` sums all numeric `cost` entries recorded in `<game-dir>/relay/calls.jsonl`:

- **Cap reached (`CAP_REACHED`):** if the recorded sum has reached or passed `--cap-credits <n>`, the call is not sent, `outcome: "CAP_REACHED"` is appended to `relay/calls.jsonl`, and nothing is written to `from-seat/` or `<Guild>.jsonl`.
- **Unknown cost (`COST_UNKNOWN`):** if an HTTP 200 completion response carries no `usage.cost` field, `league-relay` logs its token counts (`prompt_tokens`, `completion_tokens`, `total_tokens`) with `cost: null` and refuses any further calls in that turn with `outcome: "COST_UNKNOWN"` (fail closed).

## Secrets

`OPENROUTER_API_KEY` is read from the environment only. Passing `--api-key` (or any key flag) on the command line or placing a key field in `seats.json` is rejected without echoing the value. `ApiKey`, `LiveGate`, and `UreqTransport` redact the key in `Debug` output, and all error, panic, and log paths scrub the key value as well as `Bearer ...` and `sk-or-...` tokens.
