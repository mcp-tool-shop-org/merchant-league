<p align="center">
  <a href="README.ja.md">日本語</a> | <a href="README.zh.md">中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.md">English</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

# Lega dei Mercanti

Un gioco di commercio e politica a turni lenti, per quattro-sei giocatori, giocato sulla **Testnet** di XRP Ledger. I giocatori possono essere persone o intelligenze artificiali, allo stesso tavolo e con le stesse regole.

**Stato: prototipo, fase di progettazione.** Il motore delle regole, il generatore di tavoli e la funzione di replay sono stati creati e testati. Il gioco vero e proprio **non è ancora giocabile**: non esiste un client di gioco e non è prevista alcuna versione. Solo Testnet: niente denaro reale e nessun percorso verso la Mainnet.

## Il gioco

- **Un anello di mercanti.** Ogni giocatore produce le proprie materie prime e ha bisogno di beni dagli altri giocatori per coprire le proprie spese, gestire le proprie attività e fornire progetti condivisi. Nessuno può prosperare da solo, e nemmeno un piccolo gruppo.
- **Turni simultanei.** Ogni turno è suddiviso in tre fasi. I giocatori parlano e commerciano nel mercato aperto; quindi, ogni giocatore invia segretamente i propri ordini; quindi, gli ordini di ogni giocatore vengono risolti contemporaneamente, secondo le stesse regole per tutti.
- **Politica.** I consigli stabiliscono le tariffe, i cartelli possono aprire mercati privati e le cariche ruotano tra i giocatori durante la stagione.
- **Una stagione di 24-32 turni.** Gli shock nell'offerta vengono annunciati due turni prima, e l'ultimo turno viene eseguito in modo casuale, in modo che nessuno possa pianificare una mossa finale perfetta.
- **Punteggio.** Un giocatore calcola il proprio patrimonio netto alla fine, più una carta obiettivo nascosta distribuita all'inizio. Ogni giocatore ha un motivo per continuare a giocare fino all'ultimo turno.
- **Tavoli controllati.** Ogni tavolo viene generato da un seme e controllato prima di essere utilizzato. Il valore iniziale di ogni giocatore deve rientrare nel 3% rispetto agli altri, e nel 7% quando il valore di un giocatore è determinato in base a ciò che aggiunge a ogni gruppo di giocatori.

Il progetto completo è disponibile qui: [`docs/design-v2.md`](docs/design-v2.md).

## Due modalità di gioco

| Modalità | Dove viene eseguito | Per chi è |
| --- | --- | --- |
| **Game mode** | la rete pubblica XRPL Testnet, con circa due turni al giorno | persone, da sole o con giocatori AI |
| **Modalità di addestramento e valutazione** | un nodo `xrpld` autonomo privato in un container Docker locale, che esegue i ledger in millisecondi | giocatori AI: addestramento, benchmark e replay |

Entrambe le modalità condividono lo stesso motore di regole. Un gioco viene riprodotto esattamente dal suo seme e dal suo registro.

## Cosa è stato creato

| Crate | Cosa fa |
| --- | --- |
| `league-core` | Il motore delle regole: le fasi del turno, i contratti, il punteggio, la chiusura della stagione, il replay e i predicati di stato alla base del sistema di suggerimenti previsto |
| `league-gen` | Il generatore di tavoli, con i suoi controlli di equità e i suoi pool di semi |
| `league-dice` | Dadi che nessun singolo giocatore può prevedere o influenzare: invio e rivelazione, combinati con un beacon di casualità pubblico |
| `league-ledger` | Il client XRPL Testnet. Rifiuta qualsiasi rete diversa dalla Testnet |
| `league-standalone` | L'ambiente `xrpld` autonomo per la modalità di addestramento, in un container con risorse limitate |
| `league-season` | I confini della stagione, che seguono i riavvii della Testnet |
| `league-relay` | Lo strumento che ha eseguito i test pratici con giocatori AI |
| `league-tui` | Il client terminale. Per ora, solo una schermata introduttiva |

## Compila dal codice sorgente

È necessario [Rust](https://rustup.rs/). La toolchain con versione specifica (1.100.0) si installa automaticamente da `rust-toolchain.toml` alla prima compilazione.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

Le build CI e i test vengono eseguiti su Linux e l'ambiente di lavoro viene sviluppato su Windows. Altre piattaforme non sono state testate.

Docker è necessario solo per l'ambiente `xrpld` autonomo e i suoi test vengono saltati a meno che non vengano richiesti:

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`scripts/verify.sh` esegue i controlli che esegue la CI, ad eccezione dell'ambiente Docker e del controllo della mappa Atlas.

## Sicurezza e dati

- **Cosa tocca.** Nella modalità di gioco (non ancora implementata), le chiavi Testnet di un giocatore rimangono sulla propria macchina, nella cartella `.league/` ignorata da Git, e non vengono mai registrate. Il relay di test pratico legge la sua chiave OpenRouter solo dalla variabile d'ambiente `OPENROUTER_API_KEY`, non effettua chiamate di rete senza `--live` e un limite di credito e conserva le trascrizioni nella cartella del gioco che gli viene fornita.
- **Cosa non tocca.** Nessun denaro reale, nessuna Mainnet e niente al di fuori delle cartelle sopra indicate. Non c'è **telemetria**: niente viene inviato da nessuna parte tranne le chiamate necessarie a una modalità (la Testnet, un beacon di casualità pubblico e, per il relay, OpenRouter).
- **Autorizzazioni.** Accesso alla rete Testnet in modalità di gioco. Docker, con la sua porta collegata solo a `127.0.0.1`, per l'ambiente autonomo.
- **Protezioni.** Ogni connessione alla Testnet deve segnalare l'ID di rete della Testnet, la CI fallisce su qualsiasi nome host Mainnet e un secondo meccanismo di sicurezza fallisce su qualsiasi copia che presenta il gioco come fonte di reddito.

Consulta [`SECURITY.md`](SECURITY.md) per il modello di minaccia e per sapere come segnalare una vulnerabilità.

## Documentazione

- [Manuale](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Progetto v2](docs/design-v2.md): le regole, l'economia e le due modalità
- [Test pratico](docs/paper-test.md): i primi test pratici, eseguiti su carta con giocatori AI
- [Ambiente `xrpld` autonomo](docs/standalone-xrpld.md)
- [Ricevute](docs/receipts/): controlli live sulla Testnet. La Testnet viene cancellata di tanto in tanto (la prossima volta il 2026-11-25), quindi i relativi hash delle transazioni smettono di essere risolti in seguito; ciò che hanno registrato e i vettori di test rimangono la prova.

## Supporto

Questo è un prototipo in fase di progettazione attiva. Segnalazioni di problemi e suggerimenti sono benvenuti sul [tracker dei problemi](https://github.com/mcp-tool-shop-org/merchant-league/issues). Non ci sono ancora versioni e non ci sono garanzie di supporto. Consulta il [REGISTRO DELLE MODIFICHE](CHANGELOG.md) per i progressi.

## Licenza

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
