//! Admin Token labels, given on the command line (`admin-token create
//! --label`): the operator is trusted, but the label is later printed on a
//! terminal, so it follows the rules of any single-line text.
//!
//! And the contents of an Admin Token file (`admin_token_files`): one Admin
//! Token in the format the server generates, `vta_` and 43 canonical
//! base64url characters, with any surrounding whitespace (a trailing newline)
//! trimmed. Nothing else may be in the file.

use untrusted_value::{SanitizeValue, SanitizeWith as _, UntrustedValue};

use super::Sanitized;
use super::base64url;
use super::text::{self, Rejection};
use crate::domain::admin_token::{AdminTokenLabel, PREFIX, ProvisionedAdminToken, SECRET_BYTES};

impl SanitizeValue<AdminTokenLabel> for UntrustedValue<String> {
    type Error = Rejection;

    fn sanitize_value(self) -> Result<AdminTokenLabel, Rejection> {
        self.sanitize_with(|raw| {
            let label = text::single_line(raw, AdminTokenLabel::MAX_CHARS)?;
            Ok(AdminTokenLabel::new(label, Sanitized::PROOF))
        })
    }
}

/// The contents of an Admin Token file are not one Admin Token. Says what is
/// expected, never what the file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedAdminToken;

impl std::fmt::Display for MalformedAdminToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "it must hold exactly one Admin Token: {PREFIX} and 43 base64url characters, \
             as `vota admin-token generate` prints it"
        )
    }
}

impl std::error::Error for MalformedAdminToken {}

/// The Admin Token in a file's contents (surrounding whitespace trimmed),
/// kept as its hash only.
pub fn admin_token_file(
    contents: UntrustedValue<Vec<u8>>,
) -> Result<ProvisionedAdminToken, MalformedAdminToken> {
    contents.sanitize_with(|raw| {
        let text = std::str::from_utf8(&raw).map_err(|_| MalformedAdminToken)?;
        let token = text.trim();
        let secret = token.strip_prefix(PREFIX).ok_or(MalformedAdminToken)?;
        if !base64url::is_canonical(secret, SECRET_BYTES) {
            return Err(MalformedAdminToken);
        }
        Ok(ProvisionedAdminToken::new(token, Sanitized::PROOF))
    })
}
