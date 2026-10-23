use chrono::{DateTime, SecondsFormat, Utc};
use clap::Subcommand;

use crate::clock::{Clock as _, SystemClock};
use crate::config::Config;
use crate::data::admin_tokens::{self, AdminTokenInfo, NewAdminToken};
use crate::data::db;
use crate::domain::admin_token::{self, AdminTokenLabel, AdminTokenSecret};
use untrusted_value::{SanitizeValue as _, UntrustedValue};

use super::escape::{PrintableJson, PrintableText};

#[derive(Subcommand)]
pub(super) enum AdminTokenCommand {
    /// Create an Admin Token and print it once on stdout. Only its hash is stored.
    Create {
        /// Note on what the token is for: single-line text of at most 200
        /// characters, without control, bidi or other invisible characters
        /// (trimmed and NFC-normalised).
        #[arg(long)]
        label: Option<String>,
        /// Lifetime from now, e.g. "90days" or "12h".
        #[arg(
            long,
            value_name = "DURATION",
            value_parser = humantime::parse_duration,
            conflicts_with = "expires_at"
        )]
        expires_in: Option<std::time::Duration>,
        /// Expiry as RFC 3339 timestamp with offset, e.g. "2030-01-01T00:00:00Z".
        #[arg(long, value_name = "RFC3339", value_parser = parse_rfc3339)]
        expires_at: Option<DateTime<Utc>>,
    },
    /// Print a new Admin Token on stdout without storing it anywhere, for an
    /// Admin Token file (`admin_token_files`). Needs no configuration or
    /// database.
    Generate,
    /// List all Admin Tokens: ID, created, expiry, whether expired, last use
    /// (to the minute) and label. Never shows secrets or their hashes.
    List {
        /// Print a JSON array of `{id, label, created_at, expires_at,
        /// expired, last_used_at}` (RFC 3339 UTC; `expires_at` null without
        /// expiry, `last_used_at` null if never used) instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Revoke (delete) Admin Tokens by ID or by label. They stop working at once.
    #[command(group(clap::ArgGroup::new("target").required(true).args(["id", "label"])))]
    Revoke {
        /// ID as printed by `create` and `list`.
        #[arg(value_parser = clap::value_parser!(i64).range(1..), allow_negative_numbers = true)]
        id: Option<i64>,
        /// Revoke every Admin Token with exactly this label (case-sensitive).
        #[arg(long)]
        label: Option<String>,
    },
}

/// A label for a new Admin Token, held to the rules of single-line text.
/// The error names the reason, never the label.
fn sanitize_label(raw: String) -> anyhow::Result<String> {
    let label: AdminTokenLabel = UntrustedValue::from(raw)
        .sanitize_value()
        .map_err(|reason| anyhow::anyhow!("invalid --label: {reason}"))?;
    Ok(label.into_string())
}

fn parse_rfc3339(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(value).map(|t| t.with_timezone(&Utc))
}

/// The latest expiry an Admin Token may have: the last moment an RFC 3339
/// timestamp (four-digit year) can show in UTC.
const LATEST_EXPIRY: &str = "9999-12-31T23:59:59Z";

/// The expiry from `--expires-in` (relative to `now`) or `--expires-at`.
/// Refuses one that doesn't lie in the future ([`admin_token::check_new_expiry`])
/// or is later than [`LATEST_EXPIRY`] with a clear error, also when the
/// lifetime is too long to compute at all.
fn expiry(
    now: DateTime<Utc>,
    expires_in: Option<std::time::Duration>,
    expires_at: Option<DateTime<Utc>>,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    let latest: DateTime<Utc> = LATEST_EXPIRY.parse().expect("a valid timestamp");
    let too_late = || anyhow::anyhow!("the expiry must be no later than {LATEST_EXPIRY}");
    let expires_at = match expires_in {
        Some(lifetime) => Some(
            chrono::Duration::from_std(lifetime)
                .ok()
                .and_then(|lifetime| now.checked_add_signed(lifetime))
                .ok_or_else(too_late)?,
        ),
        None => expires_at,
    };
    admin_token::check_new_expiry(expires_at, now)?;
    match expires_at {
        Some(expires_at) if expires_at > latest => Err(too_late()),
        expires_at => Ok(expires_at),
    }
}

