-- Rebuild the token table: legacy rows have no trustworthy resource/tenant namespace.
DROP TABLE ash_tokens;
CREATE TABLE ash_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    resource TEXT NOT NULL,
    tenant_key TEXT NOT NULL,
    jti TEXT NOT NULL,
    subject TEXT NOT NULL,
    purpose TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0,
    consumed INTEGER NOT NULL DEFAULT 0,
    extra TEXT
);
CREATE UNIQUE INDEX idx_ash_tokens_token_namespace ON ash_tokens (resource, tenant_key, subject, jti);
