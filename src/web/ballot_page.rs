//! The voting page of one Ballot.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{any, get};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use minijinja::context;

use super::themes::Look;
use crate::AppState;
use crate::data::ballot_store::{self, Recorded};
use crate::domain::ballot::{Ballot, BallotOption, BallotStatus, ChoiceMode, SubmissionMode};
use crate::domain::ballot::{BallotResult, OptionId, Refusal, ResultSource, Selection};
use crate::domain::theme::AccentColor;
use crate::domain::token_generator::random_base64url;
use crate::http::error_page;
use crate::input::{
    self, BallotTokenPath, BodyRejection, FormProblem, NotFound, ResponseForm, UntrustedForm,
};

/// The voting-page surface. Its single not-found page serves unknown and
/// malformed Ballot Tokens alike.
pub(crate) struct Page;

impl NotFound for Page {
    fn not_found() -> Response {
        error_page::page(StatusCode::NOT_FOUND, error_page::NOT_FOUND_PAGE)
    }
}

pub(super) fn router() -> Router<AppState> {
    // Anything else below `/b/` (empty, multi-segment or trailing-slash
    // paths) gets the same uniform not-found, from explicit routes: a nested
    // router's fallback is lost once the app is nested under a base path.
    Router::new()
        .route("/b/{token}", get(show).post(submit))
        .route("/b/", any(not_found))
        .route("/b/{token}/", any(not_found))
        .route("/b/{token}/{*rest}", any(not_found))
        .route("/b//{*rest}", any(not_found))
}

/// Logs `error` (whose messages never carry Ballot Tokens or Voter
/// References: neither is in a page's template context, and database errors
/// don't echo bound values) and answers with the neutral error page.
fn server_error(context: &str, error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "{context}");
    error_page::page(
        StatusCode::INTERNAL_SERVER_ERROR,
        error_page::SERVER_ERROR_PAGE,
    )
}

async fn not_found() -> Response {
    Page::not_found()
}

async fn show(
    State(state): State<AppState>,
    BallotTokenPath { token, .. }: BallotTokenPath<Page>,
) -> Response {
    let now = state.clock.now();
    match ballot_store::find_by_token(&state.db, &token, now).await {
        Ok(Some(ballot)) => render(&state, &ballot, now),
        Ok(None) => Page::not_found(),
        Err(error) => server_error("looking up a Ballot failed", error),
    }
}

