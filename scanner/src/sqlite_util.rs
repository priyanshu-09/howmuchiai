use crate::types::ScanError;
use rusqlite::Connection;
use std::path::Path;
use tempfile::TempDir;

/// Safely opens a SQLite database that may be locked by another process (browser, IDE).
/// Copies the DB + WAL + SHM files to a temp directory and opens read-only.
pub struct SafeSqlite {
    _temp_dir: TempDir, // Held to prevent cleanup until dropped
    conn: Connection,
}

impl SafeSqlite {
    pub fn open(source: &Path) -> Result<Self, ScanError> {
        let temp_dir = tempfile::tempdir()?;
        let db_name = source
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("db"))
            .to_string_lossy()
            .to_string();
        let temp_path = temp_dir.path().join(&db_name);

        // Copy main DB file
        std::fs::copy(source, &temp_path)?;

        // Copy WAL and SHM companion files (critical for consistency)
        let source_str = source.to_string_lossy();
        for suffix in &["-wal", "-shm"] {
            let src_companion = PathBuf::from(format!("{}{}", source_str, suffix));
            if src_companion.exists() {
                let dst_companion =
                    PathBuf::from(format!("{}{}", temp_path.to_string_lossy(), suffix));
                // Non-fatal if companion copy fails
                let _ = std::fs::copy(&src_companion, &dst_companion);
            }
        }

        // Open read-write briefly to checkpoint WAL, then reopen read-only
        {
            let rw_conn = Connection::open_with_flags(
                &temp_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            // Merge WAL into main DB for a consistent snapshot
            let _ = rw_conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }

        // Restrict permissions on temp copy (contains sensitive data like browser history)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600));
        }

        let conn = Connection::open_with_flags(
            &temp_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        Ok(SafeSqlite {
            _temp_dir: temp_dir,
            conn,
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

use std::path::PathBuf;
