mod history;
mod render;
mod search;
mod tui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::history::{Conversation, LoadOptions};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Args {
    /// Open a specific Codex JSONL session file.
    input_file: Option<PathBuf>,

    /// Override Codex home. Defaults to CODEX_HOME or ~/.codex.
    #[arg(long)]
    codex_home: Option<PathBuf>,

    /// Start with the current workspace filter enabled.
    #[arg(short, long)]
    local: bool,

    /// Search query for non-interactive output, or initial TUI query.
    #[arg(short, long)]
    query: Option<String>,

    /// Print the selected conversation file path and exit.
    #[arg(long)]
    show_path: bool,

    /// Print the selected session id and exit.
    #[arg(long)]
    show_id: bool,

    /// Render a selected/input conversation in plain text.
    #[arg(long)]
    plain: bool,

    /// Include function calls and tool outputs in rendered/searchable text.
    #[arg(long)]
    show_tools: bool,

    /// Include reasoning summaries when present.
    #[arg(long)]
    show_reasoning: bool,

    /// Resume the selected session with `codex resume <SESSION_ID>`.
    #[arg(short, long)]
    resume: bool,

    /// Fork the selected session with `codex fork <SESSION_ID>`.
    #[arg(short, long)]
    fork: bool,

    /// Limit non-interactive search results.
    #[arg(long, default_value_t = 20)]
    limit: usize,

    /// Show parse warnings while loading conversations.
    #[arg(long)]
    debug: bool,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let codex_home = history::codex_home(args.codex_home)?;

    if let Some(path) = args.input_file {
        let conversation =
            history::load_conversation(&path, &history::history_titles(&codex_home)?)
                .with_context(|| format!("failed to read {}", path.display()))?;
        if args.show_path {
            println!("{}", conversation.path.display());
            return Ok(());
        }
        if args.show_id {
            println!("{}", conversation.session_id);
            return Ok(());
        }
        if args.resume || args.fork {
            return launch_codex(&conversation, args.fork);
        }
        render::print_conversation(&conversation, args.show_tools, args.show_reasoning);
        return Ok(());
    }

    let current_dir = std::env::current_dir().context("failed to read current directory")?;
    let load_options = LoadOptions {
        codex_home,
        current_dir,
        show_tools: args.show_tools,
        show_reasoning: args.show_reasoning,
        debug: args.debug,
    };
    let conversations = history::load_conversations(&load_options)?;
    if conversations.is_empty() {
        bail!(
            "no Codex sessions found under {}",
            load_options.codex_home.display()
        );
    }

    if args.plain || !std::io::stdout().is_terminal() {
        print_matches(
            &conversations,
            args.query.as_deref().unwrap_or_default(),
            args.local,
            args.limit,
            args.show_path,
            args.show_id,
            &load_options.current_dir,
        );
        return Ok(());
    }

    let action = tui::run(tui::TuiInput {
        conversations,
        initial_query: args.query.unwrap_or_default(),
        local_filter: args.local,
        current_dir: load_options.current_dir.clone(),
        show_tools: args.show_tools,
        show_reasoning: args.show_reasoning,
    })?;

    match action {
        tui::Action::Quit => Ok(()),
        tui::Action::Select(conversation) => {
            if args.show_path {
                println!("{}", conversation.path.display());
            } else if args.show_id {
                println!("{}", conversation.session_id);
            } else if args.resume || args.fork {
                launch_codex(&conversation, args.fork)?;
            } else {
                render::print_conversation(&conversation, args.show_tools, args.show_reasoning);
            }
            Ok(())
        }
        tui::Action::Resume(conversation) => launch_codex(&conversation, false),
        tui::Action::Fork(conversation) => launch_codex(&conversation, true),
    }
}

fn print_matches(
    conversations: &[Conversation],
    query: &str,
    local_filter: bool,
    limit: usize,
    show_path: bool,
    show_id: bool,
    current_dir: &std::path::Path,
) {
    let mut matches = search::filter_and_rank(conversations, query, local_filter, current_dir);
    matches.truncate(limit);
    for conversation in matches {
        if show_path {
            println!("{}", conversation.path.display());
        } else if show_id {
            println!("{}", conversation.session_id);
        } else {
            println!(
                "{}  {}  {}",
                conversation.started_at.format("%Y-%m-%d %H:%M"),
                conversation.session_id,
                conversation.title
            );
            if let Some(cwd) = &conversation.cwd {
                println!("    {}", cwd.display());
            }
            if !conversation.preview.is_empty() {
                println!("    {}", conversation.preview);
            }
        }
    }
}

fn launch_codex(conversation: &Conversation, fork: bool) -> Result<()> {
    let subcommand = if fork { "fork" } else { "resume" };
    let mut command = Command::new("codex");
    command.arg(subcommand).arg(&conversation.session_id);
    if let Some(cwd) = &conversation.cwd
        && cwd.is_dir()
    {
        command.arg("--cd").arg(cwd);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        bail!("failed to launch codex {subcommand}: {err}");
    }

    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .with_context(|| format!("failed to launch codex {subcommand}"))?;
        if !status.success() {
            bail!("codex {subcommand} exited with {status}");
        }
        Ok(())
    }
}
