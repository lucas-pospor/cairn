//! Shared harness for the adversarial sync robustness tests
//! (adv_sync_robust*.rs). Included with `#[path] mod common;`.
//!
//! * `Server`: the real cairn-server in-process over HTTP (like two_devices.rs).
//! * `Device`: a vault + state dir + engine that can be crashed and reloaded.
//! * `FaultTransport`: wraps any Transport; a closure decides per call whether
//!   to fail before/after the real call or to panic (simulated crash).
//! * `HookFs`: wraps a VaultFs; a closure runs before each mutation (to
//!   simulate a crash or a concurrent user edit).
#![allow(dead_code)]

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cairn_core::{FileStat, StdFs, TrashMode, Vault, VaultFs};
use cairn_sync::engine::{SyncEngine, SyncReport, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};
use cairn_sync::SyncError;
use parking_lot::Mutex;

pub const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
pub const TOKEN: &str = "test-token-0123456789";
pub const PASS: &str = "correct horse battery";
pub const VAULT_ID: &str = "notes";

// ---------------------------------------------------------------- server

pub struct Server {
    pub url: String,
    pub db_path: PathBuf,
    pub rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

pub fn server() -> Server {
    server_with_body(50 << 20)
}

pub fn server_with_body(max_body: usize) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body });
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { url: format!("http://{addr}"), db_path, rt, _dir: dir }
}

pub fn http(url: &str) -> Box<dyn Transport> {
    Box::new(HttpTransport::new(url, TOKEN))
}

// ---------------------------------------------------------------- transport faults

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    None,
    /// Return a network error without contacting the server.
    ErrBefore,
    /// Make the real call, then pretend the response was lost.
    ErrAfter,
    /// Simulated crash before the call.
    PanicBefore,
    /// Simulated crash after the server processed the call.
    PanicAfter,
}

/// `op` is one of get_vault, create_vault, changes, put, history, revision.
pub type Decide = Box<dyn FnMut(&str, usize) -> Fault + Send>;
/// Runs before the call is forwarded (e.g. a user edit while the request is
/// in flight).
pub type BeforeHook = Box<dyn FnMut(&str, usize) + Send>;

pub struct FaultTransport {
    pub inner: Box<dyn Transport>,
    pub decide: Mutex<Decide>,
    pub before: Mutex<Option<BeforeHook>>,
    pub calls: Arc<AtomicUsize>,
    /// Sum of base64 blob bytes received in changes responses.
    pub blob_bytes_in: Arc<AtomicUsize>,
    pub blob_bytes_out: Arc<AtomicUsize>,
    pub puts: Arc<AtomicUsize>,
}

impl FaultTransport {
    pub fn new(inner: Box<dyn Transport>, decide: Decide) -> FaultTransport {
        FaultTransport {
            inner,
            decide: Mutex::new(decide),
            before: Mutex::new(None),
            calls: Arc::new(AtomicUsize::new(0)),
            blob_bytes_in: Arc::new(AtomicUsize::new(0)),
            blob_bytes_out: Arc::new(AtomicUsize::new(0)),
            puts: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn passthrough(inner: Box<dyn Transport>) -> FaultTransport {
        Self::new(inner, Box::new(|_, _| Fault::None))
    }

    fn run<T>(&self, op: &str, f: impl FnOnce() -> Result<T, SyncError>) -> Result<T, SyncError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = self.before.lock().as_mut() {
            h(op, n);
        }
        let fault = (self.decide.lock())(op, n);
        match fault {
            Fault::ErrBefore => return Err(SyncError::Network(format!("injected failure before {op} #{n}"))),
            Fault::PanicBefore => panic!("SIMULATED CRASH before {op} #{n}"),
            _ => {}
        }
        let r = f();
        match fault {
            Fault::ErrAfter => Err(SyncError::Network(format!("injected lost response after {op} #{n}"))),
            Fault::PanicAfter => panic!("SIMULATED CRASH after {op} #{n}"),
            _ => r,
        }
    }
}

impl Transport for FaultTransport {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.run("get_vault", || self.inner.get_vault(v))
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.run("create_vault", || self.inner.create_vault(v, k))
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        self.run("changes", || {
            let r = self.inner.changes(v, s, l)?;
            let n: usize = r.heads.iter().map(|h| h.blob.len()).sum();
            self.blob_bytes_in.fetch_add(n, Ordering::SeqCst);
            Ok(r)
        })
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.blob_bytes_out.fetch_add(r.blob.len(), Ordering::SeqCst);
        self.run("put", || self.inner.put(v, f, r))
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.run("history", || self.inner.history(v, f))
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.run("revision", || self.inner.revision(v, s))
    }
}

