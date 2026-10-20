//! `vota ballot delete`: asks in a terminal, refuses without one unless
//! `--yes` is given.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use std::process::Command;

use common::ballot_cli::{Admin, run_in_terminal, texts};

/// `vota ballot delete 1` as a plain process, for the pseudo-terminal.
fn delete_command(admin: &Admin) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("vota"));
    command
        .env_clear()
        .env("VOTA_URL", admin.url())
        .env("VOTA_ADMIN_TOKEN", &admin.token)
        .args(["ballot", "delete", "1"]);
    command
}

fn exists(admin: &Admin, id: &str) -> bool {
    let output = admin.ballot().args(["get", id]).output().unwrap();
    match output.status.code() {
        Some(0) => true,
        Some(3) => false,
        _ => panic!("{output:?}"),
    }
}

#[test]
fn delete_in_a_terminal_shows_the_ballot_and_deletes_it_when_confirmed() {
    let admin = Admin::start();
    admin.create(&["--group", "lunch", "--voter-reference", "alice"]);

    let (code, shown) = run_in_terminal(delete_command(&admin), "[y/N]", "y\n");

    assert_eq!(code, 0, "{shown}");
    for text in [
        "Ballot 1: Lunch on Friday?",
        "Group:           lunch",
        "Voter Reference: alice",
        "Status:          unanswered",
        "Delete Ballot 1 \"Lunch on Friday?\"? [y/N] ",
        "Deleted Ballot 1.",
    ] {
        assert!(shown.contains(text), "{text:?} in:\n{shown}");
    }
    assert!(!exists(&admin, "1"));
}

#[test]
fn delete_in_a_terminal_keeps_the_ballot_when_declined() {
    let admin = Admin::start();
    admin.create(&[]);

    for answer in ["n\n", "\n", "yes please\n"] {
        let (code, shown) = run_in_terminal(delete_command(&admin), "[y/N]", answer);

        assert_eq!(code, 1, "{answer:?}: {shown}");
        assert!(shown.contains("Error: Ballot 1 was not deleted"), "{shown}");
        assert!(!shown.contains("Deleted Ballot"), "{shown}");
        assert!(exists(&admin, "1"));
    }
}

#[test]
fn delete_without_a_terminal_is_refused_unless_yes_is_given() {
    let admin = Admin::start();
    admin.create(&[]);

    let output = admin.ballot().args(["delete", "1"]).output().unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let (_, stderr) = texts(&output);
    assert!(stderr.contains("--yes"), "{stderr}");
    assert!(exists(&admin, "1"));
}

#[test]
fn delete_with_yes_deletes_without_asking() {
    let admin = Admin::start();
    admin.create(&[]);

    let output = admin
        .ballot()
        .args(["delete", "1", "--yes"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let (stdout, stderr) = texts(&output);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "Deleted Ballot 1.\n");
    assert!(!exists(&admin, "1"));
}

#[test]
fn delete_with_json_prints_nothing() {
    let admin = Admin::start();
    admin.create(&[]);

    let output = admin
        .ballot()
        .args(["delete", "1", "--yes", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(texts(&output), (String::new(), String::new()));
    assert!(!exists(&admin, "1"));
}

#[test]
fn deleting_an_unknown_ballot_exits_3() {
    let admin = Admin::start();

    let output = admin.ballot().args(["delete", "7", "-y"]).output().unwrap();

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert_eq!(texts(&output).1, "Error: 404 not_found\n");
}
