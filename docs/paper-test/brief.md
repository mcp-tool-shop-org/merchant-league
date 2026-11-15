# Merchant League: the seat brief (v1)

You are playing **Merchant League**, a trading game for 4 or 5 players. Each player runs a guild with its own craft. Guilds trade goods across five towns and build the towns' great works. A referee runs the game and resolves each day by the written rules. Your own card (your guild, your purse and your two secret mandates) comes with this brief.

**Your goal:** finish with the highest score. Score comes only from goods delivered to town works and from your revealed mandates.

## The map

Five towns on a ring, with one short cut. Every road takes one day.

```
          Varn (salt flats)
         /                 \
  Ostra (port) ---------- Mire (marsh)     the Ostra–Mire road is the short cut
        |                     |
  Kell (hills) ---------- Hath (woods)
```

| Road | Kind |
|---|---|
| Varn–Ostra | safe |
| Ostra–Kell | safe |
| Kell–Hath | hill |
| Hath–Mire | safe |
| Mire–Varn | marsh |
| Ostra–Mire | short cut |

Your guild has one caravan. Each day it either stays or travels one road. Your travel order is **sealed**: only the referee sees it until the day is resolved, and everyone sees where every caravan ended up.

## Goods

| Good | Where it comes from | Works value |
|---|---|---|
| brine, wool | Ostra's market | 1 |
| ore, hide | Kell's market | 1 |
| timber | the Foresters cut it; Hath's market | 1 |
| madder | Mire's market | 1 |
| salt | Saltworks: 2 brine → 1 salt, burning 1 timber on any day it converts | 2 |
| charcoal | Foresters: 1 timber → 1 charcoal | 2 |
| dye | Dyers: 1 madder + 1 salt → 1 dye | 3 |
| tools | Smiths: 1 ore + 1 charcoal → 1 tools | 3 |
| leather | Tanners: 1 hide + 1 salt → 1 leather | 3 |
| cloth | Weavers: 2 wool + 1 dye → 1 cloth | 3 |

**Hide and madder spoil.** A unit still held at the close of the third day after you bought it is gone.

## Guilds

There are six guilds, and a game seats 4 or 5 of them.

| Guild | Home | Converter | Upgrade (spend once, keep) |
|---|---|---|---|
| Saltworks | Varn | 2 brine → 1 salt (+1 timber a day) | — |
| Weavers | Ostra | 2 wool + 1 dye → 1 cloth | — |
| Smiths | Kell | 1 ore + 1 charcoal → 1 tools | bellows: 1 leather, then 3 batches a day |
| Foresters | Hath | 1 timber → 1 charcoal; cuts 2 timber a day in Hath | better axes: 1 tools, then cut 3 a day |
| Dyers | Mire | 1 madder + 1 salt → 1 dye | — |
| Tanners | Hath | 1 hide + 1 salt → 1 leather | — |

- **Converting.** A guild converts up to 2 batches a day, only on a day its caravan is in its home town.
- **Sitting out.** The guilds not seated in your game sit out. Their good is still sold, at a high price, by the market in their home town.
- **The start.** Every guild starts with 60 coin, no goods, standing 0 and its caravan at home.

## Town markets

Each town's market sells its raw goods and buys converted goods.

- **Buying from a market.** Its prices step up with each unit sold in a day, and its stock is limited.
- **Selling to a market.** It pays well below a fair price.

Your card gives this game's market prices.

**You must be in town** (your caravan there that day) to buy from or sell to its market, to deliver to its works, and to convert at home. Trading with other guilds doesn't need you in the same town.

## A day

Each day has two rounds.

**1. Talk.** You get the public state and your inbox:
- where every caravan is;
- the markets and open offers;
- the works' progress;
- contracts, bonds and standings;
- any edict;
- your own purse, goods and mandates.

You may send up to **3 private messages**, to named guilds, of at most 80 words each. The referee delivers them next round. You may bargain, promise, bluff or break a promise, within the rules.

**2. Act.** Reply once to the referee, in this form. Leave out any line you don't use.

