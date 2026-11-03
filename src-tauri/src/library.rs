//! Requêtes et modifications de la bibliothèque. Toute modification manuelle pose
//! un verrou de provenance : aucune actualisation ne pourra l'écraser.

use std::fs;
use std::path::Path;

use rusqlite::{named_params, params, Connection, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::storage::Storage;
use crate::text::{natural_cmp, sort_title};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SeriesFilter {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub collection_id: Option<String>,
    pub sort: SeriesSort,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeriesSort {
    #[default]
    Title,
    Added,
    Updated,
    Chapters,
    Read,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCard {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub cover_asset_id: Option<String>,
    pub volume_count: i64,
    pub chapter_count: i64,
    pub missing_count: i64,
    pub read_count: i64,
    #[serde(skip)]
    sort_title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDetail {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub published_from: Option<String>,
    pub published_to: Option<String>,
    pub original_language: Option<String>,
    pub cover_asset_id: Option<String>,
    pub alt_titles: Vec<String>,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub collection_ids: Vec<String>,
    pub volumes: Vec<VolumeRow>,
    pub chapters: Vec<ChapterRow>,
    /// Chapitre à ouvrir avec le bouton de lecture.
    pub resume_chapter_id: Option<String>,
    pub has_progress: bool,
    pub metadata_mode: Option<String>,
    pub external_links: Vec<ExternalLink>,
    pub field_sources: Vec<FieldSource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLink {
    pub source_id: String,
    pub source_name: String,
    pub external_id: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSource {
    pub field: String,
    pub source_name: String,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeRow {
    pub id: String,
    pub number: Option<f64>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub published_on: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterRow {
    pub id: String,
    pub series_id: String,
    pub series_title: String,
    pub volume_id: Option<String>,
    pub number: Option<f64>,
    pub title: Option<String>,
    pub page_count: i64,
    pub language: Option<String>,
    pub published_on: Option<String>,
    pub source_path: Option<String>,
    pub source_status: Option<String>,
    pub percent: Option<f64>,
    pub completed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPatch {
    pub title: String,
    pub kind: String,
    pub status: String,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub published_from: Option<String>,
    pub published_to: Option<String>,
    pub original_language: Option<String>,
    pub alt_titles: Vec<String>,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterPatch {
    pub number: Option<f64>,
    pub title: Option<String>,
    pub volume_id: Option<String>,
    pub series_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumePatch {
    pub number: Option<f64>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub published_on: Option<String>,
}

// Lecture -----------------------------------------------------------------------

/// Transforme une saisie libre en requête FTS5 sûre : chaque mot devient un
/// préfixe entre guillemets, aucun opérateur de l'utilisateur n'est interprété.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Le LIKE complète FTS pour les titres sans espaces (japonais, coréen…), que
/// l'index ne découpe pas en mots.
fn like_pattern(input: &str) -> String {
    let escaped = input.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

pub fn list_series(connection: &Connection, filter: &SeriesFilter) -> Result<Vec<SeriesCard>> {
    let order = match filter.sort {
        SeriesSort::Title => "s.sort_title COLLATE NOCASE",
        SeriesSort::Added => "s.created_at DESC",
        SeriesSort::Updated => "s.updated_at DESC",
        SeriesSort::Chapters => "chapter_count DESC, s.sort_title COLLATE NOCASE",
        SeriesSort::Read => "last_read_at IS NULL, last_read_at DESC, s.sort_title COLLATE NOCASE",
    };
    let query = filter.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let sql = format!(
        "SELECT s.id, s.title, s.kind, s.status, s.cover_asset_id,
                (SELECT count(*) FROM volumes v WHERE v.series_id = s.id) AS volume_count,
                (SELECT count(*) FROM chapters c WHERE c.series_id = s.id) AS chapter_count,
                (SELECT count(*) FROM chapters c JOIN source_files f ON f.id = c.source_id
                 WHERE c.series_id = s.id AND f.status <> 'available') AS missing_count,
                (SELECT count(*) FROM chapters c JOIN reading_progress p ON p.chapter_id = c.id
                 WHERE c.series_id = s.id AND p.completed_at IS NOT NULL) AS read_count,
                (SELECT max(p.last_read_at) FROM chapters c JOIN reading_progress p ON p.chapter_id = c.id
                 WHERE c.series_id = s.id) AS last_read_at,
                s.sort_title
         FROM series s
         WHERE (:kind IS NULL OR s.kind = :kind)
           AND (:status IS NULL OR s.status = :status)
           AND (:collection IS NULL OR s.id IN
                (SELECT series_id FROM collection_items WHERE collection_id = :collection AND series_id IS NOT NULL))
           AND (:fts IS NULL
                OR s.id IN (SELECT series_id FROM search_index WHERE search_index MATCH :fts)
                OR s.title LIKE :like ESCAPE '\\')
         ORDER BY {order}"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(
        named_params! {
            ":kind": filter.kind,
            ":status": filter.status,
            ":collection": filter.collection_id,
            ":fts": query.and_then(fts_query),
            ":like": query.map(like_pattern),
        },
        |row| {
            Ok(SeriesCard {
                id: row.get(0)?,
                title: row.get(1)?,
                kind: row.get(2)?,
                status: row.get(3)?,
                cover_asset_id: row.get(4)?,
                volume_count: row.get(5)?,
                chapter_count: row.get(6)?,
                missing_count: row.get(7)?,
                read_count: row.get(8)?,
                sort_title: row.get(10)?,
            })
        },
    )?;
    let mut cards: Vec<SeriesCard> = rows.collect::<rusqlite::Result<_>>()?;
    // SQLite ne sait pas trier « Tome 2 » avant « Tome 10 ».
    if filter.sort == SeriesSort::Title {
        cards.sort_by(|a, b| natural_cmp(&a.sort_title, &b.sort_title));
    }
    Ok(cards)
}

/// Séries sans aucune fiche externe associée.
pub fn list_unidentified(connection: &Connection) -> Result<Vec<SeriesCard>> {
    let all = list_series(connection, &SeriesFilter::default())?;
    let mut statement = connection.prepare("SELECT DISTINCT series_id FROM external_ids")?;
    let linked: std::collections::HashSet<String> =
        statement.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(all.into_iter().filter(|s| !linked.contains(&s.id)).collect())
}

pub const CHAPTER_COLUMNS: &str = "c.id, c.series_id, s.title, c.volume_id, c.number, c.title, c.page_count,
        c.language, c.published_on, f.path, f.status, rp.percent, rp.completed_at IS NOT NULL";
pub const CHAPTER_JOINS: &str = "JOIN series s ON s.id = c.series_id
     LEFT JOIN source_files f ON f.id = c.source_id
     LEFT JOIN reading_progress rp ON rp.chapter_id = c.id";

pub fn chapter_row(row: &Row<'_>) -> rusqlite::Result<ChapterRow> {
    Ok(ChapterRow {
        id: row.get(0)?,
        series_id: row.get(1)?,
        series_title: row.get(2)?,
        volume_id: row.get(3)?,
        number: row.get(4)?,
        title: row.get(5)?,
        page_count: row.get(6)?,
        language: row.get(7)?,
        published_on: row.get(8)?,
        source_path: row.get(9)?,
        source_status: row.get(10)?,
        percent: row.get(11)?,
        completed: row.get::<_, Option<bool>>(12)?.unwrap_or(false),
    })
}

/// Chapitres dont le titre correspond, pour la recherche globale.
pub fn search_chapters(connection: &Connection, query: &str) -> Result<Vec<ChapterRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT {CHAPTER_COLUMNS} FROM chapters c {CHAPTER_JOINS} WHERE c.title LIKE ?1 ESCAPE '\\' ORDER BY s.sort_title, c.sort_key LIMIT 30");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([like_pattern(query)], chapter_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get_series(connection: &Connection, id: &str) -> Result<SeriesDetail> {
    let mut detail = connection
        .query_row(
            "SELECT id, title, kind, status, description, publisher, published_from, published_to,
                    original_language, cover_asset_id, metadata_mode
             FROM series WHERE id = ?1",
            [id],
            |row| {
                Ok(SeriesDetail {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    kind: row.get(2)?,
                    status: row.get(3)?,
                    description: row.get(4)?,
                    publisher: row.get(5)?,
                    published_from: row.get(6)?,
                    published_to: row.get(7)?,
                    original_language: row.get(8)?,
                    cover_asset_id: row.get(9)?,
                    alt_titles: Vec::new(),
                    authors: Vec::new(),
                    artists: Vec::new(),
                    genres: Vec::new(),
                    tags: Vec::new(),
                    collection_ids: Vec::new(),
                    volumes: Vec::new(),
                    chapters: Vec::new(),
                    resume_chapter_id: None,
                    has_progress: false,
                    metadata_mode: row.get(10)?,
                    external_links: Vec::new(),
                    field_sources: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| Error::NotFound("série".into()))?;

    detail.alt_titles = strings(connection, "SELECT title FROM series_titles WHERE series_id = ?1 ORDER BY title", id)?;
    detail.authors = strings(
        connection,
        "SELECT p.name FROM series_people sp JOIN people p ON p.id = sp.person_id
         WHERE sp.series_id = ?1 AND sp.role = 'author' ORDER BY p.name",
        id,
    )?;
    detail.artists = strings(
        connection,
        "SELECT p.name FROM series_people sp JOIN people p ON p.id = sp.person_id
         WHERE sp.series_id = ?1 AND sp.role = 'artist' ORDER BY p.name",
        id,
    )?;
    detail.genres = strings(
        connection,
        "SELECT g.name FROM series_genres sg JOIN genres g ON g.id = sg.genre_id WHERE sg.series_id = ?1 ORDER BY g.name",
        id,
    )?;
    detail.tags = strings(
        connection,
        "SELECT t.name FROM series_tags st JOIN tags t ON t.id = st.tag_id WHERE st.series_id = ?1 ORDER BY t.name",
        id,
    )?;
    detail.collection_ids = strings(
        connection,
        "SELECT collection_id FROM collection_items WHERE series_id = ?1",
        id,
    )?;

    let mut statement = connection.prepare(
        "SELECT id, number, title, description, published_on FROM volumes WHERE series_id = ?1 ORDER BY sort_key, number",
    )?;
    let volumes = statement.query_map([id], |row| {
        Ok(VolumeRow {
            id: row.get(0)?,
            number: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            published_on: row.get(4)?,
        })
    })?;
    detail.volumes = volumes.collect::<rusqlite::Result<_>>()?;

    let sql = format!("SELECT {CHAPTER_COLUMNS} FROM chapters c {CHAPTER_JOINS} WHERE c.series_id = ?1 ORDER BY c.sort_key, c.created_at");
    let mut statement = connection.prepare(&sql)?;
    let chapters = statement.query_map([id], chapter_row)?;
    detail.chapters = chapters.collect::<rusqlite::Result<_>>()?;

    let mut statement = connection.prepare(
        "SELECT e.source_id, m.name, e.external_id, e.url FROM external_ids e
         JOIN metadata_sources m ON m.id = e.source_id WHERE e.series_id = ?1 ORDER BY m.name",
    )?;
    let links = statement.query_map([id], |row| {
        Ok(ExternalLink { source_id: row.get(0)?, source_name: row.get(1)?, external_id: row.get(2)?, url: row.get(3)? })
    })?;
    detail.external_links = links.collect::<rusqlite::Result<_>>()?;

    let mut statement = connection.prepare(
        "SELECT p.field, m.name, p.locked FROM field_provenance p
         JOIN metadata_sources m ON m.id = p.source_id
         WHERE p.entity_type = 'series' AND p.entity_id = ?1 ORDER BY p.field",
    )?;
    let sources = statement.query_map([id], |row| {
        Ok(FieldSource { field: row.get(0)?, source_name: row.get(1)?, locked: row.get(2)? })
    })?;
    detail.field_sources = sources.collect::<rusqlite::Result<_>>()?;

    // Reprise : le dernier chapitre commencé et non terminé, sinon le premier non
    // lu dans l'ordre de lecture, sinon le début.
    detail.has_progress = detail.chapters.iter().any(|c| c.percent.is_some());
    detail.resume_chapter_id = next_to_read(connection, id)?.or_else(|| {
        crate::reader::reading_order(connection, id).ok().and_then(|order| order.first().cloned())
    });
    Ok(detail)
}

/// Chapitre à reprendre : le dernier commencé et non terminé, sinon le premier
/// non lu dans l'ordre de lecture. `None` si tout est lu.
fn next_to_read(connection: &Connection, series_id: &str) -> Result<Option<String>> {
    let started: Option<String> = connection
        .query_row(
            "SELECT p.chapter_id FROM reading_progress p JOIN chapters c ON c.id = p.chapter_id
             WHERE c.series_id = ?1 AND p.completed_at IS NULL
             ORDER BY p.last_read_at DESC LIMIT 1",
            [series_id],
            |row| row.get(0),
        )
        .optional()?;
    if started.is_some() {
        return Ok(started);
    }
    let mut statement = connection.prepare_cached(
        "SELECT c.id FROM chapters c LEFT JOIN reading_progress p ON p.chapter_id = c.id
         WHERE c.series_id = ?1 AND p.completed_at IS NULL",
    )?;
    let unread: Vec<String> = statement.query_map([series_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let order = crate::reader::reading_order(connection, series_id)?;
    Ok(order.into_iter().find(|chapter| unread.contains(chapter)))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueItem {
    pub chapter: ChapterRow,
    pub cover_asset_id: Option<String>,
    pub last_read_at: String,
}

/// Séries lues récemment et leur prochain chapitre ; une série entièrement lue
/// n'y figure plus.
pub fn continue_reading(connection: &Connection, limit: usize) -> Result<Vec<ContinueItem>> {
    let mut statement = connection.prepare(
        "SELECT c.series_id, s.cover_asset_id, max(p.last_read_at) AS last
         FROM reading_progress p JOIN chapters c ON c.id = p.chapter_id JOIN series s ON s.id = c.series_id
         GROUP BY c.series_id ORDER BY last DESC LIMIT ?1",
    )?;
    let recent: Vec<(String, Option<String>, String)> = statement
        .query_map([limit as i64 * 2], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let sql = format!("SELECT {CHAPTER_COLUMNS} FROM chapters c {CHAPTER_JOINS} WHERE c.id = ?1");
    let mut items = Vec::new();
    for (series_id, cover_asset_id, last_read_at) in recent {
        let Some(chapter_id) = next_to_read(connection, &series_id)? else { continue };
        let chapter = connection.query_row(&sql, [&chapter_id], chapter_row)?;
        items.push(ContinueItem { chapter, cover_asset_id, last_read_at });
        if items.len() == limit {
            break;
        }
    }
    Ok(items)
}

fn strings(connection: &Connection, sql: &str, id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare_cached(sql)?;
    let rows = statement.query_map([id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Fichiers présents ou non, regroupés : utile pour le titre de la série.
pub fn cover_asset_path(connection: &Connection, id: &str) -> Result<Option<String>> {
    Ok(connection
        .query_row("SELECT path FROM assets WHERE id = ?1", [id], |row| row.get(0))
        .optional()?)
}

// Modification ------------------------------------------------------------------

pub fn update_series(connection: &mut Connection, id: &str, patch: &SeriesPatch) -> Result<()> {
    let title = patch.title.trim();
    if title.is_empty() {
        return Err(Error::Invalid("une série doit avoir un titre".into()));
    }
    let before = get_series(connection, id)?;
    let tx = connection.transaction()?;
    tx.execute(
        "UPDATE series SET title = ?1, sort_title = ?2, kind = ?3, status = ?4, description = ?5, publisher = ?6,
                published_from = ?7, published_to = ?8, original_language = ?9,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?10",
        params![
            title,
            sort_title(title),
            patch.kind,
            patch.status,
            blank_to_none(&patch.description),
            blank_to_none(&patch.publisher),
            blank_to_none(&patch.published_from),
            blank_to_none(&patch.published_to),
            blank_to_none(&patch.original_language),
            id,
        ],
    )?;

    tx.execute("DELETE FROM series_titles WHERE series_id = ?1", [id])?;
    for alt in clean_list(&patch.alt_titles) {
        tx.execute("INSERT OR IGNORE INTO series_titles (series_id, title) VALUES (?1, ?2)", params![id, alt])?;
    }
    tx.execute("DELETE FROM series_people WHERE series_id = ?1 AND role IN ('author', 'artist')", [id])?;
    for author in clean_list(&patch.authors) {
        link_person(&tx, id, &author, "author")?;
    }
    for artist in clean_list(&patch.artists) {
        link_person(&tx, id, &artist, "artist")?;
    }
    tx.execute("DELETE FROM series_genres WHERE series_id = ?1", [id])?;
    for genre in clean_list(&patch.genres) {
        link_named(&tx, "genres", "series_genres", "genre_id", id, &genre)?;
    }
    tx.execute("DELETE FROM series_tags WHERE series_id = ?1", [id])?;
    for tag in clean_list(&patch.tags) {
        link_named(&tx, "tags", "series_tags", "tag_id", id, &tag)?;
    }

    let changed: [(&str, bool); 13] = [
        ("title", before.title != title),
        ("kind", before.kind != patch.kind),
        ("status", before.status != patch.status),
        ("description", before.description != blank_to_none(&patch.description)),
        ("publisher", before.publisher != blank_to_none(&patch.publisher)),
        ("published_from", before.published_from != blank_to_none(&patch.published_from)),
        ("published_to", before.published_to != blank_to_none(&patch.published_to)),
        ("original_language", before.original_language != blank_to_none(&patch.original_language)),
        ("alt_titles", sorted(&before.alt_titles) != sorted(&clean_list(&patch.alt_titles))),
        ("authors", sorted(&before.authors) != sorted(&clean_list(&patch.authors))),
        ("artists", sorted(&before.artists) != sorted(&clean_list(&patch.artists))),
        ("genres", sorted(&before.genres) != sorted(&clean_list(&patch.genres))),
        ("tags", sorted(&before.tags) != sorted(&clean_list(&patch.tags))),
    ];
    for (field, _) in changed.iter().filter(|(_, changed)| *changed) {
        lock_field(&tx, "series", id, field)?;
    }
    tx.commit()?;
    Ok(())
}

fn sorted(items: &[String]) -> Vec<String> {
    let mut items = items.to_vec();
    items.sort();
    items
}

fn clean_list(items: &[String]) -> Vec<String> {
    let mut seen = Vec::new();
    for item in items {
        let item = item.trim();
        if !item.is_empty() && !seen.iter().any(|s: &String| s.eq_ignore_ascii_case(item)) {
            seen.push(item.to_string());
        }
    }
    seen
}

fn blank_to_none(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn lock_field(tx: &Transaction<'_>, entity: &str, id: &str, field: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO field_provenance (entity_type, entity_id, field, source_id, locked)
         VALUES (?1, ?2, ?3, 'manual', 1)
         ON CONFLICT (entity_type, entity_id, field) DO UPDATE SET
             source_id = 'manual', locked = 1, snapshot_id = NULL,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![entity, id, field],
    )?;
    Ok(())
}

pub fn link_person(tx: &Transaction<'_>, series_id: &str, name: &str, role: &str) -> Result<()> {
    let person = upsert_named(tx, "people", name)?;
    tx.execute(
        "INSERT OR IGNORE INTO series_people (series_id, person_id, role) VALUES (?1, ?2, ?3)",
        params![series_id, person, role],
    )?;
    Ok(())
}

/// `table`, `link` et `column` sont des constantes du code, jamais une saisie.
pub fn link_named(tx: &Transaction<'_>, table: &str, link: &str, column: &str, series_id: &str, name: &str) -> Result<()> {
    let id = upsert_named(tx, table, name)?;
    tx.execute(
        &format!("INSERT OR IGNORE INTO {link} (series_id, {column}) VALUES (?1, ?2)"),
        params![series_id, id],
    )?;
    Ok(())
}

fn upsert_named(tx: &Transaction<'_>, table: &str, name: &str) -> Result<String> {
    let existing: Option<String> = tx
        .query_row(&format!("SELECT id FROM {table} WHERE name = ?1"), [name], |row| row.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = Uuid::now_v7().to_string();
    tx.execute(&format!("INSERT INTO {table} (id, name) VALUES (?1, ?2)"), params![id, name])?;
    Ok(id)
}

pub fn update_chapter(connection: &mut Connection, id: &str, patch: &ChapterPatch) -> Result<()> {
    if patch.number.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Err(Error::Invalid("numéro invalide".into()));
    }
    let tx = connection.transaction()?;
    let (series_before, sort_before): (String, f64) = tx
        .query_row("SELECT series_id, sort_key FROM chapters WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .ok_or_else(|| Error::NotFound("chapitre".into()))?;
    let sort_key = patch.number.unwrap_or(sort_before);
    tx.execute(
        "UPDATE chapters SET number = ?1, title = ?2, volume_id = ?3, series_id = ?4, sort_key = ?5 WHERE id = ?6",
        params![
            patch.number,
            patch.title.as_deref().map(str::trim).filter(|t| !t.is_empty()),
            patch.volume_id,
            patch.series_id,
            sort_key,
            id,
        ],
    )?;
    for field in ["number", "title"] {
        lock_field(&tx, "chapter", id, field)?;
    }
    if series_before != patch.series_id {
        touch_series(&tx, &series_before)?;
    }
    touch_series(&tx, &patch.series_id)?;
    tx.commit()?;
    Ok(())
}

fn touch_series(tx: &Transaction<'_>, id: &str) -> Result<()> {
    tx.execute("UPDATE series SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1", [id])?;
    Ok(())
}

pub fn create_volume(connection: &mut Connection, series_id: &str, patch: &VolumePatch) -> Result<String> {
    let id = Uuid::now_v7().to_string();
    let tx = connection.transaction()?;
    write_volume(&tx, &id, Some(series_id), patch)?;
    tx.commit()?;
    Ok(id)
}

pub fn update_volume(connection: &mut Connection, id: &str, patch: &VolumePatch) -> Result<()> {
    let tx = connection.transaction()?;
    write_volume(&tx, id, None, patch)?;
    for field in ["number", "title", "description", "published_on"] {
        lock_field(&tx, "volume", id, field)?;
    }
    tx.commit()?;
    Ok(())
}

fn write_volume(tx: &Transaction<'_>, id: &str, new_series: Option<&str>, patch: &VolumePatch) -> Result<()> {
    if patch.number.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Err(Error::Invalid("numéro invalide".into()));
    }
    let sort_key = patch.number.unwrap_or(f64::from(i32::MAX));
    let title = blank_to_none(&patch.title);
    let description = blank_to_none(&patch.description);
    let published_on = blank_to_none(&patch.published_on);
    match new_series {
        Some(series_id) => {
            tx.execute(
                "INSERT INTO volumes (id, series_id, number, title, description, published_on, sort_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, series_id, patch.number, title, description, published_on, sort_key],
            )?;
        }
        None => {
            let changed = tx.execute(
                "UPDATE volumes SET number = ?1, title = ?2, description = ?3, published_on = ?4, sort_key = ?5 WHERE id = ?6",
                params![patch.number, title, description, published_on, sort_key, id],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("volume".into()));
            }
        }
    }
    Ok(())
}

pub fn delete_volume(connection: &mut Connection, id: &str) -> Result<()> {
    // Les chapitres restent dans la série, sans volume (ON DELETE SET NULL).
    connection.execute("DELETE FROM volumes WHERE id = ?1", [id])?;
    Ok(())
}

/// Retire de la bibliothèque, sans jamais toucher aux fichiers référencés.
pub fn delete_chapter(connection: &mut Connection, storage: &Storage, id: &str) -> Result<()> {
    connection.execute("DELETE FROM chapters WHERE id = ?1", [id])?;
    cleanup_orphans(connection, storage)
}

pub fn delete_series(connection: &mut Connection, storage: &Storage, id: &str) -> Result<()> {
    connection.execute("DELETE FROM series WHERE id = ?1", [id])?;
    cleanup_orphans(connection, storage)
}

/// Sources et couvertures qui ne servent plus. Seules les copies gérées par
/// Komago sont effacées du disque ; un fichier référencé n'est jamais supprimé.
pub fn cleanup_orphans(connection: &mut Connection, storage: &Storage) -> Result<()> {
    let tx = connection.transaction()?;
    let orphans: Vec<(String, String, String)> = {
        let mut statement = tx.prepare(
            "SELECT id, path, storage FROM source_files
             WHERE NOT EXISTS (SELECT 1 FROM chapters c WHERE c.source_id = source_files.id)",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    tx.execute(
        "DELETE FROM source_files WHERE NOT EXISTS (SELECT 1 FROM chapters c WHERE c.source_id = source_files.id)",
        [],
    )?;
    let assets: Vec<(String, String)> = {
        let mut statement = tx.prepare(
            "SELECT id, path FROM assets
             WHERE NOT EXISTS (SELECT 1 FROM series s WHERE s.cover_asset_id = assets.id)
               AND NOT EXISTS (SELECT 1 FROM volumes v WHERE v.cover_asset_id = assets.id)",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (id, _) in &assets {
        tx.execute("DELETE FROM assets WHERE id = ?1", [id])?;
    }
    tx.execute("DELETE FROM people WHERE NOT EXISTS (SELECT 1 FROM series_people sp WHERE sp.person_id = people.id)", [])?;
    tx.execute("DELETE FROM genres WHERE NOT EXISTS (SELECT 1 FROM series_genres sg WHERE sg.genre_id = genres.id)", [])?;
    tx.execute("DELETE FROM tags WHERE NOT EXISTS (SELECT 1 FROM series_tags st WHERE st.tag_id = tags.id)", [])?;
    tx.commit()?;

    // Après validation : un fichier n'est effacé qu'une fois la base à jour.
    for (_, path, kind) in orphans {
        if kind == "managed" {
            remove_inside(Path::new(&path), &storage.library_dir());
        }
    }
    for (_, relative) in assets {
        if let Some(name) = Path::new(&relative).file_name() {
            let _ = fs::remove_file(storage.covers_dir().join(name));
        }
    }
    Ok(())
}

fn remove_inside(path: &Path, root: &Path) {
    if !path.starts_with(root) || path == root {
        return;
    }
    let _ = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
}

pub fn set_metadata_mode(connection: &mut Connection, id: &str, mode: Option<&str>) -> Result<()> {
    if mode.is_some_and(|m| !["automatic", "assisted", "manual"].contains(&m)) {
        return Err(Error::Invalid("mode de métadonnées inconnu".into()));
    }
    connection.execute("UPDATE series SET metadata_mode = ?1 WHERE id = ?2", params![mode, id])?;
    Ok(())
}

pub fn set_series_cover(connection: &mut Connection, storage: &Storage, series_id: &str, asset_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE series SET cover_asset_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
        params![asset_id, series_id],
    )?;
    let tx = connection.transaction()?;
    lock_field(&tx, "series", series_id, "cover")?;
    tx.commit()?;
    cleanup_orphans(connection, storage)
}

/// Première page d'un chapitre : source, type et entrée.
pub fn first_page(connection: &Connection, chapter_id: &str) -> Result<(String, String, String)> {
    connection
        .query_row(
            "SELECT f.path, f.kind, p.entry_name FROM pages p
             JOIN chapters c ON c.id = p.chapter_id
             JOIN source_files f ON f.id = c.source_id
             WHERE p.chapter_id = ?1 ORDER BY p.page_index LIMIT 1",
            [chapter_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound("page du chapitre".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::{tempdir, TempDir};

    fn setup() -> (TempDir, Database, Storage) {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().join("data"), dir.path().join("cache")).unwrap();
        let database = Database::open(&storage.database_path()).unwrap();
        (dir, database, storage)
    }

    fn add_series(database: &Database, id: &str, title: &str) {
        database
            .write(|c| {
                c.execute(
                    "INSERT INTO series (id, title, sort_title) VALUES (?1, ?2, ?3)",
                    params![id, title, sort_title(title)],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn titles(database: &Database, filter: &SeriesFilter) -> Vec<String> {
        database
            .read(|c| list_series(c, filter))
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect()
    }

    fn search(database: &Database, query: &str) -> Vec<String> {
        titles(database, &SeriesFilter { query: Some(query.into()), ..Default::default() })
    }

    fn patch(title: &str) -> SeriesPatch {
        SeriesPatch {
            title: title.into(),
            kind: "manga".into(),
            status: "ongoing".into(),
            description: None,
            publisher: None,
            published_from: None,
            published_to: None,
            original_language: None,
            alt_titles: vec![],
            authors: vec![],
            artists: vec![],
            genres: vec![],
            tags: vec![],
        }
    }

    /// Mesure à la demande : `cargo test --release mesure -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn mesure_grande_bibliotheque() {
        let (_d, database, _s) = setup();
        let start = std::time::Instant::now();
        database
            .write(|c| {
                let tx = c.transaction()?;
                for s in 0..5_000 {
                    let id = format!("s{s}");
                    let title = format!("Série numéro {s} du grand catalogue");
                    tx.execute(
                        "INSERT INTO series (id, title, sort_title) VALUES (?1, ?2, ?3)",
                        params![id, title, sort_title(&title)],
                    )?;
                    for n in 0..40 {
                        let chapter = format!("{id}-c{n}");
                        tx.execute(
                            "INSERT INTO chapters (id, series_id, number, page_count, sort_key) VALUES (?1, ?2, ?3, 20, ?3)",
                            params![chapter, id, n],
                        )?;
                        if n < 5 {
                            tx.execute(
                                "INSERT INTO reading_progress (chapter_id, percent, completed_at, last_read_at) VALUES (?1, 100, 'x', 'x')",
                                [&chapter],
                            )?;
                        }
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();
        println!("création : {:?}", start.elapsed());

        let timed = |label: &str, work: &dyn Fn()| {
            let start = std::time::Instant::now();
            work();
            println!("{label} : {:?}", start.elapsed());
        };
        for (label, sort) in [("titre", SeriesSort::Title), ("chapitres", SeriesSort::Chapters), ("lecture", SeriesSort::Read)] {
            let filter = SeriesFilter { sort, ..Default::default() };
            timed(&format!("liste, tri {label}"), &|| {
                assert_eq!(database.read(|c| list_series(c, &filter)).unwrap().len(), 5_000);
            });
        }
        let search = SeriesFilter { query: Some("numéro 4242".into()), ..Default::default() };
        timed("recherche", &|| {
            assert!(!database.read(|c| list_series(c, &search)).unwrap().is_empty());
        });
        timed("fiche de série", &|| {
            assert_eq!(database.read(|c| get_series(c, "s2500")).unwrap().chapters.len(), 40);
        });
        timed("recherche de chapitres", &|| {
            database.read(|c| search_chapters(c, "introuvable")).unwrap();
        });
    }

    #[test]
    fn sorting_ignores_accents_and_articles() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "Zéro");
        add_series(&database, "2", "Éclipse");
        add_series(&database, "3", "The Beast");
        assert_eq!(titles(&database, &SeriesFilter::default()), ["The Beast", "Éclipse", "Zéro"]);
    }

    #[test]
    fn numbered_titles_sort_naturally() {
        let (_d, database, _s) = setup();
        for (id, title) in [("1", "Série 10"), ("2", "Série 2"), ("3", "Série 100")] {
            add_series(&database, id, title);
        }
        assert_eq!(titles(&database, &SeriesFilter::default()), ["Série 2", "Série 10", "Série 100"]);
    }

    #[test]
    fn search_handles_prefixes_accents_operators_and_cjk() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "Éclipse Totale");
        add_series(&database, "2", "進撃の巨人");
        add_series(&database, "3", "100% Réel");

        assert_eq!(search(&database, "ecli"), ["Éclipse Totale"]);
        assert_eq!(search(&database, "tota ecl"), ["Éclipse Totale"]);
        assert_eq!(search(&database, "巨人"), ["進撃の巨人"]);
        assert_eq!(search(&database, "100%"), ["100% Réel"]);
        // Une saisie qui ressemble à la syntaxe FTS n'est ni une erreur, ni un opérateur.
        for hostile in ["\"", "NOT", "a OR", "title:x", "(", "*", "%", "_"] {
            database
                .read(|c| list_series(c, &SeriesFilter { query: Some(hostile.into()), ..Default::default() }))
                .unwrap_or_else(|e| panic!("« {hostile} » : {e}"));
        }
        assert!(search(&database, "_").is_empty(), "le joker LIKE est échappé");
    }

    #[test]
    fn search_finds_alt_titles_and_people_after_an_edit() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "Shingeki");
        let mut edited = patch("Shingeki");
        edited.alt_titles = vec!["Attack on Titan".into()];
        edited.authors = vec!["Isayama".into()];
        database.write(|c| update_series(c, "1", &edited)).unwrap();

        assert_eq!(search(&database, "titan"), ["Shingeki"]);
        assert_eq!(search(&database, "isaya"), ["Shingeki"]);
    }

    #[test]
    fn filters_combine() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "A");
        add_series(&database, "2", "B");
        database.write(|c| update_series(c, "2", &patch("B"))).unwrap();

        let manga = SeriesFilter { kind: Some("manga".into()), ..Default::default() };
        assert_eq!(titles(&database, &manga), ["B"]);
        let none = SeriesFilter { kind: Some("manga".into()), status: Some("completed".into()), ..Default::default() };
        assert!(titles(&database, &none).is_empty());
    }

    #[test]
    fn manual_edits_lock_only_what_changed() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "Avant");
        let mut edited = patch("Avant");
        edited.kind = "unknown".into();
        edited.status = "unknown".into();
        edited.description = Some("Résumé".into());
        database.write(|c| update_series(c, "1", &edited)).unwrap();

        let locked: Vec<String> = database
            .read(|c| {
                let mut s = c.prepare("SELECT field FROM field_provenance WHERE entity_id = '1' AND locked = 1")?;
                let rows = s.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .unwrap();
        assert_eq!(locked, ["description"]);
    }

    #[test]
    fn resume_prefers_the_started_chapter_then_the_first_unread() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "A");
        database
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO chapters (id, series_id, page_count, sort_key) VALUES ('a', '1', 5, 1), ('b', '1', 5, 2), ('c', '1', 5, 3);",
                )?;
                Ok(())
            })
            .unwrap();
        let resume = |db: &Database| db.read(|c| get_series(c, "1")).unwrap().resume_chapter_id.unwrap();
        assert_eq!(resume(&database), "a");

        database.write(|c| crate::reader::set_read(c, "a", true)).unwrap();
        assert_eq!(resume(&database), "b");

        let update = crate::reader::ProgressUpdate {
            chapter_id: "c".into(),
            history_id: None,
            page_index: 2,
            page_offset: 0.0,
            percent: 40.0,
        };
        database.write(|c| crate::reader::save_progress(c, &update)).unwrap();
        assert_eq!(resume(&database), "c");

        let detail = database.read(|c| get_series(c, "1")).unwrap();
        assert!(detail.has_progress);
        assert!(detail.chapters[0].completed);
        assert_eq!(detail.chapters[2].percent, Some(40.0));
    }

    #[test]
    fn continue_reading_offers_the_next_chapter_and_drops_finished_series() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "A");
        add_series(&database, "2", "B");
        database
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO chapters (id, series_id, page_count, sort_key) VALUES
                        ('a1', '1', 5, 1), ('a2', '1', 5, 2), ('b1', '2', 5, 1);",
                )?;
                Ok(())
            })
            .unwrap();
        let ids = |db: &Database| -> Vec<String> {
            db.read(|c| continue_reading(c, 10)).unwrap().into_iter().map(|i| i.chapter.id).collect()
        };
        assert!(ids(&database).is_empty(), "rien de commencé");

        database.write(|c| crate::reader::set_read(c, "a1", true)).unwrap();
        assert_eq!(ids(&database), ["a2"], "chapitre suivant après un chapitre terminé");

        database.write(|c| crate::reader::set_read(c, "b1", true)).unwrap();
        assert_eq!(ids(&database), ["a2"], "B est entièrement lue");
    }

    #[test]
    fn a_title_cannot_be_emptied() {
        let (_d, database, _s) = setup();
        add_series(&database, "1", "A");
        assert!(database.write(|c| update_series(c, "1", &patch("   "))).is_err());
    }

    #[test]
    fn deleting_a_series_keeps_referenced_files_but_removes_managed_copies() {
        let (dir, database, storage) = setup();
        let referenced = dir.path().join("original.cbz");
        fs::write(&referenced, b"x").unwrap();
        let managed = storage.library_dir().join("copy.cbz");
        fs::write(&managed, b"x").unwrap();
        add_series(&database, "1", "A");
        database
            .write(|c| {
                c.execute(
                    "INSERT INTO source_files (id, path, kind, storage, size_bytes) VALUES ('r', ?1, 'cbz', 'referenced', 1), ('m', ?2, 'cbz', 'managed', 1)",
                    params![referenced.to_string_lossy(), managed.to_string_lossy()],
                )?;
                c.execute_batch(
                    "INSERT INTO chapters (id, series_id, source_id) VALUES ('c1', '1', 'r'), ('c2', '1', 'm');",
                )?;
                Ok(())
            })
            .unwrap();

        database.write(|c| delete_series(c, &storage, "1")).unwrap();

        assert!(referenced.exists(), "un fichier référencé n'est jamais supprimé");
        assert!(!managed.exists());
        let sources: i64 = database.read(|c| Ok(c.query_row("SELECT count(*) FROM source_files", [], |r| r.get(0))?)).unwrap();
        assert_eq!(sources, 0);
    }

    #[test]
    fn managed_cleanup_never_leaves_the_library_folder() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("library");
        fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.cbz");
        fs::write(&outside, b"x").unwrap();
        remove_inside(&outside, &root);
        remove_inside(&root, &root);
        assert!(outside.exists() && root.exists());
    }

    #[test]
    fn chapter_edit_validates_the_volume_and_locks_fields() {
        let (_d, database, _s) = setup();
        add_series(&database, "a", "A");
        add_series(&database, "b", "B");
        database
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO volumes (id, series_id) VALUES ('v', 'a');
                     INSERT INTO chapters (id, series_id) VALUES ('c', 'a');",
                )?;
                Ok(())
            })
            .unwrap();
        let foreign = ChapterPatch { number: Some(3.0), title: None, volume_id: Some("v".into()), series_id: "b".into() };
        assert!(database.write(|c| update_chapter(c, "c", &foreign)).is_err());

        let valid = ChapterPatch { number: Some(3.0), title: Some(" T ".into()), volume_id: Some("v".into()), series_id: "a".into() };
        database.write(|c| update_chapter(c, "c", &valid)).unwrap();
        let (title, sort): (String, f64) = database
            .read(|c| Ok(c.query_row("SELECT title, sort_key FROM chapters", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
            .unwrap();
        assert_eq!((title.as_str(), sort), ("T", 3.0));
    }
}
