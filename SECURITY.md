# Security

## Scope

Merchant League plays only on the XRPL **Testnet**. Test XRP and the game's tokens have no value, and the game's ledger client refuses to connect to any other network. (The stand-alone harness used for evaluation runs connects only to a private local node.) There is no real money anywhere in the game.

That still leaves things worth protecting:

- **Players' Testnet seeds.** In Game mode (not built yet), they will stay on the player's machine, in the git-ignored `.league/` folder, and will never be logged or printed.
- **Committed orders and secrets.** A seat's sealed orders stay hidden until they are revealed, and secret types redact themselves in debug output. The engine checks commitments in pure state; the reveal on the ledger is not built yet.
- **Fairness.** A roll must not be predictable or steerable by any player, by the studio, or by the randomness beacon's operators.
- **The Testnet-only guarantee.** Any path that could reach Mainnet is a security bug.

## Supported versions

The latest release (currently v0.1.0, a source-only prototype) is the supported one. Security fixes land on `main` and ship in the next release.

## Reporting

Please report a vulnerability privately through GitHub's security advisories for this repository ("Report a vulnerability" on the [Security tab](https://github.com/mcp-tool-shop-org/merchant-league/security)), not in a public issue.

Include what you found, how to reproduce it, and what it could let someone do. We aim to acknowledge reports within a week.

## Threat model, in short

| Threat | Defence |
| --- | --- |
| The client is pointed at Mainnet | The game's ledger client names only Testnet endpoints, and every connection must report network id 1; the stand-alone harness accepts only its private node (network id 21337, on loopback); a CI tripwire fails on Mainnet hostnames written in the repository |
| A player biases a roll by not revealing | A seat that commits but does not reveal loses its secret actions for the turn; there is never a re-roll |
| The last player to reveal sees the outcome first | No seat's reveal feeds the randomness. Game mode (not built yet) is designed to take each turn's beacon from a public drand round; checking the round's signature is open (#29). Evaluation runs use a sealed verifiable random function |
| A player makes many free accounts | Planned for Game mode, not built yet: ranked queues need verified matchmaking credentials, hide handles behind seasonal aliases, and flag pairs of seats that funnel wealth to each other |
| A seed or secret leaks into logs | Secret types redact themselves in debug output; in Game mode (not built yet), seeds will stay in `.league/` |

## Dependency advisories

- **2026-11-17:** the site's npm audit (CI, "Site build" step) carries one reviewed, time-boxed exception in `.github/audit-allowlist.json`: GHSA-fc5q-39xc-vjpf (npm id 1247710, picomatch `<=4.0.7`, ReDoS, high, no patched release yet), expiring 2026-12-17. picomatch reaches this repo only through the static-site build toolchain (astro, vite, tinyglobby); exploiting it needs an attacker-supplied glob, every glob in our site config is our own, and the site ships static HTML, so nothing uses picomatch at run time. The exception stops applying at its expiry, if the advisory's range changes, or once npm reports a fix, and it is removed in the PR that bumps picomatch. Every other high or critical advisory still fails CI.
