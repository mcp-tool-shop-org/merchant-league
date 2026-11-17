<p align="center">
  <a href="README.ja.md">日本語</a> | <a href="README.md">English</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

# 商人联盟

一款慢节奏的贸易和政治游戏，适合四到六名玩家，在 XRP Ledger **测试网**上进行。玩家可以是真人，也可以是 AI 玩家，在同一张桌子上，遵循相同的规则。

**状态：原型，设计阶段。** 规则引擎、桌子生成器和重播功能已经构建并经过测试。游戏本身**尚未可玩**：没有游戏客户端，也没有发布版本。仅限测试网：不涉及真实货币，也没有连接到主网的途径。

## 游戏

- **一个商人圈。** 每个玩家生产自己的原材料，需要从其他玩家那里获取商品，以支付运营成本、运营工作室并交付到共享项目中。没有人可以单独繁荣，也没有小团体可以做到。
- **同步回合。** 每个回合有三个阶段。玩家在开放市场上进行交流和交易；然后，每个玩家秘密地提交自己的订单；然后，所有玩家的订单同时执行，所有玩家都遵循相同的规则。
- **政治。** 议会设定关税，卡特尔可以开设私人市场，特许经营权在整个赛季中在玩家之间轮换。
- **一个赛季有 24 到 32 个回合。** 冲击（供应冻结、关税假期、需求激增或腐败高峰）会在发生前一个回合公开预测，并且最后一个回合是随机的，因此没有人可以计划出完美的最终行动。
- **计分。** 玩家在游戏结束时计算其净资产，再加上在游戏开始时分配的隐藏目标卡。每个玩家都有理由玩到最后一回合。
- **经过检查的桌子。** 每个桌子都是从一个种子生成，并在分配之前进行检查。每个玩家的起始值必须在桌子平均值的 3% 以内，并且在计算某个玩家对每个玩家群体所做的贡献时，其值必须在平均值的 7% 以内。

完整的设计可以在 [`docs/design-v2.md`](docs/design-v2.md) 中找到。

## 两种游戏模式

| 模式 | 运行环境 | 适合人群 |
| --- | --- | --- |
| **游戏模式** | 公共 XRPL 测试网，大约每天进行两个回合 | 玩家，可以单独玩，也可以与 AI 玩家一起玩 |
| **训练和评估模式** | 一个私有的独立 `xrpld` 节点，在本地 Docker 容器中运行，以毫秒为单位完成回合。 | AI 玩家：训练、基准测试和重播 |

两种模式都共享一个规则引擎。游戏会完全按照其种子和日志进行重播。

## 已构建的内容

| 组件 | 其作用 |
| --- | --- |
| `league-core` | 规则引擎：回合阶段、合约、计分、赛季结束、重播以及计划的提示系统背后的状态谓词 |
| `league-gen` | 桌子生成器，包括其公平性检查和种子池 |
| `league-dice` | 目前只是占位符。游戏本身的随机性（提交和揭示，与游戏模式中的公共 drand 信标和训练模式中的可验证随机函数混合）是在 `league-core` 中构建的。 |
| `league-ledger` | XRPL 测试网客户端。它拒绝任何不是测试网的网络。 |
| `league-standalone` | 用于训练模式的独立 `xrpld` 框架，位于已固定的、资源受限的容器中。 |
| `league-season` | 赛季边界，遵循测试网的重置 |
| `league-relay` | 用于运行与 AI 玩家一起进行的纸质测试的工具 |
| `league-tui` | 终端客户端。目前只有标题屏幕。 |

## 从源代码构建

您需要 [Rust](https://rustup.rs/)。固定的工具链（1.100.0）会在第一次构建时从 `rust-toolchain.toml` 安装。

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

CI 在 Linux 上构建和测试，工作区在 Windows 上开发。其他平台未经测试。

Docker 仅用于独立的 `xrpld` 框架，并且除非您要求，否则其测试将被跳过：

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh` 运行 CI 运行的检查，但不包括 Docker 框架和 Atlas 地图检查。覆盖率和 `cargo audit` 仅在安装了 `cargo-llvm-cov` 和 `cargo-audit` 时运行，并且脚本会列出它跳过的任何检查。

## 安全性和数据

- **它会触及什么。** 在游戏模式（尚未构建）中，玩家的测试网密钥保留在其自己的机器上，位于 git 忽略的 `.league/` 文件夹中，并且绝不会被记录。纸质测试中继仅从 `OPENROUTER_API_KEY` 环境变量中读取其 OpenRouter 密钥，并且在没有 `--live` 和信用额度限制的情况下不会进行任何 OpenRouter 调用。其本地玩家拒绝任何 API 密钥，仅连接到循环回地址或您使用 `--allow-vm-host` 允许的 VM 地址，并且需要 GPU 秒数限制。它将其记录保存在您提供的游戏文件夹中。
- **它不会触及什么。** 没有真实货币，没有主网，也没有任何超出上述文件夹的内容。**没有遥测数据**：除了模式需要的调用（测试网、公共随机数信标以及中继的 OpenRouter 或您允许的本地模型服务器）之外，没有任何内容会被发送到任何地方。
- **权限。** 在游戏模式下，需要访问测试网的网络。Docker，其端口仅绑定到 `127.0.0.1`，用于独立的框架。
- **保护措施。** 每个测试网连接都必须报告测试网的网络 ID，CI 会在任何主网主机名上失败，并且第二个触发器会在任何将游戏描述为收入来源的副本上失败。

有关威胁模型以及如何报告漏洞，请参阅 [`SECURITY.md`](SECURITY.md)。

## 文档

- [手册](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [设计 v2](docs/design-v2.md)：规则、经济和两种模式
- [纸质测试](docs/paper-test.md)：第一次测试，与 AI 玩家一起在纸上进行
- [独立 `xrpld` 框架](docs/standalone-xrpld.md)
- [收据](docs/receipts/)：实时测试网检查。测试网会不时被清除（下次在 2026-11-25），因此其事务哈希在之后将停止解析；它们记录的内容以及测试向量将作为证据保留。

## 支持

这是一个正在积极设计中的原型。欢迎在 [问题跟踪器](https://github.com/mcp-tool-shop-org/merchant-league/issues) 上提出问题和想法。目前还没有发布版本，也没有支持保证。有关进度，请参阅 [CHANGELOG](CHANGELOG.md)。

## 许可证

[MIT](LICENSE)。

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
