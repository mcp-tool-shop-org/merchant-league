# Paper test, game 6: setup

**Not played.** The paper test ended after game 2 (Publisher, 2026-11-15), once the v2 design (`docs/design-v2.md`) was approved. This setup stays as it was fixed, and its seals (the cards and the bell) are not opened.

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | GPT-OSS | Antigravity card, clean project | gpt-oss-120b |
| Weavers | Claude | subagent | claude-sonnet-5-5 |
| Smiths | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Foresters | Claude | subagent | claude-sonnet-5-5 |
| Tanners | Gemini | AI Studio | R&D records it at seating, before day 1 |

**Fallback seating,** if this game can't start and finish by the end of 17 November 2026 ET (GPT-OSS 120B leaves Antigravity on the 18th), or if the GPT-OSS seat can't be opened:

- the Saltworks: Gemini, AI Studio (a second conversation), R&D records it at seating, before day 1.

**The GPT-OSS seat** (Publisher, 2026-11-14; one Antigravity conversation open at a time):
- it opens in a clean Antigravity project whose tree holds no other seat's setup, brief, mandate or order;
- its card says to read no files and play only from the conversation (every card does);
- each turn, the referee checks that it still remembers its live contracts. A lost contract is logged in `metrics.md` as a harness fault, not a play choice.

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
| the bell: SHA-256(`<day>:<salt>`) | `90d8043369d7f89f73043d2aa146b6ae98087137d7495c49233b60df41c4c6e7` |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966760 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966760 | `740de7c3c3ff352aa7f1ef25d5f6fe113bdef2212c08ea5ff8b67dbcb9a5cc8c` |
| 2 | 33966765 | `7e2e9e67abd9a8dbb91c306bcc5230d06fe410e40b81336617e4d52a962be052` |
| 3 | 33966770 | `894ef6fdc2fc3b205c156de4b105513661397e6ee45185a43f70257a914aceb7` |
| 4 | 33966775 | `f0a3d8a043de8affe732480c73307821c44a73e41a3b0bd5b500d3cb391c5eb8` |
| 5 | 33966780 | `0d49810418b4b0147e00cfed038f40caecfcf5c69c6fa33eee8eaba1d3cc48f7` |
| 6 | 33966785 | `03c035258a0e0523fbf84486d12e724c3adb41645ff4594a63e80a539256507d` |
| 7 | 33966790 | `675d13010215eda0280e609e212431fdd3b5f44fdbc4476ff89ecc296bbd2ebb` |
| 8 | 33966795 | `5486b5026b68e30737912087bd4c3c11264c67784e4e01977b72623ad07ecd5e` |
| 9 | 33966800 | `4d568dd9fb01f7d77fe04ad72267ac7b217023788671e594c332a007006a297a` |
| 10 | 33966805 | `e54c0753d441c3590c09348ec4c7b7272bcb37d0cb3a4632d1ace9520c6d838b` |
| 11 | 33966810 | `3a5bf8847a439ce5fdc30f4ceaeeba3912d7b2835a57b65e63ecaaa59c800439` |
| 12 | 33966815 | `50d9e83ac23cb1ce75f8513435c48241d010884ec3b5c40b4270609c47208034` |
| 13 | 33966820 | `a477afeddb68adb70d02754fb45daed9e129770cee17928a484a1cf6db7e8426` |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `e1a27079fe2537cb9f1318e475970aaf84d41b8ed446737a4330842d72243ecc` |
