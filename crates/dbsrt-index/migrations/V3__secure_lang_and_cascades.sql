-- V3__secure_lang_and_cascades.sql: Convert lang column to TEXT and secure constraints

-- 1. Ensure subtitles.lang is converted to TEXT (from any legacy enum)
ALTER TABLE subtitles ALTER COLUMN lang TYPE TEXT;

-- 2. Clean up legacy single-column unique constraints that broke multi-release couplings
ALTER TABLE subtitles DROP CONSTRAINT IF EXISTS subtitles_subtitles_hash_key;
ALTER TABLE subtitles DROP CONSTRAINT IF EXISTS subtitles_subtitles_hash_name_lang_key;

-- 3. Ensure composite uniqueness constraint exists idempotently
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'subtitles_media_id_subtitles_hash_lang_key'
    ) THEN
        ALTER TABLE subtitles ADD CONSTRAINT subtitles_media_id_subtitles_hash_lang_key
            UNIQUE(media_id, subtitles_hash, lang);
    END IF;
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;
