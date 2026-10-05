//! Two devices syncing through a real server over HTTP.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::KdfParams;
use cairn_sync::transport::HttpTransport;
use cairn_sync::SyncError;

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "test-token-0123456789";

struct Server {
    url: String,
    db: Arc<parking_lot::Mutex<()>>, // keeps type simple; the server owns the db
    _rt: tokio::runtime::Runtime,
    db_path: PathBuf,
    _dir: tempfile::TempDir,
}

fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body: 50 << 20 });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { axum_serve(listener, st).await });
    Server { url: format!("http://{addr}"), db: Arc::new(parking_lot::Mutex::new(())), _rt: rt, db_path, _dir: dir }
}

async fn axum_serve(listener: tokio::net::TcpListener, st: cairn_server::Shared) {
    cairn_server::serve(listener, st).await;
}

struct Device {
    root: PathBuf,
    vault: Arc<Vault>,
    engine: SyncEngine,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Device {
    fn new(srv: &Server, name: &str, files: &[(&str, &str)]) -> Device {
        Self::with_pass(srv, name, files, "correct horse battery").unwrap()
    }

    fn with_pass(srv: &Server, name: &str, files: &[(&str, &str)], pass: &str) -> Result<Device, SyncError> {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let abs = vd.path().join(p);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, c).unwrap();
        }
        let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: name.into() };
        let engine = SyncEngine::connect_with(
            vault.clone(),
            sd.path(),
            settings,
            pass,
            Box::new(HttpTransport::new(&srv.url, TOKEN)),
            FAST_KDF,
        )?;
        Ok(Device { root: vd.path().to_path_buf(), vault, engine, _dirs: (vd, sd) })
    }

    fn sync(&mut self) -> cairn_sync::engine::SyncReport {
        self.engine.sync().expect("sync failed")
    }

    fn write(&self, p: &str, c: &str) {
        let abs = self.root.join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        // make sure the mtime moves even on coarse file systems
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(abs, c).unwrap();
    }

    fn read(&self, p: &str) -> Option<String> {
        fs::read_to_string(self.root.join(p)).ok()
    }

    fn rm(&self, p: &str) {
        fs::remove_file(self.root.join(p)).unwrap();
    }

    fn mv(&self, a: &str, b: &str) {
        let to = self.root.join(b);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::rename(self.root.join(a), to).unwrap();
    }

    /// Rename in the app, which records the rename for the next sync. The
    /// vault is rescanned first, as the app's file watcher would have.
    fn app_mv(&self, a: &str, b: &str) {
        self.vault.rescan().unwrap();
        self.vault.ensure_folder(cairn_core::path::parent(b)).unwrap();
        cairn_sync::engine::rename(&self.vault, self._dirs.1.path(), a, b).unwrap();
    }

    /// Visible files with their contents (sorted), excluding .trash.
    fn files(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for e in fs::read_dir(dir).unwrap() {
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
            out.push((rel, String::from_utf8_lossy(&fs::read(&p).unwrap()).to_string()));
        }
    }
}

fn converge(a: &mut Device, b: &mut Device) {
    a.sync();
    b.sync();
    a.sync();
    b.sync();
    assert_eq!(a.files(), b.files(), "devices did not converge");
}

#[test]
fn initial_upload_and_download() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Welcome.md", "hello"), ("dir/sub/Note.md", "[[Welcome]]"), ("img/pic.png", "\u{89}PNG")]);
    let r = a.sync();
    assert_eq!(r.pushed, 3);
    let mut b = Device::new(&srv, "phone", &[]);
    let r = b.sync();
    assert_eq!(r.pulled, 3);
    assert_eq!(b.read("dir/sub/Note.md").as_deref(), Some("[[Welcome]]"));
    assert_eq!(a.files(), b.files());
    // the index on B knows the new notes
    assert_eq!(b.vault.backlinks("Welcome.md")[0].source, "dir/sub/Note.md");
    // nothing more to do
    let r = a.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
}

