# Testnet Merchant League design

Design of 2026-11-14. The game design is under an outside review (research seed 25); the game rules are not built until that review has been ruled on.

Merchant League is a turn-based trading game for 2 to 6 players, played entirely on the XRPL Testnet, where every move is a ledger transaction and every Testnet reset ends a season.

## Decisions

Made on 2026-11-14.

| Decision | Chosen | Why |
| --- | --- | --- |
| What the game is | Merchant League: caravans, towns, goods and contracts between players | The XRPL already has a trading engine (offers, an order book, escrows, checks), so a trading game uses the ledger for what it is good at |
| How it is played | A Rust client; which kind (terminal or other) is still being confirmed | A transaction takes 3 to 5 seconds to settle, so real-time play is out; slow turns suit a public ledger |
| Who runs the season issuer | One studio-run issuer account per season | One issuer keeps the game currency's supply readable and auditable |
| Where the fairness comes from | Hash-locked escrows for commit and reveal, mixed with the public drand beacon | No single player, and not the studio, can steer a roll |

## The game

Each player is a merchant with one caravan. A table of 2 to 6 merchants plays 12 market days across five towns. The richest reputation at the end wins the table, not the richest purse.

One market day, in order:

1. **Commit.** Each merchant commits to a secret for the day's road dice, as a hash-locked escrow (see Fair dice).
2. **Move.** Each merchant sends one transaction: where the caravan goes, what it buys and sells there. Goods are game tokens on the ledger; buying and selling are real offers on the XRPL order book, so merchants trade with each other at prices they set.
3. **Reveal.** Each merchant reveals the secret by finishing the escrow. The day's roll is computed from every reveal plus a drand beacon round named in advance.
4. **Road events.** The roll decides each road: bandits, a toll, a flood, a fair. Every client computes the same outcome from the ledger, so no server decides.
5. **Contracts settle.** Promises between merchants come due. "Deliver 3 salt to Varn by day 6 for 20 marks" is a Check the buyer writes and the seller cashes on delivery. A broken promise costs reputation, in public, on the ledger.

A day takes about two minutes when everyone is online, and up to a set number of ledgers (about a day of real time) when they are not. Turns are slow on purpose: the ledger settles in 3 to 5 seconds, and slow turns let a table play across a week.

**A season** runs from one Testnet reset to the next. It starts with the season's issuer, five towns and one new rule (a new good, road or contract type). It ends with a hall of fame and a signed season record for every player, and the reset wipes the ledger.

## Hard constraints

These conditions are fixed. Each is enforced by the code or by CI, not by good intentions.

| Condition | How it is enforced |
| --- | --- |
| Testnet only, no mainnet code paths | The client knows only Testnet endpoints, and on connecting it checks the server's `network_id` is 1 (Testnet) and refuses anything else. A CI test fails on any Mainnet hostname, or on `network_id` 0, anywhere in the source. |
| No real-money language | Game words only: marks, goods, purse, caravan. A copy check in CI fails on money words (buy-in, cash out, profit, invest, wallet value) in strings and docs. |
| Faucet calls rate-limited | One faucet call per new player per season. The seed stays on the player's machine; a 429 waits and retries with backoff. 100 test XRP covers the reserves for about 490 owned objects, far more than a season needs. |
| Wipes designed in from day one | Seasons end at resets (see Seasons and resets); the season record lives off the ledger. |
| One shared reset module | The announced-reset list and the "ledger went backwards" check come from an existing shared module used by three other Testnet tools. A new reset is added in all four places. |

## Research grounding

Four research questions, 2026-11-14. "Checked" says how each finding was confirmed: read at the source, measured on the live ledger, or recalled and still to be confirmed. The evidence on seasons is the weakest: press and forums, no developer data.

