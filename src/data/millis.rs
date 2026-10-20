//! The data layer's one conversion between stored timestamps (UTC Unix
//! milliseconds in `INTEGER` columns) and [`DateTime<Utc>`]. Writing is
//! `DateTime::timestamp_millis`, which can't fail; reading back a value
//! outside chrono's range is a decode error, never a silent substitute.

use chrono::{DateTime, Utc};

/// The moment a stored timestamp names; a decode error if it is out of range
/// (which the app never writes: every stored value came from a `DateTime`).
pub(crate) fn to_datetime(millis: i64) -> sqlx::Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(millis).ok_or_else(|| {
        sqlx::Error::Decode(format!("stored timestamp {millis} is out of range").into())
    })
}

/// `t` as it reads back once stored: truncated to whole milliseconds.
pub(crate) fn truncate(t: DateTime<Utc>) -> sqlx::Result<DateTime<Utc>> {
    to_datetime(t.timestamp_millis())
}
