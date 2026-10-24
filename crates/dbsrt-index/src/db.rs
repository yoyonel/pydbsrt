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

/// Initial string allocation capacity for batching PostgreSQL `COPY ... FROM STDIN` rows (64 KiB).
pub const COPY_BUFFER_INITIAL_CAPACITY: usize = 64 * 1024;

/// Flush threshold buffer size before writing batch chunks to PostgreSQL `copy_in` stream (32 KiB).
pub const COPY_BUFFER_FLUSH_THRESHOLD: usize = 32 * 1024;

/// Connection timeout in seconds for PostgreSQL client.
pub const DB_CONNECT_TIMEOUT_SECONDS: u64 = 2;

/// Default baseline frame rate when media FPS cannot be parsed from file metadata.
pub const DEFAULT_BASELINE_FPS: f64 = 25.0;

/// Standard NTSC film frame rate.
pub const NTSC_FILM_FPS: f64 = 23.976;

/// Standard PAL broadcast frame rate.
pub const PAL_FPS: f64 = 25.0;

/// Minimum character length for valid ISO 639 language code (e.g. "fr", "en").
pub const MIN_LANG_CODE_LEN: usize = 2;

/// Maximum character length for valid ISO 639 / BCP 47 language code (e.g. "fra", "pt-BR").
pub const MAX_LANG_CODE_LEN: usize = 8;

/// Fallback default language code when none specified or empty.
pub const DEFAULT_LANG_CODE: &str = "FR";

/// Default PostgreSQL port for custom Docker container.
pub const DEFAULT_POSTGRES_PORT_CUSTOM: u16 = 5433;

/// Standard PostgreSQL default port.
pub const DEFAULT_POSTGRES_PORT_STANDARD: u16 = 5432;

/// Validates and normalizes an ISO 639 / BCP 47 language code:
/// - Rejects strings containing SQL injections, metacharacters, or invalid lengths.
/// - Requires ASCII letters and optional subtag (e.g. `FR`, `EN`, `pt-BR`, `es-419`).
/// - Normalizes standard prefixes (`FR`, `EN`) or uppercase format.
pub fn validate_lang_code(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_LANG_CODE.to_string());
    }
    if trimmed.len() < MIN_LANG_CODE_LEN || trimmed.len() > MAX_LANG_CODE_LEN {
        anyhow::bail!(
            "Invalid language code length: expected {}-{} chars, got {:?}",
            MIN_LANG_CODE_LEN,
            MAX_LANG_CODE_LEN,
            raw
        );
    }
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() > 2 {
        anyhow::bail!("Invalid language code format (multiple hyphens): {:?}", raw);
    }
    let primary = parts[0];
    if primary.len() < 2 || primary.len() > 3 || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        anyhow::bail!(
            "Invalid primary language tag (expected 2-3 ASCII letters): {:?}",
            raw
        );
    }
    if parts.len() == 2 {
        let subtag = parts[1];
        if subtag.len() < 2
            || subtag.len() > 4
            || !subtag.chars().all(|c| c.is_ascii_alphanumeric())
        {
            anyhow::bail!(
                "Invalid language subtag (expected 2-4 alphanumeric chars): {:?}",
                raw
            );
        }
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("FR") {
        Ok("FR".to_string())
    } else if upper.starts_with("EN") {
        Ok("EN".to_string())
    } else {
        Ok(upper)
    }
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
    if lower.contains("pal")
        || lower.contains("576i")
        || lower.contains("576p")
        || lower.contains("25fps")
    {
        return PAL_FPS;
    }
    if lower.contains("1080p")
        || lower.contains("720p")
        || lower.contains("2160p")
        || lower.contains("480p")
        || lower.contains("bluray")
        || lower.contains("web-dl")
        || lower.contains("webrip")
        || lower.contains("web")
        || lower.contains("h264")
        || lower.contains("x264")
        || lower.contains("h265")
        || lower.contains("x265")
        || lower.contains("hevc")
        || lower.contains("ntsc")
        || lower.contains("[ember]")
        || lower.contains("[erai-raws]")
        || lower.contains("[subsplease]")
        || lower.contains("kimetsu")
        || lower.contains("haikyuu")
        || lower.contains("anime")
    {
        return NTSC_FILM_FPS;
    }
    DEFAULT_BASELINE_FPS
}

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

