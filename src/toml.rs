//! TOML 1.0 documents for the configuration file, with the acceptance rules and error messages the reports print:
//! positions count characters of the text after every `\r\n` became `\n`, and a key in a message is written as its
//! quoted parts in parentheses.

use std::collections::HashMap;

use unicode_general_category::{GeneralCategory, get_general_category};

/// A parsed value. Numbers, dates and times only keep their kind: the configuration never reads them.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Boolean(bool),
    Integer,
    Float,
    Datetime,
    Array(Vec<Value>),
    Table(Table),
}

/// A table in document order.
pub type Table = Vec<(String, Value)>;

pub fn get<'a>(table: &'a Table, key: &str) -> Option<&'a Value> {
    table.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

type Key = Vec<String>;

const FROZEN: u8 = 1;
const EXPLICIT_NEST: u8 = 2;

#[derive(Default)]
struct FlagNode {
    flags: u8,
    recursive_flags: u8,
    nested: HashMap<String, FlagNode>,
}

#[derive(Default)]
struct Flags {
    root: HashMap<String, FlagNode>,
    pending: Vec<(Key, u8)>,
}

impl Flags {
    fn add_pending(&mut self, key: Key, flag: u8) {
        self.pending.push((key, flag));
    }

    fn finalize_pending(&mut self) {
        for (key, flag) in std::mem::take(&mut self.pending) {
            self.set(&key, flag, false);
        }
    }

    fn unset_all(&mut self, key: &[String]) {
        let mut cont = &mut self.root;
        for part in &key[..key.len() - 1] {
            match cont.get_mut(part) {
                Some(node) => cont = &mut node.nested,
                None => return,
            }
        }
        cont.remove(&key[key.len() - 1]);
    }

    fn set(&mut self, key: &[String], flag: u8, recursive: bool) {
        let mut cont = &mut self.root;
        for part in &key[..key.len() - 1] {
            cont = &mut cont.entry(part.clone()).or_default().nested;
        }
        let node = cont.entry(key[key.len() - 1].clone()).or_default();
        if recursive {
            node.recursive_flags |= flag;
        } else {
            node.flags |= flag;
        }
    }

    fn is(&self, key: &[String], flag: u8) -> bool {
        let Some((stem, parent)) = key.split_last() else {
            return false;
        };
        let mut cont = &self.root;
        for part in parent {
            let Some(node) = cont.get(part) else {
                return false;
            };
            if node.recursive_flags & flag != 0 {
                return true;
            }
            cont = &node.nested;
        }
        cont.get(stem)
            .is_some_and(|node| (node.flags | node.recursive_flags) & flag != 0)
    }
}

/// The table `key` names below `cont`, created on the way; the last table of an array when `access_lists`.
/// `None` when a value that is not a table is in the way.
fn nest<'a>(mut cont: &'a mut Table, key: &[String], access_lists: bool) -> Option<&'a mut Table> {
    for part in key {
        let index = match cont.iter().position(|(name, _)| name == part) {
            Some(index) => index,
            None => {
                cont.push((part.clone(), Value::Table(Vec::new())));
                cont.len() - 1
            }
        };
        let mut value = &mut cont[index].1;
        if access_lists && let Value::Array(items) = value {
            value = items.last_mut()?;
        }
        match value {
            Value::Table(table) => cont = table,
            _ => return None,
        }
    }
    Some(cont)
}

fn append_nest_to_list(root: &mut Table, key: &[String]) -> Option<()> {
    let (stem, parent) = key.split_last()?;
    let cont = nest(root, parent, true)?;
    match cont.iter_mut().find(|(name, _)| name == stem) {
        Some((_, Value::Array(items))) => items.push(Value::Table(Vec::new())),
        Some(_) => return None,
        None => cont.push((stem.clone(), Value::Array(vec![Value::Table(Vec::new())]))),
    }
    Some(())
}

fn is_printable(ch: char) -> bool {
    if ch.is_ascii() {
        return (' '..='~').contains(&ch);
    }
    !matches!(
        get_general_category(ch),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::SpaceSeparator
    )
}

