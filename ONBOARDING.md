# Onboarding

This page is for developers working on Vota. To run Vota, start with the [README](README.md). The reference documentation is in [docs/](docs/README.md).

## Setup

The flake provides a dev shell with the Rust toolchain (`cargo`, `rustc`, `clippy`, `rustfmt`, `rust-analyzer`), `sqlx-cli` and `sqlite`:

```sh
nix develop          # needs experimental-features = nix-command flakes
```

A rustup toolchain (current stable; the crate uses Rust edition 2024) works just as well. The build needs no database and no network access beyond fetching crates. All SQL is checked at runtime, and migrations are embedded with `sqlx::migrate!()` (`build.rs` makes Cargo rebuild when `migrations/` changes).

## Build, test, lint

```sh
cargo build                     # debug binary: target/debug/vota
cargo build --release           # release binary: target/release/vota
cargo test                      # the whole suite: in-process HTTP tests and CLI tests
nix build                       # the package; runs `cargo test` in its checkPhase
nix flake check -L              # evaluates the flake, builds the package and runs the NixOS VM test
```

**Cargo features.** `server` (`serve`, `admin-token`, the web server and the database), `client` (`vota ballot`, the admin API client) and `qr` (`--qr` of `ballot create` and `ballot get`; turns on `client`) are all on by default; `completions` and `man` are in every build. The other builds:

```sh
cargo build --no-default-features --features client,qr  # client only: ballot (with --qr), completions, man
cargo build --no-default-features --features client   # client only, without --qr
cargo build --no-default-features --features server   # server only: serve, admin-token, completions, man
cargo build --no-default-features                     # completions and man only
nix build .#vota-client                               # the client-only package (with qr)
```

Every combination must build, pass clippy and pass its tests (CI runs all five):

```sh
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features --features server -- -D warnings
cargo clippy --all-targets --no-default-features --features client -- -D warnings
cargo clippy --all-targets --no-default-features --features client,qr -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features --features client   # likewise for the others
```

Server code is gated in `src/lib.rs` (every module but `cli`, and `input`'s character rules, which the client's output escaping shares) and in `src/cli.rs`; its dependencies are optional and listed under `server` in `Cargo.toml`. A test file that needs the server starts with `#![cfg(feature = "server")]` (the client tests with `all(unix, feature = "server", feature = "client")`, as they start a real server), so none is lost from the default run. `tests/commands_per_feature.rs` checks which commands each build has, and that `--qr` exists only with `qr`. Code that needs `qr` is gated with `#[cfg(feature = "qr")]` (`cli/ballot/qr.rs` and the `--qr` flags in `cli/ballot.rs`); its one dependency, `qrcode`, is built without its image features.

**The NixOS module** (`nix/module.nix`, exported as `nixosModules.default`) has an end-to-end VM test in `nix/vm-test.nix`, wired into the flake's `checks` on Linux (`x86_64-linux`, `aarch64-linux`). It boots a VM with `services.vota`, creates an Admin Token with `vota-admin-token` and provisions one from a root-only file through `adminTokenFiles` (systemd credentials), creates a Ballot through the admin API under the sub-path `/polls/`, creates and lists one with the `vota ballot` client on the PATH (and checks that the man pages and the bash, zsh and fish completions of `vota` and the wrapper are installed, and that bash and fish complete `vota bal`), reads a local Voting Deadline in the system's zone (`time.timeZone`, which `default_time_zone` follows), fetches it through the voter API (with its `logo_url` and logo under `/polls/`), lists and revokes Admin Tokens with the wrapper (and checks that no token reaches the journal), checks the sandbox (dynamic user, no writes outside the state directory), that the `vota-admin-token` wrapper passes `$VAR` in arguments literally, the connection limits (`LimitNOFILE`, a silent client disconnected), that `systemctl stop` is graceful, that state survives a restart under a new dynamic user, the admin audit log and `last_used_at`, the failed-Admin-Token-check limit behind the proxy (three `401`s, then `429`), and the health check: `/healthz` refuses forwarded requests, startup waits for it, a frozen service (`kill -STOP`) is restarted by the check timer, and a check whose probes failed while the service was stopped and started again on purpose leaves the new run alone, and that a database file the service can't make `0600` (owned by another user) is logged as a warning while the service still starts. Run it alone with:

