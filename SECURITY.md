# Security

## Scope

Merchant League plays only on the XRPL **Testnet**. Test XRP and the game's tokens have no value, and the code refuses to connect to any other network. There is no real money anywhere in the game.

That still leaves things worth protecting:

- **Players' Testnet seeds.** They stay on the player's machine, in the git-ignored `.league/` folder, and are never logged or printed.
- **Dice secrets.** Each merchant's secret for a day's roll is hidden until it is revealed on the ledger, and is never printed or logged before that.
- **Fairness.** A roll must not be predictable or steerable by any player, by the studio, or by the randomness beacon's operators.
- **The Testnet-only guarantee.** Any path that could reach Mainnet is a security bug.

## Reporting

Please report a vulnerability privately through GitHub's security advisories for this repository ("Report a vulnerability" on the Security tab), not in a public issue.

Include what you found, how to reproduce it, and what it could let someone do. We aim to acknowledge reports within a week.

## Threat model, in short

| Threat | Defence |
| --- | --- |
| The client is pointed at Mainnet | Only Testnet endpoints exist in the code; every connection must report network id 1; CI fails on Mainnet hostnames |
| A player biases the dice by not revealing | A missing reveal loses the day; there is never a re-roll |
| The last player to reveal sees the outcome first | The roll also mixes in a drand round published after the reveal deadline |
| A player makes many free accounts | Earnings are capped per account and per day; ranked play needs an invitation |
| A seed or secret leaks into logs | Secret types redact themselves in debug output; seeds stay in `.league/` |
