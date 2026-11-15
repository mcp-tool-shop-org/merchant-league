# Merchant League paper test, game 1: referee log

Referee: referee-g01 (Claude session, non-playing). Brief: R&D's referee brief (kept outside this repository), SHA-256 `567eae0223849f50c74cdac5003b47db2d9afb515fd0505ef85317510a009d49` (checked before reading).

## Header

**Setup commit:** this repository at `5666e926aa27da0817519c00f01f5a88d559815c`, read with `git show` only (no checkout, no edit).

**Pins checked before day 1** (SHA-256 of `git show <commit>:<path>` output, against `docs/paper-test/game-01/setup.md` "Pins"):

| File | Measured | Pin in setup.md | Result |
|---|---|---|---|
| `docs/paper-test/brief.md` | `b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded` | same | match |
| `docs/paper-test/board.md` | `48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c` | same | match |
| `docs/design.md` | `b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf` | same | match |
| `docs/paper-test.md` | `cc262c63cded587503e58ffbb38aba9e7c3cec86c21ac76700f3fa5d1172c70e` | not pinned in setup.md | recorded only |
| `docs/paper-test/game-01/setup.md` | `16ad6c489c0686dc1c83ad496396bcab27178db5757cf28ef62bdd504433e6a4` | (the setup itself) | recorded |
| Skeleton | — | `c95d0546d7c440b2c0a7fa7f65a490465b7c770702c44c22a676f605d3edc8ac` | not checkable: no file or recipe for it is in the referee's read list |

**Cards** (`cards/<Guild>.md`, against setup.md "Sealed until the bell"):

| Card | Measured | Result |
|---|---|---|
| Saltworks | `3ff82e125439d1ddf76da0015d0c1cf056f888a7c97b47a51f0457ba2014ef76` | match |
| Weavers | `2bfbb14cdbc97aed7bcee2e87095923940c09eef7e20e7d4e1e1e493588ec2b1` | match |
| Smiths | `047975e41e27b6e30591dbdb3d2d5a9d901b518c44d6ec86fb141c77dfd45545` | match |
| Foresters | `41c047326aac2841cadf46b51a0d61ee8edeab7c5d47ab8028aeabae13558571` | match |

**state.json:** its three pins match setup.md; its 13 day rounds are 33966260 + 5·(k − 1) for k = 1..13 and each randomness value equals setup.md's "Rounds" table (13 of 13). Its bell commitment string equals setup.md's. **The bell itself is checked against the commitment only at the bell**, per the brief.

**Seats** (setup.md "Seats"; surfaces and models as recorded by R&D at seating):

| Guild | Family | Surface and model |
|---|---|---|
| Saltworks | Claude | Claude Sonnet subagent (claude-sonnet-5-5) |
| Weavers | GPT-OSS | GPT-OSS 120B, Medium, in a clean Antigravity project |
| Smiths | Gemini | Gemini 4 Carbon, High, in AI Studio, grounding off |
| Foresters | Gemini | Gemini Carbon, High, Gemini app, temporary chat |

Sitting out: Dyers, Tanners. Main seating used (not the fallback).

---

## Day 1

**Drand round:** 33966260, randomness `4b9a645f44faf1ca8ef3b4602d4b0090505d333eb92b471b8e2487fc3724461f` (applied at resolution).

### Talk round: public state sent (start of day 1)

- Caravans: Saltworks in Varn; Weavers in Ostra; Smiths in Kell; Foresters in Hath (board.md "Guilds and converters": the start).
- Markets: full stock at setup prices; nothing sold.
- Open offers: none.
- Works: nothing delivered. Varn sea wall 3 tools, 4 timber, 2 cloth; Kell bridge 4 charcoal, 2 tools, 2 salt; Ostra crane 3 tools, 2 cloth, 3 salt; Hath lodge 4 salt, 3 charcoal; Mire causeway 4 timber, 2 tools, 2 dye.
- Contracts and bonds: none. Standings: all 0.
- Edict: none in force, none announced (the day-4 edict is announced in day 3's public state, setup.md "Edicts").
- Each guild: purse 60 coin, no goods, standing 0, caravan at home; inbox empty.

Each block held: the full text of `brief.md` (pinned), that seat's own card only, the public state above, its own position, and the talk instruction with reply form `MESSAGE n TO <Guild>: <text>` (up to 3, at most 80 words each) or `NO MESSAGES`.

SHA-256 of each day-1 talk block as sent to R&D (UTF-8, LF, from `=== TO` to `=== END … ===` inclusive):

| Seat | Block SHA-256 |
|---|---|
| Saltworks | `1f8614660852133b88e5165e112b5a0f735aeaf7f78863ed7755feb8dd663288` |
| Weavers | `c8dac98d66c85034011ffabb5bd4d10e2a7137c2112a475e23b8ccff3c92c81c` |
| Smiths | `89d5471a704f7ad9e5673ce6e191e68786bb6a6f4b3ec1e315bb987e46e198f6` |
| Foresters | `4dfb3482963c9711a082b6ce94be58aa4376965523f8ec46b9de112238f616a2` |

**Carry protocol change (R&D, after the day-1 talk blocks were sent):** every handoff is now a file. The referee writes each seat's block to `to-seat/day-DD-<talk|act>-<Guild>.md` (the text between the delimiters, delimiters excluded, UTF-8, LF, no trailing newline added) and records that file's SHA-256 here; seat replies come back as `from-seat/day-DD-<talk|act>-<Guild>.md`, written verbatim by R&D, or `NO REPLY`. This adds `to-seat/` to the files the referee writes beyond the brief's list (R&D's instruction).

**Hash convention:** the table above hashes the whole block *including* the `=== TO <Guild> ===` and `=== END <Guild> ===` lines (each followed/preceded by one LF). Re-checked: the bodies cut from those same blocks hash back to the table's values when the delimiters are put back. The files hash as follows (body only, the convention from here on):

| Seat | File | File SHA-256 (body, no delimiters) |
|---|---|---|
| Saltworks | `to-seat/day-01-talk-Saltworks.md` | `10dff61f9d20195117488cc2f7f1fca47d2828d5416f9785f3882d481995386b` |
| Weavers | `to-seat/day-01-talk-Weavers.md` | `3f9ce7695cba74e47e6e6f9df66cd777a79278b2f2b12072dc0272d57b071b6d` |
| Smiths | `to-seat/day-01-talk-Smiths.md` | `5c0dc7ba9e94322bf8ef1e5023a0332411f34e11fde6c5a2e7437553d5fc234d` |
| Foresters | `to-seat/day-01-talk-Foresters.md` | `227fc74e0953b41ecb8a270c8290dee1b30d0a741d16032ae0aeaf394546ff8e` |

### Talk replies

Read from `from-seat/` (written verbatim by R&D). SHA-256 measured on disk; each matches the hash R&D listed.

#### Saltworks: `from-seat/day-01-talk-Saltworks.md`, `948a49985502d7a915b67736424ac42d46500b9f2d6f2beb0bf6aa8cbd6a45ce`

```
MESSAGE 1 TO Foresters: Saltworks here. My pans burn 1 timber each conversion day, and I'm in Varn, far from Hath. I'd buy timber from you at 3 coin each, 4 units, delivered by day 3. I can also pay in salt later. Salt is the only thing that unlocks the Hath lodge, Kell bridge and Ostra crane, and I'm its only maker. Want a standing deal?
MESSAGE 2 TO Smiths: Saltworks here. Kell bridge needs 2 salt and Ostra crane needs 3. I'm the only salt source in this game. I can sell salt at 4 coin each, starting around day 3. Tell me how many salt you want and where, so I can plan my pans. Happy to agree a contract with bonds.
MESSAGE 3 TO Weavers: Saltworks here. You're next to Ostra's brine market, and I need brine for salt. If you pass through Ostra, would you buy 4 brine for me at cost plus 1 coin each, paid on delivery? In return I'll give you first call on salt for the Ostra crane, 3 salt needed.
```

#### Weavers: `from-seat/day-01-talk-Weavers.md`, `6d2aedf776b31bfc37f0bb58127f1f4f5e4da46d3858485974600af2bd626ef1`

```
NO MESSAGES
```

#### Smiths: `from-seat/day-01-talk-Smiths.md`, `ee379c99bbc412e9575ea23c27c920b72a12244207a3fc45324b73ed795912b4`

```
MESSAGE 1 TO Foresters: Let's partner! Guild offers need no shared town, saving hill travel. If today in Hath you buy 1 leather (14 coin) and make 2 charcoal, Day 2 I'll pay 17 coin for 1 leather and 10 coin for 2 charcoal to forge tools Day 2. Day 3 we swap 1 tools for 2 charcoal, giving you better axes on Day 3, earliest possible! I'll keep buying charcoal after too.
MESSAGE 2 TO Saltworks: Greetings from Kell! Notice Varn sea wall needs 3 tools and zero salt, while Kell bridge needs 2 salt. Since guild offers work across towns without travel, once your salt batches start around Day 3 or 4, I'd love to swap tools for 2 salt (or buy 2 salt for good coin) so neither of us wastes travel days. When do you expect salt ready, and will you want tools for Varn?
MESSAGE 3 TO Weavers: Hello neighbor! I'm ramping up tools production in Kell starting Day 2-3. Both Ostra crane and Varn sea wall need 3 tools, and I know Mire's 14-coin dye makes cloth expensive to fund. Would you prefer to coordinate joint deliveries on Ostra crane, or trade for tools via Offers or short safe-road contracts for standing? Let me know your plans so we can work together smoothly!
```

#### Foresters: `from-seat/day-01-talk-Foresters.md`, `43d55c4a740ce2398d4d9ed2d3eb9f985d4f2db5278f7b307bfed5e4f34c5d23`

```
MESSAGE 1 TO Smiths: Staying in Hath today to make 2 charcoal; grab ore in Kell! Since offers fill before conversions, let's run a 50/50 split with zero idle days: Day 2 we cross 2 charcoal for 16 coin so you convert 2 tools immediately on Day 2. Day 3 we cross 2 charcoal for 16 coin and 2 tools for 32 coin. Zero net coin, anti-snipe prices, and 2 tools each by Day 3! Deal?
MESSAGE 2 TO Saltworks: Heading to Ostra for 4 brine today? Since offers fill across towns before conversions, when you return to Varn on Day 2 I can sell you timber remotely so you convert 2 salt immediately without trekking to Hath! Proposal: Day 2 cross 2 timber for 10 coin (unlocks 4 salt); Day 3 cross 1 salt for 10 coin (or I'll add coin for 2 salt). Interested?
MESSAGE 3 TO Weavers: Quick win-win math: two full cloth runs (8 wool plus 4 dye) cost 76 coin, 16 above starting purse, so solo you're capped at 3 cloth. If I buy your 4th cloth for 22 coin, you fund both full runs in the exact same days, still keep 3 cloth, and pocket extra coin! Also, let's coordinate Mire dye days when I bring timber to Mire causeway. Interested?
```

**Whispers check:** 9 messages (Saltworks 3, Smiths 3, Foresters 3, Weavers `NO MESSAGES`). Every message is in the form, addressed to a seated guild, and at most 80 words (longest 72). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Smiths (msg 2), from Foresters (msg 2).
- Weavers: from Saltworks (msg 3), from Smiths (msg 3), from Foresters (msg 3).
- Smiths: from Saltworks (msg 2), from Foresters (msg 1).
- Foresters: from Saltworks (msg 1), from Smiths (msg 1).

**Weavers contract-memory check (talk, day 1):** no live contracts; nothing to check.

### Act round: blocks sent

Each block: the public state (unchanged), the seat's own position, its inbox verbatim (prefixed `FROM <Guild>:`), and the act form. Format conventions stated to every seat (form, not new rules): several orders on one line are separated by ` ; `; ids for offers and contracts posted today appear in tomorrow's public state; a reply without a TRAVEL line counts as a missed reveal (design.md "A market day" item 2: every day's transaction carries the sealed travel order, and "Fair dice": a guild that doesn't reveal stays and pays the toll; board.md rule 2 sets that toll at 2).

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-01-act-Saltworks.md` | `dc7e614d346ab53bfa4651fe87b8240860aa6934bc9cad6ba3a8a0e02cfbf1d6` |
| Weavers | `to-seat/day-01-act-Weavers.md` | `047a652f8a0d37008533578c4454e9d6c1b9a0390dc85fbfe3ae7bbd294f93c5` |
| Smiths | `to-seat/day-01-act-Smiths.md` | `dc87692776820a42e041eb817f3cb6dfa3a1966e9b400085048d4a2ab89136d8` |
| Foresters | `to-seat/day-01-act-Foresters.md` | `2834cb549e457835d30177cabdb9497ac1440e872c42fbc2d77b39a5d9c80868` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths reply is AI Studio's synced chat record (raw model text); the Foresters reply is the Gemini app's rendered text (the arrow in its CONVERT line is as the seat wrote it).

#### Saltworks: `from-seat/day-01-act-Saltworks.md`, `8d3ffefb67bad1c858dad788bce8079995d0cc7f03c10c354ea986eb54593345`

```
OFFERS: post 10 coin for 4 brine @Ostra
TRAVEL: go to Ostra
```

#### Weavers: `from-seat/day-01-act-Weavers.md`, `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155`

```
TRAVEL: stay
```

#### Smiths: `from-seat/day-01-act-Smiths.md`, `37b3be4aa8b27bd0bf06d2dcfd15bf6cd53a7adc78ff96183543ed3de76daaa1`

```
OFFERS: post 9 coin for 3 ore @Kell
TRAVEL: stay
```

#### Foresters: `from-seat/day-01-act-Foresters.md`, `8d702ec8caa9f64fca0723d9efc6186381c9cdc7ac38b150c4b5754a1749bbfe`

```
OFFERS: post 14 coin for 4 timber @Hath ; post 14 coin for 1 leather @Hath
CONVERT: 1 timber → 1 charcoal x2
TRAVEL: stay
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 1):** no live contracts; `TRAVEL: stay` only. Consistent.

### Resolution, day 1

**Step 1. Sealed travel orders** (paper-test.md "turn protocol" step 1). SHA-256 of each seat's TRAVEL line exactly as written (UTF-8, no newline), recorded before the reveal. On paper there is no salt, so identical orders hash alike.

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Ostra` | `e81dca8522bda28e703276369bdaab4c461138b5c641a68feca607d271d2cc03` |
| Weavers | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |

Revealed: Saltworks Varn → Ostra (road 0); Weavers, Smiths, Foresters stay.

**Step 2. The day's roll** (design.md "Fair dice"; setup.md "Rounds"): quicknet round 33966260, randomness `4b9a645f44faf1ca8ef3b4602d4b0090505d333eb92b471b8e2487fc3724461f`.

**Step 3. Road events, then movement** (board.md "Road events" and "Reading dice": road r = (byte 2r × 256 + byte 2r+1) mod 20 + 1).

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 4b 9a | 19354 | 15 | clear |
| 1 Ostra–Kell | safe | 64 5f | 25695 | 16 | clear |
| 2 Kell–Hath | hill | 44 fa | 17658 | 19 | clear |
| 3 Hath–Mire | safe | f1 ca | 61898 | 19 | clear |
| 4 Mire–Varn | marsh | 8e f3 | 36595 | 16 | clear |
| 5 Ostra–Mire | short cut | b4 60 | 46176 | 17 | clear |

Saltworks travels road 0: clear, arrives in Ostra. The others stay and meet no event (board.md: "A caravan that stays in town meets no road event"). No tolls, bandits, floods or fairs.

Caravans after moving: Saltworks Ostra; Weavers Ostra; Smiths Kell; Foresters Hath.

**Step 4. Offers fill** (paper-test.md step 4; board.md "Town ladders"; board.md rule 1: a ladder trade needs the caravan in that town, checked after movement, since caravans move before offers fill).

Fill order among equal prices, SHA-256(`<randomness>:<guild>`), lowest first: Smiths `410965de…`, Saltworks `77eb05c1…`, Weavers `e3ee2aaf…`, Foresters `eb50e02a…`. Not needed today: no two offers competed for the same ladder stock, and no guild-to-guild offers were posted.

| Offer | Caravan in town? | Ladder price | Result |
|---|---|---|---|
| Saltworks: 10 coin for 4 brine @Ostra | yes (arrived) | brine step 1 from 1: 1+2+3+4 = 10 | filled: Saltworks −10 coin, +4 brine |
| Smiths: 9 coin for 3 ore @Kell | yes | ore step 1 from 2: 2+3+4 = 9 | filled: Smiths −9 coin, +3 ore |
| Foresters: 14 coin for 4 timber @Hath | yes | timber step 1 from 2: 2+3+4+5 = 14 | filled: Foresters −14 coin, +4 timber |
| Foresters: 14 coin for 1 leather @Hath | yes | sit-out leather 14, no stepping, 2 a day | filled: Foresters −14 coin, +1 leather |

Every bid equalled the ladder's price exactly, so no partial or overpaid fill arose. Nothing stays open.

**Step 5. Conversions and works deliveries** (board.md "Guilds and converters").
- Foresters in Hath: cut 2 timber free (board.md "Cutting"), then convert 1 timber → 1 charcoal ×2 (2 batches, the daily cap; at home). Timber 4 + 2 − 2 = 4; charcoal +2.
- No other conversions. No works deliveries.

**Step 6. Perishables** (board.md "Perishables": hide and madder). None held. Nothing spoils.

**Step 7. Contracts at deadline** (design.md "The steward's bond rule"). No contracts.

**Step 8. Ladders refill** at the close (board.md "Stepped prices"): every ladder back to full stock for day 2.

**Rule gap noted (provisional):** *When the Foresters' free cut happens in the day's order.* board.md says they cut 2 timber "on any day the Foresters' caravan is in Hath" but doesn't place it in the resolution order. Decided provisionally: at step 5, before conversions, with the caravan's position after movement (so a Forester who arrives in Hath that day cuts; one who leaves doesn't). Today it changed nothing: the Foresters had 4 bought timber to convert either way.

**Rule gap noted (provisional):** *Are filled offers public?* design.md "A market day": "Offers are public by nature; only travel is sealed." Decided: yes. The next day's public state lists the previous day's filled offers (guild, what, where) and the day's road events. Holdings and purses of other guilds are not listed (the brief gives each seat its own).

### State after day 1

| Guild | Caravan | Purse | Goods | Standing |
|---|---|---|---|---|
| Saltworks | Ostra | 50 | 4 brine | 0 |
| Weavers | Ostra | 60 | — | 0 |
| Smiths | Kell | 51 | 3 ore | 0 |
| Foresters | Hath | 32 | 4 timber, 2 charcoal, 1 leather | 0 |

Works: nothing delivered (40 units still needed). Open offers: none. Contracts and bonds: none. Steward's take: 0 (no tolls). Missed reveals: 0 of 4.

### Referee word count, day 1

1578 words in this log's day-1 section, excluding the seats' verbatim replies (fenced blocks). The per-seat blocks are counted separately in metrics.md.

---

## Day 2

**Drand round:** 33966265, randomness `a2c50779b9bdb91915409ac6d21ead8f532ea5ae76884b47dfc12091763c536d` (applied at resolution).

### Talk round: blocks sent

Each block: day-1 public results (road events, moves, filled offers, per rule gap G2), the public state at the start of day 2 (as "State after day 1" above, without other guilds' holdings), the seat's own purse, goods, standing, caravan and mandates, an empty inbox, and the talk form. No edict in force or announced (day 4's is announced on day 3).

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-02-talk-Saltworks.md` | `3c2ba14ebc666e3fbdf85a31d83ff5d45fded1106a4cc51fe4be3b533fb583a5
` |
| Weavers | `to-seat/day-02-talk-Weavers.md` | `21189af20f386d13e4dc20379edc76e7eb79d8780d823a33871f490c4cc4a13b
` |
| Smiths | `to-seat/day-02-talk-Smiths.md` | `7aa59fe7116e7450699e0c319211632c3a682dcd9516efe11046187723dac77e
` |
| Foresters | `to-seat/day-02-talk-Foresters.md` | `cdd462a420a00eb544b880ede1f79dc27dde9b6d92cd3a5fb18e029f07a34a07
` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry note from R&D: the Smiths text was taken from the rendered AI Studio page (the synced record had not caught up); R&D will check it against the synced copy and report any difference.

