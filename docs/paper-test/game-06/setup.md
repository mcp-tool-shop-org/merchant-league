# Paper test, game 6: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Weavers | Claude | subagent | claude-sonnet-5-5 |
| Smiths | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Foresters | Claude | subagent | claude-sonnet-5-5 |
| Tanners | Gemini | Gemini app (a second conversation) | R&D records it at seating, before day 1 |

**Sitting out:** Dyers.

## Markets

Step 1, stock 3 a day, buying at 1/3 of the reference price, sit-out goods at 3/2 times.

| Town | Sells |
|---|---|
| Varn | nothing |
| Ostra | brine from 1, +1 a unit, 3 a day; wool from 1, +1 a unit, 3 a day |
| Kell | ore from 2, +1 a unit, 3 a day; hide from 2, +1 a unit, 3 a day |
| Hath | timber from 2, +1 a unit, 3 a day |
| Mire | madder from 2, +1 a unit, 3 a day; **dye at 14** (the Dyers sit out), 2 a day |

Every market buys converted goods, up to 3 of each a day, at: salt 1, charcoal 1, dye 3, tools 3, leather 3, cloth 5.

## Works

| Town | Works | Needs |
|---|---|---|
| Varn | Varn sea wall | 3 tools, 4 timber, 2 cloth |
| Kell | Kell bridge | 4 charcoal, 2 leather, 2 tools |
| Hath | Hath lodge | 3 leather, 3 salt, 2 tools |
| Hath | Hath kiln yard | 4 charcoal, 2 timber |
| Ostra | Ostra crane | 2 cloth, 3 tools, 2 leather |
| Mire | Mire causeway | 4 timber, 2 salt, 1 dye |

45 units in all. A completed works scores a bonus of 4 to the guild that delivers its last unit.

## Edicts

Announced to the seats one day ahead, in that day's public state.

| Day | Edict | The referee's rule |
|---|---|---|
| 4 | Levy on charcoal | On days 4 and 5, each unit of charcoal that passes between guilds (an offer fill or a payment) costs the receiving guild 1 coin, paid to the steward. |
| 8 | Shortage ore at Kell | Kell's market sells at most half its usual stock of ore, rounded down, on days 8 to 10. |

## What's scarce

No Dyers: dye is on Mire's market at 14, and the Weavers need one per cloth. Hath has two works and two seated guilds. Ore runs short at Kell on days 8–10.

## Sealed until the bell

Each seat's card (its mandates) and the bell day are sealed; the referee holds them. They are added here after the game, and anyone can check them against these hashes.

| Sealed | SHA-256 |
|---|---|
| the Saltworks' card | `7b95c041e8aac128bb205bcb91d648fa05e9ecb0b092e87434c0f0b51d5d77f2` |
| the Weavers' card | `592e58ab88263de7efff4e16a5c0eefdd9ffe365be63f26b59d4ce595bd21d25` |
| the Smiths' card | `fae5bf6b7e78f420b6c0a5f6f139cd87f650a04329e0ba4cba5d30be79c755f5` |
| the Foresters' card | `cedbaed0e80320c69c8f1cd816a6e4b6c843fb3c817fecea88a1e647e6da3dff` |
| the Tanners' card | `fcbb9352b769c164b0e66d9d884bfd885bc7d5ee4ec788cd01dfc1d877251f21` |
| the bell | committed once its round is published |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966760 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966760 | not yet published |
| 2 | 33966765 | not yet published |
| 3 | 33966770 | not yet published |
| 4 | 33966775 | not yet published |
| 5 | 33966780 | not yet published |
| 6 | 33966785 | not yet published |
| 7 | 33966790 | not yet published |
| 8 | 33966795 | not yet published |
| 9 | 33966800 | not yet published |
| 10 | 33966805 | not yet published |
| 11 | 33966810 | not yet published |
| 12 | 33966815 | not yet published |
| 13 | 33966820 | not yet published |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `e1a27079fe2537cb9f1318e475970aaf84d41b8ed446737a4330842d72243ecc` |
