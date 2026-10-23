//! Resolve local links without network requests or repository code execution.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Config, excluded, matches};
use crate::error::AuditError;
use crate::markdown::{Document, Link, html_anchors, parse};
use crate::paths::{absolute, relative_from, resolve};
use crate::similarity::close_matches;
use crate::text::decode_utf8_sig;
use crate::urls::{SplitUrl, quote, split, unquote_lossy, unquote_strict, unsplit};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug)]
pub struct Issue {
    pub file: String,
    pub line: usize,
    pub target: String,
    pub code: &'static str,
    pub message: String,
    pub suggestion: Option<String>,
    /// `error` or `warning` when the configuration has a `severity` table; `None` otherwise.
    pub severity: Option<&'static str>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub root: String,
    pub files_scanned: usize,
    pub links_checked: usize,
    pub links_skipped: usize,
    pub skipped_external: usize,
    pub skipped_ignored: usize,
    pub issues: Vec<Issue>,
    /// Issues suppressed by `--baseline`; `None` without that option.
    pub baselined: Option<usize>,
}

impl Report {
    /// Whether an issue that is not a warning remains: such a run fails (exit 1, `FAIL`).
    pub fn failing(&self) -> bool {
        self.issues.iter().any(|issue| issue.severity != Some("warning"))
    }
}

fn is_html(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_lowercase().as_str(), "html" | "htm"))
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_lowercase().as_str(), "md" | "markdown"))
}

