//! `vota man <dir>` writes one man page per command and subcommand of the
//! enabled features (`vota.1`, `vota-ballot-create.1`, …), without a config
//! file or database, in every build.

use std::collections::BTreeSet;
use std::path::Path;

use assert_cmd::Command;
use clap::CommandFactory as _;

fn man(dir: &Path) -> std::process::Output {
    Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_CONFIG", "/nonexistent/vota.toml")
        .arg("man")
        .arg(dir)
        .output()
        .unwrap()
}

/// `vota`, `vota-serve`, `vota-ballot-create`, …: one page name per
/// visible command, from the CLI's own definition.
fn page_names(command: &clap::Command, prefix: &str, names: &mut BTreeSet<String>) {
    names.insert(prefix.to_owned());
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        page_names(sub, &format!("{prefix}-{}", sub.get_name()), names);
    }
}

fn written_pages(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .map(|file| file.strip_suffix(".1").expect(&file).to_owned())
        .collect()
}

fn page(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(format!("{name}.1"))).unwrap()
}

#[test]
fn every_command_and_subcommand_gets_a_page() {
    let dir = tempfile::tempdir().unwrap();
    // Created if missing, with its parents.
    let out = dir.path().join("share/man/man1");

    let output = man(&out);

    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
    let mut expected = BTreeSet::new();
    page_names(&vota::cli::Cli::command(), "vota", &mut expected);
    assert_eq!(written_pages(&out), expected);
    for name in ["vota", "vota-completions", "vota-man"] {
        assert!(expected.contains(name), "{name}");
    }
    if cfg!(feature = "server") {
        for name in [
            "vota-serve",
            "vota-admin-token",
            "vota-admin-token-generate",
        ] {
            assert!(expected.contains(name), "{name}");
        }
    }
    if cfg!(feature = "client") {
        for name in ["vota-ballot", "vota-ballot-create", "vota-ballot-extend"] {
            assert!(expected.contains(name), "{name}");
        }
    }
    for name in &expected {
        let page = page(&out, name);
        let title = format!(".TH {} 1", name.to_uppercase());
        assert!(page.contains(&title), "{name}: {page}");
        for section in [
            ".SH NAME",
            ".SH SYNOPSIS",
            ".SH \"EXIT STATUS\"",
            ".SH \"SEE ALSO\"",
        ] {
            assert!(page.contains(section), "{name}: {section}");
        }
    }
}

#[test]
fn a_parent_page_lists_and_links_its_subcommands() {
    let dir = tempfile::tempdir().unwrap();
    assert!(man(dir.path()).status.success());

    let top = page(dir.path(), "vota");

    assert!(top.contains(".SH SUBCOMMANDS"), "{top}");
    assert!(top.contains("vota\\-completions(1)"), "{top}");
    assert!(top.contains("\\fBvota\\-man\\fR(1)"), "{top}");
    let child = page(dir.path(), "vota-man");
    assert!(child.contains("\\fBvota\\fR(1)"), "{child}");
    assert!(child.contains(".SH EXAMPLES"), "{child}");
}

#[cfg(feature = "client")]
#[test]
fn the_ballot_pages_carry_exit_codes_and_environment() {
    let dir = tempfile::tempdir().unwrap();
    assert!(man(dir.path()).status.success());

    for name in ["vota-ballot", "vota-ballot-create", "vota-ballot-delete"] {
        let page = page(dir.path(), name);
        for text in [
            ".SH ENVIRONMENT",
            "VOTA_URL",
            "VOTA_ADMIN_TOKEN",
            "\\fB7\\fR\nNetwork error",
            "\\fB6\\fR\n401 or 429",
        ] {
            assert!(page.contains(text), "{name}: {text}\n{page}");
        }
    }
    let create = page(dir.path(), "vota-ballot-create");
    assert!(create.contains("\\-\\-voting\\-deadline"), "{create}");
    assert!(create.contains(".SH EXAMPLES"), "{create}");
}

/// The relative date forms of `--voting-deadline` and `--purge-date`, with
/// their units (`m` is months there), are on the pages of the commands
/// that take them.
#[cfg(feature = "client")]
#[test]
fn the_ballot_pages_document_the_relative_dates() {
    let dir = tempfile::tempdir().unwrap();
    assert!(man(dir.path()).status.success());

    for (name, flags) in [
        ("vota-ballot-create", &["<DATETIME|DURATION|+DURATION>"][..]),
        ("vota-ballot-edit", &["<DATETIME|DURATION|+DURATION>"][..]),
        ("vota-ballot-extend", &["<DATETIME|DURATION|+DURATION>"][..]),
    ] {
        let page = page(dir.path(), name);
        for text in flags
            .iter()
            .copied()
            .chain(["m (calendar months)", "min (minutes)"])
        {
            assert!(page.contains(text), "{name}: {text}\n{page}");
        }
    }
}

#[cfg(feature = "server")]
#[test]
fn the_server_pages_name_the_config_variables_and_the_nixos_wrapper() {
    let dir = tempfile::tempdir().unwrap();
    assert!(man(dir.path()).status.success());

    for name in ["vota", "vota-serve", "vota-admin-token-create"] {
        let page = page(dir.path(), name);
        for text in ["VOTA_CONFIG", "RUST_LOG", "VOTA_<OPTION>"] {
            assert!(page.contains(text), "{name}: {text}");
        }
    }
    let admin_token = page(dir.path(), "vota-admin-token");
    assert!(admin_token.contains(".SH NOTES"), "{admin_token}");
    assert!(admin_token.contains("services.vota"), "{admin_token}");
}

#[test]
fn an_unwritable_directory_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    std::fs::write(&file, "").unwrap();

    let output = man(&file.join("man1"));

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Error: creating"), "{stderr}");
}

#[test]
fn the_directory_is_required() {
    let output = Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .arg("man")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}
