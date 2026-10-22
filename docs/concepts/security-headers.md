# Security headers and Content Security Policy

The voter's browser must never contact a third-party server, and nothing an admin or Theme author puts on a page may run as script. Vota enforces this with strict response headers. All values below were captured from a real instance.

## On every response

One layer around the whole app adds `X-Content-Type-Options: nosniff` to **every** response, whatever produced it: pages, JSON of the voter and admin APIs, the empty `401`, `404`, `405` and `415`, the `413` text of an oversized body, and `/healthz`. No browser may read a body as anything but its declared type.

**HTML responses** (the voting and confirmation pages and the HTML error pages below) also get:

```http
x-frame-options: DENY
permissions-policy: accelerometer=(), autoplay=(), camera=(), display-capture=(), encrypted-media=(), fullscreen=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), picture-in-picture=(), publickey-credentials-get=(), screen-wake-lock=(), usb=(), xr-spatial-tracking=()
cross-origin-opener-policy: same-origin
cross-origin-resource-policy: same-origin
```

| Header | Effect |
| ------ | ------ |
| `X-Frame-Options: DENY` | The page can't be framed, also in browsers that predate the CSP's `frame-ancestors 'none'`. |
| `Permissions-Policy` | Every powerful feature (camera, microphone, location, payment, …) is off for the page. |
| `Cross-Origin-Opener-Policy: same-origin` | A page of another site that opened the voting page in a window gets no handle on it. |
| `Cross-Origin-Resource-Policy: same-origin` | Other sites can't embed the page as a resource. |

These headers are the same on every HTML response, so they don't affect the uniform not-found's byte-identity. JSON, plain-text and empty responses get only `nosniff`.

## Voting page and confirmation page

Every themed page (`GET /b/{token}`, the form answers, the confirmation step) gets a fresh 128-bit nonce and these headers:

```http
content-security-policy: default-src 'none'; script-src 'self'; style-src 'self' 'nonce-Mr7LGLMFnQfK2mX7vfGTqw'; img-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'
x-content-type-options: nosniff
referrer-policy: no-referrer
cache-control: no-store
```

