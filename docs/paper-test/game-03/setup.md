# Paper test, game 3: setup

Built by hand to the generator's recipe (`docs/design.md`, "Generated tables"), on the board in `../board.md`. Fixed 2026-11-14 19:02 ET, before any of its rounds was published.

## Seats

| Guild | Family | Surface | Model |
|---|---|---|---|
| Weavers | Gemini | AI Studio | R&D records it at seating, before day 1 |
| Foresters | GPT-OSS | Antigravity card, clean project | gpt-oss-120b |
| Dyers | Gemini | Gemini app | R&D records it at seating, before day 1 |
| Tanners | Claude | subagent | claude-sonnet-5-5 |

**Fallback seating,** if this game can't start and finish by the end of 17 November 2026 ET (GPT-OSS 120B leaves Antigravity on the 18th), or if the GPT-OSS seat can't be opened:

- the Foresters: Claude, subagent, claude-sonnet-5-5.

**The GPT-OSS seat** (Publisher, 2026-11-14; one Antigravity conversation open at a time):
- it opens in a clean Antigravity project whose tree holds no other seat's setup, brief, mandate or order;
- its card says to read no files and play only from the conversation (every card does);
- each turn, the referee checks that it still remembers its live contracts. A lost contract is logged in `metrics.md` as a harness fault, not a play choice.

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
| the bell: SHA-256(`<day>:<salt>`) | `90d8e268fad9d07a21a6e1fa2e44cf8ee18197da69383909e6a77bad524c9ccf` |

## Rounds

drand quicknet, read as in `../board.md`. Day *k* uses round 33966460 + 5·(*k* − 1). The bell's round is sealed.

| Day | Round | Randomness |
|---|---|---|
| 1 | 33966460 | `7442aabca34fca5364bfe0bbc84eb7483869561f39e2b2f68e2cdf7fc398d8cd` |
| 2 | 33966465 | `0a7d093d9823cc29d50c15eb6c2ddbb95be9630ebcd285112448121b3e4f1de7` |
| 3 | 33966470 | `8dce66616c70ec1cb661e1892ed9b3046549c6c578bc3810b2f2e160590d5d1c` |
| 4 | 33966475 | `8ff042463ca4ddc253d4b8d6102e904bc82148a6dcfa1657ebea33cd83032d62` |
| 5 | 33966480 | `d7e6b19e0cf1cb73f0d4c8dc8ed7466d12318985ade9fafb6fd6e2c7a2efd2c4` |
| 6 | 33966485 | `1bfeb70cfb26dd0f33ff3c18084d390b8e860d85bfe4243e70eca3261eb08c07` |
| 7 | 33966490 | `7b8f5d721b7726a0331504781a2a7dc27f3891de8d48f92f7f0a68c47c06a7b7` |
| 8 | 33966495 | `5c1ed214ab6f267d92cc6c4dd9bfd53ee277ff58364aa9a21c4881f4136224f5` |
| 9 | 33966500 | `9035b4904d5eac094e02577d22cff1785057b8d314e35d1a47cc73d4e88b35f1` |
| 10 | 33966505 | `c99ba9159dba061eb0c93e44363092e99346a2c0b7f7a2749cfdc06b624d8ca9` |
| 11 | 33966510 | `ef4fa5bfc5f60d501c2a4ca6b764981b83e5569e28a602607288008f4a94bec0` |
| 12 | 33966515 | `10f98cd5807f43d85fbbe958a9b06a268d3f0a2b0c247aaba3de9fa2c39a229e` |
| 13 | 33966520 | `6459e3c85f71775ee7db5b053af840afd4bccd7843fbe19ed2575ec1f1e3c78e` |

## Pins

| What | Pin |
|---|---|
| Seat brief | `brief.md` v1, SHA-256 `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` |
| Board | `board.md`, SHA-256 `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` |
| Rules | `docs/design.md`, SHA-256 `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` |
| Skeleton | `9cdedd4a6f6e3fcd82d7b7ada08e72df77bbbad167a56ed655324ae9c6b05a62` |
