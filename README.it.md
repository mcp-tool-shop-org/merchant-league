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

**Stato: prototipo, in fase di sviluppo attivo.** La **modalità Club** offline in `league-tui` è giocabile partendo dal codice sorgente su un terminale condiviso (da 4 a 6 posti umani e `ClubBot`, dalla lobby fino alla classifica di fine stagione e all'esportazione delle partite; le comunicazioni private sono posticipate dopo `v0.1.0`). La modalità di gioco sulla Testnet pubblica e l'immagine del container non sono ancora state create e **non c'è ancora nessuna versione rilasciata**. Solo Testnet: niente denaro reale e nessun percorso verso la Mainnet.

## Il gioco

- **Un anello di mercanti.** Ogni giocatore produce le proprie materie prime e ha bisogno di beni dagli altri giocatori per coprire le spese, gestire le proprie attività e fornire beni per progetti condivisi. Nessuno può prosperare da solo, e nemmeno un piccolo gruppo.
- **Turni simultanei.** Ogni turno è suddiviso in tre fasi. I giocatori parlano e commerciano nel mercato aperto; quindi, ogni giocatore inserisce segretamente i propri ordini; infine, gli ordini di ogni giocatore vengono elaborati contemporaneamente, secondo le stesse regole per tutti.
- **Politica.** I consigli stabiliscono le tariffe, i cartelli possono aprire mercati privati e le concessioni (charter) ruotano tra i posti durante la stagione.
- **Una stagione di 24-32 turni.** Gli eventi imprevisti (un blocco delle forniture, una sospensione delle tariffe, un aumento della domanda o un picco di deterioramento) vengono annunciati pubblicamente un turno prima che si verifichino, e il turno finale viene estratto a sorte, in modo che nessuno possa pianificare una mossa finale perfetta.
- **Punteggio.** Un giocatore ottiene un punteggio in base al proprio patrimonio netto alla fine della stagione, più una carta obiettivo segreta distribuita all'inizio. Ogni giocatore ha un motivo per continuare a giocare fino all'ultimo turno.
- **Tavoli controllati.** Ogni tavolo viene generato da un seme e controllato prima di essere utilizzato. Il valore iniziale di ogni giocatore deve rientrare nel 3% della media del tavolo e nel 7% della media quando il valore di un giocatore è determinato in base a ciò che aggiunge a ogni gruppo di giocatori.

Il progetto completo è disponibile su [`docs/design-v2.md`](docs/design-v2.md).

## Due modalità di gioco

| Modalità | Dove viene eseguito | A chi è rivolto |
| --- | --- | --- |
| **Modalità di gioco** | la rete pubblica XRPL Testnet, con circa due turni al giorno | persone, da sole o con giocatori controllati dall'IA |
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
| `league-tui` | L'host del terminale per la modalità Club offline: configurazione della lobby, turni hot-seat a quattro zone, sipario della privacy con PIN opzionali, passaggi a tempo, salvataggio/ripresa a metà turno, classifica di fine stagione ed esportazione del replay |

## Gioca una stagione

La modalità Club offline esegue una stagione hot-seat completa di `24..=32` turni sul tuo terminale (`4–6` posti umani o `ClubBot` su una singola macchina):

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configurazione della lobby:** Scegli il numero di posti (`4–6`, `Left`/`Right`), alterna ogni posto tra `Human` e `ClubBot` (`Space`), inserisci un seme `SeedPool::Dev` in chiaro in `0..=9999` (i semi di test riservati e gli altri pool di semi vengono rifiutati per nome), scegli il tempo per turno (`180–300 s`, `210 s` predefinito, oppure `Untimed` con `Space`) e attiva o disattiva i PIN a 4 cifre del sipario, propri del tavolo. Premi `Enter` su **Start Club Mode Table**. Avviare mentre in `--save-dir` esiste un `club-*.seal` non concluso viene rifiutato (`UNFINISHED_SEASON_EXISTS`); dopo l’esportazione, il tavolo successivo riceve il primo ID di tavolo libero senza sovrascrivere i salvataggi precedenti. Le opzioni da riga di comando (`--turn-clock`, `--untimed`, `--pins`) prevalgono sulle scelte in conflitto della lobby e del `.seal` salvato (`league-tui --help` descrive l’ordine di precedenza; passare le cifre del PIN con `--pin` da riga di comando viene rifiutato).
- **Tasti del turno:** `1`/`2`/`3`/`Tab`/`BackTab` passano da una scheda abilitata del compositore all’altra (`4: Diplomatic Desk (Disabled)` resta disabilitata); `F6` sposta il focus tra il registro pubblico, le schede del compositore e il caveau privato; `[`/`]` scorrono i tipi di azione o le sezioni del pacchetto di ordini; `Up`/`Down`/`PageUp`/`PageDown`/`Home` selezionano o scorrono; `Left`/`Right`/`+`/`-`/`Space` regolano i campi; `e` modifica le sostituzioni a testo libero; `n` modifica le note personali del caveau privato; `Enter` invia un’azione, prepara una sezione del pacchetto o esegue la sandbox What-If di `p`; `Del`/`Backspace`/`X` cancellano gli ordini preparati; `c` alterna i nomi delle merci della gilda e i codici a 3 lettere (`[CRED]`, `[TMB]`); `r`/`F2` apre la finestra della Cronaca dello sweep; `Ctrl+l`/`Ctrl+L` blocca il tuo passaggio e apre il sipario della privacy; `q`/`Esc`/`Ctrl+c` escono in modo pulito.
- **Sipario della privacy, tempo morto e PIN:** Ogni passaggio di mano, ogni timeout e ogni ripresa a stagione in corso apre un sipario della privacy pubblico (che cancella sul posto lo schermo alternativo con `CSI 2 J`, mai con `CSI 3 J`, che svuoterebbe la cronologia di scorrimento), scarta l’input in coda e impone un tempo morto di input di `400 ms` sia all’apertura sia al sollevamento (il tempo morto del sollevamento non conta sull’orologio del posto che arriva). Premi `Space` e poi `Enter` per sollevarlo; se i PIN sono attivi, digita il tuo PIN mascherato a 4 cifre (inserito solo dietro il sipario: il 1° turno lo imposta, i passaggi successivi lo verificano, i tentativi sbagliati non producono alcuna vista del posto e 3 errori riportano il sipario allo stato non armato).
- **Orologio:** I turni a tempo dividono ogni budget in `75%` per il sotto-passaggio A e `25%` per il sotto-passaggio B tra tutti gli `H` posti umani (`floor(0.75 * B / H)` s e `floor(0.25 * B / H)` s). Bloccare in anticipo fa avanzare subito; i timeout per inattività nel sotto-passaggio B si risolvono con la dottrina di riserva del Reggente permanente.
- **Salvataggio e ripresa:** Con `--save-dir <DIR>`, l’host salva automaticamente in modo atomico a ogni sipario, al primo impegno del PIN e allo sweep che lascia `TalkAndMarket`. Lo stato pubblico viene salvato in `club-<id>.save.json`; i mandati nascosti, i sali, le prove VRF derivate in anticipo, le note, i pacchetti non impegnati e i PIN restano solo in `club-<id>.seal`. Riprendi quando vuoi con `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (oppure `--resume ./saves`, che seleziona il tavolo del `.seal` attivo): una ripresa a stagione in corso arriva sempre dietro il sipario del posto umano attivo con un nuovo orologio del passaggio (facendo prima avanzare eventuali cursori `ClubBot` precedenti), mentre una ripresa a stagione finita arriva direttamente sulla schermata pubblica di rivelazione `Session Standby` (`AwaitingMandateReveal`) o sulle schede di fine stagione `Completed`, così puoi riprovare l’esportazione del replay. Passare `--resume` insieme a un `--save-dir` diverso viene rifiutato quando quella directory contiene un altro file `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, con qualsiasi ID di tavolo) oppure contiene `club-<id>.save.json`, `.seal` o `.replay.json` dello stesso ID di tavolo (`SEASON_SAVE_EXISTS`, tranne quando si riprende una stagione conclusa il cui `club-<id>.replay.json` regolare, identico byte per byte, è già in quella directory senza un `.save.json` o un `.seal` in conflitto, così che un nuovo tentativo di esportazione dopo una cancellazione fallita del `.seal` di origine vada avanti); una directory che contiene solo una stagione conclusa ed esportata con un altro ID di tavolo viene accettata senza sovrascrivere nulla. Indicare un `--save-dir` mancante o vuoto lo crea se serve durante l’esportazione, e il `.seal` ripreso viene comunque cancellato se l’esportazione riesce. Una ripresa a stagione in corso in un altro `--save-dir` prosegue lì la stagione e lascia al loro posto il `club-<id>.save.json` e il `.seal` della directory originale. Esportare il replay nella stessa sessione cancella anche il `.seal` originale; se invece esci e riprendi più tardi dalla nuova directory, il `.seal` originale resta, contiene ancora i valori privati di quella stagione e fa sì che la lobby della directory originale continui a rifiutare una nuova stagione (`UNFINISHED_SEASON_EXISTS`). Elimina quella copia rimasta solo dopo essere uscito e quando la stagione è salvata nella nuova directory. A parte quella copia rimasta, tieni il `club-<id>.save.json`, il `.seal` e il `.replay.json` di ogni stagione con i nomi e nella directory che `league-tui` ha dato loro; per spostare una stagione altrove, riprendila con `--save-dir <EMPTY_DIR>` invece di rinominare o spostare i file a mano.
- **Fine stagione ed esportazione del replay:** Dopo che la campana di chiusura ha chiuso `TalkAndMarket`, avanza nella fase pubblica di attesa e rivelazione dei mandati (`Enter`/`Space` per ogni posto, nessun mandato mostrato prima) fino alla schermata di fine stagione con quattro schede (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). Nella scheda 4, premi `e` o `Enter` per scrivere e verificare `club-<id>.replay.json` e cancellare il file privato `.seal` (se il replay di un’altra stagione occupa già quel percorso di destinazione, l’esportazione viene rifiutata con `REPLAY_TARGET_EXISTS` senza sovrascriverlo; se l’esportazione non riesce, il `.seal` resta su disco così puoi riprovare).

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

