-- Migration: 20261002012949_chart_the_sol_system.postgres.up.sql
-- Generated automatically by ash-rust.

CREATE EXTENSION IF NOT EXISTS citext WITH SCHEMA public;

CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;

CREATE TABLE IF NOT EXISTS "planets" (
  "id" UUID PRIMARY KEY,
  "name" CITEXT NOT NULL,
  "surface_gravity" DOUBLE PRECISION NOT NULL,
  "position" VECTOR(3) NOT NULL,
  "atmosphere" VARCHAR(255) NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_planets_atmosphere_one_of" CHECK ("atmosphere" IN ('vacuum', 'thin', 'breathable', 'toxic'))
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_planets_unique_name" ON "planets" ("name");

CREATE TABLE IF NOT EXISTS "ports" (
  "id" UUID PRIMARY KEY,
  "planet_id" UUID NOT NULL,
  "code" CITEXT NOT NULL,
  "name" TEXT NOT NULL,
  "kind" VARCHAR(255) NOT NULL,
  "relay" INET NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_ports_kind_one_of" CHECK ("kind" IN ('ground', 'low_orbit', 'geostationary', 'lagrange')),
  CONSTRAINT "fk_ports_planet" FOREIGN KEY ("planet_id") REFERENCES "planets" ("id") ON DELETE RESTRICT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ports_unique_code" ON "ports" ("code");

CREATE INDEX IF NOT EXISTS "idx_ports_by_planet" ON "ports" ("planet_id") INCLUDE ("name");

CREATE INDEX IF NOT EXISTS "idx_ports_by_kind" ON "ports" USING hash ("kind");

CREATE TABLE IF NOT EXISTS "docks" (
  "id" UUID PRIMARY KEY,
  "port_id" UUID NOT NULL,
  "code" TEXT NOT NULL,
  "clamp" VARCHAR(255) NOT NULL,
  "max_mass_tonnes" BIGINT NOT NULL,
  "version" BIGINT NOT NULL,
  CONSTRAINT "ck_docks_positive_mass" CHECK (max_mass_tonnes > 0),
  CONSTRAINT "ck_docks_clamp_one_of" CHECK ("clamp" IN ('standard', 'heavy_lift', 'cryogenic')),
  CONSTRAINT "fk_docks_port" FOREIGN KEY ("port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_docks_port_code" ON "docks" ("port_id", "code");

CREATE TABLE IF NOT EXISTS "shipping_lines" (
  "id" UUID PRIMARY KEY,
  "slug" CITEXT NOT NULL,
  "name" TEXT NOT NULL,
  "dispatch_email" CITEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_shipping_lines_unique_slug" ON "shipping_lines" ("slug");

CREATE TABLE IF NOT EXISTS "crew_members" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "email" CITEXT NOT NULL,
  "name" TEXT NOT NULL,
  "role" VARCHAR(255) NOT NULL,
  "hashed_password" TEXT,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_crew_members_role_one_of" CHECK ("role" IN ('dispatcher', 'captain', 'customs', 'shipper', 'port_authority'))
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_crew_members_unique_email" ON "crew_members" ("email");

CREATE TABLE IF NOT EXISTS "ships" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "registry" CITEXT NOT NULL,
  "name" TEXT NOT NULL,
  "class" VARCHAR(255) NOT NULL,
  "dry_mass_tonnes" BIGINT NOT NULL,
  "slot_capacity" BIGINT NOT NULL,
  "captain_id" UUID,
  "version" BIGINT NOT NULL,
  "archived_at" TIMESTAMPTZ,
  "status" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_ships_sane_hull" CHECK (dry_mass_tonnes > 0 AND slot_capacity > 0),
  CONSTRAINT "ck_ships_class_one_of" CHECK ("class" IN ('shuttle', 'hauler', 'tanker', 'freighter')),
  CONSTRAINT "fk_ships_captain" FOREIGN KEY ("captain_id") REFERENCES "crew_members" ("id") ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ships_live_registry" ON "ships" ("registry") WHERE archived_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_ships_by_captain" ON "ships" ("captain_id") WHERE captain_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS "berth_reservations" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "port_id" UUID NOT NULL,
  "dock_code" TEXT NOT NULL,
  "ship_id" UUID NOT NULL,
  "starts_at" TIMESTAMPTZ NOT NULL,
  "ends_at" TIMESTAMPTZ NOT NULL,
  "status" VARCHAR(255) NOT NULL DEFAULT 'active',
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_berth_reservations_window" CHECK (ends_at > starts_at),
  CONSTRAINT "ck_berth_reservations_status_one_of" CHECK ("status" IN ('active', 'released')),
  CONSTRAINT "fk_berth_reservations_berth" FOREIGN KEY ("port_id", "dock_code") REFERENCES "docks" ("port_id", "code") ON DELETE RESTRICT ON UPDATE CASCADE,
  CONSTRAINT "fk_berth_reservations_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE NO ACTION
);

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_active_by_berth" ON "berth_reservations" ("port_id", "dock_code") INCLUDE ("starts_at", "ends_at") WHERE status = 'active';

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_by_ship" ON "berth_reservations" ("ship_id");

CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA public;

ALTER TABLE berth_reservations ADD CONSTRAINT berth_reservations_no_overlap EXCLUDE USING gist (port_id WITH =, dock_code WITH =, tstzrange(starts_at, ends_at) WITH &&) WHERE (status = 'active');

CREATE TABLE IF NOT EXISTS "transponders" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "ship_id" UUID NOT NULL,
  "label" TEXT NOT NULL,
  "kind" TEXT NOT NULL DEFAULT 'transponder',
  "api_key_hash" TEXT,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "fk_transponders_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_transponders_one_per_ship" ON "transponders" ("ship_id");

CREATE TABLE IF NOT EXISTS "contracts" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "external_ref" TEXT NOT NULL,
  "shipper_email" CITEXT NOT NULL,
  "origin_port_id" UUID NOT NULL,
  "destination_port_id" UUID NOT NULL,
  "freight_credits" BIGINT NOT NULL,
  "insured_value" NUMERIC NOT NULL,
  "deliver_by" DATE NOT NULL,
  "version" BIGINT NOT NULL,
  "status" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_contracts_positive_freight" CHECK (freight_credits > 0),
  CONSTRAINT "ck_contracts_distinct_ports" CHECK (origin_port_id <> destination_port_id),
  CONSTRAINT "fk_contracts_origin" FOREIGN KEY ("origin_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_contracts_destination" FOREIGN KEY ("destination_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_contracts_external" ON "contracts" ("line", "external_ref");

CREATE INDEX IF NOT EXISTS "idx_contracts_open_by_destination" ON "contracts" ("destination_port_id") WHERE status <> 'closed' AND status <> 'cancelled';

CREATE TABLE IF NOT EXISTS "containers" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "code" CITEXT NOT NULL,
  "contract_id" UUID NOT NULL,
  "mass_tonnes" BIGINT NOT NULL,
  "hazmat" VARCHAR(255),
  "hazardous" BOOLEAN NOT NULL DEFAULT FALSE,
  "manifest" JSONB,
  "seal" BYTEA,
  "sealed" BOOLEAN NOT NULL DEFAULT FALSE,
  "version" BIGINT NOT NULL,
  "archived_at" TIMESTAMPTZ,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_containers_positive_mass" CHECK (mass_tonnes > 0),
  CONSTRAINT "ck_containers_sealed_has_seal" CHECK (sealed = (seal IS NOT NULL)),
  CONSTRAINT "ck_containers_hazmat_one_of" CHECK ("hazmat" IN ('flammable', 'corrosive', 'radioactive', 'cryogenic')),
  CONSTRAINT "fk_containers_contract" FOREIGN KEY ("contract_id") REFERENCES "contracts" ("id") ON DELETE RESTRICT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_containers_live_code" ON "containers" ("code") WHERE archived_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_containers_by_contract" ON "containers" ("contract_id");

CREATE INDEX IF NOT EXISTS "idx_containers_hazardous_by_contract" ON "containers" ("contract_id") WHERE hazardous;

CREATE TABLE IF NOT EXISTS "voyages" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "ship_id" UUID NOT NULL,
  "captain_id" UUID NOT NULL,
  "origin_port_id" UUID NOT NULL,
  "destination_port_id" UUID NOT NULL,
  "launch_window" DATE NOT NULL,
  "distance_au" DOUBLE PRECISION NOT NULL,
  "transit_hours" BIGINT NOT NULL,
  "reservation_id" UUID,
  "departed_at" TIMESTAMPTZ,
  "arrived_at" TIMESTAMPTZ,
  "version" BIGINT NOT NULL,
  "status" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL,
  "updated_at" TIMESTAMPTZ NOT NULL,
  CONSTRAINT "ck_voyages_distinct_ports" CHECK (origin_port_id <> destination_port_id),
  CONSTRAINT "ck_voyages_arrives_after_departure" CHECK (arrived_at IS NULL OR departed_at IS NULL OR arrived_at > departed_at),
  CONSTRAINT "fk_voyages_ship" FOREIGN KEY ("ship_id") REFERENCES "ships" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_voyages_origin" FOREIGN KEY ("origin_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION,
  CONSTRAINT "fk_voyages_destination" FOREIGN KEY ("destination_port_id") REFERENCES "ports" ("id") ON DELETE NO ACTION
);

CREATE INDEX IF NOT EXISTS "idx_voyages_in_flight" ON "voyages" ("ship_id") WHERE status = 'in_transit';

CREATE INDEX IF NOT EXISTS "idx_voyages_by_window" ON "voyages" USING brin ("launch_window");

CREATE TABLE IF NOT EXISTS "stowage" (
  "id" UUID PRIMARY KEY,
  "line" TEXT NOT NULL,
  "voyage_id" UUID NOT NULL,
  "container_id" UUID NOT NULL,
  "bay" BIGINT NOT NULL,
  "stack" BIGINT NOT NULL,
  "tier" BIGINT NOT NULL,
  CONSTRAINT "ck_stowage_slot_in_hold" CHECK (bay BETWEEN 1 AND 40 AND stack BETWEEN 1 AND 16 AND tier BETWEEN 1 AND 8),
  CONSTRAINT "fk_stowage_voyage" FOREIGN KEY ("voyage_id") REFERENCES "voyages" ("id") ON DELETE CASCADE,
  CONSTRAINT "fk_stowage_container" FOREIGN KEY ("container_id") REFERENCES "containers" ("id") ON DELETE RESTRICT
);

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_slot" ON "stowage" ("voyage_id", "bay", "stack", "tier");

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_once_per_voyage" ON "stowage" ("voyage_id", "container_id");
