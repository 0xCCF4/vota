# CLI reference

`vota` is a single binary. It runs the web server, manages Admin Tokens, and is a client for the admin API of a running server.

```text
vota [--config <FILE>] serve
vota [--config <FILE>] admin-token create [--label <LABEL>] [--expires-in <DURATION> | --expires-at <RFC3339>]
vota [--config <FILE>] admin-token list [--json]
vota [--config <FILE>] admin-token revoke (<ID> | --label <LABEL>)
vota admin-token generate
vota ballot <create|get|list|edit|extend|delete> [<connection flags>] [--json] …
vota completions <bash|zsh|fish|elvish|powershell|nushell>
vota man <DIR>
vota --help | --version
```

## Builds and cargo features

Which commands a `vota` binary has depends on the cargo features it was built with. Both are on by default. A command whose feature is off is not in `--help`, and calling it is a usage error (`error: unrecognized subcommand`, exit 2). There is no default command in any build: `vota` alone prints the help (exit 2), and the server starts only with `vota serve`.

| Feature  | Commands | What it brings in |
| -------- | -------- | ----------------- |
| `server` | [`serve`](#serve), [`admin-token …`](#admin-token-create), the `--config` option | The web server and the database: axum, hyper, SQLite (sqlx), templates, Themes and static assets, configuration loading. |
| `client` | [`ballot …`](#ballot) | The admin API client: reqwest with rustls. |

[`completions`](#completions), [`man`](#man), `--help` and `--version` are in every build.

| Build | Command | Commands it has |
| ----- | ------- | --------------- |
| Full (default) | `cargo build --release` | all |
| Client only, for admin workstations | `cargo build --release --no-default-features --features client` | `ballot`, `completions`, `man` |
| Server only | `cargo build --release --no-default-features --features server` | `serve`, `admin-token`, `completions`, `man` |
| Neither | `cargo build --release --no-default-features` | `completions`, `man` |

`admin-token` belongs to `server`: it works directly on the server's database (it needs the configuration, SQLite and the database migrations), so it runs on the server's host anyway. A client-only build manages Ballots through the admin API with a token created there. [`admin-token generate`](#admin-token-generate) needs no database, but stays with the other `admin-token` commands in `server`; where only a client build is at hand, its [shell one-liner](#admin-token-generate) makes the same token.

The flake has the full build as `packages.default` (and `vota`) and the client-only build as `packages.vota-client`; `cargo install vota --no-default-features --features client` installs the client from crates.io.

## Global options

| Option            | Env var       | Meaning |
| ----------------- | ------------- | ------- |
| `--config <FILE>` | `VOTA_CONFIG` | Builds with `server` only. TOML config file. `VOTA_*` environment variables override its values. Without it, the defaults plus environment variables apply. A missing file is an error: `Error: config file nope.toml not found`. |
| `-h`, `--help`    |               | Help for the command or subcommand. |
| `-V`, `--version` |               | Prints `vota 0.1.0`. |

`--config` is global, so it may come before or after the subcommand. See the [configuration reference](configuration.md) for every option. [`ballot`](#ballot), [`completions`](#completions) and [`man`](#man) ignore it (and `VOTA_CONFIG`): they need no configuration and no database, so they run on any machine.

**Logging** (of the server commands; `ballot`, `completions` and `man` log nothing) goes to stderr. Its level is set with the standard `RUST_LOG` variable (default `info`; e.g. `RUST_LOG=debug`, `RUST_LOG=vota=debug,info`). It is coloured only when stderr is a terminal, so a journal or log file gets plain text. stdout carries only command output, such as a new Admin Token. Admin Tokens, Ballot Tokens and Voter References are never logged. `serve` logs every admin request in the [audit log](admin-api.md#audit-log).

**Every server command** (`serve`, `admin-token …`, except `admin-token generate`) loads the configuration, opens (and if needed creates) the database and applies pending migrations (except a `create` refused for its label or its expiry, which stops before the database is opened and leaves no trace). Only `serve` also loads the Theme and logo folders. The `admin-token` commands never read them, so a broken Theme or logo stops the server but never token management. An unknown `VOTA_*` environment variable is ignored with a warning on every command; an unknown key in the config file stops every command (see the [configuration reference](configuration.md#environment-variables)).

**File modes.** Every file the app creates (the database and its `-wal` and `-shm` files) is readable and writable by its own user only (mode `0600`), whatever umask it was started with: the process sets its umask to `077`. Opening a database whose files others could read also resets them to `0600`. If that fails, the command logs a warning naming the file and carries on, e.g. for a database file owned by another user (from the NixOS VM test):

```text
WARN vota::data::db: could not make /var/lib/vota/vota.db readable by its owner only (mode 0600); other users may be able to read it error=Operation not permitted (os error 1)
```

The database holds every live Ballot Token in plaintext.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| `0`  | Success. |
| `1`  | Runtime error, printed to stderr as `Error: …`: bad configuration, database problem, Theme or logo problem (`serve` only), nothing to revoke, an invalid `--label`, an expiry in the past or after `9999-12-31T23:59:59Z`, a failed write (no token is printed), port already in use, … |
| `2`  | Usage error from argument parsing (unknown subcommand, invalid or conflicting flags), printed by clap with a usage line. |

`ballot` has more exit codes of its own, see [`ballot`: exit codes](#exit-codes-of-ballot).

## `serve`

Runs the web server. It must be named: `vota` without a command only prints the help.

```sh
vota --config vota.toml serve
```

stderr:

```text
2026-10-20T11:09:14.608625Z  INFO vota::web::themes: Themes: acme, classic, dark, minimal; logos: acme
2026-10-20T11:09:14.612462Z  INFO vota::cli::serve: listening on 127.0.0.1:8080
```

At startup it:

1. loads the configuration and fails on any invalid value or unknown key in the config file (unknown `VOTA_*` environment variables only log a warning);
2. loads the built-in Themes and the folder Themes and logos, and trial-renders every Theme's templates (see the [Themes and logos guide](../guides/themes-and-logos.md));
3. opens the SQLite database (creating it if missing) and applies pending migrations;
4. binds `bind_address:port`;
5. starts the purge sweep, which runs once immediately and then every `purge_interval`.

**Shutdown.** `serve` shuts down gracefully on **SIGTERM** (systemd's default stop signal) and on **Ctrl-C (SIGINT)**: it logs `received SIGTERM, shutting down gracefully` (or `SIGINT`), stops accepting connections, lets running requests finish (waiting at most [`connections.shutdown_grace_period`](configuration.md#connections), default 10 s, before dropping connections still open), stops the purge sweep (a sweep already running finishes first), logs `purge sweep stopped; shut down` and exits with `0`. The database stays consistent even if the process is killed, because SQLite runs in WAL mode and every write is a transaction. See the [deployment guide](../guides/deployment.md#stopping-the-server).

## `admin-token create`

Creates an Admin Token and prints it **once** on stdout. Only its SHA-256 hash is stored. The server does not need to be running: the command works on the configured database directly.

| Flag                      | Meaning |
| ------------------------- | ------- |
| `--label <LABEL>`         | Note on what the token is for. Optional. Several tokens may share a label. Single-line text like a Ballot's title: trimmed and NFC-normalised, not empty, at most 200 characters, at most 4 combining marks on one character, without control, bidi or other invisible characters (see [Trust boundary](../concepts/trust-boundary.md#text)), so a label can't steer the terminal of whoever lists the tokens. |
| `--expires-in <DURATION>` | Lifetime from now, in [humantime](https://docs.rs/humantime/latest/humantime/fn.parse_duration.html) syntax: `90days`, `12h`, `1y 6months`, `30m`. |
| `--expires-at <RFC3339>`  | Expiry as an RFC 3339 timestamp with offset, e.g. `2030-01-01T00:00:00Z`. Must lie in the future. Unlike the admin API's dates, an offset is required here (`default_time_zone` doesn't apply). |

Either way the expiry must be no later than `9999-12-31T23:59:59Z`, the last instant an RFC 3339 timestamp can show. A longer lifetime (e.g. `--expires-in 300000years`) or a later expiry (e.g. `9999-12-31T23:59:59-01:00`, which is in the year 10000 in UTC) is refused, as is an expiry that doesn't lie in the future and an invalid label. Each is refused before the database is opened, so a missing database file is not created.

`--expires-in` and `--expires-at` cannot be combined. Without either, the token never expires.

```sh
vota --config vota.toml admin-token create --label ci --expires-in 90days
```

stdout (shortened here):

```text
vta_AsUKO…
```

stderr:

```text
Created Admin Token 2. It is shown only this once; store it securely.
```

The token is `vta_` followed by 43 base64url characters (256 random bits). Because stdout holds only the token, it can be captured directly:

```sh
TOKEN=$(vota --config vota.toml admin-token create --label ci)
```

If the token can't be stored (e.g. the disk is full and the write can't be committed), `create` fails with `Error: …` (exit 1) and prints no token, so a token is never shown that wasn't stored:

```text
Error: error returned from database: (code: 778) disk I/O error

Caused by:
    (code: 778) disk I/O error
```

Errors:

```text
$ vota admin-token create --expires-at 2020-01-01T00:00:00Z
Error: the expiry must lie in the future                          (exit 1)

$ vota admin-token create --expires-in 300000years
Error: the expiry must be no later than 9999-12-31T23:59:59Z      (exit 1)

$ vota admin-token create --expires-at 2030-01-01T00:00:00
error: invalid value '2030-01-01T00:00:00' for '--expires-at <RFC3339>': premature end of input   (exit 2)

$ vota admin-token create --expires-in 1d --expires-at 2030-01-01T00:00:00Z
error: the argument '--expires-in <DURATION>' cannot be used with '--expires-at <RFC3339>'          (exit 2)

$ vota admin-token create --label $'red\e[31m'
Error: invalid --label: contains control, invisible or unassigned characters      (exit 1)

$ vota admin-token create --label ''
Error: invalid --label: must not be empty                         (exit 1)

$ vota admin-token create --label "$(printf 'a%.0s' {1..201})"
Error: invalid --label: too long                                  (exit 1)
```

A label with more than four combining marks on one character gets `Error: invalid --label: too many combining marks`.

A refused label is never printed back.

## `admin-token list`

Lists all Admin Tokens, oldest first: ID, creation time, expiry, whether it has expired, when it was last used, and label. It never shows secrets or hashes.

```sh
vota --config vota.toml admin-token list
```

```text
ID  CREATED               EXPIRES               EXPIRED  LAST-USED             LABEL
1   2026-10-19T11:40:24Z  never                 no       2026-10-19T11:40:30Z  docs
2   2026-10-19T11:40:28Z  2027-01-17T11:40:28Z  no       never                 ci
3   2026-10-19T11:40:28Z  2027-01-01T00:00:00Z  no       never                 old
4   2026-10-19T11:40:29Z  never                 no       never                 ci
```

Columns are aligned. The label comes last because it may contain spaces. Times are RFC 3339 UTC. An expired token stays listed (`EXPIRED yes`) until it is revoked.

`LAST-USED` is when the admin API last accepted the token, to the minute: a running server records a use only if the recorded one is a minute old or more, so a busy script doesn't turn every request into a database write. `never` means it hasn't been used since it was created (or since the upgrade that started recording uses). A request the token was refused for is no use. Which requests a token made is in the [audit log](admin-api.md#audit-log).

A label stored by a release before labels were checked may hold characters `create` now refuses. `list` never prints them raw: in the table each such character is written as `\u{…}`, in the JSON as a JSON escape (`\u001b`, `\u202e`, a surrogate pair such as `\udb40\udc41` above U+FFFF, `\n` for a newline): the same JSON string. For a stored label `red`, ESC `[31m`, U+202E, U+E0041 and a newline:

```text
ID  CREATED               EXPIRES  EXPIRED  LAST-USED  LABEL
1   2026-10-20T06:47:32Z  never    no       never      red\u{1b}[31m\u{202e}\u{e0041}\u{a}
```

```json
[{"created_at":"2026-10-20T06:47:32Z","expired":false,"expires_at":null,"id":1,"label":"red\u001b[31m\u202e\udb40\udc41\n","last_used_at":null}]
```

### `--json`

Prints a JSON array instead, one object per token, for scripts:

```sh
vota --config vota.toml admin-token list --json
```

```json
[{"created_at":"2026-10-19T11:40:24Z","expired":false,"expires_at":null,"id":1,"label":"docs","last_used_at":"2026-10-19T11:40:30Z"},{"created_at":"2026-10-19T11:40:28Z","expired":false,"expires_at":"2027-01-17T11:40:28Z","id":2,"label":"ci","last_used_at":null},{"created_at":"2026-10-19T11:40:28Z","expired":false,"expires_at":"2027-01-01T00:00:00Z","id":3,"label":"old","last_used_at":null},{"created_at":"2026-10-19T11:40:29Z","expired":false,"expires_at":null,"id":4,"label":"ci","last_used_at":null}]
```

| Key          | Type                   |
| ------------ | ---------------------- |
| `id`         | integer                |
| `label`      | string or `null`       |
| `created_at` | RFC 3339 UTC           |
| `expires_at` | RFC 3339 UTC or `null` |
| `expired`    | boolean                |
| `last_used_at` | RFC 3339 UTC or `null` (never used) |

With no tokens it prints `[]`.

## `admin-token revoke`

Deletes Admin Tokens by ID or by label. They stop working at once, also in a running server (the middleware checks the database on every request), with no restart needed. Exactly one of `<ID>` or `--label` is required.

| Argument          | Meaning |
| ----------------- | ------- |
| `<ID>`            | The ID printed by `create` and `list`. A positive integer. |
| `--label <LABEL>` | Revokes **every** Admin Token whose label is exactly this (case-sensitive, no patterns), as stored: `create` stores labels trimmed and NFC-normalised. |

```text
$ vota admin-token revoke 3
Revoked Admin Token 3.

$ vota admin-token revoke --label ci
Revoked Admin Tokens 2, 4.
```

`--label` uses the plural even for a single token (`Revoked Admin Tokens 3.`).

These messages go to stderr. Errors:

```text
$ vota admin-token revoke 3
Error: no Admin Token with ID 3                                   (exit 1)

$ vota admin-token revoke --label nobody
Error: no Admin Token with label "nobody"                         (exit 1)

$ vota admin-token revoke 0
error: invalid value '0' for '[ID]': 0 is not in 1..9223372036854775807   (exit 2)

$ vota admin-token revoke
error: the following required arguments were not provided:
  <ID|--label <LABEL>>                                            (exit 2)
```

Tokens from [`admin_token_files`](configuration.md#admin_token_files) are not in the database, so `revoke` can't reach them: remove the file from the list and restart the server.

## `admin-token generate`

Prints a new Admin Token on stdout, for an [Admin Token file](configuration.md#admin_token_files), and stores it nowhere. It reads no configuration (`--config` and `VOTA_*` are ignored), opens no database, logs nothing and creates no file. Each call prints a fresh token from the operating system's CSPRNG, in the same format as `create`: `vta_` and 43 base64url characters.

```text
$ vota admin-token generate
vta_XVrvwXzowx8k2P8D5tlMcVWLWDwuaaqHeiCRMszgqtQ
```

Write it straight into the secret, e.g. `vota admin-token generate > admin-token` (with `umask 077`), or into an agenix secret (see [Deployment: NixOS](../guides/deployment.md#admin-tokens-from-files-agenix)). On a machine without a full `vota` build, this shell one-liner (OpenSSL and GNU coreutils' `basenc`) prints a token of the same format: 32 random bytes, base64url without the `=` padding.

```sh
printf 'vta_%s\n' "$(openssl rand 32 | basenc --base64url | tr -d '=\n')"
```

The token works once a server lists its file in `admin_token_files` and is (re)started. `admin-token list` never shows it.

## `ballot`

A client for the [admin API](admin-api.md) of a running Vota server: it builds the request from flags, sends it over HTTP(S), and prints the answer. It is a thin client. The server checks every value, and the client only parses what it needs to build the JSON (the values of `--choice-mode`, `--submission-mode` and `--status`, which flags go together, and [relative dates](#relative-dates), which it turns into timestamps), so its errors are the API's. It needs no config file and no database, so it may run on any machine that reaches the server. It is part of the default build (cargo feature `client`); a client-only build has it without the server (see [Builds and cargo features](#builds-and-cargo-features)).

```text
vota ballot create (<field flags> | --body <FILE|->)
vota ballot get <ID>
vota ballot list [--group <GROUP>] [--status unanswered|answered|auto_fallback]
vota ballot edit <ID> (<field flags> <--clear-* flags> | --body <FILE|->)
vota ballot extend <ID> --voting-deadline <DATETIME|DURATION|+DURATION>
vota ballot delete <ID> [--yes]
```

### Connection and Admin Token

These flags may come before or after the verb.

| Flag | Env var | Meaning |
| ---- | ------- | ------- |
| `--url <URL>` | `VOTA_URL` | The server's public base URL, sub-path included: its [`public_base_url`](configuration.md#public_base_url), e.g. `https://example.org/polls/`. A missing trailing slash is added (`https://example.org/polls` works too). Required. |
| `--token-file <PATH\|->` | | Read the Admin Token from this file, or from stdin for `-`. Surrounding whitespace (such as the final newline) is trimmed. |
| `--token <SECRET>` | | The Admin Token itself. **Unsafe**: other local users see it in `ps`, and it lands in the shell history. Prefer `--token-file` or `VOTA_ADMIN_TOKEN`. Can't be combined with `--token-file`. |
| | `VOTA_ADMIN_TOKEN` | The Admin Token, used when neither `--token` nor `--token-file` is given. |
| `--timeout <DURATION>` | | Give up on a request after this long ([humantime](https://docs.rs/humantime/latest/humantime/fn.parse_duration.html), e.g. `10s`, `2m`). Default `30s`. |
| `--ca-cert <PEM>` | | Also trust the CA certificates in this PEM file, on top of the system's root certificates (e.g. for a private CA). |
| `--insecure` | | Don't verify the server's TLS certificate, and print a warning on stderr. **Unsafe**: anyone on the way can read the Admin Token. |
| `--allow-http` | | Allow `http://` to a host other than loopback. **Unsafe**: the Admin Token travels in clear text. |
| `--json` | | Print the server's JSON answer instead of the readable output (see [Output](#output)). |

The Admin Token is taken from `--token`, else `--token-file`, else `VOTA_ADMIN_TOKEN`. It is never printed, and the client logs nothing (`RUST_LOG` has no effect on it). The server commands don't warn about `VOTA_URL` and `VOTA_ADMIN_TOKEN`, so both may stay set in an admin's shell (see [Environment variables](configuration.md#environment-variables)).

The URL must be absolute. `https://` is always allowed. `http://` is allowed to a loopback host (`localhost`, `127.0.0.0/8`, `::1`), and to any other host only with `--allow-http`. Any other scheme, or none, is refused; the client doesn't guess one. The TLS certificate is checked against the system's root certificates (rustls, no OpenSSL), plus those of `--ca-cert`. For an `http://` URL no TLS is used, and no CA store is needed.

Each command sends exactly one request (`delete` in a terminal sends two: see below), with `User-Agent: vota/<version>`. Redirects are never followed: a `3xx` is an error naming its `Location` (exit 1), which usually means `--url` lacks the sub-path or uses `http` where the proxy redirects to `https`. Nothing is retried.

```sh
export VOTA_URL=https://ballots.example.org/
export VOTA_ADMIN_TOKEN=$(cat ~/.config/vota/token)   # or: --token-file ~/.config/vota/token
```

The examples below ran against `vota serve` with `public_base_url = "http://localhost:8080/"`, so `VOTA_URL=http://localhost:8080/`.

### Field flags

`create` and `edit` take the Ballot's fields as flags. Each flag becomes one field of the JSON body; a field without a flag is left out, so on `edit` it keeps its value. Values go to the server as given (except [relative dates](#relative-dates), which become timestamps): it checks text, timestamps, colours and limits, see the [request fields](admin-api.md#request-fields).

| Flag | Field |
| ---- | ----- |
| `--title <TITLE>` | `title` |
| `--description <TEXT>` | `description` (limited Markdown) |
| `--description-file <PATH\|->` | `description`, read from a file or stdin. Can't be combined with `--description`. |
| `--option <LABEL>` | `options`: repeat it for each Option, in display order (`--option Pizza --option Sushi`). On `edit` the given Options **replace all Options**, and each gets a new ID, exactly as in the API. |
| `--choice-mode single\|multiple` | `choice_mode` |
| `--allow-no-option[=true\|false]` | `allow_no_option`: [Allow No Option](admin-api.md#request-fields), the voter may answer with no Option (multiple choice only). The flag alone sends `true`; `--allow-no-option=false` sends `false`, which is how `edit` turns it off (the value needs the `=`; anything but `true` or `false` is a usage error). |
| `--submission-mode final\|editable` | `submission_mode` |
| `--voting-deadline <DATETIME\|DURATION>` | `voting_deadline`: RFC 3339, e.g. `2026-12-01T12:00:00+01:00`, or without an offset (`2026-12-01T12:00`) a local time in the server's [`default_time_zone`](configuration.md#default_time_zone); see [Conventions](admin-api.md#conventions). The client passes a timestamp on unchecked. Or a [relative date](#relative-dates): `3d` from now, and on `edit` `+1d` after the current Voting Deadline. |
| `--purge-date <DATETIME\|DURATION\|+DURATION>` | `purge_date`, in the same forms; `+7d` is 7 days after the Voting Deadline of the same command, or on `edit` without one, the Ballot's current one. |
| `--fallback <LABEL>` | `fallback`: an Option's label; repeat it for more than one. |
| `--group <GROUP>` | `group` |
| `--voter-reference <REF>` | `voter_reference` |
| `--theme <NAME>` | `theme` |
| `--accent-color <#RRGGBB>` | `accent_color` |
| `--logo <NAME>` | `logo` |

**Removing a value** (`edit` only): `--clear-description`, `--clear-group`, `--clear-voter-reference`, `--clear-accent-color`, `--clear-logo` and `--clear-fallback` send `null` for their field. Each can't be combined with its own setter (`--clear-group --group x` is a usage error).

**Raw JSON**: `--body <FILE|->` sends the file (or stdin) unchanged as the body, for fields or combinations the flags don't cover. It can't be combined with any field or `--clear-*` flag, and the client checks only that it is JSON.

```text
$ vota ballot create --body ballot.json --title x
error: the argument '--body <FILE|->' cannot be used with:
  --title <TITLE>
  --description <DESCRIPTION>
  …
$ echo 'not json' | vota ballot create --body -
Error: --body: the file is not JSON                               (exit 2)
```

### Relative dates

`--voting-deadline` and `--purge-date` (on `create` and `edit`) and `extend`'s `--voting-deadline` also take a duration. The client turns it into an RFC 3339 UTC timestamp before sending, so the API sees only timestamps:

| Form | Example | Means |
| ---- | ------- | ----- |
| Timestamp | `2026-12-01T12:00:00+01:00`, `2026-12-01T12:00` | Sent as given; the server reads it (see the flag table). Any value that starts like a date (`YYYY-`) is a timestamp. |
| Duration | `--voting-deadline 3d`, `--purge-date 2m` | That long from now, by the clock of the machine the client runs on, to the whole second. |
| `+` duration | `--purge-date +7d`, `extend --voting-deadline +1d` | That long after the Voting Deadline: for `--purge-date`, the one given in the same command, otherwise (on `edit`) the Ballot's current one; for `--voting-deadline` (on `edit` and `extend`), the Ballot's current one. |

A duration is one or more `<number><unit>` groups without spaces, such as `3d`, `1m2w` or `2d12h`. The units, in lower case:

| Unit | Means |
| ---- | ----- |
| `min` (`mins`, `minute`, `minutes`) | minutes |
| `h` | hours |
| `d` | days (24 hours) |
| `w` | weeks (7 days) |
| `m` (`mo`, `month`, `months`) | calendar months |
| `y` | calendar years (12 months) |

**`m` is months here, not minutes.** The server's configuration (`max_date_horizon`, `default_purge_gap`, …) and the client's own `--timeout` use [humantime](https://docs.rs/humantime/latest/humantime/fn.parse_duration.html) durations, where `m` is minutes and `M` months; the date flags don't (`M` is refused there). Minutes are `min`: `--voting-deadline 90min`.

Months and years are added first, as calendar months: the same day and time of day in the target month, or the month's last day if it has no such day (`+1m` from 31 January is 28 or 29 February, `+1m` from 31 March is 30 April). Then minutes, hours, days and weeks are added as fixed spans in UTC: `d` is always 24 hours, also across a daylight saving change in the server's `default_time_zone`, so `+1d` from 12:00 local time can be 11:00 or 13:00 local time the next day. Give a timestamp to pin a local time of day.

Each resolved value is shown on stderr before the request is sent, so stdout keeps its contract (the URL of `create`, the JSON with `--json`):

```text
$ vota ballot create --title "Lunch on Friday?" --option Pizza --option Sushi --voting-deadline 3d --purge-date +2w
Voting Deadline: 2026-10-25T14:00:00Z (in 3d)
Purge Date: 2026-11-08T14:00:00Z (2w after the Voting Deadline)
http://localhost:8080/b/Xsa6qTjvUXT_6_Y-sWMdpQ
Created Ballot 1: Lunch on Friday?
…
$ vota ballot edit 1 --purge-date +1m
Purge Date: 2026-11-25T14:00:00Z (1m after the Voting Deadline)
Ballot 1: Lunch on Friday?
…
$ vota ballot extend 1 --voting-deadline +1d
Voting Deadline: 2026-10-26T14:00:00Z (1d after the current Voting Deadline)
…
```

Notes:

- **A `+` that needs the Ballot's current deadline** (`edit` or `extend` with `--voting-deadline +…`, or `edit` with `--purge-date +…` and no `--voting-deadline`) first sends `GET /admin/ballots/{id}`, then the change. If another admin moves the deadline in between, the value is relative to the deadline the `GET` saw. Admin edits are not revision-checked, so nothing detects this; it only matters with concurrent admins.
- **`+` on `create`**: `--voting-deadline +1d` is a usage error, since a new Ballot has no deadline to be relative to, and so is `--purge-date +7d` without a `--voting-deadline` in the same command.
- **`+` after a deadline the client can't read**: `--purge-date +7d` with a `--voting-deadline` that is a local time without offset (`2026-12-01T12:00`) is a usage error, since only the server knows its time zone. Give the deadline with an offset or as a duration, or the Purge Date as a timestamp.
- **The client checks only the form.** Whether the result is in the future, within `max_date_horizon` and in order (Purge Date not before the deadline; `extend` later than the current deadline) is the server's to check, as for any timestamp. A lone `--purge-date 8min` (8 minutes from now) is likely before the deadline and gets the server's `422`.
- `--body` is sent unchanged: relative dates in a JSON file are not resolved.

An unreadable value is a usage error (exit 2) that names the flag and lists the forms and units, and nothing is sent:

```text
$ vota ballot create --title T --option A --option B --voting-deadline "next friday"
Error: --voting-deadline next friday: not a timestamp or a duration; give an RFC 3339 timestamp (2026-12-01T12:00:00+01:00), a local date-time in the server's default time zone (2026-12-01T12:00), a duration from now (3d, 1m2w, 90min) or + and a duration after the Voting Deadline (+7d); a duration is numbers with units, without spaces: min (minutes), h (hours), d (days), w (weeks), m (months), y (years)      (exit 2)
$ vota ballot create --title T --option A --option B --voting-deadline 3M
Error: --voting-deadline 3M: unknown unit "M"; give an RFC 3339 timestamp …      (exit 2)
$ vota ballot create --title T --option A --option B --voting-deadline +1d
Error: --voting-deadline +1d: + is relative to the current Voting Deadline, and a new Ballot has none; give a timestamp or a duration from now (3d)      (exit 2)
```

### Output

By default the output is for people. With `--json`, every command prints the server's JSON answer instead, pretty-printed, on stdout (`delete` prints nothing), for scripts and `jq`.

Text from the server is escaped before it is printed, like the [`admin-token list`](#admin-token-list) output: control, bidi and other invisible characters (and newlines inside a value) appear as `\u{…}` in the readable output and as JSON escapes with `--json`, so nothing from the server can steer the terminal. The server already refuses such characters in every text field.

### `ballot create`

`POST /admin/ballots`. Prints only the Ballot's URL on stdout, ready to hand to the voter, and a summary on stderr:

```sh
vota ballot create --title "Lunch on Friday?" --option Pizza --option Sushi \
  --voting-deadline 2026-10-27T12:00:00Z
```

stdout:

```text
http://localhost:8080/b/Xsa6qTjvUXT_6_Y-sWMdpQ
```

stderr:

```text
Created Ballot 1: Lunch on Friday?
Status:          unanswered
Voting Deadline: 2026-10-27T12:00:00Z
Purge Date:      2026-11-24T12:00:00Z
```

So `URL=$(vota ballot create …)` captures the URL. Every field at once:

```sh
vota ballot create --title "Team offsite: which date works?" \
  --description "Pick **every** date you can make." \
  --option "Thursday 14 January" --option "Friday 15 January" --option "Saturday 16 January" \
  --choice-mode multiple --submission-mode editable \
  --voting-deadline 2026-10-27T12:00:00Z --fallback "Friday 15 January" \
  --group offsite-2027 --voter-reference alice@example.org --theme dark --accent-color "#E07A1F"
```

With `--json` it prints the [admin view](admin-api.md#the-admin-view-of-a-ballot), as `get --json` does.

### `ballot get`

`GET /admin/ballots/{id}`. Prints the admin view as a block: the URL, Ballot Status, Group, Voter Reference, modes, dates, Theme Tweaks, description, Options with their IDs, Fallback, Response and result. `-` stands for no value. This Ballot (2, above) had a Response from its voter:

```text
$ vota ballot get 2
Ballot 2: Team offsite: which date works?
URL:             http://localhost:8080/b/XxIB9j-wUFFvYtBzcVhg-g
Status:          answered
Group:           offsite-2027
Voter Reference: alice@example.org
Choice Mode:     multiple
Allow No Option: no
Submission Mode: editable
Voting Deadline: 2026-10-27T12:00:00Z
Purge Date:      2026-11-24T12:00:00Z
Created:         2026-10-20T08:58:12.565Z
Theme:           dark
Accent Colour:   #e07a1f
Logo:            -
Description:
  Pick **every** date you can make.
Options:
  -s7PwP4cHm8  Thursday 14 January
  _Dm4qGewlEM  Friday 15 January
  GoctB57_Sj0  Saturday 16 January
Fallback:        Friday 15 January
Response:        Thursday 14 January (answered 2026-10-20T08:58:12.767Z, changed 2026-10-20T08:58:12.767Z)
Result:          Thursday 14 January (from the Response)
```

`Result` names its source: `from the Response`, or `from the Fallback` once the Fallback applied (Ballot Status `auto_fallback`). A Response of no Option (on a Ballot with Allow No Option) shows as `(none)`, e.g. `Result:          (none) (from the Response)`, while `-` means no Response or no result at all.

```text
$ vota ballot get 1 --json
{
  "accent_color": null,
  "allow_no_option": false,
  "choice_mode": "single",
  "created_at": "2026-10-20T08:58:12.524Z",
  "description": null,
  "fallback": null,
  "group": null,
  "id": 1,
  "logo": null,
  "options": [
    {
      "id": "-t9aTrCMVu0",
      "label": "Pizza"
    },
    {
      "id": "qR37rjq-03E",
      "label": "Sushi"
    }
  ],
  "purge_date": "2026-11-24T12:00:00Z",
  "response": null,
  "result": null,
  "status": "unanswered",
  "submission_mode": "final",
  "theme": "classic",
  "title": "Lunch on Friday?",
  "token": "Xsa6qTjvUXT_6_Y-sWMdpQ",
  "url": "http://localhost:8080/b/Xsa6qTjvUXT_6_Y-sWMdpQ",
  "voter_reference": null,
  "voting_deadline": "2026-10-27T12:00:00Z"
}
```

### `ballot list`

`GET /admin/ballots`. Prints a table of the live Ballots in creation order: ID, Ballot Status, Group, Voter Reference, Voting Deadline and the result's Option labels (last, because labels may contain spaces).

| Flag | Meaning |
| ---- | ------- |
| `--group <GROUP>` | Only Ballots in this Group. Sent percent-encoded, so any text works (`--group "Q3 survey & more"`). |
| `--status <STATUS>` | Only Ballots with this Ballot Status: `unanswered`, `answered` or `auto_fallback`. |

```text
$ vota ballot list
ID  STATUS      GROUP         VOTER-REFERENCE    VOTING-DEADLINE       RESULT
1   unanswered  -             -                  2026-10-27T12:00:00Z  -
2   answered    offsite-2027  alice@example.org  2026-10-27T12:00:00Z  Thursday 14 January
3   unanswered  offsite-2027  bob@example.org    2026-10-27T12:00:00Z  -
```

RESULT is `(none)` for a Response of no Option (Allow No Option), and `-` without a result. With no Ballot it prints only the header line. With `--json` it prints the API's `{"ballots": [...]}`, e.g. `vota ballot list --group offsite-2027 --status answered --json | jq -r '.ballots[] | [.voter_reference, .result.options[].label] | @tsv'`.

### `ballot edit`

`PATCH /admin/ballots/{id}` with the given [field flags](#field-flags) and `--clear-*` flags (or `--body`). Prints the Ballot as it is now, like `get`:

```text
$ vota ballot edit 1 --title "Lunch on Thursday?" --group lunch --clear-description
Ballot 1: Lunch on Thursday?
URL:             http://localhost:8080/b/Xsa6qTjvUXT_6_Y-sWMdpQ
Status:          unanswered
Group:           lunch
Voter Reference: -
Choice Mode:     single
Allow No Option: no
Submission Mode: final
Voting Deadline: 2026-10-27T12:00:00Z
Purge Date:      2026-11-24T12:00:00Z
Created:         2026-10-20T08:58:12.524Z
Theme:           classic
Accent Colour:   -
Logo:            -
Description:     -
Options:
  -t9aTrCMVu0  Pizza
  qR37rjq-03E  Sushi
Fallback:        -
Response:        -
Result:          -
```

The API's [rules for edits](admin-api.md#what-may-be-edited-when) apply: only the Group and Voter Reference may change once the Ballot is Answered or Auto-fallback.

```text
$ vota ballot edit 2 --title "New title"
Error: 409 already_answered: this Ballot has a Response; it can no longer be edited      (exit 4)
```

### `ballot extend`

`POST /admin/ballots/{id}/extend`. `--voting-deadline` (a timestamp with or without offset, as for `create`, a duration from now, or `+` and a duration after the current deadline: see [Relative dates](#relative-dates)) is required and must be later than the current one; an extension reopens a closed Ballot without a Response. Prints the Ballot like `get`:

```text
$ vota ballot extend 1 --voting-deadline 2026-11-03T12:00:00Z
Ballot 1: Lunch on Thursday?
URL:             http://localhost:8080/b/Xsa6qTjvUXT_6_Y-sWMdpQ
Status:          unanswered
…
Voting Deadline: 2026-11-03T12:00:00Z
Purge Date:      2026-11-24T12:00:00Z
…
```

### `ballot delete`

`DELETE /admin/ballots/{id}`, which deletes the Ballot with its Response at once. On success it prints `Deleted Ballot <id>.` on stderr (nothing with `--json`).

- **In a terminal** (stdin is a terminal) it first fetches the Ballot, shows it and asks. Only `y` or `yes` (any case) deletes; anything else keeps the Ballot and exits 1:

  ```text
  $ vota ballot delete 1
  Ballot 1: Lunch on Friday?
  Group:           lunch
  Voter Reference: alice
  Status:          unanswered
  Delete Ballot 1 "Lunch on Friday?"? [y/N] y
  Deleted Ballot 1.
  ```

  ```text
  Delete Ballot 1 "Lunch on Friday?"? [y/N] n
  Error: Ballot 1 was not deleted                                   (exit 1)
  ```

- **`--yes`** (`-y`) deletes without asking.
- **Without a terminal and without `--yes`** (in a script, a pipe, cron) nothing is sent, since nobody could be asked:

  ```text
  $ vota ballot delete 1 < /dev/null
  Error: not deleting Ballot 1 without confirmation: stdin is not a terminal; pass --yes to delete without asking      (exit 2)
  ```

### Errors

Errors go to stderr as `Error: <status> <error code>: <message>`, with ` (field: <field>)` for a `422` that names a field, and the waiting time for a `429`. An answer without an error code shows the status's reason instead. With `--json`, the server's error body also goes to stdout (pretty-printed; a non-JSON body as escaped text):

```text
$ vota ballot get 3
Error: 404 not_found                                              (exit 3)

$ vota ballot create --title "" --option A --option B --voting-deadline 2026-10-27T12:00:00Z
Error: 422 invalid_input: must not be empty (field: title)        (exit 5)

$ vota ballot create --title T --option A --option B --voting-deadline "2026-12-01 12:00"
Error: 422 invalid_input: not an RFC 3339 timestamp, or a local date-time in the server's default time zone (YYYY-MM-DDTHH:MM[:SS]) (field: voting_deadline)   (exit 5)

$ vota ballot list --token vta_wrong
Error: 401 Unauthorized (the Admin Token was refused: unknown, revoked or expired)          (exit 6)

$ vota ballot list --token vta_wrong --json
{
  "error": "too_many_requests"
}
Error: 429 too_many_requests (retry after 600 seconds)            (exit 6)
```

(The `429` came after the server's limit of failed Admin Token checks was reached, here set to 2; see the [rate limit](admin-api.md#rate-limit-on-failed-admin-token-checks).)

Errors of the client itself:

```text
$ vota ballot list
Error: no Admin Token: give --token-file <PATH|->, VOTA_ADMIN_TOKEN or --token          (exit 2)

$ vota ballot list          # without --url or VOTA_URL
Error: no server URL: give --url or VOTA_URL, e.g. https://example.org/polls/           (exit 2)

$ vota ballot list --url localhost:8080
Error: --url localhost:8080: not an absolute http:// or https:// URL                         (exit 2)

$ vota ballot list --url ftp://example.org/
Error: --url ftp://example.org/: the scheme must be http or https                            (exit 2)

$ vota ballot list --url http://ballots.example.org/
Error: refusing http:// to a host other than loopback: the Admin Token would travel in clear text; use https:// (or --allow-http)   (exit 2)

$ vota ballot list --url http://localhost:1/
Error: can't connect: error sending request for url (http://localhost:1/admin/ballots): client error (Connect): tcp connect error: Connection refused (os error 111)   (exit 7)

$ vota ballot list --url http://192.0.2.1/ --allow-http --timeout 2s
Error: timed out: error sending request for url (http://192.0.2.1/admin/ballots): operation timed out   (exit 7)

$ vota ballot list --insecure
Warning: --insecure: the server's TLS certificate is not verified; anyone on the way can read the Admin Token.
…
```

### Exit codes of `ballot`

| Code | When |
| ---- | ---- |
| `0` | Success. |
| `2` | Usage error: from clap (unknown or conflicting flags, a bad `--choice-mode`, `--submission-mode` or `--status` value), no Admin Token, no URL, a refused URL, an unreadable `--token-file`, `--description-file`, `--body` or `--ca-cert`, a `--body` that is not JSON, a date flag that is neither a timestamp nor a [relative date](#relative-dates) (or a `+` with nothing to be relative to), or `delete` without a terminal and without `--yes`. Nothing was sent. |
| `3` | `404`: no live Ballot with this ID (or a wrong `--url`). |
| `4` | `409`: the Ballot can't be edited now, or the Voter Reference is taken in the Group. |
| `5` | `400`, `413`, `415` or `422`: the server refused the request's content. |
| `6` | `401` or `429`: the Admin Token was refused, or this client's address is locked out for a while. |
| `7` | Network error, TLS error or timeout: no answer arrived. |
| `1` | Anything else: a `5xx`, a redirect, an unexpected answer, or a declined `delete` prompt. |

## `completions`

Prints a completion script for the whole CLI (the commands of the build, their flags, and the values of flags with fixed values) on stdout, for `bash`, `zsh`, `fish`, `elvish`, `powershell` or `nushell`. It needs no config file or database. Files and directories are completed for the flags that take a path (`--config`, `--body`, `--description-file`, `--token-file`, `--ca-cert`, the directory of `man`), URLs for `--url` where the shell supports it (zsh's `_urls`). zsh, fish, elvish, PowerShell and nushell also show each flag's help text.

```sh
vota completions bash > ~/.local/share/bash-completion/completions/vota
vota completions zsh > ~/.zfunc/_vota          # a directory in $fpath
vota completions fish > ~/.config/fish/completions/vota.fish
vota completions nushell | save -f ~/.config/nushell/autoload/vota.nu   # in nu: a directory of $nu.user-autoload-dirs
eval (vota completions elvish | slurp)         # in ~/.config/elvish/rc.elv
vota completions powershell | Out-String | Invoke-Expression   # in $PROFILE
```

The first lines of each:

```text
$ vota completions bash
_vota() {
    local i cur prev opts cmd
    COMPREPLY=()
…
$ vota completions zsh
#compdef vota

autoload -U is-at-least
…
$ vota completions fish
# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_vota_global_optspecs
    string join \n config= h/help V/version
…
```

Any other shell is a usage error (exit 2). The Nix packages install the bash, zsh, fish and nushell scripts, the shells with a standard place for them in a Nix profile (see [Shell completions and man pages with Nix](#shell-completions-and-man-pages-with-nix)).

A hidden flag, `--admin-token-wrapper` (builds with `server`), prints the completions of `admin-token` under the name `vota-admin-token` instead, for the NixOS module's wrapper of that name, which runs `vota admin-token "$@"` (see [Deployment: NixOS](../guides/deployment.md#creating-admin-tokens)). The module installs them with the wrapper.

## `man`

Writes a man page for every command and subcommand of the build into a directory, which is created if missing (existing pages in it are overwritten): `vota.1`, `vota-serve.1`, `vota-admin-token.1`, `vota-admin-token-create.1`, …, `vota-ballot.1`, `vota-ballot-create.1`, …, `vota-completions.1` and `vota-man.1`. It prints nothing, and needs no config file or database.

```sh
vota man ~/.local/share/man/man1     # on man's search path, e.g. through $MANPATH or ~/.local/bin on $PATH
man vota-ballot-create
```

Each page has the command's description, synopsis, options (grouped as in `--help`, with their environment variables, defaults and possible values) and subcommands, then:

- **ENVIRONMENT**: `VOTA_CONFIG`, `VOTA_<OPTION>` and `RUST_LOG` on the pages of the server commands, `VOTA_URL` and `VOTA_ADMIN_TOKEN` on those of `ballot`;
- **EXIT STATUS**: the [exit codes](#exit-codes), on the `ballot` pages [those of `ballot`](#exit-codes-of-ballot);
- **EXAMPLES** on some pages (`serve`, `admin-token create`, `admin-token generate`, `ballot`, `ballot create`, `completions`, `man`);
- **NOTES** on `vota-admin-token(1)`: the NixOS module's `vota-admin-token` wrapper takes the same subcommands and flags, so this page is the wrapper's manual too;
- **SEE ALSO**: the parent and subcommand pages, and this reference.

An unwritable directory is an error (exit 1, `Error: creating …` or `Error: writing …`).

## Shell completions and man pages with Nix

Both flake packages, `vota` (also `default`) and `vota-client`, generate the man pages and completions of their own build when they are built (`vota man`, `vota completions`) and install them: `share/man/man1/vota*.1.gz`, `share/bash-completion/completions/vota.bash`, `share/zsh/site-functions/_vota`, `share/fish/vendor_completions.d/vota.fish` and `share/nushell/vendor/autoload/vota.nu`. `vota-client` has the pages and completions of `ballot`, `completions` and `man` only. A cross-compiled package has neither, since the build can't run the binary.

Where they are found:

- **NixOS module** (`services.vota`): see [Deployment: NixOS](../guides/deployment.md#the-ballot-client-and-shell-completions).
- **`environment.systemPackages`** on NixOS without the module: the same, minus the wrapper.
- **`nix profile install`**, **home-manager's `home.packages`**: the files land in `~/.nix-profile/share` (or `/etc/profiles/per-user/<user>/share`). `man` finds the pages there through `$PATH` (man-db looks next to each `bin` directory). bash-completion and fish find the completions through `$XDG_DATA_DIRS`, which the Nix profile scripts and home-manager set. NixOS's zsh puts every profile in `$NIX_PROFILES` on its `fpath`; with home-manager, `programs.zsh.enableCompletion` does. nushell reads the vendor autoload directories under `$XDG_DATA_DIRS` (nushell 0.101 or later).
