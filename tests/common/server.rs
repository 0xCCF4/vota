//! `vota serve` as a real process on a free port and a temp database,
//! for what only the binary shows: signals, the listener's connection
//! handling, file modes.
#![cfg(unix)]

use std::io::{BufRead as _, BufReader};
use std::net::SocketAddr;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A running `vota serve` and its stderr lines as they arrive.
pub struct Server {
    child: Child,
    stderr: mpsc::Receiver<String>,
    pub log: Vec<String>,
    /// Where it listens.
    pub addr: SocketAddr,
    dir: tempfile::TempDir,
}

impl Server {
    /// Starts the server and waits until it logs `listening on …`.
    pub fn start() -> Self {
        Self::start_with(|_, _| {})
    }

    /// Like [`Self::start`], after letting the test adjust the command (e.g.
    /// add `VOTA_*` variables) and prepare the temp directory, which
    /// holds the database `vota.db`.
    pub fn start_with(adjust: impl FnOnce(&mut Command, &Path)) -> Self {
        Self::start_in(tempfile::tempdir().unwrap(), adjust)
    }

    /// Like [`Self::start_with`], in a given temp directory.
    pub fn start_in(dir: tempfile::TempDir, adjust: impl FnOnce(&mut Command, &Path)) -> Self {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("vota"));
        command
            .arg("serve")
            .env_clear()
            .env("VOTA_DATABASE_PATH", dir.path().join("vota.db"))
            .env("VOTA_PORT", "0")
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        adjust(&mut command, dir.path());
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            stderr: receiver,
            log: Vec::new(),
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            dir,
        };
        let line = server.wait_for_line("listening on");
        let addr = line.rsplit(' ').next().unwrap();
        server.addr = addr.parse().unwrap_or_else(|_| panic!("{line}"));
        server
    }

    /// The temp directory holding the database.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// The database file.
    pub fn database(&self) -> PathBuf {
        self.dir.path().join("vota.db")
    }

    /// Waits (30 seconds at most) for a stderr line containing `needle`;
    /// returns it.
    pub fn wait_for_line(&mut self, needle: &str) -> String {
        while let Ok(line) = self.stderr.recv_timeout(Duration::from_secs(30)) {
            let found = line.contains(needle);
            self.log.push(line.clone());
            if found {
                return line;
            }
        }
        let _ = self.child.kill();
        panic!("no line containing {needle:?} in:\n{}", self.log.join("\n"));
    }

    pub fn signal(&self, signal: &str) {
        let status = Command::new("kill")
            .args([format!("-{signal}"), self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success(), "kill -{signal} failed");
    }

    /// Waits (`limit` at most) for the process to exit; returns its exit
    /// code, its whole stderr and how long it took.
    pub fn wait_for_exit_within(mut self, limit: Duration) -> (Option<i32>, String, Duration) {
        let start = std::time::Instant::now();
        while start.elapsed() < limit {
            if let Some(status) = self.child.try_wait().unwrap() {
                let took = start.elapsed();
                // The reader thread ends at EOF, once the pipe is closed.
                while let Ok(line) = self.stderr.recv_timeout(Duration::from_secs(5)) {
                    self.log.push(line);
                }
                return (status.code(), self.log.join("\n"), took);
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        panic!(
            "serve did not exit within {limit:?}:\n{}",
            self.log.join("\n")
        );
    }

    /// Waits (a few seconds at most) for the process to exit; returns its
    /// exit code and its whole stderr.
    pub fn wait_for_exit(self) -> (Option<i32>, String) {
        let (code, log, _) = self.wait_for_exit_within(Duration::from_secs(10));
        (code, log)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs the command with this umask (e.g. `0o022`, a common default that
/// makes new files world-readable).
pub fn with_umask(command: &mut Command, mask: u32) {
    // SAFETY: `umask` is async-signal-safe and can't fail.
    unsafe {
        command.pre_exec(move || {
            libc::umask(mask as libc::mode_t);
            Ok(())
        });
    }
}
