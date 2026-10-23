//! `vota completions <shell>` prints completions for the whole CLI (the
//! commands of the enabled features), without a config file or database,
//! in every build.

use assert_cmd::Command;
use clap::CommandFactory as _;

const SHELLS: [(&str, &str); 6] = [
    ("bash", "complete -F _vota"),
    ("zsh", "#compdef vota"),
    ("fish", "complete -c vota"),
    ("elvish", "edit:completion:arg-completer[vota]"),
    (
        "powershell",
        "Register-ArgumentCompleter -Native -CommandName 'vota'",
    ),
    ("nushell", "export extern vota"),
];

fn completions(args: &[&str]) -> std::process::Output {
    Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_CONFIG", "/nonexistent/vota.toml")
        .arg("completions")
        .args(args)
        .output()
        .unwrap()
}

/// The names of every visible subcommand, at any depth, from the CLI's
/// own definition: a new command is covered without touching this test.
fn subcommand_names(command: &clap::Command, names: &mut Vec<String>) {
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        names.push(sub.get_name().to_owned());
        subcommand_names(sub, names);
    }
}

#[test]
fn completions_cover_every_command_for_every_shell() {
    let mut names = Vec::new();
    subcommand_names(&vota::cli::Cli::command(), &mut names);
    assert!(names.iter().any(|name| name == "man"), "{names:?}");
    if cfg!(feature = "server") {
        assert!(names.iter().any(|name| name == "generate"), "{names:?}");
    }
    if cfg!(feature = "client") {
        assert!(names.iter().any(|name| name == "extend"), "{names:?}");
    }
    for (shell, marker) in SHELLS {
        let output = completions(&[shell]);

        assert!(output.status.success(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains(marker), "{shell}: {script}");
        for name in &names {
            assert!(script.contains(name.as_str()), "{shell}: {name}");
        }
    }
}

#[test]
fn completions_cover_the_newer_flags() {
    let mut flags = vec![];
    if cfg!(feature = "server") {
        flags.extend(["expires-in", "expires-at", "label"]);
    }
    if cfg!(feature = "client") {
        flags.extend([
            "allow-no-option",
            "voting-deadline",
            "purge-date",
            "token-file",
            "clear-fallback",
        ]);
    }
    for (shell, _) in SHELLS {
        let script = String::from_utf8(completions(&[shell]).stdout).unwrap();
        for flag in &flags {
            assert!(script.contains(flag), "{shell}: {flag}");
        }
    }
}

/// The shells that show help texts show those of the date flags, with
/// their relative forms and units.
#[cfg(feature = "client")]
#[test]
fn completions_describe_the_relative_dates() {
    for (shell, _) in SHELLS.iter().filter(|(shell, _)| *shell != "bash") {
        let script = String::from_utf8(completions(&[shell]).stdout).unwrap();
        for text in ["calendar months", "+1d"] {
            assert!(script.contains(text), "{shell}: {text}");
        }
    }
}

#[cfg(feature = "client")]
#[test]
fn paths_and_urls_are_completed_as_such() {
    let script = String::from_utf8(completions(&["zsh"]).stdout).unwrap();
    let line = |flag: &str| {
        script
            .lines()
            .find(|line| line.contains(&format!("{flag}=[")))
            .unwrap_or_else(|| panic!("{flag}"))
            .to_owned()
    };

    assert!(line("--url").ends_with(":_urls' \\"), "{}", line("--url"));
    for flag in ["--body", "--description-file", "--token-file", "--ca-cert"] {
        assert!(line(flag).ends_with(":_files' \\"), "{}", line(flag));
    }
}

/// The NixOS module's `vota-admin-token` wrapper runs `vota admin-token
/// "$@"`; its completions are those of `admin-token` under its own name.
#[cfg(feature = "server")]
#[test]
fn the_admin_token_wrapper_gets_completions_of_its_own() {
    for (shell, marker) in [
        ("bash", "complete -F _vota__admin__token"),
        ("zsh", "#compdef vota-admin-token"),
        ("fish", "complete -c vota-admin-token"),
    ] {
        let output = completions(&[shell, "--admin-token-wrapper"]);

        assert!(output.status.success(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains(marker), "{shell}: {script}");
        for name in ["create", "generate", "list", "revoke", "expires-in"] {
            assert!(script.contains(name), "{shell}: {name}");
        }
        // The wrapper passes the service's config itself; no other command.
        for absent in ["--config", "ballot", "serve", "completions"] {
            assert!(!script.contains(absent), "{shell}: {absent}");
        }
    }
}

#[cfg(not(feature = "server"))]
#[test]
fn the_admin_token_wrapper_flag_needs_the_server() {
    let output = completions(&["bash", "--admin-token-wrapper"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn other_shells_are_refused() {
    let output = completions(&["tcsh"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn a_reader_that_stops_early_is_no_error() {
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin!("vota"))
        .args(["completions", "bash"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Closed before anything was read, as `| head -0` would.
    drop(child.stdout.take());

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}
