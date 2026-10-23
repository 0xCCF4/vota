# Admin API reference

The admin API creates, inspects, edits, extends, deletes and lists Ballots. It lives under `/admin` on the same host and port as everything else ([ADR 0001](../adr/0001-admin-api-shares-public-port.md)); under a [sub-path](../guides/deployment.md#under-a-sub-path) under the prefix, e.g. `/polls/admin`. Every request needs an **Admin Token**.

All examples were run against a real `vota serve` with `public_base_url = "http://localhost:8080/"`. Responses are shown pretty-printed (the server sends compact JSON with keys in alphabetical order).

- [Conventions](#conventions)
- [Authentication](#authentication)
- [`POST /admin/ballots`: create a Ballot](#post-adminballots-create-a-ballot)
- [`GET /admin/ballots/{id}`: fetch one Ballot](#get-adminballotsid-fetch-one-ballot)
- [`PATCH /admin/ballots/{id}`: edit a Ballot](#patch-adminballotsid-edit-a-ballot)
- [`POST /admin/ballots/{id}/extend`: extend the Voting Deadline](#post-adminballotsidextend-extend-the-voting-deadline)
- [`DELETE /admin/ballots/{id}`: delete a Ballot](#delete-adminballotsid-delete-a-ballot)
- [`GET /admin/ballots`: list Ballots with results](#get-adminballots-list-ballots-with-results)
- [The admin view of a Ballot](#the-admin-view-of-a-ballot)
- [Errors](#errors)

## Conventions

- **Request bodies** are JSON objects with `Content-Type: application/json`. Anything else gets `415` with an empty body. A body that is not valid JSON (truncated, empty, not UTF-8, trailing data after the object) gets `422 invalid_json`. Unknown fields are rejected with `422 invalid_input`. Bodies over 64 KiB are refused with `413`.
- **Ballots are identified by their internal ID**, a positive integer, never by their Ballot Token. The ID in the path must be written literally, the way the server writes it: digits only, no sign, no leading zero, no percent-encoding. Anything else is the same `404` as an unknown ID.
- **Timestamps** in requests (`voting_deadline`, `purge_date`) are either RFC 3339 with an explicit offset (`Z` or `±hh:mm`), e.g. `2026-12-01T12:00:00+01:00`, or a **local date-time without offset**, `YYYY-MM-DDTHH:MM:SS` or `YYYY-MM-DDTHH:MM` (fractional seconds allowed after the seconds, as in RFC 3339), e.g. `2026-12-01T12:00`. A local date-time is read in the server's [`default_time_zone`](configuration.md#default_time_zone) (`UTC` unless configured; on NixOS the system's zone). A local time that doesn't exist there (skipped when the clocks go forward, e.g. `2027-03-28T02:30` in `Europe/Berlin`) or exists twice (when they go back, e.g. `2027-10-31T02:30`) is refused with `422`; give an offset for those. Nothing else is accepted, such as a date alone, a space instead of `T`, or a zone name. Either way the time is converted to UTC and stored to the millisecond, and the future and horizon checks apply to the converted instant. Responses always use UTC with a `Z` suffix, with fractional seconds only when there are any (`2026-12-01T10:00:00.123Z`). The API takes no durations; the [`vota ballot`](cli.md#relative-dates) client turns relative dates such as `7d` or `+1d` into timestamps before sending.
- **Text** fields are Unicode-NFC-normalised and trimmed before they are checked, so the stored value is the normalised one. See [Trust boundary and sanitization](../concepts/trust-boundary.md) for the full rules.
- **Purged Ballots** (Purge Date at or before now) don't exist for any endpoint, even before the purge sweep has deleted them.

The shell examples assume the Admin Token is in `$TOKEN` (see [Admin Tokens](../concepts/admin-tokens.md)):

```sh
TOKEN=$(vota --config vota.toml admin-token create --label docs)
```

## Authentication

Send the Admin Token as a bearer token on every request under `/admin`, including unknown paths:

```http
Authorization: Bearer vta_…
```

An Admin Token is either one created with [`admin-token create`](cli.md#admin-token-create) (stored in the database) or one from a file of [`admin_token_files`](configuration.md#admin_token_files) (read at startup). Both are accepted the same way.

The scheme may be written in any case (`Bearer`, `bearer`, `BEARER`), followed by one space and the token. A missing, malformed, unknown, revoked or expired Admin Token gets the same bare response, so the response never reveals which one it was:

```sh
curl -s -i http://localhost:8080/admin/ballots
```

```http
HTTP/1.1 401 Unauthorized
www-authenticate: Bearer
x-content-type-options: nosniff
content-length: 0
```

### Rate limit on failed Admin Token checks

Each `401` counts against the client IP (resolved as for the voter rate limit: the TCP peer, or `X-Forwarded-For` behind a [trusted proxy](configuration.md#trusted_proxies); IPv6 per /64). A client may have [`rate_limit.admin_failed_auth`](configuration.md#rate_limit) failed checks (default 10) per [`rate_limit.admin_window`](configuration.md#rate_limit) (default 10 minutes, a fixed window from its first failure). After that, **every** admin request from it gets `429` until the window ends, even with a valid Admin Token, and its token isn't even checked:

```http
HTTP/1.1 429 Too Many Requests
content-type: application/json
content-security-policy: default-src 'none'; style-src 'sha256-VOff0Gj0DrKquTZBjdtxMO7iduoPD6gMHxobtltf1y4='; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
x-content-type-options: nosniff
referrer-policy: no-referrer
cache-control: no-store
retry-after: 60

{"error":"too_many_requests"}
```

Requests that pass the check never count, and this budget is separate from the voter routes' budget for failed Ballot Token lookups. An unreadable `X-Forwarded-For` from a trusted proxy gets `400` `{"error":"bad_request","message":"the X-Forwarded-For header could not be read"}`, counted nowhere. Both refusals carry the [error-response headers](../concepts/security-headers.md#error-responses). An admin who shares an IP address with a client that is guessing tokens is locked out with it for the window; lower `admin_window` or raise `admin_failed_auth` if that is a concern.

### Audit log

Every admin request is logged as one event, without the Admin Token itself:

```text
INFO vota::admin_audit: admin request token_id=1 method=PATCH route=/admin/ballots/{id} status=422 client_ip=127.0.0.1
```

- `token_id` is present only when the token was accepted: the Admin Token's ID as [`admin-token list`](cli.md#admin-token-list) shows it (e.g. `token_id=1`), or `file:<n>` for the token of the n-th file of [`admin_token_files`](configuration.md#admin_token_files), counted from 1 (e.g. `token_id=file:1`). Database IDs are plain numbers, so the two never collide. On NixOS, `file:<n>` is the n-th entry of `services.vota.adminTokenFiles` (credential `vota-admin-token-<n>`);
- `route` is the route template (`/admin/ballots/{id}`; `/admin`, `/admin/` or `/admin/{*rest}` for unknown paths; with the base path in front under a sub-path), never the request's path or query. `method` is the standard method's name, or `OTHER` for any other method;
- `client_ip` is resolved as for the rate limit. It is absent when it can't be resolved: on the `400` for an unreadable `X-Forwarded-For`. `status` is the response's status.

Failed checks are summed up per client IP in two warnings: one at its first failure in a window, one when it spends the budget:

```text
WARN vota::admin_audit: failed Admin Token check; further ones from this client within 1m are only logged when it reaches the limit client_ip=203.0.113.9
WARN vota::admin_audit: too many failed Admin Token checks; this client is refused (429) on the admin API until its window of 1m ends client_ip=203.0.113.9 failures=3
```

Admin Tokens, Ballot Tokens, Voter References and request bodies are never logged. The events have the target `vota::admin_audit`, so they can be selected with `RUST_LOG` (e.g. `RUST_LOG=info,vota::admin_audit=warn` keeps only the warnings). Each Admin Token in the database also records when it was last accepted, to the minute: see [`admin-token list`](cli.md#admin-token-list). Tokens from files record no last use; find their uses in this log (`token_id=file:<n>`).

## `POST /admin/ballots`: create a Ballot

Creates one Ballot and answers `201 Created` with its [admin view](#the-admin-view-of-a-ballot), which includes the Ballot Token and the public `url` to hand to the voter. It can also create [many Ballots at once](#many-ballots-at-once), one per Voter Reference.

### Request fields

| Field             | Type                     | Required | Default                                         | Rules |
| ----------------- | ------------------------ | -------- | ----------------------------------------------- | ----- |
| `title`           | string                   | yes      |                                                 | Single-line text, 1–200 characters. |
| `description`     | string                   | no       | none                                            | Limited Markdown, 1–10 000 characters. May span lines (`\r\n` and `\r` become `\n`; tabs allowed). Stored and returned as source; rendered only on the voting page. |
| `options`         | array of `{"label": string}` | yes  |                                                 | At least 2 Options, in display order. Each `label` is single-line text, 1–200 characters. Labels must be unique (exact match after normalisation). No upper count other than the 64 KiB body limit. |
| `choice_mode`     | `"single"` or `"multiple"` | no     | `"single"`                                      | Exact match: no trimming, no case folding. |
| `allow_no_option` | boolean                  | no       | `false`                                         | **Allow No Option**: the voter may answer with no Option at all ("none of these"). Only with `"choice_mode": "multiple"`; `true` on a single-choice Ballot is a `422` naming `allow_no_option`. Such a Response makes the Ballot Answered (the Fallback doesn't apply); the Fallback itself is still never empty. CLI: [`--allow-no-option`](cli.md#field-flags). |
| `submission_mode` | `"final"` or `"editable"`  | no     | `"final"`                                       | Exact match. |
| `voting_deadline` | [timestamp](#conventions) string | yes      |                                                 | Must lie in the future and no later than now + [`max_date_horizon`](configuration.md#max_date_horizon) (default 1 year). |
| `purge_date`      | [timestamp](#conventions) string | no       | `voting_deadline` + [`default_purge_gap`](configuration.md#default_purge_gap) (4 weeks) | If given: in the future, no later than now + `max_date_horizon`, and not before `voting_deadline` (equal is allowed). The computed default is not capped by the horizon. |
| `fallback`        | array of strings         | no       | no Fallback                                     | **Option labels**, not IDs: the Options get their IDs only now. Each label is normalised like an Option label and must match one of this Ballot's Options. No duplicates. Must obey the Choice Mode: exactly one label for single choice, one or more for multiple choice. `[]` is rejected. |
| `preselection`    | array of strings         | no       | no Preselection                                 | **Preselection**: Options ticked on the voting page before the voter does. **Option labels**, with the same rules as `fallback` (never empty, even with Allow No Option). Independent of the Fallback: both may be set and may differ. On a **Final** Ballot nothing is recorded: the voter still submits and confirms. On an **Editable** Ballot it is recorded as the Response at once (`response.origin` `"preselection"`), so the Ballot is Answered from the start and the Fallback never applies; see [Preselection](#preselection). CLI: [`--preselect`](cli.md#field-flags). |
| `group`           | string                   | no       | none                                            | Single-line text, 1–200 characters. |
| `voter_reference` | string, or array of strings | no    | none                                            | Single-line text, 1–200 characters. Unique within its Group: a second Ballot with the same Group and Voter Reference is a `409`. Without a Group it is not constrained. An array creates [many Ballots](#many-ballots-at-once), one per entry. |
| `count`           | integer                  | no       | none                                            | Creates [this many Ballots](#many-ballots-at-once) with numbered Voter References, 1 to 1000. Needs a `group`; not together with `voter_reference`. |
| `theme`           | string                   | no       | `"classic"`                                     | `^[a-z0-9][a-z0-9-]{0,63}$`, exact match, and a Theme of that name must exist (built-in or in the theme folder). |
| `accent_color`    | string                   | no       | none (the Theme's own accent)                   | `#rrggbb`, exactly `^#[0-9a-fA-F]{6}$`. Stored lowercase. |
| `logo`            | string                   | no       | none                                            | Same name rule as `theme`. A logo of that name must exist in the logo folder. |

"Single-line text" means NFC-normalised, trimmed, non-empty, free of control and invisible characters (Unicode general categories Cc, Cf such as bidi and zero-width characters, Co private use, Cn unassigned, noncharacters, fillers, variation selectors, U+2800 and U+FFFC), with at most four combining marks on one base character. Lengths count Unicode characters after normalisation. An optional field may be left out, but `null` is rejected on creation (`"wrong type or value"`).

### Minimal example

```sh
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "Lunch on Friday?", "options": [{"label": "Pizza"}, {"label": "Sushi"}], "voting_deadline": "2026-12-01T12:00:00+01:00"}'
```

`201 Created`:

```json
{
  "accent_color": null,
  "allow_no_option": false,
  "choice_mode": "single",
  "created_at": "2026-10-17T10:51:05.876Z",
  "description": null,
  "fallback": null,
  "group": null,
  "id": 1,
  "logo": null,
  "options": [
    { "id": "9AP_l_daq5c", "label": "Pizza" },
    { "id": "c8lVx4rmBwQ", "label": "Sushi" }
  ],
  "preselection": null,
  "purge_date": "2026-12-29T11:00:00Z",
  "response": null,
  "result": null,
  "status": "unanswered",
  "submission_mode": "final",
  "theme": "classic",
  "title": "Lunch on Friday?",
  "token": "tTzFz9vEGN93JsiOCem9vQ",
  "url": "http://localhost:8080/b/tTzFz9vEGN93JsiOCem9vQ",
  "voter_reference": null,
  "voting_deadline": "2026-12-01T11:00:00Z"
}
```

The deadline was given in `+01:00` and is returned in UTC. The Purge Date defaulted to four weeks after it.

### Full example

`ballot.json`:

```json
{
  "title": "Team offsite: which date works?",
  "description": "Pick **every** date you can make.\n\nDetails: [wiki](https://wiki.example.org/offsite)",
  "options": [
    { "label": "Thursday 14 January" },
    { "label": "Friday 15 January" },
    { "label": "Saturday 16 January" }
  ],
  "choice_mode": "multiple",
  "allow_no_option": true,
  "submission_mode": "editable",
  "voting_deadline": "2026-12-20T18:00:00+01:00",
  "purge_date": "2027-01-31T00:00:00Z",
  "fallback": ["Friday 15 January"],
  "group": "offsite-2027",
  "voter_reference": "alice@example.org",
  "theme": "dark",
  "accent_color": "#E07A1F"
}
```

```sh
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d @ballot.json
```

`201 Created`:

```json
{
  "accent_color": "#e07a1f",
  "allow_no_option": true,
  "choice_mode": "multiple",
  "created_at": "2026-10-17T10:51:05.902Z",
  "description": "Pick **every** date you can make.\n\nDetails: [wiki](https://wiki.example.org/offsite)",
  "fallback": [
    { "id": "otO0JCJbJ88", "label": "Friday 15 January" }
  ],
  "group": "offsite-2027",
  "id": 2,
  "logo": null,
  "options": [
    { "id": "asWoSUcc7Ts", "label": "Thursday 14 January" },
    { "id": "otO0JCJbJ88", "label": "Friday 15 January" },
    { "id": "kDH9L13ZxRg", "label": "Saturday 16 January" }
  ],
  "preselection": null,
  "purge_date": "2027-01-31T00:00:00Z",
  "response": null,
  "result": null,
  "status": "unanswered",
  "submission_mode": "editable",
  "theme": "dark",
  "title": "Team offsite: which date works?",
  "token": "fqps4Y5QXH9JOgytT1czCw",
  "url": "http://localhost:8080/b/fqps4Y5QXH9JOgytT1czCw",
  "voter_reference": "alice@example.org",
  "voting_deadline": "2026-12-20T17:00:00Z"
}
```

### Errors

`422 invalid_input` names the first offending field (see [Errors](#errors) for all messages). `409 duplicate_voter_reference` means another live Ballot in the same Group already has this Voter Reference. A Ballot whose Purge Date has passed no longer blocks it, even before the sweep:

```json
{
  "error": "duplicate_voter_reference",
  "message": "another Ballot in this Group already has this Voter Reference"
}
```

Because of this rule, a creation script that sets a Group and a Voter Reference on every Ballot is safe to re-run: Ballots that already exist give `409`, and no duplicate is created.

### Many Ballots at once

One request can create up to 1000 Ballots that are alike but for their Voter References. Each gets its own ID, Ballot Token, `url` and Option IDs. There are two forms:

- **A list**: `"voter_reference": ["alice", "bob", "carol"]` creates one Ballot per entry, in this order. Each entry follows the rules of a single Voter Reference. The list must hold 1 to 1000 entries and no entry twice (compared after normalisation, like stored ones). A Group is optional, as for one Ballot.
- **A count**: `"count": 3` with a `group` creates 3 Ballots whose Voter References are plain numbers, continuing after the highest purely numeric Voter Reference in the Group. `count` must be 1 to 1000, needs a `group`, and can't be combined with `voter_reference` (string or list).

**Numbering.** A Voter Reference is *purely numeric* when it consists of one or more ASCII digits `0`–`9` and nothing else: no sign, space, separator, decimal point, exponent or digits of another script (`-5`, `+5`, `1e9`, `１２` and `٣` are not numeric). Its number is its decimal value, so leading zeros don't count (`007` is 7). The highest such number among the Group's live Ballots is *n* (0 if there is none); the new Voter References are `n+1`, `n+2`, … written in plain decimal without leading zeros. Other Groups, and Ballots past their Purge Date, don't count. Example: a Group holding `3`, `007`, `12`, `alice` and `99a` gets `13`, `14` from `"count": 2`. The numbers can't clash with existing Voter References. Numbers stop at 18446744073709551615 (2⁶⁴−1): a Group whose numbering would pass it gets `409 numbering_exhausted`.

**All or nothing.** The Ballots are created in one database transaction. If any listed Voter Reference is already taken by a live Ballot in the Group, nothing is created, and the `409 duplicate_voter_reference` names all of them in `voter_references`, in the order of the request:

```json
{
  "error": "duplicate_voter_reference",
  "message": "another Ballot in this Group already has this Voter Reference",
  "voter_references": ["carol@example.org", "alice@example.org"]
}
```

A `422` for any field (for example `voter_reference[2]`: `duplicate Voter Reference`) also creates nothing.

**Answer.** The shape follows the request's form: a string `voter_reference` (or none) answers with the single admin view as above; a list or a `count` answers `201 Created` with `{"ballots": [...]}`, the admin view of each new Ballot in order, even for a list of one:

```sh
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "Room booking", "options": [{"label": "Yes"}, {"label": "No"}], "voting_deadline": "2026-12-01T12:00:00Z", "group": "parking", "count": 2}'
```

```json
{
  "ballots": [
    { "id": 9, "group": "parking", "voter_reference": "1", "token": "bngbdaaJHVo9VV01r4_SkA", "url": "http://localhost:8080/b/bngbdaaJHVo9VV01r4_SkA", … },
    { "id": 10, "group": "parking", "voter_reference": "2", "token": "HiqbocnI4J3rB3ylDE6yvQ", "url": "http://localhost:8080/b/HiqbocnI4J3rB3ylDE6yvQ", … }
  ]
}
```

(Shortened, so not valid JSON as shown: each entry is the full admin view, with every key of a single create's answer.)

**Body size.** The [64 KiB body limit](#errors) applies as to any request. 1000 Voter References of up to about 50 characters fit; for longer ones, send the list in several requests (each is all or nothing on its own), or use `count`.

The CLI sends these forms with a repeated [`--voter-reference`, `--voters FILE` or `--count N`](cli.md#many-ballots-at-once).

## `GET /admin/ballots/{id}`: fetch one Ballot

Answers `200` with the [admin view](#the-admin-view-of-a-ballot), or `404 {"error":"not_found"}` if there is no live Ballot with this ID.

```sh
curl -s http://localhost:8080/admin/ballots/2 -H "Authorization: Bearer $TOKEN"
```

The response has the same shape as the creation response above.

## `PATCH /admin/ballots/{id}`: edit a Ballot

The body holds any subset of the creation fields. Fields you leave out keep their current values. The rules are applied to the Ballot *as the edit leaves it*, and the whole result is validated like a new Ballot. Answers `200` with the admin view.

### What may be edited when

- **Content edit**: the body names any field other than `group` and `voter_reference`. This is allowed only while the Ballot is **Unanswered**, or Answered only by its [Preselection](#preselection) while voting is open:
  - once it has a Response of the voter (Answered): `409 already_answered`;
  - once its Voting Deadline has passed with the Preselection as its Response: `409 already_answered` too ([extend](#post-adminballotsidextend-extend-the-voting-deadline) the deadline first);
  - once its Voting Deadline has passed without a Response and its Fallback has applied (Auto-fallback): `409 fallback_applied`. [Extend](#post-adminballotsidextend-extend-the-voting-deadline) the deadline first; that makes the Ballot Unanswered again, and content edits are allowed again.

  Either way nothing changes. The check is made before the values are checked, so an edit of such a Ballot with an invalid value (an empty title, say) still gets the `409`. A body that doesn't have the right shape is refused first, with `422`: bad JSON, an unknown field, a wrong JSON type, or `null` where it isn't allowed. The check is made again in the same write as the edit, at the request's time, so an edit arriving exactly at the deadline gets `409 fallback_applied` too. A Ballot whose deadline passed **without** a Fallback is still Unanswered and stays editable; a new `voting_deadline` reopens it, and a new `fallback` applies at once: the Ballot becomes Auto-fallback, with that Fallback as its result, and further content edits get `409 fallback_applied`.
- **Bookkeeping edit**: the body names only `group` and/or `voter_reference`, or nothing at all (`{}` answers `200` with the Ballot unchanged). This is allowed at any time, also on an Answered or Auto-fallback Ballot, because the voter never sees these fields. It does not count as a change of the Ballot for a Response being submitted at the same moment.

Whether an edit is a content edit depends on which fields are present, not on whether their values change. `{"title": "<the same title>"}` is still a content edit.

### Preselection

A **Preselection** is a set of the Ballot's Options that the voting page shows ticked before the voter does. What it means depends on the Submission Mode:

- **Final**: only the ticks. Nothing is recorded; the voter still submits and confirms, as without a Preselection. Until then the Ballot is Unanswered, and the Fallback (if any) applies at the deadline as usual.
- **Editable**: the Preselection is **saved as the answer already**. Creating the Ballot records it as the Response, with `response.origin` `"preselection"` (and `result.source` `"preselection"`): the Ballot is Answered from the start, the Fallback never applies, and the voter can change the answer until the deadline. Once the voter saves anything, **even the very same Options**, the Response is the voter's (`origin` `"voter"`, `result.source` `"response"`; `answered_at` becomes the time of that save), and the usual rules apply.

While voting is open and the only Response is the Preselection's, the voter hasn't acted, so **content edits stay allowed**: the Ballot is edited as if Unanswered, and the edit records the resulting Preselection anew as the Response (answered at the time of the edit), or none if the edit removes the Preselection or makes the Ballot Final. Making a Final Ballot with a Preselection Editable records it. Once the voter saves, content edits get `409 already_answered`. Once the deadline has passed with the Preselection as the Response, it is the recorded result, so content edits get `409 already_answered` until the deadline is [extended](#post-adminballotsidextend-extend-the-voting-deadline), as on an Auto-fallback Ballot.

### Field semantics on edit

| Field                                           | Absent        | `null`                         | Value |
| ----------------------------------------------- | ------------- | ------------------------------ | ----- |
| `title`, `choice_mode`, `submission_mode`       | kept          | `422 wrong type or value`      | as on creation |
| `allow_no_option`                               | kept: re-checked against the resulting Choice Mode | `422 wrong type or value` | `true` or `false`, checked against the resulting Choice Mode |
| `theme`                                         | kept (not re-checked, even if the Theme has since been removed) | `422` (name `"classic"` to go back to the default) | must exist now |
| `description`, `group`, `voter_reference`, `accent_color` | kept | removes the value             | as on creation |
| `logo`                                          | kept (not re-checked) | removes the logo       | must exist now |
| `fallback`                                      | kept: re-resolved by label against the resulting Options and re-checked against the resulting Choice Mode | removes the Fallback | labels of the resulting Options |
| `preselection`                                  | kept, like `fallback` (rejected with a `422` naming `preselection` when one of its Options is gone) | removes the Preselection | labels of the resulting Options |
| `options`                                       | kept, with their IDs | `422`                  | **replaces all Options**; every Option gets a new ID |
| `voting_deadline`                               | kept          | `422`                          | in the future, within the horizon. It may be earlier than the current one (the Ballot is Unanswered, as for every content edit). |
| `purge_date`                                    | kept, or shifted when the new `voting_deadline` passes it (see below) | `422` | in the future, within the horizon, not before the resulting Voting Deadline |

When `voting_deadline` moves past the current Purge Date and no `purge_date` is given, the Purge Date moves to the new deadline plus the current gap between the two dates. See [Deadlines, Fallback and Purge](../concepts/deadlines-fallback-purge.md). The shifted Purge Date must lie within the horizon too (no later than now + [`max_date_horizon`](configuration.md#max_date_horizon)). If it would not, the edit is refused with a `422` naming `voting_deadline`, since moving the deadline is what shifts it, and nothing changes. Give a `purge_date` within the horizon in the same request instead. (An [extension](#post-adminballotsidextend-extend-the-voting-deadline) shifts the Purge Date without this cap.)

For example, with the default horizon of one year, a Ballot whose Purge Date lies 7 days after its deadline can't have its deadline moved to 360 days from now on its own:

```http
HTTP/1.1 422 Unprocessable Entity
content-type: application/json
x-content-type-options: nosniff

{"error":"invalid_input","field":"voting_deadline","message":"would move the Purge Date further ahead than max_date_horizon allows; give a purge_date as well"}
```

**Allow No Option on edit.** Turning it on or off is a content edit like any other: only while the Ballot is Unanswered, and the voter's page must be reloaded before a Response is accepted (the revision changes). So it can't be turned off under a recorded Response of no Option: once the voter has answered (with or without Options), the edit gets `409 already_answered`, and the Response stays as it was. A Response of no Option sent on the page as it was before the admin turned the setting off gets `409 ballot_changed`. Switching `choice_mode` to `"single"` while it is on is a `422` naming `choice_mode`; send `"allow_no_option": false` in the same request. (It is never switched off silently.)

If you replace the Options and the current Fallback names a label that no longer exists, the edit is rejected. Give the Fallback again (or `null`) in the same request:

```json
{
  "error": "invalid_input",
  "field": "fallback",
  "message": "the Fallback's Options are no longer all Options of this Ballot; give the Fallback again"
}
```

Read-only fields of the admin view (`id`, `token`, `url`, `created_at`, `status`, …) are unknown fields here: `{"field": null, "message": "unknown field"}`.

### Examples

A content edit of an Answered Ballot is refused:

```sh
curl -s -i -X PATCH http://localhost:8080/admin/ballots/2 \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "New title"}'
```

```http
HTTP/1.1 409 Conflict
content-type: application/json
x-content-type-options: nosniff

{"error":"already_answered","message":"this Ballot has a Response; it can no longer be edited"}
```

So is a content edit of an Auto-fallback Ballot (here Ballot 3, created with the Options `Wed` and `Thu`, a deadline 20 seconds ahead and the Fallback `["Wed"]`, then left to pass the deadline):

```sh
curl -s -i -X PATCH http://localhost:8080/admin/ballots/3 \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "New title"}'
```

```http
HTTP/1.1 409 Conflict
content-type: application/json
x-content-type-options: nosniff

{"error":"fallback_applied","message":"the Fallback of this Ballot has applied; extend the Voting Deadline to edit it again"}
```

After `POST /admin/ballots/3/extend` with a new deadline, the same request answers `200` with `"status":"unanswered"` and `"title":"New title"`.

A bookkeeping edit of the Answered Ballot works (and of an Auto-fallback one, too):

```sh
curl -s -X PATCH http://localhost:8080/admin/ballots/2 \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"voter_reference": "alice@example.com"}'
```

`200` with the admin view, now with `"voter_reference": "alice@example.com"`.

Setting a Fallback and an accent colour, then clearing them again (on Ballot 4, an Unanswered Ballot whose Options are `Wed` and `Thu`):

```sh
curl -s -X PATCH http://localhost:8080/admin/ballots/4 \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"fallback": ["Wed"], "accent_color": "#3366CC"}'
# → … "accent_color":"#3366cc", … "fallback":[{"id":"vKsqzX4Cufs","label":"Wed"}] …

curl -s -X PATCH http://localhost:8080/admin/ballots/4 \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"accent_color": null, "fallback": null, "description": null}'
# → … "accent_color":null, … "fallback":null …
```

## `POST /admin/ballots/{id}/extend`: extend the Voting Deadline

Moves the Voting Deadline later. Allowed at any time before the Purge Date, also on Answered Ballots. On a closed Ballot without a Response, it reopens voting (an Auto-fallback Ballot becomes Unanswered again). Answers `200` with the admin view.

| Field             | Type            | Required | Rules |
| ----------------- | --------------- | -------- | ----- |
| `voting_deadline` | [timestamp](#conventions) string | yes      | In the future, no later than now + `max_date_horizon`, and **later than the current Voting Deadline**. |

No other field is accepted (`purge_date` is an unknown field here). If the new deadline passes the Purge Date, the Purge Date shifts to keep its current gap. The shifted Purge Date is computed, so it is not capped by the horizon.

```sh
curl -s -X POST http://localhost:8080/admin/ballots/2/extend \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"voting_deadline": "2027-02-15T12:00:00Z"}'
```

The Ballot's deadline was `2026-12-20T17:00:00Z` and its Purge Date `2027-01-31T00:00:00Z`, a gap of 41 days 7 hours. The new deadline passes the Purge Date, so the Purge Date shifts. The `200` response is the full admin view; these are the fields that changed:

```json
"purge_date": "2027-03-28T19:00:00Z",
"voting_deadline": "2027-02-15T12:00:00Z"
```

An earlier deadline is refused:

```json
{
  "error": "invalid_input",
  "field": "voting_deadline",
  "message": "must be later than the current Voting Deadline"
}
```

To move a deadline *earlier*, use `PATCH` while the Ballot is Unanswered (no Response, and no Fallback applied yet).

## `DELETE /admin/ballots/{id}`: delete a Ballot

Deletes the Ballot at once, with its Options, Response and Fallback, and frees its Voter Reference. Answers `204 No Content`. Afterwards the Ballot Token gets the voter surfaces' uniform not-found, exactly like a token that never existed.

```sh
curl -s -i -X DELETE http://localhost:8080/admin/ballots/2 -H "Authorization: Bearer $TOKEN"
```

```http
HTTP/1.1 204 No Content
x-content-type-options: nosniff
```

Deleting again, or deleting a purged Ballot, gives `404 {"error":"not_found"}`.

## `GET /admin/ballots`: list Ballots with results

Lists every matching live Ballot in creation order (internal ID ascending), each in the full [admin view](#the-admin-view-of-a-ballot) with its raw result. There is no pagination and no tally across Ballots: [`vota ballot tally`](cli.md#ballot-tally) sums one up in the client from this list, and [`vota ballot list --csv`](cli.md#ballot-list) exports it as CSV.

| Query parameter | Rules |
| --------------- | ----- |
| `group`         | Only Ballots in this Group. Sanitized exactly like a Group on creation (trimmed, NFC-normalised, 1–200 characters), so `?group=%20offsite%20` matches `offsite`. Matched literally (no patterns). |
| `status`        | `unanswered`, `answered` or `auto_fallback`, compared with the Ballot Status derived at request time. |

Each parameter may appear at most once. Unknown parameters are rejected. Without parameters, all live Ballots are listed. An empty result is `{"ballots": []}`, not an error.

```sh
curl -s 'http://localhost:8080/admin/ballots?group=offsite-2027&status=answered' \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "ballots": [
    {
      "accent_color": "#e07a1f",
      "allow_no_option": true,
      "choice_mode": "multiple",
      "created_at": "2026-10-17T10:51:05.902Z",
      "description": "Pick **every** date you can make.\n\nDetails: [wiki](https://wiki.example.org/offsite)",
      "fallback": [
        { "id": "otO0JCJbJ88", "label": "Friday 15 January" }
      ],
      "group": "offsite-2027",
      "id": 2,
      "logo": null,
      "options": [
        { "id": "asWoSUcc7Ts", "label": "Thursday 14 January" },
        { "id": "otO0JCJbJ88", "label": "Friday 15 January" },
        { "id": "kDH9L13ZxRg", "label": "Saturday 16 January" }
      ],
      "preselection": null,
      "purge_date": "2027-01-31T00:00:00Z",
      "response": {
        "answered_at": "2026-10-17T10:51:05.966Z",
        "changed_at": "2026-10-17T10:51:05.966Z",
        "option_ids": ["asWoSUcc7Ts"],
        "origin": "voter"
      },
      "result": {
        "option_ids": ["asWoSUcc7Ts"],
        "options": [
          { "id": "asWoSUcc7Ts", "label": "Thursday 14 January" }
        ],
        "source": "response"
      },
      "status": "answered",
      "submission_mode": "editable",
      "theme": "dark",
      "title": "Team offsite: which date works?",
      "token": "fqps4Y5QXH9JOgytT1czCw",
      "url": "http://localhost:8080/b/fqps4Y5QXH9JOgytT1czCw",
      "voter_reference": "alice@example.org",
      "voting_deadline": "2026-12-20T17:00:00Z"
    }
  ]
}
```

## The admin view of a Ballot

Every successful Ballot response (create, fetch, edit, extend, and each list entry) has these fields:

| Field             | Type                      | Meaning |
| ----------------- | ------------------------- | ------- |
| `id`              | integer                   | Internal ID, used in admin paths. |
| `token`           | string (22 chars)         | The Ballot Token: 128 random bits, base64url without padding. The voter's only credential. |
| `url`             | string                    | `<public_base_url>b/<token>` (e.g. `https://example.org/polls/b/<token>` under a sub-path), built from configuration only (never from request headers). |
| `title`           | string                    | As normalised on input. |
| `description`     | string or `null`          | Markdown source, not HTML. |
| `options`         | array of `{id, label}`    | In display order. `id` is 11 base64url characters (64 random bits), generated by the server. |
| `choice_mode`     | `"single"` / `"multiple"` | |
| `allow_no_option` | boolean                   | Allow No Option: whether a Response may select no Option. Only ever `true` on a multiple-choice Ballot. |
| `submission_mode` | `"final"` / `"editable"`  | |
| `voting_deadline` | RFC 3339 UTC              | Exclusive: voting is closed *at* this instant. |
| `purge_date`      | RFC 3339 UTC              | From this instant on the Ballot is gone. Never before `voting_deadline`. |
| `created_at`      | RFC 3339 UTC              | |
| `group`           | string or `null`          | Admin only, never shown to the voter. |
| `voter_reference` | string or `null`          | Admin only, never shown to the voter. |
| `theme`           | string                    | The Theme name as stored. If that Theme was removed from the theme folder since, the voting page uses `classic` instead, but this field still shows the stored name. |
| `accent_color`    | `#rrggbb` or `null`       | Lowercase. |
| `logo`            | string or `null`          | The logo name as stored (it may have been removed from the logo folder since; the page then shows none). |
| `fallback`        | array of `{id, label}` or `null` | The Fallback's Options in Option order. |
| `preselection`    | array of `{id, label}` or `null` | The [Preselection](#preselection)'s Options in Option order. |
| `status`          | `"unanswered"` / `"answered"` / `"auto_fallback"` | Derived at request time; see [Ballot lifecycle](../concepts/ballot-lifecycle.md). |
| `result`          | object or `null`          | `{source, option_ids, options}`: `source` is `"response"` (the voter's Response), `"preselection"` (the Preselection, recorded as the Response of an Editable Ballot the voter hasn't saved) or `"fallback"`, and `options` holds `{id, label}` of `option_ids`. `null` while there is no Response and the Fallback doesn't apply (yet). Empty `option_ids` and `options` only for a Response of no Option (Allow No Option): that is an answer, unlike `null`. |
| `response`        | object or `null`          | `{option_ids, answered_at, changed_at, origin}`: `answered_at` is when the first Response came, `changed_at` when the latest did (equal on Final Ballots). `option_ids` is `[]` for a Response of no Option; `null` means no Response. `origin` is `"voter"`, or `"preselection"` for the [Preselection](#preselection) recorded on an Editable Ballot (then `answered_at` is when it was recorded, by the creation or the latest edit). |

## Errors

| Status | Body | When |
| ------ | ---- | ---- |
| `400` | `{"error":"bad_request","message":"the X-Forwarded-For header could not be read"}` | A trusted proxy passed on an unreadable `X-Forwarded-For` ([rate limit](#rate-limit-on-failed-admin-token-checks)). |
| `401` | empty, `WWW-Authenticate: Bearer` | Missing, malformed, unknown, revoked or expired Admin Token, on every path under `/admin`. |
| `404` | `{"error":"not_found"}` | No live Ballot with this ID: unknown, deleted, purged, malformed (`007`, `-1`, `abc`) or percent-encoded (`%31`). |
| `404` | empty | Any other path under `/admin` (with a valid Admin Token), e.g. `/admin/nothing` or `/admin/ballots/`. |
| `405` | empty, with `Allow` header | Wrong method on a known path, e.g. `PUT /admin/ballots/1`. |
| `409` | `{"error":"already_answered","message":"this Ballot has a Response; it can no longer be edited"}` | Content edit of a Ballot that has a Response of the voter, or that closed with its Preselection as the Response. (The [voter API](voter-api.md) uses the same code with its own message, `this Ballot already has a Response, which can't be changed`.) |
| `409` | `{"error":"fallback_applied","message":"the Fallback of this Ballot has applied; extend the Voting Deadline to edit it again"}` | Content edit of an Auto-fallback Ballot (deadline passed without a Response, Fallback applied). |
| `409` | `{"error":"duplicate_voter_reference","message":"another Ballot in this Group already has this Voter Reference"}` | Create or edit would give two live Ballots in one Group the same Voter Reference. A [bulk create](#many-ballots-at-once) adds `"voter_references"`, the list of the request's Voter References that are taken. |
| `409` | `{"error":"numbering_exhausted","message":"the Group's numeric Voter References are too large to number on"}` | A create with `count` whose numbers would pass 2⁶⁴−1 ([numbering](#many-ballots-at-once)). |
| `413` | `Failed to buffer the request body: length limit exceeded` (text/plain) | Body over 64 KiB. |
| `415` | empty | `Content-Type` is not `application/json`. |
| `422` | `{"error":"invalid_json","message":"the request body is not valid JSON"}` | The body is not valid JSON: truncated, empty, not UTF-8, or trailing data after the object (see below). |
| `422` | `{"error":"invalid_input","field":…,"message":…}` | Validation failed (see below). |
| `429` | `{"error":"too_many_requests"}`, `Retry-After` | The client spent its budget of failed Admin Token checks ([rate limit](#rate-limit-on-failed-admin-token-checks)). |
| `500` | `{"error":"internal"}` | Internal error, without details, on every path under `/admin` (the Admin Token check included). It is logged without request data: no Admin Token, Ballot Token or Voter Reference. Carries the [error-page security headers](../concepts/security-headers.md#error-responses), like the voter API's. |

### `422 invalid_json`

A body that does not parse as JSON at all gets one fixed answer, without a `field`, whatever is wrong with it:

```sh
curl -s -X POST http://localhost:8080/admin/ballots -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"title": "Lunch?", "options": [{"label": "Piz'
# {"error":"invalid_json","message":"the request body is not valid JSON"}
```

Valid JSON of the wrong shape (a wrong type, an unknown or missing field, `[]` instead of an object) is `invalid_input` with the field's path instead.

### `422 invalid_input`

```json
{ "error": "invalid_input", "field": "options[1].label", "message": "duplicate Option label" }
```

`field` is a JSON path into the request body (`title`, `options[1].label`, `fallback[0]`), or a query parameter name. It is `""` when the body as a whole is wrong, and `null` for a field or parameter the endpoint doesn't know: the request's own names are never echoed, and neither is the rejected value. Only the first problem is reported. Possible messages:

| Message | Fields |
| ------- | ------ |
| `missing` | any required field |
| `unknown field` | `null`: a field (at any depth, e.g. `id` in an Option) or query parameter the endpoint doesn't know |
| `wrong type or value` | wrong JSON type, or `null` where not allowed |
| `must be a JSON object` | `""`: the body is not an object (e.g. `[]`) |
| `must not be empty` | text fields, `group` filter |
| `contains control, invisible or unassigned characters` | text fields |
| `too many combining marks` | text fields (more than four on one base character) |
| `too long` | text fields (200 characters; `description` 10 000) |
| `at least two Options are required` | `options` |
| `duplicate Option label` | `options[i].label` |
| `at least one Voter Reference is required` / `at most 1000 Voter References per request` | `voter_reference` (a list) |
| `duplicate Voter Reference` | `voter_reference[i]` (a list) |
| `must be at least 1` / `at most 1000 Ballots per request` / `requires a group` / `can't be combined with voter_reference` | `count` |
| `not an Option of this Ballot` / `duplicate Option` | `fallback[i]` |
| `select an Option` / `single choice: select exactly one Option` | `fallback` |
| `the Fallback's Options are no longer all Options of this Ballot; give the Fallback again` | `fallback` (edit) |
| `not an Option of this Ballot` / `duplicate Option` | `preselection[i]` |
| `select an Option` / `single choice: select exactly one Option` | `preselection` |
| `the Preselection's Options are no longer all Options of this Ballot; give the Preselection again` | `preselection` (edit) |
| `not an RFC 3339 timestamp, or a local date-time in the server's default time zone (YYYY-MM-DDTHH:MM[:SS])` | `voting_deadline`, `purge_date` |
| `does not exist in the server's default time zone (skipped by a clock change); give a UTC offset` | `voting_deadline`, `purge_date` (a local date-time) |
| `is ambiguous in the server's default time zone (repeated by a clock change); give a UTC offset` | `voting_deadline`, `purge_date` (a local date-time) |
| `must lie in the future` | `voting_deadline`, `purge_date` |
| `must not lie further ahead than max_date_horizon allows` | `voting_deadline`, `purge_date` |
| `must not be before the Voting Deadline` | `purge_date` |
| `would move the Purge Date further ahead than max_date_horizon allows; give a purge_date as well` | `voting_deadline` (edit without `purge_date`) |
| `must be later than the current Voting Deadline` | `voting_deadline` (extend) |
| `must be one of "single", "multiple"` | `choice_mode` |
| `only a multiple-choice Ballot can allow no Option` | `allow_no_option` |
| `the Ballot allows no Option, which needs multiple choice; set allow_no_option to false as well` | `choice_mode` (edit to `"single"` while Allow No Option stays on) |
| `must be one of "final", "editable"` | `submission_mode` |
| `must be one of "unanswered", "answered", "auto_fallback"` | `status` query parameter |
| `must be given at most once` | repeated query parameter |
| `must be a name of lowercase letters, digits and '-' (at most 64)` | `theme`, `logo` |
| `no Theme of this name` / `no logo of this name` | `theme` / `logo` |
| `must be a hex colour of the form #rrggbb` | `accent_color` |
| `invalid UTF-8` | a query parameter whose value isn't UTF-8 after percent-decoding (`field` names the parameter); `null` when the parameter's name isn't UTF-8 |

Some real examples:

```sh
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "T", "options": [{"label": "A"}, {"label": "B"}], "voting_deadline": "2026-12-01 12:00"}'
# {"error":"invalid_input","field":"voting_deadline","message":"not an RFC 3339 timestamp, or a local date-time in the server's default time zone (YYYY-MM-DDTHH:MM[:SS])"}

# With default_time_zone = "Europe/Berlin":
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "T", "options": [{"label": "A"}, {"label": "B"}], "voting_deadline": "2027-10-31T02:30"}'
# {"error":"invalid_input","field":"voting_deadline","message":"is ambiguous in the server's default time zone (repeated by a clock change); give a UTC offset"}

curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "T", "options": [{"label": "A"}, {"label": "B"}], "voting_deadline": "2026-12-01T12:00:00Z", "fallback": ["A", "B"]}'
# {"error":"invalid_input","field":"fallback","message":"single choice: select exactly one Option"}

curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "T", "options": [{"label": "A"}, {"label": "B"}], "voting_deadline": "2026-12-01T12:00:00Z", "theme": "Dark"}'
# {"error":"invalid_input","field":"theme","message":"must be a name of lowercase letters, digits and '-' (at most 64)"}

curl -s 'http://localhost:8080/admin/ballots?status=closed' -H "Authorization: Bearer $TOKEN"
# {"error":"invalid_input","field":"status","message":"must be one of \"unanswered\", \"answered\", \"auto_fallback\""}
```
