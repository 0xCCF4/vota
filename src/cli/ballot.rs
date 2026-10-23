//! `vota ballot …`: a thin client for the admin API of a running Vota
//! server. It builds requests from flags, sends them ([`connection`]) and prints
//! the answers ([`render`]). The server is the only place input is checked:
//! the client parses only what it needs to build the JSON (enum values, flag
//! combinations, and relative dates, which [`dates`] turns into timestamps).
//!
//! The server's answers are untrusted input: [`answer`] is the only place
//! they are unboxed, into typed views whose text can only be printed
//! escaped (`cli/escape.rs`).

mod answer;
mod body;
mod connection;
mod csv;
mod dates;
#[cfg(test)]
mod docs_drift;
mod exit;
#[cfg(feature = "qr")]
mod qr;
mod render;
mod tally;

use std::io::{IsTerminal as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Subcommand, ValueEnum};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::Method;

use super::escape::{PrintableJson, PrintableLines, PrintableText};
use answer::{Body, Created, Reply};
use body::{Clears, Fields};
use connection::{Connection, ConnectionArgs};
use dates::{Dates, Verb};
pub(super) use exit::Exit;

#[derive(clap::Args)]
pub(super) struct BallotArgs {
    #[command(flatten)]
    connection: ConnectionArgs,
    /// Print the server's JSON answer (pretty-printed) on stdout instead of
    /// the readable output (`tally`: its own JSON summary); on an error,
    /// the server's error body too.
    #[arg(long, global = true, help_heading = "Output")]
    json: bool,
    #[command(subcommand)]
    command: BallotCommand,
}

#[derive(Subcommand)]
enum BallotCommand {
    /// Create a Ballot (POST /admin/ballots) and print its URL; or many
    /// at once, one per Voter Reference (repeated --voter-reference,
    /// --voters, --count), and print a table of their URLs.
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
        /// Create one Ballot per Voter Reference in this file (`-`: stdin):
        /// one per line, surrounding whitespace trimmed, empty lines and
        /// lines starting with `#` skipped. All or none are created.
        #[arg(
            long,
            value_name = "FILE|-",
            conflicts_with_all = ["voter_reference", "count", "body"],
            help_heading = "Many Ballots"
        )]
        voters: Option<PathBuf>,
        /// Create this many Ballots (at most 1000) in the --group, numbered:
        /// their Voter References are 1, 2, … after the Group's highest
        /// purely numeric Voter Reference.
        #[arg(
            long,
            value_name = "N",
            conflicts_with_all = ["voter_reference", "body"],
            help_heading = "Many Ballots"
        )]
        count: Option<i64>,
        /// Print the created Ballots as CSV on stdout: `voter_reference,id,url`
        /// (RFC 4180, CRLF line ends; a value starting with =, +, -, @, tab
        /// or CR gets a `'` in front against formula injection).
        #[arg(long, conflicts_with = "json", help_heading = "Output")]
        csv: bool,
        /// Also draw the Ballot's URL as a QR code on stderr, the URL below
        /// it. Not with many Ballots.
        #[cfg(feature = "qr")]
        #[arg(long, help_heading = "Output")]
        qr: bool,
    },
    /// Show one Ballot (GET /admin/ballots/{id}).
    Get {
        /// The Ballot's ID.
        id: u64,
        /// Also draw the Ballot's URL as a QR code on stderr, the URL below
        /// it.
        #[cfg(feature = "qr")]
        #[arg(long, help_heading = "Output")]
        qr: bool,
    },
    /// List Ballots with their results (GET /admin/ballots).
    List {
        /// Only Ballots in this Group.
        #[arg(long)]
        group: Option<String>,
        /// Only Ballots with this Ballot Status.
        #[arg(long, value_enum)]
        status: Option<Status>,
        /// Print the list as CSV (RFC 4180, CRLF), one record per Ballot,
        /// without URLs or Ballot Tokens.
        #[arg(long, conflicts_with = "json")]
        csv: bool,
    },
    /// Sum up the results of a Group's Ballots by Option label (from GET
    /// /admin/ballots).
    Tally {
        /// Only Ballots in this Group; without it, all live Ballots.
        #[arg(long)]
        group: Option<String>,
        /// Print the table as CSV (RFC 4180, CRLF):
        /// option,answered,preselected,fallback,total.
        #[arg(long, conflicts_with = "json")]
        csv: bool,
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
        // The units from `dates`, so not a doc comment.
        #[arg(
            long,
            value_name = "DATETIME|DURATION|+DURATION",
            required = true,
            help = format!(
                "The new Voting Deadline: RFC 3339, or without an offset local time in the \
                 server's default time zone; a duration from now (`3d`); or `+` and a \
                 duration after the current one (`+1d`, `+1m`). {}",
                dates::units_help()
            )
        )]
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
    exit: Exit,
    /// Escaped as a whole: it may quote flag values or, through a
    /// transport error, the server.
    message: PrintableText,
    body: Option<ErrorBody>,
}

