//! Reproduction for FINDING-060.
//!
//! A note named with 70 CJK characters that conflicts on device "phone" gets
//! a conflict name of 210 + 33 + 3 = 246 bytes, which is LEGAL; it can fail
//! only because `StdFs::write` adds `.` + `.cairn-tmp-<pid>` (FINDING-050).
//! So that scenario passes with a short enough temp name and does not guard
//! the conflict-name length.
//!
//! These tests isolate the second cause, `SyncEngine::conflict_path`,
//! which appends " (conflict YYYY-MM-DD HHMM <device>)" (28 bytes + device
//! name) to the stem and must bound the length: it shortens the stem.
//! The notes used here are short enough that the current temp-file scheme
//! writes them fine (checked in each test), but an unshortened conflict-copy
//! name would be over 255 bytes by itself.
//!
//! Run:
//!   cargo test -p cairn-sync --test adv_verify_ss_06

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

fn cjk(n: usize) -> String {
    "日本語のとても長いノートのタイトル".chars().cycle().take(n).collect()
}

fn run(stem_chars: usize, phone_name: &str) -> Result<(), String> {
    let srv = server();
    let name = format!("{}.md", cjk(stem_chars));
    let conflict_len = cjk(stem_chars).len() + " (conflict 2026-10-04 0146 )".len() + phone_name.len() + ".md".len();
    assert!(conflict_len > 255, "test setup: conflict name must exceed NAME_MAX by itself ({conflict_len})");
    // Current temp name: "." + name + ".cairn-tmp-" + pid digits.
    let tmp_len = 1 + name.len() + 11 + std::process::id().to_string().len();
    assert!(tmp_len <= 255, "test setup: the note itself must be writable with the current temp scheme ({tmp_len})");

    let mut a = Device::new(&srv, "laptop", &[(&name, "l1\nl2\nl3\n")]);
    a.sync();
    let mut b = Device::new(&srv, phone_name, &[]);
    b.sync();

    // Control: a non-conflicting edit is written on the phone through
    // StdFs::write, so the note's own name is not the problem.
    a.write(&name, "l1 laptop\nl2\nl3\n");
    a.sync();
    b.sync();
    assert_eq!(b.read(&name).as_deref(), Some("l1 laptop\nl2\nl3\n"));

    // Same-line conflict.
    a.write(&name, "l1 laptop\nl2\nl3 laptop\n");
    b.write(&name, "l1 laptop\nl2\nl3 phone\n");
    a.sync();
    let r1 = b.try_sync();
    b.write("other.md", "unrelated\n");
    let r2 = b.try_sync();
    a.sync();
    if r1.is_err() || r2.is_err() {
        return Err(format!(
            "conflict name {conflict_len} bytes; phone sync errors: {:?} / {:?}",
            r1.err().map(|e| e.to_string()),
            r2.err().map(|e| e.to_string())
        ));
    }
    if a.read("other.md").as_deref() != Some("unrelated\n") {
        return Err("other.md never reached the laptop".into());
    }
    // Other files keep syncing (FINDING-017); the conflict itself must
    // also be resolved, so the phone's version reaches the laptop.
    if a.read(&name).as_deref() != Some("l1 laptop\nl2\nl3 phone\n") {
        return Err(format!("conflict name {conflict_len} bytes; the phone's edit never reached the laptop"));
    }
    Ok(())
}

/// 74 CJK characters (note name 225 bytes, writable today), device "phone":
/// the unshortened conflict name would be 258 bytes.
#[test]
fn conflict_name_over_255_bytes_with_short_device_name() {
    run(74, "phone").unwrap();
}

/// The length from the module docs (70 CJK characters, note name 213 bytes)
/// with a realistic host-name device name (the app's default is the host name):
/// the unshortened conflict name alone would be 258 bytes.
#[test]
fn conflict_name_over_255_bytes_with_hostname_device_name() {
    run(70, "alice-thinkpad-x1").unwrap();
}

/// A file without a real extension but with an early dot and a long tail
/// ("Mr. " + 215 x 'a', 219 bytes, writable today). The text after the dot
/// is not an extension and must be shortened with the rest of the name:
/// kept whole, the stem would be cut to nothing and the copy
/// " (conflict ... phone). aaa…" (250 bytes) could not be written.
#[test]
fn conflict_name_with_a_long_text_after_the_last_dot() {
    let srv = server();
    let name = format!("Mr. {}", "a".repeat(215));
    let mut a = Device::new(&srv, "laptop", &[(&name, "base\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.write(&name, "laptop\n");
    b.write(&name, "phone\n");
    a.sync();
    let r = b.sync();
    assert!(r.skipped.is_empty(), "phone left out: {:?}", r.skipped);
    assert_eq!(r.conflicts.len(), 1);
    a.sync();
    assert_eq!(a.read(&name).as_deref(), Some("phone\n"), "the phone's edit never reached the laptop");
    for d in [&a, &b] {
        for p in d.paths() {
            assert!(p.len() <= 223 && cairn_core::path::validate_name(&p).is_ok(), "{} on {}: {} bytes", p, d.name, p.len());
        }
    }
}