#### Saltworks: `from-seat/day-02-talk-Saltworks.md`, `f9591a587eb4bd590abff0118f4aa8c912f1abe4d1a3069225a26787cfadd719`

```
MESSAGE 1 TO Foresters: Yes to timber. I'm heading home to Varn with 4 brine. Please post 2 timber for 10 coin so my pans run on day 3 (I convert 2 batches, 1 timber burned). I'll offer 1 salt for 10 coin on day 3 as you proposed. Cross offers at those prices and they fill.
MESSAGE 2 TO Smiths: Salt comes from day 3 at Varn, 2 per conversion day. I can sell you 2 salt for Kell bridge at 5 coin each, delivered to Kell by day 6. Say yes and I'll propose a contract. Tools for Varn can wait; salt is what I'm short on funds for.
```

#### Weavers: `from-seat/day-02-talk-Weavers.md`, `615f4b77bfdf7c2e83d3e402d8d18fecdf131525cff97d3b11d42eec51498c8f`

```
MESSAGE 1 TO Saltworks: I can buy 4 brine from you at cost +1 coin each, paid on delivery at Ostra. In return, I’d like first call on the 3 salt needed for the Ostra crane.
MESSAGE 2 TO Foresters: Your offer to buy my 4th cloth for 22 coin is appreciated. I’ll work to produce enough cloth and will coordinate timing and price once ready.
```

#### Smiths: `from-seat/day-02-talk-Smiths.md`, `78f944840ea68928b6d0006aa29feab5a5005bd32bea7947b309180ed956bc1e`

```
MESSAGE 1 TO Foresters: Thanks for grabbing the leather! Deal on your 50/50 split! Today in Day 2 Act, let's BOTH definitely use 17 coin for 1 leather and 16 coin for 2 charcoal (even if your Day 2 note said 10, post 16 so we're guaranteed to match!). Day 3 we cross 16 coin for 2 charcoal and 2 tools for 32 coin. Days 4-5, can I buy 4 more charcoal at 5 coin each?
MESSAGE 2 TO Saltworks: 4 coin per salt is a deal! I'll take 2 salt (8 coin) from your first Varn batch. Since guild trades don't require me in Varn (only the seller must be in the contract's town), do you prefer crossing an Offer (8 coin for 2 salt) on Day 3 or 4, or a contract at Varn by Day 4 or 5 so you gain +1 standing? Also happy to supply tools!
MESSAGE 3 TO Weavers: Checking in from Kell! I'll begin forging tools starting today. If you're planning cloth for Ostra crane or Varn sea wall, let me know how you'd like to coordinate works deliveries, trade for tools via Offers, or set up easy contracts along the safe Kell-Ostra road!
```

#### Foresters: `from-seat/day-02-talk-Foresters.md`, `0f25c894f66fa5ad71551bc917896397f3caa83dfa783088913c6a86fa3581e4`

```
MESSAGE 1 TO Smiths: Deal! I bought 1 leather and made 2 charcoal yesterday as agreed. Today (Day 2) let's cross your exact offers: I'll post 1 leather for 17 coin and 2 charcoal for 10 coin; you post 17 coin for 1 leather and 10 coin for 2 charcoal. Day 3 we cross 2 charcoal for 1 tools. Want to cross another 2 charcoal for 1 tools on Day 4 too, plus coin for extra tools?
MESSAGE 2 TO Saltworks: Happy to partner! Good news: guild OFFERS fill immediately across any towns with zero travel, and I already hold 4 timber. Replacing 4 timber at Hath's market costs 14 coin (2, 3, 4, 5), so let's cross today (Day 2): you post 15 coin for 4 timber, and I'll post 4 timber for 15 coin. Once you boil salt, I'll gladly buy salt from you at a fair price!
MESSAGE 3 TO Weavers: Hey Weavers! Reaching out again in case you want to team up. Mire's market stocks only 2 dye a day at 14 coin each, so let's coordinate days if either of us heads to Mire so we never bid against each other. Also, once you weave cloth, I'd gladly buy 1 or 2 cloth at 20 to 22 coin each to help fund your dye runs. What's your plan?
```

**Whispers check:** 10 messages (Saltworks 2, Weavers 2, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 73). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), Smiths (msg 2), Foresters (msg 2).
- Weavers: from Smiths (msg 3), Foresters (msg 3).
- Smiths: from Saltworks (msg 2), Foresters (msg 1).
- Foresters: from Saltworks (msg 1), Weavers (msg 2), Smiths (msg 1).

**Weavers contract-memory check (talk, day 2):** no live contracts (nothing proposed or accepted yet); its messages discuss possible deals only. Nothing to check.

### Act round: blocks sent

Same layout and format conventions as day 1.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-02-act-Saltworks.md` | `fd445a7646e407919aa6de0b6bf87a693f425c6dab3cf558f3e07ceb0f9ba0ad` |
| Weavers | `to-seat/day-02-act-Weavers.md` | `595289acc2973117645711e51415de460a3f3d4d867932b93b0cc65b3dfc8009` |
| Smiths | `to-seat/day-02-act-Smiths.md` | `bf3d9b96094dfa9a36d38c538ad4ba54f95f778d9885d4303123c1a80cdc1f8c` |
| Foresters | `to-seat/day-02-act-Foresters.md` | `3850e78317d3fc13baf0c54c35cd34e5d842afd695c4865a4dd60d62c567595d` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths' day-2 talk text was checked against AI Studio's synced record and matches exactly; today's Smiths act text is from the rendered page, pending the same check.

#### Saltworks: `from-seat/day-02-act-Saltworks.md`, `78938b42bf177326345dc9682b1db2b9bf263b06e7be03e97d84acf4a9494af5`

```
OFFERS: post 10 coin for 2 timber
TRAVEL: go to Varn
```

#### Weavers: `from-seat/day-02-act-Weavers.md`, `f9050d63126fa5cd2a61d05587d64c66944826c19f87caff22faf6d008cbe7a9`

```
OFFERS: post 4 coin for 4 wool @Ostra
TRAVEL: stay
```

#### Smiths: `from-seat/day-02-act-Smiths.md`, `09ead5cd8b58ebb0e459404b85305206dd63740439ecf6615cda7e1d14448a10`

```
OFFERS: post 17 coin for 1 leather ; post 16 coin for 2 charcoal ; post 5 coin for 2 ore @Kell
CONVERT: bellows x1 ; 1 ore + 1 charcoal -> 1 tools x2
TRAVEL: stay
```

#### Foresters: `from-seat/day-02-act-Foresters.md`, `854bdada2a6def26b78cb5a5d4664795c56a1b66790f2c14827cdb2ba831eee9`

```
OFFERS: post 14 coin for 4 timber @Hath ; post 1 leather for 17 coin ; post 2 charcoal for 16 coin ; post 2 timber for 10 coin
CONVERT: 1 timber → 1 charcoal x2
TRAVEL: stay
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals. The Smiths' `bellows x1` on the CONVERT line is read as installing the upgrade (rule gap G4).

**Weavers contract-memory check (act, day 2):** no live contracts. Consistent.

### Resolution, day 2

**Step 1. Sealed travel orders** (SHA-256 of the TRAVEL line as written, recorded before the reveal):

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Varn` | `04b9fa155844701211698d2fe32c50fcdedb94a61386dac324c8d640585c8723` |
| Weavers | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |

Revealed: Saltworks Ostra to Varn (road 0); the others stay.

**Step 2. The day's roll:** quicknet round 33966265, randomness `a2c50779b9bdb91915409ac6d21ead8f532ea5ae76884b47dfc12091763c536d`.

**Step 3. Road events, then movement** (board.md "Road events", "Reading dice"):

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | a2 c5 | 41669 | 10 | clear |
| 1 Ostra–Kell | safe | 07 79 | 1913 | 14 | clear |
| 2 Kell–Hath | hill | b9 bd | 47549 | 10 | clear |
| 3 Hath–Mire | safe | b9 19 | 47385 | 6 | clear (safe road: 6 is clear) |
| 4 Mire–Varn | marsh | 15 40 | 5440 | 1 | bandits (no caravan on it) |
| 5 Ostra–Mire | short cut | 9a c6 | 39622 | 3 | bandits (no caravan on it) |

Saltworks travels road 0: clear, arrives in Varn. No caravan met an event.

Caravans after moving: Saltworks Varn; Weavers Ostra; Smiths Kell; Foresters Hath.

**Step 4. Offers fill** (by price, then the day's order; board.md rule 1 for ladders). Day's order, SHA-256(`<randomness>:<guild>`), lowest first: Weavers `28d15f75…`, Saltworks `9733c822…`, Smiths `ba862f54…`, Foresters `dfe38923…`. No two offers competed for the same goods, so the order changed nothing.

Guild-to-guild offers (no location needed, board.md rule 1):

| Offers that cross | Result |
|---|---|
| Foresters: 1 leather for 17 coin × Smiths: 17 coin for 1 leather | filled: Foresters −1 leather +17 coin; Smiths +1 leather −17 coin |
| Foresters: 2 charcoal for 16 coin × Smiths: 16 coin for 2 charcoal | filled: Foresters −2 charcoal +16 coin; Smiths +2 charcoal −16 coin |
| Foresters: 2 timber for 10 coin × Saltworks: 10 coin for 2 timber | filled: Foresters −2 timber +10 coin; Saltworks +2 timber −10 coin |

Each seller held the goods at the start of the day (Foresters: 1 leather, 2 charcoal, 4 timber), and each buyer the coin, so no fill depended on another.

Ladder offers:

| Offer | Caravan in town? | Ladder total for the quantity | Result |
|---|---|---|---|
| Foresters: 14 coin for 4 timber @Hath | yes | 2+3+4+5 = 14 | filled: −14 coin, +4 timber |
| Smiths: 5 coin for 2 ore @Kell | yes | 2+3 = 5 | filled: −5 coin, +2 ore |
| Weavers: 4 coin for 4 wool @Ostra | yes | 1+2+3+4 = 10 | **not filled**: 4 coin is below the ladder's 10 (rule gap G3). Stays open as **O1**. |

**Step 5. Conversions and works deliveries.**
- Foresters in Hath: cut 2 timber (board.md "Cutting"; timing per G1), then 1 timber → 1 charcoal ×2. Timber 4 − 2 (sold) + 4 (bought) + 2 (cut) − 2 = 6; charcoal 2 − 2 (sold) + 2 = 2.
- Smiths in Kell (home): install the bellows, spending 1 leather (board.md "Guilds and converters", upgrade; G4). Then 1 ore + 1 charcoal → 1 tools ×2 (2 batches ordered; the bellows allow 3 from now on). Ore 3 + 2 − 2 = 3; charcoal 2 − 2 = 0; tools +2; leather 0.
- Saltworks in Varn: no conversion ordered (holds 4 brine, 2 timber).
- No works deliveries.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** O1 (Weavers, 4 coin for 4 wool @Ostra) stays open into day 3 (brief: "Offers stay open until filled or cancelled").

**Rule gap noted (provisional) G3: a ladder bid below the ladder's price.** The brief's own example ("post 10 coin for 4 brine @Ostra") prices the whole quantity at the stepped total. Decided provisionally: a ladder buy fills in full when the stepped total for its quantity (from that day's first unsold unit) is at most the coin offered, and the buyer pays the ladder's total; otherwise nothing fills and the offer stays open, re-checked each day after movement while the caravan is in that town. No partial fills. This keeps day 1 consistent. The alternative (the ledger's per-unit rate, which would fill 1 wool at 1 coin) was not used: it would have half-filled day 1's brine order (rate 2.5 a unit against units priced 3 and 4).

**Rule gap noted (provisional) G4: installing an upgrade.** The board says "spend 1 leather, then 3 batches a day" but not when, where or by which order. Decided: an upgrade is ordered on the CONVERT line, installed at step 5 with the guild's caravan at home, from goods held at that step; the extra batch is available from the same step onward.

### State after day 2

| Guild | Caravan | Purse | Goods | Standing | Upgrade |
|---|---|---|---|---|---|
| Saltworks | Varn | 40 | 4 brine, 2 timber | 0 | — |
| Weavers | Ostra | 60 | — | 0 | — |
| Smiths | Kell | 13 | 3 ore, 2 tools | 0 | bellows (day 2) |
| Foresters | Hath | 61 | 6 timber, 2 charcoal | 0 | — |

Coin check: 240 at the start; ladders took 10+9+14+14 (day 1) + 14+5 (day 2) = 66; tolls 0; 240 − 66 = 174 = 40 + 60 + 13 + 61. Checked.

Open offers: O1 Weavers, 4 coin for 4 wool @Ostra. Works: nothing delivered. Contracts and bonds: none. Missed reveals: 0 of 4.

### Referee word count, day 2

1415 words in this log's day-2 section, excluding the seats' verbatim replies.

---

## Day 3

**Drand round:** 33966270, randomness `ced76f9fceb709bf9cdaab5b3ded3f2fbe1a2b6288c6088e04546b007cc62073` (applied at resolution).

### Talk round: blocks sent

Each block: day-2 public results (road events, moves, filled and unfilled offers, per G2), the public state at the start of day 3, open offer O1, the seat's own position and mandates (the Smiths' block notes its bellows), an empty inbox, and the talk form. **Edict announced** (setup.md "Edicts", day 4, one day ahead per design.md "The closing bell"): "Gate closed at Kell", with the referee's rule from setup.md quoted in full.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-03-talk-Saltworks.md` | `18fb8a744b35815b5e3544cd529d456d2559a99fe46e4d831b9c0f6fc9772051` |
| Weavers | `to-seat/day-03-talk-Weavers.md` | `15c0152c7b48ee24f180f417271c600c227d3cbc91ecbb054e8f7bab50d96f44` |
| Smiths | `to-seat/day-03-talk-Smiths.md` | `0b25d330f574fe8cae70eaaf547b08e5474c9b110e86dc563c019ca2d0bff689` |
| Foresters | `to-seat/day-03-talk-Foresters.md` | `028d8c0cd8b3d94a1c345352b1e2b8c6ee679d465e22b71b6c737e142c739057` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry note from R&D: the Smiths' day-2 act text matches AI Studio's synced record exactly (checked); the day-3 Smiths talk text is from the rendered page, pending the same check.

#### Saltworks: `from-seat/day-03-talk-Saltworks.md`, `0a8ca9b46efd4746f9252fa42e3c971106b7d1af7782668b56e6df292e3ec269`

```
MESSAGE 1 TO Smiths: I convert 2 salt today in Varn. Kell is gated days 4 and 5, so I can't reach it soon. I'll post 2 salt for 8 coin as an open offer tomorrow, as agreed. Please cross it with 8 coin for 2 salt on day 4. Salt will reach you through the offer, no travel needed.
MESSAGE 2 TO Foresters: Thanks for the timber. I now make 2 salt a day, 4 in two days. I'm heading toward Hath with salt for the lodge, which also needs 3 charcoal. Will you sell me charcoal, or deliver it yourself? I'd rather work together than race you on the lodge.
```

#### Weavers: `from-seat/day-03-talk-Weavers.md`, `48eb9a971cd27ac39573233b7d3d77adaae06d819d5bf43cadff35f2baf7766a`

```
MESSAGE 1 TO Saltworks: I can buy 4 brine from you at cost +1 coin each, paid on delivery at Ostra. In return I’d like first call on the 3 salt needed for the Ostra crane.
MESSAGE 2 TO Foresters: I’ll soon have cloth; happy to sell 1‑2 cloth at 20‑22 coin each to fund your dye runs at Mire. Let’s coordinate timing.
```

#### Smiths: `from-seat/day-03-talk-Smiths.md`, `bf478043d18824f6d5ff14cf4c5512a066595e655bf6034e0e61d8ceff5200e3`

```
MESSAGE 1 TO Foresters: Clean fills yesterday! Crucial check for Day 3 Act: my purse is 13 coin, so I can't bid 16 coin first. Let's run the exact same net trade today as: 1 tools for 2 charcoal AND 1 tools for 16 coin (use these even if your Day 3 note said 32!). Days 4-5, let's cross 2 charcoal for 10 coin daily, and a 3rd tools for 16 coin!
MESSAGE 2 TO Saltworks: Yes to 2 salt for 10 coin! With Kell's gate closed Days 4-5 and contract bonds locking up cash you need for brine, let's skip the trip and bond: cross a direct Offer from Varn (2 salt for 10 coin) for instant cash! I will post 10 coin for 2 salt today (Day 3 Act), cross it today if you converted yesterday, or Day 4 after converting today!
MESSAGE 3 TO Weavers: With Kell's gate closing Days 4-5, I'll stay in Kell forging tools through Day 5, then can head up the safe road to Ostra on Day 6 (or trade tools anytime via guild Offers, which ignore closed gates!). Once you've got wool and Mire dye underway, let me know how you'd like to team up on Ostra crane!
```

#### Foresters: `from-seat/day-03-talk-Foresters.md`, `58bd9d726d6d35ea26192a101ed8718f037653fab7a1a47566c685b9bddd3644`

```
MESSAGE 1 TO Smiths: Heads up: you have 13 coin left, so posting 16 coin upfront today exceeds your purse! Let's keep our exact 16 coin net split today by crossing 4 coin for 2 charcoal and 2 tools for 20 coin (leaves you 9 coin for 3 ore). If your Day 3 note already named different prices up to 13 coin, I'll match yours; otherwise post 4 and 20! Yes to Days 4 and 5 charcoal!
MESSAGE 2 TO Saltworks: Glad our timber trade filled! Quick rules catch: offers fill BEFORE conversions each day. Since your pans run today, your salt finishes after offers fill, so let's cross on Day 4 instead: 1 salt for 10 coin AND 2 timber for 10 coin (zero net coin, stocking your next two boils!). If you post 1 salt today anyway, leave it open for Day 4. Want to team up on Hath lodge too?
MESSAGE 3 TO Weavers: Glad to partner! Friendly tip on O1: town markets step up 1 coin per unit sold each day, so 4 wool at Ostra costs 10 coin total (cancel O1 and post 10 coin for 4 wool @Ostra while in Ostra). Which day do you plan to visit Mire for dye? Once you weave cloth, I would still love to buy 1 cloth for 22 coin to help fund your second run!
```

**Whispers check:** 10 messages (Saltworks 2, Weavers 2, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 73). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 2), from Foresters (msg 2).
- Weavers: from Smiths (msg 3), from Foresters (msg 3).
- Smiths: from Saltworks (msg 1), from Foresters (msg 1).
- Foresters: from Saltworks (msg 2), from Weavers (msg 2), from Smiths (msg 1).

**Weavers contract-memory check (talk, day 3):** no live contracts. Nothing to check. (Observation, not a fault: its message 1 to Saltworks repeats its day-2 message 1 nearly word for word.)

### Act round: blocks sent

