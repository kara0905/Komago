use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::archive::{SourceKind, SourceReader};
use crate::collections::{self, Collection};
use crate::covers;
use crate::error::{Error, Result};
use crate::import::{self, AnalyzeProgress, ImportEdits, ImportPlan, ImportResult};
use crate::metadata::{self, ApplyOutcome, MetadataPreview, SearchOutcome};
use crate::reader::{self, ProgressUpdate, ReaderChapter};
use crate::library::{self, ChapterPatch, ChapterRow, SeriesCard, SeriesDetail, SeriesFilter, SeriesPatch, VolumePatch};
use crate::settings::{self, Settings};
use crate::storage::StorageInfo;
use crate::AppState;

// `async` place ces commandes hors du thread principal : une requête ou un
// parcours de dossier ne doit jamais figer l'interface.

/// Présente seulement si l'initialisation a échoué.
pub struct StartupFailure(pub String);

#[tauri::command]
pub fn startup_failure(app: AppHandle) -> Option<String> {
    app.try_state::<StartupFailure>().map(|f| f.0.clone())
}

#[tauri::command(async)]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    state.database.read(settings::load)
}

#[tauri::command(async)]
pub fn update_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    state.database.write(|c| settings::save(c, &settings))?;
    state.metadata.client.set_enabled(settings.network_enabled);
    Ok(settings)
}

#[tauri::command(async)]
pub fn get_storage_info(state: State<'_, AppState>) -> Result<StorageInfo> {
    state.storage.info()
}

#[tauri::command(async)]
pub fn clear_cache(state: State<'_, AppState>) -> Result<StorageInfo> {
    state.storage.clear_cache()?;
    state.storage.info()
}

// Import ------------------------------------------------------------------------

/// Les sélecteurs natifs sont ouverts côté Rust : le webview n'a aucun accès au
/// système de fichiers, seulement les chemins choisis par l'utilisateur.
#[tauri::command(async)]
pub fn pick_import_files(app: AppHandle) -> Vec<String> {
    // Builds de développement seulement : les sélecteurs natifs ne s'automatisent pas.
    #[cfg(debug_assertions)]
    if let Ok(paths) = std::env::var("KOMAGO_DEV_IMPORT") {
        return paths.split(':').map(str::to_string).collect();
    }
    app.dialog()
        .file()
        .add_filter("Mangas et images", &["cbz", "zip", "jpg", "jpeg", "png", "webp", "gif", "avif", "bmp"])
        .blocking_pick_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[tauri::command(async)]
pub fn pick_import_folder(app: AppHandle) -> Vec<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|folder| folder.into_path().ok())
        .map(|path| vec![path.to_string_lossy().into_owned()])
        .unwrap_or_default()
}

#[tauri::command(async)]
pub fn analyze_import(
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
    on_progress: Channel<AnalyzeProgress>,
) -> Result<ImportPlan> {
    state.import_cancel.store(false, Ordering::SeqCst);
    let (session, plan) = import::analyze(&paths, &state.database, &state.import_cancel, |progress| {
        let _ = on_progress.send(progress);
    })?;
    *state.import_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
    Ok(plan)
}

#[tauri::command(async)]
pub fn commit_import(state: State<'_, AppState>, session_id: String, edits: ImportEdits) -> Result<ImportResult> {
    let mut guard = state.import_session.lock().unwrap_or_else(|e| e.into_inner());
    let session = guard
        .as_ref()
        .filter(|s| s.id == session_id)
        .ok_or_else(|| Error::Invalid("cet import n'est plus actif, relancez l'analyse".into()))?;
    let result = import::commit(&state.database, &state.storage, session, &edits)?;
    *guard = None;
    Ok(result)
}

