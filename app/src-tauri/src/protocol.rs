//! `vault://` protocol: serves files from the open vault to the web view
//! (images, embedded media). Going through `VaultFs` instead of the asset
//! protocol means it also works for Android vaults that have no file path.
//!
//! URLs are `vault://localhost/<percent-encoded vault path>` on Linux and
//! macOS and `http://vault.localhost/<path>` on Windows and Android.

use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::AppState;

pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "md" | "markdown" | "txt" => "text/plain; charset=utf-8",
        "css" => "text/css",
        _ => "application/octet-stream",
    }
}

fn respond(status: StatusCode, mime: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .unwrap()
}

pub fn handle(ctx: UriSchemeContext<'_, Wry>, req: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let vault = ctx.app_handle().state::<AppState>().vault.read().clone();
    let raw = req.uri().path().trim_start_matches('/').to_string();
    std::thread::spawn(move || {
        let path = percent_encoding::percent_decode_str(&raw).decode_utf8_lossy().into_owned();
        let res = match vault {
            None => respond(StatusCode::SERVICE_UNAVAILABLE, "text/plain", b"no notebook open".to_vec()),
            Some(v) => match v.read_file(&path) {
                Ok(bytes) => respond(StatusCode::OK, mime_for(&path), bytes),
                Err(e) => respond(StatusCode::NOT_FOUND, "text/plain", e.to_string().into_bytes()),
            },
        };
        responder.respond(res);
    });
}
