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

# Merchant League

Un gioco di commercio e politica a turni lenti, per quattro-sei giocatori, giocato sulla **Testnet** di XRP Ledger. I giocatori possono essere persone o intelligenze artificiali, allo stesso tavolo e con le stesse regole.

**Stato: prototipo, fase di progettazione.** Il motore delle regole, il generatore di tavoli e la funzione di replay sono stati creati e testati. Il gioco vero e proprio **non è ancora giocabile**: non esiste un client di gioco e non esiste ancora alcuna versione. Solo Testnet: niente denaro reale e nessun percorso verso la Mainnet.

## Il gioco

- **Un anello di mercanti.** Ogni giocatore produce le proprie materie prime e ha bisogno di beni dagli altri giocatori per coprire le spese, gestire le proprie attività e fornire progetti condivisi. Nessuno può prosperare da solo, e nemmeno un piccolo gruppo.
- **Turni simultanei.** Ogni turno è suddiviso in tre fasi. I giocatori parlano e commerciano nel mercato aperto; quindi, ogni giocatore invia segretamente i propri ordini; infine, gli ordini di ogni giocatore vengono risolti contemporaneamente, secondo le stesse regole per tutti.
- **Politica.** I consigli stabiliscono le tariffe, i cartelli possono aprire mercati privati e le cariche ruotano tra i giocatori durante la stagione.
- **Una stagione di 24-32 turni.** Gli eventi imprevisti (blocco delle forniture, esenzione dalle tariffe, aumento della domanda o picco di deterioramento) vengono annunciati pubblicamente un turno prima che si verifichino e l'ultimo turno viene eseguito in modo casuale, in modo che nessuno possa pianificare una mossa finale perfetta.
- **Punteggio.** Alla fine, ogni giocatore calcola il proprio patrimonio netto, a cui si aggiunge una carta obiettivo segreta distribuita all'inizio. Ogni giocatore ha un motivo per continuare a giocare fino all'ultimo turno.
- **Tavoli controllati.** Ogni tavolo viene generato da un seme e controllato prima di essere utilizzato. Il valore iniziale di ogni giocatore deve rientrare nel 3% della media del tavolo e nel 7% della media quando il valore di un giocatore è determinato in base a ciò che aggiunge a ogni gruppo di giocatori.

Il progetto completo è disponibile qui: [`docs/design-v2.md`](docs/design-v2.md).

## Due modalità di gioco

| Modalità | Dove viene eseguito | A chi è destinato |
| --- | --- | --- |
| **Modalità di gioco** | la rete pubblica XRPL Testnet, con circa due turni al giorno | persone, da sole o con giocatori AI |
| **Modalità di addestramento e valutazione** | un nodo `xrpld` autonomo privato in un container Docker locale, che chiude i ledger in millisecondi | giocatori AI: addestramento, benchmark e replay |

Entrambe le modalità condividono lo stesso motore di regole. Un gioco viene riprodotto esattamente a partire dal suo seme e dal suo registro.

## Cosa è stato creato

| Crate | Cosa fa |
| --- | --- |
| `league-core` | Il motore delle regole: le fasi del turno, i contratti, il punteggio, la chiusura della stagione, il replay e i predicati di stato alla base del sistema di suggerimenti previsto |
| `league-gen` | Il generatore di tavoli, con i suoi controlli di equità e i suoi pool di semi |
| `league-dice` | Solo bozze, da ritirare. La casualità del gioco e il suo impegno e rivelazione si trovano in `league-core` |
| `league-ledger` | Il client XRPL Testnet. Rifiuta qualsiasi rete diversa dalla Testnet |
| `league-standalone` | L'ambiente `xrpld` autonomo per la modalità di addestramento, in un container con risorse limitate e configurazione fissa |
| `league-season` | Le date di ripristino annunciate della Testnet e un controllo che individua un ripristino non annunciato |
| `league-relay` | Lo strumento che ha eseguito i test su carta con giocatori AI |
| `league-tui` | Il client terminale. Per ora, solo una schermata introduttiva |

