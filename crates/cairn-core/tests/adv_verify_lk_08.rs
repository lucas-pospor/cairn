// Reproduction for FINDING-085: the core and the UI must agree on where
// frontmatter ends.
//
// parse.rs split_frontmatter and the UI (markdown.ts stripFrontmatter,
// editor/livePreview.ts buildBlocks, app.svelte.ts loadTab) both accept a
// closing fence with trailing spaces or tabs, and an empty "---\n---" block.
// A core that took only a closing line of exactly "---" or "..." would
// disagree on the first; a UI that needed a newline before the closing fence
// would run an empty block on to the next "---" line.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_08 -- --nocapture

use cairn_core::parse;

fn heads(p: &parse::ParsedNote) -> Vec<String> {
    p.headings.iter().map(|h| h.text.clone()).collect()
}
fn targets(p: &parse::ParsedNote) -> Vec<String> {
    p.links.iter().map(|l| l.target.clone()).collect()
}

/// Control: the same note without the trailing spaces is frontmatter.
#[test]
fn control_exact_close_is_frontmatter() {
    let p = parse::parse("---\ntitle: Hello\ntags: [alpha]\nrelated: \"[[Other]]\"\n---\n# Real\n");
    assert!(p.frontmatter.is_some());
    assert_eq!(p.tags, vec!["alpha"]);
    assert_eq!(heads(&p), vec!["Real"]);
    assert!(targets(&p).is_empty(), "links inside frontmatter are skipped");
}

#[test]
fn trailing_space_close_is_frontmatter() {
    let p = parse::parse("---\ntitle: Hello\ntags: [alpha]\nrelated: \"[[Other]]\"\n---  \n# Real\n");
    assert!(p.frontmatter.is_some(), "frontmatter=None tags={:?} headings={:?} links={:?}", p.tags, heads(&p), targets(&p));
    assert_eq!(p.tags, vec!["alpha"]);
    assert_eq!(heads(&p), vec!["Real"]);
    assert_eq!(p.body_start, 58, "same as the UI (app/src/lib/adv_verify_lk_08.test.ts)");
}

/// Worse variant: when a later line is exactly "---" (a thematic break), a
/// core that skipped the "---  " fence would close the frontmatter at the
/// rule. The YAML would then be invalid (frontmatter=None) but body_start
/// would still point past the rule, so links, tags and headings the user sees
/// in the first section would drop out of the index (backlinks, tags panel,
/// outline).
#[test]
fn trailing_space_close_with_later_rule_keeps_first_section_indexed() {
    let src = "---\ntitle: Hello\n---  \n# Intro\nSee [[Linked]] #intro\n\n---\n\nRest\n";
    let p = parse::parse(src);
    let where_body = &src[p.body_start..];
    assert!(
        targets(&p).contains(&"Linked".to_string()),
        "first section not indexed: body_start={} body={where_body:?} frontmatter={:?} links={:?} tags={:?} headings={:?}",
        p.body_start,
        p.frontmatter,
        targets(&p),
        p.tags,
        heads(&p)
    );
    assert!(p.tags.contains(&"intro".to_string()));
    assert!(heads(&p).contains(&"Intro".to_string()));
}

/// Core side of the empty-block case: "---\n---" is empty frontmatter and the
/// intro paragraph is body (indexed). A UI that treated everything up to the
/// next "---" rule as YAML would hide it (see app/src/lib/adv_verify_lk_08.test.ts).
#[test]
fn observed_empty_block_core_indexes_intro() {
    let src = "---\n---\nIntro paragraph with [[Intro]] #introtag\n\n---\n\nRest\n";
    let p = parse::parse(src);
    assert_eq!(p.frontmatter, Some(serde_json::json!({})));
    assert_eq!(p.body_start, 8);
    assert_eq!(targets(&p), vec!["Intro"]);
    assert_eq!(p.tags, vec!["introtag"]);
}