Same layout and format conventions as day 1; the day-4 edict is repeated in each block.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-03-act-Saltworks.md` | `0919b82b22cdcd0893fe19249991673f44cf113f238f5b6aa5fb45a0d80f8850` |
| Weavers | `to-seat/day-03-act-Weavers.md` | `ab7191e94ec0c6bf1627c37d0ec4bfe6c5d7eee9e95a83c31ff30a02c1a6e858` |
| Smiths | `to-seat/day-03-act-Smiths.md` | `105c903b43dd1b90124461a73fad8bf3d703dde8db5475ec5c57d5954795dadd` |
| Foresters | `to-seat/day-03-act-Foresters.md` | `ee2f44aa472d87b7ef8bbb1722544befd2c36e4c6d5dd80e9c573db6efc1aee2` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: both Smiths day-3 texts (talk and act) match AI Studio's synced record exactly. Weavers: on its talk turn the seat's chat line (not its file) carried a stray sentence saying its response had been "cut off"; the file was clean (Publisher). Harness noise, logged in metrics.md.

#### Saltworks: `from-seat/day-03-act-Saltworks.md`, `8624a1c807eb9d6d735ca2e6f68cec4798ea0282b10f073d045fd5d28c634ca1`

```
CONVERT: 2 brine -> 1 salt x2
TRAVEL: stay
```

#### Weavers: `from-seat/day-03-act-Weavers.md`, `9a9535022a2a49a3b126652988c68bca43ba66a1c15d091faa82de98a685542f`

```
OFFERS: post 10 coin for 4 wool @Ostra ; cancel O1
TRAVEL: stay
```

#### Smiths: `from-seat/day-03-act-Smiths.md`, `2e851fc3172b852ab9fdce2e24fc0ec9923ac0e0541c40885dca35005acee5a0`

```
OFFERS: post 1 tools for 2 charcoal ; post 1 tools for 16 coin
CONVERT: 1 ore + 1 charcoal -> 1 tools x2
TRAVEL: stay
```

#### Foresters: `from-seat/day-03-act-Foresters.md`, `cb55b6ab3f41522726634d443c5283ff2840cddd65d9a63d88f8583d249f3026`

```
OFFERS: post 5 coin for 2 timber @Hath ; post 2 charcoal for 1 tools ; post 16 coin for 1 tools
CONVERT: 1 timber → 1 charcoal x2
TRAVEL: stay
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 3):** no live contracts. Its `cancel O1` names its own open offer correctly. Consistent.

### Resolution, day 3

**Step 1. Sealed travel orders** (SHA-256 of the TRAVEL line as written, recorded before the reveal):

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Weavers | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |

Revealed: every caravan stays.

**Step 2. The day's roll:** quicknet round 33966270, randomness `ced76f9fceb709bf9cdaab5b3ded3f2fbe1a2b6288c6088e04546b007cc62073`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | ce d7 | 52951 | 12 | clear |
| 1 Ostra–Kell | safe | 6f 9f | 28575 | 16 | clear |
| 2 Kell–Hath | hill | ce b7 | 52919 | 20 | fair (no caravan on it) |
| 3 Hath–Mire | safe | 09 bf | 2495 | 16 | clear |
| 4 Mire–Varn | marsh | 9c da | 40154 | 15 | clear |
| 5 Ostra–Mire | short cut | ab 5b | 43867 | 8 | clear |

No caravan travels, so none meets an event. The Kell–Hath fair applies only to a caravan that reaches a town by that road (board.md "Fair"), so it has no effect. Caravans: Saltworks Varn; Weavers Ostra; Smiths Kell; Foresters Hath.

**Step 4. Offers fill.** Day's order, SHA-256(`<randomness>:<guild>`), lowest first: Saltworks `bce4de02…`, Weavers `bd0db048…`, Foresters `bd9cf7ef…`, Smiths `cb9fe2bf…`. No two offers competed for the same goods, so it changed nothing.

Cancellations first: Weavers cancels O1 (its own open offer; brief: "Offers stay open until filled or cancelled"). O1 is closed.

Guild-to-guild offers:

| Offers that cross | Result |
|---|---|
| Foresters: 2 charcoal for 1 tools × Smiths: 1 tools for 2 charcoal | filled: Foresters −2 charcoal +1 tools; Smiths −1 tools +2 charcoal |
| Foresters: 16 coin for 1 tools × Smiths: 1 tools for 16 coin | filled: Foresters −16 coin +1 tools; Smiths +16 coin −1 tools |

Both sellers held the goods at the start of the day (Smiths 2 tools, Foresters 2 charcoal); the Foresters held the coin.

Ladder offers (G3):

| Offer | Caravan in town? | Ladder total | Result |
|---|---|---|---|
| Weavers: 10 coin for 4 wool @Ostra | yes | 1+2+3+4 = 10 | filled: −10 coin, +4 wool |
| Foresters: 5 coin for 2 timber @Hath | yes | 2+3 = 5 | filled: −5 coin, +2 timber |

**Step 5. Conversions and works deliveries.**
- Saltworks in Varn (home): 2 brine → 1 salt ×2; the pans burn 1 timber (board.md "Guilds and converters"). Brine 4 − 4 = 0; salt +2; timber 2 − 1 = 1.
- Smiths in Kell (home): 1 ore + 1 charcoal → 1 tools ×2 (bellows allow 3; 2 ordered). Ore 3 − 2 = 1; charcoal 2 (received today) − 2 = 0; tools 2 − 2 (sold) + 2 = 2.
- Foresters in Hath: cut 2 timber (G1), then 1 timber → 1 charcoal ×2. Timber 6 + 2 (bought) + 2 (cut) − 2 = 8; charcoal 2 − 2 (sold) + 2 = 2; tools 2 (better axes not ordered, so not installed; G4).
- No works deliveries.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** No offers remain open.

### State after day 3

| Guild | Caravan | Purse | Goods | Standing | Upgrade |
|---|---|---|---|---|---|
| Saltworks | Varn | 40 | 2 salt, 1 timber | 0 | — |
| Weavers | Ostra | 50 | 4 wool | 0 | — |
| Smiths | Kell | 29 | 1 ore, 2 tools | 0 | bellows (day 2) |
| Foresters | Hath | 40 | 8 timber, 2 charcoal, 2 tools | 0 | — |

Coin check: 174 after day 2; ladders took 10 + 5 = 15 today; 174 − 15 = 159 = 40 + 50 + 29 + 40. Checked.

Open offers: none. Works: nothing delivered. Contracts and bonds: none. Missed reveals: 0 of 4. Edict from tomorrow: Kell gate closed, days 4 and 5.

### Referee word count, day 3

1181 words in this log's day-3 section, excluding the seats' verbatim replies.

---

## Day 4

**Drand round:** 33966275, randomness `7f0b05a7e44cd4e30d023a10de3a492e685a1420ed995bf79e03bd330f47f344` (applied at resolution).

**Edict in force, days 4 and 5** (setup.md "Edicts"): Gate closed at Kell. Referee's rule from setup.md: no caravan enters or leaves Kell on days 4 and 5; a travel order across the gate is refused at the reveal; the caravan stays and pays no toll.

### Talk round: blocks sent

Each block: day-3 public results (G2), the public state at the start of day 4, the edict in force, the seat's own position and mandates, an empty inbox, and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-04-talk-Saltworks.md` | `32b88e9baa04cbc912c1d356ecd20040c6badbc58b78535c21e75be7111a6540` |
| Weavers | `to-seat/day-04-talk-Weavers.md` | `2a4f6a4d57c3e859a2c67889a59d5f7bae42754a3f8761b5b278631f3181c410` |
| Smiths | `to-seat/day-04-talk-Smiths.md` | `377c43b6b555a0ae6e11252c00ed4507405689ebc799f7a2ab86a0728da24fe7` |
| Foresters | `to-seat/day-04-talk-Foresters.md` | `3c0443f7fb140c273e44df5f72ff18c3747e123a4fae61eafccafc857ee688bc` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry note from R&D: the Smiths text matches AI Studio's synced record. Harness notes from R&D (game held about 20:56–21:02 ET; Weavers' turns now sent from R&D's pane; Weavers' empty-output errors) are in metrics.md.

#### Saltworks: `from-seat/day-04-talk-Saltworks.md`, `17abe7e618763dca442c5cafbefa073e888a9c2ddfb940c26bf809e2c0b5ebb0`

```
MESSAGE 1 TO Foresters: I hold 2 salt and head for Hath via Mire. I'll need charcoal for the lodge: will you sell me charcoal at a fair price, say 3 coin each, or hold your charcoal for the lodge delivery together with my salt? Salt for you: 1 for 10 coin, as you proposed, if you still want it.
MESSAGE 2 TO Smiths: I held my salt for now, since Kell is gated until day 6. I can sell 1 salt for 8 coin today if you want it for the bridge once the gate opens. Otherwise I'll have more salt in a few days.
```

#### Weavers: `from-seat/day-04-talk-Weavers.md`, `9492825dd2118747dbd148217ff762527e6c354dc31275c978fff8661ce69677`

```
MESSAGE 1 TO Saltworks: I have 4 wool ready. Would you like to purchase it at market price (10 coin) or trade it for brine or salt needed for my cloth production? Let me know your terms.
MESSAGE 2 TO Foresters: I’ll soon convert wool into cloth. I can sell 1‑2 cloth to you for 20‑22 coin each to help fund your dye runs. Tell me when you’d like to buy.
MESSAGE 3 TO Smiths: If you have any tools you can spare, I could trade cloth later for them, supporting your production.
```

#### Smiths: `from-seat/day-04-talk-Smiths.md`, `5d0d128d9aaf724f89ca8d4146a236b8e1626406bb8d68626a1390a67de38894`

```
MESSAGE 1 TO Foresters: Perfect Day 3 trades! Today (Day 4 Act), let's cross ONLY 10 coin for 2 charcoal (no tools trade today). Will you stay in Hath through Day 5 so we can cross 2 charcoal for 10 coin on Day 5 AND again on Day 6 (you can travel to Mire on Day 6 while crossing offers!)? If yes, on Day 5 we'll also cross your 3rd tools for 16 coin!
MESSAGE 2 TO Saltworks: Saw your note that you converted yesterday, so I waited until today to post! Since your Day 3 note said 8 coin and mine said 10, let's split the difference in Day 4 Act: I'll post 9 coin for 2 salt and you post 2 salt for 9 coin (even if your Day 4 note said 8 or 10, please use 9 today so we definitely match!). Need tools later?
MESSAGE 3 TO Weavers: Nice wool pickup yesterday! Safe travels if you're heading to Mire for dye while Kell's gate is closed Days 4-5. Once Kell reopens on Day 6, I'll have tools production ready to support Ostra crane or Varn sea wall. Let me know whether you'd like to buy tools via Offers or coordinate joint deliveries at Ostra crane around Days 6-8!
```

#### Foresters: `from-seat/day-04-talk-Foresters.md`, `42050459b393c5e8a751a526796f605c3475575ac5da972521de72d1aeb81b59`

```
MESSAGE 1 TO Smiths: Deal! Today in Day 4 Act, let's cross both exact offers: I'll post 2 charcoal for 10 coin AND 16 coin for 1 tools; you post 10 coin for 2 charcoal AND 1 tools for 16 coin. That keeps your forge running during Kell's gate closure! Tomorrow (Day 5) let's cross another 2 charcoal for 10 coin. Tell me if you'll have an extra tools to sell later too!
MESSAGE 2 TO Weavers: Awesome! I won't buy dye in Mire, leaving Mire's stock all yours. If you grab 2 dye in Mire today (28 coin) and weave 2 cloth in Ostra tomorrow (Day 5), let's cross on Day 6! Would you sell me 2 cloth for 42 coin on Day 6 (giving you 64 coin for full wool, dye, and crane runs), or 1 cloth for 22 coin? Let me know so I hold coin!
MESSAGE 3 TO Saltworks: Definitely let's team up on Hath lodge instead of racing! My current charcoal is promised to Smiths while Kell's gate is shut, so I'll deliver the 3 charcoal myself once that wraps up. Feel free to deliver all 4 salt whenever you reach Hath (you even get 8 points vs my 6!). Also, want 2 timber for 10 coin today for future boils, or 2 timber for 1 salt?
```

**Whispers check:** 11 messages (Saltworks 2, Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 72). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 2), from Foresters (msg 3).
- Weavers: from Smiths (msg 3), from Foresters (msg 2).
- Smiths: from Saltworks (msg 2), from Weavers (msg 3), from Foresters (msg 1).
- Foresters: from Saltworks (msg 1), from Weavers (msg 2), from Smiths (msg 1).

**Weavers contract-memory check (talk, day 4):** no live contracts. Nothing to check.

### Act round: blocks sent

Same layout and format conventions as day 1; the edict in force is repeated in each block.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-04-act-Saltworks.md` | `2a07f53df28284731f46b147a3e24aa9439b829750c4cc439ea53f43274a3310` |
| Weavers | `to-seat/day-04-act-Weavers.md` | `0620fd94d6d23e1fbb94749de5c4fd3b43684d2e7fdc82b76bc7cac53fbc7ca7` |
| Smiths | `to-seat/day-04-act-Smiths.md` | `5af2722143f7b38115f18897400b8c6a611650e981f5340bae086a0672b1d3cb` |
| Foresters | `to-seat/day-04-act-Foresters.md` | `262a31d4be4995d374c57587edc6731422bbe2ec86b415909652a0e65be83d1a` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. (The Saltworks reply is byte-identical to its day-1 reply, hence the same hash.)

#### Saltworks: `from-seat/day-04-act-Saltworks.md`, `8d3ffefb67bad1c858dad788bce8079995d0cc7f03c10c354ea986eb54593345`

```
OFFERS: post 10 coin for 4 brine @Ostra
TRAVEL: go to Ostra
```

#### Weavers: `from-seat/day-04-act-Weavers.md`, `6ee7a6c533d1e26906ac2f8c09047d714a7315b3aa20f8d3499b98638665bd85`

```
OFFERS: post 28 coin for 2 dye @Mire
TRAVEL: go to Mire
```

#### Smiths: `from-seat/day-04-act-Smiths.md`, `aa49a93252bfaa3975637907f74350958a767d624feaafc9cfd71e406856aa48`

```
OFFERS: post 10 coin for 2 charcoal ; post 9 coin for 2 salt
TRAVEL: stay
```

#### Foresters: `from-seat/day-04-act-Foresters.md`, `f0e11040d3d5ba74ae6ef98bbf40baf17def388c2d6fa526c99a2dabbf143cf4`

```
OFFERS: post 2 charcoal for 10 coin
CONVERT: 1 timber → 1 charcoal x2
TRAVEL: stay
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 4):** no live contracts. Consistent.

### Resolution, day 4

**Edict in force:** Gate closed at Kell (days 4 and 5). No travel order enters or leaves Kell today (the Smiths stay), so the edict refuses nothing.

**Step 1. Sealed travel orders** (SHA-256 of the TRAVEL line as written, recorded before the reveal):

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Ostra` | `e81dca8522bda28e703276369bdaab4c461138b5c641a68feca607d271d2cc03` |
| Weavers | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |

Revealed: Saltworks Varn → Ostra (road 0); Weavers Ostra → Mire (road 5, the short cut); Smiths and Foresters stay.

**Step 2. The day's roll:** quicknet round 33966275, randomness `7f0b05a7e44cd4e30d023a10de3a492e685a1420ed995bf79e03bd330f47f344`.

**Step 3. Road events, then movement** (board.md "Road events"):

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 7f 0b | 32523 | 4 | clear (safe road: 3–4 clear) |
| 1 Ostra–Kell | safe | 05 a7 | 1447 | 8 | clear |
| 2 Kell–Hath | hill | e4 4c | 58444 | 5 | flood (no caravan on it) |
| 3 Hath–Mire | safe | d4 e3 | 54499 | 20 | fair (no caravan on it) |
| 4 Mire–Varn | marsh | 0d 02 | 3330 | 11 | clear |
| 5 Ostra–Mire | short cut | 3a 10 | 14864 | 5 | **flood** |

- Saltworks on road 0: clear, arrives in Ostra.
- Weavers on road 5: **flood**. "The caravan doesn't move" (board.md "Flood"): the Weavers stay in Ostra. No toll, no loss. The flood is a *holds* event (design.md bond rule), recorded against the Weavers' caravan for day 4; the Weavers have no contracts, so it affects none.

Caravans after moving: Saltworks Ostra; Weavers Ostra; Smiths Kell; Foresters Hath.

**Step 4. Offers fill.** Day's order, lowest first: Foresters `3ac94c33…`, Smiths `3bb9bede…`, Saltworks `7aadc1f0…`, Weavers `8428df43…`. No two offers competed for the same goods.

Guild-to-guild:

| Offers that cross | Result |
|---|---|
| Foresters: 2 charcoal for 10 coin × Smiths: 10 coin for 2 charcoal | filled: Foresters −2 charcoal +10 coin; Smiths +2 charcoal −10 coin |
| Smiths: 9 coin for 2 salt | no crossing offer; stays open as **O3** |

Ladder offers (board.md rule 1; G3):

| Offer | Caravan in town? | Ladder total | Result |
|---|---|---|---|
| Saltworks: 10 coin for 4 brine @Ostra | yes (arrived) | 1+2+3+4 = 10 | filled: −10 coin, +4 brine |
| Weavers: 28 coin for 2 dye @Mire | **no** (held in Ostra by the flood) | sit-out dye 14 × 2 = 28 | not filled: the caravan is not in Mire (board.md rule 1). Stays open as **O2**, re-checked each day the Weavers' caravan is in Mire (G3). |

**Step 5. Conversions and works deliveries.**
- Foresters in Hath: cut 2 timber (G1), then 1 timber → 1 charcoal ×2. Timber 8 + 2 − 2 = 8; charcoal 2 − 2 (sold) + 2 = 2.
- Smiths: no conversion ordered (holds 1 ore, 2 charcoal, 2 tools).
- Saltworks is away from Varn; no conversion ordered.
- No works deliveries.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** Open into day 5: O2 (Weavers, 28 coin for 2 dye @Mire), O3 (Smiths, 9 coin for 2 salt).

### State after day 4

| Guild | Caravan | Purse | Goods | Standing | Upgrade |
|---|---|---|---|---|---|
| Saltworks | Ostra | 30 | 4 brine, 2 salt, 1 timber | 0 | — |
| Weavers | Ostra | 50 | 4 wool | 0 | — |
| Smiths | Kell | 19 | 1 ore, 2 charcoal, 2 tools | 0 | bellows (day 2) |
| Foresters | Hath | 50 | 8 timber, 2 charcoal, 2 tools | 0 | — |

Coin check: 159 after day 3; ladders took 10 today; 159 − 10 = 149 = 30 + 50 + 19 + 50. Checked.

Open offers: O2, O3. Works: nothing delivered. Contracts and bonds: none. Missed reveals: 0 of 4 (0 of 16 so far). Edict: Kell gate closed through day 5.

### Referee word count, day 4

1142 words in this log's day-4 section, excluding the seats' verbatim replies.

---

## Day 5

**Drand round:** 33966280, randomness `2b2aaf6c54aeabb345b0d062ff2c6253252ca8aea090c62288323d708c28fa02` (applied at resolution).

**Edict in force, day 5 (its last day):** Gate closed at Kell (setup.md "Edicts").

### Talk round: blocks sent

Each block: day-4 public results (G2), the public state at the start of day 5, open offers O2 and O3, the edict in force, the seat's own position and mandates, an empty inbox, and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-05-talk-Saltworks.md` | `b3ffe98d5749fef395a142c1a2f8155b0de822b371361908a2a164aac2903e7a` |
| Weavers | `to-seat/day-05-talk-Weavers.md` | `c6d0935bc56b6fbd653de86156b1c69bd98b11855f26db81b4a5835217e68bc1` |
| Smiths | `to-seat/day-05-talk-Smiths.md` | `dc8b42c20d24a714c30707a9eaad3b30e658ce8245f0ea49a2bfacc376e25079` |
| Foresters | `to-seat/day-05-talk-Foresters.md` | `41ca4aed39f8719ab54e7e72e9c2f396764078ee5eb3a2c3550b241b4dd20359` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths' day-4 act and day-5 talk texts both match AI Studio's synced record. The Foresters' Gemini-app seat took about 20 minutes this round (slow service); not a missed reply (metrics.md).

