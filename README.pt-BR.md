<p align="center">
  <a href="README.ja.md">日本語</a> | <a href="README.zh.md">中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.md">English</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

Um jogo de comércio e política em turnos lentos, para quatro a seis jogadores, jogado na **Testnet** do XRP Ledger. Os jogadores podem ser pessoas ou jogadores de IA, na mesma mesa e sob as mesmas regras.

**Estado: protótipo, em fase ativa de desenvolvimento.** O **modo Club** offline em `league-tui` pode ser jogado a partir do código fonte em um terminal compartilhado (4 a 6 assentos humanos e `ClubBot`, desde o lobby até o placar final da temporada e a exportação de replays; as mensagens privadas são adiadas após `v0.1.0`). O modo de jogo Public-Testnet e a imagem do contêiner ainda não foram criados, e ainda não há **lançamento**. Apenas Testnet: sem dinheiro real e sem caminho para a Mainnet.

## O jogo

- **Um círculo de comerciantes.** Cada jogador produz seus próprios bens brutos e precisa de bens dos outros jogadores para pagar suas despesas, manter suas oficinas e entregar em projetos compartilhados. Ninguém pode prosperar sozinho, e nenhum pequeno grupo também.
- **Turnos simultâneos.** Cada turno tem três fases. Os jogadores conversam e negociam no mercado aberto; em seguida, cada jogador define seus pedidos em segredo; em seguida, os pedidos de cada jogador são resolvidos de uma vez, pelas mesmas regras para todos.
- **Política.** Os conselhos definem tarifas, os cartéis podem abrir mercados privados e as concessões (charters) são rotacionadas entre os jogadores ao longo da temporada.
- **Uma temporada de 24 a 32 turnos.** Os eventos inesperados (congelamento de fornecimento, isenção de tarifas, aumento da demanda ou pico de deterioração) são previstos publicamente um turno antes de ocorrerem, e o último turno é aleatório, para que ninguém possa planejar um movimento final perfeito.
- **Pontuação.** Um jogador recebe pontos com base em seu patrimônio líquido no final, mais um cartão de objetivo oculto distribuído no início. Cada jogador tem um motivo para jogar até o último turno.
- **Mesas verificadas.** Cada mesa é gerada a partir de uma semente e verificada antes de ser distribuída. O valor inicial de cada jogador deve estar dentro de 3% da média da mesa e dentro de 7% da média quando um jogador é avaliado com base no que ele contribui para cada grupo de jogadores.

O design completo está em [`docs/design-v2.md`](docs/design-v2.md).

## Duas formas de jogar

| Modo | Onde é executado | Para quem é |
| --- | --- | --- |
| **Modo de jogo** | a rede pública XRPL Testnet, com cerca de duas rodadas por dia | pessoas, sozinhas ou com assentos de IA |
| **Modo de treinamento e avaliação** | um nó `xrpld` autônomo privado em um contêiner Docker local, fechando ledgers em milissegundos | Assentos de IA: treinamento, testes de desempenho e replays |

Ambos os modos compartilham o mesmo motor de regras. Um jogo é reproduzido exatamente a partir de sua semente e de seu registro.

## O que já está construído

| Pacote | O que faz |
| --- | --- |
| `league-core` | O motor de regras: as fases da rodada, os contratos, a pontuação, o encerramento da temporada, o replay e os predicados de estado por trás do sistema de dicas planejado |
| `league-gen` | O gerador de tabelas, com suas verificações de imparcialidade e seus pools de sementes |
| `league-ledger` | O cliente XRPL Testnet. Ele rejeita qualquer rede que não seja a Testnet |
| `league-standalone` | O ambiente `xrpld` independente para o modo de treinamento, em um contêiner fixo com recursos limitados |
| `league-season` | As datas de reinicialização anunciadas da Testnet e uma verificação que detecta uma reinicialização não anunciada |
| `league-relay` | A ferramenta que executou os testes em papel com jogadores de IA |
| `league-tui` | O host de terminal do modo Club offline: configuração do lobby, rodadas compartilhadas (hot-seat) em quatro zonas, cortina de privacidade com PINs opcionais, passes com tempo limitado, salvamento/retomada no meio da rodada, placar final da temporada e exportação de replays |

## Jogue uma temporada

