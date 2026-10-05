// Property test: random vaults and link texts, resolved by the Rust index,
// written to a JSON fixture that app/src/lib/adv_links.test.ts replays
// against the TypeScript LinkIndex and Markdown renderer.
//
//   cargo test -p cairn-core --test adv_links_fixture
//       checks that the committed fixture matches what the current Rust code
//       produces (so it cannot go stale) and that Rust agrees with the PLAN
//       rules outside the known finding categories.
//   ADV_LINKS_REGEN=1 cargo test -p cairn-core --test adv_links_fixture
//       rewrites crates/cairn-core/tests/fixtures/adv_links_resolution.json
//   cargo test -p cairn-core --test adv_links_fixture -- --ignored
//       runs the FINDING tests (Rust vs PLAN per category).
//   cd app && npx vitest run src/lib/adv_links.test.ts
//       replays the fixture in TypeScript.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::{hash_bytes, Index};
use cairn_core::parse::{self, LinkKind};
use cairn_core::path as vpath;
use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

const SEED: u64 = 0x5eed_1ced_c0ffee;
const VAULTS: usize = 100;
const CASES_PER_VAULT: usize = 20;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pct(&mut self, p: usize) -> bool {
        self.below(100) < p
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

const FOLDERS: &[&str] = &[
    "", "", "", "a", "A", "a/b", "notes", "Notes", "notes/daily", "ééé", "abcd", "日本", "my folder", "x.y", "Ünï",
    "z", "😀", "ｱ", "q/sub", "r/sub",
];
const STEMS: &[&str] = &[
    "Note", "note", "NOTE", "Daily 2024-01-01", "Café", "café", "v1.2", "Σίσυφος", "straße", "İstanbul", "Ünïcode",
    "日本語", "x.md", "a b", "README", "Readme", "index", "Leaf", "Ǆemal", "ﬁle",
];
const ATTACH: &[&str] = &[
    "pic.png", "Pic.PNG", "doc.pdf", "Note.png", "audio.mp3", "Note", "archive.tar.gz", "Café.jpg", "x.md.png",
];
const JUNK: &[&str] = &[
    "Missing", ".hidden/Note", ".trash/Note", "a/../Note", "Note/", "Note.", "#", "nope/Note", "日本/Missing",
];

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn gen_files(r: &mut Rng) -> Vec<String> {
    let n = 4 + r.below(16);
    let mut set = BTreeSet::new();
    while set.len() < n {
        let folder = *r.pick(FOLDERS);
        let name = if r.pct(75) {
            let ext = match r.below(20) {
                0 => ".MD",
                1 | 2 => ".markdown",
                _ => ".md",
            };
            format!("{}{}", r.pick(STEMS), ext)
        } else {
            r.pick(ATTACH).to_string()
        };
        set.insert(nfc(&vpath::join(folder, &name)));
    }
    set.into_iter().collect()
}

fn dirs_of(files: &[String]) -> Vec<String> {
    let mut d = BTreeSet::new();
    for f in files {
        let mut p = vpath::parent(f);
        while !p.is_empty() {
            d.insert(p.to_string());
            p = vpath::parent(p);
        }
    }
    d.into_iter().collect()
}

fn build(files: &[String], dirs: &[String]) -> Index {
    let mut idx = Index::default();
    for d in dirs {
        idx.put_entry(FileStat { path: d.clone(), kind: EntryKind::Dir, size: 0, mtime: 0 }, None);
    }
    for f in files {
        let st = FileStat { path: f.clone(), kind: EntryKind::File, size: 0, mtime: 0 };
        if vpath::is_markdown(f) {
            idx.put_note(st, String::new(), hash_bytes(b""));
        } else {
            idx.put_entry(st, None);
        }
    }
    idx
}

fn strip_md_ext(p: &str) -> &str {
    if vpath::is_markdown(p) {
        &p[..p.rfind('.').unwrap()]
    } else {
        p
    }
}

/// Lowercase link text without a note extension: `.markdown` files are notes
/// like `.md` files, so link text drops either (FINDING-178).
fn strip_note_suffix(s: &str) -> &str {
    s.strip_suffix(".md").or_else(|| s.strip_suffix(".markdown")).unwrap_or(s)
}

/// Relative path from folder `from_dir` to `to` ("../x/y").
fn relative(from_dir: &str, to: &str) -> String {
    let a: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = to.split('/').collect();
    let mut i = 0;
    while i < a.len() && i + 1 < b.len() && a[i] == b[i] {
        i += 1;
    }
    let mut out: Vec<String> = Vec::new();
    for _ in i..a.len() {
        out.push("..".into());
    }
    for c in &b[i..] {
        out.push((*c).to_string());
    }
    out.join("/")
}

fn case_variant(r: &mut Rng, s: &str) -> String {
    match r.below(6) {
        0 => s.to_uppercase(),
        1 => s.to_lowercase(),
        2 => {
            let mut c = s.chars();
            match c.next() {
                Some(f) if f.is_lowercase() => f.to_uppercase().chain(c).collect(),
                Some(f) => f.to_lowercase().chain(c).collect(),
                None => String::new(),
            }
        }
        _ => s.to_string(),
    }
}

/// Generate the path part of a link to `f` as seen from `source`.
fn gen_target(r: &mut Rng, files: &[String], source: &str) -> String {
    if r.pct(12) {
        return r.pick(JUNK).to_string();
    }
    let f = r.pick(files).clone();
    let md = vpath::is_markdown(&f);
    let src_dir = vpath::parent(source);
    let mut t = match r.below(8) {
        0 | 1 => vpath::file_name(&f).to_string(),
        2 => f.clone(),
        3 => {
            // last folder + name
            let p = vpath::parent(&f);
            if p.is_empty() {
                vpath::file_name(&f).to_string()
            } else {
                format!("{}/{}", vpath::file_name(p), vpath::file_name(&f))
            }
        }
        4 | 5 => relative(src_dir, &f),
        6 => format!("./{}", relative(src_dir, &f)).replace("./../", "../"),
        _ => format!("/{f}"),
    };
    if md {
        // extension: none (most common), .md, or as on disk
        match r.below(4) {
            0 | 1 => t = strip_md_ext(&t).to_string(),
            2 => t = format!("{}.md", strip_md_ext(&t)),
            _ => {}
        }
    }
    if r.pct(25) {
        // vary the case of the name part only
        let (dir, name) = match t.rfind('/') {
            Some(i) => (t[..=i].to_string(), t[i + 1..].to_string()),
            None => (String::new(), t.clone()),
        };
        t = format!("{dir}{}", case_variant(r, &name));
    }
    if r.pct(8) {
        t = t.nfd().collect();
    }
    if r.pct(5) {
        t = t.replace('/', "\\");
    }
    t
}

fn gen_ws(r: &mut Rng, t: &str) -> String {
    match r.below(30) {
        0 | 1 => format!("  {t}  "),
        2 => format!("{t}\t"),
        3 => format!("\u{85}{t}"),
        4 => format!("{t}\u{feff}"),
        5 => format!("\u{a0}{t}"),
        _ => t.to_string(),
    }
}

fn gen_suffix(r: &mut Rng) -> &'static str {
    match r.below(12) {
        0 => "#Heading",
        1 => "#^blk1",
        2 => "|Alias text",
        3 => "#Sub heading|alias",
        4 => "\\|table alias",
        5 => " # spaced | spaced ",
        _ => "",
    }
}

