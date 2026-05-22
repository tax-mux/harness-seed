//! 起動時に一度だけ検出する OS / シェル実行環境。

use std::env;
use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

/// OS ファミリ（プロンプト・コマンド方針用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsFamily {
    Windows,
    Unix,
}

/// 検出したシェル種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Cmd,
    PowerShell,
    Posix,
}

/// 起動時に検出した実行環境（`run_cmd` と LLM プロンプトで共有）。
#[derive(Debug, Clone)]
pub struct RuntimeEnvironment {
    pub os_family: OsFamily,
    pub os: String,
    pub arch: String,
    pub shell_kind: ShellKind,
    /// `Command::new` に渡すプログラム（PATH 上の名前または絶対パス）。
    pub shell_program: String,
    /// 人間向け表示名（例: `PowerShell (pwsh)`）。
    pub shell_label: String,
}

impl RuntimeEnvironment {
    /// OS と利用可能なシェルを自動検出する。
    pub fn detect() -> Self {
        let os = std::env::consts::OS.to_string();
        let arch = std::env::consts::ARCH.to_string();
        #[cfg(windows)]
        {
            let _ = (&os, &arch);
            Self::detect_windows()
        }
        #[cfg(not(windows))]
        {
            let _ = (&os, &arch);
            Self::detect_unix(os, arch)
        }
    }

    #[cfg(windows)]
    fn detect_windows() -> Self {
        if program_runs("pwsh", &["-NoLogo", "-Command", "exit 0"]) {
            return Self {
                os_family: OsFamily::Windows,
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                shell_kind: ShellKind::PowerShell,
                shell_program: "pwsh".into(),
                shell_label: "PowerShell (pwsh)".into(),
            };
        }
        if program_runs("powershell", &["-NoLogo", "-Command", "exit 0"]) {
            return Self {
                os_family: OsFamily::Windows,
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                shell_kind: ShellKind::PowerShell,
                shell_program: "powershell".into(),
                shell_label: "Windows PowerShell".into(),
            };
        }
        let comspec = env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        let label = if comspec.to_ascii_lowercase().ends_with("cmd.exe") {
            "Command Prompt (cmd)"
        } else {
            "COMSPEC shell"
        };
        Self {
            os_family: OsFamily::Windows,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            shell_kind: ShellKind::Cmd,
            shell_program: comspec,
            shell_label: label.into(),
        }
    }

    #[cfg(not(windows))]
    fn detect_unix(os: String, arch: String) -> Self {
        if let Ok(shell) = env::var("SHELL") {
            let shell = shell.trim().to_string();
            if !shell.is_empty() && program_runs(&shell, &["-c", "exit 0"]) {
                let label = shell_label_from_path(&shell);
                return Self {
                    os_family: OsFamily::Unix,
                    os,
                    arch,
                    shell_kind: ShellKind::Posix,
                    shell_program: shell,
                    shell_label: label,
                };
            }
        }
        if program_runs("bash", &["-c", "exit 0"]) {
            return Self {
                os_family: OsFamily::Unix,
                os,
                arch,
                shell_kind: ShellKind::Posix,
                shell_program: "bash".into(),
                shell_label: "bash".into(),
            };
        }
        Self {
            os_family: OsFamily::Unix,
            os,
            arch,
            shell_kind: ShellKind::Posix,
            shell_program: "sh".into(),
            shell_label: "sh".into(),
        }
    }

    /// 起動ログ用 1 行。
    pub fn summary_line(&self) -> String {
        format!(
            "{} / {} — {}",
            self.os, self.arch, self.shell_label
        )
    }

    /// system プロンプトへ追記する短文。
    pub fn prompt_hint(&self) -> String {
        let shell_notes = match (self.os_family, self.shell_kind) {
            (OsFamily::Windows, ShellKind::PowerShell) => {
                "Use PowerShell syntax for run_cmd (e.g. Get-ChildItem, not Unix-only commands unless available in PATH)."
            }
            (OsFamily::Windows, ShellKind::Cmd) => {
                "Use cmd.exe syntax for run_cmd (e.g. dir, type). Prefer single-line commands."
            }
            (OsFamily::Windows, ShellKind::Posix) | (OsFamily::Unix, ShellKind::Posix) => {
                "Use POSIX shell syntax for run_cmd (sh/bash). Paths use forward slashes."
            }
            (OsFamily::Unix, ShellKind::Cmd) | (OsFamily::Unix, ShellKind::PowerShell) => {
                "Use the detected shell's syntax for run_cmd."
            }
        };
        format!(
            "OS: {} ({}) | Shell: {} | Program: {}\n{shell_notes}",
            self.os, self.arch, self.shell_label, self.shell_program
        )
    }

    /// ワークスペース内でシェルコマンドを実行する。
    pub fn run_shell_command(&self, command: &str, cwd: &Path) -> Result<std::process::Output, String> {
        let mut cmd = Command::new(&self.shell_program);
        match self.shell_kind {
            ShellKind::Cmd => {
                cmd.args(["/C", command]);
            }
            ShellKind::PowerShell => {
                cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
            }
            ShellKind::Posix => {
                cmd.args(["-c", command]);
            }
        }
        cmd.current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.output()
            .map_err(|e| format!("run_cmd spawn failed ({}): {e}", self.shell_program))
    }
}

impl fmt::Display for RuntimeEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary_line())
    }
}

fn program_runs(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn shell_label_from_path(shell: &str) -> String {
    Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(shell)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_usable_environment() {
        let env = RuntimeEnvironment::detect();
        assert!(!env.shell_program.is_empty());
        assert!(!env.shell_label.is_empty());
        assert!(!env.prompt_hint().is_empty());
    }

    #[test]
    fn run_shell_echo() {
        let env = RuntimeEnvironment::detect();
        let cwd = std::env::current_dir().expect("cwd");
        #[cfg(windows)]
        let command = "echo harness_runtime_test";
        #[cfg(not(windows))]
        let command = "echo harness_runtime_test";
        let out = env.run_shell_command(command, &cwd).expect("shell run");
        assert!(out.status.success() || !out.stdout.is_empty());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("harness_runtime_test"));
    }
}
