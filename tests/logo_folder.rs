//! The logo folder: `<logo_folder>/<name>.{svg,png,webp}`, loaded at
//! startup. Misnamed files are ignored; ambiguous, oversized or mislabelled
//! logos fail startup with an error naming the file.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{PNG_LOGO, SVG_LOGO, TestApp, WEBP_LOGO, ballot_request, write_file};
use serde_json::json;

async fn start_with_logos(files: Vec<(&'static str, Vec<u8>)>) -> anyhow::Result<TestApp> {
    TestApp::try_with_config(move |config| {
        let folder = config.logo_folder.clone().unwrap();
        for (path, contents) in files {
            write_file(&folder, path, contents);
        }
    })
    .await
}

fn startup_error(started: anyhow::Result<TestApp>) -> String {
    match started {
        Ok(_) => panic!("startup should fail"),
        Err(error) => format!("{error:#}"),
    }
}

#[tokio::test]
async fn files_that_are_not_named_like_logos_are_ignored() {
    let app = start_with_logos(vec![
        ("README.txt", b"hello".to_vec()),
        ("Club.svg", SVG_LOGO.into()),
        ("club.SVG", SVG_LOGO.into()),
        ("club.jpg", b"\xff\xd8\xff".to_vec()),
        (".hidden.svg", SVG_LOGO.into()),
        ("my logo.png", PNG_LOGO.into()),
        ("noextension", PNG_LOGO.into()),
        ("sub/inner.svg", SVG_LOGO.into()),
        ("real.png", PNG_LOGO.into()),
    ])
    .await
    .unwrap();
    let admin = app.admin_token().await;

    for name in [
        "Club",
        "club",
        "my logo",
        "noextension",
        "inner",
        "sub",
        "hidden",
    ] {
        let mut request = ballot_request();
        request["logo"] = json!(name);
        let response = app
            .post_json("/admin/ballots", Some(&admin), &request)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{name}"
        );
    }
    let mut request = ballot_request();
    request["logo"] = json!("real");
    assert_eq!(app.create_ballot(&request).await["logo"], "real");
}

#[tokio::test]
async fn two_files_for_one_logo_name_fail_startup_naming_both() {
    let error = startup_error(
        start_with_logos(vec![
            ("club.svg", SVG_LOGO.into()),
            ("club.png", PNG_LOGO.into()),
        ])
        .await,
    );

    assert!(error.contains("club.svg"), "{error}");
    assert!(error.contains("club.png"), "{error}");
}

#[tokio::test]
async fn an_oversized_logo_fails_startup_naming_the_file() {
    let mut big = PNG_LOGO.to_vec();
    big.resize(512 * 1024 + 1, 0);

    let error = startup_error(start_with_logos(vec![("huge.png", big)]).await);

    assert!(error.contains("huge.png"), "{error}");
}

#[tokio::test]
async fn a_logo_of_exactly_the_maximum_size_is_accepted() {
    let mut max = PNG_LOGO.to_vec();
    max.resize(512 * 1024, 0);

    let app = start_with_logos(vec![("max.png", max)]).await.unwrap();

    assert_eq!(app.get("/assets/logos/max").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_logo_whose_contents_do_not_match_its_extension_fails_startup() {
    for (file, contents) in [
        (
            "fake.png",
            b"<html><script>alert(1)</script></html>".to_vec(),
        ),
        ("fake.webp", PNG_LOGO.to_vec()),
        ("fake.svg", WEBP_LOGO.to_vec()),
    ] {
        let error = startup_error(start_with_logos(vec![(file, contents)]).await);

        assert!(error.contains(file), "{error}");
    }
}

#[tokio::test]
async fn a_configured_logo_folder_that_does_not_exist_fails_startup() {
    let started = TestApp::try_with_config(|config| {
        config.logo_folder = Some(config.logo_folder.clone().unwrap().join("missing"));
    })
    .await;

    let error = startup_error(started);
    assert!(error.contains("missing"), "{error}");
}
