use untrusted_value::{SanitizeValue, SanitizeWith as _, UntrustedValue};

use super::Sanitized;
use super::text::Rejection;
use crate::domain::ballot::BallotId;

impl SanitizeValue<BallotId> for UntrustedValue<String> {
    type Error = Rejection;

    /// Strict format check, before any lookup: a positive decimal integer as
    /// the server writes it (ASCII digits only, no sign, no leading zero, no
    /// whitespace) that fits the ID range.
    fn sanitize_value(self) -> Result<BallotId, Rejection> {
        self.sanitize_with(|raw| {
            let digits_only = !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit());
            if !digits_only || raw.starts_with('0') {
                return Err("not a Ballot ID");
            }
            raw.parse::<i64>()
                .map(|id| BallotId::new(id, Sanitized::PROOF))
                .map_err(|_| "not a Ballot ID")
        })
    }
}