#### Saltworks: `from-seat/day-05-talk-Saltworks.md`, `3ae4794f93a9135179de8d64f599b2d143be976f39c984fc6a51e9f163fb132d`

```
MESSAGE 1 TO Foresters: Plan: I deliver 4 salt to the Hath lodge around day 8 or 9, and you bring the charcoal. Please keep your charcoal for the lodge and deliver it only after my salt, or on the same day. I'll take your 2 timber for 1 salt only if I have spare salt after the lodge. Agreed?
MESSAGE 2 TO Smiths: Your 9 for 2 salt offer is open, but I'm saving salt for works delivery now. I may have 2 spare salt after day 7, so ask again then. Kell bridge needs 2 salt and I could deliver them to Kell directly if the timing fits.
```

#### Weavers: `from-seat/day-05-talk-Weavers.md`, `9a903267a35f1b77537587b35ecc1b22778014d69628eb8fd5662404bebb9e95`

```
MESSAGE 1 TO Saltworks: I have 4 wool ready; would you like to buy it at market price (10 coin) or trade it for brine or salt needed for my cloth production? Let me know your terms.
MESSAGE 2 TO Foresters: I’ll soon convert wool into cloth and can sell you 1‑2 cloth for 20‑22 coin each to help fund your dye runs. Tell me when you’d like to buy.
MESSAGE 3 TO Smiths: When Kell reopens, I’d like to trade cloth for tools or coordinate joint deliveries for the Ostra crane. Let me know your preferences.
```

#### Smiths: `from-seat/day-05-talk-Smiths.md`, `b89bafe7d2add1320cac8d1ac28407f60097d06e0f70c4cb730a1994577d19e4`

```
MESSAGE 1 TO Foresters: Awesome! Today in Day 5 Act, let's cross BOTH exact offers: I'll post 10 coin for 2 charcoal AND 1 tools for 16 coin; you post 2 charcoal for 10 coin AND 16 coin for 1 tools. Then tomorrow (Day 6 Act), let's cross another 2 charcoal for 10 coin as agreed! After Day 6, if you want to keep trading charcoal for coin or tools, I'm game!
MESSAGE 2 TO Saltworks: Fair point on scarce salt! Pick whichever deal you like best for 2 salt: (A) TODAY in Day 5 Act, swap 2 salt for 1 tools (16-coin value for Varn sea wall or Ostra crane, I'm cancelling O3 and posting 1 tools for 2 salt today, so post 2 salt for 1 tools to cross!), OR (B) TOMORROW on Day 6, sell me 2 salt for 14 coin cash! Which do you prefer?
MESSAGE 3 TO Weavers: Unlucky flood on the short cut yesterday! Yes, I would love to trade tools for cloth (or coin) once you grab your Mire dye and convert back in Ostra around Day 6-7, right as Kell's gate reopens. We can also coordinate deliveries to finish Ostra crane and Varn sea wall together. Let me know your timeline once you reach Mire!
```

#### Foresters: `from-seat/day-05-talk-Foresters.md`, `b2b722753a7c061d512143b7af9a86cebb38089415b3ccf5c9ff58beefe3b91f`

```
MESSAGE 1 TO Smiths: Yes to all of it! I'm staying in Hath today (Day 5). In Day 5 Act, let's cross both exact offers: 2 charcoal for 10 coin AND 1 tools for 16 coin. Tomorrow (Day 6) we'll cross 2 charcoal for 10 coin as well! Question for Day 6: would you also want to cross a 4th tools for 16 coin tomorrow (keeping our 50/50 split on that batch too), or just the charcoal?
MESSAGE 2 TO Saltworks: Smart move grabbing 4 more brine at Ostra so you'll have all 4 salt! Let's stick to our Hath lodge plan: I'll deliver the 3 charcoal myself to pair with your 4 salt. My coin is tied up in tools right now so I'll pass on buying 1 salt, but since your next boil burns your second timber, want to cross 2 timber for 10 coin on Day 6 for more salt runs?
MESSAGE 3 TO Weavers: Sorry about yesterday's flood! I'm staying in Hath today (Day 5), so Mire's dye is all yours, and your open O2 offer auto-fills once you reach Mire. After you return to Ostra on Day 6 and weave cloth, conversions finish after offers, so Day 7 is our first trade window. Let's plan to cross 1 cloth for 20 coin on Day 7 to fund your next dye run!
```

**Whispers check:** 11 messages (Saltworks 2, Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 73). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 2), from Foresters (msg 2).
- Weavers: from Smiths (msg 3), from Foresters (msg 3).
- Smiths: from Saltworks (msg 2), from Weavers (msg 3), from Foresters (msg 1).
- Foresters: from Saltworks (msg 1), from Weavers (msg 2), from Smiths (msg 1).

**Weavers contract-memory check (talk, day 5):** no live contracts. Nothing to check. (Observation, not a fault: its three messages repeat its day-4 messages almost word for word.)

### Act round: blocks sent

Same layout and format conventions as day 1; the edict in force is repeated in each block.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-05-act-Saltworks.md` | `b79fa1232e6c7953a2f23d8dc4354fde94c785897d1273cd68d3fa16775ad33f` |
| Weavers | `to-seat/day-05-act-Weavers.md` | `a97f2196352c18c5c8209ea1a95324597897d6335e5d9db7c8f468f14ddaf530` |
| Smiths | `to-seat/day-05-act-Smiths.md` | `7f919c5700fba72ac12097f68894cde4a841735f4770b5c73dec19f564aa0748` |
| Foresters | `to-seat/day-05-act-Foresters.md` | `47c83ebbb288dfd71cf3bbfab8d1cf55d39016973677ffeb2cdb77bedfb41d43` |

Referee correction before sending: the first draft of these four files carried a stale caravan line (Saltworks in Varn); it was regenerated from the live state before any hash was given to R&D. The hashes above are of the files as sent.

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list.

#### Saltworks: `from-seat/day-05-act-Saltworks.md`, `eff13e15d6eef2fc493ce7b4adf7056899506f1cfe5209f6124c9a1c0a07da4a`

```
TRAVEL: go to Varn
```

#### Weavers: `from-seat/day-05-act-Weavers.md`, `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937`

```
TRAVEL: go to Mire
```

#### Smiths: `from-seat/day-05-act-Smiths.md`, `ee1d03c2cb101a9f6c45baa71676ecdb966547d674606435c0b6f07983c6ebf3`

```
OFFERS: cancel O3 ; post 10 coin for 2 charcoal ; post 1 tools for 16 coin ; post 1 tools for 2 salt
CONVERT: 1 ore + 1 charcoal -> 1 tools x1
TRAVEL: stay
```

#### Foresters: `from-seat/day-05-act-Foresters.md`, `b81eb3a1410be7358cc9f6fc01562e8d26e321f070247860a0be9d5b42fc3f92`

```
OFFERS: post 2 charcoal for 10 coin ; post 16 coin for 1 tools
CONVERT: 1 timber → 1 charcoal x2
TRAVEL: stay
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 5):** no live contracts. Its reply is a travel order only; its open offer O2 stays in force without being restated. Consistent.

### Resolution, day 5

**Edict in force (last day):** Gate closed at Kell. No travel order enters or leaves Kell (the Smiths stay), so it refuses nothing. It lapses at the close of day 5.

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Varn` | `04b9fa155844701211698d2fe32c50fcdedb94a61386dac324c8d640585c8723` |
| Weavers | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |

Revealed: Saltworks Ostra → Varn (road 0); Weavers Ostra → Mire (road 5, short cut); Smiths and Foresters stay.

**Step 2. The day's roll:** quicknet round 33966280, randomness `2b2aaf6c54aeabb345b0d062ff2c6253252ca8aea090c62288323d708c28fa02`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 2b 2a | 11050 | 11 | clear |
| 1 Ostra–Kell | safe | af 6c | 44908 | 9 | clear |
| 2 Kell–Hath | hill | 54 ae | 21678 | 19 | clear |
| 3 Hath–Mire | safe | ab b3 | 43955 | 16 | clear |
| 4 Mire–Varn | marsh | 45 b0 | 17840 | 1 | bandits (no caravan on it) |
| 5 Ostra–Mire | short cut | d0 62 | 53346 | 7 | clear |

Saltworks on road 0: clear, arrives in Varn. Weavers on road 5: clear, arrives in Mire.

Caravans after moving: Saltworks Varn; Weavers Mire; Smiths Kell; Foresters Hath.

**Step 4. Offers fill.** Day's order, lowest first: Foresters `72d5f26e…`, Saltworks `7c994dd5…`, Smiths `bb519880…`, Weavers `c185ea81…`. No two offers competed for the same goods.

Cancellations first: Smiths cancels O3 (its own).

Guild-to-guild:

| Offers that cross | Result |
|---|---|
| Foresters: 2 charcoal for 10 coin × Smiths: 10 coin for 2 charcoal | filled: Foresters −2 charcoal +10 coin; Smiths +2 charcoal −10 coin |
| Foresters: 16 coin for 1 tools × Smiths: 1 tools for 16 coin | filled: Foresters +1 tools −16 coin; Smiths −1 tools +16 coin |
| Smiths: 1 tools for 2 salt | no crossing offer; stays open as **O4** |

Ladder offers (board.md rule 1; G3):

| Offer | Caravan in town? | Ladder total | Result |
|---|---|---|---|
| O2 Weavers: 28 coin for 2 dye @Mire (open since day 4) | yes (arrived) | sit-out dye 14 + 14 = 28 | filled: −28 coin, +2 dye. O2 closed. |

**Step 5. Conversions and works deliveries.**
- Smiths in Kell: 1 ore + 1 charcoal → 1 tools ×1. Ore 1 − 1 = 0; charcoal 2 + 2 − 1 = 3; tools 2 − 1 (sold) + 1 = 2.
- Foresters in Hath: cut 2 timber (G1), then 1 timber → 1 charcoal ×2. Timber 8 + 2 − 2 = 8; charcoal 2 − 2 (sold) + 2 = 2; tools 2 + 1 = 3.
- Saltworks in Varn: no conversion ordered. Weavers away from Ostra.
- No works deliveries.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** Open into day 6: O4 (Smiths, 1 tools for 2 salt). The Kell gate lapses.

### State after day 5

| Guild | Caravan | Purse | Goods | Standing | Upgrade |
|---|---|---|---|---|---|
| Saltworks | Varn | 30 | 4 brine, 2 salt, 1 timber | 0 | — |
| Weavers | Mire | 22 | 4 wool, 2 dye | 0 | — |
| Smiths | Kell | 25 | 3 charcoal, 2 tools | 0 | bellows (day 2) |
| Foresters | Hath | 44 | 8 timber, 2 charcoal, 3 tools | 0 | — |

Coin check: 149 after day 4; Mire's ladder took 28; 149 − 28 = 121 = 30 + 22 + 25 + 44. Checked.

Open offers: O4. Works: nothing delivered. Contracts and bonds: none. Missed reveals: 0 of 4 (0 of 20 so far).

### Referee word count, day 5

1132 words in this log's day-5 section, excluding the seats' verbatim replies.

---

## Day 6

**Drand round:** 33966285, randomness `735c6ff5e4f5bdb6907849fd75ada87a0244a038a80c53827a910a44dc03950f` (applied at resolution).

### Talk round: blocks sent

Each block: day-5 public results (G2), the public state at the start of day 6, open offer O4, no edict (Kell gate ended; the day-8 levy is announced on day 7), the seat's own position and mandates, an empty inbox, and the talk form. Caravan lines are generated from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-06-talk-Saltworks.md` | `63589d540d323775f1003948b5999ace1ae28f8f748b44c9381b9e47fe242b32` |
| Weavers | `to-seat/day-06-talk-Weavers.md` | `ee3d0a29b25db7f5ef8f247a5e7799fc6466fcbef87905565b7ddd5d3fb217fe` |
| Smiths | `to-seat/day-06-talk-Smiths.md` | `9a08ab66b3cb7a6620118ef6c12543cb36e1358c5ac25d8511890102ef0c6a94` |
| Foresters | `to-seat/day-06-talk-Foresters.md` | `f4df7bd4c521f9a3924102635c37ad8c157996537236dda0c90513ba9e18f66a` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry note from R&D: the Smiths' day-5 act and day-6 talk texts both match AI Studio's synced record.

#### Saltworks: `from-seat/day-06-talk-Saltworks.md`, `63e228f1a51650cb7f5997fab48575404ed54086f8ea3e7a76f715849e542c39`

```
MESSAGE 1 TO Foresters: I convert 2 salt today, then travel to Hath, arriving day 7 or 8 with 4 salt for the lodge. Please deliver your charcoal after my salt, so the lodge totals read right. Please don't deliver salt to the lodge yourself; I have all four. Keep the timber, I don't need more.
```

#### Weavers: `from-seat/day-06-talk-Weavers.md`, `04fb06443103e2759f4228c6316d2362253113b4563629a641e7c63331ebaa1a`

```
MESSAGE 1 TO Saltworks: I have 2 dye now; would you like to purchase them at market price (28 coin total) or trade for brine or salt needed for your salt production? Let me know your terms.
MESSAGE 2 TO Smiths: When Kell reopens I can weave cloth from my wool and dye. I’m willing to trade 1–2 cloth for tools or coin (20–22 coin each). Let me know your needs.
MESSAGE 3 TO Foresters: I’ll have cloth soon; can we arrange a trade of 1 cloth for 22 coin on Day 7 to fund your dye runs? Also happy to buy any timber you have at market price.
```

#### Smiths: `from-seat/day-06-talk-Smiths.md`, `7ea920a2f49b1a1f54854fbbde7a84ba5fe0b6801a18401032ee2a0845d88060`

```
MESSAGE 1 TO Foresters: Yes to BOTH today! In Day 6 Act, let's cross 10 coin for 2 charcoal AND 1 tools for 16 coin (I'll post 10 coin for 2 charcoal and 1 tools for 16 coin; you post 2 charcoal for 10 coin and 16 coin for 1 tools). What is your travel plan for Days 6-8? Whenever you make more charcoal in Hath, I'd love to keep buying charcoal or running more 50/50 tools splits!
MESSAGE 2 TO Saltworks: Two great options for 2 salt! (1) TODAY in Day 6 Act: if you're getting Ostra brine today and converting again Day 7, post 2 salt for 13 coin today (I'll cancel O4 and post 13 coin for 2 salt today!) for instant cash while still rebuilding your salt Day 7! (2) Or let's contract: deliver 2 salt at Varn by day 8 for 14 coin (+1 standing for you!).
MESSAGE 3 TO Weavers: Congrats on getting the 2 dye! Assuming you head back to Ostra today to convert 2 cloth, I'd love to work together starting Day 7. For Ostra crane (3 tools, 2 cloth, 3 salt), would you rather swap cloth for tools via Offers on Day 7, or each deliver our own goods to Ostra crane (and do you need completion there or at Varn sea wall)? Let me know!
```

#### Foresters: `from-seat/day-06-talk-Foresters.md`, `c093212dad01530db3b99f209c52f587aa881e8927f5b92db6206780e4fb3dfc`

```
MESSAGE 1 TO Smiths: Confirmed! Today in Day 6 Act let's cross 2 charcoal for 10 coin: I'll post 2 charcoal for 10 coin and you post 10 coin for 2 charcoal. Also, once your forge runs with Kell's gate open again, will you have 1 extra tools to sell for around 12 to 14 coin on Day 7 or Day 8? Let me know so I keep coin ready!
MESSAGE 2 TO Weavers: Helpful tip for today (Day 6): caravans move before conversions, so if you go to Ostra AND submit your cloth conversions today, you weave 2 cloth today! Tomorrow (Day 7) let's cross 2 cloth for 42 coin (21 each), boosting you to 64 coin for 4 wool plus 4 dye! If bandits leave 1 cloth, we'll cross 1 cloth for 21 coin. Deal?
MESSAGE 3 TO Saltworks: Agreed! Keep all 4 salt for Hath lodge (8 points for you, better than splitting 2 salt away). Today I'll deliver 2 charcoal to Hath lodge to prove my commitment, while holding back the 3rd charcoal until on or after you deliver your 4 salt on Day 8 or 9, just as you asked! Let me know your exact Hath arrival day.
```

**Whispers check:** 10 messages (Saltworks 1, Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 74). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 2), from Foresters (msg 3).
- Weavers: from Smiths (msg 3), from Foresters (msg 2).
- Smiths: from Weavers (msg 2), from Foresters (msg 1).
- Foresters: from Saltworks (msg 1), from Weavers (msg 3), from Smiths (msg 1).

**Weavers contract-memory check (talk, day 6):** no live contracts. Nothing to check. (Observation, not a fault: it offers its 2 dye to Saltworks and says it will weave "when Kell reopens"; neither affects a contract.)

### Act round: blocks sent

Same layout and format conventions as day 1; caravan lines from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-06-act-Saltworks.md` | `ef2ebe638a0dcfc1b18e524ccd7b65044fd3d399e08bdee3b4d553df4ed6bf54` |
| Weavers | `to-seat/day-06-act-Weavers.md` | `a1bb83fd48b783bef7621fb7399a6d66e606540d1726bcba4e797a22a04f75d5` |
| Smiths | `to-seat/day-06-act-Smiths.md` | `747631c09e23dcc9c3f3a0a4ae3a8c9160b85cbaf03ef6c359c59385167f1827` |
| Foresters | `to-seat/day-06-act-Foresters.md` | `88b2324fa6f9371569b72570e7af69e2205fe09c15e8608bcb863411cb0c8a74` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry note from R&D: the Smiths text matches AI Studio's synced record. R&D asked for a ruling on the Weavers' CONVERT line (below, G5).

#### Saltworks: `from-seat/day-06-act-Saltworks.md`, `c02f0127b91d27cc1e3597372bd0f296d40c41fd5f3bd3d1bbafee3b95a1cb91`

```
CONVERT: 2 brine -> 1 salt x2
TRAVEL: go to Mire
```

#### Weavers: `from-seat/day-06-act-Weavers.md`, `47baa36b00bd26daea6038e1720941e710d8fe0adad1059a5ececa04c3c748f7`

```
CONVERT: 2 wool + 1 dye x2
TRAVEL: go to Ostra
```

#### Smiths: `from-seat/day-06-act-Smiths.md`, `72fd931a0ef91735d92587d41b351cca69d05e580f6e91d07592a68470b31494`

```
OFFERS: cancel O4 ; post 10 coin for 2 charcoal ; post 1 tools for 16 coin ; post 13 coin for 2 salt ; post 2 coin for 1 ore @Kell
CONVERT: 1 ore + 1 charcoal -> 1 tools x1
TRAVEL: stay
```

#### Foresters: `from-seat/day-06-act-Foresters.md`, `5e2dc89f3fa6a2e76f073c3032c3c8ebde4de9f7b1c4a87fe0de62c21534aef1`

```
OFFERS: post 2 charcoal for 10 coin ; post 16 coin for 1 tools
CONVERT: 1 timber → 1 charcoal x2
WORKS: deliver 2 charcoal to Hath lodge
TRAVEL: stay
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Ruling on the Weavers' `CONVERT: 2 wool + 1 dye x2` (no output named): valid.** The form is `CONVERT: <recipe> x<n>` (brief "A day", act form). It doesn't require the output to be written; a recipe is identified when it names one of the guild's own converters unambiguously. The Weavers have one converter, 2 wool + 1 dye → 1 cloth (board.md "Guilds and converters"), and the line names exactly its inputs, so it can mean nothing else. This is the same reading as the Smiths' `bellows x1` (G4), which named the upgrade without its input. A CONVERT line naming inputs that match none of the guild's converters, or more than one, would be invalid for that order only (the rest of the reply stands). Logged as part of G5.

