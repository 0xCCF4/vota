//! Recording the voter's Response, in one conditional write.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::domain::ballot::{Ballot, SeenRevision, Selection, SubmissionMode};

/// Whether [`record_response`] stored the Response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recorded {
    Yes,
    /// The Ballot is Final and already had a Response; nothing changed.
    AlreadyAnswered,
    /// The Ballot's Voting Deadline (as stored) is not after `now`; nothing
    /// changed.
    VotingClosed,
    /// The Ballot's revision is no longer the one the voter saw (an admin
    /// edit landed since), so the selection was made on an outdated Ballot;
    /// nothing changed.
    BallotChanged,
}

/// Records the voter's Response on `ballot` at `now`, if voting is still open
/// by the Ballot's stored Voting Deadline (exclusive) and the Ballot isn't
/// purged, as its Submission Mode allows:
/// - Final: unless it already has one. The Response row is a single
///   conditional insert (at most one per Ballot), so concurrent or repeated
///   submissions can never store two Responses.
/// - Editable: replaces any earlier Response. `answered_at` stays the time of
///   the first Response, `changed_at` becomes `now`.
///
/// The deadline and the revision the voter saw are part of the same
/// conditional write, so a submission racing a change of the deadline can't
/// slip in after it, and one made on (or racing) an admin edit can't land on
/// the edited Ballot. The selected Options are written in the same
/// transaction either way, so a Response is never seen half-written.
pub(crate) async fn record_response(
    db: &SqlitePool,
    ballot: &Ballot,
    seen: SeenRevision,
    selection: &Selection,
    now: DateTime<Utc>,
) -> sqlx::Result<Recorded> {
    let mut tx = db.begin().await?;
    let now = now.timestamp_millis();
    // (The `WHERE` also keeps SQLite from parsing `ON CONFLICT` as a join.)
    let insert = match ballot.submission_mode {
        SubmissionMode::Final => {
            "INSERT INTO responses (ballot_id, answered_at, changed_at)
             SELECT id, ?2, ?2 FROM ballots
             WHERE id = ?1 AND voting_deadline > ?2 AND purge_date > ?2 AND revision = ?3
             ON CONFLICT (ballot_id) DO NOTHING"
        }
        // `max` keeps changed_at from running backwards if the system clock does.
        SubmissionMode::Editable => {
            "INSERT INTO responses (ballot_id, answered_at, changed_at)
             SELECT id, ?2, ?2 FROM ballots
             WHERE id = ?1 AND voting_deadline > ?2 AND purge_date > ?2 AND revision = ?3
             ON CONFLICT (ballot_id)
             DO UPDATE SET changed_at = max(changed_at, excluded.changed_at)"
        }
    };
    let written = sqlx::query(insert)
        .bind(ballot.id)
        .bind(now)
        .bind(seen.get())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if written == 0 {
        let stored: Option<(bool, i64)> =
            sqlx::query_as("SELECT voting_deadline > ?2, revision FROM ballots WHERE id = ?1")
                .bind(ballot.id)
                .bind(now)
                .fetch_optional(&mut *tx)
                .await?;
        return Ok(match stored {
            Some((false, _)) => Recorded::VotingClosed,
            // Deleted since it was loaded: changed for good.
            None => Recorded::BallotChanged,
            Some((true, revision)) if revision != seen.get() => Recorded::BallotChanged,
            Some((true, _)) => Recorded::AlreadyAnswered,
        });
    }
    sqlx::query("DELETE FROM response_options WHERE ballot_id = ?")
        .bind(ballot.id)
        .execute(&mut *tx)
        .await?;
    for option_id in selection.option_ids() {
        sqlx::query("INSERT INTO response_options (ballot_id, option_id) VALUES (?, ?)")
            .bind(ballot.id)
            .bind(option_id.as_str())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Recorded::Yes)
}
