//! The domain: Ballots and their rules (bulk creation's in `bulk`), validated value types, the Clock and
//! token generation. No database, no HTTP; "now" is always an input.

pub mod admin_token;
pub mod ballot;
pub mod bulk;
pub mod clock;
pub mod theme;
pub(crate) mod token_generator;
