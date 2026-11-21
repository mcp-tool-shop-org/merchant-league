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

4～6人のプレイヤーが参加する、ゆっくりとしたペースで進行する取引と政治をテーマにしたゲーム。XRP Ledgerの**テストネット**上でプレイします。プレイヤーは人間でもAIでも構いません。同じテーブルで同じルールに従ってプレイします。

**ステータス：プロトタイプ段階、現在設計中。** `league-tui`におけるオフラインの「クラブモード」は、1つの共有端末でソースコードからプレイ可能（ロビーからシーズン終了時のスコアボードとリプレイのエクスポートまで、人間と`ClubBot`を合わせて4～6席）。プライベートなささやき機能は`v0.1.0`より後に先送りされています。パブリックテストネット版のゲームモードとコンテナイメージはまだ作成されておらず、**まだリリースされていません**。テストネットのみ：実際の通貨は使用せず、メインネットへの移行もありません。

## ゲームについて

- **商人たちの輪。** 各プレイヤーは独自の原材料を生産し、他のプレイヤーから必要な物資を調達して、維持費を支払い、工房を運営し、共同プロジェクトに供給する必要があります。誰もが単独で成功することはできず、少数のグループでも同様です。
- **同時ターン制。** 各ターンには3つのフェーズがあります。プレイヤーはオープンマーケットで話し合い、取引を行い、その後、各プレイヤーは秘密裏に注文を決定し、最後にすべてのプレイヤーの注文が同時に、同じルールに基づいて解決されます。
- **政治。** 評議会は関税を設定し、カルテルは非公開の市場を開設し、チャーター（特権の付与）はシーズンを通してプレイヤー間でローテーションされます。
- **24ターンから32ターンのシーズン。** 供給停止、関税免除、需要の急増、または品質の悪化などの「ショック」は、発生する1ターン前に公開されます。最後のターンはランダムに決定されるため、誰も完璧な最終戦略を立てることはできません。
- **スコアリング。** 各プレイヤーは、ゲーム終了時に純資産をスコアし、さらにゲーム開始時に配布された非公開の目的カードに基づいてスコアが加算されます。すべてのプレイヤーは、最後のターンまでプレイする理由を持ち続けます。
- **チェックされたテーブル。** 各テーブルはシードから生成され、配布前にチェックされます。各プレイヤーの開始値は、テーブルの平均値の3%以内である必要があり、また、席のあらゆるグループに加える価値で各席を評価した場合も、平均値の7%以内である必要があります。

完全な設計は[`docs/design-v2.md`](docs/design-v2.md)にあります。

## プレイ方法

| モード | 実行環境 | 対象者 |
| --- | --- | --- |
| **ゲームモード** | パブリックXRPLテストネット。1日に約2ターン進行します。 | 人間（人間のみ、またはAI席と同卓） |
| **トレーニングおよび評価モード** | ローカルのDockerコンテナで実行される、独立した`xrpld`ノード。数ミリ秒で帳簿を閉じます。 | AIプレイヤー：トレーニング、ベンチマーク、リプレイ |

両方のモードは、1つのルールエンジンを共有します。ゲームは、そのシードとログから正確にリプレイされます。

## 構築済みのもの

| クレート | 機能 |
| --- | --- |
| `league-core` | ルールエンジン：ターンのフェーズ、契約、スコアリング、シーズンの終了、リプレイ、および計画されているヒントシステムの背後にある状態述語 |
| `league-gen` | 公平性のチェックとシードプールを備えたテーブルジェネレーター |
| `league-ledger` | XRPLテストネットクライアント。テストネット以外のネットワークは拒否します。 |
| `league-standalone` | トレーニングモード用の独立した`xrpld`ハーネス。固定された、リソース制限のあるコンテナで実行されます。 |
| `league-season` | テストネットの発表済みリセット日と、発表されていないリセットを検出するチェック。 |
| `league-relay` | AIプレイヤーとのペーパーテストを実行したツール。 |
| `league-tui` | オフラインクラブモードターミナルホスト：ロビーのセットアップ、4つのゾーンでのホットシートターン、オプションのPINを備えたプライバシーカーテン、時間制限付きのパス、ターンの途中での保存/再開、シーズン終了時のスコアボード、リプレイのエクスポート |

