//! Runs sync for the open vault in a background thread and reports status
//! to the UI (`sync-status` events). Vault changes made by sync are emitted
//! as `sync-changed` so open notes and the tree update. Both events name
//! the vault they are for: a sync still finishes after the user switched
//! to another vault.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cairn_core::{Change, Vault};
use cairn_sync::crypto::DEFAULT_KDF;
use cairn_sync::engine::{Skipped, SyncEngine, SyncSettings};
use cairn_sync::protocol::{valid_id, ChangesResponse, HistoryEntry, KeyEnvelope, PutRevision, RevisionBlob, VaultInfo};
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};
use cairn_sync::SyncError;
use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// How often to sync when nothing happens locally.
const INTERVAL: Duration = Duration::from_secs(60);
/// Wait this long after a local change before syncing (to batch typing).
const DEBOUNCE: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// The vault this is for (as `open_vault` returned it).
    pub root: String,
    pub configured: bool,
    /// "off", "idle", "syncing" or "error"
    pub state: String,
    pub server: Option<String>,
    pub vault_id: Option<String>,
    pub device: Option<String>,
    /// Unix milliseconds.
    pub last_sync: Option<i64>,
    pub last_error: Option<String>,
    pub last_pulled: usize,
    pub last_pushed: usize,
    /// Conflict copies created by recent syncs that are still there
    /// (newest first).
    pub conflicts: Vec<String>,
    /// Files the last successful sync left out, and why.
    pub skipped: Vec<Skipped>,
}

#[derive(Serialize, Clone)]
struct SyncChanges<'a> {
    root: &'a str,
    changes: &'a [Change],
}

struct Inner {
    engine: Option<SyncEngine>,
    status: SyncStatus,
}

pub struct SyncManager {
    inner: Mutex<Inner>,
    /// Serializes sync runs on this vault's state folder, also with the
    /// manager it had before being opened again (see `run_lock`).
    running: Arc<Mutex<()>>,
    wake: Condvar,
    poked: Mutex<Option<Instant>>,
    app: AppHandle,
    dir: PathBuf,
    vault: Arc<Vault>,
    root: String,
    stopped: Mutex<bool>,
    /// Gives up the server requests of the running sync or setup.
    cancel: Arc<Cancel>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Folder for one vault's sync state, keyed by the vault's location.
pub fn state_dir(app_data: &std::path::Path, vault_root: &str) -> PathBuf {
    let h = blake3_hex(vault_root.as_bytes());
    app_data.join("sync").join(&h[..16])
}

fn blake3_hex(b: &[u8]) -> String {
    cairn_core::index::hash_hex(&cairn_core::index::hash_bytes(b))
}

/// The lock for sync runs on one state folder, shared by every manager of
/// that folder: when a vault is opened again while it syncs, the stopped
/// manager's sync ends before the new manager's starts.
fn run_lock(dir: &Path) -> Arc<Mutex<()>> {
    static LOCKS: Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());
    LOCKS.lock().entry(dir.to_path_buf()).or_default().clone()
}

