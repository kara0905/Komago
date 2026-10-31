use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Migrations appliquées dans l'ordre. La version de la base est stockée dans
/// `PRAGMA user_version` et correspond au nombre de migrations appliquées.
/// Une migration publiée ne se modifie jamais : on en ajoute une nouvelle.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_settings.sql"),
    include_str!("../../migrations/0002_library.sql"),
    include_str!("../../migrations/0003_metadata.sql"),
    include_str!("../../migrations/0004_import.sql"),
    include_str!("../../migrations/0005_metadata_matching.sql"),
];

pub fn run(connection: &mut Connection, db_path: &Path) -> Result<()> {
    run_migrations(connection, db_path, MIGRATIONS)
}

fn run_migrations(connection: &mut Connection, db_path: &Path, migrations: &[&str]) -> Result<()> {
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let target = migrations.len() as i64;

    if current > target {
        return Err(Error::DatabaseTooNew {
            found: current,
            supported: target,
        });
    }
    if current == target {
        return Ok(());
    }
    if current > 0 {
        backup(connection, db_path, current)?;
    }

    for (index, sql) in migrations.iter().enumerate().skip(current as usize) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", index as i64 + 1)?;
        transaction.commit()?;
    }
    Ok(())
}

/// Copie cohérente de la base avant toute migration d'une base existante.
fn backup(connection: &Connection, db_path: &Path, version: i64) -> Result<()> {
    let target = backup_path(db_path, version);
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    connection.execute("VACUUM INTO ?1", [target.to_string_lossy()])?;
    Ok(())
}