/// Prints a fresh Admin Token for an Admin Token file: nothing is stored,
/// and no configuration or database is read.
pub(super) fn generate() {
    println!("{}", AdminTokenSecret::generate().expose());
}

impl AdminTokenCommand {
    /// Runs the command on the configured database alone: the theme and logo
    /// folders are never read, so a broken Theme or logo can't block token
    /// management.
    pub(super) async fn run(self, config: Config) -> anyhow::Result<()> {
        let clock = SystemClock;
        // Checked before the database is opened: a refused expiry leaves no trace.
        let now = clock.now();
        let (expires_at, label) = match &self {
            Self::Create {
                expires_in,
                expires_at,
                label,
            } => (
                expiry(now, *expires_in, *expires_at)?,
                label.clone().map(sanitize_label).transpose()?,
            ),
            _ => (None, None),
        };
        let db = db::open(&config.database_path).await?;
        match self {
            Self::Create { .. } => {
                let issued =
                    admin_tokens::create(&db, NewAdminToken { label, expires_at }, now).await?;
                eprintln!(
                    "Created Admin Token {}. It is shown only this once; store it securely.",
                    issued.id
                );
                println!("{}", issued.secret.expose());
                Ok(())
            }
            Self::List { json } => {
                let tokens = admin_tokens::list(&db).await?;
                if json {
                    println!(
                        "{}",
                        PrintableJson::compact(&render_json(&tokens, clock.now()))
                    );
                } else {
                    print!("{}", render_table(&tokens, clock.now()));
                }
                Ok(())
            }
            Self::Revoke { id: Some(id), .. } => {
                anyhow::ensure!(
                    admin_tokens::revoke(&db, id).await?,
                    "no Admin Token with ID {id}"
                );
                eprintln!("Revoked Admin Token {id}.");
                Ok(())
            }
            Self::Revoke {
                label: Some(label), ..
            } => {
                let ids = admin_tokens::revoke_by_label(&db, &label).await?;
                anyhow::ensure!(!ids.is_empty(), "no Admin Token with label {label:?}");
                let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
                eprintln!("Revoked Admin Tokens {}.", ids.join(", "));
                Ok(())
            }
            Self::Revoke {
                id: None,
                label: None,
            } => unreachable!("clap requires an ID or a label"),
            Self::Generate => unreachable!("handled by Cli::run, without a database"),
        }
    }
}

/// A JSON array with one `{id, label, created_at, expires_at, expired,
/// last_used_at}` object per token, for scripts. Built from [`AdminTokenInfo`], which holds neither
/// the secret nor its hash.
fn render_json(tokens: &[AdminTokenInfo], now: DateTime<Utc>) -> serde_json::Value {
    let timestamp = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
    tokens
        .iter()
        .map(|token| {
            serde_json::json!({
                "id": token.id,
                "label": token.label,
                "created_at": timestamp(token.created_at),
                "expires_at": token.expires_at.map(timestamp),
                "expired": admin_token::is_expired(token.expires_at, now),
                "last_used_at": token.last_used_at.map(timestamp),
            })
        })
        .collect()
}

/// Aligned columns ID, CREATED, EXPIRES, EXPIRED, LAST-USED, LABEL. The
/// label comes last because it is free text and may contain spaces. It is
/// printed as a [`PrintableText`]: a label stored before labels were
/// checked may hold characters `create` now refuses.
fn render_table(tokens: &[AdminTokenInfo], now: DateTime<Utc>) -> String {
    let timestamp = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
    let rows: Vec<[String; 6]> = tokens
        .iter()
        .map(|token| {
            [
                token.id.to_string(),
                timestamp(token.created_at),
                token.expires_at.map_or_else(|| "never".into(), timestamp),
                if admin_token::is_expired(token.expires_at, now) {
                    "yes"
                } else {
                    "no"
                }
                .into(),
                token.last_used_at.map_or_else(|| "never".into(), timestamp),
                (token.label.as_deref())
                    .map(|label| PrintableText::new(label).to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    let header = ["ID", "CREATED", "EXPIRES", "EXPIRED", "LAST-USED", "LABEL"].map(String::from);
    let mut widths = header.each_ref().map(|cell| cell.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    std::iter::once(&header)
        .chain(&rows)
        .map(|row| {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate().take(5) {
                line += &format!("{cell:<width$}  ", width = widths[i]);
            }
            line += &row[5];
            line.trim_end().to_owned() + "\n"
        })
        .collect()
}
