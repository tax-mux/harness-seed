# HarnessSeed

組み込み用の ReAct ハーネス（Rust クレート `harness-seed`）。**チャット UI ではなく**、既存アプリに載せるエージェント層の「種」です。ライブラリと CLI（`harness-seed`）を同じクレートで提供します。

## 必要環境

- [Rust](https://www.rust-lang.org/tools/install)（`rustup` 推奨）
- Cargo（Rust ツールチェーンに同梱）

```bash
rustc --version
cargo --version
```

## プロジェクト構成

```
harness-seed/
├── Cargo.toml
├── config/
│   ├── config.json       # 実行時の正本（編集・上書きする）
│   ├── samples/          # コネクタ別ひな形（config.*.json）
│   └── README.md
├── doc/             # ドキュメント（現行仕様 + ideas/）
│   └── ideas/       # 未実装の設計メモ
├── src/
│   ├── main.rs      # CLI エントリポイント
│   └── lib.rs       # ライブラリ本体・公開 API
├── tests/           # 統合テスト
└── benches/         # ベンチマーク（必要に応じて追加）
```

### ドキュメント

| ファイル | 内容 |
|----------|------|
| [doc/agent-minimum-action-unit.md](doc/agent-minimum-action-unit.md) | AIエージェントの最少行動単位（図解） |
| [doc/react-implementation.md](doc/react-implementation.md) | 現行 ReAct 実装の構成・フロー・制限 |
| [doc/advance-loop.md](doc/advance-loop.md) | 外側推進ループ（ロングコンテキスト分割） |
| [doc/wire-protocol.md](doc/wire-protocol.md) | JSON Lines ワイヤプロトコル（`--json`） |
| [doc/builtin_tools/README.md](doc/builtin_tools/README.md) | 組み込みツール仕様（1 ツール 1 ファイル） |
| [doc/context-memory-mapping.md](doc/context-memory-mapping.md) | コンテキストの用途別マッピング（図解） |
| [doc/ideas/README.md](doc/ideas/README.md) | 未実装の設計メモ一覧（具体案は ideas 内のみ） |

## 使い方

### ビルド

```bash
cargo build
```

リリースビルド:

```bash
cargo build --release
```

### 実行

対話型 ReAct REPL:

```bash
cargo run
# 例: help / echo hello / time / 任意の文（Thought→echo→Answer）
# 詳細ログ: cargo run -- -v
# JSON Lines REPL: cargo run -- --json  （doc/wire-protocol.md）

# LLM 頭脳は config/config.json の llm.provider で決まる（初期状態は Ollama）
# ルール頭脳のみ: cargo run -- --no-llm

# プロバイダ切替: サンプルを config.json に上書きコピー
cp config/samples/config.lmstudio.json config/config.json
cargo run

# ローカル Ollama（要: ollama serve / ollama pull gemma4）
cp config/samples/config.ollama.json config/config.json

# OpenAI
cp config/samples/config.openai.json config/config.json
export OPENAI_API_KEY=sk-...
cargo run

# Google Gemini（triage-mail の Copilot と同系 API）
cp config/samples/config.gemini.json config/config.json
export GEMINI_API_KEY=your-key
cargo run

# Anthropic Claude（Messages API 直。OpenAI 互換ではない）
cp config/samples/config.anthropic.json config/config.json
export ANTHROPIC_API_KEY=your-key
cargo run
```

| 環境変数 | 説明 |
|----------|------|
| `OPENAI_API_KEY` / `HARNESS_SEED_API_KEY` | OpenAI 用 API キー（`MYHARNESS_*` も可） |
| `GEMINI_API_KEY` | Gemini 用 API キー（`llm.provider: gemini` 時） |
| `GEMINI_MODEL` | Gemini モデル（既定: `gemini-2.5-flash`） |
| `HARNESS_SEED_LLM_PROVIDER=gemini` | Gemini を明示 |
| `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY` | Claude 用 API キー（`llm.provider: anthropic` 時） |
| `ANTHROPIC_MODEL` | Claude モデル（既定: `claude-3-5-sonnet-20241022`） |
| `HARNESS_SEED_LLM_PROVIDER=anthropic` | Anthropic を明示 |
| `OPENAI_BASE_URL` / `HARNESS_SEED_BASE_URL` | OpenAI 互換エンドポイント |
| `HARNESS_SEED_MODEL` / `OPENAI_MODEL` | OpenAI モデル（既定: `gpt-4o-mini`） |
| `OLLAMA_HOST` | Ollama ホスト（既定: `http://127.0.0.1:11434`、自動で `/v1` 付与） |
| `OLLAMA_MODEL` | Ollama モデル（既定: `gemma4`） |
| `HARNESS_SEED_LLM_PROVIDER=ollama` | Ollama を明示 |
| `HARNESS_SEED_LLM_PROVIDER=lmstudio` | LM Studio を明示 |
| `LM_STUDIO_HOST` | LM Studio（既定: `http://127.0.0.1:1234`） |
| `LM_STUDIO_MODEL` | LM Studio 上のモデル名 |

### 設定ファイル

| ファイル | 用途 |
|----------|------|
| `config/config.json` | **実行・テストが読む正本**（ここを編集する） |
| `config/samples/config.ollama.json` | Ollama ひな形 |
| `config/samples/config.lmstudio.json` | LM Studio ひな形 |
| `config/samples/config.openai.json` | OpenAI ひな形 |
| `config/samples/config.gemini.json` | Google Gemini ひな形 |
| `config/samples/config.anthropic.json` | Anthropic Claude ひな形 |

切替例:

```bash
cp config/samples/config.lmstudio.json config/config.json
cargo run
```

環境変数は `config.json` より優先されます。別パス指定: `--config` または `HARNESS_SEED_CONFIG`（旧 `MYHARNESS_CONFIG` も可）。

詳細は [config/README.md](config/README.md)。

### コンテキストサイズ計測（LLM モード）

各 ReAct ステップの LLM 呼び出しごとに計測します。

| 指標 | 説明 |
|------|------|
| `chars` / `bytes` | プロンプト・出力テキストの文字数・バイト数（常に計測） |
| `tok` | API の `usage` または Ollama の `prompt_eval_count` / `eval_count`（`api`） |
| `tok (est)` | API が無い場合は約 4 文字 = 1 トークンで概算 |

ターン終了時に stderr へ合計が出ます（`show_context_metrics: true` 時）:

```
[context turn] llm_calls=3 prompt: 1200 chars / 310 tok (api) | completion: 180 chars / 45 tok (api) | total_tokens=355
```

`-v` では各ステップの `[context step]` も表示されます。`TurnResult.context` にプログラムからも参照できます。

### 統合テスト（LLM）

`tests/` 以下の LLM テストは **`config/config.json`** を読みます（`HARNESS_SEED_CONFIG` でパス変更可）。LM Studio 用テストのみ `config/samples/config.lmstudio.json` を直接参照します。

```bash
cp config/samples/config.ollama.json config/config.json
ollama pull gemma4   # config の model に合わせる
cargo test
```

LLM が未起動、またはモデル未インストールの場合は該当テストを **SKIP** します。

**ファイルログ**（`config/config.json` の `log.context_metrics`）:

```json
"log": {
  "context_metrics": "logs/context.jsonl"
}
```

各ターンを 1 行の JSON として追記します（`logs/` は `.gitignore` 済み）。LLM 呼び出しごとに `steps[].prompt` に API へ送った全文（`system:` / `user:` 形式）が入ります。計測フックで自動記録され、`-v` は不要です。

ビルド済みバイナリを直接実行する場合:

```bash
cargo build --release
./target/release/harness-seed   # Windows: target\release\harness-seed.exe
```

### テスト

```bash
cargo test
```

特定のテストのみ:

```bash
cargo test version_is_set
```

### サンプル

```bash
cargo run --example hello
```

### ベンチマーク

`benches/` にベンチ用の `.rs` を追加したあと:

```bash
cargo bench
```

（初回は `Cargo.toml` に `criterion` などのベンチ用依存を追加してください。）

## ライブラリとして使う

```toml
[dependencies]
harness-seed = { path = "../harness-seed" }
```

```rust
use harness_seed::{AppConfig, BrainMode, ReActLoop};

let app = AppConfig::load_default()?;
let brain = BrainMode::from_cli(&app, false, false)?;
let mut react = ReActLoop::new(brain, app.react_config(false, false));
let result = react.run_turn("hello")?;
println!("{}", result.answer);
```

## 開発メモ

- **クレート名**: `harness-seed`（Rust では `harness_seed`）
- **エディション**: Rust 2024（`Cargo.toml` の `edition`）
- **バージョン**: `VERSION` は `CARGO_PKG_VERSION` から取得
- 本体ロジックは `src/lib.rs` に置き、`main.rs` は薄い CLI エントリ

## ライセンス

MIT ライセンス（詳細は [LICENSE](LICENSE) を参照してください）。