/// Share one FaultTransport between the engine and the test.
pub struct SharedTransport(pub Arc<FaultTransport>);

impl Transport for SharedTransport {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.0.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.0.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        self.0.changes(v, s, l)
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        self.0.put(v, f, r)
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.0.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.0.revision(v, s)
    }
}

/// Asks the server for at most this many records per changes page, so that
/// a small vault comes in as many pages as a large one does.
pub struct PageLimit(pub Box<dyn Transport>, pub u32);

impl Transport for PageLimit {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.0.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.0.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        self.0.changes(v, s, l.min(self.1))
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        self.0.put(v, f, r)
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.0.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.0.revision(v, s)
    }
    fn max_file_size(&self) -> u64 {
        self.0.max_file_size()
    }
}

// ---------------------------------------------------------------- fs hooks

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsOp {
    Read,
    Write,
    Rename,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsAction {
    Pass,
    /// Simulated crash before the operation reaches the disk.
    PanicBefore,
    /// Simulated crash right after the operation reached the disk.
    PanicAfter,
    /// The operation fails with an I/O error (e.g. a name the local file
    /// system does not accept).
    Fail,
}

pub type FsDecide = Box<dyn FnMut(FsOp, &str, usize) -> FsAction + Send>;
/// Runs after a read returned its data (the data is what the engine sees).
pub type AfterRead = Box<dyn FnMut(&str) + Send>;

pub struct HookFs {
    pub inner: Arc<dyn VaultFs>,
    pub decide: Mutex<Option<FsDecide>>,
    pub after_read: Mutex<Option<AfterRead>>,
    pub mutations: AtomicUsize,
}

impl HookFs {
    pub fn new(inner: Arc<dyn VaultFs>) -> HookFs {
        HookFs { inner, decide: Mutex::new(None), after_read: Mutex::new(None), mutations: AtomicUsize::new(0) }
    }

    fn check(&self, op: FsOp, path: &str) -> FsAction {
        let n = if op == FsOp::Read { usize::MAX } else { self.mutations.fetch_add(1, Ordering::SeqCst) };
        let mut d = self.decide.lock();
        match d.as_mut() {
            Some(f) => {
                let a = f(op, path, n);
                if a == FsAction::PanicBefore {
                    drop(d);
                    panic!("SIMULATED CRASH before {op:?} {path} (#{n})");
                }
                a
            }
            None => FsAction::Pass,
        }
    }

    fn after(&self, a: FsAction, op: FsOp, path: &str) {
        if a == FsAction::PanicAfter {
            panic!("SIMULATED CRASH after {op:?} {path}");
        }
    }
}

impl VaultFs for HookFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        self.inner.list(dir)
    }
    fn list_partial(&self, dir: &str) -> cairn_core::Result<(Vec<FileStat>, Vec<String>)> {
        self.inner.list_partial(dir)
    }
    fn refuses_case_twins(&self) -> bool {
        self.inner.refuses_case_twins()
    }
    fn under_skipped_link(&self, path: &str) -> bool {
        self.inner.under_skipped_link(path)
    }
    fn real_path(&self, path: &str) -> Option<String> {
        self.inner.real_path(path)
    }
    fn other_names(&self, path: &str) -> Vec<String> {
        self.inner.other_names(path)
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        self.inner.stat(path)
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        let a = self.check(FsOp::Read, path);
        let r = self.inner.read(path);
        self.after(a, FsOp::Read, path);
        let hook = self.after_read.lock().take();
        if let Some(mut h) = hook {
            h(path);
            // keep it installed (hooks decide themselves when to act)
            *self.after_read.lock() = Some(h);
        }
        r
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        let a = self.check(FsOp::Write, path);
        if a == FsAction::Fail {
            return Err(cairn_core::CoreError::Io(format!("{path}: Invalid argument (os error 22)")));
        }
        let r = self.inner.write(path, data);
        self.after(a, FsOp::Write, path);
        r
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        let a = self.check(FsOp::Rename, &format!("{from} -> {to}"));
        if a == FsAction::Fail {
            return Err(cairn_core::CoreError::Io(format!("{to}: Invalid argument (os error 22)")));
        }
        let r = self.inner.rename(from, to);
        self.after(a, FsOp::Rename, to);
        r
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        let a = self.check(FsOp::Remove, path);
        let r = self.inner.remove(path);
        self.after(a, FsOp::Remove, path);
        r
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn change_stamp(&self, path: &str) -> Option<u64> {
        self.inner.change_stamp(path)
    }
    fn skipped_backslash_names(&self) -> Vec<String> {
        self.inner.skipped_backslash_names()
    }
}