## シーズンをプレイ

オフライン**クラブモード**は、端末で完全な`24..=32`ターンのホットシートシーズンを実行します（1台の機械で`4–6`人の人間または`ClubBot`のAI）。

```bash
cargo run -p league-tui --locked -- --save-dir ./saves
```

- **ロビー設定:** 参加人数を選択（`4–6`、`Left`/`Right`）、各席を`Human`と`ClubBot`の間で切り替え（`Space`）、明確な`SeedPool::Dev`シードを`0..=9999`に入力（予約済みのテストシードやその他のプールは名前で拒否されます）、ターンの予算を選択（`180–300 s`、デフォルトは`210 s`、または`Space`で`Untimed`を選択）、4桁のテーブルローカルのカーテンPINを切り替えます。**Start Club Mode Table**で`Enter`を押します。未完了の`club-*.seal`が`--save-dir`に存在した状態で開始することは拒否されます（`UNFINISHED_SEASON_EXISTS`）。エクスポートされると、次のテーブルは以前の保存を上書きすることなく、次の空いているテーブルIDを受け取ります。コマンドラインフラグ（`--turn-clock`、`--untimed`、`--pins`）は、競合するロビーと保存された`.seal`の選択を上書きします（優先順位は`league-tui --help`に記載されています。CLIでPINの数字を`--pin`経由で渡すことは拒否されます）。
- **ターンのキー:** `1`/`2`/`3`/`Tab`/`BackTab`で、有効なコンポーザータブを切り替えます（`4: Diplomatic Desk (Disabled)`は無効のまま）。`F6`で、パブリックレジャー、コンポーザータブ、プライベートボルト間をフォーカスを移動します。`[`/`]`で、アクションの種類またはバンドルセクションを切り替えます。`Up`/`Down`/`PageUp`/`PageDown`/`Home`で、選択またはスクロールします。`Left`/`Right`/`+`/`-`/`Space`で、フィールドを調整します。`e`で、フリーテキストのオーバーライドを編集します。`n`で、個人のプライベートボルトのメモを編集します。`Enter`で、アクションを送信し、バンドルセクションをステージングし、または`p`の「もしも」サンドボックスを実行します。`Del`/`Backspace`/`X`で、ステージングされた注文をクリアします。`c`で、ギルドの商品名と3文字のコードを切り替えます（`[CRED]`、`[TMB]`）。`r`/`F2`で、スウィープクロニクルモーダルを開きます。`Ctrl+l`/`Ctrl+L`で、パスをロックし、プライバシーカーテンを開きます。`q`/`Esc`/`Ctrl+c`で、正常に終了します。
- **プライバシーカーテン、デッドタイム、PIN:** 各ハンドオフ、タイムアウト、およびシーズン中の再開時に、パブリックプライバシーカーテンが開きます（代替画面を`CSI 2 J`でクリアし、決して`CSI 3 J`のスクロールバックパージは行いません）。キューに登録された入力を排出し、開くときと上げるときの両方で`400 ms`の入力デッドタイムを適用します（上げるときのデッドタイムは、到着した席のクロックにはカウントされません）。`Space`を押してから`Enter`を押すとカーテンが上がります。PINが有効になっている場合は、マスクされた4桁のPINを入力します（カーテンの後ろでのみ入力されます。最初のターンで設定され、後続のハンドオフで検証されます。誤入力では席の画面は一切作られず、3回ミスするとカーテンは最初の状態に戻ります）。
- **クロック:** 時間制限のあるターンは、各予算`75%`のサブパスA / `25%`のサブパスBをすべての`H`人の席に分割します（`floor(0.75 * B / H)`秒と`floor(0.25 * B / H)`秒）。早期にロックすると、すぐに進みます。アイドル状態のサブパスBのタイムアウトは、スタンディングリージェントのフォールバックドクトリンによって解決されます。
- **保存と再開:** `--save-dir <DIR>`を使用すると、ホストはすべてのカーテン、最初のPINコミット、および`TalkAndMarket`を抜けるスウィープの時点で、アトミックに自動保存します。パブリック状態は`club-<id>.save.json`に保存されます。非公開のマンデート、ソルト、事前に導出されたVRF証明、メモ、コミットされていないバンドル、およびPINは、すべて`club-<id>.seal`にのみ保存されます。いつでも`cargo run -p league-tui --locked -- --resume ./saves/club-1.save.json`（または`--resume ./saves`、アクティブな`.seal`テーブルを選択）で再開できます。シーズン中の再開は、常にアクティブな人の席のカーテンの後ろに、新しいパスクロックとともに表示されます（先行する`ClubBot`カーソルを最初に進めます）。シーズン後の再開は、パブリックの`Session Standby`リビール画面（`AwaitingMandateReveal`）または`Completed`のシーズン終了タブに直接表示されるため、リプレイのエクスポートを再試行できます。異なる`--save-dir`とともに`--resume`を渡すことは、そのディレクトリに別の`club-*.seal`ファイル（`UNFINISHED_SEASON_EXISTS`、任意のテーブルID）が含まれている場合、またはそれ以外に同じテーブルIDの`club-<id>.save.json`、`.seal`、または`.replay.json`が含まれている場合は拒否されます（`SEASON_SAVE_EXISTS`、ただし、完了したシーズンを再開する場合、そのシーズン自身のバイト単位で同一の通常の`club-<id>.replay.json`がすでにそのディレクトリに存在し、競合する`.save.json`または`.seal`がない場合は除きます。したがって、ソースの`.seal`のワイプに失敗した場合のエクスポートの再試行は続行されます）。別のテーブルIDの、完了してエクスポート済みのシーズンのみを含むディレクトリは、何も上書きせずに受け入れられます。存在しないまたは空の`--save-dir`をターゲットにすると、エクスポート時に必要に応じて作成され、再開された`.seal`は正常にエクスポートされた後にワイプされます。別の`--save-dir`へのシーズン中の再開は、そこでシーズンを継続し、元のディレクトリの`club-<id>.save.json`と`.seal`はそのままにします。その同じセッションでリプレイをエクスポートすると、元の`.seal`もワイプされます。代わりに終了して、後で新しいディレクトリから再開すると、元の`.seal`は残り、そのシーズンの非公開の値を保持し続けるため、元のディレクトリのロビーは新しいシーズンの開始を拒否し続けます（`UNFINISHED_SEASON_EXISTS`）。残ったコピーは、終了してシーズンが新しいディレクトリに保存された後にのみ削除してください。それ以外の場合、各シーズンの`club-<id>.save.json`、`.seal`、および`.replay.json`は、`league-tui`が与えた名前とディレクトリの下に保持します。シーズンを別の場所に移動するには、ファイルを手動で名前変更または移動するのではなく、`--save-dir <EMPTY_DIR>`を指定して再開してください。
- **シーズンの終了とリプレイのエクスポート:** クロージングベルが`TalkAndMarket`を閉じると、パブリックのスタンバイ・マンデート公開ステージ（各席につき`Enter`/`Space`、それまでマンデートは一切表示されません）をステップ実行して、4つのタブのシーズン終了画面（`1: Scoreboard`、`2: Mandates`、`3: Metrics & Marks`、`4: Replay Export`）に最終的に移行します。タブ4で、`e`または`Enter`を押して、`club-<id>.replay.json`を書き込み、検証し、プライベートの`.seal`ファイルをワイプします（別のシーズンのリプレイがすでにそのターゲットパスを占有している場合、エクスポートはそれを上書きせずに`REPLAY_TARGET_EXISTS`で拒否します。エクスポートが失敗した場合、`.seal`はディスクに残るため、再試行できます）。

