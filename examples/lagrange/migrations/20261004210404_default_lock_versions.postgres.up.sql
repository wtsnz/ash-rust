-- Migration: 20261004210404_default_lock_versions.postgres.up.sql
-- Generated automatically by ash-rust.

ALTER TABLE "berths" ALTER COLUMN "version" SET DEFAULT 1;

ALTER TABLE "ships" ALTER COLUMN "version" SET DEFAULT 1;

ALTER TABLE "contracts" ALTER COLUMN "version" SET DEFAULT 1;

ALTER TABLE "containers" ALTER COLUMN "version" SET DEFAULT 1;

ALTER TABLE "voyages" ALTER COLUMN "version" SET DEFAULT 1;
