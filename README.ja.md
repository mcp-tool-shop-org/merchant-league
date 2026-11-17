<p align="center">
  <a href="README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.hi.md">हिन्दी</a> | <a href="README.it.md">Italiano</a> | <a href="README.pt-BR.md">Português (BR)</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/mcp-tool-shop-org/brand/main/logos/merchant-league/readme.png" width="400" alt="Merchant League">
</p>

<p align="center">
  <a href="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml"><img src="https://github.com/mcp-tool-shop-org/merchant-league/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://mcp-tool-shop-org.github.io/merchant-league/"><img src="https://img.shields.io/badge/landing-page-2ea44f.svg" alt="Landing Page"></a>
</p>

# マーチャントリーグ

4人から6人でプレイする、ゆっくりとしたペースの取引と政治をテーマにしたゲーム。XRP Ledgerの**テストネット**上でプレイします。プレイヤーは人間でもAIでも構いません。同じテーブルで同じルールに従ってプレイします。

**ステータス：プロトタイプ、設計段階。** ルールエンジン、テーブルジェネレーター、リプレイ機能は構築され、テストされています。ゲーム自体は**まだプレイできません**：ゲームクライアントはなく、リリースもされていません。テストネットのみ：実際の資金は使用せず、メインネットへの移行もありません。

## ゲームについて

- **商人たちの輪。** 各プレイヤーは独自の原材料を生産し、他のプレイヤーから必要な物資を調達して、維持費を支払い、工房を運営し、共同プロジェクトに供給する必要があります。誰もが単独で成功することはできず、少数のグループでも同様です。
- **同時ターン制。** 各ターンには3つのフェーズがあります。プレイヤーはオープンマーケットで話し合い、取引を行い、その後、各プレイヤーは秘密裏に注文を決定し、最後にすべてのプレイヤーの注文が同時に、同じルールに基づいて解決されます。
- **政治。** 評議会は関税を設定し、カルテルは非公開の市場を開設し、特許はシーズンを通してプレイヤー間でローテーションされます。
- **24ターンから32ターンのシーズン。** 供給停止、関税免除、需要の急増、または品質の悪化などの「ショック」は、発生する1ターン前に公開されます。最後のターンはランダムに決定されるため、誰も完璧な最終戦略を立てることはできません。
- **スコアリング。** 各プレイヤーは、ゲーム終了時に純資産をスコアし、さらにゲーム開始時に配布された非公開の目的カードに基づいてスコアが加算されます。すべてのプレイヤーは、最後のターンまでプレイする理由を持ち続けます。
- **チェックされたテーブル。** 各テーブルはシードから生成され、配布前にチェックされます。各プレイヤーの開始値は、テーブルの平均値の3%以内である必要があり、また、プレイヤーがグループ全体に与える影響に基づいて評価された場合、平均値の7%以内である必要があります。

完全な設計は[`docs/design-v2.md`](docs/design-v2.md)にあります。

## プレイ方法

| モード | 実行環境 | 対象者 |
| --- | --- | --- |
| **ゲームモード** | パブリックXRPLテストネット。1日に約2ターンプレイできます。 | 人間。単独またはAIプレイヤーと一緒。 |
| **トレーニングおよび評価モード** | ローカルのDockerコンテナに配置された、独立した`xrpld`ノード。数ミリ秒でレジャーを閉じます。 | AIプレイヤー：トレーニング、ベンチマーク、リプレイ |

両方のモードは、1つのルールエンジンを共有します。ゲームは、シードとログから正確にリプレイされます。

## 構築されたもの

