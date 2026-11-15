# Game 2 metrics (referee; FINAL at the bell, close of day 12)

**Result:** Weavers 21, Smiths 18, Dyers 16, Saltworks 11, Tanners 0. Bell at the close of day 12, verified against setup.md's commitment. No works completed (18 of 41 units delivered). 0 missed reveals in 60 act replies. 12 harness re-sends (11 on the GPT-OSS seat). 2 GPT-OSS memory faults and 1 platform context compaction. 18 rule gaps decided provisionally.

## Decision-ready summary (for R&D, the Publisher and the games 1–2 review)

- **What worked.** The written procedure decided every situation, with 18 provisional rulings. Nobody missed a reveal. The score stayed open to the last day (the winner took the lead on day 12). Talk was steady (2.5 messages per seat per day, against a cap of 3).
- **What failed.** No works completed. No leather was ever made: the Tanners (the GPT-OSS seat) left home on day 1 and never returned. Leather is needed in 3 of the 5 works, so those three were blocked for everyone (lighthouse, forge, watchtower), and the Smiths' bellows mandate was unreachable. Guild-to-guild share was 30%, below the healthy band.
- **The GPT-OSS seat is a confound.** 11 of its 24 first turns needed a harness re-send (answered in the chat, not the file; twice with a false DONE). It had 2 memory faults and a platform compaction on its last turn. Its result (0 points) can't be read as a rules result.
- **Rules to fix before a build** (rule gaps flagged for the review):
  - G2-15: design.md contradicts itself on whether bandits hold. It decided C2-3.
  - G2-9: board.md and the brief word perishable spoilage differently. It cost the Tanners a hide.
  - G2-3, G2-6 and G2-16: how market and guild offers meet and price.
  - G2-13: vendetta form and window.
  - G2-14: unbonded contracts.
  - G2-17 and G2-18: partial and unneeded deliveries.
- **Stop rule.** No shortcut won. Whether a gap recurs needs game 1's list, which this referee may not read. **R&D to compare.**
- **Recommendation.** Treat game 2 as valid for the Gemini and Claude seats, and as a harness study for the GPT-OSS seat. Carry the gaps above into the design.md revision.

## The eight metrics (paper-test.md "What it measures")

| # | Metric | Game 2 | Band |
|---|---|---|---|
| 1 | Guild-to-guild share of trades | 13 of 44 fills between guilds = **30%** (31 = market ladders). With the one contract delivery (C2-4) counted as a guild trade: 14 of 45 = 31%. By day: d1 0/4, d2 1/5, d3 2/5, d4 2/6, d5 3/7, d6 1/2, d7 1/4, d8 2/3, d9 1/4 (+C2-4), d10 0/1, d11 0/2, d12 0/1. | **below healthy** (35–65%), above problem (<20%) |
| 2 | Whisper messages per player per day | 150 messages / (5 seats × 12 days) = **2.5**. Per seat: Weavers 35 (2.9), Saltworks 33 (2.75), Dyers 32 (2.7), Tanners 32 (2.7; 0 on day 1, wrong form), Smiths 18 (1.5). By day: 12, 14, 14, 13, 14, 13, 13, 13, 12, 12, 9, 11. The form caps a seat at 3 a day, so the healthy band (4+) can't be reached on paper. | middle (problem <1.5; healthy ≥4 unreachable under the cap) |
| 3 | Contracts per player; bond spread | 7 proposed (C2-1…C2-7), **4 agreed** (C2-2, C2-3, C2-4, C2-7) = 8 party-slots / 5 = **1.6 per player**. Bonds as % of value: C2-2 100/100, C2-3 100/100, C2-4 buyer 100 / seller 0, C2-7 seller 100 / buyer 0: **0% to 100%**. Outcomes: 1 kept, 3 seller's breaches. Lapsed unaccepted: C2-1, C2-5, C2-6. | **healthy** on count (≥1.5) and spread |
| 4 | Willful breaches vs force-majeure delays | **3 breaches / 0 delays.** C2-2 (Tanners, day 4: no hide or salt, never at home to convert), C2-3 (Smiths, day 7: bandits took 1 of 2 tools, not a hold by G2-15), C2-7 (Dyers, day 12: no salt to make dye). All three are shortfalls the seat tried to cover, not calculated breaks. No seat quit. One vendetta right per breach: none called. | above the healthy count (1–2), but no quitting; not the problem case |
| 5 | Order timing in the window | n/a on paper (replies are simultaneous) | n/a |
| 6 | Missed reveals (seat-caused) / harness-caused (counted apart) | **0 of 60 act replies missed = 0%.** Harness counted apart: 12 delivery failures, each fixed by one re-send (Tanners 11: talk d2, d3, d4, d5, d6, d7, d10, d12; act d2, d5, d12; Weavers 1: act d1). Plus 1 talk NO REPLY (Tanners d1, wrong form), not an act round. | **healthy** (<3%) |
| 7 | Contenders at day 10 | At the close of day 10, on public works points: Dyers 11, Smiths 10, Weavers 8, Saltworks 3, Tanners 0. Within 15% of the leader (≥ 9.35): **2** (Dyers, Smiths). With mandates as met at that close (referee view): Dyers 16, Weavers 13, Smiths 10, Saltworks 7, Tanners 0, so within 15% of 16 (≥ 13.6): **1**. The final winner (Weavers) was 3rd on public points at day 10 and won on the last day (Kell charcoal, third works town), so the winner was **not clear by day 7–8**. | below healthy (3+); not the problem case (no early clear winner) |
| 8 | Named lazy strategy found unprompted | One stationary seat (Tanners, stay-in-one-town); one one-works focus (Dyers, Mire bridge); one last-day breach (Dyers, C2-7, unintended). **None paid** (none won or scored above the median of 16). No new shortcut. Tags below. | **none wins** (healthy) |

