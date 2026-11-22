# `league-scorer`

Arm-blind, deterministic CPU decision-scoring kernel for Merchant League v2 evaluation (see [docs/decision-scorer.md](../../docs/decision-scorer.md)).

## What S1 builds

- **Compile-time blinding (`Cargo.toml`):** depends only on `league-core`, `league-seat-types`, `serde`, `serde_json`, and `sha2` — zero dependency on `league-eval` or `league-relay`, enforced by `blinding_cargo_toml_forbids_unapproved_dependencies` and `league-core`'s workspace `raw_api_backdoor_guard`.
- **Schema v1 validation (`types.rs`):** validates `BlindScoringRequest` and `BlindCandidate` (`#[serde(deny_unknown_fields)]`, sorted unique `candidate_id == sha256_hex(serde_json::to_vec(&submission))`, valid 64-hex digests, `focal_seat` in bounds, `scorer_config_sha256` match, path-safe `state_id`), plus `ScorerConfig` carrying every parameter and pilot default from `docs/decision-scorer.md` §5.2 (`Default::default` is the documented pilot config; `ScorerConfig::s1_standard(M)` configures an all-Standard S1 run).
- **Undealt mandate scenarios (`scenarios.rs`):** reads `spec.mandate_deck` from the checkpoint observation, removes one instance of the focal seat's dealt mandate, and draws `(seat_count - 1)` distinct peer mandates without replacement via domain-separated SHA-256 reverse Fisher-Yates using integer arithmetic and `wrapping_add` bot seeds.
- **Sequential season-close rollouts (`rollout.rs`):** verifies `.save.json`, `.seal`, and domain-separated pair SHA-256 hashes before calling `EvalHost::fork_from_checkpoint`, applies each candidate with counterfactual peer `ClubBot` turn-`day` submissions, advances every subsequent turn to natural close using unmodified `ClubBot` (for both peers and focal continuation), calls `finalize_eval_season`, and records exact integer `(total_equity, project_equity, net_asset_equity, mandate_bonus)`.
- **Deterministic scoring math & CLI (`scoring.rs`, `cli.rs`):** integer `i128` scenario sums in `0..M` order before one final division, `[v_min, v_max]` span over `in_span == true` candidates only, `null` `score_norm` with `degenerate_flat_set: true` on flat spans, paired SE against `c_best`, arm-independent rank and mid-rank percentile sensitivity score, split-half Winner's curse disclosure, §3.2 arm-blind pre-screen flags, and `league-scorer score --blind-inputs <dir> --checkpoints <dir> --config <file> --out <dir>` resolving strictly `<sha256>.save.json` / `<sha256>.seal` blobs and writing sorted-key `<out>/scores/<state_id>.json` with zero timestamps, durations, hostnames, paths, or arm labels.

## What S1 does not do yet (deferred to S2–S4)

- **S2:** `ClubBot` + synthetic candidate generators (`K_synth_max`), non-Standard peer postures (`active`, `conservative`), and talk-commitment extraction/probabilistic compliance (`talk_commitments` and non-Standard postures are refused with explicit named errors in S1).
- **S3:** unblinded `BlindScoringRequest` / `arm_map/` builder in `league-eval`, `scorer_freeze.json` pre-registration join gate, and post-unblinding mechanism measures.
- **S4:** optional deterministic parallelism across rollouts (S1 executes sequentially without `rayon`).
