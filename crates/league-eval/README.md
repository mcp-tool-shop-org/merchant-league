# `league-eval`

Merchant League Evaluation Mode harness, frozen-state branch runner, deterministic state selector, and S3a unblinded decision-scorer pipeline (see [docs/eval-harness.md](../../docs/eval-harness.md) and [docs/decision-scorer.md](../../docs/decision-scorer.md)).

## Subcommands

- **`play`:** Play up to `--turns` full turns from standard setup or turn-0 fork under `--arm <off|control|on>`, saving pre-act checkpoints and recorded act submissions.
- **`branch`:** Replay a frozen pre-act checkpoint across `--arms <off,control,on>` holding peer actions fixed.
- **`select`:** Deterministically select stratified early/mid/late pre-act checkpoints across games.
- **`build-blind-inputs`:** Scan evaluation `--run <dir>...` directories (`decision.json`, `outcome.json`, `manifest.json`, `setup.json`, `to-seat/*.negotiation.json`, `decisions/day-DD-talk-*.json`, and `relay/seats/*.tool-round-*.json`), parse committed decisions (`parse_eval_act_reply`), recover first sent valid tool drafts in `(round_index, attempt, call_index)` order using the recorded `call.call_index` field, extract deterministic `talk_rules_v2` peer trade commitments scaled to `league_seat_types::UNIT_SCALE` milli-units/milli-`CRED` (with retraction-dominant cross-source merging and verb-governed negation scoping), merge with `league-scorer` arm-independent candidates by canonical `candidate_id` with OR-merged flags, and write disjoint `--out-blind` (`BlindScoringRequest` v1 without paths, timestamps, arms, wording labels, or tool traces) and `--out-arm-map` (`ArmMapStateFile`). Optionally pass `--out-checkpoints <dir>` to populate content-addressed `.save.json` / `.seal` blobs.
- **`freeze`:** Refuse dirty git worktrees (`DIRTY_GIT_WORKTREE`) and existing target files (`FREEZE_FILE_EXISTS` via atomic `create_new`), validate blind inputs and `ScorerConfig`, compute canonical POSIX-path `scorer_tree_sha256` across `crates/league-scorer/**`, and write write-once `scorer_freeze.json` with `scorer_git_sha`, `scorer_tree_sha256`, `scorer_config_sha256`, `control_wording_arm`, `talk_extractor_version`, `talk_extractor_rules_digest`, and `blind_inputs_manifest_sha256`.
- **`join`:** Fail closed on missing freeze files (`MISSING_FREEZE_FILE`), blind-input manifest mismatches (`FREEZE_MANIFEST_MISMATCH`, verified directly against each scored file's echoed `blind_request_sha256` even when `--blind` is omitted), config hash mismatches (`FREEZE_CONFIG_MISMATCH`), or `control` wording-arm mismatches (`CONTROL_WORDING_ARM_MISMATCH` on `setup.json` / `manifest.json` / `relay/setup.json`), then join `scores/*.json` with `arm_map/` by `(state_id, candidate_id)` into descriptive-only `joined_decisions.json` and `per_arm_summary.json` tables.

## Boundary With `league-scorer`

`league-eval` depends on [`league-scorer`](../league-scorer/README.md) to generate arm-independent candidates and compute canonical digests, while `league-scorer` has zero compile-time or runtime dependency on `league-eval` or `league-relay`. Only `league-eval` handles arm labels, `arm_map/`, wording labels, tool sidecars, and whisper prose.