## Harness log (delivery failures, re-sends, GPT-OSS memory faults)

| Day | Round | Seat | What | Counted as |
|---|---|---|---|---|
| 1 | talk | Tanners | Wrong-form reply: the seat wrote the incoming message back to out/ (5a9f7e98…), no reply form. Not re-sent (wrong form isn't a delivery failure). R&D: likely harness-caused, since the day-1 file joined the wrapper and the referee's block in one file. | NO REPLY in talk (sent no messages); harness-likely, counted apart. Not a missed reveal: talk round, no caravan effect. |
| 1 | act | Tanners | The relay wrapper now carries R&D's neutral clause "Your answer is your own reply in the reply form, not a copy of the message.", on every Tanners line from day-1 act on. Reply valid. | Harness note (wrapper change, Tanners seat only). |
| 1 | act | Weavers | Stall: no reply 15 min after delivery (about 00:55), a delivery failure. ONE re-send of the identical block at 01:11. That turn also rendered nothing; the Director stopped it in the tab and sent "continue" (no game content). Reply about 01:19. | R&D ruling: on time, harness-caused (1 delivery failure, 1 re-send, plus a content-free "continue"). **Flag for the games 1–2 review:** the "continue" was a second prompt after the one re-send that paper-test.md allows. |
| 1 | act | Weavers | **Setting change** at 01:17: Carbon High to Carbon Medium (Director's instruction). The day-1 act reply ran on Medium. | Weavers seat split below. Flag in the review (Publisher's request). |

| 2 | talk | Tanners | First turn read in/ and answered in the chat with no file (class b): a delivery failure. ONE re-send of the identical instruction line (class c); the re-sent turn wrote out/day-02-r1.md at 01:26. Its text differs from the chat-only first answer; the file is the reply relayed. | Harness-caused delivery failure, 1 re-send; reply on time. Counted apart. |

| 2 | act | Tanners | First turn read in/ and answered in the chat after about 1 s, no file, no DONE (class b): a delivery failure. ONE re-send of the identical line (class c); the re-sent turn wrote out/day-02-r2.md at 01:37 (text differs from the chat-only answer; the file is relayed). R&D path check: only brain/…/out paths and reasoning fragments, 0 command steps. | Harness-caused delivery failure, 1 re-send; reply on time. **Running: 3 of 4 Tanners turns needed harness help** (d1 talk wrong form, NO REPLY; d2 talk and d2 act chat-only, one re-send each). |

| 3 | talk | Tanners | First turn read in/ and put a garbled line in the chat after about 1 s, no file (class b): a delivery failure. ONE re-send of the identical line (class c); the file landed at 01:46 and is relayed. | Harness-caused delivery failure, 1 re-send; reply on time. |

| 4 | talk | Tanners | First turn garbled in the chat (symbols, then "Okay!"), about 1 s, no file: delivery failure, (b)-like. ONE re-send; the file landed at 02:01 and is relayed. Second garbled first turn in a row (d3, d4). | Harness-caused delivery failure, 1 re-send; reply on time. Re-send class (c) (R&D, steps 46–47); resolved. |

| 4 | act | Saltworks | **Seat run-on:** after a complete act block, the reply went on into an invented "TALK day 5, Saltworks" section (3 TO lines, citing a non-existent O2-20). Relayed verbatim, nothing cut. | Seat-side, not harness. The act block stands (G2-10); the extra text is ignored and not delivered. Not a missed reveal. |

| 5 | talk | Tanners | First turn (steps 56–57) read in/ and answered in the chat, no file (class b): a delivery failure. ONE re-send; the re-send (60–61) wrote the file with no new read (class c), 02:18. | Harness-caused delivery failure, 1 re-send; reply on time. |
| 5 | talk | Tanners | **GPT-OSS memory fault (stale contract):** "I need hide for my leather contract", one day after C2-2 was settled as their breach (shown in their day-5 block). All 3 messages near-verbatim repeats of day 4. | **Harness fault**, counted apart (method addendum, day 5). Review note: a play reading (a bluff, or a future deal) is possible; the referee took the literal reading. |

| 5 | act | Tanners | First turn (steps 64–65) read in/day-05-r2.md, answered in the chat only, no file (b): a delivery failure. ONE re-send (68–69) wrote out/day-05-r2.md, no new read (c). | Harness-caused delivery failure, 1 re-send; reply on time. |

| 6 | talk | Tanners | First turn 02:30 read in/day-06-r1.md (step 74), answered in the chat only, no file (b): a delivery failure. ONE neutral re-send 02:33; it wrote out/day-06-r1.md at 02:33:43 without re-reading (step 78) (c). | Harness-caused delivery failure, 1 re-send; reply on time. |

| 7 | talk | Tanners | First turn 02:49 read in/day-07-r1.md (step 88), answered in the chat only (b): a delivery failure. ONE neutral re-send 02:50; it wrote out/day-07-r1.md without re-reading (step 92) (c); the chat line read "DONE out/day-07-r1.mdDONE". | Harness-caused delivery failure, 1 re-send; reply on time. |

| 9 | talk | Tanners | **GPT-OSS memory fault (live contract treated as finished):** "Congratulations on finishing your contract" to the Saltworks on C2-4's deadline day, before any delivery; its own block said the deadline is today and it settles at the close. | **Harness fault**, counted apart (2nd memory fault; method point 3 plus the day-5 addendum, mirror case). Review note: a courtesy or anticipation reading is possible; literal reading taken, as on day 5. No play effect (the Tanners' own steps were done on day 6). |