```sh
nix build .#checks.x86_64-linux.nixos-vm -L
```

It uses KVM if `/dev/kvm` exists and otherwise falls back to emulation, which is slower: without KVM the test script takes about three minutes on a 16-core machine. Run it after changing the module, the CLI, configuration loading, startup or shutdown. A failing run prints the VM's console and the test script's output; for an interactive VM run `nix build .#checks.x86_64-linux.nixos-vm.driverInteractive && ./result/bin/nixos-test-driver`. The user-facing docs of the module are in [Deployment: NixOS](docs/guides/deployment.md#nixos); a new configuration option also gets a typed sub-option under `settings` in `nix/module.nix` and a row there.

The voting page's script is a few native ES modules in `static/js/` (no build step, no bundler): `main.js` (the entry: reads the config, wires everything), `config.js` (the `data-*` attributes of `<script type="module" id="vota-enhancement">`), `dom.js` (helpers, finding the no-JS form), `time.js` (the local deadline and the countdown placeholder), `api.js` (the `PUT`, its timeout, what each answer means), `autosave.js` (an Editable Ballot's serialised save queue: coalescing, debounce, retries with backoff, per-Option and status-line feedback, `beforeunload`), `dialog.js` (the Final confirmation and result dialogs) and `refresh.js` (swapping in the fresh page after a save). `src/web/enhancement.rs` embeds them in a table (`MODULES`; add a new module there and to `tests/voting_page_script.rs`) and serves them under `/assets/app/<version>/`, where `<version>` is a hash over all of them, so relative imports carry the version and the files are cached for good. Browsers without module support get the plain no-JS page. The stylesheet stays one file, `static/ballot.css`.

The script has a separate browser check in headless Chromium. It is not part of `cargo test`. Run it after changing the script, its stylesheet, a Theme, or the markup it relies on:

```sh
cargo build
nix shell nixpkgs#nodejs nixpkgs#chromium -c node tests/browser/ballot_js.mjs
BASE_PATH=/polls/ nix shell nixpkgs#nodejs nixpkgs#chromium -c node tests/browser/ballot_js.mjs
```

It starts `target/debug/vota` (or `$VOTA_BIN`) on a free port against a temp database (with a folder Theme without a countdown placeholder), drives `chromium` (or `$CHROMIUM`), and prints one `ok`/`FAIL` line per check. It forces the voter API's answers by intercepting the request (a canned status, a network error, or holding requests back to check that saves are serialised and coalesced). It takes about two minutes, mostly waiting for retries and deadlines. The second run serves the app under the sub-path `/polls/` and also checks that the page requests nothing outside it.

**Before every commit** run both, and fix everything they report (the rule from [`CLAUDE.md`](CLAUDE.md)):

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

The flake's source filter excludes `docs/`, `.scratch/`, `nix/`, `flake.nix`, `flake.lock` and every `*.md` file, so documentation changes don't trigger a rebuild of the package.

