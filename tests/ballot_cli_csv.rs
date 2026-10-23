//! `vota ballot list --csv`: the list as CSV, run as a real binary against
//! a real server (and a stand-in for values the server would refuse).
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use common::ballot_cli::{Admin, FakeServer, texts, vote};

#[test]
fn list_csv_has_one_record_per_ballot_with_its_result_and_no_urls_or_tokens() {
    let admin = Admin::start();
    let open = admin.create(&["--group", "lunch", "--voter-reference", "alice"]);
    let answered = admin.create(&[
        "--group",
        "lunch",
        "--voter-reference",
        "bob",
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
    ]);
    let ids: Vec<&str> = (answered["options"].as_array().unwrap().iter())
        .map(|option| option["id"].as_str().unwrap())
        .collect();
    vote(
        &admin,
        answered["token"].as_str().unwrap(),
        &[ids[0], ids[2]],
    );
    let none = admin.create(&["--choice-mode", "multiple", "--allow-no-option"]);
    vote(&admin, none["token"].as_str().unwrap(), &[]);

    let output = admin.ballot().args(["list", "--csv"]).output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    let date = |ballot: &serde_json::Value, key: &str| ballot[key].as_str().unwrap().to_owned();
    assert_eq!(
        stdout,
        format!(
            "id,status,group,voter_reference,voting_deadline,purge_date,choice_mode,submission_mode,result,result_source\r\n\
             1,unanswered,lunch,alice,{},{},single,final,,\r\n\
             2,answered,lunch,bob,{},{},multiple,editable,Pizza; Tacos,answered\r\n\
             3,answered,,,{},{},multiple,final,,answered\r\n",
            date(&open, "voting_deadline"),
            date(&open, "purge_date"),
            date(&answered, "voting_deadline"),
            date(&answered, "purge_date"),
            date(&none, "voting_deadline"),
            date(&none, "purge_date"),
        )
    );
    for ballot in [&open, &answered, &none] {
        assert!(
            !stdout.contains(ballot["token"].as_str().unwrap()),
            "{stdout}"
        );
    }
    assert!(!stdout.contains("http"), "{stdout}");
}

#[test]
fn list_csv_quotes_tricky_values_and_keeps_formulas_text() {
    // Values the server itself would refuse (line breaks, an escape
    // sequence) come from a stand-in.
    let option = |label: &str| serde_json::json!({ "id": "x", "label": label });
    let ballot = serde_json::json!({
        "id": 7, "status": "answered", "group": "Team, \"Nord\"",
        "voter_reference": "=HYPERLINK(\"http://evil.example\",\"click\")",
        "voting_deadline": "2026-12-01T12:00:00Z", "purge_date": "2026-12-31T12:00:00Z",
        "choice_mode": "multiple", "submission_mode": "final",
        "url": "http://localhost:8080/b/secret", "token": "secret",
        "result": {
            "source": "fallback", "option_ids": ["x", "y"],
            "options": [option("Grüße\nzwei Zeilen"), option("-1 \u{1b}[31mred")],
        },
    });
    let server = FakeServer::json(&serde_json::json!({ "ballots": [ballot] }));

    let output = common::ballot_cli::client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "list", "--csv"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    assert_eq!(
        stdout,
        "id,status,group,voter_reference,voting_deadline,purge_date,choice_mode,submission_mode,result,result_source\r\n\
         7,answered,\"Team, \"\"Nord\"\"\",\"'=HYPERLINK(\"\"http://evil.example\"\",\"\"click\"\")\",\
         2026-12-01T12:00:00Z,2026-12-31T12:00:00Z,multiple,final,\
         \"Grüße\nzwei Zeilen; -1 \\u{1b}[31mred\",fallback\r\n"
    );
    assert!(!stdout.contains("secret"), "{stdout}");
}

#[test]
fn csv_and_json_are_refused_together() {
    for command in [&["list"][..], &["tally", "--group", "g"]] {
        let output = common::ballot_cli::client()
            .env("VOTA_URL", "http://127.0.0.1:1/")
            .env("VOTA_ADMIN_TOKEN", "vta_x")
            .arg("ballot")
            .args(command)
            .args(["--csv", "--json"])
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2), "{command:?}: {output:?}");
        let (_, stderr) = texts(&output);
        assert!(stderr.contains("cannot be used with"), "{stderr}");
    }
}
