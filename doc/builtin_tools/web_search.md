# `web_search`

Brave Search API で Web 検索し、タイトル・URL・スニペット（必要ならページ本文）を返す。

## 設定（`config/config.json`）

```json
{
  "tools": {
    "brave_search": {
      "api_key": "YOUR_BRAVE_API_KEY",
      "max_results": 5,
      "fetch_content": false,
      "max_content_chars": 2048
    }
  }
}
```

| キー | 意味 | 既定 |
|------|------|------|
| `api_key` | Brave Search API キー | 未設定時は環境変数 `BRAVE_SEARCH_API_KEY` |
| `max_results` | 1 回の検索で返す最大件数（1–20） | `5` |
| `fetch_content` | API の description が空のとき、結果 URL を HTTP 取得して本文化 | `false` |
| `max_content_chars` | 本文取得時の上限文字数 | `2048` |

API キーは [Brave Search API](https://brave.com/search/api/) で取得。

## 引数

```json
{ "query": "Rust async book", "count": 3 }
```

| フィールド | 必須 | 説明 |
|------------|------|------|
| `query` | はい | 検索クエリ（400 文字で切り詰め） |
| `count` | いいえ | 件数（設定の `max_results` を超えない） |

## 実装

- `src/brave_search.rs` — `https://api.search.brave.com/res/v1/web/search`（gemini-pad の `braveSearch.mjs` と同様に `X-Subscription-Token`）
- `src/tool.rs` — `ToolRuntime::run_web_search`

キー未設定時は `web_search: configure tools.brave_search.api_key...` で失敗する。
