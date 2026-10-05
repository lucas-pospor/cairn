//! Reproduction of FINDING-090 through sync: the engine
//! detects renames by content hash regardless of extension (engine.rs
//! classify), so a todo.txt -> todo.md rename on one device is pulled by
//! the other as Vault::rename, and that device's index never parses the
//! note (index.rs rename_tree carries the non-note entry over).
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_lk_20 -- --include-ignored --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

const TODO: &str = "buy zucchini\nsee [[Target]] #errand\n";

fn indexed(d: &Device, path: &str) -> (usize, usize, bool) {
    let hits = d.vault.search("zucchini", 10).iter().filter(|h| h.path == path).count();
    let bl = d.vault.backlinks("Target.md").iter().filter(|b| b.source == path).count();
    let tag = d.vault.tags().iter().any(|t| t.tag == "errand");
    (hits, bl, tag)
}

#[test]
fn pulled_rename_txt_to_md_is_indexed() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("todo.txt", TODO), ("Target.md", "")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    assert_eq!(b.read("todo.txt").as_deref(), Some(TODO));

    // Rename on the laptop in a file manager; the laptop indexes it fine.
    a.mv("todo.txt", "todo.md");
    a.sync();
    assert_eq!(indexed(&a, "todo.md"), (1, 1, true), "laptop (control)");

    b.sync();
    assert_eq!(b.read("todo.md").as_deref(), Some(TODO), "file arrived on phone");
    assert!(!b.exists("todo.txt"));
    let st = indexed(&b, "todo.md");
    eprintln!("phone (search, backlinks, tag) for todo.md: {st:?}");
    // A second sync round (rescan first) does not repair it either.
    b.sync();
    let st2 = indexed(&b, "todo.md");
    eprintln!("phone after another sync: {st2:?}");
    assert_eq!(st, (1, 1, true), "phone indexes the pulled note");
}
