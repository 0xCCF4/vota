# Under a sub-path, the proxy forwards the prefix and the app serves every route under it

Vota can run under a path prefix such as `https://example.org/polls/`. The path of `public_base_url` is the base path, and the app nests every route under it (`/polls/b/…`, `/polls/api/…`, `/polls/admin/…`, `/polls/assets/…`); the reverse proxy forwards requests with the prefix unchanged. We chose this over a proxy that strips the prefix because it keeps every proxy configuration a plain path match (`handle /polls/*` in Caddy, `location /polls/ { proxy_pass http://127.0.0.1:8080; }` in nginx), with no rewriting that could be got subtly wrong, and because the app then sees the same paths the browser uses, so the URLs it generates and the paths it routes can't drift apart. The uniform not-found stays intact: the Ballot routes are the same, only nested.

## Consequences

- Outside the base path only `/healthz` answers (so local probes need no prefix); every other request there is a plain, empty `404` that doesn't count toward the rate limit, since it carries no Ballot Token lookup.
- The base path is restricted to plain segments (ASCII letters, digits, `-`, `.`, `_`, `~`), so it goes into routes, HTML attributes and templates without escaping. A `public_base_url` with a query, fragment or other characters stops startup.
- Themes get the base path as the template variable `base_path` and must not hardcode `/`; the trial render renders every Theme under a sample prefix too.
- With the base path `/`, routes and URLs are exactly as without sub-path support.
