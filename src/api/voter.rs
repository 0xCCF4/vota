//! Voter API: the Ballot as its voter may see it, by Ballot Token, with its
//! Ballot Status, Response and result, and its description as sanitised
//! Markdown source, not HTML (`GET /api/b/{token}`), and submitting the
//! Response (`PUT /api/b/{token}/response` with the `revision` it was
//! fetched at; 409 `voting_closed` from the Voting Deadline on, 409
//! `ballot_changed` if an admin edit landed since it was fetched or while it
//! was being submitted, or it names an Option the Ballot no longer has).
//! Internal errors are 500 `{"error":"internal"}` with the not-found's
//! security headers. Never exposes admin-only fields (internal ID, Purge Date, Group, Voter
//! Reference). The Fallback is shown from the start
//! (`fallback`), so the voter knows what is recorded if they don't answer.
//! The Theme and Theme Tweaks are given as the voting page applies them:
//! `theme` (the name in use: the default if the Ballot's Theme has gone),
//! `accent_color` and `logo_url` (absolute, or `null` without a logo or if
//! it has gone), never a filesystem path.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::AppState;
use crate::api::conflict::Conflict;
use crate::api::{ballot_json, internal_error, rfc3339};
use crate::data::ballot_store::{self, Recorded};
use crate::domain::ballot::{Ballot, BallotToken};
use crate::domain::theme::AccentColor;
use crate::http::error_page;
use crate::input::{
    self, BallotTokenPath, NotFound, ResponseRequest, SelectionRejection, UntrustedJson,
};

/// The voter API surface. Its single not-found response serves unknown and
/// malformed Ballot Tokens alike.
pub(crate) struct Api;

impl NotFound for Api {
    fn not_found() -> Response {
        error_page::harden(
            (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
        )
    }
}

pub(crate) fn router() -> Router<AppState> {
    // Anything else below `/api/b/` (empty, multi-segment or trailing-slash
    // paths) gets the same uniform not-found, from explicit routes: a nested
    // router's fallback is lost once the app is nested under a base path.
    Router::new()
        .route("/api/b/{token}", get(get_ballot))
        .route("/api/b/{token}/response", put(put_response))
        .route("/api/b/", any(not_found))
        .route("/api/b/{token}/", any(not_found))
        .route("/api/b/{token}/{*rest}", any(not_found))
        .route("/api/b//{*rest}", any(not_found))
}

async fn not_found() -> Response {
    Api::not_found()
}

async fn get_ballot(
    State(state): State<AppState>,
    BallotTokenPath { token, .. }: BallotTokenPath<Api>,
) -> Response {
    show(&state, &token, state.clock.now()).await
}

/// The voter view of the Ballot with this token, or not found.
async fn show(state: &AppState, token: &BallotToken, now: DateTime<Utc>) -> Response {
    match ballot_store::find_by_token(&state.db, token, now).await {
        Ok(Some(ballot)) => Json(voter_view(state, &ballot, now)).into_response(),
        Ok(None) => Api::not_found(),
        Err(error) => internal_error("looking up a Ballot failed", error),
    }
}

/// `PUT /api/b/{token}/response`: records the voter's Response and answers
/// with the updated voter view.
async fn put_response(
    State(state): State<AppState>,
    BallotTokenPath { token, .. }: BallotTokenPath<Api>,
    UntrustedJson(request): UntrustedJson<ResponseRequest>,
) -> Response {
    let now = state.clock.now();
    let ballot = match ballot_store::find_by_token(&state.db, &token, now).await {
        Ok(Some(ballot)) => ballot,
        Ok(None) => return Api::not_found(),
        Err(error) => return internal_error("looking up a Ballot failed", error),
    };
    if let Err(refusal) = ballot.accepts_response(now) {
        return Conflict::from(refusal).into_response();
    }
    let response = match input::api_response(request, &ballot) {
        Ok(response) => response,
        Err(SelectionRejection::Invalid(error)) => return error.into_response(),
        // An admin edit since the voter fetched the Ballot, or an Option
        // the admin has since replaced.
        Err(SelectionRejection::Stale | SelectionRejection::NotOnBallot) => {
            return Conflict::BallotChanged.into_response();
        }
    };
    let recorded = ballot_store::record_response(
        &state.db,
        &ballot,
        response.revision,
        &response.selection,
        now,
    )
    .await;
    match recorded {
        Ok(Recorded::Yes) => show(&state, &token, now).await,
        Ok(Recorded::AlreadyAnswered) => Conflict::ResponseLocked.into_response(),
        Ok(Recorded::VotingClosed) => Conflict::VotingClosed.into_response(),
        Ok(Recorded::BallotChanged) => Conflict::BallotChanged.into_response(),
        Err(error) => internal_error("recording a Response failed", error),
    }
}

/// The Ballot as its voter may see it at `now`, with its status and current
/// Response, and its Theme and Theme Tweaks as the voting page applies them.
fn voter_view(state: &AppState, ballot: &Ballot, now: DateTime<Utc>) -> Value {
    // As on the voting page: a Theme that has gone is the default Theme, a
    // logo that has gone is none. URLs are absolute, under the public base URL.
    let look = state
        .themes
        .look(ballot, state.config.public_base_url.as_str());
    json!({
        "title": ballot.title,
        // Sent back with a Response: recorded only if the Ballot still
        // stands as fetched.
        "revision": ballot.revision,
        // The sanitised Markdown source, not rendered HTML: API clients
        // render (and sanitise) it as suits them, and get no markup to trust.
        "description": ballot.description,
        "options": ballot_json::options(ballot),
        "choice_mode": ballot.choice_mode.as_str(),
        // Whether a Response may select no Option at all.
        "allow_no_option": ballot.allow_no_option,
        "submission_mode": ballot.submission_mode.as_str(),
        "voting_deadline": rfc3339(ballot.voting_deadline),
        "voting_open": ballot.is_open(now),
        "status": ballot.status(now).as_str(),
        // What will be recorded without a Response by the Voting Deadline
        // (null without a Fallback). Only `result` says whether it applied.
        "fallback": ballot.fallback.as_ref().map(|option_ids| json!({
            "option_ids": option_ids,
        })),
        "result": ballot.result(now).map(|result| json!({
            "source": result.source.as_str(),
            "option_ids": result.option_ids,
        })),
        "response": ballot.response.as_ref().map(ballot_json::response),
        "theme": look.theme,
        "accent_color": ballot.accent_color.as_ref().map(AccentColor::as_str),
        // Absolute, from the configured public base URL.
        "logo_url": look.logo_url(),
    })
}
