//! Creating Ballots: a fresh Ballot Token and fresh Option IDs, retried on
//! a random ID collision.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::{
    DuplicateVoterReference, MAX_ATTEMPTS, free_purged_voter_reference,
    is_duplicate_voter_reference,
};
use crate::data::millis;
use crate::domain::ballot::{
    Ballot, BallotDraft, BallotOption, BallotToken, Description, OptionId,
};
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
    free_purged_voter_reference(
        &mut tx,
        draft.group.as_ref(),
        draft.voter_reference.as_ref(),
        now,
    )
    .await?;
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
    .bind(
        draft
            .voter_reference
            .as_ref()
            .map(|reference| reference.as_str()),
    )
    .bind(draft.theme.as_str())
    .bind(draft.accent_color.as_ref().map(AccentColor::as_str))
    .bind(draft.logo.as_ref().map(LogoName::as_str))
    .bind(draft.allow_no_option)
    .fetch_one(&mut *tx)
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
        .execute(&mut *tx)
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
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

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
        response: None,
        fallback,
        group: draft.group.as_ref().map(|group| group.as_str().to_owned()),
        voter_reference: draft
            .voter_reference
            .as_ref()
            .map(|reference| reference.as_str().to_owned()),
        theme: draft.theme.as_str().to_owned(),
        accent_color: draft.accent_color.clone(),
        logo: draft.logo.as_ref().map(|logo| logo.as_str().to_owned()),
        revision: 0,
    })
}
