//! Admin Token secrets (ADR 0001): 256 CSPRNG bits, base64url, with a
//! recognisable prefix. Only their SHA-256 hash is ever stored. Also the
//! expiry rules: an expiry must lie in the future when the token is created,
//! and a token is expired from its expiry on (inclusive). And the Admin
//! Tokens provisioned from files (`FileAdminTokens`), also kept as hashes
//! only.

use std::fmt;

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use crate::domain::token_generator::random_base64url;

/// Recognisable prefix of every Admin Token, e.g. for secret scanners.
pub const PREFIX: &str = "vta_";

/// Random bytes in an Admin Token: 43 base64url characters after the prefix.
pub(crate) const SECRET_BYTES: usize = 32;

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

    /// Whether both are the same hash, in time independent of where they
    /// differ. (Comparing hashes leaks nothing useful about the token
    /// anyway, as the database's lookup by hash relies on; this costs
    /// nothing more.)
    fn matches(&self, other: &Self) -> bool {
        let diff = (self.0.iter().zip(&other.0)).fold(0, |diff, (a, b)| diff | (a ^ b));
        self.0.len() == other.0.len() && diff == 0
    }
}

/// A well-formed Admin Token read from a file (`admin_token_files`), kept
/// as its hash only: the secret is dropped once it is hashed.
pub(crate) struct ProvisionedAdminToken(AdminTokenHash);

impl ProvisionedAdminToken {
    pub(crate) fn new(secret: &str, _: crate::input::Sanitized) -> Self {
        Self(AdminTokenHash::of(secret))
    }
}

/// The Admin Tokens provisioned from files, in the order of
/// `admin_token_files`. They have no ID, label, expiry or last use in the
/// database; one stops working when its file leaves the list (at the next
/// start).
#[derive(Default)]
pub(crate) struct FileAdminTokens(Vec<ProvisionedAdminToken>);

impl FileAdminTokens {
    pub(crate) fn new(tokens: Vec<ProvisionedAdminToken>) -> Self {
        Self(tokens)
    }

    /// The position in `admin_token_files` (counted from 1) of the first file
    /// holding the token `presented` is. Every file is compared, match or
    /// not.
    pub(crate) fn position(&self, presented: &str) -> Option<usize> {
        let presented = AdminTokenHash::of(presented);
        self.0
            .iter()
            .enumerate()
            .filter(|(_, token)| token.0.matches(&presented))
            .map(|(index, _)| index + 1)
            .min()
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
