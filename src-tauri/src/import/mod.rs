//! Import en deux temps : `analyze` lit les sources sans rien écrire et produit
//! un plan ; `commit` applique le plan corrigé par l'utilisateur. Les chemins et
//! les pages restent côté Rust : le frontend ne renvoie que des corrections.

pub mod names;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::archive::{self, Entry, SourceKind, SourceReader};
use crate::comicinfo::{self, ComicInfo};
use crate::covers;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::storage::Storage;
use crate::text::{natural_cmp, normalize_title, sort_title};
use names::{clean, parse_name, ParsedName};

const MAX_DEPTH: usize = 8;
const MAX_UNITS: usize = 100_000;
const HASH_SAMPLE_BYTES: u64 = 64 * 1024;

// Plan envoyé au frontend ------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub session_id: String,
    pub series: Vec<PlanSeries>,
    pub issues: Vec<PlanIssue>,
    pub relocated: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSeries {
    pub key: String,
    pub title: String,
    pub existing_series_id: Option<String>,
    pub cover_chapter_id: Option<String>,
    pub chapters: Vec<PlanChapter>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanChapter {
    pub id: String,
    pub display_name: String,
    pub source_path: String,
    pub volume: Option<f64>,
    pub number: Option<f64>,
    pub title: Option<String>,
    pub page_count: usize,
    pub duplicate: bool,
    pub has_comic_info: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEdits {
    pub series: Vec<SeriesEdit>,
    pub chapters: Vec<ChapterEdit>,
    #[serde(default)]
    pub copy_files: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesEdit {
    pub key: String,
    pub title: String,
    pub existing_series_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterEdit {
    pub id: String,
    pub series_key: String,
    pub volume: Option<f64>,
    pub number: Option<f64>,
    pub title: Option<String>,
    pub include: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub series_created: usize,
    pub chapters_added: usize,
    pub relocated: usize,
    pub covers_failed: usize,
    pub first_series_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeProgress {
    pub done: usize,
    pub total: usize,
}

// État interne d'une session ----------------------------------------------------

#[derive(Debug)]
pub struct ImportSession {
    pub id: String,
    sources: Vec<DraftSource>,
    chapters: HashMap<String, DraftChapter>,
    relocations: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
struct DraftSource {
    path: PathBuf,
    kind: SourceKind,
    db_kind: &'static str,
    size: u64,
    modified_at: Option<String>,
    partial_hash: String,
}

#[derive(Debug, Clone)]
struct DraftChapter {
    source: usize,
    subpath: String,
    pages: Vec<Entry>,
    comic_info: Option<ComicInfo>,
}

impl ImportSession {
    /// Source et entrée de la page à montrer en aperçu.
    pub fn cover_page(&self, chapter_id: &str) -> Option<(PathBuf, SourceKind, String)> {
        let chapter = self.chapters.get(chapter_id)?;
        let source = self.sources.get(chapter.source)?;
        let page = cover_page(&chapter.pages)?;
        Some((source.path.clone(), source.kind, page.name.clone()))
    }
}

/// Une page nommée « cover » est préférée à la première page.
fn cover_page(pages: &[Entry]) -> Option<&Entry> {
    pages
        .iter()
        .find(|page| {
            let lower = archive::file_name(&page.name).to_lowercase();
            lower.starts_with("cover") || lower.starts_with("couverture")
        })
        .or_else(|| pages.first())
}

// Analyse -----------------------------------------------------------------------

/// Élément à examiner, avec les dossiers qui le précèdent depuis la sélection.
#[derive(Debug)]
struct Unit {
    path: PathBuf,
    kind: UnitKind,
    /// Du plus proche au plus lointain ; contient le dossier sélectionné.
    ancestors: Vec<String>,
    /// L'élément est directement dans le dossier sélectionné.
    at_selection_root: bool,
}

#[derive(Debug)]
enum UnitKind {
    Archive,
    Directory,
    LooseImages(Vec<String>),
}

/// Chaque requête prend le verrou séparément : parcourir un gros dossier ne doit
/// pas bloquer la bibliothèque.
pub fn analyze(
    paths: &[PathBuf],
    database: &Database,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(AnalyzeProgress),
) -> Result<(ImportSession, ImportPlan)> {
    let mut issues = Vec::new();
    let units = collect_units(paths, &mut issues);
    let total = units.len();

    let mut sources = Vec::new();
    let mut chapters = HashMap::new();
    let mut relocations = Vec::new();
    let mut groups: BTreeMap<String, PlanSeries> = BTreeMap::new();
    let existing = database.read(existing_series)?;

    for (done, unit) in units.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        on_progress(AnalyzeProgress { done, total });
        match read_unit(unit) {
            Ok(read) => {
                issues.extend(read.warnings.iter().map(|message| PlanIssue {
                    path: unit.path.display().to_string(),
                    message: message.clone(),
                }));
                let source_index = sources.len();
                let relocation = database.read(|c| find_relocation(c, &read.source))?;
                if let Some(source_id) = &relocation {
                    relocations.push((source_id.clone(), source_index));
                }
                let known = database.read(|c| known_subpaths(c, &read.source))?;
                for chapter in read.chapters {
                    let duplicate = relocation.is_some() || known.contains(&chapter.subpath);
                    add_chapter(unit, chapter, source_index, &read.source, duplicate, &existing, &mut groups, &mut chapters);
                }
                sources.push(read.source);
            }
            Err(error) => issues.push(PlanIssue {
                path: unit.path.display().to_string(),
                message: error.to_string(),
            }),
        }
    }
    on_progress(AnalyzeProgress { done: total, total });

    let mut series: Vec<PlanSeries> = groups.into_values().collect();
    for group in &mut series {
        group.chapters.sort_by(compare_plan_chapters);
        group.cover_chapter_id = group.chapters.first().map(|c| c.id.clone());
    }
    series.sort_by(|a, b| natural_cmp(&a.title, &b.title));

    let id = Uuid::now_v7().to_string();
    let relocated = relocations.len();
    let plan = ImportPlan { session_id: id.clone(), series, issues, relocated };
    Ok((ImportSession { id, sources, chapters, relocations }, plan))
}

fn compare_plan_chapters(a: &PlanChapter, b: &PlanChapter) -> std::cmp::Ordering {
    let key = |c: &PlanChapter| (c.volume.unwrap_or(f64::MAX), c.number.unwrap_or(f64::MAX));
    let (a_key, b_key) = (key(a), key(b));
    a_key.0.total_cmp(&b_key.0)
        .then(a_key.1.total_cmp(&b_key.1))
        .then_with(|| natural_cmp(&a.display_name, &b.display_name))
}

#[allow(clippy::too_many_arguments)]
fn add_chapter(
    unit: &Unit,
    read: ReadChapter,
    source: usize,
    draft_source: &DraftSource,
    duplicate: bool,
    existing: &HashMap<String, String>,
    groups: &mut BTreeMap<String, PlanSeries>,
    chapters: &mut HashMap<String, DraftChapter>,
) {
    let inferred = infer(unit, &read);
    let key = normalize_title(&inferred.series);
    let key = if key.is_empty() { inferred.series.clone() } else { key };
    let id = Uuid::now_v7().to_string();
    let info = read.comic_info.as_ref();

    let plan_chapter = PlanChapter {
        id: id.clone(),
        display_name: read.display_name,
        source_path: draft_source.path.display().to_string(),
        volume: info.and_then(|i| i.volume).or(inferred.volume),
        number: info.and_then(|i| i.number).or(inferred.number),
        title: info.and_then(|i| i.title.clone()).or(inferred.title),
        page_count: read.pages.len(),
        duplicate,
        has_comic_info: info.is_some(),
    };
    groups
        .entry(key.clone())
        .or_insert_with(|| PlanSeries {
            existing_series_id: existing.get(&key).cloned(),
            key: key.clone(),
            title: inferred.series.clone(),
            cover_chapter_id: None,
            chapters: Vec::new(),
        })
        .chapters
        .push(plan_chapter);
    chapters.insert(id, DraftChapter { source, subpath: read.subpath, pages: read.pages, comic_info: read.comic_info });
}

struct Inferred {
    series: String,
    volume: Option<f64>,
    number: Option<f64>,
    title: Option<String>,
}

/// Priorité : ComicInfo (appliqué par l'appelant), puis structure des dossiers,
/// puis nom du fichier.
fn infer(unit: &Unit, read: &ReadChapter) -> Inferred {
    let own: ParsedName = read.parsed.clone();
    let mut volume = own.volume;
    let mut ancestor_title = None;
    let mut volume_dir_hint = None;

    for ancestor in &unit.ancestors {
        let parsed = parse_name(ancestor);
        if parsed.is_volume_only() {
            volume = volume.or(parsed.volume);
            volume_dir_hint = volume_dir_hint.or(parsed.series_hint);
            continue;
        }
        ancestor_title = Some(clean(ancestor));
        break;
    }

    let series = read
        .comic_info
        .as_ref()
        .and_then(|info| info.series.clone())
        // À la racine de la sélection, le nom du fichier est plus précis que celui
        // du dossier : « Téléchargements/One Piece 001.cbz ».
        .or_else(|| if unit.at_selection_root { own.series_hint.clone() } else { None })
        .or(volume_dir_hint)
        .or(ancestor_title)
        .or_else(|| own.series_hint.clone())
        .unwrap_or_else(|| clean(&read.unit_name));

    // Un nom sans numéro qui a servi de titre de série ne devient pas aussi le
    // titre du chapitre.
    Inferred { series, volume, number: own.chapter, title: own.title }
}

fn collect_units(paths: &[PathBuf], issues: &mut Vec<PlanIssue>) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut loose: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();

    for path in paths {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            issues.push(PlanIssue { path: path.display().to_string(), message: "élément introuvable".into() });
            continue;
        };
        if metadata.is_dir() {
            let root_name = base_name(path);
            walk(path, &selection_context(path), &root_name, 0, &mut units, issues);
        } else if metadata.is_file() {
            let name = base_name(path);
            if archive::has_image_extension(&name) {
                if let Some(parent) = path.parent() {
                    loose.entry(parent.to_path_buf()).or_default().push(name);
                }
            } else {
                classify_file(path, selection_context(path), false, &mut units, issues);
            }
        }
    }
    for (parent, names) in loose {
        units.push(Unit {
            ancestors: selection_context(&parent),
            path: parent,
            kind: UnitKind::LooseImages(names),
            at_selection_root: false,
        });
    }
    units
}

/// Un élément sélectionné dont le nom ne dit rien de la série (« Chapitre 001 »,
/// « Volume 2 ») appartient à la série de son dossier parent.
fn selection_context(path: &Path) -> Vec<String> {
    let name = if path.is_dir() { base_name(path) } else { stem(path) };
    if parse_name(&name).series_hint.is_some() {
        return Vec::new();
    }
    path.parent().filter(|p| p.file_name().is_some()).map(|p| vec![base_name(p)]).unwrap_or_default()
}

fn walk(dir: &Path, parents: &[String], root_name: &str, depth: usize, units: &mut Vec<Unit>, issues: &mut Vec<PlanIssue>) {
    if depth > MAX_DEPTH || units.len() >= MAX_UNITS {
        issues.push(PlanIssue { path: dir.display().to_string(), message: "dossier ignoré : arborescence trop profonde".into() });
        return;
    }
    let Ok(listing) = fs::read_dir(dir) else {
        issues.push(PlanIssue { path: dir.display().to_string(), message: "dossier illisible".into() });
        return;
    };
    let mut items: Vec<_> = listing.flatten().collect();
    items.sort_by(|a, b| natural_cmp(&a.file_name().to_string_lossy(), &b.file_name().to_string_lossy()));

    // Ancêtres de ce qui est dans `dir`, du plus proche au plus lointain.
    let mut ancestors = vec![base_name(dir)];
    ancestors.extend_from_slice(parents);

    let mut has_images = false;
    for item in items {
        let Ok(file_type) = item.file_type() else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if file_type.is_dir() {
            walk(&item.path(), &ancestors, root_name, depth + 1, units, issues);
        } else if file_type.is_file() {
            if archive::has_image_extension(&name) {
                has_images = true;
            } else {
                classify_file(&item.path(), ancestors.clone(), depth == 0, units, issues);
            }
        }
    }
    if has_images && units.len() < MAX_UNITS {
        units.push(Unit {
            path: dir.to_path_buf(),
            kind: UnitKind::Directory,
            ancestors: parents.to_vec(),
            at_selection_root: false,
        });
    }
}

fn classify_file(path: &Path, ancestors: Vec<String>, at_selection_root: bool, units: &mut Vec<Unit>, issues: &mut Vec<PlanIssue>) {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let message = match extension.as_str() {
        "cbz" | "zip" => {
            units.push(Unit { path: path.to_path_buf(), kind: UnitKind::Archive, ancestors, at_selection_root });
            return;
        }
        "cbr" | "rar" | "cb7" | "7z" | "pdf" => "format pas encore pris en charge",
        _ => return,
    };
    issues.push(PlanIssue { path: path.display().to_string(), message: message.into() });
}

fn base_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| base_name(path))
}

struct ReadUnit {
    source: DraftSource,
    chapters: Vec<ReadChapter>,
    warnings: Vec<String>,
}

struct ReadChapter {
    display_name: String,
    unit_name: String,
    subpath: String,
    pages: Vec<Entry>,
    parsed: ParsedName,
    comic_info: Option<ComicInfo>,
}

fn read_unit(unit: &Unit) -> Result<ReadUnit> {
    match &unit.kind {
        UnitKind::Archive => read_archive(&unit.path),
        UnitKind::Directory => read_directory(&unit.path, None),
        UnitKind::LooseImages(names) => read_directory(&unit.path, Some(names)),
    }
}

fn read_archive(path: &Path) -> Result<ReadUnit> {
    let mut prefix = Vec::new();
    File::open(path)?.take(archive::SIGNATURE_BYTES as u64).read_to_end(&mut prefix)?;
    if !archive::is_zip_signature(&prefix) {
        return Err(Error::Archive("ce fichier n'est pas une archive ZIP malgré son extension".into()));
    }

    let mut reader = SourceReader::open(path, SourceKind::Zip)?;
    let entries = reader.entries()?;
    let (pages, comic_info_entry, mut warnings) = keep_images(&mut reader, entries);
    if pages.is_empty() {
        return Err(Error::Archive("aucune image lisible dans cette archive".into()));
    }
    let comic_info = comic_info_entry.and_then(|name| read_comic_info(&mut reader, &name, &mut warnings));
    let name = stem(path);

    // Regroupement par dossier interne : un CBZ peut contenir un tome entier.
    let mut folders: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for page in pages {
        let folder = page.name.rsplit_once('/').map(|(folder, _)| folder.to_string()).unwrap_or_default();
        folders.entry(folder).or_default().push(page);
    }
    let split = folders.len() > 1;
    let mut chapters = Vec::new();
    let mut ordered: Vec<_> = folders.into_iter().collect();
    ordered.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    let archive_parsed = parse_name(&name);
    for (folder, mut pages) in ordered {
        pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        let (parsed, display_name, subpath, info) = if split {
            let folder_name = archive::file_name(&folder);
            let mut parsed = if folder.is_empty() { ParsedName::default() } else { parse_name(folder_name) };
            parsed.series_hint = archive_parsed.series_hint.clone();
            parsed.volume = parsed.volume.or(archive_parsed.volume);
            let display = if folder.is_empty() { name.clone() } else { format!("{name} / {folder}") };
            // ComicInfo décrit l'archive entière : seuls les champs de série valent.
            (parsed, display, folder.clone(), comic_info.clone().map(archive_level_info))
        } else {
            (archive_parsed.clone(), name.clone(), String::new(), comic_info.clone())
        };
        chapters.push(ReadChapter { display_name, unit_name: name.clone(), subpath, pages, parsed, comic_info: info });
    }

    let metadata = fs::metadata(path)?;
    let source = DraftSource {
        path: path.to_path_buf(),
        kind: SourceKind::Zip,
        db_kind: if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) { "zip" } else { "cbz" },
        size: metadata.len(),
        modified_at: modified_at(&metadata),
        partial_hash: hash_file(path, metadata.len())?,
    };
    Ok(ReadUnit { source, chapters, warnings })
}

