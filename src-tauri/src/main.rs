//! PixiAnime desktop — Tauri entry point.
//!
//! Layers (kept separate on purpose):
//!   src/protocol.rs   URI-scheme handler — streaming/HLS (`/request`, `/m3u8`)
//!   src/commands/     typed IPC surface used by the frontend bridge
//!   src/services/     upstream HTTP, security policy, HLS rewriting
//!
//! The frontend never gets unrestricted network or filesystem access: it can
//! only call the commands below, and those re-run pixiClient's validators.

mod commands;
mod error;
mod protocol;
mod services;

use tauri_plugin_opener::OpenerExt;

/// Base URL the frontend uses to build streaming URLs (see protocol.rs).
#[tauri::command]
fn protocol_base() -> &'static str {
    protocol::base()
}

/// Self-check/debug channel: the frontend has no console I can read from a
/// GUI process, so it reports through here (lands on the dev server log).
#[tauri::command]
fn report(line: String) {
    println!("[js] {line}");
}

/// Open an external link in the system browser (never in the webview).
#[tauri::command]
fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http(s) links can be opened".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| err.to_string())
}

/// WebKitGTK's DMABUF renderer disagrees with the NVIDIA proprietary driver on
/// Wayland: scrolling repaints flicker (sticky panels, blurred header) and
/// video playback (GStreamer's dmabuf sink, decode on the Optimus iGPU and
/// display on the NVIDIA dGPU) hangs the whole webview process. Tauri's
/// documented workaround is the shared-memory fallback — see
/// tauri-apps/tauri#9394 and tauri-apps/tauri#13498; upstream WebKit bug
/// 260654 tracks the NVIDIA dmabuf path itself.
///
/// A value already in the environment always wins, so a user can still test
/// the GPU path (`WEBKIT_DISABLE_DMABUF_RENDERER=0 …`) or escalate to the last
/// resort (`WEBKIT_DISABLE_COMPOSITING_MODE=1`) without a rebuild.
fn apply_webgpu_fallback() {
    for var in ["WEBKIT_DISABLE_DMABUF_RENDERER", "__NV_DISABLE_EXPLICIT_SYNC"] {
        if std::env::var_os(var).is_none() {
            std::env::set_var(var, "1");
        }
    }
}

fn main() {
    apply_webgpu_fallback();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            protocol_base,
            open_url,
            report,
            commands::http_request,
            commands::fetch_playlist,
            commands::download_stream,
            commands::reveal_item,
        ])
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .run(tauri::generate_context!())
        .expect("error while running PixiAnime");
}
