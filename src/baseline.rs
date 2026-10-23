//! `--baseline FILE`: issues recorded in an earlier `--format json` report are suppressed.
//!
//! The file is read as standard JSON plus the `NaN`, `Infinity` and `-Infinity` literals, integers of any
//! size, lone `\u` surrogates allowed in strings, and the last of duplicate object keys winning; a syntax
//! error is reported with its message, line, column and character position.

use std::collections::HashSet;
use std::path::Path;

use crate::checker::Issue;
use crate::error::AuditError;
use crate::text::decode_utf8;

/// A parsed JSON value. Strings holding a lone surrogate cannot equal any text, so they are `None`.
#[derive(Debug)]
enum Json {
    Null,
    Bool,
    Int(String),
    Float,
    Str(Option<String>),
    Array(Vec<Json>),
    Object(Vec<(Option<String>, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs
                .iter()
                .rev()
                .find(|(name, _)| name.as_deref() == Some(key))
                .map(|(_, value)| value),
            _ => None,
        }
    }
}

/// A syntax error: the reader's message and the character position it refers to.
type Failure = (&'static str, usize);

struct Reader {
    chars: Vec<char>,
}

fn is_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

impl Reader {
    fn at(&self, pos: usize) -> Option<char> {
        self.chars.get(pos).copied()
    }

    fn skip_whitespace(&self, mut pos: usize) -> usize {
        while self.at(pos).is_some_and(is_whitespace) {
            pos += 1;
        }
        pos
    }

    fn follows(&self, pos: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(offset, ch)| self.at(pos + offset) == Some(ch))
    }

    /// One value starting exactly at `pos`; returns it and the position after it.
    fn value(&self, pos: usize) -> Result<(Json, usize), Failure> {
        let Some(ch) = self.at(pos) else {
            return Err(("Expecting value", pos));
        };
        let constants: [(&str, Json); 6] = [
            ("null", Json::Null),
            ("true", Json::Bool),
            ("false", Json::Bool),
            ("NaN", Json::Float),
            ("Infinity", Json::Float),
            ("-Infinity", Json::Float),
        ];
        match ch {
            '"' => return self.string(pos + 1).map(|(text, end)| (Json::Str(text), end)),
            '{' => return self.object(pos + 1),
            '[' => return self.array(pos + 1),
            _ => {}
        }
        for (word, value) in constants {
            if word.starts_with(ch) && self.follows(pos, word) {
                return Ok((value, pos + word.len()));
            }
        }
        self.number(pos)
    }

    fn digits(&self, mut pos: usize) -> usize {
        while self.at(pos).is_some_and(|ch| ch.is_ascii_digit()) {
            pos += 1;
        }
        pos
    }

    fn number(&self, start: usize) -> Result<(Json, usize), Failure> {
        let mut pos = start;
        if self.at(pos) == Some('-') {
            pos += 1;
        }
        match self.at(pos) {
            Some('1'..='9') => pos = self.digits(pos + 1),
            Some('0') => pos += 1,
            _ => return Err(("Expecting value", start)),
        }
        let integer_end = pos;
        if self.at(pos) == Some('.') && self.at(pos + 1).is_some_and(|ch| ch.is_ascii_digit()) {
            pos = self.digits(pos + 2);
        }
        if matches!(self.at(pos), Some('e' | 'E')) {
            let mut exponent = pos + 1;
            if matches!(self.at(exponent), Some('+' | '-')) {
                exponent += 1;
            }
            let end = self.digits(exponent);
            if end > exponent {
                pos = end;
            }
        }
        if pos == integer_end {
            return Ok((Json::Int(self.chars[start..pos].iter().collect()), pos));
        }
        Ok((Json::Float, pos))
    }

    fn hex4(&self, pos: usize) -> Option<u32> {
        let digits: String = self.chars.get(pos..pos + 4)?.iter().collect();
        if !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(&digits, 16).ok()
    }

    /// The rest of a string whose opening quote is just before `start`; `None` text for a lone surrogate.
    fn string(&self, start: usize) -> Result<(Option<String>, usize), Failure> {
        let begin = start - 1;
        let unterminated = ("Unterminated string starting at", begin);
        let mut out = String::new();
        let mut valid = true;
        let mut pos = start;
        loop {
            let Some(ch) = self.at(pos) else {
                return Err(unterminated);
            };
            match ch {
                '"' => return Ok((valid.then_some(out), pos + 1)),
                '\\' => {}
                ch if (ch as u32) < 0x20 => return Err(("Invalid control character at", pos)),
                ch => {
                    out.push(ch);
                    pos += 1;
                    continue;
                }
            }
            let Some(escape) = self.at(pos + 1) else {
                return Err(unterminated);
            };
            let simple = match escape {
                '"' => Some('"'),
                '\\' => Some('\\'),
                '/' => Some('/'),
                'b' => Some('\u{8}'),
                'f' => Some('\u{c}'),
                'n' => Some('\n'),
                'r' => Some('\r'),
                't' => Some('\t'),
                'u' => None,
                _ => return Err(("Invalid \\escape", pos)),
            };
            if let Some(ch) = simple {
                out.push(ch);
                pos += 2;
                continue;
            }
            // `\u` and four hexadecimal digits.
            let digits = pos + 2;
            if digits + 4 > self.chars.len() {
                return Err(("Invalid \\uXXXX escape", pos + 1));
            }
            let unit = self.hex4(digits).ok_or(("Invalid \\uXXXX escape", pos + 1))?;
            pos = digits + 4;
            if (0xd800..0xdc00).contains(&unit)
                && pos + 6 <= self.chars.len()
                && self.at(pos) == Some('\\')
                && self.at(pos + 1) == Some('u')
            {
                let low = self.hex4(pos + 2).ok_or(("Invalid \\uXXXX escape", pos + 1))?;
                if (0xdc00..0xe000).contains(&low) {
                    let code = 0x10000 + ((unit - 0xd800) << 10) + (low - 0xdc00);
                    out.extend(char::from_u32(code));
                    pos += 6;
                    continue;
                }
            }
            match char::from_u32(unit) {
                Some(ch) => out.push(ch),
                None => valid = false,
            }
        }
    }

    fn array(&self, start: usize) -> Result<(Json, usize), Failure> {
        let mut items = Vec::new();
        let mut pos = self.skip_whitespace(start);
        if self.at(pos) == Some(']') {
            return Ok((Json::Array(items), pos + 1));
        }
        loop {
            let (item, end) = self.value(pos)?;
            items.push(item);
            pos = self.skip_whitespace(end);
            match self.at(pos) {
                Some(']') => return Ok((Json::Array(items), pos + 1)),
                Some(',') => {}
                _ => return Err(("Expecting ',' delimiter", pos)),
            }
            let comma = pos;
            pos = self.skip_whitespace(pos + 1);
            if self.at(pos) == Some(']') {
                return Err(("Illegal trailing comma before end of array", comma));
            }
        }
    }

    fn object(&self, start: usize) -> Result<(Json, usize), Failure> {
        let mut pairs = Vec::new();
        let mut pos = self.skip_whitespace(start);
        if self.at(pos) == Some('}') {
            return Ok((Json::Object(pairs), pos + 1));
        }
        loop {
            if self.at(pos) != Some('"') {
                return Err(("Expecting property name enclosed in double quotes", pos));
            }
            let (key, end) = self.string(pos + 1)?;
            pos = self.skip_whitespace(end);
            if self.at(pos) != Some(':') {
                return Err(("Expecting ':' delimiter", pos));
            }
            let (value, end) = self.value(self.skip_whitespace(pos + 1))?;
            pairs.push((key, value));
            pos = self.skip_whitespace(end);
            match self.at(pos) {
                Some('}') => return Ok((Json::Object(pairs), pos + 1)),
                Some(',') => {}
                _ => return Err(("Expecting ',' delimiter", pos)),
            }
            let comma = pos;
            pos = self.skip_whitespace(pos + 1);
            if self.at(pos) == Some('}') {
                return Err(("Illegal trailing comma before end of object", comma));
            }
        }
    }

    /// The error text with its line, column and character position.
    fn describe(&self, (message, pos): Failure) -> String {
        let before = &self.chars[..pos.min(self.chars.len())];
        let line = before.iter().filter(|&&ch| ch == '\n').count() + 1;
        let column = match before.iter().rposition(|&ch| ch == '\n') {
            Some(newline) => pos - newline,
            None => pos + 1,
        };
        format!("{message}: line {line} column {column} (char {pos})")
    }
}

