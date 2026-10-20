//! Server-rendered voter pages (`/b/{token}`), via minijinja templates, and
//! the Theme registry with its assets (`/assets/…`).

mod assets;
mod ballot_page;
mod enhancement;
mod logos;
mod markdown;
mod revision_field;
mod templates;
pub(crate) mod themes;
mod trial_render;

use axum::Router;

use crate::AppState;

/// The voting pages (`/b/…`), whose not-found responses count toward the
/// rate limit.
pub(crate) fn router() -> Router<AppState> {
    ballot_page::router()
}

/// Theme stylesheets and logos (`/assets/…`): not rate-limited, as they
/// involve no Ballot Token.
pub(crate) fn assets_router() -> Router<AppState> {
    assets::router()
}
