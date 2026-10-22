//! `admin_token_files` through the real binary, as an operator sets it: in
//! the TOML file or the environment (`VOTA_ADMIN_TOKEN_FILES`, a list). A
//! missing, unreadable or malformed file stops `vota serve` with an error
//! naming the file but never its contents; with good files the admin API
//! (here through `vota ballot`) accepts their tokens.

#![cfg(feature = "server")]

mod common;

use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;

/// A well-formed Admin Token, as `admin-token generate` prints it.
const FILE_TOKEN: &str = "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1uvY";

/// `vota serve` on a temp database and a free port, which must stop on
/// its own (a startup error) within a few seconds.
fn serve(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .env("VOTA_PORT", "0")
        .arg("serve")
        .timeout(Duration::from_secs(10));
    command
}

/// The environment's form of a list of paths.
fn env_list(paths: &[&Path]) -> String {
    let quoted: Vec<String> = paths
        .iter()
        .map(|path| format!("{:?}", path.to_str().unwrap()))
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// Runs `serve` with `VOTA_ADMIN_TOKEN_FILES` set to these files and
/// returns its stderr, after checking that it failed to start.
fn startup_error(dir: &Path, files: &[&Path]) -> String {
    let output = serve(dir)
        .env("VOTA_ADMIN_TOKEN_FILES", env_list(files))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn a_missing_token_file_stops_startup_naming_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-token");

    let stderr = startup_error(dir.path(), &[&missing]);

    assert!(
        stderr.contains(&format!(
            "admin_token_files: cannot read the Admin Token file {}",
            missing.display()
        )),
        "{stderr}"
    );
    assert!(!stderr.contains("listening on"), "{stderr}");
}

#[test]
fn an_unreadable_token_file_stops_startup_naming_its_path() {
    // A directory: unreadable as a file even for root, unlike mode 000.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("a-folder");
    std::fs::create_dir(&folder).unwrap();

    let stderr = startup_error(dir.path(), &[&folder]);

    assert!(
        stderr.contains(&format!(
            "cannot read the Admin Token file {}",
            folder.display()
        )),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn a_token_file_without_read_permission_stops_startup() {
    use std::os::unix::fs::PermissionsExt as _;
    // Root reads it regardless.
    if rustix::process::geteuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, FILE_TOKEN).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();

    let stderr = startup_error(dir.path(), &[&file]);

    assert!(
        stderr.contains(&format!(
            "cannot read the Admin Token file {}",
            file.display()
        )),
        "{stderr}"
    );
}

#[test]
fn a_malformed_token_file_stops_startup_naming_its_path_but_not_its_contents() {
    let secretish = "vta_SECRETCONTENT0123456789abcdefghijklmnopqrst";
    for contents in [
        // One character short, one too many, a wrong prefix, a non-canonical
        // last character (low bits set).
        &secretish[..46],
        &format!("{secretish}x"),
        &format!("vtb_{}", &FILE_TOKEN[4..]),
        "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1uvZ",
        // Two tokens, inner whitespace, padding, base64 (not url) characters.
        &format!("{FILE_TOKEN}\n{FILE_TOKEN}\n"),
        "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZ pjk7Xx9bt7v1uvY",
        &format!("{FILE_TOKEN}="),
        "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1u+Y",
        "",
        "\n",
        "SECRETCONTENT-but-no-token",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("token");
        std::fs::write(&file, contents).unwrap();

        let stderr = startup_error(dir.path(), &[&file]);

        assert!(
            stderr.contains(&format!(
                "admin_token_files: the Admin Token file {} is malformed",
                file.display()
            )),
            "{contents:?}: {stderr}"
        );
        assert!(
            stderr
                .contains("it must hold exactly one Admin Token: vta_ and 43 base64url characters"),
            "{contents:?}: {stderr}"
        );
        assert!(!stderr.contains("SECRETCONTENT"), "{contents:?}: {stderr}");
        assert!(!stderr.contains(&FILE_TOKEN[4..]), "{contents:?}: {stderr}");
        assert!(
            !stderr.contains("q2bKIu0bSWsRCnMyOt3"),
            "{contents:?}: {stderr}"
        );
    }
}

#[test]
fn a_token_file_that_is_not_text_stops_startup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    let mut contents = FILE_TOKEN.as_bytes().to_vec();
    contents.push(0xff);
    std::fs::write(&file, contents).unwrap();

    let stderr = startup_error(dir.path(), &[&file]);

    assert!(stderr.contains("is malformed"), "{stderr}");
}

#[test]
fn a_token_file_with_more_than_4_kib_stops_startup_even_if_it_starts_with_a_token() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    let padded = format!("{FILE_TOKEN}{}", " ".repeat(4096 - FILE_TOKEN.len()));
    std::fs::write(&file, format!("{padded}SECRETCONTENT")).unwrap();
    let fits = dir.path().join("fits");
    std::fs::write(&fits, &padded).unwrap();

    let stderr = startup_error(dir.path(), &[&file]);

    assert!(stderr.contains("is malformed"), "{stderr}");
    assert!(!stderr.contains("SECRETCONTENT"), "{stderr}");
    // 4 KiB exactly is still read (and trimmed).
    let stderr = startup_error(dir.path(), &[&fits, &file]);
    assert!(
        stderr.contains(&format!(
            "the Admin Token file {} is malformed",
            file.display()
        )),
        "{stderr}"
    );
}

