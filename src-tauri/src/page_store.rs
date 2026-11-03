//! Accès rapide aux pages : un cache mémoire borné et quelques archives gardées
//! ouvertes, pour ne pas relire l'index d'un ZIP à chaque page.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::archive::{SourceKind, SourceReader};
use crate::error::Result;

const CACHE_BYTES: usize = 384 * 1024 * 1024;
const OPEN_SOURCES: usize = 3;

/// Cache LRU limité en octets, jamais en nombre de pages : un chapitre de webtoon
/// en très haute définition ne peut pas faire exploser la mémoire.
pub struct PageCache<K> {
    budget: usize,
    used: usize,
    entries: HashMap<K, Arc<Vec<u8>>>,
    /// Du moins récent au plus récent.
    order: VecDeque<K>,
}

impl<K: Clone + Eq + std::hash::Hash> PageCache<K> {
    pub fn new(budget: usize) -> Self {
        Self { budget, used: 0, entries: HashMap::new(), order: VecDeque::new() }
    }

    pub fn get(&mut self, key: &K) -> Option<Arc<Vec<u8>>> {
        let value = self.entries.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    pub fn insert(&mut self, key: K, value: Arc<Vec<u8>>) {
        if value.len() > self.budget {
            return;
        }
        if let Some(previous) = self.entries.insert(key.clone(), value.clone()) {
            self.used -= previous.len();
            self.touch(&key);
        } else {
            self.order.push_back(key);
        }
        self.used += value.len();
        while self.used > self.budget {
            let Some(oldest) = self.order.pop_front() else { break };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.used -= evicted.len();
            }
        }
    }

    fn touch(&mut self, key: &K) {
        if let Some(position) = self.order.iter().position(|k| k == key) {
            if let Some(moved) = self.order.remove(position) {
                self.order.push_back(moved);
            }
        }
    }

    #[cfg(test)]
    fn used(&self) -> usize {
        self.used
    }
}

struct OpenSource {
    path: PathBuf,
    /// Si le fichier change sur le disque, l'index gardé en mémoire n'est plus valable.
    signature: Option<(u64, SystemTime)>,
    reader: SourceReader,
}

fn signature(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

pub struct PageStore {
    cache: Mutex<PageCache<(String, i64)>>,
    sources: Mutex<VecDeque<OpenSource>>,
}

impl PageStore {
    pub fn new() -> Self {
        Self { cache: Mutex::new(PageCache::new(CACHE_BYTES)), sources: Mutex::new(VecDeque::new()) }
    }

    pub fn page(
        &self,
        chapter_id: &str,
        index: i64,
        path: &Path,
        kind: SourceKind,
        entry: &str,
    ) -> Result<Arc<Vec<u8>>> {
        let key = (chapter_id.to_string(), index);
        if let Some(found) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Ok(found);
        }
        let bytes = Arc::new(self.read_entry(path, kind, entry)?);
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, bytes.clone());
        Ok(bytes)
    }

    fn read_entry(&self, path: &Path, kind: SourceKind, entry: &str) -> Result<Vec<u8>> {
        let current = signature(path);
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        let position = sources.iter().position(|s| s.path == path && s.signature == current);
        let mut source = match position.and_then(|p| sources.remove(p)) {
            Some(source) => source,
            None => OpenSource {
                path: path.to_path_buf(),
                signature: current,
                reader: SourceReader::open(path, kind)?,
            },
        };
        let mut bytes = source.reader.read(entry);
        // Une extraction effacée avec le cache : on rouvre une fois la source.
        if bytes.is_err() {
            if let Ok(reader) = SourceReader::open(path, kind) {
                source.reader = reader;
                bytes = source.reader.read(entry);
            }
        }
        sources.push_back(source);
        if sources.len() > OPEN_SOURCES {
            sources.pop_front();
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::test_support::{write_zip, JPEG};
    use tempfile::tempdir;

    fn bytes(size: usize) -> Arc<Vec<u8>> {
        Arc::new(vec![0; size])
    }

    #[test]
    fn evicts_the_least_recently_used_within_budget() {
        let mut cache = PageCache::new(100);
        cache.insert("a", bytes(40));
        cache.insert("b", bytes(40));
        assert!(cache.get(&"a").is_some(), "a redevient récent");
        cache.insert("c", bytes(40));

        assert!(cache.get(&"b").is_none());
        assert!(cache.get(&"a").is_some());
        assert!(cache.get(&"c").is_some());
        assert_eq!(cache.used(), 80);
    }

    #[test]
    fn an_oversized_page_is_not_cached_and_evicts_nothing() {
        let mut cache = PageCache::new(100);
        cache.insert("a", bytes(50));
        cache.insert("huge", bytes(101));
        assert!(cache.get(&"huge").is_none());
        assert!(cache.get(&"a").is_some());
    }

    #[test]
    fn replacing_an_entry_keeps_the_accounting_right() {
        let mut cache = PageCache::new(100);
        cache.insert("a", bytes(50));
        cache.insert("a", bytes(10));
        assert_eq!(cache.used(), 10);
    }

    #[test]
    fn reads_pages_and_serves_them_from_cache_once_the_file_is_gone() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        write_zip(&path, &[("1.jpg", JPEG)]);
        let store = PageStore::new();

        let first = store.page("c", 0, &path, SourceKind::Zip, "1.jpg").unwrap();
        assert_eq!(first.as_slice(), JPEG);

        fs::remove_file(&path).unwrap();
        assert!(store.page("c", 0, &path, SourceKind::Zip, "1.jpg").is_ok());
        assert!(store.page("c", 1, &path, SourceKind::Zip, "1.jpg").is_err());
    }

    #[test]
    fn a_replaced_file_is_reopened() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        write_zip(&path, &[("1.jpg", JPEG)]);
        let store = PageStore::new();
        store.page("c", 0, &path, SourceKind::Zip, "1.jpg").unwrap();

        write_zip(&path, &[("1.jpg", JPEG), ("nouvelle-page-qui-change-la-taille.jpg", JPEG)]);
        let page = store.page("c", 1, &path, SourceKind::Zip, "nouvelle-page-qui-change-la-taille.jpg");
        assert!(page.is_ok());
    }
}