**Weavers contract-memory check (act, day 6):** no live contracts. Consistent.

### Resolution, day 6

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |
| Weavers | `TRAVEL: go to Ostra` | `e81dca8522bda28e703276369bdaab4c461138b5c641a68feca607d271d2cc03` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |

Revealed: Saltworks Varn → Mire (road 4, marsh); Weavers Mire → Ostra (road 5, short cut); Smiths and Foresters stay.

**Step 2. The day's roll:** quicknet round 33966285, randomness `735c6ff5e4f5bdb6907849fd75ada87a0244a038a80c53827a910a44dc03950f`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 73 5c | 29532 | 13 | clear |
| 1 Ostra–Kell | safe | 6f f5 | 28661 | 2 | bandits (no caravan on it) |
| 2 Kell–Hath | hill | e4 f5 | 58613 | 14 | clear |
| 3 Hath–Mire | safe | bd b6 | 48566 | 7 | clear |
| 4 Mire–Varn | marsh | 90 78 | 36984 | 5 | **flood** |
| 5 Ostra–Mire | short cut | 49 fd | 18941 | 2 | **bandits** |

- **Saltworks on road 4: flood.** "The caravan doesn't move" (board.md "Flood"): Saltworks stays in Varn. No toll. Recorded as a *holds* event on the Saltworks' caravan for day 6 (no contracts affected).
- **Weavers on road 5: bandits.** "The caravan loses its single most valuable unit (highest works value, then highest reference price)" (board.md "Bandits"). Held: 4 wool (value 1, price 1), 2 dye (value 3, price 9). Lost: **1 dye**, taken by the steward. Bandits don't stop movement (only a flood holds): the Weavers arrive in Ostra.

Caravans after moving: Saltworks Varn; Weavers Ostra; Smiths Kell; Foresters Hath.

**Step 4. Offers fill.** Day's order, lowest first: Smiths `1107abf3…`, Foresters `3681ceee…`, Saltworks `e3e08be0…`, Weavers `e4a96683…`. No two offers competed for the same goods.

Cancellations first: Smiths cancels O4 (its own).

Guild-to-guild:

| Offers that cross | Result |
|---|---|
| Foresters: 2 charcoal for 10 coin × Smiths: 10 coin for 2 charcoal | filled: Foresters −2 charcoal +10 coin; Smiths +2 charcoal −10 coin |
| Foresters: 16 coin for 1 tools × Smiths: 1 tools for 16 coin | filled: Foresters +1 tools −16 coin; Smiths −1 tools +16 coin |
| Smiths: 13 coin for 2 salt | no crossing offer; stays open as **O5** |

Ladder offers:

| Offer | Caravan in town? | Ladder total | Result |
|---|---|---|---|
| Smiths: 2 coin for 1 ore @Kell | yes | 2 | filled: −2 coin, +1 ore |

**Step 5. Conversions, then works deliveries** (G6: conversions first).
- **Saltworks in Varn** (held there by the flood, so at home at step 5): 2 brine → 1 salt ×2; the pans burn 1 timber. Brine 4 − 4 = 0; salt 2 + 2 = 4; timber 1 − 1 = 0.
- **Weavers in Ostra** (home): ordered 2 wool + 1 dye ×2; holds 4 wool and 1 dye after the bandits, enough for **1 batch** (G5: batches run up to the inputs held). Wool 4 − 2 = 2; dye 1 − 1 = 0; cloth +1.
- **Smiths in Kell:** 1 ore + 1 charcoal → 1 tools ×1. Ore 1 − 1 = 0; charcoal 3 + 2 − 1 = 4; tools 2 − 1 + 1 = 2.
- **Foresters in Hath:** cut 2 timber (G1), then 1 timber → 1 charcoal ×2. Timber 8 + 2 − 2 = 8; charcoal 2 − 2 (sold) + 2 = 2; tools 3 + 1 = 4.
- **Works: Foresters deliver 2 charcoal to the Hath lodge.** Caravan in Hath (board.md rule 1); 2 charcoal held after conversion. The lodge needs 3 charcoal, so both are taken. Score: 2 × works value 2 = **4 works points** to the Foresters. Not the last unit (the lodge still needs 4 salt and 1 charcoal), so no completion bonus. Foresters' charcoal 2 − 2 = 0.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** Open into day 7: O5 (Smiths, 13 coin for 2 salt).

**Rule gap noted (provisional) G5: a CONVERT order for more batches than the inputs held, and how a recipe is named.** The board gives the daily cap but not what happens when inputs fall short (here, after bandits took an input on the road). Decided: the order runs as many whole batches as the held inputs allow, up to the number ordered; the rest is void, with no penalty. A recipe is valid when it names one of the guild's own converters unambiguously, with or without its output.

**Rule gap noted (provisional) G6: the order within step 5.** The plan lists "conversions and works deliveries" as one step. Decided: conversions first, then deliveries, in the order the plan names them, so goods converted that day can be delivered that day.

### State after day 6

| Guild | Caravan | Purse | Goods | Standing | Upgrade | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Varn | 30 | 4 salt | 0 | — | 0 |
| Weavers | Ostra | 22 | 2 wool, 1 cloth | 0 | — | 0 |
| Smiths | Kell | 29 | 4 charcoal, 2 tools | 0 | bellows (day 2) | 0 |
| Foresters | Hath | 38 | 8 timber, 4 tools | 0 | — | 4 |

Coin check: 121 after day 5; Kell's ladder took 2; 121 − 2 = 119 = 30 + 22 + 29 + 38. Checked. (The bandits took goods, not coin: 1 dye to the steward.)

Works: Hath lodge has 2 charcoal (Foresters); still needs 4 salt, 1 charcoal. All other works untouched. Open offers: O5. Contracts and bonds: none. Missed reveals: 0 of 4 (0 of 24 so far).

### Referee word count, day 6

1538 words in this log's day-6 section, excluding the seats' verbatim replies.

---

## Day 7

**Drand round:** 33966290, randomness `abecf4569b0f22e56e98c90aa8ddd2cff54a99de2d81a215bf3b421ddabcd27e` (applied at resolution).

### Talk round: blocks sent

Each block: day-6 public results (G2; the Weavers' bandit loss is stated only in the Weavers' own block), the public state at the start of day 7 with the Hath lodge's delivery, open offer O5, **the day-8 edict announced** (setup.md "Edicts": levy on salt, quoted in full), the seat's own day-6 results, position, works points and mandates, an empty inbox, and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-07-talk-Saltworks.md` | `1dd69b4eaed829f6335ad723d39f5526e005c2f1a9c48a64a93e3dba874b822f` |
| Weavers | `to-seat/day-07-talk-Weavers.md` | `a3c8c378a207c540b1ae2386205cf1ec8277db47afc6a2dd2eeb6bcaa2501d1c` |
| Smiths | `to-seat/day-07-talk-Smiths.md` | `5dec1a4513377c7f7cb4905210495f2af8483d258668697e16d9ed0a8e71dd37` |
| Foresters | `to-seat/day-07-talk-Foresters.md` | `881e80d774778ef355db5c53c862aec2fb5e946531753a831fd411b06d1a1561` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the rendered page (synced-record check pending). The Weavers' day-7 talk prompt was delayed about 20 minutes (held 22:41 ET, sent 22:58 ET; harness, metrics.md).

#### Saltworks: `from-seat/day-07-talk-Saltworks.md`, `c449e1ab10e6bae67e6ec35f75176a4de250408160cd61c1f9dd1cce0bbb4a5e`

```
MESSAGE 1 TO Foresters: The flood held me in Varn. I have all 4 salt and set out for Hath via Mire, arriving day 8 or 9 if the road lets me. Please keep your last charcoal until my salt is in. Thanks for the 2 charcoal. No salt trades for me with the levy on, so I'm not selling any.
```

#### Weavers: `from-seat/day-07-talk-Weavers.md`, `0c4d5e513bea60b365037d30f0d82122358210bb818c5f7f90d95b8f9a6336f6`

```
MESSAGE 1 TO Saltworks: I have 1 cloth ready; would you like to buy it for 20 coin or trade it for brine or salt needed for your salt production? Let me know your terms.
MESSAGE 2 TO Smiths: When Kell reopens, I can offer 1 cloth for 2 tools (or 20‑22 coin). Could we set up an offer or contract for Day 7?
MESSAGE 3 TO Foresters: Do you have any dye to sell? I need 1 dye for another cloth conversion; I can pay market price (4 coin) or trade timber for it.
```

#### Smiths: `from-seat/day-07-talk-Smiths.md`, `69bf7b5bd2dfb82c323ae4a757214e09da66bc27be23d7950f06447dc531f8c9`

```
MESSAGE 1 TO Saltworks: Unlucky flood yesterday! Day 8 starts the salt levy, so trading today beats the tax! In Day 7 Act I'm cancelling O5 and posting TWO offers for 2 salt: 1 tools for 1 salt (3 Works VP deliverable at Ostra, Varn, or Mire!) AND 14 coin for 1 salt (fully funds 4 brine + timber to replace both salt!). Please post 1 salt for 1 tools AND 1 salt for 14 coin today!
MESSAGE 2 TO Foresters: Great Day 6 trades, and nice delivery on Hath lodge! Since Hath lodge needs 1 more charcoal, I figure you're staying in Hath today (Day 7) converting 2 charcoal. Tomorrow (Day 8), can I buy your 1 spare charcoal for 6 coin (or 2 charcoal for 12 coin if you make more)? That charcoal directly fuels my forge so I can also forge that extra tools for you on Day 8-9!
MESSAGE 3 TO Weavers: Ouch on those Mire-Ostra bandits! Did you manage to weave 1 cloth yesterday? Once I finish forging in Kell over Days 7-8, I plan to bring tools up the safe road to Ostra for Ostra crane around Day 8-9, or we can swap 1 tools for 1 cloth via Offers if you'd like! Let me know your cloth plan for Ostra crane so we can team up!
```

#### Foresters: `from-seat/day-07-talk-Foresters.md`, `50d9e912dc7d0d5d94b35cac6bed075e34741abb6ac56f3eb2b46162318652f4`

```
MESSAGE 1 TO Weavers: Rough luck with those shortcut bandits yesterday! Since they took a dye, if you're weaving your first cloth in Ostra today, conversions resolve after offers fill, so let's cross 1 cloth for 22 coin tomorrow (Day 8) instead of today. Stay safe in Ostra today to weave and grab cheap wool so you have 4 wool ready, then use tomorrow's 22 coin to buy both dye in one Mire trip!
MESSAGE 2 TO Saltworks: Sorry about yesterday's flood! As promised, I delivered 2 charcoal to Hath lodge and left the 3rd charcoal open to deliver after your 4 salt goes in, preserving your 8 to 6 lead on the lodge. I'm taking a quick road delivery loop while you travel to Hath, and will return to Hath by Day 10 to cut and drop the final charcoal right after your salt is in!
MESSAGE 3 TO Smiths: Great trades all week! Since I delivered yesterday's 2 charcoal to Hath lodge, I'm out of charcoal today and heading out on a quick road loop to deliver timber before the bell window opens. I plan to return to Hath around Day 10, so if the bell hasn't rung let's trade more charcoal then! Good luck landing that salt for Kell bridge today before tomorrow's salt levy hits!
```

**Whispers check:** 10 messages (Saltworks 1, Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 73). All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 1), from Foresters (msg 2).
- Weavers: from Smiths (msg 3), from Foresters (msg 1).
- Smiths: from Weavers (msg 2), from Foresters (msg 3).
- Foresters: from Saltworks (msg 1), from Weavers (msg 3), from Smiths (msg 2).

**Weavers contract-memory check (talk, day 7):** no live contracts. Nothing to check. (Observation, not a fault: it again writes "when Kell reopens" though the gate reopened after day 5, and offers to trade timber it doesn't hold; neither touches a contract.)

### Act round: blocks sent

Same layout and format conventions as day 1; caravan lines from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-07-act-Saltworks.md` | `5abeac25880807029737384e0fa8775a6a5ad16bce9bb5e77ba2e5c1be6f895b` |
| Weavers | `to-seat/day-07-act-Weavers.md` | `ce3fdf4a8a250b0f0ce6fce02bad30f19c82d7d480e221c212d65f17e3791a76` |
| Smiths | `to-seat/day-07-act-Smiths.md` | `33004099ca7f79b6e41770b2f1a53c14600ae2bf97358bb62e3716c51535a699` |
| Foresters | `to-seat/day-07-act-Foresters.md` | `e54acadfc1ce8e3fe814583de5085acc26627d84750b677ee1f503c235f490c7` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry note from R&D: the Smiths text was checked against the AI Studio chat file.

#### Saltworks: `from-seat/day-07-act-Saltworks.md`, `fd8d0521f94952e09337e564c607c9846828815048835a2c3baf87ced04852a9`

```
TRAVEL: go to Mire
```

#### Weavers: `from-seat/day-07-act-Weavers.md`, `f976e19f1eca4ff0783fd12446bbae183bc8e50c51dbd8a35ea07b19695506eb`

```
OFFERS: post 14 coin for 1 dye @Mire ; post 1 cloth for 1 tools
TRAVEL: go to Mire
```

#### Smiths: `from-seat/day-07-act-Smiths.md`, `e2715c3013f0bca35b452ff3d82e751c2842da0a621405f5142efdecb3948be9`

```
OFFERS: cancel O5 ; post 14 coin for 1 salt
TRAVEL: stay
```

#### Foresters: `from-seat/day-07-act-Foresters.md`, `959d2f2b0490afa0e9e690c3948dd928d416e41e66fefd8fe14d6bc57be2aaa9`

```
OFFERS: post 30 coin for 2 dye @Mire
WORKS: deliver 4 timber to Mire causeway ; deliver 2 tools to Mire causeway ; deliver 2 dye to Mire causeway
TRAVEL: go to Mire
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 7):** no live contracts. Consistent.

### Resolution, day 7

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |
| Weavers | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |

Revealed: Saltworks Varn → Mire (road 4, marsh); Weavers Ostra → Mire (road 5, short cut); Foresters Hath → Mire (road 3, safe); Smiths stay.

**Step 2. The day's roll:** quicknet round 33966290, randomness `abecf4569b0f22e56e98c90aa8ddd2cff54a99de2d81a215bf3b421ddabcd27e`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | ab ec | 44012 | 13 | clear |
| 1 Ostra–Kell | safe | f4 56 | 62550 | 11 | clear |
| 2 Kell–Hath | hill | 9b 0f | 39695 | 16 | clear |
| 3 Hath–Mire | safe | 22 e5 | 8933 | 14 | clear |
| 4 Mire–Varn | marsh | 6e 98 | 28312 | 13 | clear |
| 5 Ostra–Mire | short cut | c9 0a | 51466 | 7 | clear |

All three travellers arrive in Mire. Caravans after moving: Saltworks Mire; Weavers Mire; Smiths Kell; Foresters Mire.

**Step 4. Offers fill.** Day's order, lowest first: Weavers `286f2d52…`, Saltworks `32bf5343…`, Foresters `9815c447…`, Smiths `9f8aa14f…`.

Cancellations first: Smiths cancels O5 (its own).

Guild-to-guild:

| Offer | Result |
|---|---|
| Smiths: 14 coin for 1 salt | no crossing offer; stays open as **O6** |
| Weavers: 1 cloth for 1 tools | no crossing offer; stays open as **O7** |

Ladder offers: two bids for Mire's sit-out dye, which sells **2 a day at 14, without stepping** (board.md "Town ladders"; setup.md "Markets"). Both caravans are in Mire (board.md rule 1).

| Offer | Bid per unit | Ladder total | Result |
|---|---|---|---|
| Foresters: 30 coin for 2 dye @Mire | 15 | 14 + 14 = 28 | filled first (higher bid, G7): −28 coin (the ladder's total, G3), +2 dye. Mire's dye stock for day 7 is exhausted. |
| Weavers: 14 coin for 1 dye @Mire | 14 | 14 | **not filled**: no dye left today. Stays open as **O8**, re-checked each day the Weavers' caravan is in Mire (G3). |

"Offers fill by price, then in the day's random order" (paper-test.md step 4). Price comes first, so the Foresters' higher bid fills before the Weavers', though the day's order would have put the Weavers first. (Rule gap G7: what "by price" means against a ladder.)

**Step 5. Conversions, then works deliveries** (G6).
- No conversions: Saltworks, Weavers and Foresters are away from home; the Smiths ordered none. The Foresters are not in Hath, so they cut no timber (G1: position after movement).
- **Works: the Foresters deliver to the Mire causeway**, caravan in Mire (board.md rule 1), in the order written:
  1. 4 timber (held 8): the causeway needs 4 timber; all taken. 4 × works value 1 = 4.
  2. 2 tools (held 4): needs 2 tools; all taken. 2 × 3 = 6.
  3. 2 dye (bought today): needs 2 dye; all taken. 2 × 3 = 6. **This is the causeway's last needed good: the Mire causeway is complete**, and the Foresters score the completion bonus of 4 (brief "Score"; setup.md "Works"). It takes no more goods.

  Foresters: 4 + 6 + 6 + 4 = **20 works points** today; 24 in all. Goods: timber 8 − 4 = 4; tools 4 − 2 = 2; dye 2 − 2 = 0.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** Open into day 8: O6 (Smiths, 14 coin for 1 salt), O7 (Weavers, 1 cloth for 1 tools), O8 (Weavers, 14 coin for 1 dye @Mire).

**Rule gap noted (provisional) G7: "by price" when bids for a ladder's goods compete.** The plan says offers fill "by price, then in the day's random order" but not which price when the buyer pays the ladder's total (G3). Decided: competing bids for the same ladder stock rank by the bid's coin per unit, highest first; equal bids go by the day's order. Each filled bid still pays the ladder's price. This is how an order book ranks bids.

### State after day 7

| Guild | Caravan | Purse | Goods | Standing | Upgrade | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Mire | 30 | 4 salt | 0 | — | 0 |
| Weavers | Mire | 22 | 2 wool, 1 cloth | 0 | — | 0 |
| Smiths | Kell | 29 | 4 charcoal, 2 tools | 0 | bellows (day 2) | 0 |
| Foresters | Mire | 10 | 4 timber, 2 tools | 0 | — | 24 |

Coin check: 119 after day 6; Mire's ladder took 28; 119 − 28 = 91 = 30 + 22 + 29 + 10. Checked.

Works: **Mire causeway complete** (Foresters: 4 timber, 2 tools, 2 dye, and the bonus). Hath lodge: Foresters 2 charcoal; still needs 4 salt, 1 charcoal. Varn sea wall, Kell bridge, Ostra crane: nothing delivered. Open offers: O6, O7, O8. Contracts and bonds: none. Missed reveals: 0 of 4 (0 of 28 so far). Edict from tomorrow: levy on salt, days 8 and 9.

### Referee word count, day 7

1343 words in this log's day-7 section, excluding the seats' verbatim replies.

---

## Day 8

**Drand round:** 33966295, randomness `68c87eb06feab8d3cecc5bada832b3eeddc0033b0b371ebbf7bd56cac5a6b82a` (applied at resolution).

**Edict in force, days 8 and 9** (setup.md "Edicts"): Levy on salt. Referee's rule from setup.md: each unit of salt that passes between guilds (an offer fill or a payment) costs the receiving guild 1 coin, paid to the steward.

### Talk round: blocks sent

Each block: day-7 public results (G2), the public state at the start of day 8 with the completed Mire causeway, open offers O6–O8, the edict in force (quoted in full), the seat's own day-7 results, position, works points and mandates, an empty inbox, and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-08-talk-Saltworks.md` | `db69659fa4254d1b706acb98ee779ea2fdf92bca14aea3c2c357df5f2cb3feb1` |
| Weavers | `to-seat/day-08-talk-Weavers.md` | `b02a034a9b82c8cff2dee58ab9e2160ae58c7e3e832847d2a1917fd1761e2e76` |
| Smiths | `to-seat/day-08-talk-Smiths.md` | `57c578dc9321845a443d52f5d2b7f0bc74f22c89183f101a6a5d20d7b689819f` |
| Foresters | `to-seat/day-08-talk-Foresters.md` | `c38d3ad7dd66ff17f6d1efbedfb4815bcd9ddd4d80c5c189d679a1c434f07372` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page.

