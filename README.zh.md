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

这是一款慢节奏的交易和政治游戏，适合四到六名玩家，在 XRP Ledger **测试网**上进行。玩家可以是真人，也可以是 AI 玩家，在同一张桌子上，遵循相同的规则。

**状态：原型，正在积极设计中。** `league-tui` 中的离线 **俱乐部模式** 可以在一台共享终端上从源代码进行游戏（真人与 `ClubBot` 座位共 4–6 个，从大厅开始，到赛季结束的排行榜和游戏回放导出；私聊功能推迟到 `v0.1.0` 之后）。公共测试网游戏模式尚未构建。**v0.1.0** 是首个带标签的原型版本：仅提供源代码，没有发布的二进制文件，也没有发布的容器镜像。仅限测试网：不涉及真实货币，也没有通往主网的途径。

## 游戏

- **一个商人圈。** 每个玩家生产自己的原材料，需要从其他玩家那里获取商品，以支付运营成本、运营工作室并交付到共享项目中。没有人可以单独繁荣，也没有小团体可以做到。
- **同步回合。** 每个回合有三个阶段。玩家在开放市场上进行交流和交易；然后，每个玩家秘密地提交自己的订单；然后，所有玩家的订单同时执行，所有玩家都遵循相同的规则。
- **政治。** 议会设定关税，卡特尔可以开设私人市场，特许经营权在整个赛季中在玩家之间轮换。
- **一个赛季有 24 到 32 个回合。** 冲击（供应冻结、关税假期、需求激增或腐烂高峰）会在发生前一个回合公开预测，并且最后一个回合是随机的，因此没有人可以计划出完美的最终行动。
- **计分。** 玩家在游戏结束时计算其净资产，再加上在游戏开始时分配的隐藏目标卡。每个玩家都有理由玩到最后一回合。
- **经过检查的桌子。** 每个桌子都是从一个种子生成，并在分配之前进行检查。每个玩家的起始值必须在桌子平均值的 3% 以内，并且在计算某个玩家对每个玩家群体所做的贡献时，其值必须在平均值的 7% 以内。

完整的设计方案请参见 [`docs/design-v2.md`](docs/design-v2.md)。

## 两种游戏方式

| 模式 | 运行环境 | 目标用户 |
| --- | --- | --- |
| **游戏模式** | 公共 XRPL 测试网，大约每天进行两轮游戏 | 真人玩家（仅真人，或与 AI 座位同桌） |
| **训练和评估模式** | 一个私有的独立 `xrpld` 节点，在本地 Docker 容器中运行，在几毫秒内关闭账本（ledger）。 | AI 玩家：训练、基准测试和游戏回放 |

两种模式共享一个规则引擎。游戏可以从其种子和日志中完全重现。

## 已构建的内容

| 模块 | 功能 |
| --- | --- |
| `league-core` | 规则引擎：回合阶段、合约、计分、赛季结束、游戏回放以及计划中的提示系统背后的状态谓词 |
| `league-gen` | 桌子生成器，包括其公平性检查和种子池 |
| `league-ledger` | XRPL 测试网客户端。它拒绝任何非测试网的网络 |
| `league-standalone` | 用于训练模式的独立 `xrpld` 框架，在一个固定的、资源受限的容器中 |
| `league-season` | 测试网已公布的重置日期，以及一个用于检测未公布重置的检查。 |
| `league-relay` | 用于运行 AI 玩家参与的纸质测试的工具 |
| `league-tui` | 离线俱乐部模式终端主机：大厅设置、四区轮流游戏、隐私帘（可选 PIN 码）、限时回合、回合中途保存/恢复、赛季结束排行榜和游戏回放导出 |

## 进行一个赛季的游戏

