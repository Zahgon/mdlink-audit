//! Opt-in static HTML extraction follows Markdown boundaries and source lines.

use std::collections::HashSet;

use mdlink_audit::markdown::{Document, parse};

fn html(source: &str) -> Document {
    parse(source, true)
}

fn links(document: &Document) -> Vec<(&str, usize)> {
    document
        .links
        .iter()
        .map(|link| (link.target.as_str(), link.line))
        .collect()
}

fn anchors(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn test_html_is_opt_in_and_existing_explicit_anchors_still_work() {
    let source = "<div id=\"container\"><a name=\"legacy\" id=\"anchor\" href=\"guide.md\"><img src=\"picture.png\"></a></div>\n";
    assert!(parse(source, false).links.is_empty());
    assert_eq!(parse(source, false).anchors, anchors(&["legacy", "anchor"]));
    let document = html(source);
    assert_eq!(links(&document), vec![("guide.md", 1), ("picture.png", 1)]);
    assert_eq!(document.anchors, anchors(&["container", "legacy", "anchor"]));
}

#[test]
fn test_html_option_is_keyword_only() {
    // The HTML switch is an explicit argument: the same source gives different links with it off and on.
    let source = "<a href=\"guide.md\">Guide</a>\n";
    assert!(parse(source, false).links.is_empty());
    assert_eq!(links(&parse(source, true)), vec![("guide.md", 1)]);
}

#[test]
fn test_block_links_use_opening_tag_line_across_multiline_attributes() {
    let document = html(
        "# Title\n\n<div>\n  <a\n    href='docs/guide.md'\n  >Guide</a>\n  <img\n    src=\"images/chart.svg\"\n  />\n\
         </div>\n\n[Markdown](other.md)\n",
    );
    assert_eq!(
        links(&document),
        vec![("docs/guide.md", 4), ("images/chart.svg", 7), ("other.md", 12)]
    );
}

#[test]
fn test_inline_links_keep_document_order_and_lines_across_fragments() {
    let document = html(
        "Intro <a href=\"first.md\">first</a> [second](second.md)\nmore prose\n\
         <img\n src=\"third.svg\"> and <a href=fourth.md>fourth</a>\n\n\n[ending](last.md)\n",
    );
    assert_eq!(
        links(&document),
        vec![
            ("first.md", 1),
            ("second.md", 1),
            ("third.svg", 3),
            ("fourth.md", 4),
            ("last.md", 7)
        ]
    );
}

#[test]
fn test_quoted_and_unquoted_attributes_and_case_insensitive_names() {
    for attribute in ["href=\"guide.md\"", "href='guide.md'", "HREF=guide.md"] {
        let document = html(&format!("<A {attribute}>Guide</A>"));
        assert_eq!(links(&document), vec![("guide.md", 1)], "{attribute}");
    }
}

#[test]
fn test_html_entities_decode_once_without_decoding_percent_escapes() {
    let document = html(
        "<a href=\"文档/安装&#32;指南.md?x=1&amp;y=2#开始\">Guide</a>\n\
         <a href=\"file%2520name.md#literal&amp;amp;anchor\">Once</a>\n\
         <span id=\"A&amp;B&#x4e2d;\"></span>\n",
    );
    assert_eq!(
        links(&document),
        vec![
            ("文档/安装 指南.md?x=1&y=2#开始", 1),
            ("file%2520name.md#literal&amp;anchor", 2)
        ]
    );
    assert_eq!(document.anchors, anchors(&["A&B中"]));
}

#[test]
fn test_angle_brackets_inside_quoted_attribute_are_not_nested_markup() {
    let document =
        html("<div title=\"<img src=hidden.png>\" id=\"visible\">\n<a href='guide.md?value=a>b'>Guide</a>\n</div>\n");
    assert_eq!(links(&document), vec![("guide.md?value=a>b", 2)]);
    assert_eq!(document.anchors, anchors(&["visible"]));
}

#[test]
fn test_all_ids_but_only_anchor_names_are_collected() {
    let document = html(
        "<section id=\"Section\"><h2 id=\"Heading\">Title</h2>\n<input name=\"not-an-anchor\"><a name=\"Legacy\"></a>\n\
         <div id=\"\"></div><span id></span></section>\n",
    );
    assert_eq!(document.anchors, anchors(&["Section", "Heading", "Legacy"]));
}

#[test]
fn test_href_and_src_resources_from_multiple_tags() {
    let document = html(
        "<div><link href=\"theme.css\"><video src=\"movie.mp4\">\n\
         <source src=\"audio.ogg\"></video><iframe src=\"page.html\"></iframe>\n\
         <script src=\"app.js\"></script><img src=\"cover.svg\" /></div>\n",
    );
    assert_eq!(
        links(&document),
        vec![
            ("theme.css", 1),
            ("movie.mp4", 1),
            ("audio.ogg", 2),
            ("page.html", 2),
            ("app.js", 3),
            ("cover.svg", 3)
        ]
    );
}

#[test]
fn test_empty_url_is_reported_but_missing_attribute_value_is_ignored() {
    let document = html("<a href=\"\">Self</a><a href>None</a>");
    assert_eq!(links(&document), vec![("", 1)]);
}

#[test]
fn test_code_comments_and_front_matter_do_not_produce_html_links_or_ids() {
    let document = html(
        "---\ntitle: '<a href=\"metadata.md\" id=\"metadata\">'\n---\n\
         ```html\n<a href=\"fenced.md\" id=\"fenced\">\n```\n\n\
         \x20   <img src=\"indented.svg\" id=\"indented\">\n\n\
         `<a href=\"inline.md\" id=\"inline\">`\n\n\
         <!--\n<a href=\"comment.md\" id=\"comment\">\n-->\n\n\
         Text <!-- <img src=\"inline-comment.svg\" id=\"inline-comment\"> --> \
         <a href=\"real.md\" id=\"real\">Real</a>\n",
    );
    assert_eq!(links(&document), vec![("real.md", 16)]);
    assert_eq!(document.anchors, anchors(&["real"]));
}

#[test]
fn test_raw_text_block_bodies_are_ignored() {
    for tag in ["script", "style"] {
        let document = html(&format!(
            "<{tag} id=\"resource\">\n<a href=\"fake.md\" id=\"fake\">\n[fake](also-fake.md)\n</{tag}>\n\n\
             <a href=\"real.md\">Real</a>\n"
        ));
        assert_eq!(links(&document), vec![("real.md", 6)], "{tag}");
        assert_eq!(document.anchors, anchors(&["resource"]), "{tag}");
    }
}

#[test]
fn test_raw_text_inline_bodies_do_not_leak_markdown_or_html_links() {
    for tag in ["script", "style"] {
        let document = html(&format!(
            "Prefix <{tag}>[fake](markdown.md) <a href=\"fake.md\" id=\"fake\">\n\
             continued</{tag}> <a href=\"real.md\" id=\"real\">Real</a>\n[after](after.md)\n"
        ));
        assert_eq!(links(&document), vec![("real.md", 2), ("after.md", 3)], "{tag}");
        assert_eq!(document.anchors, anchors(&["real"]), "{tag}");
    }
}

#[test]
fn test_unclosed_inline_script_keeps_later_html_and_markdown_as_raw_text() {
    let document =
        html("Prefix <script>[fake](markdown.md)\n\n<a href=\"fake.md\" id=\"fake\">\n\n[also fake](later.md)\n");
    assert!(document.links.is_empty());
    assert!(document.anchors.is_empty());
}

#[test]
fn test_html_is_not_inferred_from_escaped_markup_or_image_alt_text() {
    let document = html("&lt;a href=\"escaped.md\"&gt;\n![<img src=\"alt.svg\">](real.svg)\n");
    assert_eq!(links(&document), vec![("real.svg", 2)]);
}

#[test]
fn test_crlf_blockquotes_and_html_source_positions() {
    let document = html(
        "# Title\r\n\r\n> Intro\r\n> <a\r\n> href=\"guide.md\">Guide</a>\r\n\r\n<div>\r\n<img src=\"chart.svg\">\r\n</div>\r\n",
    );
    assert_eq!(links(&document), vec![("guide.md", 4), ("chart.svg", 8)]);
}

#[test]
fn test_unmatched_end_tags_do_not_prevent_later_links() {
    let document = html("<div></p><a href=guide.md>Guide</span>\n<img src=\"chart.svg\">\n");
    assert_eq!(links(&document), vec![("guide.md", 1), ("chart.svg", 2)]);
}

#[test]
fn test_unterminated_html_attribute_is_not_a_complete_link() {
    let document = html("<div>\n<a href=\"unterminated.md\n");
    assert!(document.links.is_empty());
}

#[test]
fn test_css_srcset_and_dynamic_attributes_are_not_interpreted() {
    let document = html(
        "<div style=\"background:url(css.png)\" :href=\"dynamic.md\">\n\
         <img srcset=\"one.svg 1x, two.svg 2x\" data-src=\"lazy.svg\">\n</div>\n",
    );
    assert!(document.links.is_empty());
}

#[test]
fn test_raw_text_across_markdown_blocks_does_not_create_heading_anchors() {
    for tag in ["script", "style"] {
        let source = format!("Prefix <{tag}>\n\n# Fake [bad](absent.md)\n\n</{tag}>\n\n# Real\n\n[after](real.md)\n");
        let document = html(&source);
        assert_eq!(links(&document), vec![("real.md", 9)], "{tag}");
        assert_eq!(document.anchors, anchors(&["real"]), "{tag}");
        // The option preserves existing Markdown-only behaviour when disabled.
        let default = parse(&source, false);
        assert_eq!(links(&default), vec![("absent.md", 3), ("real.md", 9)], "{tag}");
        assert_eq!(default.anchors, anchors(&["fake-bad", "real"]), "{tag}");
    }
}

#[test]
fn test_raw_text_ignores_other_tag_names_and_code_formatted_end_tags() {
    for tag in ["script", "style"] {
        let other = if tag == "script" { "style" } else { "script" };
        let document = html(&format!(
            "Prefix <{tag}>\n<{other}><a href=\"fake.md\" id=\"fake\"></{other}>\n\
             `</{tag}>` <a href=\"still-fake.md\" id=\"still-fake\">\n\
             </{tag}> <a href=\"real.md\" id=\"real\">Real</a>\n"
        ));
        assert_eq!(links(&document), vec![("real.md", 4)], "{tag}");
        assert_eq!(document.anchors, anchors(&["real"]), "{tag}");
    }
}

#[test]
fn test_mixed_html_blocks_inline_tags_and_containers_keep_source_lines() {
    let document = html(
        "<div>\n<a href=\"block.md\">block</a>\n</div>\n\nPlain text\nwith a <span id=\"inline\">span</span>\n\n\
         > Quoted <a\n> href=\"quote.md\">quote</a>\n\n* <img\n  src=\"list.svg\"> and [markdown](list.md)\n\n\
         <section>\n  <a href=\"last.md\">last</a>\n</section>\n",
    );
    assert_eq!(
        links(&document),
        vec![
            ("block.md", 2),
            ("quote.md", 8),
            ("list.svg", 11),
            ("list.md", 12),
            ("last.md", 15)
        ]
    );
    assert_eq!(document.anchors, anchors(&["inline"]));
}

#[test]
fn test_entity_newlines_do_not_shift_later_source_positions() {
    let document = html(
        "Intro <a href=\"first.md\" title=\"&#10;&#13;\">first</a>\n\
         <span id=\"A&amp;amp;B\"></span> <a href=\"second.md\">second</a>\n\n<a\n href=\"third.md\">third</a>\n",
    );
    assert_eq!(
        links(&document),
        vec![("first.md", 1), ("second.md", 2), ("third.md", 4)]
    );
    assert_eq!(document.anchors, anchors(&["A&amp;B"]));
}

#[test]
fn test_default_legacy_anchors_work_across_multiline_fragments() {
    let source =
        "Intro <a\n name=\"Legacy&amp;Anchor\">legacy</a>\n\n<div>\n<a\n id=\"Other&amp;amp;Anchor\"></a>\n</div>\n";
    let document = parse(source, false);
    assert!(document.links.is_empty());
    assert_eq!(document.anchors, anchors(&["Legacy&Anchor", "Other&amp;Anchor"]));
}
