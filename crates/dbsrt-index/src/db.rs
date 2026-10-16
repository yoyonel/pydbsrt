//! PostgreSQL BK-tree client interface.

use anyhow::{Context, Result};
use dbsrt_core::read_phash_file;
use postgres::{Client, NoTls};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DbMatch {
    pub frame_offset: i32,
    pub media_id: i32,
}

pub struct DbClient {
    client: Client,
}

impl DbClient {
    pub fn connect(host: &str, port: u16, user: &str, pass: &str, dbname: &str) -> Result<Self> {
        let conn_str = format!(
            "host={} port={} user={} password={} dbname={}",
            host, port, user, pass, dbname
        );
        let client = Client::connect(&conn_str, NoTls)
            .with_context(|| format!("Failed to connect to PostgreSQL at {}:{}", host, port))?;
        Ok(Self { client })
    }

    /// Ensures tables and BK-tree SP-GiST indexes exist.
    pub fn init_schema(&mut self) -> Result<()> {
        self.client.batch_execute(
            r#"
            CREATE TABLE IF NOT EXISTS medias (
                id SERIAL PRIMARY KEY,
                media_hash CHAR(32) UNIQUE,
                name TEXT UNIQUE
            );

            CREATE TABLE IF NOT EXISTS frames (
                id SERIAL PRIMARY KEY,
                p_hash BIGINT,
                frame_offset INT,
                media_id INT REFERENCES medias(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS frames_phash_bk_tree_index
            ON frames USING spgist ( p_hash bktree_ops );
            "#,
        )?;
        Ok(())
    }

    /// Import a single .phash file into PostgreSQL BK-Tree database.
    /// Returns (media_id, frames_inserted). If already exists, returns (existing_id, 0).
    pub fn import_phash_file(&mut self, path: &Path) -> Result<(i32, usize)> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        let hashes = read_phash_file(path)?;
        if hashes.is_empty() {
            return Ok((0, 0));
        }

        // Compute 32-char media_hash using Sha256
        let mut hasher = Sha256::new();
        for &h in &hashes {
            hasher.update(h.to_be_bytes());
        }
        let digest = hasher.finalize();
        let mut digest_hex = String::with_capacity(64);
        for byte in &digest {
            use std::fmt::Write as FmtWrite;
            let _ = write!(digest_hex, "{:02x}", byte);
        }
        let media_hash = &digest_hex[..32];

        // Check if media already in DB by media_hash or name
        let check_row = self.client.query_opt(
            "SELECT id FROM medias WHERE media_hash = $1 OR name = $2",
            &[&media_hash, &name],
        )?;

        if let Some(row) = check_row {
            let existing_id: i32 = row.get(0);
            return Ok((existing_id, 0));
        }

        // Insert media record
        let row = self.client.query_one(
            "INSERT INTO medias (media_hash, name) VALUES ($1, $2) RETURNING id",
            &[&media_hash, &name],
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
}
