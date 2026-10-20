-- Markdown description (ticket 09): optional. Holds the sanitised Markdown
-- source, not rendered HTML, so the voting page renders it at display time
-- and an improved renderer or sanitizer applies to existing Ballots too.
ALTER TABLE ballots ADD COLUMN description TEXT;
