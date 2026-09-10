//! stdin 行読取（TTY 判定・パイプ用 lossy 読取）。

use std::io::{self, BufRead, IsTerminal};

use crate::io_utf8::ensure_utf8_stdio;

/// 対話 REPL 用: stdin が TTY か（パイプ/リダイレクトなら false）。
pub fn stdin_is_tty() -> bool {
    io::stdin().is_terminal()
}

/// 子プロセス UTF-8 環境を整えてから REPL を開始する。
pub fn prepare_stdio_for_repl() {
    ensure_utf8_stdio();
}

/// lossy 行を stderr に警告して捨てる。呼び出し側は `true` なら continue する。
pub fn reject_lossy_line(lossy: bool) -> bool {
    if lossy {
        eprintln!(
            "warning: stdin line was not valid UTF-8; input discarded — please re-enter"
        );
        true
    } else {
        false
    }
}

/// 1 行読む。EOF は `Ok(None)`。
/// 不正 UTF-8 は U+FFFD に置換し、`lossy == true` を返す（呼び出し側は [`reject_lossy_line`] で捨てる）。
pub fn read_line_lossy<R: BufRead>(reader: &mut R) -> io::Result<Option<(String, bool)>> {
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    let lossy = std::str::from_utf8(&buf).is_err();
    let line = String::from_utf8_lossy(&buf).into_owned();
    Ok(Some((line, lossy)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn valid_utf8_line() {
        let mut r = Cursor::new(b"hello\n");
        let (line, lossy) = read_line_lossy(&mut r).unwrap().unwrap();
        assert_eq!(line, "hello");
        assert!(!lossy);
    }

    #[test]
    fn strips_crlf() {
        let mut r = Cursor::new(b"a\r\n");
        let (line, lossy) = read_line_lossy(&mut r).unwrap().unwrap();
        assert_eq!(line, "a");
        assert!(!lossy);
    }

    #[test]
    fn invalid_utf8_is_lossy_not_error() {
        let mut r = Cursor::new([0xff, b'\n']);
        let (line, lossy) = read_line_lossy(&mut r).unwrap().unwrap();
        assert!(lossy);
        assert_eq!(line, "\u{FFFD}");
    }

    #[test]
    fn eof_returns_none() {
        let mut r = Cursor::new(b"");
        assert!(read_line_lossy(&mut r).unwrap().is_none());
    }

    #[test]
    fn reject_lossy_line_warns() {
        assert!(!reject_lossy_line(false));
        assert!(reject_lossy_line(true));
    }

    #[test]
    fn japanese_utf8_ok() {
        let mut r = Cursor::new("あれ？\n".as_bytes());
        let (line, lossy) = read_line_lossy(&mut r).unwrap().unwrap();
        assert_eq!(line, "あれ？");
        assert!(!lossy);
    }
}
