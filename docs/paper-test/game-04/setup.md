# Paper test, game 4: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Smiths | Claude | subagent | claude-sonnet-5-5 |
| Foresters | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Dyers | Gemini | AI Studio (a second conversation) | R&D records it at seating, before day 1 |
| Tanners | Claude | subagent | claude-sonnet-5-5 |

**Sitting out:** Weavers.

## Markets

Step 1, stock 3 a day, buying at 1/2 of the reference price, sit-out goods at 2 times.

| Town | Sells |
|---|---|
| Varn | nothing |
| Ostra | brine from 1, +1 a unit, 3 a day; wool from 1, +1 a unit, 3 a day; **cloth at 30** (the Weavers sit out), 2 a day |
| Kell | ore from 2, +1 a unit, 3 a day; hide from 2, +1 a unit, 3 a day |
| Hath | timber from 2, +1 a unit, 3 a day |
| Mire | madder from 2, +1 a unit, 3 a day |

Every market buys converted goods, up to 3 of each a day, at: salt 2, charcoal 1, dye 4, tools 4, leather 4, cloth 7.

## Works

| Town | Works | Needs |
|---|---|---|
| Varn | Varn sea wall | 4 tools, 6 timber, 2 cloth |
| Kell | Kell bridge | 3 tools, 3 charcoal, 2 leather |
| Kell | Kell gate tower | 2 tools, 2 leather |
| Hath | Hath mill | 3 tools, 3 salt, 3 timber |
| Mire | Mire dye house | 3 dye, 3 salt, 2 tools |
| Ostra | Ostra harbour | 3 leather, 3 dye, 2 charcoal |

49 units in all. A completed works scores a bonus of 4 to the guild that delivers its last unit.

## Edicts

Announced to the seats one day ahead, in that day's public state.

| Day | Edict | The referee's rule |
|---|---|---|
| 4 | Rough weather | On days 4 and 5, the safe roads roll on the hill-and-marsh table. |
| 8 | Gate closed at Varn | No caravan enters or leaves Varn on days 8 and 9. A travel order across the gate is refused at the reveal; the caravan stays and pays no toll. |

## What's scarce

No Weavers: the sea wall's 2 cloth cost 30 each on Ostra's market. Tools are wanted by five of six works, so the Smiths are the bottleneck. Varn's gate closes on days 8–9.

## Sealed until the bell

Each seat's card (its mandates) and the bell day are sealed; the referee holds them. They are added here after the game, and anyone can check them against these hashes.

| Sealed | SHA-256 |
|---|---|
| the Saltworks' card | `79849f260e0956497769363935489c96e54e9a6c8f67be8821832e8df671f7bf` |
| the Smiths' card | `d1cb8eff3b29e5b9aaa6b8fbca8c42d3927535af8b30c7b6c14c2484a5726d94` |
| the Foresters' card | `cc54b4de75c9648f7a2f64c804c46c2ceaabe6c4c3d083b7f407f2818b1972a2` |
| the Dyers' card | `fb8594878a58e4d58c08ad07866ca3854856dc9baffd74c88713912a08f8dfb9` |
| the Tanners' card | `56f9da73d47e28d175f6da32d66b2de5ab29d77e260dd138af81464f1c700bbc` |
| the bell | committed once its round is published |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966560 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966560 | not yet published |
| 2 | 33966565 | not yet published |
| 3 | 33966570 | not yet published |
| 4 | 33966575 | not yet published |
| 5 | 33966580 | not yet published |
| 6 | 33966585 | not yet published |
| 7 | 33966590 | not yet published |
| 8 | 33966595 | not yet published |
| 9 | 33966600 | not yet published |
| 10 | 33966605 | not yet published |
| 11 | 33966610 | not yet published |
| 12 | 33966615 | not yet published |
| 13 | 33966620 | not yet published |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `72f31c4900bf9e900a0dfa6afc171e6ecd6aaec72b65009d7914ee0842c61aa1` |
