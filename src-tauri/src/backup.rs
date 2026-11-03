//! Sauvegarde et restauration de la bibliothèque. La restauration est préparée et
//! vérifiée à part, puis appliquée au démarrage suivant, avant l'ouverture de la
//! base : on ne remplace jamais une base ouverte. L'ancienne est conservée.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::archive::{self, Limits, SourceKind, SourceReader, LIMITS};
use crate::db::{latest_version, Database};
use crate::error::{Error, Result};
use crate::storage::Storage;

const MANIFEST: &str = "komago-backup.json";
const DATABASE: &str = "komago.db";
const PENDING: &str = "restore-pending";
const READY: &str = "READY";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub app_version: String,
    pub database_version: i64,
    pub created_at: String,
    pub includes_library: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedRestore {
    pub manifest: Manifest,
    pub series_count: i64,
    pub chapter_count: i64,
}

fn now(connection: &Connection) -> Result<String> {
    Ok(connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |r| r.get(0))?)
}

/// Écrit une archive ZIP : base cohérente (VACUUM INTO), couvertures et, si
/// demandé, les fichiers copiés dans la bibliothèque Komago.
pub fn export(database: &Database, storage: &Storage, destination: &Path, include_library: bool) -> Result<Manifest> {
    let snapshot = storage.cache_dir().join("backup-snapshot.db");
    let _ = fs::remove_file(&snapshot);
    let manifest = database.read(|connection| {
        connection.execute("VACUUM INTO ?1", [snapshot.to_string_lossy()])?;
        Ok(Manifest {
            app_version: env!("CARGO_PKG_VERSION").into(),
            database_version: connection.query_row("PRAGMA user_version", [], |r| r.get(0))?,
            created_at: now(connection)?,
            includes_library: include_library,
        })
    })?;

    // Écriture à côté, puis renommage : une sauvegarde interrompue ne remplace rien.
    let partial = destination.with_extension("partial");
    let written = (|| -> Result<()> {
        let mut zip = ZipWriter::new(File::create(&partial)?);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).large_file(true);
        let zip_error = |e: zip::result::ZipError| Error::Archive(format!("sauvegarde impossible : {e}"));

        zip.start_file(MANIFEST, deflated).map_err(zip_error)?;
        zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        zip.start_file(DATABASE, deflated.large_file(true)).map_err(zip_error)?;
        std::io::copy(&mut File::open(&snapshot)?, &mut zip)?;
        add_directory(&mut zip, &storage.covers_dir(), "covers", stored)?;
        if include_library {
            add_directory(&mut zip, &storage.library_dir(), "library", stored)?;
        }
        zip.finish().map_err(zip_error)?;
        Ok(())
    })();
    let _ = fs::remove_file(&snapshot);
    if let Err(error) = written {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, destination)?;
    Ok(manifest)
}

fn add_directory(zip: &mut ZipWriter<File>, dir: &Path, prefix: &str, options: SimpleFileOptions) -> Result<()> {
    let mut pending = vec![(dir.to_path_buf(), prefix.to_string())];
    while let Some((current, name)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let file_type = entry.file_type()?;
            let Some(file_name) = entry.file_name().to_str().map(str::to_string) else { continue };
            let entry_name = format!("{name}/{file_name}");
            if file_type.is_dir() {
                pending.push((entry.path(), entry_name));
            } else if file_type.is_file() {
                zip.start_file(&entry_name, options)
                    .map_err(|e| Error::Archive(format!("sauvegarde impossible : {e}")))?;
                std::io::copy(&mut File::open(entry.path())?, zip)?;
            }
        }
    }
    Ok(())
}

/// Vérifie l'archive et la prépare. Rien de la bibliothèque actuelle ne change.
pub fn stage(storage: &Storage, archive_path: &Path) -> Result<StagedRestore> {
    let pending = storage.data_dir().join(PENDING);
    let _ = fs::remove_dir_all(&pending);
    fs::create_dir_all(&pending)?;
    let result = stage_into(&pending, archive_path);
    if result.is_err() {
        let _ = fs::remove_dir_all(&pending);
    }
    result
}

