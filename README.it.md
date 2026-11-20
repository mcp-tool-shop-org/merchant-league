<p align="center">
  <a href="README.md">English</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh.md">中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

Un gioco di commercio e politica a turni lenti, per quattro-sei giocatori, ambientato sulla **Testnet** di XRP Ledger. I giocatori possono essere persone o avversari controllati dall'IA, seduti allo stesso tavolo e soggetti alle stesse regole.

**Stato: prototipo, in fase di sviluppo attivo.** La modalità **Club Mode** offline in `league-tui` è giocabile partendo dal codice sorgente su un terminale condiviso (4-6 giocatori umani e `ClubBot` giocatori controllati dall'IA, tramite la lobby, fino alla classifica di fine stagione e all'esportazione delle partite; le comunicazioni private sono posticipate dopo `v0.1.0`). La modalità di gioco Public-Testnet e l'immagine del container non sono ancora stati creati e non è prevista alcuna uscita al momento. Solo Testnet: niente denaro reale e nessun percorso verso la Mainnet.

## Il gioco

- **Un anello di mercanti.** Ogni giocatore produce le proprie materie prime e ha bisogno di beni dagli altri giocatori per coprire le spese, gestire le proprie attività e fornire beni per progetti condivisi. Nessuno può prosperare da solo, e nemmeno un piccolo gruppo.
- **Turni simultanei.** Ogni turno è suddiviso in tre fasi. I giocatori parlano e commerciano nel mercato aperto; quindi, ogni giocatore inserisce segretamente i propri ordini; infine, gli ordini di ogni giocatore vengono elaborati contemporaneamente, secondo le stesse regole per tutti.
- **Politica.** I consigli stabiliscono le tariffe, i cartelli possono aprire mercati privati e le cariche ruotano tra i giocatori durante la stagione.
- **Una stagione di 24-32 turni.** Gli eventi imprevisti (un blocco delle forniture, una sospensione delle tariffe, un aumento della domanda o un picco di deterioramento) vengono annunciati pubblicamente un turno prima che si verifichino, e l'ultimo turno viene eseguito in modo casuale, in modo che nessuno possa pianificare una mossa finale perfetta.
- **Punteggio.** Un giocatore ottiene un punteggio in base al proprio patrimonio netto alla fine della stagione, più una carta obiettivo segreta distribuita all'inizio. Ogni giocatore ha un motivo per continuare a giocare fino all'ultimo turno.
- **Tavoli controllati.** Ogni tavolo viene generato da un seme e controllato prima di essere utilizzato. Il valore iniziale di ogni giocatore deve rientrare nel 3% della media del tavolo e nel 7% della media quando il valore di un giocatore è determinato in base a ciò che aggiunge a ogni gruppo di giocatori.

Il progetto completo è disponibile su [`docs/design-v2.md`](docs/design-v2.md).

## Due modalità di gioco

| Modalità | Dove viene eseguito | A chi è rivolto |
| --- | --- | --- |
| **Game mode** | la rete pubblica XRPL Testnet, con circa due turni al giorno | persone, da sole o con giocatori controllati dall'IA |
| **Modalità di addestramento e valutazione** | un nodo `xrpld` autonomo privato in un container Docker locale, che chiude i ledger in millisecondi | Giocatori controllati dall'IA: addestramento, benchmark e replay |

Entrambe le modalità condividono lo stesso motore di gioco. Una partita viene riprodotta esattamente a partire dal suo seme e dal suo registro.

## Cosa viene creato

