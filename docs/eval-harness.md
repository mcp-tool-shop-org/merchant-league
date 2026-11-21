# Merchant League Evaluation Harness

This document specifies the Evaluation Mode harness (`TableMode::Eval`) introduced across Wave 2 Card 10 (`PR 1`–`PR 4`), its governing design decisions (`D1`–`D6`), the model-run hold, persisted files, and what is recorded per game and per branched decision.

## Design Decisions (`D1`–`D6`)

| # | Decision | Status |
|---|---|---|
| `D1` | **Three arms:** `off` (`v2_neutral_2026_11_15_r6`), `control` (`v2_control_2026_11_21_r1`), and `on` (`v2_preview_2026_11_20_r2`). | Landed in Card 7/9 (`off`, `on`); `control` arrives in PR 3. |
| `D2` | **Duplicate tables:** Deal Turn-1 mandates once using OS entropy, then start every duplicate table from a copy of that table's turn-0 `.save.json` + `.seal` via `EvalHost::fork_duplicate_from_turn_0` / `EvalHost::fork_duplicate_from_game_dir`, which assigns a fresh per-game 32-hex `game_id` (`R5`) and requires turn 0 (`INVALID_TURN_0_CHECKPOINT`). No new secret key, no raw `SealedEvalSeed` on disk; the seal carries the turn-1 deal, including its `deal_seed`, as Club seals do, and the engine's rule against deriving secrets from the public layout seed remains intact. | Landed (`EvalHost` PR 1; `fork_duplicate_*` + `game_id` PR 2). |
| `D3` | **Control arm wording:** The `control` arm carries the `on` arm's (`r2`) rules text byte-for-byte; only the tool block and refusal/fixed-reply slots differ (`draft_turn_orders`, same input schema as `preview_turn_orders`, returning a fixed note with zero rule validation or simulation). | Planned (`PR 3`; `--arm control` exits `1` `CONTROL_ARM_NOT_IMPLEMENTED` in PR 2). |
| `D4` | **Frozen candidate-set regret inputs:** Card 10 builds **no** regret scorer (that belongs to Card 11); `league-eval` records every input a frozen-set scorer needs in `decisions/*.json` and `outcome.json`, including `ClubBot::decide_full_turn` (`market_actions`, `pass_b_actions`, `bundle`) beside each arm's submission and `raw_reply`. | Landed in `league-eval` (`PR 2`); `control` + serving fields in PR 3. |
| `D5` | **First run plan:** `20` seeds × `6` pre-act states (`1` all-`off` `4`-seat game per seed; `2` early, `2` mid, `2` late states per game), each branched three ways (`off`, `control`, `on`) = `120` states, `360` branch runs (`league-eval select`). | Driver landed (`PR 2`); run awaits Card 10 merge + GPU grant. |
| `D6` | **Table size:** `4` seats only for every Evaluation run under Card 10, enforced at startup (`INVALID_EVAL_SEAT_COUNT`, exit `1`, `R9`). | Enforced (`league-eval`, `PR 2`). |

## Hold (No Model Runs Before Merge)

**HOLD:** Card 10 builds and verifies on CPU with mock transports only. No model is loaded and nothing runs on a GPU until Card 10 merges, the GPU pre-flight (`10` states run twice in reverse order expecting byte-identical replies) passes, and the Publisher's GPU grant authorizes `D5`.

## Implemented in PR 1: `EvalHost` (`crates/league-core/src/eval/host.rs`)

`EvalHost` owns `TurnEngine` strictly for `TableMode::Eval` (`TableMode::Club` and `TableMode::Game` are refused with `INVALID_TABLE_MODE`). On initialization (`EvalHost::new`), all `32` `EvalVrf` proofs and `32` turns of Phase-2 reveal salts are pre-derived and the raw `SealedEvalSeed` and ephemeral salt root entropy are dropped in memory. Seat modules read solely through `EvalHost::observe_seat` (`SeatObservation`) and `EvalHost::public_receipts` (`PublicSweepReceipt`).

### Persisted Files (`PR 1` + `PR 2`)

Written atomically (`*.tmp` + `flush` + `sync_all` + `rename` in `EvalHost`; `*.tmp` + `flush` + `rename` in `league-eval`):

- `eval-<id>.save.json` — Public, spectator-safe `EvalSaveSnapshot` (`format_version`, `layout_seed` via `ReplayLayoutSeed`, `table_id`, `turn`, `phase`, `cancel_locked`, officers, `PublicTableSpec`, `parities`, `seats`, market/contract books, `eval_vrf_pubkey`, `public_receipts`). Never stores `SealedEvalSeed`, VRF proofs, dealt mandate commitments, mandate reveals, reveal salts, or `bot_tie_seed`. `league-eval` writes strictly through `EvalHost::write_game_save_and_seal` (`R8`).
- `eval-<id>.seal` — Private `EvalSealPayload` (`format_version`, `game_id`, `layout_seed`, `32` pre-derived `vrf_proofs`, serialized `TurnEngine`, `sealed_mandate_deal`, `mandate_commits`, `mandate_reveals`, `logged_turns`, in-progress turn state, `bot_tie_seed`, `live_state_digests`, `turn_salts`, `recorded_act_submissions`, and `pre_act_checkpoint_sha256`). Stores no raw `SealedEvalSeed` on disk; the seal carries the turn-1 deal, including its `deal_seed`, as Club seals do. Both `turn_reveal_salt` and `recorded_act_submissions` stay non-`pub` (`R4`), and recorded-act seals bind strictly to the active `(game_id, table_id, turn, pre_act_checkpoint_sha256)` (`R5`).
- `checkpoints/day-DD-pre-act.save.json` + `checkpoints/day-DD-pre-act.seal` — Written by `EvalHost::write_pre_act_checkpoint` inside `league-eval play` **after** `talk` / writing `negotiation.json` and **before** observing/rendering `to-seat/day-DD-act-*` or collecting any `act` reply (`R9` timing enforced; `TalkAndMarket`, `!cancel_locked`, zero applied actions).
- `checkpoints/day-DD-act-recorded.seal` — Written by `EvalHost::write_act_recorded_seal` after `EvalHost::apply_act_submissions` commits and reveals all seats' `EvalActSubmission`s (`market_actions`, `pass_b_actions`, `bundle`) and before `EvalHost::sweep_turn`. Loading a `day-DD-pre-act` checkpoint via `EvalHost::fork_from_checkpoint` and applying `day-DD-act-recorded.seal` via `EvalHost::apply_recorded_act_submissions_from_seal` / `read_recorded_act_submissions` yields a `PublicSweepReceipt` byte-identical to the original sweep.

