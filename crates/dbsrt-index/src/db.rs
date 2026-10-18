//! PostgreSQL 17 BK-tree & Relational Certified Couplings client interface.
//!
//! Manages the 4 relational tables:
//! 1. `medias` (`id`, `media_hash`, `name`, `fps`, `frame_count`, `phash_path`)
//! 2. `frames` (`id`, `p_hash`, `frame_offset`, `media_id`) with SP-GiST `bktree_ops` index
//! 3. `subtitles` (`id`, `subtitles_hash`, `name`, `lang`, `media_id`, `srt_path`, `certified`)
//! 4. `sub_frames` (`id`, `index`, `start_frame_offset`, `end_frame_offset`, `text`, `subtitles_id`)

use anyhow::{Context, Result};
use dbsrt_core::{
    build_and_validate_sub_frames, compute_bytes_digest_32, compute_media_phash_digest,
    read_phash_file, SubFrameAnchor, SubtitleFile,
};
use postgres::{Client, NoTls};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DbMatch {
    pub frame_offset: i32,
    pub media_id: i32,
}

#[derive(Debug, Clone)]
pub struct DbMediaRecord {
    pub id: i32,
    pub media_hash: String,
    pub name: String,
    pub fps: f64,
    pub frame_count: i32,
    pub phash_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DbSubtitleCoupling {
    pub id: i32,
    pub subtitles_hash: String,
    pub name: String,
    pub lang: String,
    pub media_id: i32,
    pub media_name: String,
    pub media_fps: f64,
    pub srt_path: Option<String>,
    pub certified: bool,
}

pub struct DbClient {
    client: Client,
}

/// Parses an optional `.XX.Xfps` suffix from a `.phash` stem.
/// Defaults to `23.976` fps for 1080p/720p/BluRay/WEB NTSC releases and `25.0` fps otherwise.
pub fn parse_fps_from_phash_stem(stem: &str) -> f64 {
    if let Some(fps_str) = stem.strip_suffix("fps") {
        if let Some(dot_idx) = fps_str.rfind('.') {
            if let Some(prev_dot) = fps_str[..dot_idx].rfind('.') {
                if let Ok(fps) = fps_str[prev_dot + 1..].parse::<f64>() {
                    if fps > 0.0 {
                        return fps;
                    }
                }
            }
        }
    }
    let lower = stem.to_lowercase();
    if lower.contains("1080p")
        || lower.contains("720p")
        || lower.contains("2160p")
        || lower.contains("bluray")
        || lower.contains("web-dl")
        || lower.contains("webrip")
    {
        return 23.976;
    }
    25.0
}

impl DbClient {
    pub fn connect(host: &str, port: u16, user: &str, pass: &str, dbname: &str) -> Result<Self> {
        let conn_str = format!(
            "host={} port={} user={} password={} dbname={} connect_timeout=2",
            host, port, user, pass, dbname
        );
        let client = Client::connect(&conn_str, NoTls)
            .with_context(|| format!("Failed to connect to PostgreSQL at {}:{}", host, port))?;
        Ok(Self { client })
    }