impl SyncManager {
    pub fn new(app: AppHandle, vault: Arc<Vault>, dir: PathBuf, root: String) -> Arc<SyncManager> {
        let cancel = Arc::new(Cancel::default());
        let engine = match load_engine(vault.clone(), &dir, cancel.clone()) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("sync state unreadable: {e}");
                None
            }
        };
        let mut status = SyncStatus { state: "off".into(), ..Default::default() };
        if let Some(e) = &engine {
            fill_settings(&mut status, e.settings());
            status.state = "idle".into();
            status.conflicts = read_conflicts(&dir);
        }
        let m = Arc::new(SyncManager {
            inner: Mutex::new(Inner { engine, status }),
            running: run_lock(&dir),
            wake: Condvar::new(),
            poked: Mutex::new(None),
            app,
            dir,
            vault,
            root,
            stopped: Mutex::new(false),
            cancel,
        });
        let bg = m.clone();
        std::thread::Builder::new()
            .name("cairn-sync".into())
            .spawn(move || bg.run_loop())
            .expect("cannot start sync thread");
        m
    }

    pub fn status(&self) -> SyncStatus {
        let mut st = self.inner.lock().status.clone();
        st.root = self.root.clone();
        // A conflict copy the user deleted or renamed is dealt with.
        let index = self.vault.index();
        st.conflicts.retain(|c| index.entry(c).is_some());
        st
    }

    /// Send an event to the UI, unless this vault was closed meanwhile.
    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let stopped = self.stopped.lock();
        if !*stopped {
            let _ = self.app.emit(event, payload);
        }
    }

    fn emit_status(&self) {
        self.emit("sync-status", self.status());
    }

    /// Note a local change; a sync follows shortly.
    pub fn poke(&self) {
        let mut p = self.poked.lock();
        *p = Some(Instant::now());
        self.wake.notify_all();
    }

    pub fn stop(&self) {
        *self.stopped.lock() = true;
        let _p = self.poked.lock();
        self.wake.notify_all();
    }

    fn run_loop(self: Arc<Self>) {
        // First sync shortly after opening the vault.
        let mut next = Instant::now() + Duration::from_secs(2);
        loop {
            {
                let mut p = self.poked.lock();
                loop {
                    if *self.stopped.lock() {
                        return;
                    }
                    let due = match *p {
                        Some(t) => next.min(t + DEBOUNCE),
                        None => next,
                    };
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    self.wake.wait_for(&mut p, due - now);
                }
                *p = None;
            }
            if self.inner.lock().engine.is_some() {
                self.sync_now();
            }
            next = Instant::now() + INTERVAL;
        }
    }

    /// Run one sync now (blocking). Safe to call from any thread.
    pub fn sync_now(&self) -> SyncStatus {
        let _run = self.running.lock();
        // A newer manager has the vault now.
        if *self.stopped.lock() {
            return self.status();
        }
        // A cancel from before was meant for an earlier sync or setup.
        self.cancel.reset();
        self.run_sync()
    }

    /// One sync, with `running` held.
    fn run_sync(&self) -> SyncStatus {
        let mut engine = {
            let mut inner = self.inner.lock();
            let Some(e) = inner.engine.take() else {
                drop(inner);
                return self.status();
            };
            inner.status.state = "syncing".into();
            e
        };
        self.emit_status();
        let mut result = engine.sync();
        let mut engine = Some(engine);
        if matches!(result, Err(SyncError::NotConfigured)) {
            // Another app instance on this vault turned sync off, or
            // connected it again: go on with what it set up, if anything.
            engine = load_engine(self.vault.clone(), &self.dir, self.cancel.clone()).unwrap_or_else(|e| {
                log::warn!("sync state unreadable: {e}");
                None
            });
            if let Some(e) = engine.as_mut() {
                result = e.sync();
            }
        }
        {
            let mut inner = self.inner.lock();
            // Turned off while syncing: the status says so already.
            if inner.status.configured {
                match &engine {
                    // As another app instance connected it, maybe.
                    Some(e) => fill_settings(&mut inner.status, e.settings()),
                    None => inner.status = SyncStatus { state: "off".into(), ..Default::default() },
                }
                match &result {
                    Ok(r) => {
                        inner.status.state = "idle".into();
                        inner.status.last_sync = Some(now_ms());
                        inner.status.last_error = None;
                        inner.status.last_pulled = r.pulled;
                        inner.status.last_pushed = r.pushed;
                        inner.status.skipped = r.skipped.clone();
                        if !r.conflicts.is_empty() {
                            let index = self.vault.index();
                            // A copy deleted earlier can come back under its name.
                            let older = inner.status.conflicts.iter().filter(|c| !r.conflicts.contains(c) && index.entry(c).is_some()).cloned();
                            let mut list: Vec<String> = r.conflicts.iter().cloned().chain(older).collect();
                            list.truncate(20);
                            write_conflicts(&self.dir, &list);
                            inner.status.conflicts = list;
                        }
                        if r.pulled + r.pushed > 0 {
                            log::info!("sync: pulled {}, pushed {}, conflicts {}", r.pulled, r.pushed, r.conflicts.len());
                        }
                    }
                    // Turned off in another app instance.
                    Err(_) if engine.is_none() => {}
                    Err(e) => {
                        log::warn!("sync failed: {e}");
                        inner.status.state = "error".into();
                        inner.status.last_error = Some(e.to_string());
                    }
                }
                inner.engine = engine;
            }
        }
        if let Ok(r) = &result
            && !r.changes.is_empty()
        {
            self.emit("sync-changed", SyncChanges { root: &self.root, changes: &r.changes });
        }
        self.emit_status();
        self.status()
    }

    /// Whether the server has the vault named in `settings`. Only asks: a
    /// vault it does not have is not created. The setup form asks the user
    /// before [`Self::setup`] creates one, as a mistyped name would start a
    /// second, empty vault. [`Self::cancel`] gives it up.
    pub fn vault_exists(&self, settings: &SyncSettings) -> Result<bool, String> {
        if !valid_id(&settings.vault_id) {
            return Err("the notebook name may only contain letters, digits, - and _ (up to 64 characters)".into());
        }
        // Like setup: a setup given up meanwhile sees that before this resets it.
        let _run = self.running.lock();
        self.cancel.reset();
        let found = Cancellable::http(settings, self.cancel.clone()).get_vault(&settings.vault_id);
        if self.cancel.is_set() {
            return Err(CANCELLED.into());
        }
        found.map(|v| v.is_some()).map_err(|e| e.to_string())
    }

    /// Connect to a server (creating or unlocking the vault there) and sync.
    /// [`Self::cancel`] gives it up; then nothing stays set up.
    pub fn setup(&self, settings: SyncSettings, passphrase: &str) -> Result<SyncStatus, String> {
        let run = self.running.lock();
        self.cancel.reset();
        let transport = Cancellable::http(&settings, self.cancel.clone());
        let engine = SyncEngine::connect_with(self.vault.clone(), &self.dir, settings, passphrase, Box::new(transport), DEFAULT_KDF);
        if self.cancel.is_set() {
            if engine.is_ok() {
                let _ = SyncEngine::disconnect(&self.dir);
            }
            return Err(CANCELLED.into());
        }
        let engine = engine.map_err(|e| e.to_string())?;
        {
            let mut inner = self.inner.lock();
            inner.status = SyncStatus { state: "idle".into(), ..Default::default() };
            fill_settings(&mut inner.status, engine.settings());
            inner.engine = Some(engine);
        }
        let status = self.run_sync();
        let cancelled = self.cancel.is_set();
        drop(run);
        if cancelled {
            // Given up during the first sync.
            self.disconnect()?;
            return Err(CANCELLED.into());
        }
        Ok(status)
    }

    /// Give up the setup or sync in progress: a request waiting on the
    /// server is dropped at once, and no further one is sent.
    pub fn cancel(&self) {
        self.cancel.set();
    }

    /// Turn sync off. A running sync is cancelled first, so this does not
    /// wait for a server that does not answer.
    pub fn disconnect(&self) -> Result<SyncStatus, String> {
        {
            let mut inner = self.inner.lock();
            inner.engine = None;
            inner.status = SyncStatus { state: "off".into(), ..Default::default() };
        }
        self.cancel.set();
        let _run = self.running.lock();
        SyncEngine::disconnect(&self.dir).map_err(|e| e.to_string())?;
        self.emit_status();
        Ok(self.status())
    }

    /// Put an old version of a file back. Syncs first, so that the current
    /// text is on the server (in the history) before it is replaced; if that
    /// sync fails, nothing is restored.
    pub fn restore(&self, path: &str, seq: u64) -> Result<Vec<cairn_core::Change>, String> {
        let st = self.sync_now();
        if st.state == "error" {
            let why = st.last_error.unwrap_or_default();
            return Err(format!("Nothing was restored: the current text could not be synced first ({why})."));
        }
        self.with_engine(|e| e.restore(path, seq))
    }

    /// Rename a file or folder in `vault` for the user, recording the rename
    /// for the next sync (see `cairn_sync::engine::rename`).
    pub fn rename(&self, vault: &Arc<Vault>, from: &str, to: &str) -> cairn_core::Result<Vec<Change>> {
        // Not this manager's vault: one is being opened.
        if !Arc::ptr_eq(vault, &self.vault) {
            return vault.rename(from, to);
        }
        cairn_sync::engine::rename(vault, &self.dir, from, to)
    }

    pub fn with_engine<T>(&self, f: impl FnOnce(&SyncEngine) -> Result<T, SyncError>) -> Result<T, String> {
        let _run = self.running.lock();
        let mut inner = self.inner.lock();
        let e = inner.engine.as_mut().ok_or_else(|| "sync is not set up".to_string())?;
        // With what the last sync saved, also one this manager did not run
        // (before the vault was opened again, or in another app instance).
        e.refresh().map_err(|e| e.to_string())?;
        f(e).map_err(|e| e.to_string())
    }

    /// Read from the server (version history) without waiting for a running
    /// sync, which has the engine: then use another one loaded from the
    /// saved configuration.
    pub fn read<T>(&self, f: impl FnOnce(&SyncEngine) -> Result<T, SyncError>) -> Result<T, String> {
        let not_set_up = || "sync is not set up".to_string();
        if let Some(_run) = self.running.try_lock() {
            let mut inner = self.inner.lock();
            let e = inner.engine.as_mut().ok_or_else(not_set_up)?;
            // As in `with_engine`: with what the last sync saved.
            e.refresh().map_err(|e| e.to_string())?;
            return f(e).map_err(|e| e.to_string());
        }
        if !self.inner.lock().status.configured {
            return Err(not_set_up());
        }
        let e = load_engine(self.vault.clone(), &self.dir, Arc::default()).map_err(|e| e.to_string())?.ok_or_else(not_set_up)?;
        f(&e).map_err(|e| e.to_string())
    }
}

