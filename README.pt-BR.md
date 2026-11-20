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

**Estado: protótipo, em fase ativa de desenvolvimento.** O modo **Club Mode** offline em `league-tui` pode ser jogado a partir do código fonte em um terminal compartilhado (4 a 6 jogadores e `ClubBot` assentos, desde o lobby até o placar final da temporada e a exportação de replays; as mensagens privadas são adiadas após `v0.1.0`). O modo de jogo Public-Testnet e a imagem do contêiner ainda não foram criados, e ainda não há **lançamento**. Apenas Testnet: sem dinheiro real e sem caminho para a Mainnet.

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
| **Game mode** | a rede pública XRPL Testnet, com cerca de duas rodadas por dia | pessoas, sozinhas ou com assentos de IA |
| **Modo de treinamento e avaliação** | um nó `xrpld` autônomo privado em um contêiner Docker local, fechando ledgers em milissegundos | Assentos de IA: treinamento, testes de desempenho e replays |

Ambos os modos compartilham o mesmo motor de regras. Um jogo é reproduzido exatamente a partir de sua semente e de seu registro.

## O que está sendo construído

| Pacote | O que faz |
| --- | --- |
| `league-core` | O motor de regras: as fases da rodada, os contratos, a pontuação, o encerramento da temporada, o replay e os predicados de estado por trás do sistema de dicas planejado |
| `league-gen` | O gerador de tabelas, com suas verificações de imparcialidade e seus pools de sementes |
| `league-ledger` | O cliente XRPL Testnet. Ele rejeita qualquer rede que não seja a Testnet |
| `league-standalone` | O ambiente `xrpld` independente para o modo de treinamento, em um contêiner fixo com recursos limitados |
| `league-season` | As datas de reinicialização anunciadas da Testnet e uma verificação que detecta uma reinicialização não anunciada |
| `league-relay` | A ferramenta que executou os testes em papel com jogadores de IA |
| `league-tui` | O host do terminal do modo Club Mode offline: configuração do lobby, rodadas em quatro zonas, cortina de privacidade com PINs opcionais, passes com tempo limitado, salvamento/retomada no meio da rodada, placar final da temporada e exportação de replays |

## Jogue uma temporada

