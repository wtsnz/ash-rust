-- Legacy rows lack a resource/tenant namespace and cannot authorize versioned JWTs.
-- Deploy subject credential_version columns before issuing new sessions.
DELETE FROM ash_tokens;
ALTER TABLE ash_tokens ADD COLUMN resource TEXT NOT NULL;
ALTER TABLE ash_tokens ADD COLUMN tenant_key TEXT NOT NULL;
ALTER TABLE ash_tokens ADD COLUMN consumed BOOLEAN NOT NULL DEFAULT FALSE;
CREATE UNIQUE INDEX idx_ash_tokens_token_namespace ON ash_tokens (resource, tenant_key, subject, jti);
