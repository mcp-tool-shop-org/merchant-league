# Paper test, game 2: score

The bell rang at the **close of day 12**. Five seats played: Smiths and Dyers (Claude), Saltworks (Gemini, AI Studio), Weavers (Gemini app) and Tanners (GPT-OSS). The Foresters sat out.

## Final scores

| Place | Guild | Works points | Mandate points | **Score** | Standing | Breaches |
|---|---|---|---|---|---|---|
| 1 | Weavers | 12 | 9 | **21** | 0 | 0 |
| 2 | Smiths | 13 | 5 | **18** | −3 | 1 |
| 3 | Dyers | 11 | 5 | **16** | −3 | 1 |
| 4 | Saltworks | 7 | 4 | **11** | 1 | 0 |
| 5 | Tanners | 0 | 0 | **0** | −3 | 1 |

No ties. The median score was 16. **No works was completed:** 18 of 41 units were delivered, so no completion bonus was scored.

**Works delivered, by works:**

| Works | Delivered / needed | Who |
|---|---|---|
| Ostra lighthouse | cloth 2/3, salt 2/2, leather 0/3 | Weavers 6, Saltworks 4 |
| Kell forge hall | charcoal 2/4, tools 0/3, leather 0/2 | Weavers 4 |
| Mire bridge | dye 3/3, timber 2/4, tools 1/2 | Dyers 11, Smiths 3 |
| Varn salt barns | salt 2/3, timber 1/3, cloth 0/2 | Weavers 2, Saltworks 3 |
| Hath watchtower | tools 3/3, leather 0/2, dye 0/2 | Smiths 10 (with the festival's +1) |

**No leather was made all game.** The Tanners left home on day 1 and never came back to convert. Three of the five works needed leather, so none of those three could be finished.

**Contracts:** 7 proposed and 4 agreed. 1 was kept (C2-4) and 3 ended in seller's breaches: C2-2 by the Tanners, C2-3 by the Smiths, C2-7 by the Dyers.

## Mandates, revealed

| Guild | Mandate | Points | Met |
|---|---|---|---|
| Saltworks | Sell at least 2 salt to the Dyers and at least 2 to the Tanners | 4 | yes |
| Saltworks | Complete the Varn salt barns | 6 | no |
| Weavers | Be the largest contributor to the Ostra lighthouse | 5 | yes |
| Weavers | Deliver goods to works in 3 different towns | 4 | yes (day 12) |
| Smiths | Deliver 4 tools to works outside Kell | 5 | yes |
| Smiths | Install the bellows by the close of day 5 | 4 | no (no leather existed) |
| Dyers | Deliver 3 dye to the Mire bridge | 5 | yes |
| Dyers | End with standing 2 or more | 4 | no (−3) |
| Tanners | Complete the Hath watchtower | 7 | no |
| Tanners | Deliver 3 leather to works outside Hath | 4 | no |

## Reading the Tanners' 0

The Tanners seat was GPT-OSS 120B. Its 0 is better read as a harness result than as a rules result:
- 11 of its 24 first turns answered in the chat instead of writing the reply file, and each needed the one re-send the plan allows;
- it had 2 contract-memory faults;
- the platform compacted its context on the last turn.

`metrics.md` keeps the full record. The other four seats' results stand as play.

## The seals, checked

The Book Keeper checked these on 2026-11-15, after the bell.

- **The cards.** Every card in `cards/` hashes to the SHA-256 `setup.md` committed before day 1.
- **The bell.** SHA-256(`12:<salt>`) matches `setup.md`'s commitment. The salt is in `sealed-state.json`.
- **The bell's round.** The bell day follows from round 33966450's value by `board.md`'s rule (n mod 27 = 18, so day 12). That value matches drand's published round.

## About these files

`log.md` and `metrics.md` are the referee's records, copied from the sealed folder (SHA-256 of the sealed originals: log `2121a7ad91f4f7d8d661ad9bff47397afd0de60ce16b5e4c36354dd465d86dfd`, metrics `90e4bfc35dece88cdfa40e38ef84bf991022c12bd4bddcb725ce16b2e598cbe2`). `log.md` differs from its original in one place only: a local path to R&D's seating note is replaced by a plain description. `metrics.md` is unchanged.

The seats' messages in `log.md` are verbatim. One of them, the Weavers' day-5 talk (lines 866–867), uses a word for a bond's release that this repository's copy check (`scripts/check-copy.sh`) refuses in player-facing text. It's left as written, because it's a record of play, not game copy.
