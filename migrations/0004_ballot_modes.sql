-- Choice Mode and Submission Mode (ticket 04). Existing Ballots were single
-- choice and Final, which the defaults keep.
ALTER TABLE ballots ADD COLUMN choice_mode TEXT NOT NULL DEFAULT 'single'
    CHECK (choice_mode IN ('single', 'multiple'));
ALTER TABLE ballots ADD COLUMN submission_mode TEXT NOT NULL DEFAULT 'final'
    CHECK (submission_mode IN ('final', 'editable'));
