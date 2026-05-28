// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit repl` runner.
//!
//! Opens a `rustyline` shell where operators can call existing
//! `invoicekit` subcommands without retyping the binary name. The
//! session keeps lightweight state for the current tenant and invoice
//! draft directory so repeated pack/verify loops stay short.

use std::path::PathBuf;
use std::process::ExitCode;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde::Serialize;
use thiserror::Error;

/// Run `invoicekit repl`.
#[must_use]
pub fn run(raw_args: &[String]) -> ExitCode {
    let parsed = match Args::parse(raw_args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };

    if let Some(line) = parsed.eval {
        let mut session = ReplSession::default();
        return match session.execute_line(&line) {
            Ok(ReplControl::Continue(code)) => code,
            Ok(ReplControl::Exit) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("repl: {err}");
                ExitCode::from(2)
            }
        };
    }

    run_interactive()
}

#[derive(Debug)]
struct Args {
    eval: Option<String>,
}

impl Args {
    fn parse(argv: &[String]) -> Result<Self, String> {
        let mut eval = None;
        let mut iter = argv.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => return Err(usage_help()),
                "--eval" => {
                    let value = iter.next().ok_or_else(|| {
                        format!("repl: --eval needs a command\n\n{}", usage_help())
                    })?;
                    eval = Some(value.clone());
                }
                flag if flag.starts_with('-') => {
                    return Err(format!("repl: unknown flag {flag:?}\n\n{}", usage_help()));
                }
                other => {
                    return Err(format!(
                        "repl: unexpected argument {other:?}\n\n{}",
                        usage_help()
                    ));
                }
            }
        }
        Ok(Self { eval })
    }
}

fn usage_help() -> String {
    "usage: invoicekit repl [--eval COMMAND]\n\nOpen an interactive InvoiceKit shell. Inside the shell, run subcommands without the `invoicekit` prefix; use `tenant`, `draft`, `state`, `help`, or `exit` for REPL-local commands."
        .to_owned()
}

