//! Creating Ballots, one or many at once (bulk creation): each with a fresh
//! Ballot Token and fresh Option IDs, retried on a random ID collision.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::{Connection as _, SqliteConnection, SqlitePool};

use super::write_preselection;
use super::{
    DuplicateVoterReference, MAX_ATTEMPTS, free_purged_voter_reference,
    is_duplicate_voter_reference,
};
use crate::data::millis;
use crate::domain::ballot::{
    Ballot, BallotDraft, BallotOption, BallotToken, Description, OptionId, VoterReference,
};
use crate::domain::bulk::{self, Bulk, NumberingExhausted};
use crate::domain::theme::{AccentColor, LogoName};

/// Stores a new Ballot with a fresh Ballot Token and fresh Option IDs,
/// retrying with new random values on a unique-constraint conflict. Refused
/// (nothing stored) if its Voter Reference is taken within its Group.
pub(crate) async fn insert(
    db: &SqlitePool,
    draft: &BallotDraft,
    now: DateTime<Utc>,
) -> anyhow::Result<Result<Ballot, DuplicateVoterReference>> {
    let mut attempt = 1;
    loop {
        match try_insert(db, draft, BallotToken::generate(), now).await {
            Err(error) if is_duplicate_voter_reference(&error) => {
                return Ok(Err(DuplicateVoterReference));
            }
            Err(sqlx::Error::Database(error))
                if error.is_unique_violation() && attempt < MAX_ATTEMPTS =>
            {
                tracing::warn!("random ID collision while creating a Ballot; retrying");
                attempt += 1;
            }
            result => return Ok(Ok(result?)),
        }
    }
}

async fn try_insert(
    db: &SqlitePool,
    draft: &BallotDraft,
    token: BallotToken,
    now: DateTime<Utc>,
) -> sqlx::Result<Ballot> {
    let mut tx = db.begin().await?;
    let voter_reference = draft.voter_reference.as_ref();
    free_purged_voter_reference(&mut tx, draft.group.as_ref(), voter_reference, now).await?;
    let ballot = write(&mut tx, draft, voter_reference, token, now).await?;
    tx.commit().await?;
    Ok(ballot)
}

/// Why a bulk creation stored nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BulkRefused {
    /// These Voter References (in the order asked for) are taken within the
    /// Group.
    Taken(Vec<String>),
    /// The Group's numeric Voter References leave no room to number on.
    NumberingExhausted,
}

/// Stores one Ballot per Voter Reference of `bulk`, all alike but for their
/// Voter References (the draft's own is ignored), each with its own fresh
/// Ballot Token and Option IDs; in one transaction, so all of them or none.
/// Refused if any Voter Reference is taken within the Group (all of those
/// are named), or if numbering would run out of numbers. Numbered Voter
/// References continue after the Group's highest numeric one at the time
/// of the transaction (purged Ballots don't count; see [`bulk::numbered`]).
pub(crate) async fn insert_bulk(
    db: &SqlitePool,
    draft: &BallotDraft,
    bulk: &Bulk,
    now: DateTime<Utc>,
) -> anyhow::Result<Result<Vec<Ballot>, BulkRefused>> {
    // IMMEDIATE: the Group's Voter References read here stay current until
    // the commit, as no other writer can start in between.
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let group = draft.group.as_ref();
    let existing: HashSet<String> = match group {
        Some(group) => sqlx::query_scalar(
            "SELECT voter_reference FROM ballots
             WHERE group_name = ? AND voter_reference IS NOT NULL AND purge_date > ?",
        )
        .bind(group.as_str())
        .bind(now.timestamp_millis())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect(),
        None => HashSet::new(),
    };
    let references = match bulk {
        Bulk::Listed(list) => list.as_slice().to_vec(),
        Bulk::Numbered(count) => {
            match bulk::numbered(existing.iter().map(String::as_str), *count) {
                Ok(references) => references,
                Err(NumberingExhausted) => return Ok(Err(BulkRefused::NumberingExhausted)),
            }
        }
    };
    let taken: Vec<String> = (references.iter())
        .map(|reference| reference.as_str())
        .filter(|reference| existing.contains(*reference))
        .map(str::to_owned)
        .collect();
    if !taken.is_empty() {
        return Ok(Err(BulkRefused::Taken(taken)));
    }
    let mut ballots = Vec::with_capacity(references.len());
    for reference in &references {
        free_purged_voter_reference(&mut tx, group, Some(reference), now).await?;
        match write_with_retries(&mut tx, draft, reference, now).await {
            Err(error) if is_duplicate_voter_reference(&error) => {
                return Ok(Err(BulkRefused::Taken(vec![reference.as_str().to_owned()])));
            }
            result => ballots.push(result?),
        }
    }
    tx.commit().await?;
    Ok(Ok(ballots))
}

