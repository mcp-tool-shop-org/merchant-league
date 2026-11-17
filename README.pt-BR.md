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

# Liga Mercantil

Um jogo de estratégia e política com ritmo lento, para quatro a seis jogadores, jogado na **Testnet** do XRP Ledger. Os jogadores podem ser pessoas ou jogadores de IA, na mesma mesa e sob as mesmas regras.

**Status: protótipo, fase de design.** O motor de regras, o gerador de mesas e a função de replay foram criados e testados. O jogo em si **ainda não está jogável**: não há cliente de jogo e nem há lançamento. Apenas na Testnet: sem dinheiro real e sem caminho para a Mainnet.

## O jogo

- **Um círculo de comerciantes.** Cada jogador produz seus próprios bens brutos e precisa de bens dos outros jogadores para pagar suas despesas, manter suas oficinas e entregar em projetos compartilhados. Ninguém pode prosperar sozinho, e nenhum pequeno grupo também.
- **Turnos simultâneos.** Cada turno tem três fases. Os jogadores conversam e negociam no mercado aberto; em seguida, cada jogador define seus pedidos em segredo; em seguida, os pedidos de cada jogador são resolvidos de uma vez, pelas mesmas regras para todos.
- **Política.** Os conselhos definem tarifas, os cartéis podem abrir mercados privados e as licenças são rotacionadas entre os jogadores ao longo da temporada.
- **Uma temporada de 24 a 32 turnos.** Os eventos inesperados (congelamento de fornecimento, isenção de tarifas, aumento da demanda ou pico de deterioração) são previstos publicamente um turno antes de ocorrerem, e o último turno é aleatório, para que ninguém possa planejar um movimento final perfeito.
- **Pontuação.** Um jogador recebe pontos com base em seu patrimônio líquido no final, mais um cartão de objetivo oculto distribuído no início. Cada jogador tem um motivo para jogar até o último turno.
- **Mesas verificadas.** Cada mesa é gerada a partir de uma semente e verificada antes de ser distribuída. O valor inicial de cada jogador deve estar dentro de 3% da média da mesa e dentro de 7% da média quando um jogador é avaliado com base no que ele contribui para cada grupo de jogadores.

O design completo está em [`docs/design-v2.md`](docs/design-v2.md).

## Duas maneiras de jogar

| Modo | Onde é executado | Para quem é |
| --- | --- | --- |
| **Modo de jogo** | a Testnet pública do XRPL, com cerca de dois turnos por dia | pessoas, sozinhas ou com jogadores de IA |
| **Modo de treinamento e avaliação** | um nó `xrpld` autônomo privado em um contêiner Docker local, processando transações em milissegundos | jogadores de IA: treinamento, testes de desempenho e replays |

Ambos os modos compartilham um único motor de regras. Um jogo é reproduzido exatamente a partir de sua semente e de seu registro.

## O que é construído

| Crate | O que faz |
| --- | --- |
| `league-core` | O motor de regras: as fases do turno, os contratos, a pontuação, o encerramento da temporada, o replay e os predicados de estado por trás do sistema de dicas planejado |
| `league-gen` | O gerador de mesas, com suas verificações de justiça e seus pools de sementes |
| `league-dice` | Por enquanto, apenas protótipos. A aleatoriedade do jogo em si (compromisso e revelação, misturados com um beacon público drand no modo de jogo e uma função aleatória verificável no modo de treinamento) é construída em `league-core` |
| `league-ledger` | O cliente XRPL Testnet. Ele rejeita qualquer rede que não seja a Testnet |
| `league-standalone` | O conjunto `xrpld` autônomo para o modo de treinamento, em um contêiner fixo com recursos limitados |
| `league-season` | Os limites da temporada, que seguem os resets da Testnet |
| `league-relay` | A ferramenta que executou os testes em papel com jogadores de IA |
| `league-tui` | O cliente de terminal. Apenas uma tela de título, por enquanto |

## Construa a partir do código-fonte

Você precisa de [Rust](https://rustup.rs/). A cadeia de ferramentas fixa (1.100.0) é instalada automaticamente a partir de `rust-toolchain.toml` na primeira construção.

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

As construções e os testes de CI são feitos no Linux, e o espaço de trabalho é desenvolvido no Windows. Outras plataformas não foram testadas.

O Docker é necessário apenas para o conjunto `xrpld` autônomo, e seus testes são ignorados, a menos que você solicite:

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` executa as verificações que o CI executa, exceto o conjunto Docker e a verificação do mapa do Atlas. A cobertura e `cargo audit` são executadas apenas quando `cargo-llvm-cov` e `cargo-audit` estão instalados, e o script lista qualquer verificação que tenha sido ignorada.

## Segurança e dados

- **O que ele acessa.** No modo de jogo (ainda não construído), as chaves da Testnet de um jogador permanecem em sua própria máquina, na pasta `.league/` ignorada pelo Git, e nunca são registradas. O relay dos testes em papel lê sua chave OpenRouter apenas da variável de ambiente `OPENROUTER_API_KEY` e não faz nenhuma chamada OpenRouter sem `--live` e um limite de crédito. Seus jogadores locais rejeitam qualquer chave de API, acessam apenas um endereço de loopback ou um endereço de VM que você permite com `--allow-vm-host` e precisam de um limite de segundos de GPU. Ele mantém suas transcrições na pasta do jogo que você fornece.
- **O que ele não acessa.** Nenhum dinheiro real, nenhuma Mainnet e nada fora das pastas acima. Não há **telemetria**: nada é enviado para lugar nenhum, exceto as chamadas que um modo precisa (a Testnet, um beacon de aleatoriedade pública e, para o relay, OpenRouter ou os servidores de modelo locais que você permite).
- **Permissões.** Acesso à rede para a Testnet no modo de jogo. Docker, com sua porta vinculada apenas a `127.0.0.1`, para o conjunto autônomo.
- **Proteções.** Cada conexão com a Testnet deve relatar o ID da rede da Testnet, o CI falha em qualquer nome de host da Mainnet e uma segunda proteção falha em qualquer cópia que apresente o jogo como uma fonte de renda.

Consulte [`SECURITY.md`](SECURITY.md) para o modelo de ameaças e como relatar uma vulnerabilidade.

## Documentação

- [Manual](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [Design v2](docs/design-v2.md): as regras, a economia e os dois modos
- [Teste em papel](docs/paper-test.md): os primeiros testes, realizados em papel com jogadores de IA
- [Conjunto `xrpld` autônomo](docs/standalone-xrpld.md)
- [Comprovantes](docs/receipts/): verificações da Testnet ao vivo. A Testnet é apagada de tempos em tempos (próxima em 25/11/2026), portanto, seus hashes de transação param de ser resolvidos depois disso; o que eles registraram e os vetores de teste permanecem como evidência.

## Suporte

Este é um protótipo em fase de design ativo. Problemas e ideias são bem-vindos no [rastreador de problemas](https://github.com/mcp-tool-shop-org/merchant-league/issues). Ainda não há lançamentos e nem garantias de suporte. Consulte o [CHANGELOG](CHANGELOG.md) para obter informações sobre o progresso.

## Licença

[MIT](LICENSE).

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
