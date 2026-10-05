//! Shared harness for the adversarial sync-semantics tests
//! (adv_sync_semantics*.rs). Included with `#[path] mod common;`.
//!
//! * `Server`: the real cairn-server in-process over HTTP.
//! * `Device`: a vault + sync state + engine, any number of them per server.
//!   Its transport has hooks that run another device's sync (or a local
//!   "user" edit) in the middle of this device's sync.
//! * `CaseInsensitiveFs`: a `VaultFs` that behaves like the default macOS /
//!   Windows file systems (case-insensitive, case-preserving).
//! * `FatNamesFs`: a `VaultFs` that refuses names that Windows and Android
//!   shared storage refuse (`"*:<>?\|`, trailing dot or space).
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cairn_core::{CoreError, EntryKind, FileStat, StdFs, TrashMode, Vault, VaultFs};
use cairn_sync::engine::{SyncEngine, SyncReport, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};
use cairn_sync::SyncError;
use parking_lot::Mutex;

pub const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
pub const TOKEN: &str = "test-token-0123456789";
pub const PASS: &str = "correct horse battery";

// ---------------------------------------------------------------- server

pub struct Server {
    pub url: String,
    pub db_path: PathBuf,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

pub fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body: 200 << 20 });
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { url: format!("http://{addr}"), db_path, _rt: rt, _dir: dir }
}

// ---------------------------------------------------------------- hooked transport

pub type Hook = Box<dyn FnOnce() + Send>;

#[derive(Default)]
pub struct Hooks {
    /// Runs once, right after the next `changes` call returned (between the
    /// pull's fetch and applying it).
    pub after_changes: Mutex<Option<Hook>>,
    /// Runs once, right before the next upload.
    pub before_put: Mutex<Option<Hook>>,
    /// Number of 409 answers seen.
    pub conflicts: AtomicUsize,
    pub puts: AtomicUsize,
    /// Uploads of a delete, and how many of them got a 409.
    pub delete_puts: AtomicUsize,
    pub refused_deletes: AtomicUsize,
    /// Records per changes page (0: the engine's).
    pub page: AtomicUsize,
    /// This many of the next history requests fail (HTTP 500).
    pub fail_history: AtomicUsize,
}

pub struct Hooked {
    inner: HttpTransport,
    hooks: Arc<Hooks>,
}

impl Transport for Hooked {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.inner.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.inner.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        let l = match self.hooks.page.load(Ordering::SeqCst) {
            0 => l,
            n => n as u32,
        };
        let r = self.inner.changes(v, s, l);
        let h = self.hooks.after_changes.lock().take();
        if let Some(h) = h {
            h();
        }
        r
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        let h = self.hooks.before_put.lock().take();
        if let Some(h) = h {
            h();
        }
        let out = self.inner.put(v, f, r)?;
        self.hooks.puts.fetch_add(1, Ordering::SeqCst);
        self.hooks.delete_puts.fetch_add(r.deleted as usize, Ordering::SeqCst);
        if matches!(out, PutOutcome::Conflict(_)) {
            self.hooks.conflicts.fetch_add(1, Ordering::SeqCst);
            self.hooks.refused_deletes.fetch_add(r.deleted as usize, Ordering::SeqCst);
        }
        Ok(out)
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        if self.hooks.fail_history.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {
            return Err(SyncError::Server("HTTP 500".into()));
        }
        self.inner.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.inner.revision(v, s)
    }
}

// ---------------------------------------------------------------- device

pub struct Device {
    pub name: String,
    pub root: PathBuf,
    pub state_dir: PathBuf,
    pub vault: Arc<Vault>,
    pub engine: SyncEngine,
    pub hooks: Arc<Hooks>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

pub fn std_fs(root: &Path) -> Arc<dyn VaultFs> {
    Arc::new(StdFs::new(root, TrashMode::Vault).unwrap())
}

impl Device {
    pub fn new(srv: &Server, name: &str, files: &[(&str, &str)]) -> Device {
        Self::with_fs(srv, name, files, std_fs)
    }

