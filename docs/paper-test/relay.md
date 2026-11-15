# Seat relay (`league-relay`)

`league-relay` sends one round (`talk` or `act`) of a market day to automated seats — either a keyless OpenAI-compatible `local` server on the VM (such as vLLM, capped in GPU-seconds via `--cap-gpu-secs`) or `openrouter` seats (gated behind `--live`, `--cap-credits`, and `OPENROUTER_API_KEY`) — records local transcripts, setup, and call metadata, and writes each seat's reply verbatim for the referee. Manual seats (such as Claude Code subagents) use the `file` backend and are reported without making any call.

No game directories or prompts live in this repository. Pass `--game-dir <dir>` pointing at an external folder for each game.

## Running one turn

### Local OpenAI-compatible VM transport (`backend: "local"`)

Local seats talk over HTTP to `/v1/chat/completions` and `/tokenize` at a configured `base_url` that must resolve to loopback (`127.0.0.0/8`, `[::1]`, or `localhost`) or a private RFC1918 (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) / IPv6 ULA (`fc00::/7`) address explicitly named with `--allow-vm-host <host>` and referenced by at least one `local` seat in `seats.json` (outside/public hosts, `0.0.0.0`/`::`, link-local `169.254.0.0/16` / `fe80::/10`, IPv4-mapped IPv6, and DNS hostnames are rejected, and `LocalOpenAiTransport` sets `proxy(None)` and `max_redirects(0)` so `HTTP_PROXY`/`ALL_PROXY` environment variables can never steer local traffic off the VM):

1. `--cap-gpu-secs <n>` sets the cumulative GPU-seconds ceiling for the game. Every `local` seat must configure `timeout_secs + 0.05 <= max_gpu_secs_per_call` (`LOCAL_TIMEOUT_MARGIN_SECS`), and post-call accounting clamps both `cost` and `charged_credits` to at most `max_gpu_secs_per_call` while recording raw `wall_secs` verbatim (and still flagging `COST_OVER_ESTIMATE` if an OS pause pushes `wall_secs > max_gpu_secs_per_call`) so a call running to its timeout can never exceed its pre-call reservation or push cumulative spend past `--cap-gpu-secs`. **Client wall-clock measurement note (`B-7`, `R2-B6`):** `--cap-gpu-secs` tracks client-observed HTTP `/v1/chat/completions` wall-clock seconds rather than server hardware profiler counters; when `k` seats execute concurrently against one shared local GPU server, their summed wall-clock seconds over-count elapsed server GPU time up to `k`-fold (fail-safe toward the cap), whereas if a client disconnects at `timeout_secs` and the server continues decoding tokens after socket close, that server-side tail is not visible in client `wall_secs` (under-counting), and lightweight `POST /tokenize` scratchpad checks are uncounted server CPU work outside `wall_secs` and `--cap-gpu-secs`.
2. **No API key may be passed:** `league-relay` refuses (`exit 1`) if `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `VLLM_API_KEY`, or `LOCAL_API_KEY` (case-insensitively) is set in the environment, or if any key flag, URL credential, or seat-file key field is supplied, so keys never leak into VM logs;
3. Omit `--live` (which applies only to OpenRouter).

```sh
cargo run -p league-relay -- turn \
  --game-dir /path/to/colab-game-01 \
  --day 01 \
  --round talk \
  --seats /path/to/colab-game-01/seats.json \
  --wrapper /path/to/wrapper.md \
  --cap-gpu-secs 600
