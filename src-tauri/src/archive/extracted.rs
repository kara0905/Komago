//! CB7 et CBR. Ces archives sont souvent « solides » : lire une page oblige à
//! décompresser tout ce qui la précède. On les extrait donc une fois, dans le
//! cache, sous des noms de fichiers numérotés : un nom d'entrée ne sert jamais de
//! chemin de destination.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::{is_junk, is_safe_entry_name, Entry, Limits, SourceKind};
use crate::error::{Error, Result};

/// Plafond d'une extraction : bien au-delà d'un tome, bien en deçà d'un disque.
const MAX_EXTRACTED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const INDEX: &str = "index.json";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Dossier des extractions, dans le cache de l'application (vidable sans perte).
pub fn set_root(path: PathBuf) {
    let _ = ROOT.set(path);
}

fn root() -> PathBuf {
    ROOT.get().cloned().unwrap_or_else(|| std::env::temp_dir().join("komago-extracted"))
}

#[derive(Serialize, Deserialize)]
struct Index {
    entries: Vec<(String, u64)>,
}

pub struct Extracted {
    #[cfg_attr(not(test), allow(dead_code))]
    dir: PathBuf,
    entries: Vec<Entry>,
    files: HashMap<String, PathBuf>,
}

impl Extracted {
    pub fn entries(&self) -> Vec<Entry> {
        self.entries.clone()
    }

    pub fn read(&self, name: &str, head: Option<usize>) -> Result<Vec<u8>> {
        let path = self.files.get(name).ok_or_else(|| Error::Archive(format!("entrée introuvable « {name} »")))?;
        let mut file = File::open(path).map_err(|_| Error::Archive("extraction effacée, réouverture nécessaire".into()))?;
        let mut buffer = Vec::new();
        match head {
            Some(bytes) => (&mut file).take(bytes as u64).read_to_end(&mut buffer)?,
            None => file.read_to_end(&mut buffer)?,
        };
        Ok(buffer)
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Une extraction terminée se réutilise tant que le fichier source n'a pas changé.
pub fn open(path: &Path, kind: SourceKind, limits: &Limits) -> Result<Extracted> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos());
    let key = blake3::hash(format!("{}\u{0}{}\u{0}{modified:?}", path.display(), metadata.len()).as_bytes()).to_hex();
    let dir = root().join(&key.as_str()[..32]);

    if let Some(extracted) = load(&dir) {
        return Ok(extracted);
    }
    // Nom propre à cette extraction : deux lectures simultanées du même fichier
    // (analyse et lecteur) ne doivent pas se marcher dessus.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = dir.with_extension(format!("partial-{}-{unique}", std::process::id()));
    fs::create_dir_all(&partial)?;
    let result = match kind {
        SourceKind::SevenZip => extract_7z(path, &partial, limits),
        SourceKind::Rar => extract_rar(path, &partial, limits),
        _ => Err(Error::Archive("type d'archive inattendu".into())),
    };
    let entries = match result {
        Ok(entries) => entries,
        Err(error) => {
            let _ = fs::remove_dir_all(&partial);
            return Err(error);
        }
    };
    fs::write(partial.join(INDEX), serde_json::to_vec(&Index { entries })?)?;
    if fs::rename(&partial, &dir).is_err() {
        // Une autre extraction a fini la première : on garde la sienne.
        let _ = fs::remove_dir_all(&partial);
    }
    load(&dir).ok_or_else(|| Error::Archive("extraction illisible".into()))
}

fn load(dir: &Path) -> Option<Extracted> {
    let index: Index = serde_json::from_slice(&fs::read(dir.join(INDEX)).ok()?).ok()?;
    let mut files = HashMap::new();
    let mut entries = Vec::new();
    for (number, (name, size)) in index.entries.into_iter().enumerate() {
        files.insert(name.clone(), dir.join(file_name(number)));
        entries.push(Entry { name, size });
    }
    Some(Extracted { dir: dir.to_path_buf(), entries, files })
}

fn file_name(number: usize) -> String {
    format!("{number:06}.bin")
}

/// Contrôles communs à chaque entrée, avant toute écriture.
struct Budget<'a> {
    limits: &'a Limits,
    total: u64,
    count: usize,
}

impl Budget<'_> {
    fn admit(&mut self, name: &str) -> Result<()> {
        if !is_safe_entry_name(name) {
            return Err(Error::Archive(format!("archive refusée : chemin dangereux « {name} »")));
        }
        self.count += 1;
        if self.count > self.limits.max_entries {
            return Err(Error::Archive("archive refusée : trop d'entrées".into()));
        }
        Ok(())
    }

    /// Écrit au plus la limite d'une entrée, en comptant ce qui est réellement
    /// décompressé et non ce que l'en-tête annonce.
    fn write(&mut self, reader: &mut dyn Read, target: &Path, name: &str) -> Result<u64> {
        let per_entry = self.limits.max_entry_bytes;
        let mut file = File::create(target)?;
        let written = std::io::copy(&mut reader.take(per_entry + 1), &mut file)
            .map_err(|e| Error::Archive(format!("entrée corrompue « {name} » : {e}")))?;
        if written > per_entry {
            return Err(Error::Archive(format!("archive refusée : entrée trop volumineuse « {name} »")));
        }
        self.total += written;
        if self.total > MAX_EXTRACTED_BYTES.min(self.limits.max_total_bytes) {
            return Err(Error::Archive("archive refusée : taille décompressée excessive".into()));
        }
        file.flush()?;
        Ok(written)
    }
}

