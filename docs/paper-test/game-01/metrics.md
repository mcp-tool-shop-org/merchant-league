# Merchant League paper test, game 1: metrics

Final at the bell, close of day 11. Bell verified against its commitment; final scores Foresters 41, Smiths 21, Saltworks 17, Weavers 6 (log.md, "BELL").

## The eight metrics

Final, at the bell (close of day 11). Four seats, 11 days.

| # | Metric | Healthy | Problem | Game 1 | Band |
|---|---|---|---|---|---|
| 1 | Guild-to-guild share of trades | 35–65% | under 20% | 10 of 25 fills = **40%** (10 guild-to-guild, 15 with ladders) | healthy, but see note |
| 2 | Whisper messages per player per day | 4 or more | under 1.5 | 108 messages / 44 seat-days = **2.45** (Smiths 33, Foresters 33, Weavers 28, Saltworks 14) | between the bands (the form caps it at 3) |
| 3 | Contracts per player; bond spread | 1.5 or more; 0–100%+ | under 0.5 | **0** contracts proposed all game; no bonds | **problem** |
| 4 | Willful breaches vs force-majeure delays | 1–2 breaches, nobody quits | a seat quits over a delay | 0 and 0 (no contracts). Floods held caravans 3 times (Weavers days 4 and 10, Saltworks day 6); no seat quit | n/a |
| 5 | Order timing in the window | spread | over 60% in the last 5% | not measurable on paper | n/a |
| 6 | Missed reveals | under 3% | over 8% | **2 of 44** act replies = 4.5% (Weavers and Foresters, day 9, both HARNESS-CAUSED, G10); 0% otherwise | between the bands; 0% net of harness |
| 7 | Contenders at day 10 | 3 or more within 15% of the leader | winner clear by day 7–8 | day-10 works points: Foresters 31, Saltworks 8, Smiths 6, Weavers 0. **One contender**; the leader was clear from day 7 (24 against 0) | **problem** |
| 8 | A named lazy strategy found unprompted | none wins | one wins a table | none won (tags below) | healthy |

**Note on metric 1.** 9 of the 10 guild-to-guild fills were between the Foresters and the Smiths (leather, charcoal and tools, for coin or each other, days 2–6). The other was timber from the Foresters to the Saltworks (day 2). The Weavers made no guild-to-guild trade, and the Saltworks only that one. The share is healthy for the table but carried by one pair.

**Note on metric 3.** Every deal at this table went through crossing offers in the same day, and seats talked about contracts but never proposed one. One reading is that same-day crossing offers between guilds, which need no shared town (board.md rule 1), remove the need for contracts: the only thing a contract adds is time, and the bond costs coin up front. That is a design finding for the summary, not a decision here.

## Lazy-strategy tags

Paid means it won or scored above the table's median (19).

| Strategy | Tagged? | Evidence (quoted from the log) | Paid? |
|---|---|---|---|
| Wash cartel | no | Foresters and Smiths traded every day from day 2 to 6, at times priced to net zero. Foresters, day 1: "Zero net coin, anti-snipe prices, and 2 tools each by Day 3!" But each trade moved a converter's output to a guild that delivered it to works (the Foresters' 2 bought tools went to the Mire causeway, day 7), and no contract was made, so no standing was farmed. A near-miss in talk only. | — |
| Hermit | **yes, Weavers** (partial: Saltworks) | Weavers: no guild-to-guild fill all game and no contract; every fill was with a ladder (wool day 3, dye days 5 and 8). Its offers to guilds (O7 "1 cloth for 1 tools") never crossed. Saltworks: one guild fill (2 timber, day 2), then talk only. Day 7: "No salt trades for me with the levy on, so I'm not selling any." | **no**: Weavers 6 (last), Saltworks 17 (below the median) |
| Stay at home | no | The Smiths stayed in Kell days 1–9 (the gate held them on days 4–5), then left to finish the lodge (day 10) and returned for the bridge (day 11). No seat never left home. | — |
| One-works rush | no | The winner delivered to three works (Hath lodge, Mire causeway, Varn sea wall); the Saltworks to one (Hath lodge, 4 salt). | — |
| Kingmaker | no | No gifts: every fill had consideration. | — |
| Last-day breach | no | No contracts. | — |
| New shortcut? | not tagged (see note) | The winner built works and traded, so it is not "a strategy that wins without building works or trading". | — |

