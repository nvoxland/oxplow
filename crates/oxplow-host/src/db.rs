//! `host.db` in the home: the host's own state (paired devices, projects,
//! a headless host's holds), and the one-host-per-home guard.
//!
//! The host opens it in SQLite's exclusive locking mode and takes the lock
//! at once, then keeps it for its whole life: a second host for the same
//! home cannot open the file, and the operating system lets go of the lock
//! when the process ends, a crash or a kill included. Everyone else reaches
//! the host's state through the host (its API, `host.sock` for the CLI and
//! the daemons); the CLI opens the file itself only when no host runs.
//!
//! One connection, behind a mutex: the host is the file's only user, and
//! its writes are small.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::Connection;

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// The file's name in the home.
pub const FILE: &str = "host.db";

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Another process holds the file: a host runs for this home (or the
    /// CLI is using it for a moment).
    #[error("another host has {0}")]
    Held(PathBuf),
    #[error("cannot open {0}: {1}; fix or move it, then start the host again")]
    Unreadable(PathBuf, String),
    #[error("the host's database failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// The open `host.db`, held for as long as this value lives.
pub struct HostDb {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl HostDb {
    /// Open the home's `host.db` (made when missing), take its lock and
    /// bring its tables up to date. A file another process holds is
    /// [`DbError::Held`], at once.
    pub fn open(config_dir: &Path) -> Result<Arc<Self>, DbError> {
        let path = path(config_dir);
        let unreadable =
            |e: &dyn std::fmt::Display| DbError::Unreadable(path.clone(), e.to_string());
        std::fs::create_dir_all(config_dir).map_err(|e| unreadable(&e))?;
        let mut conn = Connection::open(&path).map_err(|e| unreadable(&e))?;
        conn.busy_timeout(Duration::ZERO)?;
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // An exclusive transaction takes the lock; the locking mode keeps it
        // once the transaction ends.
        match conn.execute_batch("BEGIN EXCLUSIVE; COMMIT;") {
            Ok(()) => {}
            Err(e) if is_busy(&e) => return Err(DbError::Held(path)),
            Err(e) => return Err(unreadable(&e)),
        }
        embedded::migrations::runner()
            .run(&mut conn)
            .map_err(|e| unreadable(&e))?;
        Ok(Arc::new(Self {
            path,
            conn: Mutex::new(conn),
        }))
    }

    /// [`open`](Self::open), trying again while another process holds the
    /// file and `wait_for(elapsed)` says to.
    pub fn open_waiting(
        config_dir: &Path,
        mut wait_for: impl FnMut(Duration) -> bool,
    ) -> Result<Arc<Self>, DbError> {
        let started = Instant::now();
        loop {
            match Self::open(config_dir) {
                Err(DbError::Held(_)) if wait_for(started.elapsed()) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                other => return other,
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The connection, for one store call.
    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_opener_of_the_same_home_is_refused_until_the_first_lets_go() {
        let home = tempfile::tempdir().unwrap();
        let first = HostDb::open(home.path()).unwrap();
        assert!(matches!(HostDb::open(home.path()), Err(DbError::Held(_))));
        // Another home is another host.
        let other = tempfile::tempdir().unwrap();
        HostDb::open(other.path()).expect("a different home");
        drop(first);
        HostDb::open(home.path()).expect("free once the first is gone");
    }

    #[test]
    fn a_waiting_opener_gets_the_file_once_it_is_free() {
        let home = tempfile::tempdir().unwrap();
        let first = HostDb::open(home.path()).unwrap();
        let path = home.path().to_path_buf();
        let ending = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        HostDb::open_waiting(&path, |waited| waited < Duration::from_secs(5))
            .expect("the file, once the first is gone");
        ending.join().unwrap();
        // One that does not wait is told at once.
        let _held = HostDb::open(&path).unwrap();
        let asked = Instant::now();
        assert!(matches!(
            HostDb::open_waiting(&path, |_| false),
            Err(DbError::Held(_))
        ));
        assert!(asked.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_new_file_has_the_hosts_tables() {
        let home = tempfile::tempdir().unwrap();
        let db = HostDb::open(home.path()).unwrap();
        let tables: Vec<String> = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'refinery%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, ["device", "hold", "other_devices", "project"]);
        assert_eq!(db.path(), home.path().join(FILE));
    }

    /// A damaged file is never read as empty: the host stops and says why.
    #[test]
    fn a_file_that_is_no_database_stops_the_host() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(FILE),
            "not a database, but long enough to look like a header to sqlite........",
        )
        .unwrap();
        let err = HostDb::open(home.path()).err().unwrap().to_string();
        assert!(err.contains(FILE), "{err}");
    }
}
