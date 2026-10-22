//! Configuration, loaded from an optional TOML file overlaid with `VOTA_*`
//! environment variables (env wins). An unknown key in the file stops
//! startup; an unknown `VOTA_*` variable is ignored with a warning naming
//! it (never its value).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono_tz::Tz;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use ipnet::{IpNet, Ipv6Net};
use serde::{Deserialize, Serialize};
use url::Url;

/// Prefix for environment variables, e.g. `VOTA_PORT=9000`.
pub const ENV_PREFIX: &str = "VOTA_";

/// One hour: how often the purge sweep runs by default.
pub const DEFAULT_PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// One year (humantime's, 365.25 days): how far ahead a Voting Deadline or
/// Purge Date may be set by default.
pub const DEFAULT_MAX_DATE_HORIZON: Duration = Duration::from_secs(31_557_600);

/// Four weeks: the default gap between Voting Deadline and Purge Date.
pub const DEFAULT_PURGE_GAP: Duration = Duration::from_secs(4 * 7 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Address the single listener binds to.
    pub bind_address: IpAddr,
    pub port: u16,
    /// SQLite database file; created if missing.
    pub database_path: PathBuf,
    /// Base for every public Ballot URL. Never derived from request headers.
    /// Its path is the base path every route is served under.
    pub public_base_url: PublicBaseUrl,
    /// Folder with additional Themes, one subdirectory `<name>/` each with a
    /// `style.css` and optional `templates/{base,ballot,confirm}.html`; one
    /// named like a built-in Theme replaces it. Read at startup only; must
    /// exist if set.
    pub theme_folder: Option<PathBuf>,
    /// Folder with logos selectable as a Theme Tweak: `<name>.svg`, `.png`
    /// or `.webp`, at most 512 KiB each. Read at startup only; must exist if
    /// set.
    pub logo_folder: Option<PathBuf>,
    /// Default gap between Voting Deadline and Purge Date, e.g. "4weeks".
    #[serde(with = "humantime_serde")]
    pub default_purge_gap: Duration,
    /// How often the purge sweep deletes Ballots past their Purge Date, e.g.
    /// "1h". Lookups treat such Ballots as gone in between anyway.
    #[serde(with = "humantime_serde")]
    pub purge_interval: Duration,
    /// How far ahead of now an admin may set a Voting Deadline or Purge Date
    /// (on create, edit and extend), e.g. "1y". A Purge Date the app computes
    /// (the default gap, or the shift on extension) is not capped.
    #[serde(with = "humantime_serde")]
    pub max_date_horizon: Duration,
    /// IANA time zone used to render times server-side, e.g. "Europe/Berlin".
    pub default_time_zone: Tz,
    /// Peers whose `X-Forwarded-For` header is trusted. An entry in
    /// IPv4-mapped IPv6 form (`::ffff:10.0.0.0/104`) means the IPv4 network
    /// (`10.0.0.0/8`), as peers and forwarded addresses are compared in IPv4
    /// form ([`canonical_network`]).
    pub trusted_proxies: Vec<IpNet>,
    /// The operator's NAT64/SIIT translation prefixes (e.g. Jool's `pool6`):
    /// a client address in one counts as the IPv4 address embedded in it
    /// (RFC 6052), for the rate limit and the audit log only. The
    /// well-known prefixes are always decoded.
    pub nat64_prefixes: Vec<Nat64Prefix>,
    /// Limits on failed Ballot Token lookups and failed Admin Token checks
    /// per client IP.
    pub rate_limit: RateLimit,
    /// Limits on the listener's connections.
    pub connections: Connections,
    /// Files that each hold one pre-provisioned Admin Token (e.g. agenix
    /// secrets passed in as systemd credentials), accepted beside the
    /// database's. Read by `serve` at startup only: a missing, unreadable or
    /// malformed file stops it. In the environment a list such as
    /// `VOTA_ADMIN_TOKEN_FILES='["/run/credentials/vota.service/token"]'`.
    pub admin_token_files: Vec<PathBuf>,
}

/// Limits on the listener's HTTP connections, so slow or idle clients can't
/// hold file descriptors and memory without bound, nor delay a shutdown. In
/// TOML a `[connections]` table; in the environment e.g.
/// `VOTA_CONNECTIONS__MAX_OPEN=512`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Connections {
    /// How many connections may be open at once. Beyond it, new connections
    /// wait in the operating system's listen queue until one closes.
    pub max_open: NonZeroU32,
    /// How long a client may take to send a request's headers, counted from
    /// when the server starts waiting for them: on a new connection, and on
    /// a kept-alive one after the previous response (so it is also how long
    /// an idle connection stays open). The connection is closed after it.
    #[serde(with = "humantime_serde")]
    pub header_read_timeout: Duration,
    /// On shutdown: how long open connections get to finish their requests
    /// before they are closed regardless.
    #[serde(with = "humantime_serde")]
    pub shutdown_grace_period: Duration,
}

