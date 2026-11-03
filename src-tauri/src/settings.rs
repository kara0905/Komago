use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::Result;
use crate::metadata::merge::MetadataMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryView {
    Grid,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReaderMode {
    Vertical,
    Ltr,
    Rtl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReaderFit {
    Width,
    Height,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReaderPageSpacing {
    None,
    Normal,
    Large,
}

/// Paramètres de l'application. Un champ absent de la base prend sa valeur par
/// défaut ; ajouter un paramètre ne demande donc pas de migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    /// Interrupteur général des connexions externes, désactivé par défaut.
    pub network_enabled: bool,
    pub library_view: LibraryView,
    /// Mode par défaut ; une série peut avoir le sien.
    pub reader_mode: ReaderMode,
    pub reader_fit: ReaderFit,
    /// À part : une longue bande ajustée à la hauteur serait illisible.
    pub reader_vertical_fit: ReaderFit,
    pub reader_double_page: bool,
    pub reader_background: String,
    pub reader_auto_hide: bool,
    pub reader_page_spacing: ReaderPageSpacing,
    pub reader_page_width: u8,
    pub reader_brightness: u16,
    pub reader_contrast: u16,
    pub reader_saturation: u16,
    pub reader_grayscale: bool,
    pub reader_night_mode: bool,
    pub reader_auto_scroll_speed: u16,
    pub reader_keyboard_scroll_step: u16,
    /// Mode par défaut ; une série peut avoir le sien.
    pub metadata_mode: MetadataMode,
    pub disabled_providers: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            network_enabled: false,
            library_view: LibraryView::Grid,
            reader_mode: ReaderMode::Ltr,
            reader_fit: ReaderFit::Height,
            reader_vertical_fit: ReaderFit::Width,
            reader_double_page: false,
            reader_background: "#111111".into(),
            reader_auto_hide: true,
            reader_page_spacing: ReaderPageSpacing::Normal,
            reader_page_width: 90,
            reader_brightness: 100,
            reader_contrast: 100,
            reader_saturation: 100,
            reader_grayscale: false,
            reader_night_mode: false,
            reader_auto_scroll_speed: 60,
            reader_keyboard_scroll_step: 120,
            metadata_mode: MetadataMode::Assisted,
            disabled_providers: Vec::new(),
        }
    }
}

pub fn load(connection: &Connection) -> Result<Settings> {
    let mut statement = connection.prepare("SELECT key, value FROM settings")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;

    let defaults = serde_json::to_value(Settings::default())?;
    let Value::Object(mut merged) = defaults else {
        unreachable!("Settings se sérialise en objet");
    };
    let known: Map<String, Value> = merged.clone();

    for row in rows {
        let (key, raw) = row?;
        // Une clé inconnue (version plus récente de l'application) ou une valeur
        // devenue invalide ne doit pas empêcher le démarrage.
        if !known.contains_key(&key) {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            let mut candidate = known.clone();
            candidate.insert(key.clone(), value.clone());
            if serde_json::from_value::<Settings>(Value::Object(candidate)).is_ok() {
                merged.insert(key, value);
            }
        }
    }
    Ok(serde_json::from_value(Value::Object(merged))?)
}

pub fn save(connection: &mut Connection, settings: &Settings) -> Result<()> {
    let Value::Object(values) = serde_json::to_value(settings)? else {
        unreachable!("Settings se sérialise en objet");
    };
    let transaction = connection.transaction()?;
    for (key, value) in values {
        transaction.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET
                 value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE value <> excluded.value",
            params![key, value.to_string()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::{tempdir, TempDir};

    fn database() -> (TempDir, Database) {
        let dir = tempdir().unwrap();
        let database = Database::open(&dir.path().join("komago.db")).unwrap();
        (dir, database)
    }

    fn raw_insert(database: &Database, key: &str, value: &str) {
        database
            .write(|c| {
                c.execute("INSERT INTO settings (key, value) VALUES (?1, ?2)", [key, value])?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn network_is_disabled_by_default() {
        let (_dir, database) = database();
        let settings = database.read(load).unwrap();
        assert_eq!(settings, Settings::default());
        assert!(!settings.network_enabled);
    }

    #[test]
    fn saved_settings_survive_reopening() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("komago.db");
        let expected = Settings {
            theme: Theme::Dark,
            network_enabled: true,
            library_view: LibraryView::List,
            reader_mode: ReaderMode::Rtl,
            reader_fit: ReaderFit::Width,
            reader_vertical_fit: ReaderFit::Original,
            reader_double_page: true,
            reader_background: "#ffffff".into(),
            reader_auto_hide: false,
            reader_page_spacing: ReaderPageSpacing::Large,
            reader_page_width: 75,
            reader_brightness: 115,
            reader_contrast: 90,
            reader_saturation: 80,
            reader_grayscale: true,
            reader_night_mode: true,
            reader_auto_scroll_speed: 140,
            reader_keyboard_scroll_step: 240,
            metadata_mode: MetadataMode::Manual,
            disabled_providers: vec!["anilist".into()],
        };
        {
            let database = Database::open(&path).unwrap();
            database.write(|c| save(c, &expected)).unwrap();
        }
        let database = Database::open(&path).unwrap();
        assert_eq!(database.read(load).unwrap(), expected);
    }

    #[test]
    fn invalid_or_unknown_values_do_not_break_loading() {
        let (_dir, database) = database();
        raw_insert(&database, "theme", "\"sepia\"");
        raw_insert(&database, "networkEnabled", "true");
        raw_insert(&database, "fromAFutureVersion", "42");

        let settings = database.read(load).unwrap();
        assert_eq!(settings.theme, Theme::System);
        assert!(settings.network_enabled);
    }
}
