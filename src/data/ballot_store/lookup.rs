//! Looking Ballots up, by Ballot Token, by internal ID or by filter, and
//! loading them whole: Options, Response and Fallback. Every lookup goes
//! through `select_ballots!`, so none returns a purged Ballot.

use chrono::{DateTime, Utc};
use sqlx::{SqliteConnection, SqlitePool};

use crate::data::millis;
use crate::domain::ballot::{
    Ballot, BallotFilter, BallotId, BallotOption, BallotToken, ChoiceMode, Group, OptionId,
    Response, ResponseOrigin, SubmissionMode,
};
use crate::domain::theme::AccentColor;

#[derive(sqlx::FromRow)]
struct BallotRow {
    id: i64,
    token: String,
    title: String,
    description: Option<String>,
    choice_mode: String,
    allow_no_option: bool,
    submission_mode: String,
    voting_deadline: i64,
    purge_date: i64,
    created_at: i64,
    group_name: Option<String>,
    voter_reference: Option<String>,
    revision: i64,
    theme: String,
    accent_color: Option<String>,
    logo: Option<String>,
}

/// The Ballot with this token, if there is one that isn't purged at `now`.
pub(crate) async fn find_by_token(
    db: &SqlitePool,
    token: &BallotToken,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<Ballot>> {
    let mut conn = db.acquire().await?;
    let row: Option<BallotRow> = sqlx::query_as(select_ballots!("token = ?2"))
        .bind(now.timestamp_millis())
        .bind(token.as_str())
        .fetch_optional(&mut *conn)
        .await?;
    match row {
        Some(row) => Ok(Some(load(&mut conn, row).await?)),
        None => Ok(None),
    }
}

/// The Ballot with this internal ID, if there is one that isn't purged at
/// `now`.
pub(crate) async fn find_by_id(
    db: &SqlitePool,
    id: BallotId,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<Ballot>> {
    find_by_id_on(&mut *db.acquire().await?, id, now).await
}

pub(super) async fn find_by_id_on(
    conn: &mut SqliteConnection,
    id: BallotId,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<Ballot>> {
    let row: Option<BallotRow> = sqlx::query_as(select_ballots!("id = ?2"))
        .bind(now.timestamp_millis())
        .bind(id.get())
        .fetch_optional(&mut *conn)
        .await?;
    match row {
        Some(row) => Ok(Some(load(conn, row).await?)),
        None => Ok(None),
    }
}

/// The Ballots matching `filter` at `now` (none purged at `now`), in creation
/// order (internal ID ascending). All matches, no pagination: admin is trusted and Groups are
/// small. Ballot Status is derived, never stored, so the status filter is
/// applied to the loaded Ballots at `now`; the Group filter is SQL.
pub(crate) async fn list(
    db: &SqlitePool,
    filter: &BallotFilter,
    now: DateTime<Utc>,
) -> sqlx::Result<Vec<Ballot>> {
    let mut conn = db.acquire().await?;
    let rows: Vec<BallotRow> = sqlx::query_as(select_ballots!(
        "?2 IS NULL OR group_name = ?2",
        "ORDER BY id"
    ))
    .bind(now.timestamp_millis())
    .bind(filter.group.as_ref().map(Group::as_str))
    .fetch_all(&mut *conn)
    .await?;
    let mut ballots = Vec::with_capacity(rows.len());
    for row in rows {
        let ballot = load(&mut conn, row).await?;
        if filter
            .status
            .is_none_or(|status| ballot.status(now) == status)
        {
            ballots.push(ballot);
        }
    }
    Ok(ballots)
}

/// The whole Ballot of `row`: with its Options, Response and Fallback.
async fn load(conn: &mut SqliteConnection, row: BallotRow) -> sqlx::Result<Ballot> {
    let id = row.id;
    // The schema's CHECK constraints admit only known names.
    let choice_mode = ChoiceMode::from_name(&row.choice_mode)
        .ok_or_else(|| sqlx::Error::Decode("unknown choice_mode".into()))?;
    let submission_mode = SubmissionMode::from_name(&row.submission_mode)
        .ok_or_else(|| sqlx::Error::Decode("unknown submission_mode".into()))?;
    let options = sqlx::query_as::<_, (String, String)>(
        "SELECT id, label FROM ballot_options WHERE ballot_id = ? ORDER BY position",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(id, label)| BallotOption {
        id: OptionId::from_store(id),
        label,
    })
    .collect();
    let response = find_response(conn, id).await?;
    let fallback = option_set(
        conn,
        "SELECT f.option_id FROM fallback_options f
         JOIN ballot_options o ON o.ballot_id = f.ballot_id AND o.id = f.option_id
         WHERE f.ballot_id = ? ORDER BY o.position",
        id,
    )
    .await?;
    let preselection = option_set(
        conn,
        "SELECT p.option_id FROM preselected_options p
         JOIN ballot_options o ON o.ballot_id = p.ballot_id AND o.id = p.option_id
         WHERE p.ballot_id = ? ORDER BY o.position",
        id,
    )
    .await?;
    Ok(Ballot {
        id,
        token: BallotToken::from_store(row.token),
        title: row.title,
        description: row.description,
        options,
        choice_mode,
        allow_no_option: row.allow_no_option,
        submission_mode,
        voting_deadline: millis::to_datetime(row.voting_deadline)?,
        purge_date: millis::to_datetime(row.purge_date)?,
        created_at: millis::to_datetime(row.created_at)?,
        response,
        fallback: (!fallback.is_empty()).then_some(fallback),
        preselection: (!preselection.is_empty()).then_some(preselection),
        group: row.group_name,
        voter_reference: row.voter_reference,
        theme: row.theme,
        accent_color: row.accent_color.map(AccentColor::from_store),
        logo: row.logo,
        revision: row.revision,
    })
}

/// The Option IDs `query` selects for the Ballot with this ID (bound as its
/// one parameter), in the order it gives.
async fn option_set(
    conn: &mut SqliteConnection,
    query: &'static str,
    ballot_id: i64,
) -> sqlx::Result<Vec<OptionId>> {
    Ok(sqlx::query_scalar::<_, String>(query)
        .bind(ballot_id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(OptionId::from_store)
        .collect())
}

async fn find_response(
    conn: &mut SqliteConnection,
    ballot_id: i64,
) -> sqlx::Result<Option<Response>> {
    let row: Option<(i64, i64, String)> =
        sqlx::query_as("SELECT answered_at, changed_at, origin FROM responses WHERE ballot_id = ?")
            .bind(ballot_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((answered_at, changed_at, origin)) = row else {
        return Ok(None);
    };
    // The schema's CHECK constraint admits only known names.
    let origin = ResponseOrigin::from_name(&origin)
        .ok_or_else(|| sqlx::Error::Decode("unknown response origin".into()))?;
    let option_ids = sqlx::query_scalar::<_, String>(
        "SELECT r.option_id FROM response_options r
         JOIN ballot_options o ON o.ballot_id = r.ballot_id AND o.id = r.option_id
         WHERE r.ballot_id = ? ORDER BY o.position",
    )
    .bind(ballot_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(OptionId::from_store)
    .collect();
    Ok(Some(Response {
        option_ids,
        answered_at: millis::to_datetime(answered_at)?,
        changed_at: millis::to_datetime(changed_at)?,
        origin,
    }))
}
