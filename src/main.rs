use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use harness_seed::{run_json_repl, run_repl, AppConfig, BrainPair, ReActLoop, VERSION};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let verbose = args.iter().any(|a| matches!(a.as_str(), "-v" | "--verbose"));
    let show_prompt = args.iter().any(|a| a == "--show-prompt");
    let json_repl = args.iter().any(|a| a == "--json");
    let use_llm = args.iter().any(|a| a == "--llm");
    let no_llm = args.iter().any(|a| a == "--no-llm");
    let config_path = parse_config_path(&args);

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let app = match AppConfig::load_path(&config_path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("failed to load config: {err}");
            return ExitCode::from(1);
        }
    };

    let brains = match BrainPair::from_cli(&app, use_llm, no_llm) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to initialize LLM brain: {err}");
            return ExitCode::from(1);
        }
    };

    let react_config = app.react_config(verbose, show_prompt);

    eprintln!("config: {}", config_path.display());
    if let Some(provider) = &app.llm.provider {
        eprintln!("llm.provider: {provider}");
    }
    eprintln!("brain: {}", brains.label());
    eprintln!(
        "react: max_steps={} max_steps_plan={} session_max_turns={} two_phase={} scout={} advance={} show_tool_output={}",
        react_config.max_steps,
        react_config.max_steps_plan,
        react_config.session_max_turns,
        react_config.two_phase,
        react_config.scout.enabled,
        react_config.advance.enabled,
        react_config.show_tool_output
    );
    if let Some(path) = &react_config.context_log_path {
        eprintln!("context log: {}", path.display());
    }

    let blocks = match app.load_prompt_blocks() {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to load prompt rules: {err}");
            return ExitCode::from(1);
        }
    };
    if !blocks.rules.is_empty() {
        eprintln!("prompt: loaded {} rule block(s)", blocks.rules.len());
    }

    let brave_search = app.resolved_brave_search();
    let tool_packs = app.resolved_tool_packs();
    eprintln!(
        "tools: packs={}",
        tool_packs
            .iter()
            .map(|p| p.id())
            .collect::<Vec<_>>()
            .join(",")
    );
    if brave_search.is_some() {
        eprintln!("tools: web_search (Brave Search API)");
    }
    let mut react = ReActLoop::with_blocks_and_tasks(
        brains.exec,
        brains.plan,
        react_config,
        blocks,
        harness_seed::TaskRegistry::load_default(),
        brave_search,
        &tool_packs,
    );
    eprintln!("runtime: {}", react.blocks.runtime.summary_line());

    let repl_result = if json_repl {
        run_json_repl(&mut react, verbose)
    } else {
        run_repl(&mut react, verbose)
    };
    if let Err(err) = repl_result {
        eprintln!("io error: {err}");
        return ExitCode::from(1);
    }

    let _ = VERSION;
    ExitCode::SUCCESS
}

fn parse_config_path(args: &[String]) -> PathBuf {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--config" {
            if let Some(path) = args.get(i + 1) {
                return PathBuf::from(path);
            }
        }
    }
    harness_seed::default_config_path()
}

fn print_usage() {
    let _ = writeln!(
        io::stdout(),
        "\
HarnessSeed v{VERSION} — ReAct REPL

Usage:
  harness-seed [OPTIONS]

Options:
  -h, --help              このヘルプ
  -v, --verbose           Thought / Action / Observation を stderr に表示
  --show-prompt           各 ReAct ステップの LLM プロンプト全文を stderr に表示
  --json                  JSON Lines REPL（stdin/stdout は 1 行 1 JSON、ログは stderr）
  --config <PATH>         設定ファイル（既定: config/config.json）
  --llm                   設定に関わらず LLM 頭脳を強制
  --no-llm                ルール頭脳を強制（設定の llm を無視）

プロバイダ切替（推奨）:
  cp config/samples/config.lmstudio.json config/config.json
  # ひな形: config/samples/config.ollama.json など
  # 詳細: config/README.md

設定ファイル:
  llm.provider            \"openai\" | \"ollama\" | \"lmstudio\" | \"gemini\" | \"anthropic\" | \"claude\"
  llm.api_key             API キー（null 可。環境変数で上書き可）
  llm.base_url            API ベース URL
  llm.model               モデル名
  llm.timeout_secs        タイムアウト秒
  llm.json_mode           OpenAI JSON モード（Ollama / LM Studio では通常 false）
  react.max_steps         1ターンの最大ステップ
  react.session_max_turns REPL 短期記憶（Previous turns）の保持数
  react.two_phase         計画層 → 実行層の直列（既定: false）
  react.advance.enabled   推進ループ（既定: false、true で two_phase より優先）
  react.max_steps_plan    計画層 ReAct の最大ステップ（既定: 4）
  react.verbose           詳細ログ
  react.show_prompt       各ステップのプロンプト全文（stderr）
  prompt.rules_paths      追加ルール（.md）の読み込みパス
  log.context_metrics     コンテキスト計測ログ（JSON Lines）

環境変数（設定より優先）:
  HARNESS_SEED_CONFIG / MYHARNESS_CONFIG   設定ファイルパス
  HARNESS_SEED_LLM_PROVIDER / MYHARNESS_LLM_PROVIDER  プロバイダ上書き
  OPENAI_API_KEY / GEMINI_API_KEY / ANTHROPIC_API_KEY / HARNESS_SEED_API_KEY / OLLAMA_* / LM_STUDIO_* など
"
    );
}