fn stage_into(pending: &Path, archive_path: &Path) -> Result<StagedRestore> {
    // Les copies de la bibliothèque peuvent être de grosses archives.
    let limits = Limits { max_entry_bytes: 16 * 1024 * 1024 * 1024, max_total_bytes: 512 * 1024 * 1024 * 1024, ..LIMITS };
    let mut reader = SourceReader::open_with_limits(archive_path, SourceKind::Zip, limits)
        .map_err(|_| Error::Invalid("ce fichier n'est pas une sauvegarde Komago".into()))?;
    let entries = reader.entries()?;
    if !entries.iter().any(|e| e.name == MANIFEST) || !entries.iter().any(|e| e.name == DATABASE) {
        return Err(Error::Invalid("ce fichier n'est pas une sauvegarde Komago".into()));
    }
    let manifest: Manifest = serde_json::from_slice(&reader.read(MANIFEST)?)
        .map_err(|_| Error::Invalid("sauvegarde illisible".into()))?;
    if manifest.database_version > latest_version() {
        return Err(Error::Invalid("cette sauvegarde vient d'une version plus récente de Komago".into()));
    }

    let database = pending.join(DATABASE);
    fs::write(&database, reader.read(DATABASE)?)?;
    let (series_count, chapter_count) = {
        let connection = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let check: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(Error::Invalid("la base de la sauvegarde est endommagée".into()));
        }
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > latest_version() || version < 1 {
            return Err(Error::Invalid("version de base inattendue dans la sauvegarde".into()));
        }
        let count = |table: &str| -> Result<i64> {
            Ok(connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
        };
        (count("series")?, count("chapters")?)
    };

    // Les noms d'entrées ont été validés par le lecteur d'archives ; on n'accepte en
    // plus que les deux dossiers attendus, et des couvertures qui sont des images.
    for entry in &entries {
        let Some((area, rest)) = entry.name.split_once('/') else { continue };
        if rest.is_empty() || !matches!(area, "covers" | "library") {
            continue;
        }
        if area == "covers" && (rest.contains('/') || archive::sniff_image(&reader.read_prefix(&entry.name)?).is_none()) {
            continue;
        }
        let target = pending.join(area).join(rest);
        fs::create_dir_all(target.parent().unwrap_or(pending))?;
        fs::write(&target, reader.read(&entry.name)?)?;
    }
    fs::write(pending.join(READY), b"")?;
    Ok(StagedRestore { manifest, series_count, chapter_count })
}

pub fn discard(storage: &Storage) {
    let _ = fs::remove_dir_all(storage.data_dir().join(PENDING));
}

