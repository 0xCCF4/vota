-- Preselection (ticket 48). A Ballot's optional Preselection is a non-empty
-- set of its own Options that the voting page shows ticked, stored as
-- membership rows like the Fallback's; no rows means no Preselection. The
-- composite foreign key lets it point only at its own Ballot's Options.
CREATE TABLE preselected_options (
    ballot_id  INTEGER NOT NULL REFERENCES ballots (id) ON DELETE CASCADE,
    option_id  TEXT    NOT NULL,
    PRIMARY KEY (ballot_id, option_id),
    FOREIGN KEY (ballot_id, option_id) REFERENCES ballot_options (ballot_id, id) ON DELETE CASCADE
) STRICT;

-- Where a Response comes from: the voter ('voter'), or, on an Editable
-- Ballot, the Preselection recorded as the Response until the voter saves
-- one ('preselection'). Every existing Response is the voter's.
ALTER TABLE responses ADD COLUMN origin TEXT NOT NULL DEFAULT 'voter'
    CHECK (origin IN ('voter', 'preselection'));
