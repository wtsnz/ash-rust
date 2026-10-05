-- Migration: 20261004210403_default_lock_versions.sqlite.down.sql
-- Generated automatically by ash-rust.

CREATE TABLE "berth_reservations__ash_hold" AS SELECT * FROM "berth_reservations";

CREATE TABLE IF NOT EXISTS "berths__ash_new" (
  "id" TEXT PRIMARY KEY,
  "port_id" TEXT NOT NULL,
  "code" TEXT NOT NULL,
  "clamp" TEXT NOT NULL,
  "max_mass_tonnes" INTEGER NOT NULL,
  "version" INTEGER NOT NULL,
  CONSTRAINT "ck_berths_positive_mass" CHECK (max_mass_tonnes > 0),
  CONSTRAINT "ck_berths_clamp_one_of" CHECK ("clamp" IN ('standard', 'heavy_lift', 'cryogenic')),
  CONSTRAINT "fk_berths_port" FOREIGN KEY ("port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

INSERT INTO "berths__ash_new" ("id", "port_id", "code", "clamp", "max_mass_tonnes", "version")
SELECT "id", "port_id", "code", "clamp", "max_mass_tonnes", "version" FROM "berths";

DROP TABLE "berths";

ALTER TABLE "berths__ash_new" RENAME TO "berths";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_berths_port_code" ON "berths" ("port_id", "code");

DELETE FROM "berth_reservations";

INSERT INTO "berth_reservations" ("id", "line", "port_id", "berth_code", "ship_id", "starts_at", "ends_at", "status", "created_at", "updated_at") SELECT "id", "line", "port_id", "berth_code", "ship_id", "starts_at", "ends_at", "status", "created_at", "updated_at" FROM "berth_reservations__ash_hold";

DROP TABLE "berth_reservations__ash_hold";

CREATE TABLE "stowage__ash_hold" AS SELECT * FROM "stowage";

CREATE TABLE IF NOT EXISTS "containers__ash_new" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "code" TEXT COLLATE NOCASE NOT NULL,
  "contract_id" TEXT NOT NULL,
  "mass_tonnes" INTEGER NOT NULL,
  "hazmat" TEXT,
  "hazardous" INTEGER NOT NULL DEFAULT 0,
  "reefer" INTEGER NOT NULL DEFAULT 0,
  "manifest" TEXT,
  "seal" BLOB,
  "sealed" INTEGER NOT NULL DEFAULT 0,
  "version" INTEGER NOT NULL,
  "archived_at" TEXT,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_containers_positive_mass" CHECK (mass_tonnes > 0),
  CONSTRAINT "ck_containers_sealed_has_seal" CHECK (sealed = (seal IS NOT NULL)),
  CONSTRAINT "ck_containers_hazmat_one_of" CHECK ("hazmat" IN ('flammable', 'corrosive', 'radioactive', 'cryogenic')),
  CONSTRAINT "fk_containers_contract" FOREIGN KEY ("contract_id") REFERENCES "contracts" ("id") ON DELETE RESTRICT
);

INSERT INTO "containers__ash_new" ("id", "line", "code", "contract_id", "mass_tonnes", "hazmat", "hazardous", "reefer", "manifest", "seal", "sealed", "version", "archived_at", "created_at", "updated_at")
SELECT "id", "line", "code", "contract_id", "mass_tonnes", "hazmat", "hazardous", "reefer", "manifest", "seal", "sealed", "version", "archived_at", "created_at", "updated_at" FROM "containers";

DROP TABLE "containers";

ALTER TABLE "containers__ash_new" RENAME TO "containers";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_containers_live_code" ON "containers" ("line", "code") WHERE archived_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_containers_by_contract" ON "containers" ("line", "contract_id");

CREATE INDEX IF NOT EXISTS "idx_containers_hazardous_by_contract" ON "containers" ("line", "contract_id") WHERE hazardous;

DELETE FROM "stowage";

INSERT INTO "stowage" ("id", "line", "voyage_id", "container_id", "bay", "stack", "tier") SELECT "id", "line", "voyage_id", "container_id", "bay", "stack", "tier" FROM "stowage__ash_hold";

DROP TABLE "stowage__ash_hold";

CREATE TABLE "stowage__ash_hold" AS SELECT * FROM "stowage";

CREATE TABLE IF NOT EXISTS "voyages__ash_new" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "ship_id" TEXT NOT NULL,
  "captain_id" TEXT NOT NULL,
  "origin_port_id" TEXT NOT NULL,
  "destination_port_id" TEXT NOT NULL,
  "launch_window" TEXT NOT NULL,
  "distance_au" REAL NOT NULL,
  "transit_hours" INTEGER NOT NULL,
  "reservation_id" TEXT,
  "departed_at" TEXT,
  "arrived_at" TEXT,
  "version" INTEGER NOT NULL,
  "status" TEXT NOT NULL,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_voyages_distinct_ports" CHECK (origin_port_id <> destination_port_id),
  CONSTRAINT "ck_voyages_arrives_after_departure" CHECK (arrived_at IS NULL OR departed_at IS NULL OR arrived_at > departed_at),
  CONSTRAINT "ck_voyages_status_one_of" CHECK ("status" IN ('planned', 'cleared', 'in_transit', 'arrived', 'completed', 'scrubbed')),
  CONSTRAINT "fk_voyages_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_voyages_origin" FOREIGN KEY ("origin_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_voyages_destination" FOREIGN KEY ("destination_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

INSERT INTO "voyages__ash_new" ("id", "line", "ship_id", "captain_id", "origin_port_id", "destination_port_id", "launch_window", "distance_au", "transit_hours", "reservation_id", "departed_at", "arrived_at", "version", "status", "created_at", "updated_at")
SELECT "id", "line", "ship_id", "captain_id", "origin_port_id", "destination_port_id", "launch_window", "distance_au", "transit_hours", "reservation_id", "departed_at", "arrived_at", "version", "status", "created_at", "updated_at" FROM "voyages";

DROP TABLE "voyages";

ALTER TABLE "voyages__ash_new" RENAME TO "voyages";

CREATE INDEX IF NOT EXISTS "idx_voyages_in_flight" ON "voyages" ("line", "ship_id") WHERE status = 'in_transit';

CREATE INDEX IF NOT EXISTS "idx_voyages_by_window" ON "voyages" ("line", "launch_window");

DELETE FROM "stowage";

INSERT INTO "stowage" ("id", "line", "voyage_id", "container_id", "bay", "stack", "tier") SELECT "id", "line", "voyage_id", "container_id", "bay", "stack", "tier" FROM "stowage__ash_hold";

DROP TABLE "stowage__ash_hold";

CREATE TABLE "containers__ash_hold" AS SELECT * FROM "containers";

CREATE TABLE IF NOT EXISTS "contracts__ash_new" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "external_ref" TEXT NOT NULL,
  "shipper_email" TEXT COLLATE NOCASE NOT NULL,
  "origin_port_id" TEXT NOT NULL,
  "destination_port_id" TEXT NOT NULL,
  "freight_credits" INTEGER NOT NULL,
  "insured_value" NUMERIC NOT NULL,
  "deliver_by" TEXT NOT NULL,
  "version" INTEGER NOT NULL,
  "status" TEXT NOT NULL,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_contracts_positive_freight" CHECK (freight_credits > 0),
  CONSTRAINT "ck_contracts_distinct_ports" CHECK (origin_port_id <> destination_port_id),
  CONSTRAINT "ck_contracts_status_one_of" CHECK ("status" IN ('booked', 'loaded', 'in_transit', 'delivered', 'closed', 'cancelled')),
  CONSTRAINT "fk_contracts_origin" FOREIGN KEY ("origin_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_contracts_destination" FOREIGN KEY ("destination_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

INSERT INTO "contracts__ash_new" ("id", "line", "external_ref", "shipper_email", "origin_port_id", "destination_port_id", "freight_credits", "insured_value", "deliver_by", "version", "status", "created_at", "updated_at")
SELECT "id", "line", "external_ref", "shipper_email", "origin_port_id", "destination_port_id", "freight_credits", "insured_value", "deliver_by", "version", "status", "created_at", "updated_at" FROM "contracts";

DROP TABLE "contracts";

ALTER TABLE "contracts__ash_new" RENAME TO "contracts";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_contracts_external" ON "contracts" ("line", "external_ref");

CREATE INDEX IF NOT EXISTS "idx_contracts_open_by_destination" ON "contracts" ("line", "destination_port_id") WHERE status <> 'closed' AND status <> 'cancelled';

DELETE FROM "containers";

INSERT INTO "containers" ("id", "line", "code", "contract_id", "mass_tonnes", "hazmat", "hazardous", "reefer", "manifest", "seal", "sealed", "version", "archived_at", "created_at", "updated_at") SELECT "id", "line", "code", "contract_id", "mass_tonnes", "hazmat", "hazardous", "reefer", "manifest", "seal", "sealed", "version", "archived_at", "created_at", "updated_at" FROM "containers__ash_hold";

DROP TABLE "containers__ash_hold";

CREATE TABLE "berth_reservations__ash_hold" AS SELECT * FROM "berth_reservations";

CREATE TABLE "transponders__ash_hold" AS SELECT * FROM "transponders";

CREATE TABLE "voyages__ash_hold" AS SELECT * FROM "voyages";

CREATE TABLE IF NOT EXISTS "ships__ash_new" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "registry" TEXT COLLATE NOCASE NOT NULL,
  "name" TEXT NOT NULL,
  "class" TEXT NOT NULL,
  "dry_mass_tonnes" INTEGER NOT NULL,
  "slot_capacity" INTEGER NOT NULL,
  "captain_id" TEXT,
  "version" INTEGER NOT NULL,
  "archived_at" TEXT,
  "status" TEXT NOT NULL,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_ships_sane_hull" CHECK (dry_mass_tonnes > 0 AND slot_capacity > 0),
  CONSTRAINT "ck_ships_class_one_of" CHECK ("class" IN ('shuttle', 'hauler', 'tanker', 'freighter')),
  CONSTRAINT "ck_ships_status_one_of" CHECK ("status" IN ('docked', 'in_transit', 'maintenance')),
  CONSTRAINT "fk_ships_captain" FOREIGN KEY ("captain_id") REFERENCES "crew_members" ("id") ON DELETE SET NULL
);

INSERT INTO "ships__ash_new" ("id", "line", "registry", "name", "class", "dry_mass_tonnes", "slot_capacity", "captain_id", "version", "archived_at", "status", "created_at", "updated_at")
SELECT "id", "line", "registry", "name", "class", "dry_mass_tonnes", "slot_capacity", "captain_id", "version", "archived_at", "status", "created_at", "updated_at" FROM "ships";

DROP TABLE "ships";

ALTER TABLE "ships__ash_new" RENAME TO "ships";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ships_live_registry" ON "ships" ("line", "registry") WHERE archived_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_ships_by_captain" ON "ships" ("line", "captain_id") WHERE captain_id IS NOT NULL;

DELETE FROM "berth_reservations";

INSERT INTO "berth_reservations" ("id", "line", "port_id", "berth_code", "ship_id", "starts_at", "ends_at", "status", "created_at", "updated_at") SELECT "id", "line", "port_id", "berth_code", "ship_id", "starts_at", "ends_at", "status", "created_at", "updated_at" FROM "berth_reservations__ash_hold";

DROP TABLE "berth_reservations__ash_hold";

DELETE FROM "transponders";

INSERT INTO "transponders" ("id", "line", "ship_id", "label", "kind", "api_key_hash", "created_at", "updated_at") SELECT "id", "line", "ship_id", "label", "kind", "api_key_hash", "created_at", "updated_at" FROM "transponders__ash_hold";

DROP TABLE "transponders__ash_hold";

DELETE FROM "voyages";

INSERT INTO "voyages" ("id", "line", "ship_id", "captain_id", "origin_port_id", "destination_port_id", "launch_window", "distance_au", "transit_hours", "reservation_id", "departed_at", "arrived_at", "version", "status", "created_at", "updated_at") SELECT "id", "line", "ship_id", "captain_id", "origin_port_id", "destination_port_id", "launch_window", "distance_au", "transit_hours", "reservation_id", "departed_at", "arrived_at", "version", "status", "created_at", "updated_at" FROM "voyages__ash_hold";

DROP TABLE "voyages__ash_hold";
