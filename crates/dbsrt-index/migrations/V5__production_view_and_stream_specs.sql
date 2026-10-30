-- V5__production_view_and_stream_specs.sql: Physical stream geometry & Conceptual Production View

-- 1. Physical stream hardware container specs (Source of truth extracted by ffprobe)
ALTER TABLE medias
    ADD COLUMN IF NOT EXISTS width INTEGER,
    ADD COLUMN IF NOT EXISTS height INTEGER,
    ADD COLUMN IF NOT EXISTS video_codec VARCHAR(32);

-- 2. Domain & SoC comments on physical media table and columns
COMMENT ON TABLE medias IS 'Physical media release instances (tangible video container, pHash visual digest, container geometry and video specs).';
COMMENT ON COLUMN medias.width IS 'Horizontal video frame resolution in pixels (probe-extracted ground-truth).';
COMMENT ON COLUMN medias.height IS 'Vertical video frame resolution in pixels (probe-extracted ground-truth).';
COMMENT ON COLUMN medias.video_codec IS 'Video stream container codec (e.g. h264, hevc, vp9, av1).';
COMMENT ON COLUMN medias.clean_title IS 'Production editorial title: canonical title of the creative work, independent of release tags.';
COMMENT ON COLUMN medias.media_type IS 'Production domain classification (movie, tv_episode, anime, other).';

-- 3. Conceptual Production View (Separation of Concerns: Creative Work vs Physical Media Release)
-- Concept: A "Production" is an intangible audiovisual creative work (movie, TV episode, documentary)
-- defined by its editorial identity (clean_title, media_type, year, season, episode).
-- Multiple physical media releases (e.g. 720p WEB-DL, 1080p BluRay Remux, 4K HEVC) can represent
-- the same underlying Production.
CREATE OR REPLACE VIEW v_productions AS
SELECT
    clean_title,
    media_type,
    year,
    season,
    episode,
    COUNT(*)::INTEGER AS release_count,
    ARRAY_AGG(id ORDER BY id) AS media_ids,
    ARRAY_AGG(media_hash ORDER BY id) AS media_hashes,
    MAX(duration_ms) AS max_duration_ms,
    MIN(created_at) AS first_ingested_at,
    MAX(updated_at) AS last_updated_at
FROM medias
WHERE clean_title IS NOT NULL
GROUP BY
    clean_title,
    media_type,
    year,
    season,
    episode;

COMMENT ON VIEW v_productions IS 'Conceptual audiovisual work (Production/Oeuvre). Aggregates physical media releases (containers, resolutions, codecs) under a single canonical creative entity defined by clean_title, media_type, year, season, and episode.';
