//! Regression test for FINDING-016 with the app's default layout:
//! attachments are saved to `attachments/`, which sorts before most note
//! folders, so one attachment the server refused (413) stopped edits, new notes
//! and deletions in later folders from ever being uploaded, and the error did
//! not say which file was the problem.
//!
//! Run: cargo test -p cairn-sync --test adv_verify_sr_12 -- --nocapture

#[path = "adv_sync_robust_common.rs"]
mod common;

use common::*;

fn bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    (0..n).map(|_| r.next() as u8).collect()
}

#[test]
fn oversized_attachment_in_default_folder_does_not_block_note_changes() {
    // CAIRN_MAX_BODY_MB=1 stands in for the 200 MB default (scaled down)
    let srv = server_with_body(1 << 20);
    let mut a = Device::new(
        &srv,
        "laptop",
        &[("daily/2026-10-02.md", "yesterday\n"), ("notes/plan.md", "v1\n"), ("projects/old.md", "obsolete\n")],
    );
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    assert_eq!(b.paths().len(), 3);

    // A pastes a video into a note (saved to attachments/) and keeps working.
    a.write_bytes("attachments/clip.mp4", &bytes(900 << 10, 5));
    a.write("daily/2026-10-03.md", "today ![[clip.mp4]]\n");
    a.write("notes/plan.md", "v2\n");
    a.rm("projects/old.md");

    let mut errors = Vec::new();
    let mut pushed = Vec::new();
    for _ in 0..3 {
        match a.sync() {
            Ok(r) => pushed.push(r.pushed),
            Err(e) => errors.push(e.to_string()),
        }
    }
    eprintln!("A's syncs: errors {errors:?}, successful pushes {pushed:?}");
    if let Some(e) = errors.first() {
        eprintln!("error names the file: {}", e.contains("clip.mp4"));
    }
    b.sync_ok();
    eprintln!("B now has {:?}, notes/plan.md = {:?}", b.paths(), b.read("notes/plan.md"));
    assert!(
        b.read("daily/2026-10-03.md").is_some() && b.read("notes/plan.md").as_deref() == Some("v2\n") && b.read("projects/old.md").is_none(),
        "after 3 syncs on A, B has {:?} (plan.md = {:?}); A's first error: {:?}",
        b.paths(),
        b.read("notes/plan.md"),
        errors.first()
    );
}
