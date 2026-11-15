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

**Fallback seating,** if this game can't start and finish by the end of 17 November 2026 ET (GPT-OSS 120B leaves Antigravity on the 18th), or if the GPT-OSS seat can't be opened:

- the Tanners: Gemini, AI Studio (a second conversation), R&D records it at seating, before day 1.

**The GPT-OSS seat** (Publisher, 2026-11-14; one Antigravity conversation open at a time):
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
| the bell: SHA-256(`<day>:<salt>`) | `d4a404be7a17e6fcc2ec9f3d92e1b29d426e90fda5422e03d8dd25fb2728d2f5` |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966360 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966360 | `abdfcdb339fa528b99b2823f226547a3f55d4a27f596dcd3983202b03879d9de` |
| 2 | 33966365 | `c710508e4562523f0b38efb188ad9740f46b5002d93e40ee4ac1250d3118776b` |
| 3 | 33966370 | `6e77c03a51d2f5f49852de6626dae60ae804a154c1ec01d9d1059245b7d16591` |
| 4 | 33966375 | `3a711f90fa73e285253849c0cc1fa848c2da0cb75bb87e693fb473cb0e8c9475` |
| 5 | 33966380 | `0a5b8cec7fb778c6e02984c42098c2ebd5d73cd57962c1017d3dcdfee04dfc23` |
| 6 | 33966385 | `4fceccc4e241b2c589cec5a33aa14a6a48a304d9d9ac704b4356a2299270ace3` |
| 7 | 33966390 | `93bf83a63e2895e794848bcd7dce26fe34f87e73e1dc0c5e775cc5c4f8c4c848` |
| 8 | 33966395 | `0efeeefd220ca55669a8567702b203c8dce2f030d89e48c63a0cc92d0a92ea3c` |
| 9 | 33966400 | `36754e7560dd35c2bef3271829827cfe9c3ec15071fd35046a340a705ad006f3` |
| 10 | 33966405 | `226e63e860fd05f067b8dd5157e391143dc2ed492e45fb36107f1e976d89a113` |
| 11 | 33966410 | `c1e1671ab5f8218b2da1f79ad698815e4a62f0ca125ed9f10490b64ca8386c80` |
| 12 | 33966415 | `0be86d80713f4a7c535689c82c76f32832ae22ac56899a76eb0539e223593595` |
| 13 | 33966420 | `6b98da08c9c7da2134e529992027c3e5a1176ff1b27ef87c9b9d50960b4fbec1` |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `51cfe978fd2d812640f3586204b531a18e825bbcd8cf171013c7ef8a8497ea91` |