```

### OpenRouter transport (`backend: "openrouter"`)

Every live OpenRouter call requires three things together:

1. `--live` on the command line;
2. `--cap-credits <n>` setting the cumulative credit ceiling for the game;
3. `OPENROUTER_API_KEY` set in the process environment (never in a file or flag).

Without `--live`, `league-relay` refuses to touch OpenRouter even when `OPENROUTER_API_KEY` is set. A single game directory and `seats.json` cannot mix `local` (`gpu_seconds`) and `openrouter` (`credits`) seats.

`--game-dir <dir>` must point to an existing directory; if `<game-dir>` does not exist (or is not a directory), `league-relay` refuses immediately (`exit 1`) before creating `relay/` or any other path.

Only **one** `league-relay turn` may run against a `<game-dir>` at a time. At the start of `run_turn` (before any ledger read, side-file migration, or cap reservation), `league-relay` acquires an exclusive OS file lock on `<game-dir>/relay/turn.lock` (recording `pid=<pid> started_utc=<ts>`), captures that open handle's OS file identity (`dev` + `ino` on Unix; volume serial number + file index on Windows), and holds it until all turn writes — including the end-of-turn ledger flush or side-file write — have finished. Before every call goes out, `league-relay` checks `relay/turn.lock` identity, writes a durable pre-call reservation with a unique `reservation_id` at `max_cost_per_call` to `<game-dir>/relay/calls.reserved.jsonl` (under a short exclusive OS lock on `<game-dir>/relay/calls.side.lock`), and checks `relay/turn.lock` identity again before dispatching the call; if `relay/turn.lock` is deleted or replaced mid-turn, `league-relay` refuses further transport calls, finishes recording what was already spent, and exits `4` (`TurnLocked`), while any new run that took a replacement `turn.lock` already sees the old run's in-flight reservations on disk in `relay/calls.reserved.jsonl`. What still remains after this protection: (1) deleting or replacing the relay's own files (`relay/calls.reserved.jsonl`, `relay/calls.side.lock`, or a live `relay/calls.resv-<run_id>.lock`) breaks its guarantees — deleting `relay/calls.reserved.jsonl` or `relay/calls.side.lock` can lose an active reservation or side-file write and under-count spend, the same as deleting the ledger (`relay/calls.jsonl` or `relay/calls.pending.jsonl`), whereas deleting a live `relay/calls.resv-<run_id>.lock` makes a live reservation appear orphaned so a later run migrates it as `RESERVED` alongside the live run's final line (over-counting rather than under-counting); (2) if `relay/turn.lock` is deleted in the microsecond gap between `try_reserve`'s post-write identity check and the transport thread sending the request, that one already-reserved call still goes out before the old run exits `4` (its `max_cost_per_call` reservation is already on disk, so `--cap-credits` still counts it); (3) `--cap-credits` can still be exceeded when an in-flight call spends more than its reserved `max_cost_per_call` (`COST_OVER_ESTIMATE`, bounded by `cost - max_cost_per_call` per in-flight seat; likewise, a reservation reconciled after a crash or exit `3` charges `max_cost_per_call` because no post-call line could be recorded); and (4) on Windows an outside opener holding `relay/turn.lock` with no share flags causes a fail-safe `TurnLocked` exit `4` after recording spend and holding a side file without delete sharing makes atomic rename fail safe, an unreadable `relay/calls.resv-<run_id>.lock` keeps its reservation counted fail-safe until readable, 64-bit file indices on ReFS volumes are not guaranteed unique by the filesystem, and `--game-dir` must reside on local disk because OS file locks on some network filesystems hold only locally. A second concurrent run on the same `<game-dir>` refuses immediately (`TurnLocked`, exit `4`) before making any transport call (note: on Windows `LockFileEx` prevents outside processes from reading `relay/turn.lock` while held — `os error 33` — so the refusal message cannot show the holder's `pid`/`started_utc` and reports that the lock is held by an active process). **Never delete `relay/turn.lock` or any file under `relay/`**: `league-relay` clears and unlocks `relay/turn.lock` on every controlled exit path (`0`, `1`, `2`, `3`, and `4`), and a leftover file after a hard kill (`SIGKILL`, `TerminateProcess`, crash, or power loss) is harmless because exclusivity is enforced by an OS kernel file lock (`File::try_lock`), which the kernel releases as soon as the dead process's file handles close so the next run takes over `relay/turn.lock` and reconciles any orphaned `relay/calls.reserved.jsonl` entries automatically without blocking.

### First turn (`--wrapper` as `role: "system"`) and `--memory-mode`

On a seat's first turn (when `<game-dir>/relay/seats/<Guild>.jsonl` has no completed turns), pass `--wrapper <path>`. `league-relay` sends that wrapper as a dedicated `role: "system"` message (`messages[0]`) and persists it as the leading `"system"` line of `<game-dir>/relay/seats/<Guild>.jsonl`.

Prompt memory delivery is controlled by `--memory-mode <snapshot|emc|full>` and locked in `<game-dir>/relay/setup.json` alongside `cost_unit`, `wording_arm`, `scratchpad`, `mixed_scratchpad_arm`, `response_schema_name` / `response_schema_sha256`, and per-seat `scratchpad` and `reasoning` pins (changing any of those mid-game on the same `--game-dir` is refused):

- **`snapshot` (default, ruled under C5 / `R2-B1`–`R2-B3`):** every turn sends bounded `[system, user]` messages containing the current turn's state snapshot. On `local` seats with a `--response-schema <path>` whose `"scratchpad"` property is present, `<game-dir>/relay/seats/<Guild>.scratchpad.md` is included and capped at `500` (`MAX_SCRATCHPAD_TOKENS`) model tokens counted on the exact escaped text shown to the model (`R2-B2`) via the local server's `POST /tokenize` endpoint (`{"model": ..., "prompt": ..., "add_special_tokens": false}`, per vLLM's Tokenizer API at <https://docs.vllm.ai/en/stable/serving/online_serving/openai_compatible_server/#tokenizer-api>); the notes instruction appears **once**, solely in that schema field's pinned `description` (`"Notes for your next turn, replacing earlier notes (at most 500 tokens; empty string for none)."`), while relay prose states only `"Your notes from earlier turns are below (at most 500 tokens)."` If `/tokenize` is unavailable on a scratchpad-keeping seat, `league-relay` refuses the turn rather than guessing; if a returned scratchpad's escaped shown text exceeds `500` tokens, the attempt is rejected (`outcome: "harness"` then `"NO_REPLY"`). OpenRouter seats in `snapshot` mode keep **no** scratchpad (`"scratchpad": "none (no tokenizer)"` in `relay/setup.json`, omitting any notes prose/header and stripping `"scratchpad"` from any shared wire schema); runs without `--response-schema` (or whose schema has no `"scratchpad"` property) likewise keep no scratchpad (`"scratchpad": "none (no response schema)"` / `"none (schema without scratchpad)"`), and asking for `--scratchpad` or `"scratchpad": true` on an OpenRouter seat or without a `"scratchpad"` schema is refused (`exit 1`). Mixing scratchpad and no-scratchpad automated seats at one table is refused unless explicitly marked via `"mixed_scratchpad_arm": true` in `seats.json` or `--mixed-scratchpad-arm`.
- **`emc` (playtest toggle):** keeps the most recent `5` completed `(user, assistant)` pairs verbatim and compacts older turns into `<game-dir>/relay/seats/<Guild>.emc.md` (first `160` characters of each earlier reply under `=== YOUR EARLIER REPLIES (the first 160 characters of each, from turns before the last 5) ===`) ahead of the current turn prompt.
- **`full`:** replays the `"system"` wrapper followed by every stored `user` and `assistant` turn in order and the new `to-seat` block. Even if `--wrapper` is still passed on later turns, the `"system"` wrapper is recorded once on turn 1 and never duplicated.

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

Once `<game-dir>/relay/seats/<Guild>.jsonl` holds turn 1, `--wrapper` can be omitted on later turns; `league-relay` reuses the stored `"system"` entry and applies the game's recorded `memory_mode` (`snapshot` by default):

```sh
OPENROUTER_API_KEY=... cargo run -p league-relay -- turn \
  --game-dir /path/to/game-03 \
  --day 01 \
  --round act \
  --seats /path/to/game-03/seats.json \
  --live \
  --cap-credits 2.50
