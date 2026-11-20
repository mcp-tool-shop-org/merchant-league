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

**État : prototype, en cours de développement.** Le mode **Club hors ligne** dans `league-tui` est jouable à partir du code source sur un terminal partagé (4 à 6 sièges humains et `ClubBot`, depuis le lobby jusqu’au tableau des scores de fin de saison et l’exportation des replays ; les messages privés sont différés après `v0.1.0`). Le mode de jeu Public-Testnet et l’image conteneur ne sont pas encore créés, et il n’y a **pas encore de version publiée**. Uniquement sur le Testnet : pas d’argent réel, et pas de possibilité de passer au Mainnet.

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
| **Mode jeu** | le réseau de test public XRPL, à environ deux tours par jour | des personnes, seules ou avec des joueurs IA |
| **Mode d’entraînement et d’évaluation** | un nœud `xrpld` autonome privé dans un conteneur Docker local, qui clôture les ledgers en quelques millisecondes | Joueurs IA : entraînement, tests de performance et replays |

Les deux modes partagent le même moteur de règles. Un jeu est rejoué exactement à partir de sa graine et de son journal.

## Ce qui est créé

| Module | Ce que cela fait |
| --- | --- |
| `league-core` | Le moteur de règles : les phases du tour, les contrats, le score, la fin de la saison, le replay et les prédicats d’état qui sous-tendent le système d’indices prévu |
| `league-gen` | Le générateur de table, avec ses vérifications d’équité et ses ensembles de graines |
| `league-ledger` | Le client XRPL Testnet. Il refuse tout réseau qui n’est pas le Testnet |
| `league-standalone` | Le module `xrpld` autonome pour le mode d’entraînement, dans un conteneur à version figée et aux ressources limitées |
| `league-season` | Les dates de réinitialisation annoncées du Testnet, et une vérification qui détecte une réinitialisation non annoncée |
| `league-relay` | L’outil qui a permis de réaliser les tests de jeu avec des joueurs IA |
| `league-tui` | L’hôte du terminal du mode Club hors ligne : configuration du lobby, tours en mode « hot-seat » à quatre voies, rideau de confidentialité avec codes PIN facultatifs, accès chronométrés, sauvegarde/reprise en milieu de tour, tableau de bord de fin de saison et exportation des replays |

## Jouer une saison