#[tauri::command(async)]
pub fn cancel_import(state: State<'_, AppState>) {
    state.import_cancel.store(true, Ordering::SeqCst);
    *state.import_session.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[tauri::command(async)]
pub fn check_sources(state: State<'_, AppState>) -> Result<usize> {
    import::check_sources(&state.database)
}

// Bibliothèque ------------------------------------------------------------------

#[tauri::command(async)]
pub fn list_series(state: State<'_, AppState>, filter: SeriesFilter) -> Result<Vec<SeriesCard>> {
    state.database.read(|c| library::list_series(c, &filter))
}

#[tauri::command(async)]
pub fn search_chapters(state: State<'_, AppState>, query: String) -> Result<Vec<ChapterRow>> {
    state.database.read(|c| library::search_chapters(c, &query))
}

#[tauri::command(async)]
pub fn get_series(state: State<'_, AppState>, id: String) -> Result<SeriesDetail> {
    state.database.read(|c| library::get_series(c, &id))
}

#[tauri::command(async)]
pub fn update_series(state: State<'_, AppState>, id: String, patch: SeriesPatch) -> Result<SeriesDetail> {
    state.database.write(|c| library::update_series(c, &id, &patch))?;
    state.database.read(|c| library::get_series(c, &id))
}

#[tauri::command(async)]
pub fn delete_series(state: State<'_, AppState>, id: String) -> Result<()> {
    state.database.write(|c| library::delete_series(c, &state.storage, &id))
}

#[tauri::command(async)]
pub fn set_cover_from_chapter(state: State<'_, AppState>, series_id: String, chapter_id: String) -> Result<()> {
    let (path, kind, entry) = state.database.read(|c| library::first_page(c, &chapter_id))?;
    let kind = if kind == "directory" { SourceKind::Directory } else { SourceKind::Zip };
    let bytes = SourceReader::open(Path::new(&path), kind)?.read(&entry)?;
    let cover = covers::render(&bytes)?;
    state.database.write(|c| {
        let asset = covers::store(c, &state.storage.covers_dir(), &cover)?;
        library::set_series_cover(c, &state.storage, &series_id, &asset)
    })
}

#[tauri::command(async)]
pub fn update_chapter(state: State<'_, AppState>, id: String, patch: ChapterPatch) -> Result<()> {
    state.database.write(|c| library::update_chapter(c, &id, &patch))
}

#[tauri::command(async)]
pub fn delete_chapter(state: State<'_, AppState>, id: String) -> Result<()> {
    state.database.write(|c| library::delete_chapter(c, &state.storage, &id))
}

#[tauri::command(async)]
pub fn create_volume(state: State<'_, AppState>, series_id: String, patch: VolumePatch) -> Result<String> {
    state.database.write(|c| library::create_volume(c, &series_id, &patch))
}

#[tauri::command(async)]
pub fn update_volume(state: State<'_, AppState>, id: String, patch: VolumePatch) -> Result<()> {
    state.database.write(|c| library::update_volume(c, &id, &patch))
}

#[tauri::command(async)]
pub fn delete_volume(state: State<'_, AppState>, id: String) -> Result<()> {
    state.database.write(|c| library::delete_volume(c, &id))
}

// Collections -------------------------------------------------------------------

#[tauri::command(async)]
pub fn list_collections(state: State<'_, AppState>) -> Result<Vec<Collection>> {
    state.database.read(collections::list)
}

#[tauri::command(async)]
pub fn create_collection(state: State<'_, AppState>, name: String) -> Result<String> {
    state.database.write(|c| collections::create(c, &name))
}

#[tauri::command(async)]
pub fn rename_collection(state: State<'_, AppState>, id: String, name: String) -> Result<()> {
    state.database.write(|c| collections::rename(c, &id, &name))
}

#[tauri::command(async)]
pub fn delete_collection(state: State<'_, AppState>, id: String) -> Result<()> {
    state.database.write(|c| collections::delete(c, &id))
}

#[tauri::command(async)]
pub fn add_series_to_collection(state: State<'_, AppState>, collection_id: String, series_id: String) -> Result<()> {
    state.database.write(|c| collections::add_series(c, &collection_id, &series_id))
}

#[tauri::command(async)]
pub fn remove_series_from_collection(state: State<'_, AppState>, collection_id: String, series_id: String) -> Result<()> {
    state.database.write(|c| collections::remove_series(c, &collection_id, &series_id))
}

#[tauri::command(async)]
pub fn add_chapter_to_collection(state: State<'_, AppState>, collection_id: String, chapter_id: String) -> Result<()> {
    state.database.write(|c| collections::add_chapter(c, &collection_id, &chapter_id))
}

#[tauri::command(async)]
pub fn remove_chapter_from_collection(state: State<'_, AppState>, collection_id: String, chapter_id: String) -> Result<()> {
    state.database.write(|c| collections::remove_chapter(c, &collection_id, &chapter_id))
}

#[tauri::command(async)]
pub fn collection_chapters(state: State<'_, AppState>, collection_id: String) -> Result<Vec<ChapterRow>> {
    state.database.read(|c| collections::chapters(c, &collection_id))
}

// Lecteur -----------------------------------------------------------------------

#[tauri::command(async)]
pub fn open_chapter(state: State<'_, AppState>, id: String) -> Result<ReaderChapter> {
    reader::open_chapter(&state.database, &id)
}

#[tauri::command(async)]
pub fn save_progress(state: State<'_, AppState>, update: ProgressUpdate) -> Result<()> {
    state.database.write(|c| reader::save_progress(c, &update))
}

#[tauri::command(async)]
pub fn set_chapter_read(state: State<'_, AppState>, id: String, read: bool) -> Result<()> {
    state.database.write(|c| reader::set_read(c, &id, read))
}

#[tauri::command(async)]
pub fn set_series_direction(state: State<'_, AppState>, series_id: String, direction: String) -> Result<()> {
    state.database.write(|c| reader::set_series_direction(c, &series_id, &direction))
}

// Métadonnées -------------------------------------------------------------------

/// Le client réseau est bloquant : il ne doit tourner ni sur le thread principal,
/// ni sur un fil du runtime async.
async fn blocking<T, F>(app: AppHandle, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || work(&app.state::<AppState>()))
        .await
        .map_err(|e| Error::Network(e.to_string()))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    id: String,
    name: String,
}

