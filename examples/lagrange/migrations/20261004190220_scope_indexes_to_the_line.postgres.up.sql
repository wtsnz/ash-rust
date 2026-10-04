-- Migration: 20261004190220_scope_indexes_to_the_line.postgres.up.sql
-- Generated automatically by ash-rust.

DROP INDEX IF EXISTS "idx_crew_members_unique_email";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_crew_members_unique_email" ON "crew_members" ("line", "email");

ALTER TABLE "ships" ALTER COLUMN "status" TYPE VARCHAR(255) USING "status"::VARCHAR(255);

DROP INDEX IF EXISTS "idx_ships_live_registry";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ships_live_registry" ON "ships" ("line", "registry") WHERE archived_at IS NULL;

DROP INDEX IF EXISTS "idx_ships_by_captain";

CREATE INDEX IF NOT EXISTS "idx_ships_by_captain" ON "ships" ("line", "captain_id") WHERE captain_id IS NOT NULL;

ALTER TABLE "ships" ADD CONSTRAINT "ck_ships_status_one_of" CHECK ("status" IN ('docked', 'in_transit', 'maintenance'));

DROP INDEX IF EXISTS "idx_berth_reservations_active_by_berth";

DROP INDEX IF EXISTS "idx_berth_reservations_by_ship";

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_active_by_berth" ON "berth_reservations" ("line", "port_id", "berth_code") INCLUDE ("starts_at", "ends_at") WHERE status = 'active';

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_by_ship" ON "berth_reservations" ("line", "ship_id");

DROP INDEX IF EXISTS "idx_transponders_one_per_ship";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_transponders_one_per_ship" ON "transponders" ("line", "ship_id");

ALTER TABLE "contracts" ALTER COLUMN "status" TYPE VARCHAR(255) USING "status"::VARCHAR(255);

DROP INDEX IF EXISTS "idx_contracts_open_by_destination";

CREATE INDEX IF NOT EXISTS "idx_contracts_open_by_destination" ON "contracts" ("line", "destination_port_id") WHERE status <> 'closed' AND status <> 'cancelled';

ALTER TABLE "contracts" ADD CONSTRAINT "ck_contracts_status_one_of" CHECK ("status" IN ('booked', 'loaded', 'in_transit', 'delivered', 'closed', 'cancelled'));

DROP INDEX IF EXISTS "idx_containers_live_code";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_containers_live_code" ON "containers" ("line", "code") WHERE archived_at IS NULL;

DROP INDEX IF EXISTS "idx_containers_by_contract";

DROP INDEX IF EXISTS "idx_containers_hazardous_by_contract";

CREATE INDEX IF NOT EXISTS "idx_containers_by_contract" ON "containers" ("line", "contract_id");

CREATE INDEX IF NOT EXISTS "idx_containers_hazardous_by_contract" ON "containers" ("line", "contract_id") WHERE hazardous;

ALTER TABLE "voyages" ALTER COLUMN "status" TYPE VARCHAR(255) USING "status"::VARCHAR(255);

DROP INDEX IF EXISTS "idx_voyages_in_flight";

DROP INDEX IF EXISTS "idx_voyages_by_window";

CREATE INDEX IF NOT EXISTS "idx_voyages_in_flight" ON "voyages" ("line", "ship_id") WHERE status = 'in_transit';

CREATE INDEX IF NOT EXISTS "idx_voyages_by_window" ON "voyages" USING brin ("line", "launch_window");

ALTER TABLE "voyages" ADD CONSTRAINT "ck_voyages_status_one_of" CHECK ("status" IN ('planned', 'cleared', 'in_transit', 'arrived', 'completed', 'scrubbed'));

DROP INDEX IF EXISTS "idx_stowage_slot";

DROP INDEX IF EXISTS "idx_stowage_once_per_voyage";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_slot" ON "stowage" ("line", "voyage_id", "bay", "stack", "tier");

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_once_per_voyage" ON "stowage" ("line", "voyage_id", "container_id");
