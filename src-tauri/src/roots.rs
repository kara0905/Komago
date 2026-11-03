//! Dossiers suivis : Komago y repère ce qui n'est pas encore dans la bibliothèque.
//! Rien n'est importé sans passer par le plan d'import habituel.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{Error, Result};
use crate::import;

/// Garde-fou : au-delà, on montre les premiers éléments et on invite à importer.
const MAX_NEW_PER_ROOT: usize = 2_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub id: String,
    pub path: String,
    pub status: String,
    pub source_count: i64,
    pub last_scanned_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootScan {
    pub root_id: String,
    pub path: String,
    pub available: bool,
    pub new_paths: Vec<String>,
}

pub fn list(connection: &Connection) -> Result<Vec<Root>> {
    let mut statement = connection.prepare(
        "SELECT r.id, r.path, r.status, (SELECT count(*) FROM source_files s WHERE s.root_id = r.id), r.last_scanned_at
         FROM library_roots r ORDER BY r.path",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(Root {
            id: row.get(0)?,
            path: row.get(1)?,
            status: row.get(2)?,
            source_count: row.get(3)?,
            last_scanned_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn paths(connection: &Connection) -> Result<Vec<PathBuf>> {
    Ok(list_ids_and_paths(connection)?.into_iter().map(|(_, path)| path).collect())
}

pub fn list_ids_and_paths(connection: &Connection) -> Result<Vec<(String, PathBuf)>> {
    let mut statement = connection.prepare("SELECT id, path FROM library_roots")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, PathBuf::from(row.get::<_, String>(1)?))))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Le chemin est gardé tel que choisi : les sources importées s'y réfèrent sous
/// la même forme, et une forme « canonique » différente les ferait paraître nouvelles.
pub fn add(connection: &mut Connection, path: &Path) -> Result<String> {
    if !path.is_dir() {
        return Err(Error::Invalid("ce dossier est introuvable".into()));
    }
    for (_, existing) in list_ids_and_paths(connection)? {
        if existing == path {
            return Err(Error::Invalid("ce dossier est déjà suivi".into()));
        }
        if path.starts_with(&existing) || existing.starts_with(path) {
            return Err(Error::Invalid(format!(
                "ce dossier recoupe un dossier déjà suivi : {}",
                existing.display()
            )));
        }
    }
    let id = Uuid::now_v7().to_string();
    let tx = connection.transaction()?;
    tx.execute("INSERT INTO library_roots (id, path) VALUES (?1, ?2)", params![id, path.to_string_lossy()])?;
    // Les fichiers déjà importés depuis ce dossier lui sont rattachés.
    let mut statement = tx.prepare("SELECT id, path FROM source_files WHERE root_id IS NULL")?;
    let sources: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for (source_id, source_path) in sources {
        if Path::new(&source_path).starts_with(path) {
            tx.execute("UPDATE source_files SET root_id = ?1 WHERE id = ?2", params![id, source_id])?;
        }
    }
    tx.commit()?;
    Ok(id)
}

/// Ne retire que le suivi : la bibliothèque et les fichiers restent intacts.
pub fn remove(connection: &mut Connection, id: &str) -> Result<()> {
    connection.execute("DELETE FROM library_roots WHERE id = ?1", [id])?;
    Ok(())
}

pub fn clear_ignored(connection: &mut Connection) -> Result<usize> {
    Ok(connection.execute("DELETE FROM ignored_sources", [])?)
}

pub fn ignored_count(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row("SELECT count(*) FROM ignored_sources", [], |r| r.get(0))?)
}