fn md_url(r: &mut Rng, t: &str) -> String {
    let mut u = t.trim().to_string();
    if u.contains(' ') {
        if r.pct(60) {
            u = u.replace(' ', "%20");
        } else {
            u = format!("<{u}>");
        }
    }
    if r.pct(15) {
        u = format!("{u}#Some%20Heading");
    }
    u
}

fn opt(s: &Option<String>) -> Value {
    match s {
        Some(x) => json!(x),
        None => Value::Null,
    }
}

/// The link resolution rules as PLAN.md states them, written independently:
/// exact vault path (with or without .md or .markdown), then a path relative
/// to the linking note's folder, then a case-insensitive basename match; ties:
/// the linking note's folder, then the shortest path (in characters), then
/// alphabetical.
/// Paths are NFC (PLAN 2.2). A target with folders must match the end of the
/// path in the basename step (as in Obsidian and in the Rust code).
pub fn plan_resolve(files: &[String], target: &str, source: &str) -> Option<String> {
    let t: String = nfc(target.trim()).replace('\\', "/");
    let rooted = t.starts_with('/');
    let t = t.trim_start_matches('/').to_string();
    if t.is_empty() || t.ends_with('/') {
        return None; // a folder is not a link target
    }
    let lower = t.to_lowercase();
    let want = strip_note_suffix(&lower).to_string();
    let noext = |f: &String| strip_md_ext(f).to_lowercase();
    let best = |c: Vec<&String>, src_dir: &str| -> Option<String> {
        c.into_iter()
            .min_by(|a, b| {
                let ka = (vpath::parent(a) != src_dir, a.chars().count(), a.as_str());
                let kb = (vpath::parent(b) != src_dir, b.chars().count(), b.as_str());
                ka.cmp(&kb)
            })
            .cloned()
    };
    let src_dir = vpath::parent(source);
    // 1. exact vault path
    let exact: Vec<&String> = files.iter().filter(|f| noext(f) == want).collect();
    if !exact.is_empty() {
        return best(exact, "\u{0}");
    }
    // 2. relative to the source folder (with . and ..). A bare name is
    //    covered by the "linking note's folder" tie-break below.
    if !rooted && t.contains('/') {
        if let Some(rel) = vpath::resolve_relative(src_dir, &t) {
            let rl = rel.to_lowercase();
            let rw = strip_note_suffix(&rl).to_string();
            let c: Vec<&String> = files.iter().filter(|f| noext(f) == rw).collect();
            if !c.is_empty() {
                return best(c, "\u{0}");
            }
        }
    }
    // 3. basename (folder part, minus ./ and ../ segments, must match the end)
    let rest: Vec<&str> = lower.split('/').filter(|s| !s.is_empty() && *s != "." && *s != "..").collect();
    let rest = rest.join("/");
    if rest.is_empty() {
        return None;
    }
    let key = vpath::link_key_for_target(&rest);
    let rest_noext = strip_note_suffix(&rest).to_string();
    let c: Vec<&String> = files
        .iter()
        .filter(|f| vpath::link_key_for_file(f) == key)
        .filter(|f| !rest_noext.contains('/') || noext(f) == rest_noext || noext(f).ends_with(&format!("/{rest_noext}")))
        .collect();
    best(c, src_dir)
}

