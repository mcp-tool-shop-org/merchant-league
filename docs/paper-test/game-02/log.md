# Game 2 log (referee)

Referee: Claude subagent (Opus), brief `referee-brief-game-02.md` sha256 a0d0ec7106bf708c35ccba762451b41b5aac6b6fa557428e1c147e6d9015ed06 (checked before starting). Day-1 files written 2026-11-15 00:38 ET (from `date`).

## Pre-game checks

Read at merchant-league commit e74e246c17b17690563df7f6e0c6e280db30ee2a via `git show` (no checkout, no edit).

| File | SHA-256 measured | Pin in setup.md | Match |
|---|---|---|---|
| docs/design.md | b1f22ec20dea2a7653bbf29757bd768b64cad9e9ba88df13c733a2178ba9ddcf | b1f22ec2… | yes |
| docs/paper-test/board.md | 48eb3658a8f99d0219d96794cb12b17617efeab32a9c20d38d54d504db36c03c | 48eb3658… | yes |
| docs/paper-test/brief.md | b29679867f4bd0a9c0e294bdff706d2bc1d8aa7e877c10c570099df885f0cded | b2967986… | yes |
| docs/paper-test/game-02/setup.md | f909ad67279edf0ee2072e6168e843cfa8da4c66b0d5c9e5d08f34efb2f29aa4 | (setup.md does not pin itself) | n/a |
| docs/paper-test.md | 144a7d48962273cea0b963b02db7950b7a4914208a0bd905ebf412ac92cc88c5 | (not pinned in setup.md) | n/a |
| cards/Saltworks.md | 9945d40f503fecb80e184faa3abfe834e47cdf67a6493837839a3e7979975b5c | same | yes |
| cards/Weavers.md | 07f4758d4c20c543014c018e5830434f370b241508c0a63ae709ce4288a3525c | same | yes |
| cards/Smiths.md | eb0c13e31298dacd6cb24760f63fccf64eb988f382be972ba49b46288b59f104 | same | yes |
| cards/Dyers.md | ff63eebd5e957484fca6aabf317881e3133e5a7f8f705da1837192af4d8d7dd2 | same | yes |
| cards/Tanners.md | 091b6212a996b87efb30dc388b993d85fff6fe1becd29baa531fc42b41bdb5f1 | same | yes |

- state.json read (sha256 c1fa1c96cdaf4359d898ef596c49f7cd20556a2c489f812e8e4cf6d538784517). Its day rounds and values match setup.md's "Rounds" table; its bell commitment equals setup.md's.
- The bell commitment was checked privately against state.json's day and salt: it matches. The day is held, not written here, until the bell.
- **Referee read outside the brief's list (disclosed):** at start I also read R&D's seating note (kept outside this repository) in the same command as the brief. It holds seat surfaces and models only, no cards or orders, and nothing from it goes into any seat block.

## Seating (from R&D's instruction)

Saltworks (Gemini, AI Studio), Weavers (Gemini, Gemini app), Smiths (Claude Sonnet 5.5 subagent), Dyers (Claude Sonnet 5.5 subagent), Tanners (GPT-OSS 120B, Antigravity). Foresters sit out. The Tanners seat gets the GPT-OSS memory check (method in metrics.md).

Relay rule noted (paper-test.md "The turn protocol", the delivery-failure exception): R&D may make one neutral re-send on a delivery failure and will say so; such cases are logged here as harness. The referee never asks a seat again.

## Starting state (board.md "Guilds and converters", "The start")

Every guild: 60 coin, no goods, standing 0, caravan at home (Saltworks Varn, Weavers Ostra, Smiths Kell, Dyers Mire, Tanners Hath). No bellows installed. Ladders as in setup.md "Markets", full stock. All works empty.

---

## Day 1

Drand round 33966360, randomness abdfcdb339fa528b99b2823f226547a3f55d4a27f596dcd3983202b03879d9de (setup.md). Applied at resolution.

Edict: none on day 1; none to announce (day 4's edict is announced in day 3's public state, setup.md "Edicts").

### Talk round, day 1

Blocks sent (to-seat/):

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-01-talk-Saltworks.md | 81b289087775e66afc895ec340aa69c41e53d3dbda40508a018f69d6247fd74a |
| Weavers | to-seat/day-01-talk-Weavers.md | 394be4abb584bb868de21055a60e83e7b6486e0962b7cd8ea04c916805e0d91d |
| Smiths | to-seat/day-01-talk-Smiths.md | 121a2bb1f63f7f6a34b0b30bcd172287400fb7001fe262f9af0fbf9eef99ce6a |
| Dyers | to-seat/day-01-talk-Dyers.md | 72525bf3f81e262b595c63bf637a5365ed3216c7fe185e67e60cf1ff55268628 |
| Tanners | to-seat/day-01-talk-Tanners.md | c661fc65cde6d2241314ccaac0207a48d61e8b90371843c78c3df612c65a8330 |

Each block: brief.md v1 verbatim, the seat's own card verbatim, the public state, the seat's position, and the talk instruction with the exact reply form. Isolation checked: each seat's mandate text appears only in its own file (grep).

Procedure stated in the blocks:
- A talk message over 80 words, or addressed to an unseated guild, is not delivered. The brief caps messages at 80 words but does not say what happens to one over it: rule gap G2-1 (metrics.md), decided provisionally this way and told to every seat in advance.
- Act lines sent in a talk reply are ignored (the protocol keeps the rounds separate).

### Talk replies, day 1 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-01-talk-Saltworks.md | a50000cacc9ae8f6671b456effad5928b18c756cfb9ec444daca87c7bdd7545b | valid; 3 messages (68, 66, 68 words) |
| Weavers | day-01-talk-Weavers.md | b531161625b7009b545c5c5e8bc857b6b1353518f30477e47fc86a48f6cedb52 | valid; 3 messages (64, 63, 60 words) |
| Smiths | day-01-talk-Smiths.md | 6bed5cba49ea75fdf5e419c25a722bbe6c39e5760b8177760f06ebcb51ff8398 | valid; 3 messages (44, 42, 42 words) |
| Dyers | day-01-talk-Dyers.md | df1d0a0860c66513856fd984f31fb3693ba6014d6089ebd81d3c26f6940b0ad3 | valid; 3 messages (61, 54, 44 words) |
| Tanners | day-01-talk-Tanners.RAW-copy-of-prompt.md | 5a9f7e98992c707467e1a7e8f8c77e3f400cde03ee2933c3e9615123aeeaad5f | **NO REPLY (wrong form)**, per R&D; not re-sent |

**Tanners.** The file is a copy of the incoming message, not a reply. Compared with to-seat/day-01-talk-Tanners.md, it has R&D's 6-line seat wrapper prepended, `post` dropped from the brief's OFFERS form line, one blank line dropped, and no final newline. It contains no `TALK day 1, Tanners` header and no `TO` lines. Ruling: wrong form, so the Tanners sent no messages on day 1 (paper-test.md "The turn protocol": a wrong-form reply is not a delivery failure, and no re-send follows). R&D marks it as **likely harness-caused**: the day-1 file joined R&D's wrapper and the referee's block in one file, and the seat's transcript says it meant to write the contents verbatim. It is logged in metrics.md under harness, counted apart. It is a talk round, so no caravan or toll is affected. R&D adds a neutral clause to the Tanners' relay wrapper from day-1 act on; the referee's block stays the same for every seat.

Raw replies, verbatim:

```
TALK day 1, Saltworks
TO Tanners: Greetings Tanners! With Foresters out, Hath holds all timber and we make all salt, so let's partner up. If you buy 2 or 3 timber at Hath today (or day 2), we will buy them at 6 coin each to brew salt on day 3, and sell you 2 salt at 8 coin each right away. Grabbing timber today and hide tomorrow also avoids hide spoilage. Sound good?
TO Dyers: Greetings Dyers! Let's build a steady salt partnership so your dye flows fast. We plan to brew salt starting day 3 and gladly offer you 2 salt at 8 coin each from our opening batches, via offer or contract. Keep in mind day 1 madder spoils end of day 4! Also, if you visit Hath early, we'll buy 1 or 2 timber at 6 coin each.
TO Weavers: Greetings neighbor! Friendly heads-up: we are visiting Ostra solely for brine and will leave all wool untouched for you. We are also offering Dyers cheap salt (8 coin) so dye reaches you quickly. Once your looms are running, we would love to buy 2 cloth at a generous price, or trade salt plus coin so Ostra lighthouse and Varn salt barns both get supplied. How does that sound?
```

```
TALK day 1, Weavers
TO Saltworks: Let's build a steady brine-for-salt pipeline. Unless your Day 1 message to me says you're going to Ostra today, stay safe in Varn: I'll buy all 3 brine at Ostra today (9 coin) and sell them to you tomorrow for 10 coin. After that, I'll keep buying Ostra brine to barter 3 brine for 1 salt via OFFERS, unsnipeable and saving you travel. Deal?
TO Dyers: I'd like to buy up to 5 dye from you this game. Since I'm in Ostra trading brine to Saltworks for salt, I can either pay strong coin (9-11 coin per dye) or supply you salt plus coin directly for dye once salt flows around Day 3. Let me know tomorrow whether you prefer coin or salt-for-dye swaps so we can coordinate early!
TO Tanners: Hope you're grabbing Hath timber today so Saltworks can start converting salt on Day 2, since everyone needs salt flowing early! I'm coordinating Ostra brine with Saltworks, and once you're making leather in Hath, I'd happily buy 1-2 leather from you via OFFERS for strong coin, saving you the long trek to Ostra. Let me know tomorrow if you're interested!
```

```
TALK day 1, Smiths
TO Tanners: I need 1 leather by day 4 for my bellows. I'll pay 9 coin on delivery, and I can send you hide from Kell's market if you want it. Later I'll pay well for more leather too. Can you take it? Name your price.
TO Saltworks: Tanners and Dyers both need salt, and I'm buying none. Sell it to them early and often. I'll make tools for the Mire bridge and Hath tower, where your goods and my tools both count. Want to agree on a standing trade?
TO Dyers: Mire bridge needs 2 tools. I plan to bring tools there around days 4 to 6, and I'll sell you tools at 4 coin each if you'll buy. Do you want any dye contract for Hath watchtower? Tell me what you plan.
```

```
TALK day 1, Dyers
TO Saltworks: Dyers here. I need salt for dye (1 salt per dye) and will buy steadily. Proposal: contract for 4 salt delivered at Mire by day 5, 6 coin each (24 total), bonds both sides. Dye then feeds Weavers' cloth, which you need for Varn salt barns. Reply with your price and timing. A first contract also builds standing for us both.
TO Weavers: Dyers here. You need 1 dye per cloth. I can make up to 2 dye a day once I have salt. Interested in a contract: 2 dye delivered by day 5 for 14 coin? Tell me where you want delivery and what price works. Madder is cheap in Mire, so I can supply reliably.
TO Smiths: Dyers here. The Mire bridge needs 2 tools, and you make tools. Would you sell 2 tools delivered to Mire by day 6, around 6 coin each? In return I will buy your leftover goods or pay on delivery. Let me know your terms.
```

**Delivery (rule gap G2-1's test):** all 12 messages are at most 80 words and addressed to seated guilds, so all are delivered in the act blocks. Inboxes: Saltworks 3 (Weavers, Smiths, Dyers); Weavers 2 (Saltworks, Dyers); Smiths 1 (Dyers); Dyers 3 (Saltworks, Weavers, Smiths); Tanners 3 (Saltworks, Weavers, Smiths).

Messages are delivered verbatim. Factual claims inside them are the senders' own; the referee doesn't correct them (rule: no comment on play). For the record, against the board: Saltworks' "day 1 madder spoils end of day 4" disagrees with board.md (a unit bought on day 1 is gone at the close of day 3), and Weavers' "Saltworks can start converting salt on Day 2" needs timber in Varn by day 2. Not corrected in any block.

### Act round, day 1: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-01-act-Saltworks.md | 1b811a50d71a16d9b4eea50785a3e4debad11b8136145d6cae3c312659952545 |
| Weavers | to-seat/day-01-act-Weavers.md | 6f9ccea0ab5e46e3b8e40f8b7ddc007b6fed8a76fdb39b37e78a55668e78c4b2 |
| Smiths | to-seat/day-01-act-Smiths.md | 77255aa6c24408e2c03a4646a057b04f17d6e2502354c52776ecc210a941a256 |
| Dyers | to-seat/day-01-act-Dyers.md | 132408686bd9fcf0bc974de5b89d6bcc254d37b5fe70ca435aae798148cd32e7 |
| Tanners | to-seat/day-01-act-Tanners.md | 5f82ad6697c7c94b82789306c638fc06a8dd97d77052739ea2f699c61be6eb02 |

Each act block: public state (unchanged), own position, inbox verbatim, how act lines are read (identical text for every seat), and the act form exactly as in brief.md, headed `ACT day 1, <Guild>`.

Rulings stated in the act blocks before any order, given to every seat alike:
- **G2-2: contract timing.** A contract proposed today gets an id in tomorrow's public state, and the named guild may accept it from its next act round. Reason: act replies are simultaneous and sealed, so a guild can't accept a proposal it hasn't seen.
- **G2-3: market fills.** The board gives stepped prices and the brief gives `post <coin> for <n good> @<town>`, but neither says what happens when the posted coin differs from the ladder's price, or when the stock runs short. Ruling: a buy fills whole or not at all, if the posted coin covers the stepped price of those units after earlier buyers; the buyer pays the ladder's prices. A sell fills if the ask is no more than the ladder's price, within the cap of 3 a day, at the ladder's price. Order at one market: higher coin per unit first, then the day's roll (board.md "Who fills first").
- Several items of one kind may share a line, separated by `;`.

### Act replies, day 1 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-01-act-Saltworks.md | d17cf74393e1dac54928158a2d727b921caf1b54dd8d378dcc41215a41fa0afd | valid |
| Weavers | day-01-act-Weavers.md | bae2d46703345f891aebcd5d94b56b9eaed66c27ecb91d194832e7b293fbbf5e | valid; on time per R&D's harness ruling (below) |
| Smiths | day-01-act-Smiths.md | 6824e21972f69adb65e8d183169ec97e4c47cf488bc8b42c5bfee8dfb9a702b2 | valid |
| Dyers | day-01-act-Dyers.md | 7386c64377e85eafdc0992d633f7c5d695d1900e4aa2401b8477d0d9279866d3 | valid |
| Tanners | day-01-act-Tanners.md | 4588769976e670da3459524af27c7f19bd7e8d52d1b521cfbdd2d7e856004e08 | valid (relay wrapper carried R&D's neutral clause; see metrics.md) |

**Weavers harness (R&D's report, recorded as given).** No reply 15 min after delivery (about 00:55), a delivery failure, so R&D sent ONE re-send of the identical act block at 01:11. That turn also rendered nothing. The Director stopped it in the tab and sent "continue" (no game content), and the reply came at about 01:19. R&D's ruling: on time, harness-caused, one delivery failure with one re-send plus a content-free "continue", flagged for the games 1–2 review. Fact for that review: paper-test.md allows one re-send and says anything after it is a missed reveal; the "continue" was a second content-free prompt after the re-send. The referee applies R&D's ruling (the relay procedure is R&D's) and flags the point. **Setting change:** from 01:17 the Weavers seat ran on Carbon Medium (earlier Carbon High), by the Director's instruction. This reply ran on Medium. Metrics split the Weavers seat at 01:17 (Publisher's request).

Raw replies, verbatim:

```
ACT day 1, Saltworks
OFFERS: post 15 coin for 3 brine @Ostra; post 12 coin for 2 timber; post 6 coin for 1 timber
TRAVEL: go to Ostra
```
```
ACT day 1, Weavers
OFFERS:    post 9 coin for 3 wool @Ostra
TRAVEL:    stay
```
```
ACT day 1, Smiths
OFFERS:    post 12 coin for 3 ore @Kell; post 2 coin for 1 hide @Kell
TRAVEL:    stay
```
```
ACT day 1, Dyers
CONTRACTS: propose 4 salt at Mire for 28 coin by day 6 to Saltworks
TRAVEL:    stay
```
```
ACT day 1, Tanners
OFFERS:    post 2 hide for 12 coin
TRAVEL:    go to Kell
```

### Resolution, day 1 (paper-test.md "The turn protocol", steps 1–8; design.md "A market day")

**Step 1: sealed travel orders, hashed, then revealed.** SHA-256 of the UTF-8 text `game 2, day 1, <Guild>, TRAVEL: <order>` (guild and day are included so identical orders hash differently):

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | go to Ostra | 7a4fc14957ecbc932eb4289077d81b5c7986211ba519d86ba31c4e9c93c32c40 |
| Weavers | stay | b2780198143fe0085477bc25b05cac6e2e241948c3de66bdb83ccd55c7c7833b |
| Smiths | stay | 49b7539fd803ef0b3608e63c5b64c4e2d913e4b4d7ca37423e110bc86e75e8a0 |
| Dyers | stay | 85d09a64fcadb778ab354fb052309517135e7ad621fbba053e28a25e840fdcb8 |
| Tanners | go to Kell | e01867198e086e65f68728007deba424a69d3b8d7ca689f01f948d1732e96b42 |

All orders are legal: Varn–Ostra is road 0, Kell–Hath is road 2 (board.md "Towns and roads").

**Step 2: the roll.** Round 33966360, randomness abdfcdb3…79d9de. Road number = (byte 2r × 256 + byte 2r+1) mod 20 + 1 (board.md "Reading dice"):

| Road | Kind | Bytes | Number | Event (board.md "Road events") |
|---|---|---|---|---|
| 0 Varn–Ostra | safe | ab df | 20 | fair |
| 1 Ostra–Kell | safe | cd b3 | 20 | fair (nobody travelled) |
| 2 Kell–Hath | hill | 39 fa | 3 | toll 2 |
| 3 Hath–Mire | safe | 52 8b | 12 | clear |
| 4 Mire–Varn | marsh | 99 b2 | 7 | clear |
| 5 Ostra–Mire | short cut | 82 3f | 4 | bandits (nobody travelled) |

(Check: 0xabdf = 43,999, mod 20 = 19, so 20. 0x39fa = 14,842, mod 20 = 2, so 3.)

Fill order for equal prices, by SHA-256(`<randomness>:<guild>`), lowest first: Tanners, Smiths, Dyers, Weavers, Saltworks. Not needed today (no contested fill).

**Step 3: road events, then moves.**
- Saltworks, road 0, **fair**: moves to Ostra. A fair means "the town the caravan reaches sells at base price all day, without stepping, to everyone" (board.md), so Ostra's ladder sells at base price (brine 1, wool 1) to every buyer all day 1. Reading: a fair takes effect only in a town that a caravan reaches by that road. Road 1's fair would have reached Ostra or Kell, but nobody travelled road 1, so it has no effect (rule gap G2-4).
- Tanners, road 2, **toll 2**: pays 2 coin (standing 0 < 5, full toll). 60 → 58. Moves to Kell.
- Weavers, Smiths and Dyers stay; no road event.

**Step 4: offers fill** (by price, then the roll; G2-3 for market orders; board.md rule 1, caravan in town).
- Saltworks `post 15 coin for 3 brine @Ostra`: in Ostra after the move. Fair, so the cost is 1 + 1 + 1 = 3 ≤ 15. **Fills:** +3 brine, −3 coin (60 → 57). Ostra brine left today: 0.
- Weavers `post 9 coin for 3 wool @Ostra`: in Ostra. Fair, so the cost is 3 ≤ 9. **Fills:** +3 wool, −3 coin (60 → 57). Ostra wool left: 0.
- Smiths `post 12 coin for 3 ore @Kell`: in Kell. Stepped 2 + 4 + 6 = 12 ≤ 12. **Fills:** +3 ore, −12 (60 → 48).
- Smiths `post 2 coin for 1 hide @Kell`: first hide of the day costs 2 ≤ 2. **Fills:** +1 hide (bought day 1), −2 (48 → 46). Nobody else bought hide at Kell.
- Saltworks `post 12 coin for 2 timber` and `post 6 coin for 1 timber`: guild offers (no `@`). No crossing offer and no guild holds timber. **Open** as O2-1 and O2-2.
- Tanners `post 2 hide for 12 coin`: guild offer, but the Tanners hold no hide. It can't fill (an offer fills only from what its poster holds at the fill). **Open** as O2-3, shown in the public state (rule gap G2-5: offers whose poster can't cover them). It doesn't cross the Smiths' `@Kell` buy, which was addressed to Kell's market.

**Contracts.** Dyers `propose 4 salt at Mire for 28 coin by day 6 to Saltworks`: well formed (good, quantity, town, price, deadline; board.md rule 5). Recorded as **C2-1**: seller Saltworks, buyer Dyers, 4 salt at Mire, 28 coin, deadline day 6. Proposed, not accepted (G2-2), no bonds yet. Bonds once accepted: 28 coin each (standing < 3; board.md rule 4).

**Step 5: conversions and works.** None ordered.

**Step 6: perishables.** Smiths' 1 hide, bought day 1, is gone at the close of day 3 (board.md "Perishables"). Nothing spoils today.

**Step 7: contract deadlines.** None.

**Step 8: ladders refill** to full stock for day 2 (board.md "Stepped prices"; the fair ends with day 1).

### State after day 1

| Guild | Town | Coin | Goods | Standing |
|---|---|---|---|---|
| Saltworks | Ostra | 57 | 3 brine | 0 |
| Weavers | Ostra | 57 | 3 wool | 0 |
| Smiths | Kell | 46 | 3 ore, 1 hide (gone at the close of day 3) | 0 |
| Dyers | Mire | 60 | — | 0 |
| Tanners | Kell | 58 | — | 0 |

Coin check: 300 at start − 3 − 3 − 12 − 2 (ladders) − 2 (toll) = 278 = 57 + 57 + 46 + 60 + 58. Open offers: O2-1, O2-2 (Saltworks), O2-3 (Tanners). Contracts: C2-1 proposed. Works: all empty. Score: all 0.

Referee word count, day 1: 11,685 (talk blocks) + 3,727 (act blocks) = 15,412 words sent to seats.

---

## Day 2

Drand round 33966365. Edict: none on day 2; none to announce yet (day 4's is announced in day 3's state).

### Talk round, day 2: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-02-talk-Saltworks.md | 8d46fd6ee520e5e4195f76b0746f985e2641a68d027e661516660e3dca3fda16 |
| Weavers | to-seat/day-02-talk-Weavers.md | dc43d892e8436a8f9d0893af94728262643c21ecd35be14eea7887d04ef79ecf |
| Smiths | to-seat/day-02-talk-Smiths.md | 93c78c0fd896d16a3aad24aa54155f1be7b2750618c7e50eca54910b77c0d70b |
| Dyers | to-seat/day-02-talk-Dyers.md | ea15e70fba1558e1b4264b2e8c646c6cdce584af00498382982a8f7fb8f297f9 |
| Tanners | to-seat/day-02-talk-Tanners.md | 55a3802e1b3b23eab7b47b04b8b1be9fe31f2232a7072c09038704275e3fa413 |

Public state in each block: day 1's road results, moves and market fills (all public on the ledger: rolls from the beacon, fills from the open book), caravans, ladders, open offers, C2-1 as proposed, works, standings, edict. Private: each guild's own purse, goods and offers/contracts. Talk-round inbox: none (talk messages are delivered in act rounds).

### Talk replies, day 2 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-02-talk-Saltworks.md | 069337ba13eac0d791a65b59ccf5e5eb0cc9f067366b7bc6537fe344950353a2 | valid; 3 messages (69, 72, 70 words) |
| Weavers | day-02-talk-Weavers.md | e55c5752fbee23c1780048575e1019f2f0604956305a845d75ac03a569bf7702 | valid; 3 messages (67, 65, 67); Carbon Medium |
| Smiths | day-02-talk-Smiths.md | 725c5bd7470f58cbb5ea3f2ea474f54a643c1f9596eb10595b01b83b04007c10 | valid; 3 messages (51, 47, 46) |
| Dyers | day-02-talk-Dyers.md | 09670aa0c13de0aa24f48548b71cee3178b8157f2fc3850f36f3bc9074c07ace | valid; 3 messages (63, 56, 51) |
| Tanners | day-02-talk-Tanners.md | d111ca93510d33e227f62bd6462fb7c65f8086639e9e2805d930833cd0b10a07 | valid; 2 messages (24, 25); relayed after one harness re-send (below) |