(plus the [HTML headers](#on-every-response) above).

What this means:

| Directive | Effect |
| --------- | ------ |
| `default-src 'none'` | Nothing loads unless listed below. |
| `script-src 'self'` | Only script files from this origin (the app's own modules under `/assets/app/<version>/`; their relative imports are same-origin too). **No inline scripts, no event-handler attributes** (`onclick=…`), no `javascript:` URLs. |
| `style-src 'self' 'nonce-…'` | Stylesheets from this origin, plus `<style>` elements carrying this page's nonce. **No `style="…"` attributes.** The accent colour gets in through such a nonce'd `<style>` element. When the script refreshes the page in place, it parses the fresh page with this page's nonce in place of the fresh one, so that copy falls under the same policy without a violation. |
| `img-src 'self'` | Images (logos) only from this origin. |
| `connect-src 'self'` | The script may only call this origin (the voter API). |
| `form-action 'self'` | Forms post only to this origin. |
| `base-uri 'none'`, `frame-ancestors 'none'` | No `<base>` tricks; the page can't be framed. |

The pages also set `<meta name="referrer" content="no-referrer">` and `<meta name="robots" content="noindex, nofollow">` (in the built-in `base.html`). Theme authors: see the [Themes guide](../guides/themes-and-logos.md#content-security-policy-rules-for-themes) for what this means for templates.

## Never cached

Every response of the voting page and the voter API carries `Cache-Control: no-store`, whatever its status: the pages, the confirmation step, the redirect after a form post, every voter API answer, and the error responses below. They hold a Ballot, its voter's Response or its Ballot Token (in the URL and the script's `data-*` attributes), so neither a cache in front of the app (a proxy, a CDN) nor the browser's disk or back/forward cache may keep them; a deleted or purged Ballot can't be served from a cache either. Only the assets below are cacheable.

## Error responses

The uniform not-found (`404`), the rate limit's refusals (`429`, and `400` for an unreadable `X-Forwarded-For` from a trusted proxy) and the internal error (`500`: the voting page's neutral error page, the voter API's `{"error":"internal"}`) of the voting page and the voter API, and the admin API's rate-limit refusals (`429`, `400`) and internal error (`500`, the same `{"error":"internal"}`), are static, so their policy allows nothing but the error pages' own inline style, by hash. The headers are identical on every such response, which keeps the not-found byte-identical whatever the token was:

```http
content-security-policy: default-src 'none'; style-src 'sha256-VOff0Gj0DrKquTZBjdtxMO7iduoPD6gMHxobtltf1y4='; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
x-content-type-options: nosniff
referrer-policy: no-referrer
cache-control: no-store
```

Error responses don't get a policy of their own for caching: they are `no-store` like every other voter response (the admin API's `429`, `400` and `500` too, as they share these headers). The HTML ones also get the [HTML headers](#on-every-response).

(The hash covers the `<style>` of `templates/not_found.html` and `templates/server_error.html` and is computed at startup, so it changes only if those files change.)

The other responses of the voter API (`200`, `409`, `413`, `415`, `422`, and the empty `405`) carry only `cache-control: no-store` and `x-content-type-options: nosniff`, and all other admin API responses only `nosniff`: they are JSON (or empty), not documents.

## Assets

| Path | `Content-Type` | `Cache-Control` | `Content-Security-Policy` |
| ---- | -------------- | --------------- | ------------------------- |
| `/assets/themes/{name}/style.css` | `text/css; charset=utf-8` | `public, max-age=3600` | `default-src 'none'` |
| `/assets/logos/{name}` (SVG) | `image/svg+xml` | `public, max-age=3600` | `default-src 'none'; style-src 'unsafe-inline'; sandbox` |
| `/assets/logos/{name}` (PNG/WebP) | `image/png` / `image/webp` | `public, max-age=3600` | `default-src 'none'` |
| `/assets/app/<current version>/<module>.js` | `text/javascript; charset=utf-8` | `public, max-age=31536000, immutable` | `default-src 'none'` |
| `/assets/app/<other version>/<module>.js` | as above (the current module) | `public, max-age=3600` | `default-src 'none'` |
| `/assets/app/ballot.css?v=<current version>` | `text/css; charset=utf-8` | `public, max-age=31536000, immutable` | `default-src 'none'` |
| `/assets/app/ballot.css` without or with another `?v=` | as above | `public, max-age=3600` | `default-src 'none'` |

All assets also send `X-Content-Type-Options: nosniff` and keep their own policy (they are not HTML, so they don't get the HTML headers). Pages embed logos only via `<img>`, where SVG can't run script. The SVG policy also disarms a logo opened directly: it may not run scripts or load anything, and `sandbox` makes it an opaque origin without scripts, forms or popups, even if the file is in fact HTML named `.svg` (the logo check only looks for `<svg` in it).

- The **app's script and stylesheet** are referenced with a content hash (the first 16 hex digits of a SHA-256), so they can be cached for a year: a new version gets a new URL. The script's modules share one versioned directory, `/assets/app/<version>/`, whose version is a hash over all of them, so their relative imports (`./autosave.js`) carry the version too; the stylesheet uses `?v=<version>`.
- **Theme stylesheets and logos** have no versioned URLs and are cached for one hour. After changing a Theme or logo and restarting, browsers may show the old file for up to an hour. Rename the Theme or logo if a change must show at once.

## No third parties

Nothing on a voting page comes from another host: no fonts, scripts, images or analytics. The built-in Themes use system fonts only. The CSP would block anything else anyway. The browser check (`tests/browser/ballot_js.mjs`) verifies that a voting page makes no requests to other hosts and causes no CSP violations, also when a Ballot with an accent colour is refreshed in place.

## Related

- [Uniform not-found and rate limiting](not-found-and-rate-limiting.md)
- [Themes and logos guide](../guides/themes-and-logos.md)
