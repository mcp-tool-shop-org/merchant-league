---
title: Playing a turn
description: How a Club Mode turn plays in the terminal — the four zones, centre tabs, Sweep Chronicle, and hot-seat Curtain hand-offs.
sidebar:
  order: 7
---

:::caution[Preview — not released]
Preview — not released. This page documents the Club Mode (`v0.1.0`) turn-screen preview in `league-tui`. Lobby options, timed turn clocks, seat PINs, local `.save.json` / `.seal` persistence wiring, the end-of-season scoreboard screen, Diplomatic Desk whispers, `--log-level`, and `0/1/2/3` CLI exit codes are not built yet and belong to the later release card.
:::

## Four-zone workspace

Each human turn renders inside a responsive four-zone layout built strictly from that seat's `SeatObservation`:

- At **`100×30` or larger**, the screen shows the Header Bar on top, three side-by-side panels in the middle (`Public Table & Ledger Mirror`, `Action & Order Composer`, and `Private Vault & Diagnostics`), and the Status Line along the bottom.
- At **`80×24`**, the four zones stack vertically without clipping panels mid-row.
- Below **`80×24`**, the terminal renders a single `"Terminal too small"` message until resized.

Goods appear with both their role-qualified guild name and 3-letter ledger code (for example `Seasoned Timber [TMB]` and `Clearinghouse Marks [CRED]`). Press `c` at any time to switch between guild names and plain codes.

## The four zones

1. **Header Bar:** current `TableId`, Turn `t`, `TurnPhase`, pass label (`Sub-Pass A` or `Sub-Pass B`), Turn `24–32` closing-window status, `[OPEN BOOK]` vs `[CANCEL LOCKED]` indicator, active seat guild and `Human` / `ClubBot` role, `trade_minister` / `cartel_chair` badges, realized `ActiveShock`s, and announced `ShockForecastEvent`s (without unrealized shock magnitudes).
2. **Public Table & Ledger Mirror:** per-seat public `CRED`, `cred_debt`, credit tier, irreversible `project_equity`, export tariffs, defaults/breaches, and wash-freeze status, plus scrollable commodity balances, open/cartel DEX offers, active `CreditCheck`s, `TokenEscrowVault`s, `DualEscrowVault`s, `IntentPledge`s, `CartelDomain`s, `CouncilPlatform`s, and `ActiveCharter`s.
3. **Action & Order Composer (Centre Tabs):**
   - **Tab 1 — Market & Contracts (`1`):** all 10 Phase-1 `MarketAction` variants (`PostOffer`, `CancelOffer`, `CreateCheck`, `CancelCheck`, `CreateEscrow`, `CancelExpiredEscrow`, `RecordIntentPledge`, `StandForOffice`, `FlagWashFreeze`, `TownImport`). `CancelOffer` is blocked once `[CANCEL LOCKED]` engages.
   - **Tab 2 — Workshop, Deals & Politics (`2`):** all `SealedOrderBundle` sections (`converter_runs`, `project_deliveries`, `atomic_swaps`, `contract_deliveries`, `dual_escrows`, `debt_repayment_cred`, `escrow_finishes`, `check_cashes`). `council_vote` appears only on turns divisible by `4`, and `charter_bids` appear only on turns divisible by `6`.
   - **Tab 3 — What-If Sandbox (`3` or `p`):** sends a `HumanWhatIfRequest` through the host's `preview_what_if` engine using selectable tie-break and counterparty assumptions, displaying post-sweep balances, hypothetical close marks, and scrubbed refusal messages.
   - **Tab 4 — Diplomatic Desk (Disabled):** shown as a disabled label only; seat-to-seat whispers are not enabled in this preview.
4. **Private Vault & Diagnostics:** visible only for the active human seat behind the Curtain — own dealt `PrivateMandate`, own active `OracleAuditor` shock hints (always omitting magnitude before `realize_turn`), all five `SeatStatePredicates` summaries (`UpkeepAndDebt`, `SpoilageAndConversion`, `GoodScarcity`, `CoalitionReach`, `SeasonEndAndMarks`), and personal scratch notes (`n`).

## Sweep Chronicle modal (`r` / `F2`)

Press `r` (or `F2`) to inspect completed turns inside the **Sweep Chronicle** modal. The modal reads solely from the 30 seat-safe fields of `PublicSweepReceipt` and never exposes `newly_drawn_shocks` or `post_state_digest`. Use `[` and `]` inside the modal to browse earlier turns, `Up`/`Down`/`PageUp`/`PageDown` to scroll, and `Esc` or `r` to close.

## Keyboard reference

| Keys | Action |
|---|---|
| `1`, `2`, `3`, `Tab`, `BackTab` | Switch enabled centre tabs (`4` is disabled) |
| `F6` | Cycle focus across Ledger, Centre Tabs, and Private Vault |
| `[` / `]` | Cycle action kind, bundle section, or Chronicle receipt turn |
| `Up` / `Down` / `PageUp` / `PageDown` / `Home` | Select parameter row or scroll the focused zone |
| `Left` / `Right` / `+` / `-` / `Space` | Adjust field value or toggle What-If assumption |
| `e` | Edit free-text composer parameter overrides (`amt=500 px=1200`), `Enter` applies |
| `n` | Edit personal notes in the Private Vault drawer (`Enter`/`Esc` finishes) |
| `Enter` | Submit `MarketAction`, stage bundle section, or run What-If preview |
| `Del` / `Backspace` / `X` | Clear selected bundle section (`X` clears all staged entries) |
| `p` | Run What-If Sandbox (`preview_what_if`) immediately |
| `c` | Toggle in-world commodity names + `[CODE]` vs plain `[CODE]` |
| `r` / `F2` | Open or close the Sweep Chronicle modal |
| `Ctrl+l` / `L` | Lock active pass, clear free text, and open Privacy Curtain |
| `Space` then `Enter` | Confirm Privacy Curtain hand-off to the next human seat |
| `q` / `Esc` / `Ctrl+c` | Quit the terminal session cleanly |

## Two passes and the Privacy Curtain

Each turn steps cleanly through:

1. **Sub-Pass A (Open Market & Talk)** in rotating seat order `(turn - 1 + offset) % N` (`ClubBot` seats execute automatically via `ClubBot`).
2. **`begin_cancel_lock()`** closes `CancelOffer` and engages `[CANCEL LOCKED]`.
3. **Sub-Pass B (Post-Lock & Sealed Commit)** lets every seat post final non-cancel Phase-1 actions and finalize its `SealedOrderBundle`.
4. **Local seal-and-sweep** runs `commit_and_reveal_all` followed by `sweep_turn`.

Whenever a human seat triggers **Lock & Hand Off (`Ctrl+l` / `L`)**, the host drops the outgoing `SeatObservation`, clears all free-text buffers (`composer_draft`, `whisper_draft`, and `notes_draft`) while preserving only tab and selection indices, clears the terminal buffer, and displays the untimed **Privacy Curtain** showing only the table ID, turn number, pass label, and next seat's guild name until `[Space]` followed by `[Enter]` confirms.