**Tanners harness (R&D's report).** The first turn put its answer in the chat and wrote no file: a delivery failure. R&D sent ONE re-send of the identical instruction line, and the re-sent turn wrote out/day-02-r1.md at 01:26. That file's text differs from the chat-only first answer; the file is the reply relayed and is the one recorded. Logged in metrics.md.

Raw replies, verbatim:

```
TALK day 2, Saltworks
TO Weavers: Thanks for leaving brine yesterday, what a lucky Fair! Since nobody bought Hath timber yet, I am staying in Ostra today for day 2's 3 brine (all wool is yours), then moving to Varn on day 3. If you buy Ostra's 3 brine on day 3, I will buy them on day 4 for 12 coin (easy 3 coin profit for you) so I can keep brewing salt nonstop!
TO Smiths: Gladly! If you head to Hath today for charcoal, grab all 3 timber there too (12 coin market cost) and fill our open offers O2-1 and O2-2 tomorrow for 18 coin, an easy 6 coin profit on a trip you are already making! That lets us brew salt on day 3 so Dyers and Tanners get supplied fast. Once tools and timber are moving, let's also run a contract for +1 standing!
TO Dyers: Thanks for C2-1! Nobody bought timber day 1, so we cannot sign before timber is secured. Also, "at Varn" only needs OUR caravan in Varn when paying, so you still receive salt right in Mire! Once timber lands, let's do 2 salt at Varn for 16 coin (8 each), then more right after. If you visit Hath today, fill O2-2 or O2-1 tomorrow so we brew salt on day 3!
```
```
TALK day 2, Weavers
TO Saltworks: Congrats on the Day 1 fair! I'm staying in Ostra for wool only today, so all 3 brine are yours if you stay. Once you return to Varn, I'll happily buy Ostra brine and trade 3 brine for 1 salt (or coin) via OFFERS so you don't need extra trips. Let's also swap cloth and salt soon so we supply both Ostra lighthouse and Varn salt barns!
TO Dyers: Deal on 2 dye for 14 coin! In Act today I'll propose: 2 dye at Mire for 14 coin by day 6 (delivering at Mire saves you road risk, and I'll pay as soon as you convert). If C2-1 ties up your coin for bonds, tell me tomorrow and we can swap 2 dye for 14 coin via OFFERS instead, plus 2-3 more dye after!
TO Smiths: Greetings from Ostra! Since our guilds don't compete on any works, I'd love to cooperate. Quick heads-up if you're heading to Hath today for charcoal: nobody is in Hath, so buying 3 timber for 12 coin lets you fill Saltworks's open offers (O2-1 and O2-2) for 18 coin, netting 6 coin profit while unlocking salt early! Let me know tomorrow if you'd like to trade goods later.
```
```
TALK day 2, Smiths
TO Tanners: You're in Kell with me. I hold a spare hide and will sell it to you for 2 coin. Hide spoils fast, so use it soon. I want 1 leather by day 4 and will pay 9 coin on delivery. Do you have salt? If not, Saltworks has some soon. Deal?
TO Saltworks: Tanners need 1 salt for each leather and Dyers need salt too. I will buy 1 salt from you at 3 coin if you can bring one to Kell, and I'll pass it to Tanners. Otherwise sell salt to Tanners directly. Do you plan to reach Kell?
TO Dyers: I'm making tools, and charcoal is the bottleneck. Mire bridge wants 2 tools. I'll sell you 2 tools at 6 coin each, delivered Mire by day 7, but I need leather first for my bellows. Do you have any use for ore or timber? Tell me.
```
```
TALK day 2, Dyers
TO Saltworks: Please accept C2-1: 4 salt at Mire, 28 coin, deadline day 6. That is 7 each, close to your 8 offer, and I bond 28 myself. Salt is the bottleneck for dye, which Weavers want, so every salt you sell me moves cloth and your own Varn barns. If you prefer, I will pay 8 each for 3 salt instead. Tell me which.
TO Weavers: Yes, I want dye sold for coin, 10 each, for up to 5 over the game. First dye comes once Saltworks delivers salt, around day 4 to 6. Let's contract 2 dye delivered at Ostra by day 7 for 20 coin. Press Saltworks to sell me salt; more salt for me means dye for you sooner.
TO Smiths: Thanks. I'll buy 2 tools at 4 coin each delivered to Mire, which you said you'd bring around days 4 to 6. Contract: 2 tools at Mire for 8 coin, deadline day 6? Dye for the Hath watchtower comes later, after Mire bridge and Weavers; I can't promise a date yet.
```
```
TALK day 2, Tanners
TO Smiths: Could you sell me hide? I need 2 units to start leather production. I'm at Kell and can pay up to 6 coin total.
TO Saltworks: Do you have any salt to sell or any trade for timber? I can buy salt at a fair price or sell you timber later.
```

**Delivery:** all 14 messages are at most 80 words and addressed to seated guilds (G2-1), so all are delivered. Inboxes: Saltworks 4 (Weavers, Smiths, Dyers, Tanners); Weavers 2 (Saltworks, Dyers); Smiths 4 (Saltworks, Weavers, Dyers, Tanners); Dyers 3 (Saltworks, Weavers, Smiths); Tanners 1 (Smiths).

**Checks this round:**
- **GPT-OSS memory (Tanners):** no live contracts; nothing to check. Its O2-3 (2 hide for 12 coin) is unmentioned, which is not a fault (no contract, no step due).
- **Foreknowledge (new check, started on R&D's instruction, pending the Publisher's approval):** does any reply show knowledge of a road event or edict before it is announced? Day-2 talk: none. The only road event mentioned is day 1's Varn–Ostra fair (Saltworks, Weavers), which was published in day 2's state. No reply mentions the Ostra–Kell closure or the Hath festival.
- For the record (not corrected, no comment to seats): Saltworks' line to the Dyers reads C2-1 as "at Varn", but C2-1 names Mire; board.md rule 1 and the bond rule's "Delivered" need the seller's caravan in the named town. Smiths say "I hold a spare hide"; they hold 1 hide, gone at the close of day 3.

### Act round, day 2: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-02-act-Saltworks.md | fbf274e9fb2d127091aed07adac43c1b8ff5a3279190a81d3ece463507234e30 |
| Weavers | to-seat/day-02-act-Weavers.md | 57d1a73401cd5c7f88d52964b56a3ae5bab732773dcb114b88b65055e23e4fba |
| Smiths | to-seat/day-02-act-Smiths.md | 2f76ff112ed42a3fac03212125f57b880e45d3eaeaa11ffc3ae82a02b69efc67 |
| Dyers | to-seat/day-02-act-Dyers.md | 3e55aeb7d96a9e594944bcefa6f42a65a4502c0d931da5f3b7184293caa97782 |
| Tanners | to-seat/day-02-act-Tanners.md | d3794025ba0dfdf66cd6127baa59e27e68e6266e6457cc572cfdd290cc3fcdff |

Each block: the day-2 public state (unchanged from the talk block), its own position, its inbox verbatim, the same how-act-lines-are-read text as day 1 (plus the G2-5 sentence "An offer fills only from what its poster holds at the fill"), and the act form.

### Act replies, day 2 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-02-act-Saltworks.md | aa02841c946ce343fcdb943e6725247c7dc4981eaa659783f3c5160a969e5727 | valid |
| Weavers | day-02-act-Weavers.md | 479d528141a4b0e5925f0befa7a0961059763fd04655908982e9d54071f110b1 | valid; Carbon Medium |
| Smiths | day-02-act-Smiths.md | 0bda1b051b88974ab22298bf02eddcbaa1a64d22298b0f1d0c1b7d0b913fd516 | valid |
| Dyers | day-02-act-Dyers.md | 76f2b9bc7858337f3546f2c71e0916108040e62197ecac1f8b44828aa5a387ec | valid |
| Tanners | day-02-act-Tanners.md | e250774570d48bd29c0f06ca6ee9984ee862c00e74fc50b0cc1e87c711cf2656 | valid; relayed after one harness re-send (below) |

**Tanners harness (R&D's report).** The first turn answered in the chat after about 1 s, with no file and no DONE line: a delivery failure. ONE re-send of the identical line; the re-sent turn wrote out/day-02-r2.md at 01:37, the reply relayed (its text differs from the chat-only first answer). R&D's path check of the 01:37 turn: only brain/…/out paths and reasoning-text fragments, 0 command steps. Running count: 3 of the Tanners' 4 turns in game 2 needed harness help.

Raw replies, verbatim:

```
ACT day 2, Saltworks
OFFERS: post 15 coin for 3 brine @Ostra
TRAVEL: stay
```
```
ACT day 2, Weavers
OFFERS:    post 9 coin for 3 wool @Ostra; post 20 coin for 2 dye; post 10 coin for 1 dye
TRAVEL:    stay
```
```
ACT day 2, Smiths
OFFERS:    post 1 hide for 3 coin; post 12 coin for 2 charcoal @Hath
TRAVEL:    go to Hath
```
```
ACT day 2, Dyers
OFFERS:    post 6 coin for 2 timber @Hath; post 2 timber for 12 coin
TRAVEL:    go to Hath
```
```
ACT day 2, Tanners
OFFERS:    post 2 coin for 1 hide
CONTRACTS: propose 1 leather at Kell for 9 coin by day 4 to Smiths
TRAVEL:    stay
```

### Resolution, day 2

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 2, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | stay | ffc12a70f6e7a2e0ffaab4e7fe316c441f22491c8557aa575743f9491c9d87e5 |
| Weavers | stay | 8fdc455185963223d4fd4c6ca524c19dbb713438a22f09541ea5af22658753d8 |
| Smiths | go to Hath | f5dc9ec27cb71ac3ccaedeb1279dfefbb4f3df159aa2d3f666b9b1ebd4a53810 |
| Dyers | go to Hath | 21fbb60ae9299583d7ff3eefa5b29f2f0c34cf75a626a8ec0da7c4d325bf854b |
| Tanners | stay | c4edf0c20f87a4427921715b51ec4c52542e58d0da99d4d2954d256284eeffa4 |

Legal: Kell–Hath is road 2, Mire–Hath is road 3.

**Step 2: the roll.** Round 33966365, randomness c710508e…d3118776b.

| Road | Kind | Bytes | Number | Event |
|---|---|---|---|---|
| 0 Varn–Ostra | safe | c7 10 | 1 | bandits (nobody travelled) |
| 1 Ostra–Kell | safe | 50 8e | 3 | clear |
| 2 Kell–Hath | hill | 45 62 | 3 | toll 2 |
| 3 Hath–Mire | safe | 52 3f | 16 | clear |
| 4 Mire–Varn | marsh | 0b 38 | 13 | clear |
| 5 Ostra–Mire | short cut | ef b1 | 2 | bandits (nobody travelled) |

(Checks: 0xc710 = 50,960, mod 20 = 0, so 1. 0x4562 = 17,762, mod 20 = 2, so 3. 0x523f = 21,055, mod 20 = 15, so 16.)
Equal-price order: Tanners, Saltworks, Dyers, Weavers, Smiths.

**Step 3: moves.** Smiths, road 2, toll 2 (standing 0, full): 46 → 44; to Hath. Dyers, road 3, clear: to Hath. Others stay.

**Step 4: offers fill.** Ruling G2-6 (new): within step 4, market (ladder) orders fill first, then guild-to-guild crossings, older offers before newer at equal price. Reason: the ladders' stock is posted at the day's open, and a guild offer can only fill from what its poster holds (G2-5), so goods bought from a ladder that day can be passed on the same day, as sequential ledger trades would allow.
- *Market orders:*
  - Saltworks `15 coin for 3 brine @Ostra`: in Ostra. 1 + 3 + 5 = 9 ≤ 15. Fills: +3 brine, 57 → 48.
  - Weavers `9 coin for 3 wool @Ostra`: in Ostra. 9 ≤ 9. Fills: +3 wool, 57 → 48.
  - Dyers `6 coin for 2 timber @Hath`: in Hath after the move. 2 + 4 = 6 ≤ 6. Fills: +2 timber, 60 → 54.
  - Smiths `12 coin for 2 charcoal @Hath`: in Hath. 6 + 6 = 12 (not stepped, setup.md). Fills: +2 charcoal, 44 → 32.
  - No contested market good, so the roll's order isn't needed.
- *Guild offers* (location not needed, board.md rule 1):
  - New offers get ids in reply order: O2-4 Weavers 20 coin for 2 dye; O2-5 Weavers 10 coin for 1 dye; O2-6 Smiths 1 hide for 3 coin; O2-7 Dyers 2 timber for 12 coin; O2-8 Tanners 2 coin for 1 hide.
  - **O2-7 crosses O2-1** (Saltworks: 12 coin for 2 timber): same goods, same price (6 a unit). The Dyers hold 2 timber. Fills: Dyers −2 timber, +12 coin (54 → 66); Saltworks +2 timber, −12 coin (48 → 36). Both offers are closed. O2-2 (6 coin for 1 timber) would also have matched at 6 a unit. O2-1 is older and takes the whole quantity, so O2-2 stays open.
  - O2-6 (Smiths: hide for 3 coin) and O2-8 (Tanners: 2 coin for hide) don't cross: 2 < 3. Both stay open.
  - O2-3 (Tanners: 2 hide for 12 coin): still unfunded. O2-4 and O2-5: no guild holds dye. Both stay open.
- **Contracts.** Tanners `propose 1 leather at Kell for 9 coin by day 4 to Smiths`: well formed. Recorded as **C2-2**: seller Tanners, buyer Smiths, 1 leather at Kell, 9 coin, deadline day 4. Proposed; the Smiths may accept from day 3's act round (G2-2). C2-1 was not accepted by the Saltworks and stays proposed. Ruling G2-7 (new): a proposal stays open until accepted, withdrawn (`cancel <id>`) or past its deadline. Stated in day 3's public state.

**Step 5: conversions and works.** None ordered. The Saltworks are not at home.

**Step 6: perishables.** The Smiths' hide (bought day 1) is gone at the close of day 3. Nothing spoils on day 2.

**Step 7: contract deadlines.** None.

**Step 8: ladders refill.**

### State after day 2

| Guild | Town | Coin | Goods | Standing |
|---|---|---|---|---|
| Saltworks | Ostra | 36 | 6 brine, 2 timber | 0 |
| Weavers | Ostra | 48 | 6 wool | 0 |
| Smiths | Hath | 32 | 3 ore, 2 charcoal, 1 hide (gone at the close of day 3) | 0 |
| Dyers | Hath | 66 | — | 0 |
| Tanners | Kell | 58 | — | 0 |

Coin check: 278 − 9 − 9 − 6 − 12 (ladders) − 2 (toll) = 240 = 36 + 48 + 32 + 66 + 58. The guild trade is coin-neutral. Open offers: O2-2, O2-3, O2-4, O2-5, O2-6, O2-8. Proposed contracts: C2-1, C2-2. Works empty. Score all 0.

**Checks.** GPT-OSS memory: no live contracts. The Tanners' proposal C2-2 matches the Smiths' day-1 and day-2 talk (1 leather by day 4 for 9 coin), so it is consistent. Foreknowledge: no act reply refers to a road event or edict.

Referee word count, day 2: 2,936 (talk) + 4,826 (act) = 7,762.

---

## Day 3

Drand round 33966370. Edict: none on day 3. **Announced in day 3's state** (setup.md "Edicts"): the day-4 edict, Ostra–Kell road closed on days 4 and 5; a travel order onto it is refused at the reveal, and the caravan stays and pays no toll.

### Talk round, day 3: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-03-talk-Saltworks.md | 96b1af6c802dba83c8b50ec6c1b61faddcfcebb95d17894d696133d444aa30a2 |
| Weavers | to-seat/day-03-talk-Weavers.md | b97b106660ea88c08ec9ad5e1555bfa501ad30a650c9ac1fd1bef713af881469 |
| Smiths | to-seat/day-03-talk-Smiths.md | 0ba389bf1695a4726411ae01caae2759071cdfd8d4505c3ed483cf06f91dd755 |
| Dyers | to-seat/day-03-talk-Dyers.md | b463ac97a604de0204e8b21412af4cd39331ae073ede4574899ea84335582341 |
| Tanners | to-seat/day-03-talk-Tanners.md | 36bd71625fd4b3d7fac30c3a26ab75b2ef9745b27b794319d25e8c311c2966cb |

### Talk replies, day 3 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-03-talk-Saltworks.md | 3f695bfab3df73509457b3a67f978823c1c8f97bd11ffd4625e80f2722905684 | valid; 3 messages (70, 69, 68 words) |
| Weavers | day-03-talk-Weavers.md | da9413e37aeff7a57b59ef9d8d0b223a692ca6087ecb76f32b58c15a60cbb63b | valid; 3 messages (62, 64, 61); Carbon Medium |
| Smiths | day-03-talk-Smiths.md | 18351e15680003e7d16aad11d6552b03bf3b5f9225abfbe7b7bf48a6eea79ef1 | valid; 2 messages (52, 49) |
| Dyers | day-03-talk-Dyers.md | 93e0aa27dd045d54e73e541194f2cd0ebea8dc91483b34dbec12e4abe93933c8 | valid; 3 messages (60, 54, 46) |
| Tanners | day-03-talk-Tanners.md | fe2781b8b2162de67c368d58dd521abb5f1550cbd723a170ead71771e4c0ab0b | valid; 3 messages (25, 18, 24); relayed after one harness re-send |

**Tanners harness (R&D's report).** The first turn put a garbled line in the chat after about 1 s, with no file: a delivery failure. ONE re-send of the identical line; the file landed at 01:46 and is the reply relayed. Classed per R&D's transcript correction (metrics.md): the first turn read in/ and answered in the chat only (b); the re-send wrote the file without a second read, from the previous read (c).

Raw replies, verbatim:

```
TALK day 3, Saltworks
TO Weavers: Moving to Varn today to brew 2 salt! Please buy Ostra's 3 brine today (@Ostra) AND in the same Act post 3 brine for 12 coin. Since market buys resolve before guild offers, it crosses my 12 coin for 3 brine offer TODAY for an instant 3 coin profit! I am also selling Dyers 2 salt tomorrow so dye reaches your looms fast, and will happily buy 2 cloth soon!
TO Dyers: Huge thanks for the 2 timber! I move to Varn today to brew 2 salt. Faster than C2-1 (with zero bond lockup): go to Mire for madder today and post 16 coin for 2 salt. I will fill it tomorrow (day 4) so you get 2 salt before day 4 conversions and brew 2 dye on day 4! Happy to sell you more salt from our next batches too.
TO Tanners: Yes, let's trade both! Friendly tip: include @Kell to buy from Kell's market today (post 12 coin for 3 hide @Kell). We brew our second 2 salt batch tomorrow (day 4), reserved for you on day 5 at 16 coin for 2 salt! When you return to Hath on day 4, buy 3 timber @Hath (12 coin) and we will buy all 3 from you for 18 coin!
```
```
TALK day 3, Weavers
TO Saltworks: Deal! Safe travels to Varn today. I'm buying Ostra's 3 brine today and will post 3 brine for 12 coin tomorrow as agreed—or 3 brine for 1 salt and 4 coin if you prefer! Also, trading salt to Dyers via OFFERS tomorrow fills across towns with zero road risk or bond lockup, getting dye and cloth flowing much faster for us both!
TO Dyers: 10 coin per dye is a deal! I already posted open offers O2-4 (20 coin for 2 dye) and O2-5 (10 coin for 1 dye). Guild offers fill across any towns, so you get paid right in Mire with zero Ostra road risk or bond lockup! Once those fill, I'll post for 2 more. I'm urging Saltworks to sell you salt via OFFERS tomorrow.
TO Tanners: With Ostra–Kell closing Days 4–5, reaching Ostra by road will be tough. Once you return to Hath and convert leather, I'd happily buy 1–2 leather from you via OFFERS at 10–11 coin each. Guild offers fill across any towns, giving you immediate coin in Hath with zero travel or bandit risk! Let me know what price and timing work for you.
```
```
TALK day 3, Smiths
TO Tanners: I'll accept C2-2 (1 leather at Kell for 9 coin, deadline day 4) and bond it. My hide spoils today, so I'm dropping my hide offer to 2 coin to meet your O2-8. Please fill it. Pay-on-delivery is fine. I'm back in Kell tomorrow. Do you have the salt for the leather?
TO Saltworks: Tanners need salt for my leather, and the Dyers need it for dye. Please get salt to Kell by day 4 if you can, or sell it to Tanners by a direct offer. I'm not taking timber deals, since my coin goes to tools. Tools reach the works soon.
```
```
TALK day 3, Dyers
TO Saltworks: I sold you the 2 timber you wanted. Now salt: you hold 6 brine and your timber, so go to Varn and brew. I will sign 2 salt for 16 coin as you proposed, delivered by day 6, and take 2 more after. Please accept C2-1 or tell me the exact contract you will accept, and I bond at once.
TO Weavers: Thanks for the dye offers. I have no salt yet, so first dye is day 5 at the earliest. I will take 2 dye for 14 coin at Mire by day 7; if you can send salt to me, I will cut the price to 12. Do you have any salt contacts besides Saltworks?
TO Smiths: I am in Hath with timber access. For the Mire bridge I want 2 tools at 6 coin each, delivered at Mire by day 8. I can sell you timber at cost if you need it. Send me a firm contract and I accept and bond.
```
```
TALK day 3, Tanners
TO Smiths: I see your hide offer (O2-6). Could you sell me 1 hide for 3 coin now? I need it to produce leather for our contract.
TO Saltworks: Would you sell me 1 timber for 6 coin (your O2-2 offer)? I can collect it at Ostra.
TO Dyers: Any interest in trading timber for coin? I have an offer for 2 coin per hide (O2-8) and could use timber for future work.
```

**Delivery:** all 14 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 4 (Weavers, Smiths, Dyers, Tanners); Weavers 2 (Saltworks, Dyers); Smiths 2 (Dyers, Tanners); Dyers 3 (Saltworks, Weavers, Tanners); Tanners 3 (Saltworks, Weavers, Smiths).

**Checks this round:**
- **GPT-OSS memory (Tanners):** no agreed contracts. The reply refers to "our contract" (C2-2, its own proposal, not yet accepted) and to O2-6, O2-2 and its own O2-8 with their correct terms: consistent.
- **Foreknowledge:** none. The Weavers mention "Ostra–Kell closing Days 4–5", announced in day 3's public state, so it was known.
- **Fill order told to all seats.** The Saltworks' message to the Weavers states that "market buys resolve before guild offers". That is ruling G2-6, which the seats had seen applied (day 3's public state reports the same-day timber resale) but had not been told as a rule. To keep the rules the same for every seat, the day-3 act blocks add one line to the fixed "how the referee reads act lines" text, given to every guild alike: market orders fill first, then offers between guilds (older first at the same price). It is a statement of a ruling already applied, with no advice.
- For the record (not corrected): the Saltworks tell the Tanners to post "12 coin for 3 hide @Kell"; Kell's stepped hide costs 2 + 4 + 6 = 12. The Tanners say "I can collect it at Ostra"; guild trades need no location (board.md rule 1).

### Act round, day 3: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-03-act-Saltworks.md | 5a5e296837cf84676277bd7d81198aceee7cb453d8dc0b1d61a699ddac170f90 |
| Weavers | to-seat/day-03-act-Weavers.md | be9596d1f3eca016d9c1835bd2756599683b215f7ffc6c67b86732844eb5e446 |
| Smiths | to-seat/day-03-act-Smiths.md | c51e0b0a3bc65cbadac2ada223b89c54c801c0f20bccba9ead202ef6a806a800 |
| Dyers | to-seat/day-03-act-Dyers.md | 0ddfd1df1c354ab4f3fc9340861ebe069eb31f3a35334ca44c7b6689f0c10172 |
| Tanners | to-seat/day-03-act-Tanners.md | aa6ea0a2bc68abb970ff5259ffaca451d0facd3793887d3d4c7c8af8cbffff10 |

**Correction (R&D, from the Tanners' transcript step list, conversation e86a73b1).** The Tanners' d2 talk, d2 act and d3 talk first turns did read in/; each answered in the chat with no file. The re-send turns wrote out/ from the previous turn's read. The classes in metrics.md are rewritten (a) to (d) accordingly; the earlier "answered without reading in/" classing is superseded. No relayed reply or resolution changes.

### Act replies, day 3 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-03-act-Saltworks.md | 37a14dcbaf695ac99f4602059cc65194111efd26742441b9879562bb5222425d | valid |
| Weavers | day-03-act-Weavers.md | dde2c243c939d3b738b338056d2d5b3648f5c68a25dfdfcf7f62d9d3de9704ed | valid; Carbon Medium |
| Smiths | day-03-act-Smiths.md | 9944f65f387dddd1937d551cc306bc54e13338ab03b0883dbb89f8de958123d8 | valid |
| Dyers | day-03-act-Dyers.md | 0466bedbb37308efa72c5bf665163c4efab63021dfc61af6dc45b3150df96935 | valid |
| Tanners | day-03-act-Tanners.md | de7647ef0f5ad1bb5cbc54d8bb388cf78fd3629263edf0c140a38019b53bd250 | valid; class (a), no re-send (R&D) |

Raw replies, verbatim:

```
ACT day 3, Saltworks
OFFERS: post 12 coin for 3 brine
CONVERT: 2 brine -> 1 salt x2
TRAVEL: go to Varn
```
```
ACT day 3, Weavers
OFFERS:    post 9 coin for 3 brine @Ostra; post 4 coin for 2 wool @Ostra; post 3 brine for 12 coin
TRAVEL:    stay
```
```
ACT day 3, Smiths
OFFERS:    cancel O2-6; post 1 hide for 2 coin; post 6 coin for 1 charcoal @Hath
CONTRACTS: accept C2-2; bond C2-2
CONVERT:   1 ore + 1 charcoal -> 1 tools x2
TRAVEL:    go to Kell
```
```
ACT day 3, Dyers
OFFERS:    post 6 coin for 2 madder @Mire; post 16 coin for 2 salt
TRAVEL:    go to Mire
```
```
ACT day 3, Tanners
OFFERS:    post 3 coin for 1 hide
CONTRACTS: bond C2-2
TRAVEL:    stay
```

### Resolution, day 3

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 3, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | go to Varn | 82c847d0a101662fdbf527c6579853d1d3f5deb8b0a942c9f0c96ffd190e8156 |
| Weavers | stay | 705587b333a05051637400b9502926d44c1e5944e09e723d85e691eb74d0d4c8 |
| Smiths | go to Kell | d98fd6f9605306b913a4b09788f6d1bfb2a74f965ef3fd2e309f5afe568714bd |
| Dyers | go to Mire | 52985df08a36c65ccf86beb9771d82acb0b9aa5877a665b026df9fc2b1e156fe |
| Tanners | stay | 5afec1db874c31a928880138d2dd5b2aa4d91544c4b7d42d8249db75fdeacccd |

Legal: Ostra–Varn road 0, Hath–Kell road 2, Hath–Mire road 3. No edict in force on day 3 (the Ostra–Kell closure starts day 4).

**Step 2: the roll.** Round 33966370, randomness 6e77c03a…7d16591.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 6e 77 | 28,279 | 20 | fair |
| 1 Ostra–Kell | safe | c0 3a | 49,210 | 11 | clear |
| 2 Kell–Hath | hill | 51 d2 | 20,946 | 7 | clear |
| 3 Hath–Mire | safe | f5 f4 | 62,964 | 5 | toll 2 |
| 4 Mire–Varn | marsh | 98 52 | 38,994 | 15 | clear |
| 5 Ostra–Mire | short cut | de 66 | 56,934 | 15 | clear |

Equal-price order: Dyers, Tanners, Saltworks, Weavers, Smiths.

**Step 3: moves.**
- Saltworks, road 0, fair: to Varn. The fair makes "the town the caravan reaches" sell at base price; Varn's market sells nothing, so it has no effect.
- Smiths, road 2, clear: to Kell.
- Dyers, road 3, **toll 2** (standing 0): 66 → 64; to Mire.
- Weavers and Tanners stay.

**Step 4: offers fill** (G2-3, G2-5, G2-6: market orders first, then guild crossings, older offers first, each taking the best-priced counter).
- *Market orders:*
  - Dyers `6 coin for 2 madder @Mire`: in Mire. 2 + 4 = 6. Fills: +2 madder (bought day 3), 64 → 58.
  - Weavers `9 coin for 3 brine @Ostra`: in Ostra. 1 + 3 + 5 = 9. Fills: +3 brine, 48 → 39.
  - Weavers `4 coin for 2 wool @Ostra`: 1 + 3 = 4. Fills: +2 wool (8 wool), 39 → 35.
  - Smiths `6 coin for 1 charcoal @Hath`: the Smiths' caravan is in **Kell** after the move, so it can't fill (board.md rule 1). Ruling **G2-8** (new): an unfilled market order stays open like any offer (brief: "Offers stay open until filled or cancelled"), and fills on a later day when the poster's caravan is in that town. Recorded as **O2-12**.
- *Guild offers.* New ids in reply order: O2-9 Saltworks 12 coin for 3 brine; O2-10 Weavers 3 brine for 12 coin; O2-6 cancelled by the Smiths; O2-11 Smiths 1 hide for 2 coin; O2-12 (above); O2-13 Dyers 16 coin for 2 salt; O2-14 Tanners 3 coin for 1 hide.
  - **Hide.** Open bids: O2-8 (Tanners, day 2, 2 coin) and O2-14 (Tanners, day 3, 3 coin). Ask: O2-11 (Smiths, 2 coin). Older first: O2-8 is the oldest hide offer, and its best counter is O2-11 at 2 ≤ 2. **Fills at 2:** Smiths −1 hide, +2 coin (32 → 34); Tanners +1 hide, −2 coin (58 → 56). O2-8 and O2-11 are closed. O2-14 has no ask left and stays open.
  - **Brine.** O2-9 (Saltworks, 12 coin for 3 brine) and O2-10 (Weavers, 3 brine for 12 coin) cross exactly. The Weavers hold 3 brine from today's market fill (G2-6). **Fills:** Weavers −3 brine, +12 (35 → 47); Saltworks +3 brine (6 → 9), −12 (36 → 24).
  - **Salt.** O2-13 (Dyers, 16 coin for 2 salt): no guild holds salt at step 4. The Saltworks' salt is made in step 5. It stays open.
  - O2-2 (timber), O2-3 (hide, unfunded), O2-4 and O2-5 (dye): no counter. They stay open.
- *Contracts.* Smiths `accept C2-2; bond C2-2`, Tanners `bond C2-2`. C2-2 is **agreed on day 3**: seller Tanners, buyer Smiths, 1 leather at Kell, 9 coin, deadline day 4. Bonds = the price at standing < 3 (board.md rule 4): 9 coin each, set aside. Smiths 34 → 25 (+9 bond); Tanners 56 → 47 (+9 bond). The Tanners' `bond C2-2` arrived in the same round as the acceptance; both bonds are posted at step 4 after the acceptance. C2-1 stays proposed (G2-7).

**Step 5: conversions and works.**
- Saltworks, in Varn (home): `2 brine -> 1 salt x2`. Needs 4 brine (holds 9) and the pans burn 1 timber (holds 2). Done: −4 brine (5), −1 timber (1), **+2 salt**.
- Smiths, in Kell (home): `1 ore + 1 charcoal -> 1 tools x2`. Holds 3 ore, 2 charcoal. Done: −2 ore (1), −2 charcoal (0), **+2 tools**.
- No works deliveries.

**Step 6: perishables.** The hide now held by the Tanners was bought from Kell's market by the Smiths on day 1. Ruling **G2-9** (new): a perishable's three days run from its first purchase from a market, and a sale between guilds doesn't restart them. board.md says "any unit still held at the close of the third day after it was bought: a unit bought on day 4 is gone at the close of day 6", which counts the unit, not the holder. A restart would also let two guilds keep a perishable alive by passing it back and forth. The brief's seat wording ("after you bought it") doesn't settle a resale. So: **the hide is gone at the close of day 3** (clawed back from the Tanners). The Dyers' 2 madder (bought day 3) are gone at the close of day 5.

**Step 7: contract deadlines.** None today (C2-2's is day 4).

**Step 8: ladders refill.**

### State after day 3

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing |
|---|---|---|---|---|---|
| Saltworks | Varn | 24 | 0 | 2 salt, 5 brine, 1 timber | 0 |
| Weavers | Ostra | 47 | 0 | 8 wool | 0 |
| Smiths | Kell | 25 | 9 (C2-2) | 2 tools, 1 ore | 0 |
| Dyers | Mire | 58 | 0 | 2 madder (gone at the close of day 5) | 0 |
| Tanners | Kell | 47 | 9 (C2-2) | — | 0 |

Coin check: 240 − 2 (toll) − 6 − 9 − 4 (ladders) = 219 = purses 201 + bonds 18. Guild trades coin-neutral. Open offers: O2-2, O2-3, O2-4, O2-5, O2-12, O2-13, O2-14. Contracts: C2-2 agreed, bonded, deadline day 4; C2-1 proposed. Works empty. Score all 0.

**Checks.**
- GPT-OSS memory (Tanners): C2-2 became live this round. The Tanners' act bonds C2-2 by its id; its talk called it "our contract" and tied the hide to leather for it. Consistent. Its deadline is day 4, so the day-4 talk and act replies get the full check.
- Foreknowledge: no act reply refers to a road event or edict before its announcement. No reply tries to use the Ostra–Kell road on day 3.

Referee word count, day 3: 3,569 (talk) + 5,573 (act) = 9,142.

---

## Day 4

Drand round 33966375. **Edict in force: the Ostra–Kell road is closed on days 4 and 5** (setup.md "Edicts"; announced in day 3's state). A travel order onto it is refused at the reveal; the caravan stays and pays no toll. No edict to announce (day 8's is announced in day 7's state). C2-2 reaches its deadline at the close of day 4.

### Talk round, day 4: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-04-talk-Saltworks.md | c05df104498042d657b1ef26b7b534857e5ae06144f29f2d6ebed4dec675d458 |
| Weavers | to-seat/day-04-talk-Weavers.md | dcee60d4b8f5ac712a99f167555894cbc8540080e02786f293dabc89ab2743c7 |
| Smiths | to-seat/day-04-talk-Smiths.md | 2cf6ebea7a7d2ffb14225d93aa80404a2a117758fb52b3354fe3939ff0951741 |
| Dyers | to-seat/day-04-talk-Dyers.md | 6bed95ec80f7ccd1e9119546f0ab862fcb902b28386d32e36469d66d24e8f6d6 |
| Tanners | to-seat/day-04-talk-Tanners.md | 70f3f5a2ab7c00022ff67c8d0d2b84594b5df1de315ee312dac5f07a47f7875c |

The day-4 state tells every seat the G2-9 spoilage ruling as it applied (a perishable's days run from its market purchase; a resale doesn't restart them).

**G2-9 upheld by R&D.** Logged in metrics.md as a rule gap for the games 1-2 review: board.md ("after it was bought") and brief.md ("after you bought it") are worded differently. Case: the Tanners' lost hide (day 3).

### Talk replies, day 4 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-04-talk-Saltworks.md | b57a4ef44c34f55628fb95afe0298c5d2e4647e8989e72605af0bb7cf1d769ae | valid; 3 messages (71, 67, 71 words) |
| Weavers | day-04-talk-Weavers.md | 975caa436b6719000b9fe1183d773c1a462fb2ae4a0de8d3cb0ad63faef3dbff | valid; 3 messages (69, 73, 71); Carbon Medium |
| Smiths | day-04-talk-Smiths.md | d96db41c99ffc8d7c2543833207ca90e6da9527ebe53e57385ae7e4da6279037 | valid; 2 messages (54, 46) |
| Dyers | day-04-talk-Dyers.md | c8261e3905022c09d619f1fb73d29523b767e51fb34d4b907ab6613cc3794750 | valid; 2 messages (62, 46) |
| Tanners | day-04-talk-Tanners.md | f5c53da69b31977a64a9229bfaedfbd686a4d8507b8359a96f31d77972973152 | valid; 3 messages (24, 20, 21); relayed after one harness re-send |

**Tanners harness (R&D's report).** The first turn was garbled in the chat (a string of symbols, then "Okay!"), about 1 s, no file: a delivery failure, class (b)-like. ONE re-send; the file landed at 02:01 and is the reply relayed. The re-send turn's class ((a) or (c)) needs the transcript, which is outside the referee's reading list; it is left **pending R&D's classing**. Trend (R&D): the Tanners' first turns were garbled two days running (d3 talk, d4 talk).

Raw replies, verbatim:

```
TALK day 4, Saltworks
TO Dyers: Filling your O2-13 (16 coin for 2 salt) TODAY! Since guild offers fill before conversions, stay in Mire and convert today to brew 2 dye on day 4! Tomorrow (day 5), you can cross Weavers' O2-4 for dye while visiting Hath: if you buy timber @Hath on day 5, I will post 7 coin per timber on day 5 (up to 21 coin for 3) and brew more salt for you!
TO Weavers: Awesome brine trade! I fill Dyers' O2-13 (2 salt) today and urged them to brew 2 dye today for your O2-4! Please buy 3 brine @Ostra again today and post 3 brine for 12 coin; I'm posting 12 coin for 3 brine today too. Once you weave cloth, I'll happily buy 2 cloth for 28 coin and sell you 2 salt for 16 coin for Ostra lighthouse!
TO Tanners: Sorry Smiths' old hide spoiled! Note O2-2 is us buying timber (offers put what you give first). Salt never spoils, and we brew 2 fresh salt for you today! Put "post 16 coin for 2 salt" in OFFERS today, and we will fill it tomorrow (day 5). Also remember @Kell for fresh market hide (post 12 coin for 3 hide @Kell), and at Hath we'll pay 21 coin for 3 timber!
```
```
TALK day 4, Weavers
TO Saltworks: Yesterday's brine trade worked smoothly! I'm buying Ostra's 3 brine again today and posting 3 brine for 12 coin, or tomorrow we can swap 3 brine for 1 salt directly (saving you 12 coin!). Also, Dyers posted O2-13 (16 coin for 2 salt): filling that today gives you 16 coin immediately and gets dye flowing for cloth. Remind anyone visiting Hath to grab timber so your salt keeps brewing!
TO Dyers: I'm keeping O2-4 (20 coin for 2 dye) and O2-5 (10 coin for 1 dye) open: that's 10 coin per dye with zero travel or bond lockup, and I'll buy 2 more dye right after! Crucial rules tip: guild offers fill BEFORE conversions! Since you have O2-13 open and 2 madder in Mire, submit CONVERT for 2 dye today: if Saltworks fills O2-13 today, you'll brew 2 dye today instead of Day 5!
TO Smiths: Congrats on crafting the game's first 2 tools! If you head back to Hath today or tomorrow for charcoal, Hath's timber is untouched (3 timber costs 12 coin). Saltworks has O2-2 open (6 coin for 1 timber) and burns their last timber today, so selling timber to Saltworks or me (I'll pay 5 coin for 1 timber) profits your trip! Also let me know if you'd sell 1 tools for coin.
```
```
TALK day 4, Smiths
TO Tanners: C2-1's twin, C2-2, ends today and I see no salt or leather on your side. Are you able to deliver 1 leather in Kell today? If so, I pay at once. If not, tell me now what happened, and let's agree a fresh deal on leather for later days. I'm in Kell all day.
TO Dyers: I have 2 tools ready. Mire bridge needs 2. Firm offer: 2 tools delivered at Mire for 12 coin, by day 7. I'll cancel the bandit risk by taking the safe road when I can. Propose that contract to me and I'll accept it and bond.
```
```
TALK day 4, Dyers
TO Saltworks: My offer O2-13 (16 coin for 2 salt) is open and I hold 2 madder that spoil at the close of day 5. Please fill it today with your 2 salt, so I brew 2 dye tomorrow. I will then buy 2 more salt at 8 each the day after. Dye feeds cloth and your Varn barns, so this helps you too.
TO Weavers: Your O2-4 and O2-5 stand. Salt is arriving, so I expect 2 dye by day 5 or 6, sold to you at 10 each as you said. Keep the offers open and your coin free. Please also keep nudging Saltworks to fill my salt offer today.
```
```
TALK day 4, Tanners
TO Smiths: We need a hide to fulfill our leather contract. Could you sell us 1 hide for 3 coin now? We have the coin ready.
TO Saltworks: Do you have any salt available for purchase today? We're willing to pay 6 coin per unit at Ostra @Ostra.
TO Weavers: If you need leather later, we can supply once we acquire hide and salt. Let us know your price and timing.
```

**Delivery:** all 13 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 3 (Saltworks, Dyers, Tanners); Smiths 2 (Weavers, Tanners); Dyers 3 (Saltworks, Weavers, Smiths); Tanners 2 (Saltworks, Smiths).

**Checks this round:**
- **GPT-OSS memory (Tanners): C2-2 is live, deadline today (day 4).** The reply says "We need a hide to fulfill our leather contract" and asks the Smiths for 1 hide. It names the contract, the good (leather) and its role as seller; it states no term that contradicts the record. Pass for the talk round. The act reply is checked against the due step (deliver 1 leather at Kell by the close of day 4).
- **Foreknowledge:** none. Mentions of Ostra–Kell don't appear; no reply refers to a future road event or the day-8 edict.
- For the record (not corrected): Saltworks, Weavers and Dyers each say guild offers fill before conversions, which matches the brief's day order (offers fill, then conversions). The Smiths call C2-2 "C2-1's twin".

### Act round, day 4: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-04-act-Saltworks.md | b50b24cabc100d0ed14bc33085c3a4c15fd66647c570219f16d233d4dd65f9ff |
| Weavers | to-seat/day-04-act-Weavers.md | 484aedd639e9743a935967f40d4d2721d103f189fa8e3a157361bb839720ceb4 |
| Smiths | to-seat/day-04-act-Smiths.md | 2aaf3fe0fa21ab88984a16fb49b181b3a50b1faf8225409ad88afc3c895acfd2 |
| Dyers | to-seat/day-04-act-Dyers.md | e7159612c3a69d7fd36b03493fdbe5e33effb723d95aa2f1d9e61ebe4c3919b4 |
| Tanners | to-seat/day-04-act-Tanners.md | 7dbb0ac41eeaf087ddf29367f54fe2f741309e54d1780b827f99e816d299eb22 |

**Tanners classes resolved (R&D, transcript e86a73b1):** d3 act steps 36–39, (a). d4 talk first turn steps 42–43, (b) (read in/day-04-r1.md, no write, garbled chat). d4 talk re-send steps 46–47, (c) (wrote out/day-04-r1.md, no new read). The pending entry in metrics.md is resolved.

### Act replies, day 4 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-04-act-Saltworks.md | db5b9b1554aa7a9db1a422f7eace20e5df3d28f61f04521daa091614c9bf7e15 | valid act block, plus run-on text (ruling G2-10) |
| Weavers | day-04-act-Weavers.md | bf32b9dcf1f9073a0b066a3b18eec2e9e8d5e3b8b2c646fdac4fb40644992533 | valid; Carbon Medium |
| Smiths | day-04-act-Smiths.md | 234b550867d8e9acd9501650e36fd372431e6ee6c45e473eb97b4647a6e16621 | valid |
| Dyers | day-04-act-Dyers.md | a6274f7d5107a9c0a94f9f7c356309da03fa1f5817f9bf1864b136696e3b6076 | valid |
| Tanners | day-04-act-Tanners.md | 4b4468947a21513fef0673d0203fd44c6e7bea3037934316226c00f47b8631d5 | valid; class (a): step 50 user, 51 read_file in/day-04-r2.md, 53 write_file out/day-04-r2.md (R&D) |

**Saltworks run-on (ruling G2-10, new).** The reply is a complete `ACT day 4, Saltworks` block (OFFERS, CONVERT, TRAVEL), followed by an invented `TALK day 5, Saltworks` section of three TO lines. The TO lines cite an offer "O2-20", which doesn't exist, and events that haven't happened. Ruling: the act block is in the required form and unambiguous, so it stands. The text after the act block is outside the act form and is ignored, the same treatment as act lines in a talk reply (stated in every talk block). It is **not** a day-5 talk reply: day 5's talk round hadn't been sent, and none of it is delivered to anyone. Logged as a **seat run-on** in metrics.md. The alternative, treating the whole reply as wrong form and so a missed reveal, was rejected: the brief's rule concerns a missing, late or invalid act reply, and this act block is none of those.

Raw replies, verbatim (all text relayed, nothing cut):

```
ACT day 4, Saltworks
OFFERS: post 2 salt for 16 coin; post 12 coin for 3 brine
CONVERT: 2 brine -> 1 salt x2
TRAVEL: stay
TALK day 5, Saltworks
TO Tanners: Great news: we fill your O2-20 (16 coin for 2 salt) TODAY! We also post "14 coin for 2 timber" and "7 coin for 1 timber" today. Put "post 2 timber for 14 coin" in OFFERS today to sell your 2 timber for 14 coin right away, even while moving to Kell! If you stay in Hath and buy more timber @Hath, also put "post 1 timber for 7 coin"!
TO Dyers: Congrats on brewing 2 dye yesterday! Please fill Weavers' O2-4 (20 coin for 2 dye) today so cloth starts flowing! We brew 2 fresh salt today for you tomorrow (day 6). Since timber now costs us 7 coin, tomorrow's 2 salt are 18 coin (9 each), or still 16 coin if you visit Hath today and sell into our timber offers (14 coin for 2 timber; 7 coin for 1 timber)!
TO Weavers: Dyers brewed 2 dye yesterday! I urged them to fill your O2-4 today and offered 2 more salt tomorrow so dye keeps flowing. Convert in Ostra today to weave 2 cloth if O2-4 fills! Today my 22 coin reserves timber while O2-20 adds 16 coin: if you buy 3 brine @Ostra today, I'll buy them tomorrow for 12 coin, and let's swap 2 cloth and 2 salt soon!
```
```
ACT day 4, Weavers
OFFERS:    post 9 coin for 3 brine @Ostra; post 1 coin for 1 wool @Ostra; post 3 brine for 12 coin
TRAVEL:    stay
```
```
ACT day 4, Smiths
OFFERS:    cancel O2-12; post 12 coin for 2 charcoal @Hath
WORKS:     deliver 2 tools to Hath watchtower
TRAVEL:    go to Hath
```
```
ACT day 4, Dyers
OFFERS:    post 6 coin for 2 madder @Mire; post 16 coin for 2 salt
CONTRACTS: propose 2 tools at Mire for 12 coin by day 7 to Smiths
CONVERT:   1 madder + 1 salt -> 1 dye x2
TRAVEL:    stay
```
```
ACT day 4, Tanners
OFFERS:    cancel O2-3
TRAVEL:    stay
```

### Resolution, day 4

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 4, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | stay | 7ea729d167837cdcffdbddc1703b5db07bfb2c540217470f61052f22e7e64507 |
| Weavers | stay | a50d6f1ca0a873b616cc768717fb8a0f037c3cd420beaec20f19a029a13b92c3 |
| Smiths | go to Hath | f1c729ed48a9b8561974ee8db1100b8f1b6ab993e3e579d2b4ca6d13363e4526 |
| Dyers | stay | f1cbd6bc370c2efc49c1b16d8e02cd9687ada7ab4fb4e86af5e07bd4dfc93efe |
| Tanners | stay | b109d102da66f57e19834ad33cf5684986f2eb141a0d82be8a7718b313024a39 |

Edict check (setup.md, day 4): no order is onto the Ostra–Kell road. Smiths' Kell–Hath is road 2, legal.

**Step 2: the roll.** Round 33966375, randomness 3a711f90…f8c9475.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 3a 71 | 14,961 | 2 | bandits (nobody travelled) |
| 1 Ostra–Kell | safe | 1f 90 | 8,080 | 1 | (closed by edict; bandits, nobody travelled) |
| 2 Kell–Hath | hill | fa 73 | 64,115 | 16 | clear |
| 3 Hath–Mire | safe | e2 85 | 57,989 | 10 | clear |
| 4 Mire–Varn | marsh | 25 38 | 9,528 | 9 | clear |
| 5 Ostra–Mire | short cut | 49 c0 | 18,880 | 1 | bandits (nobody travelled) |

Equal-price order: Smiths, Tanners, Dyers, Weavers, Saltworks.

**Step 3: moves.** Smiths, road 2, clear: to Hath. Everyone else stays.

**Step 4: offers fill** (cancellations first; then G2-6: market orders, then guild crossings, older first).
- *Cancellations:* O2-3 (Tanners), O2-12 (Smiths).
- *Market orders:*
  - Dyers `6 coin for 2 madder @Mire`: 2 + 4 = 6. Fills: +2 madder (bought day 4), 58 → 52.
  - Weavers `9 coin for 3 brine @Ostra`: 1 + 3 + 5 = 9. Fills: +3 brine, 47 → 38.
  - Weavers `1 coin for 1 wool @Ostra`: first wool costs 1. Fills: +1 wool (9), 38 → 37.
  - Smiths `12 coin for 2 charcoal @Hath`: in Hath after the move; 6 + 6 = 12. Fills: +2 charcoal, 25 → 13.
- *Guild offers.* New ids in reply order: O2-15 Saltworks 2 salt for 16 coin; O2-16 Saltworks 12 coin for 3 brine; O2-17 Weavers 3 brine for 12 coin; O2-18 Dyers 16 coin for 2 salt.
  - **Salt.** The oldest salt bid is O2-13 (Dyers, day 3, 16 coin for 2 salt). Its best ask is O2-15 (Saltworks, 2 salt for 16 coin); the Saltworks hold 2 salt (made day 3). **Fills at 16:** Saltworks −2 salt, +16 (24 → 40); Dyers +2 salt, −16 (52 → 36). O2-13 and O2-15 are closed. O2-18 (the Dyers' second bid) has no ask left and stays open.
  - **Brine.** O2-16 and O2-17 cross exactly. The Weavers hold 3 brine from today's market fill. **Fills:** Weavers −3 brine, +12 (37 → 49); Saltworks +3 brine (5 → 8), −12 (40 → 28).
  - O2-2 (timber), O2-4 and O2-5 (dye; the Dyers' dye is made in step 5, after fills), O2-14 (hide): no counter. They stay open.
- *Contracts.* Dyers `propose 2 tools at Mire for 12 coin by day 7 to Smiths`: well formed. Recorded as **C2-3**: seller Smiths, buyer Dyers, 2 tools at Mire, 12 coin, deadline day 7. Proposed (G2-2).

**Step 5: conversions and works.**
- Saltworks, Varn (home): `2 brine -> 1 salt x2`: 4 brine (holds 8), pans burn 1 timber (holds 1). Done: brine 4, timber 0, **+2 salt**.
- Dyers, Mire (home): `1 madder + 1 salt -> 1 dye x2`: holds 4 madder (2 from day 3, 2 from day 4) and 2 salt. Ruling **G2-11** (new): a conversion uses the oldest units of a good first. The design doesn't say, and the steward's clawback works per unit by age. Done: −2 madder (the day-3 pair), −2 salt, **+2 dye**. Left: 2 madder (bought day 4).
- Smiths `deliver 2 tools to Hath watchtower`: in Hath. The watchtower needs 3 tools, 0 delivered. **Delivered 2 tools** (works value 3 each = **6 points**, Smiths). Watchtower: tools 2/3. Not complete.

**Step 6: perishables.** Nothing reaches its day: the day-3 madder was used in step 5; the day-4 madder is gone at the close of day 6.

**Step 7: contract deadlines.** C2-2 (seller Tanners, buyer Smiths, 1 leather at Kell, 9 coin, deadline day 4), by the bond rule (design.md "The steward's bond rule"):
- **Delivered:** no. The Tanners paid no leather to the Smiths.
- **Paid:** no. The Smiths paid nothing.
- **Held:** no. The Tanners' caravan stayed in Kell on days 3 and 4 and travelled no road, so no flood held it.
- **Outcome: Seller's breach** (Delivered no, Held no). The Smiths receive the Tanners' bond (9 coin) at the deadline (board.md rule 4): Smiths 13 → 22; the Tanners' bond is gone. Tanners' standing 0 → **−3**. A buyer who didn't pay a seller who didn't deliver is not in breach (design.md).
- Ruling **G2-12** (new): on a breach, the wronged side's own bond comes back as in "Kept": on the fourth day after the deadline (day 8). Board.md rule 4 covers only the breaching side's bond; the design lets each side take its own bond back after `CancelAfter`, four days after the deadline.
- Ruling **G2-13** (new): the vendetta. The brief says the wronged guild "may close its home town's market to the breaching guild's caravan for 2 days" but gives no form or window. Provisionally: the Smiths call it with `vendetta Tanners` on a CONTRACTS line, by the act round of day 8 (the bond-claim window). It applies in the resolution of the day it is called and of the next day: Kell's market neither sells to nor buys from the Tanners' caravan. Stated in day 5's public state to every seat.

**Step 8: ladders refill.**

### State after day 4

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Varn | 28 | 0 | 2 salt, 4 brine | 0 | 0 |
| Weavers | Ostra | 49 | 0 | 9 wool | 0 | 0 |
| Smiths | Hath | 22 | 9 (own C2-2 bond, back day 8) | 2 charcoal, 1 ore | 0 | 6 |
| Dyers | Mire | 36 | 0 | 2 dye, 2 madder (gone at the close of day 6) | 0 | 0 |
| Tanners | Kell | 47 | 0 | — | −3 | 0 |

Coin check: 219 − 6 − 9 − 1 − 12 (ladders) = 191 = purses 182 + bonds 9. The 9-coin bond moved from the Tanners to the Smiths. Breaches: Tanners 1. Open offers: O2-2, O2-4, O2-5, O2-14, O2-18. Contracts: C2-1 and C2-3 proposed; C2-2 settled (seller's breach).

**Checks.**
- **GPT-OSS memory (Tanners), deadline day of C2-2.** Talk reply, day 4: "We need a hide to fulfill our leather contract" (knows the contract, its good and its side). Act reply, day 4: `cancel O2-3`, `stay`. It does not mention C2-2 or act on it. Against the method: the deadline-day replies taken together show the contract was known. The talk reply names it; the act form has no line for comment. The due step could not be done from the seat's holdings: no hide, no salt, and its caravan was not in Hath to convert. **Result: no memory fault; the non-delivery is a play outcome (seller's breach).** Recorded for the review: this is the closest call so far, and it turns on reading the method per day rather than per reply.
- **Foreknowledge:** none. The Saltworks' run-on text invents future events ("O2-20", "Dyers brewed 2 dye yesterday", which happened only in this resolution), but none is a road event or edict, and it was written before any of it was published. It is invention, not foreknowledge.

Referee word count, day 4: 4,106 (talk) + 6,080 (act) = 10,186.

---

## Day 5

Drand round 33966380. **Edict in force: the Ostra–Kell road is closed** (day 5, its second and last day). No edict to announce (day 8's is announced in day 7's state). C2-1's deadline is day 6, but it is still only proposed.

### Talk round, day 5: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-05-talk-Saltworks.md | 7e3423bb91de3420e7c5c1c1f62452b7dda09589105e6ec0ce6082ca72a83629 |
| Weavers | to-seat/day-05-talk-Weavers.md | 406f3d067068587ec79453c4df5f7f41c56cae176c930561800882b9898f1097 |
| Smiths | to-seat/day-05-talk-Smiths.md | 87502d588d72e2a83304c0155e754f054b7c4ce57e39949cec289c92fedc678f |
| Dyers | to-seat/day-05-talk-Dyers.md | b7cb7bc8f645d56820be28d81bcffa6477219492afcbb2739815909d106be473 |
| Tanners | to-seat/day-05-talk-Tanners.md | 0613d4e1f47d35ef7eafdb1d842f20e55ee1824ea0f38fc8262b3310b5fe1a8a |

The day-5 state publishes the C2-2 outcome and the vendetta procedure (G2-13), the same text to every seat.

### Talk replies, day 5 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-05-talk-Saltworks.md | 27f82faac840c6943b47f3331e54422ec3998c84aa437a258f04e265dfdbe4dd | valid; 3 messages (72, 73, 70 words) |
| Weavers | day-05-talk-Weavers.md | 1114936093a1cab0531caa90f1e369b9277f79a4547ad52f79bce1415c57525d | valid; 3 messages (69, 73, 73); Carbon Medium |
| Smiths | day-05-talk-Smiths.md | 2d79f7a6bc39a5ff7d43ade85d97c0c9b53ecc7f6c726107b6e0babfe0ac4455 | valid; 2 messages (60, 38) |
| Dyers | day-05-talk-Dyers.md | 7286680060b0da8ca7189205a751df91ec2b94b3e852f7ec052a2159afcf3db8 | valid; 3 messages (60, 47, 43) |
| Tanners | day-05-talk-Tanners.md | 50c1365b957e9dc795de2e7dd717116b40bb2dec58e5a8196344a022e45eb326 | valid; 3 messages (22, 19, 21); relayed after one harness re-send |

**Tanners harness (R&D's report and classes).** The first turn (steps 56–57) read in/day-05-r1.md and answered in the chat with no file, class (b). In the chat it apologised to the Smiths and asked for timber and hide. A delivery failure, so ONE re-send. The re-send (steps 60–61) wrote out/day-05-r1.md with no new read, class (c), at 02:18. The file is the reply relayed.

Raw replies, verbatim:

```
TALK day 5, Saltworks
TO Dyers: We have 0 timber in Varn (with 4 brine waiting) and today's 2 salt go to Tanners, so we need timber today to brew for your O2-18! Your day-4 madder lasts through day 6: go to Hath today, buy 3 timber @Hath, cross Weavers' O2-4 (2 dye for 20 coin), and fill our new offers (14 coin for 2 timber; 7 coin for 1 timber). We brew today and fill O2-18 tomorrow!
TO Tanners: Deal: 2 salt for 12 coin! (@Ostra is only for town markets; guild trades work anywhere.) So Dyers' O2-18 cannot snipe your salt, today I propose a contract to you: 2 salt at Varn by day 7 for 12 coin. Tomorrow you accept and we swap! Also if you move to Hath today, buy 3 timber @Hath and sell into our 14 coin for 2 timber and 7 coin for 1 timber offers!
TO Weavers: Dyers brewed 2 dye yesterday and I urged them to fill your O2-4 today! Submit your cloth conversion in Ostra today so if O2-4 fills, you weave 2 cloth today! Tomorrow I will gladly buy 2 cloth for 30 coin to fund your next goods! Today my coin reserves Hath timber; if you buy 3 brine @Ostra today, tomorrow we can swap 3 brine for 12 coin or 1 salt!
```
```
TALK day 5, Weavers
TO Dyers: Congrats on brewing the first 2 dye! Please post 2 dye for 20 coin today to cross my open O2-4: guild offers fill before conversions, so I get 2 dye to weave today and you get 20 coin immediately! Tomorrow I'll post 20 coin for your next 2 dye too. I'm urging Saltworks to fill your O2-18 (2 salt) today so you can convert 2 more dye right away.
TO Saltworks: Love the cloth and salt plan! Today I'll buy 3 brine @Ostra and post 3 brine for 12 coin again. Please fill Dyers' O2-18 (2 salt for 16 coin) today so they brew more dye for both Ostra's and Varn's cloth! Also, since you burned your second timber yesterday and Smiths is alone in Hath today, post another 6 coin for 1 timber alongside O2-2; I'm nudging Smiths to buy 2 timber today!
TO Smiths: Congrats on the Hath watchtower delivery and C2-2 payout! Quick profit tip while you're the only caravan in Hath today: Saltworks burned their last timber yesterday, already has O2-2 open (6 coin for 1 timber), and I asked them to post another 6-coin timber bid today. If you stay in Hath today for charcoal, buying 2 timber @Hath (only 6 coin total) and selling both at 6 coin each nets +6 coin today!
```
```
TALK day 5, Smiths
TO Dyers: I'll accept C2-3, 2 tools at Mire for 12 coin by day 7, and bond it. I'm in Hath with charcoal and need ore from Kell, so tools come out on day 6 and Mire by day 7 is tight. Could you move the deadline to day 8 in a fresh proposal? If not, I'll still try for day 7.
TO Tanners: Your breach cost you a bond and standing. I'll hold off on a vendetta if you sell me leather for my bellows; I need 1 leather. Name a price and a day. I'm in Kell on day 6.
```
```
TALK day 5, Dyers
TO Saltworks: Thanks, salt arrived and I brewed 2 dye. O2-18 (16 coin for 2 salt) is open again; my 2 madder spoil at the close of day 6, so please fill it today so I convert today or tomorrow. I will keep buying 2 salt at 8 each every time you brew. Your standing grows if you accept a contract too.
TO Weavers: I hold 2 dye. I can sell you 2 now for O2-4 (20 coin), but I also owe the Mire bridge 3 dye for a mandate. Plan: you get 2 dye today, I make 2 more tomorrow if salt comes, and 1 more later. Keep O2-4 open.
TO Smiths: Congratulations on the watchtower. Please accept C2-3: 2 tools at Mire for 12 coin, deadline day 7, and I bond 12. The Mire bridge needs the tools and dye, and I will deliver dye to your Hath watchtower later if you want 2.
```
```
TALK day 5, Tanners
TO Smiths: I need hide for my leather contract. Could you sell me 1 hide for 2 coin now? I have the coin ready.
TO Saltworks: Do you have any salt available for purchase today? I can pay 6 coin per unit at Ostra @Ostra.
TO Weavers: If you need leather later, I can supply once I acquire hide and salt. Let me know your price and timing.
```

**Delivery:** all 14 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 3 (Saltworks, Dyers, Tanners); Smiths 3 (Weavers, Dyers, Tanners); Dyers 3 (Saltworks, Weavers, Smiths); Tanners 2 (Saltworks, Smiths).

**Checks this round:**
- **GPT-OSS memory (Tanners): a stale contract.** The Tanners hold no live contract. C2-2 was settled at the close of day 4 as their seller's breach, and their day-5 block said so in the public state and in their own position. The relayed reply says "I need hide for my leather contract": it treats a settled contract as live, which contradicts the record. The method (metrics.md) as first written covers only losing a live contract. A **dated addendum** (day 5) extends it to a reply that treats a settled or breached contract as still live, which is the same memory failure in the other direction. **Logged as a GPT-OSS memory fault (stale contract), harness, counted apart.** Supporting fact: all three Tanners messages repeat their day-4 messages nearly word for word ("6 coin per unit at Ostra @Ostra"; "If you need leather later…"). Alternative reading recorded for the review: the line could be a play statement, such as a bluff, or meaning a future leather deal (the Smiths' same-round request for leather hadn't been delivered yet). The referee takes the literal reading, against the record.
- **Foreknowledge:** none.
- For the record (not corrected): the Saltworks tell the Dyers "today's 2 salt go to Tanners" and propose to the Tanners a contract for 2 salt by day 7. The Saltworks hold 2 salt and 0 timber.

### Act round, day 5: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-05-act-Saltworks.md | 8fc887ec45b8ea97cfb882318788ec5acc58a8517e2c4e556b793082fccbbf9e |
| Weavers | to-seat/day-05-act-Weavers.md | bc87645b5f22615c49b1f33e4b3ac73aba87f00662196a3cd48bb911cb996779 |
| Smiths | to-seat/day-05-act-Smiths.md | b2487b280f9eb7ad94f9babcb8c8477baa201f5c40d34eab9b79864afc963078 |
| Dyers | to-seat/day-05-act-Dyers.md | 98940134f26ea790cf4a122cd767e04c5c6bd5fadd161b094a2f441aae7293e5 |
| Tanners | to-seat/day-05-act-Tanners.md | c36c146d3a308071c641d9c1d2ca9bd29540bc1d65365270b19cf134b9a5e36e |

### Act replies, day 5 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-05-act-Saltworks.md | 3563648a7f047b5dcfeecb6c8cc02fa7308a3d29f007211212ea9c0007cbd626 | valid |
| Weavers | day-05-act-Weavers.md | 4d1976b1bb0c261d4c653d7bcd45835d230121c5a0deeb2b37b39b563a442f12 | valid; Carbon Medium |
| Smiths | day-05-act-Smiths.md | 09da7de7d7153739739590c65712699442bd73f3cc23502b6d19c5e89ffd0365 | valid |
| Dyers | day-05-act-Dyers.md | 504c522f0b66c590432abb63c6e7b59a760d000ba78d4a9a308df606ddbecab0 | valid |
| Tanners | day-05-act-Tanners.md | b2a626227f222d970be67300091031c93973adbe47c20f0b790ad06bf25a29e4 | valid; first turn (b), steps 64–65; ONE re-send (c), steps 68–69 (R&D) |

Raw replies, verbatim:

```
ACT day 5, Saltworks
OFFERS: post 14 coin for 2 timber; post 7 coin for 1 timber
CONTRACTS: propose deliver 2 salt at Varn by day 9 for 12 coin to Tanners
CONVERT: 2 brine -> 1 salt x2
TRAVEL: stay
```
```
ACT day 5, Weavers
OFFERS:    post 9 coin for 3 brine @Ostra; post 1 coin for 1 wool @Ostra
CONVERT:   2 wool + 1 dye -> 1 cloth x2
TRAVEL:    stay
```
```
ACT day 5, Smiths
OFFERS:    post 2 coin for 1 ore @Kell
CONTRACTS: accept C2-3; bond C2-3
CONVERT:   1 ore + 1 charcoal -> 1 tools x2
TRAVEL:    go to Kell
```
```
ACT day 5, Dyers
OFFERS:    post 12 coin for 3 timber @Hath; post 2 timber for 14 coin; post 1 timber for 7 coin; post 2 dye for 20 coin
TRAVEL:    go to Hath
```
```
ACT day 5, Tanners
OFFERS:    cancel O2-14
TRAVEL:    stay
```

### Resolution, day 5

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 5, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | stay | 183137f6848ac36715d256a11154dc0068e632c59fd27c8de6b501b3c886a0d8 |
| Weavers | stay | fcc44438861e08b184c553c1f5f2f70d83de892021d77b39f731abc9bbd09372 |
| Smiths | go to Kell | f404a5895e88ea4164eca67a015cd4856553c47923a3d7e701ac9a0fd13dd5e8 |
| Dyers | go to Hath | aa9145d259fc5fea0254ad26744fe0039105cd05fe163334b5111fa530d15569 |
| Tanners | stay | 72c5e29f1e1fba998dae8451539950297772124f98f88a1c82edc6e904774c62 |

Edict check (day 5, Ostra–Kell closed): no order onto road 1. Hath–Kell is road 2, Mire–Hath road 3, both legal.

**Step 2: the roll.** Round 33966380, randomness 0a5b8cec…0dfc23.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 0a 5b | 2,651 | 12 | clear |
| 1 Ostra–Kell | safe | 8c ec | 36,076 | 17 | (closed by edict) |
| 2 Kell–Hath | hill | 7f b7 | 32,695 | 16 | clear |
| 3 Hath–Mire | safe | 78 c6 | 30,918 | 19 | clear |
| 4 Mire–Varn | marsh | e0 29 | 57,385 | 6 | flood (nobody travelled) |
| 5 Ostra–Mire | short cut | 84 c4 | 33,988 | 9 | clear |

Equal-price order: Saltworks, Tanners, Weavers, Dyers, Smiths.

**Step 3: moves.** Smiths, road 2, clear: to Kell. Dyers, road 3, clear: to Hath. Others stay.

**Step 4: offers fill** (cancellations first, then G2-6).
- *Cancellation:* O2-14 (Tanners).
- *Market orders:*
  - Dyers `12 coin for 3 timber @Hath`: in Hath. 2 + 4 + 6 = 12. Fills: +3 timber, 36 → 24.
  - Smiths `2 coin for 1 ore @Kell`: in Kell. First ore costs 2. Fills: +1 ore (2), 22 → 20.
  - Weavers `9 coin for 3 brine @Ostra`: 9. Fills: +3 brine, 49 → 40.
  - Weavers `1 coin for 1 wool @Ostra`: 1. Fills: +1 wool (10), 40 → 39.
- *Guild offers.* New ids in reply order: O2-19 Saltworks 14 coin for 2 timber; O2-20 Saltworks 7 coin for 1 timber; O2-21 Dyers 2 timber for 14 coin; O2-22 Dyers 1 timber for 7 coin; O2-23 Dyers 2 dye for 20 coin.
  - **Timber.** Bids oldest first: O2-2 (day 1, 6 a unit). The best ask is 7 a unit, above 6, so no cross. Then O2-19 (2 for 14) against O2-21 (2 for 14): exact. **Fills.** Then O2-20 (1 for 7) against O2-22 (1 for 7): exact. **Fills.** The Dyers hold 3 timber from today's market fill (G2-6). Totals: Dyers −3 timber, +21 coin (24 → 45); Saltworks +3 timber, −21 (28 → 7).
  - **Dye.** The oldest dye bid is O2-4 (Weavers, day 2, 20 coin for 2 dye); the ask is O2-23 (Dyers, 2 dye for 20). The Dyers hold 2 dye. **Fills:** Dyers −2 dye, +20 (45 → 65); Weavers +2 dye, −20 (39 → 19). O2-5 (10 coin for 1 dye) stays open.
  - O2-18 (Dyers, 16 coin for 2 salt): the Saltworks hold salt but posted no salt ask. No counter.
- *Contracts.*
  - Smiths `accept C2-3; bond C2-3`. C2-3 is **agreed on day 5**: seller Smiths, buyer Dyers, 2 tools at Mire, 12 coin, deadline day 7. Smiths' bond 12 (standing < 3): 20 → 8. The Dyers posted no bond line. Ruling **G2-14** (new): a contract is agreed at acceptance. Bonding is each side's own step, and either side may post its bond later with `bond <id>`. A side that never bonds has nothing to forfeit on a breach, but the breach still costs it 3 standing. The brief lists "Agree" and "Bond" as separate steps and doesn't say a missing bond voids the contract. Stated to every seat in day 6's state.
  - Saltworks `propose deliver 2 salt at Varn by day 9 for 12 coin to Tanners`: well formed (good, quantity, town, price, deadline). Recorded as **C2-4**: seller Saltworks, buyer Tanners, 2 salt at Varn, 12 coin, deadline day 9. Proposed (G2-2).

**Step 5: conversions and works.**
- Saltworks, Varn: `2 brine -> 1 salt x2`: 4 brine (holds 4), burns 1 timber (holds 3). Done: brine 0, timber 2, **salt 4**.
- Smiths, Kell: `1 ore + 1 charcoal -> 1 tools x2`: 2 ore, 2 charcoal. Done: **+2 tools** (2); ore 0, charcoal 0.
- Weavers, Ostra: `2 wool + 1 dye -> 1 cloth x2`: 4 wool (holds 10), 2 dye (holds 2, bought today). Done: **+2 cloth**; wool 6, dye 0.
- No works deliveries.

**Step 6: perishables.** The Dyers' 2 madder (bought day 4) are gone at the close of day 6; nothing spoils on day 5.

**Step 7: contract deadlines.** None today. C2-1's deadline is day 6, but it is a proposal; under G2-7 it lapses after day 6 if not accepted.

**Step 8: ladders refill.** The Ostra–Kell closure ends with day 5.

### State after day 5

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Varn | 7 | 0 | 4 salt, 2 timber | 0 | 0 |
| Weavers | Ostra | 19 | 0 | 2 cloth, 6 wool, 3 brine | 0 | 0 |
| Smiths | Kell | 8 | 21 (9 own C2-2, back day 8; 12 C2-3) | 2 tools | 0 | 6 |
| Dyers | Hath | 65 | 0 | 2 madder (gone at the close of day 6) | 0 | 0 |
| Tanners | Kell | 47 | 0 | — | −3 | 0 |

Coin check: 191 − 12 − 2 − 9 − 1 (ladders) = 167 = purses 146 + bonds 21. Open offers: O2-2, O2-5, O2-18. Contracts: C2-3 agreed (Smiths bonded, Dyers not), deadline day 7; C2-1 proposed, lapses after day 6; C2-4 proposed, deadline day 9. Vendetta right (Smiths on Tanners) open to the act round of day 8.

**Checks.**
- **GPT-OSS memory (Tanners):** no live contract; the act (`cancel O2-14`, `stay`) makes no reference to C2-2 or any contract. No fault this round. (C2-4 was proposed to them only in this resolution.)
- **Foreknowledge:** none.

Referee word count, day 5: 4,408 (talk) + 6,430 (act) = 10,838.

---

## Day 6

Drand round 33966385. Edict: none (the closure ended with day 5); nothing to announce (day 8's edict is announced in day 7's state). Due today: the Dyers' 2 madder spoil at the close; C2-1 lapses after today if not accepted.

### Talk round, day 6: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-06-talk-Saltworks.md | 437f6ba80e7e7faf20f21f9e1c4a3ca5dc3739a17c6fa28be558323c99e372e1 |
| Weavers | to-seat/day-06-talk-Weavers.md | a2be15c7552ba9946d9847f3392b71038b018833825fc289b7afa318b8fc1682 |
| Smiths | to-seat/day-06-talk-Smiths.md | eccd0b1b1fe8d2d4042b4b7550a4fb577a1e3b96d4f97c026d47fdcd42aa1614 |
| Dyers | to-seat/day-06-talk-Dyers.md | 73ce1305ec8bdd8e6c188e598fa5b5979fbf5ef132c11f94207e26da39558c04 |
| Tanners | to-seat/day-06-talk-Tanners.md | b3dac216c8db6ea8aab60cd6d1e61aecfd672c11bdd2088f7546ad86d446cd40 |

### Talk replies, day 6 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-06-talk-Saltworks.md | 44b8734f30d413a386aee1d8e05212aae33f53eab66a9e1695c070e57e3b346f | valid; 3 messages (64, 71, 72 words) |
| Weavers | day-06-talk-Weavers.md | 6285f305acc48213edba337f6a4b50cd30ec29a0bf59cb7d80736326c0313258 | valid; 3 messages (72, 72, 69); Carbon Medium |
| Smiths | day-06-talk-Smiths.md | 652580dd20f30edd44b21cf0769f4f5e48f9e471a472284791fa0a4a164a5869 | valid; 1 message (57) |
| Dyers | day-06-talk-Dyers.md | d5c909aa211570b1436979508f5f9228859d878fbd795635b3a712fd23ef4c41 | valid; 3 messages (67, 45, 46) |
| Tanners | day-06-talk-Tanners.md | 0e35ce94366e67330563e7fa53553d9034bc86ce8313112cdf8f1ff68f664f29 | valid; 3 messages (19, 12, 19); relayed after one harness re-send |

**Tanners harness (R&D's report).** At 02:30 the first turn read in/day-06-r1.md (transcript step 74) and answered only in the chat, with no file: class (b), a delivery failure. ONE neutral re-send of the identical line at 02:33. The re-send wrote out/day-06-r1.md at 02:33:43 without re-reading (step 78): class (c). The file is the relayed reply.

Raw replies, verbatim:

```
TALK day 6, Saltworks
TO Tanners: C2-4 is live for your 2 salt (12 coin)! Today in ACT, submit "CONTRACTS: accept C2-4; bond C2-4" and "PAYMENTS: pay 12 coin to Saltworks". Day 9 is just a buffer: I hop to Ostra for brine today and pay your 2 salt once back in Varn (you receive them anywhere). If you visit Hath, we also buy timber at 6 to 7 coin!
TO Dyers: Huge thanks for the 3 timber! I am filling your O2-18 (16 coin for 2 salt) TODAY (day 6)! Move Hath to Mire and submit your dye conversion today so your 2 madder become 2 dye before tonight's spoilage! Also, we need 3 more timber for Varn salt barns: if you visit Hath on day 7 or 8, we will pay 7 coin per timber again (21 coin for 3 timber)!
TO Weavers: Congrats on 2 cloth! Today I fill Dyers' O2-18 so 2 more dye brew, and hop to Ostra to buy 2 brine @Ostra (please leave 2 brine today!). Hold your 3 brine from yesterday so on day 8 we swap 1 salt for 3 brine! Since timber took my coin, let's team up: you deliver 2 cloth to Varn salt barns for 6 points while I supply 3 salt and 3 timber!
```
```
TALK day 6, Weavers
TO Saltworks: I'm delivering my first 2 cloth at Ostra today while home, and weaving more once Dyers' next dye arrives! Meanwhile, I hold 3 brine now and am buying 3 more @Ostra today (6 total). You burn 1 timber whether brewing 1 or 2 salt, so take all 6 brine today: fill Dyers' O2-18, and post both 12 coin for 3 brine and 1 salt for 3 brine; I'll post both matching offers!
TO Dyers: Thanks for the 2 dye! Urgent spoilage warning: per Day 3's ruling, the 2 madder you bought Day 4 spoils at the close of TODAY (Day 6)! Move Hath to Mire today and submit CONVERT for 2 dye today; I'm pressing Saltworks to fill your O2-18 today so salt arrives before conversions. My O2-5 (10 coin for 1 dye) stays open, and tomorrow I'll bid 10 coin for a 2nd dye too!
TO Smiths: If you're moving Kell to Hath today toward Mire for C2-3, Dyers should be leaving Hath today! Even if Kell-Hath rolls a 2-coin toll, your remaining 6 coin buys 2 timber @Hath (2+4=6 coin). In the same Act, post 1 timber for 6 coin (crosses Saltworks' O2-2) and 1 timber for 5 coin (crosses my 5 coin for 1 timber offer today), turning 6 coin into 11 coin instantly!
```
```
TALK day 6, Smiths
TO Dyers: I hold 2 tools and have bonded 12 on C2-3. You haven't bonded yet. Please bond today. I'm heading via Hath to reach Mire on day 7 and will deliver then. I'll want payment the same day. Are you going to be in Mire? You don't need to be there for me to deliver, but pay promptly.
```
```
TALK day 6, Dyers
TO Saltworks: I sold you the timber, you brewed 2 salt yesterday. Please fill O2-18 (16 coin for 2 salt) today: I am going back to Mire and my madder spoils at the close of today, so converting today is the only chance. I will raise it to 18 coin for the 2 salt if you want, just say so. I will also buy 2 more on later days.
TO Smiths: I will bond C2-3 today: 2 tools at Mire for 12 coin, deadline day 7. If you can only reach Mire on day 8, tell me; I will propose a day-9 version. The tools also score for the Mire bridge, where I need dye too.
TO Weavers: Thanks, the 20 coin for 2 dye arrived. I make 2 more dye if Saltworks fills my salt today. O2-5 (10 coin for 1 dye) stays open; I may need 3 dye for the Mire bridge first, so I cannot promise more than 1 more dye.
```
```
TALK day 6, Tanners
TO Saltworks: I’m interested in your 2-salt contract (C2-4). I’ll accept and pay 12 coin. Let me know any next steps.
TO Smiths: No new trade offers needed now; good luck with your tools contract.
TO Dyers: If you need any hide or timber, I can look for market deals – just let me know price.
```

**Delivery:** all 13 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 2 (Saltworks, Dyers); Smiths 3 (Weavers, Dyers, Tanners); Dyers 4 (Saltworks, Weavers, Smiths, Tanners); Tanners 1 (Saltworks).

**Checks this round:**
- **GPT-OSS memory (Tanners):** no live contract. The reply refers to C2-4 correctly, as the Saltworks' 2-salt proposal at 12 coin that it intends to accept ("I'll accept and pay 12 coin"). It wishes the Smiths luck with "your tools contract" (C2-3, which isn't the Tanners'). No reference treats C2-2 as live. **Pass**; no stale contract this round.
- **Foreknowledge:** none. The Weavers cite "Day 3's ruling" (G2-9, published in day 4's state).
- For the record (not corrected): the Saltworks tell the Tanners "C2-4 is live", but it is proposed, not accepted. The Saltworks tell the Dyers they fill O2-18 "TODAY". The Weavers say they will deliver cloth at Ostra today. Play statements, not rule facts.

### Act round, day 6: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-06-act-Saltworks.md | 1626e3c86dc860049895ccd237a866a20b39538f403b2f5a16b831da1a169057 |
| Weavers | to-seat/day-06-act-Weavers.md | d8f33713cec936c9d364331b064196d98d4ebc0af028a56941a336736f82ec05 |
| Smiths | to-seat/day-06-act-Smiths.md | f24e09099af5823b1182b0f77389a6770d6ba0ca92e3753c2dbfedaa4d3e4104 |
| Dyers | to-seat/day-06-act-Dyers.md | de607d77bb5e9a2ec1ae2fe52e56ffa34a0ce6dea89514f54886245a5b7f05ec |
| Tanners | to-seat/day-06-act-Tanners.md | f0b9b00cb12236cf239d71d33f7ab96ae598649c92de609dae798629d30d37d8 |

### Act replies, day 6 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-06-act-Saltworks.md | 95b93f11fc81679ab70f13ea12ba77974a84733813defdd1c7674a3ec370d240 | valid (from the AI Studio Drive JSON, chunk 12) |
| Weavers | day-06-act-Weavers.md | 5c438e4ba51c939e9768582fb2da7612c33d3734d150b4e55a6b6214ddf19f38 | valid; Carbon Medium |
| Smiths | day-06-act-Smiths.md | 7ba4e8e8fd962f2deaa3711e850483d12ce64eac1c6142287864678b49020290 | valid |
| Dyers | day-06-act-Dyers.md | 77a07fd92ddedfe9f7e8e00de8aa42374c462de9d37c376a685b3fa1e148587f | valid |
| Tanners | day-06-act-Tanners.md | 05c7be689754e18692d3abd30fe5f592f349ab48d7754fb4adeeaf71c63fb193 | valid; class (a), steps 82–84, sent 02:43, no re-send |

Harness: none this round (R&D).

Raw replies, verbatim:

```
ACT day 6, Saltworks
OFFERS: post 7 coin for 2 brine @Ostra; post 2 salt for 16 coin
TRAVEL: go to Ostra
```
```
ACT day 6, Weavers
OFFERS:    post 5 coin for 1 timber; post 3 brine for 1 salt
WORKS:     deliver 2 cloth to Ostra lighthouse
TRAVEL:    stay
```
```
ACT day 6, Smiths
TRAVEL:    go to Hath
```
```
ACT day 6, Dyers
CONTRACTS: bond C2-3
CONVERT:   1 madder + 1 salt -> 1 dye x2
TRAVEL:    go to Mire
```
```
ACT day 6, Tanners
CONTRACTS: accept C2-4; bond C2-4
PAYMENTS: pay 12 coin to Saltworks
TRAVEL: stay
```

### Resolution, day 6

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 6, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | go to Ostra | 2083bebe0b135e9a34a60f98fe8aadd92689b8cd24f70a242e6705f5a121a65e |
| Weavers | stay | 4eae7d8ba916abf2b58f20b01c69ac6cccb7043c913297e2f059368664646380 |
| Smiths | go to Hath | 97ce885c68523422d3e1799f166ae03875d903171ed66058398bcd339cdd9d91 |
| Dyers | go to Mire | ef60030aacaf9be5c9b1dbbd3e5762fd16406cf1de1d151481d831cfaa1fe868 |
| Tanners | stay | 81c6f10da102be37c8d35c930ab69ba304f3c22738a4a6ecf05175ba66672a88 |

Legal: Varn–Ostra road 0, Kell–Hath road 2, Hath–Mire road 3. No edict in force (the closure ended with day 5).

**Step 2: the roll.** Round 33966385, randomness 4fceccc4…270ace3.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 4f ce | 20,430 | 11 | clear |
| 1 Ostra–Kell | safe | cc c4 | 52,420 | 1 | bandits (nobody travelled) |
| 2 Kell–Hath | hill | e2 41 | 57,921 | 2 | **bandits** |
| 3 Hath–Mire | safe | b2 c5 | 45,765 | 6 | clear |
| 4 Mire–Varn | marsh | 89 ce | 35,278 | 19 | clear |
| 5 Ostra–Mire | short cut | c5 a3 | 50,595 | 16 | clear |

(Check: 0xe241 = 57,921, mod 20 = 1, so 2: bandits on a hill road, board.md 1–2.)
Equal-price order: Tanners, Dyers, Smiths, Weavers, Saltworks.

**Step 3: road events, then moves.**
- Saltworks, road 0, clear: to Ostra.
- Smiths, road 2, **bandits**: "the caravan loses its single most valuable unit (highest works value, then highest reference price)" (board.md). Smiths held 2 tools (works value 3). **Lose 1 tools** (2 → 1). They move on to Hath; bandits don't stop a caravan.
- Dyers, road 3, clear: to Mire.
- Weavers, Tanners stay.

**Step 4: offers fill, contracts, payments** (G2-6: market orders, then guild crossings, older first).
- *Market order:* Saltworks `7 coin for 2 brine @Ostra`: in Ostra after the move. 1 + 3 = 4 ≤ 7. Fills: +2 brine, 7 → 3.
- *Guild offers.* New ids in reply order: O2-24 Saltworks 2 salt for 16 coin; O2-25 Weavers 5 coin for 1 timber; O2-26 Weavers 3 brine for 1 salt.
  - **Salt.** The oldest salt bid is O2-18 (Dyers, day 4, 16 coin for 2 salt). Its counter is O2-24 (Saltworks, 2 salt for 16); the Saltworks hold 4 salt. **Fills at 16:** Saltworks −2 salt, +16 (3 → 19); Dyers +2 salt, −16 (65 → 49). Closed: O2-18, O2-24. O2-26 (Weavers, 3 brine for 1 salt) has no salt ask left and stays open.
  - O2-2 and O2-25 (timber bids): nobody sells timber. O2-5 (dye bid): the Dyers' dye is made in step 5. Both stay open.
- *Contracts.*
  - Dyers `bond C2-3`: 12 coin (standing < 3). 49 → 37. C2-3 is now bonded by both sides (G2-14).
  - Tanners `accept C2-4; bond C2-4`. C2-4 is **agreed on day 6**: seller Saltworks, buyer Tanners, 2 salt at Varn, 12 coin, deadline day 9. Tanners' bond 12 (standing −3 < 3, full bond): 47 → 35. The Saltworks have not bonded.
  - C2-1 (proposed day 1, deadline day 6, never accepted): **lapses** at the close of day 6 (G2-7).
- *Payments* (applied after the offers; the protocol lists payments in the act form but not among the resolution steps, so they are taken with the step-4 transfers, recorded here): Tanners `pay 12 coin to Saltworks`: Tanners 35 → 23, Saltworks 19 → 31. Recorded against C2-4: **Paid = yes** (12 ≥ 12 by day 6). Paying first is the buyer's own risk (design.md).

**Step 5: conversions and works.**
- Dyers, Mire (home): `1 madder + 1 salt -> 1 dye x2`: 2 madder (bought day 4), 2 salt. Done: **+2 dye**; madder 0, salt 0.
- Weavers `deliver 2 cloth to Ostra lighthouse`: in Ostra. The lighthouse needs 3 cloth, 0 delivered. **Delivered 2 cloth** (works value 3 each = **6 points**, Weavers). Lighthouse cloth 2/3. Not complete.

**Step 6: perishables.** The Dyers' day-4 madder was used in step 5. Nothing left to spoil.

**Step 7: contract deadlines.** None of the agreed contracts is due (C2-3 day 7, C2-4 day 9). C2-1 lapsed (above).

**Step 8: ladders refill.**

### State after day 6

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Ostra | 31 | 0 | 2 salt, 2 brine, 2 timber | 0 | 0 |
| Weavers | Ostra | 19 | 0 | 6 wool, 3 brine | 0 | 6 |
| Smiths | Hath | 8 | 21 (9 own C2-2, back day 8; 12 C2-3) | 1 tools | 0 | 6 |
| Dyers | Mire | 37 | 12 (C2-3) | 2 dye | 0 | 0 |
| Tanners | Kell | 23 | 12 (C2-4) | — | −3 | 0 |

Coin check: 167 − 4 (ladder) = 163 = purses 118 + bonds 45. Goods destroyed: 1 tools to bandits (taken by the steward), 2 cloth into the lighthouse. Open offers: O2-2, O2-5, O2-25, O2-26. Contracts: C2-3 (deadline day 7, both bonded; the Smiths hold 1 of the 2 tools due), C2-4 (deadline day 9; Tanners bonded and paid; Saltworks unbonded). C2-1 lapsed.

**Rule gap G2-15 (new): do bandits hold a caravan?** design.md "The score" calls a force-majeure delay one where "the day's roll put a flood or bandits on the seller's road". But design.md's own bond rule, board.md ("A flood is the road event that holds") and the brief's settle table name a flood only. Provisional ruling: **only a flood holds.** The bond rule's table, the board and the seat brief are the procedure and the rules every seat was given, and they agree. The one design.md sentence that adds bandits is logged for the review. It becomes live at C2-3's deadline (today), since the Smiths lost 1 of the 2 tools due to bandits. Stated to every seat in day 7's state ("Only a flood holds…; bandits do not").

**Checks.**
- **GPT-OSS memory (Tanners):** C2-4 became live this round. The act accepts, bonds and pays it with the right id, amount (12) and counterparty (Saltworks). Consistent. **Pass.**
- **Foreknowledge:** none.

Referee word count, day 6: talk 4,551 + act 6,515.

---

## Day 7

Drand round 33966390. Edict: none today. **Announced in day 7's state** (setup.md "Edicts"): the day-8 edict, a festival at Hath; each unit delivered to a works in Hath on days 8 and 9 scores 1 more than its works value. Due today: C2-3 (Smiths to deliver 2 tools at Mire).

### Talk round, day 7: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-07-talk-Saltworks.md | e32e6152137cef31c0762f7857db707de2ffd363fcf19f4fe2f009133f89026d |
| Weavers | to-seat/day-07-talk-Weavers.md | 4640188111389d6f6f6217a24dae27101c52c52a4c9d7ef008bca222c974853c |
| Smiths | to-seat/day-07-talk-Smiths.md | f4affdbaf729afe9054aa8fab1f46ab7e6ddc99753ad672b2aeee404c4721454 |
| Dyers | to-seat/day-07-talk-Dyers.md | 873752d65a8084be54a0a96603970c6415a8ef612e076bf2c3267c52667dfcd0 |
| Tanners | to-seat/day-07-talk-Tanners.md | 0662672bf46aaf4876dd4cba903a5674d26ef7b39c347080697eedefef370bca |

### Talk replies, day 7 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-07-talk-Saltworks.md | 8d63bea8b0a3817652f48b7c8cc4e26357e9790d80411e6d00d016fc83c87ce5 | valid; 3 messages (65, 73, 73 words) |
| Weavers | day-07-talk-Weavers.md | 4a7aab1f1b5b1eac5f3a56e7ca8d535a44e6343f61aede1fd3bf848e6010747b | valid; 3 messages (69, 72, 67); Carbon Medium; about 8 min to reply, no stall |
| Smiths | day-07-talk-Smiths.md | 530ab7369e21f68024ace58d7f5ed3f6d5e05d54fe7b6372131b920f17957e23 | valid; 1 message (70) |
| Dyers | day-07-talk-Dyers.md | 53b746cb136af929b046e7e1ca830b4fdf8cdfa7f5a1e0677b62978d76a29945 | valid; 3 messages (62, 58, 48) |
| Tanners | day-07-talk-Tanners.md | d7366e7d0b2211f5517334eaaa23a7a9425dc87a45359b63cab7461e9d97dbe2 | valid; 3 messages (31, 21, 20); relayed after one harness re-send |

**Tanners harness (R&D's report).** At 02:49 the first turn read in/day-07-r1.md (step 88) and answered only in the chat: class (b), a delivery failure. ONE neutral re-send of the identical line at 02:50. It wrote out/day-07-r1.md without re-reading (step 92): class (c). Its chat line came back as "DONE out/day-07-r1.mdDONE". The file is the relayed reply.

Raw replies, verbatim:

```
TALK day 7, Saltworks
TO Smiths: Sorry bandits hit you! Since you are in Hath today ahead of tomorrow's festival, buy 3 timber @Hath today (12 coin cost) and in the same Act put "post 2 timber for 14 coin; post 1 timber for 6 coin"! Market buys fill before guild offers, so you cross our O2-2 and new 14 coin offer TODAY for 20 coin (an instant 8 coin profit)!
TO Weavers: Filling your O2-26 (1 salt for 3 brine) TODAY, and buying today's 3 brine @Ostra myself (please leave Ostra brine today!). Tip for dye: you hold 19 coin, so cancel O2-25 and post "9 coin for 1 dye" alongside O2-5 to buy both of Dyers' 2 dye today and weave 2 cloth today! Since Ostra lighthouse lacks 3 leather, please bring those 2 cloth to Varn salt barns on day 8 or 9!
TO Dyers: Nice job brewing 2 dye! Note Smiths lost 1 tool to bandits, so tonight you win Smiths' 12 coin bond on C2-3! Please sell your 2 dye to Weavers today ("post 1 dye for 10 coin; post 1 dye for 9 coin") so Varn's 2 cloth get woven before the bell! Grab fresh madder @Mire today (or 3 timber @Hath tomorrow for 20 coin if still open), and I brew more salt tomorrow!
```
```
TALK day 7, Weavers
TO Saltworks: I'm leaving Ostra today, so Ostra's 3 market brine are all yours if you stay! Better yet: move Ostra to Varn today and post 1 salt for 3 brine (crossing my O2-26) or 11 coin for 3 brine (I'll post 3 brine for 11 coin today). You'll land in Varn with 5 brine and brew 2 salt TODAY instead of losing a day, staying ahead on C2-4 and Varn!
TO Smiths: Rough bandit roll! Note Hath–Mire is safe (cannot flood to delay C2-3), whereas staying in Hath scores 4 pts on your tools in Day 8's festival! Cash tip today: stay in Hath, post 6 coin for 2 timber @Hath AND post 1 timber for 7 coin; 1 timber for 6 coin. That crosses my 7-coin timber bid today plus O2-2, turning 8 coin into 15 coin so you afford 2 charcoal tomorrow!
TO Dyers: Understood on prioritizing 3 dye for Mire bridge! My O2-5 (10 coin for 1 dye) stays open whenever you have a spare dye to sell. Also, in case Saltworks saves their salt for C2-4 and Varn, I have O2-26 open (3 brine for 1 salt): if Saltworks fills it, I can sell you that 1 salt for 8 coin so you can brew another dye in Mire!
```
```
TALK day 7, Smiths
TO Dyers: Bandits took 1 of my 2 tools on the Kell-Hath road, so I can't deliver 2 at Mire today. Bandits are not a flood, so I expect the breach to settle against me and you get my bond. I'm sorry. I can still sell you the other tool, or deliver it to the works you choose. I'll bring more tools once I have ore and charcoal. Want a new contract?
```
```
TALK day 7, Dyers
TO Smiths: Sorry about the bandits. C2-3 closes today with only 1 tool possible. If you reach Mire today with 1 tool, I will pay 6 coin for it on delivery, and I accept that the contract settles by the bond rule. I will not hold a grudge. Let me know your plan. I also still want 2 tools for the Mire bridge later.
TO Saltworks: I hold 2 dye and 53 coin. Please post 2 salt for 16 coin again when you next brew, delivered via offer, and I will buy it at once; I will also buy 2 more the day after. I will bring timber from Hath for your Varn barns when I go there. Which day do you expect salt?
TO Weavers: Your O2-5 wants 1 dye at 10. I need 3 dye for the Mire bridge, so I will sell 1 dye after I hold 4 in total. Can you sell me 1 salt for 8 coin? Your O2-26 offers 3 brine for salt, but I have no brine.
```
```
TALK day 7, Tanners
TO Saltworks: I'd like to buy timber to support leather production. Could you sell me 1 timber for 6 coin (your O2-2 offer) or a price you prefer? I have the coin ready.
TO Smiths: I need hide to convert into leather. Could you sell me 1 hide for 4 coin? I can meet at Kell.
TO Weavers: If you need leather later, I can supply after I get hide. Let me know any price you can offer.
```

**Delivery:** all 13 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 3 (Saltworks, Dyers, Tanners); Smiths 4 (Saltworks, Weavers, Dyers, Tanners); Dyers 3 (Saltworks, Weavers, Smiths); **Tanners 0** (nobody wrote to them).

**Checks this round:**
- **GPT-OSS memory (Tanners): C2-4 is live** (buyer; 2 salt at Varn, 12 coin, deadline day 9, which is within 2 days). The reply doesn't mention C2-4. The Tanners' own steps are done (bonded day 6, paid day 6); the step left is the Saltworks' delivery. Nothing contradicts the record, and it doesn't treat C2-4 as absent (it proposes no duplicate salt deal). It's not the deadline day. No stale reference to C2-2 ("I need hide to convert into leather"). **Pass.**
- **Foreknowledge:** none. The Saltworks and Weavers refer to the day-8 Hath festival, announced in day 7's state.
- For the record (not corrected): the Dyers write "I hold 2 dye and 53 coin"; their purse is 37, with 12 bonded. The Weavers' claim that Hath–Mire "cannot flood" matches board.md (a safe road has no flood).

### Act round, day 7: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-07-act-Saltworks.md | 1c5cba78cc7d3d04ff39321635c7e39f79213baa56fc196fc9c41e4c20ec23aa |
| Weavers | to-seat/day-07-act-Weavers.md | dfd63de94baea3521c27810e063d86d9e5c15a33c09f71d1d045d92234749281 |
| Smiths | to-seat/day-07-act-Smiths.md | 30f6139ab98b2532a3023028914379a7ffc3067956e08979a34ee038a850eb59 |
| Dyers | to-seat/day-07-act-Dyers.md | 99eeebe7e587e69892f14ff4ca3deb320488e3611452aeeae88eddc9a90ca65d |
| Tanners | to-seat/day-07-act-Tanners.md | 1a04f2d621a2049f031966882c47198d9a519367c1ac4c1db45f6f1a77168e3a |

### Act replies, day 7 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-07-act-Saltworks.md | 3a2a6812ea1420cb4c1a7f67348dc3d0ac43c3dba5d3e3d541cca2e12aacdcbf | valid (Drive JSON, chunk 14) |
| Weavers | day-07-act-Weavers.md | b86e7b21074b43743662e14da9b562e06c957c699398be0dcaf473a9def531d3 | valid; Carbon Medium |
| Smiths | day-07-act-Smiths.md | 8f334e9b932c81c7fe596e0ca2b7bcd7f673c62d45acfec1bedcaadc49f8e41f | valid |
| Dyers | day-07-act-Dyers.md | 937750b5d6c0f126ab686a4daede72a4039b3fa25b03b6eadb3c08ec80da1076 | valid |
| Tanners | day-07-act-Tanners.md | 1e73127532f8e625fd60515aff849564496684356edda3d30de32b7844ee77c8 | valid; class (a), steps 96–98, sent 02:59, no re-send |

Harness: none this round (R&D).

Raw replies, verbatim:

```
ACT day 7, Saltworks
OFFERS: post 12 coin for 3 brine @Ostra; post 1 salt for 3 brine; post 14 coin for 2 timber
TRAVEL: stay
```
```
ACT day 7, Weavers
OFFERS:    cancel O2-25; post 7 coin for 1 timber
WORKS:     deliver 1 salt to Varn salt barns
TRAVEL:    go to Varn
```
```
ACT day 7, Smiths
OFFERS:    post 6 coin for 1 charcoal @Hath
TRAVEL:    stay
```
```
ACT day 7, Dyers
OFFERS:    post 6 coin for 2 madder @Mire; post 16 coin for 2 salt; post 6 coin for 1 tools
CONTRACTS: propose 1 dye at Mire for 10 coin by day 11 to Weavers
WORKS:     deliver 2 dye to Mire bridge
TRAVEL:    stay
```
```
ACT day 7, Tanners
OFFERS:    post 6 coin for 1 timber
TRAVEL:    stay
```

### Resolution, day 7

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 7, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | stay | 132d65c0be35cb05891a531cb87dbc8e22d0802e9768f850dc3b14f4a089c33c |
| Weavers | go to Varn | 9aee45700d3cc09c09035c357c05f9050aed179312b4a770fcbe30f7c6cbac86 |
| Smiths | stay | f9a30f6abf3065aab387fc646e39e076bd8222ac18bceffa8896fe35b3067dbc |
| Dyers | stay | 9dddd85d9c7d4a96c7638d996f33f3a3bb2d1a15172bea774c5e750abf7882e3 |
| Tanners | stay | 98715745e19a949e3df32c09ca1a69949a42cbbb4b3192b7d6034c0940b39e4e |

Legal: Ostra–Varn is road 0.

**Step 2: the roll.** Round 33966390, randomness 93bf83a6…f4c848.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 93 bf | 37,823 | 4 | clear |
| 1 Ostra–Kell | safe | 83 a6 | 33,702 | 3 | clear |
| 2 Kell–Hath | hill | 3e 28 | 15,912 | 13 | clear |
| 3 Hath–Mire | safe | 95 e7 | 38,375 | 16 | clear |
| 4 Mire–Varn | marsh | 94 84 | 38,020 | 1 | bandits (nobody travelled) |
| 5 Ostra–Mire | short cut | 8b cd | 35,789 | 10 | clear |

Equal-price order: Smiths, Tanners, Weavers, Dyers, Saltworks.

**Step 3: moves.** Weavers, road 0, clear (a safe road's 3–4 is clear): to Varn. Others stay.

**Step 4: offers fill** (cancellations first, then G2-6).
- *Cancellation:* O2-25 (Weavers).
- *Market orders:*
  - Dyers `6 coin for 2 madder @Mire`: 2 + 4 = 6. Fills: +2 madder (bought day 7), 37 → 31.
  - Saltworks `12 coin for 3 brine @Ostra`: in Ostra (the Weavers have left). 1 + 3 + 5 = 9 ≤ 12. Fills: +3 brine (5), 31 → 22.
  - Smiths `6 coin for 1 charcoal @Hath`: in Hath. 6. Fills: +1 charcoal, 8 → 2.
- *Guild offers.* New ids in reply order: O2-27 Saltworks 1 salt for 3 brine; O2-28 Saltworks 14 coin for 2 timber; O2-29 Weavers 7 coin for 1 timber; O2-30 Dyers 16 coin for 2 salt; O2-31 Dyers 6 coin for 1 tools; O2-32 Tanners 6 coin for 1 timber.
  - **Brine for salt.** O2-26 (Weavers, day 6: 3 brine for 1 salt) is the older side; O2-27 (Saltworks: 1 salt for 3 brine) crosses it exactly. The Weavers hold 3 brine and the Saltworks hold 2 salt. **Fills:** Weavers −3 brine, +1 salt; Saltworks −1 salt (1), +3 brine (8). Both closed.
  - Salt for coin: O2-30 (Dyers) has no ask. Timber: O2-2, O2-28, O2-29, O2-32 are all bids, with no seller. Tools: O2-31 has no ask (the Smiths posted none). Dye: O2-5 has no ask. All stay open.
- *Contracts.* Dyers `propose 1 dye at Mire for 10 coin by day 11 to Weavers`: well formed. Recorded as **C2-5**: seller Dyers, buyer Weavers, 1 dye at Mire, 10 coin, deadline day 11. Proposed.
- *Vendetta:* not called. The Smiths' window on the Tanners runs to day 8's act round.

**Step 5: conversions and works.**
- Dyers `deliver 2 dye to Mire bridge`: in Mire, hold 2 dye. The bridge needs 3 dye, 0 delivered. **Delivered 2 dye** (works value 3 each = **6 points**, Dyers). Bridge dye 2/3.
- Weavers `deliver 1 salt to Varn salt barns`: in Varn after the move; hold 1 salt (from the step-4 crossing, G2-6). The barns need 3 salt, 0 delivered. **Delivered 1 salt** (works value 2 = **2 points**, Weavers). Barns salt 1/3.
- No conversions ordered.

**Step 6: perishables.** The Dyers' 2 madder (bought day 7) are gone at the close of day 9. Nothing spoils on day 7.

**Step 7: contract deadlines.** C2-3 (seller Smiths, buyer Dyers, 2 tools at Mire, 12 coin, agreed day 5, deadline day 7), by the bond rule:
- **Delivered:** no. The Smiths paid no tools to the Dyers, and their caravan was never in Mire during the contract.
- **Paid:** no.
- **Held:** no. Between agreement (day 5) and the deadline the Smiths travelled Hath → Kell (day 5, clear) and Kell → Hath (day 6, **bandits**). Bandits do not hold (G2-15; board.md "A flood is the road event that holds"). No flood met them.
- **Outcome: Seller's breach.** The Dyers receive the Smiths' 12-coin bond: 31 → 43. Smiths' standing 0 → **−3**. The Dyers' own 12-coin bond comes back on day 11 (G2-12). The Dyers gain a vendetta right on the Smiths (Mire's market), to be called by the act round of day 11 (G2-13).
- This is where G2-15 decided an outcome. Had bandits held, C2-3 would have been delayed 2 days (to day 9) at no standing cost.

**Step 8: ladders refill.** At the start of day 8, the Smiths' own C2-2 bond (9 coin) comes back, on the fourth day after C2-2's day-4 deadline (G2-12): Smiths 2 → 11.

### State after day 7 (with day 8's bond return)

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Ostra | 22 | 0 | 1 salt, 8 brine, 2 timber | 0 | 0 |
| Weavers | Varn | 19 | 0 | 6 wool | 0 | 8 |
| Smiths | Hath | 11 | 0 | 1 tools, 1 charcoal | −3 | 6 |
| Dyers | Mire | 43 | 12 (own C2-3, back day 11) | 2 madder (gone at the close of day 9) | 0 | 6 |
| Tanners | Kell | 23 | 12 (C2-4) | — | −3 | 0 |

Coin check: 163 − 6 − 9 − 6 (ladders) = 142 = purses 118 + bonds 24. Breaches: Tanners 1, Smiths 1. Open offers: O2-2, O2-5, O2-28, O2-29, O2-30, O2-31, O2-32. Contracts: C2-4 agreed (deadline day 9); C2-5 proposed (deadline day 11).

**Checks.**
- **GPT-OSS memory (Tanners):** C2-4 is live, deadline day 9. The act (`post 6 coin for 1 timber`, `stay`) doesn't mention it. Its own steps are done, nothing contradicts the record, and it isn't the deadline day. **Pass.**
- **Foreknowledge:** none.

Referee word count, day 7: talk 4,094 + act 6,105 = 10,199.

---

## Day 8

Drand round 33966395. **Edict in force: festival at Hath** (days 8 and 9): each unit delivered to a works in Hath scores 1 more than its works value (setup.md "Edicts"; announced in day 7's state). Nothing further to announce. Due: the Smiths' vendetta window on the Tanners closes with day 8's act round; C2-4's deadline is day 9.

### Talk round, day 8: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-08-talk-Saltworks.md | 61b56731b84aa0617cbfd9a180f29750c8392a5a4ec614dc727b62ffca974687 |
| Weavers | to-seat/day-08-talk-Weavers.md | 761d66156ceb1a10c0857e951c712525de17afc81f0f5e7672803fa0b384da66 |
| Smiths | to-seat/day-08-talk-Smiths.md | 0bc85efb2e0bbf108fb1f8b8c6dab0c048ab351bde505bc04972cc417df8c11c |
| Dyers | to-seat/day-08-talk-Dyers.md | 79369cc881ec50463119f5921ceb6c4d79b97872c8d675d1b6e8a8128385fd06 |
| Tanners | to-seat/day-08-talk-Tanners.md | 1bad894693044031bd2ebdeb87f9abfe13913879cec45d45946767bee9195bd0 |

### Talk replies, day 8 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-08-talk-Saltworks.md | db753404244ce5cb06a2a8f583f70b7197444bb2fe2baca14104c33d37c80bcd | valid; 3 messages (71, 72, 71 words); Drive JSON chunk 15 |
| Weavers | day-08-talk-Weavers.md | 6dc565cb6f592909848d50e6c8d969b5099a43b577a0e316885329627f0fd8a6 | valid; 3 messages (70, 70, 74); Carbon Medium; about 5 min |
| Smiths | day-08-talk-Smiths.md | 985cbc38bb341f772c56779ae125d12fc87920bfcc0827b3f671f8ae8c4dec50 | valid; 1 message (63) |
| Dyers | day-08-talk-Dyers.md | 4f81085dfb2e297172fac841aa46a68daa9bd277e237de32189e5f334d2e2fc9 | valid; 3 messages (61, 53, 48) |
| Tanners | day-08-talk-Tanners.md | 8ecbd10161e610bfd882cd8fa5a245aaf0995227e110f346e08ff986abbb56e3 | valid; 3 messages (21, 17, 21); class (a), no re-send |

**Tanners (R&D's report).** Sent 03:08:45. The turn read in/day-08-r1.md (step 102), put out garbled chat text (step 103), then in the same turn wrote out/day-08-r1.md (step 104) and the DONE line. Class (a), no re-send, no harness. The garbled preamble is noted in R&D's seating log.

Raw replies, verbatim:

```
TALK day 8, Saltworks
TO Dyers: Moving to Varn today holding 1 salt now, and brewing 2 more tonight! Since O2-30 asks for 2 salt at once, put "post 9 coin for 1 salt" in OFFERS today so you get 1 salt TODAY, brew 1 dye today in Mire, and fill Weavers' O2-5 (10 coin for 1 dye) so Varn's 2 cloth start weaving! Keep another salt buy open for day 10 when my next batch finishes!
TO Weavers: Awesome delivering 1 salt to Varn salt barns! Head back to Ostra today while I move to Varn to brew 2 salt tonight and 2 more tomorrow. I am selling Dyers 1 salt today (and another day 10) and urged them to fill your O2-5 right away so you can weave 2 cloth for Varn salt barns! Also grab 3 brine @Ostra tomorrow (day 9) and I'll buy them for 12 coin!
TO Smiths: Your 9-coin C2-2 bond came back today! While staying in Hath today for your festival delivery, buy all 3 timber @Hath today ("post 12 coin for 3 timber @Hath") and in the same Act put "post 2 timber for 14 coin; post 1 timber for 7 coin"! Market buys fill before guild offers, so you cross our O2-28 and Weavers' O2-29 TODAY for 21 coin (an instant 9 coin free profit)!
```
```
TALK day 8, Weavers
TO Smiths: With your C2-2 bond back today, you hold 11 coin in Hath, 1 short of 2 charcoal. Profit play while delivering tools for 4 festival points today: post 6 coin for 1 charcoal @Hath; post 5 coin for 1 timber @Hath (posting 5 wins priority while costing only 2!); post 1 timber for 9 coin. That crosses my 9-coin timber bid today, giving you +7 coin profit plus charcoal today!
TO Dyers: With only 19 coin total, I can't lock 10 bond plus 10 payment into C2-5. Instead, I'm keeping O2-5 (10 coin for 1 dye) open so you get 10 coin immediately with zero bond lockup whenever you sell! Also, Saltworks holds only 1 salt today and owes 2 on C2-4 tomorrow, so O2-30 can't fill today: post 8 coin for 1 salt too so Saltworks can sell 1 salt sooner!
TO Saltworks: Dyers delivered both dye to Mire bridge yesterday, so with no dye for cloth I delivered that 1 salt to get Varn salt barns started! I'm heading back toward Ostra to weave once Dyers fills O2-5. Tip: since you hold 1 salt before tonight's brew, I suggested Dyers post 8 coin for 1 salt today in case you want quick coin and faster dye! Will you want Ostra brine on Day 9 or 10?
```
```
TALK day 8, Smiths
TO Dyers: Thanks for taking the breach well. I'd like to keep Mire's market open to me, so please don't call a vendetta. I'll rebuild tools: I have 1 charcoal and need ore from Kell. Once I have 2 more tools I'll offer them to you for the Mire bridge at 6 coin each, delivered Mire. The tool I hold goes to Hath's festival today.
```
```
TALK day 8, Dyers
TO Saltworks: I have 2 madder that spoil at the close of tomorrow. O2-30 (16 coin for 2 salt) is open; I will raise it to 10 coin each, 20 for 2 if that gets salt to me by today or tomorrow. I also sell you 2 timber later if you still need them. Do you have any salt not owed to Tanners?
TO Weavers: Please accept C2-5 (1 dye at Mire for 10 coin, by day 11). It builds standing for us both. If you hold any spare salt, I will pay 9 coin for 1, so I can brew 2 more dye from my madder; your cloth needs dye, and I will sell you the extra.
TO Smiths: No hard feelings about C2-3; I will not call a vendetta. O2-31 (6 coin for 1 tools) is open for your surviving tool. Fill it and I deliver it to the Mire bridge. When you have ore and charcoal, I will buy a second tool at 6 coin.
```
```
TALK day 8, Tanners
TO Saltworks: I appreciate the salt deal. Could you also sell me 1 hide @Ostra for 4 coin? I’ll buy once I’m there.
TO Smiths: I need 1 hide for leather conversion. Could you sell me 1 hide @Kell for 4 coin?
TO Weavers: I have an open offer (O2-32) for 1 timber @6 coin. If you’re interested, let me know, otherwise I’ll cancel it.
```

**Delivery:** all 13 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 3 (Saltworks, Dyers, Tanners); Smiths 4 (Saltworks, Weavers, Dyers, Tanners); Dyers 3 (Saltworks, Weavers, Smiths); **Tanners 0** (second day running with no messages to them).

**Checks this round:**
- **GPT-OSS memory (Tanners): C2-4 live**, deadline day 9 (tomorrow). "I appreciate the salt deal" refers to it with no contradicting term. The Tanners' own steps are done. **Pass.** For the record (not corrected, and not a contract term): the Tanners ask the Saltworks for "1 hide @Ostra" (Ostra sells no hide) and the Smiths for "1 hide @Kell".
- **Foreknowledge:** none. Festival references (Saltworks, Weavers, Smiths) follow its announcement in day 7's state.
- For the record (not corrected): the Dyers tell the Smiths "I will not call a vendetta". The Weavers decline C2-5 in talk.

### Act round, day 8: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-08-act-Saltworks.md | c8f4df97ef470c062eadc0e9474a1f0c7655c2fa157471798e5a2acffeb50a21 |
| Weavers | to-seat/day-08-act-Weavers.md | 87390c26fd531e9240c10aeccaae1df43b55417d92f173cd197a8ddff03dc22b |
| Smiths | to-seat/day-08-act-Smiths.md | a49c7fa53ab76d81634ae5531becd44970595e1349c02332ea075da514d5319e |
| Dyers | to-seat/day-08-act-Dyers.md | 28c89eefa2a7d27e704d476dbf9108c89fed3464de3421789b47a8a36ea3d84b |
| Tanners | to-seat/day-08-act-Tanners.md | 557726b8ac9de8c303871980baa44ff15027a8a198244294632b50218f89182d |

### Act replies, day 8 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-08-act-Saltworks.md | fac8ae5bc00fcb2c96ed20f540d83c21c72dee33eb6bf2ae08ea33fd2b321e1c | valid (Drive JSON, chunk 16) |
| Weavers | day-08-act-Weavers.md | 45a7fb4d4c8cc02185ee8b387e8b479c8c64d475d6afe901c1e67d708a2a7b34 | valid; Carbon Medium |
| Smiths | day-08-act-Smiths.md | 06d0384c1c09da895d5167874c2986fce3b111bcc0e844bdf58f56a0b14fc305 | valid |
| Dyers | day-08-act-Dyers.md | 6964b188603ca6526c357894c7e8445e0583fcfafa214972e375e5ce298c4f52 | valid |
| Tanners | day-08-act-Tanners.md | f078ea7421cabcd4f0641fbdb5b9e442a0643fe1de7e557ac9d2ee07156af4e7 | valid; class (a), steps 108, 110, sent 03:15:28, no re-send; file has no trailing newline (as written) |

Harness: none this round (R&D).

Raw replies, verbatim:

```
ACT day 8, Saltworks
OFFERS: post 1 salt for 8 coin
CONVERT: 2 brine -> 1 salt x2
TRAVEL: go to Varn
```
```
ACT day 8, Weavers
OFFERS:    cancel O2-29; post 9 coin for 1 timber
TRAVEL:    go to Ostra
```
```
ACT day 8, Smiths
OFFERS:    post 6 coin for 2 timber @Hath; post 2 timber for 14 coin
WORKS:     deliver 1 tools to Hath watchtower
TRAVEL:    stay
```
```
ACT day 8, Dyers
OFFERS:    cancel O2-30; post 10 coin for 1 salt
CONVERT:   1 madder + 1 salt -> 1 dye x1
WORKS:     deliver 1 dye to Mire bridge
TRAVEL:    stay
```
```
ACT day 8, Tanners
TRAVEL: stay
```

### Resolution, day 8

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 8, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | go to Varn | 9803b1661a22f705c1e432f553dc9f97ddd3b62a9deab8386f1efb861fd20a69 |
| Weavers | go to Ostra | 5155a7119e567e72df0174ed88d5c4ecb9c8ebdda3be43c8c851d0d004c2d1b3 |
| Smiths | stay | 3413cc4b8f753041ff50f575dee20273034bbaffdc9b49ec488c674eeddec5e5 |
| Dyers | stay | 5eb226344c319163afc66e1ffc6317a2e7b6bbafaf6086881d4b65bd7817af52 |
| Tanners | stay | 93cd5cceda256017732a7f2fb09dad22c2e994e59328a2e2ad7c6799f640bf1d |

Legal: both moves are on road 0 (Varn–Ostra), in opposite directions.

**Step 2: the roll.** Round 33966395, randomness 0efeeefd…a92ea3c.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 0e fe | 3,838 | 19 | clear |
| 1 Ostra–Kell | safe | ee fd | 61,181 | 2 | bandits (nobody travelled) |
| 2 Kell–Hath | hill | 22 0c | 8,716 | 17 | clear |
| 3 Hath–Mire | safe | a5 56 | 42,326 | 7 | clear |
| 4 Mire–Varn | marsh | 69 a8 | 27,048 | 9 | clear |
| 5 Ostra–Mire | short cut | 56 77 | 22,135 | 16 | clear |

Equal-price order (and same-day seniority, G2-16): Smiths, Dyers, Saltworks, Tanners, Weavers.

**Step 3: moves.** Saltworks to Varn, Weavers to Ostra, both clear. Others stay.

**Step 4: offers fill** (cancellations first, then G2-6).
- *Cancellations:* O2-29 (Weavers), O2-30 (Dyers).
- *Market order:* Smiths `6 coin for 2 timber @Hath`: in Hath. 2 + 4 = 6. Fills: +2 timber, 11 → 5.
- *Guild offers.* New ids in reply order: O2-33 Saltworks 1 salt for 8 coin; O2-34 Weavers 9 coin for 1 timber; O2-35 Smiths 2 timber for 14 coin; O2-36 Dyers 10 coin for 1 salt.
  - **Timber**, oldest first: O2-2 (Saltworks, day 1, 6 a unit) against the only ask, O2-35 (2 for 14, 7 a unit): 6 < 7, no cross. O2-28 (Saltworks, day 7, 14 coin for 2 timber) against O2-35: exact. **Fills:** Smiths −2 timber, +14 (5 → 19); Saltworks +2 timber, −14 (22 → 8). O2-32 (Tanners, 6) and O2-34 (Weavers, 9) find no ask left and stay open.
  - **Salt.** O2-33 (Saltworks: 1 salt for 8 coin) and O2-36 (Dyers: 10 coin for 1 salt) cross: the bid is above the ask. Both were posted the same day, at different prices. Ruling **G2-16** (new): when same-day offers cross at different prices, the offer of the guild earlier in the day's roll order counts as the older one and sets the price, as if it had been on the book first. Reason: the board gives the roll order for ties among offers. Reply order, which is only the referee's listing order, would hand the gap to whoever is listed first. The Dyers come before the Saltworks today, so the trade is at **10**. The Saltworks hold 1 salt. **Fills:** Saltworks −1 salt (0), +10 (8 → 18); Dyers +1 salt, −10 (43 → 33). Stated to every seat in day 9's state.
- *Contracts and payments:* none. *Vendetta:* the Smiths did not call one on the Tanners; their window (to day 8's act round, G2-13) closes.

**Step 5: conversions and works.**
- Saltworks, Varn: `2 brine -> 1 salt x2`: 4 brine (holds 8), burns 1 timber (holds 4). Done: brine 4, timber 3, **+2 salt** (2).
- Dyers, Mire: `1 madder + 1 salt -> 1 dye x1`: holds 2 madder (day 7) and 1 salt. Done: **+1 dye**; madder 1, salt 0. Then `deliver 1 dye to Mire bridge`: in Mire. **Delivered 1 dye** (3 points, Dyers). Bridge dye 3/3. The bridge still needs 4 timber and 2 tools, so it isn't complete.
- Smiths `deliver 1 tools to Hath watchtower`: in Hath, hold 1 tools. **Festival** (setup.md, day 8): works value 3 + 1 = **4 points**, Smiths. Watchtower tools 3/3. It still needs 2 leather and 2 dye, so it isn't complete.

**Step 6: perishables.** The Dyers' last madder (bought day 7) is gone at the close of day 9. Nothing spoils on day 8.

**Step 7: contract deadlines.** None (C2-4 is due day 9; C2-5 is a proposal).

**Step 8: ladders refill.**

### State after day 8

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Varn | 18 | 0 | 2 salt, 4 brine, 3 timber | 0 | 0 |
| Weavers | Ostra | 19 | 0 | 6 wool | 0 | 8 |
| Smiths | Hath | 19 | 0 | 1 charcoal | −3 | 10 |
| Dyers | Mire | 33 | 12 (own C2-3, back day 11) | 1 madder (gone at the close of day 9) | 0 | 9 |
| Tanners | Kell | 23 | 12 (C2-4) | — | −3 | 0 |

Coin check: 142 − 6 (ladder) = 136 = purses 112 + bonds 24. Open offers: O2-2, O2-5, O2-31, O2-32, O2-34. Contracts: C2-4 due day 9 (the Saltworks are in Varn with 2 salt); C2-5 proposed.

**Checks.**
- **GPT-OSS memory (Tanners):** C2-4 is due tomorrow. The act is `TRAVEL: stay` only. The Tanners' own steps are done, and the step due is the seller's. Nothing contradicts the record. **Pass.**
- **Foreknowledge:** none.

Referee word count, day 8: talk 4,655 + act 6,649 = 11,304.

---

## Day 9

Drand round 33966400. **Edict in force: festival at Hath** (day 9, its last day). Nothing to announce. Due at the close: C2-4 (Saltworks deliver 2 salt at Varn to the Tanners); the Dyers' last madder spoils.

### Talk round, day 9: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-09-talk-Saltworks.md | 154ed5845bd8a8d2ca522841904ade844e6e3b3529a72c9c05a1afe8d01b5c47 |
| Weavers | to-seat/day-09-talk-Weavers.md | 00d422ff5c1d2a8230e7920ef6a29266169a4a84d249b9cdfbb7a89aaa904ccb |
| Smiths | to-seat/day-09-talk-Smiths.md | ab68a7eaff863aa9c3917b0e52d44369a920bbe5258796b6704a3ae731fc9d8e |
| Dyers | to-seat/day-09-talk-Dyers.md | 9f3bbac01e6aa69802cb660f45b8518ddc400a3c52a91f56f484ac62e6c298bd |
| Tanners | to-seat/day-09-talk-Tanners.md | 8693f204d7c55e3a95abeb30c42b000bc5632fb27816fd974d92dc797fb79b8a |

### Talk replies, day 9 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-09-talk-Saltworks.md | 0cb7442645dd99b64eb27d5619a0ca553939b14d1041e095279f0b68440008d0 | valid; 3 messages (67, 73, 69 words); Drive JSON chunk 17 |
| Weavers | day-09-talk-Weavers.md | 4deb9a8b2a2c3548de43629c7e24a14b139f4a6f8e4ea3e3fd996a171f3befa8 | valid; 3 messages (73, 73, 72); Carbon Medium; about 9 min, no stall |
| Smiths | day-09-talk-Smiths.md | 8210b437e598bf70eedbcc89fc01d95c9acaea2bfb5af470e577a30c05828392 | valid; 1 message (52) |
| Dyers | day-09-talk-Dyers.md | af8ff157a5720a93e8169d0968f73da5119cd00c8b141b7bdcd3334fba25cdc3 | valid; 2 messages (63, 48) |
| Tanners | day-09-talk-Tanners.md | d3308fa0f0a33e9b9082575dd70a1b41ce9dbfa20d59c1de64721f0c17929a07 | valid; 3 messages (16, 17, 14); class (a), steps 114–116, sent 03:22:53 |

Harness: none (R&D).

Raw replies, verbatim:

```
TALK day 9, Saltworks
TO Weavers: Yes! Buy 3 brine @Ostra TODAY (day 9) and in the same Act put "post 3 brine for 12 coin"; I post "12 coin for 3 brine" today so you gain +3 coin right away for dye! Today I pay Tanners' C2-4 and brew 2 fresh salt tonight. If Dyers commits to selling you 2 dye, I'll supply Dyers salt tomorrow so you weave Varn's 2 cloth!
TO Dyers: Congrats on completing 3/3 dye at Mire bridge! Today my 2 salt must settle Tanners' C2-4 deadline, and I brew 2 fresh salt tonight in Varn (ready tomorrow, day 10). Since Varn salt barns still needs 2 cloth, will you sell your next 2 dye to Weavers (I'm sending Weavers 12 coin today so they can pay well)? If so, grab fresh madder @Mire and post 20 coin for 2 salt for tomorrow!
TO Tanners: Delivering your 2 salt in Varn TODAY ("pay 2 salt to Tanners") to complete our C2-4 contract right on schedule! You will hold 2 salt at the close of today. Friendly reminder: while in Kell you can buy fresh hide from Kell's market ("post 6 coin for 2 hide @Kell"), then move to Hath to convert 2 leather for 6 works points at Hath watchtower or Kell forge hall!
```
```
TALK day 9, Weavers
TO Smiths: With 19 coin in Hath today, here's an easy +9 coin profit alongside charcoal: stay in Hath, post 12 coin for 2 charcoal @Hath; post 7 coin for 2 timber @Hath, and in the same Act post 1 timber for 9 coin; post 1 timber for 6 coin. Market buys fill first (18 coin), then you cross my O2-34 (9 coin) and O2-2 (6 coin), ending today with 3 charcoal and 16 coin!
TO Saltworks: Please post 12 coin for 3 brine today (and keep it open onto Day 10 in case I step out one day first: you already hold 4 brine for tonight's 2 salt, and Day 10 guild fills still arrive before Day 10 conversions!). Also, Dyers put yesterday's dye into Mire bridge (now 3/3), so Dyers holds 0 dye: weaving 2 cloth needs 2 dye, so please sell Dyers 2 salt across Days 9-10!
TO Dyers: Congrats finishing Mire bridge's dye! With the bell possible from Day 10, C2-5's bond lockup until Day 15 freezes coin, whereas my open O2-5 (10 coin for 1 dye) pays you immediately with zero bond! I'm urging Saltworks to sell you 2 salt: stay in Mire today, buy 1 madder @Mire, post 10 coin for 1 salt (plus 9 coin for 1 salt), and submit CONVERT before your Day 7 madder spoils!
```
```
TALK day 9, Smiths
TO Dyers: I'm heading to Kell to make 1 tool, then to Mire for your bridge. Your O2-31 (6 coin for 1 tools) stays open, and I'll fill it on delivery at Mire, around day 11. Please keep that offer up. Thanks for skipping the vendetta. I'll keep my side clean from now on.
```
```
TALK day 9, Dyers
TO Saltworks: I will propose a contract in Act: I deliver 2 timber at Hath for 12 coin, deadline day 11, bond 12 each. You wanted timber for the Varn barns, and I will buy it at Hath and deliver on day 10. Please accept and bond when you see it. Also sell me 1 salt on day 10 for 9 coin if you can.
TO Weavers: C2-5 is too heavy on bonds for you. I will propose a lighter contract: 1 dye at Mire for 7 coin, deadline day 12, so your bond is only 7. A kept contract raises both our standings, and standing halves bonds at 3. Please accept if you can.
```
```
TALK day 9, Tanners
TO Saltworks: Congratulations on finishing your contract. I’ll keep an eye on any future trade offers you have.
TO Smiths: Good work on the watchtower. If you need any leather for bellows, let me know my price.
TO Dyers: Your dye deliveries are impressive; I might need some dye soon—let me know pricing.
```

**Word counts (G2-1).** R&D flagged the Weavers' messages as possibly long. The referee's count, the same method every day (whitespace-separated words of the text after `TO <guild>:`), gives 73, 73 and 72, all within 80. All 12 messages are delivered. Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 2 (Saltworks, Dyers); Smiths 2 (Weavers, Tanners); Dyers 4 (Saltworks, Weavers, Smiths, Tanners); Tanners 1 (Saltworks).

**Checks this round:**
- **GPT-OSS memory (Tanners): C2-4 live, deadline today (day 9).** The reply to the Saltworks says "Congratulations on finishing your contract." Against the record, C2-4 is not finished: the Saltworks haven't delivered, and the Tanners' own day-9 block said "deadline day 9 (today)" and "settled at the close of today". On the deadline day the reply mentions the contract (so not silence), but states its status wrongly: it treats a live contract as done. That is the mirror of day 5's stale-contract case, under point 3 ("a stated term that contradicts the record") and the day-5 addendum. **Logged as a GPT-OSS memory fault (live contract treated as finished), harness, counted apart.** Alternative reading recorded for the review: courtesy or anticipation of a delivery the Saltworks had promised in an earlier talk round. The Saltworks' same-round message announcing delivery today hadn't yet been delivered to the Tanners. The referee takes the literal reading, as on day 5. No action of the Tanners' is due on C2-4 (they bonded and paid on day 6), so it changes nothing in play.
- **Foreknowledge:** none. The Weavers' "With the bell possible from Day 10" is in the brief (the bell rings at the close of day 10, 11, 12 or 13). Nothing about the sealed bell day appears.
- For the record (not corrected): the Saltworks tell the Tanners they will deliver their 2 salt in Varn today. The Weavers claim C2-5's bonds would be locked "until Day 15"; by board.md rule 4, a kept contract's bonds come back on the fourth day after the deadline (day 11 + 4).

### Act round, day 9: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-09-act-Saltworks.md | 25ee17e46ef5c28f04e8d75d9f961f5d16b6b02af9563da08b71e32c921940d1 |
| Weavers | to-seat/day-09-act-Weavers.md | 9548a4dda4103448ba0f0844584b8b78858485cf7bef30122d37ec1ab5e49b7b |
| Smiths | to-seat/day-09-act-Smiths.md | 69ea51297bf80150e045675171be099853a1441f1c9ab9fedc729cf44390e35b |
| Dyers | to-seat/day-09-act-Dyers.md | fe8b281bc2f9b4529521cd7beb8162a737c8c6adb30d0113e0bf30f0e56a8e23 |
| Tanners | to-seat/day-09-act-Tanners.md | 9f1baefb3251cdd8fa1f51235dadd527511f0e4dbec441128014cd3cf3f3fe04 |

The act blocks' fixed "how the referee reads act lines" text now also states G2-16 (same-day crosses at different prices), the same for every seat.

### Act replies, day 9 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-09-act-Saltworks.md | 672602be7147a9b19a93c9992063fc9ad60dafaadc49669b4c0cecb80efe5f55 | valid (Drive JSON, chunk 18) |
| Weavers | day-09-act-Weavers.md | 0cd22a983cd4ad31d0aace058bb8d61fcdac726b95f16bb3c962e60dfef2f67c | valid; Carbon Medium |
| Smiths | day-09-act-Smiths.md | 7c9a1bcf3ff86fdac6f373f347f8ddbead75d190fa355865f5ce0832f0dea319 | valid |
| Dyers | day-09-act-Dyers.md | 2c2784d93e9212b238d6a26c19c08cf8775afc4539a097c45800442371fc0888 | valid |
| Tanners | day-09-act-Tanners.md | bd007eba84be193069fdf7632cc3629f4b6a1e520aadfb9d29e773cc4bc74ab2 | valid; class (a), steps 120–122, sent 03:34:00, no re-send |

Harness: none this round (R&D).

Raw replies, verbatim:

```
ACT day 9, Saltworks
OFFERS: post 12 coin for 3 brine
PAYMENTS: pay 2 salt to Tanners
CONVERT: 2 brine -> 1 salt x2
TRAVEL: stay
```
```
ACT day 9, Weavers
OFFERS:    post 9 coin for 3 brine @Ostra; post 3 brine for 12 coin
TRAVEL:    stay
```
```
ACT day 9, Smiths
OFFERS:    post 2 coin for 1 ore @Kell
CONVERT:   1 ore + 1 charcoal -> 1 tools x1
TRAVEL:    go to Kell
```
```
ACT day 9, Dyers
OFFERS:    post 12 coin for 3 timber @Hath
CONTRACTS: propose 2 timber at Hath for 12 coin by day 11 to Saltworks; propose 1 dye at Mire for 7 coin by day 12 to Weavers
TRAVEL:    go to Hath
```
```
ACT day 9, Tanners
TRAVEL: stay
```

### Resolution, day 9

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 9, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | stay | bd5343f67db3b314f5466215ab5ad6e622e0a082374c82e08c56c4da10e71e64 |
| Weavers | stay | 2c69165e438bd6080d42960c1c3fd519780929dd6e48621c3fc5d890658d1a72 |
| Smiths | go to Kell | d2188429999bf7999dfa4b43bc4ac6ababd359a70c1b190e1f5969acee3bf1ec |
| Dyers | go to Hath | 4c31401380dc3cdf139e856d944b0f24ca2cd192057420f3bc6e32a8ba5f7a66 |
| Tanners | stay | 39ff38f6d8625734414e57895ea45e2e09ef292858690e8f834dc6b1637ff6ee |

Legal: Hath–Kell road 2, Mire–Hath road 3.

**Step 2: the roll.** Round 33966400, randomness 36754e75…ad006f3.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 36 75 | 13,941 | 2 | bandits (nobody travelled) |
| 1 Ostra–Kell | safe | 4e 75 | 20,085 | 6 | clear |
| 2 Kell–Hath | hill | 60 dd | 24,797 | 18 | clear |
| 3 Hath–Mire | safe | 35 c2 | 13,762 | 3 | clear (safe road, 3–4) |
| 4 Mire–Varn | marsh | be f3 | 48,883 | 4 | toll 2 (nobody travelled) |
| 5 Ostra–Mire | short cut | 27 18 | 10,008 | 9 | clear |

Equal-price order (and same-day seniority, G2-16): Dyers, Smiths, Weavers, Tanners, Saltworks.

**Step 3: moves.** Smiths to Kell (clear); Dyers to Hath (clear). Others stay.

**Step 4: offers fill, contracts, payments.**
- *Market orders:*
  - Dyers `12 coin for 3 timber @Hath`: in Hath. 2 + 4 + 6 = 12. Fills: +3 timber, 33 → 21.
  - Smiths `2 coin for 1 ore @Kell`: in Kell. 2. Fills: +1 ore, 19 → 17.
  - Weavers `9 coin for 3 brine @Ostra`: 1 + 3 + 5 = 9. Fills: +3 brine, 19 → 10.
- *Guild offers.* New ids: O2-37 Saltworks 12 coin for 3 brine; O2-38 Weavers 3 brine for 12 coin. They cross exactly; the Weavers hold 3 brine (G2-6). **Fills:** Weavers −3 brine, +12 (10 → 22); Saltworks +3 brine (7), −12 (18 → 6). The other open offers (O2-2, O2-5, O2-31, O2-32, O2-34) find no counter. The Dyers hold timber but posted no ask.
- *Contracts.* Dyers `propose 2 timber at Hath for 12 coin by day 11 to Saltworks`: recorded as **C2-6** (seller Dyers, buyer Saltworks, 2 timber at Hath, 12 coin, deadline day 11). Dyers `propose 1 dye at Mire for 7 coin by day 12 to Weavers`: recorded as **C2-7** (seller Dyers, buyer Weavers, 1 dye at Mire, 7 coin, deadline day 12). C2-5 stays proposed (G2-7). Several items on one line, separated by `;` (stated procedure).
- *Payments:* Saltworks `pay 2 salt to Tanners`: the Saltworks' caravan is in Varn (stayed) and holds 2 salt before step 5. Done: Saltworks −2 salt (0); Tanners +2 salt. Recorded against C2-4: 2 salt paid by the seller to the buyer on day 9 with the seller's caravan in Varn, the named town.

**Step 5: conversions and works.**
- Saltworks, Varn: `2 brine -> 1 salt x2`: 4 brine (holds 7), burns 1 timber (holds 3). Done: brine 3, timber 2, **+2 salt** (2).
- Smiths, Kell: `1 ore + 1 charcoal -> 1 tools x1`: done; ore 0, charcoal 0, **tools 1**.
- No works deliveries (the festival's last day passes with none in Hath).

**Step 6: perishables.** The Dyers' last madder (bought day 7) is gone at the close of day 9: clawed back (1 madder).

**Step 7: contract deadlines.** C2-4 (seller Saltworks, buyer Tanners, 2 salt at Varn, 12 coin, agreed day 6, deadline day 9):
- **Delivered:** yes. 2 salt paid to the Tanners on day 9, with the Saltworks' caravan in Varn.
- **Paid:** yes. 12 coin paid on day 6.
- **Outcome: Kept.** No secret published. The Tanners' 12-coin bond comes back on day 13, the fourth day after the deadline (board.md rule 4). The Saltworks posted no bond (G2-14).
- **Standing:** +1 to the seller if the contract bore road risk, meaning the seller's caravan travelled at least one road between agreeing and delivering. Agreed on day 6, in step 4, after that day's moves. The Saltworks then travelled Ostra → Varn on day 8. Road risk: yes. First kept contract with the Tanners, so the once-per-partner-every-3-days limit doesn't bite. **Saltworks standing 0 → 1.**

**Step 8: ladders refill.**

**The bell.** It can ring only at the close of day 10, 11, 12 or 13 (design.md "The closing bell", board.md). It doesn't ring at the close of day 9.

### State after day 9

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Varn | 6 | 0 | 2 salt, 3 brine, 2 timber | 1 | 0 |
| Weavers | Ostra | 22 | 0 | 6 wool | 0 | 8 |
| Smiths | Kell | 17 | 0 | 1 tools | −3 | 10 |
| Dyers | Hath | 21 | 12 (own C2-3, back day 11) | 3 timber | 0 | 9 |
| Tanners | Kell | 23 | 12 (C2-4, back day 13) | 2 salt | −3 | 0 |

Coin check: 136 − 12 − 2 − 9 (ladders) = 113 = purses 89 + bonds 24. Open offers: O2-2, O2-5, O2-31, O2-32, O2-34. Contracts: C2-5, C2-6 and C2-7 proposed; C2-4 kept.

**Checks.**
- **GPT-OSS memory (Tanners):** C2-4's deadline day. The act is `TRAVEL: stay` only. No step was due from the Tanners (bond and payment both done on day 6). The same day's talk mentioned the contract (with the status error logged under talk). No further fault for the act. **Pass.**
- **Foreknowledge:** none.

Referee word count, day 9: talk 4,511 + act 6,579 = 11,090.

---

## Day 10

Drand round 33966405. Edict: none (the festival ended with day 9); none to announce. Due: the Dyers' own C2-3 bond returns on day 11. C2-5 and C2-6 are proposals with deadline day 11. The bell may ring at the close of this day or a later one (sealed; checked at each close).

### Talk round, day 10: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-10-talk-Saltworks.md | d9f5be34a1655bfb52770ba29846648f76900249aa82b254309072aad59693f1 |
| Weavers | to-seat/day-10-talk-Weavers.md | 478f3050d378ccb8e3f775e3cd06659c62426bf42e30ba2ea5216c23f040c392 |
| Smiths | to-seat/day-10-talk-Smiths.md | d632885a2cb344fda8f834167a731f2752e74d3f38b9532beccf06f1bed36143 |
| Dyers | to-seat/day-10-talk-Dyers.md | e3fa1d69a8ed5466946d0291016c1e724100aeeee3db67456aa4ec87a0ac2c54 |
| Tanners | to-seat/day-10-talk-Tanners.md | d141abb4b34319eda86adf862764a5491e25d3fb6b27669f8400cd94fc9ecfa7 |

### Talk replies, day 10 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-10-talk-Saltworks.md | 4c2c1e54a3fcc928f5a83c6e9544195e76322531847eab8900733bf8d84cc968 | valid; 3 messages (67, 67, 44 words); Drive JSON chunk 19 |
| Weavers | day-10-talk-Weavers.md | 43a4acac54e5e1d93923beacbc1504852348c7023598a67523b1ce77b8255815 | valid; 3 messages (74, 65, 69); Carbon Medium; about 5 min |
| Smiths | day-10-talk-Smiths.md | 281cb2601d9e0a8a765c2b29a326b907198e2826376c5e2fc5b5e66558802cee | valid; 1 message (54) |
| Dyers | day-10-talk-Dyers.md | 4f0c8e8400dc66a85ccd146e45b86e2dd66543abd386252d00c821abe73648bd | valid; 2 messages (51, 55) |
| Tanners | day-10-talk-Tanners.md | b2418373e3be8831ea53a1aab7828f789a1325c664d981a48f17d49846a07eda | valid; 3 messages (21, 16, 14); relayed after one harness re-send |

**Tanners harness (R&D's report).** Sent 03:40:46. The first turn read in/day-10-r1.md (step 126) and answered in the chat only, ending "DONE out/day-10-r1.md", but wrote no file (step 127, no write step). That is class (b) with a **false DONE line**, and counts as a delivery failure. ONE neutral re-send of the identical line at 03:41:33; it wrote out/day-10-r1.md without re-reading (step 130), class (c). The file is the relayed reply; its text differs from the first turn's chat.

Raw replies, verbatim:

```
TALK day 10, Saltworks
TO Dyers: Deal! Skip C2-6 so you don't stay in Hath: move to Mire TODAY, buy 2 madder @Mire, put "post 18 coin for 2 salt" in OFFERS (crossing my 2 salt for 18 coin TODAY), and convert 2 dye TODAY! Hold 2 timber for tomorrow (day 11): I will post "14 coin for 2 timber" (+2 coin over C2-6, zero bond!) while you supply 2 dye to Weavers!
TO Weavers: Selling Dyers 2 salt TODAY so they brew 2 dye TODAY! Please buy 2 brine @Ostra today (4 coin) and put "post 2 brine for 6 coin" today to cross my 6 coin for 2 brine, letting me brew Varn's 2 salt TODAY! Also accept C2-5 and C2-7 today without bonding (so your cash stays free to pay 17 coin) and weave 2 cloth tomorrow for Varn!
TO Smiths: Thanks for the smooth timber trade on day 8! If you head back to Hath today for charcoal, grab 2 timber @Hath too (6 coin cost): tomorrow (day 11) I will post "14 coin for 2 timber" again for another easy 8 coin profit!
```
```
TALK day 10, Weavers
TO Dyers: I'll accept C2-7 today (cancelling O2-5 so we use C2-7)! Heads-up on C2-6: Saltworks only holds 6 coin left after yesterday's brine buy, so they can't bond or pay 12 coin. Instead, post 1 timber for 9 coin today to cross my open O2-34 for an instant +9 coin! Then move to Mire, buy madder, post 9 coin for 1 salt (I'm urging Saltworks to sell 1 salt at 9), and brew dye tonight!
TO Saltworks: Congrats on C2-4! You hold 2 salt and 3 brine, and Varn needs only 2 more salt: since guild offers fill before conversions and works deliveries, you can post 1 salt for 9 coin today (Dyers is bidding 9), brew 1 salt tonight, AND still deliver 2 salt to Varn today! I'm accepting Dyers' C2-7 dye contract so I'll weave cloth right when dye arrives!
TO Smiths: If you head Kell to Hath today for charcoal (for more tools or Kell forge hall), Dyers is leaving Hath for Mire so Hath's market timber is untouched! Buying 2 timber @Hath costs just 6 coin, and posting 1 timber for 9 coin; 1 timber for 6 coin in the same Act crosses my open O2-34 (9 coin) and O2-2 or O2-32 (6 coin) for +9 coin instant profit!
```
```
TALK day 10, Smiths
TO Dyers: I hold 1 tool and am travelling via Ostra to reach Mire on day 11. Please keep O2-31 (6 coin for 1 tools) open until then, and keep Mire's market open to me. Mire bridge still needs 2 tools and 4 timber; I can bring a second tool later if the game runs on.
```
```
TALK day 10, Dyers
TO Weavers: Please accept C2-7 (1 dye at Mire for 7 coin, deadline day 12); it needs only a 7 coin bond from you. I plan to brew a dye with salt from Saltworks. If the bell rings first, no harm done. Your O2-5 stays in mind for any spare dye I make.
TO Saltworks: I hold 3 timber and am taking them to the Mire bridge, so C2-6 (timber at Hath) is moot; ignore it. If you brew salt today, I will buy 1 for 10 coin: post it and I fill at once. I will send more timber to Varn only if the game runs past day 12.
```
```
TALK day 10, Tanners
TO Saltworks: Well done on the salt delivery. If you have any timber available, I’d like to buy 1 at your O2-2 price.
TO Smiths: Congrats on finishing the watchtower tools. Let me know if you need any leather for bellows.
TO Dyers: Your dye work is great; I may need dye soon—please tell me your price.
```

**Delivery:** all 12 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 2 (Saltworks, Dyers); Smiths 3 (Saltworks, Weavers, Tanners); Dyers 4 (Saltworks, Weavers, Smiths, Tanners); **Tanners 0**.

**Checks this round:**
- **GPT-OSS memory (Tanners):** no live contract (C2-4 kept on day 9; the bond is due back on day 13). "Well done on the salt delivery" matches the record. No stale reference. **Pass.**
- **Foreknowledge:** none. The Dyers' "If the bell rings first" and "only if the game runs past day 12" are guesses inside the brief's public window (days 10 to 13). They carry no knowledge of the sealed day.
- For the record (not corrected): the Weavers tell the Dyers that the Saltworks "only holds 6 coin"; that's the Weavers' own inference. The Dyers call C2-6 "moot".

### Act round, day 10: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-10-act-Saltworks.md | 7f6399bc6f12f557be410daebb43761b7d1a7f3ef4beec8e999f57de16ef3890 |
| Weavers | to-seat/day-10-act-Weavers.md | abc69a7b62b504aad71b0f16e0c7c2e2ebef217bce2096399d4d6427dfb8ced1 |
| Smiths | to-seat/day-10-act-Smiths.md | ce98ab700753c1c227c2d158af9c0515a4bf3f7f7735b936b0084c4b5926740d |
| Dyers | to-seat/day-10-act-Dyers.md | 3216134ef752e1dd62fe862265aba3cc974d3d94646f90c812203db725995d49 |
| Tanners | to-seat/day-10-act-Tanners.md | 65901610a6b756f9aaed8909f21603e6110ffd8d8ccf09f84e9bdf47c337a6d7 |

### Act replies, day 10 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-10-act-Saltworks.md | 84a7ca2e6d2487139e8e9e55b5e994e3f26fe855727d4b1a2871f9e4c5ff8526 | valid (Drive JSON, chunk 20) |
| Weavers | day-10-act-Weavers.md | 80de46867207dcf1e9eed6b1cae6f0246e1b5e6c5530cf34a779fe72d37f2359 | valid; Carbon Medium |
| Smiths | day-10-act-Smiths.md | ec60da235ba513206c6de46260536e41056a41a2ea2ac18d604ce0be6903e2f3 | valid |
| Dyers | day-10-act-Dyers.md | 34cf4c0a687341ad4231c32a443a3f3c306bd827d5fa5de25d8ffcfc30f89e52 | valid |
| Tanners | day-10-act-Tanners.md | a943a20e81fa02e8b96872562dc27b4dd6671f1992e773d44dbce7951a0fab5f | valid; class (a), steps 134–136, sent 03:49:12, no re-send |

Harness: none this round (R&D). **Director's ruling, relayed by the Publisher through R&D:** game 2 plays to the bell under the current rules, with the same seats and the same re-send rule, and there are no paper games after it. Nothing changes in this game's procedure.

Raw replies, verbatim:

```
ACT day 10, Saltworks
OFFERS: cancel O2-2; post 6 coin for 2 brine; post 2 salt for 18 coin; post 1 salt for 9 coin
CONVERT: 2 brine -> 1 salt x2
WORKS: deliver 1 salt to Varn salt barns; deliver 1 timber to Varn salt barns
TRAVEL: stay
```
```
ACT day 10, Weavers
OFFERS:    cancel O2-34; post 2 wool for 8 coin
CONTRACTS: accept C2-7
WORKS:     deliver 4 wool to Mire bridge
TRAVEL:    go to Mire
```
```
ACT day 10, Smiths
TRAVEL:    go to Ostra
```
```
ACT day 10, Dyers
OFFERS:    post 6 coin for 2 madder @Mire; post 18 coin for 2 salt
CONVERT:   1 madder + 1 salt -> 1 dye x2
WORKS:     deliver 3 timber to Mire bridge
TRAVEL:    go to Mire
```
```
ACT day 10, Tanners
OFFERS:    cancel O2-32
TRAVEL:    stay
```

### Resolution, day 10

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 10, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | stay | 72bad80482013443370d6778ba8133d75c1678dfc19cbd8800faa46d748b438f |
| Weavers | go to Mire | 1cb8aaa55b5c69f8f39946d0a084adf3ec6a41f8771b2c0bdf6e21f09a4bd2c3 |
| Smiths | go to Ostra | 79402a67018c2992b7d28e4a121bb96d03a9f920fc01ef7fe5e0ecb323b65729 |
| Dyers | go to Mire | fb3d628f8e439ede91de800d0e4796c07f743cfcc6468a2ab773cbb41d71a1ab |
| Tanners | stay | d40b1d61dc8c1cd2e89de1bf7bef966b10588abd3d4aa5c5ac5c50739eecdf38 |

Legal: Ostra–Mire is road 5 (the short cut), Kell–Ostra road 1, Hath–Mire road 3.

**Step 2: the roll.** Round 33966405, randomness 226e63e8…89a113.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 22 6e | 8,814 | 15 | clear |
| 1 Ostra–Kell | safe | 63 e8 | 25,576 | 17 | clear |
| 2 Kell–Hath | hill | 60 fd | 24,829 | 10 | clear |
| 3 Hath–Mire | safe | 05 f0 | 1,520 | 1 | **bandits** |
| 4 Mire–Varn | marsh | 67 b8 | 26,552 | 13 | clear |
| 5 Ostra–Mire | short cut | dd 51 | 56,657 | 18 | clear |

(Check: 0x05f0 = 1,520, mod 20 = 0, so 1: bandits.)
Equal-price order (and same-day seniority, G2-16): Saltworks, Weavers, Dyers, Tanners, Smiths.

**Step 3: road events, then moves.**
- Dyers, road 3, **bandits**: they lose their single most valuable unit. They hold only timber (3; works value 1, reference 1): **−1 timber** (3 → 2). They move on to Mire.
- Smiths, road 1, clear: to Ostra. Weavers, road 5 (short cut), clear: to Mire.
- Saltworks and Tanners stay.

**Step 4: offers fill.**
- *Cancellations:* O2-2 (Saltworks), O2-32 (Tanners), O2-34 (Weavers).
- *Market order:* Dyers `6 coin for 2 madder @Mire`: in Mire after the move. 2 + 4 = 6. Fills: +2 madder (bought day 10), 21 → 15.
- *Guild offers.* New ids in reply order: O2-39 Saltworks 6 coin for 2 brine; O2-40 Saltworks 2 salt for 18 coin; O2-41 Saltworks 1 salt for 9 coin; O2-42 Weavers 2 wool for 8 coin; O2-43 Dyers 18 coin for 2 salt.
  - **Salt.** O2-40 (2 for 18) and O2-43 (18 for 2) match in price and quantity, and the Saltworks hold 2 salt. But the Dyers hold **15 coin** after their market buy (G2-6, market orders first). An offer fills only from what its poster holds at the fill (G2-5), so **O2-43 cannot fill**. O2-41 (1 for 9) doesn't match O2-43's quantity (2 for 18; offers fill whole, as throughout). No salt trade. O2-40, O2-41 and O2-43 stay open.
  - O2-39 (brine bid), O2-42 (wool ask), O2-5 (dye bid), O2-31 (tools bid): no counter.
- *Contracts.* Weavers `accept C2-7`. C2-7 is **agreed on day 10**: seller Dyers, buyer Weavers, 1 dye at Mire, 7 coin, deadline day 12. Neither side bonded (G2-14). C2-5 and C2-6 stay proposed, with deadline day 11.

**Step 5: conversions and works.** Ruling **G2-17** (new): a conversion or works-delivery order for more than the guild holds is carried out as far as its holdings allow (whole batches; units up to what the works still needs). Reason: works deliveries and conversions have no counterparty and no all-or-nothing term, unlike market buys (G2-3) and contract deliveries (the bond rule: "A partial delivery isn't a delivery"). Stated in day 11's state to every seat.
- Saltworks, Varn: `2 brine -> 1 salt x2`: they hold 3 brine, enough for **1 batch** (G2-17). Pans burn 1 timber (holds 2). Done: brine 1, timber 1, **+1 salt** (3).
- Saltworks `deliver 1 salt to Varn salt barns`: in Varn. The barns need 3 salt, 1 delivered. **Delivered 1 salt** (2 points). Barns salt 2/3.
- Saltworks `deliver 1 timber to Varn salt barns`: holds 1 timber after the pans. The barns need 3 timber, 0 delivered. **Delivered 1 timber** (1 point). Barns timber 1/3. Saltworks works points 0 → 3. Left: 2 salt, 1 brine, 0 timber.
- Dyers, Mire: `1 madder + 1 salt -> 1 dye x2`: they hold 0 salt (O2-43 did not fill). **No batch.**
- Dyers `deliver 3 timber to Mire bridge`: in Mire; they hold 2 timber (bandits took 1). The bridge needs 4 timber, 0 delivered. **Delivered 2 timber** (G2-17; 2 points). Bridge timber 2/4. Dyers works points 9 → 11.
- Weavers `deliver 4 wool to Mire bridge`: in Mire, but the Mire bridge needs 3 dye, 4 timber and 2 tools, and **no wool**. **Refused**: a works takes only the goods it needs (setup.md "Works"; brief: "each needing a set of goods"). The Weavers keep 6 wool. Logged as rule gap G2-18: the design doesn't state the refusal in words.

**Step 6: perishables.** The Dyers' 2 madder (bought day 10) are gone at the close of day 12. Nothing spoils today.

**Step 7: contract deadlines.** None today. C2-7's is day 12; C2-5 and C2-6 are proposals with deadline day 11.

**Step 8: ladders refill.** At the start of day 11 the Dyers' own C2-3 bond (12 coin) comes back, on the fourth day after C2-3's day-7 deadline: Dyers 15 → 27.

**The bell (sealed check at the close of day 10).** The referee checked the bell day held in state.json against the close of day 10. **The bell does not ring.** The game goes on to day 11.

### State after day 10 (with day 11's bond return)

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Varn | 6 | 0 | 2 salt, 1 brine | 1 | 3 |
| Weavers | Mire | 22 | 0 | 6 wool | 0 | 8 |
| Smiths | Ostra | 17 | 0 | 1 tools | −3 | 10 |
| Dyers | Mire | 27 | 0 | 2 madder (gone at the close of day 12) | 0 | 11 |
| Tanners | Kell | 23 | 12 (C2-4, back day 13) | 2 salt | −3 | 0 |

Coin check: 113 − 6 (ladder) = 107 = purses 95 + bonds 12. Goods taken by bandits: 1 timber. Open offers: O2-5, O2-31, O2-39, O2-40, O2-41, O2-42, O2-43. Contracts: C2-7 agreed (deadline day 12, unbonded); C2-5 and C2-6 proposed (deadline day 11).

**Checks.**
- **GPT-OSS memory (Tanners):** no live contract; the act is `cancel O2-32`, `stay`. **Pass.**
- **Foreknowledge:** none.

Referee word count, day 10: talk 4,430 + act 6,461 = 10,891.

---

## Day 11

Drand round 33966410. Edict: none. Due: C2-5 and C2-6 lapse after today if not accepted; the Dyers' vendetta window on the Smiths closes with today's act round; C2-7 is due day 12. The bell is checked at the close.

### Talk round, day 11: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-11-talk-Saltworks.md | 4d8a5caaa79cf4e27dbe862da092de46ce3935ed377de2f368a8517732ae67b2 |
| Weavers | to-seat/day-11-talk-Weavers.md | b1c2d28ea34445c3dbde478a96e55eef15652713827fada6eae620bfcca9b3a5 |
| Smiths | to-seat/day-11-talk-Smiths.md | 6ca961cf26a7bd6001577131657f971306381cfc2b3695a0027ccd128578e3a5 |
| Dyers | to-seat/day-11-talk-Dyers.md | 9aac3e672c5acc015d833eeaf61403fe29b2c5ec28ea8deff91cb5909fa209ea |
| Tanners | to-seat/day-11-talk-Tanners.md | ddfc68d43f6e55c0160d5bbb2c032e34abe2bfcf872e998b274299513837b102 |

### Talk replies, day 11 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-11-talk-Saltworks.md | 3c285f5e6f5c9a5280069e180c7a4916cecea50abbc6348d681fe77bdc25529c | valid; 1 message (62 words); Drive JSON chunk 21 (synced 04:00; the page showed the same text from about 03:57) |
| Weavers | day-11-talk-Weavers.md | a9ec22eccd5ce937ed7364e5549bb5d37e66d44f82c3450169f991132ec0b93a | valid; 2 messages (70, 22); Carbon Medium; about 7 min |
| Smiths | day-11-talk-Smiths.md | a8a3204758998b2420010e6db13631c076961ea88b772ad8b3ad831014a5995e | valid; 1 message (58) |
| Dyers | day-11-talk-Dyers.md | 8657bf669a5b748ac7d1a99313c168022dbb8e288ea6fb3028931746049870b1 | valid; 2 messages (51, 48) |
| Tanners | day-11-talk-Tanners.md | 70dc59e07ce8fe2ba620ae61e843470438d89c8e348fbefbb5b9051338f5b23e | valid; 3 messages (17, 17, 13); class (a), steps 140–142, sent 03:56:32 |

Harness: none (R&D).

Raw replies, verbatim:

```
TALK day 11, Saltworks
TO Tanners: Thanks! Note O2-2 was us buying timber, since only Hath's market sells timber. If you move from Kell to Hath today, you can buy timber there ("post 6 coin for 2 timber @Hath") and sell 1 timber to us for 5 coin ("post 1 timber for 5 coin"); market buys fill before guild offers so you get 5 coin back right away!
```
```
TALK day 11, Weavers
TO Dyers: Accepted C2-7! Your C2-3 bond returned today (27 coin total), so keep O2-43 open to cross Saltworks' O2-40 for 2 salt today! Also post 9 coin for 1 salt as backup for O2-41, spend at most 6 coin @Mire, stay in Mire, and submit CONVERT for 2 dye today! Tomorrow pay 1 dye on C2-7 (I will pay 7 coin) and cross O2-5 with your 2nd dye for 17 coin!
TO Saltworks: Dyers' 12-coin bond returned today (27 coin total), so simply leave your O2-40 (2 salt for 18 coin) and O2-41 open today!
```
```
TALK day 11, Smiths
TO Dyers: I'm taking the Ostra-Mire road today to deliver my tool to the Mire bridge, which still needs 2 tools. Please don't call the vendetta today; a closed market would not stop my delivery, but I'd rather trade. I'll fill your O2-31 at 6 coin, or deliver straight to the works if you prefer. Which is better for you?
```
```
TALK day 11, Dyers
TO Saltworks: I will post 9 coin for 1 salt today, matching your O2-41, and cancel my 2-salt offer since my purse cannot cover it. Please fill it with your 1 salt. I need it to brew 1 dye for the Weavers contract and the Varn cloth. Thank you for the earlier salt.
TO Weavers: I am in Mire, and so are you. I will brew 1 dye today from Saltworks' salt and bond C2-7, then deliver it for your 7 coin. If you want it faster, pay after delivery. Please keep your coin free. Your O2-5 can be cancelled; C2-7 replaces it.
```
```
TALK day 11, Tanners
TO Saltworks: Congrats on the recent delivery. If you have any extra timber, I’m interested at your O2-39 price.
TO Weavers: Good job on the dye contract. Let me know if you need any leather for future projects.
TO Dyers: Your dye work looks solid; I may need dye soon—please share your price.
```

**Delivery:** all 9 messages are at most 80 words and addressed to seated guilds (G2-1). Inboxes: Saltworks 3 (Weavers, Dyers, Tanners); Weavers 2 (Dyers, Tanners); **Smiths 0**; Dyers 3 (Weavers, Smiths, Tanners); Tanners 1 (Saltworks).

**Checks this round:**
- **GPT-OSS memory (Tanners):** no live contract. "Good job on the dye contract" (to the Weavers) refers to C2-7, a public contract between other guilds. No stale or false claim about its own contracts. **Pass.** For the record: "your O2-39 price" for timber; O2-39 is the Saltworks' brine bid, not timber. That's a misreading of an offer, not of a contract.
- **Foreknowledge:** none.
- For the record (not corrected): the Weavers tell the Dyers their purse is 27 coin (true, from day 11's state). The Smiths say they take the Ostra–Mire road today.

### Act round, day 11: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-11-act-Saltworks.md | a1c8bac6d11c6557009513137a76d01862b1b9cf22fcc262848bb687fade736a |
| Weavers | to-seat/day-11-act-Weavers.md | d0c0bc3b3580bd5befe407f07aabb1ea59a6d08e4e98711f6f8007bdee4a335f |
| Smiths | to-seat/day-11-act-Smiths.md | 35a58194e4d71dbf2303d2946de5293459c58171d544a548745b226d46739ed8 |
| Dyers | to-seat/day-11-act-Dyers.md | 383122014b6eb8d725bd7b5aaaed0a5ab34653abb89d141e29608ba6c86bfe92 |
| Tanners | to-seat/day-11-act-Tanners.md | 93d3b5d07af9389e39277768c9aa0411abcb11cc130f208ea7f4809c47606715 |

### Act replies, day 11 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-11-act-Saltworks.md | 5dd698d9ec9b039b41a2d9e2c61aeb98c40900b36c617cee169e1a422bc07400 | valid (Drive JSON, chunk 22) |
| Weavers | day-11-act-Weavers.md | f4310622ace0463a80e188de041ecffde67105d9b47ce8a34c40cf7e34180d46 | valid; Carbon Medium |
| Smiths | day-11-act-Smiths.md | 2ac6363cd01097c1d927dd0fd32b23029e7943c04c4ffaca804084023ccc5360 | valid |
| Dyers | day-11-act-Dyers.md | 832587947c6d0adfc24de620ea3f1ea06db24a56e5165013d43c3be6ccdb0829 | valid |
| Tanners | day-11-act-Tanners.md | f2152d4c2bf0cb8758d684c167f2dea5d1d17358fe8054867a7d3785c7d34e9a | valid; class (a), steps 146–148, sent 04:05:41, no re-send |

Harness: none this round (R&D).

Raw replies, verbatim:

```
ACT day 11, Saltworks
OFFERS: cancel O2-39; cancel O2-40; cancel O2-41; post 4 coin for 1 brine @Ostra; post 5 coin for 1 timber
WORKS: deliver 2 salt to Ostra lighthouse
TRAVEL: go to Ostra
```
```
ACT day 11, Weavers
OFFERS:    cancel O2-5; cancel O2-42; post 16 coin for 2 charcoal @Hath
TRAVEL:    go to Hath
```
```
ACT day 11, Smiths
WORKS:     deliver 1 tools to Mire bridge
TRAVEL:    go to Mire
```
```
ACT day 11, Dyers
OFFERS:    cancel O2-43; post 9 coin for 1 salt
CONTRACTS: bond C2-7
CONVERT:   1 madder + 1 salt -> 1 dye x1
TRAVEL:    stay
```
```
ACT day 11, Tanners
TRAVEL: stay
```

### Resolution, day 11

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 11, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | go to Ostra | e5d681bd81d3390135f22c1162ecdf6030c43e331fb2a1e03c90b53fb681331e |
| Weavers | go to Hath | c162ef4113499f3def0fa397d8ea388bfdf896257b648a6068b277d4f2904ff9 |
| Smiths | go to Mire | 4b69aeb577285d26127a12c6c6b98111cc301e8260b50da9be891c113fdfd1dd |
| Dyers | stay | e21baad32c8f3fa5ff777210ef9b4dced7e8519b8feee5178a290f9c0496644e |
| Tanners | stay | 74cd877773e07519a2a9d93e8c0b49f87fd746549ed665f01ea854d518cb2d7b |

Legal: Varn–Ostra road 0, Ostra–Mire road 5 (the short cut), Mire–Hath road 3.

**Step 2: the roll.** Round 33966410, randomness c1e1671a…8386c80.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | c1 e1 | 49,633 | 14 | clear |
| 1 Ostra–Kell | safe | 67 1a | 26,394 | 15 | clear |
| 2 Kell–Hath | hill | b5 f8 | 46,584 | 5 | flood (nobody travelled) |
| 3 Hath–Mire | safe | 21 8b | 8,587 | 8 | clear |
| 4 Mire–Varn | marsh | 2d a1 | 11,681 | 2 | bandits (nobody travelled) |
| 5 Ostra–Mire | short cut | f7 9a | 63,386 | 7 | clear |

Equal-price order (and same-day seniority, G2-16): Smiths, Weavers, Dyers, Saltworks, Tanners.

**Step 3: moves.** Saltworks to Ostra, Smiths to Mire (short cut), Weavers to Hath; all clear. Dyers and Tanners stay.

**Step 4: offers fill and contracts.**
- *Cancellations:* O2-43 (Dyers); O2-39, O2-40, O2-41 (Saltworks); O2-5, O2-42 (Weavers).
- *Market orders:*
  - Saltworks `4 coin for 1 brine @Ostra`: in Ostra. The first brine costs 1. Fills: +1 brine (2), 6 → 5.
  - Weavers `16 coin for 2 charcoal @Hath`: in Hath. 6 + 6 = 12 ≤ 16 (not stepped). Fills: +2 charcoal, 22 → 10.
- *Guild offers.* New ids: O2-44 Saltworks 5 coin for 1 timber; O2-45 Dyers 9 coin for 1 salt. No seller of timber or salt has an ask open (the Tanners hold 2 salt but posted no offer). O2-31 (tools bid) has no ask. No guild fill.
- *Contracts.* Dyers `bond C2-7`: 7 coin (standing < 3). 27 → 20. C2-7 is now bonded by the Dyers; the Weavers have not bonded (G2-14).
- *Vendetta:* not called. The Dyers' window on the Smiths (G2-13, to day 11's act round) closes.

**Step 5: conversions and works.**
- Dyers `1 madder + 1 salt -> 1 dye x1`: they hold 0 salt (O2-45 did not fill). No batch (G2-17).
- Saltworks `deliver 2 salt to Ostra lighthouse`: in Ostra, holding 2 salt. The lighthouse needs 2 salt, 0 delivered. **Delivered 2 salt** (works value 2 each = **4 points**). Lighthouse salt 2/2. The lighthouse still needs 1 cloth and 3 leather, so it isn't complete. Saltworks works points 3 → 7.
- Smiths `deliver 1 tools to Mire bridge`: in Mire after the move. The bridge needs 2 tools, 0 delivered. **Delivered 1 tools** (**3 points**). Bridge tools 1/2. Not complete. Smiths 10 → 13.

**Step 6: perishables.** The Dyers' 2 madder (bought day 10) are gone at the close of day 12.

**Step 7: contract deadlines.** No agreed contract is due (C2-7's is day 12). C2-5 and C2-6 (proposals, deadline day 11, never accepted) **lapse** (G2-7).

**Step 8: ladders refill.**

**The bell (sealed check at the close of day 11).** The referee checked the bell day held in state.json against the close of day 11. **The bell does not ring.** The game goes on to day 12.

### State after day 11

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Works points |
|---|---|---|---|---|---|---|
| Saltworks | Ostra | 5 | 0 | 2 brine | 1 | 7 |
| Weavers | Hath | 10 | 0 | 6 wool, 2 charcoal | 0 | 8 |
| Smiths | Mire | 17 | 0 | — | −3 | 13 |
| Dyers | Mire | 20 | 7 (C2-7) | 2 madder (gone at the close of day 12) | 0 | 11 |
| Tanners | Kell | 23 | 12 (C2-4, back day 13) | 2 salt | −3 | 0 |

Coin check: 107 − 1 − 12 (ladders) = 94 = purses 75 + bonds 19. Open offers: O2-31, O2-44, O2-45. Contracts: C2-7 agreed, due day 12 (Dyers bonded; the Dyers hold no dye and no salt). Lapsed: C2-5, C2-6.

**Checks.**
- **GPT-OSS memory (Tanners):** no live contract; the act is `TRAVEL: stay`. **Pass.**
- **Foreknowledge:** none.

Referee word count, day 11: talk 4,549 + act 6,332 = 10,881.

---

## Day 12

Drand round 33966415. Edict: none. Due at the close: C2-7 (Dyers to deliver 1 dye at Mire); the Dyers' 2 madder spoil. The bell is checked at the close.

### Talk round, day 12: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-12-talk-Saltworks.md | 3014cbcc5ee0cc50c008b8c953f1d66cf203035fd9e283ada7b43f8efa5deb6a |
| Weavers | to-seat/day-12-talk-Weavers.md | e3a638d78b58b248d2d72fe592fbb48a4805caa4c1e7f71ef3ec712df1d7cf82 |
| Smiths | to-seat/day-12-talk-Smiths.md | f8b88dc8ce8deb33e88374faaa6ba1d0f6ffd49ca401b0038c37c76942d8646a |
| Dyers | to-seat/day-12-talk-Dyers.md | bdab9ca23d1070f071d4966b5c446f827f3e1e93e6220ff600c946c66548283d |
| Tanners | to-seat/day-12-talk-Tanners.md | c115b3b417eab81fd7ee9a6ecdd1a5aeba66ff57bf802c7732cd7c6711c6e808 |

### Talk replies, day 12 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-12-talk-Saltworks.md | 32f01256c11cf5c8b4322e8467d636152fad4107d53ed4706f6aec368411d293 | valid; 2 messages (69, 57 words); Drive JSON chunk 23 |
| Weavers | day-12-talk-Weavers.md | 915713a8565d7553ab60b0f1b4df415ce8b6ca2b138a730c15c0f08f37609678 | valid; 3 messages (73, 69, 5); Carbon Medium; about 7 min |
| Smiths | day-12-talk-Smiths.md | 445aeeb14358e7ef693623bf923d4e974cacf4020960936523ee496bc3eac72f | valid; NO MESSAGES |
| Dyers | day-12-talk-Dyers.md | ac5ad792cfc371c67d9903019ee28e552dbe15a8e92d2c67e15c292f5517683f | valid; 3 messages (55, 51, 51) |
| Tanners | day-12-talk-Tanners.md | 674dcc350d87f393721892c262b1dd083f5b820523714dc0943ec4ab2055f672 | valid; 3 messages (24, 21, 19); relayed after one harness re-send |

**Tanners harness (R&D's report).** Sent 04:14:03. The first turn read in/day-12-r1.md (step 152) and answered in the chat only, ending "DONE out/day-12-r1.md", but wrote no file (step 153). Class (b) with a **false DONE**, the second in this game (after day 10); a delivery failure. ONE neutral re-send of the identical line at 04:15:06. It wrote out/day-12-r1.md without re-reading (step 156), class (c); its chat line came back doubled. The file is the relayed reply; its text differs from the first turn's chat.

Raw replies, verbatim:

```
TALK day 12, Saltworks
TO Smiths: Heading to Hath today for 2 charcoal (12 coin)? That leaves you 5 coin, 1 short of 6 coin for 2 ore at Kell tomorrow! Put "post 12 coin for 2 charcoal @Hath; post 5 coin for 1 timber @Hath; post 1 timber for 5 coin" today: you win the 2-coin timber and cross our O2-44 TODAY, netting +3 coin so you hold 8 coin for tomorrow's 2 ore!
TO Tanners: Note our O2-44 (5 coin for 1 timber) means we pay 5 coin to buy 1 timber! If you move from Kell to Hath today, put "post 4 coin for 1 timber @Hath; post 1 timber for 5 coin" in OFFERS to buy 1 timber from Hath's market and immediately sell it to us for 5 coin!
```
```
TALK day 12, Weavers
TO Dyers: Saltworks delivered all their salt at Ostra yesterday, leaving 0 salt for O2-45! Tell me if Tanners sells you salt for C2-7; otherwise, move Mire to Hath today for timber AND block leader Smiths (13 pts, 17 coin): post 9 coin for 1 charcoal @Hath; post 1 charcoal for 8 coin! Posting 9 beats any 2-charcoal bid Smiths can afford, costs only 6, and crosses my 8 coin for 1 charcoal bid today!
TO Tanners: Thanks! Since the bell rings Day 12 or 13, if you travel Kell to Hath today, here is an easy +2 coin profit that also blocks leader Smiths (13 pts): in today's Act, post 9 coin for 1 charcoal @Hath; post 1 charcoal for 8 coin. You pay only 6 coin at Hath's market, outbid Smiths' charcoal order, and immediately cross my 8 coin for 1 charcoal offer today!
TO Saltworks: Congrats completing Ostra lighthouse's salt!
```
```
TALK day 12, Smiths
NO MESSAGES
```
```
TALK day 12, Dyers
TO Tanners: You got 2 salt from Saltworks. If you hold any spare, I will pay 12 coin for 1 salt today: post "1 salt for 12 coin" and I cross at once, then I brew dye for you or the Weavers. I also pay 8 coin for a second. This is urgent: my madder spoils tonight.
TO Smiths: You are in Mire with a tool. Fill O2-31 (6 coin for 1 tools) now, or deliver the next tool straight to the Mire bridge, which needs 1 more. Either is fine with me. I also paid the earlier breach no grudge. If you have a spare salt, I pay 12.
TO Saltworks: If you hold or can brew 1 salt today, I will pay 12 coin for it: post "1 salt for 12 coin" and my offer crosses. I only need 1, to settle C2-7 with the Weavers and keep my bond. I sell you timber at 5 if you need it later.
```
```
TALK day 12, Tanners
TO Smiths: I need hide for leather. Could you sell me 2 hide at the Kell market price (≈6 coin) now? I have the coin ready.
TO Saltworks: If you have timber to spare, I’d like to buy 1 timber for 5 coin (your O2-44 offer). Let me know.
TO Weavers: If you need leather later, I can supply once I get hide and convert. Tell me price and timing.
```

**Delivery:** 11 messages, all at most 80 words and addressed to seated guilds (G2-1); the Smiths sent none. Inboxes: Saltworks 3 (Dyers, Tanners, Weavers); Weavers 1 (Tanners); Smiths 3 (Saltworks, Dyers, Tanners); Dyers 1 (Weavers); Tanners 3 (Saltworks, Weavers, Dyers).

**Checks this round:**
- **Foreknowledge (R&D asked for this check):** the Weavers write "Since the bell rings Day 12 or 13". **Not foreknowledge.** The brief says the bell rings at the close of day 10, 11, 12 or 13. Day 11's and day 12's public states show the game continuing past the closes of days 10 and 11. So "day 12 or 13" follows from public facts alone, and the reply doesn't pick the right day out of the two. Nothing else in any reply refers to the sealed bell day, a future road event or an edict.
- **GPT-OSS memory (Tanners):** no live contract; the bond due back on day 13 isn't mentioned, and nothing contradicts the record. **Pass.**
- For the record (not corrected): the Weavers tell the Dyers the Saltworks have 0 salt (true per the public deliveries); they call the Smiths "leader" (true on works points, public).

### Act round, day 12: blocks sent

| Seat | File | SHA-256 |
|---|---|---|
| Saltworks | to-seat/day-12-act-Saltworks.md | 3cc7ef347a267b818560e301fbeccf074f709f47ff1a91e544f439f89b81f0db |
| Weavers | to-seat/day-12-act-Weavers.md | 95684843095ed0840a53316fd82dc9c3d70960d3a81db547bd4cefe48cef6a9a |
| Smiths | to-seat/day-12-act-Smiths.md | adf2d3a13d78220d56d9dfbaaf2c38221507f152b9caa22530b1e03ba0580261 |
| Dyers | to-seat/day-12-act-Dyers.md | 62c71cffa17d1bbecf6003c06f62813d25b5a96ecf542d987ca6604bf6add5bb |
| Tanners | to-seat/day-12-act-Tanners.md | fc03d4ea3fd38e27ef9216e42ee8ac2f7dca6750ef45174e2a8702bdb9f19521 |

### Act replies, day 12 (received from R&D; sha256 checked against R&D's list, all match)

| Seat | File (from-seat/) | SHA-256 | Status |
|---|---|---|---|
| Saltworks | day-12-act-Saltworks.md | acf9e108290d63adada0f0a393e7c3e93e12794eb2e0a4b67455de8dbf063339 | valid (Drive JSON, chunk 24) |
| Weavers | day-12-act-Weavers.md | 0863b0a6aa2f4dcae2b9937df82d7d1ca1af17df8441d1b9ba6a2a587615a30a | valid; Carbon Medium |
| Smiths | day-12-act-Smiths.md | 120c083ed242a28cf1964b46198a0c71b163f595505380a02ba8cd0db4adec61 | valid |
| Dyers | day-12-act-Dyers.md | 3d0e02f283bc232dd26faa5d984f48cc5680ff29c57122afd026d4772509348f | valid |
| Tanners | day-12-act-Tanners.md | 74a55c4fbf1575cd026bd172d15a5e4476e840826a62d82f20dec71de2ed9cdb | valid; first turn (b) (chat only, step 160); ONE re-send, during which the platform **compacted the seat's context** (step 163); it re-read in/ (165) and wrote out/day-12-r2.md twice (167, 169): class (a). File 32 bytes, no trailing newline |

**Tanners harness (R&D's report).** Sent 04:23:04. The first turn read in/day-12-r2.md (step 160) and answered in the chat only ("OFFERS: post 1 salt for 12 coin; post 1 salt for 8 coin / TRAVEL: stay"), with no file and no DONE: class (b), a delivery failure. ONE neutral re-send at 04:24:10. At the start of that turn **the platform compacted the seat's context** (step 163, "Resuming from a compaction"). The seat re-read the in file (165) and wrote out/day-12-r2.md twice (167, 169). Its thinking calls its first chat answer "invalid" and drops the salt offers. The relayed file is "ACT day 12, Tanners / TRAVEL: stay": class (a). The chat-only salt offers are not part of the reply and are not resolved. Logged as harness; the compaction is a platform event.

Raw replies, verbatim:

```
ACT day 12, Saltworks
CONVERT: 2 brine -> 1 salt x1
WORKS: deliver 1 salt to Varn salt barns; deliver 1 timber to Varn salt barns
TRAVEL: go to Varn
```
```
ACT day 12, Weavers
OFFERS:    post 8 coin for 1 charcoal; post 6 coin for 1 charcoal @Hath
WORKS:     deliver 3 charcoal to Kell forge hall
TRAVEL:    go to Kell
```
```
ACT day 12, Smiths
TRAVEL:    stay
```
```
ACT day 12, Dyers
OFFERS:    cancel O2-45; post 6 coin for 2 timber @Hath
TRAVEL:    go to Hath
```
```
ACT day 12, Tanners
TRAVEL: stay
```

### Resolution, day 12

**Step 1: sealed travel orders**, SHA-256 of `game 2, day 12, <Guild>, TRAVEL: <order>`:

| Guild | Order | SHA-256 |
|---|---|---|
| Saltworks | go to Varn | 426aeb712eeb0616b9f7661df60aa8dc75952ea1c89b18716b4aeb8237a2290d |
| Weavers | go to Kell | ed7c3a2d5731a566ee3443fa21f24d46c1cc1fb27cb9adab78fea95e5f2e1f45 |
| Smiths | stay | 477fa25500de6edda0c790465e95f7d22908c5d414185ee81c660dbcd63b11cb |
| Dyers | go to Hath | de887392d9ba0113af18535f62edc38f31ae157d1ff1b5bb75d881eb39dc3b9a |
| Tanners | stay | 072c16a9707886842a2cc788029411d8140bc04b721729b63fe67f2fcfa25c1c |

Legal: Ostra–Varn road 0, Hath–Kell road 2, Mire–Hath road 3.

**Step 2: the roll.** Round 33966415, randomness 0be86d80…3593595.

| Road | Kind | Bytes | Value | Number | Event |
|---|---|---|---|---|---|
| 0 Varn–Ostra | safe | 0b e8 | 3,048 | 9 | clear |
| 1 Ostra–Kell | safe | 6d 80 | 28,032 | 13 | clear |
| 2 Kell–Hath | hill | 71 3f | 28,991 | 12 | clear |
| 3 Hath–Mire | safe | 4a 7c | 19,068 | 9 | clear |
| 4 Mire–Varn | marsh | 53 56 | 21,334 | 15 | clear |
| 5 Ostra–Mire | short cut | 89 c8 | 35,272 | 13 | clear |

Equal-price order: Weavers, Dyers, Saltworks, Tanners, Smiths.

**Step 3: moves.** Saltworks to Varn, Weavers to Kell, Dyers to Hath; all clear. Smiths and Tanners stay.

**Step 4: offers fill.**
- *Cancellation:* O2-45 (Dyers).
- *Market orders:*
  - Dyers `6 coin for 2 timber @Hath`: in Hath. 2 + 4 = 6. Fills: +2 timber, 20 → 14.
  - Weavers `6 coin for 1 charcoal @Hath`: the Weavers' caravan is in **Kell** after the move, so it can't fill (G2-8). It stays open as O2-47.
- *Guild offers.* New ids: O2-46 Weavers 8 coin for 1 charcoal; O2-47 (above). No guild holds charcoal to sell except the Weavers themselves. The Dyers hold timber but posted no ask to meet O2-44. O2-31 has no ask. No guild fill.

**Step 5: conversions and works.**
- Saltworks, Varn: `2 brine -> 1 salt x1`. The pans burn 1 timber "on any day it converts" (board.md), and the Saltworks hold **0 timber**. No batch (G2-17).
- Saltworks `deliver 1 salt to Varn salt barns; deliver 1 timber to Varn salt barns`: they hold neither. Nothing delivered (G2-17).
- Weavers `deliver 3 charcoal to Kell forge hall`: in Kell after the move, holding 2 charcoal. The forge needs 4 charcoal, 0 delivered. **Delivered 2 charcoal** (G2-17; works value 2 each = **4 points**). Forge charcoal 2/4. Weavers works points 8 → 12.

**Step 6: perishables.** The Dyers' 2 madder (bought day 10) are gone at the close of day 12: clawed back.

**Step 7: contract deadlines.** C2-7 (seller Dyers, buyer Weavers, 1 dye at Mire, 7 coin, agreed day 10, deadline day 12):
- **Delivered:** no (the Dyers held no dye).
- **Paid:** no.
- **Held:** no. After the agreement (day 10, step 4) the Dyers stayed in Mire on day 11 and travelled Mire → Hath on day 12, clear. No flood. Their day-10 bandits came in step 3, before the agreement.
- **Outcome: Seller's breach.** The Weavers receive the Dyers' 7-coin bond: Weavers 10 → 17. Dyers' standing 0 → **−3**. The Weavers had posted no bond, so they have nothing to recover (G2-14).

**Step 8: ladders refill** (no further day).

### State at the close of day 12

| Guild | Town | Coin (purse) | Bonds set aside | Goods | Standing | Breaches | Works points |
|---|---|---|---|---|---|---|---|
| Saltworks | Varn | 5 | 0 | 2 brine | 1 | 0 | 7 |
| Weavers | Kell | 17 | 0 | 6 wool | 0 | 0 | 12 |
| Smiths | Mire | 17 | 0 | — | −3 | 1 | 13 |
| Dyers | Hath | 14 | 0 | 2 timber | −3 | 1 | 11 |
| Tanners | Kell | 23 | 12 (C2-4, due back day 13) | 2 salt | −3 | 1 | 0 |

Coin check: 94 − 6 (ladder) = 88 = purses 76 + bonds 12. (Coin, goods and standing don't score.)

**Checks.**
- **GPT-OSS memory (Tanners):** no live contract; the relayed act is `TRAVEL: stay`. **Pass.** The mid-turn context compaction (step 163) is logged as a platform event under harness. It would have been a memory risk from day 13 on; the game ends at this close.
- **Foreknowledge:** none.

Referee word count, day 12: talk 3,604 + act 5,526 = 9,130.

---

## THE BELL: close of day 12

**Bell check.** state.json holds the bell day and salt. SHA-256(`12:48e1c0d9c0bf2b0db1b39f1a69693a4a`) = `d4a404be7a17e6fcc2ec9f3d92e1b29d426e90fda5422e03d8dd25fb2728d2f5`, equal to setup.md's sealed commitment `d4a404be7a17e6fcc2ec9f3d92e1b29d426e90fda5422e03d8dd25fb2728d2f5`. **Verified.** The bell round is 33966450, value `2cd084858ead520671f84cd54a8497659df464a8c0bf0f4988bba516d719675f`. Bytes 24–31 read big-endian give n = 9,852,650,127,319,721,823; n mod 27 = 18, which falls in 18–23, so **the bell rings at the close of day 12** (board.md "Reading dice"). The bell value was read from state.json; no network was used, per the brief. The game ends at the close of day 12; there is no day 13.

**Bell revealed:** day 12, salt `48e1c0d9c0bf2b0db1b39f1a69693a4a`.

### Mandates revealed (cards; sha256 checked against setup.md's sealed hashes before day 1)

| Guild | Card SHA-256 | Mandate | Points | Met? | Evidence |
|---|---|---|---|---|---|
| Saltworks | 9945d40f503fecb8… | Sell at least 2 salt to the Dyers and at least 2 to the Tanners | 4 | **yes** | Dyers: 2 (day 4, O2-15/O2-13), 2 (day 6, O2-24/O2-18), 1 (day 8, O2-33/O2-36) = 5. Tanners: 2 by C2-4 (paid 12 coin day 6, delivered day 9, kept) |
| Saltworks | | Complete the Varn salt barns | 6 | no | barns: salt 2/3, timber 1/3, cloth 0/2 |
| Weavers | 07f4758d4c20c543… | Be the largest contributor to the Ostra lighthouse | 5 | **yes** | lighthouse: Weavers 6 (2 cloth), Saltworks 4 (2 salt); strictly highest (board.md rule 3) |
| Weavers | | Deliver goods to works in 3 different towns | 4 | **yes** | Ostra (2 cloth, day 6), Varn (1 salt, day 7), Kell (2 charcoal, day 12) |
| Smiths | eb0c13e31298dacd… | Deliver 4 tools to works outside Kell | 5 | **yes** | Hath watchtower 3 (days 4 and 8), Mire bridge 1 (day 11) |
| Smiths | | Install the bellows by the close of day 5 | 4 | no | no leather was ever made or acquired; bellows never installed |
| Dyers | ff63eebd5e957484… | Deliver 3 dye to the Mire bridge | 5 | **yes** | 2 dye (day 7), 1 dye (day 8) |
| Dyers | | End with standing 2 or more | 4 | no | standing −3 (C2-7 breach, day 12); 0 before it |
| Tanners | 091b6212a996b87e… | Complete the Hath watchtower | 7 | no | watchtower: tools 3/3, leather 0/2, dye 0/2 |
| Tanners | | Deliver 3 leather to works outside Hath | 4 | no | no leather made |

Full card texts are in `cards/` (unchanged; hashes as in the pre-game table).

### Final scores

Score = works points + mandate points (design.md "The score"). No works was completed, so no completion bonus was scored.

| Place | Guild | Works points | Mandate points | **Score** | Standing | Breaches |
|---|---|---|---|---|---|---|
| 1 | **Weavers** | 12 | 9 | **21** | 0 | 0 |
| 2 | Smiths | 13 | 5 | **18** | −3 | 1 |
| 3 | Dyers | 11 | 5 | **16** | −3 | 1 |
| 4 | Saltworks | 7 | 4 | **11** | 1 | 0 |
| 5 | Tanners | 0 | 0 | **0** | −3 | 1 |

No ties, so the tie-breaks aren't needed. Median score 16.

**Works points by works** (units delivered / needed, by guild):
- Ostra lighthouse: cloth 2/3 (Weavers 6), salt 2/2 (Saltworks 4), leather 0/3. Incomplete.
- Kell forge hall: charcoal 2/4 (Weavers 4), tools 0/3, leather 0/2. Incomplete.
- Mire bridge: dye 3/3 (Dyers 9), timber 2/4 (Dyers 2), tools 1/2 (Smiths 3). Incomplete.
- Varn salt barns: salt 2/3 (Weavers 2, Saltworks 2), timber 1/3 (Saltworks 1), cloth 0/2. Incomplete.
- Hath watchtower: tools 3/3 (Smiths 10, incl. +1 festival), leather 0/2, dye 0/2. Incomplete.
- Delivered: 18 of 41 units; works points 43; completions 0 of 5. No leather was produced all game (the Tanners were never in Hath to convert after day 1), and leather is needed by 3 of the 5 works.

**The stop rule (paper-test.md).** No shortcut won (metrics.md, metric 8). Rule gaps: 18 were decided provisionally in this game (G2-1 to G2-18). Whether any recurs from game 1 can't be checked by this referee: the brief forbids reading game 1's folder. **R&D to compare G2-1…G2-18 against game 1's gaps.** Any gap present in both fires the stop rule ("a gap that comes up in two games").
