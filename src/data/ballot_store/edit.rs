//! Admin edits of a Ballot and extensions of its Voting Deadline, each
//! read, validated and written in one write-locked transaction.

use chrono::{DateTime, Utc};
use sqlx::{SqliteConnection, SqlitePool};

use super::lookup::find_by_id_on;
use super::{MAX_ATTEMPTS, free_purged_voter_reference, is_duplicate_voter_reference};
use crate::domain::ballot::{
    Ballot, BallotChanges, BallotId, Description, EditRefusal, EditScope, Extension, Group,
    OptionId, VoterReference,
};
use crate::domain::theme::{AccentColor, LogoName};

/// What became of an admin edit ([`edit`]).
#[derive(Debug)]
pub(crate) enum Edited<E> {
    /// Stored; the Ballot as it now stands.
    Yes(Box<Ballot>),
    /// No Ballot with this ID.
    NotFound,
    /// The Ballot can't be edited any more; nothing changed.
    Refused(EditRefusal),
    /// The edit itself was invalid for this Ballot; nothing changed.
    Invalid(E),
    /// Another Ballot in the resulting Group already has the resulting Voter
    /// Reference; nothing changed.
    DuplicateVoterReference,
}

/// Edits the Ballot with this ID (unless it is purged at `now`), if it
/// accepts an edit of this `scope` (refused before `changes` runs):
/// `changes` validates the edit against the Ballot as it stands and yields
/// the whole resulting Ballot, which replaces it. A [`EditScope::Bookkeeping`]
/// edit writes only the Group and Voter Reference and keeps the revision, as
/// the voter sees nothing of it. Reading, validating and writing happen in one
/// immediate (write-locked) transaction, and a content write is conditional
/// on the Ballot still having no Response and no applied Fallback (stored
/// Voting Deadline not after `now` with a Fallback), so a content edit and a
/// voter's Response can never both win, nor can a content edit land once the
/// Fallback has applied.
pub(crate) async fn edit<E>(
    db: &SqlitePool,
    id: BallotId,
    now: DateTime<Utc>,
    scope: EditScope,
    changes: impl FnOnce(&Ballot) -> Result<BallotChanges, E>,
) -> sqlx::Result<Edited<E>> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let Some(ballot) = find_by_id_on(&mut tx, id, now).await? else {
        return Ok(Edited::NotFound);
    };
    if let Err(refusal) = ballot.accepts_edit(scope, now) {
        return Ok(Edited::Refused(refusal));
    }
    let changes = match changes(&ballot) {
        Ok(changes) => changes,
        Err(error) => return Ok(Edited::Invalid(error)),
    };
    free_purged_voter_reference(
        &mut tx,
        changes.group.as_ref(),
        changes.voter_reference.as_ref(),
        now,
    )
    .await?;
    if scope == EditScope::Bookkeeping {
        let written =
            sqlx::query("UPDATE ballots SET group_name = ?2, voter_reference = ?3 WHERE id = ?1")
                .bind(id.get())
                .bind(changes.group.as_ref().map(Group::as_str))
                .bind(changes.voter_reference.as_ref().map(VoterReference::as_str))
                .execute(&mut *tx)
                .await;
        match written {
            Err(error) if is_duplicate_voter_reference(&error) => {
                return Ok(Edited::DuplicateVoterReference);
            }
            written => written?,
        };
        let Some(edited) = find_by_id_on(&mut tx, id, now).await? else {
            return Ok(Edited::NotFound);
        };
        tx.commit().await?;
        return Ok(Edited::Yes(Box::new(edited)));
    }
    let written = sqlx::query(
        "UPDATE ballots SET title = ?2, choice_mode = ?3, submission_mode = ?4,
                            group_name = ?5, voter_reference = ?6,
                            voting_deadline = ?7, purge_date = ?8,
                            description = ?9, theme = ?10, accent_color = ?11,
                            logo = ?12, revision = revision + 1
         WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM responses WHERE ballot_id = ?1)
           AND NOT (voting_deadline <= ?13
                    AND EXISTS (SELECT 1 FROM fallback_options WHERE ballot_id = ?1))",
    )
    .bind(id.get())
    .bind(changes.title.as_str())
    .bind(changes.choice_mode.as_str())
    .bind(changes.submission_mode.as_str())
    .bind(changes.group.as_ref().map(Group::as_str))
    .bind(changes.voter_reference.as_ref().map(VoterReference::as_str))
    .bind(changes.voting_deadline.timestamp_millis())
    .bind(changes.purge_date.timestamp_millis())
    .bind(changes.description.as_ref().map(Description::as_str))
    .bind(changes.theme.as_str())
    .bind(changes.accent_color.as_ref().map(AccentColor::as_str))
    .bind(changes.logo.as_ref().map(LogoName::as_str))
    .bind(now.timestamp_millis())
    .execute(&mut *tx)
    .await;
    let written = match written {
        Err(error) if is_duplicate_voter_reference(&error) => {
            return Ok(Edited::DuplicateVoterReference);
        }
        written => written?.rows_affected(),
    };
    if written == 0 {
        // Answered or Auto-fallback since it was read: say which.
        let Some(stored) = find_by_id_on(&mut tx, id, now).await? else {
            return Ok(Edited::NotFound);
        };
        return match stored.accepts_edit(scope, now) {
            Err(refusal) => Ok(Edited::Refused(refusal)),
            Ok(()) => Err(sqlx::Error::Protocol(
                "a content edit matched no row though the Ballot accepts it".into(),
            )),
        };
    }
    let option_ids: Vec<OptionId> = match &changes.options {
        Some(labels) => {
            // Cascades to the Fallback's membership rows.
            sqlx::query("DELETE FROM ballot_options WHERE ballot_id = ?")
                .bind(id.get())
                .execute(&mut *tx)
                .await?;
            let mut option_ids = Vec::new();
            for (position, label) in labels.iter().enumerate() {
                option_ids.push(insert_option(&mut tx, id.get(), position, label.as_str()).await?);
            }
            option_ids
        }
        None => ballot.options.into_iter().map(|option| option.id).collect(),
    };
    sqlx::query("DELETE FROM fallback_options WHERE ballot_id = ?")
        .bind(id.get())
        .execute(&mut *tx)
        .await?;
    for &position in changes.fallback.iter().flat_map(|f| f.positions()) {
        sqlx::query("INSERT INTO fallback_options (ballot_id, option_id) VALUES (?, ?)")
            .bind(id.get())
            .bind(option_ids[position].as_str())
            .execute(&mut *tx)
            .await?;
    }
    let Some(edited) = find_by_id_on(&mut tx, id, now).await? else {
        return Ok(Edited::NotFound);
    };
    tx.commit().await?;
    Ok(Edited::Yes(Box::new(edited)))
}

