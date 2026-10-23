-- V1__initial_schema.sql: Initial medias & frames BK-tree relational schema

CREATE TABLE IF NOT EXISTS medias (
    id SERIAL PRIMARY KEY,
    media_hash CHAR(32) UNIQUE,
    name TEXT UNIQUE,
    fps DOUBLE PRECISION DEFAULT 25.0,
    frame_count INT DEFAULT 0,
    phash_path TEXT
);

CREATE TABLE IF NOT EXISTS frames (
    id SERIAL PRIMARY KEY,
    p_hash BIGINT,
    frame_offset INT,
    media_id INT REFERENCES medias(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS frames_phash_bk_tree_index
ON frames USING spgist ( p_hash bktree_ops );
