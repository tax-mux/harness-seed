use std::fmt;

/// 検索ヒット（プロンプトへ載せる要約単位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub body: String,
    pub ref_id: Option<String>,
    pub score: Option<String>,
}

/// diary_read の 1 エントリ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiaryReadEntry {
    pub title: String,
    pub body: String,
    pub ref_id: Option<String>,
}

#[derive(Debug)]
pub enum MempalaceError {
    Config(String),
    Http(String),
    Parse(String),
    Backend(String),
}

impl fmt::Display for MempalaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(m) => write!(f, "mempalace config: {m}"),
            Self::Http(m) => write!(f, "mempalace http: {m}"),
            Self::Parse(m) => write!(f, "mempalace parse: {m}"),
            Self::Backend(m) => write!(f, "mempalace: {m}"),
        }
    }
}

impl std::error::Error for MempalaceError {}
