use untrusted_value::{SanitizeValue, SanitizeWith as _, UntrustedValue};

use super::Sanitized;
use super::base64url;
use super::text::Rejection;
use crate::domain::ballot::BallotToken;

impl SanitizeValue<BallotToken> for UntrustedValue<String> {
    type Error = Rejection;

    /// Strict format check, before any lookup: exactly 22 base64url characters
    /// that decode canonically to 16 bytes.
    fn sanitize_value(self) -> Result<BallotToken, Rejection> {
        self.sanitize_with(|raw| {
            if base64url::is_canonical(&raw, BallotToken::BYTES) {
                Ok(BallotToken::new(raw, Sanitized::PROOF))
            } else {
                Err("not a Ballot Token")
            }
        })
    }
}
