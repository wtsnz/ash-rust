-- Migration: 20261004210404_default_lock_versions.postgres.down.sql
-- Generated automatically by ash-rust.

ALTER TABLE "berths" ALTER COLUMN "version" DROP DEFAULT;

ALTER TABLE "containers" ALTER COLUMN "version" DROP DEFAULT;

ALTER TABLE "voyages" ALTER COLUMN "version" DROP DEFAULT;

ALTER TABLE "contracts" ALTER COLUMN "version" DROP DEFAULT;

ALTER TABLE "ships" ALTER COLUMN "version" DROP DEFAULT;