#[test]
fn server_never_sees_plaintext() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Secret plans.md", "the eagle lands at midnight")]);
    a.sync();
    drop(srv.db.lock());
    let raw = fs::read(&srv.db_path).unwrap();
    let wal = fs::read(srv.db_path.with_extension("sqlite-wal")).unwrap_or_default();
    for hay in [&raw, &wal] {
        assert!(!hay.windows(5).any(|w| w == b"eagle"), "content leaked");
        assert!(!hay.windows(7).any(|w| w == b"Secret "), "path leaked");
    }
}

#[test]
fn wrong_passphrase_is_rejected() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "x")]);
    a.sync();
    let err = Device::with_pass(&srv, "phone", &[], "not the passphrase").err().unwrap();
    assert!(matches!(err, SyncError::WrongPassphrase), "{err:?}");
}

#[test]
fn wrong_token_is_rejected() {
    let srv = server();
    let vd = tempfile::tempdir().unwrap();
    let sd = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
    let settings = SyncSettings { server: srv.url.clone(), token: "nope".into(), vault_id: "notes".into(), device: "x".into() };
    let err = SyncEngine::connect_with(vault, sd.path(), settings, "correct horse battery", Box::new(HttpTransport::new(&srv.url, "nope")), FAST_KDF)
        .err()
        .unwrap();
    assert!(matches!(err, SyncError::Unauthorized), "{err:?}");
}

#[test]
fn concurrent_edits_to_different_lines_merge() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "line 1\nline 2\nline 3\nline 4\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    // both offline, editing different lines
    a.write("n.md", "line 1 (laptop)\nline 2\nline 3\nline 4\n");
    b.write("n.md", "line 1\nline 2\nline 3\nline 4 (phone)\n");
    a.sync();
    let r = b.sync();
    assert!(r.conflicts.is_empty());
    a.sync();
    let want = "line 1 (laptop)\nline 2\nline 3\nline 4 (phone)\n";
    assert_eq!(a.read("n.md").as_deref(), Some(want));
    assert_eq!(b.read("n.md").as_deref(), Some(want));
}

#[test]
fn overlapping_edits_keep_both_versions() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "same line\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.write("n.md", "laptop wrote this\n");
    b.write("n.md", "phone wrote this\n");
    a.sync();
    let r = b.sync();
    assert_eq!(r.conflicts.len(), 1);
    converge(&mut a, &mut b);
    let files = a.files();
    assert_eq!(files.len(), 2, "{files:?}");
    let contents: Vec<&str> = files.iter().map(|f| f.1.as_str()).collect();
    assert!(contents.contains(&"laptop wrote this\n"));
    assert!(contents.contains(&"phone wrote this\n"));
    assert!(files.iter().any(|f| f.0.starts_with("n (conflict ") && f.0.contains("phone") || f.0 == "n.md"));
}

#[test]
fn edit_wins_over_delete_both_ways() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("x.md", "v1"), ("y.md", "v1")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    // x: A deletes, B edits.  y: A edits, B deletes.
    a.rm("x.md");
    b.write("x.md", "v2 from phone");
    a.write("y.md", "v2 from laptop");
    b.rm("y.md");
    a.sync();
    b.sync();
    converge(&mut a, &mut b);
    assert_eq!(a.read("x.md").as_deref(), Some("v2 from phone"));
    assert_eq!(a.read("y.md").as_deref(), Some("v2 from laptop"));
}

#[test]
fn deletes_propagate_to_the_trash() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("old/gone.md", "bye"), ("anchor.md", "untouched")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.rm("old/gone.md");
    a.sync();
    b.sync();
    assert_eq!(b.read("old/gone.md"), None);
    // kept in B's vault trash, and the emptied folder is gone
    assert_eq!(fs::read_to_string(b.root.join(".trash/gone.md")).unwrap(), "bye");
    assert!(!b.root.join("old").exists());
}

#[test]
fn rename_on_one_side_edit_on_the_other() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("draft.md", "a\nb\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.mv("draft.md", "projects/final.md");
    b.write("draft.md", "a\nb\nc from phone\n");
    a.sync();
    b.sync();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), vec![("projects/final.md".to_string(), "a\nb\nc from phone\n".to_string())]);
}