O modo Club Mode offline executa uma temporada completa de `24..=32` rodadas em seu terminal (`4–6` jogadores ou `ClubBot` assentos em uma máquina):

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **Configuração do lobby:** Escolha o número de jogadores (`4–6`, `Left`/`Right`), alterne cada jogador entre `Human` e `ClubBot` (`Space`), insira uma sequência `SeedPool::Dev` clara em `0..=9999` (sequências de teste reservadas e outros conjuntos são recusados por nome), escolha um limite de turnos (`180–300 s`, `210 s` por padrão ou `Untimed` com `Space`) e alterne os códigos PIN de 4 dígitos locais da mesa. Pressione `Enter` em **Iniciar Modo Clube na Tabela**. Iniciar o processo enquanto existir um ficheiro `club-*.seal` incompleto em `--save-dir` não é permitido (`UNFINISHED_SEASON_EXISTS`); após a exportação, a tabela seguinte recebe o próximo ID de tabela disponível, sem substituir os ficheiros guardados anteriormente. As opções da linha de comando (`--turn-clock`, `--untimed`, `--pins`) têm precedência sobre as configurações conflitantes do Lobby e as opções salvas (`.seal`; os documentos `league-tui --help` têm precedência; o envio de dígitos do PIN através de `--pin` na linha de comando não é permitido).
- **Teclas de atalho:** `1`/`2`/`3`/`Tab`/`BackTab` ativa/desativa as guias do Composer (`4: Diplomatic Desk (Disabled)` permanece desativada); `F6` alterna o foco entre o Livro Razão Público, as guias do Composer e o Cofre Privado; `[`/`]` alterna os tipos de ação ou as seções do pacote; `Up`/`Down`/`PageUp`/`PageDown`/`Home` seleciona ou percorre; `Left`/`Right`/`+`/`-`/`Space` ajusta os campos; `e` edita as substituições de texto livre; `n` edita as notas pessoais do Cofre Privado; `Enter` submete uma ação, prepara uma seção do pacote ou executa o `p` Sandbox “E se”; `Del`/`Backspace`/`X` limpa os pedidos em preparação; `c` alterna entre os nomes completos da guilda e os códigos de 3 letras (`[CRED]`, `[TMB]`); `r`/`F2` abre a janela modal do Sweep Chronicle; `Ctrl+l`/`Ctrl+L` bloqueia o seu passe e abre a Cortina de Privacidade; `q`/`Esc`/`Ctrl+c` encerra o programa de forma limpa.
- **Barreira de privacidade, tempo de inatividade e códigos PIN:** Cada transferência, tempo limite e retomada durante a temporada ativa uma barreira de privacidade pública (limpando a tela alternativa no local com `CSI 2 J` e nunca `CSI 3 J`, eliminando o histórico de rolagem), esvazia as entradas em fila e impõe um tempo de inatividade de `400 ms` tanto ao abrir quanto ao liberar (o tempo de inatividade ao liberar não é contabilizado no tempo do assento que está sendo liberado). Pressione `Space` e, em seguida, `Enter` para levantar; se os códigos PIN estiverem ativados, digite o seu código PIN de 4 dígitos (inserido apenas atrás da Cortina — a primeira vez que o faz, ele é definido; nas transferências subsequentes, é verificado; tentativas incorretas não registam observações e três tentativas falhadas fazem com que o sistema volte à configuração inicial, sem código PIN).
- **Tempo:** Os turnos programados dividem cada orçamento `75%` Sub-Pass A / `25%` Sub-Pass B entre todos os `H` jogadores humanos (`floor(0.75 * B / H)` s e `floor(0.25 * B / H)` s). Consolidar imediatamente os ganhos iniciais; os períodos de inatividade do Sub-Pass B são resolvidos através da doutrina de contingência do Regente Permanente.
- **Guardar e retomar:** Com `--save-dir <DIR>`, o sistema faz automaticamente uma cópia de segurança de forma atómica em cada transição de ecrã, no momento da introdução inicial do PIN e quando termina uma operação de varredura, deixando `TalkAndMarket`. Os dados públicos do estado são armazenados em `club-<id>.save.json`; os dados confidenciais, como os parâmetros obrigatórios, os valores aleatórios, as provas VRF pré-calculadas, as notas, os conjuntos de dados não confirmados e os códigos PIN, permanecem exclusivamente em `club-<id>.seal`. Retome a qualquer momento com `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json` (ou `--resume ./saves`, selecionando a tabela `.seal` ativa): as retomadas durante a temporada sempre são exibidas atrás da cortina do assento do operador, com um novo contador de tempo (movendo primeiro quaisquer cursores `ClubBot` à frente), enquanto as retomadas após a temporada são exibidas diretamente na tela pública `Session Standby` (`AwaitingMandateReveal`) ou nas abas `Completed` «Fim da Temporada», para que possa tentar exportar o replay novamente. Tentar passar `--resume` juntamente com um `--save-dir` diferente é recusado quando esse diretório contém outro arquivo `club-*.seal` (`UNFINISHED_SEASON_EXISTS`, qualquer ID de tabela) ou, de outra forma, contém `club-<id>.save.json`, `.seal` ou `.replay.json` para a mesma ID de tabela (`SEASON_SAVE_EXISTS`, exceto quando se retoma uma temporada concluída cujo arquivo regular idêntico em bytes `club-<id>.replay.json` já está nesse diretório, sem conflitos de `.save.json` ou `.seal`, de modo que uma nova tentativa de exportação após uma falha na limpeza da fonte `.seal` prossegue); um diretório que contém apenas uma temporada concluída e exportada com uma ID de tabela diferente é aceito sem sobrescrever nada. Ao definir como alvo um `--save-dir` ausente ou vazio, este será criado, se necessário, durante a exportação, e o `.seal` retomado será apagado após a exportação ser concluída com sucesso. Ao importar um ficheiro de época para outro `--save-dir`, a época continua nesse ficheiro e os ficheiros `club-<id>.save.json` e `.seal` do diretório original permanecem inalterados. Exportar a gravação nessa mesma sessão também apaga o arquivo original `.seal`; se, em vez disso, você sair e depois retomar a partir do novo diretório, o arquivo original `.seal` será preservado, mantendo os valores privados daquela temporada e fazendo com que o diretório original continue a recusar uma nova temporada (`UNFINISHED_SEASON_EXISTS`). Apague essa cópia antiga somente depois de sair do programa e de verificar que a temporada foi salva no novo diretório. Além desse ficheiro remanescente, mantenha os ficheiros `club-<id>.save.json`, `.seal` e `.replay.json` de cada época sob os respetivos nomes e na pasta `league-tui` que lhes atribuiu; para mover uma época para outro local, utilize o comando `--save-dir <EMPTY_DIR>` em vez de renomear ou mover os ficheiros manualmente.
- **Fim da temporada e exportação de replays:** Após o encerramento do período de negociação `TalkAndMarket`, avance para a fase pública de revelação das instruções (`Enter`/`Space` por assento, mostrando zero instruções previamente) para finalizar na tela de quatro abas do Fim da Temporada (`1: Scoreboard`, `2: Mandates`, `3: Metrics & Marks`, `4: Replay Export`). Na guia 4, pressione `e` ou `Enter` para gravar e verificar `club-<id>.replay.json` e apagar o ficheiro privado `.seal` (se a repetição de outra época já ocupar esse caminho de destino, a exportação será recusada `REPLAY_TARGET_EXISTS` sem substituir o ficheiro; se a exportação falhar, `.seal` permanece no disco para que possa tentar novamente).

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

