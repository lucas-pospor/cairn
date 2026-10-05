//! Extract links, headings, tags and frontmatter from a note.

use std::ops::Range;
use std::sync::LazyLock;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    /// `[[target]]`
    Wiki,
    /// `[text](target.md)`
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// Path part of the link, without `#heading` or `|alias`.
    pub target: String,
    /// `heading` or `^block` after `#`.
    pub subpath: Option<String>,
    /// Alias (`[[a|alias]]`) or Markdown link text.
    pub display: Option<String>,
    /// `![[...]]` or `![](...)`.
    pub embed: bool,
    pub kind: LinkKind,
    /// Byte range of the whole link in the source.
    pub start: usize,
    pub end: usize,
    /// 0-based line number.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedNote {
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    /// Lowercased, without `#`, deduplicated, in order of first appearance.
    pub tags: Vec<String>,
    /// YAML frontmatter converted to JSON. `None` if absent or invalid.
    pub frontmatter: Option<serde_json::Value>,
    /// Byte offset where the body starts (after a BOM and frontmatter).
    pub body_start: usize,
}

static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(!?)\[\[([^\[\]\n]+?)\]\]").unwrap());
// A tag must follow start-of-line or whitespace and contain a non-digit.
// Combining marks (\p{M}) belong to the tag: NFD accents, and the vowel
// signs of scripts such as Devanagari and Thai.
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(?:^|[\s,;])#([\p{L}\p{M}\p{N}_/\-]*[\p{L}_/\-][\p{L}\p{M}\p{N}_/\-]*)").unwrap()
});

/// Length of a leading UTF-8 byte order mark (Windows editors write one), or 0.
fn bom_len(content: &str) -> usize {
    if content.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    }
}

/// Split frontmatter off the top of a note. Returns (yaml text, body offset).
///
/// The opening line is exactly `---` (after an optional BOM); the block ends at
/// the first line that is `---` or `...` plus optional spaces or tabs, and may
/// be empty. The UI uses the same rule (app/src/lib/markdown.ts FRONTMATTER_RE).
pub fn split_frontmatter(content: &str) -> Option<(&str, usize)> {
    let top = &content[bom_len(content)..];
    let rest = top
        .strip_prefix("---\n")
        .or_else(|| top.strip_prefix("---\r\n"))?;
    let open_len = content.len() - rest.len();
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let eol = line
            .strip_suffix('\n')
            .map_or(line, |l| l.strip_suffix('\r').unwrap_or(l));
        let trimmed = eol.trim_end_matches([' ', '\t']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = &rest[..offset];
            return Some((yaml, open_len + offset + line.len()));
        }
        offset += line.len();
    }
    None
}

/// Parse the YAML frontmatter into JSON, if valid.
pub fn parse_frontmatter(yaml: &str) -> Option<serde_json::Value> {
    if yaml.trim().is_empty() {
        return Some(serde_json::Value::Object(Default::default()));
    }
    let v: serde_yaml::Value = serde_yaml::from_str(yaml).ok()?;
    serde_json::to_value(v).ok()
}

struct LineIndex(Vec<usize>);

