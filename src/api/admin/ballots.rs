//! Admin endpoints for Ballots.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::AppState;
use crate::api::conflict::Conflict;
use crate::api::{ballot_json, internal_error, rfc3339};
use crate::data::ballot_store::{self, DuplicateVoterReference, Edited, Extended};
use crate::domain::ballot::{Ballot, BallotDraft};
use crate::domain::theme::AccentColor;
use crate::input::{
    self, BallotIdPath, CreateBallotRequest, DateBounds, EditBallotRequest, ExtendRequest,
    ListBallotsQuery, NotFound, UntrustedJson, UntrustedQuery,
};

/// `POST /admin/ballots`: 201 with the admin view, including the public URL.
pub(super) async fn create(
    State(state): State<AppState>,
    UntrustedJson(request): UntrustedJson<CreateBallotRequest>,
) -> Response {
    let now = state.clock.now();
    let new = match input::new_ballot(request, date_bounds(&state, now), &*state.themes) {
        Ok(new) => new,
        Err(error) => return error.into_response(),
    };
    let draft = BallotDraft::new(new, state.config.default_purge_gap);
    match ballot_store::insert(&state.db, &draft, now).await {
        Ok(Ok(ballot)) => {
            let view = admin_view(&state, &ballot, now);
            (StatusCode::CREATED, Json(view)).into_response()
        }
        Ok(Err(DuplicateVoterReference)) => Conflict::DuplicateVoterReference.into_response(),
        Err(error) => internal_error("creating a Ballot failed", error),
    }
}

/// How admin-given dates are read (local ones in the configured
/// `default_time_zone`) and where they may lie at `now`, by the configured
/// horizon.
fn date_bounds(state: &AppState, now: DateTime<Utc>) -> DateBounds {
    let config = &state.config;
    DateBounds::new(now, config.max_date_horizon, config.default_time_zone)
}

/// The admin API's response for a Ballot ID that is malformed or unknown.
pub(crate) struct Admin;

impl NotFound for Admin {
    fn not_found() -> Response {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
    }
}

/// `GET /admin/ballots/{id}`: the admin view of one Ballot, or 404.
pub(super) async fn get_one(
    State(state): State<AppState>,
    BallotIdPath { id, .. }: BallotIdPath<Admin>,
) -> Response {
    let now = state.clock.now();
    match ballot_store::find_by_id(&state.db, id, now).await {
        Ok(Some(ballot)) => Json(admin_view(&state, &ballot, now)).into_response(),
        Ok(None) => Admin::not_found(),
        Err(error) => internal_error("looking up a Ballot failed", error),
    }
}

/// `PATCH /admin/ballots/{id}`: edits the Ballot while it has no Response
/// and no applied Fallback; 200 with the admin view, 409 `already_answered`
/// once it has a Response, 409 `fallback_applied` once its Fallback has
/// applied (Auto-fallback), unless the edit names only `group` and/or
/// `voter_reference` (the admin's bookkeeping), which may change at any time.
pub(super) async fn edit(
    State(state): State<AppState>,
    BallotIdPath { id, .. }: BallotIdPath<Admin>,
    UntrustedJson(request): UntrustedJson<EditBallotRequest>,
) -> Response {
    let now = state.clock.now();
    let (scope, request) = input::edit_scope(request);
    let edited = ballot_store::edit(&state.db, id, now, scope, |ballot| {
        input::ballot_changes(request, ballot, date_bounds(&state, now), &*state.themes)
    })
    .await;
    match edited {
        Ok(Edited::Yes(ballot)) => Json(admin_view(&state, &ballot, now)).into_response(),
        Ok(Edited::NotFound) => Admin::not_found(),
        Ok(Edited::Refused(refusal)) => Conflict::from(refusal).into_response(),
        Ok(Edited::Invalid(error)) => error.into_response(),
        Ok(Edited::DuplicateVoterReference) => Conflict::DuplicateVoterReference.into_response(),
        Err(error) => internal_error("editing a Ballot failed", error),
    }
}

