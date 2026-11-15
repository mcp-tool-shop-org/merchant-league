# Paper test, game 4: setup

**Not played.** The paper test ended after game 2 (Publisher, 2026-11-15), once the v2 design (`docs/design-v2.md`) was approved. This setup stays as it was fixed, and its seals (the cards and the bell) are not opened.

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Saltworks | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Smiths | Claude | subagent | claude-sonnet-5-5 |
| Foresters | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Dyers | GPT-OSS | Antigravity card, clean project | gpt-oss-120b |
| Tanners | Claude | subagent | claude-sonnet-5-5 |

**Fallback seating,** if this game can't start and finish by the end of 17 November 2026 ET (GPT-OSS 120B leaves Antigravity on the 18th), or if the GPT-OSS seat can't be opened:

- the Dyers: Gemini, AI Studio (a second conversation), R&D records it at seating, before day 1.

**The GPT-OSS seat** (Publisher, 2026-11-14; one Antigravity conversation open at a time):
- it opens in a clean Antigravity project whose tree holds no other seat's setup, brief, mandate or order;
- its card says to read no files and play only from the conversation (every card does);
- each turn, the referee checks that it still remembers its live contracts. A lost contract is logged in `metrics.md` as a harness fault, not a play choice.

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
| the bell: SHA-256(`<day>:<salt>`) | `391e1657af4b4de4876845c0b90084a8de9500ffc52f76591ffa9a0696591d35` |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966560 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966560 | `22f9de975c29f12891642d4cebbb9ccd85e3971eaa48aa984b03b3d1f3119a22` |
| 2 | 33966565 | `5829aad88fbe6d8b8762013eb13a6379d6a58245bb72f284cc1c08cd13a2b6e2` |
| 3 | 33966570 | `0fc1420c6be27f7d5a729e223001df02918e776915f507a916f179649900c11b` |
| 4 | 33966575 | `5989018805d4ba422295ced7c329f4c48d109370cc2b4e8c7fe597cf3ad0f1b6` |
| 5 | 33966580 | `24b80ae964963eb8b8416808a2a61099c184ab6a8b8d4477947355c6a5b1f642` |
| 6 | 33966585 | `4e9df11070f179c0eac7501da1dea3724ca433adc01964faba625cf9c7762cf0` |
| 7 | 33966590 | `4f75c095dc5df867b5c4d7cb1c0ca1fd5406d84f8f10f296d8ff421f90626dbb` |
| 8 | 33966595 | `568402790d21ca8df65c052ed3c098f28039ffadae32c1ed6cbc135d8e7f3462` |
| 9 | 33966600 | `7e9f7240e144f7e04b90dcaae35974f5d88c41e546181c1c4f139d473f0ddbb7` |
| 10 | 33966605 | `738cbcb14057cd84cf129eee99d5f85620ff160fda468b6b94c0bb8e02b37294` |
| 11 | 33966610 | `a95d40d546b1afe32dec8049c6ddc67ee44a984a09b38d4ad67ced0d9a869fb4` |
| 12 | 33966615 | `5c5f229590c9b800e171cb97b690a7fb6487c134e116ce3737f577582113558a` |
| 13 | 33966620 | `7e873ac7ff13ec908159e91cbaed5f2f2662c7a39b4ddd5a50d03e1c5544726d` |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `72f31c4900bf9e900a0dfa6afc171e6ecd6aaec72b65009d7914ee0842c61aa1` |