| クレート | 機能 |
| --- | --- |
| `league-core` | ルールエンジン：ターンのフェーズ、契約、スコアリング、シーズンの終了、リプレイ、および計画されているヒントシステムの背後にある状態述語 |
| `league-gen` | テーブルジェネレーター。公平性のチェックとシードプールの機能が含まれます。 |
| `league-dice` | 現時点ではスタブのみ。ゲーム自体のランダム性（コミットとリビール、ゲームモードではパブリックdrandビーコンと、トレーニングモードでは検証可能なランダム関数と組み合わせて使用）は、`league-core`で構築されています。 |
| `league-ledger` | XRPLテストネットクライアント。テストネット以外のネットワークは拒否します。 |
| `league-standalone` | トレーニングモード用の独立した`xrpld`ハーネス。ピン留めされ、リソース制限が設定されたコンテナに配置されます。 |
| `league-season` | シーズン境界。テストネットのリセットに従います。 |
| `league-relay` | AIプレイヤーとのペーパーテストを実行したツール。 |
| `league-tui` | ターミナルクライアント。現時点ではタイトル画面のみです。 |

## ソースコードからビルド

[Rust](https://rustup.rs/)が必要です。ピン留めされたツールチェーン（1.100.0）は、最初のビルド時に`rust-toolchain.toml`から自動的にインストールされます。

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

CIはLinuxでビルドとテストを実行し、ワークスペースはWindowsで開発されています。他のプラットフォームはテストされていません。

Dockerは、独立した`xrpld`ハーネスでのみ必要であり、そのテストは、要求しない限りスキップされます。

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`bash scripts/verify.sh`は、CIが実行するチェックを実行します。ただし、DockerハーネスとAtlasマップチェックは除きます。カバレッジと`cargo audit`は、`cargo-llvm-cov`と`cargo-audit`がインストールされている場合にのみ実行され、スクリプトはスキップされたチェックをすべて記録します。

## セキュリティとデータ

- **アクセス範囲。** ゲームモード（まだ構築されていません）では、プレイヤーのテストネットキーは、gitで無視される`.league/`フォルダに保存され、ログに記録されることはありません。ペーパーテスト用のリレーは、そのOpenRouterキーを`OPENROUTER_API_KEY`環境変数からのみ読み取り、`--live`とクレジット制限がない限り、いかなるOpenRouter呼び出しも行いません。ローカルのプレイヤーは、APIキーを拒否し、ループバックアドレスまたは`--allow-vm-host`で許可されたVMアドレスにのみアクセスし、GPU秒数の制限が必要です。トランスクリプトは、指定されたゲームフォルダに保存されます。
- **アクセスしない範囲。** 実際の資金、メインネット、および上記のフォルダ外のものは一切使用しません。**テレメトリーは存在しません**：モードが必要とする呼び出し（テストネット、パブリックランダムネスビーコン、およびリレーの場合はOpenRouterまたは許可されたローカルモデルサーバー）を除いて、何も送信されません。
- **権限。** ゲームモードでは、テストネットへのネットワークアクセスが必要です。スタンドアロンハーネスの場合は、ポートを`127.0.0.1`にバインドしたDockerが必要です。
- **保護。** すべてのテストネット接続は、テストネットのネットワークIDを報告する必要があります。CIは、メインネットのホスト名がある場合に失敗し、2回目のトリップワイヤは、ゲームを収入源として表現するコピーがある場合に失敗します。

脅威モデルと脆弱性の報告方法については、[`SECURITY.md`](SECURITY.md)を参照してください。

## ドキュメント

- [ハンドブック](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [デザインv2](docs/design-v2.md)：ルール、経済、および2つのモード
- [ペーパーテスト](docs/paper-test.md)：最初のプレイテスト。AIプレイヤーとペーパーでプレイしました。
- [スタンドアロン`xrpld`ハーネス](docs/standalone-xrpld.md)
- [レシート](docs/receipts/)：ライブテストネットチェック。テストネットは定期的にワイプされます（次回は2026年11月25日）。そのため、トランザクションハッシュはその後解決されなくなります。ただし、記録された内容とテストベクトルは、証拠として残ります。

## サポート

これは、現在設計中のプロトタイプです。問題やアイデアは、[イシュートラッカー](https://github.com/mcp-tool-shop-org/merchant-league/issues)で歓迎します。現時点ではリリースはなく、サポートも保証されていません。進捗状況については、[CHANGELOG](CHANGELOG.md)を参照してください。

## ライセンス

[MIT](LICENSE)。

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