/// `POST /b/{token}`: the no-JS form. On a Final Ballot in two steps: a post
/// without `confirm=yes` shows the confirmation step (nothing is stored); the
/// confirmation page posts the same selection again with `confirm=yes`, which
/// records the Response and redirects to the page (post/redirect/get), now a
/// read-only view of it. On an Editable Ballot every valid post records (or
/// replaces) the Response at once and redirects to the page, which shows the
/// form pre-filled with it.
async fn submit(
    State(state): State<AppState>,
    BallotTokenPath { token, .. }: BallotTokenPath<Page>,
    form: Result<UntrustedForm<ResponseForm>, BodyRejection>,
) -> Response {
    let now = state.clock.now();
    let ballot = match ballot_store::find_by_token(&state.db, &token, now).await {
        Ok(Some(ballot)) => ballot,
        Ok(None) => return Page::not_found(),
        Err(error) => return server_error("looking up a Ballot failed", error),
    };
    // Deliberately before the form's own errors: a Ballot that takes no
    // Response answers 409 with the page saying why (closed, answered), even
    // to an oversized (413) or wrongly typed (415) form. It tells nothing new:
    // only a valid Ballot Token gets this far.
    if let Err(refusal) = ballot.accepts_response(now) {
        return refused(&state, &ballot, now, refusal);
    }
    let form_response = match form.map(|UntrustedForm(form)| input::form_response(form, &ballot)) {
        Ok(Ok(form_response)) => form_response,
        // An admin edit since the page was rendered, or an Option the admin
        // has since replaced: the Ballot as it now stands, as after an admin
        // edit lost the race.
        Ok(Err(FormProblem::Stale | FormProblem::NotOnBallot)) => {
            return refused(&state, &ballot, now, Refusal::BallotChanged);
        }
        Ok(Err(problem)) => return invalid_response(&state, &ballot, now, problem),
        Err(BodyRejection::Invalid(_)) => {
            return invalid_response(&state, &ballot, now, FormProblem::Unreadable);
        }
        Err(rejection) => return rejection.into_response(),
    };
    if ballot.needs_confirmation() && !form_response.confirmed {
        return render_confirmation(&state, &ballot, &form_response.selection);
    }
    let recorded = ballot_store::record_response(
        &state.db,
        &ballot,
        form_response.revision,
        &form_response.selection,
        now,
    )
    .await;
    let refusal = match recorded {
        // Post/redirect/get: the page now shows the recorded Response.
        Ok(Recorded::Yes) => {
            let base_path = state.config.public_base_url.base_path();
            return Redirect::to(&format!("{base_path}b/{}", token.as_str())).into_response();
        }
        Ok(Recorded::AlreadyAnswered) => Refusal::AlreadyAnswered,
        Ok(Recorded::VotingClosed) => Refusal::VotingClosed,
        Ok(Recorded::BallotChanged) => Refusal::BallotChanged,
        Err(error) => return server_error("recording a Response failed", error),
    };
    // Lost a race (against another submission, the deadline or an admin
    // edit): show the Ballot as it now stands.
    match ballot_store::find_by_token(&state.db, &token, now).await {
        Ok(Some(ballot)) => refused(&state, &ballot, now, refusal),
        Ok(None) => Page::not_found(),
        Err(error) => server_error("looking up a Ballot failed", error),
    }
}

/// 409: the read-only page with a notice why the Response can't be submitted
/// or changed.
fn refused(state: &AppState, ballot: &Ballot, now: DateTime<Utc>, refusal: Refusal) -> Response {
    let message = match refusal {
        Refusal::AlreadyAnswered => {
            "This Ballot has already been answered. Your answer cannot be changed."
        }
        Refusal::VotingClosed => "Voting has closed. Your answer could not be accepted.",
        Refusal::BallotChanged => {
            "This Ballot was changed by its organiser, so your answer was not saved. Please check it and answer again."
        }
    };
    render_with(state, ballot, now, StatusCode::CONFLICT, Some(message))
}

/// 422: the voting form again, with a message naming the problem. The
/// message is fixed per problem and never echoes what was submitted.
fn invalid_response(
    state: &AppState,
    ballot: &Ballot,
    now: DateTime<Utc>,
    problem: FormProblem,
) -> Response {
    let message = match (problem, ballot.choice_mode) {
        (FormProblem::WrongCount, ChoiceMode::Single) => "Select exactly one Option.",
        // Never with Allow No Option: then any number of Options is fine.
        (FormProblem::WrongCount, ChoiceMode::Multiple) => "Select at least one Option.",
        // `NotOnBallot` is answered by `refused` instead; should it get
        // here, it reads like a malformed value.
        (FormProblem::UnknownOption | FormProblem::NotOnBallot, _) => {
            "That Option is not part of this Ballot."
        }
        // `Stale` is answered by `refused` instead.
        (FormProblem::InvalidRevision | FormProblem::Stale, _) => {
            "The page was out of date or incomplete. Please check the Ballot and answer again."
        }
        (FormProblem::DuplicateOption, _) => "Select each Option only once.",
        (FormProblem::InvalidConfirmation, _) => {
            "The confirmation was not valid. Please try again."
        }
        (FormProblem::Unreadable, _) => "The form could not be read. Please try again.",
    };
    render_with(
        state,
        ballot,
        now,
        StatusCode::UNPROCESSABLE_ENTITY,
        Some(message),
    )
}