离线 **俱乐部模式** 在您的终端上运行完整的 `24..=32` 回合轮流游戏（一台机器上 `4–6` 个真人或 `ClubBot` 座位）：

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **大厅设置：** 选择座位数（`4–6`，`Left`/`Right`），在 `Human` 和 `ClubBot` 之间切换每个座位（`Space`），输入 `0..=9999` 范围内一个明确的 `SeedPool::Dev` 种子（保留的测试种子和其他种子池会被按名称拒绝），选择回合时限（`180–300 s`，默认 `210 s`，或用 `Space` 选择 `Untimed`），并开关四位数的本桌幕布 PIN。在 **Start Club Mode Table** 上按 `Enter`。如果 `--save-dir` 中存在未完成的 `club-*.seal`，开局会被拒绝（`UNFINISHED_SEASON_EXISTS`）；导出之后，下一桌会得到下一个空闲的桌号，不会覆盖之前的存档。命令行参数（`--turn-clock`、`--untimed`、`--pins`）会覆盖与之冲突的大厅选择和已保存的 `.seal` 选择（`league-tui --help` 说明了优先级；在命令行用 `--pin` 传入 PIN 数字会被拒绝）。
- **回合按键：** `1`/`2`/`3`/`Tab`/`BackTab` 切换已启用的编辑标签页（`4: Diplomatic Desk (Disabled)` 保持禁用）；`F6` 在公共账本、编辑标签页和私人保险库之间循环焦点；`[`/`]` 循环切换行动类型或订单包的各部分；`Up`/`Down`/`PageUp`/`PageDown`/`Home` 选择或滚动；`Left`/`Right`/`+`/`-`/`Space` 调整字段；`e` 编辑自由文本覆盖；`n` 编辑个人私人保险库笔记；`Enter` 提交一个行动、暂存订单包的一部分，或运行 `p` What-If 沙盒；`Del`/`Backspace`/`X` 清除已暂存的订单；`c` 在公会商品名和三字母代码（`[CRED]`、`[TMB]`）之间切换；`r`/`F2` 打开清算编年史窗口；`Ctrl+l`/`Ctrl+L` 锁定你的回合并打开隐私幕布；`q`/`Esc`/`Ctrl+c` 干净地退出。
- **隐私幕布、静默时间和 PIN：** 每次交接、超时和赛季中的恢复都会打开公共的隐私幕布（就地用 `CSI 2 J` 清空备用屏幕，从不使用会清除回滚记录的 `CSI 3 J`），清空排队的输入，并在打开和抬起时都强制 `400 ms` 的输入静默时间（抬起时的静默时间不计入到达座位的计时）。按 `Space` 再按 `Enter` 抬起幕布；如果启用了 PIN，输入你被遮挡显示的四位数 PIN（只在幕布后输入——第 1 回合设置它，之后的交接验证它，输错时不会生成任何座位视图，连续 3 次输错会使幕布退回尚未准备输入的初始状态）。
- **计时：** 计时回合把每次的时限按 `75%` 子阶段 A / `25%` 子阶段 B 分配给全部 `H` 个人类座位（`floor(0.75 * B / H)` 秒和 `floor(0.25 * B / H)` 秒）。提前锁定会立即推进；子阶段 B 空闲超时按常任摄政的后备规则处理。
- **保存与恢复：** 使用 `--save-dir <DIR>` 时，主机会在每次幕布、首次提交 PIN 以及离开 `TalkAndMarket` 的清算时原子地自动保存。公共状态保存到 `club-<id>.save.json`；隐藏的任务、盐值、预先导出的 VRF 证明、笔记、未提交的订单包和 PIN 只保存在 `club-<id>.seal` 中。随时可以用 `cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json`（或 `--resume ./saves`，选择当前的 `.seal` 桌）恢复：赛季中的恢复总是落在当前人类座位的幕布后面，并重置回合计时（先推进排在前面的 `ClubBot` 游标），而赛季结束后的恢复直接进入公共的 `Session Standby` 揭示界面（`AwaitingMandateReveal`）或 `Completed` 的赛季结束标签页，方便你重试导出回放。同时传入 `--resume` 和另一个 `--save-dir` 时，如果该目录中有另一个 `club-*.seal` 文件（`UNFINISHED_SEASON_EXISTS`，任何桌号），或有同一桌号的 `club-<id>.save.json`、`.seal` 或 `.replay.json`（`SEASON_SAVE_EXISTS`），就会被拒绝；例外是恢复一个已结束的赛季、且该目录中已有它自己逐字节相同的常规 `club-<id>.replay.json`、没有冲突的 `.save.json` 或 `.seal`，这样在源 `.seal` 清除失败之后重试导出可以继续进行。一个只包含另一个桌号的已结束且已导出赛季的目录会被接受，不会覆盖任何内容。指定一个不存在或为空的 `--save-dir` 时，导出时会按需创建它，并在导出成功后仍然清除被恢复的 `.seal`。在赛季中恢复到另一个 `--save-dir` 会在那里继续赛季，并把原目录中的 `club-<id>.save.json` 和 `.seal` 留在原处。在同一次会话中导出回放也会清除原来的 `.seal`；如果你改为退出并稍后从新目录恢复，原来的 `.seal` 会保留，其中仍有该赛季的私密数据，并会让原目录的大厅继续拒绝开始新赛季（`UNFINISHED_SEASON_EXISTS`）。只有在你已退出且赛季已保存在新目录之后，才删除那份残留的副本。除了那份残留之外，请让每个赛季的 `club-<id>.save.json`、`.seal` 和 `.replay.json` 保持 `league-tui` 给它们的文件名和所在目录；要把赛季移到别处，请用 `--save-dir <EMPTY_DIR>` 恢复它，而不要手动重命名或移动文件。
- **赛季结束与导出回放：** 收盘钟声结束 `TalkAndMarket` 之后，逐步完成公共的待命任务揭示阶段（每个座位按 `Enter`/`Space`，在此之前不显示任何任务），进入有四个标签页的赛季结束界面（`1: Scoreboard`、`2: Mandates`、`3: Metrics & Marks`、`4: Replay Export`）。在第 4 个标签页按 `e` 或 `Enter`，写入并验证 `club-<id>.replay.json`，并清除私密的 `.seal` 文件（如果目标路径已被另一个赛季的回放占用，导出会以 `REPLAY_TARGET_EXISTS` 拒绝且不覆盖它；如果导出失败，`.seal` 会留在磁盘上，方便你重试）。

