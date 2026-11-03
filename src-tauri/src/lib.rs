mod archive;
mod backup;
mod collections;
mod comicinfo;
mod commands;
mod covers;
mod db;
mod error;
mod import;
mod library;
mod metadata;
mod page_store;
mod protocol;
mod reader;
mod roots;
mod settings;
mod storage;
mod text;

use std::sync::Mutex;

use std::sync::atomic::AtomicBool;

use tauri::Manager;

use db::Database;
use error::Error;
use import::ImportSession;
use page_store::PageStore;
use storage::Storage;

pub struct AppState {
    database: Database,
    storage: Storage,
    /// Un seul import à la fois : le plan affiché dans l'interface.
    import_session: Mutex<Option<ImportSession>>,
    import_cancel: AtomicBool,
    pages: PageStore,
    metadata: metadata::MetadataService,
}

fn initialize(app: &tauri::App) -> error::Result<AppState> {
    let paths = app.path();
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut data = paths.app_data_dir().map_err(|e| Error::AppDirectory(e.to_string()))?;
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut cache = paths.app_cache_dir().map_err(|e| Error::AppDirectory(e.to_string()))?;

    // Builds de développement seulement : essais sur une bibliothèque jetable.
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("KOMAGO_DEV_ROOT").map(std::path::PathBuf::from) {
        data = root.join("data");
        cache = root.join("cache");
    }

    // Une restauration préparée s'applique avant l'ouverture de la base.
    backup::apply_pending(&data)?;
    let storage = Storage::new(data, cache)?;
    archive::set_extraction_root(storage.cache_dir().join("extracted"));
    let database = Database::open(&storage.database_path())?;
    let network_enabled = database.read(settings::load)?.network_enabled;
    let metadata = metadata::MetadataService::new(Box::new(metadata::http::ReqwestTransport::new()), network_enabled);
    Ok(AppState { database, storage, import_session: Mutex::new(None), import_cancel: AtomicBool::new(false), pages: PageStore::new(), metadata })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("komago", protocol::handle)
        .setup(|app| {
            // Une base illisible ou trop récente doit s'expliquer, pas planter.
            match initialize(app) {
                Ok(state) => {
                    app.manage(state);
                }
                Err(error) => {
                    app.manage(commands::StartupFailure(error.to_string()));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup_failure,
            commands::get_settings,
            commands::update_settings,
            commands::get_storage_info,
            commands::clear_cache,
            commands::pick_import_files,
            commands::pick_import_folder,
            commands::analyze_import,
            commands::commit_import,
            commands::cancel_import,
            commands::check_sources,
            commands::list_series,
            commands::search_chapters,
            commands::get_series,
            commands::update_series,
            commands::delete_series,
            commands::set_cover_from_chapter,
            commands::update_chapter,
            commands::delete_chapter,
            commands::create_volume,
            commands::update_volume,
            commands::delete_volume,
            commands::list_collections,
            commands::create_collection,
            commands::rename_collection,
            commands::delete_collection,
            commands::add_series_to_collection,
            commands::remove_series_from_collection,
            commands::add_chapter_to_collection,
            commands::remove_chapter_from_collection,
            commands::collection_chapters,
            commands::open_chapter,
            commands::save_progress,
            commands::set_chapter_read,
            commands::set_series_direction,
            commands::list_metadata_providers,
            commands::search_metadata,
            commands::preview_metadata,
            commands::refresh_metadata,
            commands::apply_metadata,
            commands::reject_metadata,
            commands::unlink_metadata,
            commands::set_series_metadata_mode,
            commands::open_series_link,
            commands::reveal_data_folder,
            commands::app_info,
            commands::list_roots,
            commands::add_root,
            commands::remove_root,
            commands::scan_roots,
            commands::clear_ignored_sources,
            commands::set_spread_offset,
            commands::continue_reading,
            commands::list_unidentified,
            commands::identify_batch,
            commands::cancel_identify_batch,
            commands::fill_chapter_titles,
            commands::export_backup,
            commands::stage_restore,
            commands::discard_restore,
            commands::restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("échec du démarrage de Komago");
}
