//! The HTTP side of `vota ballot`: the server's URL, the Admin Token,
//! the HTTP(S) client, and a reply turned into JSON or a [`Failure`] with
//! its exit code. Knows nothing about Ballots.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::{self, HeaderValue};
use reqwest::{Method, StatusCode};
use url::Url;

use super::{Failure, read_input};

/// Where the server is and how to reach it: global flags of `ballot`.
#[derive(clap::Args)]
#[command(next_help_heading = "Connection")]
pub(super) struct ConnectionArgs {
    /// Public base URL of the Vota server, sub-path included, e.g.
    /// `https://example.org/polls/` (its `public_base_url`). A missing
    /// trailing slash is added. Only `https://`, or `http://` to a loopback
    /// host (see --allow-http).
    #[arg(
        long,
        env = "VOTA_URL",
        global = true,
        value_name = "URL",
        value_hint = clap::ValueHint::Url
    )]
    url: Option<String>,
    /// The Admin Token. Unsafe: it is visible in `ps` and the shell
    /// history; prefer --token-file or the environment variable
    /// VOTA_ADMIN_TOKEN, which is used when neither flag is given.
    #[arg(
        long,
        global = true,
        value_name = "SECRET",
        conflicts_with = "token_file"
    )]
    token: Option<String>,
    /// Read the Admin Token from this file (`-`: stdin); surrounding
    /// whitespace is trimmed.
    #[arg(long, global = true, value_name = "PATH|-")]
    token_file: Option<PathBuf>,
    /// Give up on a request after this long, e.g. `30s`, `2m`.
    #[arg(
        long,
        global = true,
        value_name = "DURATION",
        value_parser = humantime::parse_duration,
        default_value = "30s"
    )]
    timeout: Duration,
    /// Trust this CA certificate (PEM) on top of the system's roots.
    #[arg(long, global = true, value_name = "PEM")]
    ca_cert: Option<PathBuf>,
    /// Don't verify the server's TLS certificate. Unsafe: anyone on the way
    /// can read the Admin Token.
    #[arg(long, global = true)]
    insecure: bool,
    /// Allow `http://` to a host other than loopback. Unsafe: the Admin
    /// Token travels in clear text.
    #[arg(long, global = true)]
    allow_http: bool,
}

/// A client for one server with one Admin Token.
pub(super) struct Connection {
    http: reqwest::Client,
    base: Url,
    authorization: HeaderValue,
}

/// A successful reply: its JSON body, `None` for `204 No Content`.
pub(super) type Reply = Option<serde_json::Value>;

impl ConnectionArgs {
    /// The connection, or a usage error (exit 2) for a missing or refused
    /// URL, a missing Admin Token or an unreadable file.
    pub(super) fn connect(self) -> Result<Connection, Failure> {
        let base = base_url(self.url.as_deref(), self.allow_http)?;
        let token = match (self.token, self.token_file) {
            (Some(token), _) => token,
            (None, Some(path)) => read_input(&path, "--token-file")?,
            (None, None) => std::env::var("VOTA_ADMIN_TOKEN").unwrap_or_default(),
        };
        let token = token.trim();
        if token.is_empty() {
            return Err(Failure::usage(
                "no Admin Token: give --token-file <PATH|->, VOTA_ADMIN_TOKEN or --token",
            ));
        }
        let mut authorization = HeaderValue::try_from(format!("Bearer {token}")).map_err(|_| {
            Failure::usage("the Admin Token contains characters a header can't hold")
        })?;
        authorization.set_sensitive(true);

        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("vota/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(self.timeout);
        let extra_roots = match &self.ca_cert {
            Some(path) => {
                let pem = std::fs::read(path).map_err(|error| {
                    Failure::usage(format!("can't read --ca-cert {}: {error}", path.display()))
                })?;
                reqwest::Certificate::from_pem_bundle(&pem)
                    .ok()
                    .filter(|certificates| !certificates.is_empty())
                    .ok_or_else(|| {
                        Failure::usage(format!(
                            "--ca-cert {}: no PEM certificate in it",
                            path.display()
                        ))
                    })?
            }
            None => Vec::new(),
        };
        if self.insecure {
            eprintln!(
                "Warning: --insecure: the server's TLS certificate is not verified; \
                 anyone on the way can read the Admin Token."
            );
        }
        builder = if base.scheme() == "http" {
            // Redirects aren't followed, so no request leaves plain HTTP:
            // no CA store is loaded, and a host without one works too.
            builder.tls_certs_only(Vec::new())
        } else {
            builder
                .https_only(true)
                .tls_certs_merge(extra_roots)
                .tls_danger_accept_invalid_certs(self.insecure)
        };
        let http = builder
            .build()
            // In practice a TLS problem, e.g. no CA certificates on the system.
            .map_err(|error| Failure::new(7, with_causes("can't set up TLS", &error)))?;
        Ok(Connection {
            http,
            base,
            authorization,
        })
    }
}

/// The base URL with a trailing slash. Refused (exit 2): none at all, one
/// without a scheme or with another scheme than `http` or `https`, and
/// `http` to a host other than loopback unless allowed.
fn base_url(url: Option<&str>, allow_http: bool) -> Result<Url, Failure> {
    let url = url.unwrap_or_default();
    if url.is_empty() {
        return Err(Failure::usage(
            "no server URL: give --url or VOTA_URL, e.g. https://example.org/polls/",
        ));
    }
    let mut base = Url::parse(url)
        .ok()
        .filter(|base| base.has_host())
        .ok_or_else(|| {
            Failure::usage(format!(
                "--url {}: not an absolute http:// or https:// URL",
                super::printable(url)
            ))
        })?;
    match base.scheme() {
        "https" => {}
        "http" if allow_http || is_loopback(&base) => {}
        "http" => {
            return Err(Failure::usage(
                "refusing http:// to a host other than loopback: the Admin Token would travel \
                 in clear text; use https:// (or --allow-http)",
            ));
        }
        _ => {
            return Err(Failure::usage(format!(
                "--url {}: the scheme must be http or https",
                super::printable(url)
            )));
        }
    }
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    Ok(base)
}

/// `localhost`, `127.0.0.0/8` or `::1`.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

impl Connection {
    /// Sends one request to `path` (relative to the base URL, with an
    /// already encoded query, if any) and reads the reply.
    pub(super) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Reply, Failure> {
        let url = self
            .base
            .join(path)
            .map_err(|error| Failure::new(1, format!("can't build the request URL: {error}")))?;
        let mut request = self
            .http
            .request(method, url)
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::ACCEPT, "application/json");
        if let Some(body) = body {
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let response = request.send().await.map_err(network_failure)?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(network_failure)?;
        reply(status, &headers, &bytes)
    }
}

/// A request that got no complete answer: exit 7.
fn network_failure(error: reqwest::Error) -> Failure {
    let what = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "can't connect"
    } else {
        "request failed"
    };
    Failure::new(7, with_causes(what, &error))
}

