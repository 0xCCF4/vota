-- Ballot revision (ticket 07). Every admin edit bumps it; recording a Response
-- is conditional on the revision the voter's selection was validated against,
-- so a Response can never land on a Ballot whose Options, Choice Mode or text
-- changed in between. Extending the Voting Deadline doesn't bump it (the
-- deadline is checked in the same conditional write anyway).
ALTER TABLE ballots ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
