//! Ballot store: Ballots, their Options and their Responses, split by
//! concern (private submodules): `create`, `lookup`, `edit` (edits and
//! extensions), `responses` (recording Responses) and `purge`. Shared here:
//! the one `select_ballots!` query every lookup goes through, and the Voter
//! Reference rules of creating and editing.

use chrono::{DateTime, Utc};
use sqlx::SqliteConnection;

use crate::data::millis;
use crate::domain::ballot::{
    Group, OptionId, Response, ResponseOrigin, SubmissionMode, VoterReference,
};

/// `SELECT <the columns of BallotRow> FROM ballots WHERE <not purged at ?1>
/// AND (<condition>) <rest>`, as a static SQL string: the one way to look up
/// Ballots, so that no lookup can forget that a Ballot is gone from its
/// Purge Date on (inclusive), whether or not the sweep has deleted it yet.
/// Bind "now" (Unix milliseconds) as `?1`; the condition's own parameters
/// start at `?2`.
macro_rules! select_ballots {
    ($condition:literal) => {
        select_ballots!($condition, "")
    };
    ($condition:literal, $rest:literal) => {
        concat!(
            "SELECT id, token, title, description, choice_mode, allow_no_option, submission_mode,
                    voting_deadline,
                    purge_date, created_at, group_name, voter_reference, revision,
                    theme, accent_color, logo
             FROM ballots WHERE purge_date > ?1 AND (",
            $condition,
            ") ",
            $rest
        )
    };
}

mod create;
mod edit;
mod lookup;
mod purge;
mod responses;

pub(crate) use create::{BulkRefused, insert, insert_bulk};
pub(crate) use edit::{Edited, Extended, edit, extend};
pub(crate) use lookup::{find_by_id, find_by_token, list};
pub(crate) use purge::{delete, sweep_purged};
pub(crate) use responses::{Recorded, record_response};

/// How often to retry on a (practically impossible) random ID collision.
const MAX_ATTEMPTS: usize = 3;

/// Another Ballot in the same Group already has this Voter Reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DuplicateVoterReference;

/// Whether `error` is the (Group, Voter Reference) uniqueness rule
/// (index `ballots_group_voter_reference`) rather than, say, a random ID
/// collision. SQLite names the index's columns in the message.
fn is_duplicate_voter_reference(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(error) => {
            error.is_unique_violation()
                && error.message().contains("ballots.group_name")
                && error.message().contains("ballots.voter_reference")
        }
        _ => false,
    }
}

/// Deletes the Ballot (if any) that is purged at `now` but not yet swept and
/// holds this Voter Reference in this Group, so that the Voter Reference can
/// be used again at once: a purged Ballot is as good as never existing.
async fn free_purged_voter_reference(
    conn: &mut SqliteConnection,
    group: Option<&Group>,
    voter_reference: Option<&VoterReference>,
    now: DateTime<Utc>,
) -> sqlx::Result<()> {
    let (Some(group), Some(voter_reference)) = (group, voter_reference) else {
        return Ok(());
    };
    sqlx::query(
        "DELETE FROM ballots
         WHERE group_name = ? AND voter_reference = ? AND purge_date <= ?",
    )
    .bind(group.as_str())
    .bind(voter_reference.as_str())
    .bind(now.timestamp_millis())
    .execute(conn)
    .await?;
    Ok(())
}

/// Stores `option_ids` as the Preselection of the Ballot with this ID (which
/// has none stored), and, if a Ballot of this Submission Mode records it
/// ([`SubmissionMode::records_preselection`]), as its Response of origin
/// [`ResponseOrigin::Preselection`], answered at `now` (it has no Response
/// stored either). Returns that Response, if recorded.
async fn write_preselection(
    conn: &mut SqliteConnection,
    ballot_id: i64,
    option_ids: &[OptionId],
    submission_mode: SubmissionMode,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<Response>> {
    for option_id in option_ids {
        sqlx::query("INSERT INTO preselected_options (ballot_id, option_id) VALUES (?, ?)")
            .bind(ballot_id)
            .bind(option_id.as_str())
            .execute(&mut *conn)
            .await?;
    }
    if !submission_mode.records_preselection() {
        return Ok(None);
    }
    let origin = ResponseOrigin::Preselection;
    sqlx::query(
        "INSERT INTO responses (ballot_id, answered_at, changed_at, origin) VALUES (?1, ?2, ?2, ?3)",
    )
    .bind(ballot_id)
    .bind(now.timestamp_millis())
    .bind(origin.as_str())
    .execute(&mut *conn)
    .await?;
    for option_id in option_ids {
        sqlx::query("INSERT INTO response_options (ballot_id, option_id) VALUES (?, ?)")
            .bind(ballot_id)
            .bind(option_id.as_str())
            .execute(&mut *conn)
            .await?;
    }
    let now = millis::truncate(now)?;
    Ok(Some(Response {
        option_ids: option_ids.to_vec(),
        answered_at: now,
        changed_at: now,
        origin,
    }))
}
