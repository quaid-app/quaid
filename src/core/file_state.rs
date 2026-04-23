// File state tracking for stat-based change detection.
//
// The `file_state` table holds (mtime_ns, ctime_ns, size_bytes, inode, sha256) for
// every indexed file. Reconciliation compares these four stat fields first; any mismatch
// triggers a re-hash.

#![allow(dead_code)]

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

// ── File Metadata ─────────────────────────────────────────────

/// Stat tuple: (mtime_ns, ctime_ns, size_bytes, inode).
///
/// On Windows, `ctime_ns` and `inode` are always `None` (Windows doesn't expose
/// inode numbers, and ctime semantics differ). On Unix, both should be populated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub mtime_ns: i64,
    pub ctime_ns: Option<i64>,
    pub size_bytes: i64,
    pub inode: Option<i64>,
}

/// Stat a file and return the tuple.
///
/// Delegates to the path-based fallback (`std::fs::metadata`).
/// For fd-relative, NOFOLLOW semantics use `stat_file_fd` directly.
///
/// # Windows behavior
/// Always uses `std::fs::metadata`. `ctime_ns` and `inode` are `None`.
pub fn stat_file(path: &Path) -> io::Result<FileStat> {
    // For now, use the fallback path-based stat
    // Task 4.2 will wire fd-relative callers to use stat_file_fd directly
    stat_file_fallback(path)
}

/// Stat a file via the fallback path-based approach (std::fs::metadata).
///
/// On Unix, this uses `stat` (follows symlinks). For NOFOLLOW semantics,
/// use `fs_safety::stat_at_nofollow` with a parent fd.
fn stat_file_fallback(path: &Path) -> io::Result<FileStat> {
    let metadata = fs::metadata(path)?;
    let size_bytes = metadata.len() as i64;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mtime_ns = (metadata.mtime() as i64) * 1_000_000_000 + (metadata.mtime_nsec() as i64);
        let ctime_ns = (metadata.ctime() as i64) * 1_000_000_000 + (metadata.ctime_nsec() as i64);
        let inode = metadata.ino() as i64;
        Ok(FileStat {
            mtime_ns,
            ctime_ns: Some(ctime_ns),
            size_bytes,
            inode: Some(inode),
        })
    }

    #[cfg(not(unix))]
    {
        use std::time::SystemTime;
        let mtime = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        let mtime_ns = (mtime.as_secs() as i64) * 1_000_000_000 + (mtime.subsec_nanos() as i64);
        Ok(FileStat {
            mtime_ns,
            ctime_ns: None,
            size_bytes,
            inode: None,
        })
    }
}

#[cfg(unix)]
use crate::core::fs_safety;

/// Stat a file via fd-relative `fstatat(AT_SYMLINK_NOFOLLOW)` (Unix only).
///
/// This is the preferred stat path for reconciler walks — provides path-traversal
/// safety and NOFOLLOW semantics.
///
/// # Conversion note
/// `fs_safety::FileStatNoFollow` always has non-nullable ctime_ns and inode.
/// We convert to `FileStat` with `Some(...)` wrappers.
#[cfg(unix)]
pub fn stat_file_fd<Fd: rustix::fd::AsFd>(parent_fd: Fd, name: &Path) -> io::Result<FileStat> {
    let stat = fs_safety::stat_at_nofollow(parent_fd, name)?;
    Ok(FileStat {
        mtime_ns: stat.mtime_ns,
        ctime_ns: Some(stat.ctime_ns),
        size_bytes: stat.size_bytes,
        inode: Some(stat.inode),
    })
}

#[cfg(not(unix))]
pub fn stat_file_fd<Fd>(_parent_fd: Fd, name: &Path) -> io::Result<FileStat> {
    // Windows fallback: use path-based stat
    stat_file_fallback(name)
}

// ── SHA-256 Hashing ───────────────────────────────────────────

/// Compute SHA-256 of a file's content.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    let result = hasher.finalize();
    Ok(hex::encode(result))
}

// ── FileState Row ─────────────────────────────────────────────

/// A row from the `file_state` table.
#[derive(Debug, Clone)]
pub struct FileStateRow {
    pub collection_id: i64,
    pub relative_path: String,
    pub page_id: i64,
    pub mtime_ns: i64,
    pub ctime_ns: Option<i64>,
    pub size_bytes: i64,
    pub inode: Option<i64>,
    pub sha256: String,
    pub last_seen_at: String,
    pub last_full_hash_at: String,
}

/// Upsert a `file_state` row after ingesting a file.
///
/// This should be called in the same transaction as the `pages` insert/update.
pub fn upsert_file_state(
    conn: &Connection,
    collection_id: i64,
    relative_path: &str,
    page_id: i64,
    stat: &FileStat,
    sha256: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256, last_full_hash_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         ON CONFLICT(collection_id, relative_path) DO UPDATE SET
             page_id = excluded.page_id,
             mtime_ns = excluded.mtime_ns,
             ctime_ns = excluded.ctime_ns,
             size_bytes = excluded.size_bytes,
             inode = excluded.inode,
             sha256 = excluded.sha256,
             last_seen_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             last_full_hash_at = excluded.last_full_hash_at",
        rusqlite::params![
            collection_id,
            relative_path,
            page_id,
            stat.mtime_ns,
            stat.ctime_ns,
            stat.size_bytes,
            stat.inode,
            sha256,
        ],
    )?;
    Ok(())
}

