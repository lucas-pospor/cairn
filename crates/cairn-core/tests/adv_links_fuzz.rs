// Parser fuzzing and pathological inputs.
//
//   cargo test -p cairn-core --test adv_links_fuzz
//   ADV_FUZZ_ROUNDS=20000 cargo test -p cairn-core --test adv_links_fuzz -- --nocapture

use std::sync::mpsc;
use std::time::{Duration, Instant};

use cairn_core::index::{hash_bytes, Index};
use cairn_core::parse::{self, LinkKind};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "[[", "]]", "[", "]", "(", ")", "![[", "![", "](", "|", "\\|", "#", "#tag", " #t/x", "^", "#^b", "`", "```\n", "~~~\n",
    "    ", "\n", "\n\n", "---\n", "...\n", "> ", "- ", "1. ", "* ", "<div>", "</div>", "<!--", "-->", "<", ">", "%20", "%",
    "é", "e\u{301}", "日本", "😀", "👍🏽", "\u{200d}", "\u{feff}", "\r\n", "\t", "a", "Note", "x.md", "pic.png", "http://x/#a",
    "mailto:a@b", "tags: [a, b]\n", "title: x\n", ":", "&a", "*a", "\"", "'", "$$", "%%", "|---|\n", "| a |", "\\", "\\[", "_",
    "**", "==", "~~", "[x]: http://y\n", "<http://z>", "[^1]", "[^1]: note\n",
];

fn random_doc(r: &mut Rng) -> String {
    let n = r.below(60);
    let mut s = String::new();
    for _ in 0..n {
        s.push_str(PIECES[r.below(PIECES.len())]);
    }
    s
}

fn check_invariants(src: &str) {
    let p = parse::parse(src);
    assert!(p.body_start <= src.len() && src.is_char_boundary(p.body_start));
    let line_count = src.split('\n').count() as u32;
    for l in &p.links {
        assert!(l.start < l.end && l.end <= src.len(), "range {l:?} in {src:?}");
        assert!(src.is_char_boundary(l.start) && src.is_char_boundary(l.end), "char boundary {l:?} in {src:?}");
        assert!(l.start >= p.body_start, "link inside frontmatter {l:?} in {src:?}");
        assert!(l.line < line_count);
        assert_eq!(l.line as usize, src[..l.start].matches('\n').count(), "line of {l:?} in {src:?}");
        let text = &src[l.start..l.end];
        match l.kind {
            LinkKind::Wiki => {
                assert!(text.starts_with(if l.embed { "![[" } else { "[[" }) && text.ends_with("]]"), "{text:?}");
                assert!(!l.target.contains(['[', ']', '\n', '|', '#']), "{l:?}");
            }
            LinkKind::Markdown => assert!(text.starts_with('[') || text.starts_with('!') || text.starts_with('<'), "{text:?} in {src:?}"),
        }
        let _ = parse::line_context(src, l.start);
    }
    for t in &p.tags {
        assert!(!t.is_empty() && !t.starts_with('#') && *t == t.to_lowercase(), "tag {t:?}");
    }
    for h in &p.headings {
        assert!((1..=6).contains(&h.level) && h.line < line_count);
    }
}

/// Run `f` on a thread and fail if it does not finish within `limit`.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> Result<(T, Duration), String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let t = Instant::now();
        let v = f();
        let _ = tx.send((v, t.elapsed()));
    });
    rx.recv_timeout(limit).map_err(|_| format!("did not finish within {limit:?}"))
}

#[test]
fn held_fuzz_parse_invariants() {
    let rounds: usize = std::env::var("ADV_FUZZ_ROUNDS").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let mut r = Rng(0x1234_5678_9abc_def1);
    for _ in 0..rounds {
        let doc = random_doc(&mut r);
        check_invariants(&doc);
        // index + search on the same text must not panic either
        let mut idx = Index::default();
        let st = cairn_core::FileStat { path: "f.md".into(), kind: cairn_core::EntryKind::File, size: 0, mtime: 0 };
        idx.put_note(st, doc.clone(), hash_bytes(doc.as_bytes()));
        let q = random_doc(&mut r);
        let _ = idx.search(&q, 10);
        let _ = idx.backlinks("f.md");
        let _ = idx.outgoing("f.md");
        let _ = idx.graph(true);
    }
}

