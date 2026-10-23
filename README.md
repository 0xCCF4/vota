# Vota

Vota is a small self-hosted web app for asking individual people a question. An admin creates one **Ballot** per voter through a JSON API. Each Ballot has its own unguessable URL, and exactly one voter answers it, on a themed web page or through a JSON API. The admin then fetches each Ballot's raw result. On its Purge Date the whole Ballot, including the answer and the admin's notes, is deleted.

It is one Rust binary with one SQLite database, meant to run behind a reverse proxy.

> ## Disclaimer
> This app is fully vibe coded under human supervision.

## Features

- **Personal Ballots**: one voter per URL. The URL carries a 128-bit **Ballot Token**, the voter's only credential. Create one at a time, or up to 1000 in one all-or-nothing request: one per Voter Reference from a list, or numbered within a Group.
- **Single or multiple choice** (multiple choice may allow answering with no Option at all), **Final** (locked after submitting, with a confirmation step) or **Editable** (changeable until the deadline) Responses.
- **Voting Deadline** with an optional **Fallback** that becomes the result if the voter doesn't answer. Deadlines can be extended, which reopens closed Ballots.
- **Purge Date**: every Ballot is deleted automatically (by default four weeks after its deadline), or earlier on request.
- **Admin bookkeeping**: a **Group** to fetch a survey's results together (and tally or export them as CSV with the client) and a hidden **Voter Reference** (unique within the Group) to know whose Ballot is whose. Neither is ever shown to the voter.
- **Results API**: list Ballots with their raw results, filtered by Group and Ballot Status (`unanswered`, `answered`, `auto_fallback`).
- **Voting page** that works without JavaScript and is enhanced by a small self-hosted script (local-time countdown; on Editable Ballots every change is saved at once, with retries and per-Option feedback). Markdown descriptions are sanitised. Nothing loads from third parties, and a strict Content Security Policy applies.
- **Themes**: three built-in ones (`classic`, `dark`, `minimal`), more from a theme folder, plus per-Ballot accent colour and logo.
- **Security**: strict input sanitization at one boundary, uniform not-found for unknown tokens, rate limiting of failed lookups and failed Admin Token checks per client IP (trusted-proxy aware), Admin Tokens stored only as hashes and never logged, an audit log of admin requests, connection limits and timeouts, strict security headers on every response, and `Cache-Control: no-store` on every voter page and API response. See [SECURITY.md](SECURITY.md).
- **Admin Tokens** created, listed and revoked with a local CLI, no restart needed; or provisioned declaratively in files the server reads at startup (e.g. agenix secrets, `vota admin-token generate`).
- **Admin CLI client**: `vota ballot create|get|list|tally|edit|extend|delete` talks to the admin API of a running server from any machine. Dates may be relative (`--voting-deadline 7d`, `extend --voting-deadline +1d`). Many Ballots at once from a file of Voter References (`--voters voters.txt`) or a count (`--count 30`), printed as a table or as CSV for a mail merge; a Ballot's URL as a QR code in the terminal (`--qr`). `tally` sums up a Group's results by Option; `create --csv`, `list --csv` and `tally --csv` export CSV for spreadsheets (RFC 4180, with formula injection protection). Man pages (`vota man`) and shell completions for six shells.

## Quickstart

Build the binary with a current stable Rust toolchain or with Nix:

```sh
cargo build --release          # → target/release/vota
# or: nix build                # → result/bin/vota
```

Write a minimal config:

```sh
cat > vota.toml <<'EOF'
database_path = "vota.db"
public_base_url = "http://localhost:8080/"
EOF
```

Start the server in a second terminal (in the same directory). It logs `listening on 127.0.0.1:8080` when ready:

```sh
./target/release/vota --config vota.toml serve
```

Back in the first terminal, create an Admin Token. It is printed once on stdout:

```sh
TOKEN=$(./target/release/vota --config vota.toml admin-token create --label quickstart)
```

Create a Ballot. Its Voting Deadline must lie in the future, here one week ahead (GNU `date`):

```sh
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "Lunch on Friday?",
       "options": [{"label": "Pizza"}, {"label": "Sushi"}],
       "voting_deadline": "'"$(date -u -d '+7 days' +%Y-%m-%dT%H:%M:%SZ)"'"}'
```

The answer (`201 Created`) contains the Ballot's `url`, among other fields:

```json
{"accent_color":null,"choice_mode":"single", … ,"status":"unanswered", … ,"token":"JA_JHNlyKkEVDmG0r34mOA","url":"http://localhost:8080/b/JA_JHNlyKkEVDmG0r34mOA", … }
```

Or use the built-in client instead of `curl`. It takes a deadline as a timestamp or as a duration from now (`7d`; see [relative dates](docs/reference/cli.md#relative-dates)), and prints just the Ballot's URL on stdout and a summary on stderr:

```sh
export VOTA_URL=http://localhost:8080/ VOTA_ADMIN_TOKEN=$TOKEN
./target/release/vota ballot create --title "Lunch on Friday?" --option Pizza --option Sushi \
  --voting-deadline 7d
```

```text
Voting Deadline: 2026-10-27T09:00:53Z (in 7d)
http://localhost:8080/b/tRV67_NxfITbtmT2VnrOqA
Created Ballot 1: Lunch on Friday?
Status:          unanswered
Voting Deadline: 2026-10-27T09:00:53Z
Purge Date:      2026-11-24T09:00:53Z
```

Open that `url` in a browser, choose an Option, press **Continue**, then **Confirm and submit**. Then fetch the result:

```sh
curl -s http://localhost:8080/admin/ballots -H "Authorization: Bearer $TOKEN"
# or, as a table:
./target/release/vota ballot list
# or summed up by Option, or as CSV for a spreadsheet:
./target/release/vota ballot tally
./target/release/vota ballot list --csv > ballots.csv
```

The Ballot now has `"status":"answered"` and a `result`. See [`vota ballot`](docs/reference/cli.md#ballot) for every command and flag, `vota completions <shell>` for shell completions (bash, zsh, fish, elvish, PowerShell, nushell) and `vota man <dir>` for man pages; the Nix packages install both. Stop the server with Ctrl-C.

**On NixOS**, skip the build and the config file: the flake's module runs Vota as a hardened systemd service, and `vota-admin-token` creates Admin Tokens against its database:

```nix
services.vota = {
  enable = true;
  settings.public_base_url = "https://ballots.example.org/";
};
```

```sh
TOKEN=$(sudo vota-admin-token create --label quickstart)
```

See [Deployment: NixOS](docs/guides/deployment.md#nixos) for importing the module, all options and a reverse proxy.

Vota can also run under a path prefix on a shared host, e.g. `public_base_url = "https://example.org/polls/"`: every route and every generated URL then sits under `/polls/`, and the reverse proxy forwards that prefix unchanged. See [Deployment: under a sub-path](docs/guides/deployment.md#under-a-sub-path).

## Documentation

- [docs/](docs/README.md): concepts, API, CLI and configuration reference, Themes and deployment guides.
  - [Admin API](docs/reference/admin-api.md) · [Voter API](docs/reference/voter-api.md) · [CLI](docs/reference/cli.md) · [Configuration](docs/reference/configuration.md)
  - [Themes and logos](docs/guides/themes-and-logos.md) · [Deployment](docs/guides/deployment.md)
- [SECURITY.md](SECURITY.md): security policy, threat model, security design and operator hardening checklist.
- [ONBOARDING.md](ONBOARDING.md): for developers working on the code.
- [CONTEXT.md](CONTEXT.md): the domain glossary.

## License

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [LICENSE](LICENSE).
