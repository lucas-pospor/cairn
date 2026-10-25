//! How much memory a phone on Android shared storage (as `SharedStorageFs`)
//! has in use at once while it pulls many changes that wait for the rest of
//! the pull, as they have one of its folders take another spelling (see
//! `follow_spelling` in engine.rs). They are kept with their records and
//! read again in batches of about the pull's batch size, and the changes
//! that wait as pending from an earlier sync are tried again in such batches
//! too, so a large notebook never has all of them in memory at once. A
//! counting allocator records the most memory in use. This file has one
//! test, so that no other test runs alongside it.
//!
//!   cargo test -p cairn-sync --test adv_sync_phone_memory -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;

/// The system allocator, counting the bytes in use, and the most in use
/// since `peak_during` started.
struct Counting;

static IN_USE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = IN_USE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            if new_size > layout.size() {
                grew(new_size - layout.size());
            } else {
                IN_USE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The result of `f`, and the most bytes in use while it ran, above those in
/// use before.
fn peak_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = IN_USE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let r = f();
    (r, PEAK.load(Ordering::SeqCst).saturating_sub(base))
}

/// Size and number of the notes: 16 MB in all, of which a few in memory at
/// once stand out. Text files, not Markdown, so that the notebook's index
/// does not keep their text, as it keeps that of notes.
const SIZE: usize = 512 << 10;
const NOTES: usize = 32;

/// The most memory a sync may have in use at once: a quarter of the notes,
/// several times what one batch of a record each takes.
const LIMIT: usize = SIZE * NOTES / 4;

#[test]
fn a_phone_on_shared_storage_reads_the_changes_that_wait_for_its_folder_a_batch_at_a_time() {
    let srv = server();
    let faults = Arc::new(StorageProbe::default());
    let storage = faults.clone();
    let mut phone = Device::with_fs(&srv, "phone", &[("DRAFTS/p.txt", "p\n")], move |root| SharedStorageFs::probed(root, storage));
    let texts: Vec<(String, String)> = (0..NOTES).map(|i| (format!("Drafts/n{i:02}.txt"), format!("note {i}\n{}\n", "x".repeat(SIZE)))).collect();
    let files: Vec<(&str, &str)> = texts.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut pc = Device::with_fs(&srv, "win-a", &files, CaseInsensitiveFs::new);
    drop(files);
    drop(texts);
    assert_eq!(phone.sync().pushed, 1);
    let r = pc.sync();
    assert_eq!((r.pulled, r.pushed), (1, NOTES));
    // Every record in a batch of its own, as a large notebook comes in many.
    phone.pull_one_by_one();
    // Each note would have the phone's folder take the PC's spelling. In the
    // first sync that fails, and they all wait as pending.
    faults.add("Drafts", RenameFault::Fail);
    let (first, most_first) = peak_during(|| phone.sync());
    // The next one tries them all again, and they come in.
    let (second, most_second) = peak_during(|| phone.sync());
    eprintln!("most memory in use: {most_first} bytes in the first sync, {most_second} in the second; the notes are {} bytes", SIZE * NOTES);
    assert!(most_first < LIMIT, "the first sync had {most_first} bytes in use at once, for {NOTES} notes of {SIZE} bytes");
    assert!(most_second < LIMIT, "the second sync had {most_second} bytes in use at once, for {NOTES} notes of {SIZE} bytes");
    assert_eq!((first.pulled, first.pushed, first.skipped.len()), (0, 0, NOTES), "{:?}", first.skipped);
    assert_eq!(faults.left(), 0);
    assert!(second.skipped.is_empty() && second.conflicts.is_empty(), "{second:?}");
    assert_eq!((second.pulled, second.pushed), (NOTES, 0));
    assert_eq!(phone.paths().len(), NOTES + 1);
    assert!(phone.paths().iter().all(|p| p.starts_with("Drafts/")), "{:?}", phone.paths());
}