impl DbClient {
    pub fn connect(host: &str, port: u16, user: &str, pass: &str, dbname: &str) -> Result<Self> {
        let conn_str = format!(
            "host={} port={} user={} password={} dbname={} connect_timeout={}",
            host, port, user, pass, dbname, DB_CONNECT_TIMEOUT_SECONDS
        );
        let client = Client::connect(&conn_str, NoTls)
            .with_context(|| format!("Failed to connect to PostgreSQL at {}:{}", host, port))?;
        Ok(Self { client })
    }

    /// Returns true if the underlying PostgreSQL connection is closed.
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// Returns mutable reference to the underlying `postgres::Client`.
    pub fn client_mut(&mut self) -> &mut Client {
        &mut self.client
    }

    /// Subscribes to a PostgreSQL LISTEN channel.
    pub fn listen(&mut self, channel: &str) -> Result<()> {
        self.client.execute(&format!("LISTEN {}", channel), &[])?;
        Ok(())
    }

    /// Iterator over pending or incoming notifications from the database.
    pub fn notifications(&mut self) -> postgres::Notifications<'_> {
        self.client.notifications()
    }

    /// Blocks until the next notification arrives on a subscribed channel.
    pub fn next_notification(&mut self) -> Result<Option<postgres::Notification>> {
        use postgres::fallible_iterator::FallibleIterator;
        self.client
            .notifications()
            .blocking_iter()
            .next()
            .map_err(|e| anyhow::anyhow!("PostgreSQL notification read error: {e}"))
    }

    /// Ensures all 4 relational tables (`medias`, `frames`, `subtitles`, `sub_frames`),
    /// columns, and BK-tree SP-GiST indexes exist by applying versioned `refinery` migrations.
    /// Thread-safe and idempotent: recorded in `refinery_schema_history`.
    pub fn init_schema(&mut self) -> Result<()> {
        let _report = embedded::migrations::runner()
            .run(&mut self.client)
            .context("Failed to run refinery migrations on PostgreSQL")?;
        Ok(())
    }

    /// Runs pending migrations and returns the detailed refinery report.
    pub fn run_migrations(&mut self) -> Result<refinery::Report> {
        let report = embedded::migrations::runner()
            .run(&mut self.client)
            .context("Failed to run refinery migrations on PostgreSQL")?;
        Ok(report)
    }

    /// Returns the list of applied migration versions from `refinery_schema_history`.
    pub fn get_applied_migrations(&mut self) -> Result<Vec<(i32, String, String)>> {
        let table_exists = self.client.query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_name = 'refinery_schema_history'
            )",
            &[],
        )?;
        let exists: bool = table_exists.get(0);
        if !exists {
            return Ok(Vec::new());
        }

        let rows = self.client.query(
            "SELECT version, name, applied_on::text FROM refinery_schema_history ORDER BY version ASC",
            &[],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect())
    }

    /// Import a single `.phash` file into PostgreSQL BK-Tree database.
    /// Runs inside an explicit transaction: rolls back completely on COPY failure.
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

        let mut tx = self.client.transaction()?;

        // Check if media already in DB by media_hash or name
        let check_row = tx.query_opt(
            "SELECT id FROM medias WHERE media_hash = $1 OR name = $2",
            &[&media_hash, &name],
        )?;

        if let Some(row) = check_row {
            let existing_id: i32 = row.get(0);
            let _ = tx.execute(
                "UPDATE medias SET fps = $1, frame_count = $2, phash_path = $3 WHERE id = $4",
                &[&fps, &frame_count, &phash_path_str, &existing_id],
            );
            tx.commit()?;
            return Ok((existing_id, 0));
        }

        // Insert media record
        let row = tx.query_one(
            "INSERT INTO medias (media_hash, name, fps, frame_count, phash_path) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&media_hash, &name, &fps, &frame_count, &phash_path_str],
        )?;
        let media_id: i32 = row.get(0);

        // Fast atomic COPY frames into PostgreSQL
        let mut writer = tx.copy_in("COPY frames (p_hash, frame_offset, media_id) FROM STDIN")?;

        let mut buffer = String::with_capacity(COPY_BUFFER_INITIAL_CAPACITY);
        for (offset, &h) in hashes.iter().enumerate() {
            let signed_h = h as i64;
            use std::fmt::Write as FmtWrite;
            let _ = writeln!(buffer, "{}\t{}\t{}", signed_h, offset, media_id);
            if buffer.len() >= COPY_BUFFER_FLUSH_THRESHOLD {
                writer.write_all(buffer.as_bytes())?;
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            writer.write_all(buffer.as_bytes())?;
        }
        writer.finish()?;

        tx.commit()?;

        Ok((media_id, hashes.len()))
    }

    /// Certifies and imports a subtitle file (`.srt`) coupled to a `.phash` reference media into PostgreSQL:
    /// 1. Ensures the `.phash` media is imported in `medias` and `frames`
    /// 2. Validates and computes all `SubFrameAnchor` frame offsets (`start_frame_offset .. end_frame_offset`)
    /// 3. Validates and normalizes `lang` code securely (rejecting injections)
    /// 4. Inserts the record into `subtitles` (`certified = true`)
    /// 5. Bulk-inserts all cue frame anchors into `sub_frames` via atomic `COPY ... FROM STDIN`
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
        let anchors = match build_and_validate_sub_frames(&sub_file, &hashes, fps) {
            Ok(a) => a,
            Err(e) if (fps - DEFAULT_BASELINE_FPS).abs() < 1e-3 => {
                if let Ok(a) = build_and_validate_sub_frames(&sub_file, &hashes, NTSC_FILM_FPS) {
                    a
                } else {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        };

        let subtitles_hash = compute_bytes_digest_32(raw_srt.as_bytes());
        let sub_name = srt_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("subtitle");
        let validated_lang = validate_lang_code(lang)?;
        let srt_path_str = srt_path.to_string_lossy().to_string();

        let mut tx = self.client.transaction()?;

        // Check if subtitle already exists
        let existing_sub = tx.query_opt(
            "SELECT id FROM subtitles WHERE subtitles_hash = $1 AND media_id = $2",
            &[&subtitles_hash, &media_id],
        )?;
        if let Some(row) = existing_sub {
            let sub_id: i32 = row.get(0);
            let _ = tx.execute(
                "UPDATE subtitles SET srt_path = $1, certified = $2, lang = $3 WHERE id = $4",
                &[&srt_path_str, &certified, &validated_lang, &sub_id],
            );
            tx.commit()?;
            return Ok((media_id, sub_id, 0));
        }

        // Insert into subtitles with parameterized validated_lang
        let sub_row = tx.query_one(
            r#"
            INSERT INTO subtitles (subtitles_hash, name, lang, media_id, srt_path, certified)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
            &[
                &subtitles_hash,
                &sub_name,
                &validated_lang,
                &media_id,
                &srt_path_str,
                &certified,
            ],
        )?;
        let subtitles_id: i32 = sub_row.get(0);

        // Bulk COPY sub_frames within the same atomic transaction
        let mut writer = tx.copy_in(
            "COPY sub_frames (index, start_frame_offset, end_frame_offset, text, subtitles_id) FROM STDIN",
        )?;
        let mut buffer = String::with_capacity(COPY_BUFFER_INITIAL_CAPACITY);
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
            if buffer.len() >= COPY_BUFFER_FLUSH_THRESHOLD {
                writer.write_all(buffer.as_bytes())?;
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            writer.write_all(buffer.as_bytes())?;
        }
        writer.finish()?;

        tx.commit()?;

        Ok((media_id, subtitles_id, anchors.len()))
    }

    /// Recognizes a sequence of query frame pHashes against the PostgreSQL BK-tree (`frames`)
    /// using temporal offset coherence (`delta = frame_offset - q_idx`) and returns the best
    /// matching `(DbMediaRecord, confidence_0_to_1)` if it meets `MIN_TEMPORAL_COHERENCE_CONFIDENCE`.
    pub fn recognize_sample(
        &mut self,
        query_hashes: &[u64],
        distance: u32,
    ) -> Result<Option<(DbMediaRecord, f64)>> {
        if query_hashes.is_empty() {
            return Ok(None);
        }
        let mut media_pairs: HashMap<i32, Vec<(usize, usize)>> = HashMap::new();
        for (q_idx, &h) in query_hashes.iter().enumerate() {
            if dbsrt_core::is_blank_phash(h) {
                continue;
            }
            if let Ok(matches) = self.search_frame(h, distance) {
                for m in matches {
                    if m.frame_offset >= 0 {
                        media_pairs
                            .entry(m.media_id)
                            .or_default()
                            .push((q_idx, m.frame_offset as usize));
                    }
                }
            }
        }

        let mut best_candidate: Option<(i32, usize, f64)> = None;
        for (media_id, pairs) in &media_pairs {
            let (coherent_count, conf) =
                crate::mmap::compute_temporal_coherence(query_hashes, pairs);
            if conf >= crate::mmap::MIN_TEMPORAL_COHERENCE_CONFIDENCE {
                let is_better = match best_candidate {
                    None => true,
                    Some((_, best_cnt, best_conf)) => {
                        conf > best_conf || (conf == best_conf && coherent_count > best_cnt)
                    }
                };
                if is_better {
                    best_candidate = Some((*media_id, coherent_count, conf));
                }
            }
        }

        let Some((best_media_id, _, confidence)) = best_candidate else {
            return Ok(None);
        };

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

    /// Deletes a media entry and its cascading child records (frames, subtitles, sub_frames) by name.
    pub fn delete_media_by_name(&mut self, name: &str) -> Result<u64> {
        let count = self
            .client
            .execute("DELETE FROM medias WHERE name = $1", &[&name])?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::write_phash_file;

    #[test]
    fn test_parse_fps_from_phash_stem_branches() {
        assert!((parse_fps_from_phash_stem("movie.mp4.25.0fps") - 25.0).abs() < 1e-6);
        assert!((parse_fps_from_phash_stem("episode.mkv.23.976fps") - 23.976).abs() < 1e-6);
        assert!((parse_fps_from_phash_stem("show.s01e01.1080p.web.h264") - 23.976).abs() < 1e-6);
        assert!((parse_fps_from_phash_stem("show.s01e01.720p.bluray") - 23.976).abs() < 1e-6);
        assert!((parse_fps_from_phash_stem("show.s01e01.2160p.web-dl") - 23.976).abs() < 1e-6);
        assert!((parse_fps_from_phash_stem("show.s01e01.webrip.x264") - 23.976).abs() < 1e-6);
        assert!((parse_fps_from_phash_stem("pal_dvd_release") - 25.0).abs() < 1e-6);
    }

    fn try_connect_test_db() -> Option<DbClient> {
        let host = std::env::var("PSQL_IP").unwrap_or_else(|_| "127.0.0.1".to_string());
        let env_port = std::env::var("PSQL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok());
        let ports: Vec<u16> = match env_port {
            Some(p) => vec![
                p,
                DEFAULT_POSTGRES_PORT_CUSTOM,
                DEFAULT_POSTGRES_PORT_STANDARD,
            ],
            None => vec![DEFAULT_POSTGRES_PORT_CUSTOM, DEFAULT_POSTGRES_PORT_STANDARD],
        };
        for port in ports {
            if let Ok(client) = DbClient::connect(&host, port, "docker", "docker", "bktreedb") {
                return Some(client);
            }
        }
        None
    }

    #[test]
    fn test_db_client_full_relational_lifecycle() {
        let Some(mut db) = try_connect_test_db() else {
            eprintln!(
                "PostgreSQL not reachable, skipping test_db_client_full_relational_lifecycle"
            );
            return;
        };

        db.init_schema().expect("init_schema must succeed");

        let tmp_dir =
            std::env::temp_dir().join(format!("dbsrt_db_unit_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp_dir);

        // 1. Empty phash file handling
        let empty_phash = tmp_dir.join("empty_test.25.0fps.phash");
        write_phash_file(&empty_phash, &[]).unwrap();
        let (empty_id, empty_cnt) = db.import_phash_file(&empty_phash).unwrap();
        assert_eq!(empty_id, 0);
        assert_eq!(empty_cnt, 0);

        // 2. Non-empty synthetic phash file (500 frames @ 25.0 fps = 20s)
        let synth_phash = tmp_dir.join("unit_test_media_cov.mp4.25.0fps.phash");
        let hashes: Vec<u64> = (0..500)
            .map(|i| 0x5555_AAAA_0000_0000u64 ^ ((i as u64) * 0x0101_0101))
            .collect();
        write_phash_file(&synth_phash, &hashes).unwrap();

        let (media_id, inserted) = db.import_phash_file(&synth_phash).unwrap();
        assert!(media_id > 0);
        // Re-importing same media updates metadata and returns (media_id, 0)
        let (same_id, re_inserted) = db.import_phash_file(&synth_phash).unwrap();
        assert_eq!(same_id, media_id);
        assert_eq!(re_inserted, 0);
        let _ = inserted;

        // 3. Test get_media_name (existing + fallback)
        let mname = db.get_media_name(media_id).unwrap();
        assert_eq!(mname, "unit_test_media_cov.mp4.25.0fps");
        let fallback_name = db.get_media_name(-999_999).unwrap();
        assert_eq!(fallback_name, "media_-999999");

        // 4. Test search_frame and recognize_sample
        assert!(db.recognize_sample(&[], 2).unwrap().is_none());
        let matches = db.search_frame(hashes[100], 0).unwrap();
        assert!(!matches.is_empty());

        let rec = db
            .recognize_sample(&hashes[50..80], 2)
            .unwrap()
            .expect("Must recognize synthetic sample");
        assert!(rec.1 > 0.8);

        // 5. Test import_subtitle_coupling (FR, EN, and ES)
        let srt_fr = tmp_dir.join("unit_test_media_cov.fr.srt");
        std::fs::write(
            &srt_fr,
            "1\n00:00:01,000 --> 00:00:03,000\nBonjour le monde\n\n2\n00:00:04,000 --> 00:00:06,000\nDeuxieme ligne\n\n",
        )
        .unwrap();
        let (m_id, sub_fr_id, sf_count) = db
            .import_subtitle_coupling(&synth_phash, &srt_fr, "fr", true)
            .unwrap();
        assert_eq!(m_id, media_id);
        assert!(sub_fr_id > 0);
        assert_eq!(sf_count, 2);

        // Idempotent re-import of same subtitle
        let (_, same_sub_id, sf_re) = db
            .import_subtitle_coupling(&synth_phash, &srt_fr, "fr", true)
            .unwrap();
        assert_eq!(same_sub_id, sub_fr_id);
        assert_eq!(sf_re, 0);

        let srt_en = tmp_dir.join("unit_test_media_cov.en.srt");
        std::fs::write(&srt_en, "1\n00:00:01,500 --> 00:00:03,500\nHello world\n\n").unwrap();
        let (_, sub_en_id, _) = db
            .import_subtitle_coupling(&synth_phash, &srt_en, "en", true)
            .unwrap();
        assert!(sub_en_id > 0);

        // 6. Test get_subtitles_for_media (by id, by name, all, and language filter)
        let by_id_fr = db
            .get_subtitles_for_media(Some(media_id), None, Some("fr"))
            .unwrap();
        assert!(!by_id_fr.is_empty());
        assert!(by_id_fr.iter().all(|s| s.lang == "fr"));

        let by_name_en = db
            .get_subtitles_for_media(
                None,
                Some("unit_test_media_cov.mp4.25.0fps.phash"),
                Some("en"),
            )
            .unwrap();
        assert!(!by_name_en.is_empty());

        let all_subs = db
            .get_subtitles_for_media(None, None, Some("auto"))
            .unwrap();
        assert!(all_subs.len() >= 2);

        // 7. Test get_sub_frames_with_phash & get_table_counts
        let anchors = db.get_sub_frames_with_phash(sub_fr_id).unwrap();
        assert_eq!(anchors.len(), 2);
        assert!(anchors[0].anchor_phash.is_some());

        let counts = db.get_table_counts().unwrap();
        assert!(counts.0 >= 1 && counts.1 >= 500 && counts.2 >= 2 && counts.3 >= 3);

        // Cleanup test media row (ON DELETE CASCADE cleans frames, subtitles, sub_frames)
        let _ = db
            .client
            .execute("DELETE FROM medias WHERE id = $1", &[&media_id]);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_validate_lang_code_accepts_valid() {
        assert_eq!(validate_lang_code("fr").unwrap(), "FR");
        assert_eq!(validate_lang_code("FR").unwrap(), "FR");
        assert_eq!(validate_lang_code("en").unwrap(), "EN");
        assert_eq!(validate_lang_code("EN").unwrap(), "EN");
        assert_eq!(validate_lang_code("es").unwrap(), "ES");
        assert_eq!(validate_lang_code("spa").unwrap(), "SPA");
        assert_eq!(validate_lang_code("de").unwrap(), "DE");
        assert_eq!(validate_lang_code("ja").unwrap(), "JA");
        assert_eq!(validate_lang_code("zh").unwrap(), "ZH");
        assert_eq!(validate_lang_code("pt-BR").unwrap(), "PT-BR");
        assert_eq!(validate_lang_code("en-US").unwrap(), "EN");
        assert_eq!(validate_lang_code("fr-CA").unwrap(), "FR");
        assert_eq!(validate_lang_code("").unwrap(), "FR");
        assert_eq!(validate_lang_code("   ").unwrap(), "FR");
    }

    #[test]
    fn test_validate_lang_code_rejects_malicious_and_invalid() {
        assert!(validate_lang_code("f").is_err());
        assert!(validate_lang_code("toolongcode").is_err());
        assert!(validate_lang_code("123").is_err());
        assert!(validate_lang_code("fr; DROP TABLE subtitles;--").is_err());
        assert!(validate_lang_code("' OR '1'='1").is_err());
        assert!(validate_lang_code("fr-FR-extra").is_err());
        assert!(validate_lang_code("fr--fr").is_err());
        assert!(validate_lang_code("fr/en").is_err());
        assert!(validate_lang_code("fr\x00").is_err());
    }

    #[test]
    fn test_copy_transaction_rollback_on_failure() {
        let Some(mut db) = try_connect_test_db() else {
            eprintln!(
                "PostgreSQL not reachable, skipping test_copy_transaction_rollback_on_failure"
            );
            return;
        };

        db.init_schema().expect("init_schema must succeed");

        let unique_media_hash = format!("dummy_{:026x}", std::process::id());
        let name = format!("test_rollback_{}", std::process::id());

        // Simulate a failure during COPY: insert media, start copy, then inject invalid data
        {
            let mut tx = db.client.transaction().unwrap();
            let row = tx
                .query_one(
                    "INSERT INTO medias (media_hash, name, fps, frame_count, phash_path) VALUES ($1, $2, $3, $4, $5) RETURNING id",
                    &[&unique_media_hash, &name, &PAL_FPS, &100i32, &"dummy.phash"],
                )
                .unwrap();
            let media_id: i32 = row.get(0);
            assert!(media_id > 0);

            let mut writer = tx
                .copy_in("COPY frames (p_hash, frame_offset, media_id) FROM STDIN")
                .unwrap();
            // Write invalid/corrupted data to COPY stream to trigger failure
            let bad_data = b"not_a_valid_row\n";
            let _ = writer.write_all(bad_data);
            let finish_res = writer.finish();
            assert!(finish_res.is_err(), "COPY with invalid data must fail");
            // Do NOT call tx.commit(); tx drops here and rolls back
        }

        // Verify that the media row was completely rolled back and DOES NOT exist in DB
        let check = db
            .client
            .query_opt(
                "SELECT id FROM medias WHERE media_hash = $1",
                &[&unique_media_hash],
            )
            .unwrap();
        assert!(
            check.is_none(),
            "Orphaned media record must NOT exist after failed COPY transaction"
        );
    }

    #[test]
    fn test_subtitle_coupling_transaction_rollback_on_failure() {
        let Some(mut db) = try_connect_test_db() else {
            eprintln!(
                "PostgreSQL not reachable, skipping test_subtitle_coupling_transaction_rollback_on_failure"
            );
            return;
        };

        db.init_schema().expect("init_schema must succeed");

        let unique_sub_hash = format!("sub_err_{:024x}", std::process::id());
        let sub_name = format!("test_sub_rollback_{}", std::process::id());

        // Insert dummy media for coupling foreign key
        let media_hash = format!("med_for_sub_{:020x}", std::process::id());
        let m_row = db
            .client
            .query_one(
                "INSERT INTO medias (media_hash, name, fps, frame_count) VALUES ($1, $2, $3, $4) RETURNING id",
                &[&media_hash, &media_hash, &PAL_FPS, &10i32],
            )
            .unwrap();
        let media_id: i32 = m_row.get(0);

        {
            let mut tx = db.client.transaction().unwrap();
            let row = tx
                .query_one(
                    "INSERT INTO subtitles (subtitles_hash, name, lang, media_id, certified) VALUES ($1, $2, $3, $4, $5) RETURNING id",
                    &[&unique_sub_hash, &sub_name, &DEFAULT_LANG_CODE, &media_id, &true],
                )
                .unwrap();
            let sub_id: i32 = row.get(0);
            assert!(sub_id > 0);

            let mut writer = tx
                .copy_in(
                    "COPY sub_frames (index, start_frame_offset, end_frame_offset, text, subtitles_id) FROM STDIN",
                )
                .unwrap();
            let _ = writer.write_all(b"corrupted_copy_data\n");
            let finish_res = writer.finish();
            assert!(finish_res.is_err(), "COPY with invalid data must fail");
            // tx drops without commit
        }

        // Verify that subtitles row was rolled back
        let check = db
            .client
            .query_opt(
                "SELECT id FROM subtitles WHERE subtitles_hash = $1",
                &[&unique_sub_hash],
            )
            .unwrap();
        assert!(
            check.is_none(),
            "Orphaned subtitle record must NOT exist after failed COPY transaction"
        );

        // Cleanup dummy media
        let _ = db
            .client
            .execute("DELETE FROM medias WHERE id = $1", &[&media_id]);
    }

    #[test]
    fn test_refinery_migrations_lifecycle() {
        let Some(mut db) = try_connect_test_db() else {
            eprintln!("PostgreSQL not reachable, skipping test_refinery_migrations_lifecycle");
            return;
        };

        db.init_schema()
            .expect("init_schema via refinery must succeed");

        let applied = db
            .get_applied_migrations()
            .expect("Must retrieve applied migrations");
        assert!(
            applied.len() >= 3,
            "Expected at least 3 migrations (V1, V2, V3), got {}",
            applied.len()
        );
        assert_eq!(applied[0].0, 1);
        assert!(applied[0].1.contains("initial_schema"));
        assert_eq!(applied[1].0, 2);
        assert!(applied[1].1.contains("certified_couplings"));
        assert_eq!(applied[2].0, 3);
        assert!(applied[2].1.contains("secure_lang_and_cascades"));
    }
}
