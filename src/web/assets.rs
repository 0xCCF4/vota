//! Theme stylesheets and logos, served from our own host under fixed routes
//! and looked up by registry name only, and the voting page's script modules
//! and their stylesheet (embedded in the binary, referenced by content
//! version). Not
//! Ballot Token routes: an unknown asset is a plain 404 that doesn't count
//! toward the rate limit.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use untrusted_value::UntrustedValue;

use super::logos::LogoKind;
use crate::AppState;
use crate::input::{self, AppModulePath, AssetNamePath, AssetQuery};

/// How long browsers may cache an asset. Assets change only on a restart.
const CACHE_CONTROL: &str = "public, max-age=3600";

/// How long browsers may cache an app asset requested under its current
/// version: for good, as another content gets another version.
const CACHE_FOREVER: &str = "public, max-age=31536000, immutable";

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/assets/themes/{name}/style.css", get(stylesheet))
        .route("/assets/logos/{name}", get(logo))
        .route(
            &format!("{}/{{version}}/{{file}}", super::enhancement::MODULES_DIR),
            get(script_module),
        )
        .route(super::enhancement::STYLESHEET_PATH, get(script_stylesheet))
}

/// `GET /assets/app/{version}/{file}`: one of the voting page's script
/// modules. Cached for good under the modules' current version (as the
/// pages load them); under any other version, the current module is cached
/// briefly. An unknown file is a plain 404.
async fn script_module(AppModulePath { version, file }: AppModulePath) -> Response {
    let Some(name) = input::app_module(file, &super::enhancement::MODULES) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(content) = super::enhancement::module(name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let current = input::is_current_version_segment(version, &super::enhancement::MODULES_VERSION);
    versioned_asset("text/javascript; charset=utf-8", content, current)
}

/// `GET /assets/app/ballot.css`: the styles of what the script adds to the
/// voting page (its dialog and countdown), built on the Themes' custom
/// properties. Cached like the script.
async fn script_stylesheet(AssetQuery(query): AssetQuery) -> Response {
    app_asset(
        "text/css; charset=utf-8",
        super::enhancement::STYLESHEET,
        &super::enhancement::STYLESHEET_VERSION,
        query,
    )
}

/// An app asset, cached for good if `query` asks for its current `version`.
fn app_asset(
    content_type: &'static str,
    content: &'static str,
    version: &str,
    query: UntrustedValue<String>,
) -> Response {
    versioned_asset(
        content_type,
        content,
        input::is_current_version(query, version),
    )
}

/// An app asset, cached for good if it was asked for under its `current`
/// version.
fn versioned_asset(content_type: &'static str, content: &'static str, current: bool) -> Response {
    let mut response = asset(
        content_type,
        NOTHING_ALLOWED,
        Bytes::from_static(content.as_bytes()),
    );
    if current {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(CACHE_FOREVER),
        );
    }
    response
}

/// `GET /assets/themes/{name}/style.css`: the Theme's stylesheet.
async fn stylesheet(State(state): State<AppState>, AssetNamePath(raw): AssetNamePath) -> Response {
    let Some(name) = input::theme_asset(raw, &*state.themes) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(theme) = state.themes.get(name.as_str()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    asset(
        "text/css; charset=utf-8",
        NOTHING_ALLOWED,
        theme.stylesheet().clone(),
    )
}

/// `GET /assets/logos/{name}`: the logo, by its file's type.
async fn logo(State(state): State<AppState>, AssetNamePath(raw): AssetNamePath) -> Response {
    let Some(name) = input::logo_asset(raw, &*state.themes) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(logo) = state.themes.logo(name.as_str()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let policy = match logo.kind {
        // Pages embed logos only via <img>, where SVG runs no scripts; this
        // also disarms an SVG opened directly: no scripts, no requests.
        LogoKind::Svg => SVG_POLICY,
        LogoKind::Png | LogoKind::Webp => NOTHING_ALLOWED,
    };
    asset(logo.kind.content_type(), policy, logo.bytes.clone())
}

/// Content Security Policy of an asset opened directly: nothing may load.
const NOTHING_ALLOWED: &str = "default-src 'none'";

/// Content Security Policy of an SVG logo: its own inline styles only, and
/// sandboxed, so a file opened directly (even HTML named `.svg`, which the
/// logo check lets through) is an opaque origin without scripts or forms.
const SVG_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

fn asset(content_type: &'static str, policy: &'static str, body: Bytes) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(policy),
    );
    response
}
