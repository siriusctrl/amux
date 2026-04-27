use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use crate::{model::Target, session_view, tmux, tui};

#[derive(Debug, Parser)]
#[command(
    name = "amux",
    version,
    about = "Friendly client for persistent local and remote terminal sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check local dependencies and backend availability.
    Doctor,

    /// Manage targets.
    Target {
        #[command(subcommand)]
        command: TargetCommand,
    },

    /// Manage sessions.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },

    /// Create a detached session.
    New(NewCommand),

    /// Open an existing session.
    Attach(AttachCommand),
}

#[derive(Debug, Subcommand)]
enum TargetCommand {
    /// List configured targets.
    List,
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// List sessions for the active target.
    List,
}

#[derive(Debug, Parser)]
struct NewCommand {
    /// Session name.
    name: String,

    /// Start directory for the session.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Command to run inside the session. Omit to start the user's shell.
    #[arg(last = true)]
    command: Vec<String>,
}

#[derive(Debug, Parser)]
struct AttachCommand {
    /// Session name.
    name: String,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => run_dashboard(),
        Some(Command::Doctor) => doctor(),
        Some(Command::Target { command }) => match command {
            TargetCommand::List => list_targets(),
        },
        Some(Command::Session { command }) => match command {
            SessionCommand::List => list_sessions(),
        },
        Some(Command::New(command)) => new_session(command),
        Some(Command::Attach(command)) => attach_session(command),
    }
}

fn doctor() -> Result<()> {
    println!("amux: ok");
    println!("target: local");
    println!("tmux: {}", tmux::version()?);
    Ok(())
}

fn list_targets() -> Result<()> {
    let target = Target::local();
    println!("ID\tKIND\tLABEL");
    println!("{}\t{}\t{}", target.id, target.kind, target.label);
    Ok(())
}

fn list_sessions() -> Result<()> {
    let sessions = tmux::list_sessions()?;
    println!("ID\tNAME\tWINDOWS\tSTATUS");
    for session in sessions {
        println!(
            "{}\t{}\t{}\t{}",
            session.id,
            session.name,
            session.windows,
            session.display_status()
        );
    }
    Ok(())
}

fn new_session(command: NewCommand) -> Result<()> {
    if let Some(cwd) = &command.cwd
        && !cwd.is_dir()
    {
        bail!("--cwd is not a directory: {}", cwd.display());
    }

    tmux::create_session(&command.name, command.cwd.as_deref(), &command.command)?;
    println!("created session {}", command.name);
    Ok(())
}

fn attach_session(command: AttachCommand) -> Result<()> {
    session_view::run(&command.name)
}

fn run_dashboard() -> Result<()> {
    if let Some(session) = tui::run()? {
        session_view::run(&session)?;
    }
    Ok(())
}
