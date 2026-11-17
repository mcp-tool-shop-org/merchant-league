<p align="center">
  <a href="README.ja.md">日本語</a> | <a href="README.zh.md">中文</a> | <a href="README.es.md">Español</a> | <a href="README.md">English</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

# Ligue des marchands

Un jeu de commerce et de politique au rythme lent, pour quatre à six joueurs, qui se déroule sur le **Testnet** du XRP Ledger. Les joueurs peuvent être des personnes ou des joueurs IA, à la même table et soumis aux mêmes règles.

**État : prototype, phase de conception.** Le moteur de règles, le générateur de table et la fonction de relecture sont développés et testés. Le jeu lui-même n’est **pas encore jouable** : il n’y a pas de client de jeu et aucune version de sortie. Uniquement sur le Testnet : pas d’argent réel et aucune possibilité de passer au Mainnet.

## Le jeu

- **Un cercle de marchands.** Chaque joueur produit ses propres matières premières et a besoin de biens provenant des autres joueurs pour couvrir ses dépenses, faire fonctionner ses ateliers et réaliser des projets communs. Personne ne peut prospérer seul, et aucun petit groupe ne peut le faire non plus.
- **Tours simultanés.** Chaque tour comporte trois phases. Les joueurs discutent et échangent sur le marché ouvert ; puis chaque joueur soumet secrètement ses commandes ; enfin, les commandes de chaque joueur sont exécutées simultanément, selon les mêmes règles pour tous.
- **Politique.** Les conseils fixent les tarifs, les cartels peuvent ouvrir des marchés privés et les chartes sont mises en rotation entre les joueurs au cours de la saison.
- **Une saison de 24 à 32 tours.** Les chocs (gel des approvisionnements, suspension des tarifs, augmentation de la demande ou pic de détérioration) sont annoncés publiquement un tour avant qu’ils ne se produisent, et le dernier tour est aléatoire, de sorte que personne ne puisse planifier un dernier mouvement parfait.
- **Score.** Un joueur calcule sa valeur nette à la fin, plus une carte d’objectif cachée distribuée au début. Chaque joueur a une raison de jouer jusqu’au dernier tour.
- **Tables vérifiées.** Chaque table est générée à partir d’une valeur de départ et vérifiée avant d’être utilisée. La valeur de départ de chaque joueur doit se situer dans une fourchette de 3 % de la moyenne de la table, et dans une fourchette de 7 % de la moyenne lorsque la valeur d’un joueur est calculée en fonction de sa contribution à chaque groupe de joueurs.

La conception complète est disponible dans [`docs/design-v2.md`](docs/design-v2.md).

## Deux modes de jeu

| Mode | Environnement d’exécution | Public cible |
| --- | --- | --- |
| **Mode jeu** | le Testnet public XRPL, à raison d’environ deux tours par jour | des personnes, seules ou avec des joueurs IA |
| **Mode entraînement et évaluation** | un nœud `xrpld` autonome privé dans un conteneur Docker local, qui exécute les registres en quelques millisecondes | joueurs IA : entraînement, tests de performance et relectures |

Les deux modes partagent le même moteur de règles. Un jeu est rejoué exactement à partir de sa valeur de départ et de son journal.

## Ce qui est développé

| Crate | Ce que cela fait |
| --- | --- |
| `league-core` | Le moteur de règles : les phases du tour, les contrats, le calcul des scores, la fin de la saison, la relecture et les prédicats d’état qui sous-tendent le système d’indices prévu |
| `league-gen` | Le générateur de table, avec ses vérifications d’équité et ses ensembles de valeurs de départ |
| `league-dice` | Pour l’instant, il s’agit de simples prototypes. L’aléatoire du jeu lui-même (engagement et révélation, combinés à un signal de drand public en mode jeu et à une fonction aléatoire vérifiable en mode entraînement) est développé en `league-core` |
| `league-ledger` | Le client XRPL Testnet. Il refuse tout réseau qui n’est pas le Testnet |
| `league-standalone` | L’ensemble `xrpld` autonome pour le mode entraînement, dans un conteneur avec des ressources limitées |
| `league-season` | Les limites de saison, qui suivent les réinitialisations du Testnet |
| `league-relay` | L’outil qui a permis de réaliser les tests sur papier avec des joueurs IA |
| `league-tui` | Le client terminal. Pour l’instant, il n’y a qu’un écran de titre. |

