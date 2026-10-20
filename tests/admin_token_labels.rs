//! Admin Token labels (security audit I-5): `admin-token create` holds them
//! to the rules of every other free text, and `admin-token list` never
//! prints a character that could move the cursor, recolour or reorder the
//! operator's terminal, even from a row stored before those rules.

#![cfg(feature = "server")]

mod common;

use std::path::Path;

use assert_cmd::Command;
use common::TestApp;
use vota::data::admin_tokens::NewAdminToken;

fn vota(database: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command.env_clear().env("VOTA_DATABASE_PATH", database);
    command
}

fn list(database: &Path, args: &[&str]) -> String {
    let output = vota(database)
        .args(["admin-token", "list"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn create_refuses_labels_with_control_bidi_or_invisible_characters() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    for label in [
        "red\u{1b}[31m",
        "bell\u{7}",
        "c1\u{9b}31m",
        "line\nbreak",
        "bidi\u{202E}txt.exe",
        "isolate\u{2066}x\u{2069}",
        "zero\u{200B}width",
        "tag\u{E0041}",
        "",
        "   ",
    ] {
        let output = vota(&database)
            .args(["admin-token", "create", "--label", label])
            .output()
            .unwrap();

        assert!(!output.status.success(), "{label:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{label:?}: no token printed");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("--label"), "{label:?}: {stderr}");
        assert!(
            !stderr.contains(label) || label.trim().is_empty(),
            "{stderr}"
        );
    }
    assert_eq!(list(&database, &["--json"]).trim(), "[]");
}

#[tokio::test]
async fn list_escapes_unprintable_characters_of_labels_stored_before_the_rules() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let app = TestApp::with_config(|config| config.database_path = database.clone()).await;
    // Stands in for a row from an older release: the store itself takes
    // any label; only the CLI's `create` holds new ones to the rules.
    app.admin_token_with(NewAdminToken {
        label: Some("red\u{1b}[31m\u{9b}2J\u{202E}gpj.exe\nnext\u{7}".into()),
        expires_at: None,
    })
    .await;

    let table = list(&database, &[]);
    let json = list(&database, &["--json"]);

    let row = table.lines().nth(1).expect("one row");
    assert!(
        row.ends_with(r"red\u{1b}[31m\u{9b}2J\u{202e}gpj.exe\u{a}next\u{7}"),
        "{table}"
    );
    assert_eq!(table.lines().count(), 2, "{table}");
    for raw in ['\u{1b}', '\u{9b}', '\u{202E}', '\u{7}'] {
        assert!(!table.contains(raw), "{table:?}");
        assert!(!json.contains(raw), "{json:?}");
    }
    // The JSON stays exact: escaped as JSON, the same label when parsed.
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        listed[0]["label"],
        "red\u{1b}[31m\u{9b}2J\u{202E}gpj.exe\nnext\u{7}"
    );
}
