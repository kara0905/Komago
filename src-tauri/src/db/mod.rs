mod migrations;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OpenFlags};

use crate::error::Result;

/// Accès à SQLite en WAL : les écritures sont sérialisées et les lectures ne
/// les bloquent pas.
pub struct Database {
    writer: Mutex<Connection>,
    reader: Mutex<Connection>,
}

impl Database {
    /// Ouvre la base, la migre si besoin (après sauvegarde) et la configure.
    pub fn open(path: &Path) -> Result<Self> {
        let mut writer = Connection::open(path)?;
        configure(&writer)?;
        migrations::run(&mut writer, path)?;

        let reader = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        configure(&reader)?;

        Ok(Self {
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
        })
    }

    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&lock(&self.reader))
    }

    pub fn write<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        f(&mut lock(&self.writer))
    }
}

// Un panic pendant une requête n'invalide pas la connexion : SQLite annule la
// transaction en cours quand elle est abandonnée.
fn lock(connection: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    connection.lock().unwrap_or_else(|e| e.into_inner())
}

fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}