Le mode **Club hors ligne** exécute une saison complète de `24..=32` tours en mode « hot-seat » sur votre terminal (`4–6` joueurs humains ou `ClubBot` joueurs sur une seule machine) :

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configuration du lobby :** Choisissez le nombre de sièges (`4–6`, `Left`/`Right`), faites passer chaque siège de `Human` à `ClubBot` et inversement (`Space`), saisissez une graine `SeedPool::Dev` en clair dans `0..=9999` (les graines de test réservées et les autres réserves de graines sont refusées par leur nom), choisissez la durée de chaque tour (`180–300 s`, `210 s` par défaut, ou `Untimed` avec `Space`) et activez ou désactivez les codes PIN de rideau à 4 chiffres propres à la table. Appuyez sur `Enter` sur **Start Club Mode Table**. Démarrer alors qu’un `club-*.seal` inachevé existe dans `--save-dir` est refusé (`UNFINISHED_SEASON_EXISTS`) ; une fois la saison exportée, la table suivante reçoit le prochain ID de table libre sans écraser les sauvegardes précédentes. Les options de ligne de commande (`--turn-clock`, `--untimed`, `--pins`) l’emportent sur les choix contradictoires du lobby et du `.seal` sauvegardé (`league-tui --help` décrit l’ordre de priorité ; passer les chiffres du code PIN avec `--pin` sur la ligne de commande est refusé).
- **Touches du tour :** `1`/`2`/`3`/`Tab`/`BackTab` passent d’un onglet actif du compositeur à l’autre (`4: Diplomatic Desk (Disabled)` reste désactivé) ; `F6` déplace le focus entre le registre public, les onglets du compositeur et le coffre privé ; `[`/`]` parcourent les types d’action ou les sections du paquet d’ordres ; `Up`/`Down`/`PageUp`/`PageDown`/`Home` sélectionnent ou font défiler ; `Left`/`Right`/`+`/`-`/`Space` ajustent les champs ; `e` modifie les remplacements en texte libre ; `n` modifie les notes personnelles du coffre privé ; `Enter` soumet une action, prépare une section du paquet ou lance le bac à sable What-If de `p` ; `Del`/`Backspace`/`X` effacent les ordres préparés ; `c` bascule entre les noms des marchandises de guilde et les codes à 3 lettres (`[CRED]`, `[TMB]`) ; `r`/`F2` ouvre la fenêtre de la Chronique du balayage (Sweep) ; `Ctrl+l`/`Ctrl+L` verrouille votre passage et ouvre le rideau de confidentialité ; `q`/`Esc`/`Ctrl+c` quittent proprement.
- **Rideau de confidentialité, temps mort et codes PIN :** Chaque passage de main, chaque délai dépassé et chaque reprise en cours de saison ouvre un rideau de confidentialité public (qui efface l’écran alternatif sur place avec `CSI 2 J`, jamais avec `CSI 3 J`, qui purgerait l’historique de défilement), vide les entrées en attente et impose un temps mort de saisie de `400 ms` à l’ouverture comme à la levée (le temps mort de la levée n’est pas décompté du temps du siège qui arrive). Appuyez sur `Space` puis sur `Enter` pour lever le rideau ; si les codes PIN sont activés, tapez votre code PIN masqué à 4 chiffres (saisi uniquement derrière le rideau : le 1er tour le définit, les passages suivants le vérifient, une saisie erronée ne produit aucune vue de siège et 3 échecs ramènent le rideau à son état non armé).
- **Horloge :** Les tours chronométrés répartissent chaque budget à `75%` pour la sous-passe A et `25%` pour la sous-passe B entre tous les `H` sièges humains (`floor(0.75 * B / H)` s et `floor(0.25 * B / H)` s). Verrouiller tôt fait avancer immédiatement ; les délais dépassés par inactivité pendant la sous-passe B sont réglés par la doctrine de repli du Régent permanent.
- **Sauvegarde et reprise :** Avec `--save-dir <DIR>`, l’hôte sauvegarde automatiquement et de façon atomique à chaque rideau, au premier engagement de PIN et au balayage qui quitte `TalkAndMarket`. L’état public est enregistré dans `club-<id>.save.json` ; les mandats cachés, les sels, les preuves VRF dérivées à l’avance, les notes, les paquets non engagés et les codes PIN restent uniquement dans `club-<id>.seal`. Reprenez à tout moment avec `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (ou `--resume ./saves`, qui sélectionne la table du `.seal` actif) : une reprise en cours de saison arrive toujours derrière le rideau du siège humain actif, avec une horloge de passage neuve (après avoir fait avancer les curseurs `ClubBot` en avance), tandis qu’une reprise après la saison arrive directement sur l’écran public de révélation `Session Standby` (`AwaitingMandateReveal`) ou sur les onglets de fin de saison `Completed`, pour que vous puissiez réessayer l’export du replay. Passer `--resume` avec un autre `--save-dir` est refusé quand ce répertoire contient un autre fichier `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, quel que soit l’ID de table) ou contient `club-<id>.save.json`, `.seal` ou `.replay.json` du même ID de table (`SEASON_SAVE_EXISTS`, sauf pour la reprise d’une saison terminée dont le `club-<id>.replay.json` normal, identique octet pour octet, est déjà dans ce répertoire sans `.save.json` ni `.seal` en conflit, afin qu’une nouvelle tentative d’export après l’échec de l’effacement du `.seal` source puisse aboutir) ; un répertoire qui ne contient qu’une saison terminée et exportée avec un autre ID de table est accepté sans rien écraser. Indiquer un `--save-dir` absent ou vide le crée au besoin lors de l’export, et le `.seal` repris est quand même effacé si l’export réussit. Une reprise en cours de saison dans un autre `--save-dir` y poursuit la saison et laisse en place le `club-<id>.save.json` et le `.seal` du répertoire d’origine. Exporter le replay dans cette même session efface aussi le `.seal` d’origine ; si vous quittez au contraire et reprenez plus tard depuis le nouveau répertoire, le `.seal` d’origine reste, contient toujours les valeurs privées de cette saison et fait que le lobby du répertoire d’origine continue de refuser une nouvelle saison (`UNFINISHED_SEASON_EXISTS`). Supprimez cette copie restante seulement après avoir quitté et une fois la saison enregistrée dans le nouveau répertoire. Hormis cette copie restante, gardez le `club-<id>.save.json`, le `.seal` et le `.replay.json` de chaque saison sous les noms et dans le répertoire que `league-tui` leur a donnés ; pour déplacer une saison ailleurs, reprenez-la avec `--save-dir <EMPTY_DIR>` au lieu de renommer ou de déplacer les fichiers à la main.
- **Fin de saison et export du replay :** Une fois que la cloche de clôture a fermé `TalkAndMarket`, avancez dans l’étape publique d’attente et de révélation des mandats (`Enter`/`Space` pour chaque siège, aucun mandat affiché avant) jusqu’à l’écran de fin de saison à quatre onglets (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). Dans l’onglet 4, appuyez sur `e` ou `Enter` pour écrire et vérifier `club-<id>.replay.json` et effacer le fichier privé `.seal` (si le replay d’une autre saison occupe déjà ce chemin de destination, l’export est refusé avec `REPLAY_TARGET_EXISTS` sans l’écraser ; si l’export échoue, le `.seal` reste sur le disque pour que vous puissiez réessayer).

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