```

### Structured output (`--response-schema`), negotiation baseline/messages, and wording table

- **Optional structured output (`B4.2`, `R2-B3`):** passing `--response-schema <path>` attaches `response_format: {"type": "json_schema", "json_schema": {"name": ..., "strict": ..., "schema": ...}}` per vLLM's `/v1/chat/completions` structured-outputs specification (defaulting bare schema objects to pinned name `"seat_turn_response"`, and automatically populating or verifying the pinned `scratchpad_schema_description` on any `"scratchpad"` property). If a returned reply is invalid JSON or violates the schema's top-level type, `required`, `enum`, or `additionalProperties: false` constraints, `league-relay` logs `outcome: "harness"` and falls back to the same single `"harness"` retry before `NO_REPLY`.
- **Inter-seat messages and no-new-deals baseline (`B4.4`, `A-5`, `B-5`, `R2-A1`, `R2-B4`):** when `<game-dir>/to-seat/day-DD-<round>-<Guild>.negotiation.json` is present (`{"batna": "...", "whispers": [{"from_guild": "...", "intent_header": ..., "body": "..."}]}`), `league-relay` requires a non-empty `batna` baseline and requires every `from_guild` to match a seated guild in `seats.json` (blocking spoofed sender names), composes `=== IF YOU MAKE NO NEW DEALS THIS TURN ===` (`no_new_deals`) and `=== MESSAGES FROM OTHER SEATS ===` (`other_seats_messages`) as active ordered sections, caps `intent_header` + `body` together at `120` words and `1800` characters (charging empty strings/containers, punctuation, escapes, and nesting depth `<= 8`), and JSON-escapes quotes, control characters, Unicode bidi/line-separator controls, and ASCII/full-width `<`/`>` (`\u003c`/`\u003e`, `\uff1c`/`\uff1e`).
- **Model-facing wording table (`A-4`, `B-1`..`B-4`, `R2-A2`):** every label, section header, JSON field name, scratchpad schema key/description, default schema identifier, and user-message section ordering shown to a model by `league-relay` lives in `MODEL_FACING_WORDING` (`crates/league-relay/src/wording.rs`) under `ACTIVE_WORDING_ARM` (`"v2_neutral_2026_11_15_r3"`), rendering shown numeric bounds directly from `MAX_SCRATCHPAD_TOKENS`, `EMC_OLDER_REPLY_EXCERPT_CHARS`, and `EMC_VERBATIM_TURN_WINDOW`, and pinned byte-for-byte in tests.

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

- `to-seat/day-DD-<round>-<Guild>.md` — input block prepared for that seat and round (with optional `to-seat/day-DD-<round>-<Guild>.negotiation.json` carrying `batna` and untrusted `whispers`).
- `from-seat/day-DD-<round>-<Guild>.md` — assistant reply written **byte-for-byte** (`reply.as_bytes()`, after stripping `<think>...</think>` when the seat's roster pins `exclude_from_transcript: true`). Trailing newlines and line endings are neither added nor stripped, so the file's SHA-256 always equals `reply_sha256` in `relay/calls.jsonl`.
- `relay/setup.json` — per-game configuration record (`memory_mode`, `cost_unit`, `wording_arm`, `scratchpad`, `mixed_scratchpad_arm`, optional `response_schema_name` and `response_schema_sha256`, and `seats` including per-seat `scratchpad` and `reasoning` pins), locked across turns.
- `relay/seats/<Guild>.jsonl` — leading `"system"` JSON line followed by `(user, assistant)` JSON line pairs for that seat (plus `relay/seats/<Guild>.scratchpad.md` in `snapshot` mode or `relay/seats/<Guild>.emc.md` in `emc` mode). Updated only when a turn delivers a non-empty valid reply.
- `relay/calls.jsonl` — one JSON line per call or pre-call refusal: `time_utc`, `day`, `round`, `guild`, `attempt`, `trigger`, `model_requested`, `model_returned`, `request_body_sha256`, `reply_sha256`, `prompt_tokens`, `completion_tokens`, optional `reasoning_tokens` (reported when the server returns `usage.completion_tokens_details.reasoning_tokens` or `usage.reasoning_tokens`; omitted/`null` on non-reasoning models, on vLLM servers run without `--reasoning-parser` where `<think>` tokens stay folded into `completion_tokens`, and on HTTP errors, timeouts, or pre-call refusals), `total_tokens`, `cost_unit` (`"credits"` or `"gpu_seconds"`), `cost` (OpenRouter credits or local `wall_secs` clamped to `max_gpu_secs_per_call`, or `null`), `charged_credits` (the amount counted against `--cap-credits` or `--cap-gpu-secs`), optional `wall_secs` (raw client wall-clock duration recorded on sent `local` attempts), `latency_ms`, `outcome`, optional redacted `error`, and optional `reservation_id` (tying each sent call or reconciled `"RESERVED"` entry to its pre-call reservation).
- `relay/turn.lock` — per-game exclusive OS lock file held for the duration of `run_turn` (`pid` and `started_utc` while active, cleared and unlocked on exit) so two turns cannot run against the same `<game-dir>` concurrently.
- `relay/calls.reserved.jsonl` — active pre-call reservations (`outcome: "RESERVED"`, `charged_credits: max_cost_per_call`, tagged with a unique `reservation_id`) written atomically (`relay/calls.reserved.jsonl.tmp` + `sync_all` + rename under `relay/calls.side.lock`) before each transport call goes out and removed only after the call's final outcome has been `sync_all`-persisted to `relay/calls.jsonl` or `relay/calls.pending.jsonl`, so the normal path leaves only one line per call in `relay/calls.jsonl`; if a process dies between reserving and recording or exits `3` when neither ledger nor side file can be written (detected when its `relay/calls.resv-<run_id>.lock` kernel lock is no longer held and no recorded entry carries its `reservation_id`), the reservation stays on disk and a later run reconciles the orphaned `RESERVED` line idempotently by `reservation_id` into `relay/calls.jsonl` at `max_cost_per_call` (do not enter an exit-`3` charge into `relay/calls.jsonl` by hand, or it will be counted twice).
- `relay/calls.pending.jsonl` — durable side file written atomically under `relay/calls.side.lock` (full merged lines written to `relay/calls.pending.jsonl.tmp` with `create_new`, `flush`, and `sync_all`, then renamed over `relay/calls.pending.jsonl`) whenever appending a sent call's line to `relay/calls.jsonl` fails; included in the cap sum on the next run and migrated once into `relay/calls.jsonl` (then removed) as soon as `relay/calls.jsonl` is writable again.
- `relay/calls.side.lock` — short exclusive OS lock held only across side-file (`calls.pending.jsonl` and `calls.reserved.jsonl`) read-merge-rename and migration updates so concurrent writers never lose a line.

All automated seats (`local` or `openrouter`) in a turn run concurrently. An error, timeout, or missing input file on one seat never stops another seat from finishing. `league-relay` exits `0` when every seat succeeds (or is `file`), `2` when any seat fails to deliver a reply, `3` when a sent call's post-call line cannot be written to either `relay/calls.jsonl` or `relay/calls.pending.jsonl` (its pre-call `max_cost_per_call` / `max_gpu_secs_per_call` reservation remains held in `relay/calls.reserved.jsonl` for the next run to count, so it must not be entered into `relay/calls.jsonl` by hand), `4` when `<game-dir>/relay/turn.lock` is held by another active run or deleted/replaced mid-turn (`TurnLocked`), and `1` when CLI arguments, seat configuration, the live/local gate, or a missing/inaccessible `--game-dir` refuse the run.

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
- `backend`: `"local"`, `"openrouter"`, or `"file"` (`"local"` and `"openrouter"` cannot be mixed in the same seat file or game directory). A `"file"` seat makes no call and prints `outcome=FILE_MANUAL`.
- `base_url` (required for `"local"`): `http://` or `https://` base URL on loopback (`127.0.0.0/8`, `[::1]`, `localhost`) or an explicitly permitted `--allow-vm-host <host>` RFC1918 / IPv6 ULA address referenced by at least one `local` seat, with no URL credentials.
- `model` (required for `"local"` and `"openrouter"`): exact pinned model identifier (`provider/model` slug without `:` routing suffixes for `"openrouter"`). Floating aliases (`auto`, `latest`, `*-latest`, `*:latest`) and credential-shaped strings are rejected.
- `temperature` (required for `"local"` and `"openrouter"`): finite number in `0.0..=2.0`.
- `max_output_tokens` (required for `"local"` and `"openrouter"`): positive integer sent as `max_tokens`.
- `timeout_secs` (required for `"local"` and `"openrouter"`): positive finite wall-clock limit per call attempt in seconds; on `"local"` seats, `timeout_secs + 0.05 <= max_gpu_secs_per_call` is required.
- `max_gpu_secs_per_call` (required for `"local"`): positive finite number in GPU-seconds (`>= timeout_secs + 0.05`) reserved against `--cap-gpu-secs` before each call; each sent local attempt records raw `wall_secs`, clamps `cost` and `charged_credits` to at most `max_gpu_secs_per_call` (while failing closed with `COST_OVER_ESTIMATE` if `wall_secs > max_gpu_secs_per_call`), and reconciles orphaned `RESERVED` entries at `max_gpu_secs_per_call`.
- `max_cost_per_call` (required for `"openrouter"`): positive finite number in credits reserved against `--cap-credits` before each call.
- `reasoning` (optional for `"local"` and `"openrouter"`): `{ "effort": ..., "budget_tokens": <u32>, "chat_template_kwargs": { ... }, "exclude_from_transcript": <bool> }` (`exclude_from_transcript` is required whenever `reasoning` is present so transcript exclusion is explicitly pinned). Per vLLM's Reasoning Outputs specification (<https://docs.vllm.ai/en/stable/features/reasoning_outputs/>), `"local"` seats accept `"effort": "none|low|medium|high"` (rejecting OpenRouter-specific `"minimal|xhigh|max"`), `"budget_tokens"` (sent as `thinking_token_budget`), `"chat_template_kwargs"`, and `include_reasoning` (`!exclude_from_transcript`), whereas `"openrouter"` seats accept all seven effort levels under `reasoning: { effort, max_tokens, exclude }`.
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

