//! Reproduction for FINDING-064: after a note is deleted and a
//! different note is later renamed onto the same path, the sync state keeps
//! two entries for that path (the deleted file id and the live one).
//! If `SyncEngine::file_id_for` took the first match in BTreeMap order of
//! the random file ids, deleted entries included, about half the time
//! Version history would list the deleted note's revisions. Restoring from
//! that list would overwrite the current note, and the current text would
//! then no longer appear in the note's history (although the confirmation
//! says "The current text stays in the history").
//!
//!   cargo test -p cairn-sync --test adv_verify_ss_15 -- --ignored

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

fn setup(srv: &Server) -> Device {
    let mut a = Device::new(srv, "laptop", &[("n.md", "deleted note v1\n"), ("anchor.md", "untouched\n")]);
    a.sync();
    a.write("n.md", "deleted note v2\n");
    a.sync();
    a.rm("n.md");
    a.sync();
    a.write("other.md", "current note\n");
    a.sync();
    a.mv("other.md", "n.md");
    a.sync();
    a
}

fn texts(d: &Device, path: &str) -> Vec<String> {
    d.engine
        .history(path)
        .unwrap()
        .iter()
        .map(|h| String::from_utf8_lossy(&d.engine.revision_content(h.seq).unwrap().data).into_owned())
        .collect()
}

/// The same happens on a second device that only pulls the changes.
#[test]
fn ss15_history_on_other_device_is_the_current_notes() {
    let mut wrong = Vec::new();
    for attempt in 0..12 {
        let srv = server();
        let _a = setup(&srv);
        let mut b = Device::new(&srv, "phone", &[]);
        b.sync();
        assert_eq!(b.read("n.md").as_deref(), Some("current note\n"));
        let t = texts(&b, "n.md");
        if t.first().map(String::as_str) != Some("current note\n") {
            wrong.push(format!("attempt {attempt}: phone's history of n.md = {t:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Restore an older entry of the note's Version history (the UI enables
/// Restore for every entry except the newest), then sync. The text the
/// restore replaced stays in the note's history. With the deleted note's
/// list, the restore would write that note's text and the current text
/// would no longer be reachable from the note's history.
#[test]
fn ss15_restore_from_wrong_history_keeps_current_text_in_history() {
    for attempt in 0..12 {
        let srv = server();
        let mut a = setup(&srv);
        a.write("n.md", "current note, edited\n");
        a.sync();
        let h = a.engine.history("n.md").unwrap();
        let t = texts(&a, "n.md");
        assert_eq!(t.first().map(String::as_str), Some("current note, edited\n"), "attempt {attempt}: history of n.md = {t:?}");
        a.engine.restore("n.md", h[h.len() - 1].seq).unwrap();
        a.sync();
        assert_eq!(a.read("n.md").as_deref(), Some("current note\n"));
        let t = texts(&a, "n.md");
        assert!(
            t.iter().any(|s| s == "current note, edited\n"),
            "attempt {attempt}: the overwritten text is not in the history of n.md: {t:?}"
        );
    }
}
