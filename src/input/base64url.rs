//! Format check for server-generated random IDs (Ballot Tokens, Option IDs).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Whether `raw` is exactly how the server encodes `bytes` random bytes:
/// base64url without padding, of the right length, decoding canonically.
pub(super) fn is_canonical(raw: &str, bytes: usize) -> bool {
    raw.len() == bytes.div_ceil(3) * 4 - (3 - bytes % 3) % 3
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && URL_SAFE_NO_PAD
            .decode(raw)
            .is_ok_and(|decoded| decoded.len() == bytes)
}
