-- Migration: 20261004190220_scope_indexes_to_the_line.postgres.down.sql
-- Generated automatically by ash-rust.

DROP INDEX IF EXISTS "idx_stowage_slot";

DROP INDEX IF EXISTS "idx_stowage_once_per_voyage";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_slot" ON "stowage" ("voyage_id", "bay", "stack", "tier");

CREATE UNIQUE INDEX IF NOT EXISTS "idx_stowage_once_per_voyage" ON "stowage" ("voyage_id", "container_id");

DROP INDEX IF EXISTS "idx_crew_members_unique_email";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_crew_members_unique_email" ON "crew_members" ("email");

DROP INDEX IF EXISTS "idx_containers_live_code";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_containers_live_code" ON "containers" ("code") WHERE archived_at IS NULL;

DROP INDEX IF EXISTS "idx_containers_by_contract";

DROP INDEX IF EXISTS "idx_containers_hazardous_by_contract";

CREATE INDEX IF NOT EXISTS "idx_containers_by_contract" ON "containers" ("contract_id");

CREATE INDEX IF NOT EXISTS "idx_containers_hazardous_by_contract" ON "containers" ("contract_id") WHERE hazardous;

ALTER TABLE "voyages" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;

DROP INDEX IF EXISTS "idx_voyages_in_flight";

DROP INDEX IF EXISTS "idx_voyages_by_window";

CREATE INDEX IF NOT EXISTS "idx_voyages_in_flight" ON "voyages" ("ship_id") WHERE status = 'in_transit';

CREATE INDEX IF NOT EXISTS "idx_voyages_by_window" ON "voyages" USING brin ("launch_window");

ALTER TABLE "voyages" DROP CONSTRAINT IF EXISTS "ck_voyages_status_one_of";

ALTER TABLE "contracts" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;

DROP INDEX IF EXISTS "idx_contracts_open_by_destination";

CREATE INDEX IF NOT EXISTS "idx_contracts_open_by_destination" ON "contracts" ("destination_port_id") WHERE status <> 'closed' AND status <> 'cancelled';

ALTER TABLE "contracts" DROP CONSTRAINT IF EXISTS "ck_contracts_status_one_of";

ALTER TABLE "ships" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;

DROP INDEX IF EXISTS "idx_ships_live_registry";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_ships_live_registry" ON "ships" ("registry") WHERE archived_at IS NULL;

DROP INDEX IF EXISTS "idx_ships_by_captain";

CREATE INDEX IF NOT EXISTS "idx_ships_by_captain" ON "ships" ("captain_id") WHERE captain_id IS NOT NULL;

ALTER TABLE "ships" DROP CONSTRAINT IF EXISTS "ck_ships_status_one_of";

DROP INDEX IF EXISTS "idx_transponders_one_per_ship";

CREATE UNIQUE INDEX IF NOT EXISTS "idx_transponders_one_per_ship" ON "transponders" ("ship_id");

DROP INDEX IF EXISTS "idx_berth_reservations_active_by_berth";

DROP INDEX IF EXISTS "idx_berth_reservations_by_ship";

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_active_by_berth" ON "berth_reservations" ("port_id", "berth_code") INCLUDE ("starts_at", "ends_at") WHERE status = 'active';

CREATE INDEX IF NOT EXISTS "idx_berth_reservations_by_ship" ON "berth_reservations" ("ship_id");
