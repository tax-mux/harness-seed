//! 汎用 SSE (Server-Sent Events) データストリームパーサ。
//!
//! 生 SSE のバイト列をチャンク単位で給餌し、イベント（空行区切り）単位へ再構成して
//! イベントの `data` バッファを呼び出し元に渡す。
//!
//! 設計方針:
//! - **トークン/Chat 概念に依存しない。** パーサは「バイト列をイベントへ分割し
//!   `data` フィールドを集約する」ことだけを行う。`data` の中身（JSON トークンの
//!   抽出や `[DONE]` 以外の解釈）は呼び出し元（各コネクタ）が所有する。
//! - **破断データの構造化。** `[DONE]` 終端・`event: error` 等は
//!   [`SseEventKind`] として構造化し、`on_event` へ種別を渡す。
//! - **バイト単位。** `from_utf8` 文字列化コピーを入れない。マルチバイト文字が
//!   チャンク境界で割れてもイベント完了時に再構成され、呼び出し元が必要なときに
//!   だけ decode する。
//! - **アロケート削減。** `line_buf` / `data` / `event` の集約バッファはイベント間で
//!   再利用される（`clear()` は容量を保持）。イベントごとに新規アロケートしない。
//!   [`SseEvent`] はパーサ内部バッファの**借用**であり、`feed()` / `finalize()`
//!   の返却前まで有効。保持が必要な場合はスコープ内で `to_vec()` /
//!   `std::str::from_utf8` 等でコピーする。
//!
//! 使い方（例、コネクタ側）:
//! ```ignore
//! let mut p = SseParser::new();
//! for chunk in byte_stream {
//!     p.feed(chunk, &mut |ev| {
//!         match ev.kind {
//!             SseEventKind::Done => finish(),
//!             SseEventKind::Error => report(ev.data),
//!             SseEventKind::Data => { /* ev.data から JSON トークンを抽出 */ }
//!         };
//!     });
//! }
//! p.finalize(&mut on_event); // 末尾が空行で終わらなかった残りイベントを送出
//! ```

/// 終端センチネル（OpenAI 互換）。`data` ペイロードがこれ（左右 ASCII
/// スペースを除いて）に一致するイベントは [`SseEventKind::Done`] として通知される。
pub const DONE_SENTINEL: &[u8] = b"[DONE]";

/// 構造化イベントの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseEventKind {
    /// 通常の `data` イベント。
    Data,
    /// `data: [DONE]` の終端イベント。
    Done,
    /// `event: error` フィールドを持つエラーイベント（ペイロードは `data`）。
    Error,
}

impl SseEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Done => "done",
            Self::Error => "error",
        }
    }
}

/// 1 解析済み SSE イベント。
///
/// `event` / `data` はパーサ内部バッファの借用であり、**次回の `feed()` /
/// `finalize()` を呼ぶと無効になる**。保持が必要な場合はスコープ内で
/// `to_vec()` / `std::str::from_utf8` 等でコピーする。
#[derive(Debug)]
pub struct SseEvent<'s> {
    /// イベント種別（`Done` / `Error` / `Data`）。
    pub kind: SseEventKind,
    /// `event:` フィールドがあればその名（バイト列・借用）。
    pub event: Option<&'s [u8]>,
    /// `data` ペイロード（複数 `data:` 行なら改行 `\n` で結合・借用）。
    pub data: &'s [u8],
}

/// 進行中イベントの集約状態（`data` / `event` バッファ + フラグ）。
///
/// パーサ本体と分離することで、[`SseEvent`] の借用とバッファのリセットが
/// 同じスコープ内で交差するのを避ける。
struct EventBuffer {
    /// `data` 集約（行間は `\n` で結合）。
    data: Vec<u8>,
    /// `event` フィールド名。
    event_name: Vec<u8>,
    have_data: bool,
    have_event: bool,
}

