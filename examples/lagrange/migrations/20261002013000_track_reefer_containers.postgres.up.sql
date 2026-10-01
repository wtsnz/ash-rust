-- Migration: 20261002013000_track_reefer_containers.postgres.up.sql
-- Generated automatically by ash-rust.

ALTER TABLE "containers" ADD COLUMN "reefer" BOOLEAN NOT NULL DEFAULT FALSE;
