//! The in-memory index: every file in the vault, parsed notes, link
//! resolution, backlinks and the search index. It is a cache; it can always
//! be rebuilt from the files.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::fs::{EntryKind, FileStat};
use crate::parse::{self, Link, LinkKind, ParsedNote};
use crate::path as vpath;
use crate::search::{self, Query, SearchIndex, Snippet};

pub type Hash = [u8; 32];

pub fn hash_bytes(b: &[u8]) -> Hash {
    *blake3::hash(b).as_bytes()
}

pub fn hash_hex(h: &Hash) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// The CPU-heavy part of indexing a note (parsing and tokenizing), which can
/// run in parallel before the result is inserted.
pub struct PreparedNote {
    parsed: ParsedNote,
    content: String,
    doc: search::PreparedDoc,
}

impl PreparedNote {
    pub fn new(path: &str, content: String) -> PreparedNote {
        let parsed = parse::parse(&content);
        // Frontmatter is included so property values are searchable.
        let doc = search::PreparedDoc::new(vpath::stem(path), &content);
        PreparedNote { parsed, content, doc }
    }
}

pub struct NoteRecord {
    pub parsed: ParsedNote,
    pub content: String,
}

#[derive(Default)]
pub struct Index {
    pub(crate) entries: BTreeMap<String, FileStat>,
    /// Content hash of every Markdown note (and small attachments).
    pub(crate) hashes: HashMap<String, Hash>,
    pub(crate) notes: HashMap<String, NoteRecord>,
    /// link key -> files with that key (see `path::link_key_for_file`)
    by_key: HashMap<String, BTreeSet<String>>,
    /// link key of a target -> notes containing a link with that key
    link_sources: HashMap<String, HashSet<String>>,
    search: SearchIndex,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkItem {
    pub line: u32,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Backlinks {
    pub source: String,
    pub items: Vec<BacklinkItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub score: f32,
    pub snippets: Vec<Snippet>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingLink {
    pub target: String,
    pub resolved: Option<String>,
    pub line: u32,
    pub embed: bool,
    pub kind: LinkKind,
}

impl Index {
    pub fn entry(&self, path: &str) -> Option<&FileStat> {
        self.entries.get(path)
    }

    pub fn entries(&self) -> impl Iterator<Item = &FileStat> {
        self.entries.values()
    }

    pub fn note(&self, path: &str) -> Option<&NoteRecord> {
        self.notes.get(path)
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    /// Entries at `path` and below it.
    pub fn entries_under(&self, dir: &str) -> Vec<FileStat> {
        if dir.is_empty() {
            return self.entries.values().cloned().collect();
        }
        let mut out: Vec<FileStat> = Vec::new();
        if let Some(e) = self.entries.get(dir) {
            out.push(e.clone());
        }
        let prefix = format!("{dir}/");
        for (k, v) in self.entries.range(prefix.clone()..) {
            if !k.starts_with(&prefix) {
                break;
            }
            out.push(v.clone());
        }
        out
    }

    /// An entry next to `path`, other than `own`, whose name differs from it
    /// only in case (`Note.md` for `note.md`). Such pairs are one file on
    /// macOS, Windows and Android shared storage.
    pub fn case_twin(&self, path: &str, own: Option<&str>) -> Option<&str> {
        let parent = vpath::parent(path);
        let prefix = if parent.is_empty() { String::new() } else { format!("{parent}/") };
        let want = vpath::file_name(path).to_lowercase();
        self.entries
            .range(prefix.clone()..)
            .map(|(k, _)| k.as_str())
            .take_while(|k| k.starts_with(&prefix))
            .find(|k| {
                let name = &k[prefix.len()..];
                *k != path && Some(*k) != own && !name.contains('/') && name.to_lowercase() == want
            })
    }

    /// Insert or update a non-note entry (folder or attachment).
    pub fn put_entry(&mut self, st: FileStat, hash: Option<Hash>) {
        if !self.entries.contains_key(&st.path) && st.kind == EntryKind::File {
            self.by_key
                .entry(vpath::link_key_for_file(&st.path))
                .or_default()
                .insert(st.path.clone());
        }
        match hash {
            Some(h) => {
                self.hashes.insert(st.path.clone(), h);
            }
            None => {
                self.hashes.remove(&st.path);
            }
        }
        self.entries.insert(st.path.clone(), st);
    }

    /// Insert or replace a Markdown note.
    pub fn put_note(&mut self, st: FileStat, content: String, hash: Hash) {
        let prepared = PreparedNote::new(&st.path, content);
        self.put_prepared(st, prepared, hash);
    }

    /// Insert a note parsed beforehand (see [`PreparedNote`]).
    pub fn put_prepared(&mut self, st: FileStat, prepared: PreparedNote, hash: Hash) {
        let path = st.path.clone();
        self.unlink_note(&path);
        let PreparedNote { parsed, content, doc } = prepared;
        for l in &parsed.links {
            self.link_sources
                .entry(link_target_key(l, &path))
                .or_default()
                .insert(path.clone());
        }
        self.search.upsert_prepared(&path, vpath::stem(&path), doc);
        self.notes.insert(path.clone(), NoteRecord { parsed, content });
        self.put_entry(st, Some(hash));
    }

    fn unlink_note(&mut self, path: &str) {
        if let Some(old) = self.notes.remove(path) {
            for l in &old.parsed.links {
                let key = link_target_key(l, path);
                if let Some(set) = self.link_sources.get_mut(&key) {
                    set.remove(path);
                    if set.is_empty() {
                        self.link_sources.remove(&key);
                    }
                }
            }
            self.search.remove(path);
        }
    }

    /// Files whose path differs from `path` only in case. On a file system
    /// that ignores case they are the same file as `path`.
    pub fn case_variants(&self, path: &str) -> Vec<String> {
        let lower = path.to_lowercase();
        let Some(set) = self.by_key.get(&vpath::link_key_for_file(path)) else { return Vec::new() };
        set.iter().filter(|p| p.as_str() != path && p.to_lowercase() == lower).cloned().collect()
    }

    /// Remove one entry (not its children).
    pub fn remove(&mut self, path: &str) {
        self.unlink_note(path);
        self.hashes.remove(path);
        if let Some(e) = self.entries.remove(path) {
            if e.kind == EntryKind::File {
                let key = vpath::link_key_for_file(path);
                if let Some(set) = self.by_key.get_mut(&key) {
                    set.remove(path);
                    if set.is_empty() {
                        self.by_key.remove(&key);
                    }
                }
            }
        }
    }

    /// Remove an entry and everything below it.
    pub fn remove_tree(&mut self, path: &str) {
        for e in self.entries_under(path) {
            self.remove(&e.path);
        }
    }

    /// Move an entry (and its children if a folder) to a new path. A note
    /// moved to a name that is not Markdown (`Note.md` -> `Note.txt`) is a
    /// plain file there. The reverse needs the file read, which is up to
    /// the caller (see `Vault::rename`).
    pub fn rename_tree(&mut self, from: &str, to: &str, mtime: Option<i64>) {
        for mut e in self.entries_under(from) {
            let old = e.path.clone();
            let new = vpath::rebase(&old, from, to);
            let note = self.notes.get(&old).map(|n| n.content.clone());
            let hash = self.hashes.get(&old).copied();
            self.remove(&old);
            e.path = new;
            if old == from {
                if let Some(m) = mtime {
                    e.mtime = m;
                }
            }
            match (note, hash) {
                (Some(content), Some(h)) if vpath::is_markdown(&e.path) => self.put_note(e, content, h),
                (Some(_), _) => self.put_entry(e, None),
                (_, h) => self.put_entry(e, h),
            }
        }
    }

    /// Resolve a link found in `source` to a vault path.
    pub fn resolve_link(&self, link: &Link, source: &str) -> Option<String> {
        if link.target.is_empty() {
            return Some(source.to_string()); // [[#heading]] points at itself
        }
        self.resolve_target(&link.target, link.kind, source)
    }

    /// Resolve the path part of a link of `kind` in `source`. A Markdown link
    /// is first a path relative to the source folder; then, like a wikilink,
    /// it follows [`Index::resolve`]. Clicks use this, so they open what
    /// backlinks, outgoing links and the graph show.
    pub fn resolve_target(&self, target: &str, kind: LinkKind, source: &str) -> Option<String> {
        if kind == LinkKind::Markdown {
            let rel = vpath::resolve_relative(vpath::parent(source), target)?;
            for cand in [rel.clone(), format!("{rel}.md")] {
                if self.entries.get(&cand).is_some_and(|e| e.kind == EntryKind::File) {
                    return Some(cand);
                }
            }
        }
        self.resolve(target, source)
    }

    /// Resolve wikilink text (without `#` or `|` parts) relative to `source`.
    ///
    /// Order: exact vault path, path relative to the source folder (`./` and
    /// `../` allowed), then any file with the same name, preferring the source
    /// folder, then the shortest path, then alphabetical order.
    /// Case-insensitive.
    pub fn resolve(&self, target: &str, source: &str) -> Option<String> {
        // NFC, like paths: link text may be NFD (pasted from macOS).
        let t: String = target.trim().replace('\\', "/").trim_start_matches('/').nfc().collect();
        if t.is_empty() {
            return None;
        }
        if t.ends_with('/') {
            return None; // a folder is not a link target
        }
        let key = vpath::link_key_for_target(&t);
        let cands = self.by_key.get(&key)?;
        let t_lower = t.to_lowercase();
        let t_noext = vpath::strip_note_ext(&t_lower).to_string();
        let src_dir = vpath::parent(source);
        // None when `../` climbs out of the vault.
        let rel_lower = vpath::resolve_relative(src_dir, &t_noext).map(|r| r.to_lowercase());
        // The folders a same-name match must end in: `../b/Note` -> `b/note`.
        let tail: Vec<&str> = t_noext.split('/').filter(|s| !matches!(*s, "" | "." | "..")).collect();
        let tail = tail.join("/");
        let without_md = |p: &str| -> String {
            let l = p.to_lowercase();
            if vpath::is_markdown(p) {
                l[..l.rfind('.').unwrap()].to_string()
            } else {
                l
            }
        };
        let has_slash = t_noext.contains('/');
        let mut best: Option<(u8, usize, &String)> = None;
        for c in cands {
            let c_noext = without_md(c);
            let rank = if c_noext == t_noext {
                0
            } else if has_slash && rel_lower.as_deref() == Some(c_noext.as_str()) {
                1
            } else if tail.contains('/') && c_noext != tail && !c_noext.ends_with(&format!("/{tail}")) {
                continue; // folder part must match
            } else if vpath::parent(c) == src_dir {
                2
            } else {
                3
            };
            // Length in characters, then code point order: links.ts
            // (LinkIndex.resolve) counts and compares the same way.
            let cand = (rank, c.chars().count(), c);
            if best.is_none_or(|b| (cand.0, cand.1, cand.2) < (b.0, b.1, b.2)) {
                best = Some(cand);
            }
        }
        best.map(|(_, _, p)| p.clone())
    }

    pub fn backlinks(&self, path: &str) -> Vec<Backlinks> {
        let key = vpath::link_key_for_file(path);
        let Some(sources) = self.link_sources.get(&key) else { return Vec::new() };
        let mut out = Vec::new();
        let mut sources: Vec<&String> = sources.iter().collect();
        sources.sort();
        for src in sources {
            if src == path {
                continue;
            }
            let Some(rec) = self.notes.get(src) else { continue };
            let items: Vec<BacklinkItem> = rec
                .parsed
                .links
                .iter()
                .filter(|l| link_target_key(l, src) == key)
                .filter(|l| self.resolve_link(l, src).as_deref() == Some(path))
                .map(|l| BacklinkItem {
                    line: l.line,
                    context: parse::line_context(&rec.content, l.start),
                })
                .collect();
            if !items.is_empty() {
                out.push(Backlinks { source: src.clone(), items });
            }
        }
        out
    }

    pub fn outgoing(&self, path: &str) -> Vec<OutgoingLink> {
        let Some(rec) = self.notes.get(path) else { return Vec::new() };
        rec.parsed
            .links
            .iter()
            .filter(|l| !l.target.is_empty())
            .map(|l| OutgoingLink {
                target: l.target.clone(),
                resolved: self.resolve_link(l, path),
                line: l.line,
                embed: l.embed,
                kind: l.kind,
            })
            .collect()
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let q = Query::parse(query);
        if q.is_empty() {
            return Vec::new();
        }
        let re = search::highlight_regex(&q);
        let candidates: Vec<(String, f32)> = if q.words.is_empty() {
            // Filters only: every note is a candidate, in path order.
            let mut all: Vec<(String, f32)> = self.notes.keys().map(|p| (p.clone(), 0.0)).collect();
            all.sort_by(|a, b| a.0.cmp(&b.0));
            all
        } else {
            self.search.search(&q)
        };
        let mut out = Vec::new();
        for (path, score) in candidates {
            let Some(rec) = self.notes.get(&path) else { continue };
            if !q.filters_match(&path, &rec.parsed.tags) {
                continue;
            }
            if !q.phrases.is_empty() {
                // The file name is indexed with the text, so a phrase may
                // be in either ("議事録" finds 議事録.md).
                let folded = search::fold(&rec.content);
                let title = search::fold(vpath::stem(&path));
                if !q.phrases.iter().all(|p| folded.contains(p.as_str()) || title.contains(p.as_str())) {
                    continue;
                }
            }
            let snippets = re
                .as_ref()
                .map(|re| search::snippets(&rec.content, re, 3))
                .unwrap_or_default();
            out.push(SearchHit { path, score, snippets });
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    /// Every tag with the number of notes using it, most used first.
    pub fn tags(&self) -> Vec<TagCount> {
        let mut counts: HashMap<&str, u32> = HashMap::new();
        for rec in self.notes.values() {
            for t in &rec.parsed.tags {
                *counts.entry(t.as_str()).or_default() += 1;
            }
        }
        let mut out: Vec<TagCount> = counts.into_iter().map(|(t, n)| TagCount { tag: t.to_string(), count: n }).collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
        out
    }

    /// Notes and the links between them. Unresolved link targets become
    /// nodes of kind `Unresolved` when `include_unresolved` is set.
    pub fn graph(&self, include_unresolved: bool) -> Graph {
        let mut paths: Vec<&String> = self.notes.keys().collect();
        paths.sort();
        let mut ids: HashMap<String, u32> = HashMap::new();
        let mut nodes: Vec<GraphNode> = Vec::with_capacity(paths.len());
        for p in &paths {
            ids.insert((*p).clone(), nodes.len() as u32);
            nodes.push(GraphNode { id: (*p).clone(), kind: GraphNodeKind::Note, degree: 0 });
        }
        let mut edges: HashSet<(u32, u32)> = HashSet::new();
        for p in &paths {
            let from = ids[*p];
            let rec = &self.notes[*p];
            for l in &rec.parsed.links {
                if l.target.is_empty() {
                    continue;
                }
                let to = match self.resolve_link(l, p) {
                    Some(t) => match ids.get(&t) {
                        Some(&id) => id,
                        None => continue, // attachment
                    },
                    None if include_unresolved => {
                        // The whole target, not just its name: [[Projects/Todo]]
                        // and [[Home/Todo]] are different missing notes. ./ and
                        // ../ count from the linking note's folder.
                        let mut t: String = l.target.replace('\\', "/").trim_start_matches('/').nfc().collect();
                        if t.split('/').any(|s| s == "." || s == "..") {
                            t = vpath::resolve_relative(vpath::parent(p), &t).unwrap_or(t);
                        }
                        let key = format!("?{}", vpath::strip_note_ext(&t.to_lowercase()));
                        match ids.get(&key) {
                            Some(&id) => id,
                            None => {
                                let id = nodes.len() as u32;
                                ids.insert(key, id);
                                nodes.push(GraphNode { id: t, kind: GraphNodeKind::Unresolved, degree: 0 });
                                id
                            }
                        }
                    }
                    None => continue,
                };
                if to != from {
                    edges.insert((from, to));
                }
            }
        }
        let mut edges: Vec<(u32, u32)> = edges.into_iter().collect();
        edges.sort_unstable();
        for &(a, b) in &edges {
            nodes[a as usize].degree += 1;
            nodes[b as usize].degree += 1;
        }
        Graph { nodes, edges }
    }

    pub fn note_info(&self, path: &str) -> Option<NoteInfo> {
        let rec = self.notes.get(path)?;
        Some(NoteInfo {
            frontmatter: rec.parsed.frontmatter.clone(),
            // A `---` block was found but could not be read.
            frontmatter_invalid: rec.parsed.body_start > 0 && rec.parsed.frontmatter.is_none(),
            tags: rec.parsed.tags.clone(),
            headings: rec.parsed.headings.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TagCount {
    pub tag: String,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphNodeKind {
    Note,
    Unresolved,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GraphNode {
    /// Vault path, or the link text for unresolved nodes.
    pub id: String,
    pub kind: GraphNodeKind,
    pub degree: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    /// Pairs of indexes into `nodes` (from, to).
    pub edges: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NoteInfo {
    pub frontmatter: Option<serde_json::Value>,
    /// The note has a frontmatter block that is not valid YAML.
    #[serde(rename = "frontmatterInvalid")]
    pub frontmatter_invalid: bool,
    pub tags: Vec<String>,
    pub headings: Vec<crate::parse::Heading>,
}

/// Key under which a link is filed in `link_sources`.
fn link_target_key(l: &Link, source: &str) -> String {
    if l.target.is_empty() {
        return vpath::link_key_for_file(source);
    }
    vpath::link_key_for_target(&l.target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(path: &str) -> FileStat {
        FileStat { path: path.into(), kind: EntryKind::File, size: 0, mtime: 0 }
    }

    fn build(files: &[(&str, &str)]) -> Index {
        let mut idx = Index::default();
        for (p, c) in files {
            if vpath::is_markdown(p) {
                idx.put_note(st(p), c.to_string(), hash_bytes(c.as_bytes()));
            } else {
                idx.put_entry(st(p), None);
            }
        }
        idx
    }

    #[test]
    fn resolution_rules() {
        let idx = build(&[
            ("Note.md", ""),
            ("a/Note.md", ""),
            ("a/b/Deep.md", ""),
            ("x/Deep.md", ""),
            ("x/y/Deep.md", ""),
            ("img/pic.png", ""),
        ]);
        // exact path wins
        assert_eq!(idx.resolve("Note", "a/b/Deep.md").as_deref(), Some("Note.md"));
        assert_eq!(idx.resolve("a/Note", "Note.md").as_deref(), Some("a/Note.md"));
        // same folder preferred over shortest
        assert_eq!(idx.resolve("Deep", "a/b/Other.md").as_deref(), Some("a/b/Deep.md"));
        // otherwise shortest path
        assert_eq!(idx.resolve("Deep", "Note.md").as_deref(), Some("x/Deep.md"));
        // partial path must match folders
        assert_eq!(idx.resolve("y/Deep", "Note.md").as_deref(), Some("x/y/Deep.md"));
        assert_eq!(idx.resolve("z/Deep", "Note.md"), None);
        // ./ and ../ are relative to the source folder; when nothing is
        // there, the folders after them must match, as for y/Deep
        assert_eq!(idx.resolve("../Note", "a/b/Deep.md").as_deref(), Some("a/Note.md"));
        assert_eq!(idx.resolve("./Deep", "x/y/q.md").as_deref(), Some("x/y/Deep.md"));
        assert_eq!(idx.resolve("../../x/Deep", "a/b/Deep.md").as_deref(), Some("x/Deep.md"));
        assert_eq!(idx.resolve("../y/Deep", "a/q.md").as_deref(), Some("x/y/Deep.md"));
        assert_eq!(idx.resolve("../z/Deep", "a/q.md"), None);
        assert_eq!(idx.resolve("Note/", "a/q.md"), None);
        // case-insensitive, with extension
        assert_eq!(idx.resolve("note.MD", "x/q.md").as_deref(), Some("Note.md"));
        // attachments by full name
        assert_eq!(idx.resolve("pic.png", "Note.md").as_deref(), Some("img/pic.png"));
        assert_eq!(idx.resolve("Missing", "Note.md"), None);
    }

    #[test]
    fn case_variants() {
        let idx = build(&[("Note.md", ""), ("a/note.MD", ""), ("A/Note.md", ""), ("img/Pic.png", ""), ("note.txt", "")]);
        assert_eq!(idx.case_variants("note.md"), vec!["Note.md".to_string()]);
        assert!(idx.case_variants("Note.md").is_empty());
        assert_eq!(idx.case_variants("a/note.md"), vec!["A/Note.md".to_string(), "a/note.MD".to_string()]);
        assert_eq!(idx.case_variants("IMG/pic.PNG"), vec!["img/Pic.png".to_string()]);
        assert!(idx.case_variants("other.md").is_empty());
    }

    #[test]
    fn backlinks_follow_resolution() {
        let mut idx = build(&[
            ("A.md", "Links to [[B]] and [[sub/B]] and [[B#Heading|alias]]"),
            ("B.md", "Back to [[A]]"),
            ("sub/B.md", "[[A]]\nsecond [[A]]"),
            ("C.md", "`[[A]]` in code, [md](A.md)"),
        ]);
        let bl = idx.backlinks("B.md");
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].source, "A.md");
        assert_eq!(bl[0].items.len(), 2);
        let bl = idx.backlinks("sub/B.md");
        assert_eq!(bl.len(), 1);
        let bl = idx.backlinks("A.md");
        let srcs: Vec<_> = bl.iter().map(|b| b.source.as_str()).collect();
        assert_eq!(srcs, vec!["B.md", "C.md", "sub/B.md"]);
        assert_eq!(bl[2].items.len(), 2);
        assert_eq!(bl[2].items[1].line, 1);

        // Removing B.md: the [[B]] links now resolve to sub/B.md
        idx.remove("B.md");
        let bl = idx.backlinks("sub/B.md");
        assert_eq!(bl[0].items.len(), 3);
    }

    #[test]
    fn unresolved_becomes_resolved_on_create() {
        let mut idx = build(&[("A.md", "[[Later]]")]);
        assert_eq!(idx.outgoing("A.md")[0].resolved, None);
        idx.put_note(st("Later.md"), String::new(), hash_bytes(b""));
        assert_eq!(idx.outgoing("A.md")[0].resolved.as_deref(), Some("Later.md"));
        assert_eq!(idx.backlinks("Later.md").len(), 1);
    }

    #[test]
    fn rename_tree_moves_children_and_links() {
        let mut idx = build(&[("f/A.md", "hello [[B]]"), ("B.md", ""), ("f/img.png", "")]);
        idx.put_entry(FileStat { path: "f".into(), kind: EntryKind::Dir, size: 0, mtime: 0 }, None);
        idx.rename_tree("f", "g", None);
        assert!(idx.entry("f/A.md").is_none());
        assert!(idx.note("g/A.md").is_some());
        assert!(idx.entry("g/img.png").is_some());
        assert_eq!(idx.backlinks("B.md")[0].source, "g/A.md");
        assert_eq!(idx.search("hello", 10)[0].path, "g/A.md");
        assert_eq!(idx.resolve("img.png", "B.md").as_deref(), Some("g/img.png"));
    }

    #[test]
    fn search_with_phrases_and_snippets() {
        let idx = build(&[
            ("a.md", "The quick brown fox"),
            ("b.md", "brown quick dog"),
            ("c.md", "---\ntitle: Frontmatter word zebra\n---\nbody"),
        ]);
        let r = idx.search("quick brown", 10);
        assert_eq!(r.len(), 2);
        let r = idx.search("\"quick brown\"", 10);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "a.md");
        assert!(r[0].snippets[0].segments.iter().any(|s| s.hit && s.text == "quick brown"));
        assert_eq!(idx.search("zebra", 10)[0].path, "c.md");
        assert!(idx.search("   ", 10).is_empty());
    }

    #[test]
    fn cjk_words_inside_sentences() {
        let idx = build(&[
            ("ja.md", "今日は東京で会議があります。"),
            // every pair of the query below, but never in a row
            ("apart.md", "会議。議が。があ。あり。りま。ます。"),
        ]);
        let paths = |q: &str| idx.search(q, 10).into_iter().map(|h| h.path).collect::<Vec<_>>();
        assert_eq!(paths("東京"), vec!["ja.md"]);
        assert_eq!(paths("会議があります"), vec!["ja.md"]);
        let hit = &idx.search("会議", 10)[0];
        assert!(hit.snippets[0].segments.iter().any(|s| s.hit && s.text == "会議"));
    }

    #[test]
    fn phrases_match_the_file_name() {
        let idx = build(&[
            ("議事録.md", "hello world"),
            ("notes/ファイルを開く.md", "How to open a file"),
            ("Trip plan.md", "packing list"),
        ]);
        let paths = |q: &str| idx.search(q, 10).into_iter().map(|h| h.path).collect::<Vec<_>>();
        assert_eq!(paths("議事録"), vec!["議事録.md"]);
        assert_eq!(paths("ファイルを開く"), vec!["notes/ファイルを開く.md"]);
        assert_eq!(paths("\"trip plan\""), vec!["Trip plan.md"]);
        assert_eq!(paths("議事録 hello"), vec!["議事録.md"]);
        assert!(paths("録議事").is_empty());
    }

    #[test]
    fn tag_and_path_filters() {
        let idx = build(&[
            ("p/a.md", "#plants basil"),
            ("p/b.md", "---\ntags: [plants/herbs]\n---\nmint"),
            ("q/c.md", "#plants tomato"),
            ("q/d.md", "no tags basil"),
        ]);
        let paths = |r: Vec<SearchHit>| r.into_iter().map(|h| h.path).collect::<Vec<_>>();
        assert_eq!(paths(idx.search("tag:plants", 10)), vec!["p/a.md", "p/b.md", "q/c.md"]);
        assert_eq!(paths(idx.search("#plants/herbs", 10)), vec!["p/b.md"]);
        assert_eq!(paths(idx.search("basil #plants", 10)), vec!["p/a.md"]);
        assert_eq!(paths(idx.search("path:q/ basil", 10)), vec!["q/d.md"]);
        let hits = idx.search("tag:plants", 10);
        assert!(hits[0].snippets[0].segments.iter().any(|s| s.hit && s.text == "#plants"));
        let tags = idx.tags();
        assert_eq!(tags[0], TagCount { tag: "plants".into(), count: 2 });
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn graph_nodes_and_edges() {
        let idx = build(&[("A.md", "[[B]] [[B]] [[Ghost]] [[A]]"), ("B.md", "[[A]] ![[pic.png]]"), ("C.md", ""), ("pic.png", "")]);
        let g = idx.graph(false);
        let ids: Vec<_> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["A.md", "B.md", "C.md"]);
        assert_eq!(g.edges, vec![(0, 1), (1, 0)]);
        assert_eq!(g.nodes[2].degree, 0);
        let g = idx.graph(true);
        assert_eq!(g.nodes.len(), 4);
        assert_eq!(g.nodes[3].kind, GraphNodeKind::Unresolved);
        assert!(g.edges.contains(&(0, 3)));
    }

    #[test]
    fn graph_ghost_nodes_are_keyed_by_whole_target() {
        let idx = build(&[("a/x.md", "[[./Todo]] [[Todo]]"), ("b/y.md", "[[./Todo]] [[todo.md]] [[/Todo]]")]);
        let g = idx.graph(true);
        let ghosts: Vec<_> = g.nodes.iter().filter(|n| n.kind == GraphNodeKind::Unresolved).map(|n| n.id.as_str()).collect();
        assert_eq!(ghosts, vec!["a/Todo", "Todo", "b/Todo"]);
    }

    #[test]
    fn note_info_tells_invalid_frontmatter_from_none() {
        let idx = build(&[
            ("Bad.md", "---\nkey: [unclosed\nother: value\n---\nbody\n"),
            ("Empty.md", "---\n---\nbody\n"),
            ("Good.md", "---\nkey: value\n---\nbody\n"),
            ("None.md", "body\n"),
        ]);
        let info = |p: &str| idx.note_info(p).unwrap();
        assert!(info("Bad.md").frontmatter.is_none() && info("Bad.md").frontmatter_invalid);
        assert!(!info("Empty.md").frontmatter_invalid);
        assert_eq!(info("Good.md").frontmatter, Some(serde_json::json!({ "key": "value" })));
        assert!(!info("Good.md").frontmatter_invalid);
        assert!(info("None.md").frontmatter.is_none() && !info("None.md").frontmatter_invalid);
    }
}
