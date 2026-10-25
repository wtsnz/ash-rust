-- Migration: 20261025081404_durable_sessions.sqlite.up.sql
-- Generated automatically by ash-rust.

ALTER TABLE "crew_members" ADD COLUMN "credential_version" INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS "ash_tokens" (
  "id" TEXT PRIMARY KEY,
  "resource" TEXT NOT NULL,
  "tenant_key" TEXT NOT NULL,
  "jti" TEXT NOT NULL,
  "subject" TEXT NOT NULL,
  "purpose" TEXT NOT NULL,
  "expires_at" INTEGER NOT NULL,
  "revoked" INTEGER NOT NULL DEFAULT 0,
  "consumed" INTEGER NOT NULL DEFAULT 0,
  "extra" TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ash_tokens_token_namespace" ON "ash_tokens" ("resource", "tenant_key", "subject", "jti");
