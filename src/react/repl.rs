//! 対話 REPL（TTY は rustyline、パイプは lossy 一行読取）。

use std::io::{self, Write};

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::brain::AgentBrain;
use crate::line_io::{prepare_stdio_for_repl, read_line_lossy, reject_lossy_line, stdin_is_tty};

use super::ReActLoop;

pub fn run_repl<E: AgentBrain>(loop_engine: &mut ReActLoop<E>, verbose: bool) -> io::Result<()> {
    loop_engine.apply_cli_verbose(verbose);
    prepare_stdio_for_repl();

    println!(
        "HarnessSeed ReAct REPL — 'help' でコマンド一覧、'clear' で短期記憶リセット、'quit' で終了"
    );

    if stdin_is_tty() {
        run_repl_tty(loop_engine, verbose)
    } else {
        eprintln!("note: stdin is not a TTY — piped input mode");
        run_repl_piped(loop_engine, verbose)
    }
}

fn run_repl_tty<E: AgentBrain>(
    loop_engine: &mut ReActLoop<E>,
    verbose: bool,
) -> io::Result<()> {
    let mut rl = DefaultEditor::new().map_err(|e| io::Error::other(e.to_string()))?;

    loop {
        let line = match rl.readline("> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => {
                println!();
                break;
            }
            Err(err) => return Err(io::Error::other(err.to_string())),
        };

        if line.trim().is_empty() {
            continue;
        }

        if !handle_input(loop_engine, verbose, line.trim())? {
            break;
        }
    }

    Ok(())
}

fn run_repl_piped<E: AgentBrain>(
    loop_engine: &mut ReActLoop<E>,
    verbose: bool,
) -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        print!("> ");
        io::stdout().flush()?;

        let Some((line, lossy)) = read_line_lossy(&mut reader)? else {
            println!();
            break;
        };
        if reject_lossy_line(lossy) {
            continue;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        if !handle_input(loop_engine, verbose, input)? {
            break;
        }
    }

    Ok(())
}

/// 入力1行を処理。終了なら `Ok(false)`。
fn handle_input<E: AgentBrain>(
    loop_engine: &mut ReActLoop<E>,
    verbose: bool,
    input: &str,
) -> io::Result<bool> {
    if matches!(input, "quit" | "exit" | "q") {
        return Ok(false);
    }
    if matches!(input, "clear" | "forget" | "reset") {
        loop_engine.session.clear();
        println!("session memory cleared");
        return Ok(true);
    }

    if loop_engine.config.stream_mode {
        // ストリームモード: 実行層の Thought・Answer を逐次出力。本文は再出力しない。
        let result = loop_engine.run_turn_stream(input, |chunk| {
            use std::io::Write;
            let _ = std::io::Stdout::write_all(&mut std::io::stdout(), chunk.as_bytes());
        });
        match result {
            Ok(result) => {
                if verbose {
                    eprintln!("--- trace ---\n{}", result.trace);
                }
                eprintln!();
            }
            Err(err) => eprintln!("error: {err}"),
        }
        return Ok(true);
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
    Ok(true)
}
