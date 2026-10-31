//! Lecture sécurisée des sources. Une archive est lue entrée par entrée, jamais
//! extraite : les noms d'entrées sont des clés de lecture opaques et ne servent
//! jamais de chemin de destination.

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_source_bytes: u64,
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    /// Rapport maximal taille décompressée / taille compressée d'une entrée.
    pub max_ratio: u64,
}

pub const LIMITS: Limits = Limits {
    max_source_bytes: 32 * GIB,
    max_entries: 20_000,
    max_entry_bytes: 256 * MIB,
    max_total_bytes: 64 * GIB,
    max_ratio: 200,
};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
/// Les petites entrées très compressibles ne sont pas dangereuses.
const RATIO_FREE_BYTES: u64 = MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Zip,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
}

pub enum SourceReader {
    Zip(Box<ZipArchive<BufReader<File>>>, Limits),
    Directory(PathBuf, Limits),
}

impl SourceReader {
    pub fn open(path: &Path, kind: SourceKind) -> Result<Self> {
        Self::open_with_limits(path, kind, LIMITS)
    }

    pub fn open_with_limits(path: &Path, kind: SourceKind, limits: Limits) -> Result<Self> {
        match kind {
            SourceKind::Zip => {
                let size = fs::metadata(path)?.len();
                if size > limits.max_source_bytes {
                    return Err(Error::Archive("archive trop volumineuse".into()));
                }
                let file = BufReader::new(File::open(path)?);
                let archive = ZipArchive::new(file)
                    .map_err(|e| Error::Archive(format!("archive illisible ou corrompue : {e}")))?;
                if archive.len() > limits.max_entries {
                    return Err(Error::Archive("archive refusée : trop d'entrées".into()));
                }
                Ok(Self::Zip(Box::new(archive), limits))
            }
            SourceKind::Directory => Ok(Self::Directory(path.to_path_buf(), limits)),
        }
    }

    /// Liste les fichiers utiles. Une seule entrée dangereuse fait rejeter toute
    /// l'archive : elle n'a pas pu être produite par un outil honnête.
    pub fn entries(&mut self) -> Result<Vec<Entry>> {
        match self {
            Self::Zip(archive, limits) => zip_entries(archive, limits),
            Self::Directory(root, limits) => directory_entries(root, limits),
        }
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        match self {
            Self::Zip(archive, limits) => {
                let entry = archive
                    .by_name(name)
                    .map_err(|e| Error::Archive(format!("entrée illisible « {name} » : {e}")))?;
                // La taille annoncée par l'en-tête n'est pas fiable : on plafonne
                // ce qui est réellement décompressé.
                let by_ratio = entry
                    .compressed_size()
                    .saturating_mul(limits.max_ratio)
                    .saturating_add(RATIO_FREE_BYTES);
                let limit = limits.max_entry_bytes.min(by_ratio);
                read_capped(entry, limit, name)
            }
            Self::Directory(root, limits) => {
                let path = directory_entry_path(root, name)?;
                read_capped(File::open(path)?, limits.max_entry_bytes, name)
            }
        }
    }

    /// Lit seulement le début d'une entrée, pour en vérifier la signature.
    pub fn read_prefix(&mut self, name: &str) -> Result<Vec<u8>> {
        self.read_head(name, SIGNATURE_BYTES)
    }

    /// Lit au plus `bytes` octets au début d'une entrée.
    pub fn read_head(&mut self, name: &str, bytes: usize) -> Result<Vec<u8>> {
        let mut buffer = Vec::with_capacity(bytes.min(64 * 1024));
        match self {
            Self::Zip(archive, _) => {
                let entry = archive
                    .by_name(name)
                    .map_err(|e| Error::Archive(format!("entrée illisible « {name} » : {e}")))?;
                entry
                    .take(bytes as u64)
                    .read_to_end(&mut buffer)
                    .map_err(|e| Error::Archive(format!("entrée corrompue « {name} » : {e}")))?;
            }
            Self::Directory(root, _) => {
                let path = directory_entry_path(root, name)?;
                File::open(path)?.take(bytes as u64).read_to_end(&mut buffer)?;
            }
        }
        Ok(buffer)
    }
}

fn read_capped(reader: impl Read, limit: u64, name: &str) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut buffer)
        .map_err(|e| Error::Archive(format!("entrée corrompue « {name} » : {e}")))?;
    if buffer.len() as u64 > limit {
        return Err(Error::Archive(format!("entrée refusée, trop volumineuse : « {name} »")));
    }
    Ok(buffer)
}

