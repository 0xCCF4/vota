//! `vota ballot …` against a hostile stand-in server: every text of an
//! answer (JSON fields, error messages, `Location`, `Retry-After`, plain
//! bodies) is full of terminal controls, and no command, in any output
//! mode, lets one of them reach the terminal raw.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use std::process::Command as StdCommand;

use common::ballot_cli::{FakeServer, client, run_in_terminal, texts};
use serde_json::{Value, json};

/// ANSI escapes (CSI and OSC), CR, BEL, a C1 CSI, bidi overrides and
/// isolates, a zero-width space, a tag character, DEL, NUL, a line
/// separator, a newline and a tab.
const HOSTILE: &str = "A\u{1b}[31m\r\u{7}\u{9b}2J\u{1b}]0;pwned\u{7}\u{202e}\u{2066}\u{200b}\u{e0041}\u{7f}\u{0}\u{2028}\n\tZ";

/// The start of [`HOSTILE`] as the readable output shows it: each
/// character written as `\u{…}`, up to the newline.
const SHOWN: &str = "A\\u{1b}[31m\\u{d}\\u{7}\\u{9b}2J\\u{1b}]0;pwned\\u{7}\\u{202e}\\u{2066}\\u{200b}\\u{e0041}\\u{7f}\\u{0}\\u{2028}";

/// Characters that may steer a terminal: controls (C0, DEL, C1), format
/// characters (bidi, zero-width, tags), private use, unassigned code
/// points, and the line and paragraph separators.
fn steers(c: char) -> bool {
    use icu_properties::CodePointMapData;
    use icu_properties::props::GeneralCategory as Gc;
    matches!(
        CodePointMapData::<Gc>::new().get(c),
        Gc::Control
            | Gc::Format
            | Gc::PrivateUse
            | Gc::Unassigned
            | Gc::LineSeparator
            | Gc::ParagraphSeparator
    )
}

/// What the output may hold besides newlines: CSV keeps tabs and line
/// feeds in a quoted cell and ends its records with CRLF, and a
/// pseudo-terminal turns each newline into CRLF.
#[derive(Clone, Copy, PartialEq)]
enum Allowed {
    Newlines,
    Csv,
    Terminal,
}

/// Panics if `text` holds a character that may steer a terminal, other
/// than those `allowed`.
fn assert_safe(what: &str, text: &str, allowed: Allowed) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let fine = match c {
            '\n' => true,
            '\r' => allowed != Allowed::Newlines && chars.peek() == Some(&'\n'),
            '\t' => allowed == Allowed::Csv,
            c => !steers(c),
        };
        assert!(
            fine,
            "{what}: a raw U+{:04X} reached the terminal:\n{text:?}",
            u32::from(c)
        );
    }
}

/// Both streams of `output`, which must be UTF-8 (a lone byte such as
/// 0x9B would be a C1 control to some terminals).
fn assert_output_safe(what: &str, output: &std::process::Output, stdout: Allowed) {
    let (out, err) = texts(output);
    assert_safe(&format!("{what} (stdout)"), &out, stdout);
    assert_safe(&format!("{what} (stderr)"), &err, Allowed::Newlines);
}

fn option(suffix: &str) -> Value {
    json!({ "id": format!("{HOSTILE}{suffix}"), "label": format!("{HOSTILE}{suffix}") })
}

/// The admin view of a Ballot with [`HOSTILE`] in every field.
fn ballot() -> Value {
    json!({
        "id": HOSTILE, "title": HOSTILE, "url": HOSTILE, "token": HOSTILE,
        "status": HOSTILE, "group": HOSTILE, "voter_reference": HOSTILE,
        "choice_mode": HOSTILE, "allow_no_option": HOSTILE,
        "submission_mode": HOSTILE, "voting_deadline": HOSTILE,
        "purge_date": HOSTILE, "created_at": HOSTILE, "theme": HOSTILE,
        "accent_color": HOSTILE, "logo": HOSTILE, "description": HOSTILE,
        "options": [option("1"), option("2")],
        "fallback": [option("1")],
        "response": {
            "option_ids": [format!("{HOSTILE}1")],
            "answered_at": HOSTILE, "changed_at": HOSTILE,
        },
        "result": {
            "source": HOSTILE, "option_ids": [format!("{HOSTILE}2")],
            "options": [option("2")],
        },
    })
}

