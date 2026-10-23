//! `vota ballot tally`: a Group's results summed up by Option label, run
//! as a real binary against a real server.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use common::ballot_cli::{Admin, texts, vote};

/// The id of the Option with this label.
fn option_id<'a>(ballot: &'a serde_json::Value, label: &str) -> &'a str {
    (ballot["options"].as_array().unwrap().iter())
        .find(|option| option["label"] == label)
        .and_then(|option| option["id"].as_str())
        .unwrap()
}

/// Group `offsite`: Ballot 1 auto-fallback (Sushi), 2 answered Sushi
/// (single), 3 answered Pizza and Tacos (multiple), 4 answered with no
/// Option, 5 open with an extra Option Curry nobody chose; Ballot 6, in
/// another Group, answered Pizza.
fn offsite() -> Admin {
    let admin = Admin::start();
    let soon = (chrono::Utc::now() + chrono::Duration::seconds(3))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let started = std::time::Instant::now();
    admin.create(&[
        "--group",
        "offsite",
        "--voting-deadline",
        &soon,
        "--fallback",
        "Sushi",
    ]);
    let single = admin.create(&["--group", "offsite"]);
    vote(
        &admin,
        single["token"].as_str().unwrap(),
        &[option_id(&single, "Sushi")],
    );
    let multiple = admin.create(&[
        "--group",
        "offsite",
        "--choice-mode",
        "multiple",
        "--option",
        "Pizza",
        "--option",
        "Sushi",
        "--option",
        "Tacos",
    ]);
    vote(
        &admin,
        multiple["token"].as_str().unwrap(),
        &[option_id(&multiple, "Pizza"), option_id(&multiple, "Tacos")],
    );
    let none = admin.create(&[
        "--group",
        "offsite",
        "--choice-mode",
        "multiple",
        "--allow-no-option",
    ]);
    vote(&admin, none["token"].as_str().unwrap(), &[]);
    admin.create(&[
        "--group", "offsite", "--option", "Pizza", "--option", "Sushi", "--option", "Curry",
    ]);
    let other = admin.create(&["--group", "elsewhere"]);
    vote(
        &admin,
        other["token"].as_str().unwrap(),
        &[option_id(&other, "Pizza")],
    );
    if let Some(left) = std::time::Duration::from_millis(3500).checked_sub(started.elapsed()) {
        std::thread::sleep(left);
    }
    admin
}

#[test]
fn tally_sums_up_a_groups_results_by_option_label() {
    let admin = offsite();

    let output = admin
        .ballot()
        .args(["tally", "--group", "offsite"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        texts(&output).0,
        "Group: offsite  Ballots: 5  Answered: 3  Preselected: 0  By Fallback: 1  Open: 1\n\
         \n\
         OPTION  ANSWERED  PRESELECTED  FALLBACK  TOTAL\n\
         Sushi          1            0         1      2\n\
         Pizza          1            0         0      1\n\
         Tacos          1            0         0      1\n\
         Curry          0            0         0      0\n\
         (none)         1            0         0      1\n\
         \n\
         Note: on multiple-choice Ballots each chosen Option counts once, \
         so the totals can exceed the number of Ballots.\n"
    );
}

#[test]
fn tally_with_json_prints_the_counts_as_a_documented_structure() {
    let admin = offsite();

    let output = admin
        .ballot()
        .args(["tally", "--group", "offsite", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let tally: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = |option: serde_json::Value, answered: u64, fallback: u64| {
        serde_json::json!({
            "option": option, "answered": answered, "preselected": 0, "fallback": fallback,
            "total": answered + fallback,
        })
    };
    assert_eq!(
        tally,
        serde_json::json!({
            "group": "offsite",
            "ballots": 5, "answered": 3, "preselected": 0, "fallback": 1, "open": 1,
            "options": [
                row("Sushi".into(), 1, 1),
                row("Pizza".into(), 1, 0),
                row("Tacos".into(), 1, 0),
                row("Curry".into(), 0, 0),
                row(serde_json::Value::Null, 1, 0),
            ],
        })
    );
}

#[test]
fn tally_with_csv_prints_one_record_per_option() {
    let admin = offsite();

    let output = admin
        .ballot()
        .args(["tally", "--group", "offsite", "--csv"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        texts(&output).0,
        "option,answered,preselected,fallback,total\r\n\
         Sushi,1,0,1,2\r\n\
         Pizza,1,0,0,1\r\n\
         Tacos,1,0,0,1\r\n\
         Curry,0,0,0,0\r\n\
         ,1,0,0,1\r\n"
    );
}

#[test]
fn tally_without_a_group_covers_all_live_ballots() {
    let admin = Admin::start();
    let first = admin.create(&["--group", "a"]);
    vote(
        &admin,
        first["token"].as_str().unwrap(),
        &[option_id(&first, "Pizza")],
    );
    admin.create(&[]);

    let output = admin.ballot().arg("tally").output().unwrap();

    assert!(output.status.success(), "{output:?}");
    // Only single choice here: no note, and no `(none)` row.
    assert_eq!(
        texts(&output).0,
        "Group: (all live Ballots)  Ballots: 2  Answered: 1  Preselected: 0  By Fallback: 0  \
         Open: 1\n\
         \n\
         OPTION  ANSWERED  PRESELECTED  FALLBACK  TOTAL\n\
         Pizza          1            0         0      1\n\
         Sushi          0            0         0      0\n"
    );
    let output = admin.ballot().args(["tally", "--json"]).output().unwrap();
    let tally: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(tally["group"], serde_json::Value::Null);
}

#[test]
fn tally_of_an_empty_group_shows_zeros() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["tally", "--group", "nobody"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        texts(&output).0,
        "Group: nobody  Ballots: 0  Answered: 0  Preselected: 0  By Fallback: 0  Open: 0\n\
         \n\
         OPTION  ANSWERED  PRESELECTED  FALLBACK  TOTAL\n"
    );
}

#[test]
fn tally_has_the_exit_codes_of_the_other_commands() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["tally", "--token", "vta_wrong"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(6), "{output:?}");
}

#[test]
fn tally_escapes_control_characters_from_the_server() {
    let hostile = "red\u{1b}[31m\u{202e}";
    let option = serde_json::json!({ "id": "x", "label": hostile });
    let list = serde_json::json!({ "ballots": [{
        "id": 1, "choice_mode": "single", "allow_no_option": false, "options": [option],
        "result": { "source": "response", "option_ids": ["x"], "options": [option] },
    }]});
    for format in [None, Some("--csv"), Some("--json")] {
        let server = common::ballot_cli::FakeServer::json(&list);

        let output = common::ballot_cli::client()
            .env("VOTA_URL", server.url())
            .env("VOTA_ADMIN_TOKEN", "vta_x")
            .args(["ballot", "tally", "--group", hostile])
            .args(format)
            .output()
            .unwrap();

        assert!(output.status.success(), "{format:?}: {output:?}");
        let (stdout, _) = texts(&output);
        assert!(!stdout.contains('\u{1b}'), "{format:?}: {stdout}");
        assert!(!stdout.contains('\u{202e}'), "{format:?}: {stdout}");
        assert!(stdout.contains("red\\u"), "{format:?}: {stdout}");
    }
}
