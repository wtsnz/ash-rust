-- Migration: 20261002012959_track_reefer_containers.sqlite.down.sql
-- Generated automatically by ash-rust.

ALTER TABLE "containers" DROP COLUMN "reefer";
