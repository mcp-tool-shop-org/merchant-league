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

Un jeu de commerce et de politique au rythme lent, pour quatre à six joueurs, qui se déroule sur le **Testnet** XRP Ledger. Les joueurs peuvent être des personnes ou des joueurs IA, assis à la même table et soumis aux mêmes règles.

**État : prototype, en cours de développement.** Le mode **Club hors ligne** dans `league-tui` est jouable à partir du code source sur un terminal partagé (4 à 6 joueurs humains et `ClubBot` joueurs IA, depuis le lobby jusqu’au tableau de bord de fin de saison et l’exportation des replays ; les messages privés sont différés après `v0.1.0`). Le mode de jeu Public-Testnet et l’image conteneur ne sont pas encore créés, et il n’y a **pas encore de version publiée**. Uniquement sur le Testnet : pas d’argent réel, et pas de possibilité de passer au Mainnet.

## Le jeu

- **Un cercle de marchands.** Chaque joueur produit ses propres matières premières et a besoin de produits provenant des autres joueurs pour couvrir ses dépenses, faire fonctionner ses ateliers et livrer des produits à des projets communs. Personne ne peut prospérer seul, et aucun petit groupe ne peut le faire non plus.
- **Tours simultanés.** Chaque tour comporte trois phases. Les joueurs discutent et échangent sur le marché ouvert ; puis chaque joueur soumet ses commandes en secret ; enfin, les commandes de chaque joueur sont exécutées en même temps, selon les mêmes règles pour tous.
- **Politique.** Les conseils fixent les tarifs, les cartels peuvent ouvrir des marchés privés, et les chartes sont mises en rotation entre les joueurs au cours de la saison.
- **Une saison de 24 à 32 tours.** Les événements imprévus (gel des approvisionnements, suspension des droits de douane, augmentation de la demande ou pic de détérioration) sont annoncés publiquement un tour avant qu’ils ne se produisent, et le dernier tour est aléatoire, de sorte que personne ne puisse planifier un dernier mouvement parfait.
- **Score.** Un joueur calcule sa valeur nette à la fin, plus une carte d’objectif cachée distribuée au début. Chaque joueur a une raison de jouer jusqu’au dernier tour.
- **Tables vérifiées.** Chaque table est générée à partir d’une graine et vérifiée avant d’être utilisée. La valeur de départ de chaque joueur doit se situer dans une fourchette de 3 % de la moyenne de la table, et dans une fourchette de 7 % de la moyenne lorsque la valeur d’un joueur est déterminée par ce qu’il apporte à chaque groupe de joueurs.

La conception complète est disponible dans [`docs/design-v2.md`](docs/design-v2.md).

## Deux façons de jouer

| Mode | Où il fonctionne | À qui il s’adresse |
| --- | --- | --- |
| **Game mode** | le réseau de test public XRPL, à environ deux tours par jour | des personnes, seules ou avec des joueurs IA |
| **Mode d’entraînement et d’évaluation** | un nœud `xrpld` autonome privé dans un conteneur Docker local, qui clôture les ledgers en quelques millisecondes | Joueurs IA : entraînement, tests de performance et replays |

Les deux modes partagent le même moteur de règles. Un jeu est rejoué exactement à partir de sa graine et de son journal.

## Ce qui est créé

| Module | Ce que cela fait |
| --- | --- |
| `league-core` | Le moteur de règles : les phases du tour, les contrats, le score, la fin de la saison, le replay et les prédicats d’état qui sous-tendent le système d’indices prévu |
| `league-gen` | Le générateur de table, avec ses vérifications d’équité et ses ensembles de graines |
| `league-ledger` | Le client XRPL Testnet. Il refuse tout réseau qui n’est pas le Testnet |
| `league-standalone` | Le module `xrpld` autonome pour le mode d’entraînement, dans un conteneur isolé avec des ressources limitées |
| `league-season` | Les dates de réinitialisation annoncées du Testnet, et une vérification qui détecte une réinitialisation non annoncée |
| `league-relay` | L’outil qui a permis de réaliser les tests de jeu avec des joueurs IA |
| `league-tui` | L’hôte du terminal du mode Club hors ligne : configuration du lobby, tours en mode « hot-seat » à quatre voies, rideau de confidentialité avec codes PIN facultatifs, accès chronométrés, sauvegarde/reprise en milieu de tour, tableau de bord de fin de saison et exportation des replays |

