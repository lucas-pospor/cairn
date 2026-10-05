//! Reproduction for FINDING-133.
//!
//! `parse::parse` must not be quadratic in (tag occurrences x links), as it
//! would be with a linear scan of `out.links` for every TAG_RE match.
//!
//! This test calls `cairn_core::parse::parse` directly (no vault, no locks, no
//! search index) on three kinds of note, each at n and 8n lines:
//!   - links and tags on every line (the case from the finding)
//!   - links only
//!   - tags only
//! A links x tags term would make only the first case grow faster than
//! linearly.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_11 -- --include-ignored --nocapture

use std::time::{Duration, Instant};

use cairn_core::parse;

fn note(n: usize, link: bool, tag: bool) -> String {
    let mut s = String::new();
    for i in 0..n {
        let l = if link { format!("[[note{}]]", i % 100) } else { format!("note{}", i % 100) };
        let t = if tag { format!("#tag{}", i % 10) } else { format!("tag{}", i % 10) };
        s.push_str(&format!("- item {i} {l} {t}\n"));
    }
    s
}

fn best_of(runs: usize, content: &str) -> Duration {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            let p = parse::parse(content);
            std::hint::black_box(&p);
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn ratio(link: bool, tag: bool) -> (f64, Duration, Duration) {
    let small = note(2_000, link, tag);
    let big = note(16_000, link, tag);
    // warm up regex lazies
    let _ = parse::parse(&small);
    let ts = best_of(5, &small);
    let tb = best_of(3, &big);
    (tb.as_secs_f64() / ts.as_secs_f64(), ts, tb)
}

#[test]
fn control_links_only_and_tags_only_scale_linearly() {
    let (rl, sl, bl) = ratio(true, false);
    let (rt, st, bt) = ratio(false, true);
    println!("links only: 2k {sl:?}, 16k {bl:?}, ratio {rl:.1}");
    println!("tags only : 2k {st:?}, 16k {bt:?}, ratio {rt:.1}");
    assert!(rl < 16.0, "links-only ratio {rl:.1}");
    assert!(rt < 16.0, "tags-only ratio {rt:.1}");
}

#[test]
fn parse_is_quadratic_when_note_has_links_and_tags() {
    let (r, s, b) = ratio(true, true);
    println!("links+tags: 2k {s:?}, 16k {b:?}, ratio {r:.1} (linear would be about 8)");
    // Sanity: the result is still right (one tag per distinct value, links all found).
    let p = parse::parse(&note(16_000, true, true));
    assert_eq!(p.links.len(), 16_000);
    assert_eq!(p.tags.len(), 10);
    assert!(r < 16.0, "8x the content took {r:.1}x the time in parse() alone ({s:?} -> {b:?})");
}

/// Absolute cost per save of a log note (prints only). Meaningful with --release:
///   cargo test --release -p cairn-core --test adv_verify_fs_11 -- --ignored --nocapture print_parse_cost
#[test]
#[ignore = "slow: timing probe (prints parse() cost of log notes at 4k..64k lines)"]
fn print_parse_cost() {
    for n in [4_000usize, 16_000, 64_000] {
        let s = note(n, true, true);
        let d = best_of(2, &s);
        println!("links+tags n={n} ({} KB): parse {d:?}", s.len() / 1024);
    }
}
