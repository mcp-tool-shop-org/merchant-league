import type { SiteConfig } from '@mcptoolshop/site-theme';

export const config: SiteConfig = {
  title: 'Merchant League',
  description:
    'A slow-turn trading and politics game for four to six seats, people or AI players, on the XRP Ledger Testnet. A prototype in its design stage.',
  logoBadge: 'ML',
  brandName: 'merchant-league',
  repoUrl: 'https://github.com/mcp-tool-shop-org/merchant-league',
  footerText:
    'MIT Licensed — built by <a href="https://mcp-tool-shop.github.io/" style="color:var(--color-muted);text-decoration:underline">MCP Tool Shop</a>',

  hero: {
    badge: 'Prototype · design stage',
    headline: 'Merchant League',
    headlineAccent: 'trade, bargain and vote on the XRPL Testnet.',
    description:
      'A slow-turn game for four to six seats. Each seat makes what its neighbours need, so every deal matters and nobody wins alone. People and AI players sit at the same table under the same rules. <strong>Not playable yet</strong>: the rules engine and the table generator are built and tested, and the game client comes next. Testnet only, with no real money.',
    primaryCta: { href: 'https://github.com/mcp-tool-shop-org/merchant-league', label: 'View on GitHub' },
    secondaryCta: { href: 'handbook/', label: 'Read the Handbook' },
    previews: [
      { label: 'Clone', code: 'git clone https://github.com/mcp-tool-shop-org/merchant-league.git' },
      { label: 'Test', code: 'cargo test --workspace --locked' },
      { label: 'Verify', code: 'bash scripts/verify.sh' },
    ],
  },

  sections: [
    {
      kind: 'features',
      id: 'game',
      title: 'The game',
      subtitle: 'What a season at the table looks like.',
      features: [
        {
          title: 'A ring of merchants',
          desc: 'Each seat produces its own raw good and needs other seats’ goods to pay upkeep, run workshops and deliver to shared projects. No seat, and no small group, can prosper alone.',
        },
        {
          title: 'Simultaneous turns',
          desc: 'Talk and trade in the open market, commit your orders in secret, then every seat’s orders resolve at once by the same rules.',
        },
        {
          title: 'Politics',
          desc: 'Councils set tariffs, cartels can open private markets, and charters rotate between seats through the season.',
        },
        {
          title: 'A season with no perfect ending',
          desc: 'Seasons last 24 to 32 turns. Supply shocks are announced two turns ahead, and the last turn is rolled.',
        },
        {
          title: 'Scoring that keeps everyone playing',
          desc: 'Net worth at the close plus a hidden objective card dealt at the start, so every seat has a reason to play until the final turn.',
        },
        {
          title: 'Checked tables',
          desc: 'Every table is generated from a seed and checked before it is dealt: each seat’s starting value sits within 3% of the others’.',
        },
      ],
    },
    {
      kind: 'data-table',
      id: 'modes',
      title: 'Two ways to play',
      subtitle: 'One rules engine; a game replays exactly from its seed and its log.',
      columns: ['Mode', 'Where it runs', 'Who it’s for'],
      rows: [
        ['Game mode', 'The public XRPL Testnet, at about two turns a day', 'People, alone or with AI seats'],
        [
          'Training and evaluation mode',
          'A private stand-alone xrpld node in a local Docker container, closing ledgers in milliseconds',
          'AI seats: training, benchmarks and replays',
        ],
      ],
    },
    {
      kind: 'data-table',
      id: 'built',
      title: 'What is built',
      subtitle: 'A Rust workspace. The game client is the next big piece.',
      columns: ['Crate', 'What it does'],
      rows: [
        ['league-core', 'The rules engine: turn phases, contracts, scoring, the season close, replay and state predicates'],
        ['league-gen', 'The table generator, with its fairness checks and seed pools'],
        ['league-dice', 'Dice no single player can predict or steer: commit and reveal, mixed with a public randomness beacon'],
        ['league-ledger', 'The XRPL Testnet client; it refuses any other network'],
        ['league-standalone', 'The stand-alone xrpld harness for training mode, in a pinned, resource-capped container'],
        ['league-season', 'Season boundaries, which follow the Testnet’s resets'],
        ['league-relay', 'The tool that ran the paper playtests with AI seats'],
        ['league-tui', 'The terminal client: a title screen, for now'],
      ],
    },
    {
      kind: 'code-cards',
      id: 'build',
      title: 'Build from source',
      subtitle: 'You need Rust; the pinned toolchain installs itself on the first build.',
      cards: [
        {
          title: 'Build and test',
          code: 'git clone https://github.com/mcp-tool-shop-org/merchant-league.git\ncd merchant-league\ncargo test --workspace --locked',
        },
        {
          title: 'The stand-alone xrpld harness (needs Docker)',
          code: 'LEAGUE_XRPLD_LIVE=smoke cargo test \\\n  -p league-standalone --test standalone_live --locked',
        },
      ],
    },
  ],
};
