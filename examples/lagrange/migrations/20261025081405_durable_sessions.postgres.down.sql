-- Migration: 20261025081405_durable_sessions.postgres.down.sql
-- Generated automatically by ash-rust.

ALTER TABLE "crew_members" DROP COLUMN "credential_version";

DROP TABLE IF EXISTS "ash_tokens";
