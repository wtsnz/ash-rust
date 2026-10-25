-- Migration: 20261025081404_durable_sessions.sqlite.down.sql
-- Generated automatically by ash-rust.

ALTER TABLE "crew_members" DROP COLUMN "credential_version";

DROP TABLE IF EXISTS "ash_tokens";