O modo Club offline executa uma temporada completa de `24..=32` rodadas em seu terminal (`4–6` assentos humanos ou `ClubBot` em uma máquina):

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configuração do lobby:** Escolha o número de assentos (`4–6`, `Left`/`Right`), alterne cada assento entre `Human` e `ClubBot` (`Space`), digite uma semente `SeedPool::Dev` em texto claro dentro de `0..=9999` (as sementes de teste reservadas e os outros conjuntos de sementes são recusados pelo nome), escolha o tempo de cada rodada (`180–300 s`, `210 s` por padrão, ou `Untimed` com `Space`) e ative ou desative os PINs de Cortina de 4 dígitos próprios da mesa. Pressione `Enter` em **Start Club Mode Table**. Começar enquanto existir um `club-*.seal` inacabado em `--save-dir` é recusado (`UNFINISHED_SEASON_EXISTS`); depois da exportação, a mesa seguinte recebe o próximo ID de mesa livre sem sobrescrever os salvamentos anteriores. As opções de linha de comando (`--turn-clock`, `--untimed`, `--pins`) prevalecem sobre as escolhas conflitantes do lobby e do `.seal` salvo (`league-tui --help` documenta a ordem de precedência; passar os dígitos do PIN com `--pin` na linha de comando é recusado).
- **Teclas da rodada:** `1`/`2`/`3`/`Tab`/`BackTab` alternam entre as abas habilitadas do compositor (`4: Diplomatic Desk (Disabled)` continua desabilitada); `F6` move o foco entre o Livro-razão público, as abas do compositor e o Cofre privado; `[`/`]` percorrem os tipos de ação ou as seções do pacote de ordens; `Up`/`Down`/`PageUp`/`PageDown`/`Home` selecionam ou rolam; `Left`/`Right`/`+`/`-`/`Space` ajustam os campos; `e` edita substituições de texto livre; `n` edita as notas pessoais do Cofre privado; `Enter` envia uma ação, prepara uma seção do pacote ou executa o Sandbox What-If de `p`; `Del`/`Backspace`/`X` limpam as ordens preparadas; `c` alterna entre os nomes das mercadorias da guilda e os códigos de 3 letras (`[CRED]`, `[TMB]`); `r`/`F2` abre a janela da Crônica da varredura; `Ctrl+l`/`Ctrl+L` trava a sua vez e abre a Cortina de privacidade; `q`/`Esc`/`Ctrl+c` saem de forma limpa.
- **Cortina de privacidade, tempo morto e PINs:** Cada passagem de vez, tempo esgotado e retomada no meio da temporada abre uma Cortina de privacidade pública (que limpa a tela alternativa no lugar com `CSI 2 J`, nunca com `CSI 3 J`, que apagaria o histórico de rolagem), descarta a entrada em fila e impõe um tempo morto de entrada de `400 ms` tanto ao abrir quanto ao levantar (o tempo morto ao levantar não conta no relógio do assento que chega). Pressione `Space` e depois `Enter` para levantar; se os PINs estiverem ativados, digite seu PIN mascarado de 4 dígitos (digitado apenas atrás da Cortina: a 1ª rodada o define, as passagens seguintes o verificam, tentativas erradas não geram nenhuma visão do assento e 3 erros devolvem a Cortina ao estado desarmado).
- **Relógio:** As rodadas cronometradas dividem cada orçamento em `75%` para o Subpasse A e `25%` para o Subpasse B entre todos os `H` assentos humanos (`floor(0.75 * B / H)` s e `floor(0.25 * B / H)` s). Travar antes avança imediatamente; tempos esgotados por inatividade no Subpasse B são resolvidos pela doutrina de reserva do Regente permanente.
- **Salvar e retomar:** Com `--save-dir <DIR>`, o anfitrião salva automaticamente de forma atômica em cada Cortina, no primeiro compromisso de PIN e na varredura que sai de `TalkAndMarket`. O estado público é salvo em `club-<id>.save.json`; os mandatos ocultos, os sais, as provas VRF derivadas de antemão, as notas, os pacotes não comprometidos e os PINs ficam apenas em `club-<id>.seal`. Retome quando quiser com `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (ou `--resume ./saves`, que seleciona a mesa do `.seal` ativo): uma retomada no meio da temporada sempre chega atrás da Cortina do assento humano ativo, com um relógio de vez novo (avançando antes os cursores `ClubBot` que estejam à frente), enquanto uma retomada depois da temporada chega direto à tela pública de revelação `Session Standby` (`AwaitingMandateReveal`) ou às abas de fim de temporada `Completed`, para que você possa tentar exportar o replay de novo. Passar `--resume` junto com um `--save-dir` diferente é recusado quando esse diretório contém outro arquivo `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, com qualquer ID de mesa) ou contém `club-<id>.save.json`, `.seal` ou `.replay.json` do mesmo ID de mesa (`SEASON_SAVE_EXISTS`, exceto ao retomar uma temporada terminada cujo próprio `club-<id>.replay.json` normal, idêntico byte a byte, já está nesse diretório sem um `.save.json` ou `.seal` em conflito, para que uma nova tentativa de exportação após uma falha ao apagar o `.seal` de origem prossiga); um diretório que contém apenas uma temporada terminada e exportada com outro ID de mesa é aceito sem sobrescrever nada. Indicar um `--save-dir` inexistente ou vazio o cria, se necessário, ao exportar, e o `.seal` retomado ainda é apagado se a exportação der certo. Uma retomada no meio da temporada em outro `--save-dir` continua a temporada lá e deixa no lugar o `club-<id>.save.json` e o `.seal` do diretório original. Exportar o replay nessa mesma sessão também apaga o `.seal` original; se, em vez disso, você sair e retomar mais tarde a partir do diretório novo, o `.seal` original permanece, ainda com os valores privados dessa temporada, e faz o lobby do diretório original continuar recusando uma temporada nova (`UNFINISHED_SEASON_EXISTS`). Apague essa cópia que sobrou só depois de sair e de a temporada estar salva no diretório novo. Fora essa cópia que sobrou, mantenha o `club-<id>.save.json`, o `.seal` e o `.replay.json` de cada temporada com os nomes e no diretório que o `league-tui` lhes deu; para mover uma temporada para outro lugar, retome-a com `--save-dir <EMPTY_DIR>` em vez de renomear ou mover os arquivos à mão.
- **Fim da temporada e exportação do replay:** Depois que o Sino de encerramento fecha `TalkAndMarket`, avance pela etapa pública de espera com revelação dos mandatos (`Enter`/`Space` por assento, sem mostrar nenhum mandato antes) até a tela de fim de temporada com quatro abas (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). Na aba 4, pressione `e` ou `Enter` para gravar e verificar `club-<id>.replay.json` e apagar o arquivo privado `.seal` (se o replay de outra temporada já ocupa esse caminho de destino, a exportação é recusada com `REPLAY_TARGET_EXISTS` sem sobrescrevê-lo; se a exportação falhar, o `.seal` fica no disco para que você possa tentar de novo).

