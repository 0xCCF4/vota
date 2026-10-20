//! Admin Token store: hashes, labels, expiries and last uses; never the
//! secret.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::millis;
use crate::domain::admin_token::{self, AdminTokenHash, AdminTokenSecret};

/// What the operator chooses when creating an Admin Token.
#[derive(Debug, Clone, Default)]
pub struct NewAdminToken {
    pub label: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// A freshly created Admin Token. The secret exists only here, never in the
/// database, so this is the one chance to show it.
#[derive(Debug)]
pub struct IssuedAdminToken {
    pub id: i64,
    pub secret: AdminTokenSecret,
}

/// Creates and stores a new Admin Token (hash only) and returns its secret;
/// refused if its expiry breaks [`admin_token::check_new_expiry`].
/// This is the code path behind `vota admin-token create`.
pub async fn create(
    db: &SqlitePool,
    new: NewAdminToken,
    now: DateTime<Utc>,
) -> anyhow::Result<IssuedAdminToken> {
    admin_token::check_new_expiry(new.expires_at, now)?;
    let secret = AdminTokenSecret::generate();
    // `fetch_all`, not `fetch_one`: the statement must run to its end, where
    // SQLite commits, so a failed commit is an error here instead of being
    // dropped with the unfinished statement (and a token reported that was
    // never stored).
    let ids: Vec<i64> = sqlx::query_scalar(
        "INSERT INTO admin_tokens (token_hash, label, created_at, expires_at)
         VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(secret.hash().0)
    .bind(new.label)
    .bind(now.timestamp_millis())
    .bind(new.expires_at.map(|t| t.timestamp_millis()))
    .fetch_all(db)
    .await?;
    let [id] = ids[..] else {
        anyhow::bail!("storing the Admin Token returned {} rows", ids.len());
    };
    Ok(IssuedAdminToken { id, secret })
}

/// How often a token's `last_used_at` is written at most: every admin
/// request would otherwise be a database write.
const LAST_USED_RESOLUTION_MS: i64 = 60_000;

/// The ID of the stored Admin Token `presented` is, if it has not expired
/// at `now` (the rule of [`admin_token::is_expired`], applied in SQL); records the use as its `last_used_at` if the recorded one is a
/// minute old or more (or there is none). Recording only reads otherwise,
/// and a failure to record (e.g. a full disk) is logged, not an error: it
/// mustn't lock admins out.
pub(crate) async fn authenticate(
    db: &SqlitePool,
    presented: &str,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<i64>> {
    let now = now.timestamp_millis();
    let found: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, last_used_at FROM admin_tokens
         WHERE token_hash = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(AdminTokenHash::of(presented).0)
    .bind(now)
    .fetch_optional(db)
    .await?;
    let Some((id, last_used_at)) = found else {
        return Ok(None);
    };
    if last_used_at.is_none_or(|last| now - last >= LAST_USED_RESOLUTION_MS) {
        let recorded = sqlx::query(
            "UPDATE admin_tokens SET last_used_at = ?
             WHERE id = ? AND (last_used_at IS NULL OR last_used_at <= ?)",
        )
        .bind(now)
        .bind(id)
        .bind(now - LAST_USED_RESOLUTION_MS)
        .execute(db)
        .await;
        if let Err(error) = recorded {
            tracing::warn!(%error, token_id = id, "recording an Admin Token's last use failed");
        }
    }
    Ok(Some(id))
}

/// Deletes the Admin Token with this ID. Returns whether one existed.
pub async fn revoke(db: &SqlitePool, id: i64) -> sqlx::Result<bool> {
    let result = sqlx::query("DELETE FROM admin_tokens WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// What `admin-token list` shows about a stored Admin Token: no secret, no hash.
#[derive(Debug, Clone)]
pub struct AdminTokenInfo {
    pub id: i64,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// When the admin API last accepted it, to the minute; `None` if never
    /// (or not since the release that started recording it).
    pub last_used_at: Option<DateTime<Utc>>,
}

/// All stored Admin Tokens, oldest first.
pub async fn list(db: &SqlitePool) -> anyhow::Result<Vec<AdminTokenInfo>> {
    /// `id, label, created_at, expires_at, last_used_at`.
    type Row = (i64, Option<String>, i64, Option<i64>, Option<i64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, label, created_at, expires_at, last_used_at FROM admin_tokens ORDER BY id",
    )
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(id, label, created_at, expires_at, last_used_at)| {
            Ok(AdminTokenInfo {
                id,
                label,
                created_at: millis::to_datetime(created_at)?,
                expires_at: expires_at.map(millis::to_datetime).transpose()?,
                last_used_at: last_used_at.map(millis::to_datetime).transpose()?,
            })
        })
        .collect()
}

/// Deletes every Admin Token whose label is exactly `label` (case-sensitive,
/// no patterns). Returns the IDs it deleted, in ascending order.
pub async fn revoke_by_label(db: &SqlitePool, label: &str) -> sqlx::Result<Vec<i64>> {
    let mut ids: Vec<i64> =
        sqlx::query_scalar("DELETE FROM admin_tokens WHERE label = ? RETURNING id")
            .bind(label)
            .fetch_all(db)
            .await?;
    ids.sort_unstable();
    Ok(ids)
}
