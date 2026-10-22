# Configuration reference

Vota reads its configuration in three layers. Each later layer overrides the earlier ones:

1. built-in defaults;
2. a TOML file, if one is given with `--config <FILE>` or `VOTA_CONFIG=<FILE>`;
3. environment variables `VOTA_<OPTION>`.

Every server command (`serve` and `admin-token …`) loads the configuration the same way. The admin API client [`ballot …`](cli.md#ballot) and [`completions`](cli.md#completions) load none: they ignore `--config` and `VOTA_CONFIG`. No option holds a secret: Admin Tokens live in the database, or in files that [`admin_token_files`](#admin_token_files) names (see [Admin Tokens](../concepts/admin-tokens.md)).

## Environment variables

The variable name is `VOTA_` plus the option name in upper case: `port` → `VOTA_PORT`. Options inside a table are reached with a **double underscore**: `rate_limit.window` → `VOTA_RATE_LIMIT__WINDOW`. Values are parsed like TOML values, so a list is written in brackets:

```sh
VOTA_PORT=9000
VOTA_DEFAULT_PURGE_GAP="2 weeks"
VOTA_TRUSTED_PROXIES='["127.0.0.1/32", "10.0.0.0/8"]'
VOTA_ADMIN_TOKEN_FILES='["/run/agenix/vota-admin-token"]'
VOTA_RATE_LIMIT__FAILED_LOOKUPS=5
VOTA_RATE_LIMIT__WINDOW=30m
VOTA_CONNECTIONS__MAX_OPEN=256
```

`VOTA_CONFIG` names the config file and is not an option itself. Neither are the two variables of the [`ballot` client](cli.md#ballot), which may be set in the same shell: `VOTA_URL` (the server's public base URL, like `--url`) and `VOTA_ADMIN_TOKEN` (the Admin Token, used when neither `--token` nor `--token-file` is given). The server commands ignore them without a warning. **Any other `VOTA_*` variable that is not an option is ignored, with one warning per variable** that names it but never shows its value. This includes unknown keys inside a table, such as `VOTA_RATE_LIMIT__BURST`. The prefix is matched in any case. For example, `VOTA_LOG=debug vota --config vota.toml admin-token list` still runs and logs on stderr:

```text
2026-10-20T11:08:51.836668Z  WARN vota::config: ignoring the environment variable VOTA_LOG: it is not a configuration option
```

So a misspelt variable doesn't stop the server, but its setting doesn't apply either: watch for this warning. Use `RUST_LOG` for the log level (see [Logging](#logging)). The **config file stays strict**: an unknown key there is a startup error (see [Startup errors](#startup-errors)).

## Logging

Logs go to stderr; stdout carries only command output, such as a new Admin Token. The level is set with the standard `RUST_LOG` variable, not with a `VOTA_*` option: the default is `info`, e.g. `RUST_LOG=debug` or `RUST_LOG=info,vota::admin_audit=warn`. A `RUST_LOG` value that can't be parsed falls back to `info`. Output is coloured only when stderr is a terminal, so a journal or a log file gets plain text. See the [CLI reference](cli.md).

## NixOS module

On NixOS, the [`services.vota` module](../guides/deployment.md#nixos) writes the config file from `services.vota.settings`: every option below is `services.vota.settings.<option>` with the same name, type and default (the **NixOS** row of each option), e.g. `services.vota.settings.rate_limit.window = "30m";`. Three defaults differ: `public_base_url` has none and must be set, `database_path` defaults to `/var/lib/vota/vota.db` in the service's state directory, and `default_time_zone` defaults to the system's `time.timeZone` (`UTC` when that is `null`, i.e. when the zone is set imperatively with `timedatectl`). Options left at `null` (`theme_folder`, `logo_folder`) are left out of the file. `admin_token_files` is the exception: it is set through `services.vota.adminTokenFiles`, which passes the files in as systemd credentials and sets `VOTA_ADMIN_TOKEN_FILES` (see [`admin_token_files`](#admin_token_files)); `services.vota.settings.admin_token_files` is refused. Otherwise the module sets no `VOTA_*` environment variables (only `NO_COLOR=1`).

## Example file

```toml
# vota.toml
bind_address = "127.0.0.1"
port = 8080
database_path = "/var/lib/vota/vota.db"
public_base_url = "https://ballots.example.org/"
theme_folder = "/etc/vota/themes"
logo_folder = "/etc/vota/logos"
default_purge_gap = "4weeks"
purge_interval = "1h"
max_date_horizon = "1y"
default_time_zone = "Europe/Berlin"
trusted_proxies = ["127.0.0.1/32", "::1/128"]
nat64_prefixes = []
admin_token_files = []

[rate_limit]
failed_lookups = 20
window = "10m"
admin_failed_auth = 10
admin_window = "10m"

[connections]
max_open = 512
header_read_timeout = "10s"
shutdown_grace_period = "10s"
```

Relative paths (`database_path`, `theme_folder`, `logo_folder`, `admin_token_files`) are resolved against the **working directory of the process**, not against the config file's location.

Durations use [humantime](https://docs.rs/humantime/latest/humantime/fn.parse_duration.html) syntax: `30m`, `1h`, `12h`, `4weeks`, `2 weeks`, `90days`, `1y` (365.25 days), `1y 6months`.

## Options

### `bind_address`

| | |
| --- | --- |
| Env | `VOTA_BIND_ADDRESS` |
| NixOS | `services.vota.settings.bind_address` |
| Type | IP address (IPv4 or IPv6, no port) |
| Default | `127.0.0.1` |
| Example | `bind_address = "0.0.0.0"` |

The address the single listener binds to. Voter UI, voter API, admin API and assets all share it. Keep it on loopback when a reverse proxy runs on the same host. The port is meant to be reachable by the proxy only: the [connection limits](#connections) bound what a client can hold, but the proxy is what buffers slow clients and absorbs floods.

### `port`

| | |
| --- | --- |
| Env | `VOTA_PORT` |
| NixOS | `services.vota.settings.port` |
| Type | integer, 0–65535 |
| Default | `8080` |
| Example | `port = 9000` |

### `database_path`

| | |
| --- | --- |
| Env | `VOTA_DATABASE_PATH` |
| NixOS | `services.vota.settings.database_path`, default `/var/lib/vota/vota.db` (must lie in that directory) |
| Type | file path |
| Default | `vota.db` (in the working directory) |
| Example | `database_path = "/var/lib/vota/vota.db"` |

The SQLite database file. It is created if it doesn't exist, and pending migrations are applied at startup. SQLite also keeps `-wal` and `-shm` files next to it, so the directory must be writable. The file holds every live Ballot Token in plaintext: `serve` and the `admin-token` commands run with umask 077 and set the database, `-wal` and `-shm` to mode `0600` when they open it (a file they can't change is logged as a warning, naming it). See [deployment](../guides/deployment.md#the-database).

### `public_base_url`

| | |
| --- | --- |
| Env | `VOTA_PUBLIC_BASE_URL` |
| NixOS | `services.vota.settings.public_base_url`, required (no default) |
| Type | absolute `http://` or `https://` URL, without query or fragment |
| Default | `http://localhost:8080/` |
| Example | `public_base_url = "https://ballots.example.org/"`, or under a sub-path `public_base_url = "https://example.org/polls/"` |

The public URL Vota is reached at. It is the base of every Ballot URL the admin API returns (`<public_base_url>b/<token>`, e.g. `https://example.org/polls/b/<token>`) and of `logo_url` in the voter API. It is taken only from configuration, never from request headers, so returned links can't be spoofed.

Its path is the **base path**: `/` at the root of a host, or e.g. `/polls/`. A missing trailing slash is added (`https://example.org/polls` is the same as `https://example.org/polls/`). Every route is served under the base path, and the reverse proxy forwards requests with the prefix unchanged; every URL the app generates (Ballot URLs, `logo_url`, the pages' asset and script links, the script's API and page URLs, redirects) carries it, and Themes get it as the [`base_path` template variable](../guides/themes-and-logos.md#templates-and-their-variables). Outside the base path only `/healthz` answers; anything else there is a plain `404` that doesn't count toward the rate limit. See [Under a sub-path](../guides/deployment.md#under-a-sub-path) for proxy examples.

Startup fails, naming the option, for a URL that isn't `http://` or `https://`, has a query (`?…`) or a fragment (`#…`), or whose path has empty segments (`//`) or characters other than ASCII letters, digits, `-`, `.`, `_` and `~` between its slashes (the base path is used in routes, HTML and templates as it is). With the root path `/`, everything behaves as before sub-paths were supported.

### `theme_folder`

| | |
| --- | --- |
| Env | `VOTA_THEME_FOLDER` |
| NixOS | `services.vota.settings.theme_folder` |
| Type | directory path |
| Default | none (built-in Themes only) |
| Example | `theme_folder = "/etc/vota/themes"` |

Additional Themes, one subdirectory per Theme. A folder Theme named like a built-in one replaces it. The folder is read once at startup, so restart to pick up changes. If it is set, it must exist, and every Theme in it must load and pass the trial render, or startup fails. See the [Themes and logos guide](../guides/themes-and-logos.md).

### `logo_folder`

| | |
| --- | --- |
| Env | `VOTA_LOGO_FOLDER` |
| NixOS | `services.vota.settings.logo_folder` |
| Type | directory path |
| Default | none (no logos) |
| Example | `logo_folder = "/etc/vota/logos"` |

Logos selectable with the `logo` Theme Tweak: files `<name>.svg`, `<name>.png` or `<name>.webp`, at most 512 KiB each. Read once at startup. If it is set, it must exist. See the [Themes and logos guide](../guides/themes-and-logos.md#logos).

### `default_purge_gap`

| | |
| --- | --- |
| Env | `VOTA_DEFAULT_PURGE_GAP` |
| NixOS | `services.vota.settings.default_purge_gap` |
| Type | duration |
| Default | `4weeks` |
| Example | `default_purge_gap = "2weeks"` |

When a Ballot is created without `purge_date`, its Purge Date is its Voting Deadline plus this gap. `0s` is allowed (Purge Date = Voting Deadline). The computed date is not capped by `max_date_horizon`.

### `purge_interval`

| | |
| --- | --- |
| Env | `VOTA_PURGE_INTERVAL` |
| NixOS | `services.vota.settings.purge_interval` |
| Type | duration, greater than zero |
| Default | `1h` |
| Example | `purge_interval = "15m"` |

How often the purge sweep deletes Ballots past their Purge Date. It also runs once at startup. Between sweeps, such Ballots are already treated as gone on every request, so this only decides how long their rows linger in the database file.

### `max_date_horizon`

| | |
| --- | --- |
| Env | `VOTA_MAX_DATE_HORIZON` |
| NixOS | `services.vota.settings.max_date_horizon` |
| Type | duration, greater than zero |
| Default | `1y` (365.25 days) |
| Example | `max_date_horizon = "2y"` |

How far ahead of now an admin may set a Voting Deadline or Purge Date, on create, edit and extend. A date exactly at the horizon is accepted. Purge Dates the app computes for creation (the default gap) or for an extension (the shift) are not capped. A Purge Date shifted by an edit of the Voting Deadline is capped: an edit that would shift it past the horizon is refused (see [PATCH](admin-api.md#field-semantics-on-edit)).

### `default_time_zone`

| | |
| --- | --- |
| Env | `VOTA_DEFAULT_TIME_ZONE` |
| NixOS | `services.vota.settings.default_time_zone` (default: the system's `time.timeZone`, or `UTC` when that is unset) |
| Type | IANA time zone name |
| Default | `UTC` |
| Example | `default_time_zone = "Europe/Berlin"` |

The server's local time zone, for two things:

- The voting page shows the Voting Deadline in it without JavaScript, e.g. `Tue 1 Dec 2026, 13:00 CET`. With JavaScript, the page shows the voter's local time instead, without a zone name (e.g. `Tomorrow, 13:00`).
- The admin API reads a Voting Deadline or Purge Date given **without** a UTC offset (`2026-12-01T12:00`) as local time in it; one with an offset is read as given. A local time skipped or repeated by a daylight-saving change is refused. See [Conventions](admin-api.md#conventions).

The APIs' answers are always in UTC. On NixOS the module's default is the system's zone instead of `UTC` (see [NixOS module](#nixos-module)).

### `trusted_proxies`

| | |
| --- | --- |
| Env | `VOTA_TRUSTED_PROXIES` |
| NixOS | `services.vota.settings.trusted_proxies` |
| Type | list of IP networks in CIDR notation |
| Default | `[]` (no proxy trusted) |
| Example | `trusted_proxies = ["127.0.0.1/32", "::1/128"]` |

Peers whose `X-Forwarded-For` header is believed when the client IP is determined for rate limiting (of failed Ballot Token lookups and of failed Admin Token checks) and for the admin [audit log](admin-api.md#audit-log). Every entry needs a prefix length: write `127.0.0.1/32`, not `127.0.0.1` (a bare address is a startup error: `invalid IP address syntax`).

An entry in IPv4-mapped IPv6 form is taken as the IPv4 network it maps: `::ffff:10.0.0.0/104` means `10.0.0.0/8`, `::ffff:127.0.0.1/128` means `127.0.0.1/32`. (Peers and forwarded addresses in that form are compared as IPv4 too, so such an entry would otherwise never match.) This applies to entries with a prefix length of 96 or more; a shorter IPv6 prefix such as `::/0` stays an IPv6 network and doesn't match IPv4 peers. See [Uniform not-found and rate limiting](../concepts/not-found-and-rate-limiting.md#the-client-ip-and-trusted-proxies).

### `nat64_prefixes`

| | |
| --- | --- |
| Env | `VOTA_NAT64_PREFIXES` |
| NixOS | `services.vota.settings.nat64_prefixes` |
| Type | list of IPv6 prefixes in CIDR notation, of length 32, 40, 48, 56, 64 or 96, without host bits set and with bits 64–71 (the u-octet) zero |
| Default | `[]` (only the well-known prefixes) |
| Example | `nat64_prefixes = ["2001:db8:64:1::/64"]` |

The IPv6 prefixes of your NAT64 or SIIT translator, e.g. Jool's `pool6`. A client address in one of them counts as the IPv4 address embedded in it, decoded as RFC 6052 §2.2 places it for the prefix length (skipping bits 64–71): for the rate limits and the `client_ip` of the admin [audit log](admin-api.md#audit-log). With `2001:db8:64:1::/64`, the client `2001:db8:64:1:c6:3364:700::` counts as `198.51.100.7`. The well-known prefix `64:ff9b::/96` and the local-use prefix `64:ff9b:1::/48` are always decoded and need no entry. Matching against [`trusted_proxies`](#trusted_proxies) and the local check of `/healthz` never look inside these addresses. See [IPv4-embedded addresses](../concepts/not-found-and-rate-limiting.md#ipv4-embedded-addresses-nat64-and-siit).

Every entry is checked at startup. RFC 6052 §2.2 requires bits 64–71 (the u-octet) to be zero; in a /96 prefix they are part of the prefix itself, so an entry such as `2001:db8:0:0:ff00::/96` is refused. Real examples (from `VOTA_NAT64_PREFIXES`; from the file, the key reads e.g. `"default.nat64_prefixes.0" in vota.toml TOML file`):

```text
Error: a NAT64 prefix must have a prefix length of 32, 40, 48, 56, 64 or 96 (RFC 6052): "64:ff9b::/80" for key "NAT64_PREFIXES.0" in `VOTA_` environment variable(s)
Error: a NAT64 prefix must be an IPv6 prefix, not an IPv4 network: "192.0.2.0/24" for key "NAT64_PREFIXES.0" in `VOTA_` environment variable(s)
Error: a NAT64 prefix must not have host bits set: "64:ff9b::1/96" (the prefix is 64:ff9b::/96) for key "NAT64_PREFIXES.0" in `VOTA_` environment variable(s)
Error: a NAT64 prefix must have bits 64 to 71 (the u-octet) set to zero (RFC 6052): "2001:db8:0:0:ff00::/96" for key "NAT64_PREFIXES.0" in `VOTA_` environment variable(s)
Error: a NAT64 prefix must be an IPv6 prefix in CIDR notation, such as "64:ff9b:1:2::/64": "64:ff9b::" for key "NAT64_PREFIXES.0" in `VOTA_` environment variable(s)
```

### `rate_limit`

A table with four options. It limits failed Ballot Token lookups and, with a separate budget, failed Admin Token checks per client IP.

| Option | Env | NixOS | Type | Default | Example |
| ------ | --- | ----- | ---- | ------- | ------- |
| `rate_limit.failed_lookups` | `VOTA_RATE_LIMIT__FAILED_LOOKUPS` | `services.vota.settings.rate_limit.failed_lookups` | integer, 1–4294967295 | `20` | `failed_lookups = 5` |
| `rate_limit.window` | `VOTA_RATE_LIMIT__WINDOW` | `services.vota.settings.rate_limit.window` | duration, greater than zero | `10m` | `window = "30m"` |
| `rate_limit.admin_failed_auth` | `VOTA_RATE_LIMIT__ADMIN_FAILED_AUTH` | `services.vota.settings.rate_limit.admin_failed_auth` | integer, 1–4294967295 | `10` | `admin_failed_auth = 5` |
| `rate_limit.admin_window` | `VOTA_RATE_LIMIT__ADMIN_WINDOW` | `services.vota.settings.rate_limit.admin_window` | duration, greater than zero | `10m` | `admin_window = "1h"` |

```toml
[rate_limit]
failed_lookups = 20
window = "10m"
admin_failed_auth = 10
admin_window = "10m"
```

A client may make `failed_lookups` failed lookups (uniform 404s on the voting page or voter API) per `window`, a fixed window starting at its first failure. After that, every Ballot Token request from it gets `429` until the window ends. See [Uniform not-found and rate limiting](../concepts/not-found-and-rate-limiting.md).

Separately, a client may fail the Admin Token check (`401`) `admin_failed_auth` times per `admin_window`. After that, every admin request from it gets `429` until that window ends, even with a valid Admin Token. Requests that pass the check never count, and neither budget counts toward the other. See [Admin API: rate limit](admin-api.md#rate-limit-on-failed-admin-token-checks).

### `connections`

A table with three options. It bounds what clients can hold on the listener: file descriptors, memory, and the time a shutdown waits.

| Option | Env | NixOS | Type | Default | Example |
| ------ | --- | ----- | ---- | ------- | ------- |
| `connections.max_open` | `VOTA_CONNECTIONS__MAX_OPEN` | `services.vota.settings.connections.max_open` | integer, 1–4294967295 (NixOS: up to 1000000) | `512` | `max_open = 256` |
| `connections.header_read_timeout` | `VOTA_CONNECTIONS__HEADER_READ_TIMEOUT` | `services.vota.settings.connections.header_read_timeout` | duration, greater than zero | `10s` | `header_read_timeout = "30s"` |
| `connections.shutdown_grace_period` | `VOTA_CONNECTIONS__SHUTDOWN_GRACE_PERIOD` | `services.vota.settings.connections.shutdown_grace_period` | duration | `10s` | `shutdown_grace_period = "30s"` |

```toml
[connections]
max_open = 512
header_read_timeout = "10s"
shutdown_grace_period = "10s"
```

- **`max_open`**: how many connections are served at once. At the limit the server stops accepting; further connections wait in the operating system's listen queue until one closes. Each connection is a file descriptor, so keep it well below the process's `ulimit -n` (often 1024; the NixOS module sets `LimitNOFILE` to `max_open + 512`).
- **`header_read_timeout`**: how long a client may take to send a request's headers, counted from when the server starts waiting for them: on a new connection, and on a kept-alive connection after the previous response. A client that sends nothing, sends its headers too slowly, or leaves a kept-alive connection idle is disconnected after it, so this is also the idle (keep-alive) timeout. A reverse proxy sends headers at once, so a few seconds suffice.
- **`shutdown_grace_period`**: on SIGTERM or SIGINT the server stops accepting, asks every connection to close after its current request, and waits at most this long. Connections still open then (e.g. a client that sent half a request) are dropped, with one warning, and the shutdown completes. `0s` drops them at once. Keep it below the service manager's stop timeout (systemd: `TimeoutStopSec`, 90 s by default).

Only HTTP/1.1 is served (no HTTP/2), with at most 100 headers and about 400 KB of them per request.

### `admin_token_files`

| | |
| --- | --- |
| Env | `VOTA_ADMIN_TOKEN_FILES` |
| NixOS | `services.vota.adminTokenFiles` (not under `settings`; see below) |
| Type | list of file paths |
| Default | `[]` (only the database's Admin Tokens) |
| Example | `admin_token_files = ["/run/credentials/vota.service/admin-token"]` |

Files that each hold one pre-provisioned [Admin Token](../concepts/admin-tokens.md#admin-tokens-from-files), for a host that provisions its secrets declaratively (e.g. agenix or sops-nix). The admin API accepts these tokens beside the ones `vota admin-token create` stores. Make one with [`vota admin-token generate`](cli.md#admin-token-generate).

- **Format**: the file holds exactly one Admin Token, `vta_` and 43 base64url characters, as `generate` prints it. Surrounding whitespace, such as the trailing newline, is trimmed; anything else (a second token, a comment, `=` padding), or more than 4 KiB in all, makes the file malformed.
- **When read**: once, when `serve` starts; only the SHA-256 hash of each token is kept in memory. A changed file takes effect at the next start, and removing a file from the list revokes its token at the next start. The `admin-token` commands never read the files.
- **Errors**: a missing, unreadable or malformed file stops `serve` with exit code 1 and an error that names the file but never its contents (see [Startup errors](#startup-errors)).
- **Not in the database**: `admin-token list` doesn't show these tokens and `admin-token revoke` can't revoke them. They have no label, no expiry and no recorded last use. The [audit log](admin-api.md#audit-log) names the token of the n-th file (counted from 1) as `token_id=file:<n>`.
- **In the environment**, a list in brackets. Quote entries that hold `,`, `[`, `]` or spaces; others may stay unquoted, as the NixOS module passes them (`VOTA_ADMIN_TOKEN_FILES=[%d/vota-admin-token-1]`, with systemd's credentials directory for `%d`). A single path without brackets is refused (`expected a sequence`).
- **Permissions**: the file must be readable by the server's user and by nobody else. Under systemd, pass a root-only secret in with `LoadCredential=` and name the file in the credentials directory (`$CREDENTIALS_DIRECTORY`, `/run/credentials/<unit>/`); the NixOS option `services.vota.adminTokenFiles` does this for you. See [Deployment: NixOS](../guides/deployment.md#admin-tokens-from-files-agenix).

## Startup errors

Configuration problems stop every command with exit code 1 and a message naming the key: an unknown key in the config file, or an invalid value in the file or in a known environment variable. (Unknown environment variables only log a warning; see [Environment variables](#environment-variables).) Real examples:

```text
Error: config file nope.toml not found
Error: unknown field: found `foo`, expected `one of `bind_address`, `port`, `database_path`, `public_base_url`, `theme_folder`, `logo_folder`, `default_purge_gap`, `purge_interval`, `max_date_horizon`, `default_time_zone`, `trusted_proxies`, `nat64_prefixes`, `rate_limit`, `connections`, `admin_token_files`` for key "default.foo" in t.toml TOML file
Error: invalid IP address syntax for key "default.trusted_proxies.0" in t.toml TOML file
Error: a NAT64 prefix must be an IPv6 prefix, not an IPv4 network: "10.0.0.0/8" for key "default.nat64_prefixes.0" in t.toml TOML file
Error: invalid type: found string "127.0.0.1/32", expected a sequence for key "TRUSTED_PROXIES" in `VOTA_` environment variable(s)
Error: failed to parse timezone: 'Mars/Base' for key "DEFAULT_TIME_ZONE" in `VOTA_` environment variable(s)
Error: invalid type: found string "abc", expected u16 for key "PORT" in `VOTA_` environment variable(s)
Error: relative URL without a base: "notaurl" for key "PUBLIC_BASE_URL" in `VOTA_` environment variable(s)
Error: a public base URL must not have a query (`?…`) for key "PUBLIC_BASE_URL" in `VOTA_` environment variable(s)
Error: a public base URL's base path consists of segments of ASCII letters, digits, '-', '.', '_' and '~', separated by single '/' for key "default.public_base_url" in t.toml TOML file
Error: invalid value unsigned int `0`, expected a nonzero u32 for key "RATE_LIMIT.FAILED_LOOKUPS" in `VOTA_` environment variable(s)
Error: unknown field: found `burst`, expected `one of `failed_lookups`, `window`, `admin_failed_auth`, `admin_window`` for key "default.rate_limit.burst" in t.toml TOML file
Error: a public base URL must be an http:// or https:// URL for key "PUBLIC_BASE_URL" in `VOTA_` environment variable(s)
Error: purge_interval must be longer than zero
Error: rate_limit.window must be longer than zero
Error: rate_limit.admin_window must be longer than zero
Error: max_date_horizon must be longer than zero
Error: invalid value unsigned int `0`, expected a nonzero u32 for key "CONNECTIONS.MAX_OPEN" in `VOTA_` environment variable(s)
Error: connections.header_read_timeout must be longer than zero
```

A theme or logo folder that is missing or broken (e.g. `Error: reading the theme folder /nonexistent`) stops only `serve`, and so does an Admin Token file of [`admin_token_files`](#admin_token_files) that is missing, unreadable or malformed. The `admin-token` commands read neither. Real examples (the cause on the following lines; never the file's contents):

```text
Error: admin_token_files: cannot read the Admin Token file /run/agenix/missing

Caused by:
    No such file or directory (os error 2)
Error: admin_token_files: the Admin Token file /etc/vota/admin-token is malformed

Caused by:
    it must hold exactly one Admin Token: vta_ and 43 base64url characters, as `vota admin-token generate` prints it
```

Theme and logo errors are listed in the [Themes and logos guide](../guides/themes-and-logos.md#startup-checks-and-errors).
