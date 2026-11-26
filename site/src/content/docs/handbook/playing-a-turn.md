---
title: Playing a Club Mode season
description: Complete guide to playing a Club Mode season in league-tui — Lobby setup, the four zones, ledger abbreviations, Privacy Curtains, dead time, save/resume, and end-of-season replay export.
sidebar:
  order: 7
---

:::caution[Prototype]
Prototype, source only. Offline **Club Mode** in `league-tui` is playable from source across a full 24–32 turn season (public-Testnet Game mode is not built yet, and no container image is published). `4: Diplomatic Desk` seat-to-seat whispers are not in Club Mode yet.
:::

## Launching and setting up a table (Lobby)

Start an offline hot-seat season from source with a save directory so progress checkpoints automatically:

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

Use `Up`/`Down` (or `Tab`/`BackTab`) in the **Club Mode Lobby** to configure your table before pressing `Enter` on **Start Club Mode Table**:

- **Seat Count (`4–6`):** Step between `4`, `5`, and `6` seats (`Left`/`Right` or `+`/`-`).
- **Seats (`Human` vs `ClubBot`):** Press `Space`, `Left`, or `Right` on any seat row to toggle between local `Human` (default Standing Regent fallback doctrine active) and deterministic rules-only `ClubBot`. At least one seat must be `Human`.
- **Layout Seed (`SeedPool::Dev`, `0..=9999`):** Type decimal digits directly (`Backspace` deletes). Reserved test seeds and `Train`/`Probe`/`HeldOut`/`Sealed` seeds are refused by name (`RESERVED_SEED_FORBIDDEN` / `SEED_POOL_MISMATCH`).
- **Turn Clock (`180–300 s`, `210 s` default, or `Untimed`):** `Left`/`Right` steps by `15 s`, and `Space` toggles `Untimed`. Timed turns divide budget `B` across `H` human seats: each human receives `floor(0.75 * B / H)` seconds in Sub-Pass A and `floor(0.25 * B / H)` seconds in Sub-Pass B (`157 s / 52 s` at `H = 1`, `B = 210 s`).
- **Curtain PINs (`Enabled` / `Disabled`):** Toggle optional 4-digit table-local Curtain PINs (`Space`/`Left`/`Right`). Never pass PIN digits on the command line; when enabled, each human seat sets its own PIN behind the Privacy Curtain at its first turn.
- **CLI overrides win:** Passing `--turn-clock <180..=300>`, `--untimed`, or `--pins` on the CLI overrides any conflicting Lobby or resumed `.seal` option (displayed inline on the Lobby row).

Every valid Lobby start opens Seat `0`'s initial **Privacy Curtain** cleanly with zero `SeatObservation`s built beforehand. If an unfinished `club-*.seal` file already exists in `--save-dir`, Lobby refuses to start with `UNFINISHED_SEASON_EXISTS` so active progress cannot be overwritten; once a completed season's replay is exported (wiping its `.seal`), starting a new season selects the next free table ID (`club-2`, …).

## Four-zone workspace

Once the Curtain lifts for an active human seat, the turn renders inside a responsive four-zone layout built strictly from that seat's `SeatObservation`:

- At **`100×30` or larger**, the screen shows the Header Bar on top, three side-by-side panels (`Public Table & Ledger Mirror`, `Action & Order Composer`, and `Private Vault & Diagnostics`) across the middle, and a two-row Status Line (refusal alert row above shortcuts) along the bottom.
- At **`80×24`**, the four zones stack vertically so every row is either shown at full height or reachable via that zone's scroll (`Up`/`Down`/`PageUp`/`PageDown`); non-table lines wider than a zone's inner width clip cleanly at that zone's width.
- Below **`80×24`**, the terminal renders `"Terminal too small"` until resized.

Goods appear with both their role-qualified guild name and 3-letter ledger code (for example `Rough Timber [TMB]`, `Seasoned Timber [TMB]`, and `Clearinghouse Marks [CRED]`). Press `c` at any time to toggle between guild names and plain codes.

## Reading the public ledger abbreviations

The compact five-column table inside **Public Table & Ledger Mirror** uses fixed abbreviations so leading digits never clip across narrow layouts (`100×30` and `120×40`), paired with full unclipped **Standing & Legend** rows scrollable directly below the table:

