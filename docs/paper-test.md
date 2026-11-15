# Merchant League paper test

Plan of 2026-11-14, for the Publisher's approval. R&D routes the seats once it's approved.

**What this is.** The design (`docs/design.md`) played on paper by AI players: no ledger and no game code. The Director ruled on 2026-11-14 that he doesn't play. The AI crew plays, over several games, and the design goes back to him with the results.

**What it's for.**
- Do players who've never been told the shortcuts find them?
- Does the market come alive?
- Do the written rules decide every situation?

## Standards compliance

| Standard | Score | Evidence |
|---|---|---|
| PIN_PER_STEP | 2 | Each game pins its setup (and that setup's hash), the seat brief's version, each seat's model and surface, and the drand rounds it uses, in `setup.md` before day 1. |
| ANDON_AUTHORITY | 2 | The stop rule halts the test when a shortcut wins two tables. A situation the rules don't decide is logged and decided provisionally. If it recurs, the test stops. |
| NAMED_COMPENSATORS | 2 | The only irreversible acts are commits and a PR; see "Compensators". There's no ledger, no money and no external post. |
| DECOMPOSE_BY_SECRETS | 2 | The referee holds what changes during play (state, sealed orders, the bell day). Each seat holds only its own brief and mandates. The rules (`design.md`) stay fixed for the whole test unless the stop rule fires. |
| UNCERTAINTY_GATED_HUMANS | 2 | The Publisher approves the plan and is told when the stop rule fires. Each game otherwise runs unattended. |
| EXTERNAL_VERIFIER | n/a | No specialized claims. The cross-family seat mix is a design choice for play, not a verifier. |

## Who does what

- **The referee** is one Claude session that doesn't play. It runs the steward's written procedure (`design.md`: the market day, the bond rule, scoring) step by step, by hand. It deals the setup, holds the sealed orders, resolves each day, and writes the log. It never advises a seat or comments on strategy.
- **The seats** are 4 or 5 AI players per table, one per seated guild. Each seat is its own conversation, with no shared context, and never sees another seat's brief or mandates.
- **R&D** routes the seats: it opens the Gemini conversations, the Antigravity card and the Claude subagents, and carries each day's prompt and reply between the referee and the seat.
- **The Book Keeper** builds the six setups before game 1 and writes the summary at the end.

## Games, setups and seats

**Six games**, each with a different setup. The generator is designed but not built yet, so the setups are made by hand to its recipe:
- different works and their needs;
- different edict kinds;
- different ladder spreads;
- a distinct skeleton per setup.

All six setups are fixed and committed before game 1. Only the stop rule can change them.

The generator's proof (lazy strategies must lose) is what these games test. A shortcut found here becomes a named strategy the generator must beat.

**Seats and sit-outs.** Six guilds, so 4- and 5-seat tables alternate. Every guild seats in at least four of the six games, which is the design's "two of every three" floor.

| Game | Seats | Sits out |
|---|---|---|
| 1 | 4 | Dyers, Tanners |
| 2 | 5 | Foresters |
| 3 | 4 | Saltworks, Smiths |
| 4 | 5 | Weavers |
| 5 | 4 | Foresters, Tanners |
| 6 | 5 | Dyers |

Each guild sits out once or twice. A sitting-out guild's good is put on a town ladder at the worse price the design gives it.

**Day count.**
- Each game's length is fixed before day 1: the referee draws the bell day from the design's bell odds, using a drand round fetched in advance. It writes the day into `setup.md` as a SHA-256 commitment (the day plus a salt), and reveals it at the end.
- Seats know only that the bell rings between days 10 and 13, as in the design.
- This keeps the work bounded and planned, and keeps the endgame unknown to the players.

## The seat mix

**Families available:**
- **Gemini:** Carbon in AI Studio and the Gemini app. These are one family.
- **Claude:** subagents, on Sonnet 5.5 by the studio rule.
- **Local (optional):** a local open-weight model of another family. It needs the Publisher's GPU grant.
- **GPT-OSS 120B, while available.** It runs on the one Antigravity card, which draws on Antigravity's Claude and GPT quota, not the Gemini one. Antigravity's models page says it will be removed on 18 November 2026.

**The mix** (Publisher, 2026-11-14, on the Director's suggestion): each game has **one GPT-OSS seat**, with one Antigravity conversation open at a time.

| Table | With GPT-OSS | Fallback |
|---|---|---|
| 4 seats | 2 Gemini, 1 Claude, 1 GPT-OSS | 2 Gemini, 2 Claude |
| 5 seats | 2 Gemini, 2 Claude, 1 GPT-OSS | 3 Gemini, 2 Claude |

- **When the fallback applies.** A game that can't start and finish by the end of 17 November ET uses the fallback mix (the Publisher's earlier ruling, relayed by R&D, the same day). Each game's `setup.md` gives both seatings.
- **Why these shapes.** No family has more than two seats at a table, and the Claude seats never outnumber the rest, because the referee is a Claude session. The fallback is the plan's mix without a third family.
- **A clean project.** The GPT-OSS seat opens in a clean Antigravity project whose tree holds no other seat's setup, brief, mandate or order. Its card says to read no files and to play only from the conversation; every card says so.
- **A memory check.** Each turn, the referee checks that the GPT-OSS seat still remembers its live contracts. A lost contract is logged in `metrics.md` as a harness fault, not a play choice.
- **A local model,** if one is granted, would replace a Gemini seat in the fallback mix.

**Within a family, seats are as separate as possible:**
- the two Gemini seats use different surfaces (AI Studio and the Gemini app);
- the Antigravity card is the GPT-OSS seat, not a Gemini seat;
- each Claude seat is a fresh subagent.

**Rotation.** Each family plays every guild across the six games: GPT-OSS plays each guild once, and Gemini and Claude each play every guild, under either seating. That stops a guild's result from being a family's result. The assignment is written into each game's `setup.md`.

**No outside knowledge** (Publisher, 2026-11-15). The repository is public, and each `setup.md` names its drand rounds and edicts. The setups stay public, because their push time is the proof they weren't tuned to their rolls.

Every seat plays without outside knowledge: no web access and no repository access. AI Studio grounding is off. Gemini-app seats run in a temporary chat with connectors and apps off for that chat; any reply that shows sources is a harness fault. Claude seats are told to use no web tools, and their transcripts are checked for WebSearch or WebFetch calls after each game. GPT-OSS seats run in a clean project with file tools only; their control is the relay's path check, which lists every path named in the seat's log outside its own folder. The check reads the seat's reasoning text as well as its tool calls, so a flag is not a verdict: it sends the referee to the seat's log to see whether a file outside the folder was actually read. Each seat's surface and tool settings are written in its setup.md seat row. After each game the referee checks every reply for knowledge of a road event or an edict before its reveal, by any channel; any case is a harness fault in metrics.md.

**Games 1 and 2, checked after the fact** (R&D, from the logs, 2026-11-15):
- No seat used a web tool.
- GPT-OSS seats used only read_file and write_file.
- AI Studio grounding was off.
- Gemini-app chats showed no sources.
- The G2 path-check flags were reasoning text, not reads: a placeholder path and a stray parenthesis.

## The seat brief

Every seat gets the same brief, version-pinned, plus its own charter and mandates. It contains:

1. **The rules,** summarised from `design.md`:
   - the towns and roads;
   - the six guilds and their converters (including that some sit out);
   - a market day and sealed travel orders;
   - offers and the town ladders;
   - contracts, bonds and the bond rule's outcomes;
   - standing and what it gates;
   - the score (works and mandates only);
   - edicts;
   - the bell between days 10 and 13.
2. **Its own guild:** charter, converter, home, starting purse and goods.
3. **Its two sealed mandates.**
4. **Its goal:** "Finish with the highest score. Score comes only from goods delivered to town works and from your revealed mandates."
5. **How to reply:** the turn form below.
6. **What talk is:** private messages to named seats, carried by the referee. A seat may bargain, promise, bluff or break promises, within the rules.

**What the brief never contains:**
- the named lazy strategies, or any hint of them;
- the metrics;
- that the test is looking for shortcuts;
- which model family any other seat is.

It describes a game to win, nothing more.

## The turn protocol

Each market day has two rounds, both carried by R&D between the referee and each seat.

**1. Talk.**
- The referee sends each seat the public state and its inbox:
  - where every caravan is (from yesterday's reveal);
  - the town ladders and the open offers;
  - the works' progress;
  - open contracts and bonds, and standings;
  - today's edict;
  - its own purse, goods and mandates.
- The seat replies with up to 3 messages, to named seats, each of at most 80 words.
- The referee delivers them in the next round.

**2. Act.** Each seat replies once, privately to the referee, in this form:
```
OFFERS:    post <give> for <get> | cancel <offer id>
CONTRACTS: propose <terms> to <seat> | accept <contract id> | bond <contract id>
PAYMENTS:  pay <amount> <good or coin> to <seat>
CONVERT:   <recipe> x<n>
WORKS:     deliver <n> <good> to <works>
TRAVEL:    <stay | go to TOWN>   (sealed: only the referee sees it until resolution)
```

**The referee resolves the day in the design's order.** At each step it writes the rule it applied.
1. It records the SHA-256 of each sealed travel order, then reveals the orders.
2. It applies the day's drand round: the pre-fetched quicknet round for that day, by number.
3. It applies road events, then moves the caravans.
4. It fills offers by price, then in the day's random order (from the roll), so no seat gains by answering first.
5. It applies conversions and works deliveries.
6. Perishables spoil.
7. It runs the bond rule at contract deadlines.
8. It refills the ladders.

**A missed or invalid reply counts as a missed reveal.** The caravan stays and pays the toll. The referee doesn't ask again, with one exception.

**The exception: a delivery failure.** Amended by the Publisher for games 2 to 6, after game 1. It's needed because silence a seat chose and silence the pipe caused aren't the same thing. The test is of the rules, not the transport.
- **What counts as one.** Any of these:
  - the seat's surface shows it finished (a DONE line, for example), but no reply file was written;
  - the surface shows a platform error;
  - the seat gives no reply for 15 minutes after the prompt was delivered.
- **What happens.** The seat gets **one** re-send of the same instruction: neutral, with no new content, no hint and no comment on the delay. The log marks it as harness.
- **What doesn't count.** Anything after that one re-send, or a reply in the wrong form, is a missed reveal under the board's rule 2, as before.
- **How it's counted.** In `metrics.md`, harness-caused misses are counted apart from missed reveals in metric 6.
- **Game 1** keeps its two day-9 re-prompts as missed reveals, marked harness-caused in its `metrics.md`.

The re-send is a procedure for whoever relays the seats, so the pinned `board.md` and the seat brief are unchanged.

## What it measures

The review's seven metrics, with their hypothesised bands, plus one more:

| # | Metric | Healthy | Problem | On paper |
|---|---|---|---|---|
| 1 | Guild-to-guild share of trades | 35–65% | under 20% | counted from fills |
| 2 | Whisper messages per player per day | 4 or more | under 1.5 | counted, at most 3 a round by the form |
| 3 | Contracts per player, and bond spread | 1.5 or more; bonds from 0% to 100%+ | under 0.5 | counted |
| 4 | Willful breaches vs force-majeure delays | 1–2 breaches, nobody quits | a seat quits over a delay | from the bond rule's outcomes |
| 5 | Order timing in the window | spread | over 60% in the last 5% | **not measurable**: replies are simultaneous on paper. Recorded as n/a. |
| 6 | Missed reveals | under 3% | over 8% | missed or invalid replies; harness-caused misses counted apart |
| 7 | Contenders at day 10 | 3 or more within 15% of the leader | winner clear by day 7–8 | from the day-10 scores |
| 8 | **A named lazy strategy found unprompted** | none wins | one wins a table | see below |

**Metric 8.** After each game, the referee tags each seat's play against the design's six named lazy strategies:
- wash cartel;
- hermit;
- stay at home;
- one-works rush;
- kingmaker;
- last-day breach.

For each tag it quotes the log lines that show it, and records whether it paid. "Paid" means it won, or scored above the table's median. Any other strategy that wins without building works or trading is logged as a new shortcut.

**Also recorded:**
- each mandate's real difficulty against its grade;
- every situation the rules didn't decide (a rule gap);
- the referee's word count per day, as a cost measure.

## What's logged, and where

Each game gets a folder, `docs/paper-test/game-NN/`, committed when the game ends:

- **`setup.md`:** the setup, with:
  - the seated guilds and who sits out;
  - the towns, ladders and works;
  - the mandates dealt (sealed in the setup, shown here after the game);
  - the edicts;
  - the drand round numbers and values;
  - the bell-day commitment and its reveal;
  - each seat's family, surface and model;
  - the brief's version and hash.
- **`log.md`:** per day:
  - the public state;
  - each seat's raw replies, verbatim;
  - the whispers;
  - the referee's resolution, step by step, with the rule cited at each step;
  - for every contract at its deadline: Delivered, Paid, Held and the outcome, as the bond rule defines them.
- **`score.md`:** final scores, works points, mandate points, standings and the bell day.
- **`metrics.md`:** the eight metrics, the lazy-strategy tags with their quoted evidence, rule gaps, and mandate difficulty.

**Replayable.** Anyone holding `setup.md` and `log.md` can rerun the referee's steps and get the same result. That's the paper version of the design's replayable steward.

**Before every push:** an identity scan of `git archive HEAD`, and a check that no file holds anything but game text.

## The stop rule

- **If the same shortcut wins two tables, stop.** That's a named lazy strategy, or a new one. The rules are fixed on paper first: a change to `design.md`, approved by the Publisher. Then the unplayed setups are rebuilt, and the count of games starts again.
- **If a rule gap recurs, stop and fix the rule.** The referee decides each gap provisionally and logs it. A gap that comes up in two games is treated as a failure of the rules.

The Publisher is told each time the rule fires, with the evidence.

## At the end

A summary for the Publisher and the Director, in this order:
1. Which bands the tables landed in.
2. Which shortcuts appeared, and whether they paid.
3. Which rules failed to decide.
4. What should change in `design.md` before the Director's approval.

The design goes back to him with this summary.

## Cost

Six games of about 11 days with about 4.5 seats come to roughly **300 seat-turns**, each with two rounds, plus the referee's resolution of about 66 days.
- About half the seat-turns are Gemini seats, driven in a browser by R&D. That's the heaviest part.
- **Suggested order:** play games 1 and 2, then review cost and early results with the Publisher before games 3–6.

## Compensators

| Irreversible act | Undo | State after | Owner |
|---|---|---|---|
| Committing a game's logs | `git revert` the commit | logs removed from the branch, history kept | Book Keeper |
| Opening the PR with the logs | close the PR | no merge | Book Keeper |
| A Gemini or Antigravity conversation | none needed: it holds only game text, no studio data or secrets | — | R&D |
