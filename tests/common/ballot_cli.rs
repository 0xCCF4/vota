//! `vota ballot …` run as a real binary against a real `vota serve`
//! on loopback, with an Admin Token created by the `admin-token` CLI on the
//! server's database.
#![cfg(unix)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::mpsc;
use std::thread;

use assert_cmd::Command;
use chrono::{Duration, SecondsFormat, Utc};

use super::server::Server;

/// A running server and a valid Admin Token for it.
pub struct Admin {
    pub server: Server,
    pub token: String,
}

impl Admin {
    pub fn start() -> Self {
        Self::start_with(|_, _| {})
    }

    /// Like [`Self::start`], after letting the test adjust the server's
    /// command (e.g. its `VOTA_*` variables).
    pub fn start_with(adjust: impl FnOnce(&mut StdCommand, &Path)) -> Self {
        let server = Server::start_with(adjust);
        let token = admin_token(&server.database());
        Self { server, token }
    }

    /// The server's base URL, `http://127.0.0.1:<port>/`.
    pub fn url(&self) -> String {
        format!("http://{}/", self.server.addr)
    }

    /// `vota ballot` with a clean environment holding only the server's
    /// URL and the Admin Token (`VOTA_URL`, `VOTA_ADMIN_TOKEN`).
    pub fn ballot(&self) -> Command {
        let mut command = client();
        command
            .env("VOTA_URL", self.url())
            .env("VOTA_ADMIN_TOKEN", &self.token)
            .arg("ballot");
        command
    }

    /// Creates a Ballot with `--json` and the given extra arguments (the
    /// title, two Options and a deadline a week ahead are given unless the
    /// arguments name them); returns the admin view.
    pub fn create(&self, extra: &[&str]) -> serde_json::Value {
        let mut args: Vec<String> = vec!["create".into(), "--json".into()];
        if !extra.contains(&"--title") {
            args.extend(["--title".into(), "Lunch on Friday?".into()]);
        }
        if !extra.contains(&"--option") {
            args.extend(["--option", "Pizza", "--option", "Sushi"].map(String::from));
        }
        if !extra.contains(&"--voting-deadline") {
            args.extend(["--voting-deadline".into(), deadline_in_days(7)]);
        }
        args.extend(extra.iter().map(|arg| arg.to_string()));
        let output = self.ballot().args(&args).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

/// `vota` with a clean environment.
pub fn client() -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command.env_clear().env("RUST_LOG", "trace");
    command
}

/// Creates an Admin Token on `database` with the `admin-token` CLI.
pub fn admin_token(database: &Path) -> String {
    let output = client()
        .env("VOTA_DATABASE_PATH", database)
        .args(["admin-token", "create"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// An RFC 3339 UTC timestamp `days` days from now (whole seconds).
pub fn deadline_in_days(days: i64) -> String {
    (Utc::now() + Duration::days(days)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// stdout and stderr of a finished command, as text.
pub fn texts(output: &std::process::Output) -> (String, String) {
    (
        String::from_utf8(output.stdout.clone()).unwrap(),
        String::from_utf8(output.stderr.clone()).unwrap(),
    )
}

/// Submits a Response on the Ballot with this token through the voter API
/// (a plain HTTP request; a new Ballot has revision 0).
pub fn vote(admin: &Admin, ballot_token: &str, option_ids: &[&str]) {
    let body = serde_json::json!({ "option_ids": option_ids, "revision": 0 }).to_string();
    let request = format!(
        "PUT /api/b/{ballot_token}/response HTTP/1.1\r\nHost: localhost\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let answer = exchange(admin.server.addr, &request);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
}

/// Sends a raw HTTP request; returns the whole answer.
fn exchange(addr: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

/// A stand-in server on loopback that answers every request with a fixed
/// raw HTTP response (`None`: never answers), and hands each request's
/// head and body to the test.
pub struct FakeServer {
    pub addr: SocketAddr,
    requests: mpsc::Receiver<String>,
}

impl FakeServer {
    pub fn start(response: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (sender, requests) = mpsc::channel();
        thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_request(&mut stream);
                if sender.send(request).is_err() {
                    break;
                }
                match &response {
                    Some(response) => {
                        let _ = stream.write_all(response.as_bytes());
                    }
                    // Kept open, silent.
                    None => held.push(stream),
                }
            }
        });
        Self { addr, requests }
    }

    /// A server answering `200` with this JSON body.
    pub fn json(body: &serde_json::Value) -> Self {
        let body = body.to_string();
        Self::start(Some(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )))
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// The requests received so far.
    pub fn requests(&self) -> Vec<String> {
        self.requests.try_iter().collect()
    }
}

/// The request head and its body (as far as `Content-Length` says).
fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut data = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(read @ 1..) = stream.read(&mut buffer) {
        data.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&data).into_owned();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&data).into_owned()
}

/// Runs `command` with a pseudo-terminal as stdin, stdout and stderr,
/// writes `answer` once `prompt` has appeared, and returns the exit code
/// and everything the terminal showed.
pub fn run_in_terminal(mut command: StdCommand, prompt: &str, answer: &str) -> (i32, String) {
    let (mut master, slave) = open_pty();
    command
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave);
    let mut child = command.spawn().unwrap();
    // The child's copies of the slave are its own now; ours (held by the
    // command) must close so that reading the master ends once it exits.
    drop(command);
    let mut shown = Vec::new();
    let mut buffer = [0; 4096];
    let mut answered = false;
    loop {
        match master.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => shown.extend_from_slice(&buffer[..read]),
        }
        if !answered && String::from_utf8_lossy(&shown).contains(prompt) {
            master.write_all(answer.as_bytes()).unwrap();
            answered = true;
        }
    }
    let status = child.wait().unwrap();
    (
        status.code().unwrap(),
        String::from_utf8_lossy(&shown).into_owned(),
    )
}

/// A new pseudo-terminal: its master and slave side.
fn open_pty() -> (std::fs::File, std::fs::File) {
    use std::os::fd::FromRawFd as _;
    let (mut master, mut slave) = (0, 0);
    // SAFETY: `openpty` writes two new descriptors into the integers given,
    // which then belong to the `File`s alone.
    unsafe {
        let result = libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        );
        assert_eq!(result, 0, "openpty failed");
        (
            std::fs::File::from_raw_fd(master),
            std::fs::File::from_raw_fd(slave),
        )
    }
}
