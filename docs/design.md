# Testnet Merchant League design

Redesign of 2026-11-14 (ruling 1). It replaces the first draft of the same day after the outside review (research seed 25, harvested 2026-11-14). **It's for the Director to approve. No game code is written until he does.** The ledger layer it relies on is built and merged: patched xrpl-rust with TokenEscrow and DomainID, proved live on the Testnet (merchant-league #5).

Merchant League is a slow-turn trading game for 4 or 5 players, played on the XRPL **Testnet**. Each player runs a guild with its own craft. Guilds trade goods across five towns to build the towns' great works. Every move is a ledger transaction.

## What changed from the draft

**Corrected 2026-11-14 after Testnet checks** (PR #6 comments; receipts in PR #7):

- **Escrow expiry:** a token escrow doesn't need `CancelAfter`. Any escrow needs `FinishAfter` or `CancelAfter`, and a condition when it has no `FinishAfter`. Bonds keep `CancelAfter` by choice.
- **Rolls:** each day's drand round is named by its **number**, computed from the close time.
- **A new constraint:** clawback can't reach tokens in escrow. So perishables are never escrowed, and the steward reads a balance before any clawback.

**Answered by the Director, 2026-11-14** (four of the five open questions):

- **Delivery:** two payments with bonds, no Batch.
- **The referee:** a trusted steward whose every decision can be replayed.
- **Guilds:** six. One or two sit out each table.
- **Gifts:** left to the paper test.

**Then the fifth (2026-11-14):** the Director doesn't play the paper test. The AI crew plays it, over several games; the plan is `docs/paper-test.md`. The design goes back to him with its results.

The steward's bond rule is now written out in full ("The steward's bond rule").

| Draft said | Now | Why |
|---|---|---|
| Every merchant has the same caravan and the same coin | **Asymmetric guilds.** Each has a converter that needs other guilds' goods. | Identical merchants have nothing to trade for. At 3 players a merchant is alone in a town about two turns in three (the review's own occupancy model). |
| "The richest reputation wins"; no formula | **A written score:** points come only from town works and sealed mandates. Standing gates bonds and tolls but never scores. | An unwritten score was solvable two ways: a wash-contract cartel or a promise-nothing hermit. |
| Contracts are Checks the seller cashes on delivery | **Escrow bonds.** Each side locks a bond in a token escrow. The steward releases a bond only on a breach, by a public rule. | A Check sets no funds aside, and the payee can cash it at any time. Token escrow locks the bond, proved live on the Testnet. |
| One order book for the season, shared by all tables | **Separate markets per table:** per-table goods on authorized trust lines, and a permissioned domain as the table's market | One issuer's tokens trade network-wide, so table A's offers crossed table B's. Live proof: a domain refuses outsiders and keeps open-book offers from crossing its book. |
| Three barriers a day: commit, move, reveal | **One transaction a day.** It reveals yesterday's sealed travel order and commits today's. Roads roll straight from a drand round whose number is set by the close time. | Each barrier multiplies the chance that somebody misses: 77–97% table-wide at a 2–5% daily miss, by the review's arithmetic. The dice escrows added risk with no gain in fairness. |
| A fixed 12-day table | **A random closing bell** between days 10 and 13, and edicts in the middle of the table | A known last day invites last-day betrayal and endgame arithmetic. |
| A season runs from one Testnet reset to the next | **Six-week studio chapters, nested in Testnet eras.** A reset is a rare era finale with a 24-hour jubilee. | The published resets were 55.8, 4.0 and 27.2 months apart, so the clock has to be the studio's own. |
| "Our own node, eventually"; no server decides | **Two named studio services from table 1:** the steward (the issuer service) and the archivist (the archive node and indexer) | Public Testnet nodes prune history, so a week-long table can't be replayed from them. The issuer must be live to mint, run town markets and spoil goods. |
| Marks rule: +25% a week cuts rewards 20% | **Fixed rewards by placement.** Prestige is sold at auction and the proceeds burned. | Every chapter starts near zero, so the growth rule would trip on every chapter's normal start. |
| A flood and a thief both "break a promise" | **Force majeure vs willful breach.** A delay the road roll caused costs no standing. | The road roll is replayable, so the game can tell the two apart. |
| 2 to 6 players | **4 or 5 players** | A 2-player table has no market. At 3 players, a merchant is alone in a town most turns. |
| Hand-written events and table setups | **Generated table setups with proofs** (role-set's generator method) | A setup is accepted only when named lazy strategies lose on it. Variety floors keep a chapter's tables from repeating. |
| Silent on private talk, absent players, bots and trust-line flags | Answered in "What we hadn't asked" | The review raised them. |

## Decisions

| Decision | Chosen | Why |
|---|---|---|
| What the game is | Merchant League: guilds, five towns, goods, town works and bonded contracts | The XRPL already has a trading engine (offers, order books, escrows), so a trading game uses the ledger for what it is good at |
| How it is played | A Rust client: a terminal view (ratatui), and a small daemon that keeps standing orders and sends alerts | A transaction settles in 3 to 5 seconds, so slow turns suit a public ledger. The daemon covers a player who is away (see "What we hadn't asked"). |
| Ledger library | xrpl-rust 1.3.0, vendored with upstream's security fix, plus TokenEscrow and DomainID (#5) | Ruled 2026-11-14. `cargo audit` is clean with no ignores. |
| Who runs the issuer | The steward, a studio service, one issuer account per chapter | It must be live to mint, run town markets, spoil goods, act as the bond referee and issue table credentials |
| Where history lives | The archivist: a studio archive node and indexer, from the first table | Public nodes prune. Every standing, score and bond decision has to be replayable. |
| Where rolls come from | A drand round named by number in advance (from the close time), read directly | No player and not the studio can steer it, and the ledger needs no dice escrows |
| Where table setups come from | A seeded generator that proves each setup against named lazy strategies | Role-set's method: build, reduce, prove the shortcut loses, dedupe, keep a receipt |

## The game

A table seats **4 or 5 guilds** across **five towns** joined by roads. A table runs **10 to 13 market days**: the closing bell is drawn (see "The closing bell"). The highest **score** wins.

### Guilds

There are **six guilds**. Each guild holds a charter: one **converter** (a craft), a home town and a starting purse of table coin. A converter turns goods the guild doesn't make into a good only it makes, and the converters interlock:

| Guild | Home | Converter | Needs from others |
|---|---|---|---|
| Saltworks | Varn (the salt flats) | 2 brine → 1 salt | brine comes from Ostra's fishers (town market) and **timber** for the pans |
| Weavers | Ostra (the port) | 2 wool + 1 dye → 1 cloth | **dye** from the Dyers and wool from the town market |
| Smiths | Kell (the hills) | 1 ore + 1 charcoal → 1 tools | **charcoal** from the Foresters, and **leather** for bellows (a converter upgrade) |
| Foresters | Hath (the woods) | 1 timber → 1 charcoal, or timber as is | **tools** to cut faster (a converter upgrade) |
| Dyers | Mire (the marsh) | 1 madder + 1 salt → 1 dye | **salt** from the Saltworks |
| Tanners | Hath (the woods, shared with the Foresters) | 1 hide + 1 salt → 1 leather | **salt** from the Saltworks, the same salt the Dyers need; hides from Kell's town market |

**Six guilds, 4 or 5 players.** Six guilds don't change the player range. A table seats 4 or 5 of the six, so one or two guilds sit out each table.

- The generator chooses who sits out, as part of the setup it proves.
- It adjusts the town markets so every seated guild's needs are still met: a sitting-out guild's good appears on a town ladder, at a worse price than a guild would charge.
- Across a chapter, each guild seats in at least two of every three tables (see "Generated tables").
- A six-player table, with no guild sitting out, isn't in v0.1.

Converting takes one market day, in the guild's home town or at a works site the charter names.

### A market day

Each guild sends **one transaction a day**. It carries:
1. **The reveal** of yesterday's sealed travel order: the order and its salt, matching the commitment in yesterday's memo.
2. **Today's sealed travel order**, as a commitment: SHA-256 of the order and a fresh 32-byte salt, in the memo.
3. Optionally, the day's conversions, deliveries to works, and contract steps, as further transactions.

Trading happens all day as **ordinary offers** in the table's market: guild to guild, and with the town makers. Offers are public by nature; only travel is sealed. A merchant can't see where the others are going before committing.

**At the day's close** (a fixed ledger-close time named at the table's start):
1. The day's **drand round** is the quicknet round whose number the close time sets (see "Fair dice"). Its value is the day's roll.
2. **Road events** take their numbers from the roll by a fixed index per road: bandits, tolls, floods, fairs. Each road's odds come from a table in `tables/` (see "Tables with roll").
3. Caravans move by the revealed orders. A missing reveal means the caravan stays in town and pays the toll, and the day goes ahead (see "Fair dice").
4. **Perishables spoil.** The steward claws back goods past their days. It reads each balance first, because a clawback for more than a line holds takes what is there and still reports success.
5. **Contracts check their deadlines** (see "Contracts").