#[tauri::command(async)]
pub fn list_metadata_providers(state: State<'_, AppState>) -> Vec<ProviderInfo> {
    state
        .metadata
        .provider_list()
        .into_iter()
        .map(|(id, name)| ProviderInfo { id: id.to_string(), name: name.to_string() })
        .collect()
}

#[tauri::command]
pub async fn search_metadata(app: AppHandle, series_id: String, title: Option<String>) -> Result<SearchOutcome> {
    blocking(app, move |state| state.metadata.search(&state.database, &series_id, title.as_deref())).await
}

#[tauri::command(async)]
pub fn preview_metadata(state: State<'_, AppState>, token: String, index: usize) -> Result<MetadataPreview> {
    state.metadata.preview(&state.database, &token, index)
}

#[tauri::command]
pub async fn refresh_metadata(app: AppHandle, series_id: String, provider_id: String) -> Result<MetadataPreview> {
    blocking(app, move |state| state.metadata.refresh(&state.database, &series_id, &provider_id)).await
}

#[tauri::command]
pub async fn apply_metadata(app: AppHandle, token: String, index: usize, fields: Option<Vec<String>>) -> Result<ApplyOutcome> {
    blocking(app, move |state| state.metadata.apply(&state.database, &state.storage, &token, index, fields)).await
}

#[tauri::command(async)]
pub fn reject_metadata(state: State<'_, AppState>, token: String, index: usize) -> Result<()> {
    state.metadata.reject(&state.database, &token, index)
}

#[tauri::command(async)]
pub fn unlink_metadata(state: State<'_, AppState>, series_id: String, source_id: String) -> Result<()> {
    state.database.write(|c| metadata::unlink(c, &series_id, &source_id))
}

#[tauri::command(async)]
pub fn set_series_metadata_mode(state: State<'_, AppState>, id: String, mode: Option<String>) -> Result<()> {
    state.database.write(|c| library::set_metadata_mode(c, &id, mode.as_deref()))
}

// Système -----------------------------------------------------------------------

/// L'adresse vient de la base et doit appartenir au site du fournisseur : le webview
/// ne peut pas faire ouvrir une adresse de son choix.
#[tauri::command(async)]
pub fn open_series_link(app: AppHandle, state: State<'_, AppState>, series_id: String, source_id: String) -> Result<()> {
    let url: Option<String> = state.database.read(|c| {
        Ok(c.query_row(
            "SELECT url FROM external_ids WHERE series_id = ?1 AND source_id = ?2",
            [&series_id, &source_id],
            |r| r.get(0),
        )?)
    })?;
    let url = url.ok_or_else(|| Error::NotFound("lien".into()))?;
    if !state.metadata.is_provider_site(&source_id, &url) {
        return Err(Error::Invalid("adresse refusée".into()));
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| Error::Invalid(e.to_string()))
}

#[tauri::command(async)]
pub fn reveal_data_folder(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    app.opener()
        .reveal_item_in_dir(state.storage.database_path())
        .map_err(|e| Error::Invalid(e.to_string()))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    database_version: i64,
}

#[tauri::command(async)]
pub fn app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    let database_version = state.database.read(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))?;
    Ok(AppInfo { version: env!("CARGO_PKG_VERSION"), database_version })
}