impl EventBuffer {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            event_name: Vec::new(),
            have_data: false,
            have_event: false,
        }
    }

    fn is_pending(&self) -> bool {
        self.have_data || self.have_event
    }

    /// 1 行を適用する（空行・コメント・`data` / `event` フィールド）。
    /// 空行はイベント送出のトリガ（パーサの `feed` / `finalize` が送出）。
    fn apply_line(&mut self, line: &[u8]) {
        if line.is_empty() {
            return;
        }
        if line.starts_with(b":") {
            // コメント（keepalive 等）は無視
            return;
        }
        // `field:value` — 冒頭のスペース 1 つは仕様に従い取り除く
        let (field, value) = match line.iter().position(|b| *b == b':') {
            Some(i) => {
                let value = &line[i + 1..];
                (&line[..i], if value.starts_with(b" ") { &value[1..] } else { value })
            }
            None => (line, &b""[..]),
        };
        match field {
            b"data" => {
                if self.have_data {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
                self.have_data = true;
            }
            b"event" => {
                self.event_name.clear();
                self.event_name.extend_from_slice(value);
                self.have_event = true;
            }
            // `id:` / `retry:` 等はトークン抽出に不要なので無視
            _ => {}
        }
    }

    /// 進行中イベントを種別付で `on_event` に渡し、バッファを再利用可能にする。
    /// 進行中イベントが無い場合は何もしない。
    fn emit(&mut self, on_event: &mut dyn for<'s> FnMut(&SseEvent<'s>)) {
        if !self.is_pending() {
            return;
        }
        let kind = if trimmed(&self.data) == DONE_SENTINEL {
            SseEventKind::Done
        } else if self.have_event && self.event_name.eq_ignore_ascii_case(b"error") {
            SseEventKind::Error
        } else {
            SseEventKind::Data
        };
        let ev = SseEvent {
            kind,
            event: if self.have_event { Some(&self.event_name) } else { None },
            data: &self.data,
        };
        on_event(&ev);
    }

    fn reset(&mut self) {
        self.data.clear();
        self.event_name.clear();
        self.have_data = false;
        self.have_event = false;
    }
}

/// SSE バイトストリームをイベントへ分割するパーサ。
pub struct SseParser {
    /// 進行中（末尾 newline 未到達）行バッファ。
    line_buf: Vec<u8>,
    /// 進行中イベントの集約状態。
    event: EventBuffer,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            line_buf: Vec::new(),
            event: EventBuffer::new(),
        }
    }

    /// 生バイトのチャンク 1 つを給餌する。完了したイベントを `on_event` に順に渡す。
    ///
    /// `on_event` が受け取った [`SseEvent`] は `feed()` 返却前（または次の
    /// `feed()` / `finalize()` 呼び出しまで）有効。`on_event` 内で再帰的に
    /// `feed()` / `finalize()` を呼ぶのは禁止。
    ///
    /// 内部バッファは行・イベント間で再利用されるため、イベントごとに新規
    /// アロケートしない（呼び出し元が保持する場合は自分でコピーする）。
    pub fn feed(&mut self, chunk: &[u8], on_event: &mut dyn for<'s> FnMut(&SseEvent<'s>)) {
        self.line_buf.extend_from_slice(chunk);
        loop {
            let Some((line_len, term_len)) = split_next_line(&self.line_buf) else {
                return;
            };
            if line_len > 0 {
                let line = &self.line_buf[..line_len];
                self.event.apply_line(line);
            }
            self.line_buf.drain(..line_len + term_len);
            if line_len == 0 {
                // 空行 = イベント送出（emit→reset は emit が借用済みバッファに
                // 触れないよう独立スコープで実行する）
                self.event.emit(on_event);
                self.event.reset();
            }
        }
    }

    /// ストリーム終端。末尾が空行で終わっておらず進行中イベントが残っていたら
    /// 1 件送出する。呼び出し後は `SseParser` を廃棄する。
    pub fn finalize(self, on_event: &mut dyn for<'s> FnMut(&SseEvent<'s>)) {
        let mut p = self;
        if !p.line_buf.is_empty() {
            // 末尾 newline 未着行を 1 行として適用
            let line = std::mem::take(&mut p.line_buf);
            p.event.apply_line(&line);
        }
        p.event.emit(on_event);
        p.event.reset();
    }
}

/// `buf` から次の行（行末 newline を含まない部分）の位置を返す。
///
/// 戻り値 `(line_len, term_len)`。`term_len` は `1`（`\n` / 単独 `\r`）または
/// `2`（`\r\n`）。行末が現れなければ（チャンク途中）`None`。
fn split_next_line(buf: &[u8]) -> Option<(usize, usize)> {
    for (i, b) in buf.iter().enumerate() {
        match b {
            b'\n' => return Some((i, 1)),
            b'\r' => {
                let term_len = if buf.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                return Some((i, term_len));
            }
            _ => {}
        }
    }
    None
}