| Abbreviation | Meaning |
| --- | --- |
| `St` | Seat column (`S0`..`S5`) plus public status markers (`[AI]` `ClubBot`/`AiSyndicate`, `[W]` wash-frozen, `[B]` bond-breached) |
| `CRED` | Seat's public Clearinghouse Marks (`CRED`) cash balance |
| `Debt` | Seat's outstanding `cred_debt` |
| `Tr` | Public credit rating tier (`Aaa`..`D`) |
| `TarR/M` | Export tariff basis points for raw / manufactured (refined) goods (`raw_tariff_bps`/`refined_tariff_bps`) |
| `k` / `M` | Unambiguous decimal SI suffixes (`1k = 1,000`, `1M = 1,000,000`) used only when the exact integer exceeds column width |
| `+` | Trailing remainder indicator when an abbreviated `k` or `M` value is not exact (scroll to Standing & Legend for exact marks) |

## The four zones and Sweep Chronicle

1. **Header Bar:** current `TableId`, Turn `t`, `TurnPhase`, pass label (`Sub-Pass A` or `Sub-Pass B`), live countdown (`Clock: MM:SS` or `Clock: Untimed`), Turn `24–32` closing hazard, `[OPEN BOOK]` vs `[CANCEL LOCKED]` badge, active guild and `Human` / `ClubBot` role, `trade_minister` / `cartel_chair` badges, realized `ActiveShock`s (with `magnitude_bps`), and already-announced `ShockForecastEvent`s (always omitting unrealized magnitudes).
2. **Public Table & Ledger Mirror:** per-seat public table above, scrollable exact-figure Standing & Legend, free `raw`/`refined` balances, open and cartel DEX offers, open checks, `TokenEscrowVault`s, `DualEscrowVault`s, `IntentPledge`s, `CartelDomain`s, `CouncilPlatform`s, and `ActiveCharter`s.
3. **Action & Order Composer (`1`, `2`, `3`):**
   - **Tab 1 — Market & Contracts (`1`):** all 10 Phase-1 `MarketAction` variants (`PostOffer`, `CancelOffer` — blocked once `[CANCEL LOCKED]` engages, `CreateCheck`, `CancelCheck`, `CreateEscrow`, `CancelExpiredEscrow`, `RecordIntentPledge`, `StandForOffice`, `FlagWashFreeze`, `TownImport`).
   - **Tab 2 — Workshop, Deals & Politics (`2`):** all `SealedOrderBundle` sections (`converter_runs`, `project_deliveries`, `atomic_swaps`, `contract_deliveries`, `dual_escrows`, `debt_repayment_cred`, `escrow_finishes`, `check_cashes`), with `council_vote` gated to turns divisible by `4` and `charter_bids` gated to turns divisible by `6`.
   - **Tab 3 — What-If Sandbox (`3` or `p`):** simulates your staged orders through `preview_what_if` with selectable tie-break and counterparty assumptions, displaying post-sweep balances, hypothetical close marks, and scrubbed refusal messages.
   - **Tab 4 — Diplomatic Desk (Disabled):** shown disabled only; whispers are not in Club Mode yet.
4. **Private Vault & Diagnostics:** visible strictly while that human seat is unlocked — own dealtTurn-1 `PrivateMandate`, own active `OracleAuditor` hint (omitting magnitude until `realize_turn`), all five `SeatStatePredicates` gauges (`UpkeepAndDebt`, `SpoilageAndConversion`, `GoodScarcity`, `CoalitionReach`, `SeasonEndAndMarks`), and personal scratch notes (`n`).

Press `r` or `F2` to open the **Sweep Chronicle** modal (the 30 seat-safe fields of `PublicSweepReceipt`, never `newly_drawn_shocks` or `post_state_digest`). Use `[` and `]` to browse turns and `Esc` or `r` to close.

## Hand-offs, the Privacy Curtain, dead time & PINs

Each turn steps cleanly through **Sub-Pass A** -> `begin_cancel_lock()` -> **Sub-Pass B** -> local commit/reveal and **Sweep**, advancing any `ClubBot` seats automatically without disturbing human screens.

