//! Vault-relative paths.
//!
//! Every path inside Cairn is relative to the vault root, uses `/` as the
//! separator and is NFC-normalized, whatever the host OS uses. The empty
//! string is the vault root itself. This is the identity of a note.

use unicode_normalization::UnicodeNormalization;

use crate::error::{CoreError, Result};

/// Normalize a user- or OS-supplied relative path into canonical vault form.
///
/// Accepts `\` separators, leading `./` or `/`, and duplicate slashes.
/// Rejects `..` and `.` components so a path can never escape the vault,
/// and NUL, which no file system allows in a name (C APIs would cut the
/// name short there). Other control characters are legal on some systems
/// and stay readable; Cairn only refuses them in names it creates.
pub fn normalize(p: &str) -> Result<String> {
    let p: String = p.nfc().collect();
    if p.contains('\0') {
        return Err(CoreError::InvalidPath(p));
    }
    let mut parts: Vec<&str> = Vec::new();
    for comp in p.split(['/', '\\']) {
        match comp {
            "" => continue,
            "." | ".." => return Err(CoreError::InvalidPath(p.clone())),
            c => parts.push(c),
        }
    }
    Ok(parts.join("/"))
}

/// Resolve `rel` (which may contain `..` and `.`) against the folder `base`.
/// Returns `None` if the result would escape the vault.
pub fn resolve_relative(base: &str, rel: &str) -> Option<String> {
    let rel: String = rel.nfc().collect();
    let mut parts: Vec<&str> = if rel.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|s| !s.is_empty()).collect()
    };
    for comp in rel.split(['/', '\\']) {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    Some(parts.join("/"))
}

/// Parent folder of `p` ("" for top-level entries).
pub fn parent(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[..i],
        None => "",
    }
}

/// Last component of `p`.
pub fn file_name(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// File name without its last extension. `a/b.c.md` -> `b.c`.
pub fn stem(p: &str) -> &str {
    let name = file_name(p);
    match name.rfind('.') {
        Some(0) | None => name,
        Some(i) => &name[..i],
    }
}

/// Lowercased extension without the dot, if any.
pub fn extension(p: &str) -> Option<String> {
    let name = file_name(p);
    match name.rfind('.') {
        Some(0) | None => None,
        Some(i) => Some(name[i + 1..].to_ascii_lowercase()),
    }
}

pub fn is_markdown(p: &str) -> bool {
    matches!(extension(p).as_deref(), Some("md") | Some("markdown"))
}

/// Join a folder and a name. `join("", "a")` is `"a"`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// True if any component starts with `.` (config folders, trash, temp files).
/// Hidden entries are not shown, indexed or synced.
pub fn is_hidden(p: &str) -> bool {
    p.split('/').any(|c| c.starts_with('.'))
}

/// True if `p` is strictly inside folder `dir` (any depth).
pub fn is_inside(p: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return !p.is_empty();
    }
    p.len() > dir.len() && p.starts_with(dir) && p.as_bytes()[dir.len()] == b'/'
}

/// `p` itself or inside `dir`.
pub fn is_same_or_inside(p: &str, dir: &str) -> bool {
    p == dir || is_inside(p, dir)
}

/// Replace the `from` prefix of `p` with `to` (used when a folder moves).
pub fn rebase(p: &str, from: &str, to: &str) -> String {
    if p == from {
        to.to_string()
    } else {
        debug_assert!(is_inside(p, from));
        join(to, &p[from.len() + 1..])
    }
}

/// Characters Cairn refuses in names it creates. The first group is invalid
/// on Windows; `[ ] # ^ |` would make the note impossible to link to.
pub const FORBIDDEN_NAME_CHARS: &[char] =
    &['/', '\\', ':', '*', '?', '"', '<', '>', '|', '[', ']', '#', '^'];