impl LineIndex {
    fn new(s: &str) -> Self {
        let mut v = vec![0];
        v.extend(s.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex(v)
    }
    fn line_of(&self, offset: usize) -> u32 {
        (self.0.partition_point(|&s| s <= offset) - 1) as u32
    }
}

fn in_ranges(ranges: &[Range<usize>], pos: usize) -> bool {
    // ranges are sorted and disjoint (merge_ranges)
    let i = ranges.partition_point(|r| r.end <= pos);
    i < ranges.len() && ranges[i].start <= pos
}

/// True if the character at `pos` follows an odd number of backslashes, so
/// a backslash escapes it.
fn escaped(content: &str, pos: usize) -> bool {
    content.as_bytes()[..pos].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1
}

/// Sort `ranges` and merge the ones that overlap. They nest: a Markdown
/// link's range contains the code spans and inline HTML in its text, and a
/// binary search over nested ranges misses positions after an inner one.
fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

fn heading_level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn is_external(url: &str) -> bool {
    url.starts_with('#')
        || url.starts_with("//")
        || url
            .split_once(':')
            .is_some_and(|(scheme, _)| {
                !scheme.is_empty()
                    && scheme.len() > 1 // keep "C:" style paths out of scope anyway
                    && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            })
}

/// Split `target#sub|alias` inside a wikilink.
pub fn split_wikilink_inner(inner: &str) -> (String, Option<String>, Option<String>) {
    let (main, alias) = match inner.find('|') {
        Some(i) => {
            // In tables the pipe is escaped as `\|`.
            let main = inner[..i].strip_suffix('\\').unwrap_or(&inner[..i]);
            (main, Some(inner[i + 1..].trim().to_string()))
        }
        None => (inner, None),
    };
    let (target, sub) = match main.find('#') {
        Some(i) => (&main[..i], Some(main[i + 1..].trim().to_string())),
        None => (main, None),
    };
    (
        target.trim().to_string(),
        sub.filter(|s| !s.is_empty()),
        alias.filter(|s| !s.is_empty()),
    )
}

pub fn parse(content: &str) -> ParsedNote {
    let mut out = ParsedNote::default();
    let lines = LineIndex::new(content);

    // A BOM is not text: without this, "\u{feff}# Title" is not a heading.
    out.body_start = bom_len(content);
    if let Some((yaml, body)) = split_frontmatter(content) {
        out.frontmatter = parse_frontmatter(yaml);
        out.body_start = body;
    }
    let body_start = out.body_start;

    // Ranges where links and tags must not be detected.
    let mut skip: Vec<Range<usize>> = Vec::new();
    if body_start > 0 {
        skip.push(0..body_start);
    }

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);

