//! Ouverture des chapitres et progression de lecture.

use std::io::Cursor;
use std::path::Path;

use image::ImageReader;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::archive::{SourceKind, SourceReader};
use crate::db::Database;
use crate::error::{Error, Result};

/// Assez pour trouver les dimensions même après de grosses métadonnées EXIF.
const HEADER_BYTES: usize = 256 * 1024;
/// Au-delà, le chapitre est considéré comme lu : la dernière page peut ne pas
/// atteindre exactement 100 % selon la taille de la fenêtre.
const COMPLETED_PERCENT: f64 = 99.5;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub page_index: i64,
    pub page_offset: f64,
    pub percent: f64,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderChapter {
    pub id: String,
    pub series_id: String,
    pub series_title: String,
    pub direction: Option<String>,
    /// Double page : appariement décalé d'une page (tome sans couverture).
    pub spread_offset: bool,
    pub number: Option<f64>,
    pub title: Option<String>,
    pub source_status: Option<String>,
    pub pages: Vec<PageInfo>,
    pub progress: Option<Progress>,
    pub previous_chapter_id: Option<String>,
    pub next_chapter_id: Option<String>,
    pub history_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressUpdate {
    pub chapter_id: String,
    pub history_id: Option<String>,
    pub page_index: i64,
    pub page_offset: f64,
    pub percent: f64,
}

pub fn source_kind(kind: &str) -> SourceKind {
    match kind {
        "directory" => SourceKind::Directory,
        "cb7" => SourceKind::SevenZip,
        "cbr" => SourceKind::Rar,
        _ => SourceKind::Zip,
    }
}

/// Source et entrée d'une page.
pub fn page_location(connection: &Connection, chapter_id: &str, index: i64) -> Result<Option<(String, String, String)>> {
    Ok(connection
        .prepare_cached(
            "SELECT f.path, f.kind, p.entry_name FROM pages p
             JOIN chapters c ON c.id = p.chapter_id
             JOIN source_files f ON f.id = c.source_id
             WHERE p.chapter_id = ?1 AND p.page_index = ?2",
        )?
        .query_row(params![chapter_id, index], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()?)
}

/// Ordre de lecture : volumes dans l'ordre, puis chapitres hors volume.
pub fn reading_order(connection: &Connection, series_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare_cached(
        "SELECT c.id FROM chapters c LEFT JOIN volumes v ON v.id = c.volume_id
         WHERE c.series_id = ?1
         ORDER BY v.id IS NULL, v.sort_key, v.id, c.sort_key, c.created_at, c.id",
    )?;
    let rows = statement.query_map([series_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn open_chapter(database: &Database, id: &str) -> Result<ReaderChapter> {
    fill_dimensions(database, id);

    let history_id = Uuid::now_v7().to_string();
    database.write(|connection| {
        connection.execute(
            "INSERT INTO reading_history (id, chapter_id, started_at) VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![history_id, id],
        )?;
        Ok(())
    })?;

    database.read(|connection| {
        let mut chapter = connection
            .query_row(
                "SELECT c.series_id, s.title, s.reading_direction, c.number, c.title, f.status, s.spread_offset
                 FROM chapters c JOIN series s ON s.id = c.series_id
                 LEFT JOIN source_files f ON f.id = c.source_id
                 WHERE c.id = ?1",
                [id],
                |row| {
                    Ok(ReaderChapter {
                        id: id.to_string(),
                        series_id: row.get(0)?,
                        series_title: row.get(1)?,
                        direction: row.get(2)?,
                        number: row.get(3)?,
                        title: row.get(4)?,
                        source_status: row.get(5)?,
                        spread_offset: row.get(6)?,
                        pages: Vec::new(),
                        progress: None,
                        previous_chapter_id: None,
                        next_chapter_id: None,
                        history_id: history_id.clone(),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("chapitre".into()))?;

        let mut statement = connection.prepare_cached("SELECT width, height FROM pages WHERE chapter_id = ?1 ORDER BY page_index")?;
        let pages = statement.query_map([id], |row| Ok(PageInfo { width: row.get(0)?, height: row.get(1)? }))?;
        chapter.pages = pages.collect::<rusqlite::Result<_>>()?;

        chapter.progress = connection
            .query_row(
                "SELECT page_index, page_offset, percent, completed_at IS NOT NULL FROM reading_progress WHERE chapter_id = ?1",
                [id],
                |row| {
                    Ok(Progress {
                        page_index: row.get(0)?,
                        page_offset: row.get(1)?,
                        percent: row.get(2)?,
                        completed: row.get(3)?,
                    })
                },
            )
            .optional()?;

        let order = reading_order(connection, &chapter.series_id)?;
        if let Some(position) = order.iter().position(|c| c == id) {
            chapter.previous_chapter_id = position.checked_sub(1).and_then(|p| order.get(p)).cloned();
            chapter.next_chapter_id = order.get(position + 1).cloned();
        }
        Ok(chapter)
    })
}

/// Lit les dimensions manquantes dans les en-têtes, une fois pour toutes. Le
/// lecteur peut ainsi réserver la place des pages avant de les charger. Tout
/// échec est sans conséquence : la vraie taille sera connue au chargement.
fn fill_dimensions(database: &Database, chapter_id: &str) {
    let missing = database.read(|connection| {
        let mut statement = connection.prepare(
            "SELECT p.page_index, p.entry_name, f.path, f.kind FROM pages p
             JOIN chapters c ON c.id = p.chapter_id
             JOIN source_files f ON f.id = c.source_id
             WHERE p.chapter_id = ?1 AND p.width IS NULL
             ORDER BY p.page_index",
        )?;
        let rows = statement.query_map([chapter_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    });
    let Ok(missing) = missing else { return };
    let Some((_, _, path, kind)) = missing.first().cloned() else { return };
    let Ok(mut reader) = SourceReader::open(Path::new(&path), source_kind(&kind)) else { return };

    let mut found = Vec::new();
    for (index, entry, _, _) in &missing {
        if let Some((width, height)) = reader.read_head(entry, HEADER_BYTES).ok().and_then(|head| dimensions(&head)) {
            found.push((*index, width, height));
        }
    }
    if found.is_empty() {
        return;
    }
    let _ = database.write(|connection| {
        let tx = connection.transaction()?;
        for (index, width, height) in &found {
            tx.execute(
                "UPDATE pages SET width = ?1, height = ?2 WHERE chapter_id = ?3 AND page_index = ?4",
                params![width, height, chapter_id, index],
            )?;
        }
        tx.commit()?;
        Ok(())
    });
}

fn dimensions(head: &[u8]) -> Option<(u32, u32)> {
    ImageReader::new(Cursor::new(head)).with_guessed_format().ok()?.into_dimensions().ok()
}

pub fn save_progress(connection: &mut Connection, update: &ProgressUpdate) -> Result<()> {
    if !update.page_offset.is_finite() || !update.percent.is_finite() {
        return Err(Error::Invalid("progression invalide".into()));
    }
    let tx = connection.transaction()?;
    let (series_id, page_count): (String, i64) = tx
        .query_row("SELECT series_id, page_count FROM chapters WHERE id = ?1", [&update.chapter_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .ok_or_else(|| Error::NotFound("chapitre".into()))?;

    let page = update.page_index.clamp(0, (page_count - 1).max(0));
    let offset = update.page_offset.clamp(0.0, 1.0);
    let percent = update.percent.clamp(0.0, 100.0);
    let completes = percent >= COMPLETED_PERCENT;

    let (is_new, was_completed): (bool, bool) = tx
        .query_row(
            "SELECT completed_at IS NOT NULL FROM reading_progress WHERE chapter_id = ?1",
            [&update.chapter_id],
            |r| Ok((false, r.get(0)?)),
        )
        .optional()?
        .unwrap_or((true, false));

    // Relire un chapitre terminé ne le fait pas redevenir « non lu ».
    tx.execute(
        "INSERT INTO reading_progress (chapter_id, page_index, page_offset, percent, completed_at, last_read_at)
         VALUES (?1, ?2, ?3, ?4,
                 CASE WHEN ?5 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT (chapter_id) DO UPDATE SET
             page_index = excluded.page_index,
             page_offset = excluded.page_offset,
             percent = excluded.percent,
             completed_at = coalesce(completed_at, excluded.completed_at),
             last_read_at = excluded.last_read_at",
        params![update.chapter_id, page, offset, percent, completes],
    )?;

    if let Some(history_id) = &update.history_id {
        tx.execute(
            "UPDATE reading_history
             SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), first_page = coalesce(first_page, ?1), last_page = ?1
             WHERE id = ?2 AND chapter_id = ?3",
            params![page, history_id, update.chapter_id],
        )?;
    }

    if is_new {
        start_reading_state(&tx, &series_id)?;
    }
    if completes && !was_completed {
        finish_reading_state(&tx, &series_id)?;
    }
    tx.commit()?;
    Ok(())
}

/// Marque lu ou non lu sans passer par le lecteur.
pub fn set_read(connection: &mut Connection, chapter_id: &str, read: bool) -> Result<()> {
    if read {
        let pages: i64 = connection
            .query_row("SELECT page_count FROM chapters WHERE id = ?1", [chapter_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound("chapitre".into()))?;
        save_progress(
            connection,
            &ProgressUpdate {
                chapter_id: chapter_id.to_string(),
                history_id: None,
                page_index: (pages - 1).max(0),
                page_offset: 0.0,
                percent: 100.0,
            },
        )
    } else {
        connection.execute("DELETE FROM reading_progress WHERE chapter_id = ?1", [chapter_id])?;
        Ok(())
    }
}

pub fn set_spread_offset(connection: &mut Connection, series_id: &str, offset: bool) -> Result<()> {
    connection.execute("UPDATE series SET spread_offset = ?1 WHERE id = ?2", params![offset, series_id])?;
    Ok(())
}

pub fn set_series_direction(connection: &mut Connection, series_id: &str, direction: &str) -> Result<()> {
    if !["ltr", "rtl", "vertical"].contains(&direction) {
        return Err(Error::Invalid("sens de lecture inconnu".into()));
    }
    connection.execute("UPDATE series SET reading_direction = ?1 WHERE id = ?2", params![direction, series_id])?;
    Ok(())
}

fn has_reading_state(tx: &Transaction<'_>, series_id: &str) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM collection_items i JOIN collections c ON c.id = i.collection_id
                        WHERE i.series_id = ?1 AND c.system_key IN ('reading', 'completed', 'dropped'))",
        [series_id],
        |r| r.get(0),
    )?)
}

fn insert_state(tx: &Transaction<'_>, series_id: &str, collection_id: &str) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO collection_items (id, collection_id, series_id, position)
         VALUES (?1, ?2, ?3, (SELECT coalesce(max(position), 0) + 1 FROM collection_items WHERE collection_id = ?2))",
        params![Uuid::now_v7().to_string(), collection_id, series_id],
    )?;
    Ok(())
}

/// Première lecture d'un chapitre : la série passe « En cours », sauf si un état
/// existe déjà, en particulier un choix manuel.
fn start_reading_state(tx: &Transaction<'_>, series_id: &str) -> Result<()> {
    if !has_reading_state(tx, series_id)? {
        insert_state(tx, series_id, "system-reading")?;
    }
    Ok(())
}

/// Tout est lu : la série passe « Terminés », sauf si elle a été abandonnée.
fn finish_reading_state(tx: &Transaction<'_>, series_id: &str) -> Result<()> {
    let unread: i64 = tx.query_row(
        "SELECT count(*) FROM chapters c
         LEFT JOIN reading_progress p ON p.chapter_id = c.id
         WHERE c.series_id = ?1 AND p.completed_at IS NULL",
        [series_id],
        |r| r.get(0),
    )?;
    let dropped: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM collection_items WHERE series_id = ?1 AND collection_id = 'system-dropped')",
        [series_id],
        |r| r.get(0),
    )?;
    if unread > 0 || dropped {
        return Ok(());
    }
    tx.execute(
        "DELETE FROM collection_items WHERE series_id = ?1 AND collection_id = 'system-reading'",
        [series_id],
    )?;
    insert_state(tx, series_id, "system-completed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::test_support::write_zip;
    use crate::covers::test_support::png;
    use tempfile::{tempdir, TempDir};

    fn setup() -> (TempDir, Database) {
        let dir = tempdir().unwrap();
        let database = Database::open(&dir.path().join("komago.db")).unwrap();
        database
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO series (id, title, sort_title) VALUES ('s', 'S', 's');
                     INSERT INTO volumes (id, series_id, number, sort_key) VALUES ('v2', 's', 2, 2), ('v1', 's', 1, 1);
                     INSERT INTO chapters (id, series_id, volume_id, page_count, sort_key) VALUES
                        ('loose', 's', NULL, 10, 0),
                        ('c3', 's', 'v2', 10, 3),
                        ('c1', 's', 'v1', 10, 1),
                        ('c2', 's', 'v1', 10, 2);",
                )?;
                Ok(())
            })
            .unwrap();
        (dir, database)
    }

    fn save(database: &Database, chapter: &str, page: i64, offset: f64, percent: f64) -> Result<()> {
        database.write(|c| {
            save_progress(
                c,
                &ProgressUpdate {
                    chapter_id: chapter.into(),
                    history_id: None,
                    page_index: page,
                    page_offset: offset,
                    percent,
                },
            )
        })
    }

    fn states(database: &Database) -> Vec<String> {
        database
            .read(|c| {
                let mut s = c.prepare(
                    "SELECT k.system_key FROM collection_items i JOIN collections k ON k.id = i.collection_id WHERE i.series_id = 's' ORDER BY 1",
                )?;
                let rows = s.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .unwrap()
    }

    #[test]
    fn reading_order_follows_volumes_then_loose_chapters() {
        let (_d, database) = setup();
        assert_eq!(database.read(|c| reading_order(c, "s")).unwrap(), ["c1", "c2", "c3", "loose"]);
    }

    #[test]
    fn opening_gives_neighbours_and_starts_a_history_entry() {
        let (_d, database) = setup();
        let chapter = open_chapter(&database, "c2").unwrap();
        assert_eq!(chapter.previous_chapter_id.as_deref(), Some("c1"));
        assert_eq!(chapter.next_chapter_id.as_deref(), Some("c3"));
        assert!(chapter.progress.is_none());
        assert_eq!(open_chapter(&database, "c1").unwrap().previous_chapter_id, None);
        assert_eq!(open_chapter(&database, "loose").unwrap().next_chapter_id, None);

        let entries: i64 = database.read(|c| Ok(c.query_row("SELECT count(*) FROM reading_history", [], |r| r.get(0))?)).unwrap();
        assert_eq!(entries, 3);
        assert!(open_chapter(&database, "inconnu").is_err());
    }

    #[test]
    fn exact_position_survives_closing_the_database() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        {
            let database = Database::open(&path).unwrap();
            database
                .write(|c| {
                    c.execute_batch(
                        "INSERT INTO series (id, title, sort_title) VALUES ('s', 'S', 's');
                         INSERT INTO chapters (id, series_id, page_count) VALUES ('c', 's', 40);",
                    )?;
                    Ok(())
                })
                .unwrap();
            save(&database, "c", 17, 0.375, 43.0).unwrap();
        }
        let database = Database::open(&path).unwrap();
        let progress = open_chapter(&database, "c").unwrap().progress.unwrap();
        assert_eq!(progress.page_index, 17);
        assert_eq!(progress.page_offset, 0.375);
        assert_eq!(progress.percent, 43.0);
        assert!(!progress.completed);
    }

    #[test]
    fn out_of_range_values_are_clamped_and_nan_refused() {
        let (_d, database) = setup();
        save(&database, "c1", 500, 4.0, 900.0).unwrap();
        let progress = open_chapter(&database, "c1").unwrap().progress.unwrap();
        assert_eq!((progress.page_index, progress.page_offset, progress.percent), (9, 1.0, 100.0));

        save(&database, "c2", -3, -1.0, -5.0).unwrap();
        let progress = open_chapter(&database, "c2").unwrap().progress.unwrap();
        assert_eq!((progress.page_index, progress.page_offset, progress.percent), (0, 0.0, 0.0));

        assert!(save(&database, "c1", 0, f64::NAN, 0.0).is_err());
        assert!(save(&database, "c1", 0, 0.0, f64::INFINITY).is_err());
        assert!(save(&database, "inconnu", 0, 0.0, 0.0).is_err());
    }

    #[test]
    fn rereading_keeps_the_chapter_completed() {
        let (_d, database) = setup();
        save(&database, "c1", 9, 0.0, 100.0).unwrap();
        save(&database, "c1", 2, 0.0, 30.0).unwrap();
        let progress = open_chapter(&database, "c1").unwrap().progress.unwrap();
        assert!(progress.completed);
        assert_eq!(progress.page_index, 2);
    }

    #[test]
    fn reading_states_follow_progress() {
        let (_d, database) = setup();
        save(&database, "c1", 1, 0.0, 20.0).unwrap();
        assert_eq!(states(&database), ["reading"]);
        for chapter in ["c1", "c2", "c3"] {
            save(&database, chapter, 9, 0.0, 100.0).unwrap();
        }
        assert_eq!(states(&database), ["reading"], "il reste un chapitre");
        save(&database, "loose", 9, 0.0, 100.0).unwrap();
        assert_eq!(states(&database), ["completed"]);
    }

    #[test]
    fn a_dropped_series_stays_dropped() {
        let (_d, database) = setup();
        database.write(|c| crate::collections::add_series(c, "system-dropped", "s")).unwrap();
        for chapter in ["c1", "c2", "c3", "loose"] {
            save(&database, chapter, 9, 0.0, 100.0).unwrap();
        }
        assert_eq!(states(&database), ["dropped"]);
    }

    #[test]
    fn a_manual_completed_state_is_not_replaced_by_rereading() {
        let (_d, database) = setup();
        database.write(|c| crate::collections::add_series(c, "system-completed", "s")).unwrap();
        save(&database, "c1", 1, 0.0, 10.0).unwrap();
        assert_eq!(states(&database), ["completed"]);
    }

    #[test]
    fn marking_read_and_unread() {
        let (_d, database) = setup();
        database.write(|c| set_read(c, "c1", true)).unwrap();
        assert!(open_chapter(&database, "c1").unwrap().progress.unwrap().completed);
        database.write(|c| set_read(c, "c1", false)).unwrap();
        assert!(open_chapter(&database, "c1").unwrap().progress.is_none());
    }

    #[test]
    fn history_records_the_pages_of_a_session() {
        let (_d, database) = setup();
        let history = open_chapter(&database, "c1").unwrap().history_id;
        for page in [2, 5] {
            database
                .write(|c| {
                    save_progress(
                        c,
                        &ProgressUpdate {
                            chapter_id: "c1".into(),
                            history_id: Some(history.clone()),
                            page_index: page,
                            page_offset: 0.0,
                            percent: 50.0,
                        },
                    )
                })
                .unwrap();
        }
        let (first, last): (i64, i64) = database
            .read(|c| Ok(c.query_row("SELECT first_page, last_page FROM reading_history", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
            .unwrap();
        assert_eq!((first, last), (2, 5));
    }

    #[test]
    fn dimensions_are_read_from_headers_and_kept() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.cbz");
        write_zip(&archive, &[("1.png", &png(120, 340)), ("2.png", b"\x89PNG\r\n\x1a\ncorrompu")]);
        let database = Database::open(&dir.path().join("komago.db")).unwrap();
        database
            .write(|c| {
                c.execute_batch(&format!(
                    "INSERT INTO series (id, title, sort_title) VALUES ('s', 'S', 's');
                     INSERT INTO source_files (id, path, kind, size_bytes) VALUES ('f', '{}', 'cbz', 1);
                     INSERT INTO chapters (id, series_id, page_count, source_id) VALUES ('c', 's', 2, 'f');
                     INSERT INTO pages (chapter_id, page_index, entry_name) VALUES ('c', 0, '1.png'), ('c', 1, '2.png');",
                    archive.display()
                ))?;
                Ok(())
            })
            .unwrap();

        let pages = open_chapter(&database, "c").unwrap().pages;
        assert_eq!((pages[0].width, pages[0].height), (Some(120), Some(340)));
        assert_eq!((pages[1].width, pages[1].height), (None, None), "un en-tête invalide n'empêche pas l'ouverture");
    }

    #[test]
    fn a_missing_source_still_opens_the_chapter() {
        let (_d, database) = setup();
        database
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO source_files (id, path, kind, size_bytes, status) VALUES ('f', '/nulle/part.cbz', 'cbz', 1, 'missing');
                     UPDATE chapters SET source_id = 'f' WHERE id = 'c1';
                     INSERT INTO pages (chapter_id, page_index, entry_name) VALUES ('c1', 0, '1.png');",
                )?;
                Ok(())
            })
            .unwrap();
        let chapter = open_chapter(&database, "c1").unwrap();
        assert_eq!(chapter.source_status.as_deref(), Some("missing"));
    }
}
