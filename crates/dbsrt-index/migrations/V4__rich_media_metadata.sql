-- V4__rich_media_metadata.sql: Enrich medias entity with parsed metadata, technical specs & timestamps

ALTER TABLE medias
    ADD COLUMN IF NOT EXISTS clean_title TEXT,
    ADD COLUMN IF NOT EXISTS media_type TEXT DEFAULT 'other',
    ADD COLUMN IF NOT EXISTS year SMALLINT,
    ADD COLUMN IF NOT EXISTS season INTEGER,
    ADD COLUMN IF NOT EXISTS episode INTEGER,
    ADD COLUMN IF NOT EXISTS resolution TEXT,
    ADD COLUMN IF NOT EXISTS source TEXT,
    ADD COLUMN IF NOT EXISTS release_group TEXT,
    ADD COLUMN IF NOT EXISTS duration_ms BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS file_size_bytes BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS moviehash CHAR(16),
    ADD COLUMN IF NOT EXISTS video_rel_path TEXT,
    ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Explicit semantic comment on the historical media_hash column
COMMENT ON COLUMN medias.media_hash IS 'SHA-256 (32 hex) digest of the video pHash stream (compute_media_phash_digest). Serves as visual fingerprint ID.';
COMMENT ON COLUMN medias.name IS 'Raw source media release filename including extension.';
COMMENT ON COLUMN medias.clean_title IS 'Canonical media title stripped of release group, quality and codec tags.';
COMMENT ON COLUMN medias.duration_ms IS 'Total duration in milliseconds computed as (frame_count / fps * 1000).';
COMMENT ON COLUMN medias.phash_path IS 'Relative path or filename of the raw binary .phash fingerprint on disk/NAS.';
COMMENT ON COLUMN medias.video_rel_path IS 'Relative storage path of the source video on NAS volume (e.g. Films/... or Series/...).';

-- Add domain constraint on media_type
ALTER TABLE medias
    ADD CONSTRAINT chk_media_type
    CHECK (media_type IN ('movie', 'tv_episode', 'anime', 'other'));

-- Indexes for frequent queries and filtering
CREATE INDEX IF NOT EXISTS idx_medias_clean_title ON medias (clean_title);
CREATE INDEX IF NOT EXISTS idx_medias_series_ep ON medias (clean_title, season, episode) WHERE season IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_medias_moviehash ON medias (moviehash) WHERE moviehash IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_medias_media_type ON medias (media_type);
