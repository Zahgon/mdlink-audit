//! Configuration file, glob matching and directory exclusion.

use std::path::Path;

use crate::error::AuditError;
use crate::text::decode_utf8_sig;
use crate::toml::{self, Table, Value};

/// Directory names never scanned, wherever they appear.
pub const DEFAULT_IGNORED_DIRS: [&str; 10] = [
    ".git",
    ".venv",
    "venv",
    "node_modules",
    "__pycache__",
    "build",
    "dist",
    ".tox",
    ".pytest_cache",
    ".ruff_cache",
];

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub exclude: Vec<String>,
    pub ignore_links: Vec<String>,
    pub include_html: bool,
    /// Enter symbolic links to directories while walking (`--walk-symlinks`); never read from a file.
    pub walk_symlinks: bool,
    /// `[from, to]` prefix rewrites applied, first match only, to a link target before it is checked.
    pub link_rewrites: Vec<(String, String)>,
    /// The `severity` table (issue code to `error`, `warning` or `off`); `None` when the file has none.
    pub severity: Option<Vec<(String, String)>>,
    /// Report link reference definition findings (`--audit-definitions`); never read from a file.
    pub audit_definitions: bool,
    /// Report images without alternative text (`--require-alt`); never read from a file.
    pub require_alt: bool,
    /// Report anchors defined more than once in a document (`--audit-anchors`); never read from a file.
    pub audit_anchors: bool,
}

/// Every issue code a report can contain.
pub const ISSUE_CODES: [&str; 12] = [
    "case_mismatch",
    "duplicate_anchor",
    "duplicate_definition",
    "invalid_target",
    "missing_alt",
    "missing_anchor",
    "missing_file",
    "normalization_mismatch",
    "not_directory",
    "outside_root",
    "undefined_reference",
    "unused_definition",
];

fn rewrites(value: Option<&Value>) -> Result<Vec<(String, String)>, AuditError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let invalid = || AuditError::new("Configuration 'link_rewrites' must be an array of [from, to] string pairs");
    let Value::Array(pairs) = value else {
        return Err(invalid());
    };
    pairs
        .iter()
        .map(|pair| match pair {
            Value::Array(items) => match items.as_slice() {
                [Value::String(from), Value::String(to)] => Ok((from.clone(), to.clone())),
                _ => Err(invalid()),
            },
            _ => Err(invalid()),
        })
        .collect()
}

fn severity(value: Option<&Value>) -> Result<Option<Vec<(String, String)>>, AuditError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Table(table) = value else {
        return Err(AuditError::new("Configuration 'severity' must be a table"));
    };
    let mut table: Vec<&(String, Value)> = table.iter().collect();
    table.sort_by(|left, right| left.0.cmp(&right.0));
    let mut levels = Vec::new();
    for (code, level) in table {
        if !ISSUE_CODES.contains(&code.as_str()) {
            return Err(AuditError::new(format!(
                "Configuration 'severity' names an unknown issue code: {code}"
            )));
        }
        match level {
            Value::String(level) if matches!(level.as_str(), "error" | "warning" | "off") => {
                levels.push((code.clone(), level.clone()))
            }
            _ => {
                return Err(AuditError::new(
                    "Configuration 'severity' values must be \"error\", \"warning\" or \"off\"",
                ));
            }
        }
    }
    Ok(Some(levels))
}

fn strings(value: Option<&Value>, name: &str) -> Result<Vec<String>, AuditError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let invalid = || AuditError::new(format!("Configuration '{name}' must be an array of strings"));
    let Value::Array(array) = value else {
        return Err(invalid());
    };
    array
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(text.clone()),
            _ => Err(invalid()),
        })
        .collect()
}

