//! stdin 行読取（不正 UTF-8 でも REPL を落とさない）。

use std::io::{self, BufRead};

/// 1 行読む。EOF は `Ok(None)`。
/// 不正 UTF-8 は U+FFFD に置換し、`lossy == true` を返す。
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
    fn japanese_utf8_ok() {
        let mut r = Cursor::new("あれ？\n".as_bytes());
        let (line, lossy) = read_line_lossy(&mut r).unwrap().unwrap();
        assert_eq!(line, "あれ？");
        assert!(!lossy);
    }
}
