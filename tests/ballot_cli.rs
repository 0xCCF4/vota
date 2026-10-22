//! `vota ballot …`, the admin API client, run as a real binary against a
//! real server on loopback: every verb in both output modes and the field
//! flags.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use common::ballot_cli::{Admin, FakeServer, deadline_in_days, texts, vote};

#[test]
fn create_prints_the_ballot_url_on_stdout_and_a_summary_on_stderr() {
    let admin = Admin::start();
    let deadline = deadline_in_days(7);

    let output = admin
        .ballot()
        .args(["create", "--title", "Lunch on Friday?"])
        .args(["--option", "Pizza", "--option", "Sushi"])
        .args(["--voting-deadline", &deadline])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, stderr) = texts(&output);
    let url = stdout.trim();
    // The server's default `public_base_url`.
    assert!(url.starts_with("http://localhost:8080/b/"), "{stdout}");
    assert_eq!(url.len(), "http://localhost:8080/b/".len() + 22, "{stdout}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stderr.contains("Created Ballot 1"), "{stderr}");
    assert!(stderr.contains("Lunch on Friday?"), "{stderr}");
    assert!(stderr.contains("unanswered"), "{stderr}");
    assert!(stderr.contains(&deadline), "{stderr}");
    assert!(stderr.contains("Purge Date"), "{stderr}");
}

#[test]
fn create_with_json_prints_the_admin_view() {
    let admin = Admin::start();

    let ballot = admin.create(&[]);

    assert_eq!(ballot["id"], 1);
    assert_eq!(ballot["title"], "Lunch on Friday?");
    assert_eq!(ballot["options"][1]["label"], "Sushi");
    assert_eq!(ballot["status"], "unanswered");
}

#[test]
fn get_prints_the_admin_view_as_a_readable_block() {
    let admin = Admin::start();
    let created = admin.create(&["--group", "lunch", "--voter-reference", "alice"]);

    let output = admin.ballot().args(["get", "1"]).output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    let pizza = created["options"][0]["id"].as_str().unwrap();
    for line in [
        "Ballot 1: Lunch on Friday?",
        &format!("URL:             {}", created["url"].as_str().unwrap()),
        "Status:          unanswered",
        "Group:           lunch",
        "Voter Reference: alice",
        "Choice Mode:     single",
        "Submission Mode: final",
        &format!(
            "Voting Deadline: {}",
            created["voting_deadline"].as_str().unwrap()
        ),
        &format!(
            "Purge Date:      {}",
            created["purge_date"].as_str().unwrap()
        ),
        "Theme:           classic",
        "Accent Colour:   -",
        "Description:     -",
        "Options:",
        &format!("  {pizza}  Pizza"),
        "Fallback:        -",
        "Response:        -",
        "Result:          -",
    ] {
        assert!(stdout.lines().any(|l| l == line), "{line:?} in:\n{stdout}");
    }
}

