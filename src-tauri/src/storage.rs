use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;

/// Répertoires de Komago. Ce qui est dans `data` ne se régénère pas ; tout ce
/// qui est dans `cache` peut être effacé sans perte.
#[derive(Debug, Clone)]
pub struct Storage {
    data: PathBuf,
    cache: PathBuf,
}

/// Sous-dossiers de cache, seuls emplacements que `clear_cache` peut vider.
const CACHE_DIRS: [&str; 3] = ["thumbnails", "pages", "extracted"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub database_bytes: u64,
    pub covers_bytes: u64,
    pub library_bytes: u64,
    pub cache_bytes: u64,
}

impl Storage {
    pub fn new(data: PathBuf, cache: PathBuf) -> Result<Self> {
        let storage = Self { data, cache };
        fs::create_dir_all(storage.covers_dir())?;
        fs::create_dir_all(storage.library_dir())?;
        for dir in CACHE_DIRS {
            fs::create_dir_all(storage.cache.join(dir))?;
        }
        Ok(storage)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    pub fn database_path(&self) -> PathBuf {
        self.data.join("komago.db")
    }

    pub fn covers_dir(&self) -> PathBuf {
        self.data.join("covers")
    }

    /// Originaux copiés à la demande de l'utilisateur.
    pub fn library_dir(&self) -> PathBuf {
        self.data.join("library")
    }

    pub fn info(&self) -> Result<StorageInfo> {
        let mut database_bytes = 0;
        for entry in fs::read_dir(&self.data)? {
            let entry = entry?;
            let is_database = entry.file_name().to_string_lossy().starts_with("komago.db");
            if is_database && entry.file_type()?.is_file() {
                database_bytes += entry.metadata()?.len();
            }
        }
        // Seuls nos sous-dossiers comptent : le webview dépose aussi ses propres
        // fichiers dans le répertoire de cache, et "Vider" n'y touche pas.
        let mut cache_bytes = 0;
        for dir in CACHE_DIRS {
            cache_bytes += directory_size(&self.cache.join(dir))?;
        }
        Ok(StorageInfo {
            data_dir: self.data.clone(),
            cache_dir: self.cache.clone(),
            database_bytes,
            covers_bytes: directory_size(&self.covers_dir())?,
            library_bytes: directory_size(&self.library_dir())?,
            cache_bytes,
        })
    }

    pub fn clear_cache(&self) -> Result<()> {
        for dir in CACHE_DIRS {
            let path = self.cache.join(dir);
            match fs::remove_dir_all(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            fs::create_dir_all(path)?;
        }
        Ok(())
    }
}

/// Les liens symboliques ne sont pas suivis : ils ne sortent pas du répertoire.
fn directory_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    fn storage() -> (TempDir, Storage) {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().join("data"), dir.path().join("cache")).unwrap();
        (dir, storage)
    }

    #[test]
    fn creates_separate_directories() {
        let (dir, storage) = storage();
        assert!(storage.covers_dir().is_dir());
        assert!(storage.library_dir().is_dir());
        for sub in CACHE_DIRS {
            assert!(dir.path().join("cache").join(sub).is_dir());
        }
    }

    #[test]
    fn info_sums_each_area_separately() {
        let (dir, storage) = storage();
        fs::write(storage.database_path(), [0; 10]).unwrap();
        fs::write(storage.covers_dir().join("a.jpg"), [0; 20]).unwrap();
        fs::create_dir(storage.library_dir().join("nested")).unwrap();
        fs::write(storage.library_dir().join("nested/b.cbz"), [0; 30]).unwrap();
        fs::write(dir.path().join("cache/pages/p.bin"), [0; 40]).unwrap();
        fs::write(dir.path().join("cache/from-the-webview"), [0; 50]).unwrap();

        let info = storage.info().unwrap();
        assert_eq!(info.database_bytes, 10);
        assert_eq!(info.covers_bytes, 20);
        assert_eq!(info.library_bytes, 30);
        assert_eq!(info.cache_bytes, 40);
    }

    #[test]
    fn clear_cache_only_touches_the_cache() {
        let (dir, storage) = storage();
        let cover = storage.covers_dir().join("a.jpg");
        let original = storage.library_dir().join("b.cbz");
        fs::write(&cover, [1]).unwrap();
        fs::write(&original, [1]).unwrap();
        fs::write(storage.database_path(), [1]).unwrap();
        fs::write(dir.path().join("cache/pages/p.bin"), [1]).unwrap();
        fs::write(dir.path().join("cache/thumbnails/t.webp"), [1]).unwrap();

        storage.clear_cache().unwrap();

        assert!(cover.exists());
        assert!(original.exists());
        assert!(storage.database_path().exists());
        assert_eq!(storage.info().unwrap().cache_bytes, 0);
        assert!(dir.path().join("cache/pages").is_dir());
    }
}