    let body = &content[body_start..];
    let mut heading: Option<(u8, usize, String)> = None;
    // Open Markdown links and images (start, url, embed, text). An image
    // can sit in a link's text: `[![badge](img.png)](Note.md)`.
    let mut md_links: Vec<(usize, String, bool, String)> = Vec::new();
    for (ev, range) in Parser::new_ext(body, opts).into_offset_iter() {
        let range = (range.start + body_start)..(range.end + body_start);
        match ev {
            Event::Start(Tag::CodeBlock(_)) => skip.push(range),
            Event::Code(ref text) => {
                skip.push(range.clone());
                if let Some((_, _, t)) = heading.as_mut() {
                    t.push_str(text);
                }
                for (_, _, _, t) in md_links.iter_mut() {
                    t.push_str(text);
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => skip.push(range),
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((heading_level(level), range.start, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, text)) = heading.take() {
                    out.headings.push(Heading {
                        level,
                        text: text.trim().to_string(),
                        line: lines.line_of(start),
                    });
                }
            }
            Event::Text(ref text) => {
                if let Some((_, _, t)) = heading.as_mut() {
                    t.push_str(text);
                }
                for (_, _, _, t) in md_links.iter_mut() {
                    t.push_str(text);
                }
            }
            // A setext heading can span lines: "Line one\nline two\n===".
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, t)) = heading.as_mut() {
                    t.push(' ');
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                md_links.push((range.start, dest_url.to_string(), false, String::new()));
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                md_links.push((range.start, dest_url.to_string(), true, String::new()));
            }
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => {
                if let Some((start, url, embed, text)) = md_links.pop() {
                    if let Some(link) = markdown_link(&url, embed, text, start, range.end, &lines) {
                        out.links.push(link);
                    }
                    // Wikilink syntax inside a markdown link is not a link.
                    skip.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    let skip = merge_ranges(skip);

    for cap in WIKILINK_RE.captures_iter(content) {
        let whole = cap.get(0).unwrap();
        let (mut start, mut embed) = (whole.start(), !cap[1].is_empty());
        if escaped(content, start) {
            // CommonMark: `\[[x]]` is literal text, `\![[x]]` a `!` and a link.
            if !embed {
                continue;
            }
            (start, embed) = (start + 1, false);
        }
        if in_ranges(&skip, start) {
            continue;
        }
        let (target, subpath, display) = split_wikilink_inner(&cap[2]);
        if target.is_empty() && subpath.is_none() {
            continue;
        }
        out.links.push(Link {
            target,
            subpath,
            display,
            embed,
            kind: LinkKind::Wiki,
            start,
            end: whole.end(),
            line: lines.line_of(start),
        });
    }
    out.links.sort_by_key(|l| l.start);

    let mut seen = std::collections::HashSet::new();
    let mut push_tag = |t: &str, tags: &mut Vec<String>| {
        // Folded like search text, so NFD and NFC spellings are one tag.
        let t = crate::search::fold(t.trim().trim_start_matches('#').trim_end_matches('/'));
        if !t.is_empty() && seen.insert(t.clone()) {
            tags.push(t);
        }
    };
    if let Some(fm) = &out.frontmatter {
        for key in ["tags", "tag"] {
            match fm.get(key) {
                Some(serde_json::Value::String(s)) => {
                    for t in s.split([',', ' ']).filter(|t| !t.is_empty()) {
                        push_tag(t, &mut out.tags);
                    }
                }
                Some(serde_json::Value::Array(a)) => {
                    for v in a {
                        if let Some(s) = v.as_str() {
                            push_tag(s, &mut out.tags);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // A tag inside a link like [[a #b]] is not a tag. Merged once, so each
    // tag is a binary search instead of a scan of every link.
    let in_link = merge_ranges(out.links.iter().map(|l| l.start..l.end).collect());
    for cap in TAG_RE.captures_iter(content) {
        let m = cap.get(1).unwrap();
        if in_ranges(&skip, m.start()) || in_ranges(&in_link, m.start()) {
            continue;
        }
        push_tag(m.as_str(), &mut out.tags);
    }
    out
}

fn markdown_link(
    url: &str,
    embed: bool,
    text: String,
    start: usize,
    end: usize,
    lines: &LineIndex,
) -> Option<Link> {
    let url = url.trim().trim_start_matches('<').trim_end_matches('>');
    if url.is_empty() || is_external(url) {
        return None;
    }
    let (path, sub) = match url.find('#') {
        Some(i) => (&url[..i], Some(&url[i + 1..])),
        None => (url, None),
    };
    let decode = |s: &str| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned();
    let path = decode(path);
    if path.is_empty() {
        return None;
    }
    Some(Link {
        target: path,
        subpath: sub.map(decode).filter(|s| !s.is_empty()),
        display: Some(text).filter(|s| !s.is_empty()),
        embed,
        kind: LinkKind::Markdown,
        start,
        end,
        line: lines.line_of(start),
    })
}

/// Text of the line containing `offset`, trimmed and capped for display.
pub fn line_context(content: &str, offset: usize) -> String {
    let start = content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = content[offset..].find('\n').map(|i| offset + i).unwrap_or(content.len());
    let line = content[start..end].trim();
    if line.chars().count() > 240 {
        let s: String = line.chars().take(240).collect();
        format!("{s}…")
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(p: &ParsedNote) -> Vec<&str> {
        p.links.iter().map(|l| l.target.as_str()).collect()
    }

    #[test]
    fn wikilinks_basic() {
        let p = parse("See [[Note A]] and [[folder/B|bee]] and ![[pic.png]]\n[[C#Intro]]");
        assert_eq!(targets(&p), vec!["Note A", "folder/B", "pic.png", "C"]);
        assert_eq!(p.links[1].display.as_deref(), Some("bee"));
        assert!(p.links[2].embed);
        assert_eq!(p.links[3].subpath.as_deref(), Some("Intro"));
        assert_eq!(p.links[3].line, 1);
        assert_eq!(p.links[0].kind, LinkKind::Wiki);
    }

    #[test]
    fn heading_only_link_and_escaped_pipe() {
        let p = parse("[[#Local]] | [[x\\|y]]");
        assert_eq!(p.links[0].target, "");
        assert_eq!(p.links[0].subpath.as_deref(), Some("Local"));
        assert_eq!(p.links[1].target, "x");
        assert_eq!(p.links[1].display.as_deref(), Some("y"));
    }

    #[test]
    fn escaped_wikilinks() {
        // As in CommonMark: `\[` is a literal bracket, `\\` a literal
        // backslash, and `\!` a literal `!` before an ordinary link.
        let p = parse("\\[[A]] \\\\[[B]] \\![[C]] ![[D]] !\\[[E]]");
        assert_eq!(targets(&p), vec!["B", "C", "D"]);
        assert!(!p.links[1].embed && p.links[2].embed);
        assert_eq!(p.links[1].start, "\\[[A]] \\\\[[B]] \\!".len());
    }

    #[test]
    fn ignores_code() {
        let src = "real [[A]]\n`[[B]]`\n```\n[[C]]\n#notatag\n```\n    [[D]] indented code\n";
        let p = parse(src);
        assert_eq!(targets(&p), vec!["A"]);
        assert!(p.tags.is_empty());
    }

    #[test]
    fn markdown_links() {
        let p = parse("[x](other%20note.md) [y](https://e.com) [z](#h) ![i](img/a.png) [w](sub/n.md#Top)");
        assert_eq!(targets(&p), vec!["other note.md", "img/a.png", "sub/n.md"]);
        assert_eq!(p.links[0].kind, LinkKind::Markdown);
        assert_eq!(p.links[0].display.as_deref(), Some("x"));
        assert!(p.links[1].embed);
        assert_eq!(p.links[2].subpath.as_deref(), Some("Top"));
    }

    #[test]
    fn mailto_is_external() {
        let p = parse("[m](mailto:a@b.c)");
        assert!(p.links.is_empty());
    }

    #[test]
    fn headings() {
        let p = parse("# Title\ntext\n## Sub `code`\nSetext\n===\n");
        let h: Vec<_> = p.headings.iter().map(|h| (h.level, h.text.as_str(), h.line)).collect();
        assert_eq!(h, vec![(1, "Title", 0), (2, "Sub code", 2), (1, "Setext", 3)]);
    }

    #[test]
    fn frontmatter_and_tags() {
        let src = "---\ntitle: Hi\ntags: [Alpha, beta/x]\n---\n# H\nBody #gamma and #123 not, #a-b.\n[[L]]";
        let p = parse(src);
        let fm = p.frontmatter.clone().unwrap();
        assert_eq!(fm["title"], "Hi");
        assert_eq!(p.tags, vec!["alpha", "beta/x", "gamma", "a-b"]);
        assert_eq!(targets(&p), vec!["L"]);
        assert_eq!(p.headings[0].line, 4);
        assert_eq!(&src[p.body_start..p.body_start + 3], "# H");
    }

    #[test]
    fn frontmatter_string_tags_and_invalid_yaml() {
        let p = parse("---\ntags: one, two\n---\n");
        assert_eq!(p.tags, vec!["one", "two"]);
        let p = parse("---\n: : bad\n  - [\n---\n[[X]]");
        assert!(p.frontmatter.is_none());
        assert_eq!(targets(&p), vec!["X"]);
    }

    #[test]
    fn no_frontmatter_when_unclosed() {
        let p = parse("---\nnot closed\n[[X]]");
        assert!(p.frontmatter.is_none());
        assert_eq!(p.body_start, 0);
    }

    #[test]
    fn frontmatter_after_bom_and_loose_close() {
        // A BOM (Windows editors) hides neither the frontmatter nor a heading.
        let src = "\u{feff}---\ntags: [a]\n---\n# H\n";
        let p = parse(src);
        assert_eq!(p.tags, vec!["a"]);
        assert_eq!(&src[p.body_start..], "# H\n");
        let p = parse("\u{feff}# Title\n");
        assert_eq!(p.headings[0].text, "Title");
        // The closing line may end in spaces or tabs, and the block may be empty.
        assert_eq!(
            split_frontmatter("---\nk: v\n--- \t\r\nbody"),
            Some(("k: v\n", 16))
        );
        assert_eq!(split_frontmatter("---\n...  \nbody"), Some(("", 10)));
        assert_eq!(split_frontmatter("---\n---\nbody"), Some(("", 8)));
        // The opening line may not, and the fence is not a longer rule.
        assert_eq!(split_frontmatter("--- \nk: v\n---\n"), None);
        assert_eq!(split_frontmatter("---\nk: v\n----\n"), None);
    }

    #[test]
    fn heading_is_not_tag() {
        let p = parse("# Heading\n#tag\nurl.com/#frag");
        assert_eq!(p.tags, vec!["tag"]);
    }

    #[test]
    fn line_context_works() {
        let s = "one\n  two [[x]] three  \nfour";
        assert_eq!(line_context(s, 10), "two [[x]] three");
    }
}