#[test]
fn held_lossy_utf8_content_does_not_break_the_parser() {
    // Vault::open reads notes with from_utf8_lossy; invalid sequences become
    // U+FFFD, which must be safe everywhere (offsets, snippets).
    let bytes: Vec<u8> = b"[[A\xff\xfe]] #t\xc3 caf\xc3\xa9 \xe2\x82 [[B]]".to_vec();
    let s = String::from_utf8_lossy(&bytes).into_owned();
    check_invariants(&s);
    let mut idx = Index::default();
    let st = cairn_core::FileStat { path: "f.md".into(), kind: cairn_core::EntryKind::File, size: 0, mtime: 0 };
    idx.put_note(st, s.clone(), hash_bytes(&bytes));
    let _ = idx.search("caf", 10);
    let _ = idx.search("\u{fffd}", 10);
}

#[test]
fn held_pathological_inputs_finish_quickly() {
    let cases: Vec<(&str, String)> = vec![
        ("100k [", "[".repeat(100_000)),
        ("50k [[", "[[".repeat(50_000)),
        ("50k [a](", "[a](".repeat(50_000)),
        ("20k ![[x", "![[x".repeat(20_000)),
        ("10k nested quotes", format!("{} x [[A]]", ">".repeat(10_000))),
        ("3k nested lists", (0..3000).map(|i| format!("{}- [[L{i}]]\n", "  ".repeat(i))).collect()),
        ("50k emphasis", "*a".repeat(50_000)),
        ("50k backticks", "`a".repeat(50_000)),
        ("100k hashes", "#".repeat(100_000)),
        ("100k tags", " #t".repeat(100_000)),
        ("20k links in one line", "[[N]] ".repeat(20_000)),
        ("20k links and 20k tags", "[[N]] #t\n".repeat(20_000)),
        ("unclosed frontmatter 100k lines", format!("---\n{}", "k: v\n".repeat(100_000))),
        ("deep yaml nesting", format!("---\nk: {}{}\n---\n", "[".repeat(5000), "]".repeat(5000))),
    ];
    for (name, src) in cases {
        let res = within(Duration::from_secs(20), move || {
            let p = parse::parse(&src);
            p.links.len()
        });
        match res {
            Ok((_, el)) => eprintln!("{name}: {el:?}"),
            Err(e) => panic!("{name}: {e}"),
        }
    }
}

/// A "billion laughs" YAML alias bomb in a note's frontmatter. serde_yaml
/// expands aliases while building a Value; if it does not limit that, opening
/// a vault that contains such a note (from sync, a shared folder or git) hangs
/// or exhausts memory.
#[test]
fn held_yaml_alias_bomb_in_frontmatter_is_bounded() {
    let mut y = String::from("---\na: &a [\"lol\",\"lol\",\"lol\",\"lol\",\"lol\",\"lol\",\"lol\",\"lol\",\"lol\"]\n");
    let names = ["b", "c", "d", "e", "f", "g", "h", "i"];
    let mut prev = "a";
    for n in names {
        y.push_str(&format!("{n}: &{n} [*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev}]\n"));
        prev = n;
    }
    y.push_str("---\nbody [[Link]]\n");
    let res = within(Duration::from_secs(20), move || {
        let p = parse::parse(&y);
        (p.frontmatter.is_some(), p.links.len())
    });
    match res {
        Ok(((fm, links), el)) => {
            eprintln!("alias bomb parsed in {el:?}, frontmatter kept: {fm}");
            assert_eq!(links, 1);
        }
        Err(e) => panic!("alias bomb: {e}"),
    }
}
