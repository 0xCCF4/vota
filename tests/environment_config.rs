//! Unknown `VOTA_*` environment variables are ignored with one warning
//! naming the variable (never its value); unknown keys in the TOML file still
//! stop startup. Driven through the real binary, as an operator would.

#![cfg(feature = "server")]

use std::path::Path;

use assert_cmd::Command;

/// `vota admin-token list` (any command loads the config) on a temp
/// database.
fn vota(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .args(["admin-token", "list"]);
    command
}

#[test]
fn an_unknown_environment_variable_is_ignored_with_one_warning_naming_it() {
    let dir = tempfile::tempdir().unwrap();

    let output = vota(dir.path())
        .env("VOTA_LOG", "s3cr3t-value")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.matches("VOTA_LOG").count(), 1, "{stderr}");
    assert!(stderr.contains("WARN"), "{stderr}");
    assert!(!stderr.contains("s3cr3t-value"), "{stderr}");
}

#[test]
fn an_unknown_key_inside_a_table_is_ignored_with_a_warning_and_the_known_ones_apply() {
    let dir = tempfile::tempdir().unwrap();

    let output = vota(dir.path())
        .env("VOTA_RATE_LIMIT__BURST", "3")
        // Still read: a zero window is refused.
        .env("VOTA_RATE_LIMIT__WINDOW", "0s")
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("VOTA_RATE_LIMIT__BURST"), "{stderr}");
    assert!(stderr.contains("rate_limit.window"), "{stderr}");
    assert!(!stderr.contains("VOTA_RATE_LIMIT__WINDOW"), "{stderr}");
}

#[test]
fn the_config_file_variable_and_known_options_are_not_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(&file, "port = 9000\n").unwrap();

    let output = vota(dir.path())
        .env("VOTA_CONFIG", &file)
        .env("VOTA_PURGE_INTERVAL", "30m")
        .env("VOTA_RATE_LIMIT__FAILED_LOOKUPS", "5")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("WARN"), "{stderr}");
}

#[test]
fn an_unknown_key_in_the_config_file_still_stops_startup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(&file, "log = \"debug\"\n").unwrap();

    let output = vota(dir.path())
        .arg("--config")
        .arg(&file)
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unknown field"), "{stderr}");
}

#[test]
fn logs_written_to_anything_but_a_terminal_carry_no_colour_codes() {
    let dir = tempfile::tempdir().unwrap();

    let output = vota(dir.path()).env("VOTA_LOG", "x").output().unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("WARN"), "{stderr}");
    assert!(!stderr.contains('\u{1b}'), "{stderr:?}");
}

#[test]
fn the_clients_variables_are_not_reported_by_the_server_commands() {
    let dir = tempfile::tempdir().unwrap();

    let output = vota(dir.path())
        .env("VOTA_URL", "https://example.org/polls/")
        .env("VOTA_ADMIN_TOKEN", "vta_secret")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("VOTA_"), "{stderr}");
    assert!(!stderr.contains("vta_secret"), "{stderr}");
}