/// Delete a `file_state` row (typically on page hard-delete).
pub fn delete_file_state(
    conn: &Connection,
    collection_id: i64,
    relative_path: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM file_state WHERE collection_id = ?1 AND relative_path = ?2",
        rusqlite::params![collection_id, relative_path],
    )?;
    Ok(())
}

/// Query a `file_state` row by (collection_id, relative_path).
pub fn get_file_state(
    conn: &Connection,
    collection_id: i64,
    relative_path: &str,
) -> rusqlite::Result<Option<FileStateRow>> {
    conn.query_row(
        "SELECT collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256, last_seen_at, last_full_hash_at
         FROM file_state WHERE collection_id = ?1 AND relative_path = ?2",
        rusqlite::params![collection_id, relative_path],
        |row| {
            Ok(FileStateRow {
                collection_id: row.get(0)?,
                relative_path: row.get(1)?,
                page_id: row.get(2)?,
                mtime_ns: row.get(3)?,
                ctime_ns: row.get(4)?,
                size_bytes: row.get(5)?,
                inode: row.get(6)?,
                sha256: row.get(7)?,
                last_seen_at: row.get(8)?,
                last_full_hash_at: row.get(9)?,
            })
        },
    )
    .optional()
}

// ── Stat Comparison ───────────────────────────────────────────

/// Compare two stat tuples and determine if a re-hash is needed.
///
/// Returns `true` if ANY of the four fields differ: mtime_ns, ctime_ns, size_bytes, inode.
/// A `true` result means the reconciler should re-hash the file.
pub fn stat_differs(a: &FileStat, b: &FileStat) -> bool {
    a.mtime_ns != b.mtime_ns
        || a.ctime_ns != b.ctime_ns
        || a.size_bytes != b.size_bytes
        || a.inode != b.inode
}

