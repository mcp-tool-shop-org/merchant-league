<p align="center">
  <a href="README.ja.md">日本語</a> | <a href="README.zh.md">中文</a> | <a href="README.md">English</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

# Liga de Comerciantes

Un juego de estrategia y política con turnos lentos, para cuatro a seis jugadores, que se juega en la **Testnet** de XRP Ledger. Los jugadores pueden ser personas o jugadores de IA, en la misma mesa y bajo las mismas reglas.

**Estado: prototipo, en fase de diseño.** El motor de reglas, el generador de mesas y la función de repetición ya están construidos y probados. El juego en sí **todavía no se puede jugar**: no hay cliente de juego ni versión disponible. Solo Testnet: no hay dinero real y no hay posibilidad de pasar a Mainnet.

## El juego

- **Un círculo de comerciantes.** Cada jugador produce sus propios bienes básicos y necesita bienes de los demás jugadores para cubrir sus gastos, mantener sus talleres y entregar a proyectos compartidos. Nadie puede prosperar solo, ni tampoco un pequeño grupo.
- **Turnos simultáneos.** Cada turno tiene tres fases. Los jugadores hablan y negocian en el mercado abierto; luego, cada jugador realiza sus pedidos en secreto; luego, los pedidos de cada jugador se resuelven a la vez, según las mismas reglas para todos.
- **Política.** Los consejos establecen los aranceles, los cárteles pueden abrir mercados privados y las licencias rotan entre los jugadores a lo largo de la temporada.
- **Una temporada de 24 a 32 turnos.** Las alteraciones en el suministro se anuncian con dos turnos de antelación, y el último turno se determina al azar, para que nadie pueda planificar un movimiento final perfecto.
- **Puntuación.** Un jugador obtiene su valor neto al final, más una carta de objetivo oculto que se le entrega al principio. Cada jugador tiene una razón para seguir jugando hasta el último turno.
- **Mesas verificadas.** Cada mesa se genera a partir de una semilla y se verifica antes de que se juegue. El valor inicial de cada jugador debe estar dentro del 3% de los demás, y dentro del 7% cuando se valora un jugador por lo que aporta a cada grupo de jugadores.

El diseño completo está en [`docs/design-v2.md`](docs/design-v2.md).

## Dos formas de jugar

| Modo | Dónde se ejecuta | Para quién es |
| --- | --- | --- |
| **Game mode** | la red de pruebas pública XRPL, a un ritmo de aproximadamente dos turnos al día | personas, solos o con jugadores de IA |
| **Modo de entrenamiento y evaluación** | un nodo `xrpld` independiente privado en un contenedor Docker local, que cierra los libros de cuentas en milisegundos | jugadores de IA: entrenamiento, pruebas de rendimiento y repeticiones |

Ambos modos comparten un motor de reglas. Un juego se repite exactamente a partir de su semilla y su registro.

## Qué se ha construido

| Crate | Qué hace |
| --- | --- |
| `league-core` | El motor de reglas: las fases del turno, los contratos, la puntuación, el cierre de la temporada, la repetición y los predicados de estado que sustentan el sistema de pistas planificado |
| `league-gen` | El generador de mesas, con sus comprobaciones de equidad y sus grupos de semillas |
| `league-dice` | Dados que ningún jugador puede predecir o manipular: compromiso y revelación, combinados con un faro de aleatoriedad pública |
| `league-ledger` | El cliente de la red de pruebas XRPL. Rechaza cualquier red que no sea la red de pruebas |
| `league-standalone` | El conjunto `xrpld` independiente para el modo de entrenamiento, en un contenedor con recursos limitados y fijados |
| `league-season` | Límites de temporada, que siguen los reinicios de la red de pruebas |
| `league-relay` | La herramienta que ejecutó las pruebas en papel con jugadores de IA |
| `league-tui` | El cliente de terminal. Solo una pantalla de título, por ahora |

## Compilar desde el código fuente

Necesitas [Rust](https://rustup.rs/). La cadena de herramientas fijada (1.100.0) se instala automáticamente desde `rust-toolchain.toml` en la primera compilación.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

Las compilaciones y pruebas de CI se realizan en Linux, y el espacio de trabajo se desarrolla en Windows. No se han probado otras plataformas.

Docker solo es necesario para el conjunto `xrpld` independiente, y sus pruebas se omiten a menos que lo solicites:

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`scripts/verify.sh` ejecuta las comprobaciones que realiza CI, excepto el conjunto Docker y la comprobación del mapa de Atlas.

## Seguridad y datos

- **A qué accede.** En el modo de juego (todavía no construido), las claves de la red de pruebas de un jugador permanecen en su propia máquina, en la carpeta `.league/` que se ignora en Git, y nunca se registran. El relé de prueba en papel lee su clave OpenRouter solo de la variable de entorno `OPENROUTER_API_KEY`, no realiza ninguna llamada de red sin `--live` y un límite de crédito, y guarda sus transcripciones en la carpeta del juego que le proporciones.
- **A qué no accede.** No hay dinero real, no hay Mainnet y nada fuera de las carpetas anteriores. No hay **telemetría**: no se envía nada a ninguna parte, excepto las llamadas que necesita un modo (la red de pruebas, un faro de aleatoriedad pública y, para el relé, OpenRouter).
- **Permisos.** Acceso a la red de pruebas en el modo de juego. Docker, con su puerto vinculado solo a `127.0.0.1`, para el conjunto independiente.
- **Protecciones.** Cada conexión a la red de pruebas debe informar del ID de la red de la red de pruebas, CI falla si hay algún nombre de host de Mainnet y una segunda trampa falla si hay alguna copia que presente el juego como una fuente de ingresos.

Consulta [`SECURITY.md`](SECURITY.md) para conocer el modelo de amenazas y cómo informar de una vulnerabilidad.

## Documentación

- [Manual](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Diseño v2](docs/design-v2.md): las reglas, la economía y los dos modos
- [Prueba en papel](docs/paper-test.md): las primeras pruebas, realizadas en papel con jugadores de IA
- [Conjunto `xrpld` independiente](docs/standalone-xrpld.md)
- [Recibos](docs/receipts/): comprobaciones en vivo en la red de pruebas. La red de pruebas se borra de vez en cuando (la próxima vez el 25 de noviembre de 2026), por lo que sus hashes de transacción dejan de resolverse después; lo que registraron y los vectores de prueba siguen siendo la prueba.

## Soporte

Este es un prototipo en fase de diseño activo. Los problemas y las ideas son bienvenidos en el [rastreador de problemas](https://github.com/mcp-tool-shop-org/merchant-league/issues). Todavía no hay versiones ni garantías de soporte. Consulta el [REGISTRO DE CAMBIOS](CHANGELOG.md) para ver el progreso.

## Licencia

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
