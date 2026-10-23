# Vota documentation

Start with the [README](../README.md) for a quickstart. Developers should read [ONBOARDING.md](../ONBOARDING.md). The security policy, threat model and operator hardening checklist are in [SECURITY.md](../SECURITY.md). Domain terms (Ballot, Ballot Token, Voter Reference, Group, Option, Choice Mode, Allow No Option, Submission Mode, Fallback, Voting Deadline, Purge Date, Ballot Status, Theme, Theme Tweak, Admin Token, Response) are defined in [CONTEXT.md](../CONTEXT.md).

## Concepts

- [Ballot lifecycle and Ballot Status](concepts/ballot-lifecycle.md): states, the derived Ballot Status, what the admin may do when, and the concurrency guarantees.
- [Voting Deadline, Fallback and Purge Date](concepts/deadlines-fallback-purge.md): extension and reopening, the Purge Date shift, `max_date_horizon`, the purge sweep, the revision guard (`ballot_changed`).
- [Trust boundary and sanitization](concepts/trust-boundary.md): the `untrusted_value` pattern and the rules and limits for every field.
- [Admin Tokens and ADR 0001](concepts/admin-tokens.md): one port, bearer tokens, the CLI as root of trust, Admin Tokens from files.
- [Uniform not-found, rate limiting and trusted proxies](concepts/not-found-and-rate-limiting.md).
- [Security headers and Content Security Policy](concepts/security-headers.md).

## Reference

- [Admin API](reference/admin-api.md): every endpoint, field and error.
- [Voter API and voting page](reference/voter-api.md): the JSON API and the no-JS form flow.
- [CLI](reference/cli.md): `serve`, `admin-token create/list/revoke/generate`, the admin API client `ballot create/get/list/tally/edit/extend/delete` with relative dates, bulk create, CSV export and QR codes, `completions` and `man`.
- [Configuration](reference/configuration.md): every option, its TOML key and `VOTA_*` variable.

## Guides

- [Themes and logos](guides/themes-and-logos.md): writing Themes, template variables, CSS custom properties, the voting page's script and hidden revision field, Content Security Policy rules, logos, folder permissions, startup checks, caching, a worked example.
- [Deployment](guides/deployment.md): running the binary, reverse proxy (Caddy, nginx) with HTTPS redirect and HSTS, keeping Ballot Tokens out of proxy logs, sub-path deployments, database, backups, upgrades, the NixOS module `services.vota` (Admin Token files with agenix, man pages and shell completions).

## Decisions

- [ADR 0001: Admin API shares the public host and port, protected only by Admin Tokens](adr/0001-admin-api-shares-public-port.md)
- [ADR 0002: Under a sub-path, the proxy forwards the prefix and the app serves every route under it](adr/0002-sub-path-proxy-forwards-the-prefix.md)
