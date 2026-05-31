use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use harness_seed::{run_json_repl, run_repl, AppConfig, BrainPair, ReActLoop, SimpleRuleBrain, TaskRegistry, VERSION};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let verbose = args.iter().any(|a| matches!(a.as_str(), "-v" | "--verbose"));
    let show_prompt = args.iter().any(|a| a == "--show-prompt");
    let json_repl = args.iter().any(|a| a == "--json");
    let plan_zone = args.iter().any(|a| a == "--plan-zone");
    let plan_zone_full = args.iter().any(|a| a == "--plan-zone-full");
    let no_monitor = args.iter().any(|a| a == "--no-monitor");
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

    if plan_zone || plan_zone_full {
        return run_plan_zone_mode(
            &app,
            plan_zone_full,
            no_monitor,
            use_llm,
            no_llm,
            verbose,
            &args,
        );
    }

    let brains = match BrainPair::from_cli(&app, use_llm, no_llm) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to initialize LLM brain: {err}");
            return ExitCode::from(1);
        }
    };

    let react_config = app.react_config(verbose, show_prompt);
    let mut react_config = react_config;
    react_config.monitor_plan_html = !no_monitor;

    eprintln!("config: {}", config_path.display());
    if let Some(provider) = &app.llm.provider {
        eprintln!("llm.provider: {provider}");
    }
    eprintln!("brain: {}", brains.label());
    eprintln!(
        "react: max_steps={} max_steps_plan={} session_max_turns={} two_phase={} advance={} show_tool_output={}",
        react_config.max_steps,
        react_config.max_steps_plan,
        react_config.session_max_turns,
        react_config.two_phase,
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

fn run_plan_zone_mode(
    app: &AppConfig,
    full: bool,
    no_monitor: bool,
    use_llm: bool,
    no_llm: bool,
    verbose: bool,
    args: &[String],
) -> ExitCode {
    let user_input = match parse_plan_zone_input(args) {
        Some(s) => s,
        None => {
            eprintln!("--plan-zone: user input required (argument or stdin line)");
            return ExitCode::from(1);
        }
    };

    let blocks = match app.load_prompt_blocks() {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to load prompt rules: {err}");
            return ExitCode::from(1);
        }
    };
    let mut react_config = app.react_config(verbose, false);
    react_config.show_plan = false;
    react_config.show_context_metrics = false;
    react_config.monitor_plan_html = !no_monitor;
    let tool_packs = app.resolved_tool_packs();

    let brains = match BrainPair::from_cli(app, use_llm, no_llm) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to initialize plan brain: {err}");
            return ExitCode::from(1);
        }
    };
    eprintln!("brain: {}", brains.label());

    let mut react = ReActLoop::with_blocks_and_tasks(
        SimpleRuleBrain::new(),
        brains.plan,
        react_config,
        blocks,
        TaskRegistry::load_default(),
        app.resolved_brave_search(),
        &tool_packs,
    );

    if full {
        if !no_monitor {
            let html_text = harness_seed::format_planner_fixed_zone_html(
                &react.blocks,
                &react.task_registry,
                None,
                None,
                Some(&user_input),
                None,
                None,
                &react.blocks.recalled,
                None,
                &[],
            );
            match write_plan_zone_monitor_html(&html_text) {
                Ok(path) => eprintln!("[monitor] wrote: {}", path.display()),
                Err(err) => {
                    eprintln!("[monitor] write failed: {err}");
                    return ExitCode::from(1);
                }
            }
        }
        print!(
            "{}",
            harness_seed::format_plan_zone_prompt_preview(
                &react.blocks,
                &react.task_registry,
                &user_input,
                &react.format_plan_layer_prompt(&user_input),
            )
        );
        return ExitCode::SUCCESS;
    }

    match react.run_plan_preview(&user_input) {
        Ok(preview) => {
            if !no_monitor {
                let html_text = harness_seed::format_planner_fixed_zone_html(
                    &react.blocks,
                    &react.task_registry,
                    Some(&preview.harness),
                    Some(&preview.planner_text),
                    Some(&user_input),
                    None,
                    None,
                    &react.blocks.recalled,
                    None,
                    &[],
                );
                match write_plan_zone_monitor_html(&html_text) {
                    Ok(path) => eprintln!("[monitor] wrote: {}", path.display()),
                    Err(err) => {
                        eprintln!("[monitor] write failed: {err}");
                        return ExitCode::from(1);
                    }
                }
            }
            print!(
                "{}",
                harness_seed::format_plan_zone_after_preview(
                    &react.blocks,
                    &react.task_registry,
                    &user_input,
                    &preview.planner_text,
                    &preview.harness,
                )
            );
            if verbose {
                eprintln!("[plan-zone] steps={}", preview.steps_used);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("plan preview failed: {err}");
            ExitCode::from(1)
        }
    }
}

fn write_plan_zone_monitor_html(html: &str) -> io::Result<PathBuf> {
    let monitor_dir = PathBuf::from("monitor");
    fs::create_dir_all(&monitor_dir)?;

    let monitor_path = monitor_dir.join("context_monitor.html");
    fs::write(&monitor_path, html)?;

    Ok(monitor_path)
}

fn parse_plan_zone_input(args: &[String]) -> Option<String> {
    let mut after_flag = false;
    let mut parts = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if plan_zone_flag_takes_value(arg) {
            skip_next = true;
            continue;
        }
        if arg == "--plan-zone" || arg == "--plan-zone-full" {
            after_flag = true;
            continue;
        }
        if after_flag && !is_plan_zone_global_flag(arg) {
            parts.push(arg.as_str());
        }
    }
    if !parts.is_empty() {
        return Some(parts.join(" "));
    }
    if after_flag {
        let mut line = String::new();
        if io::stdin().read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn is_plan_zone_global_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--no-llm"
            | "--llm"
            | "-v"
            | "--verbose"
            | "--show-prompt"
            | "--json"
            | "--no-monitor"
    )
}

fn plan_zone_flag_takes_value(arg: &str) -> bool {
    arg == "--config"
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
    --no-monitor            monitor/context_monitor.html の更新を抑制
  --plan-zone [TEXT]      固定ゾーン表示 → Planner 実行 → 作業指示書を stdout に出力
  --plan-zone-full [TEXT] 計画層 1 ステップ目のプロンプト全文のみ（LLM 未使用）
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