struct Generated {
    json: Value,
    /// (category, example) for every Rust vs PLAN disagreement
    plan_diffs: Vec<(String, String)>,
}

fn classify_plan_diff(files: &[String], target: &str, rust: &Option<String>, plan: &Option<String>) -> String {
    let t = target.trim();
    // NFD first: an NFD name never reaches the path rules (its link key
    // differs), so a target that also has dot segments fails for that reason.
    if nfc(t) != t {
        return "nfd-target".into();
    }
    if t.split(['/', '\\']).any(|s| s == "." || s == "..") {
        return "dot-segments".into();
    }
    if t.contains('\\') {
        return "backslash".into();
    }
    if let (Some(a), Some(b)) = (rust, plan) {
        if a.len().cmp(&b.len()) != a.chars().count().cmp(&b.chars().count()) {
            return "length-in-bytes".into();
        }
    }
    let _ = files;
    "other".into()
}

fn generate() -> Generated {
    let mut r = Rng(SEED);
    let mut vaults = Vec::new();
    let mut plan_diffs = Vec::new();
    for _ in 0..VAULTS {
        let files = gen_files(&mut r);
        let dirs = dirs_of(&files);
        let idx = build(&files, &dirs);
        let notes: Vec<&String> = files.iter().filter(|f| vpath::is_markdown(f)).collect();
        let mut cases = Vec::new();
        for _ in 0..CASES_PER_VAULT {
            let source = if !notes.is_empty() && r.pct(70) {
                (*r.pick(&notes)).clone()
            } else if r.pct(10) {
                String::new()
            } else {
                nfc(&vpath::join(r.pick(FOLDERS), "Source.md"))
            };
            let target = gen_target(&mut r, &files, &source);
            let kind = match r.below(20) {
                0..=8 => "wiki",
                9..=11 => "embed",
                12..=15 => "md",
                _ => "mdimage",
            };
            let case = match kind {
                "wiki" | "embed" => {
                    let inner = format!("{}{}", gen_ws(&mut r, &target), gen_suffix(&mut r));
                    let raw = format!("{}[[{inner}]]", if kind == "embed" { "!" } else { "" });
                    let p = parse::parse(&raw);
                    let (t, sub, alias) = parse::split_wikilink_inner(&inner);
                    let (rust, rust_link) = match p.links.first() {
                        Some(l) => (idx.resolve_link(l, &source), json!(l.target)),
                        None => (None, Value::Null),
                    };
                    let rust_direct = idx.resolve(&t, &source);
                    assert!(p.links.is_empty() || rust == rust_direct || t.is_empty(), "wiki resolve_link == resolve");
                    let plan = if t.is_empty() { None } else { plan_resolve(&files, &t, &source) };
                    if !t.is_empty() && plan != rust_direct {
                        plan_diffs.push((
                            classify_plan_diff(&files, &t, &rust_direct, &plan),
                            format!("files={files:?} source={source:?} target={t:?} rust={rust_direct:?} plan={plan:?}"),
                        ));
                    }
                    json!({
                        "kind": kind, "source": source, "raw": raw, "inner": inner,
                        "target": t, "subpath": opt(&sub), "alias": opt(&alias),
                        "parsedTarget": rust_link, "rust": opt(&rust_direct), "plan": opt(&plan),
                    })
                }
                _ => {
                    let url = md_url(&mut r, &target);
                    let raw = if kind == "md" { format!("[text]({url})") } else { format!("![alt]({url})") };
                    let p = parse::parse(&raw);
                    let l = p.links.iter().find(|l| l.kind == LinkKind::Markdown);
                    let rust = l.and_then(|l| idx.resolve_link(l, &source));
                    // What a click does: the UI percent-decodes the path part
                    // and calls the resolve_link command with the Markdown
                    // kind (Index::resolve_target).
                    let click_target = l.map(|l| l.target.clone());
                    let rust_click = click_target.as_ref().and_then(|t| idx.resolve_target(t, LinkKind::Markdown, &source));
                    json!({
                        "kind": kind, "source": source, "raw": raw, "url": url,
                        "parsedTarget": opt(&click_target),
                        "parsedSubpath": opt(&l.and_then(|l| l.subpath.clone())),
                        "rust": opt(&rust), "rustClick": opt(&rust_click),
                    })
                }
            };
            cases.push(case);
        }
        // linkText round trip: the TS autocomplete inserts linkText(path), the
        // first of these texts that [[text]] reads back as the same file, or
        // nothing (null) when none does; record what the core resolves it to
        // from another note.
        let mut roundtrip = Vec::new();
        for f in &files {
            let key = vpath::link_key_for_file(f);
            let same = files.iter().filter(|g| vpath::link_key_for_file(g) == key).count();
            let mut texts = Vec::new();
            if same <= 1 {
                texts.push(if vpath::is_markdown(f) { vpath::stem(f) } else { vpath::file_name(f) });
                texts.push(vpath::file_name(f));
            }
            texts.extend([strip_md_ext(f), f.as_str()]);
            let reads_back = |t: &&str| {
                !t.contains(['[', ']', '\n'])
                    && parse::split_wikilink_inner(t).0 == *t
                    && idx.resolve(t, "").as_deref() == Some(f.as_str())
            };
            let text = texts.into_iter().find(reads_back).map(str::to_string);
            for src in ["notes/daily/Source.md"] {
                let rust = text.as_ref().and_then(|t| idx.resolve(t, src));
                roundtrip.push(json!({ "path": f, "text": opt(&text), "source": src, "rust": opt(&rust) }));
            }
        }
        vaults.push(json!({ "files": files, "dirs": dirs, "cases": cases, "linkText": roundtrip }));
    }
    Generated { json: json!({ "seed": SEED, "generator": "crates/cairn-core/tests/adv_links_fixture.rs", "vaults": vaults }), plan_diffs }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adv_links_resolution.json")
}

