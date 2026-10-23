# CLAUDE.md

## Agent skills

### Issue tracker

Issues live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default canonical labels (needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Development

After adding or changing a feature, always run both and fix everything they report before committing:

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

When a change touches what cargo features gate (`server`, `client`), also run clippy and the tests for the other feature combinations (see ONBOARDING.md, "Build, test, lint"). When it touches the voting page's script, markup or a Theme, also run the browser check (`tests/browser/`).

## Code layout

`src/` is split into folder submodules by responsibility. Keep new code in the matching module:

- `cli/`: clap commands (`serve`, `admin-token …`, `ballot …`, `completions`, `man`). `main.rs` only parses and dispatches. `serve` and `admin-token` need cargo feature `server`, `ballot` needs `client`, `completions` and `man` are in every build. `cli/serve/connections.rs` accepts connections; `cli/completions.rs` and `cli/man.rs` generate the shell completions and man pages from the clap tree (the Nix packages install both); `cli/escape.rs` escapes text the CLI prints but didn't write. The admin API client `ballot` lives in `cli/ballot.rs`: its HTTP code (URL, Admin Token, reqwest, status → exit code) in `cli/ballot/connection.rs`, request bodies in `cli/ballot/body.rs`, relative date flags (`3d`, `+7d`; `m` = months) in `cli/ballot/dates.rs`, readable output in `cli/ballot/render.rs`. It never validates what the server validates.
- `app.rs`: `AppState`, the router and `/healthz` (cargo feature `server`, like every module but `cli/` and `input/characters.rs`).
- `config.rs` (split into `config/` once it grows): config loading.
- `domain/`: Ballot rules, validated newtypes, Ballot Status, Theme rules, Admin Token secrets and expiry rules, Clock, token generation. Pure, no I/O.
- `input/`: `untrusted_value` request types and **all** sanitizers. This is the only place raw input is unboxed.
- `data/`: SQLite pool, migrations, stores. This is the only place SQL lives.
- `api/`: JSON handlers, `api/admin/` (with Admin Token middleware, and `token_files.rs`: the Admin Token files read at startup, kept as hashes) and `api/voter.rs`, plus the JSON both share (`ballot_json.rs`, `conflict.rs`).
- `web/`: server-rendered voter pages and templates, the Theme registry and logos, the trial render, and what the app inserts into Theme pages (script, hidden revision field). The voting page's script is native ES modules in `static/js/` (no build step), embedded and served by `web/enhancement.rs` and `web/assets.rs` under a content-versioned path; its stylesheet is `static/ballot.css`.
- `http/`: HTTP middleware shared by `api/` and `web/`: rate limiting of failed Ballot Token lookups and failed Admin Token checks, the security headers on every response, `Cache-Control: no-store` on the voter surfaces, the error responses' headers. No handlers.
- `jobs/`: background jobs beside the server (the purge sweep). They schedule work; their SQL stays in `data/`.