impl Default for Connections {
    fn default() -> Self {
        Self {
            max_open: NonZeroU32::new(512).expect("non-zero"),
            header_read_timeout: Duration::from_secs(10),
            shutdown_grace_period: Duration::from_secs(10),
        }
    }
}

/// How many failed Ballot Token lookups one client IP may make per window
/// before its Ballot Token requests are refused with 429 until the window
/// ends; and, as a separate budget, how many failed Admin Token checks
/// before its admin requests are. In TOML a `[rate_limit]` table; in the
/// environment e.g. `VOTA_RATE_LIMIT__FAILED_LOOKUPS=20`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RateLimit {
    pub failed_lookups: NonZeroU32,
    /// Length of the window, e.g. "10m".
    #[serde(with = "humantime_serde")]
    pub window: Duration,
    /// Failed Admin Token checks (401) one client IP may make per
    /// `admin_window`.
    pub admin_failed_auth: NonZeroU32,
    /// Length of the window of `admin_failed_auth`, e.g. "10m".
    #[serde(with = "humantime_serde")]
    pub admin_window: Duration,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            failed_lookups: NonZeroU32::new(20).expect("non-zero"),
            window: Duration::from_secs(10 * 60),
            admin_failed_auth: NonZeroU32::new(10).expect("non-zero"),
            admin_window: Duration::from_secs(10 * 60),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 8080,
            database_path: PathBuf::from("vota.db"),
            public_base_url: "http://localhost:8080/".parse().expect("valid default URL"),
            theme_folder: None,
            logo_folder: None,
            default_purge_gap: DEFAULT_PURGE_GAP,
            purge_interval: DEFAULT_PURGE_INTERVAL,
            max_date_horizon: DEFAULT_MAX_DATE_HORIZON,
            default_time_zone: Tz::UTC,
            trusted_proxies: Vec::new(),
            nat64_prefixes: Vec::new(),
            rate_limit: RateLimit::default(),
            connections: Connections::default(),
            admin_token_files: Vec::new(),
        }
    }
}

impl Config {
    /// Defaults, then the TOML file (if given), then `VOTA_*` env vars.
    pub fn load(file: Option<&Path>) -> anyhow::Result<Self> {
        let mut figment = Figment::from(Serialized::defaults(Config::default()));
        if let Some(file) = file {
            anyhow::ensure!(file.is_file(), "config file {} not found", file.display());
            figment = figment.merge(Toml::file_exact(file));
        }
        // Unlike the file, the environment is shared with everything else on
        // the host, so an unknown variable is only a warning, never its value.
        let options = Self::option_keys();
        for name in unknown_env_vars(&options) {
            tracing::warn!(
                "ignoring the environment variable {name}: it is not a configuration option"
            );
        }
        // A double underscore reaches into a table: `VOTA_RATE_LIMIT__WINDOW`.
        let env = Env::prefixed(ENV_PREFIX)
            .filter(move |key| is_option(&options, key.as_str()))
            .split("__");
        let config: Self = figment.merge(env).extract()?;
        anyhow::ensure!(
            !config.rate_limit.window.is_zero(),
            "rate_limit.window must be longer than zero"
        );
        anyhow::ensure!(
            !config.rate_limit.admin_window.is_zero(),
            "rate_limit.admin_window must be longer than zero"
        );
        anyhow::ensure!(
            !config.purge_interval.is_zero(),
            "purge_interval must be longer than zero"
        );
        anyhow::ensure!(
            !config.connections.header_read_timeout.is_zero(),
            "connections.header_read_timeout must be longer than zero"
        );
        anyhow::ensure!(
            !config.max_date_horizon.is_zero(),
            "max_date_horizon must be longer than zero"
        );
        Ok(config)
    }

    /// Every option as the tree of its keys, taken from the defaults so it
    /// can't drift from the fields: e.g. `{"port": 8080, "rate_limit":
    /// {"window": …}, …}`.
    fn option_keys() -> serde_json::Value {
        serde_json::to_value(Config::default()).expect("the defaults serialize")
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_address, self.port)
    }
}

/// `net` with an IPv4-mapped IPv6 network (`::ffff:a.b.c.d/n`, n ≥ 96, i.e.
/// wholly inside `::ffff:0:0/96`) given as the IPv4 network it maps
/// (`a.b.c.d/(n − 96)`). Peers and forwarded addresses are canonicalised to
/// IPv4 the same way, so such an entry would otherwise never match. Any other
/// network is returned as it is.
pub fn canonical_network(net: IpNet) -> IpNet {
    match net {
        IpNet::V6(v6) if v6.prefix_len() >= 96 => match v6.addr().to_ipv4_mapped() {
            Some(v4) => ipnet::Ipv4Net::new(v4, v6.prefix_len() - 96)
                .map(IpNet::V4)
                .unwrap_or(net),
            None => net,
        },
        _ => net,
    }
}

/// An IPv6 prefix that IPv4 addresses are embedded in as RFC 6052 §2.2
/// specifies, e.g. `64:ff9b:1:2::/64`: its length is 32, 40, 48, 56, 64 or 96,
/// it has no host bits set, and its bits 64 to 71 (the u-octet, part of a /96
/// prefix itself) are zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Nat64Prefix(Ipv6Net);