    pub fn with_fs(srv: &Server, name: &str, files: &[(&str, &str)], make_fs: impl FnOnce(&Path) -> Arc<dyn VaultFs>) -> Device {
        let vd = tempfile::Builder::new().prefix("ss-vault-").tempdir().unwrap();
        let sd = tempfile::Builder::new().prefix("ss-state-").tempdir().unwrap();
        for (p, c) in files {
            let abs = vd.path().join(p);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, c).unwrap();
        }
        let root = vd.path().canonicalize().unwrap();
        let vault = Arc::new(Vault::open(make_fs(&root)).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: name.into() };
        let hooks = Arc::new(Hooks::default());
        let transport = Box::new(Hooked { inner: HttpTransport::new(&srv.url, TOKEN), hooks: hooks.clone() });
        let engine = SyncEngine::connect_with(vault.clone(), sd.path(), settings, PASS, transport, FAST_KDF).unwrap();
        Device { name: name.into(), root, state_dir: sd.path().to_path_buf(), vault, engine, hooks, _dirs: (vd, sd) }
    }

    pub fn try_sync(&mut self) -> Result<SyncReport, SyncError> {
        self.engine.sync()
    }

    /// From now on pull one record per changes page, each in a batch of its
    /// own (see `SyncEngine::set_pull_batch`).
    pub fn pull_one_by_one(&mut self) {
        self.hooks.page.store(1, Ordering::SeqCst);
        self.engine.set_pull_batch(1);
    }

    pub fn sync(&mut self) -> SyncReport {
        match self.engine.sync() {
            Ok(r) => r,
            Err(e) => panic!("sync on {} failed: {e}", self.name),
        }
    }

    pub fn abs(&self, p: &str) -> PathBuf {
        self.root.join(p)
    }

    pub fn write(&self, p: &str, c: &str) {
        self.write_bytes(p, c.as_bytes());
    }

    pub fn write_bytes(&self, p: &str, c: &[u8]) {
        let abs = self.abs(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        // make sure the mtime moves (stat has millisecond resolution)
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(abs, c).unwrap();
    }

    pub fn read(&self, p: &str) -> Option<String> {
        fs::read_to_string(self.abs(p)).ok()
    }

    pub fn exists(&self, p: &str) -> bool {
        self.abs(p).exists()
    }

    pub fn rm(&self, p: &str) {
        fs::remove_file(self.abs(p)).unwrap();
    }

    pub fn mv(&self, a: &str, b: &str) {
        let to = self.abs(b);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::rename(self.abs(a), to).unwrap();
    }

    /// Rename in the app, as its rename command does: the rename is recorded
    /// for the next sync (`cairn_sync::engine::rename`). The vault is
    /// rescanned first, as the app's file watcher would have, and a missing
    /// folder for the new name is made, as the user would make it first.
    pub fn app_mv(&self, a: &str, b: &str) {
        self.vault.rescan().unwrap();
        self.vault.ensure_folder(cairn_core::path::parent(b)).unwrap();
        cairn_sync::engine::rename(&self.vault, &self.state_dir, a, b).unwrap();
    }

    /// Visible files with their contents (sorted). Hidden entries
    /// (`.trash`, `.archive`, ...) are skipped.
    pub fn files(&self) -> Vec<(String, String)> {
        self.files_bytes().into_iter().map(|(p, b)| (p, String::from_utf8_lossy(&b).into_owned())).collect()
    }

    pub fn files_bytes(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, false, &mut out);
        out.sort();
        out
    }

    pub fn paths(&self) -> Vec<String> {
        self.files_bytes().into_iter().map(|f| f.0).collect()
    }

    /// Everything in the folder, hidden folders and the vault trash included.
    pub fn all_text(&self) -> String {
        let mut v = Vec::new();
        walk(&self.root, &self.root, true, &mut v);
        let mut out = String::new();
        for (_, b) in v {
            out.push_str(&String::from_utf8_lossy(&b));
            out.push('\n');
        }
        out
    }

