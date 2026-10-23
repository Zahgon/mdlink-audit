//! `--audit-definitions`: link reference definitions no reference uses, definitions that repeat an earlier label,
//! and full or collapsed references whose label matches no definition.

use std::collections::{HashMap, HashSet};

use markdown::mdast::Node;
use markdown::{Constructs, ParseOptions};

use crate::text::is_space;

/// One finding of `--audit-definitions`, reported at `line` with the label as its target.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionIssue {
    pub line: usize,
    pub label: String,
    pub code: &'static str,
    pub message: String,
}

/// The label as reports print it: surrounding whitespace removed and inner whitespace runs collapsed to one space.
pub fn label_text(label: &str) -> String {
    label
        .split(is_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The key two labels match by: the printed label, lower-cased and then upper-cased.
pub fn label_key(label: &str) -> String {
    label_text(label).to_lowercase().to_uppercase()
}

/// Every link reference definition of `text` in document order, as (line, label as written).
fn definitions(text: &str) -> Vec<(usize, String)> {
    let options = ParseOptions {
        constructs: Constructs {
            gfm_table: true,
            gfm_strikethrough: true,
            ..Constructs::default()
        },
        gfm_strikethrough_single_tilde: false,
        ..ParseOptions::default()
    };
    let mut found = Vec::new();
    // Plain CommonMark (no MDX) never fails to parse; an error leaves no definitions.
    if let Ok(tree) = markdown::to_mdast(text, &options) {
        collect(text, &tree, &mut found);
    }
    found
}

fn collect(text: &str, node: &Node, found: &mut Vec<(usize, String)>) {
    if let Node::Definition(definition) = node
        && let Some(position) = &definition.position
    {
        // The position may start at the indentation before the opening bracket.
        found.push((
            position.start.line,
            raw_label(text[position.start.offset..].trim_start_matches([' ', '\t'])),
        ));
    }
    for child in node.children().into_iter().flatten() {
        collect(text, child, found);
    }
}

/// The label of the definition that `source` starts with, as written: backslash escapes kept, and on each later line
/// the block quote markers and indentation of the definition's containers left out.
fn raw_label(source: &str) -> String {
    let mut label = String::new();
    let mut chars = source.chars().skip(1);
    while let Some(ch) = chars.next() {
        match ch {
            ']' => break,
            '\\' => {
                label.push(ch);
                if let Some(next) = chars.next() {
                    label.push(next);
                }
            }
            '\n' => {
                label.push(ch);
                let rest: String = chars.collect();
                let trimmed = rest.trim_start_matches([' ', '\t', '>']);
                return label + &raw_label(&format!("[{trimmed}"));
            }
            _ => label.push(ch),
        }
    }
    label
}

/// The findings for one document: `used` holds the keys of the labels its resolved references use, `undefined`
/// the line and label of each full or collapsed reference that matched no definition.
pub fn audit(text: &str, used: &HashSet<String>, undefined: &[(usize, String)]) -> Vec<DefinitionIssue> {
    let mut issues = Vec::new();
    let mut first_lines: HashMap<String, usize> = HashMap::new();
    for (line, label) in definitions(text) {
        let key = label_key(&label);
        if let Some(first) = first_lines.get(&key) {
            issues.push(DefinitionIssue {
                line,
                label: label_text(&label),
                code: "duplicate_definition",
                message: format!("Duplicate link reference definition; the first is on line {first}"),
            });
        } else {
            first_lines.insert(key.clone(), line);
            if !used.contains(&key) {
                issues.push(DefinitionIssue {
                    line,
                    label: label_text(&label),
                    code: "unused_definition",
                    message: "Link reference definition is never used".to_string(),
                });
            }
        }
    }
    for (line, label) in undefined {
        issues.push(DefinitionIssue {
            line: *line,
            label: label_text(label),
            code: "undefined_reference",
            message: "No link reference definition matches this label".to_string(),
        });
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_collapse_whitespace_and_fold_case() {
        assert_eq!(label_text("  Foo \t\n bar "), "Foo bar");
        assert_eq!(label_key("straße"), label_key("STRASSE"));
        assert_eq!(label_key("ΣΑΣ"), label_key("σας"));
    }

    #[test]
    fn definitions_inside_containers_and_not_in_code() {
        let found = definitions("> [a]: x\n\n- [b]:\n  y\n\ntext\n\n    [c]: z\n\n```\n[d]: w\n```\n");
        assert_eq!(found, vec![(1, "a".to_string()), (3, "b".to_string())]);
    }
}
