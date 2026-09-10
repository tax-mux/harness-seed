# HarnessSeed

Embedded ReAct harness (Rust crate `harness-seed`). It is a "seed" for the agent layer to be embedded in existing applications, **not a chat UI**. It provides both a library and a CLI (`harness-seed`) in the same crate.

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) (`rustup` recommended)
- Cargo (included with the Rust toolchain)

```bash
rustc --version
cargo --version
```

## Project Structure

```
harness-seed/
├── Cargo.toml
├── config/
│   ├── config.json       # Active configuration (edit/overwrite this)
│   ├── samples/          # Connector templates (config.*.json)
│   └── README.md
├── doc/             # Documentation
│   ├── README.md    # Language index
│   ├── ja/          # Japanese (architecture, tools, ideas, principles)
│   └── en/          # English (architecture, principles; tools/ideas indexes)
├── src/
│   ├── main.rs      # CLI entry point
│   ├── lib.rs       # Library core and public API
│   ├── advance/     # Outer advance loop (evidence, gates, phase)
│   ├── react/       # ReAct loop, two-phase, step driver, advance turn
│   ├── config.rs    # AppConfig (section types in config/)
│   ├── tasks/       # Task registry, contracts, step driver
│   ├── plan/        # Plan parse, display, queue
│   └── ...          # layer, lifecycle, llm, memory, tool, …
├── tests/           # Integration tests
└── benches/         # Benchmarks (add if necessary)
```

### Documentation

Index: [doc/README.md](doc/README.md)

| File | Content |
|------|------|
| [doc/README.md](doc/README.md) | Language index (ja / en) |
| [doc/ja/README.md](doc/ja/README.md) | Japanese docs home |
| [doc/en/README.md](doc/en/README.md) | English docs home |
| [doc/ja/development-principles.md](doc/ja/development-principles.md) | Development principles (JA) |
| [doc/en/development-principles.md](doc/en/development-principles.md) | Development principles (EN) |
| [doc/ja/architecture/README.md](doc/ja/architecture/README.md) | Architecture (JA) |
| [doc/en/architecture/README.md](doc/en/architecture/README.md) | Architecture (EN) |

## Usage

### Install (run from anywhere)

Put the CLI on your PATH (`~/.cargo/bin` must be on `PATH`):

```bash
cargo install --path .
```

Then from any directory:

```bash
harness-seed --help
```

Default config is **`~/.config/harness-seed/config.json`** (or under `XDG_CONFIG_HOME` when set). If that file is missing, cwd `config/config.json` is still read for backward compatibility. Explicit override:

```bash
harness-seed --config /path/to/config.json
# or
export HARNESS_SEED_CONFIG=/path/to/config.json
harness-seed
```

Re-install after pulling changes that affect the binary:

```bash
cargo install --path . --force
```

### Build

```bash
cargo build
```

Release build:

```bash
cargo build --release
```

### Execution

Interactive ReAct REPL:

```bash
cargo run
# Example: help / echo hello / time / any text (Thought -> echo -> Answer)
# Verbose logs: cargo run -- -v
# JSON Lines REPL: cargo run -- --json  (doc/ja/architecture/11_ワイヤプロトコル.md)

# First-time setup (user config; keep secrets here)
mkdir -p ~/.config/harness-seed
cp config/config.json.sample ~/.config/harness-seed/config.json

# The LLM brain is determined by llm.provider in ~/.config/harness-seed/config.json
# Rules-only brain: cargo run -- --no-llm

# Switching providers: overwrite the user config with a sample template
cp config/samples/config.lmstudio.json ~/.config/harness-seed/config.json
cargo run

# Local Ollama (Requires: ollama serve / ollama pull gemma4)
cp config/samples/config.ollama.json ~/.config/harness-seed/config.json

# OpenAI
cp config/samples/config.openai.json ~/.config/harness-seed/config.json
export OPENAI_API_KEY=sk-...
cargo run

# Google Gemini (Uses the same API series as the Copilot in triage-mail)
cp config/samples/config.gemini.json ~/.config/harness-seed/config.json
export GEMINI_API_KEY=your-key
cargo run

# Anthropic Claude (Uses Messages API directly, not OpenAI compatible)
cp config/samples/config.anthropic.json ~/.config/harness-seed/config.json
export ANTHROPIC_API_KEY=your-key
cargo run
```

| Environment Variable | Description |
|------|------|
| `OPENAI_API_KEY` / `HARNESS_SEED_API_KEY` | API key for OpenAI (can also use `MYHARNESS_*`) |
| `GEMINI_API_KEY` | API key for Gemini (when `llm.provider: gemini`) |
| `GEMINI_MODEL` | Gemini model (default: `gemini-2.5-flash`) |
| `HARNESS_SEED_LLM_PROVIDER=gemini` | Explicitly use Gemini |
| `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY` | API key for Claude (when `llm.provider: anthropic`) |
| `ANTHROPIC_MODEL` | Claude model (default: `claude-3-5-sonnet-20241022`) |
| `HARNESS_SEED_LLM_PROVIDER=anthropic` | Explicitly use Anthropic |
| `OPENAI_BASE_URL` / `HARNESS_SEED_BASE_URL` | OpenAI-compatible endpoint |
| `HARNESS_SEED_MODEL` / `OPENAI_MODEL` | OpenAI model (default: `gpt-4o-mini`) |
| `OLLAMA_HOST` | Ollama host (default: `http://127.0.0.1:11434`, automatically appends `/v1`) |
| `OLLAMA_MODEL` | Ollama model (default: `gemma4`) |
| `HARNESS_SEED_LLM_PROVIDER=ollama` | Explicitly use Ollama |
| `HARNESS_SEED_LLM_PROVIDER=lmstudio` | Explicitly use LM Studio |
| `LM_STUDIO_HOST` | LM Studio (default: `http://127.0.0.1:1234`) |
| `LM_STUDIO_MODEL` | Model name on LM Studio |

