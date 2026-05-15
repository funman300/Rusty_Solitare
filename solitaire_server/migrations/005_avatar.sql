-- Migration 005: user avatar
-- Adds a nullable avatar_url column to users.
-- Existing rows receive NULL (no avatar set).
ALTER TABLE users ADD COLUMN avatar_url TEXT;
