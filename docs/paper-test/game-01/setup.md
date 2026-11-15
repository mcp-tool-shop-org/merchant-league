# Paper test, game 1: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Claude | subagent | claude-sonnet-5-5 |
| Weavers | GPT-OSS | Antigravity card, clean project | gpt-oss-120b |
| Smiths | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Foresters | Gemini | Gemini app | R&D records it at seating, before day 1 |

**Fallback seating,** if the GPT-OSS seat can't be opened:

- the Weavers: Claude, subagent, claude-sonnet-5-5.

**The GPT-OSS seat** (Publisher, 2026-11-14):
- it opens in a clean Antigravity project whose tree holds no other seat's setup, brief, mandate or order;
- its card says to read no files and play only from the conversation (every card does);
- each turn, the referee checks that it still remembers its live contracts. A lost contract is logged in `metrics.md` as a harness fault, not a play choice.

**Sitting out:** Dyers, Tanners.

## Markets

Step 1, stock 4 a day, buying at 1/2 of the reference price, sit-out goods at 3/2 times.

| Town | Sells |
|---|---|
| Varn | nothing |
| Ostra | brine from 1, +1 a unit, 4 a day; wool from 1, +1 a unit, 4 a day |
| Kell | ore from 2, +1 a unit, 4 a day; hide from 2, +1 a unit, 4 a day |
| Hath | timber from 2, +1 a unit, 4 a day; **leather at 14** (the Tanners sit out), 2 a day |
| Mire | madder from 2, +1 a unit, 4 a day; **dye at 14** (the Dyers sit out), 2 a day |

Every market buys converted goods, up to 3 of each a day, at: salt 2, charcoal 1, dye 4, tools 4, leather 4, cloth 7.

## Works

| Town | Works | Needs |
|---|---|---|
| Varn | Varn sea wall | 3 tools, 4 timber, 2 cloth |
| Kell | Kell bridge | 4 charcoal, 2 tools, 2 salt |
| Ostra | Ostra crane | 3 tools, 2 cloth, 3 salt |
| Hath | Hath lodge | 4 salt, 3 charcoal |
| Mire | Mire causeway | 4 timber, 2 tools, 2 dye |

40 units in all. A completed works scores a bonus of 4 to the guild that delivers its last unit.

## Edicts

Announced to the seats one day ahead, in that day's public state.

| Day | Edict | The referee's rule |
|---|---|---|
| 4 | Gate closed at Kell | No caravan enters or leaves Kell on days 4 and 5. A travel order across the gate is refused at the reveal; the caravan stays and pays no toll. |
| 8 | Levy on salt | On days 8 and 9, each unit of salt that passes between guilds (an offer fill or a payment) costs the receiving guild 1 coin, paid to the steward. |

## What's scarce

Dye (Mire market) and leather (Hath market) come only from sit-out prices, so cloth and the bellows are dear. Salt is the shared input: three works need it.

## Sealed until the bell

Each seat's card (its mandates) and the bell day are sealed; the referee holds them. They are added here after the game, and anyone can check them against these hashes.

| Sealed | SHA-256 |
|---|---|
| the Saltworks' card | `3ff82e125439d1ddf76da0015d0c1cf056f888a7c97b47a51f0457ba2014ef76` |
| the Weavers' card | `2bfbb14cdbc97aed7bcee2e87095923940c09eef7e20e7d4e1e1e493588ec2b1` |
| the Smiths' card | `047975e41e27b6e30591dbdb3d2d5a9d901b518c44d6ec86fb141c77dfd45545` |
| the Foresters' card | `41c047326aac2841cadf46b51a0d61ee8edeab7c5d47ab8028aeabae13558571` |
| the bell | committed once its round is published |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966260 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966260 | not yet published |
| 2 | 33966265 | not yet published |
| 3 | 33966270 | not yet published |
| 4 | 33966275 | not yet published |
| 5 | 33966280 | not yet published |
| 6 | 33966285 | not yet published |
| 7 | 33966290 | not yet published |
| 8 | 33966295 | not yet published |
| 9 | 33966300 | not yet published |
| 10 | 33966305 | not yet published |
| 11 | 33966310 | not yet published |
| 12 | 33966315 | not yet published |
| 13 | 33966320 | not yet published |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `c95d0546d7c440b2c0a7fa7f65a490465b7c770702c44c22a676f605d3edc8ac` |
