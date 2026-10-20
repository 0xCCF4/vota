-- Group and Voter Reference (ticket 06): optional admin bookkeeping. A Voter
-- Reference is unique within its Group; one without a Group (or a Group
-- without Voter References) is unconstrained.
ALTER TABLE ballots ADD COLUMN group_name TEXT;
ALTER TABLE ballots ADD COLUMN voter_reference TEXT;

CREATE UNIQUE INDEX ballots_group_voter_reference ON ballots (group_name, voter_reference)
    WHERE group_name IS NOT NULL AND voter_reference IS NOT NULL;

-- Admin listing filters by Group.
CREATE INDEX ballots_group ON ballots (group_name) WHERE group_name IS NOT NULL;
