//! The HTTP layer shared by [`crate::api`] and [`crate::web`]: middleware that
//! applies across surfaces, such as rate limiting of failed Ballot Token
//! lookups and `Cache-Control: no-store` on the voter surfaces, the headers
//! on every response of the app, and the security headers of the shared
//! error responses. No handlers live here.

pub(crate) mod error_page;
pub(crate) mod no_store;
pub(crate) mod rate_limit;
pub(crate) mod security_headers;
