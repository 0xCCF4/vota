//! Theme stylesheets and logos are served from our own host under fixed
//! routes, by registry name only, with safe headers. Unknown or malformed
//! asset names are a plain 404 that doesn't count toward the rate limit.

#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use common::{PNG_LOGO, SVG_LOGO, TestApp, WEBP_LOGO, write_file};
use http_body_util::BodyExt;

async fn app_with_logos() -> TestApp {
    TestApp::with_config(|config| {
        let folder = config.logo_folder.clone().unwrap();
        write_file(&folder, "club.svg", SVG_LOGO);
        write_file(&folder, "badge.png", PNG_LOGO);
        write_file(&folder, "seal.webp", WEBP_LOGO);
    })
    .await
}

#[tokio::test]
async fn logos_are_served_with_their_content_type_and_safe_headers() {
    let app = app_with_logos().await;

    for (name, content_type, bytes) in [
        ("club", "image/svg+xml", SVG_LOGO.as_bytes()),
        ("badge", "image/png", PNG_LOGO),
        ("seal", "image/webp", WEBP_LOGO),
    ] {
        let response = app.get(&format!("/assets/logos/{name}")).await;

        assert_eq!(response.status(), StatusCode::OK, "{name}");
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{name}");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{name}");
        assert!(
            headers[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("max-age="),
            "{name}"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], bytes, "{name}");
    }
}

#[tokio::test]
async fn svg_logos_are_served_with_a_sandboxed_policy_that_forbids_scripts_and_requests() {
    let app = app_with_logos().await;

    let response = app.get("/assets/logos/club").await;

    // `sandbox`: an SVG (or HTML named `.svg`) opened directly is also an
    // opaque origin without scripts, forms or popups.
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'; style-src 'unsafe-inline'; sandbox"
    );
}

/// Asset paths that must not resolve: unknown names, traversal (plain and
/// percent-encoded), uppercase, extensions, overlong, empty, extra segments,
/// percent-encoded variants of real names.
fn hostile_asset_paths() -> Vec<String> {
    let mut paths: Vec<String> = [
        "/assets/logos/nope",
        "/assets/logos/CLUB",
        "/assets/logos/Club",
        "/assets/logos/club.svg",
        "/assets/logos/club%2Esvg",
        "/assets/logos/%63lub",
        "/assets/logos/club%00",
        "/assets/logos/club%20",
        "/assets/logos/..",
        "/assets/logos/..%2F..%2FCargo.toml",
        "/assets/logos/%2e%2e%2f%2e%2e%2fCargo.toml",
        "/assets/logos/..%5C..%5CCargo.toml",
        "/assets/logos/%252e%252e",
        "/assets/logos/../../Cargo.toml",
        "/assets/logos/",
        "/assets/logos",
        "/assets/logos/club/extra",
        "/assets/logos/-club",
        "/assets/themes/nope/style.css",
        "/assets/themes/CLASSIC/style.css",
        "/assets/themes/%63lassic/style.css",
        "/assets/themes/../style.css",
        "/assets/themes/%2e%2e/style.css",
        "/assets/themes/..%2F..%2Fsrc/style.css",
        "/assets/themes/classic/../../Cargo.toml",
        "/assets/themes/classic/base.html",
        "/assets/themes/classic/templates/base.html",
        "/assets/themes/classic/style.css/extra",
        "/assets/themes/classic/STYLE.CSS",
        "/assets/themes//style.css",
        "/assets/",
        "/assets",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    paths.push(format!("/assets/logos/{}", "a".repeat(65)));
    paths.push(format!("/assets/themes/{}/style.css", "a".repeat(65)));
    paths
}

#[tokio::test]
async fn unknown_and_malformed_asset_paths_are_not_found() {
    let app = app_with_logos().await;

    for path in hostile_asset_paths() {
        let response = app.get(&path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn unknown_assets_do_not_count_toward_the_ballot_token_rate_limit() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 2.try_into().unwrap();
        let folder = config.logo_folder.clone().unwrap();
        write_file(&folder, "club.svg", SVG_LOGO);
    })
    .await;
    let (token, _) = app.create_standard_ballot().await;

    for path in hostile_asset_paths() {
        let response = app.get_from("192.0.2.7", None, &path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }

    let page = app
        .get_from("192.0.2.7", None, &format!("/b/{token}"))
        .await;
    assert_eq!(page.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_throttled_client_still_gets_assets() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
    })
    .await;
    let unknown = app
        .get_from("192.0.2.8", None, "/b/AAAAAAAAAAAAAAAAAAAAAA")
        .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let blocked = app
        .get_from("192.0.2.8", None, "/b/AAAAAAAAAAAAAAAAAAAAAA")
        .await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);

    let css = app
        .get_from("192.0.2.8", None, "/assets/themes/classic/style.css")
        .await;

    assert_eq!(css.status(), StatusCode::OK);
}
