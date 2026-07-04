//! クエリと本文の関連判定（話題転換時の文脈混入を抑える）。

/// 空白区切りトークン、または CJK 向けの部分文字列で照合する。
///
/// CJK は 3 文字未満の n-gram を使わない（「って」「なん」など助詞・断片の誤爆防止）。
pub fn text_matches_query(hay: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() || hay.is_empty() {
        return false;
    }
    let hay_l = hay.to_lowercase();
    let query_l = query.to_lowercase();
    if hay_l.contains(&query_l) {
        return true;
    }

    let tokens: Vec<&str> = query_l
        .split_whitespace()
        .filter(|t| t.chars().count() >= 2)
        .collect();
    if tokens.len() >= 2 {
        let hits = tokens.iter().filter(|t| hay_l.contains(*t)).count();
        return hits * 2 >= tokens.len();
    }
    if tokens.len() == 1 {
        let t = tokens[0];
        if hay_l.contains(t) {
            return true;
        }
        // 空白なし CJK 一文は「全文一致」ではなく n-gram へ（単一 ASCII 語はここで終了）
        if t.chars().all(|c| c.is_ascii()) {
            return false;
        }
    }

    // CJK: 長い部分一致を優先し、最短は 3 文字
    let chars: Vec<char> = query_l.chars().collect();
    if chars.len() < 3 {
        return false;
    }
    let max_len = chars.len().min(8);
    for len in (3..=max_len).rev() {
        for i in 0..=(chars.len() - len) {
            let sub: String = chars[i..i + len].iter().collect();
            if hay_l.contains(&sub) {
                return true;
            }
        }
    }
    false
}

/// 「続きやって」系の曖昧フォローアップか。
pub fn looks_like_continuation(query: &str) -> bool {
    let t = query.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_lowercase();
    const HINTS: &[&str] = &[
        "続き",
        "つづき",
        "それで",
        "あと",
        "もう少し",
        "もっと",
        "同じ",
        "さっき",
        "前の",
        "上記",
        "それも",
        "あれも",
        "これも",
        "もう一度",
        "再度",
        "continue",
        "again",
        "same as",
        "more detail",
        "もっと詳しく",
    ];
    HINTS.iter().any(|h| lower.contains(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_does_not_match_on_common_bigram() {
        // このプロジェクトについて説明して / ファルモってなんじゃ
        let hay = "\u{3053}\u{306e}\u{30d7}\u{30ed}\u{30b8}\u{30a7}\u{30af}\u{30c8}\u{306b}\u{3064}\u{3044}\u{3066}\u{8aac}\u{660e}\u{3057}\u{3066}\nHarnessSeed";
        let q = "\u{30d5}\u{30a1}\u{30eb}\u{30e2}\u{3063}\u{3066}\u{306a}\u{3093}\u{3058}\u{3083}";
        assert!(!text_matches_query(hay, q));
    }

    #[test]
    fn cjk_matches_shared_content_word() {
        // ファルモ導入のメモと事例 / ファルモってなんじゃ
        let hay = "\u{30d5}\u{30a1}\u{30eb}\u{30e2}\u{5c0e}\u{5165}\u{306e}\u{30e1}\u{30e2}\u{3068}\u{4e8b}\u{4f8b}";
        let q = "\u{30d5}\u{30a1}\u{30eb}\u{30e2}\u{3063}\u{3066}\u{306a}\u{3093}\u{3058}\u{3083}";
        assert!(
            text_matches_query(hay, q),
            "hay={hay:?} q={q:?} contains={}",
            hay.contains("\u{30d5}\u{30a1}\u{30eb}\u{30e2}")
        );
    }

    #[test]
    fn latin_token_majority() {
        let hay = "fix harness seed architecture";
        assert!(text_matches_query(hay, "harness architecture notes"));
        assert!(!text_matches_query(hay, "falmo chatbot platform"));
    }

    #[test]
    fn continuation_hints() {
        assert!(looks_like_continuation("続きやって"));
        assert!(looks_like_continuation("もっと詳しく"));
        assert!(!looks_like_continuation("ファルモってなんじゃ"));
        assert!(!looks_like_continuation("このプロジェクトについて説明して"));
    }
}
