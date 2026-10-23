-- V2__certified_couplings.sql: Certified subtitles and cue frame anchors

CREATE TABLE IF NOT EXISTS subtitles (
    id SERIAL PRIMARY KEY,
    subtitles_hash CHAR(32) NOT NULL,
    name TEXT NOT NULL,
    lang TEXT DEFAULT 'FR',
    media_id INT NOT NULL REFERENCES medias(id) ON DELETE CASCADE,
    srt_path TEXT,
    certified BOOLEAN DEFAULT TRUE,
    UNIQUE(media_id, subtitles_hash, lang)
);

CREATE TABLE IF NOT EXISTS sub_frames (
    id SERIAL PRIMARY KEY,
    index INT NOT NULL,
    start_frame_offset INT NOT NULL,
    end_frame_offset INT NOT NULL,
    text TEXT DEFAULT '',
    subtitles_id INT NOT NULL REFERENCES subtitles(id) ON DELETE CASCADE,
    CHECK (end_frame_offset >= start_frame_offset)
);