/// Parcours rapide : seuls les noms et la présence des fichiers sont lus, aucune
/// archive n'est ouverte. Un disque débranché rend le dossier indisponible sans
/// rien effacer.
pub fn scan(database: &Database) -> Result<Vec<RootScan>> {
    let (roots, known, ignored) = database.read(|connection| {
        let roots = list_ids_and_paths(connection)?;
        let mut statement = connection.prepare("SELECT path FROM source_files")?;
        let known: HashSet<String> = statement.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut statement = connection.prepare("SELECT path FROM ignored_sources")?;
        let ignored: HashSet<String> = statement.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok((roots, known, ignored))
    })?;

    let mut scans = Vec::new();
    for (id, path) in roots {
        let available = path.is_dir();
        let mut new_paths: Vec<String> = if available {
            import::discover(&path)
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .filter(|p| !known.contains(p) && !ignored.contains(p))
                .collect()
        } else {
            Vec::new()
        };
        new_paths.truncate(MAX_NEW_PER_ROOT);
        database.write(|connection| {
            connection.execute(
                "UPDATE library_roots SET status = ?1,
                        last_scanned_at = CASE WHEN ?1 = 'available' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') ELSE last_scanned_at END
                 WHERE id = ?2",
                params![if available { "available" } else { "missing" }, id],
            )?;
            Ok(())
        })?;
        scans.push(RootScan { root_id: id, path: path.to_string_lossy().into_owned(), available, new_paths });
    }
    Ok(scans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::test_support::{write_zip, JPEG};
    use std::fs;
    use tempfile::{tempdir, TempDir};

    fn setup() -> (TempDir, Database) {
        let dir = tempdir().unwrap();
        let database = Database::open(&dir.path().join("komago.db")).unwrap();
        (dir, database)
    }

    fn cbz(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_zip(path, &[("1.jpg", JPEG)]);
    }

    #[test]
    fn overlapping_or_missing_folders_are_refused() {
        let (dir, database) = setup();
        let root = dir.path().join("Mangas");
        fs::create_dir_all(root.join("Berserk")).unwrap();
        database.write(|c| add(c, &root)).unwrap();

        assert!(database.write(|c| add(c, &root)).is_err());
        assert!(database.write(|c| add(c, &root.join("Berserk"))).is_err());
        assert!(database.write(|c| add(c, dir.path())).is_err());
        assert!(database.write(|c| add(c, &dir.path().join("absent"))).is_err());
    }

    #[test]
    fn scan_lists_only_what_is_new_and_not_ignored() {
        let (dir, database) = setup();
        let root = dir.path().join("Mangas");
        cbz(&root.join("A/Chapter 1.cbz"));
        cbz(&root.join("A/Chapter 2.cbz"));
        cbz(&root.join("B/Chapter 1.cbz"));
        fs::create_dir_all(root.join("C/Chapter 1")).unwrap();
        fs::write(root.join("C/Chapter 1/1.jpg"), JPEG).unwrap();
        let known = root.join("A/Chapter 1.cbz");
        let ignored = root.join("B/Chapter 1.cbz");
        database
            .write(|c| {
                c.execute(
                    "INSERT INTO source_files (id, path, kind, size_bytes) VALUES ('s', ?1, 'cbz', 1)",
                    [known.to_string_lossy()],
                )?;
                c.execute("INSERT INTO ignored_sources (path) VALUES (?1)", [ignored.to_string_lossy()])?;
                Ok(())
            })
            .unwrap();
        database.write(|c| add(c, &root)).unwrap();

        let scan = scan(&database).unwrap();
        let mut found = scan[0].new_paths.clone();
        found.sort();
        assert_eq!(found, [
            root.join("A/Chapter 2.cbz").to_string_lossy().into_owned(),
            root.join("C/Chapter 1").to_string_lossy().into_owned(),
        ]);
        let roots = database.read(list).unwrap();
        assert_eq!(roots[0].source_count, 1, "le fichier déjà importé est rattaché au dossier");
        assert!(roots[0].last_scanned_at.is_some());

        assert_eq!(database.write(clear_ignored).unwrap(), 1);
    }

    #[test]
    fn an_unplugged_folder_is_reported_without_losing_anything() {
        let (dir, database) = setup();
        let root = dir.path().join("Disque");
        cbz(&root.join("A/Chapter 1.cbz"));
        database.write(|c| add(c, &root)).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let scan = scan(&database).unwrap();
        assert!(!scan[0].available);
        assert!(scan[0].new_paths.is_empty());
        assert_eq!(database.read(list).unwrap()[0].status, "missing");

        database.write(|c| remove(c, &scan[0].root_id)).unwrap();
        assert!(database.read(list).unwrap().is_empty());
    }
}
