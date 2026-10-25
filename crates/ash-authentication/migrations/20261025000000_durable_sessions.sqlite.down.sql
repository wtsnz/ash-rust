-- Downgrades invalidate the new sessions too; session state is not recoverable.
DROP TABLE ash_tokens;
CREATE TABLE ash_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    jti TEXT NOT NULL,
    subject TEXT NOT NULL,
    purpose TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0,
    extra TEXT
);
