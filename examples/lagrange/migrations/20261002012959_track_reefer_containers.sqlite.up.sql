-- Migration: 20261002012959_track_reefer_containers.sqlite.up.sql
-- Generated automatically by ash-rust.

ALTER TABLE "containers" ADD COLUMN "reefer" INTEGER NOT NULL DEFAULT 0;