/// `POST /admin/ballots/{id}/extend`: moves the Voting Deadline later, with
/// or without a Response; a closed Ballot without a Response reopens (its
/// status is derived). 200 with the admin view.
pub(super) async fn extend(
    State(state): State<AppState>,
    BallotIdPath { id, .. }: BallotIdPath<Admin>,
    UntrustedJson(request): UntrustedJson<ExtendRequest>,
) -> Response {
    let now = state.clock.now();
    let extended = ballot_store::extend(&state.db, id, now, |ballot| {
        input::extension(request, ballot, date_bounds(&state, now))
    })
    .await;
    match extended {
        Ok(Extended::Yes(ballot)) => Json(admin_view(&state, &ballot, now)).into_response(),
        Ok(Extended::NotFound) => Admin::not_found(),
        Ok(Extended::Invalid(error)) => error.into_response(),
        Err(error) => internal_error("extending a Voting Deadline failed", error),
    }
}

/// `DELETE /admin/ballots/{id}`: deletes the Ballot at once, Response and
/// all; 204, or 404 if there is no such Ballot (e.g. deleted or purged
/// already).
/// Afterwards its Ballot Token gives the voter surfaces' uniform not-found.
pub(super) async fn delete(
    State(state): State<AppState>,
    BallotIdPath { id, .. }: BallotIdPath<Admin>,
) -> Response {
    match ballot_store::delete(&state.db, id, state.clock.now()).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => Admin::not_found(),
        Err(error) => internal_error("deleting a Ballot failed", error),
    }
}

/// `GET /admin/ballots[?group=…][&status=…]`: the matching Ballots in creation
/// order, each in the admin view with its raw result; all matches, no
/// pagination, no tallies across Ballots. The status filter compares the
/// Ballot Status at the Clock's now.
pub(super) async fn list(
    State(state): State<AppState>,
    UntrustedQuery(query): UntrustedQuery<ListBallotsQuery>,
) -> Response {
    let filter = match input::ballot_filter(query) {
        Ok(filter) => filter,
        Err(error) => return error.into_response(),
    };
    let now = state.clock.now();
    match ballot_store::list(&state.db, &filter, now).await {
        Ok(ballots) => {
            let views: Vec<_> = ballots
                .iter()
                .map(|ballot| admin_view(&state, ballot, now))
                .collect();
            Json(json!({ "ballots": views })).into_response()
        }
        Err(error) => internal_error("listing Ballots failed", error),
    }
}

/// The admin's view of a Ballot at `now`, with its derived Ballot Status and
/// result (the Response, or the applied Fallback, marked by `source`).
fn admin_view(state: &AppState, ballot: &Ballot, now: DateTime<Utc>) -> Value {
    json!({
        "id": ballot.id,
        "token": ballot.token.as_str(),
        "url": ballot_url(state, ballot),
        "title": ballot.title,
        "description": ballot.description,
        "options": ballot_json::options(ballot),
        "choice_mode": ballot.choice_mode.as_str(),
        "allow_no_option": ballot.allow_no_option,
        "submission_mode": ballot.submission_mode.as_str(),
        "voting_deadline": rfc3339(ballot.voting_deadline),
        "purge_date": rfc3339(ballot.purge_date),
        "created_at": rfc3339(ballot.created_at),
        "group": ballot.group,
        "voter_reference": ballot.voter_reference,
        "theme": ballot.theme,
        "accent_color": ballot.accent_color.as_ref().map(AccentColor::as_str),
        "logo": ballot.logo,
        "fallback": ballot.fallback.as_ref().map(|ids| ballot_json::options_with_ids(ballot, ids)),
        "status": ballot.status(now).as_str(),
        "result": ballot.result(now).map(|result| json!({
            "source": result.source.as_str(),
            "option_ids": result.option_ids,
            "options": ballot_json::options_with_ids(ballot, result.option_ids),
        })),
        "response": ballot.response.as_ref().map(ballot_json::response),
    })
}

/// `<public_base_url>b/<token>` (the base URL ends in `/`): from
/// configuration only, never from request headers, so returned links can't
/// be spoofed.
fn ballot_url(state: &AppState, ballot: &Ballot) -> String {
    let base = state.config.public_base_url.as_str();
    format!("{base}b/{}", ballot.token.as_str())
}
