# Paper test, game 5: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Claude | subagent | claude-sonnet-5-5 |
| Weavers | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Smiths | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Dyers | Claude | subagent | claude-sonnet-5-5 |

**Sitting out:** Foresters, Tanners.

## Markets

Step 2, stock 4 a day, buying at 1/3 of the reference price, sit-out goods at 3/2 times.

| Town | Sells |
|---|---|
| Varn | nothing |
| Ostra | brine from 1, +2 a unit, 4 a day; wool from 1, +2 a unit, 4 a day |
| Kell | ore from 2, +2 a unit, 4 a day; hide from 2, +2 a unit, 4 a day |
| Hath | timber from 2, +2 a unit, 4 a day; **charcoal at 5** (the Foresters sit out), 2 a day; **leather at 14** (the Tanners sit out), 2 a day |
| Mire | madder from 2, +2 a unit, 4 a day |

Every market buys converted goods, up to 3 of each a day, at: salt 1, charcoal 1, dye 3, tools 3, leather 3, cloth 5.

## Works

| Town | Works | Needs |
|---|---|---|
| Ostra | Ostra crane | 3 tools, 3 cloth |
| Ostra | Ostra customs house | 2 cloth, 2 dye, 2 salt |
| Kell | Kell bridge | 3 tools, 3 timber, 2 salt |
| Mire | Mire causeway | 3 dye, 4 timber |
| Varn | Varn sea wall | 3 tools, 2 cloth, 3 salt |
| Hath | Hath lodge | 2 leather, 3 tools, 2 charcoal |

42 units in all. A completed works scores a bonus of 4 to the guild that delivers its last unit.

## Edicts

Announced to the seats one day ahead, in that day's public state.

| Day | Edict | The referee's rule |
|---|---|---|
| 4 | Town buyer tools at Kell | Kell's market buys tools at its full reference price (9), up to 3 a day, on days 4 and 5. |
| 8 | Road closed at Hath–Mire | The Hath–Mire road is closed on days 8 and 9. A travel order onto it is refused at the reveal; the caravan stays and pays no toll. |

## What's scarce

No Foresters or Tanners: charcoal and leather are on Hath's market at 5 and 14, and timber only there. Ostra has two works. Kell buys tools at full price on days 4–5, a pull away from the works.

## Sealed until the bell

Each seat's card (its mandates) and the bell day are sealed; the referee holds them. They are added here after the game, and anyone can check them against these hashes.

| Sealed | SHA-256 |
|---|---|
| the Saltworks' card | `631d5e39b27a2d776495ee590ca9e19f786e97013db84e16d37f190a2e1e904a` |
| the Weavers' card | `15a10bbe6ba55352afed752b37ffad454d8538818718b072e6a9e89016734eb3` |
| the Smiths' card | `04d5feec109002ae8e9563616b9d59fdae9fe74ca1e956a5718afb12b03716de` |
| the Dyers' card | `44e5e0376d96b90faf9b0a66e09f01edfa3759baf30e62dab5d1717ca7c3bf6c` |
| the bell | committed once its round is published |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966660 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966660 | not yet published |
| 2 | 33966665 | not yet published |
| 3 | 33966670 | not yet published |
| 4 | 33966675 | not yet published |
| 5 | 33966680 | not yet published |
| 6 | 33966685 | not yet published |
| 7 | 33966690 | not yet published |
| 8 | 33966695 | not yet published |
| 9 | 33966700 | not yet published |
| 10 | 33966705 | not yet published |
| 11 | 33966710 | not yet published |
| 12 | 33966715 | not yet published |
| 13 | 33966720 | not yet published |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `b5aac82c4b7996c82eb4a40efbad064a4bb32f8841870326f49eacf3d6188ac9` |
