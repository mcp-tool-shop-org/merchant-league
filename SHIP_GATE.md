# Ship Gate

> No repo is "done" until every applicable line is checked.
> Copy this into your repo root. Check items off per-release.

**Tags:** `[all]` every repo · `[npm]` `[pypi]` `[vsix]` `[desktop]` `[container]` published artifacts · `[mcp]` MCP servers · `[cli]` CLI tools

---

## A. Security Baseline

- [x] `[all]` SECURITY.md exists (report email, supported versions, response timeline) — executed by `npx @mcptoolshop/shipcheck security-docs` (A1: present + reporting contact, not an empty stub) (2026-11-17: reporting through GitHub security advisories; supported-versions section; `shipcheck security-docs` run on this branch: passed)
- [x] `[all]` README includes threat model paragraph (data touched, data NOT touched, permissions required) — executed by `npx @mcptoolshop/shipcheck security-docs` (A2: trust/threat-model section present + non-empty; *quality* is not machine-checkable) (2026-11-17: README "Security and data": what it touches, what it does not, permissions)
- [x] `[all]` No secrets, tokens, or credentials in source or diagnostics output — executed by `npx @mcptoolshop/shipcheck secrets` (scans every publishable tarball; matches redacted; not a manual attestation) (2026-11-17: `shipcheck secrets` has no publishable tarball to scan here (skipped); checked instead by the identity scan, CLEAN on the tracked tree, and the relay reads its key only from the environment and redacts errors and panics)
- [x] `[all]` No telemetry by default — state it explicitly even if obvious (2026-11-17: stated in the README and the handbook security page)

### Default safety posture

- [x] `[cli|mcp|desktop]` Dangerous actions (kill, delete, restart) require explicit `--allow-*` flag (2026-11-17: league-relay makes no OpenRouter call without `--live`, which also needs a credit cap; its local seats need `--cap-gpu-secs` and reach only loopback or `--allow-vm-host` addresses)
- [x] `[cli|mcp|desktop]` File operations constrained to known directories (2026-11-17: league-relay writes only inside the `--game-dir` it is given, and `prebook-check` only inside a fresh temporary folder)
- [ ] SKIP: not an MCP server — `[mcp]` Network egress off by default
- [ ] SKIP: not an MCP server — `[mcp]` Stack traces never exposed — structured error results only

## B. Error Handling

- [ ] `[all]` Errors follow the Structured Error Shape: `code`, `message`, `hint`, `cause?`, `retryable?` **OPEN, blocks the v0.1.0 tag (2026-11-17):** errors are typed enums with stable codes (e.g. `SEED_POOL_MISMATCH`), but carry no `hint` or `retryable` field; only league-gen puts a stable code in its messages, and league-relay's `RelayError`, league-ledger's `NotTestnet` and league-dice's `DiceError` carry none
- [ ] `[cli]` Exit codes: 0 ok · 1 user error · 2 runtime error · 3 partial success **OPEN, blocks the v0.1.0 tag (2026-11-17):** league-relay uses 0 ok, 1 error, 2 a seat not clean, 3 an unrecorded call, 4 locked; documented in the handbook, but not the 0/1/2/3 convention
- [x] `[cli]` No raw stack traces without `--debug` (2026-11-17: the relay has no `--debug` flag; its panic hook prints one redacted line and never a backtrace)
- [ ] SKIP: not an MCP server — `[mcp]` Tool errors return structured results — server never crashes on bad input
- [ ] SKIP: not an MCP server — `[mcp]` State/config corruption degrades gracefully (stale data over crash)
- [ ] SKIP: not a desktop app — `[desktop]` Errors shown as user-friendly messages — no raw exceptions in UI
- [ ] SKIP: not a VS Code extension — `[vscode]` Errors surface via VS Code notification API — no silent failures

## C. Operator Docs

- [x] `[all]` README is current: what it does, install, usage, supported platforms + runtime versions (2026-11-17: prototype status, build from source, Rust 1.100.0, platforms)
- [x] `[all]` CHANGELOG.md (Keep a Changelog format) (2026-11-17: Unreleased section)
- [x] `[all]` LICENSE file present and repo states support status (2026-11-17: MIT; README states prototype, no releases, no support guarantees)
- [x] `[cli]` `--help` output accurate for all commands and flags (2026-11-17: league-relay USAGE_TEXT; the handbook reference table follows it)
- [ ] `[cli|mcp|desktop]` Logging levels defined: silent / normal / verbose / debug — secrets redacted at all levels **OPEN, blocks the v0.1.0 tag (2026-11-17):** league-relay has no silent/verbose/debug levels; secrets are redacted in all its output
- [ ] SKIP: not an MCP server — `[mcp]` All tools documented with description + parameters
- [ ] SKIP: no service to operate; the Starlight handbook covers build, design and security — `[complex]` HANDBOOK.md: daily ops, warn/critical response, recovery procedures

## D. Shipping Hygiene

