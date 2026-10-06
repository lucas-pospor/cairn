//! The three-way merge the engine uses for notes (`diffy::merge`), checked
//! directly against the invariants a clean merge must keep. The engine
//! accepts any `Ok` from diffy as "clean" and writes it without a conflict
//! copy, so a wrong clean merge would lose or duplicate text silently.
//!
//! With unique lines, a correct clean merge of (base, ours, theirs):
//! * keeps every base line that neither side deleted, exactly once;
//! * drops every base line that a side deleted;
//! * contains every line either side inserted, exactly once (twice only if
//!   both sides inserted it, which we never do here);
//! * keeps the relative order of ours and of theirs.
//!
//!   cargo test -p cairn-sync --test adv_sync_semantics_merge
//!   CAIRN_SS_MERGE_CASES=200000 cargo test -p cairn-sync --test adv_sync_semantics_merge -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::collections::HashMap;

use common::{server, Device, Rng};

fn edit(rng: &mut Rng, base: &[String], tag: &str, n_edits: u64) -> Vec<String> {
    let mut v: Vec<String> = base.to_vec();
    let mut k = 0;
    for _ in 0..n_edits {
        match rng.below(3) {
            0 => {
                let pos = rng.below(v.len() as u64 + 1) as usize;
                v.insert(pos, format!("{tag}-ins-{k}"));
                k += 1;
            }
            1 if !v.is_empty() => {
                let pos = rng.below(v.len() as u64) as usize;
                v.remove(pos);
            }
            _ if !v.is_empty() => {
                let pos = rng.below(v.len() as u64) as usize;
                v[pos] = format!("{tag}-mod-{k}");
                k += 1;
            }
            _ => {}
        }
    }
    v
}

fn subsequence(needle: &[&str], hay: &[&str]) -> bool {
    let mut it = hay.iter();
    needle.iter().all(|n| it.any(|h| h == n))
}

fn check(base: &[String], ours: &[String], theirs: &[String], merged: &str) -> Result<(), String> {
    let m: Vec<&str> = merged.lines().collect();
    let mut count: HashMap<&str, usize> = HashMap::new();
    for l in &m {
        *count.entry(l).or_default() += 1;
    }
    let in_ours: std::collections::HashSet<&str> = ours.iter().map(|s| s.as_str()).collect();
    let in_theirs: std::collections::HashSet<&str> = theirs.iter().map(|s| s.as_str()).collect();
    for b in base {
        let kept = in_ours.contains(b.as_str()) && in_theirs.contains(b.as_str());
        let c = count.get(b.as_str()).copied().unwrap_or(0);
        if kept && c != 1 {
            return Err(format!("base line {b:?} kept by both sides appears {c} times"));
        }
        if !kept && c != 0 {
            return Err(format!("base line {b:?} deleted by a side appears {c} times"));
        }
    }
    for l in ours.iter().chain(theirs.iter()) {
        if l.contains("-ins-") || l.contains("-mod-") {
            let c = count.get(l.as_str()).copied().unwrap_or(0);
            if c != 1 {
                return Err(format!("inserted line {l:?} appears {c} times"));
            }
        }
    }
    // lines of each side that survive must keep that side's order
    let o: Vec<&str> = ours.iter().map(|s| s.as_str()).filter(|l| count.contains_key(l)).collect();
    let t: Vec<&str> = theirs.iter().map(|s| s.as_str()).filter(|l| count.contains_key(l)).collect();
    if !subsequence(&o, &m) || !subsequence(&t, &m) {
        return Err("order of a side not preserved".into());
    }
    if m.len() != count.len() {
        return Err("a line appears twice".into());
    }
    Ok(())
}