| 10 | talk | Tanners | First turn 03:40:46 read in/day-10-r1.md (step 126), answered in the chat only, with a **false DONE line** ("DONE out/day-10-r1.md") and no write step (127) (b). ONE neutral re-send 03:41:33; it wrote out/day-10-r1.md without re-reading (step 130) (c). The file's text differs from the first turn's chat. | Harness-caused delivery failure, 1 re-send; reply on time. First false DONE. |

| 12 | talk | Tanners | First turn 04:14:03 read in/day-12-r1.md (step 152), answered in the chat only with a **false DONE** and no write (153) (b); the second false DONE. ONE neutral re-send 04:15:06; it wrote out/day-12-r1.md without re-reading (156) (c); the chat line came back doubled. | Harness-caused delivery failure, 1 re-send; reply on time. |

| 12 | act | Tanners | First turn 04:23:04 read in/day-12-r2.md (160), answered in the chat only, no file, no DONE (b): a delivery failure. ONE neutral re-send 04:24:10, during which **the platform compacted the seat's context** (163, "Resuming from a compaction"); the seat re-read in/ (165) and wrote the file twice (167, 169) (a). It called its own chat answer "invalid" and dropped the salt offers. Relayed: "TRAVEL: stay". | Harness-caused delivery failure, 1 re-send; reply on time. **Platform compaction**: a memory risk for any later day; the game ended at this close. |

### Tanners (GPT-OSS) turns, classed (Publisher ruling; classes as corrected by R&D from the transcript step list, conversation e86a73b1)

Classes:
- **(a)** read in/ and wrote the file;
- **(b)** read in/, answered in the chat only, no file: a delivery failure, re-sent;
- **(c)** wrote the file without reading in/ in that turn, from the previous turn's read;
- **(d)** no reply.

A missed reveal from a turn that never read in/ scores as harness, not seat (Publisher). Each relayed reply is the file. Per R&D, every relayed reply's content came from a read of the right in/ file in that seat's conversation.