- **O que é acessado.** No modo Club offline (`league-tui`), `--save-dir` grava `club-<id>.save.json` públicos, `club-<id>.seal` privados (excluídos após a exportação bem-sucedida da reprodução), `club-<id>.replay.json` verificados e `--log-file` opcionais de diagnóstico de metadados (nunca registrando PINs, notas do Vault, requisitos, pedidos em andamento ou sal de confirmação e removendo as credenciais da API/XRPL e as chaves JSON seladas em todos os quatro níveis de registro) em sua própria máquina, sem chamadas de rede. No modo de jogo (ainda não implementado), as chaves da Testnet de um jogador permanecem em sua própria máquina, na pasta `.league/` ignorada pelo Git, e nunca são registradas. O relé de teste lê sua chave OpenRouter apenas da variável de ambiente `OPENROUTER_API_KEY` e não faz nenhuma chamada OpenRouter sem `--live` e um limite de crédito. Seus locais aceitam apenas chaves de API, acessam apenas um endereço de loopback ou um endereço de VM que você permite com `--allow-vm-host` e precisam de um limite de segundos de GPU. Ele mantém suas transcrições na pasta do jogo que você fornece.
- **O que não é acessado.** Nenhum dinheiro real, nenhuma Mainnet e nada fora das pastas acima. Não há **telemetria**: nada é enviado para lugar nenhum, exceto as chamadas que um modo precisa (a Testnet, um beacon de aleatoriedade público e, para o relé, OpenRouter ou os servidores de modelo locais que você permite).
- **Permissões.** Acesso à rede da Testnet no modo de jogo. Docker com `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"` (ou volume nomeado `-v merchant-league-saves:/saves`) ao executar o contêiner do modo Club local (acessando apenas `/saves`) ou Docker com sua porta vinculada a `127.0.0.1` apenas para o conjunto autônomo.
- **Proteções.** Cada conexão com a Testnet deve relatar o ID da rede da Testnet. Um gatilho de CI falha em qualquer nome de host da Mainnet escrito no repositório fora do código fornecido, e um segundo gatilho falha em cópias que enquadram o jogo como uma fonte de renda. A revisão cobre o que uma lista de palavras não pode ver.

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
