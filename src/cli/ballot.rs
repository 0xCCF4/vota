//! `vota ballot …`: a thin client for the admin API of a running Vota
//! server. It builds requests from flags, sends them ([`connection`]) and prints
//! the answers ([`render`]). The server is the only place input is checked:
//! the client parses only what it needs to build the JSON (enum values, flag
//! combinations, and relative dates, which [`dates`] turns into timestamps).

mod body;
mod connection;
mod dates;
mod render;

use std::io::{IsTerminal as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Subcommand, ValueEnum};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::Method;

use super::escape::{escape_json, printable};
use body::{Clears, Fields};
use connection::{Connection, ConnectionArgs};
use dates::{Dates, Verb};

#[derive(clap::Args)]
pub(super) struct BallotArgs {
    #[command(flatten)]
    connection: ConnectionArgs,
    /// Print the server's JSON answer (pretty-printed) on stdout instead of
    /// the readable output; on an error, its body too.
    #[arg(long, global = true, help_heading = "Output")]
    json: bool,
    #[command(subcommand)]
    command: BallotCommand,
}

#[derive(Subcommand)]
enum BallotCommand {
    /// Create a Ballot (POST /admin/ballots) and print its URL.
    Create {
        #[command(flatten)]
        fields: Fields,
        /// Send this JSON file (`-`: stdin) unchanged as the body, instead
        /// of the field flags.
        #[arg(
            long,
            value_name = "FILE|-",
            conflicts_with = "fields",
            help_heading = "Ballot fields"
        )]
        body: Option<PathBuf>,
    },
    /// Show one Ballot (GET /admin/ballots/{id}).
    Get {
        /// The Ballot's ID.
        id: u64,
    },
    /// List Ballots with their results (GET /admin/ballots).
    List {
        /// Only Ballots in this Group.
        #[arg(long)]
        group: Option<String>,
        /// Only Ballots with this Ballot Status.
        #[arg(long, value_enum)]
        status: Option<Status>,
    },
    /// Edit a Ballot (PATCH /admin/ballots/{id}). Fields without a flag
    /// keep their values.
    Edit {
        /// The Ballot's ID.
        id: u64,
        #[command(flatten)]
        fields: Fields,
        #[command(flatten)]
        clears: Clears,
        /// Send this JSON file (`-`: stdin) unchanged as the body, instead
        /// of the field flags.
        #[arg(
            long,
            value_name = "FILE|-",
            conflicts_with_all = ["fields", "clears"],
            help_heading = "Ballot fields"
        )]
        body: Option<PathBuf>,
    },
    /// Move the Voting Deadline later (POST /admin/ballots/{id}/extend).
    /// Reopens a closed Ballot without a Response.
    Extend {
        /// The Ballot's ID.
        id: u64,
        /// The new Voting Deadline: RFC 3339, or without an offset local
        /// time in the server's default time zone; a duration from now
        /// (`3d`); or `+` and a duration after the current one (`+1d`,
        /// `+1m`). Units, combined without spaces (`2d12h`): min
        /// (minutes), h (hours), d (days), w (weeks), m (calendar months),
        /// y (years). Note: `m` is months here, unlike in the server's
        /// configuration.
        #[arg(long, value_name = "DATETIME|DURATION|+DURATION", required = true)]
        voting_deadline: String,
    },
    /// Delete a Ballot with its Response (DELETE /admin/ballots/{id}). In a
    /// terminal, shows the Ballot and asks first.
    Delete {
        /// The Ballot's ID.
        id: u64,
        /// Don't ask. Required when stdin is not a terminal.
        #[arg(long, short)]
        yes: bool,
    },
}

/// A Ballot Status, as the list filter takes it.
#[derive(Clone, Copy, ValueEnum)]
enum Status {
    Unanswered,
    Answered,
    #[value(name = "auto_fallback")]
    AutoFallback,
}