| Pacchetto | A cosa serve |
| --- | --- |
| `league-core` | Il motore di gioco: le fasi del turno, i contratti, il punteggio, la fine della stagione, il replay e i predicati di stato alla base del sistema di suggerimenti previsto |
| `league-gen` | Il generatore di tavoli, con i suoi controlli di equità e i suoi pool di semi |
| `league-ledger` | Il client XRPL Testnet. Rifiuta qualsiasi rete diversa dalla Testnet |
| `league-standalone` | L'ambiente `xrpld` autonomo per la modalità di addestramento, in un container con risorse limitate e configurazione fissa |
| `league-season` | Le date di ripristino annunciate della Testnet e un controllo che individua un ripristino non annunciato |
| `league-relay` | Lo strumento che ha eseguito i test su carta con giocatori AI |
| `league-tui` | L'host del terminale per la modalità Club offline: configurazione della lobby, turni a quattro giocatori, Privacy Curtain con PIN opzionali, passaggi a tempo, salvataggio/ripresa a metà turno, classifica di fine stagione ed esportazione del replay |

## Gioca una stagione

La modalità Club offline esegue una stagione completa di `24..=32` turni sul tuo terminale (`4–6` giocatori umani o `ClubBot` giocatori su una singola macchina):

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configurazione della lobby:** scegli il numero di posti (`4–6`, `Left`/`Right`), attiva o disattiva ogni posto alternando tra `Human` e `ClubBot` (`Space`), inserisci un codice univoco `SeedPool::Dev` nel campo `0..=9999` (i codici di test riservati e altri codici predefiniti vengono rifiutati), scegli un limite di turni (`180–300 s`, `210 s` è l’impostazione predefinita, oppure `Untimed` con `Space`) e attiva i PIN a 4 cifre specifici per ogni tavolo. Premi `Enter` su **Avvia tavolo in modalità Club**. L’avvio di una partita mentre è in corso una partita incompleta `club-*.seal` nel tavolo `--save-dir` viene rifiutato (`UNFINISHED_SEASON_EXISTS`); una volta esportata, il tavolo successivo riceve il successivo ID tavolo disponibile senza sovrascrivere i salvataggi precedenti. Le opzioni da riga di comando (`--turn-clock`, `--untimed`, `--pins`) sovrascrivono le impostazioni della lobby e i salvataggi `.seal` in conflitto (`league-tui --help` ha la precedenza; l’inserimento dei PIN tramite `--pin` nella riga di comando viene rifiutato).
- **Tasti di scelta rapida:** `1`/`2`/`3`/`Tab`/`BackTab` attiva/disattiva le schede del compositore (`4: Diplomatic Desk (Disabled)` rimane disattivata); `F6` sposta il focus tra il registro pubblico, le schede del compositore e il caveau privato; `[`/`]` scorre i tipi di azione o le sezioni del pacchetto; `Up`/`Down`/`PageUp`/`PageDown`/`Home` seleziona o scorre; `Left`/`Right`/`+`/`-`/`Space` regola i campi; `e` modifica le sostituzioni di testo libero; `n` modifica le note personali del caveau privato; `Enter` invia un’azione, prepara una sezione del pacchetto o esegue il `p` ambiente di test “What-If”; `Del`/`Backspace`/`X` cancella gli ordini in sospeso; `c` attiva/disattiva la visualizzazione dei nomi completi della gilda rispetto ai codici di 3 lettere (`[CRED]`, `[TMB]`); `r`/`F2` apre la finestra modale della Cronaca Sweep; `Ctrl+l`/`Ctrl+L` blocca il pass e apre la tenda della privacy; `q`/`Esc`/`Ctrl+c` esce in modo pulito.
- **Schermo di protezione della privacy, tempo di inattività e PIN:** ogni passaggio di consegne, timeout e ripresa durante la stagione apre uno schermo di protezione della privacy (cancellando lo schermo alternativo con `CSI 2 J` e non attivando mai la funzione di cancellazione dello storico `CSI 3 J`), svuota gli input in coda e impone un tempo di inattività `400 ms` sia all’apertura che al rilascio (il tempo di inattività al rilascio non viene conteggiato nel tempo di utilizzo del posto). Premere `Space`, quindi `Enter` per rilasciare; se i PIN sono abilitati, digitare il PIN a 4 cifre mascherato (da inserire solo dietro lo schermo: alla prima attivazione viene impostato, nei passaggi successivi viene verificato; tentativi errati comportano l’annullamento delle osservazioni del posto e 3 errori reimpostano lo schermo allo stato iniziale).
- **Tempo:** le rotazioni a tempo suddividono ogni budget `75%` Sub-Pass A / `25%` Sub-Pass B tra tutti i `H` posti a sedere occupati (`floor(0.75 * B / H)` s e `floor(0.25 * B / H)` s). L’attivazione anticipata viene bloccata immediatamente; i timeout di inattività di Sub-Pass B vengono risolti tramite la procedura di riserva del Standing Regent.
- **Salva e riprendi:** con `--save-dir <DIR>`, l'host salva automaticamente in modo atomico ogni volta che si raggiunge una fase, si effettua il primo commit del PIN e si esegue una scansione, lasciando `TalkAndMarket`. Lo stato pubblico viene salvato in `club-<id>.save.json`; i dati sensibili, i valori di hash, le prove VRF pre-calcolate, le note, i pacchetti non ancora salvati e i PIN rimangono esclusivamente in `club-<id>.seal`. È possibile riprendere in qualsiasi momento con `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (o `--resume ./saves`, selezionando la tabella `.seal` attiva): quando si riprende durante la stagione, si riparte sempre dalla fase successiva a quella in cui si trova il giocatore, con un nuovo contatore di tempo (prima vengono avanzati eventuali cursori `ClubBot`), mentre quando si riprende dopo la stagione, si riparte direttamente dalla schermata di visualizzazione pubblica `Session Standby` (`AwaitingMandateReveal`) o dalle schede `Completed` "Fine stagione", in modo da poter riprovare l'esportazione. L'invio di `--resume` insieme a un `--save-dir` diverso viene rifiutato se tale directory contiene un altro file `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, qualsiasi ID di tabella) o contiene altrimenti `club-<id>.save.json`, `.seal` o `.replay.json` per lo stesso ID di tabella (`SEASON_SAVE_EXISTS`, tranne quando si riprende una stagione completata, la cui copia esatta `club-<id>.replay.json` si trova già in quella directory senza conflitti `.save.json` o `.seal`, in modo che un tentativo di esportazione dopo un fallimento nell'eliminazione della sorgente `.seal` vada a buon fine); una directory che contiene solo una stagione completata ed esportata con un ID di tabella diverso viene accettata senza sovrascrivere nulla. Se si specifica una directory `--save-dir` mancante o vuota, questa viene creata, se necessario, durante l'esportazione e la directory `.seal` ripresa viene comunque eliminata in caso di esportazione riuscita. La ripresa di una stagione in un'altra directory `--save-dir` continua la stagione in quella directory e lascia intatti i file `club-<id>.save.json` e `.seal` della directory originale. L'esportazione della registrazione in quella stessa sessione elimina anche il file `.seal` originale; se invece si esce e successivamente si riprende dalla nuova directory, il file `.seal` originale rimane, contenendo ancora i valori privati di quella stagione e mantenendo la lobby della directory originale, che rifiuta una nuova stagione (`UNFINISHED_SEASON_EXISTS`). Eliminare tale copia residua solo dopo aver chiuso l'applicazione e aver salvato la stagione nella nuova directory. A parte questa copia residua, conservare i file `club-<id>.save.json`, `.seal` e `.replay.json` di ciascuna stagione con i nomi e nella directory `league-tui` in cui sono stati salvati; per spostare una stagione in un'altra posizione, riprenderla con `--save-dir <EMPTY_DIR>` anziché rinominare o spostare manualmente i file.
- **Fine stagione ed esportazione della registrazione:** Dopo la chiusura del mercato `TalkAndMarket`, procedi attraverso la fase pubblica di rivelazione dei mandati (`Enter`/`Space` per posto, mostrando inizialmente zero mandati) per arrivare alla schermata finale a quattro schede (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). Nella scheda 4, premi `e` o `Enter` per scrivere e verificare `club-<id>.replay.json` ed eliminare il file privato `.seal` (se la registrazione di un’altra stagione occupa già quel percorso di destinazione, l’esportazione verrà rifiutata `REPLAY_TARGET_EXISTS` senza sovrascriverla; in caso di errore nell’esportazione, `.seal` rimarrà sul disco in modo che tu possa riprovare).