fn archive_level_info(info: ComicInfo) -> ComicInfo {
    ComicInfo { title: None, number: None, ..info }
}

fn read_directory(path: &Path, only: Option<&Vec<String>>) -> Result<ReadUnit> {
    let mut reader = SourceReader::open(path, SourceKind::Directory)?;
    let mut entries = reader.entries()?;
    if let Some(names) = only {
        entries.retain(|entry| names.contains(&entry.name));
    }
    let (mut pages, comic_info_entry, mut warnings) = keep_images(&mut reader, entries);
    if pages.is_empty() {
        return Err(Error::Archive("aucune image lisible dans ce dossier".into()));
    }
    pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    let comic_info = comic_info_entry.and_then(|name| read_comic_info(&mut reader, &name, &mut warnings));
    let name = base_name(path);
    let (subpath, display_name) = match only {
        // Une sélection d'images a son identité propre dans le dossier.
        Some(_) => (pages[0].name.clone(), format!("{name} / {} images", pages.len())),
        None => (String::new(), name.clone()),
    };

    let source = DraftSource {
        path: path.to_path_buf(),
        kind: SourceKind::Directory,
        db_kind: "directory",
        size: pages.iter().map(|p| p.size).sum(),
        modified_at: fs::metadata(path).ok().as_ref().and_then(modified_at),
        partial_hash: hash_listing(&pages),
    };
    let chapter = ReadChapter {
        display_name,
        unit_name: name.clone(),
        subpath,
        parsed: parse_name(&name),
        pages,
        comic_info,
    };
    Ok(ReadUnit { source, chapters: vec![chapter], warnings })
}

