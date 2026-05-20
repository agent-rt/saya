//! SQLite storage layer.
//!
//! - WAL journal mode, busy_timeout 5s
//! - sqlite-vec extension registered globally via auto_extension
//! - r2d2 pool size 4; SQLite's own write lock serializes writers
//! - Vector table is always created; rows are only inserted when the
//!   `embedding` feature is enabled at runtime.

mod schema;
mod text_index;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OptionalExtension, params};

use text_index::TextIndex;

pub type Conn = PooledConnection<SqliteConnectionManager>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub id: i64,
    pub content: String,
    pub byte_size: i64,
    pub created_at: i64,
}

/// Aggregated launch statistics for one app, used by the launcher matcher
/// to bias scores toward recently / frequently used apps.
#[derive(Debug, Clone, Copy)]
pub struct MruInfo {
    pub count: u32,
    pub last_used_ms: i64,
}

#[derive(Clone)]
pub struct Database {
    pool: Pool<SqliteConnectionManager>,
    text_index: Arc<TextIndex>,
}

static EXT_INIT: Once = Once::new();

fn register_sqlite_vec() {
    EXT_INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        register_sqlite_vec();
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Step 1: bootstrap with a single connection so the WAL mode switch
        // and schema migration aren't racing pool-spawned siblings.
        {
            let conn = rusqlite::Connection::open(path)?;
            conn.busy_timeout(Duration::from_secs(5))?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.execute_batch(schema::SCHEMA)?;
        }

        // Step 2: build the pool. WAL is already persisted in the file; we
        // only apply per-connection PRAGMAs here.
        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.busy_timeout(Duration::from_secs(5))?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
            c.pragma_update(None, "temp_store", "MEMORY")?;
            c.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        });
        let pool = Pool::builder().max_size(4).build(manager)?;
        let text_index_dir = path
            .parent()
            .map(|p| p.join("text_index"))
            .unwrap_or_else(|| std::path::PathBuf::from("text_index"));
        let text_index = Arc::new(TextIndex::open(&text_index_dir)?);
        Ok(Self { pool, text_index })
    }

    pub fn open_in_memory() -> crate::Result<Self> {
        register_sqlite_vec();
        // Pool size 1: an in-memory SQLite is per-connection, so multiple
        // connections would each see an empty private DB.
        let manager = SqliteConnectionManager::memory().with_init(|c| {
            c.pragma_update(None, "foreign_keys", "ON")?;
            c.execute_batch(schema::SCHEMA)?;
            Ok(())
        });
        let pool = Pool::builder().max_size(1).build(manager)?;
        let text_index = Arc::new(TextIndex::open_in_memory()?);
        Ok(Self { pool, text_index })
    }

    pub fn conn(&self) -> crate::Result<Conn> {
        Ok(self.pool.get()?)
    }

    /// Insert a clipboard entry. Returns `None` if the content matches the most
    /// recent entry (per spec: drop consecutive duplicates).
    pub fn insert_entry(&self, content: &str) -> crate::Result<Option<i64>> {
        let conn = self.conn()?;
        let last: Option<String> = conn
            .query_row(
                "SELECT content FROM clipboard_entries ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if last.as_deref() == Some(content) {
            return Ok(None);
        }
        conn.execute(
            "INSERT INTO clipboard_entries (content, byte_size, created_at) VALUES (?1, ?2, ?3)",
            params![content, content.len() as i64, unix_ms()],
        )?;
        let id = conn.last_insert_rowid();
        // Best-effort: BM25 indexing failure must not block clipboard capture.
        if let Err(e) = self.text_index.insert(id, content) {
            tracing::warn!(error = %e, id, "text_index insert failed");
        }
        Ok(Some(id))
    }

    /// BM25 search over clipboard text. Returns (id, score) sorted by score desc.
    pub fn bm25_search(&self, query: &str, limit: usize) -> crate::Result<Vec<(i64, f32)>> {
        self.text_index.search(query, limit)
    }

    pub fn recent(&self, limit: usize) -> crate::Result<Vec<Entry>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, content, byte_size, created_at
             FROM clipboard_entries
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// LIKE-based substring search. Stand-in until Tantivy BM25 lands.
    pub fn like_search(&self, query: &str, limit: usize) -> crate::Result<Vec<Entry>> {
        let pattern = format!(
            "%{}%",
            query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
        );
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, content, byte_size, created_at
             FROM clipboard_entries
             WHERE content LIKE ?1 ESCAPE '\\'
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_older_than(&self, cutoff_unix_ms: i64) -> crate::Result<usize> {
        let conn = self.conn()?;
        // Two-step so the BM25 index stays consistent: fetch ids → delete from
        // Tantivy → delete from SQLite. If Tantivy fails we still proceed; a
        // stale BM25 id is harmless (the SQLite join drops orphans).
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM clipboard_entries WHERE created_at < ?1")?
            .query_map(params![cutoff_unix_ms], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in &ids {
            if let Err(e) = self.text_index.delete(*id) {
                tracing::warn!(error = %e, id, "text_index delete failed");
            }
        }
        let n = conn.execute(
            "DELETE FROM clipboard_entries WHERE created_at < ?1",
            params![cutoff_unix_ms],
        )?;
        Ok(n)
    }

    pub fn count(&self) -> crate::Result<i64> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT COUNT(*) FROM clipboard_entries", [], |r| r.get(0))?)
    }

    pub fn get_entry(&self, id: i64) -> crate::Result<Option<Entry>> {
        let conn = self.conn()?;
        let entry = conn
            .query_row(
                "SELECT id, content, byte_size, created_at FROM clipboard_entries WHERE id = ?1",
                params![id],
                row_to_entry,
            )
            .optional()?;
        Ok(entry)
    }

    /// Returns entries whose `id` is not yet present in `clipboard_vectors`.
    /// Used by the reindex pipeline to backfill embeddings.
    pub fn entries_missing_vectors(&self, limit: usize) -> crate::Result<Vec<Entry>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT e.id, e.content, e.byte_size, e.created_at
             FROM clipboard_entries e
             LEFT JOIN clipboard_vectors v ON e.id = v.rowid
             WHERE v.rowid IS NULL
             ORDER BY e.id DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Increment launch count for `path` and bump `last_used` to now.
    pub fn record_launch(&self, path: &str) -> crate::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO app_launches (path, count, last_used) VALUES (?1, 1, ?2)
             ON CONFLICT(path) DO UPDATE SET count = count + 1, last_used = ?2",
            params![path, unix_ms()],
        )?;
        Ok(())
    }

    /// Full launch history keyed by path. Sized in the dozens for most users.
    pub fn launch_history(&self) -> crate::Result<HashMap<String, MruInfo>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path, count, last_used FROM app_launches")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                MruInfo {
                    count: r.get::<_, i64>(1)? as u32,
                    last_used_ms: r.get(2)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    pub fn upsert_vector(&self, entry_id: i64, embedding: &[f32]) -> crate::Result<()> {
        if embedding.len() != 384 {
            return Err(crate::Error::Other(format!(
                "expected 384-dim embedding, got {}",
                embedding.len()
            )));
        }
        let json = serde_json::to_string(embedding)
            .map_err(|e| crate::Error::Other(format!("serialize embedding: {e}")))?;
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO clipboard_vectors (rowid, embedding) VALUES (?1, ?2)",
            params![entry_id, json],
        )?;
        Ok(())
    }

    /// Returns (entry_id, distance) sorted by ascending distance.
    pub fn vector_search(&self, query: &[f32], k: usize) -> crate::Result<Vec<(i64, f32)>> {
        if query.len() != 384 {
            return Err(crate::Error::Other(format!(
                "expected 384-dim query, got {}",
                query.len()
            )));
        }
        let json = serde_json::to_string(query)
            .map_err(|e| crate::Error::Other(format!("serialize query: {e}")))?;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT rowid, distance
             FROM clipboard_vectors
             WHERE embedding MATCH ?1 AND k = ?2
             ORDER BY distance",
        )?;
        let rows = stmt
            .query_map(params![json, k as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)? as f32))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        content: r.get(1)?,
        byte_size: r.get(2)?,
        created_at: r.get(3)?,
    })
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_dedup_and_recent() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.insert_entry("hello").unwrap().is_some());
        assert!(db.insert_entry("hello").unwrap().is_none(), "consecutive dup must be dropped");
        assert!(db.insert_entry("world").unwrap().is_some());
        assert!(db.insert_entry("hello").unwrap().is_some(), "non-consecutive dup is allowed");
        let recent = db.recent(10).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].content, "hello");
        assert_eq!(recent[2].content, "hello");
    }

    #[test]
    fn like_search_returns_matches() {
        let db = Database::open_in_memory().unwrap();
        db.insert_entry("the quick brown fox").unwrap();
        db.insert_entry("lazy dog").unwrap();
        db.insert_entry("brownish bear").unwrap();
        let hits = db.like_search("brown", 10).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn vector_upsert_and_search() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_entry("vec target").unwrap().unwrap();
        let v = vec![0.1_f32; 384];
        db.upsert_vector(id, &v).unwrap();
        let hits = db.vector_search(&v, 5).unwrap();
        assert!(!hits.is_empty(), "vector search must return the inserted row");
        assert_eq!(hits[0].0, id);
    }

    #[test]
    fn record_launch_increments_count_and_updates_timestamp() {
        let db = Database::open_in_memory().unwrap();
        db.record_launch("/Applications/Foo.app").unwrap();
        db.record_launch("/Applications/Foo.app").unwrap();
        db.record_launch("/Applications/Bar.app").unwrap();
        let hist = db.launch_history().unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist["/Applications/Foo.app"].count, 2);
        assert_eq!(hist["/Applications/Bar.app"].count, 1);
        let now = unix_ms();
        assert!((now - hist["/Applications/Foo.app"].last_used_ms).abs() < 1_000);
    }

    #[test]
    fn delete_older_than_purges() {
        let db = Database::open_in_memory().unwrap();
        db.insert_entry("old").unwrap();
        let future = unix_ms() + 60_000;
        let deleted = db.delete_older_than(future).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(db.count().unwrap(), 0);
    }
}
