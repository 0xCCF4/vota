-- When each Admin Token was last accepted by the admin API (UTC Unix
-- milliseconds), recorded at most once a minute per token; NULL until its
-- first use, and for every token from before this migration.
ALTER TABLE admin_tokens ADD COLUMN last_used_at INTEGER;
