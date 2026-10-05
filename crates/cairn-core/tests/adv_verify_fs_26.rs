//! Measurement for FINDING-047: `write_note` checks `base_hash`, then
//! writes + fsyncs a temp file, then renames it over the note. It checks
//! again right before the rename, but an external save that lands between
//! that last check and the rename is still overwritten while `write_note`
//! returns Ok (rename(2) cannot compare and swap).
//!
//! Unlike fs26 in adv_fs_ops.rs (external write keyed to the appearance of
//! Cairn's temp file), this one uses
//!   * a normal-sized note (4 KB),
//!   * a vault on the real disk (cargo's CARGO_TARGET_TMPDIR, not /tmp tmpfs),
//!   * an external writer whose timing is random, NOT keyed to Cairn's temp file.
//!
//! Cairn saves back to back (much faster than the 600 ms autosave debounce),
//! so the test measures what fraction of Cairn's save loop is vulnerable and
//! shows the race is hit without adversarial timing.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_26 -- --ignored --nocapture

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

fn disk_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("adv-verify-fs26-")
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

/// Atomic external save (temp + rename), like vim, VS Code (atomic mode), Syncthing.
fn external_save(root: &Path, body: &str) {
    let tmp = root.join(".ext-tmp");
    fs::write(&tmp, body).unwrap();
    fs::rename(&tmp, root.join("n.md")).unwrap();
}

#[test]
#[ignore = "FINDING-047 (residual): an external save in the microseconds between write_note's last check and its rename is still overwritten; saving back to back hits it for about 2% of external saves"]
fn fs26_random_timing_external_saves_on_real_disk() {
    let pad = "lorem ipsum dolor sit amet, consectetur adipiscing\n".repeat(80); // ~4 KB
    let d = disk_tempdir();
    let root = d.path().to_path_buf();
    fs::write(root.join("n.md"), format!("START\n{pad}")).unwrap();
    let v = open(&root);

    // 1. How long is one write_note on this disk (≈ the vulnerable window)?
    let mut base = v.read_note("n.md").unwrap().hash;
    let mut times = Vec::new();
    for i in 0..50 {
        let t = Instant::now();
        base = v.write_note("n.md", &format!("CAIRN warm{i} base=x\n{pad}"), Some(&base)).unwrap().hash;
        times.push(t.elapsed());
    }
    times.sort();
    println!(
        "write_note on disk, 4 KB note: median {:?}, p90 {:?}, max {:?}",
        times[25], times[45], times[49]
    );

    // 2. Race with an external writer at random times.
    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, root2, pad2) = (stop.clone(), root.clone(), pad.clone());
    let external = std::thread::spawn(move || {
        let mut rng = Rng(0x9e3779b97f4a7c15);
        let (mut ok, mut clobbered, mut examples) = (0u32, 0u32, Vec::new());
        for k in 0..400u32 {
            std::thread::sleep(Duration::from_micros(rng.next() % 20_000));
            let label = format!("EXT {k}");
            external_save(&root2, &format!("{label}\n{pad2}"));
            // Wait until Cairn replaces it, then see whether Cairn had seen it.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let cur = fs::read_to_string(root2.join("n.md")).unwrap_or_default();
                let l = first_line(&cur);
                if l != label && !l.is_empty() {
                    if l.ends_with(&format!("base={label}")) {
                        ok += 1;
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
                std::thread::yield_now();
            }
        }
        stop2.store(true, Ordering::Relaxed);
        (ok, clobbered, examples)
    });

    // "Cairn": save back to back; on conflict, load theirs and continue from it.
    let mut base_label = String::from("x");
    let (mut saves, mut conflicts) = (0u32, 0u32);
    let t0 = Instant::now();
    let mut j = 0u64;
    while !stop.load(Ordering::Relaxed) {
        j += 1;
        let body = format!("CAIRN {j} base={base_label}\n{pad}");
        match v.write_note("n.md", &body, Some(&base)) {
            Ok(r) => {
                base = r.hash;
                base_label = format!("CAIRN {j}");
                saves += 1;
            }
            Err(CoreError::Conflict(_)) => {
                let n = v.read_note("n.md").unwrap();
                base = n.hash;
                base_label = first_line(&n.content).to_string();
                conflicts += 1;
            }
            Err(e) => panic!("unexpected error {e:?}"),
        }
    }
    let elapsed = t0.elapsed();
    let (ok, clobbered, examples) = external.join().unwrap();
    println!(
        "Cairn: {saves} saves + {conflicts} conflicts in {elapsed:?} (mean loop {:?}); external saves: {ok} detected, {clobbered} silently overwritten; e.g. {examples:?}",
        elapsed / (saves + conflicts).max(1)
    );
    assert_eq!(clobbered, 0, "{clobbered} external saves were overwritten without a conflict: {examples:?}");
}
