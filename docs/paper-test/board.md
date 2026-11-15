# Paper-test board

The fixed board for all six paper games: the towns, roads, goods, converters, base ladders, road tables, and how dice are read from drand. Each game's `game-NN/setup.md` sets the rest on top of this board: works, ladder spreads, edicts, sit-outs and seats. Built by hand to the generator's recipe in `docs/design.md`; the generator isn't built yet.

**Numbers the design leaves open** (purses, prices, capacities, perishables) are chosen here for the paper test. They are setup choices, not design rulings, and the test may show they need to change.

## Towns and roads

Five towns on a ring, with one short cut:

```
          Varn (salt flats)
         /                 \
  Ostra (port) ---------- Mire (marsh)     the Ostra–Mire road is the short cut
        |                     |
  Kell (hills) ---------- Hath (woods)
```

| # | Road | Kind |
|---|---|---|
| 0 | Varn–Ostra | safe |
| 1 | Ostra–Kell | safe |
| 2 | Kell–Hath | hill |
| 3 | Hath–Mire | safe |
| 4 | Mire–Varn | marsh |
| 5 | Ostra–Mire | short cut |

Every road takes one day. A caravan travels at most one road a day; its order is `stay` or `go to TOWN` along one road.

## Goods

| Good | Made by | Inputs | Works value | Reference price |
|---|---|---|---|---|
| brine | Ostra's ladder | — | 1 | 1 |
| wool | Ostra's ladder | — | 1 | 1 |
| ore | Kell's ladder | — | 1 | 2 |
| hide | Kell's ladder | — | 1 | 2 |
| timber | the Foresters cut it; Hath's ladder | — | 1 | 1 |
| madder | Mire's ladder | — | 1 | 2 |
| salt | Saltworks | 2 brine (+ pan fuel) | 2 | 5 |
| charcoal | Foresters | 1 timber | 2 | 3 |
| dye | Dyers | 1 madder + 1 salt | 3 | 9 |
| tools | Smiths | 1 ore + 1 charcoal | 3 | 9 |
| leather | Tanners | 1 hide + 1 salt | 3 | 9 |
| cloth | Weavers | 2 wool + 1 dye | 3 | 15 |

Works values follow the design: 1 for a raw good, 2 for a converted good, 3 for a good converted twice. Cloth is converted three times; it takes the design's top value, 3.

**Perishables:** hide and madder. The steward takes back any unit still held at the close of the third day after it was bought: a unit bought on day 4 is gone at the close of day 6. Perishables are never bonded.

## Guilds and converters

| Guild | Home | Converter, per batch | Batches a day | Upgrade (spent once, kept) |
|---|---|---|---|---|
| Saltworks | Varn | 2 brine → 1 salt; the pans burn **1 timber** on any day it converts | 2 | — |
| Weavers | Ostra | 2 wool + 1 dye → 1 cloth | 2 | — |
| Smiths | Kell | 1 ore + 1 charcoal → 1 tools | 2 | **bellows:** spend 1 leather, then 3 batches a day |
| Foresters | Hath | 1 timber → 1 charcoal | 2 | **better axes:** spend 1 tools, then cut 3 timber a day |
| Dyers | Mire | 1 madder + 1 salt → 1 dye | 2 | — |
| Tanners | Hath | 1 hide + 1 salt → 1 leather | 2 | — |

- **Cutting.** On any day the Foresters' caravan is in Hath, they cut 2 timber free (3 with better axes).
- **Where converting happens.** A guild converts only on a day its caravan is in its home town.
- **The start.** Each guild starts with **60 coin**, no goods, standing 0, and its caravan at home.

## Town ladders

The ladders are the steward's markets: they sell raw goods and buy converted ones. A setup sets each game's step, stock and buying share.

| Town | Sells (base price) |
|---|---|
| Ostra | brine 1, wool 1 |
| Kell | ore 2, hide 2 |
| Hath | timber 2 |
| Mire | madder 2 |
| Varn | — |