/// `vota ballot <args>` against `server`.
fn run(server: &FakeServer, args: &[&str]) -> std::process::Output {
    client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .arg("ballot")
        .args(args)
        .output()
        .unwrap()
}

/// Runs `args` against a server answering `answer`: readable, with
/// `--json` (which must print the same JSON value), and with each of
/// `extra` (e.g. `--csv`).
fn assert_every_mode_safe(answer: &Value, args: &[&str], csv: bool) {
    let what = args.join(" ");

    let output = run(&FakeServer::json(answer), args);
    assert!(output.status.success(), "{what}: {output:?}");
    assert_output_safe(&what, &output, Allowed::Newlines);
    let (stdout, stderr) = texts(&output);
    assert!(
        (stdout + &stderr).contains(SHOWN),
        "{what}: the hostile text isn't shown escaped: {output:?}"
    );

    let output = run(&FakeServer::json(answer), &[args, &["--json"]].concat());
    assert!(output.status.success(), "{what} --json: {output:?}");
    assert_output_safe(&format!("{what} --json"), &output, Allowed::Newlines);
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(&printed, answer, "{what} --json: not the same JSON value");

    if csv {
        let output = run(&FakeServer::json(answer), &[args, &["--csv"]].concat());
        assert!(output.status.success(), "{what} --csv: {output:?}");
        assert_output_safe(&format!("{what} --csv"), &output, Allowed::Csv);
        assert!(
            texts(&output).0.contains(SHOWN),
            "{what} --csv: the hostile text isn't shown escaped: {output:?}"
        );
    }
}

// --- Answers of every command ---

#[test]
fn get_shows_no_raw_control_character() {
    assert_every_mode_safe(&ballot(), &["get", "1"], false);
}

#[test]
fn list_shows_no_raw_control_character() {
    let list = json!({ "ballots": [ballot(), ballot()] });

    assert_every_mode_safe(&list, &["list"], true);
}

#[test]
fn create_of_one_ballot_shows_no_raw_control_character() {
    assert_every_mode_safe(
        &ballot(),
        &["create", "--title", "T", "--option", "A", "--option", "B"],
        true,
    );
}

#[test]
fn create_of_many_ballots_shows_no_raw_control_character() {
    let answer = json!({ "ballots": [ballot(), ballot()] });

    assert_every_mode_safe(
        &answer,
        &["create", "--voter-reference", "a", "--voter-reference", "b"],
        true,
    );
}

#[test]
fn edit_shows_no_raw_control_character() {
    assert_every_mode_safe(&ballot(), &["edit", "1", "--title", "T"], false);
}

#[test]
fn extend_shows_no_raw_control_character() {
    assert_every_mode_safe(
        &ballot(),
        &["extend", "1", "--voting-deadline", "2099-01-01T00:00:00Z"],
        false,
    );
}

#[test]
fn a_relative_date_against_a_hostile_current_deadline_shows_no_raw_control_character() {
    for args in [
        ["edit", "1", "--purge-date", "+1d"],
        ["extend", "1", "--voting-deadline", "+1d"],
    ] {
        let output = run(&FakeServer::json(&ballot()), &args);

        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_output_safe(&args.join(" "), &output, Allowed::Newlines);
    }
}

#[test]
fn delete_with_yes_shows_no_raw_control_character() {
    for json in [false, true] {
        let mut args = vec!["delete", "1", "--yes"];
        if json {
            args.push("--json");
        }

        let output = run(&FakeServer::json(&ballot()), &args);

        assert!(output.status.success(), "{output:?}");
        assert_output_safe(&args.join(" "), &output, Allowed::Newlines);
    }
}

