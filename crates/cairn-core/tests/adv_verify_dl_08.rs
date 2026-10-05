//! Measurement for FINDING-047: `Vault::write_note` checks `base_hash`
//! against the bytes on disk, then `StdFs::write_if` writes and fsyncs a
//! temp file and renames it over the note. An external save that lands
//! between the check and the rename would be overwritten while `write_note`
//! returns Ok. The write checks again right before the rename, so only a
//! window of a few syscalls remains.
//!
//! This test is about how often a real user would hit the race, not whether
//! it exists:
//!   1. It times `write_note` (which bounds the window) for note sizes from
//!      1 KB to 2 MB on the real disk (CARGO_TARGET_TMPDIR), not tmpfs.
//!   2. It runs a race where "Cairn" saves once every 50 ms. The app's
//!      autosave waits 600 ms after the last keystroke (`AUTOSAVE_MS` in
//!      app.svelte.ts), so real saves are at least 12x sparser. The external
//!      writer saves at random times that are not keyed to Cairn's temp file.
//!
//! The clobbered fraction at a 50 ms save period, scaled by 50/600, estimates
//! the chance that one external save is lost while the user is typing in the
//! same note. The test expects none of the 500 external saves to be lost.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_dl_08 -- --ignored --nocapture

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

fn disk_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("adv-verify-dl08-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

fn open(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

#[test]
#[ignore = "FINDING-047: slow measurement (about a minute); only the microseconds between the last check and the rename are open, 0 of 500 lost in testing"]
fn dl08_window_size_and_hit_rate_at_realistic_cadence() {
    // 1. Window size per note size on the real disk.
    for &(label, bytes) in &[("1 KB", 1usize << 10), ("16 KB", 16 << 10), ("256 KB", 256 << 10), ("2 MB", 2 << 20)] {
        let d = disk_tempdir();
        let pad = "x".repeat(bytes);
        fs::write(d.path().join("n.md"), &pad).unwrap();
        let v = open(d.path());
        let mut base = v.read_note("n.md").unwrap().hash;
        let mut t = Vec::new();
        for i in 0..30 {
            let s = Instant::now();
            base = v.write_note("n.md", &format!("{i}{pad}"), Some(&base)).unwrap().hash;
            t.push(s.elapsed());
        }
        t.sort();
        println!("write_note {label:>6}: median {:?}  p90 {:?}  max {:?}", t[15], t[27], t[29]);
    }

    // 2. Race at a 50 ms save period with randomly timed external saves.
    let period = Duration::from_millis(50);
    let pad = "lorem ipsum dolor sit amet, consectetur adipiscing\n".repeat(80); // ~4 KB
    let d = disk_tempdir();
    let root = d.path().to_path_buf();
    fs::write(root.join("n.md"), format!("START\n{pad}")).unwrap();
    let v = open(&root);

    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, root2, pad2) = (stop.clone(), root.clone(), pad.clone());
    let n_ext = 500u32;
    let external = std::thread::spawn(move || {
        let mut rng = Rng(0x2545f4914f6cdd1d);
        let (mut detected, mut clobbered, mut examples) = (0u32, 0u32, Vec::new());
        for k in 0..n_ext {
            std::thread::sleep(Duration::from_micros(rng.next() % 100_000));
            let label = format!("EXT {k}");
            // Atomic save (temp + rename), like vim, VS Code, Syncthing, Dropbox.
            let tmp = root2.join(".ext-tmp");
            fs::write(&tmp, format!("{label}\n{pad2}")).unwrap();
            fs::rename(&tmp, root2.join("n.md")).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let cur = fs::read_to_string(root2.join("n.md")).unwrap_or_default();
                let l = first_line(&cur).to_string();
                if l != label && !l.is_empty() {
                    if l.ends_with(&format!("base={label}")) {
                        detected += 1;
                    } else {
                        clobbered += 1;
                        if examples.len() < 3 {
                            examples.push(format!("{label} replaced by {l:?}"));
                        }
                    }
                    break;
                }
                if Instant::now() > deadline {
                    break;
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        stop2.store(true, Ordering::Relaxed);
        (detected, clobbered, examples)
    });

    let mut base = v.read_note("n.md").unwrap().hash;
    let mut base_label = String::from("START");
    let mut j = 0u64;
    let mut durations = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(period);
        j += 1;
        let body = format!("CAIRN {j} base={base_label}\n{pad}");
        let s = Instant::now();
        match v.write_note("n.md", &body, Some(&base)) {
            Ok(r) => {
                durations.push(s.elapsed());
                base = r.hash;
                base_label = format!("CAIRN {j}");
            }
            Err(CoreError::Conflict(_)) => {
                // Like "Load theirs" after the conflict banner.
                let n = v.read_note("n.md").unwrap();
                base = n.hash;
                base_label = first_line(&n.content).to_string();
            }
            Err(e) => panic!("unexpected error {e:?}"),
        }
    }
    let (detected, clobbered, examples) = external.join().unwrap();
    durations.sort();
    let mean = durations.iter().sum::<Duration>() / durations.len().max(1) as u32;
    let total = (detected + clobbered).max(1) as f64;
    let frac = clobbered as f64 / total;
    println!(
        "50 ms save period: {} successful saves (mean write_note {:?}); external saves: {detected} raised a conflict, {clobbered} silently overwritten ({:.1}%); e.g. {examples:?}",
        durations.len(),
        mean,
        frac * 100.0
    );
    println!(
        "scaled to one save per 600 ms (autosave minimum): about {:.2}% of external saves made while the user is typing in the same note are lost",
        frac * 100.0 * 50.0 / 600.0
    );
    assert_eq!(clobbered, 0, "{clobbered} external saves were overwritten without a conflict: {examples:?}");
}
