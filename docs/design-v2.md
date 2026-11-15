# Merchant League v2 Design: The Chiral Concordat

**Status:** Draft specification for Part 1 of the Merchant League v2 build-out, translated directly from the seed-26 research specification (*Designing a Dual-Purpose On-Ledger Strategy Game and LLM Multi-Agent Benchmark*) and checked against live XRPL Mainnet and Testnet `feature` RPC state on **2026-11-15**. Supersedes `docs/design.md` (`main` and PR #6 `design/redesign-r1`), pending the Director's rulings on the named conflicts in [Section 9](#9-conflicts-with-docsdesignmd-and-decisions-for-the-director).

Merchant League v2 is one shared politico-economic game architecture serving two execution modes over identical C++ `xrpld` protocol rules without smart contracts:
1. **Game Mode (Human and Mixed Human–AI Play):** runs on the public XRPL Testnet (`s.altnet.rippletest.net`, `network_id = 1`), backed by an off-ledger event-sourced Archivist mirror and an automated 3-wave Phoenix state rehydrator that survives unconsented Testnet resets.
2. **Headless Training and Evaluation Mode (AI League and Benchmark):** runs on local containerized `xrpld` nodes in Stand-Alone Mode (`xrpld -a --start`), closing ledgers in milliseconds via the admin `ledger_accept` RPC so multi-agent reinforcement learning, heuristic baselines, and mirrored benchmark suites run free of public rate limits and network wipes.

---

## Hard Invariants and Epistemic Rules

1. **Testnet-only for public network play; no real money:** Live player-facing code connects strictly to XRPL Testnet endpoints and enforces `require_testnet` (`network_id == Some(1)`) in `league-ledger`. There is no real money, no real-currency pricing, and no off-platform transfer of game standing; all game copy uses game terms (`CRED`, marks, raw goods, refined goods, syndicate projects, table equity) and must pass `scripts/check-testnet-only.sh` and `scripts/check-copy.sh` with zero allow markers. (Note: how the Part 4 Stand-Alone `xrpld` harness enforces loopback + non-Mainnet `!= Some(0)` alongside `league-ledger::Testnet` is flagged as Conflict 11 for the Director.)
2. **Mainnet-enabled XRPL features only:** Every transaction type, flag, and amendment relied upon by v2 must be verified `enabled: true` on XRPL Mainnet via `feature` RPC before use, because Testnet resets align Testnet amendments with Mainnet. Testnet-only or unactivated proposals (`ConfidentialTransfer` XLS-96, `MPTokensV2`, unimplemented MPT DEX trading) are forbidden.
3. **Report figures are hypotheses, not facts:** All quantitative projections in the seed-26 report—random bilateral lock-in probabilities (`79.7%` and `64.2%`), Herfindahl-Hirschman Index comparisons (`0.655` / `1.53` effective dyads vs. `0.130` / `7.72` effective dyads), benchmark sample reductions (`372` vs. `64` vs. `~22` games), `~36.8 XRP` 6-seat table reserves, `~12–15 s` 3-wave Phoenix rehydration across `~95` inner transactions, context-compaction token reductions (`23.90M` to `3.29M` input tokens / `86.2%`), Rath's `73`-interaction drift onset (derived from simulated enterprise workflows, not strategy games), and Mode A / Mode B Playtest 1 target bands—are **engineering priors and hypotheses**. Parts 2–4 and Playtest 1 must measure them empirically before treating any number as settled.
4. **Unverified external references and legal claims:** Several 2026 citations in the seed-26 report (including CT-Bench, QUACK, MafiaScope, Zhang 2026, Kaggle Game Arena 2026, Wang et al. NSR 2026, and Jiang et al. 2025) have not yet been audited at source by R&D, and no peer-reviewed study has measured human–LLM coalition dynamics across multi-week asynchronous seasons. The EU AI Act Article 50(1) disclosure note is a legal claim requiring an external check before any public EU launch.
5. **Deterministic replayability:** Every turn transition, Steward bond ruling, check bounce, clawback, tariff shift, shock draw, and final equity score must replay identically from a table's initial seed plus its ordered log of committed/revealed actions and beacon/oracle outputs.

### Resolving the Six Structural Conflicts Between Human Fun and AI Benchmarking

| # | Conflict | Human Strategy Requirement | AI Training & Eval Requirement | v2 Chiral Concordat Resolution |
|---|---|---|---|---|
| 1 | Kingmaking vs. Psychometric Signal | Betrayal invites credible late-game retaliation and political drama | Resist arbitrary dead-player griefing that injects noise into ratings | **Cardinal Net Equity + Private Mandates:** Continuous equity scoring replaces binary solo win/loss so every seat retains positive marginal incentive through the final turn |
| 2 | Multi-Week Pacing vs. Drift & Compute | 10–14 day saga playable in short daily check-ins | High episode throughput while controlling long-horizon context drift and token cost | **Clock-Agnostic 24–28 Turn Seasons:** Two 12-hour turns/day (or 10-minute club turns) for humans; millisecond `ledger_accept` in Stand-Alone Mode for AI |
| 3 | Expressive Chat vs. Objective Auditing | Open rhetoric, bluffing, and coalition persuasion | Ground-truth verification free of noisy LLM judges, prompt injection, and steganography | **Typed Intent Header + Free-Text Body:** Structured JSON proposal headers referencing exact XRPL object IDs/vote targets paired with concise off-ledger chat hashed into `Memos` |
| 4 | Faction Asymmetry vs. Seat Variance | Distinct asymmetric economic roles | Low seat-bias variance and small sample size for model comparisons | **Rotational Chiral Base + Mirrored Duplicate Seeds:** Rotationally symmetric converter ring with modular `Credentials`, evaluated across seat-swapped duplicate runs |
| 5 | Non-Transitive Politics vs. Scalar Ranks | Shifting table metas and counter-coalitions | Clone-invariant ratings across cyclic matchups where Elo breaks down | **Dual Rating Layer:** Bayesian TrueSkill tiers displayed to human players; Coarse Correlated Equilibrium (CCE) Deviation Ratings on held-out populations for AI |
| 6 | Dramatic Shocks vs. Sample Efficiency | Supply disruptions that fracture static mega-alliances | Low luck variance so skill separates cleanly in ~20–30 matches | **Telegraphed Horizon Oracles + Control Variates:** 2-turn-ahead `OracleSet` shock forecasts on identical duplicate seeds scored with AIVAT-style control variates |

---

## 1. The Chiral Converter Ring and Economy (`N = 4, 5, 6` Seats)

### 1.1 Elimination of Spatial Travel and Random Endowments

Early prototype playtests stalled because random resource endowments frequently allowed two seats to form a closed bilateral swap loop while multi-turn spatial travel locked up working capital across empty turns. Merchant League v2 removes spatial caravan movement across towns entirely and replaces random endowments with a **directed chiral converter ring** over $N \in \{4, 5, 6\}$ seats indexed modulo $N$ ($i \in \{0, \dots, N-1\}$).

### 1.2 Token Representation (Trust Line IOUs Only)

- **No Multi-Purpose Tokens (MPTs) for commodities:** As of November 2026, `MPTokensV1` does not implement DEX `OfferCreate` order-book trading or AMMs. All tradable game commodities and the table numeraire coin `CRED` are issued as **XRPL Trust Line Tokens (IOUs)** by a single per-table **Authority (Steward)** account using table-scoped 3-character currency codes (`CRED`, Raw $R_0 \dots R_{N-1}$, Refined $M_0 \dots M_{N-1}$).
- **Mandatory genesis account-flag order (on empty Authority directory):**
  1. `asfRequireAuth` — restricts token custody exclusively to seated accounts authorized for that table, preventing cross-table token leakage.
  2. `asfAllowTrustLineClawback` — **must be enabled before any trust line or owner-directory object exists** (fails permanently otherwise, and must never be combined with `asfNoFreeze`, which returns `tecNO_PERMISSION`). Enables automated spoilage and upkeep sinks.
  3. `asfAllowTrustLineLocking` — enables `TokenEscrow` (`XLS-85`) for IOU bonds and conditional vaults.
  4. `asfDefaultRipple` on Authority, paired with `NoRipple` (`tfSetNoRipple`) on every seat's trust lines so seats cannot involuntarily bridge third-party balances.
- **Auto-bridging faucet-drain prevention:** Because XRPL DEX `OfferCreate` auto-bridges `Token A -> XRP -> Token B` whenever composite XRP pairs are cheaper—and Testnet faucet XRP is free—trading on an open XRP pair would let any player drain scarce commodities with free XRP. Every seat and Authority sets `asfDisallowXRP` after initial reserve funding, all game markets price strictly in `CRED`, and all cartel books are isolated inside `PermissionedDomains` (`DomainID` auto-bridges strictly within the same domain).

### 1.3 Three-Tier Chiral Production and Upkeep Cycle

Every turn, each Seat $i \in \{0, \dots, N-1\}$ operates three economic tiers on the directed ring (indices modulo $N$):

1. **Tier 1 — Raw Extraction:** Seat $i$ receives turn yield $y_i > 0$ of its exclusive raw commodity $R_i$ (minted by Authority at Sweep).
2. **Tier 2 — Chiral Primary Converter (and Optional Cross-Converters):** Seat $i$'s primary converter consumes strictly foreign raw inputs from two distinct neighbors in one direction around the ring to produce Refined commodity $M_i$:
   $$a_i\,R_{(i+1) \bmod N} + b_i\,R_{(i+2) \bmod N} \longrightarrow c_i\,M_i$$
   Converter runs settle atomically on-ledger via `BatchV1_1` (`tfAllOrNothing`), delivering inputs to Authority (burn/sink) and minting $c_i M_i$. Optional secondary cross-converters sampled by the table generator must preserve zero 2-seat self-sufficiency.
3. **Tier 3 — Syndicate Projects (Irreversible Equity Sinks):** Multi-commodity Syndicate Projects consume specified bundles of Refined goods $(M_0, \dots, M_{N-1})$ transferred to Authority (destroyed on return to issuer) and award irreversible **Project Equity** points.
4. **Chiral Upkeep:** At the end of every turn, Seat $i$ owes mandatory upkeep $u_i > 0$ in the refined good produced by its opposite neighbor $M_{(i-1) \bmod N}$:
   - Seat $i$ extracts $R_i$, needs $R_{i+1}$ and $R_{i+2}$ to produce $M_i$, and owes upkeep in $M_{i-1}$ (which is produced by Seat $i-1$ from $R_i$ and $R_{i+1}$).
   - Under this topology, **1-seat autarky satisfies `0%` of upkeep** (`0` of `1` seat fed) and **any 2-seat dyad satisfies `0%` of its internal upkeep** (`0` of `2` seats fed), because for any pair $\{i, j\}$, Seat $i$ only produces $M_i$ which feeds $i+1$, while Seat $i$ needs $M_{i-1}$; even if $j = i-1$, Seat $j$ cannot feed itself ($M_{j-1} = M_{i-2}$ is absent) and Seat $j$ needs $R_i$ AND $R_{i+1}$ (missing $R_{i+1}$ when $N \ge 4$ unless another external seat supplies it). For $N=5$, a 3-seat bloc satisfies at most `1/3 = 33.3%` of its members' upkeep and a 4-seat bloc satisfies at most `2/4 = 50.0%`.

*Hypotheses to verify in Part 2:* The seed-26 report models that random 2-of-4 resource endowments in a 5-seat table produce a `79.7%` probability of a closed 100% self-sufficient 2-seat dyad (`64.2%` probability of $\ge 75\%$ mutual coverage with 6 resources), and projects a trade-dispersion shift from bilateral lock-in (`HHI = 0.655`, `1.53` effective pairs of 10) to chiral ring play (`HHI = 0.130`, `7.72` effective pairs). Part 2's generator crate must measure these distributions directly rather than assuming them.

### 1.4 Perishable Decay and Tiered Auto-Debt Cushion

- **Perishable spoilage (`Clawback`):** Unconverted Raw goods $R_k$ (and designated perishable outputs) decay at the end of every turn at the table's spoilage rate (baseline `20%`/turn, rounded deterministically). Because XRPL `Clawback` cannot seize tokens locked inside `TokenEscrow` (`docs/receipts/design-check-requireauth-2026-11-14/`), **perishable commodities are never eligible to be locked in multi-turn `TokenEscrow` collateral vaults**. Collateralized vaults and bonds lock only non-perishable `CRED`. Authority always queries the current trust-line balance before submitting `Clawback` and logs the exact clawed-back amount.
- **Auto-debt cushion (preventing death spirals):** If Seat $i$ lacks sufficient $M_{(i-1) \bmod N}$ at Sweep upkeep collection, Authority claws back any available $M_{(i-1) \bmod N}$ and converts the unpaid shortfall at fundamental parity into interest-bearing `CRED` debt owed to the clearinghouse. Debt accrues per-turn interest set by the seat's on-ledger credit rating credential:
  - `CREDIT_AAA`: `4%`/turn interest (report cites `2%–6%` range; `4%` baseline).
  - `CREDIT_BBB` / `CREDIT_B`: intermediate stepped tiers (e.g., `10%` and `18%`/turn).
  - `CREDIT_D` (defaulted): `30%`/turn interest plus immediate exclusion from prime cartel `PermissionedDomains`.
  Auto-debt prevents early accidental elimination while `CREDIT_D` penalties block deliberate "debt diving" exploits.

### 1.5 Procedural Table Generator and Feasibility Filter (Part 2 Specification)

Static table layouts invite opening-book memorization. Part 2 builds a pure-Rust, ledger-free seeded table generator paired (in Training/Eval Mode) with **Prioritized Level Replay (PLR)** sampling across five table parameters:
1. Seat count $N \in \{4, 5, 6\}$.
2. Primary converter stoichiometry $(a_i, b_i \to c_i)$ and optional secondary cross-converters.
3. Baseline commodity export `TransferRate` tariffs (`0%–20%` initial baseline; dynamic minister bounds `0%–35%`).
4. Perishable `Clawback` decay rates.
5. `OracleSet` shock schedules and Syndicate Project requirements.

Before any generated table seed is accepted for play or training, a deterministic **Linear-Programming (LP) Feasibility Filter** must prove all three invariants:
1. **Zero 2-seat self-sufficiency:** For every unordered pair $\{i, j\}$, internal extraction plus pair-internal conversion cannot satisfy both seats' turn upkeep (`0%` pair self-sufficiency).
2. **Strict aggregate converter surplus:** Table-wide feasible production strictly exceeds aggregate turn upkeep by a positive project-building margin so cooperative trade is strictly positive-sum.
3. **Seat equity balance:** Under optimal competitive/cooperative shadow-price allocation, every seat $i \in \{0, \dots, N-1\}$ starts with expected potential within a stated tolerance ($\pm 3\%$ of table mean baseline).

In Headless Training Mode, PLR weights replay priority toward valid seeds exhibiting high policy value-loss / regret variance while preserving non-zero coverage of fresh seeds.

---

## 2. Simultaneous Turn Engine: Talk & Market, Commit, Sweep

Sequential turns impose an $(N-1)/N$ waiting penalty and let backward induction trivialize bilateral bargains. Merchant League v2 uses **simultaneous discrete turns** modeled on Diplomacy and Subterfuge, executed identically in pure state (Part 3) and on-ledger (later card):

### Phase 1: Diplomatic and Market Window ("Talk and Market")

- **Dual-format negotiation messages (off-ledger Relay):** Every negotiation message pairs:
  1. A **Typed JSON Intent Header** naming exact counterparties, commodity quantities, `CRED` amounts, deadlines, referenced XRPL object IDs (`Offer`, `Escrow`, `Check` sequence/ID), or council vote targets.
  2. A concise **Free-Text Body** (`<120` words per message in Game Mode) carrying rhetoric, explanations, and persuasion.
  - Raw negotiation text is **never written onto the public XRPL ledger** (GDPR and transcript-privacy rule). Only salted SHA-256 hashes of committed deal digests may appear in XRPL `Memos` ($\le 1\text{ KB}$ total per transaction).
- **Live ledger market actions:** Throughout the window, seats post/cancel `OfferCreate`/`OfferCancel` orders in `CRED` (within cartel `DomainID` or open/hybrid `tfHybrid` books), create deferred `CheckCreate` credit notes, or post collateralized `TokenEscrow` offers.
- **Parallel non-stalling submission (`TicketBatch`):** Seats pre-allocate XRPL `Tickets` (`TicketCreate`, `Sequence: 0`) so multiple independent offers, checks, and commits per turn never suffer cascading `Sequence`-number failures if one transaction fails.

### Phase 2: Simultaneous Cryptographic Commit Cutoff ("Commit")

- **Sealed order bundle:** Before the turn deadline, each seat submits one cryptographic commitment for its simultaneous discretionary actions:
  - Converter runs (`Tier 2`) and Syndicate Project deliveries (`Tier 3`).
  - Sealed-bid Charter auction bids (Turns 6, 12, 18, 24).
  - Hidden-value trade challenges and escrow preimage handover commits.
  - Council election and tariff votes (Turns 4, 8, 12, 16, 20, 24).
- **On-ledger commit-reveal primitive:** Lock 1 drop (or deal collateral) in `EscrowCreate` with `Condition = PREIMAGE-SHA-256` and a salted SHA-256 digest in `Memos` ($\le 1\text{ KB}$), then reveal by submitting `EscrowFinish` with `Fulfillment` (paying the elevated XRPL fulfillment fee floor, $\ge 330$ drops) at the reveal gate. Pure-state simulation (Part 3) models the identical `(commitment_hash, reveal_payload, salt)` envelope deterministically with no ledger I/O.
- **Standing Regent Delegation (`PermissionDelegationV1_1`):** To prevent absent humans ("Civil Disorder") from starving downstream ring neighbors of $R_i$ or $M_i$, each human seat may pre-authorize a scoped **Standing Regent Agent** via XRPL `DelegateSet` (`PermissionDelegationV1_1`) without exposing its master seed. The Regent is strictly scoped by a 3-slider player Doctrine (risk tolerance, minimum conversion reserve, maximum credit spread) to run upkeep converters, cash maturing Checks/Escrows, and maintain standing quotes. After **3 consecutive missed turn cutoffs**, the seat converts cleanly to a disclosed `SEAT_AI_SYNDICATE` seat.
- **Non-reveal penalty (EIP-4399 rule):** Any seat that fails to submit a valid reveal matching its Phase 2 commitment forfeits its sealed discretionary actions for that turn (falling back to Regent/hold defaults where eligible) and **never triggers a table re-roll**.

### Phase 3: Deterministic Resolution Sweep ("Sweep")

At the turn cutoff, the Turn Engine (pure-state in Part 3; Authority/Steward + ledger transactions on-chain) executes **eight deterministic steps in fixed order**:

1. **Validate Reveals:** Verify each Phase 2 reveal against its Phase 1/2 SHA-256 commitment; mark unrevealed or malformed seats as non-reveals and apply default fallback behavior.
2. **Settle Enforced Swaps, Escrows & Converters (`BatchV1_1` & `TokenEscrow`):**
   - Finish valid `EscrowFinish` reveals and evaluate any expiring Steward-refereed delivery bond escrows using the deterministic pure-function replay rule.
   - Settle accepted bilateral swaps and Tier-2 chiral converter transformations atomically (`tfAllOrNothing`).
3. **Clear Maturing Breakable Credit (`CheckCash`):**
   - Present due `CheckCreate` notes via `CheckCash` (`Amount` / `DeliverMin`) in canonical deterministic ID order.
   - Because `CheckCreate` locks zero funds upfront, if the drawer drained its `CRED` or commodity balance before Sweep, `CheckCash` bounces (`tecUNFUNDED_PAYMENT` / insufficient liquidity): the payee receives zero from that Check, the bounce logs a **Hard Credit Default**, and the drawer's on-ledger credit credential downgrades (`CREDIT_AAA -> CREDIT_BBB -> ... -> CREDIT_D`), raising auto-credit rates toward `30%` and revoking prime cartel domain access.
4. **Apply Tier-3 Syndicate Projects & Extraction Yields:**
   - Burn delivered `Tier 3` project bundles to Authority and credit Project Equity; mint each seat's new turn Raw extraction yield $y_i$ of $R_i$.
5. **Collect Chiral Upkeep & Price Auto-Debt:**
   - Collect each seat's turn upkeep $u_i$ in $M_{(i-1) \bmod N}$ (Authority `Clawback`); convert any shortfall into `CRED` auto-debt at fundamental parity and accrue per-turn debt interest according to the seat's current credit tier.
6. **Apply Perishable Spoilage (`Clawback`):**
   - Read live trust-line balances of remaining perishable commodities and claw back the table's decay fraction (baseline `20%`/turn, integer floor/rounding specified deterministically).
7. **Execute Political Cadences & Charter Rotations:**
   - On turns divisible by **4** (`4, 8, 12, 16, 20, 24`): tally council votes to install the new Trade Minister (updating commodity `TransferRate`, `0%–35%`) and Cartel Chairs (updating `PermissionedDomainSet` member credentials and executing `CredentialDelete` embargoes).
   - On turns divisible by **6** (`6, 12, 18, 24`): settle sealed-bid Syndicate Charter auctions and prune expired `Credentials` (`Expiration` elapsed).
8. **Advance Horizon Shocks & Evaluate Stochastic Close:**
   - Reveal Turn $t$ private `OracleAuditor` hint, narrow Turn $t+1$ public `OracleSet` forecast range, and enact Turn $t+2$ realized shock (supply freeze, tariff holiday, demand surge, or spoilage spike).
   - On **Turns 24–28**, evaluate the public stochastic closing roll; if triggered (or unconditionally at Turn 28), reveal Turn-1 committed Private Charter Mandates and compute final Cardinal Net Equity scores.

---

## 3. Dual-Track Contracts and Two-Part Reputation

Requiring every multi-turn contract to lock 100% collateral freezes illiquid seats, while making every promise unenforceable collapses multi-turn trade into spot barter. Merchant League v2 separates contracts into two explicit tracks:

| Track | Mechanism | On-Ledger Primitive | Collateral & Default Semantics | Reputation Impact |
|---|---|---|---|---|
| **Track 1: Ledger-Enforced** | Atomic multi-leg swaps & converter runs | `BatchV1_1` (`tfAllOrNothing`, up to 8 inner txs) | **Zero default risk:** all legs settle or none do | Honors commitment if executed |
| **Track 1: Ledger-Enforced** | Conditional payments, preimage locks & delivery bonds | `TokenEscrow` (`PREIMAGE-SHA-256`, `FinishAfter`, `CancelAfter`) | **Zero counterparty default risk:** locks non-perishable `CRED` on-ledger; Steward-refereed bonds publish referee secret only on deterministic replayable breach | Fulfilled escrow proves compliance; Steward-adjudicated bond breach penalizes honor |
| **Track 2: Breakable Credit** | Deferred working-capital loans & raw-input credit | `CheckCreate` (`SendMax`, `Expiration`) settled via `CheckCash` | **Zero upfront lockup; defaultable:** drawer can spend liquidity elsewhere before `CheckCash`, causing the Check to bounce | **Hard Credit Solvency:** bounce triggers automatic `CREDIT_AAA -> ... -> CREDIT_D` downgrade |
| **Track 2: Breakable Promises** | Council vote pledges, cartel admission/embargo pacts, preimage handovers | Signed Typed JSON Intent Header (hashed in `Memo`) | **Unenforced on-ledger:** counterparty can break vote or withhold off-ledger preimage | **Soft Diplomatic Honor:** deterministic promise-vs-ledger audit records betrayal $\Delta\text{EV}$ |

Every bilateral deal therefore becomes an explicit economic negotiation over **collateral lockup (`TokenEscrow`) versus breakable credit spread (`CheckCreate`)**, directly creating contract diversity without artificial rules.

---

## 4. Cardinal Net-Equity Scoring, Private Mandates, and PBRS

### 4.1 Terminal Cardinal Net Equity (Replacing Binary Win/Loss)

Every seat $i$ maximizes continuous **Total Equity** at terminal turn $T \in [24, 28]$:
$$\text{Total Equity}_i(T) = \text{Project Equity}_i(T) + \text{Net Asset Equity}_i(T) + \text{Mandate Bonus}_i(T)$$
- **Project Equity:** Irreversible points earned by consuming Tier-3 Refined bundles at Authority during Turns `1` through $T$.
- **Net Asset Equity ($\Phi_i(T)$):** Fundamental mark-to-market value of remaining `CRED`, Raw/Refined commodities (valued at converter/upkeep fundamental shadow parity rather than thin last-turn spot quotes so late wash trades cannot inflate marks), plus collectible collateralized `TokenEscrow` receivables, **minus** unpaid `CRED` auto-debt and outstanding `CheckCreate` credit liabilities.
- **Private Charter Mandates:** On Turn 1, each seat commits a SHA-256 hash of its dealt secret Charter Mandates (graded asymmetric endgame objective weights/bonuses). Revealing a valid mandate at close adds its bonus; an unrevealed mandate scores `0`.

Because every marginal unit of Equity increases final standing and score, trailing seats retain constructive trading incentives through the final turn: punishing a traitor is priced as rational deterrence rather than zero-cost "dead-player" kingmaking.

### 4.2 Anti-Wash Invariance via Potential-Based Reward Shaping (PBRS)

Neither pure activity proxies (trade count, DEX volume, message count—which trigger ping-pong wash trading) nor purely sparse terminal win/loss rewards suffice for stable training. v2 defines dense intermediate shaping via **Potential-Based Reward Shaping (PBRS)**:
$$F_i(s_t, a_t, s_{t+1}) = \gamma\,\Phi_i(s_{t+1}) - \Phi_i(s_t)$$
where $\Phi_i(s_t)$ is Seat $i$'s fundamental mark-to-market Net Equity at state $s_t$. By Ng, Harada, and Russell (1999), PBRS preserves optimal policy invariance and guarantees every zero-surplus wash cycle ($A \leftrightarrow B$) or self-bidding loop telescopes to $\le 0$ net shaped return (strictly negative once `TransferRate` tariffs and transaction fees apply), while legitimate converter runs and trade surpluses receive immediate dense credit.

---

## 5. Politics: Tariffs, Cartels, and Rotating Institutional Charters

### 5.1 Quadrennial Council Elections (Every 4 Turns: Turns 4, 8, 12, 16, 20, 24)

Seats vote simultaneously in Phase 2 (weighted by rule-defined franchise—equal seat vote or Project-Equity/credit-gated franchise as parameterized by the chapter pack) to fill two political offices for the next 4-turn term:
1. **Trade Minister (Tariff Authority):** Sets per-commodity export tariffs (`0%` to `35%`) applied on-ledger by Authority via issuer `AccountSet` `TransferRate`. Tariffs tax external settlement flows and transfer tariff incidence between net exporters and importers without blocking trade altogether.
2. **Cartel Chairs (Bloc Governance):** Govern cartel trade blocs implemented on-ledger via `PermissionedDomainSet` (`1` to `10` `AcceptedCredentials`) and `PermissionedDEX` (`OfferCreate` carrying `DomainID`).
   - Within a cartel domain, members trade at reduced or zero bloc tariff surcharges without crossing open-book offers.
   - **Instant Embargo (`CredentialDelete`):** A Cartel Chair (or council majority rule) can revoke a target seat's cartel credential via `CredentialDelete`, immediately invalidating that rival's resting domain offers (`tecNO_PERMISSION`) on subsequent fills.

### 5.2 Rotating Syndicate Charters (Every 6 Turns: Turns 6, 12, 18, 24)

Every 6 turns, seats submit simultaneous sealed equity/`CRED` bids for time-bounded institutional **Syndicate Charters**, issued on-ledger as `Credentials` carrying explicit `Expiration` timestamps (automatically invalidated upon domain checks once expired):
1. **Clearinghouse Broker:** Holds the `tfHybrid` cross-domain bridge credential, enabling `OfferCreate` with `tfHybrid` to arbitrage and bridge liquidity between rival cartel `PermissionedDomains` and the open `CRED` book.
2. **Customs Inspector:** Receives deterministic audit rights to flag and trigger rule-governed temporary trade freezes on suspected negative-BATNA wash transfers or tariff evasion.
3. **Oracle Auditor:** Receives the Turn $t$ private `Horizon Shock` preimage/hint encrypted to its registered 33-byte `AccountSet` `MessageKey` and hash-anchored inside its `Credential` `URI`. The Oracle Auditor can **sell the cryptographically verifiable preimage** for `CRED` or goods, or **bluff with unverified rumors**.

---

## 6. Horizon Shocks and the 24–28 Turn Season with Random Close

### 6.1 Three-Stage Telegraphed Horizon Shocks (`OracleSet`)

Unannounced shocks add pure lottery variance, while static 24-turn tables solidify into permanent "Blue Donut" non-aggression pacts. v2 uses **3-stage telegraphed crises** anchored on-ledger by Authority via `OracleSet` (`PriceOracle`):
- **Turn $t$ (Private Foresight):** Authority anchors the hidden shock seed hash and delivers the verifiable preimage/hint privately to the current **Oracle Auditor** (encrypted via `MessageKey`).
- **Turn $t+1$ (Narrowed Public Forecast):** Authority publishes a narrowed probabilistic range (commodity sector, direction, bounded magnitude) via `OracleSet` so all seats can hedge via `TokenEscrow`, `CheckCreate`, and inventory positioning.
- **Turn $t+2$ (Realized Outcome):** Authority commits the realized outcome via `OracleSet` and executes the shock deterministically:
  - *Supply Freeze:* temporary trust-line freeze / reduced Turn $t+2$ extraction yield on one $R_k$.
  - *Tariff Holiday:* temporary `TransferRate` reset to `0%` on a targeted commodity.
  - *Demand Surge:* elevated Project Equity multiplier for specific Refined bundles for 2 turns.
  - *Spoilage Spike:* temporary elevation of `Clawback` decay on one raw good.

**Randomness Source:** In Game Mode on public Testnet, shock draws and closing rolls consume pre-named **drand quicknet** rounds (`round = floor((close - 1692803367) / 3) + 1`, never falling back to steerable ledger hashes if drand stalls). In Headless Evaluation Mode, mirrored duplicate tables share identical pre-sampled deterministic PRNG shock schedules across all seat rotations to isolate agent skill.

### 6.2 24–28 Turn Season with Stochastic Closing Roll

Every season runs between **24 and 28 simultaneous turns**, preventing both Rath-style extreme-horizon drift and deterministic final-turn betrayal math:
- **Turns 1–23:** Season remains open unconditionally.
- **Turns 24–27:** At Step 8 of Sweep, a public `roll`-validated probability table (`tables/`) evaluated against the turn's beacon/seed determines whether the Closing Bell rings immediately (exact per-turn schedule fixed in `tables/` and audited in CI; e.g., escalating hazard probabilities across Turns 24–27).
- **Turn 28:** Unconditional season close. All unrevealed Turn-1 Private Charter Mandates reveal (or score `0`), and terminal Cardinal Net Equity is finalized.

---

## 7. Game Mode vs. Headless Training and Evaluation Mode

### 7.1 Shared Core

Both modes share the exact same:
- Chiral converter math, LP-verified table schema, and 24–28 turn horizon (`league-core` / Part 2–3 crates);
- Simultaneous Talk & Market -> Commit -> Sweep turn rules (`league-core` Part 3);
- Dual-track contracts (`BatchV1_1`, `TokenEscrow`, `CheckCreate`), credit tiers (`CREDIT_AAA`..`CREDIT_D`), politics (`TransferRate`, `PermissionedDomains`, `Credentials`), `OracleSet` shocks, and Cardinal Equity + PBRS accounting;
- Identical C++ `xrpld` transaction validation rules (when executing against live Testnet in Game Mode or containerized Stand-Alone `xrpld` in Training/Eval Mode).

### 7.2 Game Mode (Public XRPL Testnet — Human and Mixed Tables)

- **Pacing Presets:**
  - *Season Mode:* Two 12-hour simultaneous turns per day (`~12–14` real-time days for a 24–28 turn season), auto-advancing early whenever all seats submit orders, paired with a deterministic "Time Machine" order preview in `league-tui` so players inspect exact post-Sweep outcomes before committing.
  - *Live Club Mode:* 10-minute turns for synchronous evening sessions (~4–4.5 hours for a full season).
- **Diegetic Interface and Silent Faucet Funding:** UI presents ledger mechanics strictly in strategic terms (`Sealed Vault Contract`, `Syndicate Credit Line`, `Cartel Charter`, `Clearinghouse Tariff`), paired with clickable `[Verified on XRPL Ledger #...]` proof links. Accounts fund once per season via the Testnet faucet (`1,000` Testnet XRP default grant vs. the report's modeled estimate of `~36.8 XRP` total base + owner reserves across 1 Authority + 6 seats, 42 trust lines, and ~90 active ledger objects—an estimate to measure during live on-ledger tests).
- **Automated Phoenix State Rehydrator (Surviving Testnet Wipes):** Because public Testnet resets delete all accounts, balances, order books, escrows, checks, credentials, and domains, an off-ledger event-sourced Archivist mirror subscribed to Testnet WebSocket streams automatically detects `actNotFound` on Authority and rebuilds the exact table deterministically across 3 dependency waves using parallel `BatchV1_1` (up to 8 inner txs per batch) and pre-allocated `Tickets`:
  1. *Wave 1 (Accounts, Flags & Gates):* Re-fund deterministic Authority and seat addresses, allocate `TicketCreate` pools, restore Authority flags (`RequireAuth` -> `Clawback` -> `Locking` -> `DefaultRipple`), recreate `PermissionedDomainSet` domains, `CredentialCreate` objects, and seat `TrustSet` lines.
  2. *Wave 2 (Credential Accepts & Balances):* Submit seat `CredentialAccept` batches and mint exact pre-wipe `CRED`, $R_i$, and $M_i$ balances from Authority.
  3. *Wave 3 (Open Contracts & Order Books):* Re-post resting `OfferCreate` orders, active `CheckCreate` lines, and `TokenEscrow` vaults with adjusted remaining `FinishAfter`/`CancelAfter` offsets, remapping new object hashes transparently in the client UI. (The report estimates `~95` transactions across `~12` parallel batches finishing in `~12–15` seconds / 3 ledger closes; verify empirically in the future Phoenix test.)
- **AI Identity Disclosure and Fair-Play Guardrails:**
  - *Standard Queues:* Seats are openly tagged on-ledger via `SEAT_HUMAN` vs. `SEAT_AI_SYNDICATE` `Credentials` (aligned with EU AI Act Article 50(1) transparency obligations—flagged for external legal verification prior to any public EU launch).
  - *Persona Calibration:* Game Mode AI agents use conditionally fair, inequity-averse reciprocal policies that honor collateralized deals and retaliate proportionally (via pricing spreads and cartel embargoes) against betrayal, avoiding both exploitable altruism and passive holds.
  - *Masked Syndicate Queue (Opt-In):* A separate opt-in queue where all entrants explicitly consent pre-match that seats are a mix of humans and AI whose `SEAT_HUMAN`/`SEAT_AI_SYNDICATE` tags unseal only at Turn $T$.
  - *Three AI Guardrails:* (a) hard Best-Alternative-To-Negotiated-Agreement (BATNA) Economic Sanity Gate blocking sub-parity giveaways unless backed by `TokenEscrow` collateral or credit history; (b) 10–15 minute human-first resting window in Season Mode before AI sweeps newly posted human DEX offers; (c) action-hierarchy prompting + `piKL` anchoring calibrated to table skill, plus natural 3–20 minute chat reply latency and `<120`-word replies.
- **Security and Integrity in Mixed Play:**
  - *Indirect Prompt Injection Defense:* All XRPL `Memos` ($\le 1\text{ KB}$) and counterparty chat strings are treated as untrusted escaped input. Signing keys live strictly in an isolated external signer/vault outside LLM memory, and every proposed transaction passes the deterministic Economic Sanity Gate before signing.
  - *Anti-Sybil & Anti-Collusion:* Ranked queues mask player handles behind randomized seasonal Syndicate aliases until season close, require verified matchmaking `Credentials`, and flag dyads exhibiting persistent negative-BATNA `CheckCreate`/`OfferCreate` wealth funnels.
  - *GDPR / Transcript Privacy:* Off-ledger chat requires explicit opt-in consent separate from Terms of Service before any training use and undergoes PII scrubbing; raw chat never enters public XRPL `Memos`.
- **Mode B Playtest Metrics (First Human/Mixed Validation, 6–10 Tables, 7-Day / 14-Turn Test):**
  1. *Session Cadence & Sleep Health:* Median check-ins/day (prior target: 2–4 sessions of 6–12 min), local `01:00–06:00` login share (red flag `>10%`), Standing Regent missed-turn activations (prior target `<8%` of human seat-turns).
  2. *Perceived Decision Density:* Empty logins (`>60 s` viewing without action or message) paired with one-click turn pulse (`Meaningful / Routine / Nothing to do`; red flag if `>20%` report "Nothing to do").
  3. *Machine Penalty & Exploitation Trajectory:* Compare `Human -> Human` vs. `Human -> Disclosed AI` surplus splits, Check bounce rates, and cartel inclusion in Rounds 1–5 vs. Rounds 10–14 (flag if humans exploit AI `>15` percentage points more without AI retaliation closing the gap).
  4. *Ledger Invisibility & Wipe Recovery:* `0%` unhandled `tec`/`tem` errors surfaced to users, plus one live unannounced Testnet state migration testing Phoenix rehydration speed and integrity.
  - *Human Ranking Display:* Bayesian TrueSkill tiers displayed to players.

### 7.3 Headless Training and Evaluation Mode (Containerized Stand-Alone `xrpld`)

- **Local Execution Substrate:** Containerized `xrpld -a --start` running on loopback in Stand-Alone Mode, exempt from public IP rate limits, starting with 100B genesis test XRP, forcing required Mainnet-matched amendments locally, and closing ledgers deterministically in milliseconds via admin `ledger_accept`. *(Hard rig rule: no container runs on the local rig until the Publisher approves its CPU/RAM limits; large-scale training sweeps belong on Colab CPU later.)*
- **League Training Stack (Transferable Competence Without Exploits):**
  - *Population Diversity (PSRO + Fictitious Co-Play):* Train $\ge 6$ independent model lineages/goal priors, freezing early-, mid-, and late-stage checkpoints into a shared partner pool.
  - *Dedicated League Exploiters (`20%–25%` of training seats):* Seed opportunistic defectors (cooperative until rivals demilitarize/over-extend, then defection pivot), high-anchor / "desperate" persona manipulators, and deterministic ledger-tracking code bots (`TrackerBot` and `OpportunistBot`).
  - *Human-Anchored Regularization (`piKL`):* Anchor policy $\pi_i$ to natural-English reference $\tau_i$ via $U_i(\pi_i, \pi_{-i}) = u_i(\pi_i, \pi_{-i}) - \lambda D_{\text{KL}}(\pi_i \parallel \tau_i)$ to prevent drift into private unintelligible codes or degenerate exploits.
  - *Reward & Process Safety:* Combine terminal Cardinal Equity with PBRS ($\Phi_i$). **Never apply RL optimization gradients to private Chain-of-Thought (CoT) scratchpads**; monitor unpenalized CoTs asynchronously with external auditors/probes strictly to patch game-rule loopholes. Pass outbound free-text chat on `50%` of training/evaluation tables through a lightweight semantic paraphraser (preserving Typed JSON Intent Headers verbatim) to strip syntactic steganography channels.
- **Context Engineering & Drift Control:**
  - Replace raw append-only JSON/chat logs with a fresh `~800`-token canonical XRPL ledger snapshot each turn, stripping obsolete raw RPC dumps.
  - Every `5` turns, run Episodic Memory Consolidation (EMC) into a structured `COUNTERPARTY_DOSSIER` combined with Adaptive Behavioral Anchoring (ABA). (The report models this stack cutting 40-turn 6-seat prompt volume from `23.90M` to `3.29M` input tokens (`-86.2%`) and cites Rath's `73`-interaction enterprise-workflow drift study; measure actual Agent Stability Index and token counts directly in Parts 4+.)
- **Held-Out Evaluation & Anti-Contamination Protocol:**
  - *Mirrored Duplicate Seeds + AIVAT Control Variates:* Evaluate candidate models across seat-swapped duplicate tables sharing identical seeds and shock schedules, reducing variance relative to raw binary win/loss (report prior: `~22` games vs. `372` for binary win/loss at `80%` power, $\alpha = 0.05$—a power hypothesis to measure empirically).
  - *Held-Out Resident & Visitor Suites:* Pair focal policies against pinned open-weight Reference Background Populations (fixing checkpoints, quantization, and seeds against API deprecation) plus `TrackerBot` and `OpportunistBot` on unseen chiral table parameters in both **Resident Mode** ($N-1$ focal copies + 1 unseen background bot) and **Visitor Mode** (1 focal agent + $N-1$ unseen background bots).
  - *Skill, Epistemic & Contamination Controls:* Rank populations via **Coarse Correlated Equilibrium (CCE) Deviation Ratings** (avoiding Elo clone inflation and cyclic distortions). Measure non-invasive out-of-context Belief Brier / ECE scores over rivals' hidden mandates/votes and deterministic QUACK-style factual audits comparing chat claims to ledger logs. Generate quarterly evaluation seeds from post-cutoff entropy, keep held-out background weights server-side, and randomize 3-character currency codes, region names, and charter labels per run.
- **Mode A Playtest 1 Metrics (30–50 Mirrored 24-Turn Games, Engineering Priors to Calibrate):**

| # | Axis | Operational Metric | Initial Prior (Hypothesis) | Red Flag & Root Cause |
|---|---|---|---|---|
| 1 | Trade Dispersion | Dyadic Trade HHI & Effective Dyads (`1 / HHI`) | $\text{HHI} < 0.18$ ($\ge 5.5$ of `15` pairs active in 6-seat) | $\text{HHI} > 0.35$: bilateral lock-in persists; tighten ring or lower DEX friction |
| 2 | Contract Diversity | % cleared value settled via `TokenEscrow`, `CheckCash`, `PermissionedDEX` vs. spot `OfferCreate` | `25%–55%` of cleared value | `<10%`: starting liquidity too generous, removing need for credit or vaults |
| 3 | Turn Agency | % seat-turns executing converters, price-improving quotes, contracts, or votes vs. passive Holds | `>80%` active; `<15%` pure Hold | `>40%` Hold: passivity bias or capital lockup starving agency |
| 4 | Skill & Generalization | CCE Deviation Rating, Duplicate Equity Lift over `TrackerBot`, Visitor / Resident score ratio | Positive lift over `TrackerBot`; Visitor/Resident `> 0.75` | Losing to `TrackerBot` (ledger blindness) or Visitor/Resident `< 0.50` (convention overfitting) |
| 5 | Promise vs. Ledger | Check solvency %, Escrow preimage fulfillment %, vote alignment, Betrayal $\Delta\text{EV}$ | Check solvency `> 85%`; broken promises have $\Delta\text{EV} > 0$ | Negative-EV broken promises (amnesia/hallucination) or `100%` gullibility |
| 6 | Epistemic Calibration | Out-of-context Belief Brier / ECE + Ledger Claim Hallucination % | Brier `< 0.22`; claim hallucinations `< 3%` | Claim hallucinations `> 10%`: noisy state serialization in prompts |
| 7 | Drift & Inference Cost | Agent Stability Index (ASI), XRPL `tem`/`tec` retry rate, Net Equity per unit of inference budget | ASI $\ge 0.80$ through Turn 28; invalid tx `< 2%` | ASI `< 0.75` or late bankrupt bids: context rot requiring tighter compaction |

---

## 8. XRPL Features and Live Mainnet Amendment Status

Checked directly via read-only `feature` RPC on **2026-11-15** against **XRPL Mainnet** (`network_id = 0`, validated ledger `#108,341,694`) and **XRPL Testnet** (`s.altnet.rippletest.net`, `network_id = 1`, `rippled 3.4.1`, validated ledger `#22,340,126`). Live Mainnet status satisfies the `docs/design.md` / `CLAUDE.md` hard invariant requiring Mainnet-enabled features only.

| XRPL Feature / Primitive | v2 Role | Canonical Amendment Name(s) (`feature` RPC) | Mainnet (`network_id = 0`, 2026-11-15) | Testnet (`s.altnet.rippletest.net`, `3.4.1`, 2026-11-15) | League Live Proof / Next Check |
|---|---|---|---|---|---|
| Issued Tokens & Trust Lines (`TrustSet`, `Payment`, `RequireAuth`, `DefaultRipple`, `NoRipple`, `lsfAllowTrustLineLocking`, `asfDisallowXRP`) | `CRED` numeraire, Raw $R_i$, Refined $M_i$, per-table custody isolation | Base protocol (`TrustSetAuth`, `fixTrustLinesToSelf`) | `TrustSetAuth`: `enabled=true`; `fixTrustLinesToSelf`: `enabled=true` | `TrustSetAuth`: `enabled=false, supported=true` (retired in 3.4.1); `fixTrustLinesToSelf`: `enabled=true` | Proved live in M2, M2c, RequireAuth check (`docs/receipts/`) |
| DEX Order Book (`OfferCreate`, `OfferCancel`, `Memos`, `AccountSet` `TransferRate` & `MessageKey`) | `CRED`-priced market quotes, memo hash anchors, export tariffs (`0%–35%`), public encryption keys | Base protocol (core) | Active (base ledger rules) | Active (base ledger rules) | Offers/Memos proved in M2/M2c; `TransferRate` & `MessageKey` to spike before Politics milestone |
| Atomic Batch Transactions (`XLS-56`, `tfAllOrNothing`, up to 8 inner txs) | Converter settlement, multi-leg atomic swaps, 3-wave Phoenix state rehydration | `BatchV1_1` (note: legacy `Batch` and `fixBatchInnerSigs` are `enabled=false` on Mainnet) | `BatchV1_1`: `enabled=true, supported=true` | `BatchV1_1`: `enabled=true` (`fixBatchV1_2` also `enabled=true` on Testnet only) | **Director allowed 2026-11-15.** Spike `BatchV1_1` (`tfAllOrNothing`) before relying on `fixBatchV1_2`-specific rules |
| Token Escrow (`XLS-85`, `PREIMAGE-SHA-256`, `FinishAfter`, `CancelAfter`) | Track 1 `CRED` collateral vaults, Steward-refereed bonds, commit-reveal locks | `TokenEscrow`, `fixTokenEscrowV1`, `Escrow` | `TokenEscrow`: `enabled=true`; `fixTokenEscrowV1`: `enabled=true`; `Escrow`: `enabled=true` | `TokenEscrow`: `enabled=true`; `fixTokenEscrowV1`: `enabled=true`; `Escrow`: `enabled=false, supported=true` | Proved live in M2c & escrow-expiry check (`docs/receipts/`) |
| Permissioned Domains & DEX (`XLS-80`, `XLS-81`, `DomainID`, `tfHybrid`) | Table-isolated markets, Cartel trade blocs, Clearinghouse Broker `tfHybrid` bridging | `PermissionedDomains`, `PermissionedDEX` | `PermissionedDomains`: `enabled=true`; `PermissionedDEX`: `enabled=true` | `PermissionedDomains`: `enabled=true`; `PermissionedDEX`: `enabled=true` | `DomainID` & refusal proved in M2c; spike `tfHybrid` before Politics milestone |
| Credentials (`XLS-70`, `CredentialCreate`, `CredentialAccept`, `CredentialDelete`, `Expiration`, `URI`) | Seat identity (`SEAT_HUMAN` / `SEAT_AI_SYNDICATE`), credit tiers (`CREDIT_AAA`..`CREDIT_D`), cartel embargoes, 6-turn expiring charters | `Credentials` | `Credentials`: `enabled=true, supported=true` | `Credentials`: `enabled=true, supported=true` | Create/Accept proved in M2c; spike `CredentialDelete` & `Expiration` |
| Permission Delegation (`XLS-75`, `DelegateSet`) | Scoped Standing Regent backup for absent human seats without master seed | `PermissionDelegationV1_1` (note: legacy `PermissionDelegation` and `fixDelegateV1_1` are `enabled=false` on Mainnet) | `PermissionDelegationV1_1`: `enabled=true, supported=true` | `PermissionDelegationV1_1`: `enabled=true, supported=true` | Spike `PermissionDelegationV1_1` before Regent milestone |
| Price Oracles (`XLS-47`, `OracleSet`, `OracleDelete`) | Canonical 3-stage telegraphed Horizon Shock forecasts and realized shock records | `PriceOracle`, `fixPriceOracleOrder` | `PriceOracle`: `enabled=true`; `fixPriceOracleOrder`: `enabled=true` | `PriceOracle`: `enabled=true`; `fixPriceOracleOrder`: `enabled=true` | Spike `OracleSet` before Shocks milestone |
| Clawback (`XLS-39`, `lsfAllowTrustLineClawback`) | Automated perishable spoilage (`20%`/turn) and upkeep collection after balance read | `Clawback`, `DeepFreeze`, `AMMClawback`, `fixAMMClawbackRounding` | All four `enabled=true, supported=true` | All four `enabled=true, supported=true` | Proved live in M2 & M2c (`AMMs` remain unused) |
| Checks (`CheckCreate`, `CheckCash`, `CheckCancel`) | Track 2 uncollateralized working-capital credit lines (`CREDIT_AAA`..`CREDIT_D` bounce enforcement) | `Checks`, `CheckCashMakesTrustLine`, `fixCheckThreading` | All three `enabled=true, supported=true` | All three `enabled=true, supported=true` | Proved live in M2/M2c |
| Ticket Batch (`TicketCreate`, `Sequence: 0`) | Parallel non-stalling turn orders and 3-wave Phoenix recovery | `TicketBatch` | `TicketBatch`: `enabled=true, supported=true` | `TicketBatch`: `enabled=true, supported=true` | Spike `TicketCreate` with `BatchV1_1` |
| **Excluded:** Multi-Purpose Tokens (`MPTokensV1`, `MPTokensV2`, `DynamicMPT`) | Not used for tradable goods (`MPTokensV1` lacks DEX `OfferCreate`/AMM support) | `MPTokensV1`, `MPTokensV2`, `DynamicMPT` | `MPTokensV1`: `enabled=true`; `MPTokensV2`: `enabled=false`; `DynamicMPT`: `enabled=false` | `MPTokensV1`: `enabled=true`; `DynamicMPT`: `enabled=false` | Excluded; all goods/coin stay Trust Line IOUs |
| **Forbidden:** Confidential Transfers (`XLS-96`) | Optional Devnet proposal mentioned in report Section 5 Layer 3 | `ConfidentialTransfer` | `enabled=false, supported=true` | `enabled=false, supported=true` | **Forbidden** by Mainnet-only rule; use off-ledger + `MessageKey` + hash-anchored `URI`/`Memo` |

**Mandatory operational check:** Re-run the `feature` RPC check on both Testnet and Mainnet immediately before and immediately after the scheduled **2026-11-25** Testnet reset (`league-season::KNOWN_RESETS`), verifying in particular that `BatchV1_1`, `PermissionDelegationV1_1`, `PriceOracle`, `TokenEscrow`, `PermissionedDEX`, `Credentials`, `Checks`, `Clawback`, and `TicketBatch` remain active after Testnet's reset alignment.

---

## 9. Conflicts with `docs/design.md` and Decisions for the Director

This section enumerates every conflict between Merchant League v2 (the seed-26 Chiral Concordat design) and the prior repository design documents (`docs/design.md` on `design/redesign-r1` PR #6, `docs/design.md` on `main`, and `CLAUDE.md` / `docs/paper-test.md`). Conflict 1 is already decided; **Conflicts 2 through 13 require explicit Director rulings** before Parts 2–4 and later on-ledger milestones rely on the affected rule.

| # | Conflict Area | Prior Design (`docs/design.md` / `CLAUDE.md`) | Merchant League v2 (Seed-26 Report) | Status / Director Decision Needed |
|---|---|---|---|---|
| **1** | **Atomic Batch (`BatchV1_1`, XLS-56)** | PR #6 `docs/design.md:17,137,145,398,408` chose two plain payments with bonds and ruled "no Batch for v0.1". | Uses `BatchV1_1` (`tfAllOrNothing`) for atomic barter, converter settlement, and 3-wave Phoenix rehydration. | **DECIDED (Director, 2026-11-15):** Batch is **allowed**. Verified `BatchV1_1` `enabled=true` on Mainnet (legacy `Batch` is `false`; spike `BatchV1_1` explicitly). |
| **2** | **Spatial 5-Town Caravan Travel vs. Non-Spatial Chiral Ring** | PR #6 `docs/design.md:5,56-94,154-163` (`main` `docs/design.md:20-27`): 5 towns joined by roads, sealed travel orders, road events, and multi-day caravan travel. | Removes spatial travel and road transit altogether; places $N$ seats on a non-spatial directed chiral converter ring to eliminate dead transit turns and capital lockups. | **[DECISION FOR THE DIRECTOR]:** Approve replacing 5-town spatial travel/roads with the non-spatial directed chiral ring? |
| **3** | **4–5 of 6 Fixed Guilds vs. 4–6 Procedural Chiral Seats** | PR #6 `docs/design.md:5,19,38,56,71-76,411`: fixed 6 guilds (`Saltworks`..`Tanners`), seating 4 or 5 per table with 1–2 sitting out; "six-player table ... isn't in v0.1." | Parameterized $N \in \{4, 5, 6\}$ seats on a rotationally symmetric chiral converter ring with modular `Credentials` and LP feasibility proof per seed. | **[DECISION FOR THE DIRECTOR]:** Approve $N \in \{4, 5, 6\}$ procedurally parameterized chiral rings (including 6-seat tables) over 4–5 of 6 fixed guilds? |
| **4** | **Town Maker Ladders vs. Zero-Autarky Markets + Auto-Debt** | PR #6 `docs/design.md:74,198-202`: Steward posts town maker price ladders as raw sources and fallback sinks (even when a guild sits out). | Removes town maker fallback ladders so no seat or pair can retreat into solitaire autarky; adds `CRED` auto-debt (`4%–30%` by credit tier) to cushion shortfalls. | **[DECISION FOR THE DIRECTOR]:** Approve removing Steward town maker ladders in favor of strict multilateral ring markets + tiered `CRED` auto-debt? |
| **5** | **Checks Dropped vs. Re-Added as Track 2 Breakable Credit** | PR #6 `docs/design.md:30,397,443`: explicitly dropped `Checks` ("no longer used") in favor of `TokenEscrow` bonds only. | Re-adds `Checks` deliberately alongside `TokenEscrow` as uncollateralized Track 2 credit where `CheckCash` bounces trigger `CREDIT_AAA -> ... -> CREDIT_D` credential downgrades. | **[DECISION FOR THE DIRECTOR]:** Approve re-enabling `Checks` as breakable Track 2 credit paired with `CREDIT_AAA`..`CREDIT_D` credentials? |
| **6** | **Works-Only Score vs. Cardinal Net Equity + PBRS** | PR #6 `docs/design.md:29,98-124`: Score = Works + Mandates only (`coin`, `goods`, and `standing` score `0`); `standing` only discounts bonds/tolls. | Continuous **Cardinal Net Equity** (Project Equity + fundamental mark-to-market Net Asset Equity + Private Mandates) + PBRS ($\Phi_i$), plus separating Hard Credit Solvency from Soft Diplomatic Honor. | **[DECISION FOR THE DIRECTOR]:** Approve continuous Cardinal Net Equity (including fundamental-parity remaining inventory/`CRED` minus debt) over pure Works + Mandates scoring? |
| **7** | **10–13 Day Reset-Terminal Tables vs. 24–28 Turn Seasons + Phoenix Rehydrator** | PR #6 `docs/design.md:33-34,56,194,275-294` (`CLAUDE.md:3`, `main` `docs/design.md:32,107-123`): 10–13 day tables; Testnet resets end chapters/eras early via `league-season` wipe handling. | 24–28 simultaneous-turn seasons (`Turns 24–28` random close) sustained across Testnet resets via event-sourced Archivist mirror + 3-wave `BatchV1_1` + `TicketCreate` **Phoenix State Rehydrator**. | **[DECISION FOR THE DIRECTOR]:** Approve 24–28 turn seasons and mid-table Phoenix rehydration across Testnet wipes (replacing reset-ends-the-season for active tables)? |
| **8** | **Static Single Domain & Day 4/8 Edicts vs. Elected Politics, Cartels, Charters & Horizon Shocks** | PR #6 `docs/design.md:196,216,389-399`: one static Steward `PermissionedDomain` per table and pre-drawn Day 4/8 edicts; no `TransferRate`, no `OracleSet`, no `Credential` expiry. | Every 4 turns: elected Trade Minister (`TransferRate` `0%–35%`) and Cartel Chairs (`PermissionedDomains`, `CredentialDelete`, `tfHybrid`). Every 6 turns: sealed-bid expiring Charter `Credentials`. Continuous 3-stage `OracleSet` Horizon Shocks. | **[DECISION FOR THE DIRECTOR]:** Approve adding elected `TransferRate` tariffs, multi-domain cartels (`CredentialDelete`/`tfHybrid`), 6-turn expiring Charter `Credentials`, and `OracleSet` Horizon Shocks? |
| **9** | **Local Daemon Standing Orders vs. `PermissionDelegationV1_1` Regent** | PR #6 `docs/design.md:47,310,422`: player's local daemon handles standing orders; explicitly deferred `PermissionDelegation` pending Mainnet check. | Authorizes scoped **Standing Regent** via `PermissionDelegationV1_1` (`DelegateSet`) per 3-slider player Doctrine, converting to disclosed AI after 3 consecutive misses. (`PermissionDelegationV1_1` verified `enabled=true` on Mainnet.) | **[DECISION FOR THE DIRECTOR]:** Approve adopting `PermissionDelegationV1_1` (`DelegateSet`) for the Standing Regent (with 3-miss conversion to `SEAT_AI_SYNDICATE`)? |
| **10** | **Separate Human/Bot Leagues vs. Disclosed Mixed Tables + Masked Queue** | PR #6 `docs/design.md:311`: "Ranked tables are for invited humans. Bots play in a separate open league with its own hall of fame." | Mixes disclosed AI (`SEAT_AI_SYNDICATE`) directly into human Game Mode tables with reciprocal fair personas, BATNA/resting guardrails, plus a separate opt-in Masked Syndicate Queue (with EU AI Act Art. 50(1) caveat flagged). | **[DECISION FOR THE DIRECTOR]:** Approve disclosed mixed Human + AI Game Mode tables (and opt-in Masked Syndicate Queue) vs. keeping human and bot ranked tables strictly separated? |
| **11** | **Strict `network_id == 1` Rule vs. Stand-Alone `xrpld` Training Mode** | PR #6 `docs/design.md:359` & `CLAUDE.md:9-11`: every ledger client connects strictly to Testnet endpoints and refuses any `network_id != Some(1)` before sending a second request. | Headless Training & Eval Mode runs on local containerized **Stand-Alone `xrpld`** (`xrpld -a --start`, loopback, `ledger_accept`), which is neither public Testnet nor Mainnet (`network_id = 0`). | **[DECISION FOR THE DIRECTOR]:** Approve adding a compile- or type-isolated `StandAlone` loopback harness (refusing `network_id == Some(0)`, `None`, and any non-loopback host, alongside `scripts/check-testnet-only.sh`) while keeping `league-ledger::Testnet` strictly `network_id == Some(1)`? |
| **12** | **Live `drand` + `Memo` Travel Commits vs. Dual Randomness + `EscrowCreate` Commits** | PR #6 `docs/design.md:32,226-244`: live `drand` quicknet round required for every table day; travel committed via `Memo` hash only (freeing dice escrows). | Game Mode uses `drand` quicknet (`tables/` + `roll`); Headless Training/Eval uses shared deterministic PRNG seeds across mirrored duplicate tables; simultaneous Phase 2 sealed bundles use 1-drop/collateralized `PREIMAGE-SHA-256` `EscrowCreate` + `EscrowFinish` (`>=330` drops) + `Memo` hashes. | **[DECISION FOR THE DIRECTOR]:** Approve shared deterministic PRNG shock/bell seeds for Headless Eval Mode (retaining `drand` in Game Mode) and 1-drop `PREIMAGE-SHA-256` commit escrows for sealed turn bundles? |
| **13** | **6-Game Manual Paper Test Gate vs. Automated Mode A / Mode B Playtest Suites** | PR #6 `docs/design.md:22,322-352` & `docs/paper-test.md`: AI crew plays 6 hand-refereed paper games scored on 8 paper metrics before rules go back to the Director and before building code. | Evaluates v2 across **Mode A** (30–50 mirrored 24-turn headless games with `TrackerBot` / `OpportunistBot`, 7 metrics including HHI, contract share, active rate, CCE, promise audit, Brier, ASI) and **Mode B** (6–10 mixed 14-turn tables, 4 experiential/machine-penalty/Phoenix metrics). | **[DECISION FOR THE DIRECTOR]:** Confirm retiring or folding the remaining paper-test plan (`docs/paper-test.md`) once Parts 2–4 stand up automated Mode A / Mode B playtests? |

---

## 10. Sources and Provenance

- **Seed-26 Blueprint:** `E:/AI/rnd-local/projects/deep-research/2026-11-15-26-league-game-and-training-REPORT.md` (*Designing a Dual-Purpose On-Ledger Strategy Game and LLM Multi-Agent Benchmark*) and its four diagrams (`1-economy-topology-and-benchmark-power.png`, `2-llm-baselines-catan-cattle-diplomacy.png`, `3-xrpl-primitives-architecture.png`, `4-drift-and-token-budget.png`).
- **Repository Rules and Prior Specs:** `CLAUDE.md`, `docs/design.md` (`main` and `design/redesign-r1` PR #6), `docs/paper-test.md`, and live Testnet receipts under `docs/receipts/` (`m2-xrpl-rust-spike-2026-11-14/`, `m2b-own-layer-proof-2026-11-14/`, `m2c-patched-xrpl-rust-proof-2026-11-14/`, `design-check-escrow-expiry-2026-11-14/`, `design-check-requireauth-2026-11-14/`).
- **Live Ledger Verification (2026-11-15):** Read-only `server_info` and `feature` JSON-RPC queries against XRPL Mainnet (`network_id = 0`, ledger `#108,341,694`) and XRPL Testnet (`s.altnet.rippletest.net`, `network_id = 1`, `rippled 3.4.1`, ledger `#22,340,126`).
- **XRPL Protocol References:** [Testnet and Devnet reset notice (Nov 2026)](https://xrpl.org/blog/2026/testnet-devnet-reset), [Stand-Alone Mode](https://xrpl.org/docs/infrastructure/testing-and-auditing/start-a-new-genesis-ledger-in-stand-alone-mode), [Escrow / TokenEscrow](https://xrpl.org/docs/concepts/payment-types/escrow), [Checks](https://xrpl.org/docs/concepts/payment-types/checks), [Permissioned DEXes](https://xrpl.org/docs/concepts/tokens/decentralized-exchange/permissioned-dexes), [Auto-bridging](https://xrpl.org/docs/concepts/tokens/decentralized-exchange/autobridging), [Clawback](https://xrpl.org/docs/concepts/tokens/fungible-tokens/clawing-back-tokens), [Batch XLS-56](https://xls.xrpl.org/xls/XLS-0056-batch.html), and [Agentic Transactions](https://xrpl.org/docs/agents/getting-started-with-agentic-transactions).
