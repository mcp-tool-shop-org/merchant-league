# Paper test, game 1: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Claude | subagent | claude-sonnet-5-5 |
| Weavers | GPT-OSS | Antigravity card, clean project | gpt-oss-120b |
| Smiths | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Foresters | Gemini | Gemini app | R&D records it at seating, before day 1 |

**Fallback seating,** if this game can't start and finish by the end of 17 November 2026 ET (GPT-OSS 120B leaves Antigravity on the 18th), or if the GPT-OSS seat can't be opened:

- the Weavers: Claude, subagent, claude-sonnet-5-5.

**The GPT-OSS seat** (Publisher, 2026-11-14; one Antigravity conversation open at a time):
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
| the bell: SHA-256(`<day>:<salt>`) | `74c41dc09c3d0b630d69d5b95845f06494ef3791326816b988e1c2a446518270` |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966260 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966260 | `4b9a645f44faf1ca8ef3b4602d4b0090505d333eb92b471b8e2487fc3724461f` |
| 2 | 33966265 | `a2c50779b9bdb91915409ac6d21ead8f532ea5ae76884b47dfc12091763c536d` |
| 3 | 33966270 | `ced76f9fceb709bf9cdaab5b3ded3f2fbe1a2b6288c6088e04546b007cc62073` |
| 4 | 33966275 | `7f0b05a7e44cd4e30d023a10de3a492e685a1420ed995bf79e03bd330f47f344` |
| 5 | 33966280 | `2b2aaf6c54aeabb345b0d062ff2c6253252ca8aea090c62288323d708c28fa02` |
| 6 | 33966285 | `735c6ff5e4f5bdb6907849fd75ada87a0244a038a80c53827a910a44dc03950f` |
| 7 | 33966290 | `abecf4569b0f22e56e98c90aa8ddd2cff54a99de2d81a215bf3b421ddabcd27e` |
| 8 | 33966295 | `68c87eb06feab8d3cecc5bada832b3eeddc0033b0b371ebbf7bd56cac5a6b82a` |
| 9 | 33966300 | `de6b318efbb6d121537d0ed1f36bf7835ea23006518168ca1f3d3ef17312bc5a` |
| 10 | 33966305 | `fbe51f5f9e9270261c84d859f11d9d8d6d82ccb7c031964d81d0353e6fa75c2c` |
| 11 | 33966310 | `ecb2950c9d9eb97dfb0c63bfb98f50fcc4db680b8ff2b8a5502f7679fe2251e7` |
| 12 | 33966315 | `48dd139d5431196db9c53bd010bcb5ba86c547403d2f80585279c73de6d29b54` |
| 13 | 33966320 | `ee2b7ebd31b9f817d9ac8edc008ad1208f21ca7f69f6a44f3398e63a2a6b92b5` |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `c95d0546d7c440b2c0a7fa7f65a490465b7c770702c44c22a676f605d3edc8ac` |
