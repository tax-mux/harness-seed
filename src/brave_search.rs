//! Brave Search API（gemini-pad の `braveSearch.mjs` を参考にした組み込み検索）。

const BRAVE_WEB_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; harness-seed/1.0; +https://github.com/)";

/// `config.json` の `tools.brave_search` および環境変数から解決した設定。
#[derive(Debug, Clone)]
pub struct BraveSearchConfig {
    pub api_key: String,
    pub max_results: u8,
    pub fetch_content: bool,
    pub max_content_chars: usize,
}

#[derive(Debug, Clone)]
pub struct WebSearchHit {
    pub title: String,
    pub url: String,
    pub content: String,
}

#[derive(Debug)]
pub enum BraveSearchError {
    MissingApiKey,
    Http(String),
    Parse(String),
}

impl std::fmt::Display for BraveSearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                write!(
                    f,
                    "Brave Search API key not configured (set tools.brave_search.api_key in config.json or BRAVE_SEARCH_API_KEY)"
                )
            }
            Self::Http(msg) => write!(f, "Brave Search HTTP error: {msg}"),
            Self::Parse(msg) => write!(f, "Brave Search response parse error: {msg}"),
        }
    }
}

impl std::error::Error for BraveSearchError {}

/// Brave Web Search API を呼び出し、整形テキストを返す。
pub fn search_web(cfg: &BraveSearchConfig, query: &str, count: Option<u8>) -> Result<String, BraveSearchError> {
    if cfg.api_key.is_empty() {
        return Err(BraveSearchError::MissingApiKey);
    }

    let q = query.trim();
    if q.is_empty() {
        return Err(BraveSearchError::Parse("query must not be empty".into()));
    }
    let q = if q.len() > 400 { &q[..400] } else { q };

    let count = count
        .map(|c| c.clamp(1, 20))
        .unwrap_or(cfg.max_results)
        .min(cfg.max_results);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| BraveSearchError::Http(e.to_string()))?;

    let count_str = count.to_string();
    // Do not set Accept-Encoding manually — reqwest decompresses gzip when the feature is on;
    // a raw gzip body breaks JSON parsing ("expected value at line 1 column 1").
    let response = client
        .get(BRAVE_WEB_SEARCH_URL)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", &cfg.api_key)
        .query(&[("q", q), ("count", count_str.as_str())])
        .send()
        .map_err(|e| BraveSearchError::Http(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|e| BraveSearchError::Http(e.to_string()))?;

    if !status.is_success() {
        let snippet: String = body.chars().take(500).collect();
        return Err(BraveSearchError::Http(format!(
            "status {}: {snippet}",
            status.as_u16()
        )));
    }

    let parsed = parse_brave_response_body(&body)?;

    let raw = parsed;
    if raw.is_empty() {
        return Ok(format!("No results for query: {q}"));
    }

    let mut hits = Vec::new();
    for item in raw.into_iter().take(count as usize) {
        let title = item.title.trim().to_string();
        let url = item.url.trim().to_string();
        if url.is_empty() {
            continue;
        }
        let snippet = item.description.trim();
        let content = if !snippet.is_empty() {
            snippet.to_string()
        } else if cfg.fetch_content {
            fetch_page_text(&client, &url, cfg.max_content_chars)
                .unwrap_or_else(|e| format!("(fetch failed: {e})"))
        } else {
            String::new()
        };
        hits.push(WebSearchHit { title, url, content });
    }

    if hits.is_empty() {
        return Ok(format!("No usable results for query: {q}"));
    }

    Ok(format_hits(&hits))
}

fn format_hits(hits: &[WebSearchHit]) -> String {
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        if i > 0 {
            out.push_str("\n---\n");
        }
        out.push_str(&format!("# {}\nURL: {}\n", h.title, h.url));
        if !h.content.is_empty() {
            out.push_str(&h.content);
            if !h.content.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

fn fetch_page_text(
    client: &reqwest::blocking::Client,
    url: &str,
    max_chars: usize,
) -> Result<String, BraveSearchError> {
    let response = client
        .get(url)
        .header("User-Agent", DEFAULT_USER_AGENT)
        .send()
        .map_err(|e| BraveSearchError::Http(e.to_string()))?;

    if !response.status().is_success() {
        return Err(BraveSearchError::Http(format!(
            "page fetch status {}",
            response.status().as_u16()
        )));
    }

    let html = response
        .text()
        .map_err(|e| BraveSearchError::Http(e.to_string()))?;

    Ok(strip_html_to_text(&html, max_chars))
}

/// HTML からざっくりプレーンテキストを抽出（gemini-pad の cheerio 簡易版）。
fn strip_html_to_text(html: &str, max_chars: usize) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript"] {
        while let Some(start) = s.to_lowercase().find(&format!("<{tag}")) {
            if let Some(end) = s[start..].to_lowercase().find(&format!("</{tag}>")) {
                let end = start + end + tag.len() + 3;
                s.replace_range(start..end.min(s.len()), " ");
            } else {
                break;
            }
        }
    }
    let mut out = String::with_capacity(s.len().min(max_chars * 2));
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                out.push(ch);
            }
            _ => {}
        }
    }
    let collapsed: String = out
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() <= max_chars {
        collapsed
    } else {
        collapsed.chars().take(max_chars).collect()
    }
}