/// The body of an error answer, for `--json`.
enum ErrorBody {
    Json(PrintableJson),
    /// Not JSON: its text.
    Text(PrintableText),
}

impl std::fmt::Display for ErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(json) => json.fmt(f),
            Self::Text(text) => text.fmt(f),
        }
    }
}

impl Failure {
    /// The message is escaped (escaping already escaped text again changes
    /// nothing).
    fn new(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: PrintableText::new(&message.into()),
            body: None,
        }
    }

    /// A usage error: exit 2, like clap's own.
    fn usage(message: impl Into<String>) -> Self {
        Self::new(Exit::Usage, message)
    }
}

impl BallotArgs {
    /// Runs the command; errors are printed here, so only the exit code is
    /// returned.
    pub(super) async fn run(self) -> ExitCode {
        let json = self.json;
        match self.execute().await {
            Ok(()) => Exit::Success.into(),
            Err(failure) => {
                if json && let Some(body) = &failure.body {
                    println!("{body}");
                }
                eprintln!("Error: {}", failure.message);
                failure.exit.into()
            }
        }
    }

    async fn execute(self) -> Result<(), Failure> {
        let json = self.json;
        let connection = self.connection.connect()?;
        let show = |body: Body, human: fn(&Body) -> String| {
            if json {
                println!("{}", body.json());
            } else {
                print!("{}", human(&body));
            }
        };
        match self.command {
            BallotCommand::Create {
                mut fields,
                body,
                voters,
                count,
                csv,
                #[cfg(feature = "qr")]
                qr,
            } => {
                let bulk = match (voters, count) {
                    (Some(path), _) => Some(body::Bulk::Voters(body::voters(&path)?)),
                    (None, Some(count)) => Some(body::Bulk::Count(count)),
                    (None, None) => None,
                };
                let body = match body {
                    Some(path) => body::raw(&path)?,
                    None => {
                        let (deadline, purge_date) = fields.dates_mut();
                        resolve_dates(&connection, Verb::Create, deadline, purge_date).await?;
                        fields.into_body(None, bulk)?
                    }
                };
                // Whether from the flags or from --body.
                #[cfg(feature = "qr")]
                refuse_bulk_qr(qr, body::is_bulk_body(&body))?;
                let answer = ask(&connection, Method::POST, "admin/ballots", Some(body))
                    .await?
                    .unwrap_or_default();
                if json {
                    println!("{}", answer.json());
                }
                let created = answer.created();
                if !json {
                    if csv {
                        print!("{}", render::created_csv(&created));
                    } else {
                        match &created {
                            Created::Many(ballots) => print!("{}", render::created_table(ballots)),
                            Created::One(ballot) => println!("{}", ballot.url.show()),
                        }
                    }
                    match &created {
                        Created::Many(ballots) => eprint!("{}", render::created_many(ballots)),
                        Created::One(ballot) => eprint!("{}", render::created(ballot)),
                    }
                }
                #[cfg(feature = "qr")]
                if qr && let Created::One(ballot) = &created {
                    eprint!("{}", draw_qr(ballot)?);
                }
            }
            BallotCommand::Get {
                id,
                #[cfg(feature = "qr")]
                qr,
            } => {
                let reply = ask(
                    &connection,
                    Method::GET,
                    &format!("admin/ballots/{id}"),
                    None,
                )
                .await?;
                // Drawn first, so a Ballot without a URL prints nothing.
                #[cfg(feature = "qr")]
                let code = match (&reply, qr) {
                    (Some(body), true) => Some(draw_qr(&body.ballot())?),
                    _ => None,
                };
                show(reply.unwrap_or_default(), |body| {
                    render::ballot(&body.ballot())
                });
                #[cfg(feature = "qr")]
                if let Some(code) = code {
                    eprint!("{code}");
                }
            }
            BallotCommand::List { group, status, csv } => {
                let path = list_path(group.as_deref(), status);
                let body = ask(&connection, Method::GET, &path, None)
                    .await?
                    .unwrap_or_default();
                if csv {
                    show(body, |body| render::list_csv(&body.list()));
                } else {
                    show(body, |body| render::table(&body.list()));
                }
            }
            BallotCommand::Tally { group, csv } => {
                let path = list_path(group.as_deref(), None);
                let list = ask(&connection, Method::GET, &path, None)
                    .await?
                    .unwrap_or_default()
                    .list();
                let tally = tally::Tally::of(&list, group)?;
                if json {
                    println!("{}", PrintableJson::pretty(&tally.json()));
                } else if csv {
                    print!("{}", tally.csv());
                } else {
                    print!("{}", tally.text());
                }
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
                        fields.into_body(Some(clears), None)?
                    }
                };
                let reply = ask(
                    &connection,
                    Method::PATCH,
                    &format!("admin/ballots/{id}"),
                    Some(body),
                )
                .await?;
                show(reply.unwrap_or_default(), |body| {
                    render::ballot(&body.ballot())
                });
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
                let reply = ask(
                    &connection,
                    Method::POST,
                    &format!("admin/ballots/{id}/extend"),
                    Some(body.to_string().into_bytes()),
                )
                .await?;
                show(reply.unwrap_or_default(), |body| {
                    render::ballot(&body.ballot())
                });
            }
            BallotCommand::Delete { id, yes } => {
                if !yes {
                    confirm_delete(&connection, id).await?;
                }
                ask(
                    &connection,
                    Method::DELETE,
                    &format!("admin/ballots/{id}"),
                    None,
                )
                .await?;
                if !json {
                    eprintln!("Deleted Ballot {id}.");
                }
            }
        }
        Ok(())
    }
}

