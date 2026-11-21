# Merchant League Evaluation Harness

This document specifies the Evaluation Mode harness (`TableMode::Eval`) introduced across Wave 2 Card 10 (`PR 1`–`PR 4`), its governing design decisions (`D1`–`D6`), the model-run hold, persisted files, and what is recorded per game and per branched decision.

## Design Decisions (`D1`–`D6`)

| # | Decision | Status |
|---|---|---|
| `D1` | **Three arms:** `off` (`v2_neutral_2026_11_15_r6`), `control` (`v2_control_2026_11_21_r1`), and `on` (`v2_preview_2026_11_20_r2`). | Landed in Card 7/9 (`off`, `on`); `control` arrives in PR 3. |
| `D2` | **Duplicate tables:** Deal Turn-1 mandates once using OS entropy, then start every duplicate table from a copy of that table's turn-0 `.save.json` + `.seal` via `EvalHost::fork_from_checkpoint`. No new secret key, no raw seed on disk, and the engine's rule against deriving secrets from the public layout seed remains intact. | Landed in `EvalHost` (`PR 1`). |
| `D3` | **Control arm wording:** The `control` arm carries the `on` arm's (`r2`) rules text byte-for-byte; only the tool block and refusal/fixed-reply slots differ (`draft_turn_orders`, same input schema as `preview_turn_orders`, returning a fixed note with zero rule validation or simulation). | Planned (`PR 3`). |
| `D4` | **Frozen candidate-set regret inputs:** Card 10 builds **no** regret scorer (that belongs to Card 11); it records every input a frozen-set scorer needs, including `ClubBot::decide_full_turn` (`pass_a_actions`, `pass_b_actions`, `bundle`) beside the arms' submissions. | Planned (`PR 3`). |
| `D5` | **First run plan:** `20` seeds × `6` pre-act states (`1` all-`off` `4`-seat game per seed; `2` early, `2` mid, `2` late states per game), each branched three ways (`off`, `control`, `on`) = `120` states, `360` branch runs. | Run plan only; awaits Card 10 merge + GPU grant. |
| `D6` | **Table size:** `4` seats only for every Evaluation run under Card 10. | Enforced (`league-eval`, `PR 2`). |

## Hold (No Model Runs Before Merge)

**HOLD:** Card 10 builds and verifies on CPU with mock transports only. No model is loaded and nothing runs on a GPU until Card 10 merges, the GPU pre-flight (`10` states run twice in reverse order expecting byte-identical replies) passes, and the Publisher's GPU grant authorizes `D5`.

## Implemented in PR 1: `EvalHost` (`crates/league-core/src/eval/host.rs`)

`EvalHost` owns `TurnEngine` strictly for `TableMode::Eval` (`TableMode::Club` and `TableMode::Game` are refused with `INVALID_TABLE_MODE`). On initialization (`EvalHost::new`), all `32` `EvalVrf` proofs and `32` turns of Phase-2 reveal salts are pre-derived and the raw `SealedEvalSeed` and ephemeral salt root entropy are dropped in memory. Seat modules read solely through `EvalHost::observe_seat` (`SeatObservation`) and `EvalHost::public_receipts` (`PublicSweepReceipt`).

### Persisted Files (`PR 1`)

Written atomically (`*.tmp` + `flush` + `sync_all` + `rename`):

- `eval-<id>.save.json` — Public, spectator-safe `EvalSaveSnapshot` (`format_version`, `layout_seed` via `ReplayLayoutSeed`, `table_id`, `turn`, `phase`, `cancel_locked`, officers, `PublicTableSpec`, `parities`, `seats`, market/contract books, `eval_vrf_pubkey`, `public_receipts`). Never stores `SealedEvalSeed`, VRF proofs, dealt mandate commitments, mandate reveals, reveal salts, or `bot_tie_seed`.
- `eval-<id>.seal` — Private `EvalSealPayload` (`format_version`, `layout_seed`, `32` pre-derived `vrf_proofs`, serialized `TurnEngine`, `sealed_mandate_deal`, `mandate_commits`, `mandate_reveals`, `logged_turns`, in-progress turn state, `bot_tie_seed`, `live_state_digests`, `turn_salts`, and `recorded_act_submissions`). Stores zero raw `SealedEvalSeed`.
- `checkpoints/day-DD-pre-act.save.json` + `checkpoints/day-DD-pre-act.seal` — Written by `EvalHost::write_pre_act_checkpoint` at the start of every act round (`TalkAndMarket`, `!cancel_locked`, zero applied actions) before any seat's act reply is read.
- `checkpoints/day-DD-act-recorded.seal` — Written by `EvalHost::write_act_recorded_seal` after `EvalHost::apply_act_submissions` commits and reveals all seats' `EvalActSubmission`s (`market_actions`, `pass_b_actions`, `bundle`) and before `EvalHost::sweep_turn`. Loading a `day-DD-pre-act` checkpoint via `EvalHost::fork_from_checkpoint` and applying `day-DD-act-recorded.seal` via `EvalHost::apply_recorded_act_submissions_from_seal` yields a `PublicSweepReceipt` byte-identical to the original sweep.

Game and checkpoint directories hold sealed game values (mandates, salts, proofs, and act submissions) and must not be committed or published.

## Planned Commands & Recording (`PR 2`–`PR 4`)

- **`PR 2` (`league-eval`):**
  - `league-eval play --game-dir <dir> --seats <seats.json> --seed <n> --turns <t> --preview-tool <off|control|on>`
  - `league-eval branch --checkpoint <dir>/checkpoints/day-DD-pre-act --seat <guild> --arms off,control,on --out <dir>`
  - `league-eval select --games <dir>... --per-game 6 --strata 2,2,2 --selection-seed <n> --out <list.json>`
- **`PR 3` (Control Arm & Manifests):**
  - `--preview-tool control` (`v2_control_2026_11_21_r1` with `draft_turn_orders`)
  - Per-game `manifest.json` with startup Andon (refusing prefix caching on, parallel slots > 1, or missing model hash / GPU name)
  - Per-decision logging (prompt/completion tokens, wall seconds, tool calls used, parse failure, refused market actions and bundle legs, cleared non-empty actions, intermediate preview/draft bundle hashes, and `ClubBot::decide_full_turn` baseline outputs)
- **`PR 4` (Metrics & Pre-Flight):**
  - `league-eval report --branches <dir>` (JSON + text summary table, paired sign-flip test over seeds, seeded studentized cluster-bootstrap interval `B = 10,000`)