// ---------------------------------------------------------------- device

pub type FsMaker = Arc<dyn Fn(&Path) -> Arc<dyn VaultFs> + Send + Sync>;

pub struct Device {
    pub fs_maker: Option<FsMaker>,
    pub name: String,
    pub root: PathBuf,
    pub state_dir: PathBuf,
    pub url: String,
    pub hookfs: Arc<HookFs>,
    pub vault: Arc<Vault>,
    pub engine: Option<SyncEngine>,
    /// Records per changes page and bytes per pull batch for every engine
    /// this device loads (see `pull_in_batches`).
    pub pull: Option<(u32, u64)>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

pub fn settings(url: &str, device: &str) -> SyncSettings {
    SyncSettings { server: url.to_string(), token: TOKEN.into(), vault_id: VAULT_ID.into(), device: device.into() }
}

impl Device {
    pub fn new(srv: &Server, name: &str, files: &[(&str, &str)]) -> Device {
        Self::new_url(&srv.url, name, files)
    }

    pub fn new_url(url: &str, name: &str, files: &[(&str, &str)]) -> Device {
        Self::new_with_fs(url, name, files, None)
    }

    /// A device whose vault sits on a custom file system (wrapped in HookFs).
    pub fn new_with_fs(url: &str, name: &str, files: &[(&str, &str)], fs_maker: Option<FsMaker>) -> Device {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let abs = vd.path().join(p);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, c).unwrap();
        }
        let root = vd.path().canonicalize().unwrap();
        let base: Arc<dyn VaultFs> = match &fs_maker {
            Some(m) => m(&root),
            None => Arc::new(StdFs::new(&root, TrashMode::Vault).unwrap()),
        };
        let hookfs = Arc::new(HookFs::new(base));
        let vault = Arc::new(Vault::open(hookfs.clone()).unwrap());
        let engine = SyncEngine::connect_with(vault.clone(), sd.path(), settings(url, name), PASS, http(url), FAST_KDF).unwrap();
        Device {
            fs_maker,
            name: name.into(),
            root,
            state_dir: sd.path().to_path_buf(),
            url: url.into(),
            hookfs,
            vault,
            engine: Some(engine),
            pull: None,
            _dirs: (vd, sd),
        }
    }

    /// A handle on an existing vault folder without an engine (for hooks).
    pub fn stub(root: &Path, vault: Arc<Vault>, name: &str) -> Device {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        let hookfs = Arc::new(HookFs::new(Arc::new(StdFs::new(root, TrashMode::Vault).unwrap())));
        Device {
            fs_maker: None,
            name: name.into(),
            root: root.to_path_buf(),
            state_dir: sd.path().to_path_buf(),
            url: String::new(),
            hookfs,
            vault,
            engine: None,
            pull: None,
            _dirs: (vd, sd),
        }
    }

    pub fn engine(&mut self) -> &mut SyncEngine {
        self.engine.as_mut().expect("engine crashed; call restart()")
    }

    pub fn sync(&mut self) -> Result<SyncReport, SyncError> {
        self.engine().sync()
    }

    pub fn sync_ok(&mut self) -> SyncReport {
        let name = self.name.clone();
        self.sync().unwrap_or_else(|e| panic!("sync on {name} failed: {e}"))
    }

    /// Run a sync that may hit a simulated crash. Returns None on crash (the
    /// engine is gone; call restart()).
    pub fn sync_may_crash(&mut self) -> Option<Result<SyncReport, SyncError>> {
        let mut e = self.engine.take().expect("no engine");
        let r = catch_unwind(AssertUnwindSafe(|| e.sync()));
        match r {
            Ok(r) => {
                self.engine = Some(e);
                Some(r)
            }
            Err(_) => {
                // a crash: the in-memory engine state is lost
                drop(e);
                None
            }
        }
    }

    /// Simulate a process restart: reopen the vault from disk (fresh
    /// index), clear fs hooks, and load the engine from the state dir.
    pub fn restart(&mut self) {
        self.restart_with(http(&self.url.clone()));
    }

    pub fn restart_with(&mut self, t: Box<dyn Transport>) {
        self.engine = None;
        let base: Arc<dyn VaultFs> = match &self.fs_maker {
            Some(m) => m(&self.root),
            None => Arc::new(StdFs::new(&self.root, TrashMode::Vault).unwrap()),
        };
        let hookfs = Arc::new(HookFs::new(base));
        self.vault = Arc::new(Vault::open(hookfs.clone()).unwrap());
        self.hookfs = hookfs;
        self.load(t);
    }

    /// Swap the transport but keep the vault (no crash).
    pub fn set_transport(&mut self, t: Box<dyn Transport>) {
        self.engine = None;
        self.load(t);
    }

    fn load(&mut self, t: Box<dyn Transport>) {
        let t: Box<dyn Transport> = match self.pull {
            Some((page, _)) => Box::new(PageLimit(t, page)),
            None => t,
        };
        let mut engine = SyncEngine::load_with(self.vault.clone(), &self.state_dir, settings(&self.url, &self.name), t).unwrap();
        if let Some((_, bytes)) = self.pull {
            engine.set_pull_batch(bytes);
        }
        self.engine = Some(engine);
    }

    /// From now on pull `page` records per changes page, in batches of about
    /// `bytes` of records (see `SyncEngine::set_pull_batch`). Returns the
    /// transport, for the test to hook, until the next `restart` or
    /// `set_transport`.
    pub fn pull_in_batches(&mut self, page: u32, bytes: u64) -> Arc<FaultTransport> {
        self.pull = Some((page, bytes));
        let t = Arc::new(FaultTransport::passthrough(http(&self.url)));
        self.set_transport(Box::new(SharedTransport(t.clone())));
        t
    }

    pub fn write(&self, p: &str, c: &str) {
        self.write_bytes(p, c.as_bytes());
    }

    pub fn write_bytes(&self, p: &str, c: &[u8]) {
        let abs = self.root.join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(abs, c).unwrap();
    }

    pub fn read(&self, p: &str) -> Option<String> {
        fs::read_to_string(self.root.join(p)).ok()
    }

    pub fn rm(&self, p: &str) {
        fs::remove_file(self.root.join(p)).unwrap();
    }

    pub fn mv(&self, a: &str, b: &str) {
        let to = self.root.join(b);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::rename(self.root.join(a), to).unwrap();
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

    /// Visible files with contents, sorted (excludes dot-folders).
    pub fn files(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }

    pub fn paths(&self) -> Vec<String> {
        self.files().into_iter().map(|f| f.0).collect()
    }

    pub fn trash(&self) -> Vec<(String, String)> {
        let t = self.root.join(".trash");
        let mut out = Vec::new();
        if t.exists() {
            walk(&t, &t, &mut out);
        }
        out.sort();
        out
    }

    pub fn all_text(&self) -> String {
        let mut s = String::new();
        for (_, c) in self.files().into_iter().chain(self.trash()) {
            s.push_str(&c);
            s.push('\n');
        }
        s
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.json")
    }
}