#[test]
fn fixture_is_current() {
    let g = generate();
    // Compact, one vault per line, so diffs stay readable and the file small.
    let vaults: Vec<String> = g.json["vaults"].as_array().unwrap().iter().map(|v| serde_json::to_string(v).unwrap()).collect();
    let text = format!(
        "{{\"seed\":{},\"generator\":{},\"vaults\":[\n{}\n]}}\n",
        g.json["seed"],
        g.json["generator"],
        vaults.join(",\n")
    );
    let path = fixture_path();
    if std::env::var_os("ADV_LINKS_REGEN").is_some() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        eprintln!("wrote {}", path.display());
        return;
    }
    let old = std::fs::read_to_string(&path).unwrap();
    assert!(
        old == text,
        "{} is stale: the Rust resolver changed. Regenerate with ADV_LINKS_REGEN=1 cargo test -p cairn-core --test adv_links_fixture, then rerun the Vitest replay.",
        path.display()
    );
}

/// Every Rust vs PLAN disagreement must fall into a known finding category.
#[test]
fn rust_matches_plan_outside_known_categories() {
    let g = generate();
    let others: Vec<&(String, String)> = g.plan_diffs.iter().filter(|(c, _)| c == "other").collect();
    let mut counts = std::collections::BTreeMap::new();
    for (c, _) in &g.plan_diffs {
        *counts.entry(c.clone()).or_insert(0usize) += 1;
    }
    eprintln!("Rust vs PLAN disagreements by category: {counts:?}");
    assert!(others.is_empty(), "unclassified Rust vs PLAN disagreements:\n{}", others.iter().take(10).map(|(_, e)| e.as_str()).collect::<Vec<_>>().join("\n"));
}

