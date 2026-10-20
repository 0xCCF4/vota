# Uniform not-found, rate limiting and trusted proxies

A Ballot Token is 128 random bits, so it can't be guessed. Two further measures stop anyone from probing for tokens: every failed lookup looks exactly the same, and too many failed lookups from one client block it.

## Uniform not-found

On each voter surface there is exactly one not-found response. It is the same whether the token

- is malformed (wrong length, wrong alphabet, non-canonical, percent-encoded junk, invalid UTF-8),
- is a real token with percent-encoding in it (`%58sA6…` for `XsA6…`): a token has exactly one spelling in a URL, so log redaction or proxy rules that match it literally can't be bypassed,
- is well-formed but never existed,
- belonged to a Ballot the admin deleted, or
- belongs to a Ballot past its Purge Date (swept or not yet).

The format is checked before any database lookup. The tests compare these responses byte for byte, headers included (`unknown_and_malformed_tokens_get_byte_identical_not_found*`).

| Surface | Response |
| ------- | -------- |
| Voting page `/b/{token}` (and any other path under `/b/`) | `404`, the static, neutral, unthemed page "Not found. This link does not lead to a Ballot. Please check that you copied it completely." |
| Voter API `/api/b/{token}`, `/api/b/{token}/response` (and any other path under `/api/b/`) | `404`, `{"error":"not_found"}` |

