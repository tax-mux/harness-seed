//! 対話 REPL。
use std::io::{self};

use crate::brain::AgentBrain;

use super::ReActLoop;

pub fn run_repl<E: AgentBrain>(loop_engine: &mut ReActLoop<E>, verbose: bool) -> io::Result<()> {
    loop_engine.apply_cli_verbose(verbose);

    let stdin = io::stdin();
    let mut line = String::new();

    println!(
        "HarnessSeed ReAct REPL — 'help' でコマンド一覧、'clear' で短期記憶リセット、'quit' で終了"
    );

    loop {
        line.clear();
        print!("> ");
        io::Write::flush(&mut io::stdout())?;

        if stdin.read_line(&mut line)? == 0 {
            println!();
            break;
        }

        let input = line.trim();
        if input.is_empty() {
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
