//! PostgreSQL BK-tree client interface.

use anyhow::{Context, Result};
use postgres::{Client, NoTls};

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
        let rows = self.client.query(
            "SELECT name FROM medias WHERE id = $1",
            &[&media_id],
        )?;
        if let Some(row) = rows.first() {
            Ok(row.get(0))
        } else {
            Ok(format!("media_{}", media_id))
        }
    }
}