/// What became of an extension of the Voting Deadline ([`extend`]).
#[derive(Debug)]
pub(crate) enum Extended<E> {
    /// Stored; the Ballot as it now stands.
    Yes(Box<Ballot>),
    /// No Ballot with this ID.
    NotFound,
    /// The extension was invalid for this Ballot; nothing changed.
    Invalid(E),
}

/// Extends the Voting Deadline of the Ballot with this ID (unless it is purged
/// at `now`), whether or not it has a Response: `extension` validates it against the Ballot as it stands
/// and yields the new Voting Deadline and Purge Date. Reading, validating and
/// writing happen in one immediate (write-locked) transaction, so concurrent
/// extensions can't base their Purge Date on a stale deadline.
pub(crate) async fn extend<E>(
    db: &SqlitePool,
    id: BallotId,
    now: DateTime<Utc>,
    extension: impl FnOnce(&Ballot) -> Result<Extension, E>,
) -> sqlx::Result<Extended<E>> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let Some(ballot) = find_by_id_on(&mut tx, id, now).await? else {
        return Ok(Extended::NotFound);
    };
    let extension = match extension(&ballot) {
        Ok(extension) => extension,
        Err(error) => return Ok(Extended::Invalid(error)),
    };
    sqlx::query("UPDATE ballots SET voting_deadline = ?2, purge_date = ?3 WHERE id = ?1")
        .bind(id.get())
        .bind(extension.voting_deadline.timestamp_millis())
        .bind(extension.purge_date.timestamp_millis())
        .execute(&mut *tx)
        .await?;
    let Some(extended) = find_by_id_on(&mut tx, id, now).await? else {
        return Ok(Extended::NotFound);
    };
    tx.commit().await?;
    Ok(Extended::Yes(Box::new(extended)))
}

/// Adds an Option with a fresh Option ID, retrying with a new one on a
/// (practically impossible) collision. SQLite undoes only the failed
/// statement, so the surrounding transaction carries on.
async fn insert_option(
    conn: &mut SqliteConnection,
    ballot_id: i64,
    position: usize,
    label: &str,
) -> sqlx::Result<OptionId> {
    let mut attempt = 1;
    loop {
        let option_id = OptionId::generate();
        let inserted = sqlx::query(
            "INSERT INTO ballot_options (id, ballot_id, position, label) VALUES (?, ?, ?, ?)",
        )
        .bind(option_id.as_str())
        .bind(ballot_id)
        .bind(position as i64)
        .bind(label)
        .execute(&mut *conn)
        .await;
        match inserted {
            Err(sqlx::Error::Database(error))
                if error.is_unique_violation() && attempt < MAX_ATTEMPTS =>
            {
                tracing::warn!("random ID collision while editing a Ballot; retrying");
                attempt += 1;
            }
            result => return result.map(|_| option_id),
        }
    }
}