/// Ne garde que les vraies images, reconnues par leur signature.
fn keep_images(reader: &mut SourceReader, entries: Vec<Entry>) -> (Vec<Entry>, Option<String>, Vec<String>) {
    let mut pages = Vec::new();
    let mut comic_info = None;
    let mut warnings = Vec::new();
    for entry in entries {
        if archive::file_name(&entry.name).eq_ignore_ascii_case("ComicInfo.xml") {
            // Celui de la racine est prioritaire.
            if comic_info.is_none() || !entry.name.contains('/') {
                comic_info = Some(entry.name);
            }
            continue;
        }
        let is_image = reader.read_prefix(&entry.name).ok().and_then(|p| archive::sniff_image(&p)).is_some();
        if is_image {
            pages.push(entry);
        } else if archive::has_image_extension(&entry.name) {
            warnings.push(format!("« {} » ignoré : ce n'est pas une image valide", entry.name));
        }
    }
    (pages, comic_info, warnings)
}

fn read_comic_info(reader: &mut SourceReader, name: &str, warnings: &mut Vec<String>) -> Option<ComicInfo> {
    let parsed = reader.read(name).ok().and_then(|bytes| comicinfo::parse(&bytes));
    if parsed.is_none() {
        warnings.push("ComicInfo.xml ignoré : fichier invalide".into());
    }
    parsed
}

fn modified_at(metadata: &fs::Metadata) -> Option<String> {
    let seconds = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.to_string())
}