| Finding | Source | Checked | What it changes |
| --- | --- | --- | --- |
| With a dishonest majority, someone can always bias a coin flip by aborting | Cleve, 1986 | From memory | Fairness needs a rule for the player who does not reveal, not just a better protocol |
| One honest reveal makes a combined roll unpredictable; a skipped reveal is the attack | EIP-4399 (Ethereum RANDAO), 2021 | Read at source | A missing reveal loses the round and never triggers a re-roll |
| A delay or beacon value published after the reveal deadline removes the last revealer's edge | Boneh et al., "Verifiable Delay Functions", 2018; drand | Read at source | Mix in a drand round named in advance |
| A block or ledger hash can be steered by whoever orders transactions | xrpl.org consensus principles | Partly read | The ledger hash is never the only source of a roll |
| Faucets add value and sinks remove it; an economy is tuned as flows | Lehdonvirta and Castronova, *Virtual Economies*, 2014; CCP's monthly economic reports | Summaries only | Every source of marks gets a paired sink, and the flows are on a dashboard from day one |
| Axie Infinity's currency collapsed when issuance scaled with players and sinks did not | Decrypt and Cointelegraph, 2022 | Press | Sinks scale with the number of players; there is a written cut rule |
| Without a trusted authority, one person can always run many identities | Douceur, "The Sybil Attack", 2002 | Search result only | Do not try to stop extra accounts; cap what each can earn |
| Social trust graphs limit fake identities rather than stop them | Yu et al., "SybilGuard", 2006 | Search result only | Ranked play needs an invitation from an established player |
| Players return for a season that brings something new, not for the reset | Diablo IV, Path of Exile and Tarkov coverage, 2020 to 2025 | Press and trackers | Each season adds one new rule, announced before the wipe |
| Rollback anger came from narrow compensation windows and exploits | Guild Wars 2 rollback, 2020 | Press | Carry-over depends on past play, not on being online at the wipe; trading freezes before it |
| A Testnet ledger closes every 3.3 seconds; reserves are 1 XRP plus 0.2 per object | Live Testnet `server_info`, rippled 3.4.1 | Measured | Turn-based play only; one faucet grant covers a season |
| Escrow, crypto-conditions and payment channels are retired amendments, so always on | rippled 3.4.1 `features.macro` | Read at source | Hash-locked escrows are available for commit and reveal |
| After the 25 Nov reset, Testnet amendments are set to match Mainnet's | xrpl.org reset notice, 2026-11-09 | Read at source | Use only features enabled on Mainnet; all the ones this design uses are |

## Economy

Test XRP is never the currency. It only pays the ledger's reserves and fees. The game has two currencies, both tokens from the season issuer, and only one of them is earned.

| Currency | How you get it | What it is for | When it goes |
| --- | --- | --- | --- |
| **Table coin** | Handed out at the start of each table, the same to every merchant | Trading inside that table | Burned at the table's end, so farming it gains nothing |
| **Marks** | Earned only from a finished table, by final standing | Things that last the season: a guild charter, a new route, titles and caravan colours | Spent on those, or wiped at the reset |

**Sources and sinks, paired.** Every source of marks has a sink that grows with the number of players:

- Finishing a table earns marks; town charters and route licences cost marks.
- Road events (tolls, repairs) cost table coin, paid back to the issuer. On the XRPL, a token sent back to its issuer is destroyed, so that is a real sink, not a transfer.
- Perishable goods (fruit, fish) spoil: the issuer takes them back after a set number of days. The ledger's Clawback feature does this, and it is enabled on Mainnet, so it survives the reset.

**Free accounts are made pointless, not stopped.** Accounts cost nothing, so one person can always make many (Douceur 2002). Instead:

- An account earns marks from at most 3 tables a day.
- A table pays marks only with at least 3 distinct accounts, at least 2 of them established (5 finished tables this season).
- Ranked league play needs an invitation from an established merchant, and an inviter whose invitees cheat loses reputation.
- Rewards are not zero-sum against other players, so a farm of alts gains only its own capped trickle.

**A flow dashboard from day one.** Marks issued and destroyed per day, read straight off the issuer's ledger balances. A written rule: if marks outstanding grow more than 25% in a week, rewards per table drop 20% the next week. The 25% and 20% are starting guesses, to tune in the first season.

## Fair dice

A day's roll is fixed by every merchant's secret plus a public beacon value that nobody knows until after the reveal deadline. No player, and not the studio, can steer it. The ledger itself holds each commitment and enforces the deadline.

1. **Commit.** Each merchant picks a random 32-byte secret and creates an escrow locked to its SHA-256 hash (a crypto-condition), with a cancel time at the reveal deadline. The escrow holds a token amount of test XRP; it is a sealed envelope, not a bet.
2. **Reveal.** After the commit window closes, each merchant finishes their own escrow with the secret. The ledger accepts the finish only if the secret matches the hash, so every reveal is checked by the network and is public.
3. **Beacon.** The rules name the drand round: the first one published after the reveal deadline. Nobody can know it while reveals are still open.
4. **Roll.** The roll is SHA-256 of the secrets in account order, followed by that drand value. Each road event takes its number from this one value, by a fixed index.