## Construa a partir do código-fonte

Você precisa de [Rust](https://rustup.rs/). O conjunto de ferramentas fixado (1.100.0) se instala a partir de `rust-toolchain.toml` na primeira construção.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

O CI constrói e testa no Linux, e o espaço de trabalho é desenvolvido no Windows. Outras plataformas não foram testadas.

O Docker é necessário para construir e executar a imagem do contêiner do Modo Clube local ou executar o conjunto de testes autônomo `xrpld` (os testes do conjunto são ignorados, a menos que você os solicite):

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` executa as verificações que o CI executa, exceto o conjunto de testes autônomo do Docker e a verificação do mapa do Atlas. A cobertura, `cargo audit`, a construção do site e o teste de fumaça do contêiner do Modo Clube retido são executados apenas quando `cargo-llvm-cov`, `cargo-audit`, `npm` e Linux `docker` (com POSIX `python3` `pty`/`fcntl`/`termios`) estiverem disponíveis, e o script lista qualquer verificação que ele ignorou.

## Execute o Modo Clube no Docker (construção local)

A imagem do contêiner do Modo Clube (`league-tui`) ainda não foi publicada em nenhum registro de contêiner; o `Dockerfile` de várias etapas é construído localmente e verificado pelo teste de fumaça offline do CI (`scripts/smoke-container.sh`, sem envio para o registro).

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

No Windows com o Docker Desktop, em PowerShell:

```powershell
docker build -t merchant-league:local .
mkdir saves
docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local
```

- **Salvamento e montagem:** No Linux Docker Engine, o usuário padrão da imagem `65532:65532` não pode gravar em um diretório do host que pertence à sua conta (ou `root` criado por um caminho `-v` ausente); portanto, criar `./saves` previamente e passar `--user "$(id -u):$(id -g)"` garante que os salvamentos pertençam ao seu usuário não-root, sem `chmod 777` ou root. Alternativamente, use um volume nomeado do Docker (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, sem `--user`), que herda o `65532:65532` pertencente à imagem `/saves`. No Docker Desktop (macOS e Windows), as permissões de montagem são virtualizadas em toda a VM, portanto, o comando PowerShell acima e a mesma montagem no macOS funcionam sem `--user`, enquanto os shells Linux do WSL 2 podem executar o comando Linux `--user` acima. Com o Docker Engine sem root, os IDs de usuário do contêiner são mapeados de forma diferente (dentro do contêiner, seus próprios arquivos aparecem como `root`, e qualquer outro ID é mapeado para um ID subordinado); portanto, o comando `--user` acima não concede ao contêiner sua própria conta; use o volume nomeado lá. O Docker Desktop para Linux executa o engine em uma VM e compartilha pastas com ele por meio de seu próprio mecanismo; se a montagem não puder salvar os dados lá, use também o volume nomeado. O Docker Engine sem root, o Docker Desktop para Linux e macOS não foram testados por nós.
- **Terminal interativo necessário:** Passe `-it` para jogar interativamente. O lançamento sem um TTY anexado encerra `1` com o código `TTY_REQUIRED` e sua dica `docker run --rm -it <image>`; `--help` e `--version` encerram `0` sem um TTY.
- **O que a imagem contém:** Criada a partir de um hash fixo `rust:1.100.0-bookworm` (`@sha256:`) para um hash fixo `gcr.io/distroless/cc-debian12:nonroot` (`@sha256:`), com o padrão de não-root `65532:65532` em `WORKDIR /saves`. Além dessa base "distroless" (que não contém shell nem gerenciador de pacotes), a construção adiciona apenas `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE` e um diretório vazio `/saves` pertencente a `65532:65532` — sem conjunto de ferramentas, sem arquivos de origem e sem chamadas de rede (`--network=none`).

## Segurança e dados

- **O que é acessado.** No modo Club offline (`league-tui`), `--save-dir` grava `club-<id>.save.json` públicos, `club-<id>.seal` privados de jogo compartilhado (excluídos após a exportação bem-sucedida da reprodução), `club-<id>.replay.json` verificados e `--log-file` opcionais de diagnóstico de metadados (nunca registrando PINs, notas do Cofre, mandatos, ordens preparadas ou sais de compromisso, e ocultando as credenciais da API/XRPL e as chaves JSON seladas em todos os quatro níveis de registro) em sua própria máquina, sem chamadas de rede. No modo de jogo (ainda não implementado), as chaves da Testnet de um jogador permanecem em sua própria máquina, na pasta `.league/` ignorada pelo Git, e nunca são registradas. O relé de teste lê sua chave OpenRouter apenas da variável de ambiente `OPENROUTER_API_KEY` e não faz nenhuma chamada OpenRouter sem `--live` e um limite de crédito. Seus assentos locais recusam qualquer chave de API, acessam apenas um endereço de loopback ou um endereço de VM que você permite com `--allow-vm-host` e precisam de um limite de segundos de GPU. Ele mantém suas transcrições na pasta do jogo que você fornece.
- **O que não é acessado.** Nenhum dinheiro real, nenhuma Mainnet e nada fora das pastas acima. Não há **telemetria**: nada é enviado para lugar nenhum, exceto as chamadas que um modo precisa (a Testnet, um beacon de aleatoriedade público e, para o relé, OpenRouter ou os servidores de modelo locais que você permite).
- **Permissões.** Acesso à rede da Testnet no modo de jogo. Docker com `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (ou volume nomeado `-v merchant-league-saves:/saves`) ao executar o contêiner do modo Club local (acessando apenas `/saves`) ou Docker com sua porta vinculada a `127.0.0.1` apenas para o conjunto autônomo.
- **Proteções.** Cada conexão com a Testnet deve relatar o ID da rede da Testnet. Uma verificação de CI falha em qualquer nome de host da Mainnet escrito no repositório fora do código de terceiros incluído (vendored), e uma segunda falha em textos que apresentem o jogo como uma fonte de renda. A revisão cobre o que uma lista de palavras não pode ver.

Consulte [`SECURITY.md`](SECURITY.md) para o modelo de ameaças e como relatar uma vulnerabilidade.

## Documentação

- [Manual](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Design v2](docs/design-v2.md): as regras, a economia e os dois modos
- [Teste em papel](docs/paper-test.md): os primeiros testes, realizados em papel com jogadores de IA
- [Conjunto `xrpld` autônomo](docs/standalone-xrpld.md)
- [Comprovantes](docs/receipts/): verificações da Testnet ao vivo. A Testnet é apagada de tempos em tempos (próxima em 25/11/2026), portanto, seus hashes de transação param de ser resolvidos depois disso; o que eles registraram e os vetores de teste permanecem como evidência.

## Suporte

Este é um protótipo em fase de design ativo. Problemas e ideias são bem-vindos no [rastreador de problemas](https://github.com/mcp-tool-shop-org/merchant-league/issues). Ainda não há lançamentos e nenhuma garantia de suporte. Consulte o [CHANGELOG](CHANGELOG.md) para obter informações sobre o progresso.

## Licença

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