/// A string quoted for an error message: in single quotes, or in double quotes when it holds a single quote and no
/// double quote; backslash, tab, newline, carriage return and the quote are escaped, and other non-printable
/// characters become `\xNN`, `\uNNNN` or `\UNNNNNNNN`.
pub fn string_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if ch == quote => {
                out.push('\\');
                out.push(ch);
            }
            _ if is_printable(ch) => out.push(ch),
            _ if (ch as u32) <= 0xff => out.push_str(&format!("\\x{:02x}", ch as u32)),
            _ if (ch as u32) <= 0xffff => out.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => out.push_str(&format!("\\U{:08x}", ch as u32)),
        }
    }
    out.push(quote);
    out
}

/// A key for an error message: its quoted parts in parentheses, separated by `, `, with a trailing comma after a
/// single part.
fn key_repr(key: &[String]) -> String {
    let parts: Vec<String> = key.iter().map(|part| string_repr(part)).collect();
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

const WS: &[char] = &[' ', '\t'];
const WS_AND_NEWLINE: &[char] = &[' ', '\t', '\n'];

fn is_ascii_ctrl(ch: char) -> bool {
    (ch as u32) < 32 || ch == '\u{7f}'
}

fn illegal_one_line(ch: char) -> bool {
    is_ascii_ctrl(ch) && ch != '\t'
}

fn illegal_multiline(ch: char) -> bool {
    is_ascii_ctrl(ch) && ch != '\t' && ch != '\n'
}

fn is_bare_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

struct Parser {
    src: Vec<char>,
}

type Parsed<T> = Result<(usize, T), String>;

impl Parser {
    fn at(&self, pos: usize) -> Option<char> {
        self.src.get(pos).copied()
    }

    fn starts_with(&self, pos: usize, text: &str) -> bool {
        (pos..).zip(text.chars()).all(|(index, ch)| self.at(index) == Some(ch))
    }

    fn error(&self, pos: usize, message: &str) -> String {
        if pos >= self.src.len() {
            return format!("{message} (at end of document)");
        }
        let line = self.src[..pos].iter().filter(|&&ch| ch == '\n').count() + 1;
        let column = match self.src[..pos].iter().rposition(|&ch| ch == '\n') {
            Some(newline) => pos - newline,
            None => pos + 1,
        };
        format!("{message} (at line {line}, column {column})")
    }

    fn skip_chars(&self, mut pos: usize, chars: &[char]) -> usize {
        while self.at(pos).is_some_and(|ch| chars.contains(&ch)) {
            pos += 1;
        }
        pos
    }

    fn find(&self, pos: usize, expect: &str) -> Option<usize> {
        (pos..self.src.len()).find(|&index| self.starts_with(index, expect))
    }

    fn skip_until(
        &self,
        mut pos: usize,
        expect: &str,
        error_on: fn(char) -> bool,
        error_on_eof: bool,
    ) -> Result<usize, String> {
        let new_pos = match self.find(pos, expect) {
            Some(found) => found,
            None if error_on_eof => {
                return Err(self.error(self.src.len(), &format!("Expected {}", string_repr(expect))));
            }
            None => self.src.len(),
        };
        if pos < new_pos && self.src[pos..new_pos].iter().any(|&ch| error_on(ch)) {
            while !error_on(self.src[pos]) {
                pos += 1;
            }
            return Err(self.error(
                pos,
                &format!("Found invalid character {}", string_repr(&self.src[pos].to_string())),
            ));
        }
        Ok(new_pos)
    }

    fn skip_comment(&self, pos: usize) -> Result<usize, String> {
        if self.at(pos) == Some('#') {
            return self.skip_until(pos + 1, "\n", illegal_one_line, false);
        }
        Ok(pos)
    }

    fn skip_comments_and_array_ws(&self, mut pos: usize) -> Result<usize, String> {
        loop {
            let before = pos;
            pos = self.skip_chars(pos, WS_AND_NEWLINE);
            pos = self.skip_comment(pos)?;
            if pos == before {
                return Ok(pos);
            }
        }
    }

    fn document(&self) -> Result<Table, String> {
        let mut data: Table = Vec::new();
        let mut flags = Flags::default();
        let mut header: Key = Vec::new();
        let mut pos = 0;
        loop {
            pos = self.skip_chars(pos, WS);
            let Some(ch) = self.at(pos) else {
                break;
            };
            if ch == '\n' {
                pos += 1;
                continue;
            }
            if is_bare_key_char(ch) || ch == '"' || ch == '\'' {
                pos = self.key_value_rule(pos, &mut data, &mut flags, &header)?;
                pos = self.skip_chars(pos, WS);
            } else if ch == '[' {
                flags.finalize_pending();
                let (next, key) = if self.at(pos + 1) == Some('[') {
                    self.create_list_rule(pos, &mut data, &mut flags)?
                } else {
                    self.create_dict_rule(pos, &mut data, &mut flags)?
                };
                header = key;
                pos = self.skip_chars(next, WS);
            } else if ch != '#' {
                return Err(self.error(pos, "Invalid statement"));
            }
            pos = self.skip_comment(pos)?;
            match self.at(pos) {
                None => break,
                Some('\n') => pos += 1,
                Some(_) => {
                    return Err(self.error(pos, "Expected newline or end of document after a statement"));
                }
            }
        }
        Ok(data)
    }

    fn create_dict_rule(&self, pos: usize, data: &mut Table, flags: &mut Flags) -> Parsed<Key> {
        let pos = self.skip_chars(pos + 1, WS);
        let (pos, key) = self.parse_key(pos)?;
        if flags.is(&key, EXPLICIT_NEST) || flags.is(&key, FROZEN) {
            return Err(self.error(pos, &format!("Cannot declare {} twice", key_repr(&key))));
        }
        flags.set(&key, EXPLICIT_NEST, false);
        if nest(data, &key, true).is_none() {
            return Err(self.error(pos, "Cannot overwrite a value"));
        }
        if !self.starts_with(pos, "]") {
            return Err(self.error(pos, "Expected ']' at the end of a table declaration"));
        }
        Ok((pos + 1, key))
    }

    fn create_list_rule(&self, pos: usize, data: &mut Table, flags: &mut Flags) -> Parsed<Key> {
        let pos = self.skip_chars(pos + 2, WS);
        let (pos, key) = self.parse_key(pos)?;
        if flags.is(&key, FROZEN) {
            return Err(self.error(pos, &format!("Cannot mutate immutable namespace {}", key_repr(&key))));
        }
        flags.unset_all(&key);
        flags.set(&key, EXPLICIT_NEST, false);
        if append_nest_to_list(data, &key).is_none() {
            return Err(self.error(pos, "Cannot overwrite a value"));
        }
        if !self.starts_with(pos, "]]") {
            return Err(self.error(pos, "Expected ']]' at the end of an array declaration"));
        }
        Ok((pos + 2, key))
    }

    fn key_value_rule(
        &self,
        pos: usize,
        data: &mut Table,
        flags: &mut Flags,
        header: &[String],
    ) -> Result<usize, String> {
        let (pos, key, value) = self.parse_key_value_pair(pos)?;
        let (stem, parent) = key.split_last().expect("a key has at least one part");
        let absolute_parent: Key = header.iter().chain(parent).cloned().collect();
        for index in 1..key.len() {
            let cont_key: Key = header.iter().chain(&key[..index]).cloned().collect();
            if flags.is(&cont_key, EXPLICIT_NEST) {
                return Err(self.error(pos, &format!("Cannot redefine namespace {}", key_repr(&cont_key))));
            }
            flags.add_pending(cont_key, EXPLICIT_NEST);
        }
        if flags.is(&absolute_parent, FROZEN) {
            return Err(self.error(
                pos,
                &format!("Cannot mutate immutable namespace {}", key_repr(&absolute_parent)),
            ));
        }
        let Some(table) = nest(data, &absolute_parent, true) else {
            return Err(self.error(pos, "Cannot overwrite a value"));
        };
        if table.iter().any(|(name, _)| name == stem) {
            return Err(self.error(pos, "Cannot overwrite a value"));
        }
        if matches!(value, Value::Table(_) | Value::Array(_)) {
            let full: Key = header.iter().chain(&key).cloned().collect();
            flags.set(&full, FROZEN, true);
        }
        table.push((stem.clone(), value));
        Ok(pos)
    }

    fn parse_key_value_pair(&self, pos: usize) -> Result<(usize, Key, Value), String> {
        let (pos, key) = self.parse_key(pos)?;
        if self.at(pos) != Some('=') {
            return Err(self.error(pos, "Expected '=' after a key in a key/value pair"));
        }
        let pos = self.skip_chars(pos + 1, WS);
        let (pos, value) = self.parse_value(pos)?;
        Ok((pos, key, value))
    }

    fn parse_key(&self, pos: usize) -> Parsed<Key> {
        let (pos, part) = self.parse_key_part(pos)?;
        let mut key = vec![part];
        let mut pos = self.skip_chars(pos, WS);
        while self.at(pos) == Some('.') {
            pos = self.skip_chars(pos + 1, WS);
            let (next, part) = self.parse_key_part(pos)?;
            key.push(part);
            pos = self.skip_chars(next, WS);
        }
        Ok((pos, key))
    }

    fn parse_key_part(&self, pos: usize) -> Parsed<String> {
        match self.at(pos) {
            Some(ch) if is_bare_key_char(ch) => {
                let end = (pos..self.src.len())
                    .find(|&index| !is_bare_key_char(self.src[index]))
                    .unwrap_or(self.src.len());
                Ok((end, self.src[pos..end].iter().collect()))
            }
            Some('\'') => self.parse_literal_str(pos),
            Some('"') => self.parse_basic_str(pos + 1, false),
            _ => Err(self.error(pos, "Invalid initial character for a key part")),
        }
    }

    fn parse_array(&self, pos: usize) -> Parsed<Value> {
        let mut array = Vec::new();
        let mut pos = self.skip_comments_and_array_ws(pos + 1)?;
        if self.starts_with(pos, "]") {
            return Ok((pos + 1, Value::Array(array)));
        }
        loop {
            let (next, value) = self.parse_value(pos)?;
            array.push(value);
            pos = self.skip_comments_and_array_ws(next)?;
            match self.at(pos) {
                Some(']') => return Ok((pos + 1, Value::Array(array))),
                Some(',') => {}
                _ => return Err(self.error(pos, "Unclosed array")),
            }
            pos = self.skip_comments_and_array_ws(pos + 1)?;
            if self.starts_with(pos, "]") {
                return Ok((pos + 1, Value::Array(array)));
            }
        }
    }

    fn parse_inline_table(&self, pos: usize) -> Parsed<Value> {
        let mut table: Table = Vec::new();
        let mut flags = Flags::default();
        let mut pos = self.skip_chars(pos + 1, WS);
        if self.starts_with(pos, "}") {
            return Ok((pos + 1, Value::Table(table)));
        }
        loop {
            let (next, key, value) = self.parse_key_value_pair(pos)?;
            pos = next;
            let (stem, parent) = key.split_last().expect("a key has at least one part");
            if flags.is(&key, FROZEN) {
                return Err(self.error(pos, &format!("Cannot mutate immutable namespace {}", key_repr(&key))));
            }
            let Some(cont) = nest(&mut table, parent, false) else {
                return Err(self.error(pos, "Cannot overwrite a value"));
            };
            if cont.iter().any(|(name, _)| name == stem) {
                return Err(self.error(pos, &format!("Duplicate inline table key {}", string_repr(stem))));
            }
            let nested = matches!(value, Value::Table(_) | Value::Array(_));
            cont.push((stem.clone(), value));
            pos = self.skip_chars(pos, WS);
            match self.at(pos) {
                Some('}') => return Ok((pos + 1, Value::Table(table))),
                Some(',') => {}
                _ => return Err(self.error(pos, "Unclosed inline table")),
            }
            if nested {
                flags.set(&key, FROZEN, true);
            }
            pos = self.skip_chars(pos + 1, WS);
        }
    }

    fn parse_basic_str_escape(&self, pos: usize, multiline: bool) -> Parsed<String> {
        let first = self.at(pos + 1);
        let pos = pos + 2;
        if multiline && matches!(first, Some(' ' | '\t' | '\n')) {
            let mut pos = pos;
            if first != Some('\n') {
                pos = self.skip_chars(pos, WS);
                match self.at(pos) {
                    None => return Ok((pos, String::new())),
                    Some('\n') => pos += 1,
                    Some(_) => return Err(self.error(pos, "Unescaped '\\' in a string")),
                }
            }
            return Ok((self.skip_chars(pos, WS_AND_NEWLINE), String::new()));
        }
        let replacement = match first {
            Some('u') => return self.parse_hex_char(pos, 4),
            Some('U') => return self.parse_hex_char(pos, 8),
            Some('b') => '\u{8}',
            Some('t') => '\t',
            Some('n') => '\n',
            Some('f') => '\u{c}',
            Some('r') => '\r',
            Some('"') => '"',
            Some('\\') => '\\',
            _ => return Err(self.error(pos, "Unescaped '\\' in a string")),
        };
        Ok((pos, replacement.to_string()))
    }

    fn parse_hex_char(&self, pos: usize, length: usize) -> Parsed<String> {
        let end = (pos + length).min(self.src.len());
        let digits: String = self.src[pos.min(end)..end].iter().collect();
        if digits.chars().count() != length || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(self.error(pos, "Invalid hex value"));
        }
        let pos = pos + length;
        let code = u32::from_str_radix(&digits, 16).map_err(|_| self.error(pos, "Invalid hex value"))?;
        match char::from_u32(code) {
            Some(ch) => Ok((pos, ch.to_string())),
            None => Err(self.error(pos, "Escaped character is not a Unicode scalar value")),
        }
    }

    fn parse_literal_str(&self, pos: usize) -> Parsed<String> {
        let start = pos + 1;
        let end = self.skip_until(start, "'", illegal_one_line, true)?;
        Ok((end + 1, self.src[start..end].iter().collect()))
    }

    fn parse_multiline_str(&self, pos: usize, literal: bool) -> Parsed<Value> {
        let mut pos = pos + 3;
        if self.starts_with(pos, "\n") {
            pos += 1;
        }
        let (delim, mut result) = if literal {
            let end = self.skip_until(pos, "'''", illegal_multiline, true)?;
            let result: String = self.src[pos..end].iter().collect();
            pos = end + 3;
            ('\'', result)
        } else {
            let (end, result) = self.parse_basic_str(pos, true)?;
            pos = end;
            ('"', result)
        };
        for _ in 0..2 {
            if self.at(pos) != Some(delim) {
                break;
            }
            pos += 1;
            result.push(delim);
        }
        Ok((pos, Value::String(result)))
    }

    fn parse_basic_str(&self, mut pos: usize, multiline: bool) -> Parsed<String> {
        let illegal = if multiline { illegal_multiline } else { illegal_one_line };
        let mut result = String::new();
        let mut start = pos;
        loop {
            let Some(ch) = self.at(pos) else {
                return Err(self.error(pos, "Unterminated string"));
            };
            if ch == '"' {
                if !multiline {
                    result.extend(&self.src[start..pos]);
                    return Ok((pos + 1, result));
                }
                if self.starts_with(pos, "\"\"\"") {
                    result.extend(&self.src[start..pos]);
                    return Ok((pos + 3, result));
                }
                pos += 1;
                continue;
            }
            if ch == '\\' {
                result.extend(&self.src[start..pos]);
                let (next, escaped) = self.parse_basic_str_escape(pos, multiline)?;
                result.push_str(&escaped);
                pos = next;
                start = pos;
                continue;
            }
            if illegal(ch) {
                return Err(self.error(pos, &format!("Illegal character {}", string_repr(&ch.to_string()))));
            }
            pos += 1;
        }
    }

    fn parse_value(&self, pos: usize) -> Parsed<Value> {
        match self.at(pos) {
            Some('"') if self.starts_with(pos, "\"\"\"") => return self.parse_multiline_str(pos, false),
            Some('"') => {
                let (end, text) = self.parse_basic_str(pos + 1, false)?;
                return Ok((end, Value::String(text)));
            }
            Some('\'') if self.starts_with(pos, "'''") => return self.parse_multiline_str(pos, true),
            Some('\'') => {
                let (end, text) = self.parse_literal_str(pos)?;
                return Ok((end, Value::String(text)));
            }
            Some('t') if self.starts_with(pos, "true") => return Ok((pos + 4, Value::Boolean(true))),
            Some('f') if self.starts_with(pos, "false") => return Ok((pos + 5, Value::Boolean(false))),
            Some('[') => return self.parse_array(pos),
            Some('{') => return self.parse_inline_table(pos),
            _ => {}
        }
        if let Some((end, valid)) = self.match_datetime(pos) {
            if !valid {
                return Err(self.error(pos, "Invalid date or datetime"));
            }
            return Ok((end, Value::Datetime));
        }
        if let Some(end) = self.match_time(pos) {
            return Ok((end, Value::Datetime));
        }
        if let Some((end, float)) = self.match_number(pos) {
            return Ok((end, if float { Value::Float } else { Value::Integer }));
        }
        for special in ["inf", "nan", "-inf", "+inf", "-nan", "+nan"] {
            if self.starts_with(pos, special) {
                return Ok((pos + special.len(), Value::Float));
            }
        }
        Err(self.error(pos, "Invalid value"))
    }

    fn digits(&self, pos: usize, count: usize) -> Option<u32> {
        let mut value = 0;
        for index in pos..pos + count {
            value = value * 10 + self.at(index)?.to_digit(10)?;
        }
        Some(value)
    }

    /// `([01][0-9]|2[0-3]):([0-5][0-9]):([0-5][0-9])(\.[0-9]+)?`, returning the end.
    fn match_time(&self, pos: usize) -> Option<usize> {
        let hour = self.digits(pos, 2)?;
        let minute = self.digits(pos + 3, 2)?;
        let second = self.digits(pos + 6, 2)?;
        if hour > 23 || minute > 59 || second > 59 || self.at(pos + 2) != Some(':') || self.at(pos + 5) != Some(':') {
            return None;
        }
        let mut end = pos + 8;
        if self.at(end) == Some('.') && self.at(end + 1).is_some_and(|ch| ch.is_ascii_digit()) {
            end += 1;
            while self.at(end).is_some_and(|ch| ch.is_ascii_digit()) {
                end += 1;
            }
        }
        Some(end)
    }

    /// A date with an optional time and offset; the flag is whether the date exists in the calendar.
    fn match_datetime(&self, pos: usize) -> Option<(usize, bool)> {
        let year = self.digits(pos, 4)?;
        let month = self.digits(pos + 5, 2)?;
        let day = self.digits(pos + 8, 2)?;
        if self.at(pos + 4) != Some('-') || self.at(pos + 7) != Some('-') {
            return None;
        }
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let mut end = pos + 10;
        if matches!(self.at(end), Some('T' | 't' | ' '))
            && let Some(time_end) = self.match_time(end + 1)
        {
            end = time_end;
            if matches!(self.at(end), Some('Z' | 'z')) {
                end += 1;
            } else if matches!(self.at(end), Some('+' | '-')) {
                let hours = self.digits(end + 1, 2);
                let minutes = self.digits(end + 4, 2);
                if hours.is_some_and(|hours| hours <= 23)
                    && self.at(end + 3) == Some(':')
                    && minutes.is_some_and(|minutes| minutes <= 59)
                {
                    end += 6;
                }
            }
        }
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        Some((end, year >= 1 && day <= days))
    }

    /// `(?:_?<digit>)*` after a first digit, returning the end.
    fn digit_run(&self, mut pos: usize, digit: fn(char) -> bool) -> usize {
        loop {
            match self.at(pos) {
                Some('_') if self.at(pos + 1).is_some_and(digit) => pos += 2,
                Some(ch) if digit(ch) => pos += 1,
                _ => return pos,
            }
        }
    }

    /// The number pattern of the TOML reader: a prefixed hexadecimal, binary or octal integer, or a decimal
    /// integer with optional fraction and exponent. Returns the end and whether it is a float.
    fn match_number(&self, pos: usize) -> Option<(usize, bool)> {
        if self.at(pos) == Some('0') {
            let radix: Option<fn(char) -> bool> = match self.at(pos + 1) {
                Some('x') => Some(|ch: char| ch.is_ascii_hexdigit()),
                Some('b') => Some(|ch: char| ch == '0' || ch == '1'),
                Some('o') => Some(|ch: char| ('0'..='7').contains(&ch)),
                _ => None,
            };
            if let Some(digit) = radix
                && self.at(pos + 2).is_some_and(digit)
            {
                return Some((self.digit_run(pos + 3, digit), false));
            }
        }
        let decimal = |ch: char| ch.is_ascii_digit();
        let mut end = pos;
        if matches!(self.at(end), Some('+' | '-')) {
            end += 1;
        }
        match self.at(end) {
            Some('0') => end += 1,
            Some('1'..='9') => end = self.digit_run(end + 1, decimal),
            _ => return None,
        }
        let integer_end = end;
        if self.at(end) == Some('.') && self.at(end + 1).is_some_and(decimal) {
            end = self.digit_run(end + 2, decimal);
        }
        if matches!(self.at(end), Some('e' | 'E')) {
            let mut exponent = end + 1;
            if matches!(self.at(exponent), Some('+' | '-')) {
                exponent += 1;
            }
            if self.at(exponent).is_some_and(decimal) {
                end = self.digit_run(exponent + 1, decimal);
            }
        }
        Some((end, end != integer_end))
    }
}

/// Parse `text` as a TOML document; the error is the reader's message with its position.
pub fn parse(text: &str) -> Result<Table, String> {
    Parser {
        src: text.replace("\r\n", "\n").chars().collect(),
    }
    .document()
}
