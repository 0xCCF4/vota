-- Allow No Option (ticket 41): whether the voter of a multiple-choice Ballot
-- may answer with no Option at all ("none of these"). Off (0) for every
-- existing Ballot; never on for a single-choice one. Such a Response is a
-- `responses` row without `response_options` rows, so it stays distinct
-- from no Response (no `responses` row) and makes the Ballot Answered.
ALTER TABLE ballots ADD COLUMN allow_no_option INTEGER NOT NULL DEFAULT 0
    CHECK (allow_no_option IN (0, 1) AND (allow_no_option = 0 OR choice_mode = 'multiple'));