Game and checkpoint directories hold sealed game values (mandates, salts, proofs, and act submissions) and must not be committed or published. Branch `--out` directories contain only public spectator artifacts and never receive `.seal` files or sealed secrets (`Item 15`).

## Implemented in PR 2: `league-eval` (`crates/league-eval/`)

- **`league-eval play --game-dir <dir> --seats <seats.json> --wrapper <wrapper.md> --arm <off|on> --seed <n> [--table-id <id>] [--turns <1..=32>] [--wrapper-version <ver>] [--memory-mode snapshot|emc|full] [--mock-dir <dir>] [--live --cap-credits <n> | --cap-gpu-secs <n>] [--allow-vm-host <host>]`**
  - Validates 4 seats (`INVALID_EVAL_SEAT_COUNT`, `exit 1`, `D6`), allows `"file"` backend seats only when `"model": "ClubBot"` (`UNSUPPORTED_FILE_SEAT_MODEL`, `exit 1`), locks `setup.json` (`game_id`, `wrapper_version`, `wrapper_sha256`, `eval_reply_wording_arm`, `relay_wording_arm`, `response_schema_sha256`), and writes `manifest.json` (with `PASS_B_LIMIT_STATEMENT`), `decisions/*.json`, `receipts/*.json`, and season-close `eval-<table>.replay.json`, `scoreboard.json`, and `metrics.json`.
- **`league-eval branch (--checkpoint <dir>/checkpoints/day-DD-pre-act --seat <guild|0..3> | --list <list.json>) --seats <seats.json> --wrapper <wrapper.md> --arms off,on --out <dir>`**
  - Forks independent `EvalHost` state per requested arm, splices the focal seat's decision into fixed peer submissions from `day-DD-act-recorded.seal`, restores focal pre-act transcript prefix and scratchpad snapshot, and writes public `outcome.json`, `receipt.json`, and `decision.json` under `--out`.
- **`league-eval select --games <d1,d2,...> --per-game 6 --strata 2,2,2 --selection-seed <n> --out <list.json>`**
  - Deterministically partitions each game's playable checkpoints (`day-DD-pre-act` + `day-DD-act-recorded.seal`) into early/mid/late thirds and selects stratified `(game, day, seat)` targets.

### Item 9 Reply Contract & Pinned Wording (`v2_eval_reply_2026_11_21_r1`)

- **Seat views (`to-seat/`):** Rendered in exact order (`eval_talk_instruction` / `eval_act_instruction`, blank line, `eval_state_intro`, blank line, `serde_json::to_string_pretty` `2`-space pretty JSON of `SeatObservation`, newline). Every `act` seat receives `day-DD-act-<Guild>.negotiation.json` carrying `eval_batna` plus delivered whispers.
- **Unified reply schema (`v2_eval_reply_2026_11_21_r1` + `r2`):** One schema per game (`messages` max 3, `market_actions`, `bundle`, plus `notes` on note-keeping local seats) accepted by `league-relay`'s `ResponseFormatConfig::from_json_str`.
- **Strict parsing & refusal codes:** Rejects `<think>`, markdown fences, trailing tokens, duplicate JSON keys, and wrong-part payloads without stripping: `ACT_REPLY_INVALID`, `TALK_REPLY_HAS_ORDERS`, `ACT_REPLY_HAS_MESSAGES`, `MESSAGE_RECIPIENT_INVALID` (invalid recipients drop only the offending message while valid siblings deliver). Tracks `WrongPartCounter` (`> 5%` pre-flight stop line), `refusal_counts_by_seat`, `refusal_counts_by_arm`, and whole-bundle refusals (`RefusedBundleRecord { submitted_bundle, error_code, bad_leg_index: "leg not named" }`, `R7`, incrementing `refused_bundles`).

## Planned Commands & Recording (`PR 3`–`PR 4`)

- **`PR 3` (Control Arm & Serving Manifests):**
  - `--arm control` (`v2_control_2026_11_21_r1` with `draft_turn_orders`, returning exit `1` `CONTROL_ARM_NOT_IMPLEMENTED` in PR 2)
  - Per-game `manifest.json` startup Andon & VM serving check (refusing prefix caching on, parallel slots > 1, or missing model hash / GPU name) plus prompt/completion token & latency fields
- **`PR 4` (Metrics & Pre-Flight):**
  - `league-eval report --branches <dir>` (JSON + text summary table, paired sign-flip test over seeds, seeded studentized cluster-bootstrap interval `B = 10,000`)
