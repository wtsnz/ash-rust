-- Migration: 20261002012957_rename_docks_to_berths.postgres.up.sql
-- Generated automatically by ash-rust.

ALTER TABLE "docks" RENAME TO "berths";

ALTER INDEX "idx_docks_port_code" RENAME TO "idx_berths_port_code";

ALTER TABLE "berths" RENAME CONSTRAINT "ck_docks_positive_mass" TO "ck_berths_positive_mass";

ALTER TABLE "berths" RENAME CONSTRAINT "ck_docks_clamp_one_of" TO "ck_berths_clamp_one_of";

ALTER TABLE "berths" RENAME CONSTRAINT "fk_docks_port" TO "fk_berths_port";

ALTER TABLE berth_reservations DROP CONSTRAINT IF EXISTS berth_reservations_no_overlap;

ALTER TABLE "berth_reservations" RENAME COLUMN "dock_code" TO "berth_code";

DROP INDEX IF EXISTS "idx_berth_reservations_active_by_berth";

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_active_by_berth" ON "berth_reservations" ("port_id", "berth_code") INCLUDE ("starts_at", "ends_at") WHERE status = 'active';

ALTER TABLE "berth_reservations" DROP CONSTRAINT IF EXISTS "fk_berth_reservations_berth";

ALTER TABLE "berth_reservations" ADD CONSTRAINT "fk_berth_reservations_berth" FOREIGN KEY ("port_id", "berth_code") REFERENCES "berths" ("port_id", "code") ON DELETE RESTRICT ON UPDATE CASCADE;

ALTER TABLE berth_reservations ADD CONSTRAINT berth_reservations_no_overlap EXCLUDE USING gist (port_id WITH =, berth_code WITH =, tstzrange(starts_at, ends_at) WITH &&) WHERE (status = 'active');
