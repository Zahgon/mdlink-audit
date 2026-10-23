//! Link and anchor extraction from Markdown, without reading files or the network.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use pulldown_cmark::{BrokenLink, Event, LinkType, Options, Parser, Tag, TagEnd};
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::html::{Token, Tokenizer};
use crate::references::{self, DefinitionIssue};
use crate::text;
use crate::urls::normalize_destination;

/// One Markdown or HTML link with its one-based source line.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub target: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    pub links: Vec<Link>,
    pub anchors: HashSet<String>,
    /// Lines whose links the comment directives switch off.
    pub ignored_lines: HashSet<usize>,
    /// The first line of a `mdlink-audit-disable` region that no `mdlink-audit-enable` closes.
    pub ignored_from: Option<usize>,
    /// What `--audit-definitions` reports for this document.
    pub definition_issues: Vec<DefinitionIssue>,
    /// Images whose alternative text is blank, for `--require-alt`.
    pub images_without_alt: Vec<Link>,
    /// Anchors equal to one defined earlier in the document, for `--audit-anchors`: the later definition's line,
    /// the anchor, and the first definition's line.
    pub duplicate_anchors: Vec<(usize, String, usize)>,
}

impl Document {
    /// Whether a comment directive switches off the links on `line`.
    pub fn ignores(&self, line: usize) -> bool {
        self.ignored_lines.contains(&line) || self.ignored_from.is_some_and(|start| line >= start)
    }
}

const IGNORE_NEXT_LINE: &str = "mdlink-audit-ignore-next-line";
const DISABLE: &str = "mdlink-audit-disable";
const ENABLE: &str = "mdlink-audit-enable";

/// Replace a closed front matter block with as many line breaks as it has lines, and drop a
/// byte order mark.
fn strip_front_matter(source: &str) -> String {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let lines = text::lines_with_ends(source);
    if lines.first().is_some_and(|line| text::strip(line) == "---") {
        for (index, line) in lines.iter().enumerate().skip(1) {
            if matches!(text::strip(line), "---" | "...") {
                return "\n".repeat(index + 1) + &lines[index + 1..].concat();
            }
        }
    }
    source.to_string()
}

/// CommonMark line endings: `\r\n` and `\r` become `\n`, and NUL becomes U+FFFD.
fn normalize_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\0', "\u{fffd}")
}

fn slug_char(ch: char) -> bool {
    if ch == ' ' || ch == '-' {
        return true;
    }
    use GeneralCategory::*;
    matches!(
        get_general_category(ch),
        UppercaseLetter
            | LowercaseLetter
            | TitlecaseLetter
            | ModifierLetter
            | OtherLetter
            | NonspacingMark
            | SpacingMark
            | EnclosingMark
            | DecimalNumber
            | LetterNumber
            | OtherNumber
            | ConnectorPunctuation
    )
}

/// GitHub-style heading slug: ASCII lower-casing, word characters, spaces and hyphens kept,
/// spaces turned into hyphens.
pub fn heading_slug(text: &str) -> String {
    text.chars()
        .map(|ch| ch.to_ascii_lowercase())
        .filter(|&ch| slug_char(ch))
        .map(|ch| if ch == ' ' { '-' } else { ch })
        .collect()
}

/// The id of a heading whose text ends with `{#id}`, the id made of ASCII letters, digits, `_` and `-`.
fn explicit_heading_id(text: &str) -> Option<&str> {
    let start = text.strip_suffix('}')?.rfind("{#")?;
    let id = &text[start + 2..text.len() - 1];
    let valid = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then_some(id)
}

/// What the raw HTML of one document contributes: anchors always come from `<a name>` and
/// `<a id>`; with HTML auditing, every `id` is an anchor and every `href` and `src` a link.
struct HtmlState {
    tokenizer: Tokenizer,
    include_html: bool,
    raw_tag: Option<String>,
    anchors: HashSet<String>,
    /// Every anchor the raw HTML defines, with its tag's line, in tag and attribute order.
    anchor_order: Vec<(usize, String)>,
    links: Vec<Link>,
    comments: Vec<(usize, String)>,
    /// `img` elements without a non-blank `alt`, as their first `src` value and line, when HTML is audited.
    images_without_alt: Vec<Link>,
}