/// Check a single new file or folder name chosen by the user: the checks
/// of [`validate_any_name`], and no device name (see [`is_reserved_name`]).
pub fn validate_name(name: &str) -> Result<()> {
    validate_any_name(name)?;
    if is_reserved_name(name) {
        return Err(CoreError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// Check a name for a file or folder that Cairn makes or moves: not empty,
/// no space at either end, no dot at the start or the end, and none of
/// [`FORBIDDEN_NAME_CHARS`] or control characters. Device names are not
/// checked: sync takes a rename to such a name from another device.
pub fn validate_any_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name {
        return Err(CoreError::InvalidName(name.to_string()));
    }
    if name.starts_with('.') || name.contains(FORBIDDEN_NAME_CHARS) {
        return Err(CoreError::InvalidName(name.to_string()));
    }
    if name.chars().any(|c| c.is_control()) || name.ends_with('.') {
        return Err(CoreError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// True if Windows keeps `name` for a device: CON, PRN, AUX, NUL, COM0 to
/// COM9 and LPT0 to LPT9 (also with ¹, ² or ³), CONIN$ and CONOUT$, in any
/// case and with any extension. Many Windows programs, File Explorer among
/// them, cannot open, rename or delete a file or folder with such a name.
pub fn is_reserved_name(name: &str) -> bool {
    // Windows ignores spaces, and only spaces, before the extension.
    let stem = name.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        s => {
            let mut rest = s.get(3..).unwrap_or_default().chars();
            (s.starts_with("COM") || s.starts_with("LPT"))
                && rest.next().is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c))
                && rest.next().is_none()
        }
    }
}

/// True if no file or folder on Windows can have `name`, not even one made
/// through a `\\?\` path: it has `< > : " | ? * \ /` or a control
/// character.
pub fn windows_never_has(name: &str) -> bool {
    name.contains(['<', '>', ':', '"', '|', '?', '*', '\\', '/']) || name.chars().any(|c| c < ' ')
}

/// True if Windows refuses `name` for a new file or folder: no name on
/// Windows has it (see [`windows_never_has`]), it ends in a dot or a space,
/// or it is a device name (see [`is_reserved_name`]).
pub fn windows_refuses(name: &str) -> bool {
    windows_never_has(name) || name.ends_with(['.', ' ']) || is_reserved_name(name)
}

/// `s` with every character [`validate_name`] refuses inside a name
/// (forbidden and control characters) replaced by `-`, NFC-normalized. For
/// text Cairn puts into names it makes up, such as the device name in a
/// conflict copy.
pub fn sanitize_name_part(s: &str) -> String {
    s.nfc()
        .map(|c| if FORBIDDEN_NAME_CHARS.contains(&c) || c.is_control() { '-' } else { c })
        .collect()
}

/// Longest file or folder name in bytes that every supported file system
/// accepts (ext4, APFS, Android storage; NTFS counts 255 UTF-16 units,
/// which is never fewer).
pub const NAME_MAX: usize = 255;

/// The longest prefix of `s` that is at most `max` bytes, cut on a char
/// boundary.
pub fn truncate(s: &str, max: usize) -> &str {
    let mut i = s.len().min(max);
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// Key used to match a link target against files by name.
/// Markdown files are keyed by stem (`[[Note]]` finds `Note.md`), other files
/// by full name (`![[pic.png]]`). Case-insensitive.
pub fn link_key_for_file(p: &str) -> String {
    if is_markdown(p) {
        stem(p).to_lowercase()
    } else {
        file_name(p).to_lowercase()
    }
}

/// Same key, computed from the text inside a link. Like a path, the text is
/// NFC (it may be NFD, pasted from macOS), and a note extension is dropped.
pub fn link_key_for_target(target: &str) -> String {
    let name = file_name(target.trim_end_matches('/'));
    let lower = name.nfc().collect::<String>().to_lowercase();
    strip_note_ext(&lower).to_string()
}

/// Lowercase link text without a trailing `.md` or `.markdown`.
pub fn strip_note_ext(lower: &str) -> &str {
    lower.strip_suffix(".md").or_else(|| lower.strip_suffix(".markdown")).unwrap_or(lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("a/b.md").unwrap(), "a/b.md");
        assert_eq!(normalize("/a//b\\c.md").unwrap(), "a/b/c.md");
        assert_eq!(normalize("").unwrap(), "");
        assert!(normalize("a/../b").is_err());
        assert!(normalize("./a").is_err());
        assert!(normalize("a\u{0}b.md").is_err());
        assert!(normalize("dir\u{0}/x.md").is_err());
        assert_eq!(normalize("tab\there.md").unwrap(), "tab\there.md");
        // NFD "é" becomes NFC
        assert_eq!(normalize("caf\u{65}\u{301}.md").unwrap(), "caf\u{e9}.md");
    }

    #[test]
    fn relative_resolution() {
        assert_eq!(resolve_relative("a/b", "../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(resolve_relative("a", "./x/y.md").as_deref(), Some("a/x/y.md"));
        assert_eq!(resolve_relative("a", "/top.md").as_deref(), Some("top.md"));
        assert_eq!(resolve_relative("", "../x"), None);
    }

    #[test]
    fn components() {
        assert_eq!(parent("a/b/c.md"), "a/b");
        assert_eq!(parent("c.md"), "");
        assert_eq!(file_name("a/b/c.md"), "c.md");
        assert_eq!(stem("a/b.c.md"), "b.c");
        assert_eq!(stem(".hidden"), ".hidden");
        assert_eq!(extension("x/Y.MD").as_deref(), Some("md"));
        assert!(is_markdown("x.Md"));
        assert!(!is_markdown("x.png"));
        assert!(is_hidden(".cairn/settings.json"));
        assert!(is_hidden("a/.git/x"));
        assert!(!is_hidden("a/b.md"));
        assert!(is_inside("a/b", "a"));
        assert!(!is_inside("ab", "a"));
        assert!(!is_inside("a", "a"));
        assert!(is_inside("x", ""));
        assert_eq!(rebase("a/b/c.md", "a/b", "z"), "z/c.md");
        assert_eq!(rebase("a/b", "a/b", "z"), "z");
    }

    #[test]
    fn names() {
        assert!(validate_name("Hello world").is_ok());
        assert!(validate_name("Ünïcödé 日本").is_ok());
        for bad in ["", " x", "x ", "a/b", "a:b", "[x]", "a#b", ".hidden", "end.", "con", "Nul.md", "aux.tar.gz", "COM1", "lpt9.md", "com²"] {
            assert!(validate_name(bad).is_err(), "{bad} should be rejected");
        }
        // Device names only as a whole stem, and only spaces before the dot.
        for ok in ["console.md", "com10.md", "lpt.md", "a nul.md", "nul-notes.md", "com0x", "con\u{3000}.md", "aux\u{a0}.md"] {
            assert!(validate_name(ok).is_ok(), "{ok} should be allowed");
        }
        // Sync takes a device name from another device.
        assert!(validate_any_name("aux.md").is_ok());
        assert_eq!(sanitize_name_part("Sam's Pixel? [old] #2\t"), "Sam's Pixel- -old- -2-");
        assert_eq!(sanitize_name_part("a/b\\c:d"), "a-b-c-d");
        assert_eq!(sanitize_name_part("cafe\u{301}"), "caf\u{e9}");
    }

    #[test]
    fn names_windows_refuses() {
        for bad in ["a:b.md", "D:", "q?.md", "a<b", "p|q", "st*r", "quo\"te", "tab\there", "end.", "end ", "nul.md", "CON", "aux .txt", "LPT³"] {
            assert!(windows_refuses(bad), "{bad:?} should be refused");
        }
        for ok in ["note.md", " lead.md", "a.b.c", "[x].md", "Über.md", "com10", "#tag"] {
            assert!(!windows_refuses(ok), "{ok:?} should be allowed");
        }
        // Names a file made through a \\?\ path can have.
        for there in ["end.", "end ", "nul.md", "CON"] {
            assert!(!windows_never_has(there), "{there:?}");
        }
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abc", 2), "ab");
        // "日" is 3 bytes: never cut inside it
        assert_eq!(truncate("日本", 5), "日");
        assert_eq!(truncate("日本", 2), "");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn link_keys() {
        assert_eq!(link_key_for_file("Folder/My Note.md"), "my note");
        assert_eq!(link_key_for_file("img/Pic.PNG"), "pic.png");
        assert_eq!(link_key_for_target("folder/My Note"), "my note");
        assert_eq!(link_key_for_target("My Note.md"), "my note");
        assert_eq!(link_key_for_target("Pic.png"), "pic.png");
        assert_eq!(link_key_for_target("Doc.Markdown"), "doc");
        assert_eq!(link_key_for_target("Cafe\u{301}"), link_key_for_file("Caf\u{e9}.md"));
    }
}