/// Why a command failed: the message for stderr, the exit code, and for an
/// error answer the body to print on stdout with `--json`.
pub(super) struct Failure {
    exit: u8,
    message: String,
    body: Option<String>,
}

impl Failure {
    fn new(exit: u8, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
            body: None,
        }
    }

    /// A usage error: exit 2, like clap's own.
    fn usage(message: impl Into<String>) -> Self {
        Self::new(2, message)
    }
}

impl BallotArgs {
    /// Runs the command; errors are printed here, so only the exit code is
    /// returned.
    pub(super) async fn run(self) -> ExitCode {
        let json = self.json;
        match self.execute().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(failure) => {
                if json && let Some(body) = &failure.body {
                    println!("{body}");
                }
                eprintln!("Error: {}", failure.message);
                ExitCode::from(failure.exit)
            }
        }
    }

    async fn execute(self) -> Result<(), Failure> {
        let json = self.json;
        let connection = self.connection.connect()?;
        let show = |reply: connection::Reply, human: fn(&serde_json::Value) -> String| {
            let value = reply.unwrap_or_default();
            if json {
                println!("{}", json_text(&value));
            } else {
                print!("{}", human(&value));
            }
        };
        match self.command {
            BallotCommand::Create { mut fields, body } => {
                let body = match body {
                    Some(path) => body::raw(&path)?,
                    None => {
                        let (deadline, purge_date) = fields.dates_mut();
                        resolve_dates(&connection, Verb::Create, deadline, purge_date).await?;
                        fields.into_body(None)?
                    }
                };
                let ballot = connection
                    .send(Method::POST, "admin/ballots", Some(body))
                    .await?
                    .unwrap_or_default();
                if json {
                    println!("{}", json_text(&ballot));
                } else {
                    println!("{}", render::text(&ballot, "url"));
                    eprint!("{}", render::created(&ballot));
                }
            }
            BallotCommand::Get { id } => {
                let reply = connection
                    .send(Method::GET, &format!("admin/ballots/{id}"), None)
                    .await?;
                show(reply, render::ballot);
            }
            BallotCommand::List { group, status } => {
                let mut query = Vec::new();
                if let Some(group) = group {
                    query.push(format!("group={}", encode(&group)));
                }
                if let Some(status) = status {
                    query.push(format!("status={}", enum_name(status)));
                }
                let mut path = "admin/ballots".to_owned();
                if !query.is_empty() {
                    path = format!("{path}?{}", query.join("&"));
                }
                let reply = connection.send(Method::GET, &path, None).await?;
                show(reply, render::table);
            }
            BallotCommand::Edit {
                id,
                mut fields,
                clears,
                body,
            } => {
                let body = match body {
                    Some(path) => body::raw(&path)?,
                    None => {
                        let (deadline, purge_date) = fields.dates_mut();
                        resolve_dates(&connection, Verb::Edit(id), deadline, purge_date).await?;
                        fields.into_body(Some(clears))?
                    }
                };
                let reply = connection
                    .send(Method::PATCH, &format!("admin/ballots/{id}"), Some(body))
                    .await?;
                show(reply, render::ballot);
            }
            BallotCommand::Extend {
                id,
                voting_deadline,
            } => {
                let mut voting_deadline = Some(voting_deadline);
                resolve_dates(
                    &connection,
                    Verb::Extend(id),
                    &mut voting_deadline,
                    &mut None,
                )
                .await?;
                let body = serde_json::json!({ "voting_deadline": voting_deadline });
                let reply = connection
                    .send(
                        Method::POST,
                        &format!("admin/ballots/{id}/extend"),
                        Some(body.to_string().into_bytes()),
                    )
                    .await?;
                show(reply, render::ballot);
            }
            BallotCommand::Delete { id, yes } => {
                if !yes {
                    confirm_delete(&connection, id).await?;
                }
                connection
                    .send(Method::DELETE, &format!("admin/ballots/{id}"), None)
                    .await?;
                if !json {
                    eprintln!("Deleted Ballot {id}.");
                }
            }
        }
        Ok(())
    }
}

