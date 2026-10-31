//! Moteur de métadonnées. Les fournisseurs sont indépendants : en ajouter un revient
//! à l'inscrire dans `providers()`, sans toucher à la bibliothèque.

pub mod anilist;
pub mod http;
pub mod mangadex;
pub mod matching;
pub mod merge;

use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::archive::sniff_image;
use crate::covers;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::library;
use crate::settings;
use crate::storage::Storage;
use http::{GuardedClient, HostRule, Transport, MAX_IMAGE_BYTES};
use merge::{ChangeKind, FieldChange, MetadataMode};

#[derive(Debug, Clone, Default)]
pub struct SeriesQuery {
    pub title: String,
    pub alt_titles: Vec<String>,
    pub authors: Vec<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AltTitle {
    pub title: String,
    pub language: String,
}

/// Fiche d'un fournisseur, dans un format commun.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSeries {
    pub external_id: String,
    pub url: Option<String>,
    pub title: String,
    pub alt_titles: Vec<AltTitle>,
    pub description: Option<String>,
    #[serde(skip)]
    pub cover_url: Option<String>,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub original_language: Option<String>,
    pub publisher: Option<String>,
    pub published_from: Option<String>,
    pub published_to: Option<String>,
    /// Réponse brute, conservée pour rejouer une fusion sans réseau.
    #[serde(skip)]
    pub payload: Value,
}

/// Synchrone : les appels tournent hors du thread principal et du runtime async.
pub trait MetadataProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn hosts(&self) -> Vec<HostRule>;
    /// Site public, seule destination permise pour ouvrir une fiche.
    fn site(&self) -> &'static str;
    fn search_series(&self, query: &SeriesQuery, http: &GuardedClient) -> Result<Vec<ProviderSeries>>;
    fn get_series(&self, external_id: &str, http: &GuardedClient) -> Result<ProviderSeries>;
}

pub fn providers() -> Vec<Box<dyn MetadataProvider>> {
    vec![Box::new(mangadex::MangaDex), Box::new(anilist::AniList)]
}

pub(crate) fn parse_json(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| Error::Network("réponse illisible du service".into()))
}

