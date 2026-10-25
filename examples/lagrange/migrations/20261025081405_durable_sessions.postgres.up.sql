-- Migration: 20261025081405_durable_sessions.postgres.up.sql
-- Generated automatically by ash-rust.

CREATE OR REPLACE FUNCTION ash_raise_error(json_data jsonb) RETURNS BOOLEAN AS $$ BEGIN RAISE EXCEPTION 'ash_error: %', json_data::text; RETURN NULL; END; $$ LANGUAGE plpgsql STABLE SET search_path = '';

ALTER TABLE "crew_members" ADD COLUMN "credential_version" BIGINT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS "ash_tokens" (
  "id" UUID PRIMARY KEY,
  "resource" TEXT NOT NULL,
  "tenant_key" TEXT NOT NULL,
  "jti" TEXT NOT NULL,
  "subject" UUID NOT NULL,
  "purpose" TEXT NOT NULL,
  "expires_at" BIGINT NOT NULL,
  "revoked" BOOLEAN NOT NULL DEFAULT FALSE,
  "consumed" BOOLEAN NOT NULL DEFAULT FALSE,
  "extra" TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ash_tokens_token_namespace" ON "ash_tokens" ("resource", "tenant_key", "subject", "jti");