/// Sends one request and reads the answer ([`answer::read`]).
async fn ask(
    connection: &Connection,
    method: Method,
    path: &str,
    body: Option<Vec<u8>>,
) -> Result<Reply, Failure> {
    answer::read(connection.send(method, path, body).await?)
}

/// `--qr` with many Ballots: a usage error, before anything is sent.
#[cfg(feature = "qr")]
fn refuse_bulk_qr(qr: bool, is_bulk: bool) -> Result<(), Failure> {
    if qr && is_bulk {
        return Err(Failure::usage(
            "--qr draws a single Ballot's URL; it can't be used when creating many Ballots",
        ));
    }
    Ok(())
}

/// The QR code of the Ballot's URL, with the URL below it. A Ballot
/// without an `http(s)` URL is an unexpected answer.
#[cfg(feature = "qr")]
fn draw_qr(ballot: &answer::ServerBallot) -> Result<String, Failure> {
    let url = ballot.url.known().ok_or_else(|| {
        Failure::new(
            Exit::Other,
            "unexpected response: a Ballot without an http(s) url",
        )
    })?;
    qr::code(url)
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
    let ballot = ask(
        connection,
        Method::GET,
        &format!("admin/ballots/{id}"),
        None,
    )
    .await?
    .unwrap_or_default()
    .ballot();
    (ballot.voting_deadline.known())
        .map(answer::Timestamp::instant)
        .ok_or_else(|| {
            Failure::new(
                Exit::Other,
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
    let ballot = ask(
        connection,
        Method::GET,
        &format!("admin/ballots/{id}"),
        None,
    )
    .await?
    .unwrap_or_default()
    .ballot();
    eprint!("{}", render::summary(&ballot));
    eprint!("Delete Ballot {id} \"{}\"? [y/N] ", ballot.title.show());
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| Failure::new(Exit::Other, format!("can't read the answer: {error}")))?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err(Failure::new(
            Exit::Other,
            format!("Ballot {id} was not deleted"),
        )),
    }
}

/// The path of `GET /admin/ballots` with these filters.
fn list_path(group: Option<&str>, status: Option<Status>) -> String {
    let mut query = Vec::new();
    if let Some(group) = group {
        query.push(format!("group={}", encode(group)));
    }
    if let Some(status) = status {
        query.push(format!("status={}", enum_name(status)));
    }
    let mut path = "admin/ballots".to_owned();
    if !query.is_empty() {
        path = format!("{path}?{}", query.join("&"));
    }
    path
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
            path.to_string_lossy()
        ))
    })
}

/// Like [`read_bytes`], as UTF-8 text.
fn read_input(path: &Path, flag: &str) -> Result<String, Failure> {
    String::from_utf8(read_bytes(path, flag)?)
        .map_err(|_| Failure::usage(format!("{flag}: the file is not UTF-8 text")))
}