/// ASCII スペース両端を削ったスライスを返す（アロケートしない）。
fn trimmed(data: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < data.len() && data[start] == b' ' {
        start += 1;
    }
    let mut end = data.len();
    while end > start && data[end - 1] == b' ' {
        end -= 1;
    }
    &data[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(chunks: &[&[u8]]) -> Vec<(SseEventKind, Option<String>, String)> {
        let mut p = SseParser::new();
        let mut out: Vec<(SseEventKind, Option<String>, String)> = Vec::new();
        let mut on = |ev: &SseEvent| {
            out.push((
                ev.kind,
                ev.event.map(|e| String::from_utf8_lossy(e).into_owned()),
                String::from_utf8_lossy(ev.data).into_owned(),
            ));
        };
        for c in chunks {
            p.feed(c, &mut on);
        }
        p.finalize(&mut on);
        out
    }

    #[test]
    fn emits_events_in_order() {
        let stream = b"data: alpha\n\ndata: beta\n\ndata: [DONE]\n\n";
        let out = feed_all(&[stream]);
        assert_eq!(
            out,
            vec![
                (SseEventKind::Data, None, "alpha".into()),
                (SseEventKind::Data, None, "beta".into()),
                (SseEventKind::Done, None, "[DONE]".into()),
            ]
        );
    }

    #[test]
    fn reassembles_events_split_across_chunks() {
        // イベント境界・行途中・行間で断つ
        let chunks: Vec<&[u8]> = vec![
            b"data: al",
            b"pha\n\nda",
            b"ta: beta\n\n",
            b"dat",
            b"a: [DONE]\n\n",
        ];
        let out = feed_all(&chunks);
        assert_eq!(
            out,
            vec![
                (SseEventKind::Data, None, "alpha".into()),
                (SseEventKind::Data, None, "beta".into()),
                (SseEventKind::Done, None, "[DONE]".into()),
            ]
        );
    }

    #[test]
    fn done_sentinel_is_structured() {
        let out = feed_all(&[&b"data:  [DONE] \n\n"[..]]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, SseEventKind::Done);
        assert_eq!(out[0].2, " [DONE] "); // 冒頭1つのスペースのみ除去される
    }

    #[test]
    fn multibyte_char_split_at_chunk_boundary_reassembles() {
        // 「ストリーム」を「ス」(E3 82 B9) の 2 バイト目 0x82 の直後にチャンクする
        let full: Vec<u8> = "data: ストリーム\n\n".as_bytes().to_vec();
        let split = full.iter().position(|b| *b == 0x82).unwrap() + 1;
        let out = feed_all(&[&full[..split], &full[split..]]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, SseEventKind::Data);
        // 呼び出し元が有効な UTF-8 として検知できる（壊れていない）
        assert_eq!(out[0].2, "ストリーム");
    }

    #[test]
    fn error_event_is_structured_with_payload() {
        let stream = br#"event: error
data: {"type":"error","message":"rate limit"}

"#;
        let out = feed_all(&[stream]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, SseEventKind::Error);
        assert_eq!(out[0].1.as_deref(), Some("error"));
        assert!(out[0].2.contains("rate limit"));
    }

    #[test]
    fn comment_lines_are_ignored_and_multi_data_lines_join() {
        let stream = b": keepalive\n\ndata: line1\ndata: line2\n\n";
        let out = feed_all(&[stream]);
        assert_eq!(
            out,
            vec![(SseEventKind::Data, None, "line1\nline2".into())]
        );
    }

    #[test]
    fn crlf_and_cr_terminators_are_supported() {
        let crlf = feed_all(&[&b"data: a\r\n\r\ndata: [DONE]\r\n\r\n"[..]]);
        assert_eq!(crlf.len(), 2);
        assert_eq!(crlf[0].2, "a");
        assert_eq!(crlf[1].0, SseEventKind::Done);

        let cr = feed_all(&[&b"data: b\r\rdata: c\r\r"[..]]);
        assert_eq!(
            cr,
            vec![
                (SseEventKind::Data, None, "b".into()),
                (SseEventKind::Data, None, "c".into()),
            ]
        );
    }

    #[test]
    fn finalize_flushes_unterminated_event() {
        let out = feed_all(&[&b"data: tail"[..]]);
        assert_eq!(out, vec![(SseEventKind::Data, None, "tail".into())]);
    }

    #[test]
    fn event_without_data_is_still_delivered_with_name() {
        let out = feed_all(&[&b"event: ping\n\n"[..]]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, SseEventKind::Data);
        assert_eq!(out[0].1.as_deref(), Some("ping"));
        assert_eq!(out[0].2, "");
    }

    #[test]
    fn data_field_preserves_inner_space() {
        // 冒頭のスペース 1 つだけを剥ぐ（データ内の空白は保持）
        let out = feed_all(&[&b"data:   spaced\n\n"[..]]);
        assert_eq!(out[0].2, "  spaced");
    }

    #[test]
    fn repeated_feed_keeps_buffers_reused_without_leaks() {
        // 同一パーサへの長尺 feed が混在してもイベントを正しく分割すること
        let mut p = SseParser::new();
        let mut count = 0u32;
        let mut on = |ev: &SseEvent| {
            count += 1;
            let _ = ev.data; // 借用を消費
        };
        for i in 0..1000u32 {
            let chunk = format!("data: tok-{i}\n\ndata: [DONE]\n\n").into_bytes();
            p.feed(&chunk, &mut on);
        }
        assert_eq!(count, 2000);
    }
}