/// [`write`]s one Ballot of a bulk creation in a savepoint, retried with
/// new random values on a (practically impossible) random ID collision.
async fn write_with_retries(
    conn: &mut SqliteConnection,
    draft: &BallotDraft,
    voter_reference: &VoterReference,
    now: DateTime<Utc>,
) -> sqlx::Result<Ballot> {
    let mut attempt = 1;
    loop {
        let mut savepoint = conn.begin().await?;
        let written = write(
            &mut savepoint,
            draft,
            Some(voter_reference),
            BallotToken::generate(),
            now,
        )
        .await;
        match written {
            Ok(ballot) => {
                savepoint.commit().await?;
                return Ok(ballot);
            }
            Err(error) if is_duplicate_voter_reference(&error) => return Err(error),
            Err(sqlx::Error::Database(error))
                if error.is_unique_violation() && attempt < MAX_ATTEMPTS =>
            {
                savepoint.rollback().await?;
                tracing::warn!("random ID collision while creating a Ballot; retrying");
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Writes the Ballot of `draft` with this Voter Reference (instead of the
/// draft's own) and Ballot Token, with fresh Option IDs.
async fn write(
    conn: &mut SqliteConnection,
    draft: &BallotDraft,
    voter_reference: Option<&VoterReference>,
    token: BallotToken,
    now: DateTime<Utc>,
) -> sqlx::Result<Ballot> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO ballots (token, title, description, choice_mode, submission_mode,
                              voting_deadline, purge_date, created_at,
                              group_name, voter_reference, theme, accent_color, logo,
                              allow_no_option)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(token.as_str())
    .bind(draft.title.as_str())
    .bind(draft.description.as_ref().map(Description::as_str))
    .bind(draft.choice_mode.as_str())
    .bind(draft.submission_mode.as_str())
    .bind(draft.voting_deadline.timestamp_millis())
    .bind(draft.purge_date.timestamp_millis())
    .bind(now.timestamp_millis())
    .bind(draft.group.as_ref().map(|group| group.as_str()))
    .bind(voter_reference.map(VoterReference::as_str))
    .bind(draft.theme.as_str())
    .bind(draft.accent_color.as_ref().map(AccentColor::as_str))
    .bind(draft.logo.as_ref().map(LogoName::as_str))
    .bind(draft.allow_no_option)
    .fetch_one(&mut *conn)
    .await?;

    let mut options: Vec<BallotOption> = Vec::new();
    for (position, label) in draft.options.iter().enumerate() {
        let option = BallotOption {
            id: OptionId::generate(),
            label: label.as_str().to_owned(),
        };
        sqlx::query(
            "INSERT INTO ballot_options (id, ballot_id, position, label) VALUES (?, ?, ?, ?)",
        )
        .bind(option.id.as_str())
        .bind(id)
        .bind(position as i64)
        .bind(&option.label)
        .execute(&mut *conn)
        .await?;
        options.push(option);
    }
    let fallback = draft.fallback.as_ref().map(|fallback| {
        fallback
            .positions()
            .iter()
            .map(|&position| options[position].id.clone())
            .collect::<Vec<_>>()
    });
    for option_id in fallback.iter().flatten() {
        sqlx::query("INSERT INTO fallback_options (ballot_id, option_id) VALUES (?, ?)")
            .bind(id)
            .bind(option_id.as_str())
            .execute(&mut *conn)
            .await?;
    }
    let preselection = draft.preselection.as_ref().map(|preselection| {
        preselection
            .positions()
            .iter()
            .map(|&position| options[position].id.clone())
            .collect::<Vec<_>>()
    });
    let response = match &preselection {
        Some(option_ids) => {
            write_preselection(conn, id, option_ids, draft.submission_mode, now).await?
        }
        None => None,
    };
    Ok(Ballot {
        id,
        token,
        title: draft.title.as_str().to_owned(),
        description: draft
            .description
            .as_ref()
            .map(|description| description.as_str().to_owned()),
        options,
        choice_mode: draft.choice_mode,
        allow_no_option: draft.allow_no_option,
        submission_mode: draft.submission_mode,
        voting_deadline: millis::truncate(draft.voting_deadline)?,
        purge_date: millis::truncate(draft.purge_date)?,
        created_at: millis::truncate(now)?,
        response,
        fallback,
        preselection,
        group: draft.group.as_ref().map(|group| group.as_str().to_owned()),
        voter_reference: voter_reference.map(|reference| reference.as_str().to_owned()),
        theme: draft.theme.as_str().to_owned(),
        accent_color: draft.accent_color.clone(),
        logo: draft.logo.as_ref().map(|logo| logo.as_str().to_owned()),
        revision: 0,
    })
}
