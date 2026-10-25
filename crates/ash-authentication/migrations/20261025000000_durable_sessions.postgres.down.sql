-- Downgrades invalidate the new sessions too; session state is not recoverable.
DELETE FROM ash_tokens;
DROP INDEX idx_ash_tokens_token_namespace;
ALTER TABLE ash_tokens DROP COLUMN consumed;
ALTER TABLE ash_tokens DROP COLUMN tenant_key;
ALTER TABLE ash_tokens DROP COLUMN resource;
