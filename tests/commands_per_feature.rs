//! Which commands a build of `vota` has follows its cargo features: `serve`
//! and `admin-token` need `server`, `ballot` needs `client`, and
//! `completions`, `man` and `--version` work in every build. A command whose feature
//! is off doesn't appear in `--help` and is refused as unknown.

use assert_cmd::Command;

fn vota(args: &[&str]) -> std::process::Output {
    Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .args(args)
        // A build that wrongly starts the server must fail, not hang.
        .timeout(std::time::Duration::from_secs(10))
        .output()
        .unwrap()
}

/// The command names listed in the `Commands:` section of `--help`.
fn listed_commands() -> Vec<String> {
    let output = vota(&["--help"]);
    assert!(output.status.success(), "{output:?}");
    let help = String::from_utf8(output.stdout).unwrap();
    help.lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn help_lists_exactly_the_commands_of_the_enabled_features() {
    let mut expected = Vec::new();
    if cfg!(feature = "server") {
        expected.extend(["serve", "admin-token"]);
    }
    if cfg!(feature = "client") {
        expected.push("ballot");
    }
    expected.extend(["completions", "man", "help"]);

    assert_eq!(listed_commands(), expected);
}

#[test]
fn version_works_in_every_build() {
    let output = vota(&["--version"]);

    assert!(output.status.success(), "{output:?}");
    let version = String::from_utf8(output.stdout).unwrap();
    assert_eq!(version, format!("vota {}\n", env!("CARGO_PKG_VERSION")));
}

#[cfg(not(feature = "server"))]
#[test]
fn server_commands_are_unknown_without_the_server_feature() {
    for command in ["serve", "admin-token"] {
        let output = vota(&[command]);

        assert_eq!(output.status.code(), Some(2), "{command}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("unrecognized subcommand"),
            "{command}: {stderr}"
        );
    }
}

#[cfg(not(feature = "server"))]
#[test]
fn without_the_server_feature_no_command_shows_the_help() {
    let output = vota(&[]);

    // Not `serve`, which this build doesn't have: clap's usage error.
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Usage: vota"), "{stderr}");
    assert!(stderr.contains("completions"), "{stderr}");
}

#[cfg(not(feature = "client"))]
#[test]
fn ballot_is_unknown_without_the_client_feature() {
    let output = vota(&["ballot", "list"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unrecognized subcommand"), "{stderr}");
}