fn backup_path(db_path: &Path, version: i64) -> PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(format!(".v{version}.bak"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn latest_version() -> i64 {
        MIGRATIONS.len() as i64
    }

    fn user_version(connection: &Connection) -> i64 {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn open_migrated(path: &Path) -> Connection {
        let mut connection = Connection::open(path).unwrap();
        connection.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        run(&mut connection, path).unwrap();
        connection
    }

    #[test]
    fn fresh_database_reaches_latest_version_without_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        let connection = open_migrated(&path);

        assert_eq!(user_version(&connection), latest_version());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn running_twice_is_a_no_op() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        let mut connection = open_migrated(&path);
        run(&mut connection, &path).unwrap();

        assert_eq!(user_version(&connection), latest_version());
    }

    #[test]
    fn upgrade_backs_up_and_keeps_data() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        let mut connection = Connection::open(&path).unwrap();

        run_migrations(&mut connection, &path, &["CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('a');"]).unwrap();
        run_migrations(
            &mut connection,
            &path,
            &["CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('a');", "ALTER TABLE t ADD COLUMN w TEXT;"],
        )
        .unwrap();

        assert_eq!(user_version(&connection), 2);
        let value: String = connection.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(value, "a");

        let backup = Connection::open(backup_path(&path, 1)).unwrap();
        assert_eq!(user_version(&backup), 1);
        let columns: i64 = backup
            .query_row("SELECT count(*) FROM pragma_table_info('t')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(columns, 1);
    }

    #[test]
    fn failed_migration_rolls_back_and_keeps_previous_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        let mut connection = Connection::open(&path).unwrap();
        let first = "CREATE TABLE t (v TEXT);";

        run_migrations(&mut connection, &path, &[first]).unwrap();
        let failed = run_migrations(&mut connection, &path, &[first, "CREATE TABLE u (v TEXT); SYNTAX ERROR;"]);

        assert!(failed.is_err());
        assert_eq!(user_version(&connection), 1);
        let u_tables: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'u'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(u_tables, 0);
    }

    #[test]
    fn refuses_database_from_a_newer_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        let mut connection = Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", latest_version() + 1).unwrap();

        assert!(matches!(run(&mut connection, &path), Err(Error::DatabaseTooNew { .. })));
    }

    fn insert_series(connection: &Connection, id: &str, title: &str) {
        connection
            .execute(
                "INSERT INTO series (id, title, sort_title) VALUES (?1, ?2, ?2)",
                [id, title],
            )
            .unwrap();
    }

    fn search(connection: &Connection, query: &str) -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT series_id FROM search_index WHERE search_index MATCH ?1")
            .unwrap();
        statement
            .query_map([query], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    #[test]
    fn chapter_cannot_join_a_volume_of_another_series() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));
        insert_series(&connection, "a", "A");
        insert_series(&connection, "b", "B");
        connection
            .execute("INSERT INTO volumes (id, series_id) VALUES ('v', 'a')", [])
            .unwrap();

        let foreign = connection.execute(
            "INSERT INTO chapters (id, series_id, volume_id) VALUES ('c1', 'b', 'v')",
            [],
        );
        assert!(foreign.is_err());

        connection
            .execute("INSERT INTO chapters (id, series_id, volume_id) VALUES ('c2', 'a', 'v')", [])
            .unwrap();
        connection
            .execute("INSERT INTO chapters (id, series_id) VALUES ('c3', 'b')", [])
            .unwrap();
        assert!(connection
            .execute("UPDATE chapters SET volume_id = 'v' WHERE id = 'c3'", [])
            .is_err());
    }

    #[test]
    fn deleting_a_volume_keeps_its_chapters_and_progress() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));
        insert_series(&connection, "a", "A");
        connection
            .execute_batch(
                "INSERT INTO volumes (id, series_id) VALUES ('v', 'a');
                 INSERT INTO chapters (id, series_id, volume_id) VALUES ('c', 'a', 'v');
                 INSERT INTO reading_progress (chapter_id, last_read_at) VALUES ('c', '2026-01-01');
                 DELETE FROM volumes WHERE id = 'v';",
            )
            .unwrap();

        let volume: Option<String> = connection
            .query_row("SELECT volume_id FROM chapters WHERE id = 'c'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(volume, None);
        let progress: i64 = connection
            .query_row("SELECT count(*) FROM reading_progress", [], |r| r.get(0))
            .unwrap();
        assert_eq!(progress, 1);
    }

    #[test]
    fn deleting_a_source_file_keeps_the_chapter() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));
        insert_series(&connection, "a", "A");
        connection
            .execute_batch(
                "INSERT INTO source_files (id, path, kind, size_bytes) VALUES ('s', '/x.cbz', 'cbz', 1);
                 INSERT INTO chapters (id, series_id, source_id) VALUES ('c', 'a', 's');
                 DELETE FROM source_files WHERE id = 's';",
            )
            .unwrap();

        let source: Option<String> = connection
            .query_row("SELECT source_id FROM chapters WHERE id = 'c'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(source, None);
    }

    #[test]
    fn collection_item_targets_exactly_one_entity_once() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));
        insert_series(&connection, "a", "A");
        connection
            .execute("INSERT INTO chapters (id, series_id) VALUES ('c', 'a')", [])
            .unwrap();
        let insert = "INSERT INTO collection_items (id, collection_id, series_id, chapter_id)
                      VALUES (?1, 'system-favorites', ?2, ?3)";

        let none: [Option<&str>; 3] = [Some("i0"), None, None];
        let both = [Some("i1"), Some("a"), Some("c")];
        let series = [Some("i2"), Some("a"), None];
        let duplicate = [Some("i3"), Some("a"), None];
        let chapter = [Some("i4"), None, Some("c")];

        assert!(connection.execute(insert, none).is_err());
        assert!(connection.execute(insert, both).is_err());
        connection.execute(insert, series).unwrap();
        assert!(connection.execute(insert, duplicate).is_err());
        connection.execute(insert, chapter).unwrap();
    }

    #[test]
    fn system_collections_exist_and_cannot_be_deleted() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));

        let count: i64 = connection
            .query_row("SELECT count(*) FROM collections WHERE system_key IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 5);
        assert!(connection
            .execute("DELETE FROM collections WHERE id = 'system-favorites'", [])
            .is_err());
    }

    #[test]
    fn search_index_follows_titles_people_and_deletion() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));
        insert_series(&connection, "a", "Éclipse totale");

        // Accents ignorés, recherche par préfixe.
        assert_eq!(search(&connection, "eclip*"), ["a"]);

        connection.execute("UPDATE series SET title = 'Nouveau' WHERE id = 'a'", []).unwrap();
        assert_eq!(search(&connection, "nouveau"), ["a"]);
        assert!(search(&connection, "eclipse").is_empty());

        connection
            .execute_batch(
                "INSERT INTO series_titles (series_id, title, language) VALUES ('a', 'Alternate Name', 'en');
                 INSERT INTO people (id, name) VALUES ('p', 'Inoue Takehiko');
                 INSERT INTO series_people (series_id, person_id, role) VALUES ('a', 'p', 'author');",
            )
            .unwrap();
        assert_eq!(search(&connection, "alternate"), ["a"]);
        assert_eq!(search(&connection, "takehiko"), ["a"]);

        connection.execute("DELETE FROM series_titles", []).unwrap();
        assert!(search(&connection, "alternate").is_empty());

        connection.execute("DELETE FROM series WHERE id = 'a'", []).unwrap();
        assert!(search(&connection, "nouveau").is_empty());
    }

    #[test]
    fn manual_provenance_must_be_locked_and_is_removed_with_its_entity() {
        let dir = tempdir().unwrap();
        let connection = open_migrated(&dir.path().join("komago.db"));
        insert_series(&connection, "a", "A");
        let insert = "INSERT INTO field_provenance (entity_type, entity_id, field, source_id, locked)
                      VALUES ('series', 'a', 'title', 'manual', ?1)";

        assert!(connection.execute(insert, [0]).is_err());
        connection.execute(insert, [1]).unwrap();

        connection.execute("DELETE FROM series WHERE id = 'a'", []).unwrap();
        let left: i64 = connection
            .query_row("SELECT count(*) FROM field_provenance", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }
}