fn zip_entries(archive: &mut ZipArchive<BufReader<File>>, limits: &Limits) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|e| Error::Archive(format!("archive corrompue : {e}")))?;
        let name = entry
            .name()
            .map_err(|e| Error::Archive(format!("nom d'entrée illisible : {e}")))?
            .into_owned();

        if !is_safe_entry_name(&name) || entry.enclosed_name().is_none() {
            return Err(Error::Archive(format!("archive refusée : chemin dangereux « {name} »")));
        }
        if entry.is_symlink() {
            return Err(Error::Archive(format!("archive refusée : lien symbolique « {name} »")));
        }
        if entry.is_dir() || is_junk(&name) {
            continue;
        }

        let size = entry.size();
        if size > limits.max_entry_bytes {
            return Err(Error::Archive(format!("archive refusée : entrée trop volumineuse « {name} »")));
        }
        if size > RATIO_FREE_BYTES && size / entry.compressed_size().max(1) > limits.max_ratio {
            return Err(Error::Archive(format!("archive refusée : compression anormale « {name} »")));
        }
        total = total.saturating_add(size);
        if total > limits.max_total_bytes {
            return Err(Error::Archive("archive refusée : taille décompressée excessive".into()));
        }
        entries.push(Entry { name, size });
    }
    Ok(entries)
}

/// `enclosed_name` normalise les séparateurs ; cette vérification refuse en plus
/// les formes qu'il tolérerait (`a/../b`, antislashs, lecteurs Windows).
fn is_safe_entry_name(name: &str) -> bool {
    if name.is_empty() || name.contains('\0') || name.contains('\\') || name.starts_with('/') {
        return false;
    }
    if name.as_bytes().get(1) == Some(&b':') {
        return false;
    }
    name.split('/').all(|part| part != "..")
}

fn is_junk(name: &str) -> bool {
    name.split('/').any(|part| part == "__MACOSX" || part == ".git")
        || file_name(name).starts_with('.')
        || file_name(name).eq_ignore_ascii_case("thumbs.db")
        || file_name(name).eq_ignore_ascii_case("desktop.ini")
}

pub fn file_name(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

fn directory_entries(root: &Path, limits: &Limits) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        // file_type ne suit pas les liens : un lien ne fait pas sortir du dossier.
        if !item.file_type()?.is_file() {
            continue;
        }
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if is_junk(&name) {
            continue;
        }
        let size = item.metadata()?.len();
        if size > limits.max_entry_bytes {
            continue;
        }
        entries.push(Entry { name, size });
        if entries.len() > limits.max_entries {
            return Err(Error::Archive("dossier refusé : trop de fichiers".into()));
        }
    }
    Ok(entries)
}

fn directory_entry_path(root: &Path, name: &str) -> Result<PathBuf> {
    let mut components = Path::new(name).components();
    let single = matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !single {
        return Err(Error::Archive(format!("nom de fichier refusé : « {name} »")));
    }
    let path = root.join(name);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(Error::Archive(format!("nom de fichier refusé : « {name} »")));
    }
    Ok(path)
}

// Signatures ------------------------------------------------------------------

pub const SIGNATURE_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
    Avif,
    Bmp,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
            Self::Bmp => "image/bmp",
        }
    }
}

/// Le format vient du contenu, jamais de l'extension.
pub fn sniff_image(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some(ImageFormat::Png),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', ..] => Some(ImageFormat::Avif),
        [b'B', b'M', ..] => Some(ImageFormat::Bmp),
        _ => None,
    }
}

pub fn is_zip_signature(bytes: &[u8]) -> bool {
    matches!(bytes, [b'P', b'K', 0x03, 0x04, ..] | [b'P', b'K', 0x05, 0x06, ..])
}