- [x] `[all]` `verify` script exists (test + build + smoke in one command) (2026-11-17: scripts/verify.sh: fmt, Clippy, tests, the two tripwires and their controls; coverage, `cargo audit` and the site build when their tools are installed; the Docker harness and the Atlas check run only in CI)
- [x] `[all]` Version in manifest matches git tag — executed by `npx @mcptoolshop/shipcheck manifest` (D2: manifest version not behind the newest released tag; `--expect <ver>` for a strict release-time match) (2026-11-17: `shipcheck manifest` reads npm/PyPI manifests only, so checked by hand: Cargo workspace 0.0.0, `publish = false`, no tags yet)
- [x] `[all]` Dependency scanning runs in CI (ecosystem-appropriate) — executed by `npx @mcptoolshop/shipcheck ci` (D3: a recognized scanner is *configured* in CI, or dependabot is present) (2026-11-17: cargo audit --deny warnings for the Cargo tree and `npm audit --audit-level=high` for the site, both in CI; `shipcheck ci` passes)
- [x] `[all]` No known high/critical vulnerabilities in any dependency tree, and Dependabot alerts are enabled — executed by `npx @mcptoolshop/shipcheck deps` (the OUTCOME: audits **every** tree incl. subtrees, not just the root; `ci` only proves a scanner is configured) (2026-11-17: cargo audit in CI; site npm audit 0; Dependabot alerts enabled)
- [ ] SKIP: optional; the org rule restricts the update bot, and Dependabot alerts are on — `[all]` Automated dependency **update** mechanism exists <!-- soft/optional: the org rule restricts the auto-PR bot (CI minutes), NOT alerts. The security outcome is enforced by `shipcheck deps`; the update bot is optional. -->
- [ ] SKIP: no npm or PyPI package (Rust workspace, `publish = false`) — `[npm]` Published via OIDC trusted publishing with `--provenance` — executed by `npx @mcptoolshop/shipcheck ci` (config/intent; `--registry <pkg>` also confirms the attestation on npm)
- [ ] SKIP: no npm or PyPI package (Rust workspace, `publish = false`) — `[npm]` **Every publishable package** passes `npx @mcptoolshop/shipcheck pack` — `npm pack --dry-run` on each workspace package includes README.md + LICENSE and all `files[]` entries resolve (executed check, not a manual attestation; in a monorepo it verifies all packages, not just the root)
- [ ] SKIP: no npm or PyPI package (Rust workspace, `publish = false`) — `[npm]` `engines.node` set · `[pypi]` `python_requires` set — executed by `npx @mcptoolshop/shipcheck manifest` (D6, checked per publishable package)
- [ ] SKIP: no npm or PyPI package (Rust workspace, `publish = false`) — `[npm]` Lockfile committed · `[pypi]` Clean wheel + sdist build — lockfile executed by `npx @mcptoolshop/shipcheck manifest` (D7); the pypi wheel/sdist build is not yet executed
- [ ] SKIP: not a VS Code extension — `[vsix]` `vsce package` produces clean .vsix with correct metadata
- [ ] SKIP: not a desktop app — `[desktop]` Installer/package builds and runs on stated platforms

## E. Identity (soft gate — does not block ship)

- [x] `[all]` Logo in README header (2026-11-17: brand logos/merchant-league/readme.png, brand#118)
- [x] `[all]` Translations (polyglot-mcp, 8 languages) (2026-11-17: TranslateGemma 27B, local; hand fixes, pinned in .polyglot-pins.json where the pin tool can hold them: 'paper test' in Japanese, Italian, Portuguese and Spanish, the evaluation-mode row in Chinese, Portuguese, Italian, Spanish and French, the league-dice row in Japanese (pinned with the model's rows in six languages), the Guards bullet in Chinese and Hindi, the league-dice and league-season rows translated by hand in all seven, and the product name kept as 'Merchant League' in every title; the bold Game mode table cell is not held by a pin and is fixed by hand after every run)
- [x] `[org]` Landing page (@mcptoolshop/site-theme) (2026-11-17: site/ with the Starlight handbook)
- [x] `[all]` GitHub repo metadata: description, homepage, topics (2026-11-17: description, homepage and 11 topics set)

---

## Gate Rules

**Hard gate (A–D):** Must pass before any version is tagged or published.
If a section doesn't apply, mark `SKIP:` with justification — don't leave it unchecked.

**Soft gate (E):** Should be done. Product ships without it, but isn't "whole."

**Executed vs attested.** `shipcheck audit` only *counts these checkboxes* — it does not read your repo, so a box can be green while the fact is false. The lines that say **"executed by `npx @mcptoolshop/shipcheck <gate>`"** are backed by a command that reads the real artifact and exits 1 on the real defect. Run those gates (they are wired into shipcheck's own `verify`); don't just tick their boxes. Executed today: **A1/A2** (`security-docs`), **A3** (`secrets`), **D2/D6/D7** (`manifest`), **D3-config + OIDC/provenance** (`ci`), **real vulnerabilities + alerting** (`deps`), **D5** (`pack`), plus front-door (`front-door`) and dogfood freshness (`dogfood`). Every other line is still an attestation you are vouching for. Note the two dependency layers: `ci` proves a scanner is *configured*; `deps` proves there are *no known vulnerabilities* — a repo can pass the first while failing the second.

**Checking off:**
```
- [x] `[all]` SECURITY.md exists (2026-02-27)
```

**Skipping:**
```
- [ ] `[pypi]` SKIP: not a Python project
```
