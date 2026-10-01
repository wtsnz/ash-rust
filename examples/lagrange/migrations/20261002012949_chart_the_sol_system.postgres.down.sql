-- Migration: 20261002012949_chart_the_sol_system.postgres.down.sql
-- Generated automatically by ash-rust.

DROP TABLE IF EXISTS "stowage";

DROP TABLE IF EXISTS "voyages";

DROP TABLE IF EXISTS "containers";

DROP TABLE IF EXISTS "contracts";

DROP TABLE IF EXISTS "transponders";

ALTER TABLE berth_reservations DROP CONSTRAINT IF EXISTS berth_reservations_no_overlap;

DROP TABLE IF EXISTS "berth_reservations";

DROP TABLE IF EXISTS "ships";

DROP TABLE IF EXISTS "crew_members";

DROP TABLE IF EXISTS "shipping_lines";

DROP TABLE IF EXISTS "docks";

DROP TABLE IF EXISTS "ports";

DROP TABLE IF EXISTS "planets";
