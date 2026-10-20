# Voting Deadline, Fallback and Purge Date

Every Ballot has two dates: the **Voting Deadline**, which closes voting, and the **Purge Date**, which deletes the Ballot. The Purge Date is never before the Voting Deadline. The database enforces this too (`CHECK (purge_date >= voting_deadline)`). An optional **Fallback** decides the result when no Response arrives in time.

## Voting Deadline

- Given by the admin as RFC 3339 with an explicit offset, stored in UTC to the millisecond.
- **Exclusive**: voting is open while `now < voting_deadline`. At the deadline itself it is closed. A Response submitted at that instant gets `409 voting_closed` (API) or `409` and "Voting has closed. Your answer could not be accepted." (form).
- Must lie in the future when it is set (create, edit, extend), and no later than now + [`max_date_horizon`](../reference/configuration.md#max_date_horizon) (default one year). A date exactly at the horizon is accepted.
- The voter sees it on the page in the configured [`default_time_zone`](../reference/configuration.md#default_time_zone). With JavaScript it is shown in their local time, with a countdown.

### Changing the deadline

| How | When allowed | Direction | Purge Date |
| --- | ------------ | --------- | ---------- |
| `POST /admin/ballots/{id}/extend` | always (also when Answered) | must be **later** than the current deadline | shifted if needed, see below |
| `PATCH /admin/ballots/{id}` with `voting_deadline` | only while the Ballot has no Response and its Fallback has not applied (not Auto-fallback) | earlier or later, as long as it is in the future | shifted if needed, unless `purge_date` is given in the same request; a shifted date must lie within the horizon (otherwise `422` on `voting_deadline`) |

So once the Fallback has applied, the only way to move the deadline is `extend`. After that, `PATCH` works again.

### Reopening

Ballot Status is derived from the current time, never stored (see [Ballot lifecycle](ballot-lifecycle.md)). So moving the deadline into the future simply makes the Ballot open again:

- an **Auto-fallback** Ballot becomes **Unanswered**: `result` becomes `null`, the voter sees the form and the upcoming Fallback again, and the admin may edit its content again;
- a closed **Unanswered** Ballot without a Fallback shows the form again (a `PATCH` that instead gives such a Ballot a Fallback applies it at once: the Ballot becomes Auto-fallback);
- an **Answered** Editable Ballot accepts changes again; an Answered Final Ballot stays locked.

Real example: a Ballot whose deadline had just passed, with a Fallback, was extended by 20 days:

```text
before:  "status":"auto_fallback"  "result":{"option_ids":["-Yg5TgAhfDk"],"source":"fallback"}  "voting_open":false
after:   "status":"unanswered"     "result":null
```

## Purge Date

- Optional on creation. The default is Voting Deadline + [`default_purge_gap`](../reference/configuration.md#default_purge_gap) (4 weeks). The computed default is **not** capped by `max_date_horizon`.
- If the admin gives it (create or edit): in the future, within the horizon, and not before the (resulting) Voting Deadline. It may equal the deadline.
- **From the Purge Date on (inclusive), the Ballot is gone** for every request: the voting page and voter API answer with the uniform not-found, and the admin API with `404`. The list leaves it out, and its Voter Reference can be used again at once. This holds even before the sweep has deleted the row, because every Ballot lookup in the store filters `purge_date > now`.
- The **purge sweep** (`src/jobs/purge_sweep.rs`) deletes such Ballots, with their Options, Response, Fallback, Group and Voter Reference, once at startup and then every [`purge_interval`](../reference/configuration.md#purge_interval) (default 1 hour). It logs only the number of deleted Ballots.
- An admin can delete a Ballot earlier with `DELETE /admin/ballots/{id}`.
- **Deleted means deleted from the files**: every connection runs with `PRAGMA secure_delete = ON`, so the deleted rows are overwritten with zeros, and after a sweep that deleted Ballots, and after a `DELETE`, the write-ahead log is checkpointed and truncated. See [the purge sweep](../guides/deployment.md#the-purge-sweep) for what this can't reach (backups, snapshots, SSDs, and the write-ahead log between checkpoints).

### The Purge Date shift

When a new Voting Deadline would pass the current Purge Date, the Purge Date moves along and keeps the **current gap** between the two dates:

```text
new_purge_date = new_deadline + (current_purge_date − current_deadline)   if new_deadline > current_purge_date
new_purge_date = current_purge_date                                       otherwise
```

(`Ballot::purge_date_for_deadline` in `src/domain/ballot.rs`.) After an **edit**, the shifted date must lie within the horizon (now + `max_date_horizon`); otherwise the edit is refused with a `422` on `voting_deadline`, and the admin can give a `purge_date` in the same request. After an **extension** the shifted date is not capped.

Real examples:

- Deadline `2026-12-20T17:00:00Z`, Purge Date `2027-01-31T00:00:00Z` (gap 41 days 7 hours). Extending to `2027-02-15T12:00:00Z` gives Purge Date `2027-03-28T19:00:00Z`.
- Deadline `2026-12-01T10:00:00.123Z`, Purge Date `2026-12-03T10:00:00Z`. An edit moves the deadline *earlier* to `2026-11-01T00:00:00Z`; the Purge Date stays, so the gap is now 32 days 10 hours. A second edit moves it to `2026-12-24T00:00:00Z`, past the Purge Date, which therefore shifts by the current gap to `2027-01-25T10:00:00Z`.
- An extension that stays at or before the Purge Date moves only the deadline.

## Fallback

The Fallback is an optional set of the Ballot's own Options, chosen by the admin. It is recorded as the result if the voter has not responded by the Voting Deadline.

- **Given by Option labels** in the admin API (`"fallback": ["Friday 15 January"]`), because Options only get their IDs on creation, and new ones whenever the admin replaces them. Responses return it as `[{id, label}]` (admin) or `{"option_ids": [...]}` (voter).
- **Obeys the Choice Mode, never empty**: exactly one Option on a single-choice Ballot, one or more on a multiple-choice Ballot. No duplicates. Only this Ballot's Options.
- **Visible to the voter from the start**: the voting page says "If you don't answer by the deadline, this will be recorded: …", and the voter API has `fallback`. Only `result` and `status` say whether it has been applied.
- **Applied, not written**: when the deadline passes without a Response, `status` becomes `auto_fallback` and `result` is `{"source": "fallback", …}`. Nothing in the database changes, which is why an extension can undo it.
- **Locks the content once applied**: while the Ballot is Auto-fallback, a `PATCH` naming any field other than `group` and `voter_reference` gets `409 fallback_applied`, and nothing changes. This is checked before the values are looked at, and again in the same conditional write as the edit, at the request's time, so an edit can't land at or after the deadline. To change the Ballot anyway, extend the deadline first (see the example below). A Ballot whose deadline passed without a Fallback stays editable.
- **On edit** (only while there is no Response and the Fallback has not applied): `fallback` with labels replaces it, and `null` removes it. If `fallback` is absent, the current Fallback follows its labels into replaced Options and is re-checked against a changed Choice Mode. If a label is gone, the edit is rejected until the Fallback is given again.
- A Response always wins over the Fallback, even if it arrives one millisecond before the deadline.

Real example (a Ballot created with a deadline 20 seconds ahead and the Fallback `["Wed"]`, then left to pass the deadline): a content edit is refused, even with an invalid value; a bookkeeping edit works; after an extension the content edit goes through.

```text
GET    …/ballots/1                                   → "status":"auto_fallback"  "result":{…"source":"fallback"}
PATCH  {"title": "New title"}                        → 409 {"error":"fallback_applied","message":"the Fallback of this Ballot has applied; extend the Voting Deadline to edit it again"}
PATCH  {"title": ""}                                 → 409 {"error":"fallback_applied",…}
PATCH  {"voter_reference": "alice@example.com"}      → 200 "status":"auto_fallback"  "voter_reference":"alice@example.com"
POST   …/extend {"voting_deadline": "2026-10-24T15:01:51Z"} → 200 "status":"unanswered"  "result":null
PATCH  {"title": "New title"}                        → 200 "status":"unanswered"  "title":"New title"
```

## The revision guard and `ballot_changed`

A voter may have a Ballot open while the admin edits it. Every content edit therefore bumps the Ballot's `revision`. The voter sees it with the Ballot (the voter API's `revision`, a hidden `revision` field in the voting page's form and on the confirmation step) and sends it back with the Response, and a Response is only recorded if it is still the Ballot's revision, checked again in the same write that records it. So a Response is never recorded against wording, Options or rules the voter didn't see: not even when the admin negates the question between the no-JS confirmation step and the voter confirming. Otherwise nothing is recorded, and the voter gets:

- voter API: `409 {"error":"ballot_changed", …}`;
- form: `409` and the page as it now stands, with "This Ballot was changed by its organiser, so your answer was not saved. Please check it and answer again.";
- with JavaScript: a dialog with that message, then a reload.

The same answer is given when a submitted Option ID is well-formed but not (any more) one of the Ballot's Options. That is the usual case after the admin replaced the Options, since replaced Options get new IDs. Extensions and bookkeeping edits (Group, Voter Reference) don't change the revision, because they don't change what the voter answers.

## Related

- [Ballot lifecycle and Ballot Status](ballot-lifecycle.md)
- [Admin API: extend](../reference/admin-api.md#post-adminballotsidextend-extend-the-voting-deadline)
