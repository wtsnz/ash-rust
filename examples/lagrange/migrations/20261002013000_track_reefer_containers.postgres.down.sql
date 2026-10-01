-- Migration: 20261002013000_track_reefer_containers.postgres.down.sql
-- Generated automatically by ash-rust.

ALTER TABLE "containers" DROP COLUMN "reefer";
