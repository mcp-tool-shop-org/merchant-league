# `league-scorer`

Arm-blind, deterministic CPU decision-scoring kernel for Merchant League v2 evaluation (see [docs/decision-scorer.md](../../docs/decision-scorer.md)).

## What S1–S2 build

- **Compile-time blinding (`Cargo.toml`):** depends only on `league-core`, `league-seat-types`,
  `serde`, `serde_json`, and `sha2` — zero dependency on `league-eval` or `league-relay`, enforced
  by `blinding_cargo_toml_forbids_unapproved_dependencies` and `league-core`'s workspace
  `raw_api_backdoor_guard`.
- **Schema v1 validation & strict request parsing (`types.rs`):** validates `BlindScoringRequest`
  and `BlindCandidate` (`#[serde(deny_unknown_fields)]`, `parse_blind_scoring_request_strict`
  rejecting unknown/extra fields inside nested `candidate.submission` objects, sorted unique
  `candidate_id == sha256_hex(serde_json::to_vec(&submission))`, valid 64-hex digests, `focal_seat`
  in bounds, `scorer_config_sha256` match, path-safe `state_id`, and validated `talk_commitments`),
  plus `ScorerConfig` carrying every parameter and named fixed constant from `docs/decision-scorer.md`
  `§5.2` (refusing unknown `composite_greedy_rules` and `posture_weights`/`posture_schedule` ratio
  mismatches; `Default::default` is the pilot config with `[Standard, Standard, Active, Conservative]`;
  `prescreen_min_arm_indep_span` = `1` and `prescreen_min_arm_indep_candidates` = `2` are
  placeholders until arm-blind pilot calibration; `ScorerConfig::s1_standard(M)` builds an
  all-Standard config).
- **Arm-independent candidate generators & `candidates` CLI (`candidates.rs`, `cli.rs`):** builds
  `passive`, unmodified `ClubBot`, optional `pass_b_actions`-cleared `ClubBot`, and legal synthetic
  drafts across `claims_council`, `upkeep_inputs`, `cross_quotes`, `batches`, `debt`,
  `composite_drops`, and `scaled`, each verified via `preview_seat_bundle`, bounded by `k_synth_max`
  + `synth_slot_fill_order` with `candidate_id` tie-breaking, and exposed via `league-scorer
  candidates --checkpoints <dir> --save-sha256 <hex> --seal-sha256 <hex> --focal-seat <n>
  --state-id <id> --config <file> --scorer-seed <u64> --out <dir>` writing sorted
  `<out>/<state_id>.json` with empty `talk_commitments`.
- **Undealt mandate scenarios (`scenarios.rs`):** reads `spec.mandate_deck` from the checkpoint
  observation, removes one instance of the focal seat's dealt mandate, and draws `(seat_count - 1)`
  distinct peer mandates without replacement via domain-separated SHA-256 reverse Fisher-Yates
  using integer arithmetic and `wrapping_add` bot seeds.
- **Peer postures & talk-commitment compliance (`postures.rs`):** applies deterministic scenario
  postures (`Standard`, `Active`, `Conservative`) parameterized solely by `ScorerConfig` on Turn
  `day` and `(day + 1)..close`, plus canonicalized Turn-`day` talk commitments whose integer
  threshold `threshold_ppm` is derived once via `p_honour_to_ppm` before pure-integer SHA-256 draws
  and fail-closed `preview_seat_bundle` legality pruning.
- **Sequential season-close rollouts (`rollout.rs`):** re-verifies `.save.json`, `.seal`, and
  domain-separated pair SHA-256 hashes before every `EvalHost::fork_from_checkpoint`, strips
  directory components from error output (`sanitize_error_text`), advances to natural close using
  counterfactual peer postures and unmodified focal `ClubBot`, calls `finalize_eval_season`, and
  records exact integer `(total_equity, project_equity, net_asset_equity, mandate_bonus)` (also
  returning the `SeasonReplayLog` via `run_single_rollout_with_replay_log`).
- **Deterministic scoring math & CLI (`scoring.rs`, `cli.rs`):** requires even `scenarios_m >= 2`
  for disjoint `M/2` split-half Winner's curse disclosure, integer `i128` scenario sums in `0..M`
  order before one final division, `[v_min, v_max]` span over `in_span == true` candidates only,
  `null` `score_norm` with `degenerate_flat_set: true` on flat spans, paired SE against `c_best`,
  arm-independent rank and mid-rank percentile sensitivity score, `§3.2` arm-blind pre-screen
  flags, and `league-scorer score --blind-inputs <dir> --checkpoints <dir> --config <file> --out
  <dir> [--freeze <file>] [--scorer-crate-dir <dir>]` enforcing filename stem `== req.state_id`,
  rejecting case-insensitive `state_id` collisions, resolving strictly `<sha256>.save.json` /
  `<sha256>.seal` blobs, echoing `blind_request_sha256` from the exact scored request bytes, and
  writing sorted-key `<out>/scores/<state_id>.json` with zero timestamps, durations, hostnames,
  paths, or arm labels. When `--freeze` is supplied, `--scorer-crate-dir` hashes the given crate
  source tree on disk, not the running binary.

## Remaining stages (S3–S4)

- **S3:** unblinded `BlindScoringRequest` / `arm_map/` builder in `league-eval`, `scorer_freeze.json`
  pre-registration join gate, and post-unblinding mechanism measures.
- **S4:** optional deterministic parallelism across rollouts (`S1`–`S2` execute sequentially without
  `rayon`).