fn has_scheme(target: &str) -> bool {
    let mut chars = target.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    for ch in chars {
        if ch == ':' {
            return true;
        }
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

/// A symbolic link, or on Windows a directory junction (a mount-point reparse point): the walk
/// never follows either on its own.
fn linked_directory(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// The nearest ancestor of `start` holding `.git`, or `start` itself.
pub fn discover_root(start: &Path) -> PathBuf {
    let start = resolve(start);
    for candidate in start.ancestors() {
        if candidate.join(".git").exists() {
            return candidate.to_path_buf();
        }
    }
    start
}

fn relative(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".to_string(),
        Ok(rest) => rest.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

fn casefold(text: &str) -> String {
    caseless::default_case_fold_str(text)
}

fn nfc(text: &str) -> String {
    text.nfc().collect()
}

enum Status {
    Ok,
    CaseMismatch,
    FormMismatch,
    Missing,
    Outside,
}

type Problem = (&'static str, String, Option<String>);

pub struct Auditor {
    root: PathBuf,
    config: Config,
    documents: HashMap<PathBuf, Document>,
    html: HashMap<PathBuf, HashSet<String>>,
    listings: HashMap<PathBuf, BTreeSet<String>>,
}

impl Auditor {
    pub fn new(root: &Path, config: Config) -> Self {
        Auditor {
            root: resolve(root),
            config,
            documents: HashMap::new(),
            html: HashMap::new(),
            listings: HashMap::new(),
        }
    }

    fn inside(&self, path: &Path) -> bool {
        resolve(path).starts_with(&self.root)
    }

    fn excluded(&self, path: &Path, directory: bool) -> bool {
        excluded(&relative(path, &self.root), &self.config, directory)
    }

    /// The Markdown files that the explicit `inputs` select, sorted by root-relative path.
    pub fn collect_inputs(&self, inputs: &[PathBuf]) -> Result<Vec<PathBuf>, AuditError> {
        self.collect(inputs)
    }

    fn collect(&self, inputs: &[PathBuf]) -> Result<Vec<PathBuf>, AuditError> {
        let mut files: Vec<PathBuf> = Vec::new();
        for supplied in inputs {
            let path = absolute(supplied);
            let shown = supplied.to_string_lossy();
            if !path.starts_with(&self.root) || !self.inside(&path) {
                return Err(AuditError::new(format!(
                    "Input is outside the repository root: {shown}"
                )));
            }
            if !path.exists() {
                return Err(AuditError::new(format!("Input does not exist: {shown}")));
            }
            if self.excluded(&path, path.is_dir()) {
                continue;
            }
            if path.is_file() {
                if !is_markdown(&path) {
                    return Err(AuditError::new(format!(
                        "Input is not a Markdown file (.md or .markdown): {shown}"
                    )));
                }
                files.push(path);
                continue;
            }
            if !path.is_dir() {
                return Err(AuditError::new(format!(
                    "Input is not a regular file or directory: {shown}"
                )));
            }
            let start: HashSet<PathBuf> = [resolve(&path)].into_iter().collect();
            self.walk(&path, &start, &mut files)?;
        }
        let mut keyed: Vec<(String, PathBuf)> = files
            .into_iter()
            .map(|file| (relative(&file, &self.root), file))
            .collect();
        keyed.sort();
        keyed.dedup_by(|a, b| a.1 == b.1);
        Ok(keyed.into_iter().map(|(_, file)| file).collect())
    }

    /// Top-down walk: a directory's Markdown files are taken before any of its subdirectories.
    /// Linked directories are entered only with `walk_symlinks`, and never when they resolve to a
    /// directory already on the walk path (`walked` holds the resolved paths of that path).
    fn walk(&self, directory: &Path, walked: &HashSet<PathBuf>, files: &mut Vec<PathBuf>) -> Result<(), AuditError> {
        let entries =
            fs::read_dir(directory).map_err(|error| AuditError::new(format!("Cannot scan directory: {error}")))?;
        let mut subdirectories = Vec::new();
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| AuditError::new(format!("Cannot scan directory: {error}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() {
                subdirectories.push(name)
            } else {
                names.push(name)
            }
        }
        let mut kept = Vec::new();
        subdirectories.sort();
        for name in subdirectories {
            let path = directory.join(&name);
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| AuditError::new(format!("Cannot inspect directory {}: {error}", path.display())))?;
            let real = resolve(&path);
            if linked_directory(&metadata) && (!self.config.walk_symlinks || walked.contains(&real)) {
                continue;
            }
            if self.inside(&path) && !self.excluded(&path, true) {
                let mut lineage = walked.clone();
                lineage.insert(real);
                kept.push((name, lineage));
            }
        }
        names.sort();
        for name in names {
            let file = directory.join(&name);
            if !is_markdown(&file) || self.excluded(&file, false) {
                continue;
            }
            if !self.inside(&file) {
                return Err(AuditError::new(format!(
                    "Markdown source resolves outside the repository root: {}",
                    file.display()
                )));
            }
            if !file.is_file() {
                return Err(AuditError::new(format!(
                    "Markdown source is not a regular file: {}",
                    file.display()
                )));
            }
            files.push(file);
        }
        for (name, lineage) in kept {
            self.walk(&directory.join(name), &lineage, files)?;
        }
        Ok(())
    }

    fn document(&mut self, file: &Path) -> Result<&Document, AuditError> {
        if !self.documents.contains_key(file) {
            if !self.inside(file) {
                return Err(AuditError::new(format!(
                    "Refusing to read outside the repository root: {}",
                    file.display()
                )));
            }
            let unreadable = |error: &dyn std::fmt::Display| {
                AuditError::new(format!("Cannot read Markdown file {}: {error}", file.display()))
            };
            let bytes = fs::read(file).map_err(|error| unreadable(&error))?;
            let text = decode_utf8_sig(&bytes).map_err(|error| unreadable(&error))?;
            self.documents
                .insert(file.to_path_buf(), parse(&text, self.config.include_html));
        }
        Ok(&self.documents[file])
    }

    fn html_document(&mut self, file: &Path) -> Result<&HashSet<String>, AuditError> {
        if !self.html.contains_key(file) {
            if !self.inside(file) {
                return Err(AuditError::new(format!(
                    "Refusing to read outside the repository root: {}",
                    file.display()
                )));
            }
            let unreadable = |error: &dyn std::fmt::Display| {
                AuditError::new(format!("Cannot read HTML file {}: {error}", file.display()))
            };
            let bytes = fs::read(file).map_err(|error| unreadable(&error))?;
            let text = decode_utf8_sig(&bytes).map_err(|error| unreadable(&error))?;
            self.html.insert(file.to_path_buf(), html_anchors(&text));
        }
        Ok(&self.html[file])
    }

    fn listing(&mut self, directory: &Path) -> Result<&BTreeSet<String>, AuditError> {
        if !self.listings.contains_key(directory) {
            let entries = fs::read_dir(directory).map_err(|error| {
                AuditError::new(format!("Cannot inspect directory {}: {error}", directory.display()))
            })?;
            let names = entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            self.listings.insert(directory.to_path_buf(), names);
        }
        Ok(&self.listings[directory])
    }

    /// Walk `target` below the root one component at a time, taking each component as spelled or,
    /// failing that, the one entry whose case-folded name equals its case-folded spelling.
    fn status(&mut self, target: &Path) -> Result<(PathBuf, Status), AuditError> {
        let mut current = self.root.clone();
        let mut wrong_case = false;
        let mut wrong_form = false;
        let parts: Vec<String> = target
            .strip_prefix(&self.root)
            .map(|rest| {
                rest.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        for part in parts {
            if !self.inside(&current) {
                return Ok((current, Status::Outside));
            }
            if !current.is_dir() {
                return Ok((target.to_path_buf(), Status::Missing));
            }
            let names = self.listing(&current)?;
            if names.contains(&part) {
                current.push(&part);
                continue;
            }
            let form = nfc(&part);
            let same_form: Vec<&String> = names.iter().filter(|name| nfc(name) == form).collect();
            if same_form.len() == 1 {
                current.push(same_form[0]);
                wrong_form = true;
                continue;
            }
            let folded = casefold(&part);
            let similar: Vec<&String> = names.iter().filter(|name| casefold(name) == folded).collect();
            if similar.len() != 1 {
                return Ok((target.to_path_buf(), Status::Missing));
            }
            current.push(similar[0]);
            wrong_case = true;
        }
        if !self.inside(&current) {
            return Ok((current, Status::Outside));
        }
        if !current.exists() {
            return Ok((current, Status::Missing));
        }
        let status = if wrong_case {
            Status::CaseMismatch
        } else if wrong_form {
            Status::FormMismatch
        } else {
            Status::Ok
        };
        Ok((current, status))
    }

    fn replacement(&self, actual: &Path, source: &Path, url: &SplitUrl) -> String {
        let mut path = if url.path.starts_with('/') {
            format!("/{}", relative(actual, &self.root))
        } else {
            relative_from(actual, source.parent().unwrap_or(&self.root))
        };
        if url.path.ends_with('/') && actual.is_dir() {
            path.push('/');
        }
        let mut replaced = url.clone();
        replaced.path = quote(&path, "/");
        unsplit(&replaced)
    }

    fn suggest_path(&mut self, target: &Path, source: &Path, url: &SplitUrl) -> Result<Option<String>, AuditError> {
        let parent_target = target.parent().unwrap_or(&self.root).to_path_buf();
        let (parent, status) = self.status(&parent_target)?;
        if matches!(status, Status::Missing | Status::Outside) || !parent.is_dir() {
            return Ok(None);
        }
        let names: Vec<String> = match self.listing(&parent) {
            Ok(names) => names.iter().cloned().collect(),
            Err(_) => return Ok(None),
        };
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for candidate in close_matches(&name, &names, 3, 0.72) {
            let path = parent.join(&candidate);
            if self.inside(&path) && path.is_file() {
                return Ok(Some(self.replacement(&path, source, url)));
            }
        }
        Ok(None)
    }

    fn lexical(&self, source: &Path, url_path: &str) -> Option<PathBuf> {
        let mut parts: Vec<String> = if url_path.starts_with('/') {
            Vec::new()
        } else {
            source
                .parent()
                .and_then(|parent| parent.strip_prefix(&self.root).ok())
                .map(|rest| {
                    rest.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        for part in url_path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                other => parts.push(other.to_string()),
            }
        }
        let mut path = self.root.clone();
        path.extend(parts);
        Some(path)
    }

    fn check(&mut self, source: &Path, link: &Link) -> Result<Option<Problem>, AuditError> {
        let url = split(&link.target);
        let (Some(url_path), Some(fragment)) = (unquote_strict(&url.path), unquote_strict(&url.fragment)) else {
            return Ok(Some((
                "invalid_target",
                "Link contains an invalid URL or UTF-8 escape".into(),
                None,
            )));
        };
        let drive = |part: &str| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next() == Some(':')
        };
        if url_path.contains('\\') || url_path.contains('\0') || url_path.split('/').any(drive) {
            return Ok(Some((
                "invalid_target",
                "Local links must use repository paths with forward slashes".into(),
                None,
            )));
        }
        let target = if url_path.is_empty() {
            source.to_path_buf()
        } else {
            match self.lexical(source, &url_path) {
                Some(target) => target,
                None => {
                    return Ok(Some(("outside_root", "Link escapes the repository root".into(), None)));
                }
            }
        };
        let (actual, status) = self.status(&target)?;
        match status {
            Status::Outside => {
                return Ok(Some((
                    "outside_root",
                    "Link resolves outside the repository root".into(),
                    None,
                )));
            }
            Status::Missing => {
                let message = format!("Target does not exist: {}", relative(&target, &self.root));
                let suggestion = self.suggest_path(&target, source, &url)?;
                return Ok(Some(("missing_file", message, suggestion)));
            }
            Status::CaseMismatch => {
                let message = format!("Path casing differs; on disk: {}", relative(&actual, &self.root));
                return Ok(Some((
                    "case_mismatch",
                    message,
                    Some(self.replacement(&actual, source, &url)),
                )));
            }
            Status::FormMismatch => {
                let message = format!("Path normalization differs; on disk: {}", relative(&actual, &self.root));
                return Ok(Some((
                    "normalization_mismatch",
                    message,
                    Some(self.replacement(&actual, source, &url)),
                )));
            }
            Status::Ok => {}
        }
        if url_path.ends_with('/') && !actual.is_dir() {
            return Ok(Some((
                "not_directory",
                "A path ending in '/' must refer to a directory".into(),
                None,
            )));
        }
        if !fragment.is_empty() && actual.is_file() && (is_markdown(&actual) || is_html(&actual)) {
            let anchors = if is_html(&actual) {
                self.html_document(&actual)?
            } else {
                &self.document(&actual)?.anchors
            };
            if !anchors.contains(&fragment) {
                let mut sorted: Vec<String> = anchors.iter().cloned().collect();
                sorted.sort();
                let suggestion = close_matches(&fragment, &sorted, 1, 0.6)
                    .into_iter()
                    .next()
                    .map(|closest| {
                        let mut replaced = url.clone();
                        replaced.fragment = quote(&closest, "-_");
                        unsplit(&replaced)
                    });
                let message = format!("Heading or custom anchor '#{fragment}' does not exist");
                return Ok(Some(("missing_anchor", message, suggestion)));
            }
        }
        Ok(None)
    }

    pub fn run(&mut self, inputs: Option<Vec<PathBuf>>) -> Result<Report, AuditError> {
        if !self.root.is_dir() {
            return Err(AuditError::new(format!(
                "Repository root is not a directory: {}",
                self.root.display()
            )));
        }
        self.documents.clear();
        self.html.clear();
        self.listings.clear();
        let mut report = Report {
            root: self.root.to_string_lossy().into_owned(),
            ..Report::default()
        };
        let files = self.collect(&inputs.unwrap_or_else(|| vec![self.root.clone()]))?;
        report.files_scanned = files.len();
        for source in files {
            let document = self.document(&source)?;
            let links = document.links.clone();
            let ignored: Vec<bool> = links.iter().map(|link| document.ignores(link.line)).collect();
            for (link, ignored) in links.into_iter().zip(ignored) {
                let ignore = &self.config.ignore_links;
                if ignored || matches(&link.target, ignore) || matches(&unquote_lossy(&link.target), ignore) {
                    report.links_skipped += 1;
                    report.skipped_ignored += 1;
                    continue;
                }
                if has_scheme(&link.target) || link.target.starts_with("//") {
                    report.links_skipped += 1;
                    report.skipped_external += 1;
                    continue;
                }
                report.links_checked += 1;
                let rewritten = self.config.link_rewrites.iter().find_map(|(from, to)| {
                    link.target.strip_prefix(from.as_str()).map(|rest| Link {
                        target: format!("{to}{rest}"),
                        ..link.clone()
                    })
                });
                if let Some((code, message, suggestion)) = self.check(&source, rewritten.as_ref().unwrap_or(&link))? {
                    report.issues.push(Issue {
                        file: relative(&source, &self.root),
                        line: link.line,
                        target: link.target.clone(),
                        code,
                        message,
                        suggestion,
                        severity: None,
                    });
                }
            }
            if self.config.audit_definitions {
                let file = relative(&source, &self.root);
                let document = self.document(&source)?;
                report
                    .issues
                    .extend(document.definition_issues.iter().map(|found| Issue {
                        file: file.clone(),
                        line: found.line,
                        target: found.label.clone(),
                        code: found.code,
                        message: found.message.clone(),
                        suggestion: None,
                        severity: None,
                    }));
            }
            if self.config.require_alt {
                let file = relative(&source, &self.root);
                let document = self.document(&source)?;
                report
                    .issues
                    .extend(document.images_without_alt.iter().map(|image| Issue {
                        file: file.clone(),
                        line: image.line,
                        target: image.target.clone(),
                        code: "missing_alt",
                        message: "Image has no alternative text".to_string(),
                        suggestion: None,
                        severity: None,
                    }));
            }
            if self.config.audit_anchors {
                let file = relative(&source, &self.root);
                let document = self.document(&source)?;
                report
                    .issues
                    .extend(document.duplicate_anchors.iter().map(|(line, anchor, first)| Issue {
                        file: file.clone(),
                        line: *line,
                        target: format!("#{anchor}"),
                        code: "duplicate_anchor",
                        message: format!("Anchor already defined on line {first}"),
                        suggestion: None,
                        severity: None,
                    }));
            }
        }
        report
            .issues
            .sort_by(|a, b| (&a.file, a.line, &a.target, a.code).cmp(&(&b.file, b.line, &b.target, b.code)));
        if let Some(levels) = &self.config.severity {
            let level_of = |code: &str| {
                levels
                    .iter()
                    .find(|(name, _)| name == code)
                    .map_or("error", |(_, level)| match level.as_str() {
                        "warning" => "warning",
                        "off" => "off",
                        _ => "error",
                    })
            };
            report.issues.retain(|issue| level_of(issue.code) != "off");
            for issue in &mut report.issues {
                issue.severity = Some(level_of(issue.code));
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::similarity::close_matches;

    #[test]
    fn case_folding_is_full() {
        assert_eq!(casefold("STRAẞE"), casefold("strasse"));
        assert_eq!(casefold("ΣΟΦΟΣ"), casefold("σοφος"));
        assert_eq!(casefold("ﬁle"), casefold("FILE"));
    }

    #[test]
    fn suggestions_rank_by_similarity() {
        let names = vec!["Guide.md".to_string(), "guide.md".to_string(), "install.md".to_string()];
        assert_eq!(
            close_matches("guied.md", &names, 3, 0.72),
            vec!["guide.md".to_string(), "Guide.md".to_string()]
        );
        assert_eq!(
            close_matches("instal", &["install".to_string()], 1, 0.6),
            vec!["install".to_string()]
        );
    }
}
