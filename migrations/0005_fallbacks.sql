-- Fallbacks (ticket 05). A Ballot's optional Fallback is a non-empty set of its
-- own Options, stored as membership rows; no rows means no Fallback. The
-- composite foreign key lets a Fallback point only at its own Ballot's Options.
CREATE TABLE fallback_options (
    ballot_id  INTEGER NOT NULL REFERENCES ballots (id) ON DELETE CASCADE,
    option_id  TEXT    NOT NULL,
    PRIMARY KEY (ballot_id, option_id),
    FOREIGN KEY (ballot_id, option_id) REFERENCES ballot_options (ballot_id, id) ON DELETE CASCADE
) STRICT;