#[test]
fn edit_on_one_side_rename_on_the_other_reverse_order() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("draft.md", "a\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.write("draft.md", "a\nedited\n");
    b.mv("draft.md", "renamed.md");
    a.sync();
    b.sync();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), vec![("renamed.md".to_string(), "a\nedited\n".to_string())]);
}

#[test]
fn folder_rename_moves_files() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Inbox/a.md", "1"), ("Inbox/b.md", "2")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.mv("Inbox", "Archive/2026");
    a.sync();
    b.sync();
    assert_eq!(b.files(), vec![("Archive/2026/a.md".into(), "1".into()), ("Archive/2026/b.md".into(), "2".into())]);
    assert!(!b.root.join("Inbox").exists());
}

#[test]
fn same_path_created_on_both_devices() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop notes"), ("Same.md", "identical")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone notes"), ("Same.md", "identical")]);
    a.sync();
    let r = b.sync();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    converge(&mut a, &mut b);
    let files = a.files();
    assert_eq!(files.iter().filter(|f| f.1 == "identical").count(), 1);
    assert!(files.iter().any(|f| f.1 == "laptop notes"));
    assert!(files.iter().any(|f| f.1 == "phone notes"));
    assert_eq!(files.len(), 3);
}

#[test]
fn binary_conflicts_keep_both() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("pic.png", "v1")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.write("pic.png", "laptop image");
    b.write("pic.png", "phone image");
    a.sync();
    b.sync();
    converge(&mut a, &mut b);
    let files = a.files();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.1 == "laptop image"));
    assert!(files.iter().any(|f| f.1 == "phone image"));
    assert!(files.iter().all(|f| f.0.ends_with(".png")));
}

#[test]
fn many_offline_edits_then_sync() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[]);
    let mut b = Device::new(&srv, "phone", &[]);
    for i in 0..40 {
        a.write(&format!("a/{i}.md"), &format!("laptop {i}"));
        b.write(&format!("b/{i}.md"), &format!("phone {i}"));
    }
    a.write("a/0.md", "laptop 0 edited");
    b.rm("b/1.md");
    converge(&mut a, &mut b);
    assert_eq!(a.files().len(), 79);
    assert_eq!(b.read("a/0.md").as_deref(), Some("laptop 0 edited"));
}

#[test]
fn racing_uploads_are_retried_and_merged() {
    // B changes the file on the server between A's pull and A's push.
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "1\n2\n3\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    b.write("n.md", "1 phone\n2\n3\n");
    b.sync();
    // A has not pulled B's change; its push hits a stale parent, which the
    // engine must detect (409) and resolve by merging in another round.
    a.write("n.md", "1\n2\n3 laptop\n");
    let r = a.sync();
    assert!(r.rounds >= 1);
    b.sync();
    assert_eq!(a.read("n.md").as_deref(), Some("1 phone\n2\n3 laptop\n"));
    assert_eq!(b.read("n.md").as_deref(), Some("1 phone\n2\n3 laptop\n"));
}

#[test]
fn history_and_restore() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "first")]);
    a.sync();
    a.write("n.md", "second");
    a.sync();
    let h = a.engine.history("n.md").unwrap();
    assert_eq!(h.len(), 2);
    let oldest = h.last().unwrap().seq;
    a.engine.restore("n.md", oldest).unwrap();
    assert_eq!(a.read("n.md").as_deref(), Some("first"));
    a.sync();
    assert_eq!(a.engine.history("n.md").unwrap().len(), 3);
}

#[test]
fn history_of_a_note_renamed_since_the_last_sync() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Old name.md", "first")]);
    a.sync();
    a.mv("Old name.md", "New name.md");
    let h = a.engine.history("New name.md").expect("history before the rename is synced");
    assert_eq!(h.len(), 1);
    // Edited as well: the next sync pushes it as a new file, so it has no history yet.
    a.write("New name.md", "second");
    assert!(a.engine.history("New name.md").is_err());
}

#[test]
fn a_deletion_cannot_be_restored() {
    let srv = server();
    // Another note stays: a vault folder with no files at all is not synced
    // (FINDING-006).
    let mut a = Device::new(&srv, "laptop", &[("n.md", "first"), ("other.md", "stays")]);
    a.sync();
    a.rm("n.md");
    a.sync();
    a.write("n.md", "again");
    a.sync();
    let h = a.engine.history("n.md").unwrap();
    let deletion = h.iter().find(|e| e.deleted).expect("a deleted entry").seq;
    assert!(a.engine.restore("n.md", deletion).is_err());
    assert_eq!(a.read("n.md").as_deref(), Some("again"));
}

