# Paper test, game 1: score

The bell rang at the **close of day 11**. Four seats played: Saltworks (Claude), Weavers (GPT-OSS), Smiths (Gemini, AI Studio) and Foresters (Gemini app). The Dyers and the Tanners sat out.

## Final scores

| Place | Guild | Works points | Mandate points | **Score** | Standing | Breaches |
|---|---|---|---|---|---|---|
| 1 | Foresters | 31 | 10 | **41** | 0 | 0 |
| 2 | Smiths | 18 | 3 | **21** | 0 | 0 |
| 3 | Saltworks | 8 | 9 | **17** | 0 | 0 |
| 4 | Weavers | 6 | 0 | **6** | 0 | 0 |

No ties. The median score was 19. No contract was made, so every standing stayed at 0.

**Where the works points came from:**
- **Foresters:** 2 charcoal to the Hath lodge (4, day 6). The whole Mire causeway, 16 plus the completion bonus of 4 (day 7). 4 timber and 1 tools to the Varn sea wall (7, day 8).
- **Smiths:** the Hath lodge's last charcoal plus its completion bonus (6, day 10), and the Kell bridge (12, day 11).
- **Saltworks:** 4 salt to the Hath lodge (8, day 9).
- **Weavers:** 2 cloth to the Ostra crane (6, day 11).

## Mandates, revealed

| Guild | Mandate | Points | Met |
|---|---|---|---|
| Saltworks | Deliver 4 salt to works outside Varn | 4 | yes |
| Saltworks | Be the largest contributor to the Hath lodge | 5 | yes |
| Weavers | Deliver 3 cloth to works | 6 | no (2) |
| Weavers | Have 2 contracts end Kept, as buyer or seller | 4 | no (no contract was made) |
| Smiths | Install the bellows by the close of day 6 | 3 | yes (day 2) |
| Smiths | Complete the Kell bridge | 6 | no (1 charcoal and 2 salt short) |
| Foresters | Deliver charcoal or timber to works in 3 different towns | 4 | yes |
| Foresters | Be the largest contributor to the Varn sea wall | 6 | yes (the only contributor) |

## The seals, checked

The Book Keeper checked these on 2026-11-15, after the bell.

- **The cards.** Every card in `cards/` hashes to the SHA-256 `setup.md` committed before day 1.
- **The bell.** SHA-256(`11:<salt>`) matches `setup.md`'s commitment. The salt is in `sealed-state.json`.
- **The bell's round.** The bell day follows from round 33966350's value by `board.md`'s rule (n mod 27 = 9, so day 11). That value matches drand's published round.

## About these files

`log.md` and `metrics.md` are the referee's records, copied from the sealed folder (SHA-256 of the sealed originals: log `84f7428217963deac0fdbbfcd4bdb8fcc27e54c82e440f22d7ef0058a8e9943c`, metrics `637cccb62090848c974bf1956abcc313a6befb1f5f3e5cebd44222d6aec2f471`). `log.md` differs from its original in two places only. Two local paths are replaced by plain descriptions: the referee's brief, and the checkout it read the setup from. `metrics.md` is unchanged.

`sealed-state.json` is the referee's sealed state: the rounds, the bell's round and salt, and the card hashes.
