// Reproduction for FINDING-179: NFD link text vs NFC note paths.
//
// Paths are NFC (path.rs normalize), so link text must be normalized too. If
// link_key_for_target / Index::resolve compared the raw (NFD) text with NFC
// keys, wikilinks would not resolve. Markdown links go through
// resolve_relative (which does NFC) so they would resolve in outgoing, but be
// filed in link_sources under the NFD key while backlinks() looks the NFC key
// up.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_04 -- --include-ignored --nocapture

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

const NFC: &str = "Caf\u{e9}.md";
const NFD_STEM: &str = "Cafe\u{301}";

fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

/// Control: NFC link text works.
#[test]
fn control_nfc_link_text() {
    let (_d, v) = vault(&[(NFC, "x"), ("src.md", "[[Caf\u{e9}]] [m](Caf\u{e9}.md)")]);
    let n: usize = v.backlinks(NFC).iter().map(|b| b.items.len()).sum();
    assert_eq!(n, 2);
}

/// Observation: what a click on the NFD wikilink does. It resolves to the NFC
/// note. Had resolve_link given None, the UI would call
/// create_note(target + ".md"), which normalizes to the existing NFC path and
/// fails with AlreadyExists: a toast, no data loss, but the link could not be
/// followed. create_note still refuses the NFD twin.
#[test]
fn observe_click_path() {
    let (_d, v) = vault(&[(NFC, "keep me"), ("src.md", &format!("[[{NFD_STEM}]]"))]);
    assert_eq!(v.resolve(NFD_STEM, "src.md").as_deref(), Some(NFC));
    let err = v.create_note(&format!("{NFD_STEM}.md"), "").unwrap_err();
    println!("create_note error: {err}");
    assert_eq!(fs::read_to_string(_d.path().join(NFC)).unwrap(), "keep me");
}

#[test]
fn finding_nfd_links() {
    let (_d, v) = vault(&[(NFC, "x"), ("src.md", &format!("[[{NFD_STEM}]] [m]({NFD_STEM}.md)"))]);
    let out: Vec<_> = v.outgoing("src.md").into_iter().map(|o| o.resolved).collect();
    let n: usize = v.backlinks(NFC).iter().map(|b| b.items.len()).sum();
    assert_eq!(out, vec![Some(NFC.to_string()), Some(NFC.to_string())], "outgoing");
    assert_eq!(n, 2, "backlinks");
}
