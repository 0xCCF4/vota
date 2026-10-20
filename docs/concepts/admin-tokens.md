# Admin Tokens and ADR 0001

An **Admin Token** is the only credential for the admin API. Whoever holds a valid one can create, read, edit and delete every Ballot, including Voter References and Responses.

## Decision: one host and port, protected only by Admin Tokens

[ADR 0001](../adr/0001-admin-api-shares-public-port.md) puts the admin API under `/admin/…` on the same host and port as the voting page and the voter API. There is no separate admin port, so one listener, one reverse proxy and one TLS certificate are enough. The consequences shape the whole token model:

- **No network isolation.** Admin Tokens alone guard every Voter Reference and Response, so they are long and random, shown once, and never logged.
- **The admin router is guarded as a whole.** All admin routes, including unknown paths under `/admin`, sit in their own router behind the token middleware (`src/api/admin.rs`). Without a valid token, even a `404` is unreachable: every request gets `401` (or `429` once the client has spent its budget of failed Admin Token checks, or `400` for an unreadable `X-Forwarded-For` from a trusted proxy).
- **No HTTP endpoint creates tokens.** Whoever can run the CLI against the database is the root of trust.
- Moving the admin API to a separate port later would be possible, but it would change every admin client's URL.

## Lifecycle

| Step | How | Notes |
| ---- | --- | ----- |
| Create | `vota admin-token create [--label …] [--expires-in …\|--expires-at …]` | Printed once on stdout. The server need not run. An expiry must lie in the future and be no later than `9999-12-31T23:59:59Z`. |
| Use | `Authorization: Bearer vta_…` on every `/admin` request | Checked against the database on every request. The scheme `Bearer` may be written in any case. |
| List | `vota admin-token list [--json]` | ID, created, expiry, expired, last use (to the minute), label. Never the secret or its hash. |
| Expire | automatically at `expires_at` (by the server's clock) | From that instant on it is refused like an unknown token. It stays listed as expired. |
| Revoke | `vota admin-token revoke <ID>` or `--label <LABEL>` | Deleted from the database. It stops working at once, with no restart. |

See the [CLI reference](../reference/cli.md) for flags, output and exit codes.

## Format and storage

- **Secret**: `vta_` followed by 256 bits from the operating system's CSPRNG, base64url without padding (43 characters, 47 in total). The `vta_` prefix makes leaked tokens easy to spot by secret scanners.
- **Storage**: only the SHA-256 hash of the secret, with ID, optional label, creation time, optional expiry and the time of its last use, recorded at most once a minute (`admin_tokens` table, `migrations/0001_admin_tokens.sql`, `0010_admin_token_last_used.sql`). A copy of the database does not reveal a usable token.
- **Checking**: the middleware hashes the presented token and looks the hash up, ignoring rows whose `expires_at` is not after now.
- **Logging**: tokens appear only on the CLI's stdout at creation. They are never logged. The secret type's `Debug` output is redacted. Every admin request is logged in the [audit log](../reference/admin-api.md#audit-log) with the token's **ID**, the method, the route template, the status and the client IP.
- **Guessing**: failed checks are [rate-limited per client IP](../reference/admin-api.md#rate-limit-on-failed-admin-token-checks); with 256 random bits a token can't be guessed anyway, the limit keeps the hashing load bounded.
- **Labels** are single-line text without control, bidi or invisible characters (checked by `create`), so `list` can print them safely; older labels are escaped.

## Refusals reveal nothing

Missing header, wrong scheme, malformed token, unknown token, revoked token, expired token: all get the same bare response.

```http
HTTP/1.1 401 Unauthorized
www-authenticate: Bearer
x-content-type-options: nosniff
content-length: 0
```

## Operational advice

- Give each script or person its own token with a `--label`, so you can revoke one without touching the others. `revoke --label` revokes *all* tokens with that exact label.
- Prefer an expiry (`--expires-in 90days`) for tokens used by automation.
- Run the CLI as the user that owns the database file, with the same configuration as the server (same `database_path`). The CLI doesn't read the theme and logo folders, so a broken Theme or logo doesn't block token management.
- After a suspected leak: `vota admin-token revoke <ID>`, then check `admin-token list` (`LAST-USED`) and the audit log (`journalctl -u vota | grep 'token_id=<ID> '`) for what the token was used for.

## Related

- [ADR 0001](../adr/0001-admin-api-shares-public-port.md)
- [Admin API reference](../reference/admin-api.md#authentication)
