//! Comparaison et fusion, champ par champ, avec provenance. Une valeur modifiée à
//! la main ne change jamais sans une demande explicite de l'utilisateur.

use std::collections::HashMap;

use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

use super::ProviderSeries;
use crate::error::Result;
use crate::library::{self, SeriesDetail};
use crate::text::{normalize_title, sort_title};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetadataMode {
    Automatic,
    Assisted,
    Manual,
}

impl MetadataMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(Self::Automatic),
            "assisted" => Some(Self::Assisted),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// Le champ est vide : la valeur le complète.
    Fill,
    /// Deux valeurs différentes.
    Conflict,
    Same,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    pub field: String,
    pub current: String,
    pub proposed: String,
    pub kind: ChangeKind,
    pub current_source: Option<String>,
    pub locked: bool,
    /// Coché par défaut.
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct Provenance {
    pub source_name: String,
    pub locked: bool,
}

pub fn load_provenance(connection: &Connection, series_id: &str) -> Result<HashMap<String, Provenance>> {
    let mut statement = connection.prepare(
        "SELECT p.field, s.name, p.locked FROM field_provenance p
         JOIN metadata_sources s ON s.id = p.source_id
         WHERE p.entity_type = 'series' AND p.entity_id = ?1",
    )?;
    let rows = statement.query_map([series_id], |row| {
        Ok((row.get::<_, String>(0)?, Provenance { source_name: row.get(1)?, locked: row.get(2)? }))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn text(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

fn known(value: &str) -> String {
    if value == "unknown" { String::new() } else { value.to_string() }
}

fn same_list(a: &[String], b: &[String]) -> bool {
    let key = |items: &[String]| {
        let mut keys: Vec<String> = items.iter().map(|i| normalize_title(i)).collect();
        keys.sort();
        keys
    };
    key(a) == key(b)
}

/// Les titres alternatifs s'additionnent : on ne retire rien.
fn merged_alt_titles(current: &[String], candidate: &ProviderSeries) -> Vec<String> {
    let mut merged = current.to_vec();
    for alternate in &candidate.alt_titles {
        let key = normalize_title(&alternate.title);
        if !merged.iter().any(|existing| normalize_title(existing) == key) {
            merged.push(alternate.title.clone());
        }
    }
    merged
}

pub fn compute_changes(
    current: &SeriesDetail,
    provenance: &HashMap<String, Provenance>,
    candidate: &ProviderSeries,
    mode: MetadataMode,
) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    let mut push = |field: &str, now: String, proposed: String| {
        if proposed.is_empty() {
            return;
        }
        let kind = if now.is_empty() {
            ChangeKind::Fill
        } else if normalize_title(&now) == normalize_title(&proposed) {
            ChangeKind::Same
        } else {
            ChangeKind::Conflict
        };
        let origin = provenance.get(field);
        let locked = origin.is_some_and(|p| p.locked);
        changes.push(FieldChange {
            field: field.to_string(),
            current: now,
            proposed,
            kind,
            current_source: origin.map(|p| p.source_name.clone()),
            locked,
            // Automatique et assisté complètent le vide ; le reste attend une décision.
            selected: mode != MetadataMode::Manual && kind == ChangeKind::Fill && !locked,
        });
    };

    push("title", current.title.clone(), candidate.title.clone());
    push("description", text(&current.description), text(&candidate.description));
    push("kind", known(&current.kind), text(&candidate.kind));
    push("status", known(&current.status), text(&candidate.status));
    push("publisher", text(&current.publisher), text(&candidate.publisher));
    push("original_language", text(&current.original_language), text(&candidate.original_language));
    push("published_from", text(&current.published_from), text(&candidate.published_from));
    push("published_to", text(&current.published_to), text(&candidate.published_to));
    for (field, now, proposed) in [
        ("authors", &current.authors, &candidate.authors),
        ("artists", &current.artists, &candidate.artists),
        ("genres", &current.genres, &candidate.genres),
        ("tags", &current.tags, &candidate.tags),
    ] {
        // Même liste dans un autre ordre : ce n'est pas une différence.
        let now_text = if same_list(now, proposed) { proposed.join(", ") } else { now.join(", ") };
        push(field, now_text, proposed.join(", "));
    }

    let merged = merged_alt_titles(&current.alt_titles, candidate);
    if merged.len() > current.alt_titles.len() {
        let added = merged[current.alt_titles.len()..].join(", ");
        let origin = provenance.get("alt_titles");
        let locked = origin.is_some_and(|p| p.locked);
        changes.push(FieldChange {
            field: "alt_titles".into(),
            current: current.alt_titles.join(", "),
            proposed: added,
            kind: ChangeKind::Fill,
            current_source: origin.map(|p| p.source_name.clone()),
            locked,
            selected: mode != MetadataMode::Manual && !locked,
        });
    }

    if candidate.cover_url.is_some() {
        let origin = provenance.get("cover");
        let locked = origin.is_some_and(|p| p.locked);
        let kind = if current.cover_asset_id.is_none() { ChangeKind::Fill } else { ChangeKind::Conflict };
        changes.push(FieldChange {
            field: "cover".into(),
            current: String::new(),
            proposed: String::new(),
            kind,
            current_source: origin.map(|p| p.source_name.clone()),
            locked,
            selected: mode != MetadataMode::Manual && kind == ChangeKind::Fill && !locked,
        });
    }
    changes
}

pub struct Applied {
    pub fields: Vec<String>,
}

/// Applique les champs demandés. `allow_locked` n'est vrai que pour un choix
/// explicite de l'utilisateur, jamais pour une actualisation automatique.
#[allow(clippy::too_many_arguments)]
pub fn apply_fields(
    tx: &Transaction<'_>,
    series_id: &str,
    source_id: &str,
    snapshot_id: &str,
    candidate: &ProviderSeries,
    fields: &[String],
    allow_locked: bool,
    cover_asset: Option<&str>,
    current: &SeriesDetail,
) -> Result<Applied> {
    let provenance = load_provenance(tx, series_id)?;
    let mut applied = Vec::new();

    for field in fields {
        if !allow_locked && provenance.get(field).is_some_and(|p| p.locked) {
            continue;
        }
        let done = match field.as_str() {
            "title" => {
                tx.execute(
                    "UPDATE series SET title = ?1, sort_title = ?2 WHERE id = ?3",
                    params![candidate.title, sort_title(&candidate.title), series_id],
                )?;
                // L'ancien titre reste trouvable par la recherche.
                if normalize_title(&current.title) != normalize_title(&candidate.title) {
                    tx.execute(
                        "INSERT OR IGNORE INTO series_titles (series_id, title) VALUES (?1, ?2)",
                        params![series_id, current.title],
                    )?;
                }
                true
            }
            "description" => set_column(tx, "description", series_id, &candidate.description)?,
            "kind" => set_column(tx, "kind", series_id, &candidate.kind)?,
            "status" => set_column(tx, "status", series_id, &candidate.status)?,
            "publisher" => set_column(tx, "publisher", series_id, &candidate.publisher)?,
            "original_language" => set_column(tx, "original_language", series_id, &candidate.original_language)?,
            "published_from" => set_column(tx, "published_from", series_id, &candidate.published_from)?,
            "published_to" => set_column(tx, "published_to", series_id, &candidate.published_to)?,
            "authors" => {
                tx.execute("DELETE FROM series_people WHERE series_id = ?1 AND role = 'author'", [series_id])?;
                for name in &candidate.authors {
                    library::link_person(tx, series_id, name, "author")?;
                }
                true
            }
            "artists" => {
                tx.execute("DELETE FROM series_people WHERE series_id = ?1 AND role = 'artist'", [series_id])?;
                for name in &candidate.artists {
                    library::link_person(tx, series_id, name, "artist")?;
                }
                true
            }
            "genres" => {
                tx.execute("DELETE FROM series_genres WHERE series_id = ?1", [series_id])?;
                for name in &candidate.genres {
                    library::link_named(tx, "genres", "series_genres", "genre_id", series_id, name)?;
                }
                true
            }
            "tags" => {
                tx.execute("DELETE FROM series_tags WHERE series_id = ?1", [series_id])?;
                for name in &candidate.tags {
                    library::link_named(tx, "tags", "series_tags", "tag_id", series_id, name)?;
                }
                true
            }
            "alt_titles" => {
                for alternate in &candidate.alt_titles {
                    if normalize_title(&alternate.title) != normalize_title(&current.title) {
                        tx.execute(
                            "INSERT OR IGNORE INTO series_titles (series_id, title, language) VALUES (?1, ?2, ?3)",
                            params![series_id, alternate.title, alternate.language],
                        )?;
                    }
                }
                true
            }
            "cover" => match cover_asset {
                Some(asset) => {
                    tx.execute("UPDATE series SET cover_asset_id = ?1 WHERE id = ?2", params![asset, series_id])?;
                    true
                }
                None => false,
            },
            _ => false,
        };
        if done {
            tx.execute(
                "INSERT INTO field_provenance (entity_type, entity_id, field, source_id, snapshot_id, locked)
                 VALUES ('series', ?1, ?2, ?3, ?4, 0)
                 ON CONFLICT (entity_type, entity_id, field) DO UPDATE SET
                     source_id = excluded.source_id, snapshot_id = excluded.snapshot_id, locked = 0,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![series_id, field, source_id, snapshot_id],
            )?;
            applied.push(field.clone());
        }
    }
    if !applied.is_empty() {
        tx.execute("UPDATE series SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1", [series_id])?;
    }
    Ok(Applied { fields: applied })
}

/// `column` est une constante du code.
fn set_column(tx: &Transaction<'_>, column: &str, series_id: &str, value: &Option<String>) -> Result<bool> {
    let Some(value) = value else { return Ok(false) };
    tx.execute(&format!("UPDATE series SET {column} = ?1 WHERE id = ?2"), params![value, series_id])?;
    Ok(true)
}