- **Raising the Curtain:** Pressing `Ctrl+l` / `Ctrl+L` (never Shift-`L`) or hitting a timed pass deadline saves the outgoing seat's Vault notes strictly inside the host / `.seal`, clears all outgoing free-text drafts (`SeatUiState::clear_for_hand_off`), drops the outgoing `SeatObservation`, clears the alternate screen in place (`CSI 2 J` — **never** `CSI 3 J` scrollback purge), and raises the **Privacy Curtain** showing only `TableId`, Turn `t`, pass label, and next seat guild.
- **400 ms dead time on both open and lift:** Whenever a Curtain opens (hand-off, timeout, resume, or Lobby start) **or** lifts, queued terminal keys are drained and keyboard actions (`Space`, `Enter`, `q`, `Esc`, `Ctrl+C`, and post-lift actions) are held inert for **`400 ms`**. Lift dead time never consumes the arriving seat's pass clock.
- **Masked 4-digit PINs:** When PINs are enabled, pressing `Space` then `Enter` prompts for that seat's 4-digit PIN without echoing digits. Turn 1's first Curtain assigns the PIN into `.seal`; every later unlock verifies against `.seal`. Wrong PINs build zero `SeatObservation`s, and a 3rd miss resets back to the unarmed Curtain so another seat can retake the screen cleanly.
- **Public Standby (`Session Standby`):** Whenever no human seat controls the keyboard (quit, `AwaitingMandateReveal`, or halt/autosave failure showing `Autosave failed [<code>]`, Save path, and Reason), only a strictly public view renders.

## Save and resume mid-season

Running with `--save-dir <DIR>` writes atomically on every Privacy Curtain, initial PIN entry commit, and Sweep that leaves `TalkAndMarket`:

- **`club-<id>.save.json` (public):** public table state matching `SeatObservation` (no mandates, no uncommitted bundles, no notes, no PINs, no commit salts, no unannounced shocks or pre-derived VRF proofs).
- **`club-<id>.seal` (private local-trust sidecar):** dealt mandates, pre-derived VRF proofs, `drawn_shocks`, Phase-2 drafts, Vault notes, commit salt entropy, and 4-digit PINs. Honest hot-seat players must not open `.seal` while a season is active.

Resume any time via:

```bash
cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json
```

