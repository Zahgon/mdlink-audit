//! `--fix`: rewrite the target of every fixable issue to its suggestion, in place.
//!
//! An issue is fixable when its code is `case_mismatch` or `normalization_mismatch` and it has a
//! suggestion. On the issue's line the first occurrence of the target that is preceded by one of
//! `( < " ' =`, a space or a tab, and followed by one of `) > " '`, whitespace or the end of the
//! line, is replaced; every other byte of the file stays as it is.

use std::path::Path;

use crate::checker::Report;
use crate::error::AuditError;

const FIX_CODES: [&str; 2] = ["case_mismatch", "normalization_mismatch"];
const BEFORE: [char; 7] = ['(', '<', '"', '\'', '=', ' ', '\t'];
const AFTER: [char; 8] = [')', '>', '"', '\'', ' ', '\t', '\r', '\n'];

/// Lines of `text`, each ending after its `\r\n`, `\r` or `\n`.
fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        current.push(ch);
        if ch == '\n' || (ch == '\r' && chars.peek() != Some(&'\n')) {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Replace the first delimited occurrence of `target` in `line`; false when there is none.
fn replace_in_line(line: &mut String, target: &str, suggestion: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = line[from..].find(target) {
        let start = from + offset;
        let end = start + target.len();
        let before = line[..start].chars().next_back();
        let after = line[end..].chars().next();
        if before.is_some_and(|c| BEFORE.contains(&c)) && after.is_none_or(|c| AFTER.contains(&c)) {
            line.replace_range(start..end, suggestion);
            return true;
        }
        from = start + line[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// Apply every fixable issue of `report` to its source file under the report's root.
pub fn apply(report: &Report) -> Result<(), AuditError> {
    let mut files: Vec<&str> = Vec::new();
    for issue in &report.issues {
        if FIX_CODES.contains(&issue.code) && issue.suggestion.is_some() && !files.contains(&issue.file.as_str()) {
            files.push(&issue.file);
        }
    }
    for file in files {
        let path = Path::new(&report.root).join(file);
        let original = std::fs::read_to_string(&path)
            .map_err(|error| AuditError::new(format!("Cannot fix {}: {error}", path.display())))?;
        let mut lines = split_lines(&original);
        for issue in report
            .issues
            .iter()
            .filter(|issue| issue.file == file && FIX_CODES.contains(&issue.code))
        {
            let (Some(suggestion), Some(line)) = (
                &issue.suggestion,
                issue.line.checked_sub(1).and_then(|i| lines.get_mut(i)),
            ) else {
                continue;
            };
            replace_in_line(line, &issue.target, suggestion);
        }
        let fixed = lines.concat();
        if fixed != original {
            std::fs::write(&path, fixed)
                .map_err(|error| AuditError::new(format!("Cannot fix {}: {error}", path.display())))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_every_line_break_kind() {
        assert_eq!(split_lines("a\r\nb\rc\nd"), vec!["a\r\n", "b\r", "c\n", "d"]);
        assert!(split_lines("").is_empty());
    }

    #[test]
    fn replaces_only_a_delimited_destination() {
        let mut line = "[docs/A.md](docs/A.md)\n".to_string();
        assert!(replace_in_line(&mut line, "docs/A.md", "docs/a.md"));
        assert_eq!(line, "[docs/A.md](docs/a.md)\n");
        let mut plain = "see docs/A.md.\n".to_string();
        assert!(!replace_in_line(&mut plain, "docs/A.md", "docs/a.md"));
    }
}