**Man pages and completions in the packages.** The flake's `postInstall` runs the freshly built binary: `vota completions bash|zsh|fish|nushell` into `installShellCompletion`, and `vota man` into `installManPage`, so both `vota` and `vota-client` carry the pages and completions of exactly their build (skipped when cross-compiling). `clap_mangen` is not behind a cargo feature: it and `roff` are small, pure Rust and depend only on clap, and every build (crates.io included) can then write its own pages. The page texts clap doesn't know (environment variables, exit codes, examples) live in `src/cli/man.rs`, except the exit codes of `ballot`: they have one source, the enum `Exit` in `src/cli/ballot/exit.rs` (number, HTTP statuses, text), which `cli/ballot.rs`, `cli/ballot/connection.rs` and the man pages use. Likewise the units of relative dates (`3d`, `+1m`) have one source, `Unit` in `src/cli/ballot/dates.rs`, for the parser, the help texts (and so the man pages) and the error message. The [CLI reference](docs/reference/cli.md#exit-codes-of-ballot) stays hand-written; a test, `src/cli/ballot/docs_drift.rs`, parses its `ballot` exit-code table and its unit table and fails on any difference from the code, so change both together (the reference's text is the enum's in Markdown). The Nix package keeps `docs/reference/cli.md` in its source for this test. The NixOS module builds the `vota-admin-token` wrapper with completions of its own from `vota completions <shell> --admin-token-wrapper`; its man page is the package's `vota-admin-token(1)`. A new command is caught by `tests/commands_per_feature.rs`, and its page and completions by `tests/man_pages_cli.rs` and `tests/completions_cli.rs`, which walk the clap definition.

## Code layout and module boundaries

```text
src/
├── main.rs          only parses the CLI and dispatches
├── lib.rs           the module tree and its cargo feature gates
├── app.rs           AppState, the router (`app`, nested under the base path), /healthz
├── cli/             clap commands (cli.rs gates them by cargo feature): serve (with its accept loop and connection limits, serve/connections.rs), admin-token create/list/revoke/generate, completions (completions.rs; also the hidden `--admin-token-wrapper` for the NixOS wrapper), man (man.rs: clap_mangen pages plus ENVIRONMENT, EXIT STATUS, EXAMPLES, SEE ALSO), and the admin API client ballot (cargo feature `client`): ballot.rs, its HTTP code in ballot/connection.rs, bodies in ballot/body.rs (with the `--voters` file), relative date flags (`3d`, `+7d`) resolved to timestamps in ballot/dates.rs, readable output and the CSV of `list` and of a bulk `create` in ballot/render.rs, the tally in ballot/tally.rs, the CSV writer (a thin wrapper around the `csv` crate, plus formula injection protection) in ballot/csv.rs, `--qr` in ballot/qr.rs (cargo feature `qr`)
├── config.rs        configuration: defaults → TOML → VOTA_* env
├── domain/          Ballot rules, validated newtypes (e.g. OptionId), Ballot Status, bulk creation's rules (bulk.rs: list limits, count, numbering), Theme names and accent colour, Admin Token secrets and expiry rules, the Admin Tokens from files (hashes), Clock, token generation. Pure, no I/O.
├── input/           untrusted_value request types and ALL sanitizers. The only place raw input is unboxed. Its character rules (characters.rs) are shared with the CLI's output escaping.
├── data/            SQLite pool, migrations, Ballot store (ballot_store/: create, lookup, edit, responses, purge), Admin Token store, millisecond timestamps. The only place SQL lives.
├── api/             JSON handlers: api/admin/ (behind the Admin Token middleware, with its audit log and failed-check limiter, and the reading of the Admin Token files at startup, token_files.rs) and api/voter.rs; the shared Ballot JSON (ballot_json.rs) and 409 bodies (conflict.rs)
├── web/             server-rendered voting page, templates, Theme registry and logos, trial render, the inserted script (enhancement.rs) and hidden revision field (revision_field.rs), assets, Markdown
├── http/            middleware shared by api/ and web/: rate limiting (failed Ballot Token lookups and failed Admin Token checks), the headers on every response (security_headers.rs), Cache-Control: no-store on the voter surfaces (no_store.rs), error-page headers. No handlers.
└── jobs/            background jobs (the purge sweep). They schedule; their SQL stays in data/.
migrations/          numbered SQL migrations, embedded in the binary
templates/           built-in page templates (base, ballot, confirm) and the static error pages
themes/              built-in Theme stylesheets (classic, dark, minimal)
static/              the voting page's script (js/, ES modules) and its stylesheet
tests/               integration tests (one file per feature), tests/common/ (harness), tests/browser/
nix/                 the NixOS module (module.nix) and its VM test (vm-test.nix)
```

Keep new code in the matching module. The boundaries exist for these reasons:

- **`input/` is the only place raw input is unboxed.** Every request value enters as an `UntrustedValue<T>`, and domain types can only be built with a `Sanitized` proof that only `input/` can create. So the compiler stops a handler from using unchecked input, and a security review only has to read `input/`. See [Trust boundary and sanitization](docs/concepts/trust-boundary.md).
- **`data/` is the only place with SQL.** Every Ballot lookup goes through one `select_ballots!` macro that filters purged Ballots, so no query can forget the Purge Date. Atomicity rules (Final Ballots get one Response, the revision guard, content edits only while Unanswered, i.e. no Response and no applied Fallback) live in single conditional writes there.
- **`domain/` is pure.** It takes "now" as an argument and never does I/O, so Ballot Status, Purge Date shifts and selection rules are plain functions, testable and reusable from the trial render.
- **`api/admin/` is one router behind one middleware** ([ADR 0001](docs/adr/0001-admin-api-shares-public-port.md)), so no admin route can be added without authentication.
- **`http/`** holds cross-surface middleware only. **`jobs/`** schedules work but calls `data/` for SQL.

## How we test

We work test-first (red, green, refactor). Tests exercise **external behaviour only**: HTTP requests in, status, headers, JSON or HTML out. They never reach into internal structs, SQL or private functions, so the internals can be refactored freely.

There is a **single test seam: the whole axum app in-process.** `tests/common/mod.rs` provides `TestApp`:

- `TestApp::new()` / `TestApp::with_config(|config| …)` build the real router (`vota::app`) with real migrations on a fresh SQLite file in a temp directory, empty temp theme and logo folders, `public_base_url = "https://ballots.example.org/"` (base path `/`, so the helpers' paths are root paths), and a `ManualClock` starting at `2030-01-15T12:00:00Z` (`start_time()`). `try_with_config` returns startup errors (for Theme tests), `restart(|config| …)` restarts on the same database and folders, and `restart_with_config(|config| …)` also lets the test change the configuration (e.g. `admin_token_files`).
- Requests go through `tower::ServiceExt::oneshot`, with no network port: `get`, `admin_get`, `post_json`, `admin_patch`, `admin_delete`, `put_json`, `post_form`, `*_raw` variants for odd content types and bodies (a voter Response sent with `put_json` or `post_form` without a `revision` gets the Ballot's current one, fetched through the voter API as a client would; one that names a `revision`, and every `*_raw` request, is sent as it is), and `get_from(peer, x_forwarded_for, path)` / `from_peer` to set the TCP peer for rate-limit and proxy tests and for `/healthz`, which answers only a loopback peer (a request without a peer, as `get` sends, gets the plain 404).
- `admin_token()` creates a valid Admin Token through the same code path as the CLI, and `admin_token_with(NewAdminToken)` one with a label or expiry (label, auth and audit tests). `create_ballot(&json)`, `create_standard_ballot()`, `create_ballot_with_modes(…)` and `create_ballot_with_replaced_options()` set up Ballots; `ballot_request()` is a minimal valid create body.
- Helpers: `CapturedLogs` (the log output while its guard lives, e.g. to check that nothing secret is logged: `tests/admin_audit_log.rs`), `voter_view` (the voter API's view of a Ballot), `current_revision`, `body_json`, `body_string`, `snapshot` (status, headers and body for byte-identity checks), `not_found_token_paths` (one unknown and many malformed tokens), `assert_response_rejected`, `assert_hardened` (the error responses' security headers), `write_file`, and sample logos `SVG_LOGO`, `PNG_LOGO`, `WEBP_LOGO`.
- **Database failures** are made through the filesystem, never with SQL: `wipe_database_files()` truncates the database file and its write-ahead log and zeroes the shared-memory index in place, so every query fails from then on, the Admin Token check included (`tests/error_page_headers.rs`). To fail only writes, as on a full disk, `tests/database_write_failure.rs` lowers the process's file size limit (`RLIMIT_FSIZE`) to the database files' current size; the token check and reads still work, so the admin API's write paths reach their own 500. That limit holds for the whole test process, so that file has a single test and lifts the limit before asserting. Admin `GET` and list have no seam-only way to fail past the token check (both only read).
- **Time** is moved explicitly: `app.clock.advance(chrono::Duration::days(8))` or `app.clock.set(…)`. Deadlines, Auto-fallback, reopening and Purge Dates are all tested this way. The purge sweep is started directly with `vota::jobs::purge_sweep::spawn` and a shutdown future (`tests/purge_sweep_task.rs`), or run once by the app's Clock with `purge_sweep::run_once(&app.state)` (`tests/purge_date.rs`).

**Sub-path deployments** are tested in `tests/sub_path.rs`: an app with `public_base_url = "https://example.org/polls/"`, driven with prefixed paths (`/polls/admin/ballots`, `/polls/b/<token>`), checking every generated URL. A new URL the app generates belongs there too: build it from `config.public_base_url` (absolute) or its `base_path()` (root-relative), never as a literal `/…`.

**Process tests** run `vota serve` as a real process through `tests/common/server.rs` (`Server::start_with`, its `addr`, `signal`, `wait_for_exit_within`, `with_umask`): graceful shutdown, the connection limits on a real socket (`tests/connection_limits.rs`) and the database file modes (`tests/database_permissions.rs`). Reading the database files' bytes (`tests/deleted_data_not_recoverable.rs`) is fine; SQL is not.

**CLI tests** run the real binary with `assert_cmd` against a temp database (`VOTA_DATABASE_PATH`), e.g. `tests/admin_token_cli.rs` (create prints a token the HTTP seam then accepts; list never shows secrets; revoke and expiry take effect) and the `*_config.rs` tests for configuration loading and startup errors. **Client tests** (`tests/ballot_cli*.rs`) run `vota ballot …` with `assert_cmd` against a real server from `Server::start_with`, over plain HTTP on loopback, through `tests/common/ballot_cli.rs`: `Admin::start` (a server and an Admin Token from the `admin-token` CLI), `Admin::ballot()` (the client with `VOTA_URL` and `VOTA_ADMIN_TOKEN` set), `vote` (a Response through the voter API), `FakeServer` (a stand-in that answers with a fixed raw response, or never, and records requests: redirects, timeouts, headers, hostile text) and `run_in_terminal` (a pseudo-terminal, for the delete prompt). There are no TLS tests: `--ca-cert` and `--insecure` are left to reqwest. The CSV writer's rules (quoting, CRLF, formula injection protection) have unit tests in `src/cli/ballot/csv.rs`; `tests/ballot_cli_csv.rs` and `tests/ballot_cli_tally.rs` cover `list --csv` and `tally` end to end, `tests/ballot_cli_bulk.rs` bulk `create` (with `--csv` and `--qr`). The client is behind the default cargo feature `client`, the server behind `server`; see the feature builds under [Build, test, lint](#build-test-lint).

Every sanitizer has hostile-input tests through the seam (`tests/*_hostile_input.rs`): overlong, control, bidi and zero-width characters, path traversal, wrong formats and types, `null`, unknown fields.

## Recipes

### Add a migration

1. Add `migrations/NNNN_short_description.sql` with the next number (currently the last is `0011_allow_no_option.sql`). Migrations are applied in order at startup and are never edited once released, because their checksums are recorded in `_sqlx_migrations`.
2. Write it for existing databases: `ALTER TABLE … ADD COLUMN` with a `DEFAULT` or nullable, and `CHECK` constraints that mirror the domain rules (see `0009_theme_tweaks.sql`). Tables are `STRICT`. Timestamps are UTC Unix milliseconds in `INTEGER` columns.
3. Use the new column only in `src/data/` (the select macro, `BallotRow`, `load`, inserts and updates).
4. `cargo test` applies all migrations to a fresh database in every test. Also try the binary on a database created by the previous release.

### Add a sanitizer

1. Write a failing test through the HTTP seam first, including hostile inputs (a `tests/*_hostile_input.rs` file).
2. In `src/domain/`, add or reuse a newtype whose constructor takes the proof, `pub(crate) fn new(value, _: Sanitized) -> Self`, plus `from_store` if values are read back from the database.
3. In `src/input/`, add the function that unboxes the value: reuse `text::single_line` / `text::multi_line` for text, return `InputError::new("<field path>", "<fixed reason>")` on failure, and never put the rejected value into the message. Build the domain value with `Sanitized::PROOF`.
4. Call it from the request's top-level sanitizer (`new_ballot`, `ballot_changes`, `extension`, `ballot_filter`, `api_response`, `form_response`) through `sanitize_with`.
5. Document the rule in the module comment of `src/input.rs` and in [docs/concepts/trust-boundary.md](docs/concepts/trust-boundary.md).

### Add an API field (walk-through)

Say Ballots get a new optional admin field `note` (single-line text, 1–200 characters). The touch points, in TDD order:

1. **Tests**: create, fetch, edit (set, keep, clear with `null`), list; hostile inputs; whether the voter may see it (voter API and page must not contain it if it is admin-only, like `group_and_voter_reference_never_reach_the_voter`).
2. **Migration**: `migrations/0012_ballot_note.sql`: `ALTER TABLE ballots ADD COLUMN note TEXT;`.
3. **Domain** (`src/domain/ballot.rs`): a `Note` newtype like `Group` (`MAX_CHARS`, `new(…, Sanitized)`, `from_store`). Add the field to `NewBallot`, `BallotDraft` (and `BallotDraft::new`), `BallotChanges` and `Ballot`.
4. **Input DTOs and sanitizer** (`src/input/ballot.rs`):
   - `CreateBallotRequest`: `#[serde(default, deserialize_with = "present")] note: Option<String>` (may be absent, `null` rejected).
   - `EditBallotRequest`: `#[serde(default, deserialize_with = "nullable")] note: Option<Option<String>>` (`null` clears).
   - Decide its **edit scope** in `EditBallotRequest::scope`: content (refused once Answered or Auto-fallback, bumps the revision) or bookkeeping (allowed any time, voter never sees it). Bookkeeping fields must also be written by the bookkeeping `UPDATE` in the store.
   - Sanitize it in `new_ballot` and in `ballot_changes` (absent keeps the current value via `from_store`).
5. **Store** (`src/data/ballot_store.rs` and its submodules in `src/data/ballot_store/`): add the column to `select_ballots!` (in `ballot_store.rs`), `BallotRow` and `load` (`lookup.rs`); to the `INSERT` in `try_insert` and the returned `Ballot` (`create.rs`); and to the content `UPDATE` (and/or the bookkeeping one) in `edit` (`edit.rs`).
6. **Admin view** (`admin_view` in `src/api/admin/ballots.rs`): `"note": ballot.note`.
7. **Voter surfaces**, only if the voter may see it: `voter_view` in `src/api/voter.rs`; the page context in `src/web/ballot_page.rs` (`voting_page_context` / `theme_context`), its documentation in `src/web/templates.rs`, and `trial_render.rs` samples (`sample_ballot` builds a `Ballot` literal, so the compiler points you there anyway).
8. **Docs**: the field tables in [docs/reference/admin-api.md](docs/reference/admin-api.md) (and [voter-api.md](docs/reference/voter-api.md) / [themes-and-logos.md](docs/guides/themes-and-logos.md) if visible), and the per-field table in [trust-boundary.md](docs/concepts/trust-boundary.md).
9. `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

A new **configuration option** goes into `Config` in `src/config.rs` (with its default in `impl Default` and any extra check in `Config::load`), gets a `*_config.rs` test through the binary, a section in [docs/reference/configuration.md](docs/reference/configuration.md), and a typed sub-option of `services.vota.settings` in `nix/module.nix` (listed in [Deployment: NixOS](docs/guides/deployment.md#options)).

## Where things are documented

| What | Where |
| ---- | ----- |
| Domain glossary (use these terms; avoid the listed synonyms, e.g. never "poll" for a Ballot) | [CONTEXT.md](CONTEXT.md) |
| Architecture decisions | [docs/adr/](docs/adr/) ([ADR 0001](docs/adr/0001-admin-api-shares-public-port.md), [ADR 0002](docs/adr/0002-sub-path-proxy-forwards-the-prefix.md)) |
| Spec (the original product requirements, with later decisions appended) | `.scratch/poll-web/spec.md`, kept locally (`.scratch/` is gitignored, not published) |
| Tickets | `.scratch/poll-web/issues/`, kept locally like the spec (not published) |
| User and operator docs | [docs/](docs/README.md) |
| Agent conventions | [CLAUDE.md](CLAUDE.md), [docs/agents/](docs/agents/) |

The code is the source of truth for behaviour. Where the spec and the code differ, the docs in `docs/` describe the code.

## Picking up a ticket

Issues are local Markdown files, one per ticket, at `.scratch/<feature>/issues/NN-slug.md` ([docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)). Each has:

- a `**Status:**` line with one of the triage labels ([docs/agents/triage-labels.md](docs/agents/triage-labels.md)): `needs-triage`, `needs-info`, `ready-for-agent` (fully specified), `ready-for-human`, `wontfix`; finished tickets say `done`;
- a `**Blocked by:**` line naming tickets that must be done first;
- acceptance criteria as `- [ ]` checkboxes;
- discussion appended under `## Comments` at the bottom.

Workflow:

1. Pick the lowest-numbered ticket that is `ready-for-agent` or `ready-for-human` (as fits you) and whose blockers are all done.
2. Read the ticket, [CONTEXT.md](CONTEXT.md), the relevant ADRs and the spec sections it refers to.
3. Work test-first through the seam, keeping to the module boundaries above.
4. Run fmt, clippy and the tests (plus the browser check, at the root and with `BASE_PATH=/polls/`, if you touched the voting page's script or markup).
5. Tick the criteria you met, set `**Status:** done` when all are met, and record new decisions in the spec or as an ADR. Update the docs in `docs/` if behaviour changed.
6. Commit the code, tests, docs and ticket together.

## CI and releases

The GitHub setup in `.github/` mirrors [frostx](https://github.com/0xCCF4/frostx/tree/main/.github); every deviation is explained in a comment in the file.

| Workflow | When | What |
| -------- | ---- | ---- |
| `ci.yaml` | push, PR to `main`, manual, merge queue | build and test on stable and nightly; `nix build` against nixpkgs `release-26.05` and `nixos-unstable`; the NixOS VM test; the browser check at the root and under `/polls/`; `cargo fmt --check`; clippy, `cargo doc` and `cargo test` with the default features, server only, client only with and without `qr`, and no features (plus `cargo doc --all-features`) |
| `check-compile.yaml` | Mondays 13:00 UTC | `cargo install vota` from crates.io on stable and nightly, and `nix build github:0xCCF4/vota` against the same nixpkgs branches |
| `devskim.yml` | push, PR, daily, merge queue | DevSkim scan, results in the Security tab |
| `release-plz.yml` | push to `main` | release-plz: opens the release PR, and after it is merged publishes to crates.io and creates the GitHub release |
| `release-archive.yml` | a release is created | builds a static `x86_64-unknown-linux-musl` binary and attaches it as a `.tar.gz` |
| `auto_merge_bots.yml` | PRs by Dependabot | turns on auto-merge |

`cargo doc` in CI fails on any rustdoc warning; check locally with `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` (also with `--all-features` and each feature combination). Doc comments of public items name private modules and items as plain code (`` `input` ``), not as links.

The published crate leaves out internal files (`exclude` in `Cargo.toml`: `.scratch/`, `docs/`, `nix/`, the flake, `tests/browser/`, `.github/`, `.claude/`). Check its contents with `cargo package --list` and the build with `cargo publish --dry-run`. The README's relative links into `docs/` stay relative: crates.io resolves them against the `repository` URL, so they open on GitHub.

Dependabot (`.github/dependabot.yml`) opens weekly PRs for Cargo dependencies and for the actions. `CODEOWNERS` makes the owner review changes under `.github/`.

Repository secrets (Settings → Secrets and variables → Actions):

- `RELEASE_PLZ_PAT`: a personal access token with contents and pull-request write access to the repository. A release created with the default `GITHUB_TOKEN` would not start `release-archive.yml`.
- `CARGO_REGISTRY_TOKEN`: a crates.io API token with the publish scope for the `vota` crate.

Repository settings to turn on:

- Dependabot alerts and version updates (Settings → Advanced Security).
- Allow auto-merge (Settings → General), plus a branch protection rule or ruleset on `main` with required checks, so auto-merge waits for CI.
- Private vulnerability reporting (Settings → Advanced Security), the contact named in `SECURITY.md`.
- Optionally the merge queue (in the ruleset of `main`); `ci.yaml` and `devskim.yml` already run on `merge_group`.
- Actions → General → Workflow permissions: "Allow GitHub Actions to create and approve pull requests" is not needed (release-plz uses the PAT).
