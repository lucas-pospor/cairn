//! Sync at scale: a 5,000-note vault through the real server binary
//! (a separate process, so the memory numbers are the client's).
//!
//! Slow, so ignored by default:
//!   cargo test -p cairn-sync --test adv_sync_robust_scale -- --ignored --nocapture --test-threads=1
//! SR_SCALE_NOTES (default 5000) and SR_SCALE_CHANGES (default 1000) set the size.
//! Debug build numbers (cargo test); a release build is several times faster.

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

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

/// Reset and read the peak resident set size (Linux).
fn reset_peak() {
    let _ = fs::write("/proc/self/clear_refs", "5");
}

fn peak_mb() -> f64 {
    let s = fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

fn note(i: usize, rng: &mut Rng) -> String {
    let words = ["garden", "river", "stone", "harvest", "lantern", "meadow", "copper", "willow", "ember", "orchard"];
    let mut s = format!("# Note {i}\n\ntags: #t{}\n\n", i % 50);
    let n = 150 + rng.below(250) as usize;
    for k in 0..n {
        s.push_str(words[rng.below(words.len() as u64) as usize]);
        s.push(if k % 14 == 13 { '\n' } else { ' ' });
    }
    s.push_str(&format!("\n[[Note {}]]\n", (i * 7) % 5000));
    s
}

struct Meter {
    t: Arc<FaultTransport>,
}

impl Meter {
    fn attach(d: &mut Device) -> Meter {
        let t = Arc::new(FaultTransport::passthrough(http(&d.url)));
        d.set_transport(Box::new(SharedTransport(t.clone())));
        Meter { t }
    }
    fn take(&self) -> (usize, usize, usize, usize) {
        (
            self.t.calls.swap(0, Ordering::SeqCst),
            self.t.puts.swap(0, Ordering::SeqCst),
            self.t.blob_bytes_in.swap(0, Ordering::SeqCst),
            self.t.blob_bytes_out.swap(0, Ordering::SeqCst),
        )
    }
}

fn phase(label: &str, d: &mut Device, m: &Meter) -> (Duration, usize) {
    reset_peak();
    let t = Instant::now();
    let r = d.sync_ok();
    let took = t.elapsed();
    let (calls, puts, inb, outb) = m.take();
    let state = fs::metadata(d.state_file()).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "{label:<44} {:>8.2} s  pulled {:>5} pushed {:>5}  requests {:>5} (puts {:>5})  down {:>7.1} MB  up {:>7.1} MB  peak RSS {:>7.1} MB  state.json {:>6.0} KB",
        took.as_secs_f64(),
        r.pulled,
        r.pushed,
        calls,
        puts,
        inb as f64 / 1e6,
        outb as f64 / 1e6,
        peak_mb(),
        state as f64 / 1024.0
    );
    (took, inb)
}

#[test]
#[ignore = "slow: scale measurement (5,000 notes, about 2-4 minutes in a debug build)"]
fn slow_scale_5000_notes() {
    let n = env("SR_SCALE_NOTES", 5000);
    let changes = env("SR_SCALE_CHANGES", 1000);
    let data = tempfile::tempdir().unwrap();
    let (_srv, url) = spawn_server(data.path());
    let mut a = Device::new_url(&url, "laptop", &[]);
    let mut rng = Rng(42);
    let mut bytes = 0;
    for i in 0..n {
        let s = note(i, &mut rng);
        bytes += s.len();
        let p = a.root.join(format!("folder{}/Note {i}.md", i % 40));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }
    eprintln!("vault: {n} notes, {:.1} MB", bytes as f64 / 1e6);
    let m = Meter::attach(&mut a);
    phase("A: initial upload", &mut a, &m);
    let (_, redownload) = phase("A: second sync, nothing changed", &mut a, &m);
    phase("A: third sync, nothing changed", &mut a, &m);
    for i in 0..changes {
        let p = a.root.join(format!("folder{}/Note {}.md", (i * 13 % n) % 40, i * 13 % n));
        let mut s = fs::read_to_string(&p).unwrap();
        s.push_str(&format!("\nedit {i}\n"));
        fs::write(&p, s).unwrap();
    }
    phase(&format!("A: {changes} small edits"), &mut a, &m);
    phase("A: next sync, nothing changed", &mut a, &m);
    let mut b = Device::new_url(&url, "phone", &[]);
    let mb = Meter::attach(&mut b);
    phase("B: first sync of the whole vault", &mut b, &mb);
    phase("B: second sync, nothing changed", &mut b, &mb);
    assert_eq!(b.files().len(), n);
    eprintln!("re-downloaded by the uploader on its next sync: {:.1} MB (FINDING-126)", redownload as f64 / 1e6);
}

#[test]
fn uploader_does_not_download_its_own_uploads_again() {
    let data = tempfile::tempdir().unwrap();
    let (_srv, url) = spawn_server(data.path());
    let mut a = Device::new_url(&url, "laptop", &[]);
    let mut rng = Rng(7);
    let mut bytes = 0;
    for i in 0..300 {
        let s = note(i, &mut rng);
        bytes += s.len();
        a.write(&format!("n/{i}.md"), &s);
    }
    let m = Meter::attach(&mut a);
    a.sync_ok();
    m.take();
    a.sync_ok();
    let (_, _, down, _) = m.take();
    assert!(down < bytes / 10, "nothing changed, yet the next sync downloaded {down} bytes (the vault is {bytes} bytes)");
}

#[test]
#[ignore = "slow: memory measurement for one 40 MB attachment upload"]
fn slow_memory_for_a_40mb_upload() {
    let data = tempfile::tempdir().unwrap();
    let (_srv, url) = spawn_server(data.path());
    let mut a = Device::new_url(&url, "laptop", &[]);
    let mut r = Rng(5);
    let big: Vec<u8> = (0..40 << 20).map(|_| r.next() as u8).collect();
    a.write_bytes("video.mp4", &big);
    drop(big);
    let m = Meter::attach(&mut a);
    let (took, _) = phase("A: upload one 40 MB file", &mut a, &m);
    eprintln!("upload took {took:?}; peak RSS {:.0} MB for a 40 MB file", peak_mb());
}
