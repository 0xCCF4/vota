//! The command line: `serve` (default) and `admin-token …` (cargo feature
//! `server`), the admin API client `ballot …` (cargo feature `client`) and
//! `completions` (every build). A build without `server` has no default
//! command: `vota` alone shows the help.

#[cfg(feature = "server")]
mod admin_token;
#[cfg(feature = "client")]
mod ballot;
#[cfg(any(feature = "server", feature = "client"))]
mod escape;
#[cfg(feature = "server")]
mod serve;

#[cfg(feature = "server")]
use std::io::IsTerminal as _;
#[cfg(feature = "server")]
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory as _, Parser, Subcommand, ValueEnum};
#[cfg(feature = "server")]
use tracing_subscriber::EnvFilter;

#[cfg(feature = "server")]
use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "vota",
    version,
    about = "Personal Ballots with an admin API and a voting UI",
    // Without the server there is no default command.
    arg_required_else_help = !cfg!(feature = "server"),
    subcommand_required = !cfg!(feature = "server")
)]
pub struct Cli {
    /// TOML config file; `VOTA_*` environment variables override its
    /// values. Not used by `ballot` and `completions`.
    #[cfg(feature = "server")]
    #[arg(long, global = true, env = "VOTA_CONFIG", value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server (the default when no subcommand is given).
    #[cfg(feature = "server")]
    Serve,
    /// Manage Admin Tokens in the configured database (the server need not run).
    #[cfg(feature = "server")]
    #[command(subcommand)]
    AdminToken(admin_token::AdminTokenCommand),
    /// Manage Ballots on a running Vota server through its admin API (any
    /// machine; no config file or database needed).
    #[cfg(feature = "client")]
    Ballot(Box<ballot::BallotArgs>),
    /// Print shell completions for the whole CLI on stdout.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// A shell `completions` writes for.
#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Cli {
    /// Runs the parsed command; returns the exit code of a command that
    /// sets its own (`ballot`).
    pub async fn run(self) -> anyhow::Result<ExitCode> {
        // The client and the completions need no configuration and no
        // database, and create no files. They log nothing, so nothing (such
        // as the Admin Token) can leak into a log.
        match self.command {
            #[cfg(feature = "client")]
            Some(Command::Ballot(ballot)) => Ok(ballot.run().await),
            Some(Command::Completions { shell }) => {
                print_completions(shell)?;
                Ok(ExitCode::SUCCESS)
            }
            #[cfg(feature = "server")]
            command => {
                run_server_command(self.config.as_deref(), command.unwrap_or(Command::Serve))
                    .await?;
                Ok(ExitCode::SUCCESS)
            }
            #[cfg(not(feature = "server"))]
            None => unreachable!("without the server, clap requires a subcommand"),
        }
    }
}

/// Runs `serve` or `admin-token …`: with logging, the umask and the
/// configuration.
#[cfg(feature = "server")]
async fn run_server_command(
    config: Option<&std::path::Path>,
    command: Command,
) -> anyhow::Result<()> {
    // Logs go to stderr so stdout carries only command output (e.g. a new
    // Admin Token). Coloured only on a terminal: in a journal or a file,
    // escape codes would garble the events' fields (e.g. the audit log's).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Every file the app creates (the database, its `-wal` and `-shm`)
    // is for its own user only: the database holds Ballot Tokens.
    restrict_new_files();
    let config = Config::load(config)?;
    match command {
        Command::Serve => serve::run(config).await,
        Command::AdminToken(command) => command.run(config).await,
        _ => unreachable!("handled by Cli::run"),
    }
}

/// Writes the completion script for `shell` to stdout. A reader that stops
/// early (`| head`) is no error.
fn print_completions(shell: Shell) -> anyhow::Result<()> {
    use std::io::Write as _;
    let shell = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
        Shell::Fish => clap_complete::Shell::Fish,
    };
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "vota", &mut script);
    match std::io::stdout().lock().write_all(&script) {
        Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => Err(error.into()),
        _ => Ok(()),
    }
}

/// Sets the process umask to 077, so new files are readable and writable by
/// the app's user only, whatever umask it was started with.
#[cfg(all(unix, feature = "server"))]
fn restrict_new_files() {
    use rustix::fs::Mode;
    rustix::process::umask(Mode::from_raw_mode(0o077));
}

#[cfg(all(not(unix), feature = "server"))]
fn restrict_new_files() {}