pub fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, String::from_utf8_lossy(&fs::read(&p).unwrap_or_default()).to_string()));
        }
    }
}

pub fn converge(a: &mut Device, b: &mut Device) {
    for _ in 0..2 {
        a.sync_ok();
        b.sync_ok();
    }
    assert_eq!(a.files(), b.files(), "devices did not converge");
}

pub fn conflict_copies(files: &[(String, String)]) -> Vec<String> {
    files.iter().filter(|f| f.0.contains("(conflict ")).map(|f| f.0.clone()).collect()
}

/// xorshift64
pub struct Rng(pub u64);

impl Rng {
    // A random-number generator, not an iterator.
    #[allow(clippy::should_implement_trait)]
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
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

/// Quiet the default panic message for simulated crashes (they are expected).
pub fn quiet_simulated_crashes() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = info
                .payload()
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("");
            if msg.starts_with("SIMULATED CRASH") {
                return;
            }
            prev(info);
        }));
    });
}

// ---------------------------------------------------------------- case-insensitive fs

/// A case-insensitive, case-preserving file system (macOS APFS default,
/// Windows NTFS, exFAT/FAT SD cards, Android shared storage), modelled on
/// top of a Linux folder: every path component is matched against the
/// existing entries ignoring case, as those systems do.
pub struct CaseInsensitiveFs {
    root: PathBuf,
    inner: StdFs,
}