Both carry the [error-page security headers](security-headers.md#error-responses). The not-found page is never themed: it must not depend on the request, and a Theme belongs to a Ballot that, from the voter's point of view, doesn't exist.

The **admin API** has its own not-found (`404 {"error":"not_found"}`) for unknown, malformed, percent-encoded, deleted or purged Ballot IDs. It sits behind the Admin Token check and doesn't count toward this rate limit; failed Admin Token checks have [a budget of their own](../reference/admin-api.md#rate-limit-on-failed-admin-token-checks).

## Rate limiting failed lookups

`src/http/rate_limit.rs` wraps the voting page and the voter API:

- **What counts**: every `404` from those two surfaces, which is every failed Ballot Token lookup, including any path below `b/` and `api/b/` that names no route (a trailing slash, more segments), at the root and under a [sub-path](../guides/deployment.md#under-a-sub-path) alike. Successful lookups never use up the budget. Failures on the page and the API share one budget per client.
- **No overshoot**: each request reserves one failure before its lookup and gets it back unless the answer is a `404` (or the request is cancelled), in one step under one lock. So a burst of concurrent requests gets exactly the budget's number of lookups, never more; while a client's last allowed lookups are still running, its further requests get `429` until they finish.
- **Budget**: [`rate_limit.failed_lookups`](../reference/configuration.md#rate_limit) (default 20) per [`rate_limit.window`](../reference/configuration.md#rate_limit) (default 10 minutes), a fixed window that starts at the client's first failure.
- **When the budget is spent**: *every* Ballot Token request from that client gets `429` with `Retry-After: <seconds until the window ends>` until the window ends, even for a valid token. A client that is guessing tokens is blocked, not filtered. The block lifts by itself when the window ends.
- **Per client**: IPv4 addresses count individually. IPv6 addresses count per /64 prefix, because one host usually controls a whole /64. An IPv6 address with an IPv4 address embedded in it counts as that IPv4 address (see [IPv4-embedded addresses](#ipv4-embedded-addresses-nat64-and-siit)).
- **Not affected**: the admin API (which limits failed Admin Token checks with [its own budget](../reference/admin-api.md#rate-limit-on-failed-admin-token-checks), from the same client IP resolution), `/healthz` (which answers only local, unproxied requests and is a plain, empty `404` for every other one), and the assets under `/assets/` (an unknown asset is a plain `404` that doesn't count). Under a [sub-path](../guides/deployment.md#under-a-sub-path), requests outside the prefix get a plain, empty `404` that doesn't count either; the uniform not-found under the prefix is byte-identical to the one at the root.
- **State** is in memory, for a single instance, and resets on restart. At most 65 536 clients are tracked. Expired windows are dropped as time passes. When the table is full, a new client takes the place of the oldest client that is **not blocked**: a blocked client is never evicted, so it can't lift its own block by failing from many new addresses. If every tracked client is blocked, new clients share one overflow window with the same budget until room frees up. Every step costs O(log n) under the lock, never a scan of the table.

The `429` responses carry the error-page security headers:

```http
HTTP/1.1 429 Too Many Requests
content-type: application/json
retry-after: 60

{"error":"too_many_requests"}
```

The voting page gets a minimal HTML page "Too many requests. Please try again later." instead.

Real run with `failed_lookups = 3`, `window = "1m"`, behind a trusted proxy:

```sh
for i in 1 2 3 4; do curl -s -o /dev/null -w '%{http_code} ' \
  -H 'X-Forwarded-For: 198.51.100.9' http://localhost:8080/api/b/AAAAAAAAAAAAAAAAAAAAAA; done; echo
# 404 404 404 429
curl -s -o /dev/null -w '%{http_code}\n' -H 'X-Forwarded-For: 203.0.113.7' http://localhost:8080/api/b/ClwSuvpBK5TgduvIOXZ9ng
# 200   (another client is not affected)
```

## The client IP and trusted proxies

The client IP comes from `src/input/client_ip.rs`:

1. If the TCP peer is **not** in [`trusted_proxies`](../reference/configuration.md#trusted_proxies), the peer address is the client. `X-Forwarded-For` is ignored entirely, so a client can't spoof it.
2. If the peer **is** a trusted proxy, `X-Forwarded-For` is parsed strictly: a comma-separated list of bare IPv4 or IPv6 addresses across all occurrences of the header, at most 32. If anything in it is not a bare address (a port, brackets, a host name, `unknown`, junk), or it has more hops, the request gets **`400 Bad Request`** (on the voting page, the voter API and the admin API alike, with the error-page headers) and is charged to **no** bucket. (Counting it as the proxy's own request would let one client, sending `X-Forwarded-For: junk` through a proxy that appends to it, spend the budget that every voter without a usable header shares.) A trusted proxy that sends no `X-Forwarded-For` at all is itself the client.
3. Otherwise the chain is read **from the right**, skipping addresses that are themselves trusted proxies. The first untrusted address is the client. Addresses further left were supplied by the client and may be forged. If every hop is trusted, the left-most is taken.

IPv4-mapped IPv6 addresses (`::ffff:198.51.100.7`), whether the TCP peer, an `X-Forwarded-For` hop or a `trusted_proxies` entry (`::ffff:127.0.0.1/128`), are taken as the plain IPv4 address, so a dual-stack listener or proxy can't make one client count as two.

So:

- **Behind a reverse proxy**, list the proxy's address in `trusted_proxies` (e.g. `["127.0.0.1/32", "::1/128"]` when it runs on the same host) and make the proxy pass the real peer in `X-Forwarded-For`: nginx appends it with `$proxy_add_x_forwarded_for`; Caddy sets the header to it by default, dropping any value the client sent. See the [deployment guide](../guides/deployment.md#reverse-proxy).
- **Without** a trusted proxy configured, all traffic through a proxy shares the proxy's IP, and one guesser blocks everyone. Configure it.
- **Never** list networks that untrusted clients can connect from.

### IPv4-embedded addresses (NAT64 and SIIT)

Behind a translator such as Jool (NAT64 or SIIT-DC), every IPv4 client reaches Vota from an IPv6 address with its IPv4 address inside, e.g. `64:ff9b::198.51.100.7`. All of these lie in one /64, so counted per /64 they would share one budget, and one guesser would block every IPv4 client. So once the client IP is found, an IPv6 client address in one of these forms counts as the IPv4 address it embeds, for both rate limits and the `client_ip` of the admin [audit log](../reference/admin-api.md#audit-log):

- `::ffff:a.b.c.d` (IPv4-mapped), `::ffff:0:a.b.c.d` (IPv4-translated, RFC 2765) and `::a.b.c.d` (IPv4-compatible, deprecated; not `::` or `::1`);
- `64:ff9b::a.b.c.d`, the well-known NAT64 prefix `64:ff9b::/96` (RFC 6052), and any address in the local-use prefix `64:ff9b:1::/48` (RFC 8215), whose last 32 bits are taken;
- any address in one of the translator's own prefixes listed in [`nat64_prefixes`](../reference/configuration.md#nat64_prefixes) (Jool's `pool6`), decoded as RFC 6052 §2.2 places the IPv4 address for the prefix length (32, 40, 48, 56, 64 or 96 bits, skipping bits 64–71).

So `64:ff9b::198.51.100.7` and `198.51.100.7` share one budget, and `64:ff9b::198.51.100.8` has its own. This applies to the TCP peer and to the client taken from `X-Forwarded-For` alike.

**Trust decisions don't look inside these addresses**, on purpose. Whether the peer or a hop is a trusted proxy, and whether a request is local for `/healthz`, is decided on the address as it is, with only `::ffff:a.b.c.d` taken as IPv4. The embedded IPv4 address is only as good as the translator that put it there, and the translator is not one of your proxies. So `64:ff9b::127.0.0.1` is not loopback, and an IPv4 entry such as `10.0.0.0/8` in `trusted_proxies` doesn't match a peer `64:ff9b::10.0.0.1`. To trust a proxy that reaches Vota through a translator, list its IPv6 address (`64:ff9b::10.0.0.1/128`).

## Related

- [Security headers](security-headers.md)
- [Configuration: `rate_limit`, `trusted_proxies`](../reference/configuration.md#rate_limit)
- [Configuration: `nat64_prefixes`](../reference/configuration.md#nat64_prefixes)