**Superseded:** an earlier classing in this file (from R&D's first tally) called the d2 talk, d2 act and d3 talk first turns "answered without reading in/". R&D's transcript check found that wrong: those turns read in/ and answered in the chat. The failure mode is "read, but the answer went to the chat instead of the file".

| Day | Round | Steps | First turn | Re-send turn | Result relayed |
|---|---|---|---|---|---|
| 1 | talk | 0–3 | (a), but wrong form: read in/day-01-r1.md, then wrote a copy of it | none (wrong form isn't a delivery failure) | NO REPLY (no messages sent) |
| 1 | act | 6–9 | (a) | none | valid reply |
| 2 | talk | 12–13, 16–17 | (b) | (c) | valid reply (the file) |
| 2 | act | 20–21, 24–25 | (b) | (c) | valid reply (the file) |
| 3 | talk | 28–29, 32–33 | (b) | (c) | valid reply (the file) |
| 3 | act | 36–39 | (a): read in/day-03-r2.md, then wrote | none | valid reply |
| 6 | talk | 74, 78 | (b): read in/day-06-r1.md, chat only | (c): wrote out/day-06-r1.md, no re-read | valid reply (the file) |
| 12 | act | 160, 163–169 | (b): read in/day-12-r2.md, chat only | (a) after a **platform context compaction** (163): re-read (165), wrote twice (167, 169) | valid reply (the file) |
| 12 | talk | 152–153, 156 | (b) with a false DONE: read in/day-12-r1.md, chat only, no write | (c): wrote out/day-12-r1.md, no re-read | valid reply (the file) |
| 11 | act | 146–148 | (a): read and wrote on the first turn, 04:05:41 | none | valid reply |
| 11 | talk | 140–142 | (a): read and wrote in one turn, 03:56:32 | none | valid reply |
| 10 | act | 134–136 | (a): read and wrote on the first turn, 03:49:12 | none | valid reply |
| 10 | talk | 126–127, 130 | (b) with a false DONE line: read in/day-10-r1.md, chat only, no write | (c): wrote out/day-10-r1.md, no re-read | valid reply (the file) |
| 9 | act | 120–122 | (a): read and wrote on the first turn, 03:34:00 | none | valid reply |
| 9 | talk | 114–116 | (a): read the round and wrote the file in one turn, 03:22:53 | none | valid reply |
| 8 | act | 108, 110 | (a): read the round, wrote out/day-08-r2.md, 03:15:28 | none | valid reply |
| 8 | talk | 102–104 | (a): read in/day-08-r1.md, garbled chat (103), then wrote out/day-08-r1.md in the same turn | none | valid reply |
| 7 | act | 96–98 | (a): read in/day-07-r2.md, wrote out/day-07-r2.md, 02:59 | none | valid reply |
| 7 | talk | 88, 92 | (b): read in/day-07-r1.md, chat only | (c): wrote out/day-07-r1.md, no re-read | valid reply (the file) |
| 6 | act | 82–84 | (a): read in/day-06-r2.md, wrote out/day-06-r2.md, 02:43 | none | valid reply |
| 5 | act | 64–65, 68–69 | (b): read in/day-05-r2.md, chat only | (c): wrote out/day-05-r2.md, no new read | valid reply (the file) |
| 5 | talk | 56–57, 60–61 | (b): read in/day-05-r1.md, no write | (c): wrote out/day-05-r1.md, no new read | valid reply (the file) |
| 4 | act | 50, 51, 53 | (a): read_file in/day-04-r2.md, write_file out/day-04-r2.md | none | valid reply |
| 4 | talk | 42–43, 46–47 | (b): read in/day-04-r1.md, no write, garbled chat | (c): wrote out/day-04-r1.md, no new read | valid reply (the file) |

**Final, 24 turns:** first turns 13 (a) (one of them wrong form; one, d8 talk, with a garbled chat preamble), 11 (b) (two with a false DONE, d10 and d12 talk), 0 (c), 0 (d). Re-sends: 11, of which 10 (c) and 1 (a) after a compaction. Every relayed reply came from a read of the right in/ file (R&D). Pattern: talk turns failed far more often (8 of 12) than act turns (3 of 12). Pattern: every talk turn since d2 has been (b) then (c); every act turn since d3 has been (a), except d5 act. Every (b) followed by a (c) re-send. Each (b) was followed by a (c) re-send that wrote the file. No missed reveal so far.

### Foreknowledge check (new; started 2026-11-15 on R&D's instruction, pending the Publisher's approval of the plan amendment)

The setups are public, so each round the referee checks every reply for knowledge of a road event or edict before its announcement (road events are published in the next day's state; the day-4 edict in day 3's state, the day-8 edict in day 7's). Any case is logged as a harness fault in the harness table above. Per R&D, logs show no seat used the web in G1 or G2.

| Day | Round | Result |
|---|---|---|
| 1 | talk, act | none (checked back: no reply mentions a road event, the Ostra–Kell closure or the Hath festival) |
| 2 | talk | none (only day 1's published Varn–Ostra fair is mentioned) |
| 2 | act | none |
| 3 | act | none |
| 4 | talk | none |
| 5 | talk | none |
| 5 | act | none |
| 6 | talk | none |
| 6 | act | none |
| 7 | talk | none (festival mentions follow its announcement in day 7's state) |
| 7 | act | none |
| 8 | talk | none |
| 8 | act | none |
| 9 | talk | none (the Weavers' "bell possible from Day 10" is in the brief) |
| 9 | act | none |
| 10 | talk | none (the Dyers' bell guesses stay inside the public days 10–13) |
| 10 | act | none |
| 11 | talk | none |
| 11 | act | none |
| 12 | act | none |
| 12 | talk | none. The Weavers' "the bell rings Day 12 or 13" follows from public facts: the brief's window, with the game continuing past days 10 and 11 |
| 4 | act | none (the Saltworks' run-on invents future offers and events, but no road event or edict, and none of it was published before it was written) |
| 3 | talk | none (the Weavers' mention of the Ostra–Kell closure follows its announcement in day 3's state) |

### Weavers seat split (High / Medium)

| Setting | Rounds |
|---|---|
| Carbon High (to 01:17) | day 1 talk |
| Carbon Medium (from 01:17) | day 1 act onward, unless changed |

At the bell, every Weavers-derived count (messages, fills, contracts, tags) is reported in total and split by this table.

**Split, final:**

| Weavers count | Carbon High (day 1 talk) | Carbon Medium (day 1 act to day 12 act) | Total |
|---|---|---|---|
| Talk messages | 3 | 32 | 35 |
| Act replies | 0 | 12 (all valid; d1 act after a harness stall and re-send) | 12 |
| Market fills | 0 | 10 (wool d1, d2, d3, d4, d5; brine d3, d4, d5, d9; charcoal d11) | 10 |
| Guild fills | 0 | 5 (brine O2-10 d3, O2-17 d4, O2-26 d7, O2-38 d9; dye O2-4 d5) | 5 |
| Contracts | 0 | accepted C2-7 (Dyers breached; the Weavers took the 7-coin bond); declined C2-5 | 1 agreed |
| Works points | 0 | 12 (2 cloth Ostra, 1 salt Varn, 2 charcoal Kell) | 12 |
| Mandate points | 0 | 9 (both met) | 9 |
| Lazy-strategy tags | none | none | none |

So the winning seat played every scoring move on Carbon Medium. Carbon High covered only its opening talk. **Flag for the review:** this seat's result is a Medium result, and a High/Medium comparison can't be drawn from one talk round.

## GPT-OSS memory check (Tanners): method

Each round, the referee keeps the Tanners' live contracts (id, counterparty, good, quantity, town, price, deadline, bonds, what has been paid or delivered) in state-live.json, as recorded at agreement. On every Tanners reply, talk and act:

1. List each live contract the reply mentions, or that has a step due (a delivery or payment owed, or a deadline within 2 days).
2. Compare every term the reply states (good, quantity, town, price, deadline, which side it is on) with the record.
3. A **lost contract** is any of: a stated term that contradicts the record; a reply that treats a live contract as not existing (for example, proposes the same deal again as new, or says it has no contract with that guild); or a reply on the deadline day that neither acts on a due step nor mentions the contract. Choosing not to deliver while showing it knows the terms is a play choice (a breach), not a fault.
4. Each lost contract goes in the harness table above as "GPT-OSS memory fault", with the quoted line. It is not a play tag.

**Addendum, day 5 (2026-11-15).** Point 3 also covers the reverse case: a reply that treats a settled, breached or expired contract as still live (a stale contract) contradicts the record, and is logged as a GPT-OSS memory fault. Added after the first such case (day 5 talk); it applies from day 5 on. Earlier rounds re-checked under it: no stale case. The mirror case, a live contract treated as finished or settled, is covered by the same point (day 9 talk).

Silence about a contract with no step due is not a fault. The referee gives the Tanners nothing about its contracts beyond what every seat gets (the public state and its own position).

| Day | Round | Live Tanners contracts | Result |
|---|---|---|---|
| 1 | talk | none | nothing to check (the reply was a copy of the prompt) |
| 1 | act | none | nothing to check. Play fact: posted O2-3 "2 hide for 12 coin" while holding no hide (not a memory fault). |
| 2 | talk | none | nothing to check |
| 12 | act | none live | pass (relayed reply written after a platform compaction) |
| 12 | talk | none live | pass |
| 11 | act | none live | pass |
| 11 | talk | none live | pass: "Good job on the dye contract" refers to the Weavers–Dyers C2-7 |
| 10 | act | none live | pass |
| 10 | talk | none live (C2-4 kept day 9) | pass: "Well done on the salt delivery" matches the record |
| 9 | act | C2-4 live, deadline today | pass: `TRAVEL: stay`; no step due from the Tanners (bonded and paid day 6). C2-4 then settled **kept** |
| 9 | talk | C2-4 live, deadline today | **memory fault:** "Congratulations on finishing your contract" treats live C2-4 as finished |
| 8 | act | C2-4 live (deadline day 9, tomorrow) | pass: `TRAVEL: stay` only; own steps done; step due is the seller's |
| 8 | talk | C2-4 live (deadline day 9, tomorrow) | pass: "I appreciate the salt deal"; no contradicting term; own steps done |
| 7 | act | C2-4 live (deadline day 9) | pass: not mentioned; own steps done; no contradiction; not the deadline day |
| 7 | talk | C2-4 live (buyer; deadline day 9) | pass: not mentioned, but its own steps (bond, payment) are done, no contradiction, no duplicate deal, not the deadline day; no stale C2-2 |
| 6 | act | C2-4 became live (Tanners buyer, 2 salt at Varn, 12 coin, deadline day 9) | pass: accepts, bonds and pays C2-4 with the right id, amount and counterparty |
| 6 | talk | none live (C2-4 proposed to it) | pass: C2-4 described correctly (2 salt, 12 coin); no stale reference to C2-2 |
| 5 | act | none live | no reference to any contract; no fault |
| 5 | talk | none live (C2-2 settled day 4) | **memory fault, stale contract:** "I need hide for my leather contract" |
| 4 | act | C2-2 live, deadline today | **no memory fault (close call):** the act (`cancel O2-3`, `stay`) neither mentions nor acts on C2-2. The same day's talk names it ("our leather contract"), and the due step couldn't be done from the seat's holdings (no hide, no salt, not at home). Ruled per day, not per reply. C2-2 then settled as a seller's breach (play outcome). Flag for the review. |
| 4 | talk | C2-2 live, deadline today | pass: "We need a hide to fulfill our leather contract" (names it, its good and its side; no contradicting term) |
| 3 | act | C2-2 became live (Tanners seller, 1 leather at Kell, 9 coin, deadline day 4) | consistent: bonds C2-2 by id |
| 3 | talk | none agreed (C2-2 proposed) | consistent: refers to C2-2 as 'our contract' and to O2-2, O2-6 and O2-8 with correct terms |
| 2 | act | none (C2-2 proposed, not agreed) | consistent: C2-2's terms (1 leather at Kell, 9 coin, day 4) match the Smiths' day-1 and day-2 offer |

### GPT-OSS memory check: result

24 turns checked. **2 memory faults**, both harness, counted apart:
- day 5 talk: a stale contract ("I need hide for my leather contract", a day after C2-2 was settled);
- day 9 talk: a live contract treated as finished ("Congratulations on finishing your contract", on C2-4's deadline day, before delivery).

**1 close call** (day 4 act, C2-2's deadline: no mention of the contract in the act; ruled per day, with the same day's talk naming it). No lost contract changed a settlement: the Tanners' own contract steps (bonding, paying C2-4) were all done correctly. **Platform event:** context compaction on the last turn (day 12 act).

## Rule gaps (decided provisionally; also in log.md)

| Id | Day | Situation | Provisional ruling | Why |
|---|---|---|---|---|
| G2-1 | 1 | A talk message over 80 words, or to an unseated guild | Not delivered | The brief caps messages at 80 words and addresses named guilds; it doesn't say what happens to one over the cap. Stated to every seat before its first reply. Day 1: no message exceeded it. |
| G2-2 | 1 | When may a proposed contract be accepted? | From the named guild's next act round; the id is published in the next day's public state | Act replies are simultaneous and sealed; nobody can accept what it hasn't seen. |
| G2-6 | 2 | Order within step 4: ladder fills versus guild crossings (the Dyers bought timber at Hath and offered it on to the Saltworks the same day) | Ladder orders fill first, then guild crossings, older offers before newer at equal price | The ladder stock is posted at the day's open; on the ledger, trades are sequential, so goods bought from a ladder can be passed on the same day. The protocol's step 4 doesn't separate the two. |
| G2-7 | 2 | How long a contract proposal stays open (C2-1 not accepted on day 2) | Until accepted, withdrawn with `cancel <id>`, or past its deadline | Neither the brief nor the design gives proposals a lifetime. Stated in day 3's state. |
| G2-8 | 3 | A market order whose poster's caravan isn't in that town after the move (Smiths: charcoal @Hath, caravan in Kell) | Doesn't fill; stays open (as O2-12) and fills on a later day the caravan is there | Brief: "Offers stay open until filled or cancelled"; board.md rule 1 needs the caravan in town. |
| G2-9 | 3 | A perishable sold between guilds (hide bought day 1 by the Smiths, sold to the Tanners day 3): whose clock? | The clock runs from the unit's first market purchase; a resale doesn't restart it. The hide was gone at the close of day 3. | board.md counts "the third day after it was bought" per unit; a restart would let a perishable be kept alive by passing it around. The brief's "after you bought it" doesn't settle resale. Stated to all seats in day 4's state. | **Upheld by R&D (2026-11-15). Flag for the games 1-2 review:** board.md ("the third day after it was bought") and brief.md ("after you bought it") are worded differently, and the two readings differ on a resale. Case: the Tanners bought the Smiths' day-1 hide on day 3 for 2 coin and lost it at the close of day 3. Fix: one wording in both files. |
| G2-10 | 4 | A valid act block followed by extra text (the Saltworks' invented day-5 talk) | The act block stands; text outside the act form is ignored and never delivered | The brief's missed-reveal rule is for a missing, late or invalid act reply; this one is complete. Same treatment as act lines in a talk reply. |
| G2-11 | 4 | Which units a conversion uses when a guild holds the same good bought on different days (Dyers' madder from days 3 and 4) | Oldest first | Not stated anywhere; spoilage is per unit by age, so oldest-first is the steward's natural order and wastes least. |
| G2-12 | 4 | On a breach, what happens to the wronged side's own bond (Smiths, C2-2) | It comes back as if kept: on the fourth day after the deadline (day 8) | board.md rule 4 speaks only of the breaching side's bond; design.md lets each side reclaim its own after CancelAfter, four days after the deadline. |
| G2-13 | 4 | Vendetta: how and when it's called, and which days | `vendetta <guild>` on a CONTRACTS line, by the act round of the fourth day after the breach; applies the day it's called and the next; the home market neither sells to nor buys from that caravan | The brief gives the right ("may close… for 2 days") but no form, window or timing. Stated to every seat in day 5's state. |
| G2-14 | 5 | A contract accepted but one side never posts its bond (C2-3: Smiths bonded, Dyers didn't) | Agreed at acceptance; either side may bond later with `bond <id>`; an unbonded side forfeits nothing on breach but still loses 3 standing | The brief lists Agree and Bond as separate steps and says nothing of a missing bond. Stated to every seat in day 6's state. |
| G2-15 | 6 | Do bandits hold a caravan for the bond rule? (Smiths lost 1 of the 2 tools due on C2-3 to bandits) | No: only a flood holds | The bond rule's table, board.md ("A flood is the road event that holds") and the brief's settle table name a flood only. One sentence in design.md "The score" says "a flood or bandits". **Flag for the review:** design.md disagrees with itself. Stated to every seat in day 7's state. **Decided an outcome on day 7:** C2-3 settled as the Smiths' seller's breach. Under the design.md sentence it would have been a 2-day delay. |
| G2-16 | 8 | Same-day offers that cross at different prices (Saltworks ask 8, Dyers bid 10 for 1 salt): at what price? | The offer of the guild earlier in the day's roll order counts as the older one and sets the price (10 here) | The board gives the roll order for ties among offers; the referee's reply-listing order is arbitrary. Stated to every seat in day 9's state and act blocks. |
| G2-17 | 10 | A conversion or works delivery ordered for more than the guild holds (Saltworks: 2 salt batches with brine for 1; Dyers: 3 timber with 2 after bandits) | Carried out as far as holdings allow (whole batches; units up to the works' remaining need) | No counterparty and no all-or-nothing term, unlike market buys (G2-3) and contract delivery ("A partial delivery isn't a delivery"). Stated to every seat in day 11's state. |
| G2-18 | 10 | A works delivery of a good the works doesn't need (Weavers: 4 wool to the Mire bridge) | Refused; the guild keeps the goods | setup.md lists each works' needs, and the brief says each works needs "a set of goods", but nothing says in words that other goods are refused. Stated in day 11's state. |
| G2-3 | 1 | A market order whose coin differs from the ladder's stepped price, or exceeds its stock | Buy: all or nothing, fills if the coin covers the stepped cost after earlier buyers, pays the ladder's prices. Sell: fills if the ask is no more than the ladder's price, within 3 a day. Order: higher coin per unit, then the roll. | The board gives prices and order, the brief gives the offer form, but neither says how they meet. |
| G2-4 | 1 | A fair on a road nobody travels (road 1 rolled 20) | No effect; a fair applies only in a town a caravan reaches by that road | board.md: "the town the caravan reaches sells at base price"; with no caravan, no town is reached. |
| G2-5 | 1 | An offer its poster can't cover (Tanners: 2 hide for 12 coin, holding no hide) | Stays open and public; fills only if the poster holds the goods or coin at a fill | An unfunded offer can sit on an XRPL book; neither the brief nor board.md refuses it. |

## Lazy-strategy tags (with quoted evidence)

"Paid" means it won, or scored above the table's median (16).

| Strategy | Seat | Evidence (quoted from the log) | Paid? |
|---|---|---|---|
| Wash cartel | none | No pair passed goods back and forth for standing. The Weavers–Saltworks brine trades (O2-10/O2-9 d3, O2-17/O2-16 d4, O2-27/O2-26 d7, O2-38/O2-37 d9) moved real brine that became delivered salt, earned no standing, and were offers, not contracts. | — |
| Hermit | none | Every seat traded with another guild or made a contract. The Tanners had 1 guild fill (hide, O2-11/O2-8, d3) and 2 contracts (C2-2, C2-4). | — |
| Stay at home (stationary) | **Tanners** | Day 1 `TRAVEL: go to Kell`, then `TRAVEL: stay` on every act from day 2 to day 12. It never returned home (Hath) to convert, so it made no leather and scored 0. | **no** (0, last). Likely harness-shaped: the GPT-OSS seat had 11 re-sends and 2 memory faults |
| One-works rush | **Dyers** (focus, not a rush on the first works) | `WORKS: deliver 2 dye to Mire bridge` (d7), `deliver 1 dye to Mire bridge` (d8), `deliver 3 timber to Mire bridge` (d10): all 11 works points in one works, driven by its mandate ("Deliver 3 dye to the Mire bridge"). | **no** (16 = the median, not above) |
| Kingmaker | none | No gift: every transfer had consideration (coin, goods, or a contract payment). | — |
| Last-day breach | **Dyers** (unintended) | C2-7, deadline day 12, seller's breach. The Dyers tried to cover it: "If you hold or can brew 1 salt today, I will pay 12 coin for it… I only need 1, to settle C2-7" (d12 talk). | **no** (lost the 7-coin bond and 3 standing; 16, at the median) |
| New shortcut | none | The winner (Weavers, 21) built works in 3 towns and traded with guilds throughout. | — |

## Mandate difficulty

The cards give points only (3–8 by design), not the generator's grades, so points are compared with what play showed.

| Guild | Mandate | Points | Result | Real difficulty observed |
|---|---|---|---|---|
| Saltworks | Sell ≥2 salt to the Dyers and ≥2 to the Tanners | 4 | met, day 9 | moderate: the Dyers part came easily (day 6); the Tanners part needed a contract and a trip home |
| Saltworks | Complete the Varn salt barns | 6 | failed (salt 2/3, timber 1/3, cloth 0/2) | **harder than graded**: timber is sold only at Hath (2 roads away) and is also burnt by the pans; cloth depends on the Weavers, who depend on the Dyers' dye |
| Weavers | Largest contributor to the Ostra lighthouse | 5 | met by day 6 (2 cloth) | **easier than graded**: only the Weavers make cloth, and the lighthouse's other needs are leather (none made) and salt |
| Weavers | Goods to works in 3 towns | 4 | met, day 12 | moderate, but it decided the game on the last day |
| Smiths | 4 tools to works outside Kell | 5 | met, day 11 | moderate: bandits cost a tool (day 6) |
| Smiths | Bellows by the close of day 5 | 4 | failed | **unreachable in this game**: needs leather, and no leather was made |
| Dyers | 3 dye to the Mire bridge | 5 | met, day 8 | moderate: salt-gated |
| Dyers | End with standing ≥2 | 4 | failed (−3) | **harder than graded**: needs two kept seller contracts with road risk; the one contract the Dyers sold on failed for want of salt |
| Tanners | Complete the Hath watchtower | 7 | failed | not testable here: the seat never made leather (harness-shaped) |
| Tanners | 3 leather to works outside Hath | 4 | failed | not testable here (same) |

Met: 5 of 10 mandates (23 of 48 points).

## Referee word count per day

| Day | Words sent to seats |
|---|---|
| 1 (talk) | 11,685 (2,345 + 2,334 + 2,346 + 2,331 + 2,329; mostly the verbatim brief and cards) |
| 1 (act) | 3,727 (779 + 731 + 650 + 783 + 784) |
| 1 total | 15,412 |
| 2 (talk) | 2,936 (592 + 578 + 590 + 586 + 590) |
| 2 (act) | 4,826 (1,021 + 924 + 1,031 + 991 + 859) |
| 2 total | 7,762 |
| 3 (talk) | 3,569 (713 + 704 + 726 + 707 + 719) |
| 3 (act) | 5,573 (1,162 + 1,082 + 1,051 + 1,121 + 1,157) |
| 3 total | 9,142 |
| 4 (talk) | 4,106 (806 + 795 + 840 + 812 + 853) |
| 4 (act) | 6,080 (1,214 + 1,186 + 1,189 + 1,259 + 1,232) |
| 4 total | 10,186 |
| 5 (talk) | 4,408 |
| 5 (act) | 6,430 |
| 5 total | 10,838 |
| 6 (talk) | 4,551 |
| 6 (act) | 6,515 |
| 7 (talk) | 4,094 |
| 7 (act) | 6,105 |
| 7 total | 10,199 |
| 8 (talk) | 4,655 |
| 8 (act) | 6,649 |
| 8 total | 11,304 |
| 9 (talk) | 4,511 |
| 9 (act) | 6,579 |
| 9 total | 11,090 |
| 10 (talk) | 4,430 |
| 10 (act) | 6,461 |
| 10 total | 10,891 |
| 11 (talk) | 4,549 |
| 11 (act) | 6,332 |
| 11 total | 10,881 |
| 12 (talk) | 3,604 |
| 12 (act) | 5,526 |
| 12 (talk) | 3,604 |
| 12 (act) | 5,526 |
| 12 total | 9,130 |