/// Parse a whole document; the error is the reader's message with its position.
fn parse(text: &str) -> Result<Json, String> {
    let reader = Reader {
        chars: text.chars().collect(),
    };
    if reader.at(0) == Some('\u{feff}') {
        return Err(reader.describe(("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0)));
    }
    let (value, end) = reader
        .value(reader.skip_whitespace(0))
        .map_err(|failure| reader.describe(failure))?;
    let end = reader.skip_whitespace(end);
    if end != reader.chars.len() {
        return Err(reader.describe(("Extra data", end)));
    }
    Ok(value)
}

/// One issue identity: file, line, target and code.
#[derive(Debug, Hash, PartialEq, Eq)]
struct Key {
    file: String,
    line: String,
    target: String,
    code: String,
}

/// The issue identities recorded in a baseline report.
#[derive(Debug, Default)]
pub struct Baseline(HashSet<Key>);

impl Baseline {
    /// Read the baseline file at `path`, printed in messages as `shown`.
    pub fn load(path: &Path, shown: &str) -> Result<Baseline, AuditError> {
        let text = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| decode_utf8(&bytes))
            .map_err(|detail| AuditError::new(format!("Cannot read baseline {shown}: {detail}")))?;
        let invalid = || {
            AuditError::new(format!(
                "Invalid baseline {shown}: expected a JSON report with an issues list"
            ))
        };
        // Read as text: every `\r\n` and lone `\r` is a line feed before the JSON reader sees it.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let document = parse(&text).map_err(|detail| AuditError::new(format!("Invalid baseline {shown}: {detail}")))?;
        let Some(Json::Array(entries)) = document.get("issues") else {
            return Err(invalid());
        };
        let mut keys = HashSet::new();
        for entry in entries {
            match (
                entry.get("file"),
                entry.get("line"),
                entry.get("target"),
                entry.get("code"),
            ) {
                (Some(Json::Str(file)), Some(Json::Int(line)), Some(Json::Str(target)), Some(Json::Str(code))) => {
                    if let (Some(file), Some(target), Some(code)) = (file, target, code) {
                        keys.insert(Key {
                            file: file.clone(),
                            line: line.clone(),
                            target: target.clone(),
                            code: code.clone(),
                        });
                    }
                }
                _ => return Err(invalid()),
            }
        }
        Ok(Baseline(keys))
    }

    /// Split `issues` into those not in the baseline, and count the suppressed ones.
    pub fn filter(&self, issues: Vec<Issue>) -> (Vec<Issue>, usize) {
        let before = issues.len();
        let kept: Vec<Issue> = issues
            .into_iter()
            .filter(|issue| {
                let key = Key {
                    file: issue.file.clone(),
                    line: issue.line.to_string(),
                    target: issue.target.clone(),
                    code: issue.code.to_string(),
                };
                !self.0.contains(&key)
            })
            .collect();
        let suppressed = before - kept.len();
        (kept, suppressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_extensions() {
        assert!(parse("{\"a\": NaN, \"b\": -Infinity, \"c\": [1e5, 2.5, -0]}").is_ok());
        assert!(matches!(parse("\"\\ud800\""), Ok(Json::Str(None))));
        assert!(matches!(parse("\"\\ud83d\\ude00\""), Ok(Json::Str(Some(text))) if text == "\u{1f600}"));
        assert!(matches!(parse("123456789012345678901234567890"), Ok(Json::Int(digits)) if digits.len() == 30));
    }

    #[test]
    fn reports_syntax_errors_with_their_position() {
        let error = |text| parse(text).err().unwrap_or_default();
        assert_eq!(
            error("\u{feff}{}"),
            "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"
        );
        assert_eq!(
            error("{\"a\": 01}"),
            "Expecting ',' delimiter: line 1 column 8 (char 7)"
        );
        assert_eq!(
            error("[1,\n]"),
            "Illegal trailing comma before end of array: line 1 column 3 (char 2)"
        );
        assert_eq!(error("{\"a\": \"\\q\"}"), "Invalid \\escape: line 1 column 8 (char 7)");
        assert_eq!(error("[1] x"), "Extra data: line 1 column 5 (char 4)");
    }

    #[test]
    fn last_duplicate_key_wins() {
        let document = parse("{\"issues\": 1, \"issues\": []}").expect("valid JSON");
        assert!(matches!(document.get("issues"), Some(Json::Array(items)) if items.is_empty()));
    }
}
