//! 対話 REPL。
use std::io::{self, Write};

use crate::brain::AgentBrain;
use crate::line_io::read_line_lossy;

use super::ReActLoop;

pub fn run_repl<E: AgentBrain>(loop_engine: &mut ReActLoop<E>, verbose: bool) -> io::Result<()> {
    loop_engine.apply_cli_verbose(verbose);

    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    println!(
        "HarnessSeed ReAct REPL — 'help' でコマンド一覧、'clear' で短期記憶リセット、'quit' で終了"
    );

    loop {
        print!("> ");
        io::stdout().flush()?;

        let Some((line, lossy)) = read_line_lossy(&mut stdin)? else {
            println!();
            break;
        };
        if lossy {
            eprintln!("warning: stdin line was not valid UTF-8; invalid bytes replaced");
        }

        let input = line.trim();
        // 不正バイトのみの行は空行扱い（文字削除で壊れた残骸だけのとき）
        if input.is_empty() || (lossy && input.chars().all(|c| c == '\u{FFFD}')) {
            continue;
        }
        if matches!(input, "quit" | "exit" | "q") {
            break;
        }
        if matches!(input, "clear" | "forget" | "reset") {
            loop_engine.session.clear();
            println!("session memory cleared");
            continue;
        }

        match loop_engine.run_turn(input) {
            Ok(result) => {
                if verbose {
                    eprintln!("--- trace ---\n{}", result.trace);
                }
                println!("{}", result.answer);
            }
            Err(err) => eprintln!("error: {err}"),
        }
    }

    Ok(())
}