#[test]
fn state_survives_restart() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "x")]);
    a.sync();
    // reload engine from disk
    let dir = a._dirs.1.path().to_path_buf();
    let engine = SyncEngine::load(a.vault.clone(), &dir).unwrap().unwrap();
    a.engine = engine;
    let r = a.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
}

// ---------- a transport that lets another device act mid-sync ----------

use cairn_sync::protocol::*;
use cairn_sync::transport::{PutOutcome, Transport};

struct Hooked {
    inner: HttpTransport,
    before_first_put: parking_lot::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    conflicts_seen: Arc<std::sync::atomic::AtomicUsize>,
}

impl Transport for Hooked {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.inner.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.inner.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        self.inner.changes(v, s, l)
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        if let Some(hook) = self.before_first_put.lock().take() {
            hook();
        }
        let out = self.inner.put(v, f, r)?;
        if matches!(out, PutOutcome::Conflict(_)) {
            self.conflicts_seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(out)
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.inner.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.inner.revision(v, s)
    }
}

/// Asks for one record per changes page: with a pull batch of one byte,
/// each record is a batch of its own.
struct OnePerPage(HttpTransport);

impl Transport for OnePerPage {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.0.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.0.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, _: u32) -> Result<ChangesResponse, SyncError> {
        self.0.changes(v, s, 1)
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

impl Device {
    /// Pull every record in a batch of its own (see `SyncEngine::set_pull_batch`).
    fn in_small_batches(mut self, url: &str) -> Device {
        let settings = self.engine.settings().clone();
        let t = Box::new(OnePerPage(HttpTransport::new(url, TOKEN)));
        self.engine = SyncEngine::load_with(self.vault.clone(), self._dirs.1.path(), settings, t).unwrap();
        self.engine.set_pull_batch(1);
        self
    }
}

#[test]
fn upload_race_gets_409_and_is_merged_in_a_new_round() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "1\n2\n3\n")]);
    a.sync();
    let b = std::sync::Arc::new(parking_lot::Mutex::new(Device::new(&srv, "phone", &[])));
    b.lock().sync();
    a.write("n.md", "1\n2\n3 laptop\n");
    // Right before A uploads, B uploads its own edit of the same file.
    let b2 = b.clone();
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let settings = a.engine.settings().clone();
    let dir = a._dirs.1.path().to_path_buf();
    let hooked = Hooked {
        inner: HttpTransport::new(&srv.url, TOKEN),
        before_first_put: parking_lot::Mutex::new(Some(Box::new(move || {
            let mut b = b2.lock();
            b.write("n.md", "1 phone\n2\n3\n");
            b.sync();
        }))),
        conflicts_seen: seen.clone(),
    };
    a.engine = SyncEngine::load_with(a.vault.clone(), &dir, settings, Box::new(hooked)).unwrap();
    let r = a.sync();
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1, "expected exactly one 409");
    assert!(r.rounds >= 2);
    b.lock().sync();
    assert_eq!(a.read("n.md").as_deref(), Some("1 phone\n2\n3 laptop\n"));
    assert_eq!(b.lock().read("n.md").as_deref(), Some("1 phone\n2\n3 laptop\n"));
}

#[test]
fn server_rows_are_encrypted() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Diary/Secret.md", "the eagle lands at midnight")]);
    a.sync();
    let conn = rusqlite::Connection::open(&srv.db_path).unwrap();
    let blobs: Vec<Vec<u8>> = conn
        .prepare("SELECT blob FROM revisions")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blobs.len(), 1);
    assert!(blobs[0].len() > 40);
    assert!(!blobs[0].windows(5).any(|w| w == b"eagle" || w == b"Diary"));
}