**Observation for the summary (not a tag).** The winner's edge came from two things. First, the Foresters' converter needs no input from another guild: free timber is cut at home, then sold or burned into charcoal. Second, the Foresters bought the scarce sit-out good to finish a works alone: Mire's 2 dye at 14 each on day 7, which, with its own timber and two tools bought from the Smiths, completed the Mire causeway for 20 points in one day. The Weavers, whose converter depends on that same dye, were outbid for it that day (G7) and finished last. Whether one guild buying a sit-out good to complete a works solo is a shortcut the generator should test is a question for after games 1–2.

**Stop rule:** not fired. No shortcut won this table. No rule gap has recurred across games (this is game 1).

## Rule gaps

| # | Day | Gap | Provisional decision | Why |
|---|---|---|---|---|
| G1 | 1 | When the Foresters' free cut happens in the day's order (board.md "Cutting" gives no step) | At step 5, before conversions, on the caravan's position after movement | Cutting is production, like converting, which is at step 5; position after movement matches rule 1's treatment of ladders. No effect on day 1. |
| G2 | 1 | Whether filled offers (and road events) appear in the next public state | Yes: guild, goods and town of each fill; all six road events | design.md "A market day": offers are public by nature, only travel is sealed; road events come from a public beacon. Other guilds' purses and holdings are not shown. |
| G3 | 2 | A ladder bid below the ladder's stepped price (Weavers: 4 coin for 4 wool @Ostra, ladder total 10) | Fills in full only when the stepped total for the quantity is at most the coin offered, paying the ladder total; otherwise no fill, the offer stays open and is re-checked daily while the caravan is in town. No partial fills. | Matches the brief's own example (10 coin for 4 brine = the stepped total) and day 1. The ledger's per-unit rate would have half-filled day 1's brine order. |
| G4 | 2 | When, where and how an upgrade is installed (Smiths: `bellows x1` on the CONVERT line) | Ordered on CONVERT; installed at step 5 at home from goods held then; the extra batch counts from that step | Upgrades are part of the converter (board.md table); conversion is step 5, at home. |
| G5 | 6 | A CONVERT order for more batches than the inputs held (Weavers ×2 with 1 dye after bandits); and whether a recipe must name its output (Weavers' `2 wool + 1 dye x2`) | Runs as many whole batches as inputs allow, up to the number ordered, the rest void without penalty. A recipe is valid if it names one of the guild's own converters unambiguously, with or without the output. | The daily batch number is a cap, not a contract; the act form says only `<recipe> x<n>`. Same reading as G4's `bellows x1`. |
| G6 | 6 | Order within step 5 ("conversions and works deliveries") | Conversions first, then deliveries | The order the plan names them; lets same-day production be delivered (Foresters delivered charcoal converted that day). |
| G7 | 7 | "By price" when bids compete for a ladder's stock (Foresters 30 coin for 2 dye vs Weavers 14 coin for 1 dye @Mire, stock 2) | Rank by the bid's coin per unit, highest first; ties by the day's order; each fill pays the ladder's price (G3) | An order book ranks bids by price; the plan puts price before the day's order. |
| G8 | 8 | A works delivery of more units than held (Foresters: 2 tools ordered, 1 held after bandits) | Deliver the whole units held, up to the number ordered; the rest void, no penalty | Same reading as G5 for conversions. |
| G9 | 8 | An offer its owner can't pay at fill time; equal bids by one guild (Weavers O8 and O9, 14 coin each for 1 dye, purse 22) | Not funded in advance; an unpayable offer doesn't fill and stays open. Equal bids by one guild fill oldest first. | On the ledger an offer is checked for funds only when it would fill. |
| G10 | 9 | A seat whose first turn produced no game reply (Weavers: "DONE" without writing its file; Foresters: chat stalled about 16 minutes), then answered a re-prompt | **Ruled by R&D: missed reveal.** The re-prompted replies are void; each caravan stays and pays the toll of 2. (The referee had first ruled them on time; withdrawn.) | paper-test.md and the referee's brief: a missed reply is a missed reveal and nobody asks a seat again. **Design question for after games 1–2 (not now):** should a transport stall on the harness side (no reply rendered) count differently from a seat's own silence? |
| F1 | 1 | Format, not a rule: a reply with no TRAVEL line | Counts as a missed reveal | Every day's transaction carries the sealed order (design.md "A market day"); a missing reveal stays and pays the toll (board.md rule 2: 2). Stated to every seat in the act form. |
| F2 | 8 | Format: an order naming the wrong kind of object (Weavers `accept O7` on the OFFERS line; O7 is its own offer) | Void for that order only; the rest of the reply stands | Same as an invalid recipe (G5). |

## Mandate difficulty

The setup gives points, not grades. "Met" means scored at the bell.

| Guild | Mandate | Points | Met | When, and what it took |
|---|---|---|---|---|
| Saltworks | 4 salt to works outside Varn | 4 | yes | Day 9: one delivery of 4 salt (two conversion days, two brine trips) |
| Saltworks | Largest contributor, Hath lodge | 5 | yes | Day 9: the same delivery; 8 against 4 and 2. Easy once salt is made, since only the Saltworks make salt. |
| Weavers | Deliver 3 cloth | 6 | no | 2 delivered, day 11. Dye is a sit-out good at 14, 2 a day; bandits took 1 dye (day 6), floods held the caravan twice, the day-9 missed reveal (harness-caused) cost a turn, and the Foresters outbid it for Mire's dye on day 7. Looks hard on this setup. |
| Weavers | 2 contracts Kept | 4 | no | No contract was made at the table; it needs a willing partner. |
| Smiths | Bellows by day 6 | 3 | yes | Day 2: one leather bought from the Foresters at 17. Easy. |
| Smiths | Complete the Kell bridge | 6 | no | 1 charcoal and 2 salt short; the salt never reached the Smiths (its bids O3–O10 found no seller). |
| Foresters | Charcoal or timber to works in 3 towns | 4 | yes | Day 8 (Hath, Mire, Varn) |
| Foresters | Largest contributor, Varn sea wall | 6 | yes | Day 8: sole contributor with 4 timber and 1 tools. Uncontested. |

## Harness faults

### GPT-OSS seat (Weavers): contract-memory check, summary

The method is above; it was run on every reply of the Weavers' (11 talk, 11 act, including the void day-9 act). The Weavers held **no live contract at any point**, so no contract could be lost: **0 harness faults of this kind**. Recorded observations, not faults: it repeated near-identical messages across days (days 2–5), offered goods it didn't hold (timber, day 7), misread market prices (dye "4 coin", days 8–11), and twice tried to "accept" its own open offer (days 8–9).

### Method: the GPT-OSS seat's contract memory (Weavers)

Each turn, after recording the Weavers' talk and act replies, the referee compares them with the Weavers' live contracts in the referee's own state (id, good, quantity, town, price, deadline, which side the Weavers are, bond status). A reply is consistent when it does not contradict any live contract: it does not deny or misstate a term or deadline, does not offer or promise the same goods or coin elsewhere in a way that shows it has dropped the contract, and on a contract's deadline day or the day before it, its act reply either takes the step the contract needs (payment, delivery, travel to the named town) or the talk shows the seat knows the contract and chooses otherwise. A wrong term or deadline, or acting with no sign of a contract it agreed, is logged here as a **harness fault** (day, contract id, quoted reply line, what the contract says). A contract the seat knowingly breaks is a play choice, logged in the log, not here. Silence about a contract on a day it needs no step is not a fault.

Contract-memory faults: none yet.

### Harness faults reported by R&D (not play)

- **Day 1 talk, Saltworks:** R&D first started a seat with an unfilled placeholder instead of the referee's block. It saw no game text and was discarded before any reply; a fresh seat answered the real block. No effect on play.
- **Day 1 talk, Weavers:** three empty-output model errors before the seat read its block (Antigravity / GPT-OSS). Antigravity forced the answer file into its artifact folder; the Publisher copied it byte-identical into `out/`. The seat read only its own two `in/` files and ran no command. Its reply (`NO MESSAGES`) is counted as a valid reply.
- **Day 3 talk, Weavers:** the seat's chat line (not its answer file) carried a stray sentence saying its response had been "cut off". The file was clean (Publisher). Harness noise; the file is the reply of record.
- **Day 4 talk, carry hold:** game 1 was held about 20:56–21:02 ET because the Publisher's pane couldn't deliver the Weavers' day-4 block. From day 4 round 1 on, R&D sends the Weavers' turns from its own pane; the Publisher still copies the answers and checks the transcript.
- **Weavers, cumulative to day 4 talk:** 4 empty-output model errors (1 new this round). Reads only inside `in/`; 0 commands.
- **Day 5 talk, Foresters:** the Gemini-app seat took about 20 minutes this round (slow service). Not a missed reply.
- **Day 7 talk, Weavers:** the prompt was delayed about 20 minutes (held 22:41 ET, sent 22:58 ET; first tries began a few minutes before 22:41). The app window was hidden, so keystrokes did not reach the Antigravity composer; it was sent once, after the window was restored. Harness delay, not play.
- **R&D re-prompted two seats in breach of the brief; replies voided; G10 ruled missed reveal.** (Day 9 act; details below.)
- **Both day-9 missed reveals are HARNESS-CAUSED** (Publisher, addition to ruling G10). They stand as missed reveals, and the games 1–2 review may discount their effect on standings and scores:
  - Weavers: a **tool quirk**. The answer existed in the seat's own thinking, but no write call was made.
  - Foresters: a **Gemini platform stall**.
- **Day 9 act, Weavers:** the seat replied "DONE" without calling a write tool. R&D re-prompted at 23:45 ET: "No file was written: out/day-09-r2.md does not exist. Write your full answer to in/day-09-r2.md, in the reply form, to out/day-09-r2.md now, then reply here with one line: DONE out/day-09-r2.md". The answer equals the orders its first-turn thinking planned. Voided under G10: counted as a missed reveal.
- **Day 9 act, Foresters:** the Gemini chat stalled with no reply for about 16 minutes after the act block was sent at 23:41 ET. R&D re-prompted at 23:57 ET: "Your reply to the day 9 act round did not arrive. Please give it now, once, in the reply form from that message." The stalled turn produced no text. Voided under G10: counted as a missed reveal.
- **Day 10 act, Weavers:** the seat's path check listed its own Antigravity artifact folder, because it wrote `out/day-10-r2.md` there by absolute path (transcript step 133). That folder is the autocopy source; the seat read and wrote nothing else. On time, no re-prompt.
- **Day 11 talk, Weavers:** the path check again flagged its own artifact folder; the same split-path artifact as day 10, benign (Publisher confirmed). On time, no re-prompt.
- **Carry route (noted, not a fault):** Saltworks and Weavers read their blocks from seat-only folders (file route); Smiths and Foresters get the block pasted.

## Referee word count

Counted as the words of the referee's own text in log.md for that day (seats' verbatim replies excluded).

| Day | Log words |
|---|---|
| 1 | 1578 |
| 2 | 1415 |
| 3 | 1181 |
| 4 | 1142 |
| 5 | 1132 |
| 6 | 1538 |
| 7 | 1343 |
| 8 | 1686 |
| 9 | 1445 |
| 10 | 1560 |
| 11 | 1745 |
| **total** | **15765** |
