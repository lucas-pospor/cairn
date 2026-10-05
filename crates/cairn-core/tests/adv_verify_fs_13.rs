//! Reproduction for FINDING-022 (read_note / read_file must not read hidden
//! files).
//!
//! `fs13_core_reads_hidden_files` checks the core side: both calls refuse
//! hidden paths. Reading them would only be exploitable through the plugin
//! bridge (FINDING-022).
//!
//! `fs13_note_content_cannot_steer_ui_to_hidden_files` passes. It shows that
//! the paths the trusted UI gets from note content (wikilink and embed
//! resolution through `Vault::resolve`) never point at hidden files, because
//! hidden files are not in the index. So a note cannot make the app's own UI
//! call read_note or read_text_file on `.git/config` or `.cairn/...`.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_13
//!   cargo test -p cairn-core --test adv_verify_fs_13 -- --ignored

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

const FILES: &[(&str, &str)] = &[
    (".git/config", "[remote \"origin\"]\nurl = https://user:ghp_secret@example.com/x"),
    (".cairn/settings.json", "{\"plugins\":[]}"),
    (".cairn/secret.md", "hidden markdown"),
    (".trash/old.md", "trashed"),
    ("n.md", "![[.git/config]] ![[config]] ![[.cairn/secret]] ![[secret]] ![[settings.json]] ![[old]]"),
];

#[test]
fn fs13_core_reads_hidden_files() {
    let (_d, v) = setup(FILES);
    let a = v.read_note(".git/config").map(|n| n.content);
    let b = v.read_file(".cairn/settings.json").map(|b| String::from_utf8_lossy(&b).into_owned());
    assert!(a.is_err() && b.is_err(), "read_note(.git/config) = {a:?}; read_file(.cairn/settings.json) = {b:?}");
}

#[test]
fn fs13_note_content_cannot_steer_ui_to_hidden_files() {
    let (_d, v) = setup(FILES);
    for t in [".git/config", "config", ".cairn/secret", ".cairn/secret.md", "secret", "settings.json", ".cairn/settings.json", "old", ".trash/old", ".trash/old.md"] {
        assert_eq!(v.resolve(t, "n.md"), None, "embed/wikilink target {t:?} resolved to a hidden file");
    }
    assert!(v.entries().iter().all(|e| !cairn_core::path::is_hidden(&e.path)), "hidden entries in the index");
}