## ソースコードからビルドする

[Rust](https://rustup.rs/)が必要です。指定されたツールチェーン（1.100.0）は、最初のビルド時に`rust-toolchain.toml`から自動的にインストールされます。

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

CIはLinux上でビルドとテストを行い、ワークスペースはWindows上で開発されています。その他のプラットフォームについてはテストされていません。

ローカルの Club Mode コンテナーイメージをビルドおよび実行するため、またはスタンドアロンの `xrpld` ハーネスを実行するために Docker が必要です（ハーネスのテストは、明示的に要求しない限りスキップされます）。

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh`は、スタンドアロンのDocker環境とAtlasマップチェックを除き、CIで実行されるすべてのチェックを実行します。カバレッジ、`cargo audit`、サイトのビルド、および保留中のClub Modeコンテナの簡易テストは、`cargo-llvm-cov`、`cargo-audit`、`npm`、およびLinux `docker`（POSIX `python3` `pty`/`fcntl`/`termios`を搭載）が利用可能な場合にのみ実行され、スクリプトは実行をスキップしたチェックの名前を記録します。

## Dockerでクラブモードを実行する（ローカルビルド）

クラブモード（`league-tui`）のコンテナイメージは、まだどのコンテナレジストリにも公開されていません。マルチステージのビルド（`Dockerfile`）はローカルで行われ、オフラインのCIによる簡易テスト（`scripts/smoke-container.sh`、レジストリへのプッシュは行われません）によって検証されます。

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

Docker Desktopを搭載したWindows環境で、PowerShellの場合：

```powershell
docker build -t merchant-league:local .
mkdir saves
docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local
```

- **保存とマウント:** Linux の Docker Engine では、イメージの既定ユーザー `65532:65532` は、あなたのアカウントが所有するホストのディレクトリ（または `-v` のパスが存在しないために `root` が作成したディレクトリ）に書き込めません。そのため、`./saves` を先に作成して `--user "$(id -u):$(id -g)"` を渡せば、`chmod 777` や root を使わずに、保存データをあなたの非 root ユーザーの所有のままにできます。代わりに Docker の名前付きボリューム（`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`、`--user` なし）を使うこともでき、その場合はイメージ内の `65532:65532` が所有する `/saves` を引き継ぎます。Docker Desktop（macOS と Windows）ではバインドマウントの権限が VM をまたいで仮想化されるため、上の PowerShell コマンドと macOS での同じバインドマウントは `--user` なしで動作し、WSL 2 の Linux シェルでは上の Linux 用 `--user` コマンドを実行できます。rootless の Docker Engine ではコンテナのユーザー ID の対応が異なる（コンテナ内ではあなた自身のファイルが `root` のものとして見え、それ以外の ID は従属 ID に対応する）ため、上の `--user` コマンドではコンテナにあなた自身のアカウントが与えられません。その場合は名前付きボリュームを使ってください。Docker Desktop for Linux はエンジンを VM 内で動かし、独自の仕組みでフォルダーを共有します。そこでバインドマウントに保存できない場合も、名前付きボリュームを使ってください。rootless の Docker Engine、Docker Desktop for Linux、macOS は、私たちはテストしていません。
- **対話型ターミナルが必要:** 対話的にプレイするには `-it` を渡してください。TTY を接続せずに起動すると、コード `TTY_REQUIRED` と `docker run --rm -it <image>` のヒントを表示して `1` で終了します。`--help` と `--version` は TTY なしでも `0` で終了します。
- **イメージの中身:** ダイジェストで固定した `rust:1.100.0-bookworm`（`@sha256:`）でビルドし、ダイジェストで固定した `gcr.io/distroless/cc-debian12:nonroot`（`@sha256:`）の上に置いています。既定では非 root の `65532:65532` として `WORKDIR /saves` で動作します。固定した distroless ベース（シェルもパッケージマネージャーも含みません）の上に、ビルドが追加するのは `/usr/local/bin/league-tui`、`/usr/share/doc/merchant-league/LICENSE`、`65532:65532` が所有する空の `/saves` ディレクトリだけです。ツールチェーンもソースファイルも含まず、ネットワーク呼び出しは一切ありません（`--network=none`）。

## セキュリティとデータ

- **アクセスするもの:** オフラインのClub Mode（`league-tui`）では、`--save-dir`は、パブリックな`club-<id>.save.json`、プライベートなホットシート`club-<id>.seal`（正常なリプレイのエクスポート時に削除）、検証済みの`club-<id>.replay.json`、およびオプションの`--log-file`メタデータ診断（PIN、ボルトのメモ、マンデート、ステージングされた注文、またはコミットソルトをログに記録せず、4つすべてのログレベルでAPI/XRPLの認証情報と封印されたJSONキーをマスク）を、ネットワーク呼び出しなしで、自分のマシンに書き込みます。ゲームモード（未実装）では、プレイヤーのTestnetキーは自分のマシン上の、gitで無視される`.league/`フォルダーにとどまり、ログに記録されることはありません。ペーパーテストリレーは、そのOpenRouterキーを、`OPENROUTER_API_KEY`環境変数からのみ読み取り、`--live`とクレジット制限なしで、OpenRouter呼び出しを行いません。ローカルのシートは、APIキーを拒否し、ループバックアドレスまたは、`--allow-vm-host`で許可されたVMアドレスにのみアクセスし、GPU秒の制限が必要です。トランスクリプトは、指定されたゲームフォルダーに保存されます。
- **アクセスしないもの:** 実際の通貨、Mainnet、および上記のフォルダー外のものは一切ありません。**テレメトリーはありません**。モードが必要とする呼び出し（Testnet、パブリックなランダム性ビーコン、およびリレーの場合は、OpenRouterまたは許可されたローカルモデルサーバー）を除いて、どこにも送信されません。
- **権限:** ゲームモードでのTestnetへのネットワークアクセス。ローカルのClub Modeコンテナを実行する場合（`/saves`にのみアクセス）、Dockerに`--network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves"`（または名前付きボリューム`-v merchant-league-saves:/saves`）が必要です。スタンドアロンハーネスの場合は、Dockerのポートを`127.0.0.1`にバインドする必要があります。
- **保護:** すべてのTestnet接続は、TestnetのネットワークIDを報告する必要があります。CIトリップワイヤは、リポジトリ内の外部取り込み（vendored）コード以外の場所にMainnetホスト名が書かれていると失敗し、2つ目のトリップワイヤは、ゲームを収入源として描く文面があると失敗します。レビューでは、単語リストでは確認できない内容をカバーします。

脅威モデルと脆弱性の報告方法については、[`SECURITY.md`](SECURITY.md)を参照してください。

## ドキュメント

- [ハンドブック](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [デザインv2](docs/design-v2.md)：ルール、経済、および2つのモード
- [ペーパーテスト](docs/paper-test.md)：最初のプレイテスト。AIプレイヤーとペーパーでプレイしました。
- [スタンドアロン`xrpld`ハーネス](docs/standalone-xrpld.md)
- [レシート](docs/receipts/)：ライブテストネットチェック。テストネットは定期的にワイプされます（次回は2026年11月25日）。そのため、トランザクションハッシュはその後解決されなくなります。ただし、記録された内容とテストベクトルは、証拠として残ります。

## サポート

これは、現在設計中のプロトタイプです。問題やアイデアは、[イシュートラッカー](https://github.com/mcp-tool-shop-org/merchant-league/issues)で歓迎します。まだリリースはなく、サポートの保証もありません。進捗状況については、[CHANGELOG](CHANGELOG.md)を参照してください。

## ライセンス

[MIT](LICENSE)。

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