/// Un identifiant stocké ne doit jamais pouvoir modifier le chemin d'une requête.
pub(crate) fn safe_id(id: &str) -> Result<&str> {
    let valid = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if valid {
        Ok(id)
    } else {
        Err(Error::Invalid("identifiant externe invalide".into()))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub index: usize,
    pub provider_id: String,
    pub provider_name: String,
    pub series: ProviderSeries,
    pub score: f64,
    pub certain: bool,
    pub has_cover: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFailure {
    pub provider_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutcome {
    pub token: String,
    pub matches: Vec<Match>,
    pub failures: Vec<ProviderFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPreview {
    pub token: String,
    pub index: usize,
    pub mode: MetadataMode,
    pub provider_name: String,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    pub applied: Vec<String>,
    pub cover_failed: bool,
}

struct PendingCandidate {
    provider_id: String,
    series: ProviderSeries,
    score: Option<f64>,
}

/// Fiches reçues, gardées côté Rust : le frontend les désigne par un numéro et ne
/// peut donc pas faire enregistrer des données ou des adresses de son choix.
struct Pending {
    token: String,
    series_id: String,
    candidates: Vec<PendingCandidate>,
}

pub struct MetadataService {
    pub client: GuardedClient,
    providers: Vec<Box<dyn MetadataProvider>>,
    pending: Mutex<Option<Pending>>,
}

impl MetadataService {
    pub fn new(transport: Box<dyn Transport>, enabled: bool) -> Self {
        Self::with_providers(transport, providers(), enabled)
    }

    pub fn with_providers(transport: Box<dyn Transport>, providers: Vec<Box<dyn MetadataProvider>>, enabled: bool) -> Self {
        let rules = providers.iter().flat_map(|p| p.hosts()).collect();
        Self { client: GuardedClient::new(transport, rules, enabled), providers, pending: Mutex::new(None) }
    }

    pub fn provider_list(&self) -> Vec<(&'static str, &'static str)> {
        self.providers.iter().map(|p| (p.id(), p.name())).collect()
    }

    pub fn is_provider_site(&self, provider_id: &str, url: &str) -> bool {
        let Some(provider) = self.providers.iter().find(|p| p.id() == provider_id) else { return false };
        let Ok(parsed) = reqwest::Url::parse(url) else { return false };
        let host = parsed.host_str().unwrap_or_default();
        parsed.scheme() == "https" && (host == provider.site() || host.ends_with(&format!(".{}", provider.site())))
    }

    fn lock_pending(&self) -> std::sync::MutexGuard<'_, Option<Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn search(&self, database: &Database, series_id: &str, title_override: Option<&str>) -> Result<SearchOutcome> {
        let (query, disabled, rejected) = database.read(|connection| {
            let detail = library::get_series(connection, series_id)?;
            let disabled = settings::load(connection)?.disabled_providers;
            let mut statement =
                connection.prepare("SELECT source_id, external_id FROM rejected_matches WHERE series_id = ?1")?;
            let rows = statement.query_map([series_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let rejected: Vec<(String, String)> = rows.collect::<rusqlite::Result<_>>()?;
            let query = SeriesQuery {
                title: title_override.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&detail.title).to_string(),
                alt_titles: detail.alt_titles,
                authors: detail.authors.into_iter().chain(detail.artists).collect(),
                kind: Some(detail.kind).filter(|k| k != "unknown"),
            };
            Ok((query, disabled, rejected))
        })?;

        let mut candidates = Vec::new();
        let mut failures = Vec::new();
        for provider in self.providers.iter().filter(|p| !disabled.iter().any(|d| d == p.id())) {
            match provider.search_series(&query, &self.client) {
                Ok(results) => {
                    for series in results {
                        if rejected.iter().any(|(s, e)| s == provider.id() && *e == series.external_id) {
                            continue;
                        }
                        let score = matching::score(&query, &series);
                        candidates.push(PendingCandidate { provider_id: provider.id().to_string(), series, score: Some(score) });
                    }
                }
                Err(Error::NetworkDisabled) => return Err(Error::NetworkDisabled),
                Err(error) => failures.push(ProviderFailure { provider_name: provider.name().to_string(), message: error.to_string() }),
            }
        }
        candidates.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));

        let matches = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let score = candidate.score.unwrap_or(0.0);
                let next = candidates[index + 1..]
                    .iter()
                    .find(|other| other.provider_id == candidate.provider_id)
                    .and_then(|other| other.score);
                let ahead = candidates[..index]
                    .iter()
                    .any(|other| other.provider_id == candidate.provider_id);
                Match {
                    index,
                    provider_id: candidate.provider_id.clone(),
                    provider_name: self.provider_name(&candidate.provider_id),
                    series: candidate.series.clone(),
                    score,
                    certain: !ahead && matching::is_certain(score, next),
                    has_cover: candidate.series.cover_url.is_some(),
                }
            })
            .collect();

        let token = Uuid::now_v7().to_string();
        *self.lock_pending() = Some(Pending { token: token.clone(), series_id: series_id.to_string(), candidates });
        Ok(SearchOutcome { token, matches, failures })
    }

    fn provider_name(&self, id: &str) -> String {
        self.providers.iter().find(|p| p.id() == id).map(|p| p.name().to_string()).unwrap_or_else(|| id.to_string())
    }

    /// Différences entre la série et une fiche déjà reçue.
    pub fn preview(&self, database: &Database, token: &str, index: usize) -> Result<MetadataPreview> {
        let guard = self.lock_pending();
        let pending = guard.as_ref().filter(|p| p.token == token).ok_or_else(expired)?;
        let candidate = pending.candidates.get(index).ok_or_else(expired)?;
        let (changes, mode) = changes_for(database, &pending.series_id, &candidate.series)?;
        Ok(MetadataPreview {
            token: token.to_string(),
            index,
            mode,
            provider_name: self.provider_name(&candidate.provider_id),
            changes,
        })
    }

    /// Actualise depuis une fiche déjà associée.
    pub fn refresh(&self, database: &Database, series_id: &str, provider_id: &str) -> Result<MetadataPreview> {
        let external_id: String = database.read(|c| {
            Ok(c.query_row(
                "SELECT external_id FROM external_ids WHERE series_id = ?1 AND source_id = ?2",
                params![series_id, provider_id],
                |r| r.get(0),
            )?)
        })?;
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == provider_id)
            .ok_or_else(|| Error::NotFound("fournisseur".into()))?;
        let series = provider.get_series(&external_id, &self.client)?;
        let (changes, mode) = changes_for(database, series_id, &series)?;
        let token = Uuid::now_v7().to_string();
        *self.lock_pending() = Some(Pending {
            token: token.clone(),
            series_id: series_id.to_string(),
            candidates: vec![PendingCandidate { provider_id: provider_id.to_string(), series, score: None }],
        });
        Ok(MetadataPreview { token, index: 0, mode, provider_name: provider.name().to_string(), changes })
    }

    /// `fields` absent : application automatique, qui ne suit que les choix par
    /// défaut et ne touche jamais un champ verrouillé.
    pub fn apply(
        &self,
        database: &Database,
        storage: &Storage,
        token: &str,
        index: usize,
        fields: Option<Vec<String>>,
    ) -> Result<ApplyOutcome> {
        let (series_id, provider_id, series, score) = {
            let guard = self.lock_pending();
            let pending = guard.as_ref().filter(|p| p.token == token).ok_or_else(expired)?;
            let candidate = pending.candidates.get(index).ok_or_else(expired)?;
            (pending.series_id.clone(), candidate.provider_id.clone(), candidate.series.clone(), candidate.score)
        };

        let explicit = fields.is_some();
        let fields = match fields {
            Some(fields) => fields,
            None => {
                let (changes, _) = changes_for(database, &series_id, &series)?;
                changes.into_iter().filter(|c| c.selected && c.kind != ChangeKind::Same).map(|c| c.field).collect()
            }
        };

        // Le téléchargement se fait sans verrou d'écriture.
        let mut cover_failed = false;
        let cover = if fields.iter().any(|f| f == "cover") {
            match self.download_cover(&series).and_then(|bytes| covers::render(&bytes)) {
                Ok(cover) => Some(cover),
                Err(_) => {
                    cover_failed = true;
                    None
                }
            }
        } else {
            None
        };

        let applied = database.write(|connection| {
            let current = library::get_series(connection, &series_id)?;
            let tx = connection.transaction()?;
            let snapshot_id = Uuid::now_v7().to_string();
            tx.execute(
                "INSERT INTO metadata_snapshots (id, series_id, source_id, external_id, payload, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![snapshot_id, series_id, provider_id, series.external_id, series.payload.to_string()],
            )?;
            tx.execute(
                "INSERT INTO external_ids (series_id, source_id, external_id, url, match_score, confirmed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 ON CONFLICT (series_id, source_id) DO UPDATE SET
                     external_id = excluded.external_id, url = excluded.url,
                     match_score = coalesce(excluded.match_score, match_score)",
                params![series_id, provider_id, series.external_id, series.url, score],
            )?;
            // Seules les dernières réponses servent : ni archive, ni copie du catalogue.
            tx.execute(
                "DELETE FROM metadata_snapshots WHERE series_id = ?1 AND source_id = ?2 AND id NOT IN
                 (SELECT id FROM metadata_snapshots WHERE series_id = ?1 AND source_id = ?2
                  ORDER BY fetched_at DESC, id DESC LIMIT 3)",
                params![series_id, provider_id],
            )?;
            let asset = match &cover {
                Some(cover) => Some(covers::store(&tx, &storage.covers_dir(), cover)?),
                None => None,
            };
            let applied = merge::apply_fields(
                &tx, &series_id, &provider_id, &snapshot_id, &series, &fields, explicit, asset.as_deref(), &current,
            )?;
            tx.commit()?;
            library::cleanup_orphans(connection, storage)?;
            Ok(applied.fields)
        })?;
        Ok(ApplyOutcome { applied, cover_failed })
    }

    fn download_cover(&self, series: &ProviderSeries) -> Result<Vec<u8>> {
        let url = series.cover_url.as_deref().ok_or_else(|| Error::NotFound("couverture".into()))?;
        let bytes = self.client.get(url, MAX_IMAGE_BYTES)?;
        if sniff_image(&bytes).is_none() {
            return Err(Error::Network("la couverture reçue n'est pas une image".into()));
        }
        Ok(bytes)
    }

    /// Couverture d'une fiche, pour l'affichage dans les résultats.
    pub fn candidate_cover(&self, token: &str, index: usize) -> Result<Vec<u8>> {
        let series = {
            let guard = self.lock_pending();
            let pending = guard.as_ref().filter(|p| p.token == token).ok_or_else(expired)?;
            pending.candidates.get(index).ok_or_else(expired)?.series.clone()
        };
        self.download_cover(&series)
    }

    /// La fiche ne sera plus proposée pour cette série.
    pub fn reject(&self, database: &Database, token: &str, index: usize) -> Result<()> {
        let (series_id, provider_id, external_id) = {
            let guard = self.lock_pending();
            let pending = guard.as_ref().filter(|p| p.token == token).ok_or_else(expired)?;
            let candidate = pending.candidates.get(index).ok_or_else(expired)?;
            (pending.series_id.clone(), candidate.provider_id.clone(), candidate.series.external_id.clone())
        };
        database.write(|c| {
            c.execute(
                "INSERT OR IGNORE INTO rejected_matches (series_id, source_id, external_id) VALUES (?1, ?2, ?3)",
                params![series_id, provider_id, external_id],
            )?;
            Ok(())
        })
    }
}