    /// Everything except the vault trash (visible files and hidden folders).
    pub fn non_trash_text(&self) -> String {
        let mut out = String::new();
        let Ok(rd) = fs::read_dir(&self.root) else { return out };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".trash" || name.contains(".cairn-tmp-") {
                continue;
            }
            let mut v = Vec::new();
            if e.path().is_dir() {
                walk(&self.root, &e.path(), true, &mut v);
            } else {
                v.push((name, fs::read(e.path()).unwrap_or_default()));
            }
            for (_, b) in v {
                out.push_str(&String::from_utf8_lossy(&b));
                out.push('\n');
            }
        }
        out
    }

    pub fn trash_text(&self) -> String {
        let t = self.root.join(".trash");
        let mut v = Vec::new();
        if t.exists() {
            walk(&t, &t, true, &mut v);
        }
        v.into_iter().map(|(_, b)| String::from_utf8_lossy(&b).into_owned()).collect::<Vec<_>>().join("\n")
    }

    pub fn conflict_copies(&self) -> Vec<String> {
        self.paths().into_iter().filter(|p| p.contains("(conflict ")).collect()
    }
}

pub fn walk(root: &Path, dir: &Path, hidden: bool, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && (!hidden || name.contains(".cairn-tmp-")) {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, hidden, out);
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, fs::read(&p).unwrap_or_default()));
        }
    }
}

/// Sync every device in order until a whole pass pushes nothing, then
/// require identical trees. Returns the number of passes.
pub fn converge(devs: &mut [&mut Device]) -> usize {
    for pass in 1..=8 {
        let mut pushed = 0;
        for d in devs.iter_mut() {
            pushed += d.sync().pushed;
        }
        if pushed == 0 {
            let first = devs[0].files_bytes();
            for d in devs.iter().skip(1) {
                let other = d.files_bytes();
                if other != first {
                    let a: Vec<String> = first.iter().map(|f| f.0.clone()).collect();
                    let b: Vec<String> = other.iter().map(|f| f.0.clone()).collect();
                    panic!(
                        "devices did not converge: {} has {:?}\n{} has {:?}\n(contents: {:?} vs {:?})",
                        devs[0].name,
                        a,
                        d.name,
                        b,
                        devs[0].files(),
                        d.files()
                    );
                }
            }
            return pass;
        }
    }
    panic!("sync never settled: devices kept pushing after 8 passes");
}

/// Like `converge` but returns a description instead of panicking.
pub fn try_converge(devs: &mut [&mut Device]) -> Result<usize, String> {
    for pass in 1..=8 {
        let mut pushed = 0;
        for d in devs.iter_mut() {
            match d.try_sync() {
                Ok(r) => pushed += r.pushed,
                Err(e) => return Err(format!("sync on {} failed in pass {pass}: {e}", d.name)),
            }
        }
        if pushed == 0 {
            let first = devs[0].files_bytes();
            for d in devs.iter().skip(1) {
                let other = d.files_bytes();
                if other != first {
                    return Err(format!("devices did not converge: {}", tree_diff(&devs[0].name, &first, &d.name, &other)));
                }
            }
            return Ok(pass);
        }
    }
    Err("sync never settled: devices kept pushing after 8 passes".into())
}

/// Human-readable difference between two trees.
pub fn tree_diff(an: &str, a: &[(String, Vec<u8>)], bn: &str, b: &[(String, Vec<u8>)]) -> String {
    use std::collections::BTreeMap;
    let am: BTreeMap<&str, &[u8]> = a.iter().map(|(p, c)| (p.as_str(), c.as_slice())).collect();
    let bm: BTreeMap<&str, &[u8]> = b.iter().map(|(p, c)| (p.as_str(), c.as_slice())).collect();
    let mut out = Vec::new();
    for (p, c) in &am {
        match bm.get(p) {
            None => out.push(format!("only on {an}: {p:?} = {:?}", String::from_utf8_lossy(c))),
            Some(c2) if c2 != c => out.push(format!(
                "differs {p:?}: {an} {:?} vs {bn} {:?}",
                String::from_utf8_lossy(c),
                String::from_utf8_lossy(c2)
            )),
            _ => {}
        }
    }
    for (p, c) in &bm {
        if !am.contains_key(p) {
            out.push(format!("only on {bn}: {p:?} = {:?}", String::from_utf8_lossy(c)));
        }
    }
    out.join("; ")
}