## Compila dal codice sorgente

Hai bisogno di [Rust](https://rustup.rs/). La toolchain fissata (1.100.0) si installa automaticamente da `rust-toolchain.toml` alla prima compilazione.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

I build e i test CI vengono eseguiti su Linux e l'ambiente di lavoro viene sviluppato su Windows. Altre piattaforme non sono state testate.

Docker è necessario per compilare ed eseguire l'immagine del container Club Mode locale o per eseguire il sistema di test autonomo `xrpld` (i test del sistema vengono saltati a meno che tu non li richieda esplicitamente):

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` esegue i controlli eseguiti dal CI, ad eccezione del sistema di test Docker autonomo e del controllo della mappa Atlas. La copertura, `cargo audit`, la compilazione del sito e il test "smoke" del container Club Mode vengono eseguiti solo quando `cargo-llvm-cov`, `cargo-audit`, `npm` e Linux `docker` (con POSIX `python3` `pty`/`fcntl`/`termios`) sono disponibili e lo script indica eventuali controlli saltati.

## Esegui Club Mode in Docker (compilazione locale)

L'immagine del container Club Mode (`league-tui`) non è ancora pubblicata in nessun registro di container; il processo multi-stadio `Dockerfile` viene compilato localmente e verificato dal test "smoke" CI offline (`scripts/smoke-container.sh`, nessuna pubblicazione nel registro).

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

Su Windows con Docker Desktop, in PowerShell:

```powershell
docker build -t merchant-league:local .
mkdir saves
docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local
```

- **Salvataggio e punti di mount:** su Linux Docker Engine, l'utente predefinito dell'immagine `65532:65532` non può scrivere in una directory dell'host di proprietà del tuo account (o `root`, creata da un percorso `-v` mancante), quindi la creazione preventiva di `./saves` e il passaggio di `--user "$(id -u):$(id -g)"` mantengono i salvataggi di proprietà del tuo utente non-root senza `chmod 777` o root. In alternativa, utilizza un volume Docker denominato (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, senza `--user`), che eredita `65532:65532` di proprietà dell'immagine `/saves`. Su Docker Desktop (macOS e Windows), i permessi di bind-mount sono virtualizzati all'interno della VM, quindi il comando PowerShell sopra e lo stesso bind mount su macOS funzionano senza `--user`, mentre gli ambienti Linux WSL 2 possono eseguire il comando Linux `--user` sopra. Con Docker Engine senza privilegi di root, gli ID utente del container vengono mappati in modo diverso (all'interno del container, i tuoi file appaiono come `root` e qualsiasi altro ID viene mappato a un ID subordinato), quindi il comando `--user` sopra non fornisce al container il tuo account; utilizza il volume denominato lì. Docker Desktop per Linux esegue l'engine in una VM e condivide le cartelle con esso tramite il suo meccanismo; se il bind mount non può salvare lì, utilizza anche il volume denominato. Docker Engine senza privilegi di root, Docker Desktop per Linux e macOS non sono stati testati da noi.
- **Terminal interattivo richiesto:** passa `-it` per giocare in modo interattivo. L'avvio senza un TTY collegato esce da `1` con il codice `TTY_REQUIRED` e il suo suggerimento `docker run --rm -it <image>`; `--help` e `--version` escono da `0` senza un TTY.
- **Cosa contiene l'immagine:** creata da `rust:1.100.0-bookworm` con hash (`@sha256:`) su `gcr.io/distroless/cc-debian12:nonroot` con hash (`@sha256:`), con impostazione predefinita su utente non-root `65532:65532` in `WORKDIR /saves`. Oltre a questa base distroless con hash (che non contiene shell né gestore di pacchetti), la build aggiunge solo `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE` e una directory `/saves` vuota di proprietà di `65532:65532`: nessun toolchain, nessun file di origine e zero chiamate di rete (`--network=none`).

## Sicurezza e dati

- **Cosa viene toccato.** In modalità Club offline (`league-tui`), `--save-dir` scrive `club-<id>.save.json` pubblici, `club-<id>.seal` privati per giocatore singolo (eliminati dopo l'esportazione riuscita della partita), `club-<id>.replay.json` verificati e `--log-file` diagnostiche di metadati opzionali (senza registrare PIN, note della Vault, mandati, ordini in sospeso o salt di commit e senza eliminare le credenziali API/XRPL e le chiavi JSON sigillate in tutti e quattro i livelli di log) sulla tua macchina, senza chiamate di rete. In modalità Gioco (non ancora implementata), le chiavi Testnet di un giocatore rimangono sulla sua macchina, nella cartella `.league/` ignorata da Git, e non vengono mai registrate. Il relay di test su carta legge la sua chiave OpenRouter solo dalla variabile d'ambiente `OPENROUTER_API_KEY` e non effettua chiamate OpenRouter senza `--live` e un limite di crediti. I suoi posti locali rifiutano qualsiasi chiave API, raggiungono solo un indirizzo loopback o un indirizzo VM che consenti con `--allow-vm-host` e richiedono un limite di secondi di GPU. Mantiene le sue trascrizioni nella cartella del gioco che gli fornisci.
- **Cosa non viene toccato.** Nessun denaro reale, nessuna Mainnet e nulla al di fuori delle cartelle sopra indicate. Non c'è **telemetria**: nulla viene inviato da nessuna parte tranne le chiamate di cui una modalità ha bisogno (la Testnet, un beacon di casualità pubblico e, per il relay, OpenRouter o i server del modello locali che consenti).
- **Permessi.** Accesso alla rete alla Testnet in modalità Gioco. Docker con `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (o volume denominato `-v merchant-league-saves:/saves`) quando si esegue il container della modalità Club locale (che tocca solo `/saves`), o Docker con la sua porta collegata a `127.0.0.1` solo per il sistema autonomo.
- **Protezioni.** Ogni connessione alla Testnet deve segnalare l'ID di rete della Testnet. Un controllo CI fallisce se viene scritto qualsiasi nome host di Mainnet nel repository al di fuori del codice fornito e un secondo controllo fallisce se viene copiato del testo che presenta il gioco come fonte di reddito. La revisione copre ciò che un elenco di parole non può vedere.

Consulta [`SECURITY.md`](SECURITY.md) per il modello di minaccia e per sapere come segnalare una vulnerabilità.

## Documentazione

- [Manuale](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Progettazione v2](docs/design-v2.md): le regole, l’economia e le due modalità
- [Test su carta](docs/paper-test.md): le prime sessioni di test, svolte su carta con avversari simulati dall’IA
- [Sistema `xrpld` autonomo](docs/standalone-xrpld.md)
- [Registrazioni](docs/receipts/): verifiche in tempo reale sulla Testnet. La Testnet viene periodicamente resettata (la prossima volta il 25-11-2026), quindi i relativi hash delle transazioni smettono di essere validi; ciò che è stato registrato e i vettori di test rimangono come prova.

## Supporto

Questo è un prototipo in fase di progettazione attiva. Segnalazioni di problemi e idee sono benvenute sul [tracker dei problemi](https://github.com/mcp-tool-shop-org/merchant-league/issues). Non ci sono ancora versioni e non ci sono garanzie di supporto. Consulta il [REGISTRO DELLE MODIFICHE](CHANGELOG.md) per i progressi.

## Licenza

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
