//! Admin Token labels, given on the command line (`admin-token create
//! --label`): the operator is trusted, but the label is later printed on a
//! terminal, so it follows the rules of any single-line text.

use untrusted_value::{SanitizeValue, SanitizeWith as _, UntrustedValue};

use super::Sanitized;
use super::text::{self, Rejection};
use crate::domain::admin_token::AdminTokenLabel;

impl SanitizeValue<AdminTokenLabel> for UntrustedValue<String> {
    type Error = Rejection;

    fn sanitize_value(self) -> Result<AdminTokenLabel, Rejection> {
        self.sanitize_with(|raw| {
            let label = text::single_line(raw, AdminTokenLabel::MAX_CHARS)?;
            Ok(AdminTokenLabel::new(label, Sanitized::PROOF))
        })
    }
}
