//! SQLite connection pool and embedded migrations.

use std::path::Path;
use std::str::FromStr as _;

use anyhow::Context as _;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

pub(crate) async fn open(path: &Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite:")?
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        // Deleted content (purged and deleted Ballots, replaced Responses)
        // is overwritten with zeros, not left in free pages.
        .pragma("secure_delete", "ON");
    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .with_context(|| format!("opening database {}", path.display()))?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("applying database migrations")?;
    restrict_to_owner(path);
    Ok(pool)
}

/// Makes the database and its `-wal` and `-shm` files (those that exist)
/// readable and writable by their owner only: they hold every live Ballot
/// Token in plaintext. A file that can't be changed is logged as a warning,
/// naming it; it doesn't stop the app.
#[cfg(unix)]
fn restrict_to_owner(database: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    for suffix in ["", "-wal", "-shm"] {
        let mut file = database.as_os_str().to_owned();
        file.push(suffix);
        let file = Path::new(&file);
        let Ok(metadata) = std::fs::metadata(file) else {
            continue;
        };
        if metadata.permissions().mode() & 0o077 == 0 {
            continue;
        }
        if let Err(error) = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(
                %error,
                "could not make {} readable by its owner only (mode 0600); \
                 other users may be able to read it",
                file.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn restrict_to_owner(_: &Path) {}

/// Copies the write-ahead log into the database and truncates it to zero
/// bytes, so page images of deleted rows don't linger in it. Called after
/// deletes. A checkpoint that can't complete (e.g. a long read in progress)
/// or fails is logged; the delete itself stands, and the next checkpoint
/// catches up.
pub(crate) async fn checkpoint(db: &SqlitePool) {
    let result: sqlx::Result<(i64, i64, i64)> = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(db)
        .await;
    match result {
        Ok((0, _, _)) => {}
        Ok(_) => tracing::warn!("the write-ahead log checkpoint after a delete could not complete"),
        Err(error) => {
            tracing::warn!(%error, "the write-ahead log checkpoint after a delete failed")
        }
    }
}

/// Whether the database answers.
pub(crate) async fn ping(db: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("SELECT 1").execute(db).await.map(|_| ())
}
