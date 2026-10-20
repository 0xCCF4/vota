//! `public_base_url` is checked at startup: its path becomes the base path
//! every route is served under, so a URL that can't be one stops every
//! command with an error naming the option. Driven through the real binary,
//! as an operator would.

#![cfg(feature = "server")]

use std::path::Path;

use assert_cmd::Command;

/// `vota admin-token list` (any command loads the config) on a temp
/// database, with `public_base_url` set to `url`.
fn vota_with_base_url(dir: &Path, url: &str) -> std::process::Output {
    Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .env("VOTA_PUBLIC_BASE_URL", url)
        .args(["admin-token", "list"])
        .output()
        .unwrap()
}

/// Asserts startup fails, naming the option and `reason`.
fn assert_refused(url: &str, reason: &str) {
    let dir = tempfile::tempdir().unwrap();
    let output = vota_with_base_url(dir.path(), url);
    assert!(!output.status.success(), "{url}: {output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("PUBLIC_BASE_URL"), "{url}: {stderr}");
    assert!(stderr.contains(reason), "{url}: {stderr}");
}

#[test]
fn a_base_url_with_a_query_is_refused() {
    assert_refused("https://example.org/polls/?x=1", "query");
}

#[test]
fn a_base_url_with_a_fragment_is_refused() {
    assert_refused("https://example.org/polls/#top", "fragment");
}

#[test]
fn a_base_path_with_characters_outside_letters_digits_and_dash_dot_underscore_tilde_is_refused() {
    for url in [
        "https://example.org/my polls/",
        "https://example.org/umfr%C3%A4ge/",
        "https://example.org/{token}/",
        "https://example.org/a:b/",
        "https://example.org/polls//2026/",
    ] {
        assert_refused(url, "base path");
    }
}

#[test]
fn a_base_path_of_several_segments_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let output = vota_with_base_url(dir.path(), "https://example.org/org-1/polls_2026.v~2");
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn a_base_url_that_is_not_http_or_https_is_refused() {
    for url in ["ftp://example.org/", "mailto:polls@example.org"] {
        assert_refused(url, "http");
    }
}
