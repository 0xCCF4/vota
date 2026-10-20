-- Admin Tokens: created by the local CLI; only the SHA-256 hash of the secret is
-- stored. Timestamps are UTC, stored as Unix milliseconds.
CREATE TABLE admin_tokens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    token_hash  BLOB    NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    label       TEXT,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER
) STRICT;