const CANCELLED: &str = "Cancelled.";

fn cancelled() -> SyncError {
    SyncError::Local("sync was cancelled".into())
}

/// [`SyncEngine::load`], with server requests that `cancel` can give up.
fn load_engine(vault: Arc<Vault>, dir: &Path, cancel: Arc<Cancel>) -> Result<Option<SyncEngine>, SyncError> {
    let Ok(cfg) = std::fs::read(dir.join("config.json")) else { return Ok(None) };
    let settings: SyncSettings = serde_json::from_slice(&cfg).map_err(|e| SyncError::Local(e.to_string()))?;
    let transport = Cancellable::http(&settings, cancel);
    SyncEngine::load_with(vault, dir, settings, Box::new(transport)).map(Some)
}

/// Conflict copies are listed until the user deletes or renames them, also
/// after a restart.
fn read_conflicts(dir: &Path) -> Vec<String> {
    std::fs::read(dir.join("conflicts.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_conflicts(dir: &Path, list: &[String]) {
    let r = serde_json::to_vec(list).map_err(std::io::Error::other).and_then(|b| std::fs::write(dir.join("conflicts.json"), b));
    if let Err(e) = r {
        log::warn!("cannot save the list of conflict copies: {e}");
    }
}

/// A flag that gives up a sync or setup waiting on the server.
#[derive(Default)]
struct Cancel {
    set: Mutex<bool>,
    wake: Condvar,
}

impl Cancel {
    fn set(&self) {
        *self.set.lock() = true;
        self.wake.notify_all();
    }

    fn reset(&self) {
        *self.set.lock() = false;
    }

    fn is_set(&self) -> bool {
        *self.set.lock()
    }
}

/// A transport whose requests a [`Cancel`] gives up. A request that only
/// reads waits on its own thread, so that a server that does not answer is
/// not waited for; its answer is dropped, as if the connection had broken.
/// Uploads finish (or time out) once started, so that this device knows
/// what the server stored; the next one is not sent.
struct Cancellable<T> {
    inner: Arc<T>,
    cancel: Arc<Cancel>,
}

impl Cancellable<HttpTransport> {
    fn http(settings: &SyncSettings, cancel: Arc<Cancel>) -> Self {
        Cancellable { inner: Arc::new(HttpTransport::new(&settings.server, &settings.token)), cancel }
    }
}

impl<T: Transport + 'static> Cancellable<T> {
    fn check(&self) -> Result<(), SyncError> {
        if self.cancel.is_set() {
            return Err(cancelled());
        }
        Ok(())
    }

    fn read<R: Send + 'static>(&self, f: impl FnOnce(&T) -> Result<R, SyncError> + Send + 'static) -> Result<R, SyncError> {
        self.check()?;
        let answer = Arc::new(Mutex::new(None));
        let (inner, cancel, slot) = (self.inner.clone(), self.cancel.clone(), answer.clone());
        std::thread::Builder::new()
            .name("cairn-sync-request".into())
            .spawn(move || {
                let r = f(&inner);
                *slot.lock() = Some(r);
                let _set = cancel.set.lock();
                cancel.wake.notify_all();
            })
            .map_err(|e| SyncError::Local(e.to_string()))?;
        let mut set = self.cancel.set.lock();
        loop {
            if let Some(r) = answer.lock().take() {
                return r;
            }
            if *set {
                return Err(cancelled());
            }
            self.cancel.wake.wait(&mut set);
        }
    }
}