pub fn has_image_extension(name: &str) -> bool {
    let lower = file_name(name).to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp", ".avif", ".bmp"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

#[cfg(test)]
pub mod test_support {
    use std::io::Write;
    use std::path::Path;

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// Assez pour passer la détection de signature, pas pour être décodé.
    pub const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    pub fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use tempfile::tempdir;

    fn open_zip(path: &Path) -> SourceReader {
        SourceReader::open(path, SourceKind::Zip).unwrap()
    }

    fn entries_of(entries: &[(&str, &[u8])]) -> Result<Vec<Entry>> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        write_zip(&path, entries);
        open_zip(&path).entries()
    }

    #[test]
    fn valid_archive_lists_and_reads_pages() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        write_zip(&path, &[("1.jpg", JPEG), ("sub/2.jpg", JPEG)]);

        let mut reader = open_zip(&path);
        let names: Vec<_> = reader.entries().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["1.jpg", "sub/2.jpg"]);
        assert_eq!(reader.read("sub/2.jpg").unwrap(), JPEG);
    }

    #[test]
    fn zip_slip_and_absolute_paths_reject_the_archive() {
        for name in ["../evil.jpg", "a/../../evil.jpg", "/etc/evil.jpg", "..\\evil.jpg", "C:/evil.jpg"] {
            let result = entries_of(&[("ok.jpg", JPEG), (name, JPEG)]);
            assert!(result.is_err(), "« {name} » aurait dû être refusé");
        }
    }

    #[test]
    fn too_many_entries_are_refused_before_reading() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        write_zip(&path, &[("1.jpg", JPEG), ("2.jpg", JPEG), ("3.jpg", JPEG)]);
        let limits = Limits { max_entries: 2, ..LIMITS };

        assert!(SourceReader::open_with_limits(&path, SourceKind::Zip, limits).is_err());
    }

    #[test]
    fn zip_bomb_is_refused_by_compression_ratio() {
        let bomb = vec![0u8; 8 * MIB as usize];
        let error = entries_of(&[("bomb.jpg", &bomb)]).unwrap_err();
        assert!(error.to_string().contains("compression anormale"));
    }

    #[test]
    fn oversized_entries_are_refused() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        write_zip(&path, &[("big.jpg", &[7u8; 4096])]);
        let limits = Limits { max_entry_bytes: 1024, ..LIMITS };

        let mut reader = SourceReader::open_with_limits(&path, SourceKind::Zip, limits).unwrap();
        assert!(reader.entries().is_err());
        assert!(reader.read("big.jpg").is_err());
    }

    #[test]
    fn corrupt_and_truncated_archives_return_errors() {
        let dir = tempdir().unwrap();
        let garbage = dir.path().join("garbage.cbz");
        fs::write(&garbage, b"PK\x03\x04 pas vraiment un zip").unwrap();
        assert!(SourceReader::open(&garbage, SourceKind::Zip).is_err());

        let valid = dir.path().join("valid.cbz");
        write_zip(&valid, &[("1.jpg", &[9u8; 2048])]);
        let bytes = fs::read(&valid).unwrap();
        let truncated = dir.path().join("truncated.cbz");
        fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
        assert!(SourceReader::open(&truncated, SourceKind::Zip).is_err());
    }

    #[test]
    fn corrupt_entry_data_fails_on_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.cbz");
        let content: Vec<u8> = (0..4096u32).map(|i| (i * 31 % 251) as u8).collect();
        write_zip(&path, &[("1.jpg", &content)]);
        let mut bytes = fs::read(&path).unwrap();
        for byte in &mut bytes[60..200] {
            *byte ^= 0xFF;
        }
        fs::write(&path, bytes).unwrap();

        let mut reader = open_zip(&path);
        assert!(reader.read("1.jpg").is_err());
    }

    #[test]
    fn os_junk_is_ignored() {
        let entries = entries_of(&[
            ("1.jpg", JPEG),
            ("__MACOSX/._1.jpg", JPEG),
            (".DS_Store", b"x"),
            ("Thumbs.db", b"x"),
        ])
        .unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn signatures_ignore_extensions() {
        assert_eq!(sniff_image(JPEG), Some(ImageFormat::Jpeg));
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n...."), Some(ImageFormat::Png));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(sniff_image(b"\0\0\0\x20ftypavif"), Some(ImageFormat::Avif));
        assert_eq!(sniff_image(b"MZ executable"), None);
    }

    #[test]
    fn directory_reader_refuses_traversal_and_symlinks() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("1.jpg"), JPEG).unwrap();
        fs::write(dir.path().join("secret.jpg"), JPEG).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("secret.jpg"), root.join("link.jpg")).unwrap();

        let mut reader = SourceReader::open(&root, SourceKind::Directory).unwrap();
        let names: Vec<_> = reader.entries().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["1.jpg"]);
        assert_eq!(reader.read("1.jpg").unwrap(), JPEG);
        assert!(reader.read("../secret.jpg").is_err());
        #[cfg(unix)]
        assert!(reader.read("link.jpg").is_err());
    }
}