/// Empreinte rapide : taille, début et fin du fichier. Elle sert à retrouver un
/// fichier déplacé sans relire des gigaoctets.
fn hash_file(path: &Path, size: u64) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    (&mut file).take(HASH_SAMPLE_BYTES).read_to_end(&mut buffer)?;
    hasher.update(&buffer);
    if size > HASH_SAMPLE_BYTES {
        buffer.clear();
        file.seek(SeekFrom::End(-(HASH_SAMPLE_BYTES.min(size) as i64)))?;
        file.read_to_end(&mut buffer)?;
        hasher.update(&buffer);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_listing(pages: &[Entry]) -> String {
    let mut hasher = blake3::Hasher::new();
    for page in pages {
        hasher.update(page.name.as_bytes());
        hasher.update(&page.size.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn existing_series(connection: &Connection) -> Result<HashMap<String, String>> {
    let mut statement = connection.prepare(
        "SELECT id, title FROM series
         UNION ALL
         SELECT series_id, title FROM series_titles",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut found = HashMap::new();
    for row in rows {
        let (id, title) = row?;
        found.entry(normalize_title(&title)).or_insert(id);
    }
    Ok(found)
}

/// Déjà importé : même chemin, ou copie de ce fichier dans la bibliothèque Komago.
fn known_subpaths(connection: &Connection, source: &DraftSource) -> Result<HashSet<String>> {
    let mut statement = connection.prepare(
        "SELECT c.source_subpath FROM chapters c
         JOIN source_files s ON s.id = c.source_id
         WHERE s.path = ?1 OR (s.storage = 'managed' AND s.partial_hash = ?2)",
    )?;
    let rows = statement.query_map(params![source.path.to_string_lossy(), source.partial_hash], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Une source signalée manquante et de même empreinte : c'est un fichier déplacé.
fn find_relocation(connection: &Connection, source: &DraftSource) -> Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT id FROM source_files
             WHERE partial_hash = ?1 AND size_bytes = ?2 AND status = 'missing' AND path <> ?3 AND kind = ?4
             LIMIT 1",
            params![source.partial_hash, source.size as i64, source.path.to_string_lossy(), source.db_kind],
            |row| row.get(0),
        )
        .optional()?)
}

// Validation --------------------------------------------------------------------

pub fn commit(database: &Database, storage: &Storage, session: &ImportSession, edits: &ImportEdits) -> Result<ImportResult> {
    let series_edits: HashMap<&str, &SeriesEdit> = edits.series.iter().map(|s| (s.key.as_str(), s)).collect();
    let mut included: Vec<&ChapterEdit> = edits.chapters.iter().filter(|c| c.include).collect();
    included.sort_by(|a, b| {
        a.volume.unwrap_or(f64::MAX).total_cmp(&b.volume.unwrap_or(f64::MAX))
            .then(a.number.unwrap_or(f64::MAX).total_cmp(&b.number.unwrap_or(f64::MAX)))
    });

    // Validation complète avant le moindre effet.
    for edit in &included {
        if !session.chapters.contains_key(&edit.id) {
            return Err(Error::Invalid("chapitre inconnu dans cet import".into()));
        }
        let series = series_edits
            .get(edit.series_key.as_str())
            .ok_or_else(|| Error::Invalid("série inconnue dans cet import".into()))?;
        if series.existing_series_id.is_none() && series.title.trim().is_empty() {
            return Err(Error::Invalid("une série doit avoir un titre".into()));
        }
        if [edit.volume, edit.number].iter().flatten().any(|n| !n.is_finite() || *n < 0.0) {
            return Err(Error::Invalid("numéro invalide".into()));
        }
    }

    // Les opérations lentes (copies, couvertures) ont lieu sans verrou d'écriture.
    let used_sources: HashSet<usize> = included.iter().map(|e| session.chapters[&e.id].source).collect();
    let mut managed_paths: HashMap<usize, PathBuf> = HashMap::new();
    if edits.copy_files {
        for index in &used_sources {
            match copy_source(storage, session, *index) {
                Ok(copy) => {
                    managed_paths.insert(*index, copy);
                }
                Err(error) => {
                    for path in managed_paths.values() {
                        remove_copy(path);
                    }
                    return Err(error);
                }
            }
        }
    }

    let mut rendered: HashMap<&str, covers::Cover> = HashMap::new();
    let mut result = ImportResult::default();
    for (key, edit) in &series_edits {
        if edit.existing_series_id.is_some() {
            continue;
        }
        let first = included.iter().find(|c| c.series_key == *key);
        let Some(first) = first else { continue };
        match render_cover(session, &first.id) {
            Ok(cover) => {
                rendered.insert(key, cover);
            }
            Err(_) => result.covers_failed += 1,
        }
    }

    let outcome = database.write(|connection| {
        let transaction = connection.transaction()?;
        apply(&transaction, storage, session, edits, &series_edits, &included, &rendered, &managed_paths, &mut result)?;
        transaction.commit()?;
        Ok(())
    });
    if let Err(error) = outcome {
        for path in managed_paths.values() {
            remove_copy(path);
        }
        return Err(error);
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn apply(
    tx: &Transaction<'_>,
    storage: &Storage,
    session: &ImportSession,
    _edits: &ImportEdits,
    series_edits: &HashMap<&str, &SeriesEdit>,
    included: &[&ChapterEdit],
    rendered: &HashMap<&str, covers::Cover>,
    managed_paths: &HashMap<usize, PathBuf>,
    result: &mut ImportResult,
) -> Result<()> {
    for (source_id, index) in &session.relocations {
        let source = &session.sources[*index];
        tx.execute(
            "UPDATE source_files SET path = ?1, status = 'available', last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
            params![source.path.to_string_lossy(), source_id],
        )?;
        result.relocated += 1;
    }

    let mut series_ids: HashMap<&str, String> = HashMap::new();
    let mut source_ids: HashMap<usize, String> = HashMap::new();
    let mut last_sort: HashMap<String, f64> = HashMap::new();

    for edit in included {
        let draft = &session.chapters[&edit.id];
        let series_edit = series_edits[edit.series_key.as_str()];
        let series_id = match series_ids.get(edit.series_key.as_str()) {
            Some(id) => id.clone(),
            None => {
                let id = match &series_edit.existing_series_id {
                    Some(id) => {
                        let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM series WHERE id = ?1)", [id], |r| r.get(0))?;
                        if !exists {
                            return Err(Error::NotFound("série de destination".into()));
                        }
                        id.clone()
                    }
                    None => {
                        let cover = match rendered.get(edit.series_key.as_str()) {
                            Some(cover) => Some(covers::store(tx, &storage.covers_dir(), cover)?),
                            None => None,
                        };
                        let id = create_series(tx, series_edit.title.trim(), cover.as_deref(), draft.comic_info.as_ref())?;
                        result.series_created += 1;
                        id
                    }
                };
                result.first_series_id.get_or_insert_with(|| id.clone());
                series_ids.insert(edit.series_key.as_str(), id.clone());
                id
            }
        };

        let source_id = match source_ids.get(&draft.source) {
            Some(id) => id.clone(),
            None => {
                let id = upsert_source(tx, &session.sources[draft.source], managed_paths.get(&draft.source))?;
                source_ids.insert(draft.source, id.clone());
                id
            }
        };

        // Déjà importé (et pas seulement à l'analyse : une autre fenêtre de temps
        // a pu s'écouler) : on ne double jamais un chapitre.
        let duplicate: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM chapters WHERE source_id = ?1 AND source_subpath = ?2)",
            params![source_id, draft.subpath],
            |r| r.get(0),
        )?;
        if duplicate {
            continue;
        }

        let volume_id = match edit.volume {
            Some(number) => Some(upsert_volume(tx, &series_id, number)?),
            None => None,
        };
        let sort_key = match edit.number {
            Some(number) => number,
            // Sans numéro : juste après le chapitre précédent, dans l'ordre du plan.
            None => last_sort.get(&series_id).copied().unwrap_or(0.0) + 0.001,
        };
        last_sort.insert(series_id.clone(), sort_key);

        let chapter_id = Uuid::now_v7().to_string();
        let info = draft.comic_info.as_ref();
        tx.execute(
            "INSERT INTO chapters (id, series_id, volume_id, number, title, published_on, language,
                                   page_count, sort_key, source_id, source_subpath)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                chapter_id,
                series_id,
                volume_id,
                edit.number,
                edit.title.as_deref().map(str::trim).filter(|t| !t.is_empty()),
                info.and_then(|i| i.published_on.as_deref()),
                info.and_then(|i| i.language.as_deref()),
                draft.pages.len() as i64,
                sort_key,
                source_id,
                draft.subpath,
            ],
        )?;
        let mut insert_page = tx.prepare_cached(
            "INSERT INTO pages (chapter_id, page_index, entry_name, size_bytes) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (index, page) in draft.pages.iter().enumerate() {
            insert_page.execute(params![chapter_id, index as i64, page.name, page.size as i64])?;
        }
        result.chapters_added += 1;
    }
    Ok(())
}

fn create_series(tx: &Transaction<'_>, title: &str, cover: Option<&str>, info: Option<&ComicInfo>) -> Result<String> {
    let id = Uuid::now_v7().to_string();
    tx.execute(
        "INSERT INTO series (id, title, sort_title, kind, description, publisher, original_language,
                             reading_direction, cover_asset_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            title,
            sort_title(title),
            if info.is_some_and(|i| i.is_manga) { "manga" } else { "unknown" },
            info.and_then(|i| i.summary.as_deref()),
            info.and_then(|i| i.publisher.as_deref()),
            info.and_then(|i| i.language.as_deref()),
            info.is_some_and(|i| i.right_to_left).then_some("rtl"),
            cover,
        ],
    )?;
    let Some(info) = info else { return Ok(id) };

    for writer in &info.writers {
        crate::library::link_person(tx, &id, writer, "author")?;
    }
    for artist in &info.artists {
        crate::library::link_person(tx, &id, artist, "artist")?;
    }
    for genre in &info.genres {
        crate::library::link_named(tx, "genres", "series_genres", "genre_id", &id, genre)?;
    }
    for tag in &info.tags {
        crate::library::link_named(tx, "tags", "series_tags", "tag_id", &id, tag)?;
    }

    // Seuls les champs réellement remplis par l'archive ont cette provenance.
    let mut fields = Vec::new();
    if info.series.is_some() {
        fields.push("title");
    }
    if info.summary.is_some() {
        fields.push("description");
    }
    if info.publisher.is_some() {
        fields.push("publisher");
    }
    if info.language.is_some() {
        fields.push("original_language");
    }
    if !info.writers.is_empty() {
        fields.push("authors");
    }
    if !info.artists.is_empty() {
        fields.push("artists");
    }
    if !info.genres.is_empty() {
        fields.push("genres");
    }
    if !info.tags.is_empty() {
        fields.push("tags");
    }
    for field in fields {
        tx.execute(
            "INSERT INTO field_provenance (entity_type, entity_id, field, source_id, locked)
             VALUES ('series', ?1, ?2, 'comicinfo', 0)",
            params![id, field],
        )?;
    }
    Ok(id)
}