## Jouer une saison

Le mode **Club hors ligne** exécute une saison complète de `24..=32` tours en mode « hot-seat » sur votre terminal (`4–6` joueurs humains ou `ClubBot` joueurs sur une seule machine) :

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configuration du lobby :** Choisissez le nombre de sièges (`4–6`, `Left`/`Right`), activez ou désactivez chaque siège en alternant entre `Human` et `ClubBot` (`Space`), entrez une valeur `SeedPool::Dev` précise dans `0..=9999` (les valeurs réservées pour les tests et les autres ensembles sont refusées), choisissez un nombre maximal de tours (`180–300 s`, `210 s` par défaut, ou `Untimed` avec `Space`), et activez ou désactivez les codes PIN à 4 chiffres spécifiques à chaque table. Appuyez sur `Enter` pour lancer le **mode Club**. Le lancement alors qu’une partie `club-*.seal` inachevée est en cours dans `--save-dir` est refusé (`UNFINISHED_SEASON_EXISTS`) ; une fois exportée, la table suivante reçoit l’ID de table libre suivant sans écraser les sauvegardes précédentes. Les options en ligne de commande (`--turn-clock`, `--untimed`, `--pins`) remplacent les options conflictuelles du lobby et les options `.seal` sauvegardées (`league-tui --help` est prioritaire ; la transmission des chiffres du code PIN via `--pin` dans l’interface en ligne de commande est refusée).
- **Touches rapides :** `1`/`2`/`3`/`Tab`/`BackTab` active/désactive les onglets du compositeur ; `4: Diplomatic Desk (Disabled)` reste désactivé ; `F6` fait défiler le focus entre le registre public, les onglets du compositeur et le coffre-fort privé ; `[`/`]` fait défiler les types d’actions ou les sections du paquet ; `Up`/`Down`/`PageUp`/`PageDown`/`Home` sélectionne ou fait défiler ; `Left`/`Right`/`+`/`-`/`Space` ajuste les champs ; `e` modifie les remplacements de texte libre ; `n` modifie les notes personnelles du coffre-fort privé ; `Enter` soumet une action, met en scène une section du paquet ou exécute le `p` environnement de test « What-If » ; `Del`/`Backspace`/`X` efface les commandes mises en scène ; `c` bascule entre les noms complets de la guilde et les codes à 3 lettres (`[CRED]`, `[TMB]`) ; `r`/`F2` ouvre la fenêtre modale du journal des récompenses ; `Ctrl+l`/`Ctrl+L` verrouille votre mot de passe et ouvre le rideau de confidentialité ; `q`/`Esc`/`Ctrl+c` quitte proprement.
- **Rideau de confidentialité, temps d’attente et codes PIN :** Chaque transfert, délai d’attente et reprise en cours de saison ouvre un rideau de confidentialité public (en effaçant l’écran alternatif en utilisant `CSI 2 J` et en empêchant toute `CSI 3 J` restauration de l’historique), vide la mémoire tampon des entrées et impose un `400 ms` temps d’attente pour les entrées, tant à l’ouverture qu’à la levée (le temps d’attente de la levée ne compte pas dans le temps d’utilisation du siège). Appuyez sur `Space` puis sur `Enter` pour lever ; si les codes PIN sont activés, saisissez votre code PIN à 4 chiffres masqué (il ne peut être saisi que derrière le rideau ; la première fois, il est défini, les transferts suivants le vérifient, les tentatives incorrectes ne génèrent aucune observation du siège, et 3 échecs réinitialisent le rideau à son état initial).
- **Minuteur :** Les tours programmées répartissent chaque budget `75%` Sub-Pass A / `25%` Sub-Pass B entre tous les `H` joueurs (`floor(0.75 * B / H)` s et `floor(0.25 * B / H)` s). Les avantages obtenus en début de partie sont immédiatement verrouillés ; les délais d’inactivité du Sub-Pass B sont résolus grâce à la doctrine de repli du Régent permanent.
- **Enregistrer et reprendre :** Avec `--save-dir <DIR>`, l’hôte enregistre automatiquement et de manière atomique à chaque étape, lors de la première validation du code PIN et après chaque balayage, en laissant `TalkAndMarket`. Les données publiques sont enregistrées dans `club-<id>.save.json` ; les données sensibles, les clés de chiffrement, les preuves VRF précalculées, les notes, les ensembles de données non validés et les codes PIN restent uniquement dans `club-<id>.seal`. Vous pouvez reprendre à tout moment avec `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (ou `--resume ./saves`, en sélectionnant la table `.seal` active) : les reprises en cours de saison se poursuivent toujours après l’étape en cours, avec un nouveau compteur de temps (en faisant avancer les curseurs `ClubBot` en premier), tandis que les reprises après la saison se poursuivent directement sur l’écran de révélation public `Session Standby` (`AwaitingMandateReveal`) ou dans les onglets `Completed` « Fin de saison », afin que vous puissiez réessayer d’exporter la relecture. Le fait de fournir `--resume` avec un autre `--save-dir` est refusé si ce répertoire contient un autre fichier `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, tout ID de table) ou s’il contient `club-<id>.save.json`, `.seal` ou `.replay.json` pour le même ID de table (`SEASON_SAVE_EXISTS`, sauf lors de la reprise d’une saison terminée dont la copie exacte `club-<id>.replay.json` se trouve déjà dans ce répertoire, sans conflit `.save.json` ou `.seal`, de sorte qu’une nouvelle tentative d’exportation après une suppression infructueuse de la source `.seal` se poursuit) ; un répertoire contenant uniquement une saison terminée et exportée avec un ID de table différent est accepté sans écraser quoi que ce soit. Si vous ciblez un fichier `--save-dir` manquant ou vide, il sera créé si nécessaire lors de l’exportation, et le fichier `.seal` repris sera supprimé après une exportation réussie. Une reprise en cours de saison dans un autre fichier `--save-dir` continue la saison à cet endroit et laisse les fichiers `club-<id>.save.json` et `.seal` du répertoire d’origine en place. L’exportation de la relecture dans la même session supprime également le fichier `.seal` d’origine ; si vous quittez et reprenez plus tard à partir du nouveau répertoire, le fichier `.seal` d’origine reste en place, contenant toujours les valeurs privées de cette saison et empêchant le répertoire d’origine d’accepter une nouvelle saison (`UNFINISHED_SEASON_EXISTS`). Supprimez cette copie restante uniquement après avoir quitté et que la saison a été enregistrée dans le nouveau répertoire. À part cette copie restante, conservez les fichiers `club-<id>.save.json`, `.seal` et `.replay.json` de chaque saison sous les noms et dans le répertoire `league-tui` qui leur ont été attribués ; pour déplacer une saison vers un autre emplacement, reprenez-la avec `--save-dir <EMPTY_DIR>` plutôt que de renommer ou de déplacer les fichiers manuellement.
- **Fin de saison et exportation de la rediffusion :** Une fois que la période de clôture `TalkAndMarket` est terminée, passez à l’étape publique de révélation des mandats (`Enter`/`Space` par siège, en affichant aucun mandat au préalable) afin de finaliser l’opération et d’accéder à l’écran « Fin de saison » à quatre onglets (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). Dans l’onglet 4, appuyez sur `e` ou `Enter` pour enregistrer et vérifier `club-<id>.replay.json`, puis effacer le fichier privé `.seal` (si une rediffusion d’une autre saison occupe déjà ce chemin d’accès, l’exportation est refusée `REPLAY_TARGET_EXISTS` sans écrasement ; en cas d’échec de l’exportation, `.seal` reste sur le disque afin que vous puissiez réessayer).