## Compilation à partir du code source

Vous avez besoin de [Rust](https://rustup.rs/). La chaîne d’outils fixée (1.100.0) s’installe automatiquement à partir de `rust-toolchain.toml` lors de la première compilation.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

Les compilations et les tests CI sont effectués sur Linux, et l’espace de travail est développé sur Windows. Les autres plateformes n’ont pas été testées.

Docker n’est nécessaire que pour l’ensemble `xrpld` autonome, et ses tests sont ignorés à moins que vous ne le demandiez :

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` exécute les vérifications que CI exécute, à l’exception de l’ensemble Docker et de la vérification de la carte Atlas. La couverture et `cargo audit` ne sont exécutées que lorsque `cargo-llvm-cov` et `cargo-audit` sont installés, et le script indique toutes les vérifications qu’il a ignorées.

## Sécurité et données

- **Ce à quoi il a accès.** En mode jeu (pas encore développé), les clés Testnet d’un joueur restent sur sa propre machine, dans le dossier `.league/` ignoré par Git, et ne sont jamais enregistrées. Le relais de test sur papier lit sa clé OpenRouter uniquement à partir de la variable d’environnement `OPENROUTER_API_KEY` et n’effectue aucun appel OpenRouter sans `--live` et une limite de crédit. Ses joueurs locaux refusent toute clé API, n’accèdent qu’à une adresse de boucle ou à une adresse VM que vous autorisez avec `--allow-vm-host`, et nécessitent une limite de secondes de GPU. Il conserve ses transcriptions dans le dossier du jeu que vous lui fournissez.
- **Ce à quoi il n’a pas accès.** Pas d’argent réel, pas de Mainnet et rien en dehors des dossiers mentionnés ci-dessus. Il n’y a **pas de télémétrie** : rien n’est envoyé nulle part, à l’exception des appels dont un mode a besoin (le Testnet, un signal de hasard public et, pour le relais, OpenRouter ou les serveurs de modèles locaux que vous autorisez).
- **Autorisations.** Accès au réseau pour le Testnet en mode jeu. Docker, avec son port lié uniquement à `127.0.0.1`, pour l’ensemble autonome.
- **Gardiens.** Chaque connexion au Testnet doit signaler l’ID du réseau du Testnet, CI échoue sur tout nom d’hôte Mainnet, et un deuxième dispositif de sécurité échoue sur toute copie qui présente le jeu comme une source de revenus.

Consultez [`SECURITY.md`](SECURITY.md) pour le modèle de menace et la manière de signaler une vulnérabilité.

## Documentation

- [Manuel](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Conception v2](docs/design-v2.md) : les règles, l’économie et les deux modes
- [Test sur papier](docs/paper-test.md) : les premiers tests, réalisés sur papier avec des joueurs IA
- [Ensemble `xrpld` autonome](docs/standalone-xrpld.md)
- [Reçus](docs/receipts/) : vérifications en direct sur le Testnet. Le Testnet est effacé de temps en temps (prochainement le 2026-11-25), de sorte que leurs hachages de transaction cessent de fonctionner par la suite ; ce qu’ils ont enregistré, ainsi que les vecteurs de test, restent la preuve.

## Assistance

Il s’agit d’un prototype en cours de conception active. Les problèmes et les idées sont les bienvenus sur le [suivi des problèmes](https://github.com/mcp-tool-shop-org/merchant-league/issues). Il n’y a pas encore de versions et aucune garantie d’assistance. Consultez le [JOURNAL DES MODIFICATIONS](CHANGELOG.md) pour suivre les progrès.

## Licence

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