impl<T: Transport + 'static> Transport for Cancellable<T> {
    fn get_vault(&self, vault: &str) -> Result<Option<VaultInfo>, SyncError> {
        let vault = vault.to_string();
        self.read(move |t| t.get_vault(&vault))
    }

    fn create_vault(&self, vault: &str, keys: &KeyEnvelope) -> Result<(), SyncError> {
        self.check()?;
        self.inner.create_vault(vault, keys)
    }

    fn changes(&self, vault: &str, since: u64, limit: u32) -> Result<ChangesResponse, SyncError> {
        let vault = vault.to_string();
        self.read(move |t| t.changes(&vault, since, limit))
    }

    fn put(&self, vault: &str, file_id: &str, rev: &PutRevision) -> Result<PutOutcome, SyncError> {
        self.check()?;
        self.inner.put(vault, file_id, rev)
    }

    /// Not a request. Without it the engine reads and encrypts each file
    /// over the limit on every sync, only for `put` to refuse it.
    fn max_file_size(&self) -> u64 {
        self.inner.max_file_size()
    }

    fn history(&self, vault: &str, file_id: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        let (vault, file_id) = (vault.to_string(), file_id.to_string());
        self.read(move |t| t.history(&vault, &file_id))
    }

    fn revision(&self, vault: &str, seq: u64) -> Result<RevisionBlob, SyncError> {
        let vault = vault.to_string();
        self.read(move |t| t.revision(&vault, seq))
    }
}