For an active **in-season** save (`TalkAndMarket`), resuming advances any leading `ClubBot` seats up to the active human cursor, **always opens behind a Privacy Curtain** (requiring that seat's PIN if enabled) with zero human `SeatObservation`s built beforehand, preserves your saved timed/untimed setting (unless overridden on CLI), and restarts the resumed pass clock at that seat's full per-pass budget. For a **post-season** save (`AwaitingMandateReveal` or `Completed`), resuming opens directly on the public `Session Standby` reveal screen (`Reveals: k/N`, showing zero mandates) or the `Completed` End-of-Season tabs so you can finish reveals or retry replay export without a Curtain. Combining `--resume` with a different `--save-dir` is refused when that directory holds another `club-*.seal` file (`UNFINISHED_SEASON_EXISTS`, any table ID) or otherwise holds `club-<id>.save.json`, `.seal` or `.replay.json` for the same table ID (`SEASON_SAVE_EXISTS`, except when resuming a finished season whose own byte-identical regular `club-<id>.replay.json` is already in that directory with no conflicting `.save.json` or `.seal`, so an export retry after a failed source-`.seal` wipe proceeds); a directory holding only a finished, exported season with a different table ID is accepted without overwriting anything. Targeting an empty or not-yet-created `--save-dir` creates it when writing and still wipes the resumed `.seal` on successful export, and resuming in place (no `--save-dir`, or `--save-dir` targeting the resumed directory) autosaves there as usual. An in-season resume into another `--save-dir` continues the season there and leaves the original directory's `club-<id>.save.json` and `.seal` in place. Exporting the replay in that same session wipes the original `.seal` too; if you instead quit and later resume from the new directory, the original `.seal` stays, still holding that season's private values and keeping the original directory's Lobby refusing a new season (`UNFINISHED_SEASON_EXISTS`). Delete that leftover copy only after you have quit and the season has saved in the new directory. Apart from that leftover, keep each season's `club-<id>.save.json`, `.seal` and `.replay.json` under the names and in the directory `league-tui` gave them; to move a season elsewhere, resume it with `--save-dir <EMPTY_DIR>` rather than renaming or moving the files by hand.

## Closing a season & exporting the replay

When Step 8 rolls a close (`Turns 24..=32`), the turn moves to `AwaitingMandateReveal`.

1. **Standby mandate reveals (`AwaitingMandateReveal`):** The screen stays on the strictly public standby counter (`Reveals: k/N`) showing **zero** mandates until all `N` seat reveals are submitted (`Enter` or `Space`).
2. **End-of-Season 4-tab summary (`Completed`):** Submitting the final reveal calls `finalize_season()` and opens four tabs:
   - `1: Scoreboard` — ranked `FinalScoreboard` and standings ties.
   - `2: Mandates` — every seat's unsealed Turn-1 `PrivateMandate` and earned `MandateBonus`.
   - `3: Metrics & Marks` — close-marking splits (`HoldingMark`) and `SeatSeasonMetrics` solvency/honor stats (`c` toggles plain codes).
   - `4: Replay Export` — press `e` or `Enter` to serialize, verify, and atomically write `club-<id>.replay.json`, then automatically wipe the private `.seal` file. If another season's replay already occupies the target path, export refuses `REPLAY_TARGET_EXISTS` without overwriting it; if export fails (for instance an unwritable directory), `.seal` stays intact on disk, Tab 4 displays the exact `TUI_REPLAY_EXPORT_FAILED` or `REPLAY_TARGET_EXISTS` reason and hint, and exiting returns CLI exit code **`3`** (partial success) so you can resolve the path or retry into another `--save-dir`.

## Keyboard reference

| Context | Keys | Action |
| --- | --- | --- |
| **Lobby** | `Up` / `Down` / `Tab` / `BackTab` | Select row |
| **Lobby** | `Left` / `Right` / `+` / `-` / `Space` | Adjust seats (`4–6`), toggle Human/Bot, adjust clock (`180–300 s`), toggle Untimed (`Space`) or PINs |
| **Lobby** | `0`–`9` / `Backspace` | Edit clear `SeedPool::Dev` seed (`0..=9999`) on Seed row |
| **Lobby** | `Enter` | Start Club Mode table (`Start Club Mode Table` row) |
| **Turn** | `1`, `2`, `3`, `Tab`, `BackTab` | Switch enabled centre tabs (`4` stays disabled) |
| **Turn** | `F6` | Cycle focus across Ledger, Centre Tabs, and Private Vault |
| **Turn** | `[` / `]` | Cycle action kind, bundle section, or Chronicle receipt turn |
| **Turn** | `Up` / `Down` / `PageUp` / `PageDown` / `Home` | Select row or scroll focused panel |
| **Turn** | `Left` / `Right` / `+` / `-` / `Space` | Adjust field or toggle What-If assumption |
| **Turn** | `e` / `n` | Edit free-text composer override (`e`) or Private Vault note (`n`), `Enter`/`Esc` finishes |
| **Turn** | `Enter` / `p` / `Del` / `Backspace` / `X` | Submit action, stage bundle, run What-If (`p`), or clear staged entries (`X` clears all) |
| **Turn / EOS** | `c` | Toggle in-world guild commodity names + `[CODE]` vs plain `[CODE]` |
| **Turn** | `r` / `F2` | Open or close the Sweep Chronicle modal |
| **Turn** | `Ctrl+l` / `Ctrl+L` | Lock active pass, clear drafts, and raise the Privacy Curtain |
| **Curtain** | `Space` then `Enter` (+ 4 digits + `Enter` if PINs on) | Lift Privacy Curtain after `400 ms` dead time |
| **Post-Season** | `Enter` / `Space` | Submit next mandate reveal during `AwaitingMandateReveal` |
| **End of Season** | `1`–`4`, `Tab`, `Left`/`Right`, `e`/`Enter` | Switch End-of-Season tabs or export `club-<id>.replay.json` & wipe `.seal` |
| **Global** | `q` / `Esc` / `Ctrl+c` | Quit cleanly (inert during `400 ms` Curtain dead time) |
