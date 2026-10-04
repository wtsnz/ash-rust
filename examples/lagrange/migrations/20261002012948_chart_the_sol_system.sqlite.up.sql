-- Migration: 20261002012948_chart_the_sol_system.sqlite.up.sql
-- Generated automatically by ash-rust.

CREATE TABLE IF NOT EXISTS "planets" (
  "id" TEXT PRIMARY KEY,
  "name" TEXT COLLATE NOCASE NOT NULL,
  "surface_gravity" REAL NOT NULL,
  "position" TEXT NOT NULL,
  "atmosphere" TEXT NOT NULL,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_planets_atmosphere_one_of" CHECK ("atmosphere" IN ('vacuum', 'thin', 'breathable', 'toxic'))
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_planets_unique_name" ON "planets" ("name");

CREATE TABLE IF NOT EXISTS "ports" (
  "id" TEXT PRIMARY KEY,
  "planet_id" TEXT NOT NULL,
  "code" TEXT COLLATE NOCASE NOT NULL,
  "name" TEXT NOT NULL,
  "kind" TEXT NOT NULL,
  "relay" TEXT NOT NULL,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_ports_kind_one_of" CHECK ("kind" IN ('ground', 'low_orbit', 'geostationary', 'lagrange')),
  CONSTRAINT "fk_ports_planet" FOREIGN KEY ("planet_id") REFERENCES "planets" ("id") ON DELETE RESTRICT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ports_unique_code" ON "ports" ("code");

CREATE INDEX IF NOT EXISTS "idx_ports_by_planet" ON "ports" ("planet_id");

CREATE INDEX IF NOT EXISTS "idx_ports_by_kind" ON "ports" ("kind");

CREATE TABLE IF NOT EXISTS "docks" (
  "id" TEXT PRIMARY KEY,
  "port_id" TEXT NOT NULL,
  "code" TEXT NOT NULL,
  "clamp" TEXT NOT NULL,
  "max_mass_tonnes" INTEGER NOT NULL,
  "version" INTEGER NOT NULL,
  CONSTRAINT "ck_docks_positive_mass" CHECK (max_mass_tonnes > 0),
  CONSTRAINT "ck_docks_clamp_one_of" CHECK ("clamp" IN ('standard', 'heavy_lift', 'cryogenic')),
  CONSTRAINT "fk_docks_port" FOREIGN KEY ("port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_docks_port_code" ON "docks" ("port_id", "code");

CREATE TABLE IF NOT EXISTS "shipping_lines" (
  "id" TEXT PRIMARY KEY,
  "slug" TEXT COLLATE NOCASE NOT NULL,
  "name" TEXT NOT NULL,
  "dispatch_email" TEXT COLLATE NOCASE NOT NULL,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_shipping_lines_unique_slug" ON "shipping_lines" ("slug");

CREATE TABLE IF NOT EXISTS "crew_members" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "email" TEXT COLLATE NOCASE NOT NULL,
  "name" TEXT NOT NULL,
  "role" TEXT NOT NULL,
  "hashed_password" TEXT,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "ck_crew_members_role_one_of" CHECK ("role" IN ('dispatcher', 'captain', 'customs', 'shipper', 'port_authority'))
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_crew_members_unique_email" ON "crew_members" ("email");

CREATE TABLE IF NOT EXISTS "ships" (
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
  CONSTRAINT "fk_ships_captain" FOREIGN KEY ("captain_id") REFERENCES "crew_members" ("id") ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ships_live_registry" ON "ships" ("registry") WHERE archived_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_ships_by_captain" ON "ships" ("captain_id") WHERE captain_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS "berth_reservations" (
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

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_active_by_berth" ON "berth_reservations" ("port_id", "dock_code") WHERE status = 'active';

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_by_ship" ON "berth_reservations" ("ship_id");

CREATE TABLE IF NOT EXISTS "transponders" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "ship_id" TEXT NOT NULL,
  "label" TEXT NOT NULL,
  "kind" TEXT NOT NULL DEFAULT 'transponder',
  "api_key_hash" TEXT,
  "created_at" TEXT NOT NULL,
  "updated_at" TEXT NOT NULL,
  CONSTRAINT "fk_transponders_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_transponders_one_per_ship" ON "transponders" ("ship_id");

CREATE TABLE IF NOT EXISTS "contracts" (
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
  CONSTRAINT "fk_contracts_origin" FOREIGN KEY ("origin_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_contracts_destination" FOREIGN KEY ("destination_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_contracts_external" ON "contracts" ("line", "external_ref");

CREATE INDEX IF NOT EXISTS "idx_contracts_open_by_destination" ON "contracts" ("destination_port_id") WHERE status <> 'closed' AND status <> 'cancelled';

CREATE TABLE IF NOT EXISTS "containers" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "code" TEXT COLLATE NOCASE NOT NULL,
  "contract_id" TEXT NOT NULL,
  "mass_tonnes" INTEGER NOT NULL,
  "hazmat" TEXT,
  "hazardous" INTEGER NOT NULL DEFAULT 0,
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

CREATE UNIQUE INDEX IF NOT EXISTS "idx_containers_live_code" ON "containers" ("code") WHERE archived_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_containers_by_contract" ON "containers" ("contract_id");

CREATE INDEX IF NOT EXISTS "idx_containers_hazardous_by_contract" ON "containers" ("contract_id") WHERE hazardous;

CREATE TABLE IF NOT EXISTS "voyages" (
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
  CONSTRAINT "fk_voyages_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_voyages_origin" FOREIGN KEY ("origin_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_voyages_destination" FOREIGN KEY ("destination_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

CREATE INDEX IF NOT EXISTS "idx_voyages_in_flight" ON "voyages" ("ship_id") WHERE status = 'in_transit';

CREATE INDEX IF NOT EXISTS "idx_voyages_by_window" ON "voyages" ("launch_window");

CREATE TABLE IF NOT EXISTS "stowage" (
  "id" TEXT PRIMARY KEY,
  "line" TEXT NOT NULL,
  "voyage_id" TEXT NOT NULL,
  "container_id" TEXT NOT NULL,
  "bay" INTEGER NOT NULL,
  "stack" INTEGER NOT NULL,
  "tier" INTEGER NOT NULL,
  CONSTRAINT "ck_stowage_slot_in_hold" CHECK (bay BETWEEN 1 AND 40 AND stack BETWEEN 1 AND 16 AND tier BETWEEN 1 AND 8),
  CONSTRAINT "fk_stowage_voyage" FOREIGN KEY ("voyage_id") REFERENCES "voyages" ("id") ON DELETE CASCADE,
  CONSTRAINT "fk_stowage_container" FOREIGN KEY ("container_id") REFERENCES "containers" ("id") ON DELETE RESTRICT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_slot" ON "stowage" ("voyage_id", "bay", "stack", "tier");

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_once_per_voyage" ON "stowage" ("voyage_id", "container_id");
