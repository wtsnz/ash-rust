-- Migration: 20261002012956_rename_docks_to_berths.sqlite.down.sql
-- Generated automatically by ash-rust.

DROP INDEX IF EXISTS "idx_berths_port_code";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_docks_port_code" ON "berths" ("port_id", "code");

ALTER TABLE "berths" RENAME TO "docks";

ALTER TABLE "berth_reservations" RENAME COLUMN "berth_code" TO "dock_code";

CREATE TABLE IF NOT EXISTS "berth_reservations__ash_new" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "port_id" TEXT NOT NULL,
  "dock_code" TEXT NOT NULL,
  "ship_id" TEXT NOT NULL,
  "starts_at" TEXT NOT NULL,
  "ends_at" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'active',
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_berth_reservations_window" CHECK (ends_at > starts_at),
  CONSTRAINT "ck_berth_reservations_status_one_of" CHECK ("status" IN ('active', 'released')),
  CONSTRAINT "fk_berth_reservations_berth" FOREIGN KEY ("port_id", "dock_code") REFERENCES "docks" ("port_id", "code") ON DELETE RESTRICT ON UPDATE CASCADE,
  CONSTRAINT "fk_berth_reservations_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE NO ACTION
);

INSERT INTO "berth_reservations__ash_new" ("id", "line", "port_id", "dock_code", "ship_id", "starts_at", "ends_at", "status", "created_at", "updated_at")
SELECT "id", "line", "port_id", "dock_code", "ship_id", "starts_at", "ends_at", "status", "created_at", "updated_at" FROM "berth_reservations";

DROP TABLE "berth_reservations";

ALTER TABLE "berth_reservations__ash_new" RENAME TO "berth_reservations";

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_active_by_berth" ON "berth_reservations" ("port_id", "dock_code") WHERE status = 'active';

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_by_ship" ON "berth_reservations" ("ship_id");
