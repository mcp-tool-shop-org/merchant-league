# Paper test: results

The paper test of the ruling-1 design (`docs/paper-test.md`) ended after **two games** (Publisher, 2026-11-15). The v2 design (`docs/design-v2.md`, merged in #13) was approved in the meantime, and it replaces the design these games tested. So this summary has two jobs:
- report what the two games showed;
- say which findings still apply to v2.

| | Game 1 | Game 2 |
|---|---|---|
| Seats | 4: Claude, GPT-OSS, Gemini ×2 | 5: Claude ×2, Gemini ×2, GPT-OSS |
| Bell | close of day 11 | close of day 12 |
| Winner | Foresters 41 (next: 21) | Weavers 21 (next: 18) |
| Works completed | 2 of 5 (Mire causeway, Hath lodge) | 0 of 5 |
| Records | [`game-01/`](game-01/) | [`game-02/`](game-02/) |

Every seal held. All nine cards and both bells were opened after the bell and match the hashes committed before day 1. Each bell day follows from its drand round.

## 1. Which bands the tables landed in

| # | Metric | Game 1 | Game 2 |
|---|---|---|---|
| 1 | Guild-to-guild share of trades | 40%, healthy, but 9 of 10 guild trades were between one pair | 30%, below healthy |
| 2 | Whisper messages per player per day | 2.45 | 2.5 |
| 3 | Contracts per player | **0: problem** | 1.6, healthy; bonds from 0% to 100% |
| 4 | Breaches vs delays | none (no contracts) | 3 breaches, 0 delays; all were shortfalls a seat tried to cover; nobody quit |
| 5 | Order timing | n/a on paper | n/a on paper |
| 6 | Missed reveals | 4.5%: 2, both harness-caused; 0% net of harness | 0%, with 12 harness re-sends counted apart |
| 7 | Contenders at day 10 | **1: problem**; the winner was clear from day 7 | 2, below healthy; the lead changed on the last day |
| 8 | A named lazy strategy found unprompted | none won | none won |

**On metric 2:** the plan capped talk at 3 messages a day, so its healthy band of "4 or more" was out of reach by construction. Both games ran close to the cap.

## 2. Which shortcuts appeared, and whether they paid

None paid. No strategy won without building works and trading.

- **Game 1.** The Weavers played as a hermit, trading only with the town markets, and came last. The winner traded and built works, but won big for two reasons:
  - **Its converter needed no other guild.** The Foresters cut timber free at home.
  - **It finished a works alone.** It bought the scarce sit-out good (dye at 14) to complete the Mire causeway in one day, outbidding the Weavers, who needed that dye for their own converter.
- **Game 2.** Three tags, none of which paid:
  - **The Tanners never left Kell after day 1** (0 points). That's shaped by the harness more than by play; see section 4.
  - **The Dyers put all their works points into one works,** led there by their mandate. They scored 16, the median.
  - **One last-day breach,** unintended: the Dyers lacked the salt to cover it.

## 3. Which rules failed to decide

The written rules didn't settle **12 situations in game 1 and 18 in game 2**. The referees decided each one provisionally; every case is in the games' `metrics.md`.

**The stop rule fired by its letter.** Four gaps came up in both games:

| Gap | Game 1 | Game 2 | Both referees ruled |
|---|---|---|---|
| A market bid that doesn't match the stepped ladder price | G3 | G2-3 | all or nothing, at the stepped total |
| A conversion or delivery ordered beyond what's held | G5, G8 | G2-17 | carried out as far as holdings allow |
| Competing bids for a ladder's stock | G7 | G2-3 | ranked by coin per unit, then the day's roll |
| An offer its owner can't fund when posted | G9 | G2-5 | stays open, and fills only if funded at the fill |

The recurrence was expected. Game 1's rulings were never written into the pinned rules, and game 2's referee wasn't allowed to read game 1's records. Both referees independently ruled each gap the same way, so the readings are consistent. **It doesn't stop anything now:** no paper games follow, and v2 replaces these rules.

**The design contradicted itself twice**, both found in game 2:
- **Bandits as force majeure.** One sentence in `design.md` lets bandits excuse a late delivery, while the bond rule and the board count only a flood. That decided contract C2-3: a seller's breach, where the other reading gives a 2-day delay.
- **The spoilage clock on a resold good.** `board.md` and the brief word it differently. It cost the Tanners a hide.

## 4. What the harness showed

- **The one-re-send rule worked.** From game 2 on, a delivery failure got one neutral re-send. Game 2 had 0 missed reveals, with 12 re-sends logged as harness.
- **The GPT-OSS seat was a confound.** In game 2, 11 of its 24 first turns answered in the chat instead of the reply file, and it had 2 contract-memory faults. Its context was compacted on the last turn. Its 0 points can't be read as a rules result.
- **A seat changed models mid-game.** Game 2's winning seat moved from Carbon High to Carbon Medium after its first talk round, on the Director's instruction. Its whole result is a Medium result.
- **No seat used outside knowledge,** as far as the logs show. No web tools were used in either game, and nobody named a road event or an edict before it was announced.
- **Refereeing by hand is heavy.** Game 1's referee wrote about 15,800 words of log. Game 2 sent seats about 10,000 words a day.

## 5. What carries into v2

v2 is a different game. It has no towns, caravans or roads: seats sit on a ring and trade through the ledger's own books. That decides most of the list.

**No longer applies:**
- the location rule;
- bandits against floods;
- the vendetta's form;
- per-unit spoilage clocks (v2 decays a whole pool each turn);
- the stepped town markets, which v2 replaces with real order books.

**Still applies.** These are worth making checks in v2's first playtest:

1. **Seats trade on the spot, not by contract, when same-day trade is enough.**
   - Game 1 made no contracts: a crossing offer needed no shared town, so a contract added only time and an up-front bond.
   - v2 makes the choice explicit, collateral (escrow) against credit (Checks), and already flags "starting liquidity too generous" as a risk in its playtest metrics.
   - **Keep that check, and measure it early.**
2. **One weak player can stall the whole table.**
   - In game 2, one seat that never produced its good blocked 3 of the 5 works for everyone, and no works was finished.
   - v2's Regent covers a player who is absent: 3 missed cutoffs turn the seat over to AI play. It doesn't cover one who is present but doesn't produce.
   - **Add a playtest check, or a design guard, for a chain blocked by a seat that is present but not producing.**
3. **A self-sufficient role wins big.**
   - The game-1 winner's converter needed no other guild, and it won by 41 to 21.
   - v2's generator filter ("zero 2-seat self-sufficiency", seat equity within ±3%) targets this by design.
   - **Keep it a tested property, not an assumption.**
4. **How offers fill should be written down.**
   - Both games needed rulings on partial fills, competing bids and unfunded offers.
   - On v2's real order books the ledger decides fills and the ranking of competing bids. The client rules should still say what a player sees and gets, and how an offer that can't be paid for behaves.
   - v2 doesn't state these yet.
5. **Mandates need grading by play, not by guesswork.**
   - Several played harder or easier than their points.
   - One became unreachable when a producer was missing: the Smiths' bellows needed leather.
   - v2's secret mandates should be graded by simulation, and refused when another seat's absence can make them impossible.
6. **Don't set a metric band a cap makes unreachable** (metric 2 above).
7. **The AI-seat harness, for v2's AI mode:**
   - pin each seat's model and settings for the whole game;
   - keep the one-re-send rule and log every re-send;
   - treat a seat that fails to write its reply file as a harness fault, not as play.

## Recommendation

- **Take these results as the paper test's final record.** Nothing in them needs another paper game.
- **Carry items 1 to 7 into v2's first playtest** as named checks.
- **One small decision for the Publisher:** `scripts/check-copy.sh` refuses one word a seat wrote in game 2's verbatim log. It's on this branch only, which has no open PR and no CI on push, so nothing is red today.
  - If these records ever go to `main`, the check needs either an exemption for recorded play or a redaction.
  - Recommendation: the exemption, so the record stays verbatim.