fn fill_settings(st: &mut SyncStatus, s: &SyncSettings) {
    st.configured = true;
    st.server = Some(s.server.clone());
    st.vault_id = Some(s.vault_id.clone());
    st.device = Some(s.device.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that takes files of at most `max` bytes. Only the limit is
    /// asked for.
    struct Small {
        max: u64,
    }

    impl Transport for Small {
        fn get_vault(&self, _: &str) -> Result<Option<VaultInfo>, SyncError> {
            unreachable!()
        }
        fn create_vault(&self, _: &str, _: &KeyEnvelope) -> Result<(), SyncError> {
            unreachable!()
        }
        fn changes(&self, _: &str, _: u64, _: u32) -> Result<ChangesResponse, SyncError> {
            unreachable!()
        }
        fn put(&self, _: &str, _: &str, _: &PutRevision) -> Result<PutOutcome, SyncError> {
            unreachable!()
        }
        fn history(&self, _: &str, _: &str) -> Result<Vec<HistoryEntry>, SyncError> {
            unreachable!()
        }
        fn revision(&self, _: &str, _: u64) -> Result<RevisionBlob, SyncError> {
            unreachable!()
        }
        fn max_file_size(&self) -> u64 {
            self.max
        }
    }

    /// The engine skips a file over the limit before reading it, but only
    /// if the transport it is given reports the limit.
    #[test]
    fn cancellable_keeps_the_upload_size_limit() {
        let t = Cancellable { inner: Arc::new(Small { max: 1000 }), cancel: Arc::default() };
        assert_eq!(t.max_file_size(), 1000);

        // The transport every engine in the app gets.
        let settings = SyncSettings { server: "http://127.0.0.1:9".into(), token: String::new(), vault_id: "v".into(), device: "d".into() };
        let http = Cancellable::http(&settings, Arc::default());
        assert_eq!(http.max_file_size(), HttpTransport::new(&settings.server, &settings.token).max_file_size());
        assert!(http.max_file_size() < u64::MAX);
    }
}