impl HtmlState {
    fn feed(&mut self, fragment: &str, line: usize) {
        for token in self.tokenizer.feed_at(fragment, line) {
            match token {
                Token::Start(tag) => {
                    for (key, value) in &tag.attrs {
                        let Some(value) = value else { continue };
                        let anchor =
                            (tag.name == "a" && (key == "name" || key == "id")) || (self.include_html && key == "id");
                        if anchor && !value.is_empty() {
                            self.anchors.insert(value.clone());
                            self.anchor_order.push((tag.line, value.clone()));
                        }
                        if self.include_html && (key == "href" || key == "src") {
                            self.links.push(Link {
                                target: value.clone(),
                                line: tag.line,
                            });
                        }
                    }
                    let has_alt = tag.attrs.iter().any(|(key, value)| {
                        key == "alt" && value.as_deref().is_some_and(|value| !text::strip(value).is_empty())
                    });
                    if self.include_html && tag.name == "img" && !has_alt {
                        let source = tag.attrs.iter().find(|(key, value)| key == "src" && value.is_some());
                        self.images_without_alt.push(Link {
                            target: source.and_then(|(_, value)| value.clone()).unwrap_or_default(),
                            line: tag.line,
                        });
                    }
                    if tag.name == "script" || tag.name == "style" {
                        self.raw_tag = Some(tag.name);
                    }
                }
                Token::End(name) => {
                    if self.raw_tag.as_deref() == Some(name.as_str()) {
                        self.raw_tag = None;
                    }
                }
                Token::Comment(text, line) => self.comments.push((line, text)),
            }
        }
    }

    /// Markdown inside an open `script` or `style` element is raw text when HTML is audited.
    fn hides_markdown(&self) -> bool {
        self.include_html && self.raw_tag.is_some()
    }
}

/// The anchors of an HTML file: every non-empty `id`, and the non-empty `name` of every `a` element, read with
/// the same tokenizer as raw HTML in Markdown.
pub fn html_anchors(source: &str) -> HashSet<String> {
    let mut html = HtmlState {
        tokenizer: Tokenizer::default(),
        include_html: true,
        raw_tag: None,
        anchors: HashSet::new(),
        anchor_order: Vec::new(),
        links: Vec::new(),
        comments: Vec::new(),
        images_without_alt: Vec::new(),
    };
    html.feed(source, 1);
    html.anchors
}