#[test]
fn one_bad_file_among_good_ones_stops_startup_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good");
    let bad = dir.path().join("bad");
    std::fs::write(&good, FILE_TOKEN).unwrap();
    std::fs::write(&bad, "nope").unwrap();

    let stderr = startup_error(dir.path(), &[&good, &bad]);

    assert!(
        stderr.contains(&format!(
            "the Admin Token file {} is malformed",
            bad.display()
        )),
        "{stderr}"
    );
    assert!(!stderr.contains(&good.display().to_string()), "{stderr}");
}

#[test]
fn admin_token_commands_never_read_the_token_files() {
    // Only `serve` uses them: `admin-token list` works with a missing one,
    // e.g. in the NixOS wrapper, which has no credentials.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-token");

    let output = Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.path().join("vota.db"))
        .env("VOTA_ADMIN_TOKEN_FILES", env_list(&[&missing]))
        .args(["admin-token", "list"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}

/// `admin_token_files` as the environment and the TOML file give it.
#[cfg(all(unix, feature = "client"))]
mod accepted {
    use super::*;
    use common::ballot_cli::{client, texts};
    use common::server::Server;

    /// `vota ballot list` against `server` with `token`: its exit code
    /// and stderr.
    fn ballot_list(server: &Server, token: &str) -> (Option<i32>, String) {
        let output = client()
            .env("VOTA_URL", format!("http://{}/", server.addr))
            .env("VOTA_ADMIN_TOKEN", token)
            .args(["ballot", "list"])
            .output()
            .unwrap();
        (output.status.code(), texts(&output).1)
    }

    fn token_file(dir: &Path) -> std::path::PathBuf {
        let file = dir.join("vota-admin-token-1");
        std::fs::write(&file, format!("{FILE_TOKEN}\n")).unwrap();
        file
    }

    #[test]
    fn a_token_file_from_the_environment_is_accepted() {
        // Unquoted, as the NixOS module passes it (systemd's `%d`).
        let server = Server::start_with(|command, dir| {
            let file = token_file(dir);
            command.env(
                "VOTA_ADMIN_TOKEN_FILES",
                format!("[{}]", file.to_str().unwrap()),
            );
        });

        assert_eq!(ballot_list(&server, FILE_TOKEN).0, Some(0));
        let wrong = "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1uvA";
        let (code, stderr) = ballot_list(&server, wrong);
        assert_ne!(code, Some(0), "{stderr}");
        assert!(stderr.starts_with("Error: 401 Unauthorized"), "{stderr}");
    }

    #[test]
    fn a_token_file_from_the_config_file_is_accepted() {
        let server = Server::start_with(|command, dir| {
            let file = token_file(dir);
            let config = dir.join("vota.toml");
            std::fs::write(
                &config,
                format!("admin_token_files = [{:?}]\n", file.to_str().unwrap()),
            )
            .unwrap();
            command.arg("--config").arg(config);
        });

        assert_eq!(ballot_list(&server, FILE_TOKEN).0, Some(0));
        server.signal("TERM");
        let (_, log) = server.wait_for_exit();
        assert!(!log.contains(&FILE_TOKEN[4..]), "{log}");
    }
}