// ---------------------------------------------------------------- simulated file systems

/// A case-insensitive, case-preserving view of a folder, like APFS / NTFS
/// defaults: `Note.md` and `note.md` are the same file.
pub struct CaseInsensitiveFs {
    inner: StdFs,
    root: PathBuf,
}

impl CaseInsensitiveFs {
    pub fn new(root: &Path) -> Arc<dyn VaultFs> {
        Arc::new(CaseInsensitiveFs { inner: StdFs::new(root, TrashMode::Vault).unwrap(), root: root.to_path_buf() })
    }

    /// Map each component to an existing entry that matches ignoring case.
    fn resolve(&self, p: &str) -> String {
        let mut real: Vec<String> = Vec::new();
        for comp in p.split('/').filter(|c| !c.is_empty()) {
            let mut dir = self.root.clone();
            for r in &real {
                dir.push(r);
            }
            let found = fs::read_dir(&dir).ok().and_then(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .find(|n| n.to_lowercase() == comp.to_lowercase())
            });
            real.push(found.unwrap_or_else(|| comp.to_string()));
        }
        real.join("/")
    }

    fn with_path(st: Option<FileStat>, p: &str) -> Option<FileStat> {
        st.map(|mut s| {
            s.path = p.to_string();
            s
        })
    }
}

impl VaultFs for CaseInsensitiveFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        self.inner.list(&self.resolve(dir))
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        Ok(Self::with_path(self.inner.stat(&self.resolve(path))?, path))
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(&self.resolve(path))
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        // Writing `note.md` when `Note.md` exists writes into `Note.md`.
        let st = self.inner.write(&self.resolve(path), data)?;
        Ok(FileStat { path: path.to_string(), ..st })
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(&self.resolve(path))
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        let rf = self.resolve(from);
        let rt = self.resolve(to);
        if rf.to_lowercase() != rt.to_lowercase() && self.inner.stat(&rt)?.is_some() {
            return Err(CoreError::AlreadyExists(to.to_string()));
        }
        // keep the requested case for the last component
        let parent = cairn_core::path::parent(&rt);
        let target = cairn_core::path::join(parent, cairn_core::path::file_name(to));
        self.inner.rename(&rf, &target)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(&self.resolve(path))
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(&self.resolve(path))
    }
    fn describe(&self) -> String {
        format!("case-insensitive {}", self.inner.describe())
    }
}

/// Refuses names that Windows and Android's shared storage refuse.
pub struct FatNamesFs {
    inner: StdFs,
}

impl FatNamesFs {
    pub fn new(root: &Path) -> Arc<dyn VaultFs> {
        Arc::new(FatNamesFs { inner: StdFs::new(root, TrashMode::Vault).unwrap() })
    }

    fn check(path: &str) -> cairn_core::Result<()> {
        for c in path.split('/') {
            if c.contains(['"', '*', ':', '<', '>', '?', '\\', '|']) || c.ends_with('.') || c.ends_with(' ') {
                return Err(CoreError::Io(format!("{path}: Invalid argument (os error 22)")));
            }
        }
        Ok(())
    }
}

impl VaultFs for FatNamesFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        self.inner.list(dir)
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        self.inner.stat(path)
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        Self::check(path)?;
        self.inner.write(path, data)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        Self::check(path)?;
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        Self::check(to)?;
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
}

pub fn is_dir_entry(st: &FileStat) -> bool {
    st.kind == EntryKind::Dir
}

// ---------------------------------------------------------------- misc

/// xorshift64 PRNG.
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

pub fn set_mtime(p: &Path, t: std::time::SystemTime) {
    let f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(t).unwrap();
}

pub fn mtime(p: &Path) -> std::time::SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}