impl Nat64Prefix {
    /// The prefix lengths RFC 6052 §2.2 defines.
    pub const LENGTHS: [u8; 6] = [32, 40, 48, 56, 64, 96];

    pub fn net(&self) -> Ipv6Net {
        self.0
    }
}

impl std::str::FromStr for Nat64Prefix {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let net: IpNet = s.parse().map_err(|_| {
            anyhow::anyhow!(
                "a NAT64 prefix must be an IPv6 prefix in CIDR notation, \
                 such as \"64:ff9b:1:2::/64\": {s:?}"
            )
        })?;
        let IpNet::V6(net) = net else {
            anyhow::bail!("a NAT64 prefix must be an IPv6 prefix, not an IPv4 network: {s:?}");
        };
        anyhow::ensure!(
            Self::LENGTHS.contains(&net.prefix_len()),
            "a NAT64 prefix must have a prefix length of 32, 40, 48, 56, 64 or 96 \
             (RFC 6052): {s:?}"
        );
        anyhow::ensure!(
            net.trunc() == net,
            "a NAT64 prefix must not have host bits set: {s:?} (the prefix is {})",
            net.trunc()
        );
        // Bits 64 to 71: zero in shorter prefixes as host bits already.
        anyhow::ensure!(
            net.addr().octets()[8] == 0,
            "a NAT64 prefix must have bits 64 to 71 (the u-octet) set to zero \
             (RFC 6052): {s:?}"
        );
        Ok(Self(net))
    }
}

impl TryFrom<String> for Nat64Prefix {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Nat64Prefix> for String {
    fn from(prefix: Nat64Prefix) -> Self {
        prefix.0.to_string()
    }
}

/// The public URL the app is reached at, e.g. `https://example.org/polls/`:
/// the base of every URL it generates. Its path is the *base path* (`/`, or
/// e.g. `/polls/`), always with a leading and a trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PublicBaseUrl(Url);

impl PublicBaseUrl {
    /// The whole URL, ending in `/`.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The base path: `/`, or e.g. `/polls/`.
    pub fn base_path(&self) -> &str {
        self.0.path()
    }
}

impl std::str::FromStr for PublicBaseUrl {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // As the URL type's own deserializer words it, naming the value.
        let mut url = Url::parse(s).map_err(|error| anyhow::anyhow!("{error}: {s:?}"))?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "a public base URL must be an http:// or https:// URL"
        );
        anyhow::ensure!(
            url.query().is_none(),
            "a public base URL must not have a query (`?…`)"
        );
        anyhow::ensure!(
            url.fragment().is_none(),
            "a public base URL must not have a fragment (`#…`)"
        );
        // Only plain segments: the base path goes into routes, HTML
        // attributes and templates as it is, so it must need no escaping.
        let segments = url.path().trim_matches('/');
        anyhow::ensure!(
            segments.is_empty()
                || segments.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
                }),
            "a public base URL's base path consists of segments of ASCII letters, digits, \
             '-', '.', '_' and '~', separated by single '/'"
        );
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self(url))
    }
}

impl TryFrom<String> for PublicBaseUrl {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<PublicBaseUrl> for String {
    fn from(url: PublicBaseUrl) -> Self {
        url.0.into()
    }
}

/// Whether `key` (an environment variable name without [`ENV_PREFIX`], any
/// case) names an option of `options`: a top-level option, or one inside a
/// table after `__` (`RATE_LIMIT__WINDOW`). `CONFIG` (`VOTA_CONFIG`)
/// names the config file itself (see the CLI), not an option.
fn is_option(options: &serde_json::Value, key: &str) -> bool {
    let mut node = options;
    for segment in key.split("__") {
        match node.get(segment.to_ascii_lowercase()) {
            Some(child) => node = child,
            None => return false,
        }
    }
    true
}

/// `VOTA_*` variables that are not options but are known, so never
/// reported: the config file (`VOTA_CONFIG`) and the variables of the
/// `ballot` client (`VOTA_URL`, `VOTA_ADMIN_TOKEN`), which an
/// operator may well have set in the same shell.
const NOT_OPTIONS: [&str; 3] = ["CONFIG", "URL", "ADMIN_TOKEN"];

/// The names of the `VOTA_*` environment variables (prefix matched in any
/// case, as figment does) that are neither an option nor in [`NOT_OPTIONS`].
fn unknown_env_vars(options: &serde_json::Value) -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(name, _)| {
            let name = name.to_string_lossy().into_owned();
            // Trimmed, as figment reads it.
            let trimmed = name.trim();
            let prefix = trimmed.get(..ENV_PREFIX.len())?;
            if !prefix.eq_ignore_ascii_case(ENV_PREFIX) {
                return None;
            }
            let key = &trimmed[ENV_PREFIX.len()..];
            let known = NOT_OPTIONS
                .iter()
                .any(|name| key.eq_ignore_ascii_case(name))
                || is_option(options, key);
            (!known).then_some(name)
        })
        .collect()
}
