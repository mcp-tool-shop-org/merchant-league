# Paper test, game 2: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Weavers | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Smiths | Claude | subagent | claude-sonnet-5-5 |
| Dyers | Claude | subagent | claude-sonnet-5-5 |
| Tanners | GPT-OSS | Antigravity card, clean project | gpt-oss-120b |

**Fallback seating,** if game 2 can't start and finish by the end of 17 November 2026 ET, when GPT-OSS 120B leaves Antigravity:

- the Tanners: Gemini, AI Studio (a second conversation), R&D records it at seating, before day 1.

**The GPT-OSS seat** (Publisher, 2026-11-14):
- it opens in a clean Antigravity project whose tree holds no other seat's setup, brief, mandate or order;
- its card says to read no files and play only from the conversation (every card does);
- each turn, the referee checks that it still remembers its live contracts. A lost contract is logged in `metrics.md` as a harness fault, not a play choice.

**Sitting out:** Foresters.

## Markets

Step 2, stock 3 a day, buying at 1/2 of the reference price, sit-out goods at 2 times.

| Town | Sells |
|---|---|
| Varn | nothing |
| Ostra | brine from 1, +2 a unit, 3 a day; wool from 1, +2 a unit, 3 a day |
| Kell | ore from 2, +2 a unit, 3 a day; hide from 2, +2 a unit, 3 a day |
| Hath | timber from 2, +2 a unit, 3 a day; **charcoal at 6** (the Foresters sit out), 2 a day |
| Mire | madder from 2, +2 a unit, 3 a day |

Every market buys converted goods, up to 3 of each a day, at: salt 2, charcoal 1, dye 4, tools 4, leather 4, cloth 7.

## Works

| Town | Works | Needs |
|---|---|---|
| Ostra | Ostra lighthouse | 3 cloth, 3 leather, 2 salt |
| Kell | Kell forge hall | 3 tools, 4 charcoal, 2 leather |
| Mire | Mire bridge | 3 dye, 4 timber, 2 tools |
| Varn | Varn salt barns | 3 salt, 2 cloth, 3 timber |
| Hath | Hath watchtower | 3 tools, 2 leather, 2 dye |

41 units in all. A completed works scores a bonus of 4 to the guild that delivers its last unit.

## Edicts

Announced to the seats one day ahead, in that day's public state.

| Day | Edict | The referee's rule |
|---|---|---|
| 4 | Road closed at Ostra–Kell | The Ostra–Kell road is closed on days 4 and 5. A travel order onto it is refused at the reveal; the caravan stays and pays no toll. |
| 8 | Festival at Hath | Each unit delivered to a works in Hath on days 8 and 9 scores 1 more than its works value. |

## What's scarce

No Foresters: charcoal is on Hath's market at twice its reference, and timber only on Hath's market. The Saltworks need timber every converting day, and Hath is two roads from Varn.

## Sealed until the bell

Each seat's card (its mandates) and the bell day are sealed; the referee holds them. They are added here after the game, and anyone can check them against these hashes.

| Sealed | SHA-256 |
|---|---|
| the Saltworks' card | `9945d40f503fecb80e184faa3abfe834e47cdf67a6493837839a3e7979975b5c` |
| the Weavers' card | `07f4758d4c20c543014c018e5830434f370b241508c0a63ae709ce4288a3525c` |
| the Smiths' card | `eb0c13e31298dacd6cb24760f63fccf64eb988f382be972ba49b46288b59f104` |
| the Dyers' card | `ff63eebd5e957484fca6aabf317881e3133e5a7f8f705da1837192af4d8d7dd2` |
| the Tanners' card | `091b6212a996b87efb30dc388b993d85fff6fe1becd29baa531fc42b41bdb5f1` |
| the bell | committed once its round is published |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966360 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966360 | not yet published |
| 2 | 33966365 | not yet published |
| 3 | 33966370 | not yet published |
| 4 | 33966375 | not yet published |
| 5 | 33966380 | not yet published |
| 6 | 33966385 | not yet published |
| 7 | 33966390 | not yet published |
| 8 | 33966395 | not yet published |
| 9 | 33966400 | not yet published |
| 10 | 33966405 | not yet published |
| 11 | 33966410 | not yet published |
| 12 | 33966415 | not yet published |
| 13 | 33966420 | not yet published |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `51cfe978fd2d812640f3586204b531a18e825bbcd8cf171013c7ef8a8497ea91` |