- **Stepped prices.** In one day, each unit a ladder sells costs the setup's **step** more than the last. At step 1, four brine cost 1 + 2 + 3 + 4 = 10. A ladder sells at most the setup's **stock** of each good a day, and refills at the close.
- **Buying.** Every ladder buys any converted good at the setup's **share** of its reference price, rounded down, up to 3 units of each good a day. Ladders buy no raw goods.
- **A sitting-out guild's good** is sold by the ladder in that guild's home town. Its price is the reference price times the setup's **sit-out multiplier**, rounded up, at 2 units a day, without stepping. That's how every seated guild's needs stay available.

## Road events

Each day, each road gets one number from 1 to 20 (see "Reading dice"). Every caravan travelling that road that day meets its event.

| Number | Safe road | Hill or marsh road | Short cut |
|---|---|---|---|
| 1–2 | bandits | bandits | bandits |
| 3–4 | clear | toll 2 | bandits |
| 5 | toll 2 | flood | flood |
| 6 | clear | flood | flood |
| 7–19 | clear | clear | clear |
| 20 | fair | fair | fair |

- **Odds.**
  - A safe road: bandits 2 in 20, toll 1 in 20.
  - A hill or marsh road: bandits 2, toll 2 and flood 2 in 20.
  - The short cut: bandits 4 and flood 2 in 20.
  - Every road has a fair on 1 in 20.
- **Bandits:** the caravan loses its single most valuable unit (highest works value, then highest reference price). With no goods, it loses 3 coin, or all it has.
- **Flood:** the caravan doesn't move. A flood is the road event that *holds*, in the bond rule's sense.
- **Toll:** the caravan pays the coin to the steward; if it has too little, it pays all it has. Standing 5 or more pays half, rounded down.
- **Fair:** the town the caravan reaches sells at base price all day, without stepping, to everyone.

A caravan that stays in town meets no road event.

## Rules for the paper game the design doesn't state

The referee needs these to decide every day. They are the paper test's first rule-gap findings: the design should state them, or replace them, before a build.

1. **Location.** These need the guild's caravan in the town:
   - buying from or selling to a town's ladder;
   - delivering to a town's works;
   - converting (its home town).

   Offers and payments between guilds don't. On the ledger, a domain offer can't see where a caravan is, so the steward would have to enforce this.
2. **A missed reveal** (a missing, late or invalid act reply) means the caravan stays and pays a toll of 2. The design says "pays the toll" but doesn't say which.
3. **Largest contributor** means the strictly highest works value delivered to that works by the bell. A tie means no one holds it.
4. **Bonds on paper** are coin the referee sets aside from each purse.
   - The bond equals the price, or half the price, rounded up, at standing 3 or more.
   - Kept: both bonds come back on the fourth day after the deadline.
   - Breach: the wronged side receives the breaching side's bond at the deadline.
5. **Contract terms** name the good, the quantity, the town, the price in coin and the deadline day. Barter contracts aren't offered on paper.

## Reading dice from drand

Every roll comes from drand's quicknet chain (period 3 s, genesis 1692803367), as in `design.md`.

- **Rounds.** Each game's day rounds are named by number in its `setup.md`. They're fixed from the time the six setups were fixed, before any round value was published, so no setup could be tuned to its rolls.
- **The value** is the round's `randomness`, 32 bytes, numbered 0–31.
- **Road number** for road *r* (0–5, as in the roads table): (byte 2r × 256 + byte 2r+1) mod 20 + 1.
- **Who fills first.** Offers fill by price. Among offers at the same price, guilds go in order of SHA-256 of the ASCII text `<randomness, lowercase hex>:<guild name>`, lowest first.
- **The bell day.** Each game has its own bell round. Read bytes 24–31 of its value as a big-endian integer, *n*.

  | n mod 27 | The bell rings at the close of |
  |---|---|
  | 0–8 | day 10 |
  | 9–17 | day 11 |
  | 18–23 | day 12 |
  | 24–26 | day 13 |

  Those are the design's odds: 1/3, 1/3, 2/9 and 1/9. The bell day and its round are sealed, and `setup.md` holds only SHA-256(`<day>:<salt>`); both are revealed after the bell.