### Configuration File

| File | Usage |
|------|------|
| `~/.config/harness-seed/config.json` | **Active configuration** (user config; keep secrets here) |
| `config/config.json.sample` | **Default template** (tracked; no secrets) |
| `config/config.json` | **Local fallback** (gitignored; used only when user config is missing) |
| `config/samples/config.ollama.json` | Ollama template |
| `config/samples/config.lmstudio.json` | LM Studio template |
| `config/samples/config.openai.json` | OpenAI template |
| `config/samples/config.gemini.json` | Google Gemini template |
| `config/samples/config.anthropic.json` | Anthropic Claude template |

Example of switching:

```bash
cp config/samples/config.lmstudio.json ~/.config/harness-seed/config.json
cargo run
```

Environment variables take precedence over settings in `config.json`. Specifying an alternative path: `--config` or `HARNESS_SEED_CONFIG` (the legacy `MYHARNESS_CONFIG` is also supported).

For details, see [config/README.md](config/README.md).

### Context Size Metrics (LLM Mode)

Measured for each LLM call in every ReAct step.

| Metric | Description |
|------|------|
| `chars` / `bytes` | Character count and byte count of prompt/output text (always measured) |
| `tok` | Token count from API `usage` or Ollama's `prompt_eval_count`/`eval_count` (`api`) |
| `tok (est)` | Rough estimate if API data is unavailable (approx. 4 characters = 1 token) |

At the end of a turn, the total is output to stderr (when `show_context_metrics: true`):

```
[context turn] llm_calls=3 prompt: 1200 chars / 310 tok (api) | completion: 180 chars / 45 tok (api) | total_tokens=355
```

With `-v`, `[context step]` for each step is also displayed. This is also accessible programmatically via `TurnResult.context`.

### Integration Tests (LLM)

LLM integration tests in `tests/` read **`default_config_path()`** (default `~/.config/harness-seed/config.json`, else cwd `config/config.json`; override with `HARNESS_SEED_CONFIG`). Only the LM Studio test directly references `config/samples/config.lmstudio.json`.

```bash
cp config/samples/config.ollama.json ~/.config/harness-seed/config.json
# or for local repo workflow: cp config/samples/config.ollama.json config/config.json
ollama pull gemma4   # Match the model specified in the config
cargo test
```

If the LLM is not running or the model is not installed, the corresponding test will be **SKIPPED**.

**File Logging** (`log.context_metrics` in the config file):

```json
"log": {
  "context_metrics": "logs/context.jsonl"
}
```

Appends each turn as a single-line JSON (`logs/` is already in `.gitignore`). For each LLM call, the full prompt text (in `system:` / `user:` format) sent to the API is saved in `steps[].prompt`. This is recorded automatically via a measurement hook, so `-v` is not required.

To run the built binary directly:

```bash
cargo build --release
./target/release/harness-seed   # Windows: target\release\harness-seed.exe
```

### Tests

```bash
cargo test
```

Running specific tests:

```bash
cargo test version_is_set
```

### Examples

```bash
cargo run --example hello
```

### Benchmarks

After adding a benchmark `.rs` file to `benches/`:

```bash
cargo bench
```

(For first-time setup, please add benchmarking dependencies such as `criterion` to your `Cargo.toml`.)

## Using as a Library

```toml
[dependencies]
harness-seed = { path = "../harness-seed" }
```

When embedding in a host app, **the host chooses the config file path** (CLI `--config` is binary-only). Prefer `AppConfig::load_path`. See [config/README.md](config/README.md#ライブラリ組み込み時のパス指定).

```rust
use harness_seed::{AppConfig, BrainPair, SeedBuilder};

// Explicit host path (recommended)
let app = AppConfig::load_path("/var/lib/my-app/harness-seed.json")?;
// Or the same default resolution as the CLI: AppConfig::load_default()?
let builder = SeedBuilder::from_app(&app)?;
let brains = BrainPair::from_cli_with_registry(&app, false, false, builder.task_registry_ref())?;
let mut react = builder.build(brains.exec, brains.plan, app.react_config(false, false));
let result = react.run_turn("hello")?;
println!("{}", result.answer);
```

Host rules, tools, tasks, and lifecycle attach on `SeedBuilder` before `build`. CLI helpers only gather files and call the same API.

## Development Notes

- **Crate Name**: `harness-seed` (represented as `harness_seed` in Rust)
- **Edition**: Rust 2024 (`edition` in `Cargo.toml`)
- **Version**: `VERSION` is retrieved from `CARGO_PKG_VERSION`
- Core logic is placed in `src/lib.rs`, and `main.rs` serves as a thin CLI entry point.

## License

MIT License (See [LICENSE](LICENSE) for details).