impl CaseInsensitiveFs {
    pub fn new(root: &Path) -> CaseInsensitiveFs {
        CaseInsensitiveFs { root: root.to_path_buf(), inner: StdFs::new(root, TrashMode::Vault).unwrap() }
    }

    fn resolve(&self, p: &str) -> String {
        let mut out: Vec<String> = Vec::new();
        for comp in p.split('/').filter(|c| !c.is_empty()) {
            let dir = out.iter().fold(self.root.clone(), |acc, c| acc.join(c));
            let found = fs::read_dir(&dir).ok().and_then(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .find(|n| n.to_lowercase() == comp.to_lowercase())
            });
            out.push(found.unwrap_or_else(|| comp.to_string()));
        }
        out.join("/")
    }
}

impl VaultFs for CaseInsensitiveFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        self.inner.list(&self.resolve(dir))
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        Ok(self.inner.stat(&self.resolve(path))?.map(|mut s| {
            s.path = path.to_string();
            s
        }))
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(&self.resolve(path))
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        let mut s = self.inner.write(&self.resolve(path), data)?;
        s.path = path.to_string();
        Ok(s)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(&self.resolve(path))
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        let (f, t) = (self.resolve(from), self.resolve(to));
        if f.to_lowercase() == t.to_lowercase() {
            return self.inner.rename(&f, to);
        }
        self.inner.rename(&f, &t)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(&self.resolve(path))
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(&self.resolve(path))
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
}

/// Android shared storage as Cairn sees it through SAF (SafPlugin.kt): it
/// ignores case like `CaseInsensitiveFs`, and refuses ("already exists") to
/// create a file, or move one, under a name that differs only in case from
/// another entry in the same folder, because it cannot hold both. It also
/// refuses a name with a character FAT forbids, with the reason SafFs gives
/// (the path, then SafPlugin's message).
pub struct SafLikeFs {
    root: PathBuf,
    inner: CaseInsensitiveFs,
}

impl SafLikeFs {
    pub fn new(root: &Path) -> SafLikeFs {
        SafLikeFs { root: root.to_path_buf(), inner: CaseInsensitiveFs::new(root) }
    }

    /// Whether `path`'s folder holds an entry, other than `skip`, whose name
    /// is `path`'s ignoring case but not exactly.
    fn twin(&self, path: &str, skip: Option<&str>) -> bool {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let real_dir = self.inner.resolve(dir);
        let skip = skip.map(|s| self.inner.resolve(s));
        let Ok(rd) = fs::read_dir(self.root.join(&real_dir)) else { return false };
        rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).any(|n| {
            let real = if real_dir.is_empty() { n.clone() } else { format!("{real_dir}/{n}") };
            n != name && n.to_lowercase() == name.to_lowercase() && skip.as_deref() != Some(real.as_str())
        })
    }
}

impl VaultFs for SafLikeFs {
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
        if self.twin(path, None) {
            return Err(cairn_core::CoreError::AlreadyExists(path.to_string()));
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.contains(['"', '*', ':', '<', '>', '?', '\\', '|']) {
            let real: String = name.chars().map(|c| if "\"*:<>?\\|".contains(c) { '_' } else { c }).collect();
            return Err(cairn_core::CoreError::Io(format!("{path}: name not allowed on this storage (it would become \"{real}\")")));
        }
        self.inner.write(path, data)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        if self.twin(to, Some(from)) {
            return Err(cairn_core::CoreError::AlreadyExists(to.to_string()));
        }
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
    fn refuses_case_twins(&self) -> bool {
        true
    }
}