## Compilation à partir du code source

Vous avez besoin de [Rust](https://rustup.rs/). La chaîne d’outils fixée (1.100.0) s’installe automatiquement à partir de `rust-toolchain.toml` lors de la première compilation.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

Les builds et les tests CI sont effectués sur Linux, et l’espace de travail est développé sur Windows. Les autres plateformes n’ont pas été testées.

Docker est nécessaire pour compiler et exécuter l’image de conteneur du mode Club local ou pour exécuter le module de test autonome `xrpld` (les tests du module sont ignorés, sauf si vous le demandez) :

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` exécute les vérifications que CI exécute, à l’exception du module Docker autonome et de la vérification de la carte Atlas. La couverture, `cargo audit`, la création du site et la vérification du conteneur Club conservé ne s’exécutent que lorsque `cargo-llvm-cov`, `cargo-audit`, `npm` et Linux `docker` (avec POSIX `python3` `pty`/`fcntl`/`termios`) sont disponibles, et le script indique toutes les vérifications qu’il a ignorées.

## Exécution du mode Club dans Docker (build local)

L’image de conteneur du mode Club (`league-tui`) n’est pas encore publiée dans un registre de conteneurs ; le `Dockerfile` multi-étapes est compilé localement et vérifié par la vérification hors ligne CI (`scripts/smoke-container.sh`, pas de publication dans le registre).

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

Sur Windows avec Docker Desktop, dans PowerShell :

```powershell
docker build -t merchant-league:local .
mkdir saves
docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local
```

- **Sauvegarde et points de montage :** Sur Linux Docker Engine, l’utilisateur par défaut de l’image `65532:65532` ne peut pas écrire dans un répertoire de l’hôte qui appartient à votre compte (ou `root`, créé par un chemin `-v` manquant). Par conséquent, la création préalable de `./saves` et le passage de `--user "$(id -u):$(id -g)"` permettent de conserver les sauvegardes appartenant à votre utilisateur non-root sans `chmod 777` ou root. Vous pouvez également utiliser un volume nommé Docker (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, sans `--user`), qui hérite du `65532:65532` appartenant à l’image `/saves`. Sur Docker Desktop (macOS et Windows), les autorisations de point de montage sont virtualisées dans la VM. Ainsi, la commande PowerShell ci-dessus, ainsi que le même point de montage sur macOS, fonctionnent sans `--user`, tandis que les shells Linux WSL 2 peuvent exécuter la commande Linux `--user` ci-dessus. Avec Docker Engine sans root, les ID d’utilisateur des conteneurs sont mappés différemment (dans le conteneur, vos propres fichiers apparaissent comme appartenant à `root`, et tout autre ID est mappé à un ID subordonné). Par conséquent, la commande `--user` ci-dessus ne donne pas au conteneur votre propre compte ; utilisez plutôt le volume nommé. Docker Desktop pour Linux exécute l’engine dans une VM et partage des dossiers avec celui-ci via son propre mécanisme. Si le point de montage ne peut pas enregistrer les données, utilisez également le volume nommé. Docker Engine sans root, Docker Desktop pour Linux et macOS n’ont pas été testés par nous.
- **Terminal interactif requis :** Passez `-it` pour jouer en mode interactif. Le lancement sans TTY attaché entraîne la sortie de `1` avec le code `TTY_REQUIRED` et son indication `docker run --rm -it <image>` ; `--help` et `--version` quittent `0` sans TTY.
- **Contenu de l’image :** Créée à partir d’une image `rust:1.100.0-bookworm` (`@sha256:`) avec un hachage fixe, puis construite sur une autre image `gcr.io/distroless/cc-debian12:nonroot` (`@sha256:`) avec un hachage fixe, par défaut, l’utilisateur non-root est `65532:65532` dans `WORKDIR /saves`. En plus de cette base distroless (qui ne contient ni shell ni gestionnaire de paquets), la construction ajoute uniquement `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE` et un répertoire `/saves` vide appartenant à `65532:65532` ; aucun ensemble d’outils, aucun fichier source et aucune communication réseau (`--network=none`).

## Sécurité et données

- **Ce à quoi il accède.** En mode Club hors ligne (`league-tui`), `--save-dir` écrit les `club-<id>.save.json` publics, les `club-<id>.seal` privés (supprimés après une exportation réussie), les `club-<id>.replay.json` vérifiés et les métadonnées `--log-file` facultatives (sans jamais enregistrer les codes PIN, les notes du coffre-fort, les mandats, les commandes en attente ou les sels de validation, et en supprimant les informations d’identification API/XRPL et les clés JSON scellées dans les quatre niveaux de journalisation) sur votre propre machine, sans aucune communication réseau. En mode Jeu (pas encore implémenté), les clés Testnet d’un joueur restent sur sa propre machine, dans le dossier `.league/` ignoré par Git, et ne sont jamais enregistrées. Le relais de test papier lit sa clé OpenRouter uniquement à partir de la variable d’environnement `OPENROUTER_API_KEY` et n’effectue aucun appel OpenRouter sans `--live` et une limite de crédit. Ses sièges locaux refusent toute clé API, n’atteignent qu’une adresse de boucle ou une adresse de VM que vous autorisez avec `--allow-vm-host`, et nécessitent une limite de GPU-secondes. Il conserve ses transcriptions dans le dossier du jeu que vous lui fournissez.
- **Ce à quoi il n’accède pas.** Pas d’argent réel, pas de Mainnet et rien en dehors des dossiers mentionnés ci-dessus. Il n’y a **aucune télémétrie** : rien n’est envoyé nulle part, sauf les appels dont un mode a besoin (le Testnet, un balise de hasard publique et, pour le relais, OpenRouter ou les serveurs de modèle locaux que vous autorisez).
- **Autorisations.** Accès réseau au Testnet en mode Jeu. Docker avec `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (ou volume nommé `-v merchant-league-saves:/saves`) lors de l’exécution du conteneur local du mode Club (n’accédant qu’à `/saves`), ou Docker avec son port lié à `127.0.0.1` uniquement pour le module autonome.
- **Gardiens.** Chaque connexion au Testnet doit signaler l’ID du réseau Testnet. Un déclencheur CI échoue si un nom d’hôte Mainnet est écrit dans le dépôt en dehors du code fourni, et un deuxième déclencheur échoue si le contenu présente le jeu comme une source de revenus. Une revue couvre ce qu’une liste de mots ne peut pas voir.

Consultez [`SECURITY.md`](SECURITY.md) pour connaître le modèle de menace et la manière de signaler une vulnérabilité.

## Documentation

- [Manuel](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Conception v2](docs/design-v2.md) : les règles, l’économie et les deux modes
- [Test papier](docs/paper-test.md) : les premiers tests, effectués sur papier avec des sièges d’IA
- [Module autonome `xrpld`](docs/standalone-xrpld.md)
- [Reçus](docs/receipts/) : vérifications en direct sur le Testnet. Le Testnet est effacé de temps en temps (prochainement le 2026-11-25), de sorte que leurs hachages de transaction cessent d’être résolus par la suite ; ce qu’ils ont enregistré et les vecteurs de test restent la preuve.

## Assistance

Il s’agit d’un prototype en cours de conception. Les problèmes et les idées sont les bienvenus sur le [suivi des problèmes](https://github.com/mcp-tool-shop-org/merchant-league/issues). Il n’y a pas encore de versions et aucune garantie d’assistance. Consultez le [JOURNAL DES MODIFICATIONS](CHANGELOG.md) pour connaître les progrès.

## Licence

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
