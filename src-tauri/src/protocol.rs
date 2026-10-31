//! Protocole `komago://` : sert les images sans passer par l'IPC JSON.
//! Routes : `cover/<asset>`, `preview/<session>/<chapitre>`, `page/<chapitre>/<index>`,
//! `candidate/<recherche>/<index>`.

use std::fs;
use std::path::Path;

use tauri::http::{header, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::archive::{sniff_image, SourceReader};
use crate::library;
use crate::reader;
use crate::AppState;

pub fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    let path = request.uri().path().replace("%2F", "/");
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<AppState>() else {
            responder.respond(empty(StatusCode::SERVICE_UNAVAILABLE));
            return;
        };
        responder.respond(serve(&state, &path).unwrap_or_else(empty));
    });
}

fn serve(state: &AppState, path: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["cover", asset] => cover(state, asset),
        ["preview", session, chapter] => preview(state, session, chapter),
        ["page", chapter, index] => page(state, chapter, index),
        ["candidate", token, index] => candidate(state, token, index),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

fn cover(state: &AppState, asset: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    let relative = state
        .database
        .read(|c| library::cover_asset_path(c, asset))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    // Le chemin vient de notre base, mais seul son nom de fichier est utilisé.
    let name = Path::new(&relative).file_name().ok_or(StatusCode::NOT_FOUND)?;
    let bytes = fs::read(state.storage.covers_dir().join(name)).map_err(|_| StatusCode::NOT_FOUND)?;
    image_response(bytes, "public, max-age=31536000, immutable")
}

fn preview(state: &AppState, session: &str, chapter: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    let (path, kind, entry) = {
        let current = state.import_session.lock().unwrap_or_else(|e| e.into_inner());
        let current = current.as_ref().filter(|s| s.id == session).ok_or(StatusCode::NOT_FOUND)?;
        current.cover_page(chapter).ok_or(StatusCode::NOT_FOUND)?
    };
    let bytes = SourceReader::open(&path, kind)
        .and_then(|mut reader| reader.read(&entry))
        .map_err(|_| StatusCode::NOT_FOUND)?;
    image_response(bytes, "no-store")
}

/// Le webview ne contacte jamais un service lui-même : la CSP le lui interdit, et
/// tout passe par le client gardé.
fn candidate(state: &AppState, token: &str, index: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    let index: usize = index.parse().map_err(|_| StatusCode::NOT_FOUND)?;
    let bytes = state.metadata.candidate_cover(token, index).map_err(|_| StatusCode::NOT_FOUND)?;
    image_response(bytes, "no-store")
}

fn page(state: &AppState, chapter: &str, index: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    let index: i64 = index.parse().map_err(|_| StatusCode::NOT_FOUND)?;
    let (path, kind, entry) = state
        .database
        .read(|c| reader::page_location(c, chapter, index))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let bytes = state
        .pages
        .page(chapter, index, Path::new(&path), reader::source_kind(&kind), &entry)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    image_response(bytes.as_ref().clone(), "private, max-age=3600")
}

/// Le type vient du contenu : un fichier qui n'est pas une image n'est pas servi.
fn image_response(bytes: Vec<u8>, cache: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    let format = sniff_image(&bytes).ok_or(StatusCode::UNSUPPORTED_MEDIA_TYPE)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, format.mime())
        .header(header::CACHE_CONTROL, cache)
        .header("X-Content-Type-Options", "nosniff")
        .body(bytes)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn empty(status: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = status;
    response
}
