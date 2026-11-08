-- V6__media_sources_and_asset_lineage.sql: Media sources registry, explicit asset lineage & container index
--
-- 1. Table des sources médias physiques et réseau
CREATE TABLE IF NOT EXISTS media_sources (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    root_uri TEXT NOT NULL,
    storage_kind VARCHAR(32) NOT NULL DEFAULT 'local',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_storage_kind CHECK (storage_kind IN ('local', 'cifs_mount', 'remote_stream'))
);

COMMENT ON TABLE media_sources IS 'Configured physical storage roots and network shares where media assets are discovered.';
COMMENT ON COLUMN media_sources.id IS 'Persistent machine identifier (e.g. synology_tvshow, local_snippets).';
COMMENT ON COLUMN media_sources.root_uri IS 'Filesystem directory root path or network URI.';
COMMENT ON COLUMN media_sources.storage_kind IS 'Underlying storage protocol (local, cifs_mount, remote_stream).';

-- 2. Évolution de la table medias pour intégrer la lignée, le soft-delete et le statut de complétude
ALTER TABLE medias
    ADD COLUMN IF NOT EXISTS source_id VARCHAR(64) REFERENCES media_sources(id),
    ADD COLUMN IF NOT EXISTS asset_kind VARCHAR(32) NOT NULL DEFAULT 'master',
    ADD COLUMN IF NOT EXISTS parent_media_id INTEGER REFERENCES medias(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS sample_start_ms BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS sample_duration_ms BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS expected_frames INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS is_vfr BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS phash_status VARCHAR(32) NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS source_mtime_unix BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- 3. Contraintes de domaine et d'intégrité de lignée
ALTER TABLE medias
    DROP CONSTRAINT IF EXISTS chk_asset_kind,
    ADD CONSTRAINT chk_asset_kind CHECK (asset_kind IN ('master', 'cut', 'synthetic_query'));

ALTER TABLE medias
    DROP CONSTRAINT IF EXISTS chk_phash_status,
    ADD CONSTRAINT chk_phash_status CHECK (phash_status IN ('pending', 'in_progress', 'complete', 'truncated', 'corrupted', 'obsolete'));

ALTER TABLE medias
    DROP CONSTRAINT IF EXISTS chk_asset_lineage,
    ADD CONSTRAINT chk_asset_lineage CHECK (
        (asset_kind = 'master'
            AND parent_media_id IS NULL
            AND sample_start_ms = 0
            AND sample_duration_ms = 0)
        OR (asset_kind IN ('cut', 'synthetic_query')
            AND parent_media_id IS NOT NULL
            AND sample_duration_ms > 0)
    );

-- 4. Index unique partiel sur l'identité conteneur pour le scan O(1)
CREATE UNIQUE INDEX IF NOT EXISTS idx_medias_container_identity
    ON medias (moviehash, file_size_bytes)
    WHERE asset_kind = 'master' AND deleted_at IS NULL AND moviehash IS NOT NULL;

-- 5. Indexation pour la navigation, la lignée et les filtres d'ingestion
CREATE INDEX IF NOT EXISTS idx_medias_asset_kind ON medias (asset_kind);
CREATE INDEX IF NOT EXISTS idx_medias_parent_id ON medias (parent_media_id) WHERE parent_media_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_medias_phash_status ON medias (phash_status);
CREATE INDEX IF NOT EXISTS idx_medias_source_id ON medias (source_id);
CREATE INDEX IF NOT EXISTS idx_medias_deleted_at ON medias (deleted_at) WHERE deleted_at IS NOT NULL;
