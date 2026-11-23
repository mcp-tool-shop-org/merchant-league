# Evaluation Mode Decision Scorer Design

This document specifies the offline, CPU-only, deterministic decision scorer referenced as `D4` in [`docs/eval-harness.md`](eval-harness.md) for the first Evaluation Mode comparison (`D5`: three arms `off`, `control`, `on` branched at saved pre-act states).

**Scope boundary:** This is the design specification for the decision scorer. Stages S1 and S2 of the blind scoring kernel are implemented in [`crates/league-scorer`](../crates/league-scorer/README.md) (schema v1 strict validation, §5.2 `ScorerConfig`, arm-independent candidate generators and `candidates` CLI, `Standard`/`Active`/`Conservative` peer postures, `Bernoulli(p_honour)` talk-commitment compliance, per-fork save/seal/pair SHA-256 re-verification, undealt-mandate sequential rollouts, and §7.1/§3.2 scoring outputs). Stage S3a (the unblinded input builder, `talk_rules_v2` commitment extractor scaled by `UNIT_SCALE`, write-once `scorer_freeze.json` writer, fail-closed `arm_map/` join gate bound to each score's `blind_request_sha256`, and `league-scorer score --freeze` verification) and Stage S3b (post-unblinding §7.2 descriptive mechanism measures inside `league-eval join`, with single-pass prose-free counting and `null` fallbacks for missing or unparseable optional sidecars) are built in [`crates/league-eval`](../crates/league-eval/README.md) and [`crates/league-scorer`](../crates/league-scorer/README.md). Stage S4 (optional parallelism) remains a future stage. It requires **no** changes to `league-core`, to what any model or `ClubBot` seat is shown, or to any persisted Evaluation Mode file (`setup.json`, `checkpoints/`, `to-seat/`, `decisions/`, `outcome.json`, `receipt.json`, `relay/`). Every input named below is already written by `league-eval` and `EvalHost`.

---

## 1. Fixed Requirements

1. **Frozen candidate set including all committed arm decisions:** Each pre-act state is graded against a frozen set of legal act-phase submissions whose normalization span (`[v_min, v_max]`) contains passive, synthetic baselines, and every evaluated arm's actually committed decision at that state, guaranteeing non-negative regret for committed decisions and never penalizing a strong decision outside an enumerated baseline list. First drafts are evaluated on the same scenarios for revision gain, outside `[v_min, v_max]`.
2. **Horizon to season close:** Values reach terminal season scoring (`total_equity = project_equity + net_asset_equity + mandate_bonus`), avoiding circularity with the `on` arm's single-turn `preview_turn_orders` output.
3. **Counterfactual marginalization:** Scores do not depend on the single realized opponent move vector recorded in the base game's `day-DD-act-recorded.seal`. They average across multiple plausible other-seat decisions (respecting explicit talk-phase trade commitments probabilistically) and uniform undealt-mandate assignments.
4. **Deterministic CPU execution:** Pure CPU execution with every pseudo-random choice keyed deterministically from explicit recorded seeds, index-ordered reductions, integer sums before final division, and no thread-count dependence, timestamps, or paths inside score records or hashed blind requests. Identical inputs reproduce bit-identical score records.
5. **Arm-blind scoring kernel & `arm_map/` governance:** The scorer reads only `blind_inputs/` (the pre-act checkpoint hashes, focal seat index, extracted neutral talk-phase trade commitments, anonymized canonical candidates with span/independence flags, and frozen scorer seed/config hash) — zero arm labels, replicate counts, wording identifiers, tool call traces, raw reply strings, or reasoning traces. `arm_map/` is held strictly by the run operator's reporting step, separate from `league-scorer` and from anyone tuning parameters.
6. **Pre-main-run freeze, arm-blind pilot tuning, and single control arm:** Calibrated and verified on the D5 pilot (`4` seeds × `6` pre-act states = `24` states × `3` arms × `3` replicates = `216` arm decisions) using **arm-blind diagnostics only** (`paired_se_equity`, measured mid/late rollout runtime, flat-set rate, arm-independent candidate coverage, and bit-identical repeat passes). Neither any person nor any tool joins `arm_map/` to any score until `scorer_freeze.json` is written and recorded, and `scorer_freeze.json`'s SHA-256 is recorded in the first run's pre-registration before any main-run decision is scored (later implementation must include an automated test proving any `arm_map/` join without a recorded freeze file is refused). Both pilot and main runs MUST use the same `control` wording arm: the `r2` control arm that runs the same legality check as `on` (being built separately), recorded as `control_wording_arm` in `scorer_freeze.json` and checked solely by the unblinded input builder and reporting step outside `league-scorer`.
7. **Phase-invariant primary metric plus sensitivity score:** Reports a normalized `[0, 1]` score within `[v_min, v_max]` (`NA` / `null` when `v_max == v_min`, excluded upfront by the arm-blind pre-screen) alongside raw expected `total_equity`, regret, Winner's-curse disclosure, a pre-registered arm-independent rank/percentile sensitivity score, and descriptive-only post-unblinding mechanism measures.

---

## 2. Existing Code and Recorded Inputs

All path citations below are verified at `main` (`55fd92a2`):

- **Saved `day-DD-pre-act` checkpoint pair:**
  - `EvalHost::write_pre_act_checkpoint` (`crates/league-core/src/eval/host.rs:1094-1108`) writes `checkpoints/day-DD-pre-act.save.json` and `checkpoints/day-DD-pre-act.seal` atomically inside `league-eval play` right after writing `to-seat/day-DD-act-<Guild>.negotiation.json` (`crates/league-eval/src/driver.rs:2043-2054`) and before rendering any `act` view or reading any `act` reply.
  - `day-DD-pre-act.save.json` holds `EvalSaveSnapshot` (`crates/league-core/src/eval/host.rs:54-121`): `format_version`, `layout_seed`, `table_id`, `turn`, `phase` (`TurnPhase::TalkAndMarket`), `cancel_locked`, `trade_minister`, `cartel_chair`, `PublicTableSpec` (`spec`), `parities`, `seats`, resting `offers`, `checks`, `escrows`, `dual_escrows`, `pledges`, `audited_pledges`, `domains`, `council_platforms`, `charters`, `active_shocks`, `public_forecasts`, monotonic IDs (`next_offer_id`, `next_check_id`, `next_escrow_id`), `delivery_ledger`, `last_potentials`, `pending_town_import_cred_spent`, `fill_log`, `pending_wash_freezes`, `eval_vrf_pubkey`, and `public_receipts`.
  - `day-DD-pre-act.seal` holds `EvalSealPayload` (`crates/league-core/src/eval/host.rs:167-188`): the full live `TurnEngine` state, pre-derived `vrf_proofs` for all `1..=HARD_TURN_CAP` turns, `sealed_mandate_deal` (`SealedMandateDeal` with `deal_seed` and all seats' `MandateReveal`s, `crates/league-core/src/commit.rs:212-218`), `logged_turns`, `bot_tie_seed`, and pre-derived per-turn `turn_salts`.
  - `EvalHost` (`crates/league-core/src/eval/host.rs:197`) does **not** implement `Clone` (manual `Debug` only at `:217`), and `from_save_and_seal_bytes` is `pub(crate)` (`:955`). Each rollout reconstructs an independent host from disk via `EvalHost::fork_from_checkpoint` (`:1195-1213`), while `EvalHost::observe_seat` (`:512-515`) and `EvalHost::mandate_for_seat` (`:521-527`) read observations and dealt mandates without mutating state.
- **Turn resolution and season close in `league-core`:**
  - `EvalHost::apply_act_submissions(&mut self, submissions: &BTreeMap<u8, EvalActSubmission>)` (`crates/league-core/src/eval/host.rs:631-734`) applies Phase-1 Pass A `market_actions` (`EvalActSubmission` at `:126-134`), triggers `begin_cancel_lock()`, applies optional Pass B actions (empty for model seats), probes each seat's Phase-2 `SealedOrderBundle` reveal (falling back to `SealedOrderBundle::default()` and recording `refused_bundle_code` if reveal validation fails at `:716-727`), and commits/reveals all bundles simultaneously (`commit_and_reveal_all`, `:731`).
  - `EvalHost::sweep_turn(&mut self)` (`:739-778`) executes the deterministic 8-step `EvalVrf` sweep using the host's pre-derived VRF proof for that turn and returns `PublicSweepReceipt`.
  - Once `phase() == TurnPhase::AwaitingMandateReveal` (up to `HARD_TURN_CAP = 32`), `EvalHost::finalize_eval_season(&mut self)` (`:783-840`) reveals all Turn-1 mandates, calls `TurnEngine::finalize_season` (`crates/league-core/src/engine.rs:1577`) -> `compute_final_scoreboard` (`crates/league-core/src/scoring.rs:545-617`), and replays the log from Turn 1 twice (`replay_eval_season` and `replay_season_with_state_predicates` at `host.rs:811-812`). For each seat, `SeatScoreEntry::total_equity` (`scoring.rs:582-591`) equals `project_equity + net_asset_equity + mandate_bonus`, and `SeatScoreEntry::broken_promises` (`scoring.rs:595`) records audited promise breaches.
- **`ClubBot` surface and `bot_tie_seed`:**
  - `ClubBot` (`crates/league-core/src/bot.rs:40-114`) exposes only `plan_pass_a` (`:52`), `plan_pass_b_and_bundle` (`:69`), and `decide_full_turn(&self, obs: &SeatObservation, own_mandate: &PrivateMandate)` (`:102-114`). Its `bot_tie_seed` (`u64`) is used **only** for the Turn-`4n` council-vote tie-break `(seat + 1 + ((self.bot_tie_seed ^ u64::from(obs.turn)) as u8 % (n_u8 - 1))) % n_u8` (`:536`), so varying `bot_tie_seed` produces almost no strategic diversity.
- **Legality checking vs pessimistic tie-break preview:**
  - `preview_seat_bundle` (`crates/league-core/src/observation.rs:542-566`) takes a `PublicPreviewState`, obtained with `obs.to_public_preview_state()` (`crates/league-core/src/bot.rs:657`). It is used to legality-check and prune synthetic candidates.
  - Inside `preview_what_if` (`observation.rs:503-537`), `PreviewTieMode::PessimisticForSeat` (`:513-535`) evaluates one caller-supplied `HumanWhatIfRequest` across 8 tie-break beacons (`"pessimistic preview candidate set cannot be empty"` at `:532`) and is a tie-break stress test, **not** an action generator.
- **Recorded `decisions/*.json` (`D4`), `branch` outputs, and relay sidecars:**
  - `EvalDecisionRecord` (`crates/league-eval/src/records.rs:328-432`), written to `decisions/day-DD-talk-<Guild>.json` (`driver.rs:2038`), `decisions/day-DD-act-<Guild>.json` (`driver.rs:2184`), and per-arm `<out_dir>/<arm>/decision.json` + `outcome.json` (`BranchArmOutcome`, `records.rs:601-618`, `driver.rs:2936-2939`), stores `arm`, `raw_reply`, `tool_calls_used`, `intermediate_bundle_sha256s`, `refused_market_actions`, `refused_bundle_legs`, `parse_failure`, `refusal_code`, `refused_bundle`, `completion_tokens`, `reply_token_ids`, `club_bot_baseline`, etc.
  - When `--arm control` or `--arm on` ran (`docs/eval-harness.md:54`), `SavedToolRoundRecord` (`crates/league-relay/src/relay.rs:2685-2718`, consumed by `league-eval` at `driver.rs:714-767`) persists every `SavedToolRoundCall::arguments` (`relay.rs:2660`) and `reasoning` (`:2710`), while `SavedHttpReplyRecord::body` (`relay.rs:2801-2844`, `:2830`) keeps the raw HTTP JSON responses. `driver.rs:747-766` sorts tool records by `(round_index, attempt)`, skips `!tool_result_sent`, `CALL_LIMIT_REACHED`, and `MALFORMED_TOOL_CALL`, parses `call.arguments` via `parse_preview_tool_arguments`, and pushes `sha256_hex` of `parsed_req.bundle` into `intermediate_bundle_sha256s` for **every** parsed sent call, including calls whose preview/legality check returned an engine rule refusal.
- **CPU timing anchor (`0.10 s` single sample, not an upper bound):**
  - `eval_host_turn_0_fork_deals_identical_mandates_and_runs_independently_to_eval_replay` (`crates/league-core/tests/eval_host.rs:447-515`) measured **`0.10 s` (`100 ms`)** in `--release` at the test runner's resolution. That test stops at the season's natural close (`fork_b.phase() != TurnPhase::TalkAndMarket` at `:490-491`), runs **three** full-season replays in total (two inside `finalize_eval_season` at `host.rs:811-812` plus a third external `replay_eval_season` at `eval_host.rs:510`), and does **not** isolate or time per-rollout `fork_from_checkpoint` disk reads, mid-season state sizes, candidate building, or peer posture logic. Accordingly, `0.10 s` is only a preliminary timing anchor — **not** an upper bound or a guarantee. The pilot explicitly times a mid-season and a late-season fork-to-close rollout inside the actual scorer loop, re-derives the cost table (§6.2), and re-checks the ~1-day 8-core cost stop before any main-run decision is scored.

---

## 3. Frozen Candidate Set and Arm-Blind Pre-Screen

### 3.1 Candidate Set Construction `C(state)`

For each `(state_id, pre_act_checkpoint, focal_seat)`, the unblinded builder constructs a deduplicated candidate set `C(state)` from four sources before passing `BlindScoringRequest` to `crates/league-scorer`:

1. **Mandatory passive baseline (`in_span: true`, `is_arm_independent: true`):** `EvalActSubmission::default()` (empty `market_actions`, empty `pass_b_actions`, empty `SealedOrderBundle`), representing `NO_REPLY` / inaction.
2. **All evaluated arms' committed decisions (`in_span: true`, `is_arm_independent: false`):** For every `(arm, replicate)` at `state_id` (`off`, `control`, `on`), parse `decision.json`'s `raw_reply` (`parse_eval_act_reply`, or `EvalActSubmission::default()` when `no_reply == true` / whole-reply refusal) using exact submitted `market_actions` and `bundle` (`pass_b_actions` empty for model seats), and include it unconditionally. Passing it through `EvalHost::apply_act_submissions` (`eval/host.rs:631-734`) reproduces the engine's exact Phase-1 partial refusal and Phase-2 bundle reveal fallback handling.
3. **First draft per tool arm/replicate (`in_span: false` unless equal to a committed/baseline candidate, `is_arm_independent: false`):** Because `intermediate_bundle_sha256s` (`crates/league-eval/src/driver.rs:749-766`) hashes only the `bundle` field (`:762-765`) and records every parsed sent call that is neither `CALL_LIMIT_REACHED` nor `MALFORMED_TOOL_CALL` (including calls whose preview/legality check returned a rule refusal), the builder reconstructs tool records from `SavedToolRoundRecord` in `(round_index, attempt, call_index)` order (`SavedToolRoundCall::call_index` at `crates/league-relay/src/relay.rs:2651`, set from enumeration at `:7237`, matching position in `rec.calls` iterated at `driver.rs:747-749`), selects the **first** sent call that parsed via `parse_preview_tool_arguments` without `CALL_LIMIT_REACHED` or `MALFORMED_TOOL_CALL` (even if its wire tool result was an engine refusal), and constructs its full `EvalActSubmission` with both `market_actions` and `bundle` from `SavedToolRoundCall::arguments` (`crates/league-relay/src/relay.rs:2660`). First drafts are scored with identical CRN rollouts so draft-to-commit revision gain uses one estimator, **but stay outside `[v_min, v_max]`** (`in_span: false`) unless that identical submission also appears as a committed, passive, or synthetic candidate — ensuring a bad draft cannot shift any arm's normalized score.
4. **Deterministic arm-independent `ClubBot` and synthetic candidates (`in_span: true`, `is_arm_independent: true`; `1` unmodified `ClubBot` + up to `K_synth_max = 32` slots):** Generated strictly from `SeatObservation` and `own_mandate`. None of the proposed `ScorerConfig` parameters (`§5.2`), proposed `crates/league-scorer` crate, or proposed fixed constants below exists in the repository today; when built, all fixed constants and rules are pinned by `scorer_tree_sha256`:
   - `1` unmodified `ClubBot::new(bot_tie_seed).decide_full_turn(&obs, &own_mandate)` baseline outside `k_synth_max` (`bot.rs:536` affects council tie-breaks only; no `bot_tie_seed` bank is used). When the proposed `include_pass_b_cleared_club_bot == true` (`§5.2`), a `pass_b_actions`-cleared copy is included as a candidate inside `k_synth_max` matching the single-pass model interface.
   - Single-axis enumerations from `obs`: partial and full crossing of top-of-book resting `CredOffer`s (proposed `partial_crossing_fractions_bps`), parity-anchored quote postings (`quote_offsets_bps` filtered inside `parity_spread_threshold_bps`), top `SYNTH_MAX_CONVERTER_BATCHES` feasible `ConverterRunOrder` batches ranked by `parities` (proposed fixed code constant/rule, pinned by `scorer_tree_sha256`), top `SYNTH_MAX_PROJECT_BATCHES` coverable `ProjectDeliveryOrder` batches ranked by `SYNTH_PROJECT_RANK_BY_EQUITY` (`obs.spec`: descending equity, ties broken by ascending project ID — a proposed fixed code constant/rule pinned by `scorer_tree_sha256`, not a `ScorerConfig` parameter), `TownImport` deficit coverage for `upkeep_cover_turns` and stalled converter inputs (`SYNTH_CONVERTER_INPUT_DEFICIT_COVER`, proposed fixed rule), voluntary debt repayment fractions (`debt_repayment_grid_bps`), maturing `CheckCash` / `EscrowFinish` exact claims (`SYNTH_CLAIM_MATURING_EXACT`, proposed fixed rule), and election/charter self-bids on `turn % 4 == 0` / `turn % 6 == 0` (`SYNTH_COUNCIL_SELF_NOMINATION`, proposed fixed rule).
   - Composite greedy combinations and local perturbations governed by proposed `ScorerConfig` fields `composite_greedy_rules` and `quantity_scales_bps` (with leg-drop tie-breaking governed by proposed fixed constant `DROP_TIE_BREAK_BY_CANDIDATE_ID`), each checked with `preview_seat_bundle(&obs.to_public_preview_state(), &req)` (`crates/league-core/src/bot.rs:657`, `observation.rs:542-566`) to discard illegal or immediately upkeep-bankrupting drafts. When more than `k_synth_max` unique candidates in this group (non-`ClubBot` synthetics plus the optional cleared-pass-B `ClubBot` copy) pass, slots are filled strictly in proposed `synth_slot_fill_order` (`§5.2`), breaking intra-bucket ties by ascending canonical `candidate_id` up to `K_synth_max = 32`.
5. **Canonical deduplication and freezing:** Each `EvalActSubmission` is hashed by canonical compact JSON bytes (`candidate_id = sha256_hex(serde_json::to_vec(&sub))`). Identical submissions merge with logical-OR flags (`in_span = a.in_span || b.in_span`, `is_arm_independent = a.is_arm_independent || b.is_arm_independent`). `C(state)` is sorted ascending by `candidate_id` with all arm/origin metadata stripped into `BlindScoringRequest` (`§8.1`).

### 3.2 Arm-Blind Pre-Screen

Before any evaluated arm's committed decision or first draft is joined into `C(state)`, an **arm-blind pre-screen** evaluates each state strictly over its arm-independent candidate set `C_indep(state) = { c in C(state) : c.is_arm_independent == true }` (passive baseline + `ClubBot` + `<= K_synth_max` synthetics) across the `M` scenarios:

- Computes `v_min_indep`, `v_max_indep`, `span_indep = v_max_indep - v_min_indep`, `is_flat_indep = (v_max_indep == v_min_indep)`, and distinct feasible candidate count `n_feasible_indep`.
- Any state failing pre-screen thresholds (`is_flat_indep == true`, `span_indep < min_arm_indep_span`, or `n_feasible_indep < min_arm_indep_candidates`) is excluded from primary `[0, 1]` D5 hypothesis evaluation (`excluded_by_prescreen: true`) and flagged **arm-blindly** before any arm identity touches the data.
- Pre-screen thresholds live in `ScorerConfig` (`§5.2`), are calibrated on the pilot strictly from arm-blind diagnostics, and are frozen in `scorer_freeze.json` prior to main-run scoring. Any post-join state where `v_max == v_min` after adding committed decisions still receives `score_norm = NA` (`null`, §7.1).

---

## 4. Other Seats: Diversity Sources, Postures, and Talk Commitments

Instead of conditioning on the single realized peer vector in `day-DD-act-recorded.seal`, each state evaluates `M` counterfactual scenarios `m in 0..M` (pilot initial `M = 16`, shared as Common Random Numbers across every `c in C(state)`).

1. **Sources of counterfactual scenario diversity (and honest limits):**
   - `bot_tie_seed` controls **only** the council-vote tie-break (`crates/league-core/src/bot.rs:536`) and creates almost no variation across rollouts; the scorer **never** uses `bot_tie_seed` variation as a diversity source and drops any tie-seed bank.
   - Scenario spread comes from three explicit mechanisms: **(a) peer policy postures**, **(b) the counterfactual mandate draw (§5)**, and **(c) the probabilistic talk-phase deal-keeping draw**.
   - Any individual source can yield small spread on its own on particular states — for example, when no bilateral deal was proposed in `talk`, when late-turn horizons leave `ClubBot` play nearly identical across mandates, or when tight `CRED`/inventory budgets constrain posture adjustments. For this reason `M` is not permanently fixed a priori: the pilot measures observed across-scenario CRN variance and sets final `M` in `ScorerConfig` before freezing.
2. **Postures implemented as new scorer code in `crates/league-scorer`:**
   - Because `league-core`'s `ClubBot` exposes only `plan_pass_a`, `plan_pass_b_and_bundle`, and `decide_full_turn` (`crates/league-core/src/bot.rs:52-114`), posture variants are implemented inside `crates/league-scorer` (leaving `league-core` untouched).
   - For each non-focal seat `s != focal_seat`, scenario `m` deterministically selects a posture from `ScorerConfig::posture_weights` (`m % 4` mapping in `§5.2`):
     - *Standard `ClubBot`* (`m % 4 in {0, 1}`): unmodified `decide_full_turn(&obs_s, &mandate_{m,s})`.
     - *Liquidity/converter active* (`m % 4 == 2`): retains `decide_full_turn` bundle and adds deterministic crossing `PostOffer` sell actions for idle excess inventory when resting public non-self book spreads exceed `parity_spread_threshold_bps`. Any good with a resting non-self bid and no resting non-self ask (`best_ask == None`) is treated as having infinite spread (`i128::MAX`) so crossing sells still post on one-sided bid books. Idle excess subtracts `upkeep_amount * upkeep_cover_turns` for `upkeep_good`, or the raw converter-input reserve (`sum over converters of amt * max_runs_per_turn`) for raw goods, before matching bid size and pruning refused actions fail-closed via `preview_seat_bundle`.
     - *Conservative/upkeep-first* (`m % 4 == 3`): retains `decide_full_turn` while suppressing speculative resting offer posts that lock `CRED` or goods below `upkeep_cover_turns`.
   - All postures respect the `ClubBot` `pass_a_actions`, `pass_b_actions`, `bundle` interface cleanly and are parameterized strictly in `ScorerConfig` (`§5.2`).
3. **Talk-phase trade commitments and conditional compliance:**
   - **Offline closed-grammar extraction (before blinding):** A frozen deterministic extractor scans `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs::whispers`) and outgoing `decisions/day-DD-talk-<Guild>.json` (`parse_eval_talk_reply`, `crates/league-eval/src/reply.rs:577`) using versioned `talk_rules_v2` whole-clause templates (`talk_extractor_version`, `§5.2`, `§5.2.1`). Three closed template families are recognized: first-person spot-offer commitments (`commitment_templates`, optional trailing `cred`/`creds`; `"we"` / `"we'll"` is read strictly as the speaker's own guild, and an inclusive `"we"` meaning `"you and I"` is an accepted risk), whole-clause explicit retractions and first-person refusals (`retraction_templates`, optional leading `actually`), and a tiny closed neutral set (`neutral_templates`, plus empty clauses, with only `colon_space_lead_templates` permitted before `": "` lead-ins). After apostrophe-only normalization and strict `allowed_ascii_chars` checks (rejecting `"`, `?`, non-ASCII text, and all non-thousands comma numbers or non-integer numbers), any message holding an unlisted clause fails closed, yields zero commitments, and latches that `(proposer, counterparty)` pair empty for that state across remaining messages and cross-source merges. Recall is intentionally traded for zero invented commitments; per-source and total `(seen, matched, ambiguous)` message counts recorded in `BlindScoringRequest::talk_extraction_counts` and reported descriptively per arm by `join` quantify what was lost.
   - **Probabilistic compliance in rollouts:** For each `(m, s)` with an extracted commitment, the deterministic scenario PRNG draws `honour_{m,s} ~ Bernoulli(p_honour)` (`p_honour = 0.75` pilot initial in `§5.2`) using wrapping SHA-256 keying. When `true`, the corresponding legal `MarketAction` is prepended to `pass_a_actions` (pruned via `preview_seat_bundle(&obs_s.to_public_preview_state(), ...)` if unaffordable).

---

## 5. Hidden-Objective Sampling and Free Parameters

### 5.1 Mandate Prior Read From Checkpoint `spec.mandate_deck`

Rather than calling `default_graded_mandate_deck`, the scorer reads the table's actual dealt mandate deck directly from the checkpoint's specification: **`obs.spec.mandate_deck`** (`TableSpec::mandate_deck` at `crates/league-core/src/types.rs:381`, exposed on every `SeatObservation` via `PublicTableSpec::mandate_deck` at `crates/league-core/src/observation.rs:86`).

At `day-DD-pre-act`:
- The focal seat's own dealt mandate is known (`focal_mandate = *host.mandate_for_seat(Seat(focal_seat))?`, `crates/league-core/src/eval/host.rs:521-527`).
- In each scenario `m in 0..M`, the scorer forms the remaining undealt pool by taking `spec.mandate_deck` minus one instance matching `focal_mandate` (`7` mandates when `mandate_deck.len() == 8`), and draws an ordered `(seat_count - 1)`-tuple (`3` distinct mandates without replacement) uniformly via SHA-256 reverse Fisher-Yates seeded from `(scorer_seed, pre_act_pair_sha256, day, m)` under `mandate_stratification_mode`.
- **Uniform prior, not behavioural posterior:** The draw is an explicit **uniform prior over the mandates the focal seat has not been dealt** in `spec.mandate_deck`, **not** a posterior inferred from earlier-turn peer actions (`§10`).
- **Exact focal scoring:** Peer counterfactual mandates drive peer `ClubBot` play across `day..=close`, while `EvalHost::finalize_eval_season()` unseals `focal_seat`'s authentic `mandate_commits[focal_seat]` and computes `pre_mandate_nws` median (`crates/league-core/src/scoring.rs:545-617`).

### 5.2 Complete Pre-Registered Free Parameter Table (Proposed `ScorerConfig`)

None of `ScorerConfig`, `crates/league-scorer`, or the fixed code constants below exists in the repository today; all names in this subsection are proposed names for the future scorer crate, to be hashed into `scorer_config_sha256` / `scorer_tree_sha256` when built, calibrated from arm-blind pilot diagnostics only, and frozen in `scorer_freeze.json` (`§8.2`):

| Proposed Parameter (`ScorerConfig` field) | Meaning | Pilot Initial Value |
|---|---|---|
| `posture_weights` & `posture_schedule` | Peer policy posture mix and deterministic `m % 4` scenario mapping | `[2 Standard, 1 Active, 1 Conservative]` (`m % 4 in {0,1} -> Std, 2 -> Act, 3 -> Cons`) |
| `p_honour` | Probability an extracted talk commitment is prepended to peer Turn-`day` actions (converted once via `p_honour_to_ppm` to `0..=HONOUR_PPM_SCALE` for pure-integer draws) | `0.75` (`Bernoulli(0.75)`) |
| `scenarios_m` (`M`) | Number of CRN scenarios per state (sized from pilot measured SE/diversity) | `16` |
| `k_synth_max` (`K_synth_max`) | Cap on non-`ClubBot` synthetic candidates plus the optional cleared-pass-B `ClubBot` copy (`include_pass_b_cleared_club_bot`, outside the unmodified `ClubBot` baseline) | `32` |
| `include_pass_b_cleared_club_bot` | Include `ClubBot` decision with `pass_b_actions` cleared (counted inside `k_synth_max`) | `true` |
| `partial_crossing_fractions_bps` | Resting top-of-book `CredOffer` crossing fractions (`PostOffer` legs) | `[5000, 10000]` bps (`50%`, `100%`) |
| `debt_repayment_grid_bps` | Fractions of spare `CRED` above upkeep reserve offered as `debt_repayment_cred` | `[5000, 10000]` bps (`50%`, `100%`) |
| `quote_offsets_bps` | Parity-relative limit-order price offsets for synthetic `PostOffer` generation | `[-1000, -500, 500, 1000]` bps (`±5%`, `±10%`) |
| `quantity_scales_bps` | Perturbation scalings applied to feasible composite drafts | `[5000, 10000]` bps (`50%`, `100%`) |
| `composite_greedy_rules` | Sole owner of composite greedy combination & leg-drop mode (`COMPOSITE_GREEDY_RULES_DEFAULT`, with `DROP_TIE_BREAK_BY_CANDIDATE_ID` breaking risk ties only; all leg-drop risks are priced in milli-`CRED` `Units`) | `"top_pass_per_axis_plus_drop_max_upkeep"` |
| `synth_slot_fill_order` | Deterministic priority queue truncating excess feasible synthetics (incl. cleared-pass-B `ClubBot` copy) at `k_synth_max` | `["cleared_pass_b", "claims_council", "upkeep_inputs", "cross_quotes", "batches", "debt", "composite_drops", "scaled"]` |
| `upkeep_cover_turns` | Minimum upkeep turns reserved by synthetic upkeep rule and conservative posture | `2` turns |
| `parity_spread_threshold_bps` | Minimum spread vs shadow parity triggering liquidity posture `PostOffer` posts | `500` bps (`5%`) |
| `talk_extractor_version` & `talk_extractor_rules_digest` | Frozen deterministic closed-grammar talk-commitment extractor version and domain-separated SHA-256 covering canonical `TalkRulesV2Table` compact JSON (`§5.2.1`) and LF-normalized `crates/league-eval/src/talk_extract.rs` source bytes (per-message `allowed_ascii_chars` validation, thousands-comma / plain-integer number parsing, whole-clause template matching across `commitment_templates` / `retraction_templates` / `neutral_templates` / `colon_space_lead_templates`, and fail-closed cross-source pair latching); upstream reply/record/whisper readers (`reply.rs`, `records.rs`, `league-relay`) are pinned by `scorer_git_sha` and `blind_inputs_manifest_sha256` | `"talk_rules_v2"` + `"5d5225ee3cbfc466cdd553bbf61c47d34c628632b8239a88f56c35f446f718a8"` |
| `mandate_stratification_mode` | Permutation stratification scheme over undealt `spec.mandate_deck` entries | `"uniform_undealt_fy_stratified"` |
| `prescreen_min_arm_indep_span` | Minimum arm-independent `v_max_indep - v_min_indep` span to pass `§3.2` | `1` (placeholder until arm-blind pilot calibration) |
| `prescreen_exclude_flat_sets` | Exclude states with `v_max_indep == v_min_indep` in `§3.2` | `true` |
| `prescreen_min_arm_indep_candidates` | Minimum feasible candidates in `C_indep(state)` | `2` (placeholder until arm-blind pilot calibration) |
| `strata_schedule` | Checkpoint stratification across early/mid/late season thirds (`league-eval select`) | `"2_early_2_mid_2_late_20_seeds"` (`docs/eval-harness.md` `D5`) |

*Proposed fixed code constants (to be pinned by `scorer_tree_sha256` when `crates/league-scorer` is built, `§3.1`):* `SYNTH_MAX_CONVERTER_BATCHES = 4`, `SYNTH_MAX_PROJECT_BATCHES = 4`, `SYNTH_PROJECT_RANK_BY_EQUITY = true` (rank coverable project batches by descending equity, then ascending project ID), `SYNTH_CONVERTER_INPUT_DEFICIT_COVER = true`, `SYNTH_CLAIM_MATURING_EXACT = true`, `SYNTH_COUNCIL_SELF_NOMINATION = true`, `DROP_TIE_BREAK_BY_CANDIDATE_ID = true` (tie-break only when legs under `composite_greedy_rules` share equal milli-`CRED` upkeep risk), `COMPOSITE_GREEDY_RULES_DEFAULT = "top_pass_per_axis_plus_drop_max_upkeep"`, `SYNTH_CHARTER_BID_CRED = 150` (fixed charter-turn bid matching `ClubBot`, `crates/league-core/src/bot.rs:554`), `SYNTH_QUOTE_CRED_SHARE_BPS = 5_000` (50% spare `CRED` allocation per parity-anchored buy quote, scaled by `league_seat_types::UNIT_SCALE = 1_000` and `BPS_SCALE = 10_000`), `SYNTH_QUOTE_MAX_UNITS = UNIT_SCALE` (`UNIT_SCALE`, i.e., `1_000`, is one whole commodity unit in fixed-point `Units`, not `1_000` whole units; tying the cap to `UNIT_SCALE` limits each parity-anchored buy quote to `1.000` whole commodity unit regardless of fixed-point scaling), `SYNTH_QUOTE_MIN_UNITS = 100` (minimum commodity unit floor for parity-anchored quotes), `SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER = 2` (2x milli-`CRED` parity-value multiplier when pricing `upkeep_good` sell drops so every leg-drop weight compares milli-`CRED` `Units`), `SYNTH_DROP_DEFAULT_ACTION_RISK_CRED = 1` (minimum `1` milli-`CRED` risk for other market actions), `SYNTH_DROP_PROJECT_RISK_CRED = i64::MAX / 2` (milli-`CRED` sentinel prioritizing `project_deliveries` drops), `SYNTH_SCALE_SEED_PER_AXIS = 2` (seed drafts per axis for `scaled` synthesis), `SYNTH_SCALE_SEED_POOL_MAX = 6` (maximum combined seed drafts scaled per `quantity_scales_bps` entry), `HONOUR_PPM_SCALE = 1_000_000` (integer PPM denominator for `p_honour_to_ppm` and `draw_honour_commitment`), `TALK_EXTRACTOR_VERSION_V2 = "talk_rules_v2"` (frozen `talk_rules_v2` commitment extractor version), and `TALK_EXTRACTOR_RULES_DIGEST_V2 = "5d5225ee3cbfc466cdd553bbf61c47d34c628632b8239a88f56c35f446f718a8"` (domain-separated SHA-256 covering the 25-field `TalkRulesV2Table` compact JSON in `§5.2.1` plus LF-normalized `crates/league-eval/src/talk_extract.rs` source bytes; upstream reply/record/negotiation readers in `reply.rs`, `records.rs`, and `league-relay` are pinned by `scorer_git_sha` and `blind_inputs_manifest_sha256`). The council-election and charter-auction turn cadences (`obs.turn.is_multiple_of(4)`, `obs.turn.is_multiple_of(6)`, `crates/league-scorer/src/candidates.rs:127, 152`) and charter-role list (`ClearinghouseBroker`, `OracleAuditor`, `CustomsInspector`, `candidates.rs:153-157`) mirror `league-core`'s engine rules (`crates/league-core/src/engine.rs:3670, 3844-3849`). **Build requirement:** when `crates/league-scorer` is built, it must contain no unlisted free value outside the proposed `ScorerConfig` fields and these proposed fixed constants.

#### 5.2.1 Frozen Talk Extractor Rule Table (`talk_rules_v2`)

`talk_rules_v2` is a **closed grammar** parameterized entirely by the 25 fields in `TalkRulesV2Table::frozen()` (`crates/league-eval/src/talk_extract.rs`), whose compact JSON + LF-normalized `talk_extract.rs` source bytes are pinned by `TALK_EXTRACTOR_RULES_DIGEST_V2`. Upstream reply parsing (`crates/league-eval/src/reply.rs`), decision records (`crates/league-eval/src/records.rs`), and `NegotiationInputs` (`league-relay`) are pinned by `scorer_git_sha` and `blind_inputs_manifest_sha256`.

- **Design tradeoff (recall vs. zero invented commitments):** Free-text talk messages outside this closed grammar are never guessed from partial keywords or loose negation windows. A message contributes commitments only if (1) after `quote_normalizations`, every byte lies in `allowed_ascii_chars` (which excludes `"` and `?`, as well as curly quotes and non-ASCII text), and (2) **every** non-empty clause's complete token sequence matches one listed template (fillers included, with `": "` lead-in breaks limited strictly to `colon_space_lead_templates`). Any disallowed byte, invalid number format, disallowed `": "` lead-in, or unmatched/extra token makes the whole message ambiguous, cancels all prior and subsequent commitments for that `(proposer, counterparty)` pair in that state across remaining messages and cross-source merges (`latch_ambiguous`), and increments `ambiguous` in `BlindScoringRequest::talk_extraction_counts` (tracked per source and in total as `seen = matched + ambiguous` so downstream summaries show how much recall was lost).
- **Template kinds and fillers:** Templates in `commitment_templates`, `retraction_templates`, `neutral_templates`, and `colon_space_lead_templates` match full clause token slices using exact lowercase literals plus slots `<SIDE>` (`sell_tokens` / `buy_tokens`), `<AMOUNT>` / `<PRICE>`, `<GOOD>` (`raw`/`refined`/`ref` + `compound_good_separators` + `seat_index` or guild name, plus optional `<GUILD> raw`/`refined` when `allow_guild_then_prefix_goods` is `true`), `<PRICE_CUE>` (`price_cues`), `<NEG>` (`negation_prefixes`), and listed optional fillers `<CRED?>` (`cred_tokens`: `cred`/`creds`) and `retraction_optional_lead_tokens` (`actually`). Empty clauses (outside `": "` breaks) and `neutral_templates` neither propose nor retract. `"we"` / `"we'll"` in first-person templates reads strictly as the speaker's own guild (`§4`).
- **Character and number rules:** `quote_normalizations` maps four typographic apostrophes (`U+2018`, `U+2019`, `U+02BC`, `U+FF07`) to `'`, `standalone_tokens` (`["@"]`) splits standalone symbol tokens even when adjacent to alphanumerics, and `allowed_ascii_chars` rejects every unlisted ASCII or non-ASCII byte (including `"` and `?`). Digit commas stay attached to adjacent digits in tokenization and are accepted **only** when `allow_thousands_comma_groups` is `true` and the token forms exact thousands groups (`1–3` non-zero-leading digits, comma, then one or more `3`-digit groups such as `1,000` or `12,500`); decimals (`1.5`), underscores (`1_000`), malformed commas (`10,5`, `5,5`, `1,00`, `1000,`, `,1000`, `1,,000`), and overflow fail closed.

- **`allow_guild_then_prefix_goods`:** When `true`, two-token `<GUILD> raw` / `<GUILD> refined` forms resolve to `AssetId::Raw(i)` / `AssetId::Refined(i)` using named guild tokens only (`true`).
- **`allow_thousands_comma_groups`:** When `true`, strictly grouped `1,000`-style comma thousands integers are parsed while all other comma-number forms fail closed (`true`).
- **`allowed_ascii_chars`:** Complete ASCII byte allowlist permitted after `quote_normalizations` (excludes `"` and `?`); any byte outside it fails the message closed and latches its `(proposer, counterparty)` pair empty (`" \t\n\r!',.:;@0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz"`).
- **`buy_tokens`:** Lowercase tokens matching `<SIDE>` as `OfferSide::Buy` (`["buy"]`).
- **`clause_break_chars`:** Single-byte ASCII delimiters separating clauses (`["\n", "\r", ";", "!", "."]`).
- **`colon_space_breaks_clause`:** When `true`, `": "` terminates a clause while keeping compact `refined:0` intact (`true`).
- **`colon_space_lead_templates`:** Closed list of neutral lead-in templates permitted immediately before a `": "` break (`[["hi"], ["hello"], ["greetings"]]`); any other clause before `": "` (including `ok`, `okay`, `noted`, `understood`, `thanks`, `thank you`, or a retraction before an offer) makes the message ambiguous.
- **`commitment_templates`:** Closed list of 12 first-person spot-offer templates (`i/we will|'ll`, `i/we commit|offer|agree to`, `i will post`, `i'll post` + `<SIDE> <AMOUNT> <GOOD> <PRICE_CUE> <PRICE> <CRED?>`).
- **`compound_good_separators`:** Single leading separator prefixes stripped between a good prefix and seat reference in compound tokens (`[":", "_"]`, supporting e.g. `raw:0` and `raw_1`).
- **`contradiction_latches_pair_asset`:** When `true`, disagreeing active proposals for the same `(proposer, counterparty, asset)` latch to `Contradicted` and are dropped (`true`).
- **`cred_tokens`:** Listed optional currency unit filler tokens matched by `<CRED?>` (`["cred", "creds"]`).
- **`decimal_dot_keeps_clause`:** When `true`, `.` followed immediately by an ASCII digit stays inside the clause so decimals fail closed as malformed numbers (`true`).
- **`negation_prefixes`:** Multi- or single-token negation prefixes expanded by `<NEG>` inside first-person refusal templates (`[["won't"], ["will", "not"], ["cannot"], ["can't"], ["don't"], ["do", "not"]]`).
- **`neutral_templates`:** Closed list of non-commitment neutral clause templates (`hi`, `hello`, `greetings`, `thanks`, `thank you`, `ok`, `okay`, `noted`, `understood`) that neither propose nor retract (`[[...]]`).
- **`number_unit_scale`:** Fixed-point multiplier applied to positive whole numbers and exact thousands groups (`league_seat_types::UNIT_SCALE = 1_000`).
- **`price_cues`:** Tokens matching `<PRICE_CUE>` between good specification and unit limit price (`["@", "at"]`; `"for"` is excluded so total-price phrasing never extracts as a unit limit price).
- **`quote_normalizations`:** Typographic single-quote `(from, to)` normalizations applied before whole-message validation (`U+2018 -> '`, `U+2019 -> '`, `U+02BC -> '`, `U+FF07 -> '`; curly double quotes remain disallowed).
- **`raw_prefixes`:** Token prefixes identifying `AssetId::Raw(i)` (`["raw"]`).
- **`refined_prefixes`:** Token prefixes identifying `AssetId::Refined(i)` (`["refined", "ref"]`).
- **`retraction_optional_lead_tokens`:** Optional single leading filler token allowed before any `retraction_templates` pattern (`["actually"]`).
- **`retraction_templates`:** Closed list of explicit global/asset-scoped retraction templates and first-person `<NEG>` refusal templates matching full clauses (asset-scoped when `<GOOD>` is present).
- **`sell_tokens`:** Lowercase tokens matching `<SIDE>` as `OfferSide::Sell` (`["sell"]`).
- **`standalone_tokens`:** Symbol tokens emitted as standalone tokens during clause tokenization even when adjacent to alphanumerics (`["@"]`).
- **`token_join_chars`:** Non-alphanumeric ASCII characters preserved inside compound tokens (`["_", "'", ":", "."]`).
- **`version`:** Version identifier matched against `ScorerConfig::talk_extractor_version` (`"talk_rules_v2"`).

<!-- talk_rules_v2_table:start -->
```json
{
  "allow_guild_then_prefix_goods": true,
  "allow_thousands_comma_groups": true,
  "allowed_ascii_chars": " \t\n\r!',.:;@0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz",
  "buy_tokens": [
    "buy"
  ],
  "clause_break_chars": [
    "\n",
    "\r",
    ";",
    "!",
    "."
  ],
  "colon_space_breaks_clause": true,
  "colon_space_lead_templates": [
    [
      "hi"
    ],
    [
      "hello"
    ],
    [
      "greetings"
    ]
  ],
  "commitment_templates": [
    [
      "i",
      "will",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i'll",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "will",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we'll",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "commit",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "commit",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "offer",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "offer",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "agree",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "agree",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "will",
      "post",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i'll",
      "post",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ]
  ],
  "compound_good_separators": [
    ":",
    "_"
  ],
  "contradiction_latches_pair_asset": true,
  "cred_tokens": [
    "cred",
    "creds"
  ],
  "decimal_dot_keeps_clause": true,
  "negation_prefixes": [
    [
      "won't"
    ],
    [
      "will",
      "not"
    ],
    [
      "cannot"
    ],
    [
      "can't"
    ],
    [
      "don't"
    ],
    [
      "do",
      "not"
    ]
  ],
  "neutral_templates": [
    [
      "hi"
    ],
    [
      "hello"
    ],
    [
      "greetings"
    ],
    [
      "thanks"
    ],
    [
      "thank",
      "you"
    ],
    [
      "ok"
    ],
    [
      "okay"
    ],
    [
      "noted"
    ],
    [
      "understood"
    ]
  ],
  "number_unit_scale": 1000,
  "price_cues": [
    "@",
    "at"
  ],
  "quote_normalizations": [
    [
      "‘",
      "'"
    ],
    [
      "’",
      "'"
    ],
    [
      "ʼ",
      "'"
    ],
    [
      "＇",
      "'"
    ]
  ],
  "raw_prefixes": [
    "raw"
  ],
  "refined_prefixes": [
    "refined",
    "ref"
  ],
  "retraction_optional_lead_tokens": [
    "actually"
  ],
  "retraction_templates": [
    [
      "cancel"
    ],
    [
      "cancel",
      "that"
    ],
    [
      "cancel",
      "all"
    ],
    [
      "cancel",
      "all",
      "offers"
    ],
    [
      "cancel",
      "my",
      "offer"
    ],
    [
      "cancel",
      "that",
      "no",
      "deal"
    ],
    [
      "i",
      "cancel"
    ],
    [
      "we",
      "cancel"
    ],
    [
      "i",
      "cancel",
      "that"
    ],
    [
      "i",
      "cancel",
      "all",
      "offers"
    ],
    [
      "retract"
    ],
    [
      "retract",
      "that"
    ],
    [
      "retract",
      "all"
    ],
    [
      "retract",
      "all",
      "offers"
    ],
    [
      "retract",
      "my",
      "offer"
    ],
    [
      "i",
      "retract"
    ],
    [
      "i",
      "retract",
      "that"
    ],
    [
      "i",
      "retract",
      "my",
      "offer"
    ],
    [
      "we",
      "retract"
    ],
    [
      "withdraw"
    ],
    [
      "withdraw",
      "that"
    ],
    [
      "withdraw",
      "my",
      "offer"
    ],
    [
      "i",
      "withdraw"
    ],
    [
      "we",
      "withdraw"
    ],
    [
      "never",
      "mind"
    ],
    [
      "nevermind"
    ],
    [
      "call",
      "off"
    ],
    [
      "call",
      "that",
      "off"
    ],
    [
      "no",
      "deal"
    ],
    [
      "rescind"
    ],
    [
      "walk",
      "back"
    ],
    [
      "cancel",
      "<GOOD>"
    ],
    [
      "cancel",
      "on",
      "<GOOD>"
    ],
    [
      "cancel",
      "for",
      "<GOOD>"
    ],
    [
      "cancel",
      "my",
      "offer",
      "on",
      "<GOOD>"
    ],
    [
      "i",
      "cancel",
      "<GOOD>"
    ],
    [
      "i",
      "cancel",
      "my",
      "offer",
      "on",
      "<GOOD>"
    ],
    [
      "retract",
      "<GOOD>"
    ],
    [
      "retract",
      "on",
      "<GOOD>"
    ],
    [
      "retract",
      "for",
      "<GOOD>"
    ],
    [
      "retract",
      "my",
      "offer",
      "on",
      "<GOOD>"
    ],
    [
      "i",
      "retract",
      "<GOOD>"
    ],
    [
      "i",
      "retract",
      "my",
      "offer",
      "on",
      "<GOOD>"
    ],
    [
      "we",
      "retract",
      "<GOOD>"
    ],
    [
      "withdraw",
      "<GOOD>"
    ],
    [
      "withdraw",
      "my",
      "offer",
      "on",
      "<GOOD>"
    ],
    [
      "i",
      "withdraw",
      "<GOOD>"
    ],
    [
      "i",
      "withdraw",
      "my",
      "offer",
      "on",
      "<GOOD>"
    ],
    [
      "no",
      "deal",
      "on",
      "<GOOD>"
    ],
    [
      "no",
      "deal",
      "for",
      "<GOOD>"
    ],
    [
      "rescind",
      "<GOOD>"
    ],
    [
      "walk",
      "back",
      "<GOOD>"
    ],
    [
      "i",
      "<NEG>",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "<NEG>",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "<NEG>",
      "commit",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "<NEG>",
      "commit",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "<NEG>",
      "offer",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "we",
      "<NEG>",
      "offer",
      "to",
      "<SIDE>",
      "<AMOUNT>",
      "<GOOD>",
      "<PRICE_CUE>",
      "<PRICE>",
      "<CRED?>"
    ],
    [
      "i",
      "<NEG>",
      "<SIDE>",
      "<GOOD>"
    ],
    [
      "we",
      "<NEG>",
      "<SIDE>",
      "<GOOD>"
    ],
    [
      "i",
      "<NEG>",
      "commit",
      "to",
      "<SIDE>",
      "<GOOD>"
    ],
    [
      "we",
      "<NEG>",
      "commit",
      "to",
      "<SIDE>",
      "<GOOD>"
    ],
    [
      "i",
      "<NEG>",
      "offer",
      "to",
      "<SIDE>",
      "<GOOD>"
    ],
    [
      "we",
      "<NEG>",
      "offer",
      "to",
      "<SIDE>",
      "<GOOD>"
    ],
    [
      "i",
      "<NEG>",
      "trade",
      "<GOOD>"
    ],
    [
      "we",
      "<NEG>",
      "trade",
      "<GOOD>"
    ],
    [
      "i",
      "<NEG>",
      "deal",
      "on",
      "<GOOD>"
    ],
    [
      "we",
      "<NEG>",
      "deal",
      "on",
      "<GOOD>"
    ],
    [
      "i",
      "<NEG>",
      "<SIDE>"
    ],
    [
      "we",
      "<NEG>",
      "<SIDE>"
    ],
    [
      "i",
      "<NEG>",
      "commit"
    ],
    [
      "we",
      "<NEG>",
      "commit"
    ],
    [
      "i",
      "<NEG>",
      "trade"
    ],
    [
      "we",
      "<NEG>",
      "trade"
    ],
    [
      "i",
      "<NEG>",
      "deal"
    ],
    [
      "we",
      "<NEG>",
      "deal"
    ]
  ],
  "sell_tokens": [
    "sell"
  ],
  "standalone_tokens": [
    "@"
  ],
  "token_join_chars": [
    "_",
    "'",
    ":",
    "."
  ],
  "version": "talk_rules_v2"
}
```
<!-- talk_rules_v2_table:end -->

---

## 6. Horizon Valuation, Determinism, and CPU Cost Estimates

### 6.1 Season-Close Rollout Procedure and Determinism Rules

For each scenario `m in 0..M` and each candidate `c in C(state)`:

1. **Reconstruct an independent host from checkpoint files (no `Clone`):** Because `EvalHost` does not implement `Clone` (`crates/league-core/src/eval/host.rs:197`, manual `Debug` only at `:217`) and `from_save_and_seal_bytes` is `pub(crate)` (`:955`), each `(c, m)` rollout calls `EvalHost::fork_from_checkpoint(save_path, seal_path)` (`host.rs:1195-1213`) directly on the immutable checkpoint pair whose contents were verified against `BlindScoringRequest::{pre_act_save_sha256, pre_act_seal_sha256, pre_act_pair_sha256}`. The fork I/O and state construction cost per rollout is therefore part of §6.2's runtime budget.
2. **Apply turn-`day` submissions:** Insert `c` at `focal_seat` and scenario `m`'s counterfactual peer submissions (`s != focal_seat`, computed deterministically once per `(state, m)`) into `BTreeMap<u8, EvalActSubmission>`. Call `host.apply_act_submissions(&subs)` (`:631-734`) followed by `host.sweep_turn()` (`:739-778`).
3. **Play turns `(day + 1)..` to natural close (`HARD_TURN_CAP = 32`):** While `host.phase() == TurnPhase::TalkAndMarket`:
   - Observe every seat (`host.observe_seat(Seat(s))`, `:512-515`).
   - Run peer posture builders (`§4`) for peers (`mandate_{m,s}`) and unmodified `ClubBot` for `focal_seat` (`focal_mandate`, `§10`), with seeds computed via domain-separated SHA-256 and explicit `wrapping_add` (`bot_seed.wrapping_add(u64::from(offset))`).
   - Call `apply_act_submissions` and `sweep_turn()`.
4. **Finalize and collect integer outcomes:** Call `host.finalize_eval_season()` (`:783-840`), read `scoreboard.standings` entry for `focal_seat`, and store exact integer outputs `(total_equity, project_equity, net_asset_equity, mandate_bonus)` in an indexed slot `(state_idx, c_idx, m)` without any intermediate floating-point arithmetic.

**Strict determinism constraints:**
- **No parallel floating-point reduction:** Any parallel worker pool executes independent `(state, c, m)` rollouts into pre-allocated index slots (`results[c_idx][m]`). Reductions iterate strictly in ascending index order (`m = 0..M`, `c = 0..|C|`).
- **Integer sums, single final division:** Mean equity sums `total_equity` as `i128` across `m = 0..M` in fixed `0..M` order and performs **one** final division `sum as f64 / (M as f64)`.
- **No wall-clock fields or filesystem paths:** Score output records omit timestamps, execution durations, hostnames, and paths. `BlindScoringRequest` hashes carry content SHA-256 digests (`pre_act_save_sha256`, `pre_act_seal_sha256`, `pre_act_pair_sha256`), never filesystem paths.
- **Parallelism & `rayon` dependency note:** `rayon` is **not** in `Cargo.lock` today. Keeping parallel rollout execution in `crates/league-scorer` requires adding `rayon` as one new workspace dependency in `Cargo.toml`/`Cargo.lock` (only inside `crates/league-scorer`), and automated tests must verify that outputs are bit-identical across thread counts (`1` thread vs `8` threads).

### 6.2 Candidate Counts, Recomputed Cost Table, and Pilot Re-Derivation

For `K_synth_max` synthetic candidates, `arms = 3` (`off`, `control`, `on`), and `tool_arms = 2` (`control`, `on`), including the mandatory passive candidate (`1`) and the unmodified `ClubBot` baseline (`1`), the maximum unique candidate count `K` per state is:

$$K \le 1\text{ (passive)} + 1\text{ (ClubBot)} + K_{\text{synth\_max}} + (\text{arms} \times \text{replicates})\text{ committed} + (\text{tool\_arms} \times \text{replicates})\text{ first drafts}$$

With `K_synth_max = 32`, `arms = 3`, `tool_arms = 2`, and `M = 16`:
- `replicates = 2`: `K = 1 + 1 + 32 + (3 × 2) + (2 × 2) = 1 + 1 + 32 + 6 + 4 = 44` candidates/state; `K × M = 44 × 16 = 704` rollouts/state.
- `replicates = 3`: `K = 1 + 1 + 32 + (3 × 3) + (2 × 3) = 1 + 1 + 32 + 9 + 6 = 49` candidates/state; `K × M = 49 × 16 = 784` rollouts/state.
- The D5 pilot (`4` seeds × `6` states = `24` states, `3` arms × `3` replicates) produces `24 × 3 × 3 = 216` arm decisions (plus up to `24 × 2 × 3 = 144` first drafts).

Anchored at the preliminary **`0.10 s` (`100 ms`)** single-core sample from `eval_host_turn_0_...` (`crates/league-core/tests/eval_host.rs:447-515`), exact arithmetic gives:

| Scope | States | Formula (`states × K × M`) | Max Rollouts | 1-Core Time (`rollouts × 0.10 s`) | 8-Core Wall Time (`1-core / 8`) |
|---|---|---|---|---|---|
| **Single State (`rep = 2`)** | `1` | `1 × 44 × 16` (`44 = 1+1+32+6+4`) | `704` | `70.4 s` (`1.17 CPU-min`) | `8.8 s` |
| **Single State (`rep = 3`)** | `1` | `1 × 49 × 16` (`49 = 1+1+32+9+6`) | `784` | `78.4 s` (`1.31 CPU-min`) | `9.8 s` |
| **D5 Pilot (`rep = 3`, `216` arm decisions)** | `24` (`4×6`) | `24 × 49 × 16` | `18,816` | `1,881.6 s` (`31.36 CPU-min`) | `235.2 s` (`3.92 min`) |
| **D5 Main Run (`rep = 2`, `720` arm decisions)** | `120` (`20×6`) | `120 × 44 × 16` | `84,480` | `8,448.0 s` (`140.80 CPU-min` / `2.35 CPU-h`) | `1,056.0 s` (`17.60 min`) |
| **D5 Main Run (`rep = 3`, `1,080` arm decisions)** | `120` (`20×6`) | `120 × 49 × 16` | `94,080` | `9,408.0 s` (`156.80 CPU-min` / `2.61 CPU-h`) | `1,176.0 s` (`19.60 min`) |

*Anchor caveats, mandatory pilot re-derivation, and post-pilot cost stop:* As documented in `§2`, the `0.10 s` figure is **not** an upper bound: the timed test stops at natural close (`crates/league-core/tests/eval_host.rs:490-491`), runs three full-season replays in total (two inside `finalize_eval_season` at `host.rs:811-812` plus a third external `replay_eval_season` at `eval_host.rs:510`), omits per-rollout `EvalHost::fork_from_checkpoint` (`host.rs:1195`) disk/JSON cost (`18,816–94,080` forks, one per rollout), mid-season checkpoint state size, candidate building, and peer posture computation, and is one sample at the test runner's resolution. At `0.10 s`, main-run 8-core wall time (`17.60 min` for `rep = 2`; `19.60 min` for `rep = 3`) is under one day. **Before freezing `scorer_freeze.json`, the pilot times a mid-season checkpoint and a late-season checkpoint fork-to-close rollout in `--release` with candidate building and postures active, re-derives this cost table from those measured timings, and re-applies the one-day wall-time limit on 8 cores: if the re-derived main run crosses ~1 day on 8 cores, scoring stops for a decision before freezing or scoring any main-run decision.**

---

## 7. Outputs, Scores, Sensitivity Score, and Mechanism Diagnostics

### 7.1 Blind Scorer Outputs (Per `state_id` and Per `candidate_id`)

For every candidate `c in C(state)` across scenarios `m in 0..M` (stored without wall-clock fields or paths):

- `ev_total_equity`: integer-summed mean `total_equity(focal_seat)` over `m = 0..M` (computed as exact `i128` sum divided once by `M as f64`, plus mean `project_equity`, `net_asset_equity`, `mandate_bonus`).
- **Committed-plus-baseline span (`v_min`, `v_max`):** minimum and maximum `ev_total_equity` computed strictly across **span-eligible candidates** (`in_span == true`: passive + synthetic + every committed arm decision). First drafts (`in_span == false`) are evaluated with identical CRN rollouts so revision gain uses the same estimator, **without** ever entering `[v_min, v_max]`. Because every committed arm decision satisfies `in_span == true`, committed `regret_equity = v_max - ev_total_equity(c) >= 0.0` always holds.
- **Primary normalized phase-invariant score (`score_norm`):**
  - If `v_max > v_min`: `(ev_total_equity(c) - v_min) / (v_max - v_min)` (inside `[0.0, 1.0]` for `in_span == true` candidates).
  - If `v_max == v_min`: **`NA` (`null` in JSON, never `1.0`)**, with `degenerate_flat_set: true`. States whose arm-independent subset is already flat are excluded upfront by `§3.2`; any post-join flat state still emits `null` rather than `1.0`.
- **Upward bias disclosure & split-half measurement (Winner's curse):** Because `v_max` takes the maximum over up to `K` noisy `M`-scenario estimates, `v_max` is biased upward (`E[max_c EV_hat(c)] >= max_c E[EV_hat(c)]`), pulling `regret_equity` upward and compressing mid-pack `score_norm`. Always require `scenarios_m` (`M`) to be **even (`>= 2`)** so the split-half estimator partitions scenarios into two equal disjoint halves (`H_A = 0..(M/2)` and `H_B = (M/2)..M`), and measure and report this upward bias via **split-half (held-out scenario) re-estimation**: selection uses one half (`c*_A = argmax_{c in span} EV_{H_A}(c)`) and evaluation uses the other half (`EV_{H_B}(c*_A)`), defining the bias estimate as the mean over both directions of `EV_{H_A}(c*_A) - EV_{H_B}(c*_A)` and `EV_{H_B}(c*_B) - EV_{H_A}(c*_B)` (`c*_B = argmax_{c in span} EV_{H_B}(c)`). State that this estimates selection bias at `M/2` scenarios, which is an upper estimate for `M` scenarios, and report it alongside raw `ev_total_equity` and `sensitivity_indep_percentile`.
- **Pre-registered second sensitivity score (`sensitivity_indep_percentile`, `sensitivity_indep_rank`):** Rank and percentile of `ev_total_equity(c)` among the **arm-independent candidates only** (`is_arm_independent == true`: passive + `ClubBot` + synthetics), insensitive to whether another arm emitted an extreme candidate.
- **CRN paired Monte Carlo standard error (`paired_se_equity`):** sample standard error across `m in 0..M` of paired per-scenario differences against `c_best` (`sum_m` integer differences ordered `0..M`).
- **Talk-extractor coverage diagnostics (`talk_extraction_counts`, `talk_counts`):** Every `StateScoreOutput` echoes `talk_extraction_counts` (`negotiation`, `talk`, and `total` counts of `{ seen, matched, ambiguous }` peer messages targeting `focal_seat`), `JoinedDecisionRow` preserves `talk_extraction_counts`, and `PerArmSummaryRow::talk_counts` reports descriptive per-arm totals across distinct states (`n_states`, `totals`) alongside per-state `min`/`max`/`mean`/`median` distributions (`per_state_seen`, `per_state_matched`, `per_state_ambiguous`). In each source, `seen` counts only non-self valid-seat peer messages targeting `focal_seat`, leaving out refused talk replies (`no_reply` or `refusal_code.is_some()`) and whispers from unknown guild names. Each counted message is classified independently into `matched` or `ambiguous`, so clean messages arriving on a `(proposer, counterparty)` pair after an earlier message already latched that pair empty still count as `matched` while leaving the pair latched empty (`seen == matched + ambiguous`). No gating or normalization check branches on these counts.
- **Descriptive-only pairwise contrasts:** Post-unblinding pairwise arm contrasts (`EV(on) - EV(off)`, `EV(on) - EV(control)`, `EV(control) - EV(off)`) and paired per-scenario SEs reported from unblinded tables are **descriptive only**. The pre-registered inferential analysis is the **mixed model** and the **cluster sign-flip test over seed means**, which are not defined in this document.

### 7.2 Unblinded D5 Pre-Registration Mechanism Measures

Joined post-scoring by `(state_id, arm, replicate)` inside `league-eval`'s unblinded reporting step (`join`, `crates/league-eval/src/freeze_gate.rs:625-890`, calling `compute_decision_mechanisms` at `crates/league-eval/src/mechanisms.rs:1165` and `summarize_arm_mechanisms` at `crates/league-eval/src/mechanisms.rs:1359`) **only after** `scorer_freeze.json` verification passes (`§8.2`), using the reporting step's own inputs (`arm_map/<state_id>.json`, `scores/<state_id>.json`, each entry's unblinded `decision.json`, `outcome.json`, sibling `relay/seats/` + `relay/calls.jsonl`, and optional `--source-game <dir>` directories holding `setup.json`, `checkpoints/day-DD-pre-act.save.json`, `decisions/day-DD-talk-<Guild>.json`, `to-seat/day-DD-act-<Guild>.negotiation.json`, and optional terminal `scoreboard.json`). No §7.2 measure alters `league-core`, `league-seat-types`, `league-relay`, `crates/league-scorer`, any `ScorerConfig` default, or any recorded Evaluation file format. Every §7.2 field is marked `inferential_status: "descriptive_only"`, and **no freeze gate, normalization rule, or pass/fail check reads any §7.2 field**.

*What `join` already computes at baseline:* `JoinedDecisionRow` (`crates/league-eval/src/freeze_gate.rs:135-178,761-804`) already resolves `first_draft_candidate_id`, `committed_ev_total_equity`, `committed_score_norm`, `first_draft_ev_total_equity`, and `first_draft_score_norm`, and `PerArmSummaryRow::n_with_first_draft` (`freeze_gate.rs:214,224,824-827,840`) already counts decisions carrying a `first_draft_candidate_id`. Measure 2 below computes the explicit signed revision-gain deltas and per-arm means on top of those joined scores without re-running `league-scorer`.

#### 7.2.1 Named Constants

All thresholds and windows used by Measures 1–5 are fixed named constants (checked against code by an automated parity test; no unrecorded literals):

| Constant | Rust Type | Value | Meaning |
|---|---|---|---|
| `TOOL_CALL_USED_MIN_COUNT` | `u32` | `1` | Minimum `EvalDecisionRecord::tool_calls_used` for a decision to count as having used at least one tool call (Measure 1). |
| `HTTP_REPLY_OK_STATUS` | `u16` | `200` | Exact HTTP status required for a non-late `SavedHttpReplyRecord` to contribute HTTP-reply reasoning text (Measure 3). |
| `REFUSED_BUNDLE_LEGS_MIN_COUNT` | `u32` | `1` | Minimum `EvalDecisionRecord::refused_bundle_legs` counting as a whole-bundle reveal rejection alongside `refused_bundle.is_some()` (Measure 4). |
| `TALK_COMMITMENT_HONOUR_WINDOW_DAYS` | `u8` | `0` | Maximum turn offset `fill.turn - day` checked for Measure 5(b) fulfillment (`0` = same day `receipt.turn == day` only, matching single-day `branch` replay). |
| `TALK_COMMITMENT_MIN_FILL_BPS` | `u32` | `10_000` | Minimum qualifying bilateral fill share in basis points (`10_000` = 100.00%, full committed `OrderProposal::amount`) required for `honoured` in Measure 5(b). |
| `TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED` | `i64` | `0` | Exact integer tolerance in milli-`CRED` per `1_000` commodity units around `committed.price_per_unit_cred` when testing counterparty at-or-better price in Measure 5(b) (`0` = strict at-or-better). |

#### 7.2.2 Measure Definitions (1–5)

For each `(state_id, arm, replicate)` entry in `arm_map/<state_id>.json`, let `arm_dir` be `entry.decision_path.parent()`. Missing or unparseable optional sidecars (`tool-round-*.json`, `http-reply-round-*.json` or non-JSON HTTP `200` body, `relay/calls.jsonl`, `to-seat/*.negotiation.json`, `decisions/day-DD-talk-*.json`, `scoreboard.json`) emit `null` (`None`) on the affected per-decision metric(s), increment `unparseable_sidecars: u32` (when a file/line on disk failed to parse, summed per arm into `n_unparseable_sidecars: u64`), and let `join` write all unaffected §7.2 metrics and every S3a field; **never** impute missing data as `0` or `0/0`. Required join inputs (`freeze.json`, `arm_map`, `scores`, verified pre-act checkpoints when `--source-game` is supplied, `decision.json`, `outcome.json`) still fail closed as before. No-reply/transport-failed decisions (`EvalDecisionRecord::no_reply == true`) retain recorded attempt/refusal/reasoning telemetry where present and are included in denominators only for fields whose source records exist and parse cleanly on disk.

1. **Tool-call rate:**
   - **Recorded source:** `<arm_dir>/decision.json` referenced by `ArmMapEntry::decision_path` (`crates/league-eval/src/blind_builder.rs:63`), reading `EvalDecisionRecord::tool_calls_used` (`crates/league-eval/src/records.rs:375`, incremented at `crates/league-eval/src/driver.rs:754-757` for every sent tool call other than `CALL_LIMIT_REACHED`, including `MALFORMED_TOOL_CALL` and rule-refused previews) and `EvalDecisionRecord::no_reply` (`records.rs:422`).
   - **Counted quantity & per-decision value:** `tool_calls_used: u32` and `used_any_tool_call: bool` (`tool_calls_used >= TOOL_CALL_USED_MIN_COUNT`), plus `no_reply: bool`. (If `decision.json` is readable, both fields are defined across all joined decisions `n_decisions`.)
   - **Per-arm aggregate:** `n_decisions`, `mean_tool_calls_used: Option<f64>` (`Some(sum / n_decisions)` when `n_decisions > 0`), `n_with_tool_call: usize` (`count(used_any_tool_call)`), and `tool_call_rate: Option<f64>` (`Some(n_with_tool_call / n_decisions)` when `n_decisions > 0`).
   - **Missing data:** If `n_decisions == 0`, both means/rates are `None` (`null`), never `0.0`.

2. **Draft-to-commit revision gain:**
   - **Recorded source:** `ArmMapEntry::{committed_candidate_id, first_draft_candidate_id}` (`crates/league-eval/src/blind_builder.rs:58-61`), backed by `<arm_dir>/decision.json` (`EvalDecisionRecord::intermediate_bundle_sha256s`, `crates/league-eval/src/records.rs:390`, `driver.rs:747-766`) and `<arm_dir>/relay/seats/<Guild>.day-DD-act.tool-round-*.json` filtered by `<arm_dir>/relay/calls.jsonl` (`blind_builder.rs:245-326`), taking the first sent parsed `arguments` (`SavedToolRoundCall::{call_index, arguments, tool_result_sent, result_wire_json}`, `crates/league-relay/src/relay.rs:2651-2674`; `SavedToolRoundRecord::{round_index, attempt, calls}`, `relay.rs:2699-2717`) in `(round_index, attempt, call_index)` order (`call_index` set at `relay.rs:7242`) alongside blind score outputs joined in `JoinedDecisionRow::{committed_ev_total_equity, committed_score_norm, first_draft_ev_total_equity, first_draft_score_norm}` (`crates/league-eval/src/freeze_gate.rs:151-163,761-804`).
   - **Counted quantity & per-decision value:**
     - `revision_gain_ev_total_equity: Option<f64>`: `Some(committed_ev_total_equity - first_draft_ev_total_equity)` whenever `first_draft_ev_total_equity` is `Some`, else `None` (`null`, e.g., `off` arm or tool-arm decision with zero parsed drafts).
     - `revision_gain_score_norm: Option<f64>`: `Some(committed_score_norm - first_draft_score_norm)` whenever **both** `committed_score_norm` and `first_draft_score_norm` are `Some` (`v_max > v_min`), else `None` (`null`).
   - **Per-arm aggregate:** `n_with_first_draft` (already tracked on `PerArmSummaryRow`, `freeze_gate.rs:214,224`), `n_with_revision_gain_score_norm: usize` (`count(revision_gain_score_norm.is_some())`), `mean_revision_gain_ev_total_equity: Option<f64>` (mean over `n_with_first_draft`; `None` if `0`), and `mean_revision_gain_score_norm: Option<f64>` (mean over `n_with_revision_gain_score_norm`; `None` if `0`).
   - **Missing data:** Decisions without a first draft (`off` arm or zero drafts) or on degenerate flat states (`v_max == v_min`, `score_norm == None`) emit `null`, never `0.0`.

3. **Reasoning length:**
   - **Recorded source:**
     - Tool-round sidecars `<arm_dir>/relay/seats/<Guild>.day-DD-act.tool-round-*.json` (`driver.rs:714`, `SavedToolRoundRecord::{reservation_id, round_index, attempt, reasoning}` at `crates/league-relay/src/relay.rs:2695-2710`), filtered strictly by reservation IDs belonging to `(day, round="act", guild)` in `<arm_dir>/relay/calls.jsonl` (matching `blind_builder.rs:252-295`; never counted unfiltered when `calls.jsonl` is absent or unparseable). Sorted by `(round_index, attempt)` with consecutive deduplication if both attempt files are present (each unique `(round_index, attempt, reservation_id)` counted once).
     - Final HTTP reply sidecars `<arm_dir>/relay/seats/<Guild>.day-DD-act.http-reply-round-*.json` (`driver.rs:715`, `SavedHttpReplyRecord::{reservation_id, round_index, attempt, status, is_late, body}` at `crates/league-relay/src/relay.rs:2810-2830`), filtered by the same reservation IDs and requiring `status == HTTP_REPLY_OK_STATUS && !is_late`. Matching HTTP `200` non-late `body` must parse as JSON (`unparseable_sidecars += 1` and HTTP metrics -> `None` if not JSON); valid JSON carrying a non-null top-level `"error"` field is skipped (matching `driver.rs:772-775`). Raw HTTP reasoning text is extracted from `choices[0].message.reasoning` falling back to `choices[0].message.reasoning_content` (`crates/league-relay/src/relay.rs:8311,8402`).
     - Token telemetry from `<arm_dir>/decision.json`: `EvalDecisionRecord::completion_tokens` (`crates/league-eval/src/records.rs:369`) and `EvalDecisionRecord::reply_token_ids` (`records.rs:405`, summing `.len()` across outer rounds).
   - **Counted quantity & per-decision value:** Because tool-round HTTP replies also produce `SavedHttpReplyRecord`s when captured, each stream is reported **separately without double-counting**, plus a reservation-deduplicated combined character sum (`measure_reasoning_length_in_memory`, `crates/league-eval/src/mechanisms.rs:449-577`). One rule governs missing vs zero across all three character metrics: emit `None` whenever `relay/seats/` or `relay/calls.jsonl` is absent, any relevant `calls.jsonl` line or sidecar failed to parse cleanly, or all clean matching sidecars omitted reasoning (`reasoning: None` / no `reasoning` or `reasoning_content` string — including excluded reasoning or zero matching files); emit `Some(0)` **only** when sidecars were read cleanly with `calls.jsonl` present and held explicit empty reasoning strings (`""`).
     - `tool_round_reasoning_chars: Option<u64>`: sum of `reasoning.chars().count()` across clean matching `SavedToolRoundRecord`s containing `reasoning: Some(_)`; `None` if any tool-round sidecar/`calls.jsonl` line failed to parse, `calls.jsonl` or `relay/seats/` is missing, or zero matching records carried a non-`None` `reasoning` string (`Some(0)` only for explicit empty `reasoning: Some("")`).
     - `http_reply_reasoning_chars: Option<u64>`: sum of `raw_reasoning.chars().count()` across clean qualifying `SavedHttpReplyRecord`s (`status == HTTP_REPLY_OK_STATUS && !is_late` and valid JSON `body`) containing a string `reasoning` or `reasoning_content`; `None` if any HTTP sidecar/`body`/`calls.jsonl` failed to parse, `calls.jsonl` or `relay/seats/` is missing, or zero qualifying records held string reasoning (`Some(0)` only for explicit `""`).
     - `total_reasoning_chars: Option<u64>`: deduplicated by `reservation_id` (taking `SavedToolRoundRecord::reasoning` when present for a `reservation_id`, otherwise qualifying `SavedHttpReplyRecord` raw reasoning for that `reservation_id`, or summing disjoint streams); `None` if `relay/seats/` or `relay/calls.jsonl` is missing, either stream/`calls.jsonl` had any unparseable record, or neither stream held a clean string reasoning field (`Some(0)` only when all clean streams yielded at least one explicit empty `""` string).
     - `completion_tokens: u64` (`EvalDecisionRecord::completion_tokens`).
     - `reply_token_ids_len: Option<u64>`: `Some(sum(round_ids.len()))` when `!reply_token_ids.is_empty()`, else `None` (backends without per-token logprobs, e.g., non-local endpoints, leave `reply_token_ids` empty).
   - **Per-arm aggregate:** Explicit denominators (`n_with_reasoning_sidecars` for non-null `total_reasoning_chars`, `n_with_reply_token_ids` for non-null `reply_token_ids_len`) alongside `mean_tool_round_reasoning_chars` (mean over non-null `tool_round_reasoning_chars`), `mean_http_reply_reasoning_chars` (mean over non-null `http_reply_reasoning_chars`), `mean_total_reasoning_chars`, `mean_completion_tokens`, and `mean_reply_token_ids_len` (each `None` when its denominator is `0`), plus `unparseable_sidecars` / `n_unparseable_sidecars`.
   - **Missing data:** If `relay/seats/` or `relay/calls.jsonl` is absent, any sidecar/`calls.jsonl` line fails to parse, or `reasoning.exclude_from_transcript == true` omitted reasoning text, `None` is emitted for unavailable character/logprob counts rather than conflating missing, excluded, or unparseable sidecars with `0`-character reasoning; when a backend emits neither `reasoning`/`reasoning_content` fields nor per-token `logprobs`, only `completion_tokens` is guaranteed and the text/token-ID breakdown cannot tell reasoning tokens apart from content tokens.

4. **Illegal-order rate:**
   - **Recorded source:** `<arm_dir>/decision.json`, reading `EvalDecisionRecord::{parse_failure, refused_market_actions, refused_bundle_legs, refusal_code, refused_bundle}` (`crates/league-eval/src/records.rs:378,381,384,414,428`).
   - **Counted quantity & per-decision value:**
     - `has_reply_refusal: bool`: `parse_failure || refusal_code.is_some()`.
     - `refused_market_actions: u32`: exact count of refused Pass-A market actions recorded on `EvalDecisionRecord`.
     - `has_bundle_rejection: bool`: `refused_bundle.is_some() || refused_bundle_legs >= REFUSED_BUNDLE_LEGS_MIN_COUNT`.
   - **Per-arm aggregate:** Across `n_decisions`: `n_reply_refusals`, `reply_refusal_rate: Option<f64>`, `mean_refused_market_actions: Option<f64>`, `n_bundle_rejections`, and `bundle_rejection_rate: Option<f64>` (`None` when `n_decisions == 0`).

5. **Deal-keeping (two explicit, separate lines):**
   - **5(a) Engine-recorded pledges (structured Track-2 on-ledger line):**
     - **Recorded source:**
       - `checkpoints/day-DD-pre-act.save.json` and `.seal` in the matched source game directory (verified against `ArmMapStateFile::pre_act_save_sha256` / `pre_act_seal_sha256` at `blind_builder.rs:90-91`), forked via `EvalHost::fork_from_checkpoint` (`verify_and_fork_pre_act_host`, `crates/league-eval/src/mechanisms.rs:631-663`) and read through `EvalHost::observe_seat` (`crates/league-core/src/eval/host.rs:512-515`), yielding `SeatObservation::{pledges, audited_pledges}` — which clones `EngineState::{pledges, audited_pledges}` identically to `EvalSaveSnapshot::{pledges, audited_pledges}` (`crates/league-core/src/eval/host.rs:86-88`, `crates/league-core/src/observation.rs:212-213`), filtered to `IntentPledge::promisor == focal_seat` (`crates/league-seat-types/src/commit.rs:260-264`).
       - `<arm_dir>/outcome.json`, reading `BranchArmOutcome::receipt.broken_promises` (`crates/league-eval/src/records.rs:617`, `crates/league-seat-types/src/observation.rs:79`) filtered to `BrokenPromiseRecord::promisor == focal_seat` (`crates/league-seat-types/src/engine_types.rs:64-68`).
       - Optional terminal `<source_game_dir>/scoreboard.json` (`driver.rs:2255`), reading `FinalScoreboard::standings` (`SeatScoreEntry::broken_promises` at `crates/league-core/src/scoring.rs:595` / `crates/league-seat-types/src/scoring_types.rs:120`) for `seat == focal_seat` (`None` and `unparseable_sidecars += 1` if corrupt without aborting `join`).
     - **Per-decision & per-arm values:**
       - `pre_act_pledges_count: Option<u32>` and `pre_act_audited_pledges_count: Option<u32>` for `promisor == focal_seat` (`None` if no source `pre-act.save.json` is available).
       - `sweep_broken_promises_count: u32` (`receipt.broken_promises` with `promisor == focal_seat` on this day's sweep).
       - `terminal_broken_promises_count: Option<u32>` (`None` on single-day `branch` runs or missing/unparseable `scoreboard.json`; **never** imputed as `0` when terminal close data is absent).
       - Per-arm sums (`total_pre_act_pledges`, `total_pre_act_audited_pledges`, `total_sweep_broken_promises`, `total_terminal_broken_promises`) with explicit availability counts (`n_with_pre_act_pledge_snapshot`, `n_with_terminal_scoreboard`), keeping engine-recorded pledges strictly on their own reporting line wherever engine pledges exist.
   - **5(b) Text-parsed talk commitments (`honoured_commitments / applicable_commitments`):**
     - **Recorded source:** For the state's `(source_game_dir, day, focal_seat)`:
       - Incoming peer `negotiation.json` files: `<source_game_dir>/to-seat/day-DD-act-<PeerGuild>.negotiation.json` (`driver.rs:2048`, `view.rs:48`, `wording.rs:266-285`), reading `NegotiationInputs::whispers` where `whisper.from_guild == focal_guild`.
       - Focal seat's talk decision record: `<source_game_dir>/decisions/day-DD-talk-<FocalGuild>.json` (`driver.rs:2038`, `records.rs:344-351`, `reply.rs:577`), parsing non-refused `EvalDecisionRecord::raw_reply` with `parse_eval_talk_reply` (`talk_extract.rs:1332-1363`). If any `negotiation.json` or `talk` decision sidecar fails to parse cleanly, Measure 5(b) emits `None` for that decision and increments `unparseable_sidecars` without aborting `join`.
       - Pre-act engine checkpoint: `<source_game_dir>/checkpoints/day-DD-pre-act.save.json` and `.seal` (SHA-256 matched against `ArmMapStateFile::pre_act_save_sha256` / `pre_act_seal_sha256`, forked via `EvalHost::fork_from_checkpoint`).
       - Evaluated arm outcome: `<arm_dir>/decision.json` (`EvalDecisionRecord::{no_reply, refusal_code, raw_reply, apply_report}` at `records.rs:351,414,422,425`; `EvalSeatApplyReport` at `host.rs:148-156`) and `<arm_dir>/outcome.json` (`BranchArmOutcome::receipt` -> `PublicSweepReceipt::fills` at `observation.rs:99`; `FillRecord` & `FillKind` at `contracts.rs:375-423`).
     - **Formal definitions of extracted set, `applicable`, `honoured`, and ambiguous/unsettled breakdowns:** see `§7.2.3`.
     - **Per-decision & per-arm values:** `extracted_outgoing_commitments: Option<u32>`, `applicable_commitments: Option<u32>`, `inapplicable_commitments: Option<u32>`, `honoured_commitments: Option<u32>`, `third_party_fill_commitments: Option<u32>`, `posted_unfilled_commitments: Option<u32>`, `partial_bilateral_fill_commitments: Option<u32>`, `focal_talk_counts: Option<TalkExtractionCounts>` (`seen`, `matched`, `ambiguous` on focal outgoing whispers), `honour_rate: Option<f64>` (`Some(honoured / applicable)` iff `applicable > 0`, else `None`), plus per-arm sums `applicable_commitments_total`, `honoured_commitments_total`, `inapplicable_commitments_total`, `third_party_fill_commitments_total`, `posted_unfilled_commitments_total`, `partial_bilateral_fill_commitments_total`, `n_with_talk_commitment_inputs`, and `honour_rate: Option<f64>` (`Some(honoured_total as f64 / applicable_total as f64)` iff `applicable_total > 0`, **never imputing `0/0` as `0.0` or `1.0`**).

#### 7.2.3 Definition of `applicable` and `honoured` for Measure 5(b), and Limits of Recorded Data

1. **Outgoing commitment extraction (exact symmetry with `talk_rules_v2`):**
   - Measure 5(b) evaluates whether **`focal_seat`** kept commitments that `focal_seat` itself made to peers during Turn `day`'s `talk` round (complementing the peer-to-focal extractor in `crates/league-eval/src/talk_extract.rs:1048-1054,1277-1393`).
   - Run the frozen `TalkRulesV2Table::frozen()` grammar (`talk_extract.rs:38-419`), normalization, retraction/ambiguity latch dominance, whole-token amount/price scaling (`UNIT_SCALE`), and two-source cross-merge (`to-seat/day-DD-act-<PeerGuild>.negotiation.json` for `whisper.from_guild == focal_guild` merged per `(focal_seat, counterparty_seat, good)` with `decisions/day-DD-talk-<FocalGuild>.json`) filtering to **`proposer_seat == focal_seat && counterparty_seat != focal_seat && counterparty_seat < seat_count`**.
   - Every outgoing focal-to-peer whisper inspected populates `focal_talk_counts` (`seen = matched + ambiguous` per source and `total`), showing what prose fell outside the closed grammar. Each surviving merged outgoing `ExtractedTalkCommitment` (`counterparty_seat`, `offered_action = MarketAction::PostOffer { seat: focal_seat, order }`, where `order: OfferProposal` at `contracts.rs:258-271`) is evaluated independently. (If `--source-game` files are absent or unparseable, all Measure 5(b) counts for that decision emit `None`/`null`, never `0`.)

2. **When an extracted outgoing commitment is `applicable`:**
   - Fork `<source_game_dir>/checkpoints/day-DD-pre-act.{save.json,seal}` (after SHA-256 verification against `ArmMapStateFile`) via `EvalHost::fork_from_checkpoint` (`crates/league-eval/src/mechanisms.rs:1035-1142`) and test `[MarketAction::PostOffer { seat: focal_seat, order }]` alone with `EvalHost::apply_market_action` (`crates/league-core/src/eval/host.rs:558-562`), which calls `TurnEngine::apply_market_action` -> `apply_market_action_inner` -> `TurnEngine::handle_post_offer` (`crates/league-core/src/engine.rs:595,609-611,1789-1855`).
   - This is equivalent to running `preview_seat_bundle` over a scrubbed `PublicPreviewState` (`crates/league-seat-types/src/observation.rs:484-544`, `crates/league-core/src/observation.rs:542-568`), because `preview_what_if_with_beacon` -> `preview_bundle_with_context_internal` (`crates/league-core/src/engine.rs:1073`) validates each `MarketAction::PostOffer` via the exact same `TurnEngine::apply_market_action` -> `handle_post_offer` check before executing Steps 0–7 preview sweeps (`refused_market_actions` records iff `apply_market_action` returns `Err`).
   - A commitment is **`applicable`** iff `EvalHost::apply_market_action` returns `Ok(())`. That tests pre-act legality on `day`: `focal_seat` is not `CustomsInspector` wash-frozen (`wash_frozen_until_turn < turn`), `amount > 0 && price_per_unit_cred > 0`, `good` is valid, cartel domain / `is_hybrid` (`ClearinghouseBroker`) access holds, and `focal_seat`'s pre-act free balance holds enough commodity (`OfferSide::Sell`) or locked `CRED` (`OfferSide::Buy`, `checked_ceil_units_parity`).
   - Otherwise (`Err(_)`), the commitment was **not legally fillable from the pre-act checkpoint** and is counted in `inapplicable_commitments` (excluded from `applicable_commitments`). A focal `no_reply`, act refusal, or self-conflict across multiple simultaneous promises by `focal_seat` does **not** render an individually legal commitment inapplicable.

3. **When an `applicable` commitment is `honoured`:**
   - **Window:** Single-day `branch` outcomes record sweeps only for `day` (`BranchArmOutcome::receipt.turn == day`). Qualifying fills must satisfy `fill.turn >= day && fill.turn <= day.saturating_add(TALK_COMMITMENT_HONOUR_WINDOW_DAYS)`. With frozen `TALK_COMMITMENT_HONOUR_WINDOW_DAYS = 0`, only **same-day (`fill.turn == day`)** settlements in `outcome.json` (`BranchArmOutcome::receipt.fills`, `observation.rs:99`) are in window.
   - **Counterparty, good, side, and price matching on `FillRecord` (`contracts.rs:404-423`):** Inspect `FillRecord` entries in `receipt.fills` with:
     - `fill.kind` in `{FillKind::BatchFill, FillKind::AtomicSwap, FillKind::ContractDelivery}` (`contracts.rs:375-380`, populated at `engine.rs:2102-2112,2598-2639,3005-3057`);
     - `fill.good == Some(order.good)`;
     - **Direction (per `FillRecord` `from` = seller/delivering seat, `to` = buyer/receiving seat):**
       - If `order.side == OfferSide::Sell`: `fill.from == Some(focal_seat) && fill.to == counterparty_seat`, and `fill.unit_price_cred == Some(p)` satisfies `(p as i64) <= (order.price_per_unit_cred as i64).saturating_add(TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED)` (counterparty buys at or below the promised ask price, i.e. at-or-better for counterparty);
       - If `order.side == OfferSide::Buy`: `fill.from == Some(counterparty_seat) && fill.to == focal_seat`, and `fill.unit_price_cred == Some(p)` satisfies `(p as i64) >= (order.price_per_unit_cred as i64).saturating_sub(TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED)` (counterparty sells at or above the promised bid price, i.e. at-or-better for counterparty).
     - Fills with `unit_price_cred == None` (unpriced barters / unpaid contract deliveries) cannot establish priced execution at-or-better and do not qualify.
   - **Volume threshold (full vs partial):** Let `bilateral_units: i128` be the sum of `fill.units as i128` across all qualifying bilateral `FillRecord`s in window. An `applicable` commitment is **`honoured`** iff:
     `bilateral_units * 10_000 >= (order.amount as i128) * (TALK_COMMITMENT_MIN_FILL_BPS as i128)`
     With frozen `TALK_COMMITMENT_MIN_FILL_BPS = 10_000`, `100.00%` of the committed `order.amount` must bilaterally settle at-or-better price on `day`. (Partial fills `0 < bilateral_units < order.amount` do **not** increment `honoured_commitments` and are reported explicitly in `partial_bilateral_fill_commitments`.)

4. **Diagnostic sub-counts for unresolved / ambiguous outcomes (reported, never guessed):**
   For each `applicable` commitment that is **not** `honoured`, `join` classifies why without guessing intent:
   - `partial_bilateral_fill_commitments`: `0 < bilateral_units` satisfies the direction/price predicate but fails `TALK_COMMITMENT_MIN_FILL_BPS`.
   - `third_party_fill_commitments`: `bilateral_units == 0`, while `receipt.fills` contains one or more same-day same-good same-focal-side `BatchFill`/`AtomicSwap`/`ContractDelivery` fills at qualifying price with a **third seat** (`other_seat != counterparty_seat`).
   - `posted_unfilled_commitments`: neither bilateral nor qualifying third-party fills settled, yet `focal_seat`'s committed `act` reply (`decision.json` parsed `raw_reply` minus any refused indices in `EvalDecisionRecord::apply_report`) submitted an accepted Pass-A `MarketAction::PostOffer` matching `(focal_seat, order.side, order.good)` at-or-better price and total posted amount `>= order.amount` (so a resting DEX order stood on the book without matching `counterparty_seat`).

5. **Plain statement of what the recorded data cannot tell (never guessed):**
   - **Public DEX order provenance vs bilateral intent (third-party fills):** `MarketAction::PostOffer` / `OfferProposal` (`contracts.rs:258-271`) carries no counterparty field, and Step 0 uniform-price batch clearing (`BatchFill`, `engine.rs:2102-2112`) matches anonymous limit orders on price/tie-break across the entire table. If `focal_seat` posts the promised public order and a third seat crosses it ahead of `counterparty_seat`, `PublicSweepReceipt::fills` records `other_seat` rather than `counterparty_seat`. `join` reports `third_party_fill_commitments` separately and does not guess whether `focal_seat` kept faith or traded away.
   - **Resting unfilled orders vs counterparty refusal (`posted_unfilled_commitments`):** Because single-day `outcome.json` stores no peer private rationale and `PublicSweepReceipt::fills` logs only executed trades, a resting accepted `PostOffer` that went unfilled cannot distinguish a counterparty that walked away from a toxic/mispriced commitment designed not to clear; reported separately in `posted_unfilled_commitments`.
   - **Partial bilateral fills (`partial_bilateral_fill_commitments`):** Partial settlement due to counterparty size limits vs `focal_seat` posting less than promised is reported in `partial_bilateral_fill_commitments` and never rounded up to `honoured`.
   - **Multi-day horizon beyond `day`:** Single-day `branch` runs stop after `receipt.turn == day`, so fulfillment occurring on `day + 1..` does not exist in `outcome.json`.
   - **Free-text promises outside `talk_rules_v2`:** Vague prose, conditional clauses, or unlisted phrasing fail closed into `focal_talk_counts.ambiguous` (`seen`, `matched`, `ambiguous` reported alongside 5(b)) and are never interpreted post hoc.
   - **Terminal season `broken_promises` in single-day branches (5(a)):** `SeatScoreEntry::broken_promises` only exists when a full-season `scoreboard.json` is produced (`terminal_broken_promises_count = null` on single-day branches).
   - **Reasoning tokens when sidecars/logprobs are absent (Measure 3):** If a backend omits `reasoning`/`reasoning_content` or per-token `logprobs`, `completion_tokens` alone cannot separate internal reasoning tokens from final JSON answer tokens.

#### 7.2.4 Descriptive Report Schema (Per-Decision and Per-Arm)

Both `JoinedDecisionRow` (inside `<out>/joined_decisions.json`) and `PerArmSummaryRow` (inside `<out>/per_arm_summary.json`) gain an explicit `mechanisms` object (`DecisionMechanismMetrics` and `PerArmMechanismSummary`). Enclosing files preserve `inferential_status: "descriptive_only"` (`crates/league-eval/src/freeze_gate.rs:247,263,856,866`), and **no gate, freeze check, or normalization rule reads any §7.2 field**:

```text
DecisionMechanismMetrics {                             // on JoinedDecisionRow::mechanisms (descriptive_only)
  // Measure 1: tool-call rate
  tool_calls_used: u32,                                // EvalDecisionRecord::tool_calls_used
  used_any_tool_call: bool,                            // tool_calls_used >= TOOL_CALL_USED_MIN_COUNT
  no_reply: bool,                                      // EvalDecisionRecord::no_reply

  // Measure 2: draft-to-commit revision gain
  revision_gain_ev_total_equity: Option<f64>,          // committed_ev - first_draft_ev (null if no draft)
  revision_gain_score_norm: Option<f64>,               // committed_norm - first_draft_norm (null if no draft or flat)

  // Measure 3: reasoning length
  tool_round_reasoning_chars: Option<u64>,             // chars across SavedToolRoundRecord::reasoning (null if calls.jsonl/sidecars missing, excluded, or unparseable)
  http_reply_reasoning_chars: Option<u64>,             // chars across HTTP 200 !is_late message.reasoning / reasoning_content (null if missing, excluded, or unparseable)
  total_reasoning_chars: Option<u64>,                  // reservation_id-deduplicated reasoning char sum (null if missing, excluded, or unparseable)
  completion_tokens: u64,                              // EvalDecisionRecord::completion_tokens
  reply_token_ids_len: Option<u64>,                    // sum(round.len()) when !reply_token_ids.is_empty(), else null

  // Measure 4: illegal-order rate
  has_reply_refusal: bool,                             // parse_failure || refusal_code.is_some()
  refused_market_actions: u32,                         // EvalDecisionRecord::refused_market_actions
  has_bundle_rejection: bool,                          // refused_bundle.is_some() || refused_bundle_legs >= REFUSED_BUNDLE_LEGS_MIN_COUNT

  // Measure 5(a): engine-recorded pledges (separate line)
  pre_act_pledges_count: Option<u32>,                  // SeatObservation::pledges (cloned from EngineState::pledges) where promisor == focal_seat
  pre_act_audited_pledges_count: Option<u32>,          // pre_act_pledges intersect SeatObservation::audited_pledges
  sweep_broken_promises_count: u32,                    // PublicSweepReceipt::broken_promises where promisor == focal_seat
  terminal_broken_promises_count: Option<u32>,         // SeatScoreEntry::broken_promises if scoreboard.json present & parseable

  // Measure 5(b): text-parsed outgoing talk commitments (separate line)
  extracted_outgoing_commitments: Option<u32>,         // talk_rules_v2 commitments where proposer_seat == focal_seat
  applicable_commitments: Option<u32>,                 // subset legal under pre-act EvalHost::apply_market_action(PostOffer)
  inapplicable_commitments: Option<u32>,               // extracted - applicable
  honoured_commitments: Option<u32>,                   // applicable commitments meeting fill + price + window
  third_party_fill_commitments: Option<u32>,           // unhonoured applicable with qualifying third-party fill
  posted_unfilled_commitments: Option<u32>,            // unhonoured applicable with qualifying accepted PostOffer
  partial_bilateral_fill_commitments: Option<u32>,     // unhonoured applicable with >0 and <100% bilateral fill
  honour_rate: Option<f64>,                            // honoured / applicable when applicable > 0, else null
  focal_talk_counts: Option<TalkExtractionCounts>,     // outgoing focal->peer { seen, matched, ambiguous } counts
  unparseable_sidecars: u32,                           // count of unparseable optional sidecars for this decision (0 when clean)
}

PerArmMechanismSummary {                               // on PerArmSummaryRow::mechanisms (descriptive_only)
  // Measure 1: tool-call rate
  n_decisions: usize,                                  // total decisions in arm
  n_with_tool_call: usize,                             // count(used_any_tool_call)
  mean_tool_calls_used: Option<f64>,                   // None when n_decisions == 0
  tool_call_rate: Option<f64>,                         // n_with_tool_call / n_decisions; None when 0

  // Measure 2: draft-to-commit revision gain (complementing PerArmSummaryRow::n_with_first_draft)
  n_with_first_draft: usize,                           // count(revision_gain_ev_total_equity.is_some())
  n_with_revision_gain_score_norm: usize,              // count(revision_gain_score_norm.is_some())
  mean_revision_gain_ev_total_equity: Option<f64>,     // None when n_with_first_draft == 0
  mean_revision_gain_score_norm: Option<f64>,          // None when n_with_revision_gain_score_norm == 0

  // Measure 3: reasoning length
  n_with_reasoning_sidecars: usize,                    // count(total_reasoning_chars.is_some())
  n_with_reply_token_ids: usize,                       // count(reply_token_ids_len.is_some())
  mean_tool_round_reasoning_chars: Option<f64>,        // mean over non-null tool_round_reasoning_chars; None when 0
  mean_http_reply_reasoning_chars: Option<f64>,        // mean over non-null http_reply_reasoning_chars; None when 0
  mean_total_reasoning_chars: Option<f64>,             // None when n_with_reasoning_sidecars == 0
  mean_completion_tokens: Option<f64>,                 // None when n_decisions == 0
  mean_reply_token_ids_len: Option<f64>,               // None when n_with_reply_token_ids == 0

  // Measure 4: illegal-order rate
  n_reply_refusals: usize,                             // count(has_reply_refusal)
  reply_refusal_rate: Option<f64>,                     // n_reply_refusals / n_decisions; None when 0
  mean_refused_market_actions: Option<f64>,            // None when n_decisions == 0
  n_bundle_rejections: usize,                          // count(has_bundle_rejection)
  bundle_rejection_rate: Option<f64>,                  // n_bundle_rejections / n_decisions; None when 0

  // Measure 5(a): engine-recorded pledges (separate line)
  n_with_pre_act_pledge_snapshot: usize,               // count(pre_act_pledges_count.is_some())
  n_with_terminal_scoreboard: usize,                   // count(terminal_broken_promises_count.is_some())
  total_pre_act_pledges: Option<u64>,                  // None when n_with_pre_act_pledge_snapshot == 0
  total_pre_act_audited_pledges: Option<u64>,          // None when n_with_pre_act_pledge_snapshot == 0
  total_sweep_broken_promises: u64,                    // sum(sweep_broken_promises_count)
  total_terminal_broken_promises: Option<u64>,         // None when n_with_terminal_scoreboard == 0

  // Measure 5(b): text-parsed outgoing talk commitments (separate line)
  n_with_talk_commitment_inputs: usize,                // count(applicable_commitments.is_some())
  extracted_outgoing_commitments_total: Option<u64>,   // None when n_with_talk_commitment_inputs == 0
  applicable_commitments_total: Option<u64>,           // None when n_with_talk_commitment_inputs == 0
  inapplicable_commitments_total: Option<u64>,         // None when n_with_talk_commitment_inputs == 0
  honoured_commitments_total: Option<u64>,             // None when n_with_talk_commitment_inputs == 0
  third_party_fill_commitments_total: Option<u64>,     // None when n_with_talk_commitment_inputs == 0
  posted_unfilled_commitments_total: Option<u64>,      // None when n_with_talk_commitment_inputs == 0
  partial_bilateral_fill_commitments_total: Option<u64>, // None when n_with_talk_commitment_inputs == 0
  honour_rate: Option<f64>,                            // honoured_total / applicable_total iff > 0, else null (never 0/0)
  focal_talk_counts: Option<PerArmTalkCountsSummary>,  // None when n_with_talk_commitment_inputs == 0; else { seen, matched, ambiguous }
  n_unparseable_sidecars: u64,                         // sum(unparseable_sidecars) across n_decisions in this arm
}
```

---

## 8. Blinding, `arm_map/` Governance, and Freezing Protocol

### 8.1 Exact Blind Input Schema (`BlindScoringRequest`) and Disjoint `arm_map/`

An unblinded builder prepares staging inputs into two disjoint locations:
- `blind_inputs/<state_id>.json` — the **only** tree readable by `crates/league-scorer`; contains strictly content hashes and anonymous candidates, with zero filesystem paths, arm labels, or tool-trace metadata. (Archive paths for `.save.json`/`.seal` blobs are resolved outside the hashed `BlindScoringRequest` via content-addressed CLI inputs and verified against `pre_act_save_sha256` / `pre_act_seal_sha256`.)
- `arm_map/<state_id>.json` — held strictly by the **run operator's reporting step**, separate from `crates/league-scorer` and from anyone tuning parameters.

```text
BlindScoringRequest {
  schema_version: u32,                            // fixed = 1
  state_id: String,                               // canonical "<source_game_id>-day-<DD>-seat-<idx>" (no arm)
  pre_act_save_sha256: String,                    // 64-hex SHA-256 of checkpoints/day-DD-pre-act.save.json bytes
  pre_act_seal_sha256: String,                    // 64-hex SHA-256 of checkpoints/day-DD-pre-act.seal bytes
  pre_act_pair_sha256: String,                    // 64-hex digest matching compute_pre_act_pair_sha256_hex
  focal_seat: u8,                                 // 0..4
  talk_commitments: Vec<ExtractedTalkCommitment>, // neutral counterfactual commitment specs (no raw text)
  talk_extraction_counts: TalkExtractionCounts,   // per-source & total { seen, matched, ambiguous } counts (no prose)
  candidates: Vec<BlindCandidate>,                // sorted ascending by candidate_id; duplicates merged
  scorer_seed: u64,                               // explicit recorded seed
  scorer_config_sha256: String,                   // SHA-256 of canonical ScorerConfig JSON (§5.2)
}

TalkExtractionCounts {
  negotiation: TalkSourceCounts,                  // counts over negotiation whispers to focal_seat
  talk: TalkSourceCounts,                         // counts over outgoing talk whispers to focal_seat
  total: TalkSourceCounts,                        // element-wise sum (negotiation + talk)
}

TalkSourceCounts {
  seen: u64,                                      // total messages inspected (matched + ambiguous)
  matched: u64,                                   // messages where every clause matched a closed template
  ambiguous: u64,                                 // messages failing closed on byte, number, or unlisted clause
}

BlindCandidate {
  candidate_id: String,                           // sha256_hex(serde_json::to_vec(&submission))
  submission: EvalActSubmission,                  // market_actions, pass_b_actions, bundle only
  in_span: bool,                                  // true iff passive, synthetic, or any committed decision
  is_arm_independent: bool,                       // true iff passive, ClubBot, or synthetic (§3.2 / §7.1)
}
```

Removed/excluded from `BlindScoringRequest` and `blind_inputs/`: filesystem paths, wall-clock timestamps, `arm` (`off` / `control` / `on`), replicate index, `wording_arm`, branch output paths (`<out>/<arm>/...`), `raw_reply`, `discarded_reply_path`, `tool_calls_used`, `intermediate_bundle_sha256s` labels, `<Guild>.jsonl`, `<Guild>.scratchpad.md`, `http-reply-round-*.json`, and `tool-round-*.json`.

> **Amendment (`2026-11-22`):** `state_id` separators were changed from `:` (`<source_game_id>:day-<DD>:seat-<idx>`) to `-` (`<source_game_id>-day-<DD>-seat-<idx>`) because `:` is illegal in Windows filenames and `league-scorer` requires request and output filenames (`<state_id>.json`) to remain portable across platforms.

Separately, `arm_map/<state_id>.json` (held exclusively by the run operator's reporting step) stores `Vec<{ arm, replicate, committed_candidate_id, first_draft_candidate_id, decision_path, outcome_path }>` for post-freeze reporting.

### 8.2 Arm-Blind Tuning Rules and Freeze Procedure Before Main Run

1. **Arm-blind pilot tuning (`§5.2` parameters) & post-pilot cost check:** Run the D5 pilot (`4` seeds × `6` states × `3` arms × `3` replicates = `216` arm decisions) using the `r2` legality-check `control` wording arm being built separately (any pilot collected on the superseded inert control arm MUST be rerun on the `r2` legality-check control arm before freezing). Build `blind_inputs/` and `arm_map/` (retained inside the reporting step's separated store), run `league-scorer` twice to verify bit-identical `scores/` JSON across runs and thread counts, time mid-season and late-season fork-to-close rollouts to re-derive `§6.2` and re-apply the ~1-day 8-core cost stop (stopping for a decision if the re-derived main run crosses one day on 8 cores), and tune only parameters in `§5.2`'s Free Parameter Table strictly from **arm-blind diagnostics** (`paired_se_equity`, measured runtime, flat-set / pre-screen rate, arm-independent candidate coverage, and repeat-pass bit identity).
2. **Pre-freeze isolation rule:** **No person or tool joins `arm_map/` to any score before `scorer_freeze.json` exists.** Later implementation MUST include an automated test proving that attempting to join `arm_map/` to scores is refused while no recorded `scorer_freeze.json` exists.
3. **Commit clean tree and write `scorer_freeze.json`:** Refuse any dirty git worktree (`DIRTY_GIT_WORKTREE`), write `scorer_freeze.json`, and record **`scorer_freeze.json`'s SHA-256 in the first run's pre-registration before any main-run decision is scored**:
   - `scorer_git_sha` (40-hex workspace commit SHA)
   - `scorer_tree_sha256` (hash of `crates/league-scorer/**`)
   - `scorer_config_sha256` (canonical SHA-256 of frozen `ScorerConfig` covering every entry in `§5.2`)
   - `control_wording_arm` (exact `r2` legality-check control arm label used identically by pilot and main run, enforced strictly outside `league-scorer`)
   - `talk_extractor_version` & `talk_extractor_rules_digest`
   - `blind_inputs_manifest_sha256` (sorted SHA-256 manifest of main-run `blind_inputs/*.json` committed before any `scores/*.json` write)
4. **Main-run enforcement (with `league-scorer` isolated from arm/wording labels):**
   - `crates/league-scorer` reads only `blind_inputs/` and `scorer_freeze.json::{scorer_config_sha256, blind_inputs_manifest_sha256, scorer_tree_sha256}`, verifying `ScorerConfig` SHA-256 and never reading `setup.json` (`relay_wording_arm`) or any wording-arm or arm label.
   - Outside `league-scorer`, the unblinded input builder and the reporting step verify `setup.json`'s `relay_wording_arm` (`crates/league-eval/src/driver.rs:1715,1921`) matches `scorer_freeze.json::control_wording_arm` on `control` runs; the reporting step additionally refuses any `arm_map/` join lacking a valid pre-registered `scorer_freeze.json`.

**S3a CLI pointer:** Outside `league-scorer`, `league-eval build-blind-inputs --run <dir>... --out-blind <dir> --out-arm-map <dir> --config <file> --scorer-seed <u64>` reads evaluation run directories, applies the frozen `talk_rules_v2` commitment extractor (scaling whole-number amounts and prices by `league_seat_types::UNIT_SCALE`), calls `league-scorer` to generate §3.1 items 1 and 4 (`passive`, `ClubBot`, and `<= k_synth_max` synthetic candidates), merges all evaluated arms' committed decisions (§3.1 item 2, `in_span: true`) and first sent parsed tool drafts (§3.1 item 3, `in_span: false`) across Sources 1–4 by canonical `candidate_id` with logical-OR flags (§3.1 item 5), and emits disjoint `blind_inputs/` and `arm_map/` trees; `league-eval freeze --blind <dir> --config <file> --control-wording-arm <label> --out <file>` refuses dirty git worktrees (`DIRTY_GIT_WORKTREE`) and existing files (`FREEZE_FILE_EXISTS` via atomic `create_new`) before writing `scorer_freeze.json`; `league-scorer score ... [--freeze <file>] [--scorer-crate-dir <dir>]` optionally verifies the frozen config, crate source tree on disk (`--scorer-crate-dir` hashes the given tree rather than the running binary), and blind-input manifest before scoring and echoes `blind_request_sha256` in each state's score JSON; and `league-eval join --freeze <file> --arm-map <dir> --scores <dir> --out <dir>` enforces freeze presence, exact `blind_request_sha256` manifest and `arm_map/` parity, config hash, and `control` run `relay_wording_arm` parity before writing descriptive-only per-arm tables (see [`crates/league-eval/README.md`](../crates/league-eval/README.md)).

---

## 9. Crate Placement (`crates/league-scorer`)

**Decision:** Place the blind scoring engine in a new dedicated workspace crate **`crates/league-scorer`** (`library` + `league-scorer` binary), accompanied by unblinded reporting hooks in `crates/league-eval` (held by the reporting step).

**Rationale over alternatives:**
- *Compile-time blinding vs placing inside `league-eval`:* `league-eval` links `EvalArm`, `EvalDecisionRecord`, `BranchArmOutcome`, `league-relay`, HTTP transcripts, and arm directories. Separating `crates/league-scorer` guarantees compile-time isolation: `crates/league-scorer/Cargo.toml` depends strictly on `league-core`, `league-seat-types`, `serde`, `serde_json`, `sha2` (and optional `rayon` — noting `rayon` is **not** in `Cargo.lock` today and would add one new dependency whose usage must collect indexed results so outputs never depend on thread count), with **zero** dependency on `league-eval` or `league-relay`.
- *In-process Rust vs external FFI/Python:* Across `84,480–94,080` full-horizon rollouts (one `EvalHost::fork_from_checkpoint` call per rollout, as in `§6.2`), external IPC or Python bindings would add overhead and risk divergence from `league-core` invariants. A Rust workspace crate calls `EvalHost::fork_from_checkpoint`, `apply_act_submissions`, `sweep_turn`, `finalize_eval_season`, `ClubBot::decide_full_turn`, and `preview_seat_bundle` directly in `--release`, inherits `[workspace.lints]` (`unsafe_code = "forbid"`, `missing_docs = "warn"`), and builds offline under `--locked`.

---

## 10. Known Limits

1. **Surrogate rollout policy (`ClubBot`) for peers and focal post-branch continuation:** After the branch day `day`, **the focal seat is played by `ClubBot` like every other seat in the rollout**, while D5's base-game peers (`play`) are interactive model seats (`docs/eval-harness.md:13`). If an action's payoff hinges on multi-turn LLM persuasion or adaptive play across subsequent turns rather than `ClubBot` continuation, single-branch rollouts will miss that strategy difference; CRN keeps within-state comparison internally consistent across `C(state)`.
2. **Fixed base-game VRF future beacons across counterfactuals:** `EvalHost::fork_from_checkpoint` restores `vrf_proofs` from the base game's `.seal`, so future close-hazard rolls (`close_roll_bps_from_beacon`) and shocks on `t > day` remain fixed to the base season trajectory across all candidates (`CRN`).
3. **Talk-deal extractor recall and separate engine pledges:** Deterministic rule matching misses vague hints or indirect conditional messages; under-capture is symmetric across arms and supplemented by the separate engine-recorded `pledges`/`audited_pledges`/`broken_promises` mechanism line (`§7.2`).
4. **Span sensitivity, `NA` flat sets, and Winner's curse:** Normalization `(EV(c) - v_min) / (v_max - v_min)` amplifies noise on narrow-span states and compresses scores near extreme failures. Flat sets (`v_max == v_min`) emit `NA` (`null`, excluded upfront by `§3.2`), Winner's-curse upward bias on `v_max` over `K` noisy estimates is measured via split-half re-estimation (an `M/2` selection-bias upper estimate for `M` scenarios) and explicitly reported, and the arm-independent percentile sensitivity score (`§7.1`) provides an outlier-resistant complement.
5. **Uniform prior over `spec.mandate_deck` vs behavioural posterior:** Peer mandates are drawn uniformly without replacement from the undealt entries of `obs.spec.mandate_deck` (`types.rs:381`, `observation.rs:86`), **not** inferred as a posterior from peer behaviour on turns `1..=day`.
6. **Untimed fork/posture costs in the `0.10 s` anchor and three-replay test overhead:** Calling `EvalHost::fork_from_checkpoint` per `(c, m)` rollout (`EvalHost` lacks `Clone`, `host.rs:197`) and `EvalHost::finalize_eval_season` (`host.rs:783-840`) adds per-rollout cost not captured in the `0.10 s` `turn-0` runner-resolution sample (`crates/league-core/tests/eval_host.rs:447-515`, which runs three replays in total: two at `host.rs:811-812` plus one external `replay_eval_season` at `eval_host.rs:510`). Pilot mid-season and late-season timed runs re-derive §6.2's budget and re-check the ~1-day 8-core cost stop before main-run freezing.
7. **Tool-draft candidate role visibility (`in_span: false`, `is_arm_independent: false`):** Inside `BlindScoringRequest`, any candidate with `in_span: false` and `is_arm_independent: false` can only be a tool-arm first draft (since `off` has no drafts, and matching passive/synthetic/committed candidates merge into `in_span: true`). Neither `control` nor `on` nor any arm label is identified, and `league-scorer` must use `in_span` strictly to exclude the candidate from `[v_min, v_max]` normalization and never to tell arms apart.
8. **Checkpoint verification-to-fork TOCTOU window:** Before invoking `EvalHost::fork_from_checkpoint(save_path, seal_path)` (`crates/league-core/src/eval/host.rs:1195-1213`), `verify_checkpoint_digests` hashes the `.save.json` and `.seal` files on disk and checks `BlindScoringRequest::{pre_act_save_sha256, pre_act_seal_sha256, pre_act_pair_sha256}`. Because `EvalHost::from_save_and_seal_bytes` (`host.rs:955`) is `pub(crate)` in `league-core`, external callers cannot pass the already-verified byte slices directly into `EvalHost`; a privileged writer racing on the checkpoint directory between digest verification and `fork_from_checkpoint` is therefore outside the single-process immutable-checkpoint assumption until a byte-buffer fork constructor or read-only storage is used.
