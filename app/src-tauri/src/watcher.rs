//! Desktop file watcher. Events are only hints: the paths they mention are
//! rescanned by `Vault::rescan_paths`, which works out what really changed.

use std::sync::Arc;
use std::time::Duration;

use cairn_core::{StdFs, Vault};
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use tauri::{AppHandle, Emitter};

pub struct Watcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
}

/// Beyond this many paths in one batch, a full rescan is cheaper.
const FULL_RESCAN_THRESHOLD: usize = 500;

pub fn start(app: AppHandle, vault: Arc<Vault>, root: std::path::PathBuf) -> notify::Result<Watcher> {
    let mapper = StdFs::new(&root, Default::default()).map_err(|e| notify::Error::generic(&e.to_string()))?;
    let v = vault.clone();
    let mut debouncer = new_debouncer(Duration::from_millis(250), None, move |res: DebounceEventResult| {
        let result = match res {
            // The kernel queue overflowed (notify sends an event with no
            // paths and the Rescan flag): events were lost, check everything.
            Ok(events) if events.iter().any(|e| e.need_rescan()) => {
                log::warn!("watcher missed events (queue overflow); doing a full rescan");
                v.rescan()
            }
            Ok(events) => {
                let mut paths: Vec<String> = events
                    .iter()
                    .flat_map(|e| e.paths.iter())
                    .filter_map(|p| mapper.to_vault_path(p))
                    .collect();
                paths.sort();
                paths.dedup();
                if paths.is_empty() {
                    return;
                }
                if paths.len() > FULL_RESCAN_THRESHOLD {
                    v.rescan()
                } else {
                    v.rescan_paths(&paths)
                }
            }
            Err(errs) => {
                log::warn!("watcher errors: {errs:?}; doing a full rescan");
                v.rescan()
            }
        };
        match result {
            Ok(changes) if !changes.is_empty() => {
                log::debug!("external changes: {changes:?}");
                let _ = app.emit("vault-changed", &changes);
                use tauri::Manager;
                if let Some(st) = app.try_state::<crate::AppState>() {
                    if let Some(s) = st.sync.lock().as_ref() {
                        s.poke();
                    }
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("rescan failed: {e}"),
        }
    })?;
    debouncer.watch(&root, RecursiveMode::Recursive)?;
    Ok(Watcher { _debouncer: debouncer })
}