/// Parse a Markdown document into its links and anchors.
pub fn parse(source: &str, include_html: bool) -> Document {
    let text = normalize_lines(&strip_front_matter(source));
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let line_of = |offset: usize| line_starts.partition_point(|&start| start <= offset);

    let mut html = HtmlState {
        tokenizer: Tokenizer::default(),
        include_html,
        raw_tag: None,
        anchors: HashSet::new(),
        anchor_order: Vec::new(),
        links: Vec::new(),
        comments: Vec::new(),
        images_without_alt: Vec::new(),
    };
    let mut links = Vec::new();
    let mut generated: HashSet<String> = HashSet::new();
    let mut explicit: HashSet<String> = HashSet::new();
    let mut next_suffix: HashMap<String, usize> = HashMap::new();
    let mut heading: Option<(String, bool)> = None;
    let mut heading_line = 0usize;
    // Heading anchors with the heading's first line, in document order.
    let mut heading_anchors: Vec<(usize, String)> = Vec::new();
    let mut image_depth = 0usize;
    // The outermost image being read and its alternative text so far.
    let mut alt: Option<(Link, String)> = None;
    let mut images_without_alt = Vec::new();

    // Unresolved references, as (start, kind, label); the callback leaves them unresolved.
    let broken: RefCell<Vec<(usize, LinkType, String)>> = RefCell::new(Vec::new());
    let record = |link: BrokenLink| {
        broken
            .borrow_mut()
            .push((link.span.start, link.link_type, link.reference.to_string()));
        None
    };
    let mut used: HashSet<String> = HashSet::new();
    let mut images: Vec<Range<usize>> = Vec::new();

    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for (event, range) in Parser::new_with_broken_link_callback(&text, options, Some(record)).into_offset_iter() {
        if let Event::Start(Tag::Link { link_type, id, .. } | Tag::Image { link_type, id, .. }) = &event
            && matches!(
                link_type,
                LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
            )
        {
            used.insert(references::label_key(id));
        }
        if let Event::Start(Tag::Image { .. }) = &event {
            images.push(range.clone());
        }
        match event {
            Event::Start(Tag::Heading { .. }) => {
                heading = Some((String::new(), html.hides_markdown()));
                heading_line = line_of(range.start);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((content, false)) = heading.take() {
                    if let Some(id) = explicit_heading_id(&content) {
                        explicit.insert(id.to_string());
                        heading_anchors.push((heading_line, id.to_string()));
                        continue;
                    }
                    let base = heading_slug(&content);
                    let mut slug = base.clone();
                    let mut suffix = next_suffix.get(&base).copied().unwrap_or(0);
                    while generated.contains(&slug) {
                        suffix += 1;
                        slug = format!("{base}-{suffix}");
                    }
                    next_suffix.insert(base, suffix);
                    heading_anchors.push((heading_line, slug.clone()));
                    generated.insert(slug);
                }
            }
            Event::Start(Tag::Link {
                link_type, dest_url, ..
            }) => {
                if image_depth == 0 && !html.hides_markdown() {
                    let raw = if link_type == LinkType::Email {
                        format!("mailto:{dest_url}")
                    } else {
                        dest_url.to_string()
                    };
                    links.push(Link {
                        target: normalize_destination(&raw),
                        line: line_of(range.start),
                    });
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if image_depth == 0 && !html.hides_markdown() {
                    let link = Link {
                        target: normalize_destination(&dest_url),
                        line: line_of(range.start),
                    };
                    alt = Some((link.clone(), String::new()));
                    links.push(link);
                }
                image_depth += 1;
            }
            Event::End(TagEnd::Image) => {
                image_depth = image_depth.saturating_sub(1);
                if image_depth == 0
                    && let Some((image, text)) = alt.take()
                    && text::strip(&text).is_empty()
                {
                    images_without_alt.push(image);
                }
            }
            // Alternative text is the description's plain text: code spans, hard breaks and raw HTML add nothing.
            Event::Text(content) => {
                if let Some((_, buffer)) = alt.as_mut() {
                    buffer.push_str(&content);
                }
                if image_depth == 0
                    && let Some((buffer, _)) = heading.as_mut()
                {
                    buffer.push_str(&content);
                }
            }
            Event::Code(content) => {
                if image_depth == 0
                    && let Some((buffer, _)) = heading.as_mut()
                {
                    buffer.push_str(&content);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let (Some((_, buffer)), Event::SoftBreak) = (alt.as_mut(), &event) {
                    buffer.push('\n');
                }
                if image_depth == 0
                    && let Some((buffer, _)) = heading.as_mut()
                {
                    buffer.push('\n');
                }
            }
            // HTML links join the Markdown links as each tag completes, so the list stays in document order.
            Event::Html(fragment) => {
                html.feed(&fragment, line_of(range.start));
                links.append(&mut html.links);
            }
            Event::InlineHtml(fragment) if image_depth == 0 => {
                html.feed(&fragment, line_of(range.start));
                links.append(&mut html.links);
            }
            _ => {}
        }
    }
    let duplicate_anchors = duplicates(heading_anchors, std::mem::take(&mut html.anchor_order));
    images_without_alt.append(&mut html.images_without_alt);
    let mut anchors = html.anchors;
    anchors.extend(generated);
    anchors.extend(explicit);
    let (ignored_lines, ignored_from) = directives(&html.comments);
    // Full and collapsed references only, each opening bracket once, none inside an image description.
    let mut seen = HashSet::new();
    let undefined: Vec<(usize, String)> = broken
        .into_inner()
        .into_iter()
        .filter(|(start, kind, _)| matches!(kind, LinkType::Reference | LinkType::Collapsed) && seen.insert(*start))
        .filter(|(start, _, _)| !images.iter().any(|image| image.start < *start && *start < image.end))
        .map(|(start, _, label)| (line_of(start), label))
        .collect();
    Document {
        links,
        anchors,
        ignored_lines,
        ignored_from,
        definition_issues: references::audit(&text, &used, &undefined),
        images_without_alt,
        duplicate_anchors,
    }
}

/// Anchors defined again later in the document, ordered by line: on one line the heading's own anchor comes before
/// the anchors of the raw HTML in it, which keep their tag and attribute order.
fn duplicates(headings: Vec<(usize, String)>, html: Vec<(usize, String)>) -> Vec<(usize, String, usize)> {
    let mut order: Vec<(usize, u8, String)> = headings
        .into_iter()
        .map(|(line, anchor)| (line, 0, anchor))
        .chain(html.into_iter().map(|(line, anchor)| (line, 1, anchor)))
        .collect();
    order.sort_by_key(|(line, rank, _)| (*line, *rank));
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut found = Vec::new();
    for (line, _, anchor) in order {
        match first.get(&anchor) {
            Some(&earlier) => found.push((line, anchor, earlier)),
            None => {
                first.insert(anchor, line);
            }
        }
    }
    found
}

/// Lines switched off by the comment directives, in document order: `mdlink-audit-ignore-next-line` switches off
/// the line after the comment's last line; `mdlink-audit-disable` switches off every line after its last line up to
/// the line before the first line of the next `mdlink-audit-enable`, or to the end of the document.
fn directives(comments: &[(usize, String)]) -> (HashSet<usize>, Option<usize>) {
    let mut ignored = HashSet::new();
    let mut disabled_from: Option<usize> = None;
    for (start, text) in comments {
        let end = start + text.matches('\n').count();
        match text::strip(text) {
            IGNORE_NEXT_LINE => {
                ignored.insert(end + 1);
            }
            DISABLE if disabled_from.is_none() => disabled_from = Some(end + 1),
            ENABLE => {
                if let Some(from) = disabled_from.take() {
                    ignored.extend(from..*start);
                }
            }
            _ => {}
        }
    }
    (ignored, disabled_from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(document: &Document) -> Vec<(&str, usize)> {
        let mut out: Vec<_> = document
            .links
            .iter()
            .map(|link| (link.target.as_str(), link.line))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn slugs_keep_unicode_letters_and_number_repeats() {
        let document = parse("# Café &amp; Co\n\n# Café & Co\n\n## Θ `x.y`\n", false);
        assert!(document.anchors.contains("café--co"));
        assert!(document.anchors.contains("café--co-1"));
        assert!(document.anchors.contains("Θ-xy"));
    }

    #[test]
    fn front_matter_counts_unicode_line_breaks() {
        let document = parse("---\na\x0cb\n---\n[x](y.md)\n", false);
        assert_eq!(targets(&document), vec![("y.md", 5)]);
    }

    #[test]
    fn html_links_are_opt_in_and_raw_text_hides_markdown() {
        let source =
            "<a name=\"n\" href=\"a.md\">x</a>\n\nPrefix <script>\n\n[hidden](h.md)\n\n</script> [after](b.md)\n";
        let off = parse(source, false);
        assert_eq!(targets(&off), vec![("b.md", 7), ("h.md", 5)]);
        assert!(off.anchors.contains("n"));
        let on = parse(source, true);
        assert_eq!(targets(&on), vec![("a.md", 1), ("b.md", 7)]);
    }
}
