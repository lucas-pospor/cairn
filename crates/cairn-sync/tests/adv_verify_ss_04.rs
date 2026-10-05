//! Reproduction for FINDING-059: an edit saved while a sync is
//! running (after the round's scan, before the pulled remote delete is
//! applied) must not be moved to the trash. PLAN section 3: "Edit vs delete:
//! the edit wins and the file is restored."
//!
//! These tests use the exact save path of the app's editor autosave
//! (app.svelte.ts save() -> commands.rs write_note -> Vault::write_note with
//! the tab's base hash), not an external write.
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_ss_04 -- --include-ignored --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use cairn_core::Change;
use common::*;

fn pair(srv: &Server, files: &[(&str, &str)]) -> (Device, Device) {
    let mut a = Device::new(srv, "laptop", files);
    a.sync();
    let mut b = Device::new(srv, "phone", &[]);
    b.sync();
    (a, b)
}

/// Every revision the server holds for the file ids this device tracks.
fn server_history_text(d: &Device) -> String {
    let mut out = String::new();
    let paths: Vec<String> = d.engine.state().files.values().map(|t| t.path.clone()).collect();
    for p in paths {
        if let Ok(h) = d.engine.history(&p) {
            for e in h {
                if let Ok(rev) = d.engine.revision_content(e.seq) {
                    out.push_str(&String::from_utf8_lossy(&rev.data));
                }
            }
        }
    }
    out
}

/// Control: the same autosave made one moment earlier (before the sync's
/// scan) wins over the remote delete, as PLAN promises. The race below is
/// the save landing inside the sync's window instead.
#[test]
fn autosave_before_sync_beats_remote_delete() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("n.md", "v1\n"), ("anchor.md", "untouched\n")]);
    let tab = a.vault.read_note("n.md").unwrap(); // the open editor tab
    b.rm("n.md");
    b.sync();
    a.vault.write_note("n.md", "v1\nedit\n", Some(&tab.hash)).unwrap();
    a.sync();
    converge(&mut [&mut a, &mut b]);
    assert_eq!(a.read("n.md").as_deref(), Some("v1\nedit\n"));
    assert_eq!(b.read("n.md").as_deref(), Some("v1\nedit\n"));
}

/// The race, through the app's own autosave API: the save succeeds (the
/// base hash matches, so the editor marks the tab clean). If the sync then
/// deleted the note and reported `Deleted n.md`, the app would close the
/// clean tab without a prompt (app.svelte.ts handleChanges), so the user's
/// note would vanish on every device, with the edit only in the laptop's
/// trash and never on the server. The edit must win instead.
#[test]
fn app_autosave_during_sync_beats_remote_delete() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("n.md", "v1\n"), ("anchor.md", "untouched\n")]);
    let tab = a.vault.read_note("n.md").unwrap();
    b.rm("n.md");
    b.sync();

    let vault = a.vault.clone();
    let base = tab.hash.clone();
    let saved = std::sync::Arc::new(parking_lot::Mutex::new(None));
    let saved2 = saved.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        let r = vault.write_note("n.md", "v1\nautosaved during sync\n", Some(&base));
        *saved2.lock() = Some(r.is_ok());
    }));
    let rep = a.sync();
    let deleted_reported = rep.changes.iter().any(|c| matches!(c, Change::Deleted { path, .. } if path == "n.md"));
    converge(&mut [&mut a, &mut b]);

    let in_history = server_history_text(&a).contains("autosaved during sync");
    println!(
        "autosave accepted: {:?}; sync reported Deleted n.md: {deleted_reported}; laptop files {:?}; phone files {:?}; laptop trash {:?}; edit in server history: {in_history}",
        *saved.lock(),
        a.files(),
        b.files(),
        a.trash_text()
    );
    assert_eq!(*saved.lock(), Some(true), "the autosave itself must have succeeded for this scenario");
    assert_eq!(
        a.read("n.md").as_deref(),
        Some("v1\nautosaved during sync\n"),
        "edit should beat delete; laptop trash: {:?}",
        a.trash_text()
    );
    assert_eq!(b.read("n.md").as_deref(), Some("v1\nautosaved during sync\n"));
}
