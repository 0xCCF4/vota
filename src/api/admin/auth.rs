//! Bearer-token check for the whole admin router. Missing, unknown and expired
//! (per Clock) Admin Tokens all get the same bare 401. Tokens are never logged.
//! A token passes if it is one of the Admin Token files' (compared by hash,
//! without the database) or a stored one.
//!
//! Failed checks are rate-limited per client IP with their own budget
//! (`rate_limit.admin_failed_auth` per `rate_limit.admin_window`, see
//! [`crate::http::rate_limit`]): a client that has spent it gets 429 for
//! every admin request until its window ends, before any token check. A
//! request that passes the check spends nothing.
//!
//! The audit log: every admin request, whatever its outcome, is one
//! `info` event "admin request" of the target `vota::admin_audit`, with
//! the fields `token_id` (absent unless the check passed: a stored Admin
//! Token's ID as `admin-token list` shows it, e.g. `7`, or `file:<n>` for
//! the token of the n-th file of `admin_token_files`, counted from 1, which
//! can't collide with an ID), `method`, `route` (the route
//! template, e.g. `/admin/ballots/{id}`, never the request's path or
//! query), `status` and `client_ip` (as the rate limit resolves it; absent
//! without a known peer). Failed checks are summed up per client IP rather
//! than warned about one by one: a `warn` event at a client's first failure
//! in a window and one when it spends the budget. Nothing else about the
//! request is logged: no Admin Token, Ballot Token, Voter Reference or body.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::api::internal_error;
use crate::data::admin_tokens;
use crate::http::rate_limit::{self, Budget};
use crate::input;

/// The target of the audit log's events, for filtering (`RUST_LOG`).
const AUDIT: &str = "vota::admin_audit";

/// What the audit event says about a request.
#[derive(Default)]
struct Audit {
    token: Option<AcceptedToken>,
    client: Option<IpAddr>,
}

/// The Admin Token a request passed the check with, as the audit log's
/// `token_id` shows it.
#[derive(Clone, Copy)]
enum AcceptedToken {
    /// A stored Admin Token: its ID, e.g. `7`.
    Stored(i64),
    /// The token of the n-th file of `admin_token_files` (from 1): `file:<n>`.
    File(usize),
}

impl fmt::Display for AcceptedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stored(id) => write!(f, "{id}"),
            Self::File(position) => write!(f, "file:{position}"),
        }
    }
}

pub(super) async fn require_admin_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let method = method_name(request.method());
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |path| path.as_str().to_owned());
    let mut audit = Audit::default();
    let response = check(&state, request, next, &mut audit).await;
    tracing::info!(
        target: AUDIT,
        token_id = audit.token.map(tracing::field::display),
        %method,
        %route,
        status = response.status().as_u16(),
        client_ip = audit.client.map(tracing::field::display),
        "admin request"
    );
    response
}

async fn check(state: &AppState, request: Request, next: Next, audit: &mut Audit) -> Response {
    let now = state.clock.now();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip());
    // A request without a known peer (the router served without connect
    // info; `serve` always provides it) is not limited.
    let mut reservation = None;
    if let Some(peer) = peer {
        let (trusted, nat64) = (&state.config.trusted_proxies, &state.config.nat64_prefixes);
        let Ok(client) = input::client_ip(peer, request.headers(), trusted, nat64) else {
            let body = rate_limit::json_bad_forwarded_for();
            return rate_limit::refuse(body, StatusCode::BAD_REQUEST, None);
        };
        audit.client = Some(client);
        let budget = Budget::admin_auth(&state.config.rate_limit);
        match state.failed_admin_auth.reserve(client, budget, now) {
            Ok(reserved) => reservation = Some(reserved),
            Err(retry_after) => {
                let body = rate_limit::json_too_many();
                let status = StatusCode::TOO_MANY_REQUESTS;
                return rate_limit::refuse(body, status, Some(retry_after));
            }
        }
    }
    // The Admin Token is the one header not routed through `input`: it is
    // only hashed and compared, never stored or echoed.
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_credentials);
    let accepted = match presented {
        // The files first: they need no database, so they still work when
        // it fails.
        Some(presented) => match state.file_admin_tokens.position(presented) {
            Some(position) => Ok(Some(AcceptedToken::File(position))),
            None => admin_tokens::authenticate(&state.db, presented, now)
                .await
                .map(|id| id.map(AcceptedToken::Stored)),
        },
        None => Ok(None),
    };
    match accepted {
        Ok(Some(token)) => {
            audit.token = Some(token);
            // Given back before the handler runs.
            drop(reservation);
            next.run(request).await
        }
        Ok(None) => {
            if let (Some(reservation), Some(client)) = (reservation, audit.client) {
                let (failures, spent) = reservation.keep();
                warn_of_failures(client, failures, spent, state);
            }
            unauthorized()
        }
        Err(error) => internal_error("checking an Admin Token failed", error),
    }
}

/// The warnings that sum up a client's failed Admin Token checks: one at
/// its first failure in a window, one when it spends the budget.
fn warn_of_failures(client: IpAddr, failures: u32, spent: bool, state: &AppState) {
    let window = humantime::format_duration(state.config.rate_limit.admin_window);
    if spent {
        tracing::warn!(
            target: AUDIT,
            client_ip = %client,
            failures,
            "too many failed Admin Token checks; this client is refused (429) on the \
             admin API until its window of {window} ends"
        );
    } else if failures == 1 {
        tracing::warn!(
            target: AUDIT,
            client_ip = %client,
            "failed Admin Token check; further ones from this client within {window} are \
             only logged when it reaches the limit"
        );
    }
}

/// The method's name for the log: one of the standard methods, or `OTHER`
/// (an extension method's name is the client's choice).
fn method_name(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PATCH => "PATCH",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::CONNECT => "CONNECT",
        Method::TRACE => "TRACE",
        _ => "OTHER",
    }
}

/// The credentials of a `Bearer` authorization: the scheme in any case
/// (RFC 9110), one space, then the token.
fn bearer_credentials(authorization: &str) -> Option<&str> {
    let (scheme, credentials) = authorization.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(credentials)
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}