/// Compare current filesystem stat against stored `file_state` row.
///
/// Returns `true` if a re-hash is needed.
pub fn needs_rehash(current: &FileStat, stored: &FileStateRow) -> bool {
    current.mtime_ns != stored.mtime_ns
        || current.ctime_ns != stored.ctime_ns
        || current.size_bytes != stored.size_bytes
        || current.inode != stored.inode
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::Write;
    use tempfile::NamedTempFile;
    #[cfg(unix)]
    use tempfile::TempDir;

    fn open_file_state_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pages (collection_id, slug, type, title) VALUES (1, 'test', 'concept', 'Test')",
            [],
        )
        .unwrap();
        conn
    }

    fn sample_stat(
        mtime_ns: i64,
        ctime_ns: Option<i64>,
        size_bytes: i64,
        inode: Option<i64>,
    ) -> FileStat {
        FileStat {
            mtime_ns,
            ctime_ns,
            size_bytes,
            inode,
        }
    }

    #[test]
    fn stat_file_returns_size() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"hello world").unwrap();
        file.flush().unwrap();

        let stat = stat_file(file.path()).unwrap();
        assert_eq!(stat.size_bytes, 11);
    }

    #[test]
    fn hash_file_computes_sha256() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"hello world").unwrap();
        file.flush().unwrap();

        let hash = hash_file(file.path()).unwrap();
        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stat_file_fd_returns_full_tuple_for_regular_file() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), b"hello").unwrap();

        let root_fd = crate::core::fs_safety::open_root_fd(dir.path()).unwrap();
        let stat = stat_file_fd(&root_fd, Path::new("note.md")).unwrap();

        assert_eq!(stat.size_bytes, 5);
        assert!(stat.mtime_ns > 0);
        assert!(stat.ctime_ns.is_some());
        assert!(stat.inode.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn stat_file_fd_uses_nofollow_semantics_for_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("target.md"), b"hello").unwrap();
        symlink(dir.path().join("target.md"), dir.path().join("link.md")).unwrap();

        let root_fd = crate::core::fs_safety::open_root_fd(dir.path()).unwrap();
        let stat = stat_file_fd(&root_fd, Path::new("link.md")).unwrap();

        assert_ne!(stat.size_bytes, 5);
        assert!(stat.ctime_ns.is_some());
        assert!(stat.inode.is_some());
    }

    #[test]
    fn upsert_file_state_inserts_new_row() {
        let conn = open_file_state_db();
        let stat = sample_stat(1000, Some(2000), 100, Some(123));

        upsert_file_state(&conn, 1, "test.md", 1, &stat, "abc123").unwrap();

        let row = get_file_state(&conn, 1, "test.md").unwrap().unwrap();
        assert_eq!(row.page_id, 1);
        assert_eq!(row.mtime_ns, 1000);
        assert_eq!(row.ctime_ns, Some(2000));
        assert_eq!(row.size_bytes, 100);
        assert_eq!(row.inode, Some(123));
        assert_eq!(row.sha256, "abc123");
    }

    #[test]
    fn upsert_file_state_updates_existing_row() {
        let conn = open_file_state_db();
        let stat1 = sample_stat(1000, Some(2000), 100, Some(123));
        upsert_file_state(&conn, 1, "test.md", 1, &stat1, "abc123").unwrap();

        let stat2 = sample_stat(3000, Some(4000), 200, Some(123));
        upsert_file_state(&conn, 1, "test.md", 1, &stat2, "def456").unwrap();

        let row = get_file_state(&conn, 1, "test.md").unwrap().unwrap();
        assert_eq!(row.mtime_ns, 3000);
        assert_eq!(row.ctime_ns, Some(4000));
        assert_eq!(row.size_bytes, 200);
        assert_eq!(row.sha256, "def456");
    }

    #[test]
    fn delete_file_state_removes_row() {
        let conn = open_file_state_db();
        let stat = sample_stat(1000, Some(2000), 100, Some(123));
        upsert_file_state(&conn, 1, "test.md", 1, &stat, "abc123").unwrap();

        delete_file_state(&conn, 1, "test.md").unwrap();

        let row = get_file_state(&conn, 1, "test.md").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn stat_differs_detects_mtime_change() {
        let a = sample_stat(1000, Some(2000), 100, Some(123));
        let b = sample_stat(3000, Some(2000), 100, Some(123));
        assert!(stat_differs(&a, &b));
    }

    #[test]
    fn stat_differs_detects_ctime_change() {
        let a = sample_stat(1000, Some(2000), 100, Some(123));
        let b = sample_stat(1000, Some(4000), 100, Some(123));
        assert!(stat_differs(&a, &b));
    }

    #[test]
    fn stat_differs_detects_size_change() {
        let a = sample_stat(1000, Some(2000), 100, Some(123));
        let b = sample_stat(1000, Some(2000), 200, Some(123));
        assert!(stat_differs(&a, &b));
    }

    #[test]
    fn stat_differs_detects_inode_change() {
        let a = sample_stat(1000, Some(2000), 100, Some(123));
        let b = sample_stat(1000, Some(2000), 100, Some(456));
        assert!(stat_differs(&a, &b));
    }

    #[test]
    fn stat_differs_returns_false_when_all_match() {
        let a = sample_stat(1000, Some(2000), 100, Some(123));
        let b = sample_stat(1000, Some(2000), 100, Some(123));
        assert!(!stat_differs(&a, &b));
    }

    #[test]
    fn needs_rehash_returns_false_when_stored_tuple_matches() {
        let current = sample_stat(1000, Some(2000), 100, Some(123));
        let stored = FileStateRow {
            collection_id: 1,
            relative_path: "notes/test.md".to_owned(),
            page_id: 10,
            mtime_ns: 1000,
            ctime_ns: Some(2000),
            size_bytes: 100,
            inode: Some(123),
            sha256: "abc123".to_owned(),
            last_seen_at: "2026-04-22T00:00:00Z".to_owned(),
            last_full_hash_at: "2026-04-22T00:00:00Z".to_owned(),
        };

        assert!(!needs_rehash(&current, &stored));
    }

    #[test]
    fn needs_rehash_detects_ctime_only_drift_when_mtime_size_and_inode_match() {
        let current = sample_stat(1000, Some(4000), 100, Some(123));
        let stored = FileStateRow {
            collection_id: 1,
            relative_path: "notes/test.md".to_owned(),
            page_id: 10,
            mtime_ns: 1000,
            ctime_ns: Some(2000),
            size_bytes: 100,
            inode: Some(123),
            sha256: "abc123".to_owned(),
            last_seen_at: "2026-04-22T00:00:00Z".to_owned(),
            last_full_hash_at: "2026-04-22T00:00:00Z".to_owned(),
        };

        assert!(needs_rehash(&current, &stored));
    }

    #[test]
    fn needs_rehash_detects_inode_only_drift_when_mtime_ctime_and_size_match() {
        let current = sample_stat(1000, Some(2000), 100, Some(456));
        let stored = FileStateRow {
            collection_id: 1,
            relative_path: "notes/test.md".to_owned(),
            page_id: 10,
            mtime_ns: 1000,
            ctime_ns: Some(2000),
            size_bytes: 100,
            inode: Some(123),
            sha256: "abc123".to_owned(),
            last_seen_at: "2026-04-22T00:00:00Z".to_owned(),
            last_full_hash_at: "2026-04-22T00:00:00Z".to_owned(),
        };

        assert!(needs_rehash(&current, &stored));
    }

    #[test]
    fn upsert_file_state_sets_last_hash_timestamps() {
        let conn = open_file_state_db();
        let stat = sample_stat(1000, Some(2000), 100, Some(123));

        upsert_file_state(&conn, 1, "test.md", 1, &stat, "abc123").unwrap();

        let row = get_file_state(&conn, 1, "test.md").unwrap().unwrap();
        assert!(!row.last_seen_at.is_empty());
        assert!(!row.last_full_hash_at.is_empty());
    }
}
