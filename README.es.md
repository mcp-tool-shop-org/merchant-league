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

Un juego de estrategia y política con turnos lentos para cuatro a seis jugadores, que se juega en la **Testnet** de XRP Ledger. Los jugadores pueden ser personas o jugadores de IA, en la misma mesa y bajo las mismas reglas.

**Estado: prototipo, en fase activa de diseño.** El modo **Club** sin conexión en `league-tui` se puede jugar desde el código fuente en un terminal compartido (4 a 6 jugadores humanos y `ClubBot` jugadores de IA, desde el lobby hasta la tabla de clasificación y la exportación de repeticiones al final de la temporada; los mensajes privados se posponen hasta después de `v0.1.0`). El modo de juego público en Testnet y la imagen del contenedor aún no están desarrollados, y **no hay versión disponible**. Solo Testnet: no se utiliza dinero real y no hay posibilidad de pasar a Mainnet.

## El juego

- **Un círculo de comerciantes.** Cada jugador produce sus propios bienes y necesita bienes de los demás jugadores para cubrir sus gastos, mantener sus talleres y entregar productos a proyectos compartidos. Nadie puede prosperar solo, ni tampoco un pequeño grupo.
- **Turnos simultáneos.** Cada turno tiene tres fases. Los jugadores hablan y negocian en el mercado abierto; luego, cada jugador realiza sus pedidos en secreto; luego, los pedidos de cada jugador se resuelven a la vez, según las mismas reglas para todos.
- **Política.** Los consejos establecen los aranceles, los cárteles pueden abrir mercados privados y las cartas rotan entre los jugadores a lo largo de la temporada.
- **Una temporada de 24 a 32 turnos.** Los eventos inesperados (congelación de suministros, exención de aranceles, aumento de la demanda o aumento de las pérdidas) se anuncian públicamente un turno antes de que ocurran, y el último turno se juega al azar, para que nadie pueda planificar un movimiento final perfecto.
- **Puntuación.** Un jugador obtiene su valor neto al final, más una carta de objetivo oculta que se reparte al principio. Cada jugador tiene una razón para seguir jugando hasta el último turno.
- **Tablas verificadas.** Cada tabla se genera a partir de una semilla y se verifica antes de que se reparta. El valor inicial de cada jugador debe estar dentro del 3% del promedio de la tabla, y dentro del 7% del promedio cuando se valora un jugador por lo que aporta a cada grupo de jugadores.

El diseño completo está en [`docs/design-v2.md`](docs/design-v2.md).

## Dos formas de jugar

| Modo | Dónde se ejecuta | Para quién es |
| --- | --- | --- |
| **Game mode** | la red de pruebas pública XRPL, a un ritmo de aproximadamente dos turnos al día | personas, solas o con jugadores de IA |
| **Modo de entrenamiento y evaluación** | un nodo `xrpld` independiente privado en un contenedor Docker local, que cierra los ledgers en milisegundos | Jugadores de IA: entrenamiento, pruebas de rendimiento y repeticiones |

Ambos modos comparten un motor de reglas. Un juego se reproduce exactamente a partir de su semilla y su registro.

## Qué se ha construido

| Crate | Qué hace |
| --- | --- |
| `league-core` | El motor de reglas: las fases del turno, los contratos, la puntuación, el final de la temporada, la repetición y los predicados de estado que sustentan el sistema de pistas planificado |
| `league-gen` | El generador de tablas, con sus comprobaciones de equidad y sus conjuntos de semillas |
| `league-ledger` | El cliente de la red de pruebas XRPL. Rechaza cualquier red que no sea la red de pruebas |
| `league-standalone` | El entorno `xrpld` independiente para el modo de entrenamiento, en un contenedor con recursos limitados y fijados |
| `league-season` | Las fechas de reinicio anunciadas de la Testnet y una comprobación que detecta un reinicio no anunciado |
| `league-relay` | La herramienta que se utilizó para realizar las pruebas de juego con jugadores de IA |
| `league-tui` | El host del terminal del modo Club sin conexión: configuración del lobby, turnos de cuatro jugadores, cortina de privacidad con PIN opcionales, pases de tiempo limitado, guardado/reanudación a mitad del turno, tabla de clasificación al final de la temporada y exportación de repeticiones |

## Juega una temporada

