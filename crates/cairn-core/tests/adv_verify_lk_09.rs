// Reproduction for FINDING-183: parse.rs in_ranges() binary searches skip
// ranges with partition_point(|r| r.end <= pos), which assumes ranges that do
// not nest. A Markdown link pushes its whole range at End(Link) while code
// spans / inline HTML inside its text push their own (contained) ranges
// first, so the ranges must be merged before the search (merge_ranges);
// sorted by start alone, the predicate would not be monotonic.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_09 -- --include-ignored --nocapture
//
// Tests named `control_*` are controls and `observe_*` only prints; the
// others reproduce the finding.

use cairn_core::parse;

fn targets(src: &str) -> Vec<String> {
    parse::parse(src).links.iter().map(|l| l.target.clone()).collect()
}

/// Control: wikilinks inside Markdown link text are skipped by design.
#[test]
fn control_plain_link_text_skips_wikilink() {
    assert_eq!(targets("[see [[Wiki]]](x.md)"), vec!["x.md"]);
}

/// Control: wikilinks inside ordinary code spans and code blocks are skipped,
/// even with many ranges around them (no nesting involved).
#[test]
fn control_code_spans_skip_wikilinks() {
    assert_eq!(targets("`a` `[[W]]` `b`"), Vec::<String>::new());
    assert_eq!(targets("[a](x.md) `[[W]]` [b](y.md)"), vec!["x.md", "y.md"]);
    assert_eq!(targets("```\n[[W]]\n```\n[a](x.md)"), vec!["x.md"]);
}

/// Does the nesting ever let a wikilink inside a *code span* through (which
/// would contradict PLAN "links inside code spans are ignored")? Observation:
/// print what each variant gives.
#[test]
fn observe_variants() {
    for src in [
        "[`code` see [[Wiki]]](x.md)",
        "[<b>bold</b> [[Wiki]]](x.md)",
        "[`a` `[[W]]`](x.md)",
        "[`a` b `[[W]]` c](x.md)",
        "[`a` [[W]]](x.md) `z`",
        "[`a` [[W]]](x.md) `z` `y` `q`",
        "[`a` `b` `c` [[W]]](x.md)",
    ] {
        println!("{src:?} -> {:?}", targets(src));
    }
}

/// The finding: same link text, different answer when a code span or inline
/// HTML precedes the wikilink.
#[test]
fn finding_nested_ranges_change_answer() {
    assert_eq!(targets("[`code` see [[Wiki]]](x.md)"), vec!["x.md"], "after a code span");
    assert_eq!(targets("[<b>bold</b> [[Wiki]]](x.md)"), vec!["x.md"], "after inline HTML");
}
