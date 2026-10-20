//! Vota: personal Ballots with an admin API and a voting UI.
//!
//! Layout: `domain` (rules and value types, no I/O), `input` (the single
//! sanitization boundary), `data` (SQLite, the only place with SQL), `api`
//! (JSON handlers), `web` (server-rendered voter pages), `http` (middleware
//! shared by `api` and `web`, e.g. rate limiting), `jobs` (background jobs
//! such as the purge sweep), `app` (the state and the router) and [`cli`].
//!
//! Cargo features: `server` (on by default) is everything but [`cli`]'s
//! client and `completions`; `client` (on by default) is `vota ballot`, the
//! admin API client. The character rules in `input` serve both.

#![forbid(unsafe_code)]

#[cfg(feature = "server")]
mod api;
#[cfg(feature = "server")]
mod app;
pub mod cli;
#[cfg(feature = "server")]
pub mod config;
#[cfg(feature = "server")]
pub mod data;
#[cfg(feature = "server")]
pub mod domain;
#[cfg(feature = "server")]
mod http;
#[cfg(any(feature = "server", feature = "client"))]
mod input;
#[cfg(feature = "server")]
pub mod jobs;
#[cfg(feature = "server")]
mod web;

#[cfg(feature = "server")]
pub use app::{AppState, app};
/// Kept at the crate root for convenience: `vota::clock::ManualClock`.
#[cfg(feature = "server")]
pub use domain::clock;
