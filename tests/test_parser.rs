//! Source positions and supported Markdown syntax of the link and anchor extraction.

use std::collections::HashSet;

use mdlink_audit::markdown::{Document, parse};

fn parse_markdown(source: &str) -> Document {
    parse(source, false)
}

fn links(document: &Document) -> Vec<(&str, usize)> {
    document
        .links
        .iter()
        .map(|link| (link.target.as_str(), link.line))
        .collect()
}

fn targets(document: &Document) -> Vec<&str> {
    document.links.iter().map(|link| link.target.as_str()).collect()
}

fn anchors(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn test_inline_images_and_reference_variants_report_use_lines() {
    let document = parse_markdown(
        "[inline](guide.md)\n![picture](images/chart.svg)\n[named][guide]\n[guide][]\n[guide]\n![photo][image]\n\n\
         [guide]: docs/guide.md \"Guide\"\n[image]: images/photo.png\n",
    );
    assert_eq!(
        links(&document),
        vec![
            ("guide.md", 1),
            ("images/chart.svg", 2),
            ("docs/guide.md", 3),
            ("docs/guide.md", 4),
            ("docs/guide.md", 5),
            ("images/photo.png", 6),
        ]
    );
}

#[test]
fn test_unused_reference_definitions_are_not_links() {
    assert!(parse_markdown("[unused]: missing.md\n").links.is_empty());
}

#[test]
fn test_multiline_paragraph_link_labels_and_destinations_keep_opening_line() {
    let document = parse_markdown(
        "Introduction\nstill introduction\n[a label\ncontinued](\n  docs/guide.md\n) and [second](other.md)\n",
    );
    assert_eq!(links(&document), vec![("docs/guide.md", 3), ("other.md", 6)]);
}

#[test]
fn test_link_wrapped_image_has_independent_line_numbers() {
    let document = parse_markdown("[\n![preview](preview.png)\n](full.png)\n");
    assert_eq!(links(&document), vec![("full.png", 1), ("preview.png", 2)]);
}

#[test]
fn test_pseudo_links_and_headings_in_code_or_comments_are_ignored() {
    let document = parse_markdown(
        "```md\n# fake\n[bad](fenced.md)\n```\n\n    [bad](indented.md)\n\n`[bad](inline.md)`\n\n\
         <!--\n# hidden\n[bad](comment.md)\n<a name='hidden'>\n-->\n\n\
         text <!-- [bad](inline-comment.md) --> [good](ok.md)\n",
    );
    assert_eq!(targets(&document), vec!["ok.md"]);
    assert!(document.anchors.is_empty());
}

#[test]
fn test_backtick_lengths_and_tilde_fences_are_parsed() {
    let document = parse_markdown(
        "`` a ` [bad](code.md) `` [good](good.md)\n\n~~~~\n[bad](fenced.md)\n~~~\n[bad](still-fenced.md)\n~~~~\n",
    );
    assert_eq!(links(&document), vec![("good.md", 1)]);
}

#[test]
fn test_heading_formatting_unicode_and_setext() {
    let document = parse_markdown(
        "# 中文 *使用* 与 `API` [指南](guide.md)！\n\nA **Setext** Heading\n--------------------\n\n\
         ## A  B Θ É e\u{301} under_score\n",
    );
    assert_eq!(
        document.anchors,
        anchors(&[
            "中文-使用-与-api-指南",
            "a-setext-heading",
            "a--b-Θ-É-e\u{301}-under_score"
        ])
    );
}

#[test]
fn test_heading_duplicate_suffixes_avoid_global_collisions() {
    let document = parse_markdown("# Foo\n# Foo-1\n# Foo\n# Foo\n# Foo-1\n");
    assert_eq!(
        document.anchors,
        anchors(&["foo", "foo-1", "foo-2", "foo-3", "foo-1-1"])
    );
}

#[test]
fn test_custom_anchors_do_not_participate_in_heading_numbering() {
    let document = parse_markdown(
        "<a name=\"hello\"></a>\n\n# Hello\n# Hello\n\n<a id=\"Custom&amp;Name\"></a>\n\n\
         A paragraph <a\n name=\"多行锚点\"></a>.\n",
    );
    assert_eq!(
        document.anchors,
        anchors(&["hello", "hello-1", "Custom&Name", "多行锚点"])
    );
}

#[test]
fn test_raw_html_links_images_and_non_anchor_ids_are_not_supported() {
    let document = parse_markdown(
        "<a href=\"missing.md\">raw</a>\n\n<img src=\"missing.png\">\n\n<div id=\"not-an-anchor\"></div>\n",
    );
    assert!(document.links.is_empty());
    assert!(document.anchors.is_empty());
}

#[test]
fn test_unicode_spaces_and_literal_hash_are_url_encoded_without_losing_meaning() {
    let document = parse_markdown(
        "[中文](<文档/安装 指南.md#开始使用>)\n[hash](docs/file%23name.md)\n[encoded](docs/already%20encoded.md)\n",
    );
    assert_eq!(
        mdlink_audit::urls::unquote_lossy(&document.links[0].target),
        "文档/安装 指南.md#开始使用"
    );
    assert_eq!(document.links[1].target, "docs/file%23name.md");
    assert_eq!(document.links[2].target, "docs/already%20encoded.md");
}

#[test]
fn test_nested_parentheses_and_escaped_brackets_in_link_text() {
    let document = parse_markdown(r"[a \[label\]](docs/a(b).md) [b](docs/a\(b\).md)");
    assert_eq!(targets(&document), vec!["docs/a(b).md", "docs/a(b).md"]);
}

#[test]
fn test_crlf_and_blockquote_positions() {
    let document = parse_markdown("# Hello\r\n\r\n> quote\r\n> [guide](guide.md)\r\n");
    assert_eq!(links(&document), vec![("guide.md", 4)]);
}

#[test]
fn test_markdown_table_links_and_strikethrough_heading_text() {
    let document =
        parse_markdown("# ~~Old~~ Guide\n\n| File | Status |\n| --- | --- |\n| [guide](guide.md) | ready |\n");
    assert_eq!(links(&document), vec![("guide.md", 5)]);
    assert!(document.anchors.contains("old-guide"));
}

#[test]
fn test_non_links_do_not_generate_findings() {
    for text in ["", "plain prose", "[undefined reference]", "\\[x](no.md)"] {
        assert!(parse_markdown(text).links.is_empty(), "{text}");
    }
}

#[test]
fn test_heading_entities_inline_html_and_image_alt_text() {
    let document = parse_markdown("# Fish &amp; <em>Chips</em> `x_y` ![badge](badge.svg)\n");
    assert_eq!(document.anchors, anchors(&["fish--chips-x_y-"]));
}

#[test]
fn test_frozen_results_are_safe_to_cache() {
    let document = parse_markdown("[x](x.md)");
    let cached = document.clone();
    assert_eq!(links(&cached), links(&document));
    assert_eq!(cached.anchors, document.anchors);
}

#[test]
fn test_closed_front_matter_ignored_without_changing_source_lines() {
    for terminator in ["---", "..."] {
        let document = parse_markdown(&format!(
            "\u{feff}---\r\ntitle: \"[metadata](absent.md)\"\r\n{terminator}\r\n# Real\r\n\r\n[body](guide.md)\r\n"
        ));
        assert_eq!(links(&document), vec![("guide.md", 6)], "{terminator}");
        assert_eq!(document.anchors, anchors(&["real"]));
    }
}

#[test]
fn test_unclosed_front_matter_does_not_hide_body() {
    let document = parse_markdown("---\n# Real\n[body](missing.md)\n");
    assert_eq!(links(&document), vec![("missing.md", 3)]);
}

#[test]
fn test_later_thematic_rule_does_not_hide_links() {
    let document = parse_markdown("# Heading\n\n---\n\n[body](guide.md)\n\n---\n");
    assert_eq!(links(&document), vec![("guide.md", 5)]);
}