El modo **Club** sin conexión ejecuta una temporada completa de `24..=32` turnos en tu terminal (`4–6` jugadores humanos o `ClubBot` jugadores en una sola máquina):

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configuración del lobby:** Elige el número de asientos (`4–6`, `Left`/`Right`), alterna cada asiento entre `Human` y `ClubBot` (`Space`), introduce una semilla `SeedPool::Dev` en claro dentro de `0..=9999` (las semillas de prueba reservadas y los demás grupos se rechazan por nombre), elige el tiempo por turno (`180–300 s`, `210 s` por defecto, o `Untimed` con `Space`) y activa o desactiva los PIN de Cortina de 4 dígitos propios de la mesa. Pulsa `Enter` en **Start Club Mode Table**. Empezar mientras exista un `club-*.seal` sin terminar en `--save-dir` se rechaza (`UNFINISHED_SEASON_EXISTS`); una vez exportada, la mesa siguiente recibe el siguiente ID de mesa libre sin sobrescribir las partidas guardadas anteriores. Las opciones de línea de comandos (`--turn-clock`, `--untimed`, `--pins`) prevalecen sobre las elecciones del lobby y del `.seal` guardado que entren en conflicto (`league-tui --help` documenta la precedencia; pasar los dígitos del PIN con `--pin` en la línea de comandos se rechaza).
- **Teclas del turno:** `1`/`2`/`3`/`Tab`/`BackTab` cambian entre las pestañas del compositor habilitadas (`4: Diplomatic Desk (Disabled)` sigue deshabilitada); `F6` mueve el foco entre el Libro público, las pestañas del compositor y la Bóveda privada; `[`/`]` recorren los tipos de acción o las secciones del paquete; `Up`/`Down`/`PageUp`/`PageDown`/`Home` seleccionan o desplazan; `Left`/`Right`/`+`/`-`/`Space` ajustan campos; `e` edita sustituciones de texto libre; `n` edita las notas personales de la Bóveda privada; `Enter` envía una acción, prepara una sección del paquete o ejecuta el Sandbox What-If de `p`; `Del`/`Backspace`/`X` borran las órdenes preparadas; `c` alterna entre los nombres de bienes del gremio y los códigos de 3 letras (`[CRED]`, `[TMB]`); `r`/`F2` abre la ventana de la Crónica del barrido; `Ctrl+l`/`Ctrl+L` bloquea tu pase y abre la Cortina de privacidad; `q`/`Esc`/`Ctrl+c` salen limpiamente.
- **Cortina de privacidad, tiempo muerto y PIN:** Cada traspaso, tiempo agotado y reanudación a mitad de temporada abre una Cortina de privacidad pública (borra la pantalla alternativa en su sitio con `CSI 2 J` y nunca con `CSI 3 J`, que purga el historial de desplazamiento), descarta la entrada en cola y aplica un tiempo muerto de entrada de `400 ms` tanto al abrirla como al levantarla (el tiempo muerto al levantarla no cuenta en el reloj del asiento que llega). Pulsa `Space` y luego `Enter` para levantarla; si los PIN están activados, escribe tu PIN de 4 dígitos enmascarado (solo se introduce tras la Cortina: el 1.er turno lo fija, los traspasos siguientes lo verifican, los intentos fallidos no generan ninguna vista de asiento y 3 fallos devuelven la Cortina a su estado sin armar).
- **Reloj:** Los turnos con tiempo reparten cada presupuesto `75%` para el Subpase A / `25%` para el Subpase B entre los `H` asientos humanos (`floor(0.75 * B / H)` s y `floor(0.25 * B / H)` s). Bloquear antes avanza de inmediato; los tiempos agotados por inactividad en el Subpase B se resuelven con la doctrina de respaldo del Regente permanente.
- **Guardar y reanudar:** Con `--save-dir <DIR>`, el anfitrión guarda automáticamente de forma atómica en cada Cortina, en el primer compromiso de PIN y en el barrido que sale de `TalkAndMarket`. El estado público se guarda en `club-<id>.save.json`; los mandatos ocultos, las sales, las pruebas VRF derivadas de antemano, las notas, los paquetes sin comprometer y los PIN se quedan únicamente en `club-<id>.seal`. Reanuda cuando quieras con `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (o `--resume ./saves`, que selecciona la mesa del `.seal` activo): las reanudaciones a mitad de temporada siempre llegan tras la Cortina del asiento humano activo con un reloj de pase nuevo (avanzando primero los cursores de `ClubBot` que vayan delante), mientras que las reanudaciones después de la temporada llegan directamente a la pantalla pública de revelación `Session Standby` (`AwaitingMandateReveal`) o a las pestañas de fin de temporada `Completed`, para que puedas reintentar la exportación de la repetición. Pasar `--resume` junto con un `--save-dir` distinto se rechaza cuando ese directorio contiene otro archivo `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, con cualquier ID de mesa) o contiene `club-<id>.save.json`, `.seal` o `.replay.json` del mismo ID de mesa (`SEASON_SAVE_EXISTS`, salvo al reanudar una temporada terminada cuyo propio `club-<id>.replay.json` normal, idéntico byte a byte, ya está en ese directorio sin un `.save.json` o `.seal` en conflicto, de modo que un reintento de exportación tras un borrado fallido del `.seal` de origen sigue adelante); un directorio que solo contiene una temporada terminada y exportada con otro ID de mesa se acepta sin sobrescribir nada. Indicar un `--save-dir` inexistente o vacío lo crea si hace falta al exportar y sigue borrando el `.seal` reanudado si la exportación tiene éxito. Una reanudación a mitad de temporada en otro `--save-dir` continúa la temporada allí y deja en su sitio el `club-<id>.save.json` y el `.seal` del directorio original. Exportar la repetición en esa misma sesión también borra el `.seal` original; si en cambio sales y más tarde reanudas desde el directorio nuevo, el `.seal` original se queda, todavía con los valores privados de esa temporada, y hace que el lobby del directorio original siga rechazando una temporada nueva (`UNFINISHED_SEASON_EXISTS`). Borra esa copia sobrante solo después de haber salido y de que la temporada se haya guardado en el directorio nuevo. Aparte de esa copia sobrante, mantén el `club-<id>.save.json`, el `.seal` y el `.replay.json` de cada temporada con los nombres y en el directorio que les dio `league-tui`; para mover una temporada a otro sitio, reanúdala con `--save-dir <EMPTY_DIR>` en lugar de renombrar o mover los archivos a mano.
- **Fin de temporada y exportación de la repetición:** Después de que la Campana de cierre cierre `TalkAndMarket`, avanza por la etapa pública de espera con revelación de mandatos (`Enter`/`Space` por asiento, sin mostrar ningún mandato antes) para pasar a la pantalla de fin de temporada con cuatro pestañas (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). En la pestaña 4, pulsa `e` o `Enter` para escribir y verificar `club-<id>.replay.json` y borrar el archivo privado `.seal` (si la repetición de otra temporada ya ocupa esa ruta de destino, la exportación se rechaza con `REPLAY_TARGET_EXISTS` sin sobrescribirla; si la exportación falla, el `.seal` se queda en disco para que puedas reintentarlo).

