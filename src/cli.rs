//! The command line: `serve` and `admin-token …` (cargo feature `server`),
//! the admin API client `ballot …` (cargo feature `client`), and
//! `completions` and `man` (every build). There is no default command:
//! `vota` alone shows the help.

#[cfg(feature = "server")]
mod admin_token;
#[cfg(feature = "client")]
mod ballot;
mod completions;
#[cfg(any(feature = "server", feature = "client"))]
mod escape;
mod man;
#[cfg(feature = "server")]
mod serve;

#[cfg(feature = "server")]
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueHint};
#[cfg(feature = "server")]
use tracing_subscriber::EnvFilter;

#[cfg(feature = "server")]
use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "vota",
    version,
    about = "Personal Ballots with an admin API and a voting UI",
    // No default command: `vota` alone shows the help.
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct Cli {
    /// TOML config file; `VOTA_*` environment variables override its
    /// values. Not used by `ballot`, `completions` and `man`.
    #[cfg(feature = "server")]
    #[arg(
        long,
        global = true,
        env = "VOTA_CONFIG",
        value_name = "FILE",
        value_hint = ValueHint::FilePath
    )]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server.
    #[cfg(feature = "server")]
    Serve,
    /// Manage Admin Tokens in the configured database (the server need not
    /// run), or generate one for an Admin Token file.
    #[cfg(feature = "server")]
    #[command(subcommand)]
    AdminToken(admin_token::AdminTokenCommand),
    /// Manage Ballots on a running Vota server through its admin API (any
    /// machine; no config file or database needed).
    #[cfg(feature = "client")]
    Ballot(Box<ballot::BallotArgs>),
    /// Print shell completions for the whole CLI on stdout.
    Completions(completions::CompletionsArgs),
    /// Write a man page for every command (`vota.1`, `vota-serve.1`,
    /// `vota-ballot-create.1` and so on) into a directory.
    Man {
        /// The directory for the pages; created if missing. Existing pages
        /// in it are overwritten.
        #[arg(value_name = "DIR", value_hint = ValueHint::DirPath)]
        out_dir: PathBuf,
    },
}

impl Cli {
    /// Runs the parsed command; returns the exit code of a command that
    /// sets its own (`ballot`).
    pub async fn run(self) -> anyhow::Result<ExitCode> {
        // The client, the completions, the man pages and `admin-token
        // generate` need no configuration and no database, and create no
        // files (but the pages). They log nothing, so nothing (such as the
        // Admin Token) can leak into a log.
        match self.command {
            #[cfg(feature = "client")]
            Command::Ballot(ballot) => Ok(ballot.run().await),
            Command::Completions(completions) => {
                completions.run()?;
                Ok(ExitCode::SUCCESS)
            }
            Command::Man { out_dir } => {
                man::write_pages(&out_dir)?;
                Ok(ExitCode::SUCCESS)
            }
            // Needs neither the configuration nor the database.
            #[cfg(feature = "server")]
            Command::AdminToken(admin_token::AdminTokenCommand::Generate) => {
                admin_token::generate();
                Ok(ExitCode::SUCCESS)
            }
            #[cfg(feature = "server")]
            command => {
                run_server_command(self.config.as_deref(), command).await?;
                Ok(ExitCode::SUCCESS)
            }
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

/// Sets the process umask to 077, so new files are readable and writable by
/// the app's user only, whatever umask it was started with.
#[cfg(all(unix, feature = "server"))]
fn restrict_new_files() {
    use rustix::fs::Mode;
    rustix::process::umask(Mode::from_raw_mode(0o077));
}

#[cfg(all(not(unix), feature = "server"))]
fn restrict_new_files() {}