- **Sauvegardes et montages :** Avec Docker Engine sous Linux, l’utilisateur par défaut de l’image, `65532:65532`, ne peut pas écrire dans un répertoire de l’hôte qui appartient à votre compte (ou que `root` a créé faute de chemin `-v` existant) : créer d’abord `./saves` et passer `--user "$(id -u):$(id -g)"` laisse donc les sauvegardes à votre utilisateur non root, sans `chmod 777` ni root. Vous pouvez aussi utiliser un volume nommé Docker (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, sans `--user`), qui hérite du répertoire `/saves` de l’image, appartenant à `65532:65532`. Sous Docker Desktop (macOS et Windows), les droits des montages liés sont virtualisés à travers la machine virtuelle : la commande PowerShell ci-dessus, et le même montage lié sous macOS, fonctionnent donc sans `--user`, tandis que les shells Linux de WSL 2 peuvent lancer la commande Linux `--user` ci-dessus. Avec Docker Engine sans root (rootless), les ID d’utilisateur du conteneur sont associés autrement (dans le conteneur, vos propres fichiers apparaissent comme appartenant à `root`, et tout autre ID est associé à un ID subordonné) : la commande `--user` ci-dessus ne donne donc pas votre propre compte au conteneur ; utilisez-y le volume nommé. Docker Desktop pour Linux exécute le moteur dans une machine virtuelle et partage les dossiers avec elle par son propre mécanisme ; si le montage lié ne peut pas y enregistrer, utilisez aussi le volume nommé. Nous n’avons pas testé Docker Engine sans root, Docker Desktop pour Linux ni macOS.
- **Terminal interactif requis :** Passez `-it` pour jouer en interactif. Un lancement sans TTY attaché se termine avec le code de sortie `1`, le code `TTY_REQUIRED` et son indication `docker run --rm -it <image>` ; `--help` et `--version` se terminent avec le code de sortie `0` sans TTY.
- **Contenu de l’image :** Construite à partir de `rust:1.100.0-bookworm` figée par digest (`@sha256:`) sur `gcr.io/distroless/cc-debian12:nonroot` figée par digest (`@sha256:`), elle s’exécute par défaut sous l’utilisateur non root `65532:65532` dans `WORKDIR /saves`. Sur cette base distroless figée (qui ne contient ni shell ni gestionnaire de paquets), la construction n’ajoute que `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE` et un répertoire `/saves` vide appartenant à `65532:65532` : aucune chaîne d’outils, aucun fichier source et aucun appel réseau (`--network=none`).

## Sécurité et données

- **Ce à quoi il accède.** En mode Club hors ligne (`league-tui`), `--save-dir` écrit les `club-<id>.save.json` publics, les `club-<id>.seal` privés (supprimés après une exportation réussie), les `club-<id>.replay.json` vérifiés et les métadonnées `--log-file` facultatives (sans jamais enregistrer les codes PIN, les notes du coffre-fort, les mandats, les commandes en attente ou les sels de validation, et en supprimant les informations d’identification API/XRPL et les clés JSON scellées dans les quatre niveaux de journalisation) sur votre propre machine, sans aucune communication réseau. En mode Jeu (pas encore implémenté), les clés Testnet d’un joueur restent sur sa propre machine, dans le dossier `.league/` ignoré par Git, et ne sont jamais enregistrées. Le relais de test papier lit sa clé OpenRouter uniquement à partir de la variable d’environnement `OPENROUTER_API_KEY` et n’effectue aucun appel OpenRouter sans `--live` et une limite de crédit. Ses sièges locaux refusent toute clé API, n’atteignent qu’une adresse de boucle ou une adresse de VM que vous autorisez avec `--allow-vm-host`, et nécessitent une limite de GPU-secondes. Il conserve ses transcriptions dans le dossier du jeu que vous lui fournissez.
- **Ce à quoi il n’accède pas.** Pas d’argent réel, pas de Mainnet et rien en dehors des dossiers mentionnés ci-dessus. Il n’y a **aucune télémétrie** : rien n’est envoyé nulle part, sauf les appels dont un mode a besoin (le Testnet, un balise de hasard publique et, pour le relais, OpenRouter ou les serveurs de modèle locaux que vous autorisez).
- **Autorisations.** Accès réseau au Testnet en mode Jeu. Docker avec `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (ou volume nommé `-v merchant-league-saves:/saves`) lors de l’exécution du conteneur local du mode Club (n’accédant qu’à `/saves`), ou Docker avec son port lié à `127.0.0.1` uniquement pour le module autonome.
- **Gardiens.** Chaque connexion au Testnet doit signaler l’ID du réseau Testnet. Un garde-fou de CI échoue si un nom d’hôte Mainnet est écrit dans le dépôt en dehors du code tiers intégré (vendored), et un second échoue si un texte présente le jeu comme une source de revenus. Une revue couvre ce qu’une liste de mots ne peut pas voir.

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