#### Saltworks: `from-seat/day-08-talk-Saltworks.md`, `70724f50033b500306f6e8096317e8311bfcbdfe3b89845104547ba95158c5b1`

```
MESSAGE 1 TO Foresters: Congratulations on the causeway. I'm going to Hath today with 4 salt for the lodge. If you go there too, deliver your last charcoal after my salt is in, or the same day, as we agreed. Then the lodge completes and you take the bonus, and I take my lead.
```

#### Weavers: `from-seat/day-08-talk-Weavers.md`, `4fdb0a78e296d7a7f27f2c1db333f3eccfa1ac1dbc9269f23e9301df4ab9c215`

```
MESSAGE 1 TO Saltworks: I have 1 cloth ready; would you like to buy it for 20 coin or trade it for brine or salt needed for your salt production? Let me know your terms.
MESSAGE 2 TO Smiths: I'd like to finalize O7 – trade my 1 cloth for 1 tool. Please accept or propose a price; I can also post an offer if needed.
MESSAGE 3 TO Foresters: Do you have a dye to sell? I can pay market price (4 coin) or trade wool for it. Let me know if you can fill O8 or arrange a trade.
```

#### Smiths: `from-seat/day-08-talk-Smiths.md`, `300af6fe1fb266a21e723437e54bd801eb31712718971e066faeda19d6ad07d5`

```
MESSAGE 1 TO Saltworks: You said ask again after Day 7! Today I'm keeping O6 (14 coin for 1 salt) open AND adding 13 coin for 1 salt: 27 COIN CASH for 2 salt! Note Foresters just hit 24 VP and would snipe Hath lodge's completion if you put 4 salt there, whereas 27 coin buys Weavers' cloth + timber for 7+ VP! Please post 1 salt for 14 coin AND 1 salt for 13 coin today!
MESSAGE 2 TO Weavers: Saw O7 and O8, glad you'll grab Mire's dye today! I must hold my current 2 tools for Kell bridge right now so I can't cross O7 yet, though if the bell goes past Day 10 I plan to forge more tools for Ostra crane! Also, Saltworks is in Mire with you and could team up with us on Ostra crane instead of feeding Foresters' 24-VP lead!
MESSAGE 3 TO Foresters: Congrats on completing Mire causeway, huge play! Safe travels on your timber loop. Keep me posted on the exact day you return to Hath: if the closing bell goes to Day 11, 12, or 13, my bellows can forge up to 3 tools a day in Kell, so we can immediately run more charcoal trades or 50/50 tools splits as soon as you're back!
```

#### Foresters: `from-seat/day-08-talk-Foresters.md`, `5a01a99a85c136cb870004b4683c47a46b1bdcf87d2b1ffaf42b17e9791d46df`

```
MESSAGE 1 TO Saltworks: Good seeing you in Mire! Hath-Mire is a safe road with zero flood risk, so your 4 salt should land in Hath lodge cleanly. As promised, I haven't touched salt, keeping your 8-point lead on the lodge completely safe. I'm finishing my timber loop and heading back to Hath to cut and deliver that final charcoal right after your salt is in!
MESSAGE 2 TO Smiths: Thanks! Yesterday's Mire detour meant I cut no timber, so I have zero charcoal on hand today. Saltworks and I are coordinating Hath lodge's finish, and as soon as I'm back in Hath cutting timber I'll gladly convert 2 charcoal for you at 12 coin! Grab cheap Kell ore while you're home so your bellows can forge extra tools right away!
MESSAGE 3 TO Weavers: Sorry we collided on Mire's dye yesterday—I needed both dye to finish Mire causeway before the bell window! I'm not buying any dye today, so Mire's market is all yours. Crucial rules tip: caravans move before offers fill, so stay in Mire today if you want your open O8 offer for 1 dye at 14 coin to fill! Good luck weaving!
```

**Whispers check:** 10 messages (Saltworks 1, Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 73). All delivered unchanged.

**Rules claims seats made to each other (recorded, not corrected; the referee never advises a seat):**
- Foresters to Saltworks: Hath–Mire "is a safe road with zero flood risk". Matches board.md (a safe road has no flood; bandits 2 in 20, toll 1 in 20 remain).
- Foresters to Weavers: "caravans move before offers fill, so stay in Mire today if you want your open O8 offer ... to fill". Matches the day's order; O8 needs the Weavers' caravan in Mire after movement (board.md rule 1, G3).
- Smiths to Saltworks: "Foresters just hit 24 VP". Derivable from the public deliveries (Hath lodge 2 charcoal = 4, Mire causeway 16 + bonus 4).
- Weavers to Foresters: offers to buy dye at "market price (4 coin)". Mire's market sells dye at 14; 4 is what a market pays for dye. A misreading by the seat, not a rule question.
- Weavers to Smiths: asks it to "accept or propose a price" on O7. Offers are not accepted; they fill when a crossing offer is posted. A misreading by the seat. All delivered.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 1), from Foresters (msg 1).
- Weavers: from Smiths (msg 2), from Foresters (msg 3).
- Smiths: from Weavers (msg 2), from Foresters (msg 2).
- Foresters: from Saltworks (msg 1), from Weavers (msg 3), from Smiths (msg 3).

**Weavers contract-memory check (talk, day 8):** no live contracts. Nothing to check. It names its own open offers O7 and O8 correctly (O7: 1 cloth for 1 tools; O8: 1 dye at 14 coin).

### Act round: blocks sent

Same layout and format conventions as day 1; caravan lines from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-08-act-Saltworks.md` | `cd292b997aff44f6a6482298038323d97b73c6356c397cf5c876a7471f46e25a` |
| Weavers | `to-seat/day-08-act-Weavers.md` | `a8759fff43818a570d7a496d2c97f531373ea21b4924a391664caa02071cbb57` |
| Smiths | `to-seat/day-08-act-Smiths.md` | `4549c7d1650172be1ff45d8dbf4bea65cdf504836184332fa47f55937fcac0e1` |
| Foresters | `to-seat/day-08-act-Foresters.md` | `e34ef4c413578950b76dda59b7e9a000f187f8eb92a8a830a7ef79da5fdd9377` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page.

#### Saltworks: `from-seat/day-08-act-Saltworks.md`, `35e4880e291420198858cf4f233e21692a059148c79ff3d889f3776b4de3a519`

```
TRAVEL: go to Hath
```

#### Weavers: `from-seat/day-08-act-Weavers.md`, `6f72978537f815c7e13a56dd44ff12bca8cea99983767456b5ab0a6b4d950f19`

```
OFFERS: post 14 coin for 1 dye @Mire ; accept O7
CONVERT: 2 wool + 1 dye x1
TRAVEL: stay
```

#### Smiths: `from-seat/day-08-act-Smiths.md`, `71fc32fa5c4cb3bcc3de16a67dd5b9ca3031e549da768cf1384445c1560aafe2`

```
OFFERS: post 13 coin for 1 salt
TRAVEL: stay
```

#### Foresters: `from-seat/day-08-act-Foresters.md`, `5e830ff742d6c251b6b3e2b220481fe0cf3ec963a9e16c88795ada09b5091296`

```
WORKS: deliver 4 timber to Varn sea wall ; deliver 2 tools to Varn sea wall
TRAVEL: go to Varn
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals. One order is void on its own terms (F2): the Weavers' `accept O7` names an offer, not a contract (the act form's `accept` takes a contract id), and O7 is the Weavers' own offer. It has no effect; the rest of the reply stands.

**Weavers contract-memory check (act, day 8):** no live contracts. Its `accept O7` treats its own open offer as something to accept; this is a misreading of offers (noted on day 8's talk too), not a lost contract, so it is not a harness fault.

### Resolution, day 8

**Edict in force:** levy on salt (days 8 and 9). No salt passes between guilds today, so no levy is charged.

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: go to Hath` | `d79aa6bf5a91011eb3080be4a78c936062d6a2c2ad3fb81ce06595af051beedf` |
| Weavers | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | `TRAVEL: go to Varn` | `04b9fa155844701211698d2fe32c50fcdedb94a61386dac324c8d640585c8723` |

Revealed: Saltworks Mire → Hath (road 3, safe); Foresters Mire → Varn (road 4, marsh); Weavers and Smiths stay.

**Step 2. The day's roll:** quicknet round 33966295, randomness `68c87eb06feab8d3cecc5bada832b3eeddc0033b0b371ebbf7bd56cac5a6b82a`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 68 c8 | 26824 | 5 | toll 2 (no caravan on it) |
| 1 Ostra–Kell | safe | 7e b0 | 32432 | 13 | clear |
| 2 Kell–Hath | hill | 6f ea | 28650 | 11 | clear |
| 3 Hath–Mire | safe | b8 d3 | 47315 | 16 | clear |
| 4 Mire–Varn | marsh | ce cc | 52940 | 1 | **bandits** |
| 5 Ostra–Mire | short cut | 5b ad | 23469 | 10 | clear |

- Saltworks on road 3: clear, arrives in Hath.
- **Foresters on road 4: bandits.** The caravan loses its single most valuable unit (board.md "Bandits"). Held: 4 timber (value 1, price 1), 2 tools (value 3, price 9). Lost: **1 tools**, to the steward. The caravan still moves (only a flood holds): it arrives in Varn.

Caravans after moving: Saltworks Hath; Weavers Mire; Smiths Kell; Foresters Varn.

**Step 4. Offers fill.** Day's order, lowest first: Weavers `57227b5a…`, Foresters `b4ce8311…`, Smiths `ca7ff0cd…`, Saltworks `f84178fc…`.

New offers get ids in the order posted: the Weavers' new dye bid is **O9**, the Smiths' new salt bid is **O10**.

Guild-to-guild: no crossing offers. O6 (Smiths, 14 coin for 1 salt), O7 (Weavers, 1 cloth for 1 tools) and O10 (Smiths, 13 coin for 1 salt) stay open. No salt passes, so no levy.

Ladder offers: the Weavers' caravan is in Mire (board.md rule 1) with two bids for Mire's dye (2 a day at 14): O8 (open since day 7) and O9 (new), both 14 coin for 1 dye.
- Equal bids by the same guild fill oldest first (G9). **O8 fills:** Weavers −14 coin, +1 dye. Purse 22 − 14 = 8.
- **O9 does not fill:** the Weavers hold 8 coin, less than the 14 it would cost (G9: an offer its owner can't pay at fill time doesn't fill, and stays open).

**Step 5. Conversions, then works deliveries** (G6).
- Weavers ordered 2 wool + 1 dye ×1, but their caravan is in Mire, not Ostra: "A guild converts only on a day its caravan is in its home town" (board.md). The order is void; no cloth.
- No other conversions (Saltworks, Foresters away from home; the Smiths ordered none).
- **Works: the Foresters deliver to the Varn sea wall,** caravan in Varn (board.md rule 1):
  1. 4 timber (held 4): the sea wall needs 4 timber; all taken. 4 × 1 = 4.
  2. 2 tools ordered, **1 held** after the bandits: 1 tools delivered (G8). 1 × 3 = 3. The sea wall still needs 2 tools and 2 cloth, so no completion bonus.

  Foresters: **7 works points** today; 31 in all. Goods: timber 0, tools 0.
- Saltworks reached Hath with 4 salt but ordered no delivery.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** Open into day 9: O6, O7, O9, O10.

**The bell:** by the design it can ring only at the close of day 10 or later (brief "Edicts and the bell"). The game goes on to day 9.

**Rule gap noted (provisional) G8: a works delivery of more units than held.** The Foresters ordered 2 tools after bandits took one on the road. Decided as for conversions (G5): deliver the whole units held, up to the number ordered; the rest is void without penalty.

**Rule gap noted (provisional) G9: an offer its owner can't pay, and equal bids by one guild.** Decided: offers are not funded in advance (on the ledger an offer is checked for funds when it would fill), so an offer whose owner lacks the goods or coin at fill time doesn't fill and stays open. Equal bids by the same guild fill oldest first.

**Format note F2:** an order that names the wrong kind of object (here `accept` with an offer id, on the OFFERS line) is void for that order only; the reply's other orders stand. Same rule as G5's invalid recipe.

### State after day 8

| Guild | Caravan | Purse | Goods | Standing | Upgrade | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Hath | 30 | 4 salt | 0 | — | 0 |
| Weavers | Mire | 8 | 2 wool, 1 dye, 1 cloth | 0 | — | 0 |
| Smiths | Kell | 29 | 4 charcoal, 2 tools | 0 | bellows (day 2) | 0 |
| Foresters | Varn | 10 | — | 0 | — | 31 |

Coin check: 91 after day 7; Mire's ladder took 14; 91 − 14 = 77 = 30 + 8 + 29 + 10. Checked. The bandits took 1 tools (goods, not coin).

Works: Mire causeway complete (Foresters). Varn sea wall: Foresters 4 timber, 1 tools; still needs 2 tools, 2 cloth. Hath lodge: Foresters 2 charcoal; still needs 4 salt, 1 charcoal. Kell bridge, Ostra crane: nothing delivered. Open offers: O6, O7, O9, O10. Contracts and bonds: none. Missed reveals: 0 of 4 (0 of 32 so far).

### Referee word count, day 8

1686 words in this log's day-8 section, excluding the seats' verbatim replies.

---

## Day 9

**Drand round:** 33966300, randomness `de6b318efbb6d121537d0ed1f36bf7835ea23006518168ca1f3d3ef17312bc5a` (applied at resolution).

**Edict in force, day 9 (its last day):** levy on salt (setup.md "Edicts").

### Talk round: blocks sent

Each block: day-8 public results (G2; the Foresters' bandit loss and the Weavers' void orders are stated only in their own blocks), the public state at the start of day 9, open offers O6, O7, O9, O10, the edict in force, the seat's own day-8 results, position, works points and mandates, an empty inbox, and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-09-talk-Saltworks.md` | `0940cc7377d242c6f09de3f19d0e634952961514f19083fc4a66aa93de61d96f` |
| Weavers | `to-seat/day-09-talk-Weavers.md` | `da553276865377d7f050adaf784a1c0be79460ee43cef940b5cbd10245268279` |
| Smiths | `to-seat/day-09-talk-Smiths.md` | `c5a9919613365db309786c006027736656571788a679ad34347cd6c2d878e86f` |
| Foresters | `to-seat/day-09-talk-Foresters.md` | `f22a0c9a6011554a72082d8ad964e7fc6ff53fb28caa50e0e6b1f9274e46d1ad` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page.

#### Saltworks: `from-seat/day-09-talk-Saltworks.md`, `0e93c51248209d25b98486fea1363bc9da1e4d74b0318a1afe12415150a802a1`

```
NO MESSAGES
```

#### Weavers: `from-seat/day-09-talk-Weavers.md`, `4fc21ad7d5082938f1bda7a08fc0b28dc03cb9d358fb7627e6255e04b3bf893c`

```
MESSAGE 1 TO Smiths: I have a cloth to trade for tools (my O7 offer). Let me know if you’d like to accept or propose a tool swap to aid the Ostra crane.
MESSAGE 2 TO Foresters: Do you have any dye for sale? I need dye to convert wool into more cloth; I can pay market price (4 coin) or trade wool.
MESSAGE 3 TO Saltworks: If you need wool or cloth, I can sell; otherwise I’m interested in buying brine or salt for future trades. Let me know your terms.
```

#### Smiths: `from-seat/day-09-talk-Smiths.md`, `102de705557774a8c25fde991eec1652795faec5955ca245de7178d3f1cb614d`

```
MESSAGE 1 TO Saltworks: Foresters just hit 31 Works VP while the rest of us sit at 0! Putting 4 salt into Hath lodge hands Foresters another +6 VP completion on Day 10. Instead, my O6 (14 coin for 1 salt) and O10 (13 coin for 1 salt) are BOTH open today for 27 COIN CASH for 2 salt! Or if you use your 4 salt at Hath, will you make 2 more salt for Day 11-12?
MESSAGE 2 TO Foresters: Sorry about those Mire-Varn bandits, still a great Varn sea wall delivery! Yes, 12 coin for 2 charcoal as soon as you're back in Hath sounds great! Are you heading Varn to Mire today (Day 9) and Mire to Hath on Day 10 to convert charcoal on Day 10 (for Day 11 trade)? Let me know your exact schedule so I time my Kell ore purchases right!
MESSAGE 3 TO Weavers: Nice pickup on that Mire dye yesterday! Are you heading back to Ostra today to weave your 2nd cloth for Ostra crane? I still have to hold my current 2 tools in Kell while waiting on salt for Kell bridge, though as soon as Foresters returns to Hath around Day 10-11 to supply charcoal, I'll forge extra tools and gladly cross O7 or deliver to Ostra crane!
```

#### Foresters: `from-seat/day-09-talk-Foresters.md`, `9709b1f3e044a6068ddf1660e0a34a6e1616a1ab71c2a521a2eb7a894bf31003`

```
MESSAGE 1 TO Saltworks: Glad you reached Hath safely! Crucial rules reminder for today: caravans move before works deliveries resolve, so make sure you stay in Hath today while delivering your 4 salt to Hath lodge (if you travel today, the delivery won't go through). I'm heading Varn to Mire today and Mire to Hath tomorrow (Day 10) to cut and deliver the final charcoal after your 4 salt is in, locking your 8 to 6 lodge lead!
MESSAGE 2 TO Weavers: I hold no dye, but good news: O8 filled yesterday, so you already hold 1 dye plus 2 wool! Cancel O7 and O9, go to Ostra today, and submit 1 cloth conversion today, weaving your 2nd cloth immediately! Definitely cancel O7: 1 cloth scores the same 3 points as 1 tools, so deliver both cloth yourself to Varn sea wall or Ostra crane on Day 10!
MESSAGE 3 TO Smiths: Bandits stole my second tools yesterday! I'm heading back via Mire today to reach Hath on Day 10, converting 2 charcoal (1 for Hath lodge, 1 spare for you, plus 2 more on Day 11). Want to cross 1 charcoal for 6 coin on Day 10, then 2 charcoal for 1 tools on Day 11 so I can replace that stolen tools? Grab cheap Kell ore today so your bellows stay busy!
```

**Whispers check:** 9 messages (Saltworks `NO MESSAGES`, Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 74). All delivered unchanged.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 3), from Smiths (msg 1), from Foresters (msg 1).
- Weavers: from Smiths (msg 3), from Foresters (msg 2).
- Smiths: from Weavers (msg 1), from Foresters (msg 3).
- Foresters: from Weavers (msg 2), from Smiths (msg 2).

**Rules claims seats made to each other (recorded, not corrected):**
- Foresters to Saltworks: "caravans move before works deliveries resolve, so make sure you stay in Hath today while delivering ... (if you travel today, the delivery won't go through)". Matches the day's order and board.md rule 1 (deliveries at step 5 need the caravan in the town after movement).
- Foresters to Weavers: go to Ostra and convert the same day; "1 cloth scores the same 3 points as 1 tools". Both match (movement before conversion; cloth and tools both have works value 3).
- Smiths to Saltworks: "Foresters just hit 31 Works VP"; the lodge's last charcoal would give the Foresters "another +6 VP completion". Both derivable from public state: 31 is correct; one charcoal (2) plus the completion bonus (4) is 6.
- Weavers to Foresters: dye at "market price (4 coin)" (Mire sells at 14; 4 is the buying price). Weavers to Smiths: "accept" O7. Both repeat the day-8 misreadings.