fn upsert_source(tx: &Transaction<'_>, source: &DraftSource, managed: Option<&PathBuf>) -> Result<String> {
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM source_files WHERE path = ?1",
            [source.path.to_string_lossy()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        tx.execute(
            "UPDATE source_files SET status = 'available', size_bytes = ?1, modified_at = ?2, partial_hash = ?3,
                    last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?4",
            params![source.size as i64, source.modified_at, source.partial_hash, id],
        )?;
        return Ok(id);
    }
    let id = Uuid::now_v7().to_string();
    let path = managed.unwrap_or(&source.path);
    tx.execute(
        "INSERT INTO source_files (id, path, kind, storage, size_bytes, modified_at, partial_hash, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            id,
            path.to_string_lossy(),
            source.db_kind,
            if managed.is_some() { "managed" } else { "referenced" },
            source.size as i64,
            source.modified_at,
            source.partial_hash,
        ],
    )?;
    Ok(id)
}

fn upsert_volume(tx: &Transaction<'_>, series_id: &str, number: f64) -> Result<String> {
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM volumes WHERE series_id = ?1 AND number = ?2",
            params![series_id, number],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = Uuid::now_v7().to_string();
    tx.execute(
        "INSERT INTO volumes (id, series_id, number, sort_key) VALUES (?1, ?2, ?3, ?3)",
        params![id, series_id, number],
    )?;
    Ok(id)
}

fn render_cover(session: &ImportSession, chapter_id: &str) -> Result<covers::Cover> {
    let (path, kind, entry) = session
        .cover_page(chapter_id)
        .ok_or_else(|| Error::NotFound("page de couverture".into()))?;
    let bytes = SourceReader::open(&path, kind)?.read(&entry)?;
    covers::render(&bytes)
}