A day takes about two minutes when everyone is online. Given a set ledger window (about a day of real time), a table can run across a week.

### The score (written, and public before the table starts)

**Score = Works points + Mandate points.** Nothing else scores: not coin, not goods, not standing.

- **Town works.** Each town has one or two great works, each needing a mix of goods: for example, the Varn sea wall needs 4 tools, 6 timber and 2 cloth. A delivery to a works is a Payment of goods to the steward, which destroys them, and it is recorded by work and guild.
  - A delivered good scores its **works value**: 1 for a raw good, 2 for a converted good, 3 for a good converted twice (dye, tools).
  - The guild that completes a works also scores the completion bonus of 4.
  - A completed works stops taking goods.
- **Sealed mandates.** Each guild draws two secret mandates at the start and commits to them as a hash in its first transaction, revealing them at the bell. A mandate scores 3 to 8 points, set by the generator's graded difficulty. Examples: "Be the largest contributor to the Kell bridge"; "Deliver 5 cloth to works outside Ostra". An unrevealed mandate scores 0.
- **Ties:** the higher standing wins, then the fewer willful breaches, then the earlier last delivery.

**Standing** is not score. It starts at 0 and gates three things:
- **bond size:** standing ≥ 3 posts half bonds;
- **tolls:** standing ≥ 5 pays half tolls;
- **vendetta:** a guild wronged by a willful breach may refuse that guild's caravan its town market for 2 days.

