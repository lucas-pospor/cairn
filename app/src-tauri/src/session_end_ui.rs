//! The app's side of [`crate::session_end`] on Windows: the page keeps what
//! it has not saved in [`crate::held`] through `session_hold`, answers the
//! question "session-end" the same way, and hears afterwards what the
//! backend wrote ("session-end-news").

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;
use crate::held::{self, Hold, HoldReply, Moved, Unsaved};
use crate::session_end;

struct AppHost {
    app: AppHandle,
    /// What the last write left unsaved, for the page after a no.
    last: Mutex<Vec<Unsaved>>,
}

/// What the page hears once the session goes on.
#[derive(Clone, serde::Serialize)]
struct News {
    written: Vec<Moved>,
    refused: Vec<Unsaved>,
}

impl session_end::Host for AppHost {
    fn listening(&self) -> bool {
        self.app.state::<AppState>().held.lock().listening()
    }

    fn ask(&self, round: u64) -> bool {
        // On the main thread Tauri hands the event's script to WebView2 at
        // once, and the page runs it when its thread is free. A WebView2
        // that is gone fails only in the log: the wait then finds no answer.
        self.app.emit("session-end", round).inspect_err(|e| log::warn!("session end: {e}")).is_ok()
    }

    fn answered(&self, round: u64) -> bool {
        self.app.state::<AppState>().held.lock().answered(round)
    }

    fn write(&self) -> Vec<String> {
        let state = self.app.state::<AppState>();
        let open = state.vault.read().clone();
        let unsaved = held::write_pass(&state.held, open);
        let names = unsaved.iter().map(|u| u.name.clone()).collect();
        *self.last.lock() = unsaved;
        names
    }

    fn tell(&self, refused: bool) {
        let state = self.app.state::<AppState>();
        let written = state.held.lock().take_news();
        let refused = if refused { self.last.lock().clone() } else { Vec::new() };
        if written.is_empty() && refused.is_empty() {
            return;
        }
        // Sync learns of the notes written here as of any local change.
        if !written.is_empty()
            && let Some(s) = state.sync.lock().as_ref()
        {
            s.poke();
        }
        if let Err(e) = self.app.emit("session-end-news", News { written, refused }) {
            log::warn!("session end: {e}");
        }
    }
}

/// Watches for Windows ending the session. Call it in setup, on the main
/// thread, once the main window exists.
pub fn install(app: &tauri::App) {
    session_end::ask_cairn_first();
    match app.get_webview_window("main").map(|w| w.hwnd()) {
        Some(Ok(main)) => session_end::install(main, AppHost { app: app.handle().clone(), last: Mutex::new(Vec::new()) }),
        Some(Err(e)) => log::warn!("session end: no handle for the main window: {e}"),
        None => log::warn!("session end: no main window"),
    }
}

/// What the page holds that is not saved yet (see held.rs). Not async:
/// Tauri runs it on the main thread, also inside the wait for the page's
/// answer.
#[tauri::command]
pub fn session_hold(state: State<'_, AppState>, hold: Hold) -> HoldReply {
    session_end::page_heard();
    if let Some(round) = hold.round.filter(|r| session_end::waiting_for() != Some(*r)) {
        log::info!("session end: the page answered round {round} after Cairn stopped waiting for it");
    }
    let reply = state.held.lock().hold(hold, || {
        let r1 = state.vault_root.read().clone();
        let vault = state.vault.read().clone();
        (r1, vault, state.vault_root.read().clone())
    });
    if state.held.lock().nothing_held() {
        session_end::settled();
    }
    reply
}

/// The file the backend put in place of `base`, the file a page save of
/// `path` found changed, when the backend itself wrote the note.
#[tauri::command]
pub fn session_moved(state: State<'_, AppState>, path: String, base: String) -> Option<String> {
    state.held.lock().moved(&path, &base)
}