/// Appelé au démarrage, avant l'ouverture de la base. Renvoie le dossier où la
/// bibliothèque précédente a été mise de côté.
pub fn apply_pending(data_dir: &Path) -> Result<Option<PathBuf>> {
    let pending = data_dir.join(PENDING);
    if !pending.join(READY).exists() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let kept = data_dir.join("backups").join(format!("avant-restauration-{stamp}"));
    fs::create_dir_all(&kept)?;

    for suffix in ["", "-wal", "-shm"] {
        let name = format!("{DATABASE}{suffix}");
        if data_dir.join(&name).exists() {
            fs::rename(data_dir.join(&name), kept.join(&name))?;
        }
    }
    fs::rename(pending.join(DATABASE), data_dir.join(DATABASE))?;
    for area in ["covers", "library"] {
        if pending.join(area).exists() {
            if data_dir.join(area).exists() {
                fs::rename(data_dir.join(area), kept.join(area))?;
            }
            fs::rename(pending.join(area), data_dir.join(area))?;
        }
    }
    fs::remove_dir_all(&pending)?;
    Ok(Some(kept))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::test_support::write_zip;
    use crate::covers::test_support::png;
    use tempfile::{tempdir, TempDir};

    fn library() -> (TempDir, Storage, Database) {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().join("data"), dir.path().join("cache")).unwrap();
        let database = Database::open(&storage.database_path()).unwrap();
        (dir, storage, database)
    }

    fn series_count(database: &Database) -> i64 {
        database.read(|c| Ok(c.query_row("SELECT count(*) FROM series", [], |r| r.get(0))?)).unwrap()
    }

    #[test]
    fn a_backup_restores_after_restart_and_keeps_the_previous_library() {
        let (dir, storage, database) = library();
        database
            .write(|c| {
                c.execute("INSERT INTO series (id, title, sort_title) VALUES ('s', 'Sauvée', 'sauvee')", [])?;
                Ok(())
            })
            .unwrap();
        fs::write(storage.covers_dir().join("c.png"), png(4, 4)).unwrap();
        fs::write(storage.library_dir().join("copie.cbz"), b"copie").unwrap();
        let backup = dir.path().join("komago.zip");
        export(&database, &storage, &backup, true).unwrap();

        // La bibliothèque change après la sauvegarde.
        database
            .write(|c| {
                c.execute("INSERT INTO series (id, title, sort_title) VALUES ('n', 'Nouvelle', 'nouvelle')", [])?;
                Ok(())
            })
            .unwrap();
        let staged = stage(&storage, &backup).unwrap();
        assert_eq!(staged.series_count, 1);
        assert!(staged.manifest.includes_library);
        assert_eq!(series_count(&database), 2, "rien ne change avant le redémarrage");
        drop(database);

        let kept = apply_pending(storage.data_dir()).unwrap().unwrap();
        let reopened = Database::open(&storage.database_path()).unwrap();
        assert_eq!(series_count(&reopened), 1);
        assert!(storage.covers_dir().join("c.png").exists());
        assert!(storage.library_dir().join("copie.cbz").exists());
        assert!(kept.join(DATABASE).exists(), "l'ancienne base est conservée");
        assert!(apply_pending(storage.data_dir()).unwrap().is_none(), "une seule application");
    }

    #[test]
    fn foreign_or_damaged_files_are_refused_and_leave_nothing() {
        let (dir, storage, _database) = library();
        let foreign = dir.path().join("autre.zip");
        write_zip(&foreign, &[("photo.jpg", b"x")]);
        assert!(stage(&storage, &foreign).is_err());

        let damaged = dir.path().join("abimee.zip");
        let manifest = serde_json::to_vec(&Manifest {
            app_version: "0".into(),
            database_version: 1,
            created_at: "x".into(),
            includes_library: false,
        })
        .unwrap();
        write_zip(&damaged, &[(MANIFEST, &manifest), (DATABASE, b"pas une base sqlite")]);
        assert!(stage(&storage, &damaged).is_err());

        let future = dir.path().join("future.zip");
        let manifest = serde_json::to_vec(&Manifest {
            app_version: "99".into(),
            database_version: 999,
            created_at: "x".into(),
            includes_library: false,
        })
        .unwrap();
        write_zip(&future, &[(MANIFEST, &manifest), (DATABASE, b"x")]);
        assert!(stage(&storage, &future).is_err());

        assert!(!storage.data_dir().join(PENDING).exists());
        assert!(apply_pending(storage.data_dir()).unwrap().is_none());
    }

    #[test]
    fn dangerous_paths_in_a_backup_are_refused() {
        let (dir, storage, database) = library();
        let backup = dir.path().join("komago.zip");
        export(&database, &storage, &backup, false).unwrap();
        let mut reader = SourceReader::open(&backup, SourceKind::Zip).unwrap();
        let manifest = reader.read(MANIFEST).unwrap();
        let db = reader.read(DATABASE).unwrap();
        let evil = dir.path().join("evil.zip");
        write_zip(&evil, &[(MANIFEST, &manifest), (DATABASE, &db), ("covers/../../evil.png", &png(2, 2))]);
        assert!(stage(&storage, &evil).is_err());
        assert!(!dir.path().join("evil.png").exists());
    }
}
