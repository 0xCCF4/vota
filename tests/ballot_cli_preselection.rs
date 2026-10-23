//! `vota ballot` and the Preselection: `--preselect` on `create` and
//! `edit`, `--clear-preselection`, and the Response's origin in `get`,
//! `list --csv` and `tally`; run as a real binary against a real server.
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

#[test]
fn create_sends_each_preselect_flag_and_get_shows_the_preselected_response() {
    let admin = Admin::start();
    let created = admin.create(&[
        "--choice-mode",
        "multiple",
        "--submission-mode",
        "editable",
        "--option",
        "Pizza",
        "--option",
        "Sushi",
        "--option",
        "Tacos",
        "--preselect",
        "Tacos",
        "--preselect",
        "Pizza",
    ]);
    assert_eq!(
        created["preselection"],
        serde_json::json!([
            { "id": option_id(&created, "Pizza"), "label": "Pizza" },
            { "id": option_id(&created, "Tacos"), "label": "Tacos" },
        ]),
        "{created}"
    );

    let output = admin.ballot().args(["get", "1"]).output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    let at = created["response"]["answered_at"].as_str().unwrap();
    for line in [
        "Status:          answered".to_owned(),
        "Preselection:    Pizza, Tacos".to_owned(),
        format!("Response:        Pizza, Tacos (preselected {at}, not saved by the voter)"),
        "Result:          Pizza, Tacos (from the Preselection)".to_owned(),
    ] {
        assert!(stdout.lines().any(|l| l == line), "{line:?} in:\n{stdout}");
    }

    vote(
        &admin,
        created["token"].as_str().unwrap(),
        &[option_id(&created, "Pizza"), option_id(&created, "Tacos")],
    );
    let (stdout, _) = texts(&admin.ballot().args(["get", "1"]).output().unwrap());
    assert!(
        stdout.contains("Result:          Pizza, Tacos (from the Response)\n"),
        "{stdout}"
    );
    assert!(stdout.contains(" (answered "), "{stdout}");
}

#[test]
fn a_ballot_without_a_preselection_shows_a_dash() {
    let admin = Admin::start();
    admin.create(&[]);

    let (stdout, _) = texts(&admin.ballot().args(["get", "1"]).output().unwrap());

    assert!(stdout.contains("Preselection:    -\n"), "{stdout}");
}

#[test]
fn edit_sets_and_clears_the_preselection() {
    let admin = Admin::start();
    admin.create(&["--submission-mode", "editable"]);

    let output = admin
        .ballot()
        .args(["edit", "1", "--json", "--preselect", "Sushi"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let edited: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(edited["preselection"][0]["label"], "Sushi", "{edited}");
    assert_eq!(edited["response"]["origin"], "preselection", "{edited}");

    let output = admin
        .ballot()
        .args(["edit", "1", "--json", "--clear-preselection"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let edited: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(edited["preselection"], serde_json::Value::Null, "{edited}");
    assert_eq!(edited["status"], "unanswered", "{edited}");
}

#[test]
fn clear_preselection_conflicts_with_preselect() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["edit", "1", "--preselect", "Sushi", "--clear-preselection"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

/// Group `team`: Ballot 1 preselected Pizza (Editable, untouched), 2
/// preselected Pizza but answered Sushi by the voter, 3 preselected
/// (Final) and open.
fn team() -> Admin {
    let admin = Admin::start();
    let editable = ["--group", "team", "--submission-mode", "editable"];
    admin.create(&[&editable[..], &["--preselect", "Pizza"]].concat());
    let changed = admin.create(&[&editable[..], &["--preselect", "Pizza"]].concat());
    vote(
        &admin,
        changed["token"].as_str().unwrap(),
        &[option_id(&changed, "Sushi")],
    );
    admin.create(&["--group", "team", "--preselect", "Sushi"]);
    admin
}

#[test]
fn tally_counts_preselected_results_apart_from_the_voters() {
    let admin = team();

    let output = admin
        .ballot()
        .args(["tally", "--group", "team"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        texts(&output).0,
        "Group: team  Ballots: 3  Answered: 1  Preselected: 1  By Fallback: 0  Open: 1\n\
         \n\
         OPTION  ANSWERED  PRESELECTED  FALLBACK  TOTAL\n\
         Pizza          0            1         0      1\n\
         Sushi          1            0         0      1\n"
    );
    let csv = admin
        .ballot()
        .args(["tally", "--group", "team", "--csv"])
        .output()
        .unwrap();
    assert_eq!(
        texts(&csv).0,
        "option,answered,preselected,fallback,total\r\n\
         Pizza,0,1,0,1\r\n\
         Sushi,1,0,0,1\r\n"
    );
    let json = admin
        .ballot()
        .args(["tally", "--group", "team", "--json"])
        .output()
        .unwrap();
    let tally: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(tally["answered"], 1, "{tally}");
    assert_eq!(tally["preselected"], 1, "{tally}");
    assert_eq!(
        tally["options"][0],
        serde_json::json!({
            "option": "Pizza", "answered": 0, "preselected": 1, "fallback": 0, "total": 1,
        })
    );
}

#[test]
fn list_csv_names_a_preselected_result_by_its_source() {
    let admin = team();

    let output = admin
        .ballot()
        .args(["list", "--group", "team", "--csv"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    let sources: Vec<&str> = (stdout.lines().skip(1))
        .map(|line| line.trim_end().rsplit(',').next().unwrap())
        .collect();
    assert_eq!(sources, ["preselected", "answered", ""], "{stdout}");
}
