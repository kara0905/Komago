use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::library::{chapter_row, ChapterRow, CHAPTER_COLUMNS, CHAPTER_JOINS};

/// Une série ne peut être que dans l'un de ces états de lecture.
const EXCLUSIVE: [&str; 3] = ["reading", "completed", "dropped"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub system_key: Option<String>,
    pub name: String,
    pub item_count: i64,
}

pub fn list(connection: &Connection) -> Result<Vec<Collection>> {
    let mut statement = connection.prepare(
        "SELECT c.id, c.system_key, c.name,
                (SELECT count(*) FROM collection_items i WHERE i.collection_id = c.id)
         FROM collections c
         ORDER BY c.system_key IS NULL, c.position, c.name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(Collection { id: row.get(0)?, system_key: row.get(1)?, name: row.get(2)?, item_count: row.get(3)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn create(connection: &mut Connection, name: &str) -> Result<String> {
    let name = valid_name(connection, name, None)?;
    let id = Uuid::now_v7().to_string();
    connection.execute("INSERT INTO collections (id, name) VALUES (?1, ?2)", params![id, name])?;
    Ok(id)
}

pub fn rename(connection: &mut Connection, id: &str, name: &str) -> Result<()> {
    let name = valid_name(connection, name, Some(id))?;
    // Les collections système gardent un nom stable.
    let changed = connection.execute(
        "UPDATE collections SET name = ?1 WHERE id = ?2 AND system_key IS NULL",
        params![name, id],
    )?;
    if changed == 0 {
        return Err(Error::Invalid("cette collection ne peut pas être renommée".into()));
    }
    Ok(())
}

fn valid_name(connection: &Connection, name: &str, except: Option<&str>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("une collection doit avoir un nom".into()));
    }
    let taken: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM collections WHERE name = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        params![name, except],
        |row| row.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!("la collection « {name} » existe déjà")));
    }
    Ok(name.to_string())
}

pub fn delete(connection: &mut Connection, id: &str) -> Result<()> {
    connection
        .execute("DELETE FROM collections WHERE id = ?1", [id])
        .map_err(|_| Error::Invalid("cette collection ne peut pas être supprimée".into()))?;
    Ok(())
}

pub fn add_series(connection: &mut Connection, collection_id: &str, series_id: &str) -> Result<()> {
    let tx = connection.transaction()?;
    let key: Option<String> = tx.query_row(
        "SELECT system_key FROM collections WHERE id = ?1",
        [collection_id],
        |row| row.get(0),
    )?;
    if key.as_deref().is_some_and(|k| EXCLUSIVE.contains(&k)) {
        tx.execute(
            "DELETE FROM collection_items WHERE series_id = ?1 AND collection_id IN
             (SELECT id FROM collections WHERE system_key IN ('reading', 'completed', 'dropped'))",
            [series_id],
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO collection_items (id, collection_id, series_id, position)
         VALUES (?1, ?2, ?3, (SELECT coalesce(max(position), 0) + 1 FROM collection_items WHERE collection_id = ?2))",
        params![Uuid::now_v7().to_string(), collection_id, series_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn add_chapter(connection: &mut Connection, collection_id: &str, chapter_id: &str) -> Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO collection_items (id, collection_id, chapter_id, position)
         VALUES (?1, ?2, ?3, (SELECT coalesce(max(position), 0) + 1 FROM collection_items WHERE collection_id = ?2))",
        params![Uuid::now_v7().to_string(), collection_id, chapter_id],
    )?;
    Ok(())
}

pub fn remove_series(connection: &mut Connection, collection_id: &str, series_id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM collection_items WHERE collection_id = ?1 AND series_id = ?2",
        params![collection_id, series_id],
    )?;
    Ok(())
}

pub fn remove_chapter(connection: &mut Connection, collection_id: &str, chapter_id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM collection_items WHERE collection_id = ?1 AND chapter_id = ?2",
        params![collection_id, chapter_id],
    )?;
    Ok(())
}

pub fn chapters(connection: &Connection, collection_id: &str) -> Result<Vec<ChapterRow>> {
    let sql = format!(
        "SELECT {CHAPTER_COLUMNS} FROM collection_items i
         JOIN chapters c ON c.id = i.chapter_id
         {CHAPTER_JOINS}
         WHERE i.collection_id = ?1
         ORDER BY i.position"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([collection_id], chapter_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::{tempdir, TempDir};

    fn setup() -> (TempDir, Database) {
        let dir = tempdir().unwrap();
        let database = Database::open(&dir.path().join("komago.db")).unwrap();
        database
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO series (id, title, sort_title) VALUES ('s', 'S', 's');
                     INSERT INTO chapters (id, series_id) VALUES ('c', 's');",
                )?;
                Ok(())
            })
            .unwrap();
        (dir, database)
    }

    fn count(database: &Database, collection: &str) -> i64 {
        database
            .read(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM collection_items WHERE collection_id = ?1",
                    [collection],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    #[test]
    fn user_collections_hold_series_and_chapters() {
        let (_d, database) = setup();
        let id = database.write(|c| create(c, " Mes préférés ")).unwrap();
        database.write(|c| add_series(c, &id, "s")).unwrap();
        database.write(|c| add_series(c, &id, "s")).unwrap();
        database.write(|c| add_chapter(c, &id, "c")).unwrap();
        assert_eq!(count(&database, &id), 2, "ajouter deux fois ne double pas");
        assert_eq!(database.read(|c| chapters(c, &id)).unwrap().len(), 1);

        database.write(|c| remove_chapter(c, &id, "c")).unwrap();
        database.write(|c| remove_series(c, &id, "s")).unwrap();
        assert_eq!(count(&database, &id), 0);
    }

    #[test]
    fn reading_states_are_exclusive_but_favorites_are_not() {
        let (_d, database) = setup();
        database.write(|c| add_series(c, "system-favorites", "s")).unwrap();
        database.write(|c| add_series(c, "system-reading", "s")).unwrap();
        database.write(|c| add_series(c, "system-completed", "s")).unwrap();

        assert_eq!(count(&database, "system-reading"), 0);
        assert_eq!(count(&database, "system-completed"), 1);
        assert_eq!(count(&database, "system-favorites"), 1);
    }

    #[test]
    fn names_must_be_present_and_unique() {
        let (_d, database) = setup();
        assert!(database.write(|c| create(c, "  ")).is_err());
        let id = database.write(|c| create(c, "Liste")).unwrap();
        assert!(database.write(|c| create(c, "liste")).is_err());
        assert!(database.write(|c| create(c, "Favoris")).is_err());
        database.write(|c| rename(c, &id, "LISTE")).unwrap();
    }

    #[test]
    fn system_collections_are_protected() {
        let (_d, database) = setup();
        assert!(database.write(|c| delete(c, "system-favorites")).is_err());
        assert!(database.write(|c| rename(c, "system-favorites", "Autre")).is_err());
    }

    #[test]
    fn deleting_a_collection_keeps_the_series() {
        let (_d, database) = setup();
        let id = database.write(|c| create(c, "Liste")).unwrap();
        database.write(|c| add_series(c, &id, "s")).unwrap();
        database.write(|c| delete(c, &id)).unwrap();
        let series: i64 = database.read(|c| Ok(c.query_row("SELECT count(*) FROM series", [], |r| r.get(0))?)).unwrap();
        assert_eq!(series, 1);
    }
}
