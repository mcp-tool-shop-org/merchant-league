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

4人から6人でプレイする、ゆっくりとしたペースの取引と政治をテーマにしたゲーム。XRP Ledgerの**テストネット**上でプレイされます。プレイヤーは人間でもAIでも構いません。同じテーブルで同じルールに従ってプレイします。

**ステータス：プロトタイプ、設計段階。** ルールエンジン、テーブルジェネレーター、リプレイ機能は構築され、テストされています。ゲーム自体は**まだプレイできません**。ゲームクライアントはなく、リリースもされていません。テストネットのみで、実際の資金は使用せず、メインネットへの移行もありません。

## ゲームについて

- **商人たちの輪。** 各プレイヤーは独自の原材料を生産し、他のプレイヤーから必要な資源を調達して、運営費を支払い、工房を運営し、共同プロジェクトに供給する必要があります。誰もが単独で成功することはできず、少数のグループでも同様です。
- **同時ターン制。** 各ターンは3つのフェーズで構成されます。プレイヤーはオープンマーケットで話し合い、取引を行い、その後、各プレイヤーは秘密裏に注文を決定し、最後にすべてのプレイヤーの注文が同時に、同じルールに基づいて解決されます。
- **政治。** 評議会は関税を設定し、カルテルは非公開の市場を開設し、特許はシーズンを通してプレイヤー間でローテーションされます。
- **24ターンから32ターンのシーズン。** 供給への影響は2ターン前に発表され、最後のターンはランダムに決定されるため、誰も完璧な最終戦略を立てることはできません。
- **スコアリング。** 各プレイヤーは、ゲーム終了時に純資産をスコアし、さらにゲーム開始時に配布された秘密の目的カードが加算されます。すべてのプレイヤーは、最後のターンまでプレイする理由を持ち続けます。
- **チェックされたテーブル。** すべてのテーブルはシードから生成され、配布前にチェックされます。各プレイヤーの開始値は、他のプレイヤーの値から3%以内に収まる必要があり、特定のプレイヤーがグループ全体に与える影響で評価された場合、7%以内に収まる必要があります。

完全な設計は[`docs/design-v2.md`](docs/design-v2.md)にあります。

## プレイ方法

| モード | 実行環境 | 対象者 |
| --- | --- | --- |
| **ゲームモード** | パブリックXRPLテストネット。1日に約2ターン進行します。 | 人間。単独またはAIプレイヤーと一緒。 |
| **トレーニングおよび評価モード** | ローカルのDockerコンテナに配置された、独立した`xrpld`ノード。数ミリ秒でレジャーを閉じます。 | AIプレイヤー：トレーニング、ベンチマーク、リプレイ |

両方のモードは、1つのルールエンジンを共有します。ゲームは、シードとログから正確にリプレイされます。

## 構築されたもの

| クレート | その機能 |
| --- | --- |
| `league-core` | ルールエンジン：ターンのフェーズ、契約、スコアリング、シーズンの終了、リプレイ、および計画されているヒントシステムの背後にある状態述語 |
| `league-gen` | 公平性チェックとシードプールを備えたテーブルジェネレーター |
| `league-dice` | 単一のプレイヤーが予測または操作できないダイス：コミットとリビール、およびパブリックランダムネスビーコンと組み合わせる |
| `league-ledger` | XRPLテストネットクライアント。テストネット以外のネットワークは拒否します。 |
| `league-standalone` | トレーニングモード用の独立した`xrpld`ハーネス。ピン留めされ、リソース制限のあるコンテナに配置されています。 |
| `league-season` | シーズン境界。テストネットのリセットに従います。 |
| `league-relay` | AIプレイヤーを使用したペーパートレーニングを実行したツール |
| `league-tui` | ターミナルクライアント。現時点ではタイトル画面のみです。 |

## ソースコードからビルド

[Rust](https://rustup.rs/)が必要です。ピン留めされたツールチェーン（1.100.0）は、最初のビルド時に`rust-toolchain.toml`から自動的にインストールされます。

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

CIはLinuxでビルドとテストを実行し、ワークスペースはWindowsで開発されています。他のプラットフォームはテストされていません。

Dockerは、独立した`xrpld`ハーネスでのみ必要であり、要求しない限りテストはスキップされます。

```bash
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked
```

`scripts/verify.sh`は、DockerハーネスとAtlasマップチェックを除き、CIが実行するチェックを実行します。

## セキュリティとデータ

- **アクセス対象。** ゲームモード（まだ構築されていません）では、プレイヤーのテストネットキーは、gitで無視される`.league/`フォルダーに保存され、ログに記録されることはありません。ペーパーテスト用のリレーは、そのOpenRouterキーを`OPENROUTER_API_KEY`環境変数からのみ読み取り、`--live`とクレジット制限なしにネットワーク呼び出しを行わず、トランスクリプトをゲームフォルダーに保存します。
- **アクセスしない対象。** 実際の資金、メインネット、および上記のフォルダー外のものは一切使用しません。**テレメトリーはありません**。モードが必要とする呼び出し（テストネット、パブリックランダムネスビーコン、およびリレーの場合はOpenRouter）を除き、何も送信されません。
- **権限。** ゲームモードでは、テストネットへのネットワークアクセスが必要です。スタンドアロンハーネスの場合は、Dockerを使用し、ポートを`127.0.0.1`にバインドします。
- **保護。** すべてのテストネット接続は、テストネットのネットワークIDを報告する必要があります。CIは、メインネットのホスト名がある場合に失敗し、2回目のトリップワイヤは、ゲームを収入源として表現するコピーがある場合に失敗します。

脅威モデルと脆弱性の報告方法については、[`SECURITY.md`](SECURITY.md)を参照してください。

## ドキュメント

- [ハンドブック](https://mcp-tool-shop-org.github.io/merchant-league/handbook/)
- [デザイン v2](docs/design-v2.md)：ルール、経済システム、および2つのモード
- [ペーパーテスト](docs/paper-test.md)：最初のプレイテスト。AIが対戦相手を務める紙上でのテスト
- [スタンドアロン `xrpld` ハーネス](docs/standalone-xrpld.md)
- [レシート](docs/receipts/)：ライブテストネットのチェック。テストネットは定期的にリセットされます（次回の予定は2026年11月25日）。そのため、リセット後はトランザクションハッシュの解決が停止します。記録された内容とテストベクトルは、証拠として残ります。

## サポート

これは、現在設計中のプロトタイプです。問題やアイデアは、[イシュートラッカー](https://github.com/mcp-tool-shop-org/merchant-league/issues)で歓迎します。まだリリースはなく、サポートの保証もありません。進捗状況については、[CHANGELOG](CHANGELOG.md)を参照してください。

## ライセンス

[MIT](LICENSE)。

---

<p align="center">Built by <a href="https://mcp-tool-shop.github.io/">MCP Tool Shop</a></p>