- **Salvataggi e mount:** Con Docker Engine su Linux, l’utente predefinito dell’immagine, `65532:65532`, non può scrivere in una directory dell’host di proprietà del tuo account (o creata da `root` perché mancava un percorso `-v`), quindi creare prima `./saves` e passare `--user "$(id -u):$(id -g)"` lascia i salvataggi di proprietà del tuo utente non root, senza `chmod 777` né root. In alternativa, usa un volume Docker con nome (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, senza `--user`), che eredita la directory `/saves` dell’immagine, di proprietà di `65532:65532`. Su Docker Desktop (macOS e Windows) i permessi dei bind mount sono virtualizzati attraverso la macchina virtuale, quindi il comando PowerShell qui sopra, e lo stesso bind mount su macOS, funzionano senza `--user`, mentre le shell Linux di WSL 2 possono eseguire il comando Linux `--user` qui sopra. Con Docker Engine rootless gli ID utente del container sono mappati diversamente (dentro il container i tuoi file risultano di `root`, e ogni altro ID è mappato su un ID subordinato), quindi il comando `--user` qui sopra non dà al container il tuo account; lì usa il volume con nome. Docker Desktop per Linux esegue il motore in una macchina virtuale e condivide le cartelle con essa con un proprio meccanismo; se lì il bind mount non riesce a salvare, usa anche in quel caso il volume con nome. Non abbiamo testato Docker Engine rootless, Docker Desktop per Linux né macOS.
- **Terminale interattivo richiesto:** passa `-it` per giocare in modo interattivo. L’avvio senza un TTY collegato termina con codice di uscita `1`, il codice `TTY_REQUIRED` e il relativo suggerimento `docker run --rm -it <image>`; `--help` e `--version` terminano con codice di uscita `0` senza un TTY.
- **Cosa contiene l’immagine:** Costruita da `rust:1.100.0-bookworm` fissata per digest (`@sha256:`) su `gcr.io/distroless/cc-debian12:nonroot` fissata per digest (`@sha256:`), con utente predefinito non root `65532:65532` in `WORKDIR /saves`. Su quella base distroless fissata (che non contiene né shell né gestore di pacchetti), la build aggiunge solo `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE` e una directory `/saves` vuota di proprietà di `65532:65532`: nessuna toolchain, nessun file sorgente e nessuna chiamata di rete (`--network=none`).

