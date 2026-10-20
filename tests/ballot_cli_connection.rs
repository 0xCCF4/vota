//! `vota ballot`: the server URL, the Admin Token, the transport rules
//! and the exit codes.
#![cfg(all(unix, feature = "server", feature = "client"))]

mod common;

use std::net::TcpListener;

use common::ballot_cli::{Admin, FakeServer, admin_token, client, deadline_in_days, texts};

fn exit_code(output: &std::process::Output) -> i32 {
    output.status.code().unwrap()
}

/// A raw HTTP response with this status line, extra headers and body.
fn raw(status: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

// --- URL ---

#[test]
fn no_url_is_a_usage_error() {
    let output = client()
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "list"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 2, "{output:?}");
    assert!(texts(&output).1.contains("--url"), "{output:?}");
}

#[test]
fn a_url_without_a_scheme_or_with_another_scheme_is_refused() {
    for url in [
        "example.org/polls/",
        "localhost:8080",
        "127.0.0.1:8080",
        "ftp://example.org/",
        "file:///tmp/x",
        "//example.org/",
    ] {
        let output = client()
            .env("VOTA_ADMIN_TOKEN", "vta_x")
            .args(["ballot", "list", "--url", url])
            .output()
            .unwrap();

        assert_eq!(exit_code(&output), 2, "{url}: {output:?}");
        assert!(texts(&output).1.starts_with("Error: "), "{url}: {output:?}");
    }
}

#[test]
fn http_to_a_host_other_than_loopback_is_refused_unless_allowed() {
    let server = FakeServer::start(None);
    for url in [
        "http://example.org/",
        "http://192.0.2.1/",
        "http://[2001:db8::1]/",
    ] {
        let output = client()
            .env("VOTA_ADMIN_TOKEN", "vta_x")
            .args(["ballot", "list", "--url", url])
            .output()
            .unwrap();

        assert_eq!(exit_code(&output), 2, "{url}: {output:?}");
        assert!(texts(&output).1.contains("--allow-http"), "{output:?}");
    }
    // Allowed, the request is made (here: to an address that doesn't
    // answer, so it times out).
    let output = client()
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "list", "--allow-http", "--timeout", "300ms"])
        .args(["--url", "http://192.0.2.1/"])
        .output()
        .unwrap();
    assert_eq!(exit_code(&output), 7, "{output:?}");
    drop(server);
}

#[test]
fn http_to_loopback_by_name_or_address_is_allowed() {
    let admin = Admin::start();
    let port = admin.server.addr.port();

    for url in [
        format!("http://localhost:{port}"),
        format!("http://LOCALHOST:{port}/"),
        format!("http://127.0.0.1:{port}"),
    ] {
        let output = admin
            .ballot()
            .args(["list", "--url", &url])
            .output()
            .unwrap();

        assert!(output.status.success(), "{url}: {output:?}");
    }
    // Other addresses in 127.0.0.0/8 and ::1 count as loopback too; nothing
    // listens there, so the request fails on the network, not before it.
    for url in [
        format!("http://127.0.0.2:{port}/"),
        format!("http://[::1]:{}/", free_port()),
    ] {
        let output = admin
            .ballot()
            .args(["list", "--url", &url])
            .output()
            .unwrap();
        assert_ne!(exit_code(&output), 2, "{url}: {output:?}");
    }
}

/// A loopback port nothing listens on.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn the_client_ignores_the_config_file() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .env("VOTA_CONFIG", "/nonexistent/vota.toml")
        .args(["list", "--config", "/nonexistent/vota.toml"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}

// --- Admin Token ---

#[test]
fn no_admin_token_is_a_usage_error() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .env_remove("VOTA_ADMIN_TOKEN")
        .arg("list")
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 2, "{output:?}");
    assert!(texts(&output).1.contains("no Admin Token"), "{output:?}");
}

#[test]
fn the_token_flag_beats_the_environment_variable() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .env("VOTA_ADMIN_TOKEN", "vta_wrong")
        .args(["list", "--token", &admin.token])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}