/// `what`, the error and its source chain, which names the cause (refused,
/// TLS, …), never the request's headers.
fn with_causes(what: &str, error: &reqwest::Error) -> String {
    let mut message = format!("{what}: {error}");
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message += &format!(": {cause}");
        source = cause.source();
    }
    message
}

/// The reply's JSON body on success; otherwise a [`Failure`] with the exit
/// code of its status and the error body for `--json`.
fn reply(status: StatusCode, headers: &header::HeaderMap, body: &[u8]) -> Result<Reply, Failure> {
    let json: Option<serde_json::Value> = serde_json::from_slice(body).ok();
    if status.is_success() {
        if status == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        return match json {
            Some(json) => Ok(Some(json)),
            None => Err(Failure::new(
                1,
                format!("unexpected response: {status} without a JSON body"),
            )),
        };
    }
    if status.is_redirection() {
        let location = headers
            .get(header::LOCATION)
            .map(|value| super::printable(&String::from_utf8_lossy(value.as_bytes())))
            .unwrap_or_else(|| "nowhere".into());
        return Err(Failure::new(
            1,
            format!("{status}: redirected to {location}; redirects are not followed (check --url)"),
        ));
    }
    let exit = match status.as_u16() {
        404 => 3,
        409 => 4,
        400 | 413 | 415 | 422 => 5,
        401 | 429 => 6,
        _ => 1,
    };
    let mut message = status.as_u16().to_string();
    match &json {
        Some(error) if error.get("error").is_some_and(|code| code.is_string()) => {
            let text = |key: &str| error.get(key).and_then(|value| value.as_str());
            message += &format!(" {}", super::printable(text("error").unwrap_or_default()));
            if let Some(detail) = text("message") {
                message += &format!(": {}", super::printable(detail));
            }
            if let Some(field) = text("field").filter(|field| !field.is_empty()) {
                message += &format!(" (field: {})", super::printable(field));
            }
        }
        _ => {
            message += &format!(" {}", status.canonical_reason().unwrap_or("unknown status"));
            // A short plain-text reason (such as the body limit's) is
            // worth showing; an HTML page, say from a proxy, is not.
            let plain = headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/plain"));
            let text = String::from_utf8_lossy(body);
            let text = text.trim();
            if plain && !text.is_empty() {
                let shown: String = text.chars().take(200).collect();
                message += &format!(": {}", super::printable(&shown));
            }
        }
    }
    if status == StatusCode::UNAUTHORIZED {
        message += " (the Admin Token was refused: unknown, revoked or expired)";
    }
    if let Some(retry_after) = headers.get(header::RETRY_AFTER) {
        let value = String::from_utf8_lossy(retry_after.as_bytes());
        message += &match value.trim().parse::<u64>() {
            Ok(seconds) => format!(" (retry after {seconds} seconds)"),
            Err(_) => format!(" (Retry-After: {})", super::printable(&value)),
        };
    }
    let body = match json {
        Some(json) => Some(super::json_text(&json)),
        None if body.is_empty() => None,
        None => Some(super::printable(&String::from_utf8_lossy(body))),
    };
    Err(Failure {
        exit,
        message,
        body,
    })
}
