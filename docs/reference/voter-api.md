# Voter API and voting page reference

The voter reaches their Ballot with its **Ballot Token**, either on the server-rendered voting page (`/b/{token}`) or through the JSON voter API (`/api/b/{token}`). Both apply the same rules. The Ballot Token is the voter's only credential, so no other authentication exists.

All examples were run against a real `vota serve`. JSON is shown pretty-printed where noted.

- [Endpoints](#endpoints)
- [`GET /api/b/{token}`: the Ballot as the voter sees it](#get-apibtoken-the-ballot-as-the-voter-sees-it)
- [`PUT /api/b/{token}/response`: submit or change the Response](#put-apibtokenresponse-submit-or-change-the-response)
- [Voter API errors](#voter-api-errors)
- [The voting page without JavaScript](#the-voting-page-without-javascript)
- [The voting page with JavaScript](#the-voting-page-with-javascript)

## Endpoints

Paths are shown for the base path `/`. Under a [sub-path](../guides/deployment.md#under-a-sub-path) they carry its prefix (e.g. `/polls/b/{token}`, `/polls/api/b/{token}`), as do all URLs the page and the API generate; `/healthz` also answers at the root.

| Method | Path                      | What                                                                 |
| ------ | ------------------------- | -------------------------------------------------------------------- |
| `GET`  | `/api/b/{token}`          | The Ballot as its voter may see it, with Ballot Status and Response. |
| `PUT`  | `/api/b/{token}/response` | Submit or change the Response (JSON).                                |
| `GET`  | `/b/{token}`              | The voting page (HTML).                                              |
| `POST` | `/b/{token}`              | The no-JS form post (`application/x-www-form-urlencoded`).           |
| `GET`  | `/healthz`                | Liveness for local probes: `{"now":"…","status":"ok"}`, or `503` if the database doesn't answer. Only for a loopback TCP peer without `X-Forwarded-For` or `Forwarded` header; any other request gets the plain, empty `404` of an unknown path. No token, no rate limit. See [Deployment](../guides/deployment.md#configure-and-run). |

A `{token}` that is not exactly 22 canonical base64url characters written literally (any percent-encoding, even `%58` for `X`, counts as malformed), or that names no live Ballot (never existed, deleted, purged), gets the **uniform not-found** response of that surface. It is byte-identical for every such token. See [Uniform not-found and rate limiting](../concepts/not-found-and-rate-limiting.md). Each not-found counts toward the per-client [rate limit](configuration.md#rate_limit).

The voter never sees the internal ID, the Purge Date, the Group or the Voter Reference, on the page or in the API.

**Never cached.** Every response of the voting page and the voter API, whatever its status (the uniform not-found, the redirect after a form post, and the rate limit's `400` and `429` included), carries `Cache-Control: no-store`: it holds a Ballot, its voter's Response or its Ballot Token, so neither a shared cache nor the browser's disk or back/forward cache may keep it. Every response, JSON included, also carries `X-Content-Type-Options: nosniff`. See [Security headers](../concepts/security-headers.md).

**Proxied requests.** Behind a [trusted proxy](configuration.md#trusted_proxies), a request on these routes whose `X-Forwarded-For` can't be read (a hop that is not a bare IP address, e.g. `junk`, `unknown` or `203.0.113.5:4711`, anywhere in the chain, or more than 32 hops) gets `400`, with `{"error":"bad_request","message":"the X-Forwarded-For header could not be read"}` from the API and a plain "Bad request" page from the voting page, both with the error headers. It is charged to no [rate-limit](../concepts/not-found-and-rate-limiting.md#the-client-ip-and-trusted-proxies) bucket. From any other peer the header is ignored.

## `GET /api/b/{token}`: the Ballot as the voter sees it

```sh
curl -s http://localhost:8080/api/b/fqps4Y5QXH9JOgytT1czCw
```

`200` (pretty-printed):

```json
{
  "accent_color": "#e07a1f",
  "allow_no_option": false,
  "choice_mode": "multiple",
  "description": "Pick **every** date you can make.\n\nDetails: [wiki](https://wiki.example.org/offsite)",
  "fallback": {
    "option_ids": ["otO0JCJbJ88"]
  },
  "logo_url": null,
  "options": [
    { "id": "asWoSUcc7Ts", "label": "Thursday 14 January" },
    { "id": "otO0JCJbJ88", "label": "Friday 15 January" },
    { "id": "kDH9L13ZxRg", "label": "Saturday 16 January" }
  ],
  "response": null,
  "result": null,
  "revision": 0,
  "status": "unanswered",
  "submission_mode": "editable",
  "theme": "dark",
  "title": "Team offsite: which date works?",
  "voting_deadline": "2026-12-20T17:00:00Z",
  "voting_open": true
}
```

| Field             | Type                       | Meaning |
| ----------------- | -------------------------- | ------- |
| `title`           | string                     | |
| `revision`        | integer, 0 or more         | The Ballot's revision: every admin edit of what the voter sees or what governs the Response advances it (edits of only the Group or Voter Reference don't). Send it back with the Response. |
| `description`     | string or `null`           | The sanitised **Markdown source**, not HTML. A client renders (and sanitises) it as it sees fit. |
| `options`         | array of `{id, label}`     | In display order. Submit the `id`s. |
| `choice_mode`     | `"single"` / `"multiple"`  | Single: exactly one Option. Multiple: one or more (or none, see `allow_no_option`). |
| `allow_no_option` | boolean                    | Allow No Option: the voter may answer with no Option at all (`"option_ids": []`). Only ever `true` on a multiple-choice Ballot. |
| `submission_mode` | `"final"` / `"editable"`   | Final: one Response, then locked. Editable: may be changed until the deadline. |
| `voting_deadline` | RFC 3339 UTC               | Exclusive: at this instant voting is closed. |
| `voting_open`     | boolean                    | `now < voting_deadline`. |
| `status`          | `"unanswered"` / `"answered"` / `"auto_fallback"` | See [Ballot lifecycle](../concepts/ballot-lifecycle.md). |
| `fallback`        | `{option_ids}` or `null`   | What is recorded if no Response comes by the deadline. Shown from the start. Only `result` says whether it has been applied. |
| `result`          | `{source, option_ids}` or `null` | `source` is `"response"` or `"fallback"`. |
| `response`        | `{option_ids, answered_at, changed_at}` or `null` | The voter's current Response; `null` without one. `option_ids` is `[]` for a Response of no Option. |
| `theme`           | string                     | The Theme the page is rendered in. If the Ballot's Theme has since been removed, this is the default `classic`. |
| `accent_color`    | `#rrggbb` or `null`        | The accent colour Theme Tweak. |
| `logo_url`        | string or `null`           | Absolute URL (`<public_base_url>assets/logos/<name>`, e.g. `https://example.org/polls/assets/logos/acme`), or `null` without a logo or if it has been removed since. |

## `PUT /api/b/{token}/response`: submit or change the Response

Request: `Content-Type: application/json`, body exactly `{"option_ids": [...], "revision": N}`.

| Field        | Type             | Rules |
| ------------ | ---------------- | ----- |
| `option_ids` | array of strings | Each exactly 11 canonical base64url characters (an Option ID as the server writes it), no duplicates, each one of this Ballot's current Options, and the count obeys the Choice Mode: exactly one for single choice, at least one for multiple choice. Empty (`[]`, "none of these") only if the Ballot has `allow_no_option`: that is a Response like any other, so the Ballot becomes Answered, a Final Ballot is then locked, and the Fallback no longer applies. |
| `revision`   | integer          | Required: the `revision` of the Ballot as the client fetched it (and showed it to the voter). A JSON integer from 0 to 2^63−1; not a string, float (`1.0`) or `null`. If it is no longer the Ballot's revision, nothing is recorded (`409 ballot_changed`), so a Response is never recorded against wording, Options or rules the voter didn't see. |

The checks run in this order:

1. Malformed Ballot Token → uniform `404`.
2. Body not declared as JSON (`415`), too large (`413`), not valid JSON (`422 invalid_json`), or valid JSON that is not a `{"option_ids": [...], "revision": …}` object, e.g. without `revision` (`422 invalid_input`).
3. No live Ballot with this token → uniform `404`.
4. Voting closed (now ≥ Voting Deadline) → `409 voting_closed`, whatever the selection.
5. Final and already answered → `409 already_answered`.
6. `revision` not a non-negative integer → `422`.
7. `revision` not the Ballot's current revision → `409 ballot_changed`: the admin changed the Ballot since the client fetched it.
8. Malformed or duplicate IDs → `422`.
9. A well-formed ID that is not one of this Ballot's current Options → `409 ballot_changed`. This is most likely an Option the admin replaced while the page was open. The answer is the same whether or not the ID exists on another Ballot.
10. Wrong count for the Choice Mode → `422`.

The Response is then recorded in one conditional write, which re-checks the deadline, the Final lock and that the Ballot's revision is still the one sent (see [Ballot lifecycle](../concepts/ballot-lifecycle.md#concurrency-guarantees)). On success the answer is `200` with the updated voter view (as for `GET`). On an Editable Ballot a new Response replaces the old one: `answered_at` stays the time of the first, and `changed_at` moves.

```sh
curl -s -X PUT http://localhost:8080/api/b/fqps4Y5QXH9JOgytT1czCw/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["asWoSUcc7Ts"], "revision": 0}'
```

`200` (pretty-printed, unchanged fields omitted):

```json
{
  "response": {
    "answered_at": "2026-10-17T10:51:05.966Z",
    "changed_at": "2026-10-17T10:51:05.966Z",
    "option_ids": ["asWoSUcc7Ts"]
  },
  "result": {
    "option_ids": ["asWoSUcc7Ts"],
    "source": "response"
  },
  "revision": 0,
  "status": "answered",
  "voting_open": true
}
```

## Voter API errors

| Status | Body | When |
| ------ | ---- | ---- |
| `404` | `{"error":"not_found"}` | Uniform not-found: malformed, unknown, deleted or purged Ballot Token, or any other path under `/api/b/`. Carries the [error-page security headers](../concepts/security-headers.md). |
| `409` | `{"error":"already_answered","message":"this Ballot already has a Response, which can't be changed"}` | Final Ballot that already has a Response. |
| `409` | `{"error":"voting_closed","message":"the Voting Deadline has passed; no Response can be submitted or changed"}` | At or after the Voting Deadline. |
| `400` | `{"error":"bad_request","message":"the X-Forwarded-For header could not be read"}` | From a trusted proxy, an unreadable `X-Forwarded-For` (see [Proxied requests](#endpoints)). Carries the error-page security headers; charged to no rate-limit bucket. |
| `409` | `{"error":"ballot_changed","message":"the Ballot was just changed; fetch it again and resubmit"}` | The `revision` sent is not the Ballot's current one (the admin edited it since it was fetched, or while the Response was being submitted), or an Option ID is not (any more) on this Ballot. Nothing was recorded; `GET` the Ballot again. |
| `405` | empty, `Allow: GET,HEAD` (or `PUT` on `…/response`) | A method the route doesn't serve, e.g. `POST /api/b/{token}`. |
| `413` | text | Body over 64 KiB. |
| `415` | empty | `Content-Type` is not `application/json`. |
| `422` | `{"error":"invalid_json","message":"the request body is not valid JSON"}` | The body is not valid JSON: truncated, empty, not UTF-8, or trailing data after the object. |
| `422` | `{"error":"invalid_input","field":…,"message":…}` | Valid JSON of the wrong shape, or an invalid selection. |
| `429` | `{"error":"too_many_requests"}`, `Retry-After: <seconds>` | This client made too many failed lookups; see [rate limiting](../concepts/not-found-and-rate-limiting.md). Carries the [error-page security headers](../concepts/security-headers.md). The voting page answers with a plain "Too many requests" page. |
| `500` | `{"error":"internal"}` | Internal error, without details (it is logged without request data). Carries the [error-page security headers](../concepts/security-headers.md). |

The `422` messages for `PUT`:

| `field`          | `message` |
| ---------------- | --------- |
| `option_ids[i]`  | `not an Option ID` (malformed) |
| `option_ids[i]`  | `duplicate Option` |
| `option_ids`     | `select an Option` (empty list, unless the Ballot has `allow_no_option`) |
| `option_ids`     | `single choice: select exactly one Option` |
| `option_ids`     | `missing`, `wrong type or value` |
| `revision`       | `missing` |
| `revision`       | `must be a non-negative integer` (e.g. `"1"`, `-1`, `1.0`, `null`) |
| `null`           | `unknown field` (any other field: its name is never echoed) |
| `""`             | `must be a JSON object` (e.g. `[]`) |

Real examples on an Editable single-choice Ballot:

```sh
curl -s -X PUT http://localhost:8080/api/b/ClwSuvpBK5TgduvIOXZ9ng/response \
  -H 'Content-Type: application/json' -d '{"option_ids": [], "revision": 0}'
# {"error":"invalid_input","field":"option_ids","message":"select an Option"}

curl -s -X PUT http://localhost:8080/api/b/ClwSuvpBK5TgduvIOXZ9ng/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["Pizza"], "revision": 0}'
# {"error":"invalid_input","field":"option_ids[0]","message":"not an Option ID"}

curl -s -X PUT http://localhost:8080/api/b/ClwSuvpBK5TgduvIOXZ9ng/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["AAAAAAAAAAA"], "revision": 0}'
# {"error":"ballot_changed","message":"the Ballot was just changed; fetch it again and resubmit"}

curl -s -X PUT http://localhost:8080/api/b/ClwSuvpBK5TgduvIOXZ9ng/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ['
# {"error":"invalid_json","message":"the request body is not valid JSON"}

curl -s -X PUT http://localhost:8080/api/b/ClwSuvpBK5TgduvIOXZ9ng/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["BExg3XGCreg"]}'
# {"error":"invalid_input","field":"revision","message":"missing"}
```

An admin edit since the Ballot was fetched (the audit's case: the question was negated after the voter read it):

```sh
curl -s http://localhost:8080/api/b/22UsGlb08dUx4eI-wESSMw
# {… "revision":0, … "title":"Do you approve proposal X?", …}
# … the admin PATCHes the title to "Do you REJECT proposal X?" …
curl -s -X PUT http://localhost:8080/api/b/22UsGlb08dUx4eI-wESSMw/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["ZG4cER4loWc"], "revision": 0}'
# {"error":"ballot_changed","message":"the Ballot was just changed; fetch it again and resubmit"}
curl -s -X PUT http://localhost:8080/api/b/22UsGlb08dUx4eI-wESSMw/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["ZG4cER4loWc"], "revision": 1}'
# {… "revision":1, "status":"answered", … "title":"Do you REJECT proposal X?", …}
```

After the deadline (here on a Ballot with a Fallback):

```sh
curl -s -X PUT http://localhost:8080/api/b/kKNCBW_dN2-Luu8lQj4q5A/response \
  -H 'Content-Type: application/json' -d '{"option_ids": ["iutTiPonBLc"], "revision": 0}'
# {"error":"voting_closed","message":"the Voting Deadline has passed; no Response can be submitted or changed"}

curl -s http://localhost:8080/api/b/kKNCBW_dN2-Luu8lQj4q5A
# … "result":{"option_ids":["-Yg5TgAhfDk"],"source":"fallback"}, "status":"auto_fallback", … "voting_open":false
```

## The voting page without JavaScript

`GET /b/{token}` renders the Ballot in its Theme. It works fully without JavaScript. Times are shown in the configured [`default_time_zone`](configuration.md#default_time_zone), e.g. `Tue 1 Dec 2026, 13:00 CET`. The page shows, depending on the state:

| State                                     | The page shows |
| ----------------------------------------- | -------------- |
| Open, no Response                         | A status pill "Open", the deadline, the form (Options as cards: radio buttons for single choice, checkboxes for multiple, under "Choose any number of options (or none)" with Allow No Option), "You can change your answer until then." (Editable) or "Once you submit your answer, you cannot change it." beside the button (Final), and, if there is a Fallback, its Options marked "default" on their cards with the hint "No answer by the deadline? The Option marked default will be recorded." |
| Open, Editable, answered                  | "Answered", the form pre-filled with the current Response, "Your answer is saved. You can change it until then.", button "Change answer". |
| Open, Final, answered                     | Read-only: "Answered", "Your answer has been recorded." and "You chose" with the chosen Options as chips ("You chose no option." for a Response of no Option). |
| Closed with a Response                    | "Closed", "You chose" with the chosen Options as chips ("You chose no option." for a Response of no Option). |
| Closed, no Response, Fallback             | "Closed", "No answer was given, so this was recorded:" with the Fallback's Options as chips. |
| Closed, no Response, no Fallback          | "Closed", "No answer was given." |

The pill's tooltip names the Ballot Status (`Ballot Status: Auto-fallback`).

### The form post

The form posts to its own URL (`POST /b/{token}`) as `application/x-www-form-urlencoded` with these fields:

| Field     | Values |
| --------- | ------ |
| `option`  | An Option ID. Repeated once per selected Option. Absent when nothing is ticked: on a Ballot with Allow No Option that is the Response of no Option (the body then holds only `revision`, and `confirm`); otherwise the notice "Select at least one Option.". A body with any other field is still unreadable, never taken for an empty selection. |
| `confirm` | Absent, or exactly one `yes` (sent by the confirmation step's button). |
| `revision` | Required, exactly once: the Ballot's revision as the page showed it, from the hidden field `<input type="hidden" name="revision" value="…">` that the voting page's form and the confirmation step's form carry (the app inserts it into every `<form method="post">` of those pages, whatever the Theme; a form that already has an input named `revision`, as older Themes' templates do, is left as it is). Plain decimal digits (no sign, leading zero or spaces). |

Any other field makes the form unreadable. The Option rules are the same as for the voter API. A `revision` that is no longer the Ballot's (the admin edited it since the page was rendered, also between the confirmation step and confirming) records nothing: `409` with the Ballot as it now stands.

**Editable Ballot.** A valid post records (or replaces) the Response at once and redirects with `303 See Other` to `/b/{token}` (post/redirect/get). The page then shows the form pre-filled with the new Response.

```sh
curl -s -i -X POST http://localhost:8080/b/06xE7y6tmQiI99olu2B0dQ \
  -d option=BExg3XGCreg -d option=EjiSvTaVK7o -d revision=0
```

```http
HTTP/1.1 303 See Other
location: /b/06xE7y6tmQiI99olu2B0dQ
cache-control: no-store
x-content-type-options: nosniff
content-length: 0
```

**Final Ballot: two steps.** A post *without* `confirm=yes` stores nothing. It answers `200` with the confirmation page (`confirm.html`), which lists the choice (or says "You chose no option."), warns "You cannot change your answer after you confirm it.", and holds a form that posts the same `option` values and `revision` again with `confirm=yes`. That second post records the Response and redirects (`303`) to the page, which now shows the Response read-only.

```sh
curl -s -i -X POST http://localhost:8080/b/zkt_58VYY9V_Om4Gr5bjsQ -d option=JozWzVChWqQ -d revision=0
# HTTP/1.1 200 OK … <h2>Confirm your answer</h2> … <input type="hidden" name="revision" value="0">
#   <input type="hidden" name="option" value="JozWzVChWqQ"> …
#   <button type="submit" name="confirm" value="yes">Confirm and submit</button>

curl -s -i -X POST http://localhost:8080/b/zkt_58VYY9V_Om4Gr5bjsQ -d option=JozWzVChWqQ -d revision=0 -d confirm=yes
# HTTP/1.1 303 See Other
# location: /b/zkt_58VYY9V_Om4Gr5bjsQ
```

On an Editable Ballot, `confirm=yes` is accepted and ignored. A `confirm` with any other value, or given twice, is an error.

### Form errors

A problem re-renders the voting page with a notice (`<p class="message" role="alert">`). The notice names the problem in words but never echoes the submitted value:

| Status | Notice | Cause |
| ------ | ------ | ----- |
| `422` | Select exactly one Option. | Single choice: none or several selected. |
| `422` | Select at least one Option. | Multiple choice without Allow No Option: none selected. |
| `422` | That Option is not part of this Ballot. | A value that is not an Option ID at all. |
| `422` | Select each Option only once. | The same Option twice. |
| `422` | The confirmation was not valid. Please try again. | `confirm` other than a single `yes`. |
| `422` | The form could not be read. Please try again. | Unknown field, invalid UTF-8. |
| `422` | The page was out of date or incomplete. Please check the Ballot and answer again. | `revision` missing, given twice, or not plain decimal digits. |
| `409` | This Ballot has already been answered. Your answer cannot be changed. | Final Ballot already answered. |
| `409` | Voting has closed. Your answer could not be accepted. | At or after the Voting Deadline (checked before the form is read). |
| `409` | This Ballot was changed by its organiser, so your answer was not saved. Please check it and answer again. | The `revision` posted is no longer the Ballot's (the admin edited it since the page or the confirmation step was rendered, or while the post was processed), or a well-formed Option ID that is no longer on the Ballot. The page shows the Ballot as it now stands. |

A body that is not `application/x-www-form-urlencoded` gets a bare `415`, and one over 64 KiB a `413`. An unknown Ballot Token (`404`) and a Ballot that takes no Response (`409`: closed, or Final and already answered) are answered first, without reading the body.

```sh
curl -s -X POST http://localhost:8080/b/zkt_58VYY9V_Om4Gr5bjsQ \
  -d option=JozWzVChWqQ -d option=JozWzVChWqQ -d revision=0 | grep 'class="message"'
# <p class="message" role="alert">Select each Option only once.</p>
```

The not-found page is neutral and unthemed: "Not found. This link does not lead to a Ballot. Please check that you copied it completely." Internal errors (for example a Theme template failing at request time) show an equally neutral "Something went wrong" page with status `500`.

## The voting page with JavaScript

The app inserts its own script (native ES modules under `/assets/app/<version>/`, entry `main.js`) and stylesheet (`/assets/app/ballot.css`) into every voting page, right before `</head>`. Themes cannot remove them. Browsers without module support get the plain page, which works fully. With JavaScript the script:

- rewrites every `<time datetime>` into the voter's local time zone, short and without a zone name ("Today, 18:00", "Tomorrow, 09:30", "Tue 1 Dec, 13:00", with the year only when it isn't this year), with the precise date and zone as its tooltip; and fills the template's countdown placeholder ("in 2 days, 4 hours"), corrected for a skewed client clock using the server's "now". At the deadline it disables the form and reloads to the closed view, with or without a placeholder.
- on an **Editable** Ballot hides the Submit button and saves every change at once through `PUT /api/b/{token}/response` (with the page's `revision`): the changed Options show "Saving…", then a fading "Saved" highlight, or "Not saved" in red until resolved, and a status line (`role="status"`) says the same. At most one request is in flight; changes made meanwhile are coalesced and the latest selection is sent next (multiple choice waits 400 ms for further clicks). With Allow No Option, unticking every box saves the Response of no Option, and while the Ballot has no Response the Save button stays visible, so the voter can save "no Option" without ticking anything first. Transient failures (no connection, timeout, `5xx`, `429` with its `Retry-After`) are retried after 1, 2, 4 and 8 seconds, then every 15 seconds, for about two minutes ("Not saved, retrying…", with "Retry now"); coming back online or to the tab retries at once. Without Allow No Option an empty multiple-choice selection is not sent ("Not saved: select at least one Option."). Leaving the page while something is not saved asks for confirmation. After the first save the page body is refreshed in place.
- on a **Final** Ballot keeps the Submit button: it asks for confirmation in a dialog instead of the confirmation page ("You chose no option." for an empty selection with Allow No Option; without it, an empty selection is refused in the page), then saves with a dialog showing "Saving…", then "Saved ✓" or the error. The page body is then refreshed in place.
- maps errors to messages: `voting_closed`, `already_answered` and `ballot_changed` show a message in a dialog and reload; `404` shows "This Ballot could not be found. It may have been deleted." and reloads; these and other `4xx` are never retried. `422` shows the server's `message`; `429` shows "Too many requests, try again later."

The script gets what it needs from `data-*` attributes on its own `<script type="module" id="vota-enhancement">` tag (`data-api`, `data-page`, `data-deadline`, `data-now`, `data-open`, `data-revision`, `data-choice-mode`, `data-submission-mode`, `data-needs-confirmation`, `data-allow-no-option`, `data-answered`), never from inline script:

```html
<script type="module" src="/assets/app/0d4c1e7a9b2f6835/main.js" id="vota-enhancement" data-api="/api/b/ClwSuvpBK5TgduvIOXZ9ng/response" data-page="/b/ClwSuvpBK5TgduvIOXZ9ng" data-deadline="2026-12-01T12:00:00Z" data-now="2026-10-17T10:52:09.659395927Z" data-open="true" data-revision="0" data-choice-mode="single" data-submission-mode="editable" data-needs-confirmation="false" data-allow-no-option="false" data-answered="false"></script>
```

It makes no requests to other hosts. For what a Theme must keep so the script works, see the [Themes and logos guide](../guides/themes-and-logos.md#the-voting-pages-script).
