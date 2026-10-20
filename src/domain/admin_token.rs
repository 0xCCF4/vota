//! Admin Token secrets (ADR 0001): 256 CSPRNG bits, base64url, with a
//! recognisable prefix. Only their SHA-256 hash is ever stored. Also the
//! expiry rules: an expiry must lie in the future when the token is created,
//! and a token is expired from its expiry on (inclusive).

use std::fmt;

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use crate::domain::token_generator::random_base64url;

/// Recognisable prefix of every Admin Token, e.g. for secret scanners.
pub const PREFIX: &str = "vta_";

const SECRET_BYTES: usize = 32;

/// A freshly generated Admin Token secret. Its `Debug` output is redacted so it
/// can't end up in logs by accident.
pub struct AdminTokenSecret(String);

impl AdminTokenSecret {
    pub(crate) fn generate() -> Self {
        Self(format!("{PREFIX}{}", random_base64url(SECRET_BYTES)))
    }

    /// The bearer secret. Show it once; never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn hash(&self) -> AdminTokenHash {
        AdminTokenHash::of(&self.0)
    }
}

impl fmt::Debug for AdminTokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AdminTokenSecret(<redacted>)")
    }
}

/// SHA-256 of an Admin Token as presented or generated.
pub(crate) struct AdminTokenHash(pub Vec<u8>);

impl AdminTokenHash {
    pub fn of(presented: &str) -> Self {
        Self(Sha256::digest(presented.as_bytes()).to_vec())
    }
}

/// An operator's note on what an Admin Token is for: single-line free text
/// under the same rules as a Ballot's text fields (see `crate::input`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTokenLabel(String);

impl AdminTokenLabel {
    pub const MAX_CHARS: usize = 200;

    pub(crate) fn new(value: String, _: crate::input::Sanitized) -> Self {
        Self(value)
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// An Admin Token's expiry was not in the future when it was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpiryNotInFuture;

impl fmt::Display for ExpiryNotInFuture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the expiry must lie in the future")
    }
}

impl std::error::Error for ExpiryNotInFuture {}

/// Whether a new Admin Token may have this expiry at `now`: none, or one
/// that lies in the future.
pub fn check_new_expiry(
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<(), ExpiryNotInFuture> {
    match expires_at {
        Some(expires_at) if expires_at <= now => Err(ExpiryNotInFuture),
        _ => Ok(()),
    }
}

/// Whether an Admin Token with this expiry is refused at `now`: from its
/// expiry on (inclusive); never without one. The Admin Token store's lookup
/// applies the same rule in SQL (`expires_at IS NULL OR expires_at > now`).
pub fn is_expired(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    expires_at.is_some_and(|expires_at| expires_at <= now)
}
