//! Token generator: unguessable secrets from the operating system's CSPRNG,
//! encoded as base64url without padding.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// `bytes` random bytes from the OS CSPRNG, base64url-encoded without padding.
pub(crate) fn random_base64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the operating system's CSPRNG is unavailable");
    URL_SAFE_NO_PAD.encode(&buf)
}