How each call is reserved and charged:

- **Single-lock cap check and pre-send durable reservation:** before every call (including a `harness` retry), `league-relay` takes one lock on the call ledger and `relay/calls.side.lock`, sums `charged_credits` across `<game-dir>/relay/calls.jsonl`, any unmigrated lines in `<game-dir>/relay/calls.pending.jsonl`, and any other run's active or orphaned reservations in `<game-dir>/relay/calls.reserved.jsonl` (plus any unrecorded in-memory charge from the current run), checks `charged + reserved_in_flight + max_cost_per_call <= cap`, and atomically writes a `RESERVED` entry (`charged_credits: max_cost_per_call`) to `relay/calls.reserved.jsonl` before the transport call goes out.
- **Recorded vs charged credits:** every sent attempt records `cost` as what OpenRouter reported and `charged_credits` as `cost` when a valid `usage.cost` is present, or that seat's `max_cost_per_call` when no valid cost was recorded (including a late HTTP 200 without `usage`, an on-time empty HTTP 200 without `usage`, a client timeout before a `harness` re-send, or an orphaned `RESERVED` entry reconciled after a crash); pre-call refusals record `charged_credits: 0.0`.
- **OpenRouter usage accounting:** per OpenRouter's usage-accounting documentation (<https://openrouter.ai/docs/use-cases/usage-accounting>, read 2026-11-15), usage including cost is returned automatically on every response (`usage: {include: true}` is deprecated and has no effect), and any sent call missing `usage.cost` is charged at `max_cost_per_call`.
- **In-flight reservation and crash reconciliation:** when a call is allowed to start, that seat's `max_cost_per_call` stays in `reserved_in_flight` and on disk in `relay/calls.reserved.jsonl` (with a unique `reservation_id` and a kernel-locked `relay/calls.resv-<run_id>.lock` handle) until the call finishes and its final line (carrying that `reservation_id`) has been flushed and `sync_all`-persisted to `relay/calls.jsonl` (or atomically synced and renamed to `relay/calls.pending.jsonl`), strictly before the transient reservation is removed so neither a process crash nor an OS power cut in that transition can drop both records, while normal runs still leave strictly one line per call in `relay/calls.jsonl`. If a run dies before recording — or exits `3` because neither `relay/calls.jsonl` nor `relay/calls.pending.jsonl` could be written — the reservation stays in `relay/calls.reserved.jsonl` and the kernel releases `calls.resv-<run_id>.lock`; the next run's `CallLedger::open` checks exact `reservation_id` identity (dropping a dead reservation only when an entry in `relay/calls.jsonl` or `relay/calls.pending.jsonl` already carries its non-empty `reservation_id`, so a same-seat re-run never erases a crashed call's spend and a blocked `calls.reserved.jsonl` rewrite never re-migrates an already-recorded `RESERVED` line) and migrates any unrecorded orphan once into `relay/calls.jsonl` at `max_cost_per_call` when writable, while sweeping any unheld `calls.resv-*.lock` files. Because an exit-`3` charge is already held as a durable reservation that the next run counts at `max_cost_per_call`, never enter an exit-`3` charge into `relay/calls.jsonl` by hand or it will be counted twice.

Refusal and fail-closed outcomes:

- **Cap reached (`CAP_REACHED`):** if `charged + reserved_in_flight + max_cost_per_call` would pass `--cap-credits <n>`, the call is not sent, `outcome: "CAP_REACHED"` is appended to `relay/calls.jsonl`, and nothing is written to `from-seat/` or `relay/seats/<Guild>.jsonl`.
- **Ledger write failure (`LEDGER_ERROR`):** if appending to `relay/calls.jsonl` fails after a sent call, the call's charge stays held in memory against `--cap-credits`, no further calls start in that turn (fail closed), and `league-relay` immediately persists the held lines to `relay/calls.pending.jsonl` so an operator `Ctrl-C` or abort before the turn ends does not lose them. Every side-file write acquires a short exclusive OS lock on `relay/calls.side.lock` (so even a compromised old run and a replacement run cannot overwrite each other's side-file lines), merges any existing unmigrated `relay/calls.pending.jsonl` lines with the new held lines, writes the full result to `relay/calls.pending.jsonl.tmp` (`create_new`, `flush`, `sync_all`), and atomically renames it over `relay/calls.pending.jsonl` (leaving any existing `relay/calls.pending.jsonl` untouched on write or rename failure, and refusing to overwrite a read-only `relay/calls.pending.jsonl`). At the end of `run_turn`, `league-relay` tries once more to append every pending line to `relay/calls.jsonl` (removing `relay/calls.pending.jsonl` if `relay/calls.jsonl` has recovered). On the next run, `CallLedger::open` counts `relay/calls.pending.jsonl` toward `--cap-credits` and, once `relay/calls.jsonl` is writable again, moves those lines into `relay/calls.jsonl` once (idempotently, with no double count) and removes `relay/calls.pending.jsonl` (note: on Windows `fs::remove_file` also deletes a read-only `relay/calls.pending.jsonl` after migration, and if directory permissions block removal on any platform, 1-to-1 deduplication still prevents double counting). If neither `relay/calls.jsonl` nor `relay/calls.pending.jsonl` can be written, `league-relay` keeps the call's pre-send `max_cost_per_call` reservation on disk in `relay/calls.reserved.jsonl` (which the next run counts automatically at `max_cost_per_call`), prints the total held credits first followed by every held line — both earlier unmigrated side-file/reservation lines and the current run's lines (`seat`, `day`, `round`, `attempt`, `charged_credits`, with secrets redacted across up to `64 KiB`) — to `stderr`, and exits `3` warning the operator to restore write access and **not** enter the charge into `relay/calls.jsonl` by hand (entering it by hand would count it twice once the kept reservation reconciles).
- **Cost over estimate (`COST_OVER_ESTIMATE`):** if a returned `usage.cost` exceeds that seat's `max_cost_per_call`, `league-relay` records the returned cost in `relay/calls.jsonl` with `outcome: "COST_OVER_ESTIMATE"` and refuses any further calls in that turn (fail closed). Seats already in flight are not stopped, so the cap can be passed by their actual cost minus their estimate; the worst case per turn is the sum over in-flight seats (for example, at cap `0.02`, two concurrent seats with `max_cost_per_call: 0.01` that both return `0.05` record `0.10` before the turn stops).
- **Unknown cost (`COST_UNKNOWN`):** if a delivered HTTP 200 completion (`outcome: "OK"`) carries no `usage.cost`, or if a non-HTTP-status-error response reports token counts (`prompt_tokens` or `total_tokens`) without `usage.cost`, `league-relay` logs `cost: null` with `charged_credits: max_cost_per_call` and refuses any further calls in that turn with `outcome: "COST_UNKNOWN"` (fail closed). An empty or late HTTP 200 that carries no `usage` block at all is charged at `max_cost_per_call` and may still use its single `harness` re-send if `--cap-credits` allows.

## Secrets

`OPENROUTER_API_KEY` is read from the environment only. Passing `--api-key` (or any key flag) on the command line or placing a key field in `seats.json` is rejected without echoing the value. `ApiKey`, `LiveGate`, and `UreqTransport` redact the key in `Debug` output; error and log paths cap untrusted input at `4 KiB` (`4096` bytes), scrub the full key, any `8+` character substring of `OPENROUTER_API_KEY`, `Bearer ...`, and `sk-or-...` tokens, and only then truncate to the display limit; and the `league-relay` binary installs `install_redacting_panic_hook` at startup so panic messages written to `stderr` apply the same redaction (library callers can install it with `install_redacting_panic_hook` as well).