#[test]
fn the_delete_prompt_shows_no_raw_control_character() {
    let server = FakeServer::json(&ballot());
    let mut command = StdCommand::new(assert_cmd::cargo::cargo_bin!("vota"));
    command
        .env_clear()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "delete", "1"]);

    let (code, shown) = run_in_terminal(command, "[y/N]", "n\n");

    assert_eq!(code, 1, "{shown}");
    assert_safe("delete prompt", &shown, Allowed::Terminal);
    assert!(shown.contains(SHOWN), "{shown}");
}

#[test]
fn tally_shows_no_raw_control_character_in_any_format() {
    let mut answered = ballot();
    answered["choice_mode"] = json!("multiple");
    answered["allow_no_option"] = json!(true);
    answered["result"]["source"] = json!("response");
    let mut by_fallback = answered.clone();
    by_fallback["result"]["source"] = json!("fallback");
    let list = json!({ "ballots": [answered, by_fallback] });

    for format in [None, Some("--csv"), Some("--json")] {
        let output = run(
            &FakeServer::json(&list),
            &[&["tally"], format.as_slice()].concat(),
        );

        assert!(output.status.success(), "{format:?}: {output:?}");
        let allowed = if format == Some("--csv") {
            Allowed::Csv
        } else {
            Allowed::Newlines
        };
        assert_output_safe(&format!("tally {format:?}"), &output, allowed);
    }
    // The JSON summary names the labels as the same JSON strings.
    let output = run(&FakeServer::json(&list), &["tally", "--json"]);
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    let labels: Vec<&Value> = (summary["options"].as_array().unwrap().iter())
        .map(|row| &row["option"])
        .collect();
    assert!(labels.contains(&&json!(format!("{HOSTILE}2"))), "{summary}");
}

