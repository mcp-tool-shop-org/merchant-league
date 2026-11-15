# Paper test, game 3: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Weavers | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Foresters | Claude | subagent | claude-sonnet-5-5 |
| Dyers | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Tanners | Claude | subagent | claude-sonnet-5-5 |

**Sitting out:** Saltworks, Smiths.

## Markets

Step 1, stock 4 a day, buying at 1/3 of the reference price, sit-out goods at 3/2 times.

| Town | Sells |
|---|---|
| Varn | **salt at 8** (the Saltworks sit out), 2 a day |
| Ostra | brine from 1, +1 a unit, 4 a day; wool from 1, +1 a unit, 4 a day |
| Kell | ore from 2, +1 a unit, 4 a day; hide from 2, +1 a unit, 4 a day; **tools at 14** (the Smiths sit out), 2 a day |
| Hath | timber from 2, +1 a unit, 4 a day |
| Mire | madder from 2, +1 a unit, 4 a day |

Every market buys converted goods, up to 3 of each a day, at: salt 1, charcoal 1, dye 3, tools 3, leather 3, cloth 5.

## Works

| Town | Works | Needs |
|---|---|---|
| Ostra | Ostra market hall | 3 cloth, 3 leather |
| Hath | Hath long house | 4 charcoal, 3 leather, 2 timber |
| Mire | Mire causeway | 4 timber, 3 dye |
| Kell | Kell shrine | 2 cloth, 2 dye, 3 charcoal |
| Varn | Varn sea wall | 4 timber, 2 cloth, 2 leather |

37 units in all. A completed works scores a bonus of 4 to the guild that delivers its last unit.

## Edicts

Announced to the seats one day ahead, in that day's public state.

| Day | Edict | The referee's rule |
|---|---|---|
| 4 | Shortage of madder at Mire | Mire's market sells at most half its usual stock of madder, rounded down, on days 4 to 6. |
| 8 | Town buyer cloth at Ostra | Ostra's market buys cloth at its full reference price (15), up to 3 a day, on days 8 and 9. |

## What's scarce

No Saltworks: salt is on Varn's market only, at 8, and both the Dyers and the Tanners need it. No Smiths: the axes' tools come from Kell's market at 14. The madder shortage on days 4–6 squeezes the Dyers.

## Sealed until the bell

Each seat's card (its mandates) and the bell day are sealed; the referee holds them. They are added here after the game, and anyone can check them against these hashes.

| Sealed | SHA-256 |
|---|---|
| the Weavers' card | `7dad897983504cacf7cb7ee051aab01bf3028eef551d0d2b7f4d45cbb920c133` |
| the Foresters' card | `5fde046ee8c9f82894b255eb38e708470df9a9f5ea97a29da7d67187bea80858` |
| the Dyers' card | `2dfe72fa14693837ce907cadd17684488463a86abe3feba49b08d785ba42f990` |
| the Tanners' card | `913ff51d7ac336d90ee14b18c4e2e06cf3b594493a9ad5d2bb789dfc2c8c54d7` |
| the bell | committed once its round is published |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966460 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966460 | not yet published |
| 2 | 33966465 | not yet published |
| 3 | 33966470 | not yet published |
| 4 | 33966475 | not yet published |
| 5 | 33966480 | not yet published |
| 6 | 33966485 | not yet published |
| 7 | 33966490 | not yet published |
| 8 | 33966495 | not yet published |
| 9 | 33966500 | not yet published |
| 10 | 33966505 | not yet published |
| 11 | 33966510 | not yet published |
| 12 | 33966515 | not yet published |
| 13 | 33966520 | not yet published |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `9cdedd4a6f6e3fcd82d7b7ada08e72df77bbbad167a56ed655324ae9c6b05a62` |