**Weavers contract-memory check (talk, day 9):** no live contracts. Nothing to check. It names O7 correctly as its 1 cloth for tools offer.

### Act round: blocks sent

Same layout and format conventions as day 1; caravan lines from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-09-act-Saltworks.md` | `c9cfcb80e74354316dbcf5e16a9b6ba0bc2c4038712525990b749102fe904dee` |
| Weavers | `to-seat/day-09-act-Weavers.md` | `2cbe83b9a39e82ad9f85d5ac51ab3b9af661bd3f9d45cba5b9085369ae56e06d` |
| Smiths | `to-seat/day-09-act-Smiths.md` | `0e485ed8518f1d90f034dd6645275d0d6ef2fba030c438ddd48aa949ad7cd8d1` |
| Foresters | `to-seat/day-09-act-Foresters.md` | `1d6f559387617cfb35f07f10c18226a3d992015efa8c4e2729963b1954c5fa8c` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page.

**Two seats did not answer the act round on their first turn, and R&D re-prompted both:**
- Weavers: the seat replied "DONE" without writing its answer file. Re-prompted at 23:45 ET.
- Foresters: the Gemini chat stalled with no reply for about 16 minutes after the act block was sent at 23:41 ET. Re-prompted at 23:57 ET.

**RULING G10 (R&D, after the referee's provisional decision): both day-9 act replies are MISSED REVEALS (`NO REPLY`).** paper-test.md and the referee's brief both say a missed reply is a missed reveal and nobody asks a seat again; the two re-prompts broke that rule. The re-prompted replies are **void**. Their files stay in `from-seat/` as a record and are shown below marked VOID; none of their orders is applied. (The referee had first ruled them on time, and had already sent day-10 talk blocks built on that. Those blocks are withdrawn, kept in `to-seat/void/`, and replaced; see Day 10.)

#### Saltworks: `from-seat/day-09-act-Saltworks.md`, `b11de601512d8d2ec7a7d1dcf34632a67e485583263ac98d0b71757f40edb49e`

```
WORKS: deliver 4 salt to Hath lodge
TRAVEL: stay
```

#### Weavers: `from-seat/day-09-act-Weavers.md`, `6cee2130b004803c425a0e59f8fc368174287186253aa00da14eca8fb33464b1` — **VOID (G10: missed reveal; re-prompted reply)**

```
OFFERS: cancel O7 ; cancel O9
TRAVEL: go to Ostra
CONVERT: 2 wool + 1 dye x1
```

#### Smiths: `from-seat/day-09-act-Smiths.md`, `4deeaf04dbe9091fed33e6646b42c1e1c9268eb254b0e0d95ac4ee17310b7d83`

```
TRAVEL: stay
```

#### Foresters: `from-seat/day-09-act-Foresters.md`, `fd8d0521f94952e09337e564c607c9846828815048835a2c3baf87ced04852a9` — **VOID (G10: missed reveal; re-prompted reply)**

```
TRAVEL: go to Mire
```

**Validity:** Saltworks and Smiths: in the form with a TRAVEL line, valid. **Weavers and Foresters: missed reveals** (G10). The Publisher added that both are marked **HARNESS-CAUSED** in metrics.md (Weavers: tool quirk; Foresters: Gemini platform stall), so the games 1–2 review can discount their effect. The ruling and this resolution are unchanged. Board.md rule 2 says a missed reveal means the caravan stays and pays a toll of 2, and none of the seat's other orders applies.

**Weavers contract-memory check (act, day 9):** no live contracts; missed reveal. Nothing to check.

### Resolution, day 9 (corrected under G10)

**Edict in force (last day):** levy on salt. No salt passes between guilds, so no levy is charged. The levy lapses at the close of day 9.

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Weavers | none: missed reveal | — |
| Smiths | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Foresters | none: missed reveal | — |

Revealed: Saltworks and Smiths stay. **Weavers and Foresters: missed reveals.** Each caravan stays in town and pays the toll of 2 (board.md rule 2; design.md "Fair dice": no re-roll and no second ask). Neither guild has standing 5 or more, so no halving.

**Step 2. The day's roll:** quicknet round 33966300, randomness `de6b318efbb6d121537d0ed1f36bf7835ea23006518168ca1f3d3ef17312bc5a`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | de 6b | 56939 | 20 | fair (no caravan on it) |
| 1 Ostra–Kell | safe | 31 8e | 12686 | 7 | clear |
| 2 Kell–Hath | hill | fb b6 | 64438 | 19 | clear |
| 3 Hath–Mire | safe | d1 21 | 53537 | 18 | clear |
| 4 Mire–Varn | marsh | 53 7d | 21373 | 14 | clear |
| 5 Ostra–Mire | short cut | 0e d1 | 3793 | 14 | clear |

No caravan travels. Missed-reveal tolls: **Weavers −2 coin** (8 → 6), **Foresters −2 coin** (10 → 8), both to the steward.

Caravans: Saltworks Hath; Weavers Mire; Smiths Kell; Foresters Varn.

**Step 4. Offers fill.** Day's order, lowest first: Saltworks `1c548548…`, Foresters `4a93554a…`, Smiths `4b18f4b9…`, Weavers `afa6e866…`.

No cancellations (the Weavers' cancels were in the void reply). No guild-to-guild offers cross: O6 and O10 (Smiths' salt bids) and O7 (Weavers, 1 cloth for 1 tools) stay open. Ladder: O9 (Weavers, 14 coin for 1 dye @Mire). The Weavers' caravan is in Mire, but they hold 6 coin, less than 14, so it **does not fill** (G9) and stays open.

**Step 5. Conversions, then works deliveries** (G6).
- No conversions: the Weavers' order was void, and they are in Mire, not Ostra.
- **Works: the Saltworks deliver 4 salt to the Hath lodge,** caravan in Hath (board.md rule 1). The lodge needs 4 salt; all taken. 4 × works value 2 = **8 works points**. It still needs 1 charcoal, so no completion bonus. Saltworks' salt 4 − 4 = 0.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** Open into day 10: O6, O7, O9, O10.

**The bell:** it can ring only at the close of day 10 or later (brief "Edicts and the bell"). The game goes on to day 10.

### State after day 9 (corrected)

| Guild | Caravan | Purse | Goods | Standing | Upgrade | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Hath | 30 | — | 0 | — | 8 |
| Weavers | Mire | 6 | 2 wool, 1 dye, 1 cloth | 0 | — | 0 |
| Smiths | Kell | 29 | 4 charcoal, 2 tools | 0 | bellows (day 2) | 0 |
| Foresters | Varn | 8 | — | 0 | — | 31 |

Coin check: 77 after day 8; tolls took 2 + 2 = 4; 77 − 4 = 73 = 30 + 6 + 29 + 8. Checked. Steward's take from tolls: 4.

Works: Mire causeway complete (Foresters). Hath lodge: Foresters 2 charcoal (value 4), Saltworks 4 salt (value 8); still needs 1 charcoal. Varn sea wall: Foresters 4 timber, 1 tools; still needs 2 tools, 2 cloth. Kell bridge, Ostra crane: nothing delivered. Open offers: O6, O7, O9, O10. Contracts and bonds: none. **Missed reveals: 2 of 4 today (2 of 36 so far, 5.6%).**

### Referee word count, day 9

1445 words in this log's day-9 section (corrected), excluding the seats' verbatim replies.

---

## Day 10

**Drand round:** 33966305, randomness `fbe51f5f9e9270261c84d859f11d9d8d6d82ccb7c031964d81d0353e6fa75c2c` (applied at resolution).

### Talk round: blocks sent

**Withdrawn first version.** The referee's first day-10 talk blocks were built on the provisional G10 decision. They were withdrawn before any seat answered, after R&D's ruling, and moved to `to-seat/void/` (not sent to seats as current):

| Seat | Withdrawn file | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/void/day-10-talk-Saltworks.v1-void.md` | `efc66130c0886b363614c61ac40f4a2cad1d010698e9ad5853f58caa3388928f` |
| Weavers | `to-seat/void/day-10-talk-Weavers.v1-void.md` | `542dac9da0212b784fbab92131ff7f8d3f79ab613c1c5c94e4ee8ed337a0dbfb` |
| Smiths | `to-seat/void/day-10-talk-Smiths.v1-void.md` | `45ce9e304abb6b687cc9e354bb91a1e3b8d0192567b3232129792bb60fb30cbc` |
| Foresters | `to-seat/void/day-10-talk-Foresters.v1-void.md` | `8414db8924314e0476c1a99dc2d42113f7db119807d0cecffacc47ebeedc7c65` |

