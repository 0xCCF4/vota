//! `vota ballot create` for many Ballots at once (repeated
//! `--voter-reference`, `--voters FILE`, `--count N`) with its table, CSV
//! and JSON output, and `--qr` (cargo feature `qr`) for a single Ballot's
//! URL as a terminal QR code on stderr.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use common::ballot_cli::{Admin, FakeServer, deadline_in_days, texts};

/// `create` with a title, two Options and a deadline, plus `extra`.
fn create(admin: &Admin, extra: &[&str]) -> assert_cmd::Command {
    let mut command = admin.ballot();
    command
        .args(["create", "--title", "Lunch on Friday?"])
        .args(["--option", "Pizza", "--option", "Sushi"])
        .args(["--voting-deadline", &deadline_in_days(7)])
        .args(extra);
    command
}

/// The Voter References of the Ballots in this Group, from `list --json`.
fn references_in(admin: &Admin, group: &str) -> Vec<String> {
    let output = admin
        .ballot()
        .args(["list", "--json", "--group", group])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let list: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    (list["ballots"].as_array().unwrap().iter())
        .map(|ballot| ballot["voter_reference"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn repeated_voter_reference_creates_one_ballot_each_and_prints_a_table() {
    let admin = Admin::start();

    let output = create(
        &admin,
        &[
            "--group",
            "team",
            "--voter-reference",
            "alice",
            "--voter-reference",
            "bob",
        ],
    )
    .output()
    .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, stderr) = texts(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(header, ["VOTER-REFERENCE", "ID", "URL"], "{stdout}");
    for (line, (reference, id)) in lines[1..].iter().zip([("alice", "1"), ("bob", "2")]) {
        let cells: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(cells[..2], [reference, id], "{stdout}");
        assert!(cells[2].starts_with("http://localhost:8080/b/"), "{stdout}");
    }
    assert!(stderr.contains("Created 2 Ballots"), "{stderr}");
    assert_eq!(references_in(&admin, "team"), ["alice", "bob"]);
}

#[test]
fn a_single_voter_reference_still_creates_one_ballot_printed_as_before() {
    let admin = Admin::start();

    let output = create(&admin, &["--voter-reference", "alice"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, stderr) = texts(&output);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.starts_with("http://localhost:8080/b/"), "{stdout}");
    assert!(stderr.contains("Created Ballot 1"), "{stderr}");
}

#[test]
fn voters_reads_one_reference_per_line_skipping_comments_and_blank_lines() {
    let admin = Admin::start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("voters.txt");
    std::fs::write(
        &file,
        "# Staff, 2030\n  alice@example.org  \n\n\t\nbob@example.org\r\n   # indented comment\ncarol\n",
    )
    .unwrap();

    let output = create(&admin, &["--group", "staff", "--voters"])
        .arg(&file)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        references_in(&admin, "staff"),
        ["alice@example.org", "bob@example.org", "carol"]
    );
}

#[test]
fn voters_dash_reads_stdin_and_one_line_is_still_a_bulk_creation() {
    let admin = Admin::start();

    let output = create(&admin, &["--group", "solo", "--voters", "-"])
        .write_stdin("only\n")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    assert!(stdout.starts_with("VOTER-REFERENCE"), "{stdout}");
    assert_eq!(references_in(&admin, "solo"), ["only"]);
}

#[test]
fn count_creates_numbered_ballots_and_json_prints_the_api_answer() {
    let admin = Admin::start();
    create(&admin, &["--group", "hall", "--voter-reference", "4"])
        .output()
        .unwrap();

    let output = create(&admin, &["--group", "hall", "--count", "3", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let references: Vec<&str> = (answer["ballots"].as_array().unwrap().iter())
        .map(|ballot| ballot["voter_reference"].as_str().unwrap())
        .collect();
    assert_eq!(references, ["5", "6", "7"]);
    assert!(answer["ballots"][0]["url"].is_string());
}

#[test]
fn the_reference_flags_exclude_each_other_and_the_body() {
    let admin = Admin::start();
    for args in [
        &["--voter-reference", "a", "--count", "2"][..],
        &["--voter-reference", "a", "--voters", "-"],
        &["--voters", "-", "--count", "2"],
        &["--voters", "-", "--body", "-"],
        &["--count", "2", "--body", "-"],
        &["--csv", "--json"],
    ] {
        let output = admin
            .ballot()
            .arg("create")
            .args(args)
            .write_stdin("{}")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
    let output = admin
        .ballot()
        .args([
            "edit",
            "1",
            "--voter-reference",
            "a",
            "--voter-reference",
            "b",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(texts(&output).1.contains("--voter-reference"), "{output:?}");
}

#[test]
fn csv_prints_voter_reference_id_and_url_with_formula_protection() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["create", "--title", "Lunch on Friday?"])
        .args(["--option", "Pizza", "--option", "Sushi"])
        .args(["--voting-deadline", &deadline_in_days(7)])
        .args(["--group", "sheet", "--voters", "-", "--csv"])
        .write_stdin("=1+1\n+49 30 1234\n-12\n@SUM(A1)\nplain, \"quoted\"\n")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    let lines: Vec<&str> = stdout.split("\r\n").collect();
    assert_eq!(lines[0], "voter_reference,id,url", "{stdout:?}");
    let expected = [
        ("'=1+1", 1),
        ("'+49 30 1234", 2),
        ("'-12", 3),
        ("'@SUM(A1)", 4),
        ("\"plain, \"\"quoted\"\"\"", 5),
    ];
    for (line, (reference, id)) in lines[1..].iter().zip(expected) {
        let prefix = format!("{reference},{id},http://localhost:8080/b/");
        assert!(line.starts_with(&prefix), "{line:?} in {stdout:?}");
    }
    assert_eq!(lines.len(), 7, "{stdout:?}");
    assert_eq!(lines[6], "", "ends with CRLF: {stdout:?}");
}

#[test]
fn csv_of_a_single_create_is_one_row() {
    let admin = Admin::start();

    let output = create(&admin, &["--voter-reference", "\tx", "--csv"])
        .output()
        .unwrap();

    // The server trims the tab away; the row is the Ballot's.
    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    assert!(
        stdout.starts_with("voter_reference,id,url\r\nx,1,http://"),
        "{stdout:?}"
    );
}

#[test]
fn csv_guards_a_leading_tab_and_escapes_a_carriage_return_from_the_server() {
    let ballot = |reference: &str| serde_json::json!({ "id": 9, "voter_reference": reference, "url": "https://x.example/b/t" });
    let server = FakeServer::json(&serde_json::json!({
        "ballots": [ballot("\tA"), ballot("\rB")]
    }));

    let output = common::ballot_cli::client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "x")
        .args([
            "ballot",
            "create",
            "--voter-reference",
            "a",
            "--voter-reference",
            "b",
        ])
        .arg("--csv")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    // A tab is kept and guarded; a CR is escaped like all server text, so
    // the cell starts with a harmless `\`.
    assert_eq!(
        stdout,
        "voter_reference,id,url\r\n'\tA,9,https://x.example/b/t\r\n\\u{d}B,9,https://x.example/b/t\r\n"
    );
}

#[test]
fn a_clash_is_refused_naming_the_taken_references_and_nothing_is_created() {
    let admin = Admin::start();
    create(&admin, &["--group", "team", "--voter-reference", "bob"])
        .output()
        .unwrap();

    let output = create(
        &admin,
        &[
            "--group",
            "team",
            "--voter-reference",
            "alice",
            "--voter-reference",
            "bob",
        ],
    )
    .output()
    .unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let (_, stderr) = texts(&output);
    assert!(stderr.contains("duplicate_voter_reference"), "{stderr}");
    assert!(stderr.contains("taken: bob"), "{stderr}");
    assert_eq!(references_in(&admin, "team"), ["bob"]);
}

#[cfg(feature = "qr")]
mod qr {
    use super::*;

    /// Whether `text` holds a QR code drawn with Unicode half blocks.
    fn has_qr_code(text: &str) -> bool {
        text.lines()
            .filter(|line| line.chars().any(|c| "▀▄█".contains(c)))
            .count()
            >= 10
    }

    #[test]
    fn qr_on_create_draws_the_url_on_stderr_and_leaves_stdout_unchanged() {
        let admin = Admin::start();

        let output = create(&admin, &["--qr"]).output().unwrap();

        assert!(output.status.success(), "{output:?}");
        let (stdout, stderr) = texts(&output);
        let url = stdout.trim();
        assert_eq!(stdout.lines().count(), 1, "{stdout}");
        assert!(url.starts_with("http://localhost:8080/b/"), "{stdout}");
        assert!(!has_qr_code(&stdout), "{stdout}");
        assert!(has_qr_code(&stderr), "{stderr}");
        let qr_end = stderr.rfind(['▀', '▄', '█']).unwrap();
        let after: Vec<&str> = stderr[qr_end..].lines().skip(1).collect();
        assert!(after.contains(&url), "the URL below the code: {stderr}");
    }

    #[test]
    fn qr_on_get_draws_the_ballots_url() {
        let admin = Admin::start();
        let ballot = admin.create(&[]);
        let url = ballot["url"].as_str().unwrap();

        let output = admin.ballot().args(["get", "1", "--qr"]).output().unwrap();

        assert!(output.status.success(), "{output:?}");
        let (stdout, stderr) = texts(&output);
        assert!(stdout.starts_with("Ballot 1: Lunch on Friday?"), "{stdout}");
        assert!(!has_qr_code(&stdout), "{stdout}");
        assert!(has_qr_code(&stderr), "{stderr}");
        assert!(stderr.lines().any(|line| line == url), "{stderr}");
    }

    #[test]
    fn qr_is_refused_for_a_bulk_create_before_sending() {
        let server = FakeServer::json(&serde_json::json!({}));
        for args in [
            &["--voter-reference", "a", "--voter-reference", "b"][..],
            &["--voters", "-"],
            &["--group", "g", "--count", "2"],
            &["--body", "-"],
        ] {
            let output = common::ballot_cli::client()
                .env("VOTA_URL", server.url())
                .env("VOTA_ADMIN_TOKEN", "x")
                .args(["ballot", "create", "--qr"])
                .args(args)
                .write_stdin(if args[0] == "--body" {
                    r#"{"voter_reference": ["a"]}"#
                } else {
                    "a\n"
                })
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
            assert!(texts(&output).1.contains("--qr"), "{args:?}: {output:?}");
        }
        assert!(server.requests().is_empty());
    }
}