/// Replaces relative values of the date flags (`3d`, `+7d`) with the UTC
/// timestamps they name, and says so on stderr. A `+` relative to the
/// Ballot's current Voting Deadline fetches the Ballot first.
async fn resolve_dates(
    connection: &Connection,
    verb: Verb,
    voting_deadline: &mut Option<String>,
    purge_date: &mut Option<String>,
) -> Result<(), Failure> {
    let dates = Dates::parse(voting_deadline.as_deref(), purge_date.as_deref(), verb)?;
    let current = match verb {
        Verb::Edit(id) | Verb::Extend(id) if dates.needs_current_deadline() => {
            Some(current_deadline(connection, id).await?)
        }
        _ => None,
    };
    let resolved = dates.resolve(chrono::Utc::now(), current)?;
    for note in &resolved.notes {
        eprintln!("{note}");
    }
    *voting_deadline = resolved.voting_deadline;
    *purge_date = resolved.purge_date;
    Ok(())
}

/// The Ballot's current Voting Deadline (`GET /admin/ballots/{id}`).
async fn current_deadline(
    connection: &Connection,
    id: u64,
) -> Result<chrono::DateTime<chrono::Utc>, Failure> {
    let ballot = connection
        .send(Method::GET, &format!("admin/ballots/{id}"), None)
        .await?
        .unwrap_or_default();
    ballot["voting_deadline"]
        .as_str()
        .and_then(|deadline| deadline.parse().ok())
        .ok_or_else(|| {
            Failure::new(
                1,
                format!("unexpected response: Ballot {id} without a voting_deadline timestamp"),
            )
        })
}

/// In a terminal: shows the Ballot and asks. Without one: refused (exit 2),
/// since nobody could have been asked.
async fn confirm_delete(connection: &Connection, id: u64) -> Result<(), Failure> {
    if !std::io::stdin().is_terminal() {
        return Err(Failure::usage(format!(
            "not deleting Ballot {id} without confirmation: stdin is not a terminal; \
             pass --yes to delete without asking"
        )));
    }
    let ballot = connection
        .send(Method::GET, &format!("admin/ballots/{id}"), None)
        .await?
        .unwrap_or_default();
    eprint!("{}", render::summary(&ballot));
    eprint!(
        "Delete Ballot {id} \"{}\"? [y/N] ",
        render::text(&ballot, "title")
    );
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| Failure::new(1, format!("can't read the answer: {error}")))?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err(Failure::new(1, format!("Ballot {id} was not deleted"))),
    }
}

/// `value` percent-encoded for a query string.
fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// The name clap and the API use for an enum value.
fn enum_name(value: impl ValueEnum) -> String {
    value
        .to_possible_value()
        .expect("no skipped values")
        .get_name()
        .to_owned()
}

/// `value` pretty-printed, with every character that could steer the
/// terminal escaped.
fn json_text(value: &serde_json::Value) -> String {
    escape_json(&serde_json::to_string_pretty(value).expect("JSON values serialize"))
}

/// The bytes of `path`, or of stdin for `-`; an unreadable file is a usage
/// error naming `flag`.
fn read_bytes(path: &Path, flag: &str) -> Result<Vec<u8>, Failure> {
    let result = if path == Path::new("-") {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).map(|_| bytes)
    } else {
        std::fs::read(path)
    };
    result.map_err(|error| {
        Failure::usage(format!(
            "can't read {flag} {}: {error}",
            printable(&path.to_string_lossy())
        ))
    })
}

/// Like [`read_bytes`], as UTF-8 text.
fn read_input(path: &Path, flag: &str) -> Result<String, Failure> {
    String::from_utf8(read_bytes(path, flag)?)
        .map_err(|_| Failure::usage(format!("{flag}: the file is not UTF-8 text")))
}
