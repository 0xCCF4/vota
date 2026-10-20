-- Ballots and their Options (ticket 02: single choice, no Response yet).
-- Later tickets add their own columns/tables (Choice Mode, Submission Mode,
-- Responses, Fallback, Group, Voter Reference, Theme, description).
-- Timestamps are UTC, stored as Unix milliseconds.
CREATE TABLE ballots (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    token            TEXT    NOT NULL UNIQUE CHECK (length(token) = 22),
    title            TEXT    NOT NULL,
    voting_deadline  INTEGER NOT NULL,
    purge_date       INTEGER NOT NULL CHECK (purge_date >= voting_deadline),
    created_at       INTEGER NOT NULL
) STRICT;

CREATE TABLE ballot_options (
    id         TEXT    PRIMARY KEY CHECK (length(id) = 11),
    ballot_id  INTEGER NOT NULL REFERENCES ballots (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    label      TEXT    NOT NULL,
    UNIQUE (ballot_id, position),
    UNIQUE (ballot_id, label)
) STRICT;