```
OFFERS:    post <give> for <get> | cancel <offer id>
CONTRACTS: propose <terms> to <guild> | accept <contract id> | bond <contract id>
PAYMENTS:  pay <amount> <good or coin> to <guild>
CONVERT:   <recipe> x<n>
WORKS:     deliver <n> <good> to <works>
TRAVEL:    stay | go to <town>
```

- **Offers** stay open until filled or cancelled.
  - "post 6 coin for 2 salt" is open to every guild.
  - To trade with a market, add `@<town>`: "post 10 coin for 4 brine @Ostra" buys from Ostra's market, and "post 2 tools for 8 coin @Kell" sells to Kell's.
  - When two offers cross, they fill. At the same price, the day's roll decides who fills first. Answering early gains nothing.
- **A missing or late Act reply** means your caravan stays in town and pays a toll of 2. The referee doesn't ask again.

**The day resolves in this order:** travel orders are revealed; road events are rolled; caravans move; offers fill; conversions and works deliveries happen; perishables spoil; contracts reach their deadlines; markets refill.

## Roads

Each road gets one roll a day, from a public random beacon. No one, the referee included, knows it in advance. Every caravan on that road meets the same event.

| Event | Safe road | Hill or marsh road | Short cut |
|---|---|---|---|
| Bandits: you lose your most valuable unit (or 3 coin) | 2 in 20 | 2 in 20 | 4 in 20 |
| Toll: pay 2 coin | 1 in 20 | 2 in 20 | — |
| Flood: you don't move | — | 2 in 20 | 2 in 20 |
| Fair: the town you reach sells at base price all day | 1 in 20 | 1 in 20 | 1 in 20 |

A caravan that stays in town meets no road event.

## Contracts and bonds

A contract is a promise across days: "deliver 3 salt at Varn by day 6 for 20 coin". Its terms name the good, the quantity, the town, the price and the deadline day.

1. **Agree.** One guild proposes it and the other accepts.
2. **Bond.** Each side bonds: coin set aside, equal to the price. At standing 3 or more, the bond is half the price.
3. **Deliver.** By the deadline:
   - the seller pays the goods, on a day its caravan is in the named town;
   - the buyer pays the price.

   Either may pay first, at its own risk.
4. **Settle.** The referee settles the contract at the close of the deadline day:

| What happened | Outcome |
|---|---|
| Delivered and paid | **Kept.** Both bonds come back on the fourth day after the deadline. |
| Delivered, not paid | **Buyer's breach.** The seller gets the buyer's bond. Buyer's standing −3. |
| Not delivered, and no flood held the seller's caravan | **Seller's breach.** The buyer gets the seller's bond. Seller's standing −3. |
| Not delivered, and a flood held the seller's caravan on a road it travelled during the contract | **Delay.** The deadline moves 2 days, once, at no cost in standing. |

## Standing

Standing starts at 0. It never scores, but it does three things:
- **Bonds:** at standing 3 or more, bonds are half price.
- **Tolls:** at standing 5 or more, tolls are half price.
- **Vendetta:** a guild wronged by a breach may close its home town's market to the breaching guild's caravan for 2 days.

How standing moves:
- **+1** for each contract you deliver on time where your caravan travelled at least one road between agreeing and delivering, at most once per partner every 3 days.
- **−3** for each breach.
- **0** for a delay.

## Score

**Score = works points + mandate points.** Coin, goods and standing don't score.

- **Works.** Each town has one or two great works, each needing a set of goods.
  - Each good you deliver scores its works value.
  - The guild that delivers a works' last needed good also scores a **completion bonus of 4**.
  - A completed works takes no more goods.
- **Mandates.** You hold two secret mandates, worth 3 to 8 points each. They're revealed and scored at the bell.
  - "The largest contributor" means the strictly highest works value delivered to that works; a tie means no one is.
- **Ties** go to the higher standing, then the fewer breaches, then the earlier last delivery.

## Edicts and the bell

- **Edicts.** On days 4 and 8 an edict takes effect, for example a town closing its gate, or a levy. Each is announced one day ahead.
- **The bell.** The game ends at a closing bell: at the close of day 10, 11, 12 or 13. Nobody at the table knows which.