fn join(v: &[String]) -> String {
    v.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn diffy_clean_merges_never_lose_or_duplicate_lines() {
    let cases: u64 = std::env::var("CAIRN_SS_MERGE_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let mut rng = Rng::new(0xC0FFEE);
    let (mut clean, mut conflicts) = (0u64, 0u64);
    let mut bad: Vec<String> = Vec::new();
    for case in 0..cases {
        let n = 1 + rng.below(25) as usize;
        let base: Vec<String> = (0..n).map(|i| format!("base-{i}")).collect();
        let (ko, kt) = (1 + rng.below(4), 1 + rng.below(4));
        let ours = edit(&mut rng, &base, "o", ko);
        let theirs = edit(&mut rng, &base, "t", kt);
        match diffy::merge(&join(&base), &join(&ours), &join(&theirs)) {
            Ok(m) => {
                clean += 1;
                if let Err(e) = check(&base, &ours, &theirs, &m)
                    && bad.len() < 5
                {
                    bad.push(format!("case {case}: {e}\nbase {base:?}\nours {ours:?}\ntheirs {theirs:?}\nmerged {m:?}"));
                }
            }
            Err(_) => conflicts += 1,
        }
    }
    eprintln!("{cases} cases: {clean} clean, {conflicts} conflicts");
    assert!(bad.is_empty(), "wrong clean merges:\n{}", bad.join("\n\n"));
}

/// Real notes repeat lines (blank lines, `---`, `- [ ]`), which makes diffs
/// ambiguous. Unique lines must still be kept, dropped and inserted exactly
/// as in the unique-lines case; the repeated filler lines are not checked.
#[test]
fn diffy_clean_merges_with_repeated_lines_never_lose_or_duplicate_unique_lines() {
    let cases: u64 = std::env::var("CAIRN_SS_MERGE_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let fillers = ["", "---", "- [ ] todo", "", "  "];
    let mut rng = Rng::new(0xBADC0DE);
    let (mut clean, mut bad) = (0u64, Vec::new());
    for case in 0..cases {
        let n = 2 + rng.below(30) as usize;
        let base: Vec<String> =
            (0..n).map(|i| if rng.chance(45) { rng.pick(&fillers).to_string() } else { format!("base-{i}") }).collect();
        let (ko, kt) = (1 + rng.below(4), 1 + rng.below(4));
        let ours = edit(&mut rng, &base, "o", ko);
        let theirs = edit(&mut rng, &base, "t", kt);
        let Ok(m) = diffy::merge(&join(&base), &join(&ours), &join(&theirs)) else { continue };
        clean += 1;
        let ml: Vec<&str> = m.lines().collect();
        let cnt = |l: &str| ml.iter().filter(|x| **x == l).count();
        let mut err = None;
        for b in base.iter().filter(|b| b.starts_with("base-")) {
            let kept = ours.contains(b) && theirs.contains(b);
            let c = cnt(b);
            if (kept && c != 1) || (!kept && c != 0) {
                err = Some(format!("base line {b:?} kept={kept} appears {c} times"));
            }
        }
        for l in ours.iter().chain(theirs.iter()).filter(|l| l.contains("-ins-") || l.contains("-mod-")) {
            if cnt(l) != 1 {
                err = Some(format!("inserted line {l:?} appears {} times", cnt(l)));
            }
        }
        if let Some(e) = err
            && bad.len() < 5
        {
            bad.push(format!("case {case}: {e}\nbase {base:?}\nours {ours:?}\ntheirs {theirs:?}\nmerged {m:?}"));
        }
    }
    eprintln!("{cases} cases with repeated lines: {clean} clean");
    assert!(bad.is_empty(), "wrong clean merges:\n{}", bad.join("\n\n"));
}

#[test]
fn diffy_merge_of_edits_far_apart_is_clean() {
    // Edits separated by at least two unchanged lines must merge cleanly
    // (otherwise users get needless conflict copies).
    let mut rng = Rng::new(42);
    let mut needless = Vec::new();
    for case in 0..2000 {
        let n = 12 + rng.below(30) as usize;
        let base: Vec<String> = (0..n).map(|i| format!("base-{i}")).collect();
        let i = rng.below((n / 2 - 2) as u64) as usize;
        let j = n / 2 + 2 + rng.below((n - n / 2 - 2) as u64) as usize;
        let mut ours = base.clone();
        let mut theirs = base.clone();
        ours[i] = "ours".into();
        theirs[j] = "theirs".into();
        if diffy::merge(&join(&base), &join(&ours), &join(&theirs)).is_err() && needless.len() < 5 {
            needless.push(format!("case {case}: n={n} i={i} j={j}"));
        }
    }
    assert!(needless.is_empty(), "{needless:?}");
}

#[test]
fn diffy_merge_edge_cases() {
    // (base, ours, theirs, expected Some(clean) / None for conflict)
    let cases: &[(&str, &str, &str, Option<&str>)] = &[
        ("", "a\n", "", Some("a\n")),
        ("", "a\n", "b\n", None),
        ("x", "x\ny", "x", Some("x\ny")),
        // an editor adding the final newline + an edit of the line before it:
        // adjacent changes, so a conflict copy (conservative, not lossy)
        ("a\nb\nc", "a\nb\nc\n", "a\nB\nc", None),
        ("a\nb\nc", "a\nb\nc\n", "A\nb\nc", Some("A\nb\nc\n")),
        ("a\n", "", "a\n", Some("")),
        // deleting adjacent lines on each side is a conflict as well
        ("a\nb\n", "b\n", "a\n", None),
        ("a\r\nb\r\n", "a\r\nb\r\nc\r\n", "z\r\na\r\nb\r\n", Some("z\r\na\r\nb\r\nc\r\n")),
        ("l1\nl2\nl3\n", "l1\nl2\nl3\n", "l1\nl2\nl3\n", Some("l1\nl2\nl3\n")),
        ("a\nb\nc\nd\n", "a\nX\nc\nd\n", "a\nX\nc\nd\n", Some("a\nX\nc\nd\n")),
    ];
    let mut wrong = Vec::new();
    for (b, o, t, want) in cases {
        let got = diffy::merge(b, o, t).ok();
        if got.as_deref() != *want {
            wrong.push(format!("merge({b:?}, {o:?}, {t:?}) = {got:?}, want {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Merge time of a note rewritten on both sides (every line differs: e.g.
/// one device converted line endings, the other reformatted). Diffy's diff
/// is O(N*D), so merging it takes time that grows with the square of the
/// note's size: at 20,000 lines (300 KB) about a minute in a debug build,
/// while the sync thread (and "Sync now") waits. The engine does not merge
/// a note with that many edits on a side: the other version becomes a
/// conflict copy at once.
#[test]
fn merge_time_of_fully_rewritten_notes() {
    let n = 20_000;
    let base: String = (0..n).map(|i| format!("base line {i}\n")).collect();
    let ours: String = (0..n).map(|i| format!("base line {i}\r\n")).collect();
    let theirs: String = (0..n).map(|i| format!("theirs line {i}\n")).collect();
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("big.md", &base)]);
    laptop.sync();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync();
    phone.write("big.md", &theirs);
    phone.sync();
    laptop.write("big.md", &ours);
    let t = std::time::Instant::now();
    let r = laptop.sync();
    let el = t.elapsed();
    eprintln!("{n} lines ({} KB): {el:?}", base.len() / 1024);
    assert!(el.as_secs() < 10, "the sync took {el:?}");
    assert_eq!(r.conflicts.len(), 1, "{:?}", laptop.paths());
    assert_eq!(laptop.read("big.md").as_deref(), Some(ours.as_str()));
    assert_eq!(laptop.read(&r.conflicts[0]).as_deref(), Some(theirs.as_str()));
}

/// The app's merge_text command (the editor merging unsaved typing with an
/// edit made on disk meanwhile, FINDING-074) calls `cairn_sync::merge_text`
/// and the editor waits for it. It gives up on a side with too many edits,
/// as the engine does: a full merge of a 2 MB note replaced by one line
/// while the user typed takes about a minute in a debug build (3 s in a
/// release build), and the UI then shows the conflict banner anyway.
#[test]
fn editor_merge_of_a_huge_rewrite_returns_quickly() {
    let base = "lorem ipsum dolor sit amet\n".repeat(80_000);
    let ours = format!("MINE {base}");
    let t = std::time::Instant::now();
    let merged = cairn_sync::merge_text(&base, &ours, "EXTERNAL WRITE DURING SAVE\n");
    let el = t.elapsed();
    eprintln!("merge of a {} KB note replaced by one line: {el:?}", base.len() / 1024);
    assert!(el.as_secs() < 5, "the merge took {el:?}");
    assert_eq!(merged, None);
    // edits far apart in a note as large still merge
    let base: String = (0..80_000).map(|i| format!("line {i}\n")).collect();
    let ours = base.replacen("line 0\n", "MINE line 0\n", 1);
    let theirs = base.replacen("line 79999\n", "theirs\n", 1);
    assert_eq!(cairn_sync::merge_text(&base, &ours, &theirs), Some(ours.replacen("line 79999\n", "theirs\n", 1)));
}