fn expired() -> Error {
    Error::Invalid("cette recherche n'est plus active, relancez-la".into())
}

fn changes_for(database: &Database, series_id: &str, series: &ProviderSeries) -> Result<(Vec<FieldChange>, MetadataMode)> {
    database.read(|connection| {
        let current = library::get_series(connection, series_id)?;
        let provenance = merge::load_provenance(connection, series_id)?;
        let mode = effective_mode(connection, &current)?;
        Ok((merge::compute_changes(&current, &provenance, series, mode), mode))
    })
}

fn effective_mode(connection: &Connection, detail: &library::SeriesDetail) -> Result<MetadataMode> {
    Ok(match detail.metadata_mode.as_deref().and_then(MetadataMode::parse) {
        Some(mode) => mode,
        None => settings::load(connection)?.metadata_mode,
    })
}

/// Défait l'association. Les valeurs déjà reçues restent, ainsi que leur provenance.
pub fn unlink(connection: &mut Connection, series_id: &str, source_id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM external_ids WHERE series_id = ?1 AND source_id = ?2",
        params![series_id, source_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::http::test_support::{rule, FakeTransport};
    use super::*;
    use crate::covers::test_support::png;
    use crate::library::SeriesPatch;
    use tempfile::{tempdir, TempDir};

    /// Fournisseur de test : ses résultats sont fixés et il passe par le client gardé.
    struct Fake {
        results: Vec<ProviderSeries>,
    }

    impl MetadataProvider for Fake {
        fn id(&self) -> &'static str {
            "mangadex"
        }
        fn name(&self) -> &'static str {
            "MangaDex"
        }
        fn hosts(&self) -> Vec<HostRule> {
            vec![rule("api.test", false), rule("img.test", false)]
        }
        fn site(&self) -> &'static str {
            "x.test"
        }
        fn search_series(&self, _query: &SeriesQuery, http: &GuardedClient) -> Result<Vec<ProviderSeries>> {
            http.get("https://api.test/search", 1024)?;
            Ok(self.results.clone())
        }
        fn get_series(&self, id: &str, http: &GuardedClient) -> Result<ProviderSeries> {
            http.get("https://api.test/get", 1024)?;
            self.results.iter().find(|r| r.external_id == id).cloned().ok_or_else(|| Error::NotFound("fiche".into()))
        }
    }

    struct World {
        _dir: TempDir,
        database: Database,
        storage: Storage,
        service: MetadataService,
        fake: FakeTransport,
    }

    fn fiche() -> ProviderSeries {
        ProviderSeries {
            external_id: "abc".into(),
            url: Some("https://x.test/abc".into()),
            title: "Vinland Saga".into(),
            alt_titles: vec![AltTitle { title: "ヴィンランド".into(), language: "ja".into() }],
            description: Some("Vikings".into()),
            cover_url: Some("https://img.test/c.png".into()),
            authors: vec!["Yukimura".into()],
            genres: vec!["Action".into()],
            status: Some("completed".into()),
            kind: Some("manga".into()),
            payload: serde_json::json!({"brut": true}),
            ..Default::default()
        }
    }

    fn world(responses: Vec<(u16, Vec<u8>)>, enabled: bool) -> World {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().join("data"), dir.path().join("cache")).unwrap();
        let database = Database::open(&storage.database_path()).unwrap();
        database
            .write(|c| {
                c.execute("INSERT INTO series (id, title, sort_title) VALUES ('s', 'Vinland Saga', 'vinland saga')", [])?;
                Ok(())
            })
            .unwrap();
        let fake = FakeTransport::with(responses);
        let service =
            MetadataService::with_providers(Box::new(fake.clone()), vec![Box::new(Fake { results: vec![fiche()] })], enabled);
        World { _dir: dir, database, storage, service, fake }
    }

    fn set_mode(world: &World, mode: MetadataMode) {
        world
            .database
            .write(|c| {
                let mut current = settings::load(c)?;
                current.metadata_mode = mode;
                settings::save(c, &current)
            })
            .unwrap();
    }

    fn edit_description(world: &World, text: &str) {
        let patch = SeriesPatch {
            title: "Vinland Saga".into(),
            kind: "unknown".into(),
            status: "unknown".into(),
            description: Some(text.into()),
            publisher: None,
            published_from: None,
            published_to: None,
            original_language: None,
            alt_titles: vec![],
            authors: vec![],
            artists: vec![],
            genres: vec![],
            tags: vec![],
        };
        world.database.write(|c| library::update_series(c, "s", &patch)).unwrap();
    }

    fn detail(world: &World) -> library::SeriesDetail {
        world.database.read(|c| library::get_series(c, "s")).unwrap()
    }

    fn search_ok() -> (u16, Vec<u8>) {
        (200, b"{}".to_vec())
    }

    #[test]
    fn nothing_is_associated_or_written_by_searching() {
        let w = world(vec![search_ok()], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        assert_eq!(outcome.matches.len(), 1);
        assert!(outcome.matches[0].certain);
        assert_eq!(outcome.matches[0].score, 1.0);

        assert!(detail(&w).external_links.is_empty());
        assert_eq!(detail(&w).description, None);
    }

    #[test]
    fn offline_is_clean_and_sends_nothing() {
        let w = world(vec![], false);
        assert!(matches!(w.service.search(&w.database, "s", None), Err(Error::NetworkDisabled)));
        assert!(w.fake.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failing_provider_is_reported_without_blocking() {
        let w = world(vec![(500, vec![])], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        assert!(outcome.matches.is_empty());
        assert_eq!(outcome.failures.len(), 1);
    }

    #[test]
    fn disabled_providers_are_not_called() {
        let w = world(vec![], true);
        w.database
            .write(|c| {
                let mut current = settings::load(c)?;
                current.disabled_providers = vec!["mangadex".into()];
                settings::save(c, &current)
            })
            .unwrap();
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        assert!(outcome.matches.is_empty());
        assert!(w.fake.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn assisted_preselects_only_empty_fields() {
        let w = world(vec![search_ok()], true);
        edit_description(&w, "Ma description");
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        let preview = w.service.preview(&w.database, &outcome.token, 0).unwrap();
        assert_eq!(preview.mode, MetadataMode::Assisted);

        let change = |field: &str| preview.changes.iter().find(|c| c.field == field).unwrap();
        assert!(change("status").selected);
        assert_eq!(change("status").kind, ChangeKind::Fill);
        assert!(change("cover").selected, "pas de couverture : elle est proposée");
        assert_eq!(change("title").kind, ChangeKind::Same);
        assert!(!change("title").selected);

        let description = change("description");
        assert_eq!(description.kind, ChangeKind::Conflict);
        assert!(description.locked);
        assert!(!description.selected);
        assert_eq!(description.current_source.as_deref(), Some("Modification manuelle"));
    }

    #[test]
    fn manual_mode_preselects_nothing() {
        let w = world(vec![search_ok()], true);
        set_mode(&w, MetadataMode::Manual);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        let preview = w.service.preview(&w.database, &outcome.token, 0).unwrap();
        assert!(preview.changes.iter().all(|c| !c.selected));

        // Application sans choix explicite : l'association est notée, rien d'autre.
        w.service.apply(&w.database, &w.storage, &outcome.token, 0, None).unwrap();
        let after = detail(&w);
        assert_eq!(after.external_links.len(), 1);
        assert_eq!(after.description, None);
        assert_eq!(after.status, "unknown");
    }

    #[test]
    fn automatic_fills_empty_fields_and_never_touches_manual_edits() {
        let w = world(vec![search_ok(), (200, png(300, 450))], true);
        set_mode(&w, MetadataMode::Automatic);
        edit_description(&w, "Ma description");
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        let result = w.service.apply(&w.database, &w.storage, &outcome.token, 0, None).unwrap();

        assert!(!result.cover_failed);
        let after = detail(&w);
        assert_eq!(after.description.as_deref(), Some("Ma description"));
        assert_eq!(after.status, "completed");
        assert_eq!(after.kind, "manga");
        assert_eq!(after.authors, ["Yukimura"]);
        assert_eq!(after.alt_titles, ["ヴィンランド"]);
        assert!(after.cover_asset_id.is_some(), "couverture stockée localement");

        let local = std::fs::read_dir(w.storage.covers_dir()).unwrap().count();
        assert_eq!(local, 1);

        let sources: Vec<(String, String)> = after.field_sources.iter().map(|f| (f.field.clone(), f.source_name.clone())).collect();
        assert!(sources.contains(&("status".to_string(), "MangaDex".to_string())));
        assert!(sources.contains(&("description".to_string(), "Modification manuelle".to_string())));
    }

    #[test]
    fn automatic_refresh_cannot_overwrite_a_locked_field_even_if_selected_upstream() {
        let w = world(vec![search_ok()], true);
        edit_description(&w, "Ma description");
        let outcome = w.service.search(&w.database, "s", None).unwrap();

        w.database
            .write(|connection| {
                let current = library::get_series(connection, "s")?;
                let tx = connection.transaction()?;
                tx.execute(
                    "INSERT INTO metadata_snapshots (id, series_id, source_id, payload, fetched_at) VALUES ('n', 's', 'mangadex', '{}', 'x')",
                    [],
                )?;
                let applied = merge::apply_fields(
                    &tx, "s", "mangadex", "n", &fiche(), &["description".to_string()], false, None, &current,
                )?;
                assert!(applied.fields.is_empty());
                tx.commit()?;
                Ok(())
            })
            .unwrap();
        assert_eq!(detail(&w).description.as_deref(), Some("Ma description"));
        drop(outcome);
    }

    #[test]
    fn an_explicit_choice_can_replace_a_manual_value() {
        let w = world(vec![search_ok()], true);
        edit_description(&w, "Ma description");
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        w.service.apply(&w.database, &w.storage, &outcome.token, 0, Some(vec!["description".into()])).unwrap();

        let after = detail(&w);
        assert_eq!(after.description.as_deref(), Some("Vikings"));
        let source = after.field_sources.iter().find(|f| f.field == "description").unwrap();
        assert_eq!(source.source_name, "MangaDex");
        assert!(!source.locked);
    }

    #[test]
    fn an_edit_after_applying_takes_over_again() {
        let w = world(vec![search_ok()], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        w.service.apply(&w.database, &w.storage, &outcome.token, 0, Some(vec!["description".into()])).unwrap();
        edit_description(&w, "Correction");

        let source_after = detail(&w);
        let source = source_after.field_sources.iter().find(|f| f.field == "description").unwrap();
        assert!(source.locked);
        assert_eq!(source.source_name, "Modification manuelle");
    }

    #[test]
    fn replacing_the_title_keeps_the_old_one_searchable() {
        let w = world(vec![search_ok()], true);
        w.database.write(|c| {
            c.execute("UPDATE series SET title = 'Nom de fichier' WHERE id = 's'", [])?;
            Ok(())
        }).unwrap();
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        w.service.apply(&w.database, &w.storage, &outcome.token, 0, Some(vec!["title".into()])).unwrap();

        let after = detail(&w);
        assert_eq!(after.title, "Vinland Saga");
        assert!(after.alt_titles.contains(&"Nom de fichier".to_string()));
    }

    #[test]
    fn a_bad_cover_does_not_block_the_rest() {
        let w = world(vec![search_ok(), (200, b"<html>pas une image".to_vec())], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        let result = w
            .service
            .apply(&w.database, &w.storage, &outcome.token, 0, Some(vec!["cover".into(), "status".into()]))
            .unwrap();
        assert!(result.cover_failed);
        assert_eq!(result.applied, ["status"]);
        assert!(detail(&w).cover_asset_id.is_none());
    }

    #[test]
    fn the_snapshot_keeps_the_raw_answer() {
        let w = world(vec![search_ok()], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        w.service.apply(&w.database, &w.storage, &outcome.token, 0, Some(vec![])).unwrap();
        let payload: String = w
            .database
            .read(|c| Ok(c.query_row("SELECT payload FROM metadata_snapshots", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(payload, r#"{"brut":true}"#);
    }

    #[test]
    fn only_the_latest_snapshots_are_kept() {
        let responses = (0..5).map(|_| search_ok()).collect();
        let w = world(responses, true);
        for _ in 0..5 {
            let outcome = w.service.search(&w.database, "s", None).unwrap();
            w.service.apply(&w.database, &w.storage, &outcome.token, 0, Some(vec![])).unwrap();
        }
        let count: i64 =
            w.database.read(|c| Ok(c.query_row("SELECT count(*) FROM metadata_snapshots", [], |r| r.get(0))?)).unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn a_rejected_match_is_no_longer_proposed() {
        let w = world(vec![search_ok(), search_ok()], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        w.service.reject(&w.database, &outcome.token, 0).unwrap();
        let again = w.service.search(&w.database, "s", None).unwrap();
        assert!(again.matches.is_empty());
    }

    #[test]
    fn refresh_uses_the_stored_association_and_unlink_keeps_values() {
        let w = world(vec![search_ok(), search_ok()], true);
        let outcome = w.service.search(&w.database, "s", None).unwrap();
        w.service.apply(&w.database, &w.storage, &outcome.token, 0, Some(vec!["status".into()])).unwrap();

        let refreshed = w.service.refresh(&w.database, "s", "mangadex").unwrap();
        assert_eq!(refreshed.changes.iter().find(|c| c.field == "status").unwrap().kind, ChangeKind::Same);

        w.database.write(|c| unlink(c, "s", "mangadex")).unwrap();
        let after = detail(&w);
        assert!(after.external_links.is_empty());
        assert_eq!(after.status, "completed");
    }

    #[test]
    fn only_the_provider_site_can_be_opened() {
        let w = world(vec![], true);
        assert!(w.service.is_provider_site("mangadex", "https://x.test/title/1"));
        assert!(w.service.is_provider_site("mangadex", "https://www.x.test/title/1"));
        assert!(!w.service.is_provider_site("mangadex", "http://x.test/title/1"));
        assert!(!w.service.is_provider_site("mangadex", "https://x.test.evil.test/"));
        assert!(!w.service.is_provider_site("mangadex", "file:///etc/passwd"));
        assert!(!w.service.is_provider_site("inconnu", "https://x.test/"));
    }

    #[test]
    fn a_stale_token_is_refused() {
        let w = world(vec![search_ok()], true);
        w.service.search(&w.database, "s", None).unwrap();
        assert!(w.service.preview(&w.database, "ancien", 0).is_err());
        assert!(w.service.apply(&w.database, &w.storage, "ancien", 0, None).is_err());
    }
}