**Current blocks,** from the corrected state. Each holds: day-9 public results (G2), including that the Weavers and Foresters missed their act reply and paid the toll; the public state at the start of day 10; open offers O6, O7, O9 and O10; no edict (the levy ended); the bell's public window (days 10–13, as in the brief, with nothing about which day); the seat's own day-9 results, position, works points and mandates; an empty inbox; and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-10-talk-Saltworks.md` | `c57b72b28969b80507045c5f94657014ff9c0cba2ddf9007ea15f45c63ff8601` |
| Weavers | `to-seat/day-10-talk-Weavers.md` | `73fd6efe0232abd3d4d13b192dbecb5cfdeb351730c842196c01c375bea69112` |
| Smiths | `to-seat/day-10-talk-Smiths.md` | `1f8425fe5cd9d44b9e7f498ddb7f3d881aede3068be890af30abf40fa2b55416` |
| Foresters | `to-seat/day-10-talk-Foresters.md` | `7b51ff7fa0b1a7502b1283de6f35f0550d64937b1313ae438a6a31553f0a9342` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page; all on time, no re-prompts.

#### Saltworks: `from-seat/day-10-talk-Saltworks.md`, `0e93c51248209d25b98486fea1363bc9da1e4d74b0318a1afe12415150a802a1`

```
NO MESSAGES
```

#### Weavers: `from-seat/day-10-talk-Weavers.md`, `f881c69fe20a20583024d13bd582cac6eba3b41c45d1d907dfe2905e22a7bbcc`

```
MESSAGE 1 TO Saltworks: I have 1 cloth ready; would you trade it for 1 tool (or 8 coin) to help me deliver cloth to the Ostra crane? Let me know your terms.
MESSAGE 2 TO Smiths: I can offer my cloth for a tool (or 6 coin) to aid your Kell bridge work. Please accept or propose a swap.
MESSAGE 3 TO Foresters: Do you have any salt or timber to sell? I can pay market price (4‑5 coin per unit) or trade wool.
```

#### Smiths: `from-seat/day-10-talk-Smiths.md`, `6e9b1da3be6e477ee2b5073a40e7cdef65748eb85baf8b0d46aef02288e55aa0`

```
MESSAGE 1 TO Saltworks: Nice 4-salt delivery! If the bell reaches Day 13, let's both score big: start moving toward Ostra TODAY (Mire or Kell Day 10, Ostra Day 11 for 4 brine, Varn Day 12) instead of staying for timber! I'll grab 1 timber today to send you via Offers by Day 12 so you convert 2 salt Day 12, and Day 13 I'll pay 1 tools + 12 coin (or 22 coin) for 2 salt!
MESSAGE 2 TO Foresters: Tough break on the missed Day 9 move! Since you'll go Varn to Mire today (Day 10) and Mire to Hath on Day 11, converting 2 charcoal on Day 11 makes it tradable on Day 12. If the bell goes to Day 12-13, I am 100% down to cross 1 tools for 2 charcoal (or 12 coin for 2 charcoal) on Day 12! Safe travels back south!
MESSAGE 3 TO Weavers: Sorry about the missed Day 9 move! Since the closing bell window opens today (Days 10-13), heading Mire to Ostra today lets you weave your 2nd cloth and score on Ostra crane right away! If the bell continues past today, would you also be interested in selling 1 cloth for 15 coin on Day 11, or swapping via O7 once I forge extra tools?
```

#### Foresters: `from-seat/day-10-talk-Foresters.md`, `ba8efdfee0d0e5b4a0d6f636e1e0667443074e2e0cf68a437111137b5a21152c`

```
MESSAGE 1 TO Saltworks: Congrats on landing all 4 salt in Hath lodge! Sorry I missed yesterday's turn. I go Varn to Mire today and Mire to Hath tomorrow (Day 11) to convert and deliver the final charcoal, locking your 8 to 6 lodge lead! Tip: head toward Ostra today so if the bell goes to Day 13, you grab brine Day 11, boil Day 12 (with my remote timber), and score on Ostra crane!
MESSAGE 2 TO Smiths: Sorry I missed yesterday's turn! Since the bell can ring tonight (Day 10), definitely bank your Kell bridge charcoal and tools deliveries safely today! I go Varn to Mire today and Mire to Hath tomorrow (Day 11), converting 2 charcoal (1 finishes Hath lodge, 1 spare). If the game continues, let's cross 1 charcoal for 6 coin on Day 12 and 2 charcoal for 1 tools on Day 13!
MESSAGE 3 TO Weavers: Looks like we both missed yesterday's turn! Urgent bell tip for today (Day 10): the game can end tonight! Since you hold 1 cloth, 2 wool, and 1 dye in Mire, cancel O7 and O9, travel to Ostra today, convert 2 wool plus 1 dye into your 2nd cloth today, and deliver both 2 cloth to Ostra crane today! Caravans move before conversions and deliveries, scoring you 6 points immediately!
```

**Whispers check:** 9 messages (Saltworks `NO MESSAGES`, the same bytes as its day-9 talk reply; Weavers 3, Smiths 3, Foresters 3). All on time with no re-prompts (R&D). All in the form, addressed to seated guilds, at most 80 words (longest 73). All delivered unchanged.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 1), from Foresters (msg 1).
- Weavers: from Smiths (msg 3), from Foresters (msg 3).
- Smiths: from Weavers (msg 2), from Foresters (msg 2).
- Foresters: from Weavers (msg 3), from Smiths (msg 2).

**Rules claims seats made to each other (recorded, not corrected):**
- Foresters to Weavers: travel to Ostra, convert and deliver both cloth the same day, since "Caravans move before conversions and deliveries". Matches the day's order and G6 (conversions before deliveries).
- Foresters to Smiths, and Smiths to Weavers: the bell can ring at the close of day 10. Matches the brief (days 10–13).
- Weavers to Foresters: salt or timber at "market price (4-5 coin per unit)". No market sells salt; Hath sells timber from 2 a unit, stepping. A misreading by the seat.
- Weavers to Saltworks and Smiths: cloth for "1 tool (or 8 coin)" and "a tool (or 6 coin)", and again "accept or propose a swap". The prices are play; "accept" repeats the day-8 misreading of offers.

**Weavers contract-memory check (talk, day 10):** no live contracts. Nothing to check. (Observation: it says it has "1 cloth ready", which matches its holding after the day-9 missed reveal.)

### Act round: blocks sent

Same layout and format conventions as day 1; caravan lines from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-10-act-Saltworks.md` | `98b9844bc4eca6d29bb10cca04e51ec6eb4bff81768e4696cbb3c33abc0d7a1a` |
| Weavers | `to-seat/day-10-act-Weavers.md` | `58f4adf152f75f1b222345b530c5bbe7ac47172129fb3b03d580a19c8650afd4` |
| Smiths | `to-seat/day-10-act-Smiths.md` | `bb0e8cb5542f845e1cc928e8d666cd1645c313560a6c8b923a47f2ba165992b8` |
| Foresters | `to-seat/day-10-act-Foresters.md` | `3df96cd039e93360986b927ba5d524cd3b01892d52149c775e195c9c5ee4ab67` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. All on time, no re-prompts (R&D). Carry notes: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page. (Saltworks' file is byte-identical to the Smiths' day-9 act reply, and the Foresters' to their day-9 void reply, hence the repeated hashes.) Harness note from R&D: the Weavers' path check listed its own Antigravity artifact folder, because it wrote `out/day-10-r2.md` there by absolute path (transcript step 133). That folder is the autocopy source; it read and wrote nothing else (metrics.md).

#### Saltworks: `from-seat/day-10-act-Saltworks.md`, `4deeaf04dbe9091fed33e6646b42c1e1c9268eb254b0e0d95ac4ee17310b7d83`

```
TRAVEL: stay
```

#### Weavers: `from-seat/day-10-act-Weavers.md`, `194f2ecb414283c082ebe4b9d5e0c56ce8b048a3e0d3debf6c4967bc2f5db0e3`

```
OFFERS: cancel O7 ; cancel O9
TRAVEL: go to Ostra
CONVERT: 2 wool + 1 dye x1
WORKS: deliver 2 cloth to Ostra crane
```

#### Smiths: `from-seat/day-10-act-Smiths.md`, `6060a2b5e7e6c6a55f1bbe0015d8e17123e21baa66dcdd17256b98b4a45fb8e2`

```
OFFERS: cancel O6 ; cancel O10 ; post 2 coin for 1 timber @Hath
WORKS: deliver 1 charcoal to Hath lodge
TRAVEL: go to Hath
```

#### Foresters: `from-seat/day-10-act-Foresters.md`, `fd8d0521f94952e09337e564c607c9846828815048835a2c3baf87ced04852a9`

```
TRAVEL: go to Mire
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 10):** no live contracts. It cancels its own O7 and O9 by their correct ids. Consistent.

### Resolution, day 10

**Edicts:** none in force (the Kell gate ended after day 5, the salt levy after day 9). The Smiths may leave Kell.

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Weavers | `TRAVEL: go to Ostra` | `e81dca8522bda28e703276369bdaab4c461138b5c641a68feca607d271d2cc03` |
| Smiths | `TRAVEL: go to Hath` | `d79aa6bf5a91011eb3080be4a78c936062d6a2c2ad3fb81ce06595af051beedf` |
| Foresters | `TRAVEL: go to Mire` | `c90700b3827074c4dacc968962b3ebc48992a7ff7e25bddbc3a5e513815af937` |

Revealed: Weavers Mire → Ostra (road 5, short cut); Smiths Kell → Hath (road 2, hill); Foresters Varn → Mire (road 4, marsh); Saltworks stays.

**Step 2. The day's roll:** quicknet round 33966305, randomness `fbe51f5f9e9270261c84d859f11d9d8d6d82ccb7c031964d81d0353e6fa75c2c`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | fb e5 | 64485 | 6 | clear (safe road: 6 is clear) |
| 1 Ostra–Kell | safe | 1f 5f | 8031 | 12 | clear |
| 2 Kell–Hath | hill | 9e 92 | 40594 | 15 | clear |
| 3 Hath–Mire | safe | 70 26 | 28710 | 11 | clear |
| 4 Mire–Varn | marsh | 1c 84 | 7300 | 1 | **bandits** |
| 5 Ostra–Mire | short cut | d8 59 | 55385 | 6 | **flood** |

- Smiths on road 2: clear, arrive in Hath.
- **Foresters on road 4: bandits.** The Foresters hold no goods, so "it loses 3 coin, or all it has" (board.md "Bandits"): −3 coin (8 → 5), to the steward. They still move: arrive in Mire.
- **Weavers on road 5: flood.** "The caravan doesn't move" (board.md "Flood"): the Weavers stay in Mire. No toll. Recorded as a *holds* event (no contracts affected).

Caravans after moving: Saltworks Hath; Weavers Mire; Smiths Hath; Foresters Mire.

**Step 4. Offers fill.** Day's order, lowest first: Saltworks `6de3043f…`, Foresters `72b9f51a…`, Weavers `8f557d64…`, Smiths `9fc52234…`.

Cancellations: Weavers cancel O7 and O9; Smiths cancel O6 and O10 (each its own). Offers need no location, so the Weavers' cancels apply though the flood held them.

Ladder offer (board.md rule 1; G3):

| Offer | Caravan in town? | Ladder total | Result |
|---|---|---|---|
| Smiths: 2 coin for 1 timber @Hath | yes (arrived) | 2 | filled: −2 coin, +1 timber |

No guild-to-guild offers. No offers remain open.

**Step 5. Conversions, then works deliveries** (G6).
- **Weavers:** ordered 2 wool + 1 dye ×1 and to deliver 2 cloth to the Ostra crane, but the flood kept their caravan in Mire. Converting needs the caravan at home (Ostra), and delivering needs it in the works' town (board.md "Where converting happens" and rule 1). **Both orders are void**; nothing changes.
- **Works: the Smiths deliver 1 charcoal to the Hath lodge,** caravan in Hath (board.md rule 1). The lodge needed exactly 1 charcoal, so this is its **last needed good: the Hath lodge is complete.** 1 × works value 2 = 2, plus the completion bonus of 4 = **6 works points** to the Smiths. The lodge takes no more goods. Smiths' charcoal 4 − 1 = 3.
- No conversions (the Smiths are away from Kell; no one else ordered one at home).

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill.** No offers open.

**The bell:** it does not ring at the close of day 10. The game goes on to day 11. (The bell day is sealed; the referee checks it against setup.md's commitment when it rings.)

### State after day 10

| Guild | Caravan | Purse | Goods | Standing | Upgrade | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Hath | 30 | — | 0 | — | 8 |
| Weavers | Mire | 6 | 2 wool, 1 dye, 1 cloth | 0 | — | 0 |
| Smiths | Hath | 27 | 3 charcoal, 2 tools, 1 timber | 0 | bellows (day 2) | 6 |
| Foresters | Mire | 5 | — | 0 | — | 31 |

Coin check: 73 after day 9; Hath's ladder took 2 and the bandits 3; 73 − 5 = 68 = 30 + 6 + 27 + 5. Checked. Steward's take so far: tolls 4, bandits 3 coin (plus goods: 1 dye on day 6, 1 tools on day 8).

Works: **Hath lodge complete** (day 10, Smiths): Saltworks 4 salt (value 8), Foresters 2 charcoal (value 4), Smiths 1 charcoal (value 2). Mire causeway complete (day 7, Foresters). Varn sea wall: Foresters 4 timber, 1 tools; still needs 2 tools, 2 cloth. Kell bridge, Ostra crane: nothing delivered. Open offers: none. Contracts and bonds: none. Missed reveals: 0 of 4 today (2 of 40 so far, 5.0%).

**Day-10 scores (works points only; mandates are scored at the bell), for metric 7:** Foresters 31, Saltworks 8, Smiths 6, Weavers 0.

### Referee word count, day 10

1560 words in this log's day-10 section, excluding the seats' verbatim replies.

---

## Day 11

**Drand round:** 33966310, randomness `ecb2950c9d9eb97dfb0c63bfb98f50fcc4db680b8ff2b8a5502f7679fe2251e7` (applied at resolution).

### Talk round: blocks sent

Each block: day-10 public results (G2; the Foresters' bandit loss and the Weavers' void orders are stated only in their own blocks), the public state at the start of day 11 with the completed Hath lodge, no open offers, no edict, the bell's remaining public window (days 11–13, which follows from day 10's close; nothing more), the seat's own day-10 results, position, works points and mandates, an empty inbox, and the talk form.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-11-talk-Saltworks.md` | `8e23526ee690c82e43fec48059c01fff66c0105eaec33191a9dea5a8a7587ad5` |
| Weavers | `to-seat/day-11-talk-Weavers.md` | `46e0268f2ce5128dd7246a35d7616d41f86b7394ba079693d71170bd6e848d4f` |
| Smiths | `to-seat/day-11-talk-Smiths.md` | `d5f21e36407eb8a064c0d15661d8cd3792956c54e5f562683da9ea09df97a56d` |
| Foresters | `to-seat/day-11-talk-Foresters.md` | `be6647b542db83d2f66b6d14f507fb8aa3d64d71f8f707160e3f93edd44a839e` |

### Talk replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. Carry notes from R&D: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page; all on time, no re-prompts. The Weavers' path check again flagged its own artifact folder, the same split-path artifact as on day 10; benign (Publisher confirmed).

#### Saltworks: `from-seat/day-11-talk-Saltworks.md`, `0e93c51248209d25b98486fea1363bc9da1e4d74b0318a1afe12415150a802a1`

```
NO MESSAGES
```

#### Weavers: `from-seat/day-11-talk-Weavers.md`, `c219f859ec82ed90fd9d57b685201caa608588e104395f1cede010016246d8e3`

```
MESSAGE 1 TO Saltworks: I have 1 cloth available; would you like to buy it for 20 coin or trade it for 1 tool to help my Ostra crane deliveries? Also, if you need wool or want to sell brine, let me know your terms.
MESSAGE 2 TO Smiths: I can offer my cloth for a tool (or 18 coin) to support your Kell bridge work. Could we set up an Offer or contract for Day 11? Let me know your preferred exchange.
MESSAGE 3 TO Foresters: Do you have any timber or charcoal for sale? I can pay market price (4‑5 coin each) or trade wool. Also, any tip on safe travel to Ostra would help.
```

#### Smiths: `from-seat/day-11-talk-Smiths.md`, `ddee53a1cde1535678c680470c7b5e18dbb8d265fe5becbd9593f4309a793719`

```
MESSAGE 1 TO Weavers: Rough flood on the short cut again! Yes, I'll happily pay MORE than 6 coin for your cloth today: in Day 11 Act, I will post 13 coin for 1 cloth, so please post 1 cloth for 13 coin today (even if your Day 11 note named 6 or 15, use 13 coin today so we definitely match!). If the bell reaches Day 12, I can buy a 2nd cloth for 12 coin too!
MESSAGE 2 TO Foresters: With the bell window open on Day 10 and Saltworks sitting on Hath lodge, I had to jump in and bank those points before a possible Day 10 bell! Heading back to Kell today. Since Hath lodge is finished, if you reach Hath and convert 2 charcoal today (Day 11), I'll have 12 coin ready on Day 12 to buy 2 charcoal (or 6 coin for 1 charcoal) if the game goes on!
MESSAGE 3 TO Saltworks: Good working alongside you at Hath lodge! I'm holding 1 spare timber right now if you decide to run to Ostra today and Varn tomorrow for one final salt batch (happy to cross 1 timber for 2 coin via Offers anytime!). Let me know if you want to set up any trade or quick contract over the final days!
```

#### Foresters: `from-seat/day-11-talk-Foresters.md`, `5ee81bd69e7f6dcb59910e0f488512aafbd9213a4e0f8525ab9f36905c765e55`

```
MESSAGE 1 TO Smiths: Nice move completing Hath lodge yesterday! I'm travelling Mire to Hath today (Day 11) and converting 2 charcoal tonight. Good luck banking your Kell bridge deliveries back in Kell today in case the closing bell rings tonight! If the bell holds off to Day 12, I'm 100% down to cross 2 charcoal for 1 tools (or 12 coin if your tools all go into the bridge today). Let's coordinate tomorrow!
MESSAGE 2 TO Weavers: Tough luck with that flood! I hold zero goods right now, so I have no salt or timber to sell. Urgent bell tip: the game could end tonight (Day 11)! If you travel Mire to Ostra today, convert your remaining wool and dye into cloth today, AND deliver both cloth to Ostra crane today, all three resolve in order today so you bank 6 points safely before tonight's bell!
MESSAGE 3 TO Saltworks: Looks like Smiths slipped in from Kell yesterday to drop the final charcoal on Hath lodge while I was delayed heading back south, though at least you still locked top contributor on the lodge with 8 points! I move Mire to Hath today (Day 11) to cut timber again. If the bell goes to Day 13 and you need 1 remote timber for another Varn salt boil, just let me know!
```

**Whispers check:** 9 messages (Saltworks `NO MESSAGES`; Weavers 3, Smiths 3, Foresters 3). All in the form, addressed to seated guilds, at most 80 words (longest 74). All delivered unchanged.

**Routing (act-round inboxes):**
- Saltworks: from Weavers (msg 1), from Smiths (msg 3), from Foresters (msg 3).
- Weavers: from Smiths (msg 1), from Foresters (msg 2).
- Smiths: from Weavers (msg 2), from Foresters (msg 1).
- Foresters: from Weavers (msg 3), from Smiths (msg 2).

**Rules claims seats made to each other (recorded, not corrected):**
- Foresters to Weavers: travel to Ostra, convert and deliver to the Ostra crane the same day, since "all three resolve in order today". Matches the day's order and G6, provided the caravan arrives.
- Foresters to Saltworks: "you still locked top contributor on the lodge with 8 points". Matches the public deliveries: Saltworks 8, Foresters 4, Smiths 2 is strictly highest (board.md rule 3).
- Foresters to Smiths: arriving in Hath and "converting 2 charcoal tonight". Matches (cut and convert at step 5 after movement, G1).
- Weavers to Foresters: timber or charcoal at "market price (4-5 coin each)". Hath sells timber from 2 a unit, stepping; no market sells charcoal. A misreading by the seat.

**Weavers contract-memory check (talk, day 11):** no live contracts. Nothing to check. It again offers "1 cloth", which matches its holding.

### Act round: blocks sent

Same layout and format conventions as day 1; caravan lines from the live state.

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | `to-seat/day-11-act-Saltworks.md` | `a907b10a38a3cd0715688d87531c201d4f3871e0e35f013a2281e7c01dfe5449` |
| Weavers | `to-seat/day-11-act-Weavers.md` | `dcad4862485c162c4d28bd0ea7508dc764fe79dd822db58ec05c9022f529810d` |
| Smiths | `to-seat/day-11-act-Smiths.md` | `220289a5888401c73f856f4f4a960acba93b442d0de29671190f997c1d96f32f` |
| Foresters | `to-seat/day-11-act-Foresters.md` | `786c08aeb5796bd9596e0e88769069bedb499ab31efc207814be79ca33aae683` |

### Act replies

Read from `from-seat/` (verbatim, by R&D). SHA-256 measured on disk; each matches R&D's list. All on time, no re-prompts (R&D). Carry notes: the Smiths text is from the AI Studio chat file; the Foresters text was transcribed from the Gemini page (its recipe arrow is the "→" character as the page rendered it).

#### Saltworks: `from-seat/day-11-act-Saltworks.md`, `4deeaf04dbe9091fed33e6646b42c1e1c9268eb254b0e0d95ac4ee17310b7d83`

```
TRAVEL: stay
```

#### Weavers: `from-seat/day-11-act-Weavers.md`, `5e466646828ed7d44437ab822e7f653872f0b64b29f25c731fb7456433b23256`

```
TRAVEL: go to Ostra
CONVERT: 2 wool + 1 dye x1
WORKS: deliver 2 cloth to Ostra crane
```

#### Smiths: `from-seat/day-11-act-Smiths.md`, `c24f5d6d93d9cd600b94c22ddeb61f88959a03f46650113dd2d1aadade9348c0`

```
OFFERS: post 13 coin for 1 cloth ; post 2 coin for 1 ore @Kell
WORKS: deliver 3 charcoal to Kell bridge ; deliver 1 tools to Kell bridge ; deliver 1 tools to Kell bridge
TRAVEL: go to Kell
```

#### Foresters: `from-seat/day-11-act-Foresters.md`, `9716572ec538e4401d09bf02964b08c5a91c3c050ee7642ff28a5803aed3d1d5`

```
CONVERT: 1 timber → 1 charcoal x2
TRAVEL: go to Hath
```

**Validity:** all four replies are in the form and carry a TRAVEL line. No missed reveals.

**Weavers contract-memory check (act, day 11):** no live contracts. Consistent.

### Resolution, day 11

**Edicts:** none in force.

**Step 1. Sealed travel orders:**

| Seat | Order | SHA-256 |
|---|---|---|
| Saltworks | `TRAVEL: stay` | `b9846de2f69191866fe79ddb2d42047fd7753b94b7a6148c6027550ec2e1c155` |
| Weavers | `TRAVEL: go to Ostra` | `e81dca8522bda28e703276369bdaab4c461138b5c641a68feca607d271d2cc03` |
| Smiths | `TRAVEL: go to Kell` | `28e6dbd7650139988924688c11f080c4a974c91fa37d47ca11b9b40f6a921956` |
| Foresters | `TRAVEL: go to Hath` | `d79aa6bf5a91011eb3080be4a78c936062d6a2c2ad3fb81ce06595af051beedf` |

Revealed: Weavers Mire → Ostra (road 5, short cut); Smiths Hath → Kell (road 2, hill); Foresters Mire → Hath (road 3, safe); Saltworks stays.

**Step 2. The day's roll:** quicknet round 33966310, randomness `ecb2950c9d9eb97dfb0c63bfb98f50fcc4db680b8ff2b8a5502f7679fe2251e7`.

**Step 3. Road events, then movement:**

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | ec b2 | 60594 | 15 | clear |
| 1 Ostra–Kell | safe | 95 0c | 38156 | 17 | clear |
| 2 Kell–Hath | hill | 9d 9e | 40350 | 11 | clear |
| 3 Hath–Mire | safe | b9 7d | 47485 | 6 | clear (safe road: 6 is clear) |
| 4 Mire–Varn | marsh | fb 0c | 64268 | 9 | clear |
| 5 Ostra–Mire | short cut | 63 bf | 25535 | 16 | clear |

All travellers arrive: Weavers in Ostra, Smiths in Kell, Foresters in Hath.

Caravans after moving: Saltworks Hath; Weavers Ostra; Smiths Kell; Foresters Hath.

**Step 4. Offers fill.** Day's order, lowest first: Foresters `448a326f…`, Weavers `4921103c…`, Smiths `97f68381…`, Saltworks `9f06f5d5…`.

| Offer | Result |
|---|---|
| Smiths: 13 coin for 1 cloth | no crossing offer (the Weavers posted none); open as **O11** at the bell |
| Smiths: 2 coin for 1 ore @Kell | caravan in Kell; ladder price 2; filled: −2 coin, +1 ore |

**Step 5. Conversions, then works deliveries** (G6).
- **Weavers in Ostra** (home): 2 wool + 1 dye → 1 cloth ×1. Wool 0; dye 0; cloth 1 + 1 = 2.
- **Foresters in Hath:** cut 2 timber (G1), then 1 timber → 1 charcoal ×2. Timber 0 + 2 − 2 = 0; charcoal 2.
- **Works, Weavers:** deliver 2 cloth to the Ostra crane, caravan in Ostra. The crane needs 2 cloth; both taken. 2 × 3 = **6 works points**. Not complete (still needs 3 tools, 3 salt).
- **Works, Smiths:** at the Kell bridge, caravan in Kell. 3 charcoal (needs 4; all 3 taken): 3 × 2 = 6. Then 1 tools and 1 tools (needs 2; both taken): 2 × 3 = 6. **12 works points.** Not complete (still needs 1 charcoal, 2 salt). Smiths' charcoal 0, tools 0.

**Step 6. Perishables:** none held.

**Step 7. Contracts at deadline:** none.

**Step 8. Ladders refill** (moot: the bell rings).

### State at the close of day 11

| Guild | Caravan | Purse | Goods | Standing | Works points |
|---|---|---|---|---|---|
| Saltworks | Hath | 30 | — | 0 | 8 |
| Weavers | Ostra | 6 | — | 0 | 6 |
| Smiths | Kell | 25 | 1 timber, 1 ore | 0 | 18 |
| Foresters | Hath | 5 | 2 charcoal | 0 | 31 |

Note on the Weavers: they held 1 cloth, wove 1 and delivered 2, so they hold **no goods** at the close.

Coin check: 68 after day 10; Kell's ladder took 2; 68 − 2 = 66 = 30 + 6 + 25 + 5. Checked.

Works at the bell:

| Works | Delivered (guild: goods, value) | Still needed | Status |
|---|---|---|---|
| Varn sea wall | Foresters: 4 timber, 1 tools (7) | 2 tools, 2 cloth | open |
| Kell bridge | Smiths: 3 charcoal, 2 tools (12) | 1 charcoal, 2 salt | open |
| Ostra crane | Weavers: 2 cloth (6) | 3 tools, 3 salt | open |
| Hath lodge | Saltworks: 4 salt (8); Foresters: 2 charcoal (4); Smiths: 1 charcoal (2) + bonus 4 | — | complete, day 10, Smiths |
| Mire causeway | Foresters: 4 timber, 2 tools, 2 dye (16) + bonus 4 | — | complete, day 7, Foresters |

28 of the 40 units were delivered.

## BELL: close of day 11

**The bell rings at the close of day 11.** Checked against setup.md's commitment:
- Bell round (state.json): 33966350, randomness `f9b03bc3d17d865b178f5a3c2ca1055980cf81e89b6282814cbc65995a6794d7`. Bytes 24–31 as a big-endian integer: n = 5529406151825265879; n mod 27 = **9**, which falls in 9–17 = **day 11** (board.md "The bell day"). The value is as recorded in state.json; the referee has no network and did not refetch it from drand.
- SHA-256(`11:6ae38bd5b90e7f576fb40ca91c54bbaa`) = `74c41dc09c3d0b630d69d5b95845f06494ef3791326816b988e1c2a446518270`. That matches setup.md's commitment. **Verified.**

### Mandates revealed (cards checked against setup.md's hashes before day 1)

| Guild | Mandate | Points | Met? | Evidence |
|---|---|---|---|---|
| Saltworks | Deliver 4 salt to works outside Varn | 4 | **yes** | 4 salt to the Hath lodge, day 9 |
| Saltworks | Be the largest contributor to the Hath lodge | 5 | **yes** | 8 against Foresters 4 and Smiths 2: strictly highest (board.md rule 3) |
| Weavers | Deliver 3 cloth to works | 6 | no | 2 cloth delivered (Ostra crane, day 11) |
| Weavers | Have 2 contracts end Kept | 4 | no | no contract was made at this table |
| Smiths | Install the bellows by the close of day 6 | 3 | **yes** | installed day 2 |
| Smiths | Complete the Kell bridge | 6 | no | the bridge still needs 1 charcoal and 2 salt |
| Foresters | Deliver charcoal or timber to works in 3 different towns | 4 | **yes** | Hath (charcoal, day 6), Mire (timber, day 7), Varn (timber, day 8) |
| Foresters | Be the largest contributor to the Varn sea wall | 6 | **yes** | only contributor (7) |

### Final scores

| Place | Guild | Works points | Mandate points | **Score** | Standing | Breaches |
|---|---|---|---|---|---|---|
| 1 | Foresters | 31 | 10 | **41** | 0 | 0 |
| 2 | Smiths | 18 | 3 | **21** | 0 | 0 |
| 3 | Saltworks | 8 | 9 | **17** | 0 | 0 |
| 4 | Weavers | 6 | 0 | **6** | 0 | 0 |

No ties. Works points: Foresters 2 charcoal (4, day 6) + Mire causeway 16 + bonus 4 (day 7) + 4 timber, 1 tools (7, day 8). Smiths: 1 charcoal + bonus at the lodge (6, day 10) + Kell bridge 12 (day 11). Saltworks: 4 salt (8, day 9). Weavers: 2 cloth (6, day 11). Median score: (21 + 17) / 2 = 19.

**The game is over.** No day-12 blocks are written.

### Referee word count, day 11

1745 words in this log's day-11 section (with the bell), excluding the seats' verbatim replies.
