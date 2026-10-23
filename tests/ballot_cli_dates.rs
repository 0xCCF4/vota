//! `vota ballot`'s relative date flags: a duration from now (`3d`) or `+`
//! a duration after the Voting Deadline (`+7d`), resolved by the client to
//! a UTC timestamp before sending, against a real server.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use chrono::{DateTime, Datelike as _, Duration, NaiveDate, SubsecRound as _, Utc};
use common::ballot_cli::{Admin, texts};

/// A timestamp from the server's answer.
fn instant(value: &serde_json::Value) -> DateTime<Utc> {
    value.as_str().unwrap().parse().unwrap()
}

#[test]
fn create_with_a_duration_sends_the_deadline_that_far_from_now() {
    let admin = Admin::start();
    let before = Utc::now().trunc_subsecs(0);

    let output = admin
        .ballot()
        .args([
            "create", "--json", "--title", "T", "--option", "A", "--option", "B",
        ])
        .args(["--voting-deadline", "3d"])
        .output()
        .unwrap();

    let after = Utc::now();
    assert!(output.status.success(), "{output:?}");
    let (stdout, stderr) = texts(&output);
    let ballot: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let deadline = instant(&ballot["voting_deadline"]);
    assert!(deadline >= before + Duration::days(3), "{deadline}");
    assert!(deadline <= after + Duration::days(3), "{deadline}");
    assert_eq!(deadline, deadline.trunc_subsecs(0));
    assert_eq!(
        stderr,
        format!(
            "Voting Deadline: {} (in 3d)\n",
            ballot["voting_deadline"].as_str().unwrap()
        )
    );
}

#[test]
fn a_plus_purge_date_is_after_the_deadline_in_the_same_command() {
    let admin = Admin::start();

    let ballot = admin.create(&["--voting-deadline", "3d", "--purge-date", "+7d"]);

    assert_eq!(
        instant(&ballot["purge_date"]) - instant(&ballot["voting_deadline"]),
        Duration::days(7)
    );
}

#[test]
fn edit_with_a_plus_purge_date_and_no_deadline_uses_the_ballots_current_deadline() {
    // The next 31 January ahead; the horizon is widened so that a month
    // after it is allowed whatever the date today.
    let admin = Admin::start_with(|command, _| {
        command.env("VOTA_MAX_DATE_HORIZON", "2years");
    });
    let year = Utc::now().year() + 1;
    let deadline = format!("{year}-01-31T12:00:00.250Z");
    let created = admin.create(&["--voting-deadline", &deadline]);

    let output = admin
        .ballot()
        .args(["edit", "1", "--json", "--purge-date", "+1m2h"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, stderr) = texts(&output);
    let ballot: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(ballot["voting_deadline"], created["voting_deadline"]);
    // One calendar month: February has no 31st, so its last day.
    let february_end = if NaiveDate::from_ymd_opt(year, 2, 29).is_some() {
        29
    } else {
        28
    };
    let purge_date = format!("{year}-02-{february_end}T14:00:00.250Z");
    assert_eq!(ballot["purge_date"], purge_date);
    assert_eq!(
        stderr,
        format!("Purge Date: {purge_date} (1m2h after the Voting Deadline)\n")
    );
}

#[test]
fn extend_with_plus_moves_the_deadline_from_the_current_one() {
    let admin = Admin::start();
    let created = admin.create(&[]);

    let output = admin
        .ballot()
        .args(["extend", "1", "--voting-deadline", "+1d"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let expected = instant(&created["voting_deadline"]) + Duration::days(1);
    let expected = expected.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
    let (stdout, stderr) = texts(&output);
    assert!(
        stdout.contains(&format!("Voting Deadline: {expected}\n")),
        "{stdout}"
    );
    assert_eq!(
        stderr,
        format!("Voting Deadline: {expected} (1d after the current Voting Deadline)\n")
    );
}

#[test]
fn edit_with_plus_on_both_dates_chains_them() {
    let admin = Admin::start();
    let created = admin.create(&[]);

    let output = admin
        .ballot()
        .args(["edit", "1", "--json", "--voting-deadline", "+1d"])
        .args(["--purge-date", "+2w"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let ballot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let deadline = instant(&ballot["voting_deadline"]);
    assert_eq!(
        deadline,
        instant(&created["voting_deadline"]) + Duration::days(1)
    );
    assert_eq!(
        instant(&ballot["purge_date"]),
        deadline + Duration::weeks(2)
    );
}

#[test]
fn m_is_months_and_min_is_minutes() {
    let admin = Admin::start();
    let before = Utc::now().trunc_subsecs(0);

    let ballot = admin.create(&["--voting-deadline", "90min", "--purge-date", "+1m"]);

    let deadline = instant(&ballot["voting_deadline"]);
    assert!(deadline >= before + Duration::minutes(90), "{deadline}");
    assert!(deadline <= Utc::now() + Duration::minutes(90), "{deadline}");
    let purge_date = instant(&ballot["purge_date"]);
    assert!(
        purge_date - deadline >= Duration::days(28),
        "{deadline} {purge_date}"
    );
}

/// Runs `vota ballot` with these arguments; asserts a usage error with
/// nothing created, and returns stderr.
fn refused(admin: &Admin, args: &[&str]) -> String {
    let output = admin.ballot().args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    let list = admin.ballot().args(["list", "--json"]).output().unwrap();
    let list: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(list["ballots"].as_array().unwrap().is_empty(), "{list}");
    texts(&output).1
}

#[test]
fn plus_on_create_is_refused_since_there_is_no_deadline_yet() {
    let admin = Admin::start();
    let fields = ["create", "--title", "T", "--option", "A", "--option", "B"];

    let deadline = refused(
        &admin,
        &[&fields[..], &["--voting-deadline", "+1d"]].concat(),
    );
    let purge_date = refused(&admin, &[&fields[..], &["--purge-date", "+7d"]].concat());

    assert!(
        deadline.starts_with("Error: --voting-deadline +1d: + is relative to"),
        "{deadline}"
    );
    assert!(
        purge_date.starts_with("Error: --purge-date +7d: + is relative to"),
        "{purge_date}"
    );
}

#[test]
fn a_value_that_is_neither_a_timestamp_nor_a_duration_is_a_usage_error() {
    let admin = Admin::start();

    let stderr = refused(
        &admin,
        &[
            "create",
            "--title",
            "T",
            "--option",
            "A",
            "--voting-deadline",
            "next friday",
        ],
    );

    assert!(
        stderr.starts_with("Error: --voting-deadline next friday: not a timestamp or a duration; "),
        "{stderr}"
    );
    for form in [
        "RFC 3339",
        "2026-12-01T12:00",
        "3d",
        "+7d",
        "m (calendar months)",
        "min (minutes)",
    ] {
        assert!(stderr.contains(form), "{form} in {stderr}");
    }

    let stderr = refused(&admin, &["create", "--voting-deadline", "3M"]);

    assert!(stderr.contains("unknown unit \"M\""), "{stderr}");
}