## 从源代码构建

您需要[Rust](https://rustup.rs/)。在第一次构建时，固定的工具链（1.100.0）将从`rust-toolchain.toml`自行安装。

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

CI在Linux上构建和测试，工作区在Windows上开发。其他平台未经测试。

Docker需要用于构建和运行本地俱乐部模式容器镜像，或运行独立的`xrpld`测试套件（除非您要求，否则跳过测试套件）。

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh`运行CI运行的检查，除了独立的Docker测试套件和Atlas地图检查。覆盖率、`cargo audit`、站点构建和保留的俱乐部模式容器烟雾测试仅在`cargo-llvm-cov`、`cargo-audit`、`npm`和Linux`docker`（具有POSIX `python3` `pty`/`fcntl`/`termios`）可用时运行，并且脚本会列出它跳过的任何检查。

## 在Docker中运行俱乐部模式（本地构建）

俱乐部模式（`league-tui`）容器镜像尚未发布到任何容器注册表；多阶段`Dockerfile`在本地构建，并通过离线CI烟雾测试进行验证（`scripts/smoke-container.sh`，不推送到注册表）。

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

在具有Docker Desktop的Windows上，在PowerShell中：

```powershell
docker build -t merchant-league:local .
mkdir saves
docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local
```

- **保存和挂载：** 在 Linux Docker 引擎中，默认镜像用户 `65532:65532` 无法写入属于您帐户的主机目录（或因 `-v` 路径缺失而由 `root` 创建的目录），因此预先创建 `./saves` 并传递 `--user "$(id -u):$(id -g)"` 可以使保存的文件归属于您的非 root 用户，而无需 root 权限或 `chmod 777`。 或者，使用 Docker 命名卷（`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`，无需 `--user`），它沿用镜像中归 `65532:65532` 所有的 `/saves`。 在 Docker Desktop（macOS 和 Windows）中，绑定挂载权限在虚拟机中进行了虚拟化，因此上述 PowerShell 命令以及在 macOS 上的相同绑定挂载，无需 `--user` 即可工作，而 WSL 2 Linux shell 可以运行上述 Linux `--user` 命令。 对于无 root 权限的 Docker 引擎，容器用户 ID 的映射方式不同（在容器内部，您自己的文件显示为 `root` 的，任何其他 ID 都会映射到从属 ID），因此上述 `--user` 命令不会授予容器您自己的帐户；请在那里使用命名卷。 Linux 版本的 Docker Desktop 在虚拟机中运行引擎，并通过其自身的机制与其共享文件夹；如果绑定挂载无法在那里保存，请也使用命名卷。 我们尚未测试无 root 权限的 Docker 引擎、Linux 版本的 Docker Desktop 和 macOS。
- **需要交互式终端：** 传递 `-it` 以进行交互式操作。 在未附加 TTY 的情况下启动会以状态码 `1` 退出，并给出错误码 `TTY_REQUIRED` 及其 `docker run --rm -it <image>` 提示；`--help` 和 `--version` 在没有 TTY 时以状态码 `0` 退出。
- **镜像包含的内容：** 从固定哈希值的 `rust:1.100.0-bookworm`（`@sha256:`）构建到固定哈希值的 `gcr.io/distroless/cc-debian12:nonroot`（`@sha256:`），默认情况下在 `WORKDIR /saves` 中为非 root `65532:65532`。 在这个固定的distroless（无发行版）基础镜像之上（该镜像不包含 shell 和包管理器），构建过程仅添加 `/usr/local/bin/league-tui`、`/usr/share/doc/merchant-league/LICENSE` 和一个由 `65532:65532` 拥有的空 `/saves` 目录——没有工具链、没有源代码文件，也没有网络调用（`--network=none`）。

## 安全与数据

- **它会访问什么。** 在离线俱乐部模式（`league-tui`）中，`--save-dir` 会写入公共 `club-<id>.save.json`、私有的同屏轮流（hot-seat）`club-<id>.seal`（在成功导出回放后删除）、经过验证的 `club-<id>.replay.json` 以及可选的 `--log-file` 元数据诊断信息（绝不会记录 PIN、保险库笔记、任务、暂存订单或提交盐值，并且会在所有四个日志级别中隐去 API/XRPL 凭据和密封的 JSON 键），所有这些都写入您自己的机器，没有任何网络调用。 在游戏模式（尚未构建）中，玩家的 Testnet 密钥保留在其自己的机器上，位于 git 忽略的 `.league/` 文件夹中，并且绝不会被记录。 纸质测试中继仅从 `OPENROUTER_API_KEY` 环境变量读取其 OpenRouter 密钥，并且在没有 `--live` 和信用上限的情况下不会进行任何 OpenRouter 调用。 它的本地席位拒绝任何 API 密钥，仅访问环回地址或您允许的 VM 地址（通过 `--allow-vm-host`），并且需要 GPU 秒数上限。 它将其记录保存在您提供的游戏文件夹中。
- **它不会访问什么。** 没有真实货币，没有主网，也没有任何超出上述文件夹之外的内容。 **没有遥测数据**：除了模式所需的调用（Testnet、公共随机数信标以及中继的 OpenRouter 或您允许的本地模型服务器）之外，没有任何内容会被发送到任何地方。
- **权限。** 在游戏模式下，需要访问 Testnet 的网络。 运行本地俱乐部模式容器时，需要 Docker 具有 `--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"`（或命名卷 `-v merchant-league-saves:/saves`）（仅访问 `/saves`），或者 Docker 的端口绑定到 `127.0.0.1`，仅用于独立的测试套件。
- **保护措施。** 每个 Testnet 连接都必须报告 Testnet 的网络 ID。 只要代码库中第三方引入（vendored）代码以外的地方写有任何主网主机名，一项 CI 检查就会失败；另一项检查会在文案把游戏描述为收入来源时失败。 审查涵盖了词表无法看到的内容。

请参阅 [`SECURITY.md`](SECURITY.md) 以了解威胁模型以及如何报告漏洞。

## 文档

- [手册](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [设计 v2](docs/design-v2.md)：规则、经济和两种模式
- [纸质测试](docs/paper-test.md)：第一次测试，在纸上进行，使用 AI 席位
- [独立的 `xrpld` 测试套件](docs/standalone-xrpld.md)
- [记录](docs/receipts/)：实时的 Testnet 检查。 Testnet 会不时被清除（下次在 2026-11-25），因此其交易哈希值之后将无法解析；它们记录的内容以及测试向量将作为证据保留。

## 支持

这是一个正在积极设计中的原型。 欢迎在 [问题跟踪器](https://github.com/mcp-tool-shop-org/merchant-league/issues) 中提出问题和想法。 尚未发布任何版本，并且没有支持保证。 请参阅 [CHANGELOG](CHANGELOG.md) 以了解进度。

## 许可证

[MIT](LICENSE)。

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
