//! The database holds every live Ballot Token in plaintext, so its files are
//! readable by the service's user only (mode 0600), whatever the umask the
//! process was started with: `serve` and `admin-token` set umask 077 and
//! restrict the database, its `-wal` and its `-shm` to 0600, also when they
//! existed before with wider modes.
#![cfg(all(unix, feature = "server"))]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use common::server::{Server, with_umask};

/// The database file and its `-wal` and `-shm` companions.
fn database_files(database: &Path) -> [PathBuf; 3] {
    ["", "-wal", "-shm"].map(|suffix| PathBuf::from(format!("{}{suffix}", database.display())))
}

/// The permission bits of each file that exists, by name.
fn modes(database: &Path) -> Vec<(String, u32)> {
    database_files(database)
        .iter()
        .filter_map(|file| {
            let mode = std::fs::metadata(file).ok()?.permissions().mode() & 0o777;
            Some((file.file_name()?.to_string_lossy().into_owned(), mode))
        })
        .collect()
}

fn admin_token_create(database: &Path, mask: u32) -> std::process::Output {
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("vota"));
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", database)
        .args(["admin-token", "create"]);
    with_umask(&mut command, mask);
    command.output().unwrap()
}

#[test]
fn serve_creates_its_database_files_for_its_user_only_despite_a_permissive_umask() {
    let server = Server::start_with(|command, _| with_umask(command, 0o022));

    let modes = modes(&server.database());

    assert_eq!(
        modes,
        [
            ("vota.db".to_owned(), 0o600),
            ("vota.db-wal".to_owned(), 0o600),
            ("vota.db-shm".to_owned(), 0o600),
        ]
    );
}

#[test]
fn serve_restricts_existing_database_files_that_others_could_read() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let created = admin_token_create(&database, 0o022);
    assert!(created.status.success(), "{created:?}");
    for file in database_files(&database) {
        if file.exists() {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }

    let server = Server::start_in(dir, |command, _| with_umask(command, 0o022));

    for (file, mode) in modes(&server.database()) {
        assert_eq!(mode, 0o600, "{file}");
    }
}

#[test]
fn admin_token_create_makes_a_new_database_for_its_user_only() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    let created = admin_token_create(&database, 0o022);

    assert!(created.status.success(), "{created:?}");
    let modes = modes(&database);
    assert!(!modes.is_empty());
    for (file, mode) in modes {
        assert_eq!(mode, 0o600, "{file}");
    }
}
