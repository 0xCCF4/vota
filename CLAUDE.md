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

## Code layout

`src/` is split into folder submodules by responsibility. Keep new code in the matching module:

- `cli/`: clap commands (`serve`, `admin-token …`, `ballot …`, `completions`). `main.rs` only parses and dispatches. The admin API client `ballot` (cargo feature `client`) lives in `cli/ballot.rs`: its HTTP code (URL, Admin Token, reqwest, status → exit code) in `cli/ballot/connection.rs`, request bodies in `cli/ballot/body.rs`, readable output in `cli/ballot/render.rs`. It never validates what the server validates.
- `config.rs` (split into `config/` once it grows): config loading.
- `domain/`: Ballot rules, validated newtypes, Ballot Status, Theme rules, Admin Token secrets and expiry rules, Clock, token generation. Pure, no I/O.
- `input/`: `untrusted_value` request types and **all** sanitizers. This is the only place raw input is unboxed.
- `data/`: SQLite pool, migrations, stores. This is the only place SQL lives.
- `api/`: JSON handlers, `api/admin/` (with Admin Token middleware) and `api/voter.rs`, plus the JSON both share (`ballot_json.rs`, `conflict.rs`).
- `web/`: server-rendered voter pages and templates, the Theme registry and logos, the trial render, and what the app inserts into Theme pages (script, hidden revision field).
- `http/`: HTTP middleware shared by `api/` and `web/`: rate limiting of failed Ballot Token lookups and failed Admin Token checks, the security headers on every response, `Cache-Control: no-store` on the voter surfaces, the error responses' headers. No handlers.
- `jobs/`: background jobs beside the server (the purge sweep). They schedule work; their SQL stays in `data/`.
