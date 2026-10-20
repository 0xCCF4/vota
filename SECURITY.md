# Security

This page is Vota's security policy, its threat model, a summary of its security design with links to the detailed pages, and a hardening checklist for operators.

- [Reporting a vulnerability](#reporting-a-vulnerability)
- [Supported versions](#supported-versions)
- [Threat model](#threat-model)
- [Security design](#security-design)
- [Operator hardening checklist](#operator-hardening-checklist)

## Reporting a vulnerability

Report vulnerabilities privately through GitHub's private vulnerability reporting: <https://github.com/0xCCF4/vota/security/advisories/new> (on the repository, the **Security** tab → **Report a vulnerability**). Only the maintainers see the report. **Don't open a public issue, pull request or discussion for a vulnerability.**

Please include the version (`vota --version`), how Vota is deployed (NixOS module or by hand, which reverse proxy), the steps to reproduce, and what an attacker gains. Please give us time to release a fix before you publish details.

**In scope:**

- the `vota` binary: the voting page, the voter API, the admin API, the CLI, the purge sweep, and the built-in templates, Themes and script;
- the NixOS module (`nix/module.nix`) and its `vota-admin-token` wrapper;
- the example configurations in the documentation (systemd unit, Caddy, nginx, backups), where following them as written leads to a weakness.

**Out of scope:**

- weaknesses of a particular deployment that the documentation warns against, e.g. a world-readable database outside the documented setup, a `trusted_proxies` entry that covers untrusted clients, or folder Themes writable by untrusted users;
- the reverse proxy, TLS, the operating system and other software Vota runs beside;
- folder Themes and logos, which are the operator's own content (see [Threat model](#threat-model));
- denial of service by sheer traffic volume, which the reverse proxy and the network have to absorb;
- anything that needs root, the service's user, or a valid Admin Token used as intended (the admin is trusted with the data, see below).

## Supported versions

Only the latest release gets security fixes. There are no backports to older releases.

| Version | Supported |
| ------- | --------- |
| 0.1.x (current) | yes |
| older | no |

## Threat model

Vota is meant to run as in the [deployment guide](docs/guides/deployment.md): one instance behind a reverse proxy on the same host, which terminates TLS, with Vota's port reachable only by that proxy. The operator and the admins are trusted; voters and everyone else are not. The 2026-10-18 audit used this model, with the actors below.

| Actor | What Vota defends against | What it doesn't |
| ----- | ----------------------------- | --------------- |
| **Anonymous internet** | Guessing or probing Ballot Tokens: 128 random bits, one byte-identical not-found for every failed lookup, and a per-client limit on failed lookups. The admin API without a valid Admin Token: every request is `401` (failed checks are rate-limited too). Script injection into pages, framing, cross-site requests (no cookies, no CORS). Slow or idle connections holding the server (connection limits and timeouts). | Floods by sheer volume, which the proxy and network must absorb. A client that shares its IP address (or IPv6 /64) with voters can use up the budget of failed lookups and lock them out until the window ends. |
| **A voter with one Ballot Token** | Reaching any other Ballot, or seeing what the admin keeps from voters (Group, Voter Reference, Purge Date, internal ID). Changing a Final Response, or answering after the Voting Deadline. Recording a Response against wording the voter never saw (the revision guard). | Whoever holds a Ballot Token *is* that Ballot's voter: a forwarded or leaked URL lets its holder vote. Vota can't tell the intended voter from someone else with the link. |
| **A holder of a revoked or expired Admin Token** | Revocation and expiry take effect on the next request, without a restart; such a token gets the same `401` as an unknown one, so it reveals nothing. Failed checks are rate-limited and logged per client IP. | Requests made with the token *before* it was revoked; the [audit log](docs/reference/admin-api.md#audit-log) shows what they were. |
| **An admin attacking voters** | Running script on the voting page: Markdown is sanitised, every value is escaped, the accent colour is a validated `#rrggbb`, Theme and logo are names from the server's registry, and a strict CSP backs this up. Changing a recorded result: content edits are refused once a Ballot is Answered or its Fallback has applied. Changing the question under a voter who is answering: the revision guard. | An admin can read every Response, Group and Voter Reference, delete any Ballot, and write a misleading question before anyone answers. Admins are trusted with the data. |
| **The operator** | Nothing: the operator is the root of trust. Whoever can run the CLI against the database creates Admin Tokens, and the configuration, the database and the Theme and logo folders are the operator's. | Folder Themes are the operator's code: a Theme template can put text where it isn't safe (e.g. the rendered description into an attribute), and a Theme's files are served as they are. |
| **Local users on the host** | Reading the database (which holds every live Ballot Token in plaintext): the app sets its umask to `077` and keeps the database, `-wal` and `-shm` at mode `0600`; the documented unit, the NixOS module and the backup commands keep directories and backups private. | Users who can write the Theme or logo folders (the app follows symlinks there), read the proxy's logs, read backups made without `umask 077`, or are root. |
| **Proxy spoofing** | A forged `X-Forwarded-For`: it is read only from a peer listed in `trusted_proxies`, from the right, skipping trusted hops; a chain that can't be read gets `400` and counts against no client. Forged `Host` or `X-Forwarded-Host` headers: generated URLs come only from `public_base_url`. `/healthz` answers only local, unproxied requests. | A `trusted_proxies` entry that covers networks untrusted clients connect from lets them choose their own client IP, and with it escape the rate limits. |

## Security design

### Ballot Tokens and Admin Tokens

- A **Ballot Token** is 128 bits from the operating system's CSPRNG, 22 base64url characters, checked for its exact format before any database lookup. It is the voter's only credential and is stored in plaintext, because the admin API returns the Ballot's URL. See [Ballot lifecycle](docs/concepts/ballot-lifecycle.md).
- An **Admin Token** is `vta_` plus 256 bits from the CSPRNG. It is printed once by the CLI; only its SHA-256 hash is stored. It may have an expiry (checked against the server's clock on every request) and is revoked by deleting it, which takes effect at once. There is no HTTP endpoint that creates tokens, and no token in the configuration. Every refusal is the same bare `401`. See [Admin Tokens](docs/concepts/admin-tokens.md) and [ADR 0001](docs/adr/0001-admin-api-shares-public-port.md).

### The uniform not-found

Malformed, unknown, deleted and purged Ballot Tokens, a percent-encoded spelling of a real one, and any other path below `/b/` and `/api/b/` all get one byte-identical `404` on the voting page and one on the voter API, headers included. A failed lookup tells nothing about whether a Ballot exists or ever existed. See [Uniform not-found](docs/concepts/not-found-and-rate-limiting.md#uniform-not-found).

### Rate limiting and trusted proxies

- **Failed Ballot Token lookups** are limited per client: by default 20 per 10 minutes, after which every Ballot request from that client gets `429` until the window ends. **Failed Admin Token checks** have their own budget (by default 10 per 10 minutes). Check and count are one atomic step, so concurrent requests can't overshoot the budget, and a blocked client can't evict its own block.
- **The client** is the TCP peer, or, if the peer is in [`trusted_proxies`](docs/reference/configuration.md#trusted_proxies), the right-most untrusted address in `X-Forwarded-For`. An unreadable chain from a trusted proxy gets `400` and is charged to no one. IPv4 addresses count one by one, IPv6 addresses per /64.
- **IPv4-embedded IPv6 addresses** count as the IPv4 address inside them: IPv4-mapped (`::ffff:a.b.c.d`), IPv4-translated and IPv4-compatible addresses, the NAT64 prefixes `64:ff9b::/96` and `64:ff9b:1::/48`, and the translator prefixes listed in [`nat64_prefixes`](docs/reference/configuration.md#nat64_prefixes) (e.g. Jool's `pool6`). So behind a NAT64 or SIIT translator every IPv4 client keeps its own budget instead of all of them sharing one /64. Trust decisions (`trusted_proxies`, the `/healthz` loopback check) never look inside these addresses, apart from the IPv4-mapped form: the embedded address is only as trustworthy as the translator.

See [Rate limiting and trusted proxies](docs/concepts/not-found-and-rate-limiting.md).

### The trust boundary and sanitization

Every value from a request (bodies, form fields, query strings, path segments, `X-Forwarded-For`) enters as an `UntrustedValue<T>`, and the only way to a domain type is a sanitizer in `src/input/`, which the compiler enforces. Text is NFC-normalised, trimmed, length-limited and refused if it contains control, invisible, private-use or unassigned characters or stacks of combining marks. Bodies are limited to 64 KiB, unknown fields are refused, and errors never echo the rejected value or an unknown field's name. Every SQL statement is static, with bound parameters. The crate forbids `unsafe` code. See [Trust boundary and sanitization](docs/concepts/trust-boundary.md).

### XSS defences and the Content Security Policy

Templates escape every value. Markdown descriptions are rendered without raw HTML and cleaned by `ammonia` against a short allowlist (links only to `http`, `https` and `mailto`). The accent colour is a validated `#rrggbb` and only ever the value of a CSS custom property. The voting page's script sets text with `textContent`, never HTML. Every themed page gets a fresh nonce and a CSP without inline scripts (`script-src 'self'`), without style attributes, and without any third-party source. SVG logos are served sandboxed. See [Security headers and Content Security Policy](docs/concepts/security-headers.md) and, for Theme authors, [CSP rules for Themes](docs/guides/themes-and-logos.md#content-security-policy-rules-for-themes).

### Security headers

Every response has `X-Content-Type-Options: nosniff`. HTML responses add `X-Frame-Options: DENY`, a `Permissions-Policy` that turns every powerful feature off, `Cross-Origin-Opener-Policy` and `Cross-Origin-Resource-Policy`, and `Referrer-Policy: no-referrer`, so a Ballot URL never leaks through `Referer`. Every voting page and voter API response is `Cache-Control: no-store`. HSTS is the reverse proxy's job (see the checklist). See [Security headers](docs/concepts/security-headers.md).

### Data integrity

- **Atomic Responses**: a Final Ballot's Response is one conditional write, so two parallel submissions store exactly one; the Voting Deadline is checked in the same write.
- **The revision guard**: every content edit bumps the Ballot's revision, which the voter's page and the voter API carry and send back; a Response is recorded only against the revision the voter saw, otherwise `409 ballot_changed`.
- **Edit locks**: content edits are refused once a Ballot has a Response or its Fallback has applied, in the same conditional write as the edit, so a recorded result never changes meaning.

See [Ballot lifecycle: concurrency guarantees](docs/concepts/ballot-lifecycle.md#concurrency-guarantees) and [the revision guard](docs/concepts/deadlines-fallback-purge.md#the-revision-guard-and-ballot_changed).

### Purge and secure delete

From its Purge Date on, a Ballot is gone for every request, even before the purge sweep deletes it. The sweep runs at startup and then every `purge_interval` (hourly by default). Every connection uses `PRAGMA secure_delete = ON`, and the write-ahead log is checkpointed and truncated after a sweep that deleted Ballots and after an admin `DELETE`. Backups, snapshots, copy-on-write filesystems and SSDs can still hold deleted data. See [Purge Date](docs/concepts/deadlines-fallback-purge.md#purge-date) and [the purge sweep](docs/guides/deployment.md#the-purge-sweep).

### Connection limits

At most `connections.max_open` connections (default 512) are served at once, a client must send its request headers within `connections.header_read_timeout` (default 10 s, also the idle timeout), and a shutdown waits at most `connections.shutdown_grace_period` (default 10 s). Only HTTP/1.1 is served. These protect the process if the port is reached directly, but the port is still meant for the proxy only. See [Configuration: `connections`](docs/reference/configuration.md#connections).

### Logging and the admin audit log

Vota never logs Admin Tokens, Ballot Tokens, Voter References or request bodies, and logs no request paths. Every admin request is logged as one event with the Admin Token's ID (not the token), the method, the route template, the status and the client IP; failed Admin Token checks are summed up per client IP in two warnings. `admin-token list` shows when each token was last used. See [Audit log](docs/reference/admin-api.md#audit-log). The reverse proxy's access log is another matter: see the checklist.

## Operator hardening checklist

The deployment guide's [checklist](docs/guides/deployment.md#checklist) has the full list. The security-relevant items:

- [ ] **File permissions and umask.** The database directory is `0700` and writable by the service user only (`StateDirectoryMode=0700` and `UMask=0077` in the unit); the app keeps the database files at `0600`. See [The database](docs/guides/deployment.md#the-database).
- [ ] **Backups** are made with `.backup` or `VACUUM INTO` under `umask 077` into a `0700` directory, kept no longer than needed, and restored as the service user's file. See [Backups](docs/guides/deployment.md#backups).
- [ ] **HTTPS only, with HSTS.** The proxy redirects HTTP to HTTPS and sends `Strict-Transport-Security`, so a Ballot URL never travels over plain HTTP. See [Reverse proxy](docs/guides/deployment.md#reverse-proxy).
- [ ] **Proxy access logs hold no Ballot Tokens.** Filter the tokens out of the access log, or log nothing for `/b/…` and `/api/b/…`. If you skip logging, cover the HTTP→HTTPS redirect too (a plain-HTTP site of its own in Caddy, `access_log off;` in nginx's port 80 server): the redirect logs the Ballot URL otherwise. Keep proxy logs, error logs included, readable by root and the proxy only, and their retention short. See [Access logs and Ballot Tokens](docs/guides/deployment.md#access-logs-and-ballot-tokens).
- [ ] **`trusted_proxies`** lists exactly the proxy's addresses, and the proxy sets or appends the client address in `X-Forwarded-For`. Behind Jool or another NAT64 or SIIT translator, list its prefix (`pool6`) in [`nat64_prefixes`](docs/reference/configuration.md#nat64_prefixes), and trust a proxy behind the translator by its IPv6 address. See [Behind Jool](docs/guides/deployment.md#behind-jool-or-another-nat64-translator).
- [ ] **Theme and logo folders are writable by root only**, including the directories above them and anything linked in: the app follows symlinks there and serves the files publicly. See [Folder permissions](docs/guides/themes-and-logos.md#folder-permissions).
- [ ] **The port is reachable only through the proxy**: `bind_address` stays on loopback, or the port is firewalled against everything but the proxy. Health probes run on the same host.
- [ ] **Admin Tokens** are one per person or script, labelled, given expiries where possible, and revoked when no longer needed; someone watches the audit log's warnings.
- [ ] **The `vota ballot` client** gets its Admin Token from `--token-file` or `VOTA_ADMIN_TOKEN`, never `--token`, which other local users can see in `ps` and which lands in the shell history. It talks to the public `https://` URL; `--insecure` (no certificate check) and `--allow-http` (plain HTTP to a host other than loopback) let anyone on the way read the token, so use `--ca-cert` for a private CA instead. See [CLI: `ballot`](docs/reference/cli.md#ballot).
- [ ] **On NixOS**, keep the [module's](docs/guides/deployment.md#nixos) defaults: a dynamic user with a `0700` state directory and `UMask=0077`, the systemd sandbox, `bind_address = "127.0.0.1"`, `openFirewall = false`, the health check on. Use the proxy examples with their log formats, since both proxy modules write access logs by default.