/// Read `path`; a missing file is an empty configuration unless `required`.
pub fn load_config(path: &Path, required: bool) -> Result<Config, AuditError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !required {
                return Ok(Config::default());
            }
            return Err(AuditError::new(format!(
                "Configuration does not exist: {}",
                path.display()
            )));
        }
        Err(error) => {
            return Err(AuditError::new(format!(
                "Cannot read configuration {}: {error}",
                path.display()
            )));
        }
    };
    let text = decode_utf8_sig(&bytes)
        .map_err(|error| AuditError::new(format!("Cannot read configuration {}: {error}", path.display())))?;
    // Read as text: every `\r\n` and lone `\r` is a line feed before the TOML reader sees it.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let table: Table =
        toml::parse(&text).map_err(|error| AuditError::new(format!("Invalid TOML in {}: {error}", path.display())))?;
    let mut unknown: Vec<&str> = table
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| {
            !matches!(
                *key,
                "exclude" | "ignore_links" | "include_html" | "link_rewrites" | "severity"
            )
        })
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(AuditError::new(format!(
            "Unknown configuration keys: {}",
            unknown.join(", ")
        )));
    }
    let exclude = strings(toml::get(&table, "exclude"), "exclude")?;
    let ignore_links = strings(toml::get(&table, "ignore_links"), "ignore_links")?;
    let include_html = match toml::get(&table, "include_html") {
        None => false,
        Some(Value::Boolean(flag)) => *flag,
        Some(_) => {
            return Err(AuditError::new(
                "Configuration 'include_html' must be a boolean".to_string(),
            ));
        }
    };
    let link_rewrites = rewrites(toml::get(&table, "link_rewrites"))?;
    let severity = severity(toml::get(&table, "severity"))?;
    Ok(Config {
        exclude,
        ignore_links,
        include_html,
        walk_symlinks: false,
        link_rewrites,
        severity,
        audit_definitions: false,
        require_alt: false,
        audit_anchors: false,
    })
}

fn class_match(pattern: &[char], start: usize, ch: char) -> Option<(bool, usize)> {
    // pattern[start] == '['; returns (matched, index after ']') or None when unterminated.
    let mut index = start + 1;
    let mut negate = false;
    if index < pattern.len() && pattern[index] == '!' {
        negate = true;
        index += 1;
    }
    let first = index;
    let mut matched = false;
    while index < pattern.len() {
        if pattern[index] == ']' && index > first {
            return Some((matched != negate, index + 1));
        }
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            if pattern[index] <= ch && ch <= pattern[index + 2] {
                matched = true;
            }
            index += 3;
        } else {
            if pattern[index] == ch {
                matched = true;
            }
            index += 1;
        }
    }
    None
}

fn glob(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        let step = if p < pattern.len() {
            match pattern[p] {
                '*' => {
                    while p < pattern.len() && pattern[p] == '*' {
                        p += 1;
                    }
                    star = Some((p, t));
                    continue;
                }
                '?' => Some(p + 1),
                '[' => match class_match(pattern, p, text[t]) {
                    Some((true, next)) => Some(next),
                    Some((false, _)) => None,
                    None => (text[t] == '[').then_some(p + 1),
                },
                literal => (literal == text[t]).then_some(p + 1),
            }
        } else {
            None
        };
        match step {
            Some(next) => {
                p = next;
                t += 1;
            }
            None => match star {
                Some((star_p, star_t)) => {
                    p = star_p;
                    t = star_t + 1;
                    star = Some((star_p, star_t + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Case-sensitive shell matching; a `**/` prefix also matches at the repository root.
pub fn matches(value: &str, patterns: &[String]) -> bool {
    let text: Vec<char> = value.chars().collect();
    patterns.iter().any(|pattern| {
        let chars: Vec<char> = pattern.chars().collect();
        glob(&chars, &text) || (pattern.starts_with("**/") && glob(&chars[3..], &text))
    })
}

/// Whether a root-relative path is skipped by the defaults or the configured patterns: the last exclusion
/// pattern that matches decides, and a pattern written `!pattern` re-includes what `pattern` matches.
pub fn excluded(relative: &str, config: &Config, directory: bool) -> bool {
    if relative.split('/').any(|part| DEFAULT_IGNORED_DIRS.contains(&part)) {
        return true;
    }
    let as_directory = format!("{}/", relative.trim_end_matches('/'));
    let mut result = false;
    for pattern in &config.exclude {
        let (negated, body) = match pattern.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, pattern.as_str()),
        };
        let body = [body.to_string()];
        if matches(relative, &body) || (directory && matches(&as_directory, &body)) {
            result = !negated;
        }
    }
    result
}
