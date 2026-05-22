//! 統合テスト共通（`config/config.json` ベース。切替は `config/samples/` を上書きコピー）。

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use harness_seed::{
    default_config_path, AppConfig, BrainMode, BrainPair, ChatMessage, LlmBrain, LlmConnector,
    LlmConnectorKind, LlmProvider, PlanBrainMode, ReActLoop,
};

/// エージェントに自己紹介させるユーザー入力。
pub const SELF_INTRO_USER_PROMPT: &str =
    "あなたは誰ですか？簡潔に自己紹介してください。";

/// カレントディレクトリのファイル一覧を `list_dir` で取得させる入力。
pub const LIST_FILES_USER_PROMPT: &str =
    "list_dir ツールを使ってカレントディレクトリ（.）のファイルとフォルダの一覧を取得し、取得した一覧をそのまま答えてください。";

/// `write_file` / `read_file` でコードを書き込む入力。
pub const WRITE_CODE_USER_PROMPT: &str =
    "write_file で tmp/agent_hello.rs に `fn main() { println!(\"hi\"); }` だけを書き込み、read_file で内容を確認してから、書き込んだ内容を報告してください。";

/// テストで読み込む設定ファイルパス。
pub fn test_config_path() -> PathBuf {
    std::env::var("HARNESS_SEED_CONFIG")
        .ok()
        .or_else(|| std::env::var("MYHARNESS_CONFIG").ok())
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path)
}

/// `config/config.json`（または `HARNESS_SEED_CONFIG`）を読み込む。
pub fn load_test_config() -> Option<AppConfig> {
    let path = test_config_path();
    match AppConfig::load_path(&path) {
        Ok(app) => {
            eprintln!("test config: {}", path.display());
            Some(app)
        }
        Err(err) => {
            eprintln!("SKIP: failed to load {}: {err}", path.display());
            None
        }
    }
}

/// 設定に記載のモデル名。
pub fn config_model_name(app: &AppConfig) -> &str {
    app.llm
        .model
        .as_deref()
        .unwrap_or("(model not set)")
}

fn default_port_for_provider(provider: LlmProvider) -> u16 {
    match provider {
        LlmProvider::LmStudio => 1234,
        LlmProvider::Ollama => 11434,
        LlmProvider::OpenAi | LlmProvider::Gemini | LlmProvider::Anthropic => 443,
    }
}

fn llm_socket_addr(app: &AppConfig) -> Option<SocketAddr> {
    let provider = app.llm_provider();
    let base = app.llm.base_url.as_deref().unwrap_or(match provider {
        LlmProvider::LmStudio => "http://127.0.0.1:1234",
        LlmProvider::Ollama => "http://127.0.0.1:11434",
        LlmProvider::OpenAi => "https://api.openai.com",
        LlmProvider::Gemini => "https://generativelanguage.googleapis.com",
        LlmProvider::Anthropic => "https://api.anthropic.com",
    });
    parse_host_port(base, default_port_for_provider(provider))
}

fn parse_host_port(base_url: &str, default_port: u16) -> Option<SocketAddr> {
    let trimmed = base_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/v1");
    let host_port = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))?;
    if host_port.contains(':') {
        host_port.parse().ok()
    } else {
        format!("{host_port}:{default_port}").parse().ok()
    }
}

/// LLM エンドポイントへ TCP 接続できるか。
pub fn llm_host_is_available(app: &AppConfig) -> bool {
    let Some(addr) = llm_socket_addr(app) else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

fn build_llm_config(app: &AppConfig) -> Option<harness_seed::LlmConfig> {
    app.build_llm_config().ok()
}

fn build_connector(app: &AppConfig) -> Option<LlmConnectorKind> {
    let llm_config = build_llm_config(app)?;
    LlmConnectorKind::from_config(llm_config).ok()
}

/// 設定どおりのコネクタでチャット API が応答するか。
pub fn llm_ready(app: &AppConfig) -> bool {
    if !llm_host_is_available(app) {
        return false;
    }

    let Some(connector) = build_connector(app) else {
        return false;
    };
    connector
        .complete(&[ChatMessage::user("ping")])
        .is_ok()
}

fn provider_hint(app: &AppConfig) -> &'static str {
    match app.llm_provider() {
        LlmProvider::LmStudio => "start LM Studio local server and load a model",
        LlmProvider::Ollama => "run `ollama serve` and pull the model",
        LlmProvider::OpenAi => "set OPENAI_API_KEY",
        LlmProvider::Gemini => "set GEMINI_API_KEY",
        LlmProvider::Anthropic => "set ANTHROPIC_API_KEY",
    }
}

/// LLM 未使用可能時にテストをスキップする。スキップしたら `true`。
pub fn skip_if_llm_not_ready() -> bool {
    let Some(app) = load_test_config() else {
        return true;
    };

    if !llm_host_is_available(&app) {
        let base = app.llm.base_url.as_deref().unwrap_or("(no base_url)");
        eprintln!("SKIP: LLM host not reachable ({base}) — {}", provider_hint(&app));
        return true;
    }

    if !llm_ready(&app) {
        eprintln!(
            "SKIP: LLM chat API failed (provider: {:?}, model: {}, {})",
            app.llm_provider(),
            config_model_name(&app),
            provider_hint(&app)
        );
        return true;
    }

    false
}

/// `config.json` から `ReActLoop` を構築する。
pub fn build_react_loop_from_config() -> Option<ReActLoop<BrainMode>> {
    let app = load_test_config()?;
    if !llm_ready(&app) {
        return None;
    }

    let connector = build_connector(&app)?;
    let mut react_config = app.react_config(false, false);
    // 統合テストは単一 trace の検証が多いため two_phase をオフにする。
    react_config.two_phase = false;

    let brains = BrainPair {
        exec: BrainMode::Llm(LlmBrain::new(connector)),
        plan: PlanBrainMode::from_cli(&app, true, false, &harness_seed::TaskRegistry::load_default())
            .ok()?,
    };
    Some(ReActLoop::new(brains.exec, brains.plan, react_config))
}

/// 単発チャット（ReAct JSON なし）で LLM 応答を得る。
pub fn llm_chat_from_config(user_prompt: &str) -> Option<String> {
    let app = load_test_config()?;
    if !llm_ready(&app) {
        return None;
    }

    let connector = build_connector(&app)?;
    let messages = vec![
        ChatMessage::system("You are a helpful assistant. Reply in Japanese."),
        ChatMessage::user(user_prompt),
    ];

    let result = connector.complete(&messages).ok()?;
    Some(result.content)
}

/// LLM エラー応答でないこと。
pub fn is_llm_error_answer(answer: &str) -> bool {
    answer.starts_with("LLM connector error:")
        || answer.starts_with("LLM response parse error:")
}
