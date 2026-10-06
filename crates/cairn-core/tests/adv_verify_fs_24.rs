//! Reproduction for FINDING-142: two threads writing the same config file
//! at once with Vault::write_config. With one temp name per (file name, PID)
//! they would share `.cairn/.settings.json.cairn-tmp-<pid>`.
//!
//! Besides spurious "not found" errors, this checks the stronger claim:
//! whether a write that *succeeds* can leave a mixed (corrupt)
//! settings.json. Each version is a JSON object whose length is encoded in
//! it, so a mixed file is detectable.
//!
//! In the app, settings.json is written only from a 300 ms debounce
//! (settings.svelte.ts update()) and snippets are awaited, so overlapping
//! calls are not expected in practice.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_24 -- --include-ignored --nocapture

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn version(t: u8, i: usize) -> String {
    let pad = "x".repeat(20_000 + (t as usize) * 7_000 + i * 37);
    format!("{{\"t\":{t},\"i\":{i},\"len\":{},\"pad\":\"{pad}\"}}", pad.len())
}

fn is_whole_version(s: &str) -> bool {
    // {"t":T,"i":I,"len":L,"pad":"xxx...x"}
    let Some(rest) = s.split("\"len\":").nth(1) else { return false };
    let Some(len) = rest.split(',').next().and_then(|n| n.parse::<usize>().ok()) else { return false };
    let Some(pad) = s.split("\"pad\":\"").nth(1) else { return false };
    pad.len() == len + 2 && pad.ends_with("\"}") && pad[..len].bytes().all(|b| b == b'x')
}

#[test]
fn fs24_concurrent_write_config_has_no_errors_or_corruption() {
    let d = tempfile::tempdir().unwrap();
    let v = Arc::new(Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let corrupt = Arc::new(AtomicUsize::new(0));
    let sample = Arc::new(parking_lot::Mutex::new(None::<String>));
    let reader = {
        let (stop, corrupt, sample, p) = (stop.clone(), corrupt.clone(), sample.clone(), d.path().join(".cairn/settings.json"));
        std::thread::spawn(move || {
            let mut reads = 0usize;
            while !stop.load(Ordering::Relaxed) {
                if let Ok(s) = fs::read_to_string(&p) {
                    reads += 1;
                    if !is_whole_version(&s) {
                        corrupt.fetch_add(1, Ordering::Relaxed);
                        sample.lock().get_or_insert_with(|| format!("len {} head {:?} tail {:?}", s.len(), &s[..s.len().min(40)], &s[s.len().saturating_sub(40)..]));
                    }
                }
            }
            reads
        })
    };
    let writers: Vec<_> = (0..4u8)
        .map(|t| {
            let v = v.clone();
            std::thread::spawn(move || {
                let mut errs = Vec::new();
                for i in 0..100 {
                    if let Err(e) = v.write_config("settings.json", &version(t, i)) {
                        errs.push(e.to_string());
                    }
                }
                errs
            })
        })
        .collect();
    let errs: Vec<String> = writers.into_iter().flat_map(|h| h.join().unwrap()).collect();
    stop.store(true, Ordering::Relaxed);
    let reads = reader.join().unwrap();
    let final_ok = is_whole_version(&fs::read_to_string(d.path().join(".cairn/settings.json")).unwrap());
    let leftovers: Vec<String> = fs::read_dir(d.path().join(".cairn")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains("cairn-tmp")).collect();
    println!("failed writes: {} of 400 (e.g. {:?})", errs.len(), errs.first());
    println!("reader: {reads} reads, {} corrupt (sample: {:?})", corrupt.load(Ordering::Relaxed), sample.lock());
    println!("final file whole: {final_ok}; temp leftovers: {leftovers:?}");
    assert!(errs.is_empty() && corrupt.load(Ordering::Relaxed) == 0 && final_ok, "see output above");
}