## Sicurezza e dati

- **Cosa viene toccato.** In modalità Club offline (`league-tui`), `--save-dir` scrive `club-<id>.save.json` pubblici, `club-<id>.seal` privati hot-seat (eliminati dopo l'esportazione riuscita della partita), `club-<id>.replay.json` verificati e `--log-file` diagnostiche di metadati opzionali (senza registrare PIN, note del caveau, mandati, ordini preparati o salt di commit, e oscurando le credenziali API/XRPL e le chiavi JSON sigillate in tutti e quattro i livelli di log) sulla tua macchina, senza chiamate di rete. In modalità Gioco (non ancora implementata), le chiavi Testnet di un giocatore rimangono sulla sua macchina, nella cartella `.league/` ignorata da Git, e non vengono mai registrate. Il relay di test su carta legge la sua chiave OpenRouter solo dalla variabile d'ambiente `OPENROUTER_API_KEY` e non effettua chiamate OpenRouter senza `--live` e un limite di crediti. I suoi posti locali rifiutano qualsiasi chiave API, raggiungono solo un indirizzo loopback o un indirizzo VM che consenti con `--allow-vm-host` e richiedono un limite di secondi di GPU. Mantiene le sue trascrizioni nella cartella del gioco che gli fornisci.
- **Cosa non viene toccato.** Nessun denaro reale, nessuna Mainnet e nulla al di fuori delle cartelle sopra indicate. Non c'è **telemetria**: nulla viene inviato da nessuna parte tranne le chiamate di cui una modalità ha bisogno (la Testnet, un beacon di casualità pubblico e, per il relay, OpenRouter o i server del modello locali che consenti).
- **Permessi.** Accesso alla rete alla Testnet in modalità Gioco. Docker con `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (o volume denominato `-v merchant-league-saves:/saves`) quando si esegue il container della modalità Club locale (che tocca solo `/saves`), o Docker con la sua porta collegata a `127.0.0.1` solo per il sistema autonomo.
- **Protezioni.** Ogni connessione alla Testnet deve segnalare l'ID di rete della Testnet. Un controllo CI fallisce su qualsiasi nome host di Mainnet scritto nel repository al di fuori del codice di terze parti incluso (vendored), e un secondo controllo fallisce su testi che presentano il gioco come fonte di reddito. La revisione copre ciò che un elenco di parole non può vedere.

Consulta [`SECURITY.md`](SECURITY.md) per il modello di minaccia e per sapere come segnalare una vulnerabilità.

## Documentazione

- [Manuale](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Progettazione v2](docs/design-v2.md): le regole, l’economia e le due modalità
- [Test su carta](docs/paper-test.md): le prime sessioni di test, svolte su carta con avversari simulati dall’IA
- [Sistema `xrpld` autonomo](docs/standalone-xrpld.md)
- [Registrazioni](docs/receipts/): verifiche in tempo reale sulla Testnet. La Testnet viene periodicamente resettata (la prossima volta il 25-11-2026), quindi i relativi hash delle transazioni non vengono più risolti; ciò che è stato registrato e i vettori di test rimangono come prova.

## Supporto

Questo è un prototipo in fase di progettazione attiva. Segnalazioni di problemi e idee sono benvenute sul [tracker dei problemi](https://github.com/mcp-tool-shop-org/merchant-league/issues). Non ci sono ancora versioni e non ci sono garanzie di supporto. Consulta il [REGISTRO DELLE MODIFICHE](CHANGELOG.md) per i progressi.

## Licenza

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