fn normalize(name: &str) -> String {
    name.replace('\\', "/")
}

fn extract_7z(path: &Path, target: &Path, limits: &Limits) -> Result<Vec<(String, u64)>> {
    let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
        .map_err(|e| Error::Archive(format!("archive illisible ou corrompue : {e}")))?;
    let mut budget = Budget { limits, total: 0, count: 0 };
    let mut entries = Vec::new();
    let mut failure: Option<Error> = None;
    let outcome = reader.for_each_entries(|entry, data| {
        if entry.is_directory || !entry.has_stream {
            return Ok(true);
        }
        let name = normalize(&entry.name);
        if let Err(error) = budget.admit(&name) {
            failure = Some(error);
            return Ok(false);
        }
        let file = target.join(file_name(entries.len()));
        match budget.write(data, &file, &name) {
            Ok(size) => {
                if !is_junk(&name) {
                    entries.push((name, size));
                } else {
                    let _ = fs::remove_file(&file);
                }
                Ok(true)
            }
            Err(error) => {
                failure = Some(error);
                Ok(false)
            }
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }
    outcome.map_err(|e| Error::Archive(format!("archive illisible ou corrompue : {e}")))?;
    Ok(entries)
}

fn extract_rar(path: &Path, target: &Path, limits: &Limits) -> Result<Vec<(String, u64)>> {
    let rar_error = |e: unrar::error::UnrarError| Error::Archive(format!("archive illisible ou corrompue : {e}"));
    let mut archive = unrar::Archive::new(path).open_for_processing().map_err(rar_error)?;
    let mut budget = Budget { limits, total: 0, count: 0 };
    let mut entries = Vec::new();
    while let Some(header) = archive.read_header().map_err(rar_error)? {
        let entry = header.entry();
        if entry.is_encrypted() {
            return Err(Error::Archive("archive protégée par un mot de passe".into()));
        }
        if entry.is_directory() {
            archive = header.skip().map_err(rar_error)?;
            continue;
        }
        let name = normalize(&entry.filename.to_string_lossy());
        budget.admit(&name)?;
        if entry.unpacked_size > limits.max_entry_bytes {
            return Err(Error::Archive(format!("archive refusée : entrée trop volumineuse « {name} »")));
        }
        let (data, next) = header.read().map_err(rar_error)?;
        let file = target.join(file_name(entries.len()));
        let size = budget.write(&mut data.as_slice(), &file, &name)?;
        if is_junk(&name) {
            let _ = fs::remove_file(&file);
        } else {
            entries.push((name, size));
        }
        archive = next;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::super::{SourceReader, LIMITS};
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    #[test]
    fn a_7z_archive_lists_and_reads_its_pages() {
        let mut reader = SourceReader::open(&fixture("pages.cb7"), SourceKind::SevenZip).unwrap();
        let mut names: Vec<String> = reader.entries().unwrap().into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, ["1.png", "10.png", "2.png", "notes.txt"]);
        assert!(super::super::sniff_image(&reader.read_prefix("10.png").unwrap()).is_some());
        assert!(reader.read("absent.png").is_err());
    }

    #[test]
    fn a_7z_entry_escaping_the_folder_rejects_the_archive() {
        let result = SourceReader::open(&fixture("traversal.cb7"), SourceKind::SevenZip);
        assert!(result.is_err());
    }

    #[test]
    fn a_rar_archive_is_read_through_extraction() {
        let mut reader = SourceReader::open(&fixture("sample.rar"), SourceKind::Rar).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "VERSION");
        assert_eq!(reader.read("VERSION").unwrap().len() as u64, entries[0].size);
    }

    #[test]
    fn a_password_protected_rar_is_refused() {
        let error = SourceReader::open(&fixture("encrypted.rar"), SourceKind::Rar).err().unwrap();
        assert!(error.to_string().contains("mot de passe"), "{error}");
    }

    #[test]
    fn extraction_is_reused_and_respects_entry_limits() {
        let first = open(&fixture("pages.cb7"), SourceKind::SevenZip, &LIMITS).unwrap();
        let again = open(&fixture("pages.cb7"), SourceKind::SevenZip, &LIMITS).unwrap();
        assert_eq!(first.dir(), again.dir());

        let tiny = Limits { max_entry_bytes: 50, ..LIMITS };
        let copy = std::env::temp_dir().join(format!("komago-limit-{}.cb7", std::process::id()));
        fs::copy(fixture("pages.cb7"), &copy).unwrap();
        assert!(open(&copy, SourceKind::SevenZip, &tiny).is_err());
        let _ = fs::remove_file(copy);
    }

    #[test]
    fn signatures_reveal_the_real_archive_type() {
        assert_eq!(super::super::archive_kind(&fs::read(fixture("pages.cb7")).unwrap()), Some(SourceKind::SevenZip));
        assert_eq!(super::super::archive_kind(&fs::read(fixture("sample.rar")).unwrap()), Some(SourceKind::Rar));
        assert_eq!(super::super::archive_kind(b"PK\x03\x04...."), Some(SourceKind::Zip));
        assert_eq!(super::super::archive_kind(b"nothing"), None);
    }
}