#[test]
fn a_result_of_unknown_source_is_refused_without_a_raw_control_character() {
    let list = json!({ "ballots": [ballot()] });

    let output = run(&FakeServer::json(&list), &["tally"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_output_safe("tally", &output, Allowed::Newlines);
    assert!(
        texts(&output).1.contains("unexpected response"),
        "{output:?}"
    );
}

// --- The Ballot's URL ---

#[test]
fn a_url_is_shown_and_drawn_as_the_parsed_url_with_its_controls_percent_encoded() {
    let mut answer = ballot();
    answer["url"] = json!("https://x.example/b/tok\u{202e}\u{1b}[31m");

    let output = run(&FakeServer::json(&answer), &["get", "1"]);

    assert!(output.status.success(), "{output:?}");
    assert_output_safe("get", &output, Allowed::Newlines);
    assert!(
        texts(&output)
            .0
            .contains("URL:             https://x.example/b/tok%E2%80%AE%1B[31m\n"),
        "{output:?}"
    );
}

#[cfg(feature = "qr")]
#[test]
fn qr_draws_no_raw_control_character() {
    let mut answer = ballot();
    answer["url"] = json!("https://x.example/b/tok\u{202e}\u{1b}[31m");
    for args in [
        &["get", "1", "--qr"][..],
        &["create", "--title", "T", "--qr"],
        &["get", "1", "--qr", "--json"],
    ] {
        let output = run(&FakeServer::json(&answer), args);

        assert!(output.status.success(), "{args:?}: {output:?}");
        assert_output_safe(&args.join(" "), &output, Allowed::Newlines);
        assert!(
            texts(&output)
                .1
                .contains("\nhttps://x.example/b/tok%E2%80%AE%1B[31m\n"),
            "{args:?}: {output:?}"
        );
    }
}

#[cfg(feature = "qr")]
#[test]
fn qr_of_a_ballot_whose_url_is_no_url_is_an_unexpected_answer() {
    let output = run(&FakeServer::json(&ballot()), &["get", "1", "--qr"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_output_safe("get --qr", &output, Allowed::Newlines);
    assert!(
        texts(&output).1.contains("unexpected response"),
        "{output:?}"
    );
}

// --- Error answers ---

/// A raw HTTP response with this status line, extra headers and body.
fn raw(status: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// `args` with and without `--json` against a server answering `response`;
/// both exit with `code`.
fn assert_error_safe(response: &str, args: &[&str], code: i32) -> Vec<std::process::Output> {
    [false, true]
        .into_iter()
        .map(|json| {
            let server = FakeServer::start(Some(response.to_owned()));
            let mut args = args.to_vec();
            if json {
                args.push("--json");
            }
            let output = run(&server, &args);
            assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
            assert_output_safe(&args.join(" "), &output, Allowed::Newlines);
            output
        })
        .collect()
}

#[test]
fn an_error_answer_shows_no_raw_control_character() {
    let body = json!({ "error": HOSTILE, "message": HOSTILE, "field": HOSTILE });
    let response = raw(
        "422 Unprocessable Entity",
        "Content-Type: application/json\r\n",
        &body.to_string(),
    );

    let outputs = assert_error_safe(&response, &["get", "1"], 5);

    assert!(
        texts(&outputs[0])
            .1
            .starts_with(&format!("Error: 422 {SHOWN}")),
        "{:?}",
        outputs[0]
    );
    let printed: Value = serde_json::from_slice(&outputs[1].stdout).unwrap();
    assert_eq!(printed, body);
}

#[test]
fn taken_voter_references_are_named_without_a_raw_control_character() {
    let body = json!({
        "error": "duplicate_voter_reference", "message": HOSTILE,
        "voter_references": [HOSTILE, 7],
    });
    let response = raw(
        "409 Conflict",
        "Content-Type: application/json\r\n",
        &body.to_string(),
    );

    let outputs = assert_error_safe(
        &response,
        &["create", "--voter-reference", "a", "--voter-reference", "b"],
        4,
    );

    let stderr = texts(&outputs[0]).1;
    assert!(stderr.contains(&format!("(taken: {SHOWN}")), "{stderr}");
    assert!(stderr.contains(", 7)"), "{stderr}");
}

#[test]
fn a_plain_text_error_body_shows_no_raw_control_character() {
    let response = raw(
        "400 Bad Request",
        "Content-Type: text/plain; charset=utf-8\r\n",
        HOSTILE,
    );

    let outputs = assert_error_safe(&response, &["list"], 5);

    assert!(texts(&outputs[0]).1.contains(SHOWN), "{:?}", outputs[0]);
    assert!(texts(&outputs[1]).0.contains(SHOWN), "{:?}", outputs[1]);
}

#[test]
fn an_html_error_body_shows_no_raw_control_character() {
    let response = raw(
        "502 Bad Gateway",
        "Content-Type: text/html\r\n",
        &format!("<html>{HOSTILE}</html>"),
    );

    assert_error_safe(&response, &["list"], 1);
}

#[test]
fn retry_after_shows_no_raw_control_character() {
    // A header value can't hold C0 controls, but any byte above 0x7F.
    let response = raw(
        "429 Too Many Requests",
        "Retry-After: soon\u{202e}\u{9b}2J\u{200b}\r\nContent-Type: application/json\r\n",
        "{\"error\":\"too_many_requests\"}",
    );

    let outputs = assert_error_safe(&response, &["list"], 6);

    assert!(
        texts(&outputs[0])
            .1
            .contains("(Retry-After: soon\\u{202e}\\u{9b}2J\\u{200b})"),
        "{:?}",
        outputs[0]
    );
}

#[test]
fn a_redirect_shows_its_location_without_a_raw_control_character() {
    for (location, shown) in [
        (
            "https://evil.example/a\u{202e}\u{9b}b",
            "https://evil.example/a%E2%80%AE%C2%9Bb",
        ),
        ("/relative\u{202e}\u{9b}", "/relative\\u{202e}\\u{9b}"),
    ] {
        let response = raw("302 Found", &format!("Location: {location}\r\n"), "");

        let outputs = assert_error_safe(&response, &["list"], 1);

        let stderr = texts(&outputs[0]).1;
        assert!(
            stderr.contains(&format!("redirected to {shown};")),
            "{stderr}"
        );
    }
}

#[test]
fn a_success_without_json_shows_no_raw_control_character() {
    let response = raw("200 OK", "Content-Type: text/plain\r\n", HOSTILE);

    assert_error_safe(&response, &["get", "1"], 1);
}
