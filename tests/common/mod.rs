//! In-process test harness: the single test seam.
//!
//! Builds the real app (real config type, real migrations) against a fresh
//! SQLite database in a temp directory, with a controllable Clock, and drives it
//! with `tower::ServiceExt::oneshot` — no network port.

#![allow(dead_code)]

pub mod ballot_cli;
pub mod server;

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use axum::http::{Request, Response};
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;
use vota::clock::{Clock as _, ManualClock};
use vota::config::Config;
use vota::data::admin_tokens::{self, NewAdminToken};
use vota::{AppState, app};

/// A fixed, well-known starting instant for every test.
pub fn start_time() -> DateTime<Utc> {
    "2030-01-15T12:00:00Z".parse().unwrap()
}

pub struct TestApp {
    pub state: AppState,
    pub clock: ManualClock,
    pub router: Router,
    /// Owns the database file, theme folder and logo folder.
    pub dir: TempDir,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(|_| {}).await
    }

    /// Build the app after letting the test adjust the default test config
    /// (and e.g. fill its theme and logo folders).
    pub async fn with_config(adjust: impl FnOnce(&mut Config)) -> Self {
        Self::try_with_config(adjust).await.unwrap()
    }

    /// Like [`Self::with_config`], but returns the startup error, if any.
    pub async fn try_with_config(adjust: impl FnOnce(&mut Config)) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir().unwrap();
        let theme_folder = dir.path().join("themes");
        let logo_folder = dir.path().join("logos");
        std::fs::create_dir(&theme_folder).unwrap();
        std::fs::create_dir(&logo_folder).unwrap();

        let mut config = Config {
            database_path: dir.path().join("vota.db"),
            public_base_url: "https://ballots.example.org/".parse().unwrap(),
            theme_folder: Some(theme_folder),
            logo_folder: Some(logo_folder),
            ..Config::default()
        };
        adjust(&mut config);

        let clock = ManualClock::new(start_time());
        let state = AppState::new(config, Arc::new(clock.clone())).await?;
        let router = app(state.clone());
        Ok(TestApp {
            state,
            clock,
            router,
            dir,
        })
    }

    /// The app restarted on the same database, Clock and folders, after
    /// letting the test change the folders' contents: the Theme registry is
    /// loaded afresh.
    pub async fn restart(self, change: impl FnOnce(&Config)) -> Self {
        self.restart_with_config(|config| change(config)).await
    }

    /// Like [`Self::restart`], but the test may also change the
    /// configuration (e.g. `admin_token_files`) for the new start.
    pub async fn restart_with_config(self, change: impl FnOnce(&mut Config)) -> Self {
        let mut config = Config::clone(&self.state.config);
        change(&mut config);
        let state = AppState::new(config, Arc::new(self.clock.clone()))
            .await
            .unwrap();
        let router = app(state.clone());
        TestApp {
            state,
            clock: self.clock,
            router,
            dir: self.dir,
        }
    }

    /// The database file and its `-wal` and `-shm` companions.
    pub fn database_files(&self) -> [std::path::PathBuf; 3] {
        let database = self.state.config.database_path.clone();
        let with_suffix = |suffix: &str| {
            let mut name = database.clone().into_os_string();
            name.push(suffix);
            std::path::PathBuf::from(name)
        };
        [with_suffix("-wal"), with_suffix("-shm"), database]
    }

    /// Wipes the database through the filesystem, as a broken disk would:
    /// the database file and its write-ahead log are truncated and the
    /// shared-memory index is zeroed, all in place, so the app's open
    /// connections see it at once. From then on every query fails.
    pub fn wipe_database_files(&self) {
        use std::io::Write as _;
        let [wal, shm, database] = self.database_files();
        for file in [database, wal] {
            let file = std::fs::OpenOptions::new().write(true).open(file).unwrap();
            file.set_len(0).unwrap();
        }
        // Zeroed, not truncated: SQLite maps this file into memory.
        let length = std::fs::metadata(&shm).unwrap().len();
        let mut shm = std::fs::OpenOptions::new().write(true).open(shm).unwrap();
        shm.write_all(&vec![0; length as usize]).unwrap();
    }

    pub fn theme_folder(&self) -> std::path::PathBuf {
        self.state.config.theme_folder.clone().unwrap()
    }

    pub fn logo_folder(&self) -> std::path::PathBuf {
        self.state.config.logo_folder.clone().unwrap()
    }

    pub async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(request).await.unwrap()
    }

    pub async fn get(&self, path: &str) -> Response<Body> {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    /// GET `path` with an Admin Token as bearer token.
    pub async fn admin_get(&self, path: &str, admin_token: &str) -> Response<Body> {
        let request = Request::get(path).header("authorization", format!("Bearer {admin_token}"));
        self.send(request.body(Body::empty()).unwrap()).await
    }

    /// A new valid Admin Token, created through the same code path as the
    /// `admin-token create` CLI command.
    pub async fn admin_token(&self) -> String {
        self.admin_token_with(NewAdminToken::default()).await
    }

    pub async fn admin_token_with(&self, new: NewAdminToken) -> String {
        admin_tokens::create(&self.state.db, new, self.clock.now())
            .await
            .unwrap()
            .secret
            .expose()
            .to_owned()
    }

    /// Creates a Ballot through the admin API and returns the response JSON.
    pub async fn create_ballot(&self, request: &serde_json::Value) -> serde_json::Value {
        let token = self.admin_token().await;
        let response = self
            .post_json("/admin/ballots", Some(&token), request)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "creating a Ballot failed"
        );
        body_json(response).await
    }

    /// Creates the [`ballot_request`] Ballot; returns its token and Option IDs.
    pub async fn create_standard_ballot(&self) -> (String, Vec<String>) {
        let created = self.create_ballot(&ballot_request()).await;
        let options = created["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap().to_owned())
            .collect();
        (created["token"].as_str().unwrap().to_owned(), options)
    }

    /// Creates a Ballot with these modes and three Options (Pizza, Sushi,
    /// Tacos); returns its token and Option IDs.
    pub async fn create_ballot_with_modes(
        &self,
        choice_mode: &str,
        submission_mode: &str,
    ) -> (String, Vec<String>) {
        let mut request = ballot_request();
        request["options"] = serde_json::json!([
            { "label": "Pizza" },
            { "label": "Sushi" },
            { "label": "Tacos" },
        ]);
        request["choice_mode"] = choice_mode.into();
        request["submission_mode"] = submission_mode.into();
        let created = self.create_ballot(&request).await;
        let options = created["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap().to_owned())
            .collect();
        (created["token"].as_str().unwrap().to_owned(), options)
    }

    /// Creates the [`ballot_request`] Ballot, then has the admin replace its
    /// Options (with new IDs, as every replacement gives); returns its token,
    /// the old (stale) Option IDs and the current ones.
    pub async fn create_ballot_with_replaced_options(&self) -> (String, Vec<String>, Vec<String>) {
        let created = self.create_ballot(&ballot_request()).await;
        let ids = |ballot: &serde_json::Value| -> Vec<String> {
            ballot["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let admin = self.admin_token().await;
        let response = self
            .admin_patch(
                &format!("/admin/ballots/{}", created["id"]),
                &admin,
                &serde_json::json!({ "options": [{ "label": "Curry" }, { "label": "Soup" }] }),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "replacing Options failed"
        );
        let edited = body_json(response).await;
        let token = created["token"].as_str().unwrap().to_owned();
        (token, ids(&created), ids(&edited))
    }

    /// GET the voter API view of the Ballot with this token.
    pub async fn voter_view(&self, token: &str) -> serde_json::Value {
        body_json(self.get(&format!("/api/b/{token}")).await).await
    }

    /// POST a JSON body, optionally with an Admin Token as bearer token.
    pub async fn post_json(
        &self,
        path: &str,
        admin_token: Option<&str>,
        body: &serde_json::Value,
    ) -> Response<Body> {
        self.post_raw(path, admin_token, "application/json", body.to_string())
            .await
    }

    /// PATCH a JSON body with an Admin Token as bearer token.
    pub async fn admin_patch(
        &self,
        path: &str,
        admin_token: &str,
        body: &serde_json::Value,
    ) -> Response<Body> {
        self.admin_patch_raw(path, admin_token, "application/json", body.to_string())
            .await
    }

    /// PATCH an arbitrary body with the given content type and an Admin Token.
    pub async fn admin_patch_raw(
        &self,
        path: &str,
        admin_token: &str,
        content_type: &str,
        body: impl Into<Body>,
    ) -> Response<Body> {
        let request = Request::patch(path)
            .header("content-type", content_type)
            .header("authorization", format!("Bearer {admin_token}"));
        self.send(request.body(body.into()).unwrap()).await
    }

    /// DELETE `path` with an Admin Token as bearer token.
    pub async fn admin_delete(&self, path: &str, admin_token: &str) -> Response<Body> {
        let request =
            Request::delete(path).header("authorization", format!("Bearer {admin_token}"));
        self.send(request.body(Body::empty()).unwrap()).await
    }

    /// PUT a JSON body (no Admin Token: the voter API needs none). A voter
    /// API Response (`…/api/b/{token}/response`, an object) without a
    /// `revision` gets the Ballot's current one, as a client that has just
    /// fetched the Ballot sends it; one that names a `revision` is sent as
    /// it is (and [`Self::put_raw`] never adds one).
    pub async fn put_json(&self, path: &str, body: &serde_json::Value) -> Response<Body> {
        let mut body = body.clone();
        if let (Some(ballot), Some(fields)) = (path.strip_suffix("/response"), body.as_object_mut())
            && path.contains("/api/b/")
            && !fields.contains_key("revision")
            && let Some(revision) = self.current_revision(ballot).await
        {
            fields.insert("revision".into(), revision.into());
        }
        self.put_raw(path, "application/json", body.to_string())
            .await
    }

    /// The `revision` the voter API shows for the Ballot at `api_path`
    /// (`…/api/b/{token}`), if it shows the Ballot at all.
    pub async fn current_revision(&self, api_path: &str) -> Option<i64> {
        let response = self.get(api_path).await;
        if response.status() != StatusCode::OK {
            return None;
        }
        body_json(response).await["revision"].as_i64()
    }

    /// PUT an arbitrary body with the given content type.
    pub async fn put_raw(
        &self,
        path: &str,
        content_type: &str,
        body: impl Into<Body>,
    ) -> Response<Body> {
        let request = Request::put(path).header("content-type", content_type);
        self.send(request.body(body.into()).unwrap()).await
    }

    /// POST a URL-encoded form body, as a browser without JavaScript does. A
    /// voting form (to `…/b/{token}`) without a `revision` field gets the
    /// Ballot's current one, as the page just rendered would carry it; one
    /// that has the field is sent as it is (and [`Self::post_raw`] never
    /// adds one).
    pub async fn post_form(&self, path: &str, body: &str) -> Response<Body> {
        let mut body = body.to_owned();
        let has_revision = body.split('&').any(|field| field.starts_with("revision="));
        if let Some((base, token)) = path.rsplit_once("/b/")
            && !has_revision
            && let Some(revision) = self
                .current_revision(&format!("{base}/api/b/{token}"))
                .await
        {
            if !body.is_empty() {
                body.push('&');
            }
            body.push_str(&format!("revision={revision}"));
        }
        self.post_raw(path, None, "application/x-www-form-urlencoded", body)
            .await
    }

    /// POST an arbitrary body with the given content type.
    pub async fn post_raw(
        &self,
        path: &str,
        admin_token: Option<&str>,
        content_type: &str,
        body: impl Into<Body>,
    ) -> Response<Body> {
        let mut request = Request::post(path).header("content-type", content_type);
        if let Some(token) = admin_token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        self.send(request.body(body.into()).unwrap()).await
    }
}

/// Writes `contents` to `path` (relative to `folder`), creating parent
/// directories.
pub fn write_file(folder: &std::path::Path, path: &str, contents: impl AsRef<[u8]>) {
    let path = folder.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A small SVG logo.
pub const SVG_LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#c00"/></svg>"##;

/// Bytes that start like a PNG file (signature and IHDR chunk header).
pub const PNG_LOGO: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";

/// Bytes that start like a WebP file (RIFF container of type WEBP).
pub const WEBP_LOGO: &[u8] =
    b"RIFF\x1a\0\0\0WEBPVP8L\x0d\0\0\0\x2f\0\0\0\x10\x07\x10\x11\x11\x88\x88\xfe\x07\0";

/// A minimal valid create-Ballot request: two Options, deadline one week
/// after [`start_time`].
pub fn ballot_request() -> serde_json::Value {
    serde_json::json!({
        "title": "Lunch on Friday?",
        "options": [{ "label": "Pizza" }, { "label": "Sushi" }],
        "voting_deadline": "2030-01-22T12:00:00Z",
    })
}

/// Submits `body` to the voter API and asserts a 422 naming `field`, with nothing recorded.
pub async fn assert_response_rejected(
    app: &TestApp,
    token: &str,
    body: &serde_json::Value,
    field: &str,
) {
    let response = app
        .put_json(&format!("/api/b/{token}/response"), body)
        .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {body}"
    );
    let error = body_json(response).await;
    assert_eq!(error["error"], "invalid_input", "{error}");
    assert_eq!(error["field"], field, "for {body}: {error}");
    let ballot = app.voter_view(token).await;
    assert_eq!(ballot["status"], "unanswered", "after {body}");
}

/// `prefix` + an unknown well-formed token, then many malformed ones: wrong
/// length, wrong alphabet, non-canonical last character, percent-encoded
/// traversal and junk, invalid UTF-8.
pub fn not_found_token_paths(prefix: &str, unknown: &str) -> Vec<String> {
    let malformed = [
        String::new(),
        "short".into(),
        format!("{unknown}A"),
        unknown[..21].to_owned(),
        // 22 characters, but outside the base64url alphabet.
        "AAAAAAAAAAAAAAAAAAAAA+".into(),
        "AAAAAAAAAAAAAAAAAAAAA=".into(),
        // 22 base64url characters whose last one carries non-zero padding bits.
        "AAAAAAAAAAAAAAAAAAAAAB".into(),
        "..%2F..%2Fetc%2Fpasswd".into(),
        "%00AAAAAAAAAAAAAAAAAAAAA".into(),
        "%E2%80%AEAAAAAAAAAAAAAAAAAAA".into(),
        "%FF%FE".into(),
        "A".repeat(10_000),
    ];
    std::iter::once(unknown.to_owned())
        .chain(malformed)
        .map(|token| format!("{prefix}{token}"))
        .collect()
}

/// The Content Security Policy of the 404, 429 and 500 pages and of the API
/// errors: no scripts, nothing fetched; only the error pages' own inline
/// style (by its hash: `body { font-family: … }` in `not_found.html` and
/// `server_error.html`).
pub const ERROR_CSP: &str = "default-src 'none'; \
                             style-src 'sha256-VOff0Gj0DrKquTZBjdtxMO7iduoPD6gMHxobtltf1y4='; \
                             base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Asserts the security headers of an error response: [`ERROR_CSP`],
/// `nosniff`, no referrer, and never stored by a cache.
pub fn assert_hardened(response: &Response<Body>, what: &str) {
    use axum::http::header;
    let headers = response.headers();
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        ERROR_CSP,
        "{what}"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{what}");
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer", "{what}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{what}");
}

/// Status, headers and body of a response, for byte-identity comparisons.
pub async fn snapshot(response: Response<Body>) -> (StatusCode, String, Vec<u8>) {
    let status = response.status();
    let headers = format!("{:?}", response.headers());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body.to_vec())
}

pub async fn body_string(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

pub async fn body_json(response: Response<Body>) -> serde_json::Value {
    serde_json::from_str(&body_string(response).await).unwrap()
}

/// Requests from a given client: the TCP peer address is what axum's
/// `ConnectInfo` would carry when serving on a real socket.
impl TestApp {
    /// GET `path` as if the TCP peer were `peer` (e.g. `"192.0.2.1"`), with an
    /// optional `X-Forwarded-For` header.
    pub async fn get_from(
        &self,
        peer: &str,
        forwarded_for: Option<&str>,
        path: &str,
    ) -> Response<Body> {
        let mut request = Request::get(path);
        if let Some(value) = forwarded_for {
            request = request.header("x-forwarded-for", value);
        }
        self.send(from_peer(peer, request.body(Body::empty()).unwrap()))
            .await
    }
}

/// Marks `request` as coming from the TCP peer `peer` (an IP address; the
/// port is arbitrary).
pub fn from_peer(peer: &str, mut request: Request<Body>) -> Request<Body> {
    let ip: std::net::IpAddr = peer.parse().unwrap();
    let addr = std::net::SocketAddr::new(ip, 40_000);
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(addr));
    request
}

/// Log output (as the `serve` command formats it, without colours)
/// captured while the guard lives, on this test's thread: requests sent
/// with [`TestApp::send`] run there.
#[derive(Clone, Default)]
pub struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLogs {
    pub fn start(&self) -> tracing::subscriber::DefaultGuard {
        let logs = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || logs.clone())
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}
