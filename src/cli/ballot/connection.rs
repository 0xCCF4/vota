//! The HTTP side of `vota ballot`: the server's URL, the Admin Token and
//! the HTTP(S) client. A response is handed over as it arrived, wrapped as
//! an [`UntrustedValue`]: this module reads none of it, [`answer`](super::answer)
//! does. Knows nothing about Ballots.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::Method;
use reqwest::header::{self, HeaderValue};
use untrusted_value::UntrustedValue;
use url::Url;

use super::answer::Arrival;
use super::{Exit, Failure, read_input};

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
            .map_err(|error| {
                Failure::new(Exit::Network, with_causes("can't set up TLS", &error))
            })?;
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
                "--url {url}: not an absolute http:// or https:// URL"
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
                "--url {url}: the scheme must be http or https"
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
    /// already encoded query, if any) and returns the response unread; a
    /// [`Failure`] (exit 7) only when no complete response arrived.
    pub(super) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<UntrustedValue<Arrival>, Failure> {
        let url = self.base.join(path).map_err(|error| {
            Failure::new(Exit::Other, format!("can't build the request URL: {error}"))
        })?;
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
        Ok(Arrival::wrap(status, &headers, bytes.to_vec()))
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
    Failure::new(Exit::Network, with_causes(what, &error))
}

/// `what`, the error and its source chain, which names the cause (refused,
/// TLS, …), never the request's headers. It may quote the server (a TLS
/// error names the certificate's names), so it is escaped like all text
/// in a [`Failure`].
fn with_causes(what: &str, error: &reqwest::Error) -> String {
    let mut message = format!("{what}: {error}");
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message += &format!(": {cause}");
        source = cause.source();
    }
    message
}
