# Deployment

Vota is one binary with one SQLite database. It serves the voting page, the voter API and the admin API on a single port and expects a reverse proxy in front of it for TLS. This guide covers running it by hand or under a service manager; NixOS gets its own module (see [NixOS](#nixos)).

- [Build](#build)
- [Configure and run](#configure-and-run)
- [Reverse proxy](#reverse-proxy)
- [Under a sub-path](#under-a-sub-path)
- [Access logs and Ballot Tokens](#access-logs-and-ballot-tokens)
- [The database](#the-database)
- [Backups](#backups)
- [Upgrades and migrations](#upgrades-and-migrations)
- [The purge sweep](#the-purge-sweep)
- [Stopping the server](#stopping-the-server)
- [Connection limits](#connection-limits)
- [Checklist](#checklist)
- [NixOS](#nixos)

## Build

```sh
cargo build --release          # → target/release/vota
# or
nix build                      # → result/bin/vota (runs the test suite while building)
```

Everything the app needs at runtime is embedded in the binary: migrations, built-in templates and Themes, the voting page's script. Only the configuration, the database and the optional theme and logo folders live outside it.

## Configure and run

Write a config file (all options: [configuration reference](../reference/configuration.md)):

```toml
# /etc/vota/vota.toml
bind_address = "127.0.0.1"
port = 8080
database_path = "/var/lib/vota/vota.db"
public_base_url = "https://ballots.example.org/"
default_time_zone = "Europe/Berlin"
trusted_proxies = ["127.0.0.1/32", "::1/128"]
```

Any option can also be set, or overridden, with an environment variable:

```sh
VOTA_CONFIG=/etc/vota/vota.toml VOTA_PORT=9000 vota serve
```

Create an Admin Token on the server, as the user that owns the database and with the same configuration:

```sh
vota --config /etc/vota/vota.toml admin-token create --label "survey scripts" --expires-in 180days
```

Logs go to stderr; set the level with `RUST_LOG` (default `info`). Admin Tokens, Ballot Tokens and Voter References never appear in Vota's logs. The reverse proxy's logs are another matter: see [Access logs and Ballot Tokens](#access-logs-and-ballot-tokens).

**Health check.** `GET /healthz` answers `{"now":"…","status":"ok"}` (the server's time, UTC) while the database responds, and `503` otherwise. It is for probes **on the same host** only: it answers only when the TCP peer is a loopback address (`127.0.0.0/8`, `::1`, or IPv4 loopback through an IPv6 listener) **and** the request has no `X-Forwarded-For` and no `Forwarded` header, whatever their value. Every other request, including one a local reverse proxy passes on, gets the same plain, empty `404` as an unknown path, so `/healthz` is invisible from outside. It answers at the root of the listener whatever the base path, and under a [sub-path](#under-a-sub-path) also at `<base path>healthz`. Probe the bind address, or loopback when Vota listens on all addresses (`0.0.0.0` or `::`); a specific non-loopback bind address can't be probed, because the probe's TCP peer is then that address:

```console
$ curl -s http://127.0.0.1:8080/healthz
{"now":"2026-10-18T10:15:00Z","status":"ok"}
$ curl -s -o /dev/null -w '%{http_code}\n' -H 'X-Forwarded-For: 192.0.2.1' http://127.0.0.1:8080/healthz
404
```

The [NixOS module](#health-check) uses it to wait for startup and to restart a service that stops answering.

A minimal systemd unit for a non-NixOS host:

```ini
# /etc/systemd/system/vota.service
[Unit]
Description=Vota
After=network.target

[Service]
ExecStart=/usr/local/bin/vota --config /etc/vota/vota.toml serve
# systemd stops with SIGTERM, on which Vota shuts down gracefully.
User=vota
Group=vota
StateDirectory=vota
# The database holds every live Ballot Token: only the service user may read it.
StateDirectoryMode=0700
UMask=0077
WorkingDirectory=/var/lib/vota
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Create the `vota` system user first and run the CLI as that user, e.g. `sudo -u vota vota --config /etc/vota/vota.toml admin-token list`. (This unit passes `systemd-analyze verify` but is a starting point, not a tested deployment. The [NixOS module](#nixos) ships a tested unit with more hardening, a good model for further `systemd.exec` settings.)

## Reverse proxy

Terminate TLS in the proxy and forward everything to Vota. Three things matter:

1. **Set [`public_base_url`](../reference/configuration.md#public_base_url) to the public URL**: the root of a host (e.g. `https://ballots.example.org/`, the examples below) or a path on a shared host (e.g. `https://example.org/polls/`, see [Under a sub-path](#under-a-sub-path)).
2. **Pass the client address** in `X-Forwarded-For`, appended to any value the client sent (as nginx does) or replacing it (as Caddy does), and list the proxy in [`trusted_proxies`](../reference/configuration.md#trusted_proxies). Otherwise the rate limiter sees every voter as the proxy's address, and one token guesser blocks everybody. See [Uniform not-found and rate limiting](../concepts/not-found-and-rate-limiting.md#the-client-ip-and-trusted-proxies).
3. **Serve HTTPS only.** Redirect plain HTTP to HTTPS, and send `Strict-Transport-Security` (HSTS) so that a browser that has been to the host once never again sends a Ballot URL, with its Ballot Token, over plain HTTP. Vota doesn't send HSTS itself, because only the proxy knows that the host is served over HTTPS. The examples use `max-age=31536000` (one year). Add `includeSubDomains` only if every subdomain of the host serves HTTPS as well. The redirect is a fallback: the Ballot URLs Vota hands out already start with `https://`, taken from `public_base_url`.

The proxy logs every request's URL, and Ballot URLs contain the Ballot Token. Strip the tokens from its access log or log nothing for those paths: see [Access logs and Ballot Tokens](#access-logs-and-ballot-tokens).

The admin API shares the host and port by design ([ADR 0001](../adr/0001-admin-api-shares-public-port.md)). You may additionally restrict `/admin` to certain networks in the proxy, but the Admin Token remains the actual protection.

### Caddy

```caddyfile
ballots.example.org {
	header Strict-Transport-Security "max-age=31536000"
	reverse_proxy 127.0.0.1:8080
}
```

Caddy obtains the certificate, redirects HTTP to HTTPS and sets `X-Forwarded-For` to the client's address by default (it drops any value the client sent, as long as Caddy's own `trusted_proxies` isn't configured). With Vota on the same host, `trusted_proxies = ["127.0.0.1/32", "::1/128"]`. (Validated with `caddy validate --adapter caddyfile`.)

### nginx

```nginx
server {
    listen 80;
    listen [::]:80;
    server_name ballots.example.org;
    return 301 https://ballots.example.org$request_uri;
}

server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name ballots.example.org;

    ssl_certificate     /etc/ssl/ballots.example.org/fullchain.pem;
    ssl_certificate_key /etc/ssl/ballots.example.org/privkey.pem;

    add_header Strict-Transport-Security "max-age=31536000" always;

    # Vota limits bodies to 64 KiB itself.
    client_max_body_size 128k;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

`$proxy_add_x_forwarded_for` appends the peer to the client's header; Vota reads the chain from the right and skips trusted proxies, so a forged left part doesn't help an attacker. A left part that isn't a list of bare IP addresses (e.g. `junk`, `unknown`, `203.0.113.5:4711`) makes the chain unreadable, and so does a chain of more than 32 addresses: that request gets `400` on the voting page, the voter API and the admin API, and counts toward no client's rate limit, so it harms only the client that sent it. (`/healthz` refuses any `X-Forwarded-For`, and the assets ignore it.) `always` adds HSTS to error responses too. nginx passes `add_header` down only to a `location` that has no `add_header` of its own, so repeat the HSTS line in any location where you add headers. (Validated with `nginx -t`.)

If the proxy runs on another machine, list its address instead, e.g. `trusted_proxies = ["10.0.0.5/32"]`, bind Vota to an address that proxy can reach, and firewall the port against everything else.

### Behind Jool or another NAT64 translator

If IPv4 clients reach Vota (or its proxy) through a NAT64 or SIIT translator such as Jool, each arrives from an IPv6 address with its IPv4 address embedded. Set [`nat64_prefixes`](../reference/configuration.md#nat64_prefixes) to the translator's prefix, Jool's `pool6`, e.g. `nat64_prefixes = ["2001:db8:64:1::/64"]`, so each such client is rate-limited as its own IPv4 address rather than all of them sharing one /64. The well-known prefix `64:ff9b::/96` and the local-use prefix `64:ff9b:1::/48` are decoded without an entry. A proxy that itself reaches Vota through the translator is trusted only by its IPv6 address: list that in `trusted_proxies`, not the IPv4 address inside it. See [IPv4-embedded addresses](../concepts/not-found-and-rate-limiting.md#ipv4-embedded-addresses-nat64-and-siit).

## Under a sub-path

Vota can share a host with other sites under a path prefix such as `/polls/`. Put the prefix in `public_base_url`:

```toml
public_base_url = "https://example.org/polls/"
```

Its path is the **base path** (`/polls/`; a missing trailing slash is added). **The proxy forwards requests with the prefix unchanged**, and Vota serves every route under it: the voting pages at `/polls/b/<token>`, the voter API at `/polls/api/b/…`, the admin API at `/polls/admin/…`, the assets at `/polls/assets/…`. Every URL it generates carries the prefix: Ballot URLs from the admin API (`https://example.org/polls/b/<token>`), `logo_url` in the voter API, the stylesheet, logo and script links on the pages, the script's API and page URLs, and the redirect after a form post. So the proxy needs only a path match, no rewriting, and the uniform not-found stays the same response for every token.

Outside the prefix Vota answers only `/healthz` (to local probes only, see [Health check](#configure-and-run)); anything else (e.g. `/b/<token>` without the prefix) is a plain, empty `404` that doesn't count toward the [rate limit](../concepts/not-found-and-rate-limiting.md). A proxy that forwards only the prefix never sends such requests anyway.

Caddy: `handle` (not `handle_path`, which would strip the prefix) sends `/polls/…` on as it is.

```caddyfile
example.org {
	header Strict-Transport-Security "max-age=31536000"
	handle /polls/* {
		reverse_proxy 127.0.0.1:8080
	}
	handle {
		root * /srv/www
		file_server
	}
}
```

nginx: a `proxy_pass` **without** a path after the address forwards the request URI unchanged. (`proxy_pass http://127.0.0.1:8080/;` with a trailing slash would replace `/polls/` with `/` and break every route.)

```nginx
server {
    listen 80;
    listen [::]:80;
    server_name example.org;
    return 301 https://example.org$request_uri;
}

server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name example.org;

    ssl_certificate     /etc/ssl/example.org/fullchain.pem;
    ssl_certificate_key /etc/ssl/example.org/privkey.pem;

    add_header Strict-Transport-Security "max-age=31536000" always;

    location /polls/ {
        # Vota limits bodies to 64 KiB itself.
        client_max_body_size 128k;
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location / {
        root /srv/www;
    }
}
```

`trusted_proxies` and `X-Forwarded-For` work as at the root. HSTS and the HTTPS redirect apply to the whole host, not just the prefix, so agree on them with the other sites on it. The Content Security Policy is origin-based (`'self'`), so it is the same under a prefix; other applications on the same host count as `'self'` too, which is the usual trade-off of sharing an origin. Folder Themes must link the app's URLs through the [`base_path` template variable](themes-and-logos.md#templates-and-their-variables) rather than as `/…`. (Both examples were validated with `caddy validate` and `nginx -t`; equivalent configurations on plain HTTP were run in front of a real `vota serve` with `public_base_url = "http://localhost:8091/polls/"`, where the voting page, its assets, the voter API, the no-JS form's redirect and the uniform not-found all worked through the proxy.)

## Access logs and Ballot Tokens

Vota never logs Ballot Tokens, but a reverse proxy logs the URL of every request, and the Ballot Token is part of the URL: `/b/<token>` for the voting page and its form posts, `/api/b/<token>` and `/api/b/<token>/response` for the page's script, and under a [sub-path](#under-a-sub-path) the same with the prefix in front (`/polls/b/<token>`, `/polls/api/b/<token>`). With a token, anyone can vote as its voter until the Purge Date. Proxy logs are often kept for months and readable by a whole group of users, such as `adm`. So:

- **strip the tokens from the access log**, or log nothing for those paths;
- **keep every proxy log readable by root and the proxy only**, error logs included.

The voting page sends no `Referer` header (`Referrer-Policy: no-referrer`), so the token doesn't get into other sites' logs or into the `Referer` field of this one. The no-JS form's redirect carries the token in its `Location` response header, which Caddy logs.

### Caddy

Caddy writes an access log only for a site with a `log` directive (the [NixOS module adds one](#reverse-proxy-on-nixos) to every virtual host). Filter the URI and the `Location` header in it:

```caddyfile
ballots.example.org {
	header Strict-Transport-Security "max-age=31536000"
	reverse_proxy 127.0.0.1:8080
	log {
		output file /var/log/caddy/ballots.example.org.log {
			mode 0600
		}
		format filter {
			request>uri regexp "/b/[^/?]*" "/b/REDACTED"
			resp_headers>Location regexp "/b/[^/?]*" "/b/REDACTED"
		}
	}
}
```

The pattern matches `/b/` anywhere in the path, so it covers `/api/b/…` and any sub-path prefix as they are. (It also shortens other `…/b/…` paths on a host shared with other sites.) A logged request then shows `"uri":"/polls/api/b/REDACTED/response"`.

Caddy logs a request that fails, e.g. while Vota isn't running, to its default log, the URI included. Filter that one too, in the global options at the top of the Caddyfile:

```caddyfile
{
	log default {
		format filter {
			request>uri regexp "/b/[^/?]*" "/b/REDACTED"
		}
	}
}
```

To log nothing for the Ballot paths instead, skip them in the site block (under a sub-path with the prefix: `path /polls/b/* /polls/api/b/*`):

```caddyfile
@ballot_tokens path /b/* /api/b/*
log_skip @ballot_tokens
```

`log_skip` covers only the HTTPS site. Caddy's automatic HTTP→HTTPS redirect logs the Ballot URL it redirects, so give the plain-HTTP side a site of its own, which has no `log` directive and so writes no access log, and keep the default-log filter above for failed requests:

```caddyfile
http://ballots.example.org {
	redir https://{host}{uri} 308
}
```

Caddy creates log files with mode `0600`; `mode 0600` above only makes that explicit. (These snippets were validated with `caddy validate`, and run in front of a test backend: with the filters, the access log and the error log showed `REDACTED` instead of the token in every Ballot path, at the root and under `/polls/`, in the redirect's `Location` and on the HTTP→HTTPS redirect; with `log_skip` and the plain-HTTP site, no token was logged.)

### nginx

nginx logs every request in its `combined` format by default, URI included. Define a format with the token replaced in the `http` block (e.g. in a file under `/etc/nginx/conf.d/`, which the usual `nginx.conf` includes there):

```nginx
# Ballot Tokens stay out of the access log.
map $request_uri $redacted_request_uri {
    "~^(.*?/b/)[^/?]*(.*)$" "$1REDACTED$2";
    default                 $request_uri;
}
log_format redacted '$remote_addr - $remote_user [$time_local] '
                    '"$request_method $redacted_request_uri $server_protocol" '
                    '$status $body_bytes_sent "$http_referer" "$http_user_agent"';
access_log /var/log/nginx/access.log redacted;
```

This is `combined` with `$redacted_request_uri` in place of the request line's URI. The `access_log` line makes it the format of every server that has no `access_log` of its own, the port 80 redirect included; a server or location with its own `access_log` must name `redacted` too. Like the Caddy filter, the pattern covers `/api/b/…` and any sub-path prefix.

To log nothing for the Ballot paths instead, give them a location of their own with `access_log off;` and the same proxy settings as `location /` (under a sub-path with the prefix: `~ ^/polls/(api/)?b/`):

```nginx
location ~ ^/(api/)?b/ {
    access_log off;
    client_max_body_size 128k;
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

Add `access_log off;` to the port 80 server too: its redirect logs the Ballot URL otherwise.

nginx's error log can't be filtered, and it names the request line, token included, when nginx can't reach Vota (`request: "GET /b/<token> HTTP/1.1"`): restrict it like the access log. (These snippets were validated with `nginx -t`, and run in front of a test backend: with the `redacted` format, the access log showed `REDACTED` instead of the token in every Ballot path, at the root and under `/polls/`, and on the port 80 redirect; with the skip location and `access_log off;` in the port 80 server, no token was logged.)

### Log files

- **Permissions.** The log directory `0750` and the files `0640` or tighter, owned by root or the proxy's user, with a group nobody else is in. On Debian and Ubuntu, nginx's logs are readable by the group `adm`, and logrotate creates new ones like that (`create 0640 www-data adm` in `/etc/logrotate.d/nginx`): change the group there, e.g. to `create 0640 www-data root`, and fix the existing files with `chgrp root /var/log/nginx/*.log`.
- **Retention.** A token in an old log stays usable until its Ballot's Purge Date. Keep logs, and their rotated copies, no longer than you need them.
- **Old logs.** Logs written before you added the filter still hold tokens: delete them, or keep them as restricted as the database.

## The database

- One SQLite file at [`database_path`](../reference/configuration.md#database_path), created on first start, plus `-wal` and `-shm` files next to it (WAL mode). The directory must be writable by the service user.
- **One instance per database.** The rate limiter's state is in memory, and running several servers on one file is not supported.
- The CLI (`admin-token …`) opens the same file directly; running it while the server runs is fine.
- **Permissions.** The file holds every live Ballot Token in plaintext, and with a token anyone can vote as its voter. `serve` and the CLI set umask 077 and make the database, `-wal` and `-shm` mode `0600` whenever they open it, also if they existed with wider modes (a file they can't change is logged as a warning). Keep the directory `0700` too (`StateDirectoryMode=0700` in the unit above), and back it up the same way (below).
- What it holds: Ballots with Options, Responses, Fallbacks, Groups and Voter References, and Admin Token *hashes*. Ballots disappear at their Purge Date. Treat the file and its backups as personal data: Voter References and Responses are exactly what Vota keeps away from everyone else.

## Backups

Don't copy the live file with `cp` while the server runs: recent writes may still be in the `-wal` file. Use SQLite's online backup instead, which is safe while the server is running. A backup holds the Ballot Tokens like the database, so create it readable by its owner only: with `umask 077`, into a directory of mode `0700`:

```sh
umask 077
install -d -m 0700 /var/backups/vota
sqlite3 /var/lib/vota/vota.db ".backup /var/backups/vota/vota-$(date +%F).db"
# or, compacted:
sqlite3 /var/lib/vota/vota.db "VACUUM INTO '/var/backups/vota/vota-$(date +%F).db'"
```

(Without `umask 077`, `sqlite3` creates the copy with the shell's umask, typically `0644`: readable by every local user. `install -d -m 0700` also tightens an existing directory.)

Check a backup:

```sh
sqlite3 /var/backups/vota/vota-2026-10-17.db "PRAGMA integrity_check;"
# ok
```

To restore, stop the server, replace `vota.db` with the backup (and remove any leftover `vota.db-wal` and `vota.db-shm`), make it the service user's (`chown vota:vota /var/lib/vota/vota.db && chmod 600 /var/lib/vota/vota.db`), and start it again. (These commands were run against a live server; the restored copy listed its Admin Token and Ballots as expected.)

Keep in mind that a backup preserves Ballots past their Purge Date: restoring an old backup brings them back until the next sweep deletes them again (at startup), and backups themselves are only purged by your own retention policy. Choose backup retention with the Purge Dates in mind.

## Upgrades and migrations

1. Back up the database (above).
2. Replace the binary.
3. Restart. Pending migrations (`migrations/*.sql`, embedded in the binary) are applied automatically at startup, before the server binds its port. The CLI applies them too, so whichever runs first migrates.

Migrations only move forward. An older binary refuses a database that a newer one has migrated:

```text
Error: applying database migrations

Caused by:
    migration 10 was previously applied but is missing in the resolved migrations
```

To roll back, restore the backup taken before the upgrade.

Folder Themes are trial-rendered at every start, so a Theme that relied on behaviour a new version changed fails loudly at startup rather than for voters. Check release notes for template variable changes and test Themes against the new binary by starting it on the same configuration with a throwaway database and any free port, e.g. `VOTA_DATABASE_PATH=/tmp/theme-check.db VOTA_PORT=0 vota --config … serve`: it fails before `listening on …` if a Theme or logo is broken; otherwise stop it with Ctrl-C. (`admin-token` commands don't load Themes, so they can't be used for this check.)

## The purge sweep

`serve` deletes Ballots past their Purge Date once at startup and then every [`purge_interval`](../reference/configuration.md#purge_interval) (default `1h`), logging only how many it deleted:

```text
2026-10-20T11:09:22.473637Z  INFO vota::jobs::purge_sweep: purge sweep deleted Ballots past their Purge Date deleted=1
```

Between sweeps such Ballots are already invisible on every route. A failed sweep is logged as a warning and retried at the next interval. The admin-token CLI does not sweep. SQLite reuses the freed pages; run `VACUUM` occasionally (server stopped) if you need the file to shrink.

**Deleted means deleted from the files.** Every database connection runs with `PRAGMA secure_delete = ON`, so SQLite overwrites deleted content (purged and deleted Ballots, replaced Responses) with zeros instead of leaving it in free pages. After a sweep that deleted Ballots, and after an admin `DELETE`, the write-ahead log is checkpointed into the database and truncated (`wal_checkpoint(TRUNCATE)`), so the old page images in the `-wal` file go too. (If a long read keeps the checkpoint from completing, that is logged as a warning and the next one catches up.) What this can't reach:

- **backups and copies** taken before, which keep everything until your retention deletes them (see [Backups](#backups));
- **the write-ahead log between checkpoints**: text an admin edit replaced, or a voter's earlier Response on an Editable Ballot, stays in the `-wal` file until the next checkpoint after a sweep or `DELETE` (or until SQLite overwrites that part of the log);
- **filesystem and volume snapshots**, and copy-on-write filesystems (Btrfs, ZFS) that keep old blocks until they are reused;
- **the storage device itself**: SSDs remap blocks, so overwritten data may survive in blocks the drive no longer exposes. Full-disk encryption is the answer there.

## Stopping the server

`vota serve` shuts down gracefully on **SIGTERM** (what systemd, Docker and most supervisors send) and on **SIGINT** (Ctrl-C): it stops accepting connections, finishes running requests and closes kept-alive connections, stops the purge sweep (a sweep already running finishes first) and exits with status 0. No special stop signal needs to be configured. It logs:

```text
INFO vota::cli::serve: received SIGTERM, shutting down gracefully
INFO vota::cli::serve: purge sweep stopped; shut down
```

It waits for open connections at most [`connections.shutdown_grace_period`](../reference/configuration.md#connections) (default `10s`): a client that has sent only half a request, for example, can't hold the shutdown up beyond it. Connections still open then are dropped, with a warning:

```text
WARN vota::cli::serve::connections: connections still open after the shutdown grace period; closing them open=1
```

The database stays consistent even if the process is killed (e.g. `SIGKILL` after a stop timeout), because every write is a SQLite transaction; only requests in flight are lost then.

## Connection limits

The listener limits what clients can hold ([`[connections]`](../reference/configuration.md#connections)): at most `max_open` connections at once (default 512; beyond it the server stops accepting until one closes), and a `header_read_timeout` (default `10s`) after which a client that hasn't sent its request headers, or leaves a kept-alive connection idle, is disconnected. These protect the process if the port is reached directly, but the design is still that **only the reverse proxy reaches it**: the proxy buffers slow clients and terminates TLS. With a proxy in front, the defaults rarely matter; raise `max_open` only together with the process's file-descriptor limit (`LimitNOFILE=` in a systemd unit; the NixOS module sets it to `max_open + 512`).

## Checklist

- [ ] `public_base_url` is the public HTTPS URL: the root of the host, or the [sub-path](#under-a-sub-path) the proxy forwards unchanged.
- [ ] `bind_address` is loopback (or firewalled) so only the proxy reaches the port; the [connection limits](#connection-limits) are a backstop, not a substitute.
- [ ] `trusted_proxies` lists exactly the proxy's addresses, and the proxy sets or appends the client address in `X-Forwarded-For`.
- [ ] The proxy redirects HTTP to HTTPS and sends `Strict-Transport-Security`.
- [ ] The proxy's access log has no Ballot Tokens ([filtered or skipped](#access-logs-and-ballot-tokens)), and its logs are readable by root and the proxy only.
- [ ] The theme and logo folders, if set, are writable by root only ([why](themes-and-logos.md#folder-permissions)).
- [ ] Health probes run on the same host and reach `/healthz` directly (not through the proxy), e.g. `curl http://127.0.0.1:8080/healthz`.
- [ ] `default_time_zone` is set for your voters.
- [ ] The database directory is writable by the service user only (`0700`, `UMask=0077`) and backed up with `.backup` / `VACUUM INTO` under `umask 077` into a `0700` directory.
- [ ] Admin Tokens are labelled, scoped to people or scripts, and given expiries where possible.
- [ ] Someone watches the journal for the [audit log](../reference/admin-api.md#audit-log)'s `vota::admin_audit` warnings (failed Admin Token checks).

## NixOS

The flake exports a NixOS module, `nixosModules.default` (also `nixosModules.vota`), that runs Vota as a hardened systemd service `vota.service` and puts a `vota-admin-token` command and the `vota` binary itself (for its admin API client `vota ballot`, with shell completions) on the system path. Everything in the sections above still applies; this section covers what the module does for you.

### Enable the service

Add the flake as an input of your system flake and import the module:

```nix
# flake.nix of your system
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Placeholder: replace with the URL you got Vota from.
    vota.url = "github:0xCCF4/vota";
  };

  outputs =
    { nixpkgs, vota, ... }:
    {
      nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          vota.nixosModules.default
          ./configuration.nix
        ];
      };
    };
}
```

Then configure it, e.g. in `configuration.nix`:

```nix
{
  services.vota = {
    enable = true;
    settings = {
      public_base_url = "https://ballots.example.org/";
      default_time_zone = "Europe/Berlin";
      # The reverse proxy on this host.
      trusted_proxies = [ "127.0.0.1/32" "::1/128" ];
      # Optional; a Nix path is copied to the store.
      logo_folder = ./logos;
      default_purge_gap = "2weeks";
      rate_limit.failed_lookups = 10;
    };
  };
}
```

`nixos-rebuild switch` starts the service, which applies the database migrations and listens on `127.0.0.1:8080`. The module writes `settings` to a TOML file in the Nix store and runs `vota --config <that file> serve`. For the example above, the file is:

```toml
bind_address = "127.0.0.1"
database_path = "/var/lib/vota/vota.db"
default_purge_gap = "2weeks"
default_time_zone = "Europe/Berlin"
logo_folder = "/nix/store/disixdc9l3p9yzfzgsai065ps4rnzqw0-logos"
max_date_horizon = "1y"
nat64_prefixes = []
port = 8080
public_base_url = "https://ballots.example.org/"
purge_interval = "1h"
trusted_proxies = ["127.0.0.1/32", "::1/128"]

[connections]
header_read_timeout = "10s"
max_open = 512
shutdown_grace_period = "10s"

[rate_limit]
admin_failed_auth = 10
admin_window = "10m"
failed_lookups = 10
window = "10m"
```

The configuration holds no secret, so it may live in the world-readable store; Admin Tokens live only in the database. A change of `settings` restarts the service on the next `nixos-rebuild switch`, which also re-reads the Theme and logo folders.

### Options

| Option | Type | Default | Meaning |
| ------ | ---- | ------- | ------- |
| `services.vota.enable` | boolean | `false` | Run the service and install `vota-admin-token` and `vota` (the package, with its completions). |
| `services.vota.package` | package | this flake's `vota` | The Vota package. |
| `services.vota.adminTokenFiles` | list of strings (absolute paths outside the store) | `[ ]` | Files with one pre-provisioned Admin Token each, e.g. agenix secrets; passed in as systemd credentials. See [Admin Tokens from files (agenix)](#admin-tokens-from-files-agenix) and [`admin_token_files`](../reference/configuration.md#admin_token_files). |
| `services.vota.openFirewall` | boolean | `false` | Open `settings.port` in the firewall. Only needed when a proxy on another machine connects; then also set `settings.bind_address`. |
| `services.vota.healthCheck.enable` | boolean | `true` | The [health check](#health-check): startup waits for `/healthz`, and a timer restarts the service when it stops answering. Needs a loopback or wildcard `settings.bind_address` (checked at evaluation). |
| `services.vota.healthCheck.interval` | string (systemd time span) | `"1min"` | How often the timer probes `/healthz`. |
| `services.vota.settings.public_base_url` | string | **required** | [`public_base_url`](../reference/configuration.md#public_base_url) |
| `services.vota.settings.bind_address` | string | `"127.0.0.1"` | [`bind_address`](../reference/configuration.md#bind_address) |
| `services.vota.settings.port` | port | `8080` | [`port`](../reference/configuration.md#port) |
| `services.vota.settings.database_path` | string | `"/var/lib/vota/vota.db"` | [`database_path`](../reference/configuration.md#database_path); must lie directly in `/var/lib/vota` (checked at evaluation) |
| `services.vota.settings.theme_folder` | null or path | `null` | [`theme_folder`](../reference/configuration.md#theme_folder) |
| `services.vota.settings.logo_folder` | null or path | `null` | [`logo_folder`](../reference/configuration.md#logo_folder) |
| `services.vota.settings.default_purge_gap` | string (duration) | `"4weeks"` | [`default_purge_gap`](../reference/configuration.md#default_purge_gap) |
| `services.vota.settings.purge_interval` | string (duration) | `"1h"` | [`purge_interval`](../reference/configuration.md#purge_interval) |
| `services.vota.settings.max_date_horizon` | string (duration) | `"1y"` | [`max_date_horizon`](../reference/configuration.md#max_date_horizon) |
| `services.vota.settings.default_time_zone` | string | `config.time.timeZone`, or `"UTC"` when that is `null` | [`default_time_zone`](../reference/configuration.md#default_time_zone): the voting page's zone, and the zone of admin dates given without an offset |
| `services.vota.settings.trusted_proxies` | list of strings | `[ ]` | [`trusted_proxies`](../reference/configuration.md#trusted_proxies) |
| `services.vota.settings.nat64_prefixes` | list of strings (IPv6 prefixes of length 32, 40, 48, 56, 64 or 96) | `[ ]` | [`nat64_prefixes`](../reference/configuration.md#nat64_prefixes) |
| `services.vota.settings.rate_limit.failed_lookups` | integer, 1–4294967295 | `20` | [`rate_limit.failed_lookups`](../reference/configuration.md#rate_limit) |
| `services.vota.settings.rate_limit.window` | string (duration) | `"10m"` | [`rate_limit.window`](../reference/configuration.md#rate_limit) |
| `services.vota.settings.rate_limit.admin_failed_auth` | integer, 1–4294967295 | `10` | [`rate_limit.admin_failed_auth`](../reference/configuration.md#rate_limit) |
| `services.vota.settings.rate_limit.admin_window` | string (duration) | `"10m"` | [`rate_limit.admin_window`](../reference/configuration.md#rate_limit) |
| `services.vota.settings.connections.max_open` | integer, 1–1000000 | `512` | [`connections.max_open`](../reference/configuration.md#connections); the unit's `LimitNOFILE` is this plus 512 |
| `services.vota.settings.connections.header_read_timeout` | string (duration) | `"10s"` | [`connections.header_read_timeout`](../reference/configuration.md#connections) |
| `services.vota.settings.connections.shutdown_grace_period` | string (duration) | `"10s"` | [`connections.shutdown_grace_period`](../reference/configuration.md#connections); keep it below `TimeoutStopSec` (90 s) |

`settings` is freeform: the keys are exactly those of the [configuration reference](../reference/configuration.md), values are checked by the app at startup (a bad duration or time zone makes the unit fail, with the error in `journalctl -u vota`), and an unknown key is passed through and then stops the service. `theme_folder` and `logo_folder` take a Nix path (`./themes`, copied to the store) or an absolute path as a string (`"/etc/vota/themes"`). A string path must be readable by any user, writable by root only (the app follows symlinks in it, see [Folder permissions](themes-and-logos.md#folder-permissions)), and must not lie under `/home`, `/root` or `/run/user`, which the service can't see; if a configured folder is missing, the service fails to start and the journal names the folder. A port below 1024 is possible: the module then grants the service `CAP_NET_BIND_SERVICE` and turns off `PrivateUsers`.

### Creating Admin Tokens

`vota-admin-token` runs `vota --config <the service's config file> admin-token …` with any arguments you give it, as the service's user and in the same sandbox, against the service's database. Run it as root; it works whether or not the service is running. From the VM test, where a token `vm-test` already existed:

```console
$ sudo vota-admin-token create --label "survey scripts" --expires-in 180days
Created Admin Token 2. It is shown only this once; store it securely.
vta_XEVBC9gGy8uHz6wlJPAHJUKh5zB3SIezB43Yp21bCtQ
$ sudo vota-admin-token list
ID  CREATED               EXPIRES               EXPIRED  LAST-USED             LABEL
1   2026-10-20T11:12:28Z  never                 no       2026-10-20T11:12:46Z  vm-test
2   2026-10-20T11:13:00Z  2027-04-18T11:13:00Z  no       never                 survey scripts
$ sudo vota-admin-token revoke --label "survey scripts"
Revoked Admin Tokens 2.
```

(`vm-test` had just created a Ballot, hence its `LAST-USED`; see [`admin-token list`](../reference/cli.md#admin-token-list).)

As with the plain CLI, only the token goes to stdout, so `TOKEN=$(sudo vota-admin-token create --label ci)` captures it. The exit code is the CLI's (see the [CLI reference](../reference/cli.md#exit-codes)). Arguments reach the CLI literally: the wrapper passes `--expand-environment=no`, so a label such as `'Bob $HOME'` is stored as typed, not with `$HOME` expanded by the service manager. Under the hood the wrapper is `systemd-run --pipe --wait` of a transient unit with the same `DynamicUser`, `StateDirectory` and hardening settings as `vota.service`, so it gets the same user and can write the database, and never a file root owns. Don't run `vota` against `/var/lib/vota/vota.db` directly as root.

### Admin Tokens from files (agenix)

To provision an Admin Token declaratively instead of creating it with `vota-admin-token`, put it in a secret and list the secret in `services.vota.adminTokenFiles`. With [agenix](https://github.com/ryantm/agenix):

```sh
# On your workstation: a fresh token (or use the shell one-liner of the CLI reference) ...
vota admin-token generate
# ... pasted into the secret in the editor agenix opens.
agenix -e secrets/vota-admin-token.age
```

```nix
# configuration.nix
age.secrets.vota-admin-token.file = ./secrets/vota-admin-token.age;  # root-owned, 0400

services.vota = {
  enable = true;
  settings.public_base_url = "https://ballots.example.org/";
  adminTokenFiles = [ config.age.secrets.vota-admin-token.path ];  # "/run/agenix/vota-admin-token"
};
```

The trailing newline the editor may add is trimmed. The service then accepts that token for the admin API, e.g. `VOTA_ADMIN_TOKEN=… vota ballot list`.

How it works: the module adds one `LoadCredential=vota-admin-token-<n>:<path>` per entry (`n` counted from 1), so systemd, as root, copies each file into the service's credentials directory, where only the service's dynamic user can read it. A root-owned `0400` secret therefore needs no looser permissions. The module points the app at the copies with `VOTA_ADMIN_TOKEN_FILES=[%d/vota-admin-token-1,…]` (`%d` is the credentials directory, `/run/credentials/vota.service`). The audit log names the n-th entry's token `token_id=file:<n>`.

- Give **string paths** (as agenix's `.path` is). A Nix path such as `./token` is refused by the option's type, because it would copy the secret into the world-readable Nix store, and so is a string inside `/nix/store` (checked at evaluation, with relative paths and paths with whitespace). `services.vota.settings.admin_token_files` is refused too: it would bypass the credentials.
- The files are read when the service starts. A missing or malformed file makes the unit fail, and `journalctl -u vota` names the file (never its contents); make sure the secret exists before `vota.service` starts (agenix decrypts its secrets during activation, before services start).
- **Rotate or revoke** by changing or removing the secret and restarting `vota.service`. Changing the list restarts it on `nixos-rebuild switch`; a changed secret behind the same path does not, so run `systemctl restart vota` after it. `vota-admin-token list` and `revoke` don't see these tokens.

### The ballot client and shell completions

The module puts the package on the system path, so the admin API client [`vota ballot`](../reference/cli.md#ballot) is there for any user; it needs no access to the service's files, only the URL and an Admin Token. The package installs completion scripts for bash, zsh and fish (`share/bash-completion/completions/vota.bash`, `share/zsh/site-functions/_vota`, `share/fish/vendor_completions.d/vota.fish`): bash picks them up through `programs.bash.completion.enable` (on by default), zsh and fish when `programs.zsh.enable` or `programs.fish.enable` is set.

Use the public URL (`https://…`). On the server itself, the loopback port works too, with the sub-path. From the VM test, with a token created by `vota-admin-token` (see [Creating Admin Tokens](#creating-admin-tokens)):

```console
$ export VOTA_URL=http://localhost:8080/polls VOTA_ADMIN_TOKEN="$TOKEN"
$ vota ballot create --title 'Lunch on Friday?' --option Pizza --option Sushi \
    --voting-deadline 2026-10-27T11:12:48Z --group vm-client --voter-reference alice
http://localhost:8080/polls/b/…
Created Ballot 2: Lunch on Friday?
Status:          unanswered
Voting Deadline: 2026-10-27T11:12:48Z
Purge Date:      2026-11-24T11:12:48Z
$ vota ballot list --group vm-client
ID  STATUS      GROUP      VOTER-REFERENCE  VOTING-DEADLINE       RESULT
2   unanswered  vm-client  alice            2026-10-27T11:12:48Z  -
```

(The VM's `public_base_url` is `http://localhost:8080/polls/`; Ballot 1 had been created with `curl`.)

Keep the token in a file readable only by you and pass it with `--token-file`, or set `VOTA_ADMIN_TOKEN`; `--token` shows it to other users in `ps`. The client never needs root.

### What the unit looks like

- **User and state.** `DynamicUser=yes` with the name `vota`: the service runs under a UID allocated at start and has no account on the system. `StateDirectory=vota` provides `/var/lib/vota` (a link to `/var/lib/private/vota`, mode `0700`), the only place the service can write; the database and its `-wal`/`-shm` files live there. `UMask=0077`.
- **Hardening.** `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `PrivateDevices=yes`, `PrivateUsers=yes` (off for a port below 1024, since a user namespace would take the capability away again), `NoNewPrivileges=yes`, `ProtectKernelTunables=yes`, `ProtectKernelModules=yes`, `ProtectKernelLogs=yes`, `ProtectControlGroups=yes`, `ProtectClock=yes`, `ProtectHostname=yes`, `ProtectProc=invisible`, `ProcSubset=pid`, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`, `RestrictNamespaces=yes`, `RestrictRealtime=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`, `RemoveIPC=yes`, `SystemCallArchitectures=native`, `SystemCallFilter=@system-service ~@privileged ~@resources`, `SystemCallErrorNumber=EPERM`, and an empty `CapabilityBoundingSet=` (only `CAP_NET_BIND_SERVICE` for a port below 1024). A configured Theme or logo folder is readable, everything else outside the state directory read-only or hidden.
- **Lifecycle.** `Restart=on-failure`; `systemctl stop vota` sends SIGTERM, on which Vota [shuts down gracefully](#stopping-the-server) within `settings.connections.shutdown_grace_period` and exits with status 0. Logs go to the journal: `journalctl -u vota`. The unit sets `NO_COLOR=1`, and logs are plain text in the journal.
- **Credentials.** With `adminTokenFiles`, one `LoadCredential=vota-admin-token-<n>:<path>` per file and `Environment=VOTA_ADMIN_TOKEN_FILES=[%d/vota-admin-token-1,…]` (see [Admin Tokens from files](#admin-tokens-from-files-agenix)). The `vota-admin-token` wrapper gets neither: the `admin-token` commands don't read token files.
- **Limits.** `LimitNOFILE=` is `settings.connections.max_open + 512` (1024 by default), so the connection cap is reached before the descriptor limit.

`systemd-analyze security vota.service` rates the unit `1.1 OK` (as measured in the VM test).

### Health check

With `healthCheck.enable` (the default) the service checks itself through a local [`/healthz`](#configure-and-run) request. The probe is `curl` to `http://<bind address>:<port><base path>healthz`, e.g. `http://127.0.0.1:8080/polls/healthz` under the sub-path `/polls/`; a wildcard bind address (`0.0.0.0` or `::`) is probed via `127.0.0.1` or `[::1]`. `/healthz` answers only loopback peers, so a specific non-loopback `bind_address` fails evaluation unless the health check is turned off.

- **Startup.** `ExecStartPost` polls `/healthz` every half second, inside the service's sandbox, until it answers. Only then is `vota.service` active, so `systemctl start` and units ordered after it wait for a server that really answers. If it doesn't answer within 60 seconds, the start fails and `Restart=on-failure` tries again.
- **While running.** The timer `vota-health-check.timer` starts `vota-health-check.service` every `healthCheck.interval`. If `vota.service` is active, it probes `/healthz` up to three times (5 seconds timeout each, 2 seconds apart). If none answers, it logs that and runs `systemctl try-restart vota.service`. The timer is started and stopped with the service, and `try-restart` restarts only a running service, so `systemctl stop vota` is never undone by the check, not even by one that was probing when the service was stopped. The check runs as root, because it restarts the service, but with no capabilities, a read-only file system, network access to localhost only (`IPAddressAllow=localhost`), and most of the service's kernel, namespace and system call restrictions (`SystemCallFilter=@system-service`).

The VM test freezes the server with `kill -STOP`: the port stays open but nothing answers. With `healthCheck.interval = "5s"`, the journal (`journalctl -u vota -u vota-health-check`) shows:

```text
vota: http://127.0.0.1:8080/polls/healthz did not answer (attempt 1 of 3)
vota: http://127.0.0.1:8080/polls/healthz did not answer (attempt 2 of 3)
vota: http://127.0.0.1:8080/polls/healthz did not answer (attempt 3 of 3)
vota: restarting vota.service
Stopping Vota...
INFO vota::cli::serve: received SIGTERM, shutting down gracefully
Started Vota.
```

systemd sends `SIGCONT` after `SIGTERM`, so even a stopped process shuts down gracefully.

### Reverse proxy on NixOS

Run the proxy on the same host and keep `bind_address` on loopback. With [Caddy](#caddy), which redirects HTTP to HTTPS by itself:

```nix
{
  services.caddy = {
    enable = true;
    virtualHosts."ballots.example.org" = {
      extraConfig = ''
        header Strict-Transport-Security "max-age=31536000"
        reverse_proxy 127.0.0.1:8080
      '';
      # The module logs every request of a virtual host to
      # /var/log/caddy/access-<host>.log; keep the Ballot Tokens out.
      logFormat = ''
        output file /var/log/caddy/access-ballots.example.org.log
        format filter {
          request>uri regexp "/b/[^/?]*" "/b/REDACTED"
          resp_headers>Location regexp "/b/[^/?]*" "/b/REDACTED"
        }
      '';
    };
    # The default log (errors only, to the journal) also names failed requests.
    logFormat = ''
      level ERROR
      format filter {
        request>uri regexp "/b/[^/?]*" "/b/REDACTED"
      }
    '';
  };
  networking.firewall.allowedTCPPorts = [ 80 443 ];
}
```

With [nginx](#nginx) and an ACME certificate; `forceSSL` redirects HTTP to HTTPS, and `recommendedProxySettings` sets `Host`, `X-Forwarded-For` (appending, `$proxy_add_x_forwarded_for`) and `X-Forwarded-Proto`:

```nix
{
  services.nginx = {
    enable = true;
    recommendedProxySettings = true;
    # The module logs every request to /var/log/nginx/access.log;
    # keep the Ballot Tokens out (see "Access logs and Ballot Tokens").
    commonHttpConfig = ''
      map $request_uri $redacted_request_uri {
          "~^(.*?/b/)[^/?]*(.*)$" "$1REDACTED$2";
          default                 $request_uri;
      }
      log_format redacted '$remote_addr - $remote_user [$time_local] '
                          '"$request_method $redacted_request_uri $server_protocol" '
                          '$status $body_bytes_sent "$http_referer" "$http_user_agent"';
      access_log /var/log/nginx/access.log redacted;
    '';
    virtualHosts."ballots.example.org" = {
      enableACME = true;
      forceSSL = true;
      extraConfig = ''
        add_header Strict-Transport-Security "max-age=31536000" always;
      '';
      locations."/" = {
        proxyPass = "http://127.0.0.1:8080";
        extraConfig = "client_max_body_size 128k;";
      };
    };
  };
  security.acme = {
    acceptTerms = true;
    defaults.email = "admin@example.org";
  };
  networking.firewall.allowedTCPPorts = [ 80 443 ];
}
```

Both proxies add the client address to `X-Forwarded-For`, so set `settings.trusted_proxies = [ "127.0.0.1/32" "::1/128" ];` as in the example above.

Both modules write access logs out of the box, which is why the examples above set a log format; see [Access logs and Ballot Tokens](#access-logs-and-ballot-tokens). The file permissions are already restrictive: Caddy's log files are `0600`, owned by `caddy`; nginx's are `0640` in a `0750` directory, owned by `nginx`, and logrotate keeps them for 26 weeks. The default log of either module (Caddy's errors, nginx's error log) goes to the journal, which members of groups such as `systemd-journal` and `wheel` can read; that is why the Caddy example filters it too.

**Under a sub-path** (see [Under a sub-path](#under-a-sub-path)), set `settings.public_base_url = "https://example.org/polls/";` and forward the prefix unchanged: in Caddy with `handle` (not `handle_path`), in nginx with a `proxyPass` that has **no** path after the address. The filters above cover the prefix as they are, but Caddy's `logFormat` is set per virtual host, so the shared host needs its own:

```nix
{
  services.caddy.virtualHosts."example.org" = {
    extraConfig = ''
      header Strict-Transport-Security "max-age=31536000"
      handle /polls/* {
        reverse_proxy 127.0.0.1:8080
      }
      handle {
        root * /srv/www
        file_server
      }
    '';
    logFormat = ''
      output file /var/log/caddy/access-example.org.log
      format filter {
        request>uri regexp "/b/[^/?]*" "/b/REDACTED"
        resp_headers>Location regexp "/b/[^/?]*" "/b/REDACTED"
      }
    '';
  };

  # or with nginx:
  services.nginx.virtualHosts."example.org" = {
    enableACME = true;
    forceSSL = true;
    extraConfig = ''
      add_header Strict-Transport-Security "max-age=31536000" always;
    '';
    root = "/srv/www";
    locations."/polls/" = {
      proxyPass = "http://127.0.0.1:8080"; # not "http://127.0.0.1:8080/"
      extraConfig = "client_max_body_size 128k;";
    };
  };
}
```

(All examples in this section were evaluated against the module with `nixpkgs.lib.nixosSystem`; the generated Caddyfiles pass `caddy validate`, and the generated `nginx.conf` files build, which includes nixpkgs' gixy check, and fail `nginx -t` only for the missing ACME certificates.)

### Backups and upgrades on NixOS

Back up `/var/lib/vota/vota.db` as root while the service runs, as in [Backups](#backups): root's shell has umask `022` on NixOS, so set `umask 077` and use a `0700` directory, or the copy is readable by every local user:

```sh
sudo sh -c 'umask 077 && install -d -m 0700 /var/backups/vota && \
  sqlite3 /var/lib/vota/vota.db ".backup /var/backups/vota/vota-$(date +%F).db"'
```

(`sqlite3` is in `pkgs.sqlite`.) To restore, `systemctl stop vota`, replace the file (and remove leftover `-wal`/`-shm` files), then `chown --reference=/var/lib/vota /var/lib/vota/vota.db` and `chmod 600` it before `systemctl start vota`. An upgrade is a new flake input revision and `nixos-rebuild switch`; migrations are applied when the service restarts, as in [Upgrades and migrations](#upgrades-and-migrations).

### The VM test

The flake's checks include a NixOS VM test of the module (`checks.x86_64-linux.nixos-vm`, in [`nix/vm-test.nix`](../../nix/vm-test.nix)). It boots a VM with the service under the sub-path `/polls/` and checks: the unit is active and answers `/healthz`; every migration is applied; a token from `vota-admin-token` creates a Ballot through `POST /polls/admin/ballots`; `vota ballot` on the PATH, with that token in `VOTA_ADMIN_TOKEN`, creates and lists a Ballot under `/polls/` (and refuses to run without a token), and its bash completions are installed; a local Voting Deadline (no offset) given through `vota ballot` is read in the system's zone (`time.timeZone = "Europe/Berlin"`, which `default_time_zone` follows by default); a token from `adminTokenFiles` (a root-owned `0400` file) works for `vota ballot list` and is audited as `token_id=file:1` but never logged or listed by `vota-admin-token`, while a bogus token gets `401`; the returned Ballot URL serves the voting page and the voter API returns the Ballot with its logo; the service runs as a dynamic user that can write only to its state directory; `vota-admin-token` passes `$HOME`, `${HOME}` and an unset `$VAR` in a label literally; `LimitNOFILE` follows `connections.max_open` and a client that sends nothing is disconnected after `connections.header_read_timeout`; `systemctl stop` ends it gracefully with exit status 0 and no warning or error in the journal; the database survives a restart; admin requests appear in the audit log (with the token's ID, never the token), `admin-token list --json` records the token's last use, and a client behind the proxy that fails the Admin Token check `rate_limit.admin_failed_auth` times gets `429` while other clients go on; `/healthz` refuses requests with `X-Forwarded-For` or `Forwarded`; startup waits for `/healthz` under the base path and the periodic check passes on a healthy service; a frozen service (`kill -STOP`) is restarted by the check and answers again under a new PID. Run it with `nix flake check`, or alone with `nix build .#checks.x86_64-linux.nixos-vm -L`. It uses KVM when available and otherwise runs emulated, which is slower: without KVM the test script takes about three minutes on a 16-core machine (plus the package build, if it isn't cached).
