-- Responses (ticket 03). A Ballot has at most one Response: `ballot_id` is the
-- primary key, so recording a Final Response is a single conditional insert
-- that can never succeed twice. The selected Options are membership rows that
-- can only point at the Ballot's own Options (composite foreign key).
-- Timestamps are UTC, stored as Unix milliseconds.
CREATE UNIQUE INDEX ballot_options_ballot_option ON ballot_options (ballot_id, id);

CREATE TABLE responses (
    ballot_id    INTEGER PRIMARY KEY REFERENCES ballots (id) ON DELETE CASCADE,
    answered_at  INTEGER NOT NULL,
    changed_at   INTEGER NOT NULL CHECK (changed_at >= answered_at)
) STRICT;

CREATE TABLE response_options (
    ballot_id  INTEGER NOT NULL REFERENCES responses (ballot_id) ON DELETE CASCADE,
    option_id  TEXT    NOT NULL,
    PRIMARY KEY (ballot_id, option_id),
    FOREIGN KEY (ballot_id, option_id) REFERENCES ballot_options (ballot_id, id) ON DELETE CASCADE
) STRICT;