/// The confirmation step: the chosen Options, a warning, and a button that
/// posts them again with `confirm=yes`.
fn render_confirmation(state: &AppState, ballot: &Ballot, selection: &Selection) -> Response {
    let nonce = csp_nonce();
    let context =
        confirmation_context(ballot, selection.option_ids(), &look(state, ballot), &nonce);
    match state.themes.render(&ballot.theme, "confirm.html", context) {
        Ok(html) => page(
            StatusCode::OK,
            super::revision_field::include(html, ballot.revision),
            &nonce,
        ),
        Err(error) => server_error("rendering the confirmation page failed", error),
    }
}

/// What `confirm.html` gets: the Ballot and the chosen Options (by ID; none
/// if the Ballot allows no Option and the voter chose none).
pub(super) fn confirmation_context(
    ballot: &Ballot,
    selected: &[OptionId],
    look: &Look<'_>,
    nonce: &str,
) -> minijinja::Value {
    context! {
        title => &ballot.title,
        description => description_html(ballot),
        selected => options_context(ballot, selected),
        // The confirmation is recorded only if the Ballot still stands as
        // shown. The app inserts the hidden field itself
        // (`revision_field`); this is for Themes that still write it.
        revision => ballot.revision,
        ..theme_context(look, ballot, nonce)
    }
}

fn render(state: &AppState, ballot: &Ballot, now: DateTime<Utc>) -> Response {
    render_with(state, ballot, now, StatusCode::OK, None)
}

/// The voting page at `now`, with an optional notice for the voter.
fn render_with(
    state: &AppState,
    ballot: &Ballot,
    now: DateTime<Utc>,
    status: StatusCode,
    message: Option<&str>,
) -> Response {
    let nonce = csp_nonce();
    let context = voting_page_context(
        ballot,
        now,
        state.config.default_time_zone,
        message,
        &look(state, ballot),
        &nonce,
    );
    match state.themes.render(&ballot.theme, "ballot.html", context) {
        Ok(html) => page(
            status,
            super::enhancement::include(
                super::revision_field::include(html, ballot.revision),
                ballot,
                now,
                state.config.public_base_url.base_path(),
            ),
            &nonce,
        ),
        Err(error) => server_error("rendering the voting page failed", error),
    }
}

/// What `ballot.html` gets at `now`: the form (pre-filled with the current
/// Response on an Editable Ballot; without a Response, with the Fallback
/// that applies if none comes), a read-only view of the Response once the
/// Ballot accepts no more, or, once voting has closed, a read-only view of
/// the outcome; with an optional notice for the voter.
pub(super) fn voting_page_context(
    ballot: &Ballot,
    now: DateTime<Utc>,
    zone: Tz,
    message: Option<&str>,
    look: &Look<'_>,
    nonce: &str,
) -> minijinja::Value {
    let deadline = ballot.voting_deadline.with_timezone(&zone);
    let current: &[OptionId] = match &ballot.response {
        Some(response) => &response.option_ids,
        None => &[],
    };
    // The form, its inputs checked as in the current Response (if any).
    let all: Vec<_> = ballot
        .options
        .iter()
        .map(|o| {
            context! {
                id => o.id.as_str(),
                label => &o.label,
                checked => current.contains(&o.id),
            }
        })
        .collect();
    let editable = ballot.submission_mode == SubmissionMode::Editable;
    let answered = ballot.response.is_some();
    context! {
        title => &ballot.title,
        description => description_html(ballot),
        options => all,
        multiple => ballot.choice_mode == ChoiceMode::Multiple,
        // Whether the voter may answer with no Option (multiple choice
        // only); `selected` is then possibly empty once answered.
        allow_no_option => ballot.allow_no_option,
        editable => editable,
        answered => answered,
        // Whether the Response is shown without a form.
        read_only => answered && ballot.accepts_response(now).is_err(),
        closed => !ballot.is_open(now),
        // While voting is open and there is no Response: what will be
        // recorded if the voter doesn't answer by the deadline.
        upcoming_fallback => match &ballot.fallback {
            Some(option_ids) if ballot.is_open(now) && !answered => {
                Some(options_context(ballot, option_ids))
            }
            _ => None,
        },
        // Once voting has closed: the Fallback, if it applies.
        fallback => match ballot.result(now) {
            Some(BallotResult { source: ResultSource::Fallback, option_ids }) => {
                Some(options_context(ballot, option_ids))
            }
            _ => None,
        },
        selected => options_context(ballot, current),
        message => message,
        deadline_utc => crate::api::rfc3339(ballot.voting_deadline),
        deadline_local => deadline.format("%a %-d %b %Y, %H:%M %Z").to_string(),
        status => status_label(ballot.status(now)),
        // The form's Response is recorded only if the Ballot still stands
        // as shown. The app inserts the hidden field itself
        // (`revision_field`); this is for Themes that still write it.
        revision => ballot.revision,
        ..theme_context(look, ballot, nonce)
    }
}