fn plan_diffs_in(cat: &str) -> Vec<String> {
    generate().plan_diffs.into_iter().filter(|(c, _)| c == cat).map(|(_, e)| e).collect()
}

#[test]
fn finding_plan_dot_segments() {
    let d = plan_diffs_in("dot-segments");
    assert!(d.is_empty(), "{} random cases, e.g.\n{}", d.len(), d.iter().take(3).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn finding_plan_nfd_targets() {
    let d = plan_diffs_in("nfd-target");
    assert!(d.is_empty(), "{} random cases, e.g.\n{}", d.len(), d.iter().take(3).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn finding_plan_length_in_bytes() {
    let d = plan_diffs_in("length-in-bytes");
    assert!(d.is_empty(), "{} random cases, e.g.\n{}", d.len(), d.iter().take(3).cloned().collect::<Vec<_>>().join("\n"));
}

/// Markdown links: what a click opens (Vault::resolve_target on the decoded
/// path) versus what backlinks/outgoing/graph say (Index::resolve_link).
#[test]
fn finding_md_click_vs_index_random() {
    let g = generate();
    let mut diffs = Vec::new();
    for v in g.json["vaults"].as_array().unwrap() {
        for c in v["cases"].as_array().unwrap() {
            if c["kind"] == "md" && c["rust"] != c["rustClick"] {
                diffs.push(format!("source={} raw={} index={} click={}", c["source"], c["raw"], c["rust"], c["rustClick"]));
            }
        }
    }
    assert!(diffs.is_empty(), "{} random Markdown links open a different note than the index says, e.g.\n{}", diffs.len(), diffs.iter().take(4).cloned().collect::<Vec<_>>().join("\n"));
}

/// The autocomplete inserts LinkIndex.linkText(path); it must resolve back to
/// that path from anywhere. (A file no wikilink reaches has no text and is
/// not offered.)
#[test]
fn finding_link_text_roundtrip() {
    let g = generate();
    let mut bad = Vec::new();
    for v in g.json["vaults"].as_array().unwrap() {
        for c in v["linkText"].as_array().unwrap() {
            if !c["text"].is_null() && c["rust"] != c["path"] {
                bad.push(format!("files={} path={} text={} -> {}", v["files"], c["path"], c["text"], c["rust"]));
            }
        }
    }
    bad.dedup();
    assert!(bad.is_empty(), "{} cases, e.g.\n{}", bad.len(), bad.iter().take(4).cloned().collect::<Vec<_>>().join("\n"));
}

// ---------------------------------------------------------------------------
// Parser parity fixture: tricky snippets parsed by parse.rs, replayed against
// the TS side (markdown-it preview, Live Preview tag regex, extractSection,
// stripFrontmatter) by app/src/lib/adv_links.test.ts.
// ---------------------------------------------------------------------------

const PARSE_CORPUS: &[&str] = &[
    "#123 #tag/sub #日本語 #a-b. #_x #ünï #Ǆemal",
    "`#code` and\n\n```\n#fenced [[Fenced]]\n```\n",
    "http://x.com/#anchor and http://x.com #real",
    "## Heading #inheading",
    "(#paren) a,#comma a;#semi word#no **b**#no",
    "\\#escaped and \\[[Escaped]]",
    "\\\\[[Twice]] \\![[Bang]] !\\[[NotEmbed]]",
    "[[A]] `[[B]]` <span>[[C]]</span>",
    "<div>\n[[HtmlBlock]] #htmltag\n</div>\n\nafter",
    "---\ntags: [Alpha, beta/x]\n---\nbody #gamma",
    "---\ntags: one, two\n---\n",
    "---\ntags:\n  - listed\n---\n#body",
    "---\n: bad\n  - [\n---\n[[AfterBad]] #afterbad",
    "---\n---\nbody #t [[E]]",
    "---\ntitle: x\n---  \n#afterfm [[AfterFm]]",
    "[![img](a.png)](Note.md)",
    "[[A]](x.md)",
    "> quote [[Q]] #qtag",
    "- [ ] task [[T]] #ttag",
    "| a | [[X\\|y]] |\n|---|---|\n| #celltag | b |",
    "Text with [[Unclosed and [[Closed]]",
    "[[A|B|C]] [[A#B#C]] [[#Local]] [[ ]] [[|x]]",
    "#tag, #tag2. #tag3! #🎉",
    "$$\n[[Math]]\n$$",
    "%% [[Comment]] %%",
    "Setext #s\n===\n",
    "    [[IndentedCode]] #indented",
    "1. [[Ordered]]",
    "[[Ref]]\n\n[Ref]: http://x",
    "[see #inlink [[InLink]]](x.md)",
    "![[Embed]] ![[pic.png|200]] ![[Note#Sec]]",
    "line one\n#start-of-line\n  #indented-tag",
    "~~~\n[[Tilde]]\n~~~\n````\n```\n[[InsideQuad]]\n````\n",
    "* list\n\n      [[ListCode]]\n",
    "<!-- [[InComment]] -->",
    "[[a\nb]] [[\u{a0}Nbsp\u{a0}]]",
    "Inline `code [[x]]` then [[Y]]",
    "`unclosed [[Z]]",
    "\u{feff}---\ntags: [bom]\n---\n#afterbom",
    "\u{feff}# Bom heading #bomtag [[BomLink]]",
    "---\n---\nIntro [[Intro]] #introtag\n\n---\n\nRest",
    "---\r\nk: v\r\n...\t\r\n#afterdots [[AfterDots]]",
    "--- \nk: v\n---\n#notfm",
];

const HEADING_CORPUS: &[&str] = &[
    "# Plain\ntext\n",
    "## **Bold** title\nx\n",
    "## C#\nx\n",
    "### Closing ###\nx\n",
    "Setext\n===\nx\n",
    "# With [[Link|alias]]\nx\n",
    "# `code` head\nx\n",
    "#   Spaced   \nx\n",
    "```\n# not a heading\n```\n# Real\n",
    "---\n# yaml comment\nk: v\n---\n# After\n",
    "    # indented code\n# Ok\n",
    "> # quoted heading\n",
    "Line one\nline two\n===\nx\n",
    "- ## In a list\n\n## A &amp; B ![alt *em*](p.png) <b>html</b>\nx\n",
];

fn generate_parse_fixture() -> Value {
    let mut cases = Vec::new();
    for src in PARSE_CORPUS {
        let p = parse::parse(src);
        let fm_tags: Vec<String> = match parse::split_frontmatter(src) {
            Some((yaml, _)) => parse::parse(&format!("---\n{yaml}---\n")).tags,
            None => Vec::new(),
        };
        let body_tags: Vec<String> = p.tags.iter().filter(|t| !fm_tags.contains(t)).cloned().collect();
        let wiki: Vec<Value> = p
            .links
            .iter()
            .filter(|l| l.kind == LinkKind::Wiki)
            .map(|l| json!({ "target": l.target, "subpath": opt(&l.subpath), "embed": l.embed }))
            .collect();
        let md: Vec<Value> = p
            .links
            .iter()
            .filter(|l| l.kind == LinkKind::Markdown)
            .map(|l| json!({ "target": l.target, "embed": l.embed }))
            .collect();
        cases.push(json!({
            "src": src, "wiki": wiki, "md": md, "bodyTags": body_tags, "fmTags": fm_tags,
            "hasFrontmatter": parse::split_frontmatter(src).is_some(), "bodyStart": p.body_start,
            "bodyStartUtf16": src[..p.body_start].encode_utf16().count(),
        }));
    }
    let mut headings = Vec::new();
    for src in HEADING_CORPUS {
        let p = parse::parse(src);
        let h: Vec<Value> = p.headings.iter().map(|h| json!({ "level": h.level, "text": h.text, "line": h.line })).collect();
        headings.push(json!({ "src": src, "headings": h }));
    }
    json!({ "generator": "crates/cairn-core/tests/adv_links_fixture.rs", "cases": cases, "headings": headings })
}

#[test]
fn parse_fixture_is_current() {
    let text = serde_json::to_string_pretty(&generate_parse_fixture()).unwrap() + "\n";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adv_links_parse.json");
    if std::env::var_os("ADV_LINKS_REGEN").is_some() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    assert!(
        std::fs::read_to_string(&path).unwrap() == text,
        "{} is stale; regenerate with ADV_LINKS_REGEN=1 cargo test -p cairn-core --test adv_links_fixture",
        path.display()
    );
}
