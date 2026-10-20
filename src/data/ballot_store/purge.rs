//! Deleting Ballots for good: at once by the admin, or by the purge sweep
//! once their Purge Date has come. Their data is overwritten and the
//! write-ahead log checkpointed.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::data::db;
use crate::domain::ballot::BallotId;

/// Deletes the Ballot with this ID at once, with its Options, Response and
/// Fallback (foreign-key cascades). Whether there was such a Ballot that
/// wasn't purged at `now`: one that is purged but not yet swept is deleted
/// all the same, but counts as not found. Its data is overwritten and the
/// write-ahead log checkpointed, so it can't be recovered from the files.
pub(crate) async fn delete(
    db: &SqlitePool,
    id: BallotId,
    now: DateTime<Utc>,
) -> sqlx::Result<bool> {
    // `fetch_all`, not `fetch_optional`: the statement must run to its end,
    // where SQLite commits, so a failed commit is an error here instead of
    // being dropped with the unfinished statement.
    let live: Vec<bool> =
        sqlx::query_scalar("DELETE FROM ballots WHERE id = ?1 RETURNING purge_date > ?2")
            .bind(id.get())
            .bind(now.timestamp_millis())
            .fetch_all(db)
            .await?;
    if !live.is_empty() {
        db::checkpoint(db).await;
    }
    Ok(live == [true])
}

/// The purge sweep: deletes every Ballot purged at `now` (Purge Date at or
/// before it) with its Options, Response and Fallback (foreign-key
/// cascades), freeing its Voter Reference. How many Ballots it deleted.
/// Lookups already treat such Ballots as gone; this removes them for good,
/// and (with `secure_delete` and a checkpoint) from the database files too.
pub(crate) async fn sweep_purged(db: &SqlitePool, now: DateTime<Utc>) -> sqlx::Result<u64> {
    let deleted = sqlx::query("DELETE FROM ballots WHERE purge_date <= ?")
        .bind(now.timestamp_millis())
        .execute(db)
        .await?
        .rows_affected();
    if deleted > 0 {
        db::checkpoint(db).await;
    }
    Ok(deleted)
}