## Compilazione dal codice sorgente

È necessario [Rust](https://rustup.rs/). La toolchain con configurazione fissa (1.100.0) si installa automaticamente da `rust-toolchain.toml` alla prima compilazione.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

Le build e i test CI vengono eseguiti su Linux e l'ambiente di lavoro viene sviluppato su Windows. Altre piattaforme non sono state testate.

Docker è necessario solo per l'ambiente `xrpld` autonomo e i suoi test vengono saltati a meno che non vengano richiesti:

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` esegue i controlli eseguiti dal sistema CI, ad eccezione del sistema Docker e del controllo della mappa Atlas. La copertura, `cargo audit` e la compilazione del sito vengono eseguite solo quando `cargo-llvm-cov`, `cargo-audit` e `npm` sono installati, e lo script indica quali controlli sono stati saltati.

## Sicurezza e dati

- **Cosa influisce.** Nella modalità di gioco (non ancora implementata), le chiavi Testnet di un giocatore rimangono sulla propria macchina, nella cartella `.league/`, che non viene inclusa nel controllo del codice sorgente, e non vengono mai registrate. Il relay dei test su carta legge la sua chiave OpenRouter solo dalla variabile d'ambiente `OPENROUTER_API_KEY` e non effettua alcuna chiamata OpenRouter senza `--live` e un limite di crediti. I suoi posti locali rifiutano qualsiasi chiave API, si connettono solo a un indirizzo di loopback o a un indirizzo VM che si autorizza con `--allow-vm-host` e richiedono un limite di secondi di GPU. Conserva le sue trascrizioni nella cartella del gioco fornita.
- **Cosa non influisce.** Nessun denaro reale, nessuna Mainnet e nulla al di fuori delle cartelle sopra indicate. **Non c'è telemetria**: nulla viene inviato da nessuna parte, tranne le chiamate necessarie a una modalità (la Testnet, un segnale di casualità pubblico e, per il relay, OpenRouter o i server del modello locale che si autorizzano).
- **Autorizzazioni.** Accesso alla rete della Testnet nella modalità di gioco. Docker, con la sua porta collegata solo a `127.0.0.1`, per il sistema autonomo.
- **Misure di sicurezza.** Ogni connessione alla Testnet deve segnalare l'ID di rete della Testnet. Un controllo del sistema CI fallisce se viene rilevato qualsiasi nome host della Mainnet scritto nel repository al di fuori del codice fornito, e un secondo controllo fallisce se viene rilevato del testo che presenta il gioco come fonte di reddito. La revisione copre ciò che un elenco di parole non può vedere.

Consultare [`SECURITY.md`](SECURITY.md) per il modello di minaccia e per sapere come segnalare una vulnerabilità.

## Documentazione

- [Manuale](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Progettazione v2](docs/design-v2.md): le regole, l’economia e le due modalità
- [Test su carta](docs/paper-test.md): le prime sessioni di test, svolte su carta con avversari simulati dall’IA
- [Sistema `xrpld` autonomo](docs/standalone-xrpld.md)
- [Registrazioni](docs/receipts/): verifiche in tempo reale sulla Testnet. La Testnet viene periodicamente resettata (la prossima volta il 25-11-2026), quindi i relativi hash delle transazioni smettono di essere validi; ciò che è stato registrato e i vettori di test rimangono come prova.

## Supporto

Questo è un prototipo in fase di progettazione attiva. Segnalazioni di problemi e idee sono ben accette sul [tracker dei problemi](https://github.com/mcp-tool-shop-org/merchant-league/issues). Non ci sono ancora versioni e non ci sono garanzie di supporto. Consultare il [REGISTRO DELLE MODIFICHE](CHANGELOG.md) per i progressi.

## Licenza

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