/// Brave Web Search API の JSON をパース（`serde_json::Value` で寛容に抽出）。
fn parse_brave_response_body(body: &str) -> Result<Vec<NormalizedResult>, BraveSearchError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(BraveSearchError::Parse("empty response body".into()));
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
        let head: String = trimmed.chars().take(120).collect();
        BraveSearchError::Parse(format!("{e} (body starts with: {head:?})"))
    })?;
    Ok(extract_results_from_value(&value))
}

fn extract_results_from_value(root: &serde_json::Value) -> Vec<NormalizedResult> {
    let mut out = Vec::new();
    if let Some(items) = root
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    {
        push_result_items(items, &mut out);
    }
    if out.is_empty() {
        if let Some(items) = root.get("results").and_then(|r| r.as_array()) {
            push_result_items(items, &mut out);
        }
    }
    out
}

fn push_result_items(items: &[serde_json::Value], out: &mut Vec<NormalizedResult>) {
    for item in items {
        let title = json_str(item, &["title", "name"])
            .or_else(|| json_str(item, &["snippet"]))
            .unwrap_or_default();
        let url = json_str(item, &["url", "link"]).unwrap_or_else(|| {
            item.get("meta_url").and_then(meta_url_to_string).unwrap_or_default()
        });
        let description = json_str(item, &["description", "snippet", "summary"]).unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        out.push(NormalizedResult {
            title,
            url,
            description,
        });
    }
}

fn json_str<'a>(obj: &'a serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = obj.get(*key).and_then(|v| v.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn meta_url_to_string(meta: &serde_json::Value) -> Option<String> {
    let scheme = meta.get("scheme")?.as_str()?;
    let netloc = meta.get("netloc")?.as_str()?;
    let path = meta.get("path").and_then(|v| v.as_str()).unwrap_or("");
    Some(format!("{scheme}://{netloc}{path}"))
}

struct NormalizedResult {
    title: String,
    url: String,
    description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_brave_fixture() {
        let body = include_str!("../tests/fixtures/brave_web_search_sample.json");
        let results = parse_brave_response_body(body).expect("fixture should parse");
        assert!(!results.is_empty());
        assert!(results[0].url.contains("rust"));
    }

    #[test]
    fn parses_brave_web_results_json() {
        let json = r#"{
            "web": {
                "results": [
                    {
                        "title": "Example",
                        "url": "https://example.com",
                        "description": "An example site."
                    }
                ]
            }
        }"#;
        let results = parse_brave_response_body(json).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example");
        assert_eq!(results[0].url, "https://example.com");
    }

    #[test]
    fn strip_html_removes_tags() {
        let html = "<html><body><p>Hello</p> <script>bad()</script> world</body></html>";
        let text = strip_html_to_text(html, 100);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("bad()"));
    }

    /// `cargo test live_brave_search --lib -- --ignored --nocapture`
    #[test]
    #[ignore = "hits Brave API; needs config or BRAVE_SEARCH_API_KEY"]
    fn live_brave_search() {
        let cfg = crate::config::AppConfig::load_path("config/config.json").unwrap();
        let brave = cfg.resolved_brave_search().expect("brave api key");
        let out = search_web(&brave, "明日 岡山市 天気", Some(3)).unwrap_or_else(|e| {
            panic!("search_web failed: {e}");
        });
        eprintln!("{out}");
        assert!(!out.is_empty());
    }

    #[test]
    fn missing_api_key_errors() {
        let cfg = BraveSearchConfig {
            api_key: String::new(),
            max_results: 3,
            fetch_content: false,
            max_content_chars: 512,
        };
        let err = search_web(&cfg, "test", None).unwrap_err();
        assert!(matches!(err, BraveSearchError::MissingApiKey));
    }
}