/// Copie seulement ce que Komago sait lire : jamais les autres fichiers du dossier.
fn copy_source(storage: &Storage, session: &ImportSession, index: usize) -> Result<PathBuf> {
    let source = &session.sources[index];
    let id = Uuid::now_v7().to_string();
    match source.kind {
        SourceKind::Zip => {
            let target = storage.library_dir().join(format!("{id}.{}", source.db_kind));
            fs::copy(&source.path, &target)?;
            Ok(target)
        }
        SourceKind::Directory => {
            let target = storage.library_dir().join(&id);
            fs::create_dir(&target)?;
            let mut reader = SourceReader::open(&source.path, SourceKind::Directory)?;
            let copy = || -> Result<()> {
                for chapter in session.chapters.values().filter(|c| c.source == index) {
                    for page in &chapter.pages {
                        let bytes = reader.read(&page.name)?;
                        fs::write(target.join(&page.name), bytes)?;
                    }
                }
                Ok(())
            };
            let mut copy = copy;
            if let Err(error) = copy() {
                remove_copy(&target);
                return Err(error);
            }
            Ok(target)
        }
    }
}

fn remove_copy(path: &Path) {
    let _ = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
}

// Disponibilité des fichiers ------------------------------------------------------

/// Vérifie la présence des sources et met à jour leur statut. Rien n'est supprimé.
pub fn check_sources(database: &Database) -> Result<usize> {
    let sources: Vec<(String, String, String)> = database.read(|connection| {
        let mut statement = connection.prepare("SELECT id, path, status FROM source_files")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;

    let mut changes = Vec::new();
    let mut missing = 0;
    for (id, path, status) in sources {
        let present = Path::new(&path).exists();
        if !present {
            missing += 1;
        }
        let target = if present { "available" } else { "missing" };
        if status != target && status != "corrupt" {
            changes.push((id, target));
        }
    }
    if !changes.is_empty() {
        database.write(|connection| {
            let transaction = connection.transaction()?;
            for (id, status) in &changes {
                transaction.execute("UPDATE source_files SET status = ?1 WHERE id = ?2", params![status, id])?;
            }
            transaction.commit()?;
            Ok(())
        })?;
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::test_support::{write_zip, JPEG};
    use crate::covers::test_support::png;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        dir: TempDir,
        database: Database,
        storage: Storage,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().join("data"), dir.path().join("cache")).unwrap();
        let database = Database::open(&storage.database_path()).unwrap();
        Fixture { dir, database, storage }
    }

    impl Fixture {
        fn input(&self) -> PathBuf {
            let path = self.dir.path().join("input");
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn analyze(&self, paths: &[PathBuf]) -> (ImportSession, ImportPlan) {
            analyze(paths, &self.database, &AtomicBool::new(false), |_| {}).unwrap()
        }

        fn commit_all(&self, session: &ImportSession, plan: &ImportPlan) -> ImportResult {
            commit(&self.database, &self.storage, session, &accept(plan)).unwrap()
        }

        fn count(&self, sql: &str) -> i64 {
            self.database.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
        }
    }

    fn accept(plan: &ImportPlan) -> ImportEdits {
        ImportEdits {
            series: plan
                .series
                .iter()
                .map(|s| SeriesEdit { key: s.key.clone(), title: s.title.clone(), existing_series_id: s.existing_series_id.clone() })
                .collect(),
            chapters: plan
                .series
                .iter()
                .flat_map(|s| {
                    s.chapters.iter().map(|c| ChapterEdit {
                        id: c.id.clone(),
                        series_key: s.key.clone(),
                        volume: c.volume,
                        number: c.number,
                        title: c.title.clone(),
                        include: !c.duplicate,
                    })
                })
                .collect(),
            copy_files: false,
        }
    }

    fn make_cbz(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_zip(path, &[("2.jpg", JPEG), ("10.jpg", JPEG), ("1.jpg", JPEG)]);
    }

    fn chapter_summary(series: &PlanSeries) -> Vec<(Option<f64>, Option<f64>)> {
        series.chapters.iter().map(|c| (c.volume, c.number)).collect()
    }

    #[test]
    fn detects_series_volumes_and_chapters_from_folders() {
        let f = fixture();
        let root = f.input().join("Ma série");
        make_cbz(&root.join("Volume 01/Chapter 001.cbz"));
        make_cbz(&root.join("Volume 01/Chapter 002.cbz"));
        make_cbz(&root.join("Volume 02/Chapter 003.cbz"));

        let (_, plan) = f.analyze(&[root]);

        assert_eq!(plan.series.len(), 1);
        assert_eq!(plan.series[0].title, "Ma série");
        assert_eq!(
            chapter_summary(&plan.series[0]),
            [(Some(1.0), Some(1.0)), (Some(1.0), Some(2.0)), (Some(2.0), Some(3.0))]
        );
        assert!(plan.issues.is_empty());
    }

    #[test]
    fn chapters_without_volume_sit_directly_in_the_series() {
        let f = fixture();
        let root = f.input().join("Webtoon");
        make_cbz(&root.join("Episode 1.cbz"));
        make_cbz(&root.join("Episode 2.cbz"));

        let (_, plan) = f.analyze(&[root]);
        assert_eq!(chapter_summary(&plan.series[0]), [(None, Some(1.0)), (None, Some(2.0))]);
    }

    #[test]
    fn a_container_folder_yields_several_series() {
        let f = fixture();
        let root = f.input().join("Mangas");
        make_cbz(&root.join("Berserk/Chapter 1.cbz"));
        make_cbz(&root.join("Vinland Saga/Volume 3/Chapter 80.cbz"));

        let (_, plan) = f.analyze(&[root]);
        let titles: Vec<_> = plan.series.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Berserk", "Vinland Saga"]);
        assert_eq!(chapter_summary(&plan.series[1]), [(Some(3.0), Some(80.0))]);
    }

    #[test]
    fn loose_files_at_the_root_group_by_their_own_names() {
        let f = fixture();
        let root = f.input().join("Téléchargements");
        make_cbz(&root.join("One Piece 001.cbz"));
        make_cbz(&root.join("One Piece 002.cbz"));
        make_cbz(&root.join("Naruto v02 c010.cbz"));

        let (_, plan) = f.analyze(&[root]);
        let titles: Vec<_> = plan.series.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Naruto", "One Piece"]);
    }

    #[test]
    fn image_folders_become_chapters() {
        let f = fixture();
        let root = f.input().join("Ma série");
        let chapter = root.join("Chapter 5");
        fs::create_dir_all(&chapter).unwrap();
        for name in ["10.jpg", "2.jpg", "1.jpg"] {
            fs::write(chapter.join(name), JPEG).unwrap();
        }

        let (session, plan) = f.analyze(&[root]);
        assert_eq!(plan.series[0].title, "Ma série");
        assert_eq!(chapter_summary(&plan.series[0]), [(None, Some(5.0))]);
        let draft = &session.chapters[&plan.series[0].chapters[0].id];
        let names: Vec<_> = draft.pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["1.jpg", "2.jpg", "10.jpg"]);
    }

    #[test]
    fn analysis_can_be_cancelled_and_leaves_nothing() {
        let f = fixture();
        let root = f.input().join("Série");
        make_cbz(&root.join("Chapter 1.cbz"));

        let cancelled = analyze(std::slice::from_ref(&root), &f.database, &AtomicBool::new(true), |_| {});
        assert!(matches!(cancelled, Err(Error::Cancelled)));
        assert_eq!(f.count("SELECT count(*) FROM series"), 0);
    }

    #[test]
    fn a_selected_chapter_takes_its_series_from_the_parent_folder() {
        let f = fixture();
        let root = f.input().join("The Glory");
        let chapter = root.join("Chapitre 001");
        fs::create_dir_all(&chapter).unwrap();
        fs::write(chapter.join("1.jpg"), JPEG).unwrap();
        let archive = root.join("Chapitre 002.cbz");
        make_cbz(&archive);
        let volume = root.join("Volume 2");
        make_cbz(&volume.join("Chapitre 003.cbz"));

        let (_, plan) = f.analyze(&[chapter, archive, volume]);
        assert_eq!(plan.series.len(), 1);
        assert_eq!(plan.series[0].title, "The Glory");
        assert_eq!(
            chapter_summary(&plan.series[0]),
            [(Some(2.0), Some(3.0)), (None, Some(1.0)), (None, Some(2.0))]
        );
    }

    #[test]
    fn a_selected_named_folder_keeps_its_own_title() {
        let f = fixture();
        let series = f.input().join("Ma Série");
        fs::create_dir_all(&series).unwrap();
        fs::write(series.join("1.jpg"), JPEG).unwrap();

        let (_, plan) = f.analyze(&[series]);
        assert_eq!(plan.series[0].title, "Ma Série");
    }

    #[test]
    fn selected_images_become_one_chapter() {
        let f = fixture();
        let input = f.input();
        for name in ["b.png", "a.png", "ignored.png"] {
            fs::write(input.join(name), JPEG).unwrap();
        }
        let (_, plan) = f.analyze(&[input.join("a.png"), input.join("b.png")]);
        assert_eq!(plan.series[0].chapters.len(), 1);
        assert_eq!(plan.series[0].chapters[0].page_count, 2);
    }

    #[test]
    fn archive_with_internal_folders_splits_into_chapters() {
        let f = fixture();
        let path = f.input().join("Berserk Tome 3.cbz");
        write_zip(&path, &[("Chapter 2/1.jpg", JPEG), ("Chapter 10/1.jpg", JPEG), ("Chapter 2/2.jpg", JPEG)]);

        let (session, plan) = f.analyze(std::slice::from_ref(&path));
        assert_eq!(plan.series[0].title, "Berserk");
        assert_eq!(chapter_summary(&plan.series[0]), [(Some(3.0), Some(2.0)), (Some(3.0), Some(10.0))]);
        assert_eq!(session.chapters[&plan.series[0].chapters[0].id].pages.len(), 2);

        f.commit_all(&session, &plan);
        assert_eq!(f.count("SELECT count(*) FROM source_files"), 1);
        assert_eq!(f.count("SELECT count(*) FROM chapters"), 2);
    }

    #[test]
    fn comic_info_takes_priority_over_names() {
        let f = fixture();
        let path = f.input().join("nom quelconque 99.cbz");
        let xml = b"<ComicInfo><Series>Vraie Serie</Series><Title>Vrai titre</Title><Number>7</Number>\
                    <Volume>2</Volume><Writer>Ecrivain</Writer><Genre>Drame</Genre><Manga>YesAndRightToLeft</Manga></ComicInfo>";
        write_zip(&path, &[("ComicInfo.xml", xml), ("1.jpg", JPEG)]);

        let (session, plan) = f.analyze(std::slice::from_ref(&path));
        let series = &plan.series[0];
        assert_eq!(series.title, "Vraie Serie");
        assert_eq!(chapter_summary(series), [(Some(2.0), Some(7.0))]);
        assert!(series.chapters[0].has_comic_info);
        assert_eq!(series.chapters[0].page_count, 1, "ComicInfo.xml n'est pas une page");

        f.commit_all(&session, &plan);
        assert_eq!(f.count("SELECT count(*) FROM series WHERE kind = 'manga' AND reading_direction = 'rtl'"), 1);
        assert_eq!(f.count("SELECT count(*) FROM series_people"), 1);
        assert_eq!(
            f.count("SELECT count(*) FROM field_provenance WHERE field = 'authors' AND source_id = 'comicinfo' AND locked = 0"),
            1
        );
    }

    #[test]
    fn bad_files_are_reported_without_stopping_the_others() {
        let f = fixture();
        let root = f.input().join("Série");
        make_cbz(&root.join("Chapter 1.cbz"));
        fs::write(root.join("Chapter 2.cbz"), b"pas une archive").unwrap();
        fs::write(root.join("Chapter 3.cbr"), b"Rar!").unwrap();
        write_zip(&root.join("Chapter 4.cbz"), &[("../../evil.jpg", JPEG)]);
        write_zip(&root.join("Chapter 5.cbz"), &[("notes.txt", b"rien")]);

        let (_, plan) = f.analyze(&[root]);
        assert_eq!(plan.series[0].chapters.len(), 1);
        assert_eq!(plan.issues.len(), 4);
    }

    #[test]
    fn deceptive_extensions_are_not_pages() {
        let f = fixture();
        let path = f.input().join("Chapter 1.cbz");
        write_zip(&path, &[("1.jpg", JPEG), ("2.jpg", b"MZ executable pas une image"), ("3.dat", JPEG)]);

        let (_, plan) = f.analyze(std::slice::from_ref(&path));
        assert_eq!(plan.series[0].chapters[0].page_count, 2, "signature seule : 1.jpg et 3.dat");
        assert_eq!(plan.issues.len(), 1);
    }

    #[test]
    fn commit_stores_pages_in_natural_order_and_keeps_the_original() {
        let f = fixture();
        let path = f.input().join("Série/Chapter 1.cbz");
        make_cbz(&path);
        let original = fs::read(&path).unwrap();

        let (session, plan) = f.analyze(std::slice::from_ref(&path));
        let result = f.commit_all(&session, &plan);

        assert_eq!(result.chapters_added, 1);
        let names: Vec<String> = f
            .database
            .read(|c| {
                let mut s = c.prepare("SELECT entry_name FROM pages ORDER BY page_index")?;
                let rows = s.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .unwrap();
        assert_eq!(names, ["1.jpg", "2.jpg", "10.jpg"]);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(f.count("SELECT count(*) FROM source_files WHERE storage = 'referenced'"), 1);
    }

    #[test]
    fn user_corrections_are_applied() {
        let f = fixture();
        let root = f.input().join("Mauvais nom");
        make_cbz(&root.join("a.cbz"));
        make_cbz(&root.join("b.cbz"));

        let (session, plan) = f.analyze(&[root]);
        let mut edits = accept(&plan);
        edits.series[0].title = "Bon nom".into();
        edits.chapters[0].number = Some(4.0);
        edits.chapters[0].volume = Some(2.0);
        edits.chapters[0].title = Some("Titre".into());
        edits.chapters[1].include = false;
        commit(&f.database, &f.storage, &session, &edits).unwrap();

        assert_eq!(f.count("SELECT count(*) FROM series WHERE title = 'Bon nom'"), 1);
        assert_eq!(f.count("SELECT count(*) FROM chapters"), 1);
        assert_eq!(f.count("SELECT count(*) FROM chapters WHERE number = 4 AND title = 'Titre' AND volume_id IS NOT NULL"), 1);
        assert_eq!(f.count("SELECT count(*) FROM volumes WHERE number = 2"), 1);
    }

    #[test]
    fn invalid_corrections_change_nothing() {
        let f = fixture();
        let path = f.input().join("Série/Chapter 1.cbz");
        make_cbz(&path);
        let (session, plan) = f.analyze(std::slice::from_ref(&path));

        let mut empty_title = accept(&plan);
        empty_title.series[0].title = "  ".into();
        assert!(commit(&f.database, &f.storage, &session, &empty_title).is_err());

        let mut unknown = accept(&plan);
        unknown.chapters[0].id = "inconnu".into();
        assert!(commit(&f.database, &f.storage, &session, &unknown).is_err());

        let mut negative = accept(&plan);
        negative.chapters[0].number = Some(-1.0);
        assert!(commit(&f.database, &f.storage, &session, &negative).is_err());

        assert_eq!(f.count("SELECT count(*) FROM series"), 0);
    }

    #[test]
    fn reimporting_creates_no_duplicates_and_merges_into_the_series() {
        let f = fixture();
        let root = f.input().join("Ma Série");
        make_cbz(&root.join("Chapter 1.cbz"));
        let (session, plan) = f.analyze(std::slice::from_ref(&root));
        f.commit_all(&session, &plan);

        make_cbz(&root.join("Chapter 2.cbz"));
        let (session, plan) = f.analyze(std::slice::from_ref(&root));
        assert!(plan.series[0].existing_series_id.is_some(), "la série existante est reconnue");
        let duplicates: Vec<_> = plan.series[0].chapters.iter().map(|c| c.duplicate).collect();
        assert_eq!(duplicates, [true, false]);

        f.commit_all(&session, &plan);
        assert_eq!(f.count("SELECT count(*) FROM series"), 1);
        assert_eq!(f.count("SELECT count(*) FROM chapters"), 2);
    }

    #[test]
    fn missing_files_are_flagged_then_relocated_without_losing_progress() {
        let f = fixture();
        let input = f.input();
        let path = input.join("Série/Chapter 1.cbz");
        make_cbz(&path);
        let (session, plan) = f.analyze(std::slice::from_ref(&path));
        f.commit_all(&session, &plan);
        f.database
            .write(|c| {
                c.execute(
                    "INSERT INTO reading_progress (chapter_id, page_index, last_read_at)
                     SELECT id, 2, '2026-01-01' FROM chapters",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let moved = input.join("Ailleurs/Chapter 1.cbz");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&path, &moved).unwrap();
        assert_eq!(check_sources(&f.database).unwrap(), 1);
        assert_eq!(f.count("SELECT count(*) FROM source_files WHERE status = 'missing'"), 1);
        assert_eq!(f.count("SELECT count(*) FROM chapters"), 1, "rien n'est supprimé");

        let (session, plan) = f.analyze(std::slice::from_ref(&moved));
        assert_eq!(plan.relocated, 1);
        let result = f.commit_all(&session, &plan);

        assert_eq!(result.relocated, 1);
        assert_eq!(result.chapters_added, 0);
        assert_eq!(f.count("SELECT count(*) FROM source_files WHERE status = 'available'"), 1);
        assert_eq!(f.count("SELECT count(*) FROM reading_progress WHERE page_index = 2"), 1);
        assert_eq!(check_sources(&f.database).unwrap(), 0);
    }

    #[test]
    fn real_images_produce_a_stored_cover_and_bad_ones_do_not_block() {
        let f = fixture();
        let input = f.input();
        let good = input.join("Bonne/Chapter 1.cbz");
        fs::create_dir_all(good.parent().unwrap()).unwrap();
        write_zip(&good, &[("001.png", &png(300, 400))]);
        let bad = input.join("Mauvaise/Chapter 1.cbz");
        make_cbz(&bad);

        let (session, plan) = f.analyze(&[input.join("Bonne"), input.join("Mauvaise")]);
        let result = f.commit_all(&session, &plan);

        assert_eq!(result.series_created, 2);
        assert_eq!(result.covers_failed, 1);
        assert_eq!(f.count("SELECT count(*) FROM series WHERE cover_asset_id IS NOT NULL"), 1);
        let files = fs::read_dir(f.storage.covers_dir()).unwrap().count();
        assert_eq!(files, 1);
    }

    #[test]
    fn a_page_named_cover_is_preferred() {
        let pages = |names: &[&str]| names.iter().map(|n| Entry { name: n.to_string(), size: 1 }).collect::<Vec<_>>();
        assert_eq!(cover_page(&pages(&["001.jpg", "cover.jpg"])).unwrap().name, "cover.jpg");
        assert_eq!(cover_page(&pages(&["001.jpg", "002.jpg"])).unwrap().name, "001.jpg");
    }

    #[test]
    fn copy_option_duplicates_into_the_library_and_leaves_the_original() {
        let f = fixture();
        let path = f.input().join("Série/Chapter 1.cbz");
        make_cbz(&path);
        let (session, plan) = f.analyze(std::slice::from_ref(&path));
        let mut edits = accept(&plan);
        edits.copy_files = true;
        commit(&f.database, &f.storage, &session, &edits).unwrap();

        assert!(path.exists());
        assert_eq!(f.count("SELECT count(*) FROM source_files WHERE storage = 'managed'"), 1);
        assert_eq!(fs::read_dir(f.storage.library_dir()).unwrap().count(), 1);

        let (_, again) = f.analyze(std::slice::from_ref(&path));
        assert!(again.series[0].chapters[0].duplicate, "la copie compte comme déjà importée");
    }
}
