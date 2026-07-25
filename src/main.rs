use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use harness_seed::{
    build_memory_rag,
    cli_agent::{
        cli_flag_takes_value, is_cli_global_flag, log_agent_setup, merge_cli_agent,
        prepare_cli_agent_workspace,
    },
    run_json_repl, run_repl, AppConfig, BrainPair, MemoryRag, ReActConfig, SeedBuilder,
    SimpleRuleBrain, VERSION,
};

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

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

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
            &cwd,
        );
    }

    if let Err(err) = prepare_cli_agent_workspace(&args, &cwd) {
        eprintln!("failed to prepare agent workspace: {err}");
        return ExitCode::from(1);
    }

    let mut builder = match SeedBuilder::from_app(&app) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to load prompt rules: {err}");
            return ExitCode::from(1);
        }
    };
    let agent_setup;
    (builder, agent_setup) = match merge_cli_agent(&args, &cwd, builder) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("failed to load agent project: {err}");
            return ExitCode::from(1);
        }
    };
    if let Some(ref setup) = agent_setup {
        log_agent_setup(setup);
    }
    if !builder.blocks_ref().rules.is_empty() {
        eprintln!(
            "prompt: loaded {} rule block(s)",
            builder.blocks_ref().rules.len()
        );
    }

    let brains = match BrainPair::from_cli_with_registry(
        &app,
        use_llm,
        no_llm,
        builder.task_registry_ref(),
    ) {
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

    eprintln!(
        "tools: packs={}",
        builder
            .tool_packs_ref()
            .iter()
            .map(|p| p.id())
            .collect::<Vec<_>>()
            .join(",")
    );
    if builder.brave_search_ref().is_some() {
        eprintln!("tools: web_search (Brave Search API)");
    }
    let memory_layers = app.memory_provider_name();
    if memory_layers != "noop" {
        eprintln!("memory.layers: {memory_layers}");
    }
    let memory_rag = build_memory_rag_for_app(&app, &react_config, no_llm);
    let mut react = builder
        .memory_rag(memory_rag)
        .build(brains.exec, brains.plan, react_config);
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
    cwd: &std::path::Path,
) -> ExitCode {
    let user_input = match parse_plan_zone_input(args) {
        Some(s) => s,
        None => {
            eprintln!("--plan-zone: user input required (argument or stdin line)");
            return ExitCode::from(1);
        }
    };

    if let Err(err) = prepare_cli_agent_workspace(args, cwd) {
        eprintln!("failed to prepare agent workspace: {err}");
        return ExitCode::from(1);
    }

    let mut builder = match SeedBuilder::from_app(app) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to load prompt rules: {err}");
            return ExitCode::from(1);
        }
    };
    let agent_setup;
    (builder, agent_setup) = match merge_cli_agent(args, cwd, builder) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("failed to load agent project: {err}");
            return ExitCode::from(1);
        }
    };
    if let Some(ref setup) = agent_setup {
        log_agent_setup(setup);
    }
    let mut react_config = app.react_config(verbose, false);
    react_config.show_plan = false;
    react_config.show_context_metrics = false;
    react_config.monitor_plan_html = !no_monitor;

    let brains = match BrainPair::from_cli_with_registry(
        app,
        use_llm,
        no_llm,
        builder.task_registry_ref(),
    ) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("failed to initialize plan brain: {err}");
            return ExitCode::from(1);
        }
    };
    eprintln!("brain: {}", brains.label());

    let memory_rag = build_memory_rag_for_app(app, &react_config, no_llm);
    let mut react = builder.memory_rag(memory_rag).build(
        SimpleRuleBrain::new(),
        brains.plan,
        react_config,
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
        if after_flag && !is_cli_global_flag(arg) {
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

fn plan_zone_flag_takes_value(arg: &str) -> bool {
    cli_flag_takes_value(arg)
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

/// 記憶 RAG（アダプタ手前）。`memory.rag.router=llm` かつ LLM 利用時のみ LlmRouter。
fn build_memory_rag_for_app(app: &AppConfig, react_config: &ReActConfig, no_llm: bool) -> MemoryRag {
    let want_llm = react_config.memory.rag_router.eq_ignore_ascii_case("llm")
        && !no_llm
        && app.uses_llm();
    let connector = if want_llm {
        match harness_seed::LlmConfig::from_app(app)
            .and_then(harness_seed::LlmConnectorKind::from_config)
        {
            Ok(c) => {
                eprintln!("memory.rag: router=llm (fallback=rule)");
                Some(c)
            }
            Err(err) => {
                eprintln!("[memory.rag] llm router unavailable ({err}); using rule");
                None
            }
        }
    } else {
        eprintln!("memory.rag: router=rule");
        None
    };
    build_memory_rag(&react_config.memory, connector)
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
  --config <PATH>         harness-seed 設定（既定: config/config.json）
  --config-agent <PATH>   プロジェクトの config.agent.json（既定: ./config.agent.json）
  --agent-dir <PATH>      エージェント資産ディレクトリ（workspace は実行時 cwd）
  --llm                   設定に関わらず LLM 頭脳を強制
  --no-llm                ルール頭脳を強制（設定の llm を無視）

config.agent.json（実行時パス直下。省略時は自動検出）:
  workspace               ファイルツールのルート（既定: .）
  agent_dir               rules / skills / tools を含むディレクトリ（既定: .agent）

agent_dir レイアウト:
  rules/**/*.md           追加ルール（再帰）
  skills/<id>/task.json   計画層タスク（スキル）
  skills/<id>/SKILL.md    スキル説明（ルールへ注入）
  tools/*.json            宣言的シェルツール

プロバイダ切替（推奨）:
  cp config/config.json.sample config/config.json
  # プロバイダ別: config/samples/config.lmstudio.json など
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
  HARNESS_WORKSPACE       ファイルツールのワークスペース（config.agent.json でも設定可）
  HARNESS_SEED_CONFIG / MYHARNESS_CONFIG   設定ファイルパス
  HARNESS_SEED_LLM_PROVIDER / MYHARNESS_LLM_PROVIDER  プロバイダ上書き
  OPENAI_API_KEY / GEMINI_API_KEY / ANTHROPIC_API_KEY / HARNESS_SEED_API_KEY / OLLAMA_* / LM_STUDIO_* など
"
    );
}
