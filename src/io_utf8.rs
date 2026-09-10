//! 子プロセス・stdio の UTF-8 既定（OpenCode StringDecoder / Hermes bootstrap 相当）。

use std::process::Command;

/// このプロセスから spawn する子の UTF-8 環境を整える（Hermes `hermes_bootstrap` 相当）。
pub fn ensure_utf8_stdio() {
    std::env::set_var("PYTHONUTF8", "1");
    std::env::set_var("PYTHONIOENCODING", "utf-8");
    #[cfg(windows)]
    {
        for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
            if std::env::var(key).is_err() {
                std::env::set_var(key, "C.UTF-8");
            }
        }
    }
}

/// `Command` に UTF-8 子プロセス用 env を付与（OpenCode PTY / Hermes 子プロセス相当）。
pub fn apply_utf8_child_env(cmd: &mut Command) {
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONIOENCODING", "utf-8");
    #[cfg(windows)]
    {
        cmd.env("LC_ALL", "C.UTF-8");
        cmd.env("LC_CTYPE", "C.UTF-8");
        cmd.env("LANG", "C.UTF-8");
    }
}

/// ストリーム chunk を UTF-8 文字列に結合（Node `StringDecoder` 相当）。
#[derive(Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(s) => {
                    out.push_str(s);
                    self.pending.clear();
                    break;
                }
                Err(err) => {
                    let valid = err.valid_up_to();
                    if valid == 0 {
                        if err.error_len().is_some() {
                            out.push('\u{FFFD}');
                            let skip = err.error_len().unwrap_or(1);
                            self.pending.drain(..skip);
                            continue;
                        }
                        break;
                    }
                    out.push_str(
                        std::str::from_utf8(&self.pending[..valid])
                            .expect("valid_up_to"),
                    );
                    self.pending.drain(..valid);
                }
            }
        }
        out
    }

    pub fn finish(self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        String::from_utf8_lossy(&self.pending).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_splits_multibyte_across_chunks() {
        let emdash = "—";
        let bytes = emdash.as_bytes();
        let mut dec = Utf8StreamDecoder::default();
        let a = dec.push(&bytes[..2]);
        assert!(a.is_empty());
        let b = dec.push(&bytes[2..]);
        assert_eq!(a + &b, emdash);
    }

    #[test]
    fn decoder_replaces_invalid_byte() {
        let mut dec = Utf8StreamDecoder::default();
        assert_eq!(dec.push(&[0xff]), "\u{FFFD}");
    }
}
