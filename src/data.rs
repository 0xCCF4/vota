//! Persistence: the SQLite pool and migrations, the Ballot store and the Admin
//! Token store. The only place SQL lives. Timestamps are stored as UTC Unix
//! milliseconds.

pub mod admin_tokens;
pub mod ballot_store;
pub(crate) mod db;
mod millis;