/// Random edits, creates, deletes and renames (every other one in the app)
/// on two devices with random sync points. The devices must always
/// converge, every sync must succeed, and no edit may vanish: each line a
/// device wrote is, at the end, in some file or in a trash folder. Seeds:
/// CAIRN_FUZZ_SEEDS (default 8).
#[test]
fn randomized_two_device_convergence() {
    let seeds: u64 = std::env::var("CAIRN_FUZZ_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    for s in 0..seeds {
        fuzz_once(0x2545F4914F6CDD1D ^ (s.wrapping_mul(0x9E3779B97F4A7C15)), false);
    }
}

/// The same with every record pulled in a batch of its own, as the records
/// of a large vault come in batches of about 64 MB: applying the changes
/// batch by batch must not lose an edit either.
#[test]
fn randomized_two_device_convergence_in_small_batches() {
    let seeds: u64 = std::env::var("CAIRN_FUZZ_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    for s in 0..seeds {
        fuzz_once(0x2545F4914F6CDD1D ^ (s.wrapping_mul(0x9E3779B97F4A7C15)), true);
    }
}

fn all_text_including_trash(d: &Device) -> String {
    let mut out = String::new();
    for (_, c) in d.files() {
        out.push_str(&c);
    }
    let trash = d.root.join(".trash");
    if trash.exists() {
        let mut v = Vec::new();
        walk(&trash, &trash, &mut v);
        for (_, c) in v {
            out.push_str(&c);
        }
    }
    out
}

const ANCHOR: &str = "anchor.md";

fn fuzz_once(mut seed: u64, small_batches: bool) {
    let srv = server();
    let archive = tempfile::tempdir().unwrap();
    let mut archived = 0;
    // A note no step touches: a vault folder with no files at all does not
    // sync (FINDING-006), and deleting every note is not what this checks.
    let mut devs = [Device::new(&srv, "laptop", &[(ANCHOR, "untouched")]), Device::new(&srv, "phone", &[])];
    if small_batches {
        devs = devs.map(|d| d.in_small_batches(&srv.url));
    }
    devs[0].sync();
    devs[1].sync();
    let mut rnd = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let mut written: Vec<String> = Vec::new();
    for step in 0..200 {
        let d = rnd(2) as usize;
        let files: Vec<String> = devs[d].files().into_iter().map(|f| f.0).filter(|p| p != ANCHOR).collect();
        match rnd(10) {
            0..=2 => {
                let p = format!("f{}/n{}.md", rnd(3), rnd(12));
                if !devs[d].root.join(&p).exists() {
                    let line = format!("created {step} on {d}\n");
                    devs[d].write(&p, &line);
                    written.push(line);
                }
            }
            3..=5 if !files.is_empty() => {
                let p = &files[rnd(files.len() as u64) as usize];
                let old = devs[d].read(p).unwrap_or_default();
                let line = format!("edit {step} on {d}\n");
                devs[d].write(p, &format!("{old}{line}"));
                written.push(line);
            }
            6 if !files.is_empty() => {
                // A user delete: keep the content aside, like a trash would.
                let p = files[rnd(files.len() as u64) as usize].clone();
                archived += 1;
                fs::copy(devs[d].root.join(&p), archive.path().join(format!("{archived}.md"))).unwrap();
                devs[d].rm(&p);
            }
            7 if !files.is_empty() => {
                let p = files[rnd(files.len() as u64) as usize].clone();
                let to = format!("r{}/m{}.md", rnd(2), rnd(20));
                if !devs[d].root.join(&to).exists() {
                    // Every other one in the app, without drawing from `rnd`.
                    if step % 2 == 0 { devs[d].app_mv(&p, &to) } else { devs[d].mv(&p, &to) }
                }
            }
            _ => {
                devs[d].sync();
            }
        }
    }
    let [a, b] = &mut devs;
    converge(a, b);
    let mut everywhere = format!("{}{}", all_text_including_trash(a), all_text_including_trash(b));
    let mut v = Vec::new();
    walk(archive.path(), archive.path(), &mut v);
    for (_, c) in v {
        everywhere.push_str(&c);
    }
    let lost: Vec<&String> = written.iter().filter(|l| !everywhere.contains(l.as_str())).collect();
    assert!(lost.is_empty(), "lines lost by sync: {lost:?}");
}