#[test]
fn the_token_file_beats_the_environment_variable_and_is_trimmed() {
    let admin = Admin::start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, format!("\n  {}\t\n", admin.token)).unwrap();

    let output = admin
        .ballot()
        .env("VOTA_ADMIN_TOKEN", "vta_wrong")
        .args(["list", "--token-file"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let output = admin
        .ballot()
        .env("VOTA_ADMIN_TOKEN", "vta_wrong")
        .args(["list", "--token-file", "-"])
        .write_stdin(format!("{}\n", admin.token))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn the_environment_variable_is_used_without_a_flag() {
    let admin = Admin::start();

    let output = admin.ballot().arg("list").output().unwrap();
    assert!(output.status.success(), "{output:?}");

    let output = admin
        .ballot()
        .env("VOTA_ADMIN_TOKEN", "vta_wrong")
        .arg("list")
        .output()
        .unwrap();
    assert_eq!(exit_code(&output), 6, "{output:?}");
}

#[test]
fn token_and_token_file_conflict() {
    let output = client()
        .args(["ballot", "list", "--url", "http://127.0.0.1:1/"])
        .args(["--token", "vta_x", "--token-file", "-"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 2, "{output:?}");
    assert!(
        texts(&output).1.contains("cannot be used with"),
        "{output:?}"
    );
}

#[test]
fn an_unreadable_or_empty_token_file_is_a_usage_error() {
    for (file, stdin) in [("/nonexistent/token", ""), ("-", "  \n")] {
        let output = client()
            .args(["ballot", "list", "--url", "http://127.0.0.1:1/"])
            .args(["--token-file", file])
            .write_stdin(stdin)
            .output()
            .unwrap();

        assert_eq!(exit_code(&output), 2, "{file}: {output:?}");
    }
}

#[test]
fn the_admin_token_is_never_printed() {
    let admin = Admin::start();
    let wrong = "vta_ThisTokenIsWrongButSecret";
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, wrong).unwrap();

    let runs = [
        admin
            .ballot()
            .args(["list", "--token", wrong])
            .output()
            .unwrap(),
        admin
            .ballot()
            .args(["list", "--json", "--token-file"])
            .arg(&file)
            .output()
            .unwrap(),
        admin
            .ballot()
            .args(["get", "1", "--token", wrong])
            .output()
            .unwrap(),
        admin
            .ballot()
            .args(["list", "--token", wrong, "--url", "http://127.0.0.1:1/"])
            .output()
            .unwrap(),
        admin.ballot().args(["get", "1"]).output().unwrap(),
        admin.ballot().arg("list").output().unwrap(),
    ];

    for output in runs {
        let (stdout, stderr) = texts(&output);
        for secret in [wrong, admin.token.as_str()] {
            assert!(
                !stdout.contains(secret) && !stderr.contains(secret),
                "{output:?}"
            );
        }
    }
}

// --- Transport ---

#[test]
fn requests_carry_the_user_agent_the_token_and_are_never_retried() {
    let server = FakeServer::start(Some(raw("503 Service Unavailable", "", "")));

    let output = client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_secret")
        .args(["ballot", "get", "4"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 1, "{output:?}");
    assert_eq!(texts(&output).1, "Error: 503 Service Unavailable\n");
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = requests[0].to_ascii_lowercase();
    assert!(
        request.starts_with("get /admin/ballots/4 http/1.1\r\n"),
        "{request}"
    );
    assert!(
        request.contains(&format!(
            "\r\nuser-agent: vota/{}\r\n",
            env!("CARGO_PKG_VERSION")
        )),
        "{request}"
    );
    assert!(
        request.contains("\r\nauthorization: bearer vta_secret\r\n"),
        "{request}"
    );
}

#[test]
fn bodies_are_sent_as_json() {
    let server = FakeServer::start(Some(raw("201 Created", "", r#"{"id":1,"url":"u"}"#)));

    let output = client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args([
            "ballot", "create", "--title", "T", "--option", "A", "--option", "B",
        ])
        .args(["--fallback", "B", "--group", "g"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let request = server.requests().remove(0);
    assert!(
        request.starts_with("POST /admin/ballots HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("\r\ncontent-type: application/json\r\n")
    );
    let body: serde_json::Value =
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "title": "T",
            "options": [{ "label": "A" }, { "label": "B" }],
            "fallback": ["B"],
            "group": "g",
        })
    );
}

#[test]
fn a_redirect_is_not_followed() {
    let server = FakeServer::start(Some(raw(
        "301 Moved Permanently",
        "Location: https://example.org/polls/admin/ballots\r\n",
        "",
    )));

    let output = client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "list"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 1, "{output:?}");
    assert!(
        texts(&output)
            .1
            .contains("https://example.org/polls/admin/ballots"),
        "{output:?}"
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn a_server_that_doesnt_answer_in_time_exits_7() {
    let server = FakeServer::start(None);

    let start = std::time::Instant::now();
    let output = client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "list", "--timeout", "500ms"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 7, "{output:?}");
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
    assert!(texts(&output).1.contains("timed out"), "{output:?}");
}

#[test]
fn a_refused_connection_exits_7() {
    let output = client()
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "list", "--url"])
        .arg(format!("http://127.0.0.1:{}/", free_port()))
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 7, "{output:?}");
    assert!(
        texts(&output).1.starts_with("Error: can't connect"),
        "{output:?}"
    );
}

#[test]
fn an_unexpected_response_exits_1() {
    let server = FakeServer::start(Some(raw("200 OK", "", "<html>hello</html>")));

    let output = client()
        .env("VOTA_URL", server.url())
        .env("VOTA_ADMIN_TOKEN", "vta_x")
        .args(["ballot", "get", "1"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 1, "{output:?}");
    assert!(
        texts(&output).1.contains("unexpected response"),
        "{output:?}"
    );
}

#[test]
fn insecure_prints_a_warning() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["list", "--insecure"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(
        texts(&output).1.starts_with("Warning: --insecure"),
        "{output:?}"
    );
}

#[test]
fn an_unreadable_ca_certificate_is_a_usage_error() {
    let admin = Admin::start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ca.pem");
    std::fs::write(&file, "not a certificate").unwrap();

    for path in [file.as_path(), std::path::Path::new("/nonexistent/ca.pem")] {
        let output = admin
            .ballot()
            .args(["list", "--ca-cert"])
            .arg(path)
            .output()
            .unwrap();

        assert_eq!(exit_code(&output), 2, "{path:?}: {output:?}");
    }
}

// --- Exit codes and errors from the server ---

#[test]
fn not_found_exits_3() {
    let admin = Admin::start();

    let output = admin.ballot().args(["get", "42"]).output().unwrap();

    assert_eq!(exit_code(&output), 3, "{output:?}");
    assert_eq!(
        texts(&output),
        (String::new(), "Error: 404 not_found\n".into())
    );
}

#[test]
fn a_conflict_exits_4_and_with_json_prints_the_error_body() {
    let admin = Admin::start();
    admin.create(&["--group", "g", "--voter-reference", "alice"]);

    let output = admin
        .ballot()
        .args([
            "create", "--json", "--title", "T", "--option", "A", "--option", "B",
        ])
        .args(["--voting-deadline", &deadline_in_days(1)])
        .args(["--group", "g", "--voter-reference", "alice"])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 4, "{output:?}");
    let (stdout, stderr) = texts(&output);
    assert_eq!(
        stderr,
        "Error: 409 duplicate_voter_reference: another Ballot in this Group already has this Voter Reference\n"
    );
    let body: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(body["error"], "duplicate_voter_reference");
}

#[test]
fn invalid_input_exits_5_and_names_the_field() {
    let admin = Admin::start();

    let output = admin
        .ballot()
        .args(["create", "--title", "", "--option", "A", "--option", "B"])
        .args(["--voting-deadline", &deadline_in_days(1)])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 5, "{output:?}");
    assert_eq!(
        texts(&output),
        (
            String::new(),
            "Error: 422 invalid_input: must not be empty (field: title)\n".into()
        )
    );
}

#[test]
fn a_body_over_the_limit_exits_5() {
    let admin = Admin::start();
    let huge = serde_json::json!({ "title": "x".repeat(70 * 1024) }).to_string();

    let output = admin
        .ballot()
        .args(["create", "--json", "--body", "-"])
        .write_stdin(huge)
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 5, "{output:?}");
    let (stdout, stderr) = texts(&output);
    assert!(
        stderr.starts_with("Error: 413 Payload Too Large"),
        "{stderr}"
    );
    // Not JSON: printed as text with --json.
    assert!(stdout.contains("length limit exceeded"), "{stdout}");
}

#[test]
fn a_refused_admin_token_exits_6_and_too_many_exit_6_with_retry_after() {
    let admin = Admin::start_with(|command, _| {
        command.env("VOTA_RATE_LIMIT__ADMIN_FAILED_AUTH", "1");
    });

    let output = admin
        .ballot()
        .args(["list", "--token", "vta_wrong"])
        .output()
        .unwrap();
    assert_eq!(exit_code(&output), 6, "{output:?}");
    assert!(
        texts(&output).1.starts_with("Error: 401 Unauthorized"),
        "{output:?}"
    );

    let output = admin.ballot().args(["list", "--json"]).output().unwrap();
    assert_eq!(exit_code(&output), 6, "{output:?}");
    let (stdout, stderr) = texts(&output);
    assert!(
        stderr.starts_with("Error: 429 too_many_requests (retry after 600 seconds)"),
        "{stderr}"
    );
    let body: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(body["error"], "too_many_requests");
}

#[test]
fn a_server_error_exits_1() {
    let admin = Admin::start();
    // Every query fails from now on, the Admin Token check included.
    let database = admin.server.database();
    for file in [database.clone(), database.with_extension("db-wal")] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(file)
            .unwrap()
            .set_len(0)
            .unwrap();
    }
    let shm = database.with_extension("db-shm");
    let length = std::fs::metadata(&shm).unwrap().len();
    std::fs::write(&shm, vec![0; length as usize]).unwrap();

    let output = admin.ballot().args(["list", "--json"]).output().unwrap();

    assert_eq!(exit_code(&output), 1, "{output:?}");
    let (stdout, stderr) = texts(&output);
    assert_eq!(stderr, "Error: 500 internal\n");
    assert!(stdout.contains("\"internal\""), "{stdout}");
}

#[test]
fn a_token_from_another_database_is_refused() {
    let admin = Admin::start();
    let other = tempfile::tempdir().unwrap();
    let foreign = admin_token(&other.path().join("other.db"));

    let output = admin
        .ballot()
        .args(["list", "--token", &foreign])
        .output()
        .unwrap();

    assert_eq!(exit_code(&output), 6, "{output:?}");
}

#[test]
fn plain_http_needs_no_ca_store() {
    let admin = Admin::start();

    // rustls-native-certs reads these instead of the system's store.
    let output = admin
        .ballot()
        .env("SSL_CERT_FILE", "/nonexistent/ca.pem")
        .env("SSL_CERT_DIR", "/nonexistent")
        .arg("list")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}
