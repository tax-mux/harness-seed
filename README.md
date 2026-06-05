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
├── doc/             # Documentation (current specs + ideas/)
│   └── ideas/       # Unimplemented design notes
├── src/
│   ├── main.rs      # CLI entry point
│   └── lib.rs       # Library core and public API
├── tests/           # Integration tests
└── benches/         # Benchmarks (add if necessary)
```

### Documentation

| File | Content |
|------|------|
| [doc/agent-minimum-action-unit.md](doc/agent-minimum-action-unit.md) | Minimum action unit of the AI agent (diagram) |
| [doc/react-implementation.md](doc/react-implementation.md) | Current ReAct implementation structure, flow, and limitations |
| [doc/advance-loop.md](doc/advance-loop.md) | Outer advance loop (long context chunking) |
| [doc/wire-protocol.md](doc/wire-protocol.md) | JSON Lines wire protocol (`--json`) |
| [doc/builtin_tools/README.md](doc/builtin_tools/README.md) | Built-in tools specification (one file per tool) |
| [doc/context-memory-mapping.md](doc/context-memory-mapping.md) | Mapping contexts for different purposes (diagram) |
| [doc/ideas/README.md](doc/ideas/README.md) | List of unimplemented design notes (concrete ideas reside only in the ideas/ directory) |

## Usage

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
# JSON Lines REPL: cargo run -- --json  (doc/wire-protocol.md)

# The LLM brain is determined by llm.provider in config/config.json (defaults to Ollama)
# Rules-only brain: cargo run -- --no-llm

# Switching providers: Overwrite config.json with a sample template
cp config/samples/config.lmstudio.json config/config.json
cargo run

# Local Ollama (Requires: ollama serve / ollama pull gemma4)
cp config/samples/config.ollama.json config/config.json

# OpenAI
cp config/samples/config.openai.json config/config.json
export OPENAI_API_KEY=sk-...
cargo run

# Google Gemini (Uses the same API series as the Copilot in triage-mail)
cp config/samples/config.gemini.json config/config.json
export GEMINI_API_KEY=your-key
cargo run

# Anthropic Claude (Uses Messages API directly, not OpenAI compatible)
cp config/samples/config.anthropic.json config/config.json
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
| `config/config.json` | **Active configuration read by runs and tests** (Edit this file) |
| `config/samples/config.ollama.json` | Ollama template |
| `config/samples/config.lmstudio.json` | LM Studio template |
| `config/samples/config.openai.json` | OpenAI template |
| `config/samples/config.gemini.json` | Google Gemini template |
| `config/samples/config.anthropic.json` | Anthropic Claude template |

Example of switching:

```bash
cp config/samples/config.lmstudio.json config/config.json
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

LLM integration tests in `tests/` read **`config/config.json`** (the path can be overridden with `HARNESS_SEED_CONFIG`). Only the LM Studio test directly references `config/samples/config.lmstudio.json`.

```bash
cp config/samples/config.ollama.json config/config.json
ollama pull gemma4   # Match the model specified in the config
cargo test
```

If the LLM is not running or the model is not installed, the corresponding test will be **SKIPPED**.

**File Logging** (`log.context_metrics` in `config/config.json`):

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

```rust
use harness_seed::{AppConfig, BrainMode, ReActLoop};

let app = AppConfig::load_default()?;
let brain = BrainMode::from_cli(&app, false, false)?;
let mut react = ReActLoop::new(brain, app.react_config(false, false));
let result = react.run_turn("hello")?;
println!("{}", result.answer);
```

## Development Notes

- **Crate Name**: `harness-seed` (represented as `harness_seed` in Rust)
- **Edition**: Rust 2024 (`edition` in `Cargo.toml`)
- **Version**: `VERSION` is retrieved from `CARGO_PKG_VERSION`
- Core logic is placed in `src/lib.rs`, and `main.rs` serves as a thin CLI entry point.

## License

MIT License (See [LICENSE](LICENSE) for details).
