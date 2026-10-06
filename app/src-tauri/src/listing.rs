//! What a folder listing shows, for a file system whose listing can hand
//! back hidden entries (`android::SafFs`).

use cairn_core::path as vpath;

/// True if a listing of folder `dir` ("" = the whole vault) shows the entry
/// at vault path `path`, as `StdFs` does: no name below `dir` starts with a
/// dot. `dir` itself may be hidden: `Vault::list_config` lists
/// `.cairn/snippets`. A path outside `dir` is shown only if no name in it
/// is hidden.
pub fn shown(dir: &str, path: &str) -> bool {
    let below = match dir {
        "" => Some(path),
        _ => path.strip_prefix(dir).and_then(|p| p.strip_prefix('/')),
    };
    !vpath::is_hidden(below.unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::{StdFs, TrashMode, VaultFs};
    use std::path::Path;

    #[test]
    fn hides_names_below_the_listed_folder_only() {
        assert!(shown("", "n.md"));
        assert!(shown("", "notes/a.md"));
        assert!(!shown("", ".cairn/settings.json"));
        assert!(!shown("", "notes/.git/x"));
        assert!(shown(".cairn/snippets", ".cairn/snippets/big.css"));
        assert!(shown(".cairn/snippets", ".cairn/snippets/sub/nested.css"));
        assert!(!shown(".cairn/snippets", ".cairn/snippets/.hidden.css"));
        assert!(!shown(".cairn/snippets", ".cairn/snippets/.sub/x.css"));
        assert!(shown("notes", "notes/a.md"));
        assert!(!shown("notes", "notes/.x.md"));
        // Not inside `dir`: judged as a whole.
        assert!(!shown(".cairn", ".cairnx/a.css"));
        assert!(!shown("notes", ".trash/a.md"));
        assert!(shown("notes", "other/a.md"));
    }

    /// Everything under `dir` on disk, hidden entries included, as vault paths.
    fn all_under(root: &Path, dir: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().into_string().unwrap();
            let path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            if e.file_type().unwrap().is_dir() {
                all_under(root, &path, out);
            }
            out.push(path);
        }
    }

    #[test]
    fn filtering_a_listing_with_hidden_entries_matches_std_fs() {
        let root = std::env::temp_dir().join(format!("cairn-listing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for f in [
            "n.md",
            "notes/a.md",
            "notes/.x.md",
            "notes/.hidden/h.md",
            ".cairn/settings.json",
            ".cairn/snippets/big.css",
            ".cairn/snippets/.hidden.css",
            ".cairn/snippets/sub/nested.css",
            ".cairn/snippets/.sub/x.css",
            ".cairn/plugins/hello.js",
            ".trash/old.md",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        let fs = StdFs::new(&root, TrashMode::Vault).unwrap();
        for dir in ["", "notes", ".cairn", ".cairn/snippets", ".cairn/plugins"] {
            let mut want: Vec<String> = fs.list(dir).unwrap().into_iter().map(|s| s.path).collect();
            want.sort();
            let mut got = Vec::new();
            all_under(&root, dir, &mut got);
            got.retain(|p| shown(dir, p));
            got.sort();
            assert_eq!(got, want, "listing {dir:?}");
        }
        // StdFs does list inside a hidden folder.
        let mut snippets: Vec<String> = fs.list(".cairn/snippets").unwrap().into_iter().map(|s| s.path).collect();
        snippets.sort();
        assert_eq!(snippets, [".cairn/snippets/big.css", ".cairn/snippets/sub", ".cairn/snippets/sub/nested.css"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