## Construir desde la fuente

Necesita [Rust](https://rustup.rs/). La cadena de herramientas fijada (1.100.0) se instala automáticamente desde `rust-toolchain.toml` en la primera compilación.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

Las compilaciones de CI se ejecutan y se prueban en Linux, y el espacio de trabajo se desarrolla en Windows. Otras plataformas no se han probado.

Se necesita Docker para compilar y ejecutar la imagen del contenedor del modo Club local o para ejecutar el conjunto de pruebas independiente `xrpld` (las pruebas del conjunto se omiten a menos que las solicite):

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` ejecuta las comprobaciones que ejecuta el CI, excepto el conjunto de pruebas independiente de Docker y la comprobación del mapa de Atlas. La cobertura, `cargo audit`, la compilación del sitio y la puerta de enlace de prueba del contenedor del modo Club retenido se ejecutan solo cuando `cargo-llvm-cov`, `cargo-audit`, `npm` y Linux `docker` (con POSIX `python3` `pty`/`fcntl`/`termios`) están disponibles, y el script enumera cualquier comprobación que haya omitido.

## Ejecutar el modo Club en Docker (compilación local)

La imagen del contenedor del modo Club (`league-tui`) aún no se ha publicado en ningún registro de contenedores; el `Dockerfile` de varias etapas se compila localmente y se verifica mediante la puerta de enlace de prueba fuera de línea del CI (`scripts/smoke-container.sh`, sin inserción en el registro).

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

En Windows con Docker Desktop, en PowerShell:

```powershell
docker build -t merchant-league:local .
mkdir saves
docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local
```

- **Guardado y puntos de montaje:** En Linux Docker Engine, el usuario predeterminado de la imagen `65532:65532` no puede escribir en un directorio del host que pertenece a su cuenta (o `root`, creado por una ruta `-v` inexistente); por lo tanto, crear `./saves` de antemano y pasar `--user "$(id -u):$(id -g)"` permite que los archivos guardados pertenezcan a su usuario que no es root, sin `chmod 777` ni permisos de root. Alternativamente, utilice un volumen con nombre de Docker (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, sin `--user`), que hereda el `65532:65532` del usuario propietario de la imagen `/saves`. En Docker Desktop (macOS y Windows), los permisos de punto de montaje se virtualizan en toda la máquina virtual, por lo que el comando PowerShell anterior y el mismo punto de montaje en macOS funcionan sin `--user`, mientras que los shells de Linux WSL 2 pueden ejecutar el comando Linux `--user` anterior. Con Docker Engine sin root, los ID de usuario del contenedor se asignan de forma diferente (dentro del contenedor, sus propios archivos aparecen como de `root`, y cualquier otro ID se asigna a un ID subordinado); por lo tanto, el comando `--user` anterior no le otorga al contenedor su propia cuenta; utilice el volumen con nombre allí. Docker Desktop para Linux ejecuta el motor en una máquina virtual y comparte carpetas con él mediante su propio mecanismo; si el punto de montaje no puede guardar los archivos allí, utilice también el volumen con nombre. No hemos probado Docker Engine sin root, Docker Desktop para Linux y macOS.
- **Terminal interactivo requerido:** Pase `-it` para jugar de forma interactiva. El inicio sin un TTY adjunto sale de `1` con el código `TTY_REQUIRED` y su indicación `docker run --rm -it <image>`; `--help` y `--version` salen de `0` sin un TTY.
- **Qué contiene la imagen:** Creada a partir de un hash `rust:1.100.0-bookworm` (`@sha256:`) en un hash `gcr.io/distroless/cc-debian12:nonroot` (`@sha256:`), con el valor predeterminado de un usuario que no es root `65532:65532` en `WORKDIR /saves`. Además de esta base distroless con hash (que no contiene shell ni administrador de paquetes), la compilación solo agrega `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE` y un directorio vacío `/saves` propiedad de `65532:65532`; no hay cadena de herramientas, archivos de origen y cero llamadas de red (`--network=none`).

## Seguridad y datos

- **A qué accede.** En el modo Club sin conexión (`league-tui`), `--save-dir` escribe `club-<id>.save.json` públicos, `club-<id>.seal` privados para un jugador (eliminados al exportar la repetición), `club-<id>.replay.json` verificados y diagnósticos de metadatos `--log-file` opcionales (nunca registra los PIN, las notas de la bóveda, los requisitos o las órdenes pendientes, ni los valores hash de confirmación, y elimina las credenciales de API/XRPL y las claves JSON selladas en los cuatro niveles de registro) en su propia máquina, sin realizar ninguna llamada de red. En el modo de juego (aún no implementado), las claves de Testnet de un jugador permanecen en su propia máquina, en la carpeta `.league/` ignorada por Git, y nunca se registran. El relé de prueba en papel lee su clave OpenRouter solo de la variable de entorno `OPENROUTER_API_KEY` y no realiza ninguna llamada OpenRouter sin `--live` y un límite de crédito. Sus asientos locales rechazan cualquier clave de API, solo acceden a una dirección de bucle o a una dirección de máquina virtual que usted permite con `--allow-vm-host` y requieren un límite de segundos de GPU. Guarda sus transcripciones en la carpeta del juego que le proporcione.
- **A qué no accede.** No hay dinero real, ni Mainnet y nada fuera de las carpetas anteriores. No hay **telemetría**: no se envía nada a ningún lugar, excepto las llamadas que necesita un modo (Testnet, un faro de aleatoriedad público y, para el relé, OpenRouter o los servidores de modelo locales que usted permite).
- **Permisos.** Acceso a la red de Testnet en el modo de juego. Docker con `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (o volumen con nombre `-v merchant-league-saves:/saves`) al ejecutar el contenedor local del modo Club (solo accede a `/saves`), o Docker con su puerto vinculado a `127.0.0.1` solo para el conjunto independiente.
- **Protecciones.** Cada conexión a Testnet debe informar del ID de red de Testnet. Un disparador de CI falla si se escribe cualquier nombre de host de Mainnet en el repositorio fuera del código proporcionado, y otro falla si se copia contenido que presenta el juego como una fuente de ingresos. La revisión cubre lo que una lista de palabras no puede ver.

Consulte [`SECURITY.md`](SECURITY.md) para conocer el modelo de amenazas y cómo informar sobre una vulnerabilidad.

## Documentación

- [Manual](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Diseño v2](docs/design-v2.md): las reglas, la economía y los dos modos
- [Prueba en papel](docs/paper-test.md): las primeras pruebas, realizadas en papel con asientos de IA
- [Conjunto independiente `xrpld`](docs/standalone-xrpld.md)
- [Recibos](docs/receipts/): comprobaciones de Testnet en vivo. Testnet se borra periódicamente (la próxima vez el 25 de noviembre de 2026), por lo que sus hashes de transacción dejan de resolverse después; lo que registraron y los vectores de prueba siguen siendo la prueba.

## Soporte

Este es un prototipo en fase de diseño activo. Los problemas y las ideas son bienvenidos en el [rastreador de problemas](https://github.com/mcp-tool-shop-org/merchant-league/issues). Aún no hay versiones y no hay garantías de soporte. Consulte el [REGISTRO DE CAMBIOS](CHANGELOG.md) para conocer el progreso.

## Licencia

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
