//! Tauri shell: exposes `cairn-core` to the UI as commands, runs the file
//! watcher and remembers recent vaults. No note logic lives here.

mod commands;
mod config;
mod protocol;
#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "android", test))]
mod listing;
#[cfg(any(target_os = "android", test))]
mod local_network;
mod sync;
#[cfg(desktop)]
mod watcher;

use std::sync::Arc;

use cairn_core::Vault;
use parking_lot::{Mutex, RwLock};

pub struct AppState {
    pub vault: RwLock<Option<Arc<Vault>>>,
    /// Root of the open vault (as `VaultInfo::root`), the key of its per-device data.
    pub vault_root: RwLock<Option<String>>,
    #[cfg(desktop)]
    pub watcher: Mutex<Option<watcher::Watcher>>,
    pub config: Mutex<config::AppConfig>,
    pub sync: Mutex<Option<Arc<sync::SyncManager>>>,
}

/// Process start, for start-up timing.
pub static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Keeps the window on the app's own pages. The UI opens links itself (notes
/// in the editor, websites in the system browser); a link it misses must not
/// replace the app with the target page. That page could be a website, or a
/// vault file served by vault://, which Tauri treats as a local origin with
/// access to every command.
fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|webview, url| {
            use tauri::Manager;
            let dev_url = if cfg!(dev) { webview.config().build.dev_url.as_ref() } else { None };
            let allowed = is_app_url(url, dev_url);
            if !allowed {
                log::warn!("refused to navigate the window to {url}");
            }
            allowed
        })
        .build()
}

/// True for the URLs the app's own pages are served from: Tauri's asset
/// protocol (`tauri://localhost`, or `http://tauri.localhost` on Windows and
/// Android) and, in development, the dev server.
fn is_app_url(url: &tauri::Url, dev_url: Option<&tauri::Url>) -> bool {
    if dev_url.is_some_and(|dev| dev.origin() == url.origin()) {
        return true;
    }
    let ours = if cfg!(any(windows, target_os = "android")) {
        matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost")
    } else {
        url.scheme() == "tauri" && url.host_str() == Some("localhost")
    };
    ours && url.port().is_none()
}

/// A SIGTERM to the app's process (`kill`, `killall`, `pkill`) would end it at
/// once, losing the edits the UI has not saved yet (autosave waits 600 ms
/// after the last key). It closes the window instead, so the UI saves every
/// tab and the settings as it does for the title-bar X. The process exits
/// anyway after a few seconds (the UI may be stuck, or asking about a tab it
/// could not save), or at a second SIGTERM.
///
/// This cannot help when every process of the app gets a signal at once:
/// systemd stopping the app's scope at logout or shutdown, or Ctrl+C and a
/// closed terminal, which signal the whole process group. The web view's
/// process dies too, and the unsaved text lives there.
#[cfg(all(desktop, unix))]
fn close_on_sigterm(app: tauri::AppHandle) {
    use tauri::Manager;
    use tokio::signal::unix::{SignalKind, signal};

    const GRACE: std::time::Duration = std::time::Duration::from_secs(3);
    const EXIT: i32 = 128 + 15;
    tauri::async_runtime::spawn(async move {
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => return log::warn!("cannot handle SIGTERM: {e}"),
        };
        term.recv().await;
        log::info!("SIGTERM: saving, then closing the window");
        if app.get_webview_window("main").is_some_and(|w| w.close().is_ok()) {
            std::thread::spawn(|| {
                std::thread::sleep(GRACE);
                log::warn!("still running {GRACE:?} after SIGTERM: exiting");
                std::process::exit(EXIT);
            });
            term.recv().await;
            log::warn!("second SIGTERM: exiting now");
        }
        std::process::exit(EXIT);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    STARTED.get_or_init(std::time::Instant::now);
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(navigation_guard())
        .register_asynchronous_uri_scheme_protocol("vault", protocol::handle);
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android::init()).plugin(android::init_local_network());
    builder
        .setup(|app| {
            use tauri::Manager;
            let cfg = config::AppConfig::load(app.handle());
            app.manage(AppState {
                vault: RwLock::new(None),
                vault_root: RwLock::new(None),
                #[cfg(desktop)]
                watcher: Mutex::new(None),
                config: Mutex::new(cfg),
                sync: Mutex::new(None),
            });
            #[cfg(all(desktop, unix))]
            close_on_sigterm(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup_vault,
            commands::open_vault,
            commands::create_vault,
            commands::recent_vaults,
            commands::forget_vault,
            commands::list_entries,
            commands::read_note,
            commands::write_note,
            commands::recreate_note,
            commands::merge_text,
            commands::create_note,
            commands::create_folder,
            commands::rename_entry,
            commands::delete_entry,
            commands::unique_path,
            commands::search,
            commands::backlinks,
            commands::outgoing_links,
            commands::resolve_link,
            commands::rescan,
            commands::ui_ready,
            commands::read_config,
            commands::write_config,
            commands::list_config,
            commands::read_config_bytes,
            commands::write_config_bytes,
            commands::trash_config,
            commands::plugin_approvals,
            commands::set_plugin_approval,
            commands::save_attachment,
            commands::read_text_file,
            commands::tags,
            commands::graph,
            commands::note_info,
            commands::open_externally,
            commands::open_externally_check,
            commands::reveal_in_file_manager,
            commands::sync_status,
            commands::sync_setup,
            commands::sync_vault_exists,
            commands::sync_now,
            commands::sync_cancel,
            commands::sync_disconnect,
            commands::sync_history,
            commands::sync_revision,
            commands::sync_restore,
            commands::default_device_name,
            commands::platform,
            commands::app_vaults,
            commands::pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cairn");
}
