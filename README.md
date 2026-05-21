# myharness

Rust で実装するプロジェクトです。ライブラリと CLI バイナリの両方を同じクレートで提供します。

## 必要環境

- [Rust](https://www.rust-lang.org/tools/install)（`rustup` 推奨）
- Cargo（Rust ツールチェーンに同梱）

```bash
rustc --version
cargo --version
```

## プロジェクト構成

```
myharness/
├── Cargo.toml
├── src/
│   ├── main.rs      # CLI エントリポイント
│   └── lib.rs       # ライブラリ本体・公開 API
├── tests/           # 統合テスト
├── examples/        # 実行例
└── benches/         # ベンチマーク（必要に応じて追加）
```

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

```bash
cargo run
```

ビルド済みバイナリを直接実行する場合:

```bash
cargo build --release
./target/release/myharness   # Windows: target\release\myharness.exe
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

## 開発メモ

- **エディション**: Rust 2024（`Cargo.toml` の `edition`）
- **バージョン**: `src/lib.rs` の `VERSION` は `CARGO_PKG_VERSION` から取得
- 本体ロジックは `src/lib.rs` に置き、`main.rs` は薄いエントリにする構成を推奨

## ライセンス

未設定（必要に応じて `LICENSE` を追加してください）。