#[test]
fn get_with_json_prints_the_servers_json_pretty_printed() {
    let admin = Admin::start();
    let created = admin.create(&[]);

    let output = admin
        .ballot()
        .args(["get", "1", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    assert!(stdout.starts_with("{\n  \""), "{stdout}");
    let fetched: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(fetched, created);
}

#[test]
fn get_shows_the_response_and_the_result_of_an_answered_ballot() {
    let admin = Admin::start();
    let created = admin.create(&["--choice-mode", "multiple"]);
    let ids: Vec<&str> = (created["options"].as_array().unwrap().iter())
        .map(|option| option["id"].as_str().unwrap())
        .collect();
    vote(&admin, created["token"].as_str().unwrap(), &ids);

    let output = admin.ballot().args(["get", "1"]).output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    let output = admin
        .ballot()
        .args(["get", "1", "--json"])
        .output()
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let answered_at = fetched["response"]["answered_at"].as_str().unwrap();
    assert!(stdout.contains("Status:          answered\n"), "{stdout}");
    assert!(
        stdout.contains(&format!(
            "Response:        Pizza, Sushi (answered {answered_at}, changed {answered_at})\n"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("Result:          Pizza, Sushi (from the Response)\n"),
        "{stdout}"
    );
}

#[test]
fn get_shows_the_fallback_and_its_result_once_it_applied() {
    let admin = Admin::start();
    let soon = (chrono::Utc::now() + chrono::Duration::seconds(2))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    admin.create(&["--voting-deadline", &soon, "--fallback", "Sushi"]);
    std::thread::sleep(std::time::Duration::from_millis(2500));

    let output = admin.ballot().args(["get", "1"]).output().unwrap();

    let (stdout, _) = texts(&output);
    assert!(
        stdout.contains("Status:          auto_fallback\n"),
        "{stdout}"
    );
    assert!(stdout.contains("Fallback:        Sushi\n"), "{stdout}");
    assert!(stdout.contains("Response:        -\n"), "{stdout}");
    assert!(
        stdout.contains("Result:          Sushi (from the Fallback)\n"),
        "{stdout}"
    );
}

#[test]
fn list_prints_a_table_of_ballots_with_their_results() {
    let admin = Admin::start();
    let first = admin.create(&["--group", "lunch", "--voter-reference", "alice"]);
    let second = admin.create(&["--group", "lunch", "--voter-reference", "bob"]);
    vote(
        &admin,
        second["token"].as_str().unwrap(),
        &[second["options"][1]["id"].as_str().unwrap()],
    );

    let output = admin.ballot().arg("list").output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let deadline =
        |ballot: &serde_json::Value| ballot["voting_deadline"].as_str().unwrap().to_owned();
    assert_eq!(
        texts(&output).0,
        format!(
            "ID  STATUS      GROUP  VOTER-REFERENCE  VOTING-DEADLINE       RESULT\n\
             1   unanswered  lunch  alice            {}  -\n\
             2   answered    lunch  bob              {}  Sushi\n",
            deadline(&first),
            deadline(&second)
        )
    );
}

#[test]
fn list_with_json_prints_the_servers_list() {
    let admin = Admin::start();
    let created = admin.create(&[]);

    let output = admin.ballot().args(["list", "--json"]).output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let list: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(list, serde_json::json!({ "ballots": [created] }));
}

#[test]
fn list_filters_by_group_percent_encoded_and_by_status() {
    let admin = Admin::start();
    let odd_group = "a&b c=d+e%20#f?";
    admin.create(&["--group", odd_group]);
    admin.create(&["--group", "a"]);
    let third = admin.create(&["--group", odd_group, "--voter-reference", "x"]);
    vote(
        &admin,
        third["token"].as_str().unwrap(),
        &[third["options"][0]["id"].as_str().unwrap()],
    );
    let ids = |args: &[&str]| -> Vec<i64> {
        let output = admin
            .ballot()
            .args(["list", "--json"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let list: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        (list["ballots"].as_array().unwrap().iter())
            .map(|ballot| ballot["id"].as_i64().unwrap())
            .collect()
    };

    assert_eq!(ids(&["--group", odd_group]), [1, 3]);
    assert_eq!(ids(&["--group", "a"]), [2]);
    assert_eq!(ids(&["--status", "answered"]), [3]);
    assert_eq!(ids(&["--status", "unanswered", "--group", odd_group]), [1]);
    assert_eq!(ids(&["--status", "auto_fallback"]), Vec::<i64>::new());
}

#[test]
fn list_refuses_an_unknown_status_before_sending() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["list", "--status", "closed"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn create_sends_every_field_flag() {
    let logos = tempfile::tempdir().unwrap();
    std::fs::write(logos.path().join("acme.svg"), common::SVG_LOGO).unwrap();
    let logo_folder = logos.path().to_owned();
    let admin = Admin::start_with(move |command, _| {
        command.env("VOTA_LOGO_FOLDER", &logo_folder);
    });
    let deadline = deadline_in_days(7);
    let purge = deadline_in_days(30);

    let ballot = admin.create(&[
        "--title",
        "Team offsite",
        "--description",
        "Pick **every** date.\n\nThanks!",
        "--option",
        "Thu",
        "--option",
        "Fri",
        "--option",
        "Sat",
        "--choice-mode",
        "multiple",
        "--submission-mode",
        "editable",
        "--voting-deadline",
        &deadline,
        "--purge-date",
        &purge,
        "--fallback",
        "Sat",
        "--fallback",
        "Thu",
        "--group",
        "offsite",
        "--voter-reference",
        "alice@example.org",
        "--theme",
        "dark",
        "--accent-color",
        "#E07A1F",
        "--logo",
        "acme",
    ]);

    assert_eq!(ballot["title"], "Team offsite");
    assert_eq!(ballot["description"], "Pick **every** date.\n\nThanks!");
    let labels: Vec<&str> = (ballot["options"].as_array().unwrap().iter())
        .map(|option| option["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Thu", "Fri", "Sat"]);
    assert_eq!(ballot["choice_mode"], "multiple");
    assert_eq!(ballot["submission_mode"], "editable");
    assert_eq!(ballot["voting_deadline"], deadline);
    assert_eq!(ballot["purge_date"], purge);
    // In Option order.
    assert_eq!(ballot["fallback"][0]["label"], "Thu");
    assert_eq!(ballot["fallback"][1]["label"], "Sat");
    assert_eq!(ballot["group"], "offsite");
    assert_eq!(ballot["voter_reference"], "alice@example.org");
    assert_eq!(ballot["theme"], "dark");
    assert_eq!(ballot["accent_color"], "#e07a1f");
    assert_eq!(ballot["logo"], "acme");
}

#[test]
fn edit_sends_only_the_given_fields_and_prints_the_ballot() {
    let admin = Admin::start();
    let created = admin.create(&["--group", "lunch"]);
    let deadline = deadline_in_days(3);
    let purge = deadline_in_days(20);

    let output = admin
        .ballot()
        .args([
            "edit",
            "1",
            "--title",
            "Dinner?",
            "--voting-deadline",
            &deadline,
        ])
        .args(["--purge-date", &purge, "--submission-mode", "editable"])
        .args([
            "--choice-mode",
            "multiple",
            "--description",
            "Bring friends.",
        ])
        .args(["--voter-reference", "bob", "--theme", "minimal"])
        .args(["--accent-color", "#3366cc", "--fallback", "Pizza"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    for line in [
        "Ballot 1: Dinner?",
        "Group:           lunch",
        "Voter Reference: bob",
        "Choice Mode:     multiple",
        "Submission Mode: editable",
        &format!("Voting Deadline: {deadline}"),
        &format!("Purge Date:      {purge}"),
        "Theme:           minimal",
        "Accent Colour:   #3366cc",
        "Description:",
        "  Bring friends.",
        "Fallback:        Pizza",
    ] {
        assert!(stdout.lines().any(|l| l == line), "{line:?} in:\n{stdout}");
    }
    // Options were not given, so they kept their IDs.
    let pizza = created["options"][0]["id"].as_str().unwrap();
    assert!(stdout.contains(&format!("  {pizza}  Pizza\n")), "{stdout}");
}

#[test]
fn edit_with_options_replaces_all_options() {
    let admin = Admin::start();
    let created = admin.create(&[]);

    let output = admin
        .ballot()
        .args([
            "edit", "1", "--json", "--option", "Tacos", "--option", "Ramen",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let edited: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let labels: Vec<&str> = (edited["options"].as_array().unwrap().iter())
        .map(|option| option["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Tacos", "Ramen"]);
    assert_ne!(edited["options"][0]["id"], created["options"][0]["id"]);
}

#[test]
fn edit_help_says_that_options_replace_all_options() {
    let output = common::ballot_cli::client()
        .args(["ballot", "edit", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(texts(&output).0.contains("replace all Options"));
}

#[test]
fn each_clear_flag_removes_its_value() {
    let logos = tempfile::tempdir().unwrap();
    std::fs::write(logos.path().join("acme.svg"), common::SVG_LOGO).unwrap();
    let logo_folder = logos.path().to_owned();
    let admin = Admin::start_with(move |command, _| {
        command.env("VOTA_LOGO_FOLDER", &logo_folder);
    });
    admin.create(&[
        "--description",
        "Hi",
        "--group",
        "g",
        "--voter-reference",
        "v",
        "--accent-color",
        "#123456",
        "--logo",
        "acme",
        "--fallback",
        "Pizza",
    ]);

    for (flag, field) in [
        ("--clear-description", "description"),
        ("--clear-group", "group"),
        ("--clear-voter-reference", "voter_reference"),
        ("--clear-accent-color", "accent_color"),
        ("--clear-logo", "logo"),
        ("--clear-fallback", "fallback"),
    ] {
        let output = admin
            .ballot()
            .args(["edit", "1", "--json", flag])
            .output()
            .unwrap();

        assert!(output.status.success(), "{flag}: {output:?}");
        let edited: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(edited[field], serde_json::Value::Null, "{flag}");
        assert_eq!(edited["title"], "Lunch on Friday?", "{flag}");
    }
}

#[test]
fn a_clear_flag_conflicts_with_its_setter() {
    let admin = Admin::start();
    admin.create(&["--group", "g"]);

    for args in [
        ["--clear-description", "--description", "x"],
        ["--clear-description", "--description-file", "-"],
        ["--clear-group", "--group", "x"],
        ["--clear-voter-reference", "--voter-reference", "x"],
        ["--clear-accent-color", "--accent-color", "#123456"],
        ["--clear-logo", "--logo", "x"],
        ["--clear-fallback", "--fallback", "Pizza"],
    ] {
        let output = admin
            .ballot()
            .args(["edit", "1"])
            .args(args)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(
            texts(&output).1.contains("cannot be used with"),
            "{output:?}"
        );
    }
    let output = admin
        .ballot()
        .args(["get", "1", "--json"])
        .output()
        .unwrap();
    let ballot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ballot["group"], "g");
}

#[test]
fn description_file_dash_reads_the_description_from_stdin() {
    let admin = Admin::start();

    admin.create(&[]);

    let output = admin
        .ballot()
        .args(["edit", "1", "--json", "--description-file", "-"])
        .write_stdin("# Lunch\n\nWhere shall we go?\n")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let edited: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(edited["description"], "# Lunch\n\nWhere shall we go?");
}

#[test]
fn description_and_description_file_conflict() {
    let output = common::ballot_cli::client()
        .args([
            "ballot",
            "create",
            "--description",
            "a",
            "--description-file",
            "-",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn body_sends_a_json_file_unchanged() {
    let admin = Admin::start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ballot.json");
    let deadline = deadline_in_days(5);
    std::fs::write(
        &file,
        format!(
            r#"{{"title": "From a file", "options": [{{"label": "A"}}, {{"label": "B"}}],
               "voting_deadline": "{deadline}", "group": "files"}}"#
        ),
    )
    .unwrap();

    let output = admin
        .ballot()
        .args(["create", "--json", "--body"])
        .arg(&file)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let ballot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ballot["title"], "From a file");
    assert_eq!(ballot["group"], "files");

    let output = admin
        .ballot()
        .args(["edit", "1", "--json", "--body", "-"])
        .write_stdin(r#"{"group": null, "voter_reference": "carol"}"#)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let ballot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ballot["group"], serde_json::Value::Null);
    assert_eq!(ballot["voter_reference"], "carol");
}

#[test]
fn body_must_be_json_and_conflicts_with_every_field_flag() {
    let admin = Admin::start();
    admin.create(&[]);

    let output = admin
        .ballot()
        .args(["create", "--body", "-"])
        .write_stdin("{\"title\": ")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(texts(&output).1.contains("not JSON"), "{output:?}");

    for field in [
        &["--title", "x"][..],
        &["--description", "x"],
        &["--description-file", "-"],
        &["--option", "x"],
        &["--choice-mode", "single"],
        &["--submission-mode", "final"],
        &["--voting-deadline", "x"],
        &["--purge-date", "x"],
        &["--fallback", "x"],
        &["--group", "x"],
        &["--voter-reference", "x"],
        &["--theme", "x"],
        &["--accent-color", "x"],
        &["--logo", "x"],
    ] {
        for command in [&["create"][..], &["edit", "1"]] {
            let output = admin
                .ballot()
                .args(command)
                .args(["--body", "-"])
                .args(field)
                .write_stdin("{}")
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{field:?}: {output:?}");
        }
    }
    let output = admin
        .ballot()
        .args(["edit", "1", "--body", "-", "--clear-group"])
        .write_stdin("{}")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn enum_values_are_checked_by_the_client() {
    for args in [["--choice-mode", "Single"], ["--submission-mode", "locked"]] {
        let output = common::ballot_cli::client()
            .args(["ballot", "create"])
            .args(args)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}

#[test]
fn other_values_go_to_the_server_as_given() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["create", "--title", "T", "--option", "A", "--option", "B"])
        .args(["--voting-deadline", "next friday", "--accent-color", "red"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(5), "{output:?}");
    assert_eq!(
        texts(&output).1,
        "Error: 422 invalid_input: not an RFC 3339 timestamp, or a local date-time in the \
         server's default time zone (YYYY-MM-DDTHH:MM[:SS]) (field: voting_deadline)\n"
    );
}

#[test]
fn a_deadline_without_offset_is_read_by_the_server_in_its_default_time_zone() {
    let admin = Admin::start_with(|command, _| {
        command.env("VOTA_DEFAULT_TIME_ZONE", "Asia/Kolkata");
    });
    // India has no daylight saving time: always +05:30.
    let local = (chrono::Utc::now() + chrono::Duration::days(7))
        .with_timezone(&chrono_tz::Asia::Kolkata)
        .format("%Y-%m-%dT%H:%M")
        .to_string();

    let created = admin.create(&["--voting-deadline", &local]);

    let deadline: chrono::DateTime<chrono::Utc> = created["voting_deadline"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        deadline
            .with_timezone(&chrono_tz::Asia::Kolkata)
            .format("%Y-%m-%dT%H:%M")
            .to_string(),
        local
    );
    assert!(created["voting_deadline"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn extend_moves_the_voting_deadline_and_prints_the_ballot() {
    let admin = Admin::start();
    admin.create(&[]);
    let later = deadline_in_days(14);

    let output = admin
        .ballot()
        .args(["extend", "1", "--voting-deadline", &later])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    assert!(
        stdout.starts_with("Ballot 1: Lunch on Friday?\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("Voting Deadline: {later}\n")),
        "{stdout}"
    );

    let even_later = deadline_in_days(21);
    let output = admin
        .ballot()
        .args(["extend", "1", "--json", "--voting-deadline", &even_later])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let ballot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ballot["voting_deadline"], even_later);
}

#[test]
fn the_url_may_name_a_sub_path_with_or_without_a_trailing_slash() {
    let admin = Admin::start_with(|command, _| {
        command.env("VOTA_PUBLIC_BASE_URL", "https://example.org/polls/");
    });

    for url in ["polls", "polls/"] {
        let url = format!("http://{}/{url}", admin.server.addr);
        let output = admin
            .ballot()
            .env("VOTA_URL", &url)
            .args(["create", "--title", "T", "--option", "A", "--option", "B"])
            .args(["--voting-deadline", &deadline_in_days(1)])
            .output()
            .unwrap();

        assert!(output.status.success(), "{url}: {output:?}");
        assert!(texts(&output).0.starts_with("https://example.org/polls/b/"));

        let output = admin
            .ballot()
            .args(["--url", &url, "list"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{url}: {output:?}");
    }
    // Without the sub-path, the admin API isn't there.
    let output = admin.ballot().arg("list").output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
}

#[test]
fn readable_output_escapes_control_characters_from_the_server() {
    let hostile = "red\u{1b}[31m\u{202e}\u{e0041}\nline";
    let option = serde_json::json!({ "id": "abc\u{7}", "label": hostile });
    let ballot = serde_json::json!({
        "id": 1, "title": hostile, "url": hostile, "status": hostile,
        "group": hostile, "voter_reference": hostile, "choice_mode": "single",
        "submission_mode": "final", "voting_deadline": hostile, "purge_date": hostile,
        "created_at": hostile, "theme": hostile, "accent_color": hostile, "logo": hostile,
        "description": hostile, "options": [option], "fallback": [option],
        "response": { "option_ids": ["abc\u{7}"], "answered_at": hostile, "changed_at": hostile },
        "result": { "source": hostile, "option_ids": ["abc\u{7}"], "options": [option] },
        "token": hostile,
    });
    let escaped = "red\\u{1b}[31m\\u{202e}\\u{e0041}\\u{a}line";
    let cases: [(&str, serde_json::Value); 3] = [
        ("get", ballot.clone()),
        ("list", serde_json::json!({ "ballots": [ballot.clone()] })),
        ("create", ballot.clone()),
    ];
    for (command, answer) in cases {
        let server = FakeServer::json(&answer);
        let mut args = vec![command];
        if command == "get" {
            args.push("1");
        }

        let output = common::ballot_cli::client()
            .env("VOTA_URL", server.url())
            .env("VOTA_ADMIN_TOKEN", "vta_x")
            .arg("ballot")
            .args(&args)
            .output()
            .unwrap();

        assert!(output.status.success(), "{command}: {output:?}");
        let (stdout, stderr) = texts(&output);
        let shown = stdout + &stderr;
        assert!(shown.contains(escaped), "{command}:\n{shown}");
        assert!(!shown.contains('\u{1b}'), "{command}:\n{shown}");
        assert!(!shown.contains('\u{202e}'), "{command}:\n{shown}");
        assert!(!shown.contains('\u{7}'), "{command}:\n{shown}");
        assert!(!shown.contains('\u{e0041}'), "{command}:\n{shown}");

        // The JSON output escapes them too, as the same JSON value.
        let server = FakeServer::json(&answer);
        let output = common::ballot_cli::client()
            .env("VOTA_URL", server.url())
            .env("VOTA_ADMIN_TOKEN", "vta_x")
            .arg("ballot")
            .args(&args)
            .arg("--json")
            .output()
            .unwrap();
        let stdout = texts(&output).0;
        assert!(
            !stdout.contains('\u{1b}') && !stdout.contains('\u{202e}'),
            "{stdout}"
        );
        assert!(!stdout.contains('\u{e0041}'), "{stdout}");
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(parsed, answer);
    }
}

#[test]
fn allow_no_option_is_set_shown_and_its_empty_answer_told_apart_from_none() {
    let admin = Admin::start();
    let created = admin.create(&["--choice-mode", "multiple", "--allow-no-option"]);
    assert_eq!(created["allow_no_option"], true, "{created}");
    let unanswered = admin.create(&["--choice-mode", "multiple"]);
    assert_eq!(unanswered["allow_no_option"], false, "{unanswered}");
    vote(&admin, created["token"].as_str().unwrap(), &[]);

    let output = admin.ballot().args(["get", "1"]).output().unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, _) = texts(&output);
    assert!(stdout.contains("Allow No Option: yes\n"), "{stdout}");
    assert!(
        stdout.contains("Response:        (none) (answered "),
        "{stdout}"
    );
    assert!(
        stdout.contains("Result:          (none) (from the Response)\n"),
        "{stdout}"
    );
    let (stdout, _) = texts(&admin.ballot().args(["get", "2"]).output().unwrap());
    assert!(stdout.contains("Allow No Option: no\n"), "{stdout}");
    assert!(stdout.contains("Response:        -\n"), "{stdout}");

    let output = admin.ballot().arg("list").output().unwrap();
    let (stdout, _) = texts(&output);
    let rows: Vec<&str> = stdout.lines().collect();
    assert!(rows[1].ends_with("  (none)"), "{stdout}");
    assert!(rows[2].ends_with("  -"), "{stdout}");
}

#[test]
fn edit_turns_allow_no_option_on_and_off() {
    let admin = Admin::start();
    admin.create(&["--choice-mode", "multiple"]);
    let edit = |flag: &str| {
        let output = admin
            .ballot()
            .args(["edit", "1", flag, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["allow_no_option"]
            .clone()
    };

    assert_eq!(edit("--allow-no-option"), true);
    assert_eq!(edit("--allow-no-option=false"), false);
    assert_eq!(edit("--allow-no-option=true"), true);
}

#[test]
fn allow_no_option_takes_only_true_or_false() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["edit", "1", "--allow-no-option=yes"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}