    /// Ensures all 4 relational tables (`medias`, `frames`, `subtitles`, `sub_frames`),
    /// columns, and BK-tree SP-GiST indexes exist while remaining 100% backward-compatible
    /// with the legacy Python schema.
    pub fn init_schema(&mut self) -> Result<()> {
        self.client.batch_execute(
            r#"
            CREATE TABLE IF NOT EXISTS medias (
                id SERIAL PRIMARY KEY,
                media_hash CHAR(32) UNIQUE,
                name TEXT UNIQUE
            );

            ALTER TABLE medias ADD COLUMN IF NOT EXISTS fps DOUBLE PRECISION DEFAULT 25.0;
            ALTER TABLE medias ADD COLUMN IF NOT EXISTS frame_count INT DEFAULT 0;
            ALTER TABLE medias ADD COLUMN IF NOT EXISTS phash_path TEXT;

            CREATE TABLE IF NOT EXISTS frames (
                id SERIAL PRIMARY KEY,
                p_hash BIGINT,
                frame_offset INT,
                media_id INT REFERENCES medias(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS frames_phash_bk_tree_index
            ON frames USING spgist ( p_hash bktree_ops );

            DO $$ BEGIN
                CREATE TYPE LANG AS ENUM ('FR', 'EN', 'ES', 'DE', 'IT', 'PT', 'JA', 'ZH');
            EXCEPTION
                WHEN duplicate_object THEN null;
            END $$;

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

            ALTER TABLE subtitles ADD COLUMN IF NOT EXISTS srt_path TEXT;
            ALTER TABLE subtitles ADD COLUMN IF NOT EXISTS certified BOOLEAN DEFAULT TRUE;
            ALTER TABLE subtitles DROP CONSTRAINT IF EXISTS subtitles_subtitles_hash_key;
            ALTER TABLE subtitles DROP CONSTRAINT IF EXISTS subtitles_subtitles_hash_name_lang_key;

            CREATE TABLE IF NOT EXISTS sub_frames (
                id SERIAL PRIMARY KEY,
                index INT NOT NULL,
                start_frame_offset INT NOT NULL,
                end_frame_offset INT NOT NULL,
                text TEXT DEFAULT '',
                subtitles_id INT NOT NULL REFERENCES subtitles(id) ON DELETE CASCADE,
                CHECK (end_frame_offset >= start_frame_offset)
            );
            "#,
        )?;
        Ok(())
    }

    /// Import a single `.phash` file into PostgreSQL BK-Tree database.
    /// Returns `(media_id, frames_inserted)`. If already exists, updates metadata and returns `(existing_id, 0)`.
    pub fn import_phash_file(&mut self, path: &Path) -> Result<(i32, usize)> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        let hashes = read_phash_file(path)?;
        if hashes.is_empty() {
            return Ok((0, 0));
        }

        let fps = parse_fps_from_phash_stem(name);
        let frame_count = hashes.len() as i32;
        let phash_path_str = path.to_string_lossy().to_string();
        let media_hash = compute_media_phash_digest(&hashes);

        // Check if media already in DB by media_hash or name
        let check_row = self.client.query_opt(
            "SELECT id FROM medias WHERE media_hash = $1 OR name = $2",
            &[&media_hash, &name],
        )?;

        if let Some(row) = check_row {
            let existing_id: i32 = row.get(0);
            let _ = self.client.execute(
                "UPDATE medias SET fps = $1, frame_count = $2, phash_path = $3 WHERE id = $4",
                &[&fps, &frame_count, &phash_path_str, &existing_id],
            );
            return Ok((existing_id, 0));
        }

        // Insert media record
        let row = self.client.query_one(
            "INSERT INTO medias (media_hash, name, fps, frame_count, phash_path) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&media_hash, &name, &fps, &frame_count, &phash_path_str],
        )?;
        let media_id: i32 = row.get(0);

        // Fast COPY frames into PostgreSQL
        let mut writer = self
            .client
            .copy_in("COPY frames (p_hash, frame_offset, media_id) FROM STDIN")?;

        let mut buffer = String::with_capacity(64 * 1024);
        for (offset, &h) in hashes.iter().enumerate() {
            let signed_h = h as i64;
            use std::fmt::Write as FmtWrite;
            let _ = writeln!(buffer, "{}\t{}\t{}", signed_h, offset, media_id);
            if buffer.len() >= 32 * 1024 {
                writer.write_all(buffer.as_bytes())?;
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            writer.write_all(buffer.as_bytes())?;
        }
        writer.finish()?;

        Ok((media_id, hashes.len()))
    }

    /// Certifies and imports a subtitle file (`.srt`) coupled to a `.phash` reference media into PostgreSQL:
    /// 1. Ensures the `.phash` media is imported in `medias` and `frames`
    /// 2. Validates and computes all `SubFrameAnchor` frame offsets (`start_frame_offset .. end_frame_offset`)
    /// 3. Inserts the record into `subtitles` (`certified = true`)
    /// 4. Bulk-inserts all cue frame anchors into `sub_frames` via `COPY ... FROM STDIN`
    ///
    /// Returns `(media_id, subtitles_id, sub_frames_inserted)`.
    pub fn import_subtitle_coupling(
        &mut self,
        phash_path: &Path,
        srt_path: &Path,
        lang: &str,
        certified: bool,
    ) -> Result<(i32, i32, usize)> {
        self.init_schema()?;
        let (media_id, _) = self.import_phash_file(phash_path)?;
        let hashes = read_phash_file(phash_path)?;
        let stem = phash_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let fps = parse_fps_from_phash_stem(stem);

        let raw_srt = fs::read_to_string(srt_path)
            .with_context(|| format!("Failed to read subtitle file {:?}", srt_path))?;
        let sub_file = SubtitleFile::parse(&raw_srt)?;
        let anchors = build_and_validate_sub_frames(&sub_file, &hashes, fps)?;

        let subtitles_hash = compute_bytes_digest_32(raw_srt.as_bytes());
        let sub_name = srt_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("subtitle");
        let lang_upper = if lang.to_lowercase().starts_with("fr") {
            "FR".to_string()
        } else if lang.to_lowercase().starts_with("en") {
            "EN".to_string()
        } else {
            lang.to_uppercase()
        };
        let srt_path_str = srt_path.to_string_lossy().to_string();

        // Check if subtitle already exists
        let existing_sub = self.client.query_opt(
            "SELECT id FROM subtitles WHERE subtitles_hash = $1 AND media_id = $2",
            &[&subtitles_hash, &media_id],
        )?;
        if let Some(row) = existing_sub {
            let sub_id: i32 = row.get(0);
            let _ = self.client.execute(
                "UPDATE subtitles SET srt_path = $1, certified = $2 WHERE id = $3",
                &[&srt_path_str, &certified, &sub_id],
            );
            return Ok((media_id, sub_id, 0));
        }

        // Insert into subtitles (cast $3::text to work whether column `lang` is ENUM `LANG` or `TEXT`)
        let sub_row = self.client.query_one(
            r#"
            INSERT INTO subtitles (subtitles_hash, name, media_id, srt_path, certified)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
            &[
                &subtitles_hash,
                &sub_name,
                &media_id,
                &srt_path_str,
                &certified,
            ],
        )?;
        let subtitles_id: i32 = sub_row.get(0);

        // Best-effort update of `lang` column (handles both legacy ENUM('FR','EN') and TEXT)
        let _ = self.client.execute(
            &format!(
                "UPDATE subtitles SET lang = '{}' WHERE id = $1",
                lang_upper.replace('\'', "")
            ),
            &[&subtitles_id],
        );

        // Bulk COPY sub_frames
        let mut writer = self.client.copy_in(
            "COPY sub_frames (index, start_frame_offset, end_frame_offset, text, subtitles_id) FROM STDIN",
        )?;
        let mut buffer = String::with_capacity(64 * 1024);
        for anchor in &anchors {
            // Escape tabs/newlines in cue text for PostgreSQL COPY format
            let clean_text = anchor
                .text
                .replace('\\', "\\\\")
                .replace('\t', " ")
                .replace('\r', "")
                .replace('\n', "\\n");
            use std::fmt::Write as FmtWrite;
            let _ = writeln!(
                buffer,
                "{}\t{}\t{}\t{}\t{}",
                anchor.index,
                anchor.start_frame_offset,
                anchor.end_frame_offset,
                clean_text,
                subtitles_id
            );
            if buffer.len() >= 32 * 1024 {
                writer.write_all(buffer.as_bytes())?;
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            writer.write_all(buffer.as_bytes())?;
        }
        writer.finish()?;

        Ok((media_id, subtitles_id, anchors.len()))
    }

    /// Recognizes a sequence of query frame pHashes against the PostgreSQL BK-tree (`frames`)
    /// and returns the best matching `(DbMediaRecord, confidence_0_to_1)`.
    pub fn recognize_sample(
        &mut self,
        query_hashes: &[u64],
        distance: u32,
    ) -> Result<Option<(DbMediaRecord, f64)>> {
        if query_hashes.is_empty() {
            return Ok(None);
        }
        let mut media_hits: HashMap<i32, usize> = HashMap::new();
        for &h in query_hashes {
            if let Ok(matches) = self.search_frame(h, distance) {
                let mut seen_for_frame = std::collections::HashSet::new();
                for m in matches {
                    if seen_for_frame.insert(m.media_id) {
                        *media_hits.entry(m.media_id).or_insert(0) += 1;
                    }
                }
            }
        }

        let Some((&best_media_id, &hits)) = media_hits.iter().max_by_key(|(_, &count)| count)
        else {
            return Ok(None);
        };

        let confidence = (hits as f64 / query_hashes.len() as f64).min(1.0);
        let row_opt = self.client.query_opt(
            "SELECT id, TRIM(media_hash), name, COALESCE(fps, 25.0), COALESCE(frame_count, 0), phash_path FROM medias WHERE id = $1",
            &[&best_media_id],
        )?;

        Ok(row_opt.map(|r| {
            (
                DbMediaRecord {
                    id: r.get(0),
                    media_hash: r.get(1),
                    name: r.get(2),
                    fps: r.get(3),
                    frame_count: r.get(4),
                    phash_path: r.get(5),
                },
                confidence,
            )
        }))
    }

    /// Queries all certified/coupled subtitles linked to `media_id` (or `media_name`) in PostgreSQL.
    pub fn get_subtitles_for_media(
        &mut self,
        media_id: Option<i32>,
        media_name: Option<&str>,
        lang_filter: Option<&str>,
    ) -> Result<Vec<DbSubtitleCoupling>> {
        let rows = if let Some(mid) = media_id {
            self.client.query(
                r#"
                SELECT s.id, TRIM(s.subtitles_hash), s.name, s.lang::text, s.media_id,
                       m.name, COALESCE(m.fps, 25.0), s.srt_path, COALESCE(s.certified, true)
                FROM subtitles s
                JOIN medias m ON m.id = s.media_id
                WHERE s.media_id = $1
                ORDER BY s.id ASC
                "#,
                &[&mid],
            )?
        } else if let Some(mname) = media_name {
            let clean = mname.strip_suffix(".phash").unwrap_or(mname);
            self.client.query(
                r#"
                SELECT s.id, TRIM(s.subtitles_hash), s.name, s.lang::text, s.media_id,
                       m.name, COALESCE(m.fps, 25.0), s.srt_path, COALESCE(s.certified, true)
                FROM subtitles s
                JOIN medias m ON m.id = s.media_id
                WHERE m.name = $1 OR m.name = $2
                ORDER BY s.id ASC
                "#,
                &[&mname, &clean],
            )?
        } else {
            self.client.query(
                r#"
                SELECT s.id, TRIM(s.subtitles_hash), s.name, s.lang::text, s.media_id,
                       m.name, COALESCE(m.fps, 25.0), s.srt_path, COALESCE(s.certified, true)
                FROM subtitles s
                JOIN medias m ON m.id = s.media_id
                ORDER BY s.id ASC
                "#,
                &[],
            )?
        };

        let norm_filter = lang_filter.map(|l| {
            let low = l.trim().to_lowercase();
            if low.starts_with("fr") {
                "fr".to_string()
            } else if low.starts_with("en") {
                "en".to_string()
            } else {
                low
            }
        });

        let mut results = Vec::new();
        for r in rows {
            let raw_lang: String = r.get(3);
            let norm_lang = if raw_lang.to_lowercase().starts_with("fr") {
                "fr".to_string()
            } else if raw_lang.to_lowercase().starts_with("en") {
                "en".to_string()
            } else {
                raw_lang.to_lowercase()
            };

            if let Some(ref f) = norm_filter {
                if f != "auto" && &norm_lang != f {
                    continue;
                }
            }

            results.push(DbSubtitleCoupling {
                id: r.get(0),
                subtitles_hash: r.get(1),
                name: r.get(2),
                lang: norm_lang,
                media_id: r.get(4),
                media_name: r.get(5),
                media_fps: r.get(6),
                srt_path: r.get(7),
                certified: r.get(8),
            });
        }

        Ok(results)
    }

    /// Reconstructs `SubFrameAnchor` entries (with their reference frame `p_hash` via SQL JOIN on `frames`)
    /// for a given `subtitles_id`.
    pub fn get_sub_frames_with_phash(&mut self, subtitles_id: i32) -> Result<Vec<SubFrameAnchor>> {
        let rows = self.client.query(
            r#"
            SELECT sf.index, sf.start_frame_offset, sf.end_frame_offset, sf.text, f.p_hash
            FROM sub_frames sf
            JOIN subtitles s ON s.id = sf.subtitles_id
            LEFT JOIN frames f ON f.media_id = s.media_id AND f.frame_offset = sf.start_frame_offset
            WHERE sf.subtitles_id = $1
            ORDER BY sf.index ASC
            "#,
            &[&subtitles_id],
        )?;

        let mut anchors = Vec::with_capacity(rows.len());
        for r in rows {
            let signed_hash: Option<i64> = r.get(4);
            anchors.push(SubFrameAnchor {
                index: r.get(0),
                start_frame_offset: r.get(1),
                end_frame_offset: r.get(2),
                text: r.get(3),
                anchor_phash: signed_hash.map(|v| v as u64),
            });
        }
        Ok(anchors)
    }

    /// Search matching frames using PostgreSQL BK-Tree operator (<@).
    pub fn search_frame(&mut self, phash: u64, distance: u32) -> Result<Vec<DbMatch>> {
        let signed_phash = phash as i64;
        let dist = distance as i64;
        let rows = self.client.query(
            "SELECT frame_offset, media_id FROM frames WHERE p_hash <@ ($1, $2)",
            &[&signed_phash, &dist],
        )?;

        let matches = rows
            .into_iter()
            .map(|r| DbMatch {
                frame_offset: r.get(0),
                media_id: r.get(1),
            })
            .collect();
        Ok(matches)
    }

    /// Look up media name by media_id.
    pub fn get_media_name(&mut self, media_id: i32) -> Result<String> {
        let rows = self
            .client
            .query("SELECT name FROM medias WHERE id = $1", &[&media_id])?;
        if let Some(row) = rows.first() {
            Ok(row.get(0))
        } else {
            Ok(format!("media_{}", media_id))
        }
    }

    /// Returns live row counts `(medias, frames, subtitles, sub_frames)` from PostgreSQL.
    pub fn get_table_counts(&mut self) -> Result<(i64, i64, i64, i64)> {
        let row = self.client.query_one(
            r#"
            SELECT
                (SELECT COUNT(*) FROM medias),
                (SELECT COUNT(*) FROM frames),
                (SELECT COUNT(*) FROM subtitles),
                (SELECT COUNT(*) FROM sub_frames)
            "#,
            &[],
        )?;
        Ok((row.get(0), row.get(1), row.get(2), row.get(3)))
    }
}
