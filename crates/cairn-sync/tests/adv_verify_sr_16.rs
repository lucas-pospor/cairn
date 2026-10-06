//! Regression test and timing breakdown for FINDING-126 (sync performance).
//!
//! 1. `uploader_does_not_redownload_its_pushes`: after a push, the next sync
//!    fetched every blob the device just uploaded (the changes feed carries
//!    the blobs inline and `last_seq` was set before the push), once per
//!    push. Now neither that sync nor the one after it fetches anything.
//! 2. `slow_initial_upload_breakdown`: splits the initial upload time into
//!    time inside `Transport::put` (HTTP + server) and time in the client,
//!    and times one `state.json` rewrite at the final size, to see which part
//!    grows with the vault.
//!
//! Run (2. is slow and ignored; `--include-ignored` runs it too):
//!   cargo test -p cairn-sync --test adv_verify_sr_16 -- --include-ignored --nocapture --test-threads=1

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_sync::engine::SyncState;
use cairn_sync::protocol::*;
use cairn_sync::transport::{PutOutcome, Transport};
use cairn_sync::SyncError;
use common::*;

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(data: &std::path::Path) -> (Proc, String) {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cairn-server");
    let child = Command::new(bin)
        .env("CAIRN_TOKENS", TOKEN)
        .env("CAIRN_DATA", data)
        .env("CAIRN_ADDR", format!("127.0.0.1:{port}"))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let t = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(t.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    (Proc(child), format!("http://127.0.0.1:{port}"))
}

/// Counts blob bytes received and time spent inside put().
#[derive(Default)]
struct Counters {
    down: AtomicUsize,
    heads: AtomicUsize,
    put_ns: AtomicU64,
    puts: AtomicUsize,
}

struct Metered {
    inner: Box<dyn Transport>,
    c: Arc<Counters>,
}

impl Transport for Metered {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.inner.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.inner.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        let r = self.inner.changes(v, s, l)?;
        self.c.down.fetch_add(r.heads.iter().map(|h| h.blob.len()).sum(), Ordering::SeqCst);
        self.c.heads.fetch_add(r.heads.len(), Ordering::SeqCst);
        Ok(r)
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        let t = Instant::now();
        let out = self.inner.put(v, f, r);
        self.c.put_ns.fetch_add(t.elapsed().as_nanos() as u64, Ordering::SeqCst);
        self.c.puts.fetch_add(1, Ordering::SeqCst);
        out
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.inner.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.inner.revision(v, s)
    }
}

fn meter(d: &mut Device) -> Arc<Counters> {
    let c = Arc::new(Counters::default());
    d.set_transport(Box::new(Metered { inner: http(&d.url), c: c.clone() }));
    c
}

fn take(c: &Counters) -> (usize, usize, f64, usize) {
    (
        c.down.swap(0, Ordering::SeqCst),
        c.heads.swap(0, Ordering::SeqCst),
        c.put_ns.swap(0, Ordering::SeqCst) as f64 / 1e9,
        c.puts.swap(0, Ordering::SeqCst),
    )
}

fn note(i: usize) -> String {
    let words = ["garden", "river", "stone", "harvest", "lantern", "meadow", "copper", "willow", "ember", "orchard"];
    let mut s = format!("# Note {i}\n\n");
    for k in 0..300 {
        s.push_str(words[(i * 31 + k * 7) % words.len()]);
        s.push(if k % 14 == 13 { '\n' } else { ' ' });
    }
    s
}

#[test]
fn uploader_does_not_redownload_its_pushes() {
    let data = tempfile::tempdir().unwrap();
    let (_srv, url) = spawn_server(data.path());
    let mut a = Device::new_url(&url, "laptop", &[]);
    let mut bytes = 0;
    for i in 0..200 {
        let s = note(i);
        bytes += s.len();
        let p = a.root.join(format!("n/{i}.md"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }
    let c = meter(&mut a);
    let r = a.sync_ok();
    let (d1, h1, _, _) = take(&c);
    eprintln!("initial upload: pushed {} | fetched {h1} heads, {d1} blob bytes", r.pushed);
    let r = a.sync_ok();
    let (d2, h2, _, _) = take(&c);
    eprintln!("2nd sync (no change): pulled {} | fetched {h2} heads, {d2} blob bytes (vault {bytes} bytes)", r.pulled);
    let r = a.sync_ok();
    let (d3, h3, _, _) = take(&c);
    eprintln!("3rd sync (no change): pulled {} | fetched {h3} heads, {d3} blob bytes", r.pulled);
    for i in 0..20 {
        a.write(&format!("n/{i}.md"), &format!("{}\nedit\n", note(i)));
    }
    let r = a.sync_ok();
    let (d4, h4, _, _) = take(&c);
    eprintln!("sync after 20 edits: pushed {} | fetched {h4} heads, {d4} blob bytes", r.pushed);
    let r = a.sync_ok();
    let (d5, h5, _, _) = take(&c);
    eprintln!("next sync (no change): pulled {} | fetched {h5} heads, {d5} blob bytes", r.pulled);
    // The one-off nature: a further sync fetches nothing.
    assert_eq!(h3, 0, "third sync should fetch nothing");
    // The defect: own uploads come back once.
    assert_eq!(h2, 0, "2nd sync re-fetched {h2} of this device's own uploads ({d2} bytes; vault {bytes} bytes)");
    assert_eq!(h5, 0, "sync after the edit push re-fetched {h5} own uploads ({d5} bytes)");
}

fn breakdown(n: usize) {
    let data = tempfile::tempdir().unwrap();
    let (_srv, url) = spawn_server(data.path());
    let mut a = Device::new_url(&url, "laptop", &[]);
    for i in 0..n {
        let p = a.root.join(format!("f{}/Note {i}.md", i % 40));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, note(i)).unwrap();
    }
    let c = meter(&mut a);
    let t = Instant::now();
    let r = a.sync_ok();
    let total = t.elapsed().as_secs_f64();
    let (_, _, put_s, puts) = take(&c);
    assert_eq!(r.pushed, n);
    // One state.json rewrite at the final size, as save_state() does it.
    let state_path = a.state_dir.join("state.json");
    let st: SyncState = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let size = fs::metadata(&state_path).unwrap().len();
    let tmp = a.state_dir.join("bench.tmp");
    let reps = 20;
    let t = Instant::now();
    for _ in 0..reps {
        fs::write(&tmp, serde_json::to_vec_pretty(&st).unwrap()).unwrap();
        fs::rename(&tmp, a.state_dir.join("bench.json")).unwrap();
    }
    let one_save = t.elapsed().as_secs_f64() / reps as f64;
    // save_state runs once per put; on average the state is half its final size.
    let est_saves = one_save * n as f64 / 2.0;
    eprintln!(
        "n={n:>5}: total {total:>7.2} s | inside put (HTTP+server) {put_s:>6.2} s ({:.2} ms/put, {puts} puts) | client {:>6.2} s | state.json {:.0} KB, one save {:.2} ms, est. all saves {:.2} s",
        put_s * 1e3 / puts as f64,
        total - put_s,
        size as f64 / 1024.0,
        one_save * 1e3,
        est_saves
    );
}

#[test]
#[ignore = "slow: FINDING-126 timing breakdown of the initial upload (1,000 / 2,000 / 4,000 notes)"]
fn slow_initial_upload_breakdown() {
    for n in [1000, 2000, 4000] {
        breakdown(n);
    }
}
