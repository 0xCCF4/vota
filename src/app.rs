//! The application: [`AppState`], the router ([`app`]) and `/healthz`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::SecondsFormat;
use serde_json::json;
use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::config::Config;
use crate::domain::admin_token::FileAdminTokens;
use crate::{api, config, data, http, input, web};

/// Everything request handlers need. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub clock: Arc<dyn Clock>,
    pub config: Arc<Config>,
    pub(crate) themes: Arc<web::themes::Themes>,
    pub(crate) failed_lookups: Arc<http::rate_limit::Failures>,
    pub(crate) failed_admin_auth: Arc<http::rate_limit::Failures>,
    /// The Admin Tokens of `admin_token_files`, as read at startup.
    pub(crate) file_admin_tokens: Arc<FileAdminTokens>,
}

impl AppState {
    /// Opens (creating if needed) the database and applies pending migrations.
    /// Trusted proxies in IPv4-mapped IPv6 form are taken as IPv4 networks.
    /// Loads the Theme registry from the theme and logo folders, which fails
    /// with an error naming the file at fault, and the Admin Token files
    /// (`admin_token_files`), which fails naming the file but not its
    /// contents.
    pub async fn new(mut config: Config, clock: Arc<dyn Clock>) -> anyhow::Result<Self> {
        config.trusted_proxies = (config.trusted_proxies.iter().copied())
            .map(config::canonical_network)
            .collect();
        let themes = web::themes::Themes::load(
            config.theme_folder.as_deref(),
            config.logo_folder.as_deref(),
        )?;
        let file_admin_tokens = api::admin::load_token_files(&config.admin_token_files)?;
        let db = data::db::open(&config.database_path).await?;
        Ok(Self {
            db,
            clock,
            config: Arc::new(config),
            themes: Arc::new(themes),
            failed_lookups: Arc::default(),
            failed_admin_auth: Arc::default(),
            file_admin_tokens: Arc::new(file_admin_tokens),
        })
    }
}

/// The whole application router. Production serves it on a socket; tests drive
/// it in-process. Every route sits under the base path (the path of
/// `public_base_url`, e.g. `/polls/`): the reverse proxy forwards the prefix
/// unchanged. Outside it only `/healthz` answers; anything else there is a
/// plain 404 that doesn't count toward the rate limit.
pub fn app(state: AppState) -> Router {
    let base_path = state.config.public_base_url.base_path().to_owned();
    let routes = Router::new()
        .route("/healthz", get(health))
        // ADR 0001: the admin API sits in its own router, guarded as a whole.
        .merge(api::admin::router(state.clone()))
        // Failed Ballot Token lookups are rate-limited per client IP. Every
        // response about a Ballot, the rate limit's own included, is
        // personal: never stored by a cache.
        .merge(
            api::voter::router()
                .layer(from_fn_with_state(state.clone(), http::rate_limit::api))
                .layer(from_fn(http::no_store::no_store)),
        )
        .merge(
            web::router()
                .layer(from_fn_with_state(state.clone(), http::rate_limit::page))
                .layer(from_fn(http::no_store::no_store)),
        )
        // Assets carry no Ballot Token: not rate-limited.
        .merge(web::assets_router());
    let routes = match base_path.trim_end_matches('/') {
        "" => routes,
        // Outside the prefix only the health check answers (for local
        // probes that don't know the prefix); anything else is a plain 404
        // that the rate limit never sees.
        prefix => Router::new()
            .route("/healthz", get(health))
            .nest(prefix, routes),
    };
    routes
        .fallback(plain_not_found)
        .layer(DefaultBodyLimit::max(input::BODY_LIMIT_BYTES))
        // Outermost: every response, the router's own included.
        .layer(from_fn(http::security_headers::security_headers))
        .with_state(state)
}

/// The plain, empty 404 of a path no route serves (and of a refused health
/// check): no body, and no headers of its own (only the global
/// `X-Content-Type-Options: nosniff` of `http::security_headers`); not
/// counted by the rate limit.
async fn plain_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// Liveness/readiness for local probes: the database answers, and the
/// Clock's "now" (UTC). Only for local, unproxied requests (a loopback TCP
/// peer, no forwarding header); every other request, including one without a
/// known peer, gets the plain 404 of an unknown path.
async fn health(State(state): State<AppState>, request: Request) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip());
    if !input::is_local_unproxied(peer, request.headers()) {
        return plain_not_found().await.into_response();
    }
    if data::db::ping(&state.db).await.is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let now = state.clock.now().to_rfc3339_opts(SecondsFormat::Secs, true);
    Json(json!({ "status": "ok", "now": now })).into_response()
}