fn run_interactive() -> ExitCode {
    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(err) => {
            eprintln!("repl: failed to initialise line editor: {err}");
            return ExitCode::FAILURE;
        }
    };
    let mut session = ReplSession::default();
    println!("invoicekit repl. Type `help` for commands, `exit` to leave.");

    loop {
        match editor.readline("invoicekit> ") {
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = editor.add_history_entry(line.as_str());
                }
                match session.execute_line(&line) {
                    Ok(ReplControl::Continue(_)) => {}
                    Ok(ReplControl::Exit) => return ExitCode::SUCCESS,
                    Err(err) => eprintln!("repl: {err}"),
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => return ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("repl: readline failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
}

/// Mutable state held for one REPL session.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ReplState {
    /// Current tenant id, injected into `pack` when no explicit
    /// `--tenant` flag is supplied.
    pub current_tenant: Option<String>,
    /// Current invoice draft directory. `pack <output.ikb>` uses
    /// this as the input directory.
    pub current_draft: Option<PathBuf>,
}

/// One stateful REPL session.
#[derive(Debug, Default)]
pub struct ReplSession {
    state: ReplState,
}

impl ReplSession {
    /// Returns the current session state.
    #[must_use]
    pub const fn state(&self) -> &ReplState {
        &self.state
    }

    fn execute_line(&mut self, line: &str) -> Result<ReplControl, ReplError> {
        let words = split_words(line)?;
        self.execute_words(&words)
    }

    fn execute_words(&mut self, words: &[String]) -> Result<ReplControl, ReplError> {
        let Some((command, args)) = words.split_first() else {
            return Ok(ReplControl::Continue(ExitCode::SUCCESS));
        };
        match command.as_str() {
            "exit" | "quit" => Ok(ReplControl::Exit),
            "help" | "?" => {
                print_repl_help();
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            "state" => self.print_state(args),
            "tenant" => self.set_tenant(args),
            "draft" => self.set_draft(args),
            "repl" => {
                eprintln!("repl: already inside the REPL; use `exit` to leave");
                Ok(ReplControl::Continue(ExitCode::from(2)))
            }
            "pack" => {
                let args = self.pack_args(args);
                Ok(ReplControl::Continue(super::pack::run(&args)))
            }
            other => super::dispatch(other, args).map_or_else(
                || {
                    eprintln!("repl: unknown command {other:?}; type `help`");
                    Ok(ReplControl::Continue(ExitCode::from(2)))
                },
                |code| Ok(ReplControl::Continue(code)),
            ),
        }
    }

    fn print_state(&self, args: &[String]) -> Result<ReplControl, ReplError> {
        match args {
            [] => {
                println!(
                    "tenant: {}\ndraft:  {}",
                    self.state.current_tenant.as_deref().unwrap_or("(unset)"),
                    self.state
                        .current_draft
                        .as_ref()
                        .map_or_else(|| "(unset)".to_owned(), |path| path.display().to_string())
                );
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            [flag] if flag == "--json" => {
                let json = serde_json::to_string_pretty(&self.state)
                    .map_err(|err| ReplError::Serialize(err.to_string()))?;
                println!("{json}");
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            _ => Err(ReplError::Usage(
                "state accepts no arguments except --json".to_owned(),
            )),
        }
    }

    fn set_tenant(&mut self, args: &[String]) -> Result<ReplControl, ReplError> {
        match args {
            [] => {
                println!(
                    "{}",
                    self.state.current_tenant.as_deref().unwrap_or("(unset)")
                );
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            [value] if value == "clear" || value == "unset" => {
                self.state.current_tenant = None;
                println!("tenant unset");
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            [value] if !value.trim().is_empty() => {
                self.state.current_tenant = Some(value.clone());
                println!("tenant set to {value}");
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            _ => Err(ReplError::Usage("usage: tenant [ID|clear]".to_owned())),
        }
    }

    fn set_draft(&mut self, args: &[String]) -> Result<ReplControl, ReplError> {
        match args {
            [] => {
                let value = self
                    .state
                    .current_draft
                    .as_ref()
                    .map_or_else(|| "(unset)".to_owned(), |path| path.display().to_string());
                println!("{value}");
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            [value] if value == "clear" || value == "unset" => {
                self.state.current_draft = None;
                println!("draft unset");
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            [value] if !value.trim().is_empty() => {
                self.state.current_draft = Some(PathBuf::from(value));
                println!("draft set to {value}");
                Ok(ReplControl::Continue(ExitCode::SUCCESS))
            }
            _ => Err(ReplError::Usage("usage: draft [PATH|clear]".to_owned())),
        }
    }

    fn pack_args(&self, args: &[String]) -> Vec<String> {
        let mut out = args.to_vec();
        if out.len() == 1 && out.first().is_some_and(|arg| !arg.starts_with('-')) {
            if let Some(draft) = &self.state.current_draft {
                out.insert(0, draft.display().to_string());
            }
        }
        if !has_option(&out, "--tenant") {
            if let Some(tenant) = &self.state.current_tenant {
                out.push("--tenant".to_owned());
                out.push(tenant.clone());
            }
        }
        out
    }
}

fn has_option(args: &[String], option: &str) -> bool {
    args.iter().any(|arg| {
        arg == option
            || arg
                .strip_prefix(option)
                .is_some_and(|rest| rest.starts_with('='))
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ReplControl {
    Continue(ExitCode),
    Exit,
}

/// Errors emitted by the REPL parser and local commands.
#[derive(Debug, Error)]
pub enum ReplError {
    /// A quoted string was not closed before end of line.
    #[error("unclosed quote in command line")]
    UnclosedQuote,
    /// A local command was called with invalid arguments.
    #[error("{0}")]
    Usage(String),
    /// State could not be serialized.
    #[error("state serialise failed: {0}")]
    Serialize(String),
}

fn split_words(line: &str) -> Result<Vec<String>, ReplError> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut in_word = false;

    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            in_word = true;
            continue;
        }
        match (quote, ch) {
            (_, '\\') => {
                escaped = true;
                in_word = true;
            }
            (Some(q), c) if c == q => {
                quote = None;
            }
            (None, '"' | '\'') => {
                quote = Some(ch);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (Some(_) | None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }

    if quote.is_some() {
        return Err(ReplError::UnclosedQuote);
    }
    if escaped {
        current.push('\\');
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

fn print_repl_help() {
    println!(
        "REPL commands:\n  help                 show this help\n  tenant [ID|clear]    show, set, or clear the current tenant\n  draft [PATH|clear]   show, set, or clear the current draft directory\n  state [--json]       print current REPL state\n  exit | quit          leave the shell\n\nInvoiceKit subcommands:\n  capabilities codelist-update diff doctor migrate-archive pack replay show timestamp unpack verify version\n\nState shortcuts:\n  pack <output.ikb>    packs the current draft directory when `draft PATH` is set\n  pack ...             injects `--tenant <ID>` when `tenant ID` is set and no --tenant was supplied"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_words_handles_quotes_and_backslashes() {
        let words = split_words("pack 'sample dir' out.ikb --trace trace\\ 1").unwrap();
        assert_eq!(
            words,
            vec!["pack", "sample dir", "out.ikb", "--trace", "trace 1"]
        );
    }

    #[test]
    fn split_words_keeps_empty_quoted_word() {
        let words = split_words("tenant \"\"").unwrap();
        assert_eq!(words, vec!["tenant", ""]);
    }

    #[test]
    fn split_words_rejects_unclosed_quote() {
        assert!(matches!(
            split_words("tenant 'acme"),
            Err(ReplError::UnclosedQuote)
        ));
    }

    #[test]
    fn tenant_and_draft_persist_in_state() {
        let mut session = ReplSession::default();
        assert_eq!(
            session.execute_line("tenant acme").unwrap(),
            ReplControl::Continue(ExitCode::SUCCESS)
        );
        assert_eq!(
            session.execute_line("draft ./sample").unwrap(),
            ReplControl::Continue(ExitCode::SUCCESS)
        );
        assert_eq!(session.state().current_tenant.as_deref(), Some("acme"));
        assert_eq!(
            session.state().current_draft.as_deref(),
            Some(PathBuf::from("./sample").as_path())
        );
    }

    #[test]
    fn pack_args_use_session_state() {
        let mut session = ReplSession::default();
        session.execute_line("tenant acme").unwrap();
        session.execute_line("draft sample").unwrap();

        let args = session.pack_args(&["dist.ikb".to_owned()]);
        assert_eq!(args, vec!["sample", "dist.ikb", "--tenant", "acme"]);
    }

    #[test]
    fn explicit_pack_tenant_wins_over_state() {
        let mut session = ReplSession::default();
        session.execute_line("tenant acme").unwrap();

        let args = session.pack_args(&[
            "sample".to_owned(),
            "dist.ikb".to_owned(),
            "--tenant".to_owned(),
            "override".to_owned(),
        ]);
        assert_eq!(args, vec!["sample", "dist.ikb", "--tenant", "override"]);
    }

    #[test]
    fn dispatches_existing_subcommand_without_binary_name() {
        let mut session = ReplSession::default();
        assert_eq!(
            session.execute_line("version").unwrap(),
            ReplControl::Continue(ExitCode::SUCCESS)
        );
    }

    #[test]
    fn unknown_command_is_usage_error() {
        let mut session = ReplSession::default();
        assert_eq!(
            session.execute_line("does-not-exist").unwrap(),
            ReplControl::Continue(ExitCode::from(2))
        );
    }

    #[test]
    fn exit_command_ends_session() {
        let mut session = ReplSession::default();
        assert_eq!(session.execute_line("exit").unwrap(), ReplControl::Exit);
    }
}