**A merchant who does not reveal loses the day.** The caravan stays in town and pays the toll. The roll goes ahead with the others' secrets, and there is never a re-roll: a re-roll would give the last revealer a free second chance (EIP-4399). The ledger proves the miss, because after the cancel time that escrow can no longer be finished.

**Why both the secrets and the beacon.** Secrets alone let the last revealer see the outcome and pull out; the beacon alone would trust drand's operators. Together, a player can only lose by not revealing. drand's value is a threshold signature that is fixed for each round, so its operators could withhold a round but not choose its value (from drand's design, still to be confirmed at its documentation).

**If drand is unreachable,** the roll waits for it. It never falls back to the ledger hash or the studio's own number, because those can be steered.

## Seasons and resets

Every Testnet reset ends a season, and the game treats that as the season finale, not an outage. The ledger keeps the season's play; a signed season record, kept off the ledger, keeps who did what.

| Survives the reset | Wiped with the ledger |
| --- | --- |
| Titles and the hall of fame | Marks and table coin |
| A signed season record per player: tables played, standings, promises kept and broken, with the transaction hashes | Goods, caravans, charters and routes |
| Reputation, as a starting rank next season (capped, so veterans do not start rich) | Open contracts and escrows |
| Caravan colours and other cosmetics | Every account, including the issuer |

**A season ends in four steps, before the announced reset:**

1. 48 hours before the earliest moment the reset could start, new tables stop opening.
2. 24 hours before, trading and contracts freeze, so nobody can exploit the last hours.
3. Each client writes the season record and the studio signs the hall of fame; both name the last ledger index seen.
4. After the reset, the next season opens with a new issuer, the carried titles and one new rule announced before the wipe.

**An unannounced reset is handled the same way, late.** The client notices the ledger index fall below the one it saw before (with a 1,000-ledger tolerance for server lag). It then closes the season at the last state it saw, with the same carry-over rule. Carry-over depends on what a player did during the season, never on being online at the moment of the wipe.

**Proofs outlive the ledger.** A season record includes its transaction hashes, which can no longer be checked once the ledger is gone. Verifying an old record reports "can no longer be checked", never a pass.

## How it runs on Testnet

The game uses only XRPL features that are enabled on Mainnet as well as Testnet. After the 25 Nov reset, the Testnet's amendments are set to match Mainnet's, and anything Testnet-only could vanish. Each feature below was checked on both networks on 2026-11-14.

| Feature | Used for | Testnet now | Mainnet, so after the reset |
| --- | --- | --- | --- |
| Issued tokens and trust lines | Marks, table coin, goods | Yes | Yes |
| Offers and the order book | Trading between merchants | Yes | Yes |
| Escrow with a crypto-condition | Dice commitments | Yes, always on (retired amendment) | Yes |
| Checks | Contracts between merchants | Yes | Yes |
| Clawback | Perishable goods spoiling | Yes | Yes |
| NFTs | Optional: charters and caravan cosmetics | Yes | Yes |
| Hooks or smart contracts | Not used | No (they run on Xahau, a separate network) | No |

**Measured on the live Testnet, 2026-11-14:**

- A ledger closes about every 3.3 seconds (rippled 3.4.1, network id 1). A move settles in 3 to 5 seconds and is final.
- The base reserve is 1 XRP and each owned object (trust line, offer, escrow, check) holds another 0.2 XRP. The fee is 0.00001 XRP.
- A transaction's memos are limited to 1 KB in total, which is plenty for a move.

**The game needs its own server, eventually.** The public Testnet servers limit requests per IP and publish no numbers. Early tables can use them; a busy season needs the studio's own Testnet node.

## Risks

The biggest risk is that the game is less fun than the ledger is clever. The first playable build is one table of three merchants, played by people, before any season or league work.

| Risk | What happens | Fallback |
| --- | --- | --- |
| The game is not fun | Nobody plays a second table | Playtest one table first; change the rules before building seasons |
| Turns feel too slow | A day takes minutes, a table takes days | Short tables (6 days) for new players; turn reminders |
| The public Testnet server rate-limits or goes down | Moves fail or stall | Retry with backoff; the studio's own Testnet node for busy seasons |
| drand is unreachable | Rolls wait | Waiting is safe; never fall back to a source that can be steered |
| A reset comes with no notice | The season ends early | The "ledger went backwards" check closes the season from the last state seen |
| The reset changes a feature the game uses | Part of the game stops working | Only features enabled on Mainnet are used; check the amendment list after every reset |
| Testnet history is pruned | Old moves can no longer be looked up | The season record keeps what matters, off the ledger |
| Players treat marks as worth money | Real-money language, trading off-platform | Marks never leave the game's rules, copy checks in CI, and every season wipes them |
| One issuer account is a single point of trust | The studio could mint marks unfairly | The flow dashboard publishes every issue and burn from the ledger itself |

## Sources

Researched on 2026-11-14. "Read" means the page was opened; "search" means only a search result was seen; "memory" means it still needs checking at the source.

**XRPL, measured or read**

- [Testnet and Devnet reset, 25 Nov 2026](https://xrpl.org/blog/2026/testnet-devnet-reset) (xrpl.org, 2026-11-09). Read.
- [rippled 3.4.1 amendment list, `features.macro`](https://raw.githubusercontent.com/XRPLF/rippled/3.4.1/include/xrpl/protocol/detail/features.macro). Read: Escrow, PayChan and CryptoConditions are retired.
- Live Testnet `server_info` and `feature`, and one read-only `feature` query on Mainnet, 2026-11-14. Measured.
- [Reserves](https://xrpl.org/docs/concepts/accounts/reserves), [Transaction common fields (memos)](https://xrpl.org/docs/references/protocol/transactions/common-fields) and [Rate limiting](https://xrpl.org/docs/references/http-websocket-apis/api-conventions/rate-limiting) (xrpl.org). Read.
- [Consensus principles and rules](https://xrpl.org/docs/concepts/consensus-protocol/consensus-principles-and-rules) (xrpl.org). Partly read.

**Fair randomness**

- [EIP-4399: the RANDAO opcode](https://eips.ethereum.org/EIPS/eip-4399) (Ethereum, 2021). Read.
- [Boneh, Bonneau, Bünz and Fisch, "Verifiable Delay Functions"](https://eprint.iacr.org/2018/601) (2018). Read.
- [drand and the League of Entropy](https://drand.love). Partly read; the threshold-signature details are from memory.
- Cleve, "Limits on the Security of Coin Flips When Half the Processors Are Faulty" (STOC 1986); Blum, "Coin Flipping by Telephone" (1981). Memory.

**Economies and identities**

- [Lehdonvirta and Castronova, *Virtual Economies: Design and Analysis*](https://mitpressbookstore.mit.edu/book/9780262535069) (MIT Press, 2014). Search; the book itself was not read.
- [CCP, EVE Online Monthly Economic Report, April 2026](https://www.eveonline.com/news/view/monthly-economic-report-april-2026). Search.
- [Decrypt on Axie Infinity's economy changes](https://decrypt.co/92190/axie-infinity-making-big-changes-to-fix-its-ailing-play-to-earn-nft-economy) (2022). Search; figures differ between sources.
- [Douceur, "The Sybil Attack"](https://www.microsoft.com/en-us/research/publication/the-sybil-attack/) (IPTPS 2002). Search.
- [Yu, Kaminsky, Gibbons and Flaxman, "SybilGuard"](https://www.comp.nus.edu.sg/~yuhf/sybilguard-sigcomm06.pdf) (SIGCOMM 2006). Search.

**Seasons and wipes** (press and forums only; no developer data was found)

- [Mein-MMO on the Escape from Tarkov 0.12 wipe](https://mein-mmo.de/en/escape-from-tarkov-patch-that-deleted-everything-attracts-more-players-than-ever-before,510423) (2020). Read; the player count is the developer's own claim.
- [Icy Veins on Diablo IV Season 11 retention](https://www.icy-veins.com/d4/news/diablo-4-season-11-player-retention-steam/). Search only (the page refused the fetch).
- [Path of Exile wiki: Leagues](https://www.poewiki.net/wiki/League). Search.
- [GameSpot: Rust force wipes explained](https://www.gamespot.com/articles/rust-force-wipes-schedule-explained/1100-6505244/). Search.
- [Mein-MMO on the Guild Wars 2 rollback compensation](https://mein-mmo.de/en/guild-wars-2-offers-compensation-for-rollback-and-is-being-criticized,504743) (2020). Read.
