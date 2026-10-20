//! `vota completions <shell>` prints completions for the whole CLI (the
//! commands of the enabled features), without a config file or database,
//! in every build.

use assert_cmd::Command;

fn completions(shell: &str) -> std::process::Output {
    Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_CONFIG", "/nonexistent/vota.toml")
        .args(["completions", shell])
        .output()
        .unwrap()
}

#[test]
fn completions_cover_every_command_for_bash_zsh_and_fish() {
    for (shell, marker) in [
        ("bash", "complete -F _vota"),
        ("zsh", "#compdef vota"),
        ("fish", "complete -c vota"),
    ] {
        let output = completions(shell);

        assert!(output.status.success(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains(marker), "{shell}: {script}");
        let mut commands = vec!["completions"];
        if cfg!(feature = "server") {
            commands.extend(["serve", "admin-token", "revoke"]);
        }
        if cfg!(feature = "client") {
            commands.extend(["ballot", "extend", "voting-deadline", "clear-fallback"]);
        }
        for command in commands {
            assert!(script.contains(command), "{shell}: {command}");
        }
    }
}

#[test]
fn other_shells_are_refused() {
    let output = completions("powershell");

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