Standing moves like this:
- **+1** per contract delivered on time that bore road risk: the seller's caravan travelled at least one road between agreeing and delivering.
- At most +1 per counterparty per 3 days.
- **−3** per willful breach.
- **0** for a force-majeure delay. The day's roll put a flood or bandits on the seller's road, provably from the replay, and the seller delivers within 2 days.

**The anti-wash rule.** Trading between guilds scores nothing. Only goods destroyed in a works, and revealed mandates, score.
- A wash cartel can pass coin and goods back and forth as often as it likes. Goods only come from converters and town makers, so passing them creates nothing.
- Wash contracts earn standing only with road risk, and at most once per counterparty per 3 days.
- Standing never scores; it only makes bonds and tolls cheaper.

The generator proves each setup against this cartel before the table is offered (see "Generated tables").

### Contracts (escrow bonds)

A contract is a promise across days: "Deliver 3 salt at Varn by day 6 for 20 coin." It runs on token escrows, which TokenEscrow supports. Both the escrow and the steward's referee secret are already proved on the Testnet.

1. **Agree.** The two guilds agree off-ledger, in the table's whisper channel. Either side then sends the terms in a memo. The steward's indexer records them.
2. **Bond.** Each side creates a **token escrow** of its bond to the other side.
   - The bond is the contract value, or half that at standing ≥ 3. Bonds are always table coin, never perishable goods (see "Spoiling and escrow" below).
   - The escrow carries a **condition**: the SHA-256 of a referee secret the steward holds for that bond.
   - The escrow's `CancelAfter` falls **four days after the deadline**, by choice: two days for a force-majeure extension, then two days for the wronged side to claim. After that, each side can take its own bond back.
   - The ledger requires every escrow, XRP or token, to have `FinishAfter` or `CancelAfter`, and a condition when it has no `FinishAfter` (rippled's `EscrowCreate` rules, checked live). A bond has both a condition and `CancelAfter`.
3. **Deliver.** Two plain payments by the deadline: the seller pays the goods, and the buyer pays the price (the Director chose this over Batch). The seller's caravan must be in the named town on the day it pays.
4. **Settle.** The steward applies the bond rule below at the deadline:
   - **Kept:** it releases nothing, and each side cancels its own bond after `CancelAfter`;
   - **Breach:** it publishes the breaching side's referee secret, and the wronged side finishes that bond to itself;
   - **Force majeure:** the deadline moves 2 days, once.

Every steward decision is a pure function of the replay and the public rules. The decision log carries each input's transaction hash, so anyone can recompute it.

**Why a referee and not a self-settling escrow.** An escrow can be finished only by its condition or by time, and the ledger can't see "the caravan was in Varn". So a third party must decide, and the steward is already trusted: it issues every token and can claw back. Escrow adds what Checks lacked, a **locked** bond. The Director accepted this referee on 2026-11-14, and chose two payments over Batch for v0.1.

### The steward's bond rule

The rule is a pure function. The same inputs always give the same outcome, and anyone holding the archivist's export can recompute it.

**Inputs.** All of them come from the table's replay, up to the first validated ledger that closes after the evaluation day:

1. **The terms:** the memo both guilds agreed. It names the contract id (that memo's transaction hash), the seller, the buyer, the good, the quantity, the town, the price, the deadline day, and both bond escrows (owner and sequence).
2. **Where each caravan was, each day:** from the revealed travel orders and that day's road events.
3. **The road events:** from each day's drand round (named by number) and the road tables in `tables/`. They are fixed by the round, so they're the same for everyone.
4. **The payments:** the seller's payments of that good to the buyer, and the buyer's payments of table coin to the seller, each with the day of the ledger that holds it.
5. **The rules version:** the tables' hash, so a later rule change can't reach back.

**Definitions.**

- **Delivered:** the seller's payments of the good to the buyer add up to at least the quantity, counting only payments made on days when the seller's caravan was in the named town, by the deadline day. A partial delivery isn't a delivery.
- **Paid:** the buyer's coin payments to the seller add up to at least the price by the deadline day. Either side may pay first; paying first is that side's own risk.
- **Held:** on some day between the agreement and the deadline, a road event marked *holds* in the tables stopped the seller's caravan on a road it was travelling.

**The rule, at the close of the deadline day:**

| Delivered | Paid | Held | Outcome |
|---|---|---|---|
| yes | yes | (any) | **Kept.** No secret is published; each side cancels its own bond after `CancelAfter`; the seller gains standing if the contract bore road risk (see "The score"). |
| yes | no | (any) | **Buyer's breach.** The steward publishes the buyer's bond secret; the seller finishes the buyer's bond. Buyer's standing −3. |
| no | (any) | no | **Seller's breach.** The steward publishes the seller's bond secret; the buyer finishes the seller's bond. Seller's standing −3. A buyer who didn't pay a seller who didn't deliver is not in breach. |
| no | (any) | yes, first time | **Force majeure.** The deadline moves 2 days, once, and the rule runs again at the new deadline. On that second run *Held* doesn't count. The delay costs no standing. |

**What the steward writes.** One decision-log entry per contract:

- the contract id and the rules version;
- the evaluation ledger index;
- every input transaction's hash;
- the three values (Delivered, Paid, Held) and the outcome;
- on a breach, the referee secret.

The secret is also sent as a memo, so it's on the ledger before the wronged side finishes the bond.

**What keeps the steward honest, and what doesn't.**

- **Fixed at bond time:** each bond's condition is the SHA-256 of a fresh 32-byte secret. It's fixed when the bond is created, so the steward can't swap the secret later.
- **A bad decision shows:** a wrong decision is visible to anyone who recomputes it. It's reversed by rule: the steward compensates the wronged side in table coin, and the log records both entries.
- **A leak can't be prevented:** the rule can't stop the steward leaking a secret early. A leaked secret shows up when a bond is finished with no breach in the log, and that is a studio fault, to be made good the same way.
- **An outage can cost a claim, and is made good the same way.** A bond's `CancelAfter` is fixed on the ledger when the bond is created, but a steward outage moves deadlines with the close. If the steward is down past a bond's `CancelAfter`, the breaching side can cancel its own bond and walk away, and the wronged side can no longer finish it. Such a bond is made good by the same rule as a wrong decision: the steward compensates the wronged side in table coin, and the log records both entries.
- **Behind, no new bonds:** to keep that rare, the steward refuses new bonds once it is a day or more behind the table's close.

### The closing bell

From the close of day 10, each day's roll also decides whether the bell rings: 1 in 3 at the close of day 10, 1 in 2 at day 11, 2 in 3 at day 12, and always at day 13. The odds are a `tables/` table, so they're on record. At the bell, mandates are revealed and the table is scored.

**Edicts:** at days 4 and 8, the generator's pre-drawn edict for the table takes effect, for example "Kell closes its gate for 2 days" or "a salt levy at Varn". It's announced one day ahead. Edicts are part of the setup and are proved with it.

### Town makers (the towns' markets)

Each town's market is the steward's **price ladder**: offers at stepped prices, posted in the table's domain. The ladder buys and sells a few goods at a spread wide enough that trading guild to guild pays better. Ladders refill each day from the setup's schedule. They are the table's sources (raw goods) and part of its sinks (the spread).

**Not AMM pools.** xrpl.org says permissioned offers and payments can't be filled by AMMs, so a pool couldn't serve a domain's book. Ladders are plain domain offers, which the live proof covered.

### Spoiling and escrow

**Clawback can't reach tokens locked in an escrow** (Testnet check, 2026-11-14). With 20 tokens escrowed, a clawback took only what was on the line, and the escrow then finished.

- **Perishable goods are never locked in an escrow.** They would escape spoiling until it settled.
- **The steward reads a line's balance before any clawback,** and records what it actually took.
- **A bond in escrow settles only by its escrow rules.** The steward's reach is the referee secret, not clawback.

## Separate markets per table

Each table gets:
- **Its own goods and coin:** tokens with table-specific currency codes from the chapter's issuer. Trust lines need the issuer's authorization (RequireAuth), and only the table's seated guilds are authorized. Nobody else can hold the table's goods, so nothing can cross to another table.
- **A permissioned domain** owned by the steward, accepting the steward's credential for that table. The town ladders post there. The live proof showed:
  - a domain refuses an uncredentialed account's offer (`tecNO_PERMISSION`);
  - an open-book offer at a crossing price doesn't cross a domain offer;
  - two members' domain offers do cross.
- **What the domain doesn't do:** a credentialed account can still trade on the open book. That's why the per-table tokens do the isolating, and the domain is the table's official market.

**No XRP pairs.** Test XRP is free, so it can't be a currency. The steward never posts an offer pairing a table token with XRP. The archivist flags any such offer, because auto-bridging through XRP would let a trade skip table coin.

## Fair dice

A day's roll is a **drand round named by its number**. The number comes from the day's close time on drand's quicknet chain, which publishes a round every 3 seconds:

- **round = floor((close − 1692803367) / 3) + 1**, where 1692803367 is quicknet's genesis, in Unix seconds.
- The rules give the formula, so a table's every round number is known before it starts.
- Nobody, including the studio, knows a round's value before it's published.

**Nobody can choose the value.**

- Each quicknet round is a BLS signature over its round number. BLS signatures are deterministic, so the value is fixed.
- A party holding drand's threshold could see a round first, or withhold it. That only delays the close; it can't change the value.
- Checked 2026-11-14: the period, genesis and formula came from drand's API, the formula matched a live round, and the randomness was the SHA-256 of its signature. A BLS signature has not yet been verified locally.

- **Road events, the bell and edicts** take their numbers from the roll by a fixed index, through the table engine (`league-dice` → roll 3.0's Rust core; see "Tables with roll").
- **Sealed travel orders** are the one thing hidden.
  - Each order is committed in one day's transaction as SHA-256 of the order plus a fresh salt, and revealed in the next day's.
  - Anyone can check the reveal against the memo. No escrow is needed, so each day needs one transaction, not three.
- **A guild that doesn't reveal** stays in town and pays the toll, and the day goes ahead. There is never a re-roll: a re-roll would give the last revealer a second chance (EIP-4399).
- **If drand is unreachable,** the close waits for it. It never falls back to the ledger hash or a studio number, since those could be steered.
- **#2's crypto-condition encoding** (PREIMAGE-SHA-256 conditions and fulfillments) isn't needed for dice any more. It is needed for the **bonds' referee secrets**, so #2 stays and is folded into the contract work.

## Generated tables (role-set's generator method)

A table's setup is built by code, and each setup is proved before it is offered. A setup is:
- the guilds and their charters;
- the five towns, with their ladders and schedules;
- the works and their needs;
- the mandate pool;
- the edicts;
- the road odds.

This takes **the method of role-set's puzzle generator, not its code**: a seeded build, a reduction, a proof that each named shortcut loses, a difficulty grade, a skeleton dedupe and a receipt. If this design is approved, the generator would be a Rust crate in this repository, after the rules.

1. **Sample** a setup from the chapter's content packs (goods, towns, works, edicts), seeded.
2. **Simulate.** A simple strategy simulator plays the setup many times, with rolls drawn from the same tables.
   - First, a **competent baseline**: guilds that convert, trade at fair prices, keep contracts and build works.
   - Then each **named lazy strategy**, like role-set's lazy readings:
     - *wash cartel:* two guilds contract with each other in a safe town;
     - *hermit:* trades only with the town ladders, makes no promises;
     - *stay at home:* never leaves the home town;
     - *one-works rush:* pours everything into the first works;
     - *kingmaker:* a trailing guild gifts goods to the leader;
     - *last-day breach:* breaks every contract due after day 9.
3. **Prove.** Accept the setup only if every lazy strategy scores clearly below the baseline: below the baseline's 25th percentile, a config value. The kingmaker is the exception: there, the leader's gain from the gifts must be smaller than a fair trade would give. A setup where a shortcut wins is refused, like a puzzle whose lazy reading gives the key.
4. **Grade** each mandate by the fewest days and transactions the simulator needs to complete it. Its points follow its grade, and the receipt keeps the grade. Mandates outside the table's difficulty band are refused.
5. **Dedupe.** A setup's skeleton is its converter graph, its works' needs, its edict kinds and its ladder spreads. No two tables in a chapter share a skeleton. Floors per chapter: each of the six guilds seats in at least two of every three tables, and no edict kind appears in more than half the tables.
6. **Receipt.** The seed, generator version, content-pack hashes, each strategy's score distribution, each mandate's grade and the skeleton hash. It's published with the table, so anyone can rebuild the setup.

The paper test's six setups are built by hand to this recipe, before the generator exists. Its games then check the recipe against AI players who haven't been told the shortcuts (see "The paper playtest").

## Seasons: chapters inside eras

- **A chapter** is about **six weeks**: studio-owned, announced, with its own issuer and one new rule (a good, a guild, an edict kind).
- **An era** runs from one Testnet reset to the next, which can be months or years apart. An era holds many chapters.

| Survives a chapter's end | Ends with the chapter |
|---|---|
| Titles, the hall of fame, cosmetics | Marks, table coin, goods, open escrows |
| A signed chapter record per player: tables, scores, standings, contracts kept and broken, with transaction hashes | The chapter's issuer and table accounts |
| A capped starting standing for the next chapter | |

**Marks** are earned at fixed rates by placement in finished tables: 5, 3, 2 and 1 for first to fourth, and the fifth place earns 1. Prestige items (titles, guild colours, a charter variant) are sold at the chapter's auction, and the marks are burned. That's the sink.

**An era finale** (an announced reset) closes the current chapter early with a **24-hour jubilee**:
- trading continues;
- no new contract may have a deadline after the reset;
- the last day's standings are snapshotted, and the hall of fame is signed.

An unannounced reset is caught by the "ledger went backwards" check (1,000-ledger tolerance) in `league-season`. That check closes the chapter at the last state the archivist saw. Carry-over depends on what a player did during the chapter, never on being online at the wipe.

**Proofs outlive the ledger.** A record's hashes can't be checked once the ledger is gone. The archivist keeps the era's full history and exports it at the finale, so a record can be checked against the export. Without it, verification reports "can no longer be checked", never a pass.

## The studio services

| Service | Does | If it's down |
|---|---|---|
| **Steward** (issuer service) | Holds the chapter issuer's key; issues table goods and coin on authorized trust lines; posts town ladders; spoils perishables (clawback, after reading the balance); issues table credentials and the domain; referees bonds; signs records | The day's close waits; deadlines move with it. Nothing settles without it. |
| **Archivist** (archive node and indexer) | A studio Testnet node with full history for the era, plus an index of every table event; serves replays, receipts and the decision log; flags rule breaches (XRP pairs, missing reveals) | Clients keep playing from public nodes; replays and settlements wait for it |
| **Relay** (whisper channel) | Carries private messages between a table's guilds, end-to-end encrypted, never on the ledger; counts messages per player per day for the press metric, never their content | Guilds can still trade; talk waits |

The steward is trusted: it can mint and claw back. The fence is publicity. Every issue, burn, clawback and referee decision is on the ledger or in the decision log, with the rule that produced it.

## What we hadn't asked (answers)

- **Private talk.** The Relay, off-ledger, encrypted end to end. On-ledger memos would leak who talks to whom. Agreements stay informal until they're bonded on the ledger.
- **Absent players.** The player's daemon keeps **standing orders**: stay in town, keep offers, reveal the committed order. A guild absent for 2 days "rests": its caravan holds, and its bonds settle by the rules. Delegated signing (XRPL permission delegation) is not used until its Mainnet status is checked.
- **Bots.** Ranked tables are for invited humans. Bots play in a separate open league with its own hall of fame. A table's setup and the generator's simulator are public, so bot authors have a fair start.
- **Trust-line flags**, set in this order by the steward before any trust line exists:
  1. RequireAuth;
  2. Allow Trust Line Clawback (it must come before any line);
  3. Allow Trust Line Locking (or a token escrow is refused, live proof);
  4. DefaultRipple on the issuer, with NoRipple on guild lines.

  The steward's setup checks each flag on the ledger before seating a table.
- **Pruned history.** Answered by the archivist, from table 1.
- **Kingmaking.** The simulator's kingmaker strategy is in every setup's proof. A gift of goods with no consideration back is flagged in the decision log. Whether to forbid gifts outright is left to the paper test.

## The paper playtest

**The full plan is `docs/paper-test.md`.** In short:

- **Who plays:** the AI crew, not people (Director, 2026-11-14). That's six games of 4 or 5 seats, from different model families: Gemini (AI Studio, the Gemini app, one Antigravity card), Claude subagents, and optionally a local model.
- **The referee:** a Claude session that doesn't play. It runs the steward's written procedure by hand, and writes a log anyone can replay.
- **The setups:** six, built by hand to the generator's recipe. The generator isn't built yet, so the games test what its proof would.
- **The stop rule:** if a shortcut wins two tables, stop and fix the rules on paper first.

**What it measures.** These are the review's seven metrics with its hypothesised bands, which are design guesses, not data:

| Metric | Healthy | Problem |
|---|---|---|
| Guild-to-guild share of trades | 35–65% | under 20%: solitaire against the towns |
| Whisper messages per player per day | 4 or more | under 1.5 |
| Contracts per player, and bond spread | 1.5 or more, bonds from 0% to 100%+ of value | under 0.5 |
| Willful breaches vs force-majeure delays | 1–2 calculated breaches; no quitting over road delays | anyone quits over a delay |
| Order timing in the window | spread across it | over 60% in the last 5% (sniping) |
| Missed reveals | under 3% | over 8% |
| Contenders at day 10 | 3 or more within 15% of the leader | winner clear by day 7–8 |

**Also recorded:**

- which named lazy strategy a seat found unprompted, and whether it paid;
- each mandate's real difficulty against its grade;
- every rule gap.

On paper, order timing can't be measured, because replies are simultaneous.

**After it.** Rules change on paper first. The design goes back to the Director with the paper test's summary. A Testnet build starts only after his approval, once the paper tables land in the healthy bands. Then come the review's four Testnet tables: two 60-minute live blitzes and two 5-day async tables.

## Hard constraints

These conditions are fixed. Each is enforced by the code or by CI, not by good intentions.

| Condition | How it is enforced |
|---|---|
| Testnet only, no mainnet code paths | The client knows only Testnet endpoints. A client can't exist without a checked network id: it holds a token only the network check makes. Each constructor has a test that every other id, and a missing one, is refused after one request. A CI tripwire also fails on any Mainnet or Devnet hostname written in the repository. |
| No real-money language | Game words only: marks, goods, purse, caravan, guild. Ordinary trading words are game language (Director, 2026-11-14). A copy tripwire in CI (`scripts/check-copy.sh`) fails on wording that treats marks as money, and review catches the rest. |
| Faucet calls rate-limited | One faucet call per new player per chapter. The seed stays on the player's machine; a 429 waits and retries with backoff. 100 test XRP covers the reserves for about 490 owned objects. |
| Wipes designed in | Chapters end on the studio's clock; eras end at resets; records live off the ledger and are exported by the archivist. |
| One shared reset module | The announced-reset list and the "ledger went backwards" check come from the module shared with three other Testnet tools. A new reset is added in all four places. |
| Advisories fixed at the root | `cargo audit --deny warnings` with no ignores (the vendored xrpl-rust carries upstream's fix). |

## Tables with roll

Road events, the bell's odds, edicts and the ladders' schedules are tables for roll, the studio's dice and table engine, kept in `tables/`. Each event's exact odds are on record, and a table change shows its new odds in the pull request.

- **Play time, in Rust.** Draws use **roll 3.0's Rust core**, owned by R&D:
  - a counter-addressed integer generator, keyed per roll;
  - integer weights;
  - validation when a table loads;
  - true sampling without replacement;
  - a receipt for every roll.

  The value for each draw comes from the day's drand round, never from a local random number generator. A disputed draw replays exactly.
- **The league's needs go to R&D as tickets**, never straight to roll:
  - a draw keyed by (table, day, road) from one drand value;
  - a weighted table whose weights a context value changes (an edict);
  - draws without replacement for mandate decks;
  - the per-roll receipt as JSON that the archivist stores.
- **Design time, in CI:** roll validates every table and computes its exact odds. This pins to roll 3.0's core once it's released, and to 2.1.1 through its library until then.

## How it runs on Testnet

The game uses only XRPL features enabled on Mainnet as well as Testnet. After the 25 Nov reset, Testnet's amendments match Mainnet's, so anything Testnet-only could vanish.

| Feature | Used for | Proved on Testnet | Mainnet status (xrpl.org known amendments, read 2026-11-14 by R&D) |
|---|---|---|---|
| Issued tokens, authorized trust lines | Goods, coin, marks | Tokens and trust lines: yes (M2, M2c). RequireAuth with clawback and token escrow: yes (design check); an unauthorized line can't receive. | Yes |
| Offers and order books | Trading | Yes | Yes |
| Permissioned Domains, Credentials, Permissioned DEX | The table's market | Yes (M2c): domain set, credentials, member offers cross, outsider refused, open book kept apart | Enabled 2026-02-04 and 2026-02-18 |
| Token escrow with a condition | Bonds | Yes (M2c): refused until the lock flag; created and finished. The expiry rule and clawback's limit are checked (design checks). | Enabled 2026-02-12 |
| Clawback | Spoiling, the referee's reach | Yes (M2) | Enabled 2024-02-08 |
| Memos | Sealed orders, contract terms | Yes | Yes |
| Checks | **No longer used** | | |
| Batch | Not used (the Director chose two payments with bonds, 2026-11-14) | | |

**Measured on the live Testnet, 2026-11-14:**
- A ledger closes about every 3.3 seconds (rippled 3.4.1, network id 1).
- The base reserve is 1 XRP, plus 0.2 XRP per owned object.
- The base fee is 10 drops (0.00001 XRP); the client never sends less than 12.
- A transaction's memos are limited to 1 KB in total.

## Questions for the Director

1. **Batch for delivery.** *Answered 2026-11-14:* two payments with bonds for v0.1, no Batch.
2. **The steward as referee.** *Answered 2026-11-14:* a trusted steward whose every decision can be replayed is acceptable. The rule is in "The steward's bond rule".
3. **Who plays the paper table.** *Answered 2026-11-14:* the AI crew, over several games; he doesn't play. The plan is `docs/paper-test.md`.
4. **Five guilds or six.** *Answered 2026-11-14:* six. The player range stays 4–5, and one or two guilds sit out each table (see "Guilds").
5. **Gifts.** *Answered 2026-11-14:* left to the paper test.

## Check before building

- **Done 2026-11-14:**
  - drand: the period, the formula and the value rule (see "Fair dice");
  - RequireAuth with clawback and token escrow, live;
  - AMMs can't fill domain offers (xrpl.org);
  - the escrow expiry rule, live.
- **Still open:** verifying a quicknet BLS signature locally.
- Permission delegation: Mainnet status, for absent players. (Batch is no longer needed.)

## Risks

| Risk | What happens | Fallback |
|---|---|---|
| The game is not fun | Nobody plays a second table | The paper table first; change rules on paper before any build |
| The towns are too good | Guilds trade only with ladders: solitaire | The generator's hermit strategy must lose; widen spreads; the guild-to-guild share metric |
| A shortcut the simulator missed | One strategy wins every table | The paper test records it; it becomes a named lazy strategy in the generator |
| The steward goes down | Days can't close; a bond's claim window can pass | Deadlines move with the close; the steward's state is on the ledger and in the log. A bond whose `CancelAfter` passes during the outage is made good in table coin by the steward, both entries logged. A steward a day or more behind takes no new bonds. |
| The steward errs | A bond goes the wrong way | Every decision replays from public inputs; a wrong one is reversed by rule and logged |
| drand is unreachable | Closes wait | Waiting is safe; never fall back to a steerable source |
| A reset with no notice | The chapter ends early | "Ledger went backwards" closes it from the archivist's last state |
| A feature changes at a reset | Part of the game stops | Only Mainnet-enabled features; the amendment list is checked after every reset |
| Players treat marks as worth money | Talk of marks as having outside value; trading them off-platform | Marks never leave the game's rules, the copy tripwire runs in CI, and every chapter wipes them |
| Absent players stall a table | Missed reveals | Standing orders in the daemon; a resting guild holds |

## Research grounding

| Finding | Source | Checked | What it changes |
|---|---|---|---|
| Checks set no funds aside, and the payee can cash at any time | xrpl.org, Checks | Read (R&D, 2026-11-14) | Contracts use escrow bonds |
| Token escrow needs the issuer's lock flag; any escrow needs `FinishAfter` or `CancelAfter`, and a condition without `FinishAfter` | Live Testnet (M2c receipt, escrow-expiry check); rippled 3.4.1 `EscrowCreate` | Measured, and read at source (2026-11-14) | The bond steps and the steward's flag order |
| Clawback can't reach escrowed tokens; an unauthorized line can't receive | Live Testnet, RequireAuth design check | Measured (2026-11-14) | Perishables never escrowed; a balance read before clawback; per-table tokens isolate |
| A domain refuses outsiders and keeps the open book apart; it doesn't stop members using the open book | Live Testnet, M2c receipt | Measured | Per-table tokens isolate; the domain is the market |
| Thin books and symmetric players give no trade | Seed 25 review (Gemini Deep Research, 2026-11-14), its occupancy model | The review's own arithmetic | Asymmetric guilds; 4–5 players |
| Converters make trade necessary | Sidereal Confluence, cited by the review | Not checked at source | The converter graph |
| Unwritten scores get solved (wash, hermit) | Seed 25 review | Reasoned in the review | The written score and the anti-wash rule |
| Finite games invite last-round defection | Kreps, Milgrom, Roberts and Wilson, 1982 | Cited by the review | The random bell |
| Noise needs forgiveness, or cooperation collapses | Wu and Axelrod, 1995 | Cited by the review | Force majeure costs no standing |
| A skipped reveal is the attack; never re-roll | EIP-4399, 2021 | Read at source | A missing reveal stays in town |
| One beacon value nobody can choose | drand | Partly read; check before building | Rolls straight from a named round |
| Prove a shortcut gives a different answer | HANS (McCoy, Pavlick and Linzen, 2019), via role-set's generator design | Cited there | A setup is refused when a lazy strategy wins |
| Seeded generators with verifiers rebuild the same item | Reasoning Gym (2025), via role-set's design | Cited there | Seeded setups with receipts |
| Published Testnet resets were 55.8, 4.0 and 27.2 months apart | Seed 25 review, xrpl.org blog dates | Not checked | Studio chapters inside eras |
| A Testnet ledger closes about every 3.3 seconds | Live `server_info` | Measured | Slow turns |

## Sources

- **Seed 25 review:** "Challenging the Testnet Merchant League Hypothesis" (Gemini Deep Research, 2026-11-14), harvested by R&D: `rnd-local/projects/deep-research/2026-11-14-25-merchant-league-HARVEST.md`. The verbatim export is kept beside it.
- **Live receipts:** `docs/receipts/m2-xrpl-rust-spike-2026-11-14/`, `docs/receipts/m2b-own-layer-proof-2026-11-14/`, `docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14/`.
- **Role-set's puzzle generator design** (Publisher, 2026-10-11, with R&D's review) and its code in `role-set/src/role_set/generate/`. Its method is taken here, not its code.
- **XRPL:**
  - [Checks](https://xrpl.org/docs/concepts/payment-types/checks)
  - [Escrow](https://xrpl.org/docs/concepts/payment-types/escrow)
  - [Permissioned DEXes](https://xrpl.org/docs/concepts/tokens/decentralized-exchange/permissioned-dexes)
  - [Auto-bridging](https://xrpl.org/docs/concepts/tokens/decentralized-exchange/autobridging)
  - [Testnet and Devnet reset, 25 Nov 2026](https://xrpl.org/blog/2026/testnet-devnet-reset)
  - [Reserves](https://xrpl.org/docs/concepts/accounts/reserves)
- **Fair randomness:**
  - [EIP-4399](https://eips.ethereum.org/EIPS/eip-4399)
  - [drand](https://drand.love)
  - Boneh, Bonneau, Bünz and Fisch, ["Verifiable Delay Functions"](https://eprint.iacr.org/2018/601) (2018)
- **Cooperation:**
  - Kreps, Milgrom, Roberts and Wilson, "Rational Cooperation in the Finitely Repeated Prisoners' Dilemma" (1982)
  - Wu and Axelrod, ["How to Cope with Noise in the Iterated Prisoner's Dilemma"](https://websites.umich.edu/~axe/research/PD_with_Noise.pdf) (1995)

  Both were cited by the review and not read here.