/// A fresh nonce for one page's Content Security Policy: 128 CSPRNG bits.
fn csp_nonce() -> String {
    random_base64url(16)
}

/// A themed page with its security headers. The Content Security Policy
/// allows no third-party sources at all: scripts, styles, images and
/// requests only from our own origin, plus inline styles carrying this
/// page's `nonce` (how the accent colour gets in; see `base.html`).
fn page(status: StatusCode, html: String, nonce: &str) -> Response {
    let csp = format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'nonce-{nonce}'; \
         img-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'; \
         frame-ancestors 'none'"
    );
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    if let Ok(csp) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

/// The Ballot's Theme and logo as the registry has them now, for
/// root-relative URLs under the base path.
fn look<'a>(state: &'a AppState, ballot: &'a Ballot) -> Look<'a> {
    state
        .themes
        .look(ballot, state.config.public_base_url.base_path())
}

/// What every page template gets about the Ballot's Theme and Theme Tweaks,
/// and the page's style nonce.
fn theme_context(look: &Look<'_>, ballot: &Ballot, nonce: &str) -> minijinja::Value {
    context! {
        // The name of the Theme in use.
        theme => look.theme,
        // `/`, or e.g. `/polls/`: plain segments only (checked at startup),
        // so it needs no escaping.
        base_path => minijinja::Value::from_safe_string(look.base.to_owned()),
        // Root-relative, from the base path and a registry name, so it needs
        // no escaping.
        logo_url => look.logo_url().map(minijinja::Value::from_safe_string),
        csp_nonce => nonce,
        // `#rrggbb` (validated), or none: the Theme's own accent colour.
        accent_color => ballot.accent_color.as_ref().map(AccentColor::as_str),
        // `#000000` or `#ffffff`, whichever reads better on the accent.
        accent_contrast => ballot.accent_color.as_ref().map(AccentColor::contrast_text),
        // Built from the base path and a well-formed name only, so it needs
        // no escaping (and minijinja would otherwise escape its `/`).
        theme_stylesheet => minijinja::Value::from_safe_string(look.theme_stylesheet_url()),
    }
}

/// The Ballot's description rendered to sanitised HTML, marked safe so the
/// template inserts it unescaped: the only free-form value on the page that
/// bypasses auto-escaping (besides it, only the asset URLs built from
/// registry names do).
fn description_html(ballot: &Ballot) -> Option<minijinja::Value> {
    let source = ballot.description.as_deref()?;
    Some(minijinja::Value::from_safe_string(super::markdown::render(
        source,
    )))
}

/// The Ballot's Options with these IDs, in the Ballot's order.
fn options_context(ballot: &Ballot, ids: &[OptionId]) -> Vec<minijinja::Value> {
    ballot
        .options
        .iter()
        .filter(|o| ids.contains(&o.id))
        .map(option_context)
        .collect()
}

fn option_context(option: &BallotOption) -> minijinja::Value {
    context! { id => option.id.as_str(), label => &option.label }
}

/// How a Ballot Status reads on the page.
fn status_label(status: BallotStatus) -> &'static str {
    match status {
        BallotStatus::Unanswered => "Unanswered",
        BallotStatus::Answered => "Answered",
        BallotStatus::AutoFallback => "Auto-fallback",
    }
}
