//! Static HTML tags from the raw HTML fragments of a Markdown document.
//!
//! Fragments are fed in document order, each at its source line, into one tokenizer: a tag that is
//! still open when its fragment ends is completed by later fragments, and the body of a `script`
//! or `style` element stays raw text until its end tag, across any number of fragments. Lines skipped
//! between fragments are counted as blank, so a tag reports the line its `<` sits on.

use std::collections::HashMap;
use std::sync::OnceLock;

/// An opening tag with lower-cased names and entity-decoded attribute values; an attribute
/// written without `=` has no value.
#[derive(Debug, PartialEq)]
pub struct StartTag {
    pub name: String,
    pub attrs: Vec<(String, Option<String>)>,
    pub line: usize,
}

#[derive(Debug, PartialEq)]
pub enum Token {
    Start(StartTag),
    End(String),
    /// A `<!-- ... -->` comment: its text between the delimiters and the line its `<` is on.
    Comment(String, usize),
}

const RAW_TEXT: [&str; 2] = ["script", "style"];

pub struct Tokenizer {
    pending: String,
    pending_line: usize,
    fed_line: usize,
    raw_text: Option<String>,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Tokenizer {
            pending: String::new(),
            pending_line: 1,
            fed_line: 1,
            raw_text: None,
        }
    }
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

impl Tokenizer {
    /// Feed a fragment that starts on document line `line` and return the tokens it completes.
    pub fn feed_at(&mut self, fragment: &str, line: usize) -> Vec<Token> {
        let padding = line.saturating_sub(self.fed_line);
        self.pending.extend(std::iter::repeat_n('\n', padding));
        self.pending.push_str(fragment);
        self.fed_line += padding + fragment.matches('\n').count();
        self.drain()
    }

    fn drain(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let text = std::mem::take(&mut self.pending);
        let bytes = text.as_bytes();
        let mut index = 0;
        let mut counted = 0;
        let mut line = self.pending_line;
        let mut line_at = |position: usize| {
            line += text[counted..position].matches('\n').count();
            counted = position;
            line
        };
        loop {
            if let Some(element) = self.raw_text.clone() {
                let Some(end) = raw_text_end(&text[index..], &element) else {
                    break;
                };
                index += end;
                let Some((name, next)) = end_tag(&text, index) else {
                    break;
                };
                if name == element {
                    self.raw_text = None;
                }
                tokens.push(Token::End(name));
                index = next;
                continue;
            }
            let Some(offset) = text[index..].find('<') else {
                index = text.len();
                break;
            };
            index += offset;
            let Some(&next) = bytes.get(index + 1) else {
                break;
            };
            if next.is_ascii_alphabetic() {
                let Some((name, attrs, end, self_closing)) = start_tag(&text, index) else {
                    break;
                };
                let start_line = line_at(index);
                if self_closing {
                    tokens.push(Token::Start(StartTag {
                        name: name.clone(),
                        attrs,
                        line: start_line,
                    }));
                    tokens.push(Token::End(name));
                } else {
                    if RAW_TEXT.contains(&name.as_str()) {
                        self.raw_text = Some(name.clone());
                    }
                    tokens.push(Token::Start(StartTag {
                        name,
                        attrs,
                        line: start_line,
                    }));
                }
                index = end;
            } else if next == b'/' {
                if bytes.get(index + 2).is_some_and(u8::is_ascii_alphabetic) {
                    let Some((name, end)) = end_tag(&text, index) else {
                        break;
                    };
                    tokens.push(Token::End(name));
                    index = end;
                } else {
                    let Some(end) = text[index..].find('>') else {
                        break;
                    };
                    index += end + 1;
                }
            } else if text[index..].starts_with("<!--") {
                let Some(end) = text[index + 4..].find("-->") else {
                    break;
                };
                let comment_line = line_at(index);
                tokens.push(Token::Comment(
                    text[index + 4..index + 4 + end].to_string(),
                    comment_line,
                ));
                index += 4 + end + 3;
            } else if next == b'!' || next == b'?' {
                let Some(end) = text[index..].find('>') else {
                    break;
                };
                index += end + 1;
            } else {
                index += 1;
            }
        }
        line_at(index);
        self.pending_line = line;
        self.pending = text[index..].to_string();
        tokens
    }
}

/// Offset of the end tag that closes a raw-text `element`, if it is already complete.
fn raw_text_end(text: &str, element: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let pattern = format!("</{element}");
    let mut from = 0;
    while let Some(offset) = lower[from..].find(&pattern) {
        let at = from + offset;
        match lower.as_bytes().get(at + pattern.len()) {
            Some(b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/' | b'>') => return Some(at),
            None => return None,
            Some(_) => from = at + 1,
        }
    }
    None
}

fn end_tag(text: &str, index: usize) -> Option<(String, usize)> {
    let close = index + text[index..].find('>')?;
    let bytes = text.as_bytes();
    let mut end = index + 2;
    while end < close && !matches!(bytes[end], b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/' | b'\0') {
        end += 1;
    }
    Some((text[index + 2..end].to_lowercase(), close + 1))
}

type Attributes = Vec<(String, Option<String>)>;

fn start_tag(text: &str, index: usize) -> Option<(String, Attributes, usize, bool)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut at = index + 1;
    while at < len && !matches!(bytes[at], b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/' | b'>' | b'\0') {
        at += 1;
    }
    let name = text[index + 1..at].to_lowercase();
    let mut attrs = Vec::new();
    loop {
        while at < len && (is_space(bytes[at]) || (bytes[at] == b'/' && bytes.get(at + 1) != Some(&b'>'))) {
            at += 1;
        }
        if at >= len {
            return None;
        }
        if bytes[at] == b'>' {
            return Some((name, attrs, at + 1, false));
        }
        if bytes[at] == b'/' {
            return Some((name, attrs, at + 2, true));
        }
        let key_start = at;
        at += 1;
        while at < len && !is_space(bytes[at]) && !matches!(bytes[at], b'/' | b'=' | b'>') {
            at += 1;
        }
        let key = text[key_start..at].to_lowercase();
        let mut cursor = at;
        while cursor < len && is_space(bytes[cursor]) {
            cursor += 1;
        }
        if cursor >= len {
            return None;
        }
        if bytes[cursor] != b'=' {
            attrs.push((key, None));
            continue;
        }
        while cursor < len && bytes[cursor] == b'=' {
            cursor += 1;
        }
        while cursor < len && is_space(bytes[cursor]) {
            cursor += 1;
        }
        if cursor >= len {
            return None;
        }
        let value = if bytes[cursor] == b'"' || bytes[cursor] == b'\'' {
            let quote = bytes[cursor] as char;
            let close = cursor + 1 + text[cursor + 1..].find(quote)?;
            at = close + 1;
            &text[cursor + 1..close]
        } else {
            let start = cursor;
            while cursor < len && !is_space(bytes[cursor]) && bytes[cursor] != b'>' {
                cursor += 1;
            }
            at = cursor;
            &text[start..cursor]
        };
        attrs.push((key, Some(unescape(value))));
    }
}

fn named_references() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        entities::ENTITIES
            .iter()
            .map(|entity| (&entity.entity[1..], entity.characters))
            .collect()
    })
}

fn numeric_reference(number: u32) -> String {
    const WINDOWS_1252: [u32; 32] = [
        0x20ac, 0x81, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8d,
        0x017d, 0x8f, 0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a,
        0x0153, 0x9d, 0x017e, 0x0178,
    ];
    let replaced = match number {
        0x00 => Some(0xfffd),
        0x0d => Some(0x0d),
        0x80..=0x9f => Some(WINDOWS_1252[(number - 0x80) as usize]),
        _ => None,
    };
    if let Some(code) = replaced {
        return char::from_u32(code).map(String::from).unwrap_or_default();
    }
    if (0xd800..=0xdfff).contains(&number) || number > 0x10ffff {
        return "\u{fffd}".to_string();
    }
    let invalid = matches!(number, 0x01..=0x08 | 0x0b | 0x0e..=0x1f | 0x7f..=0x9f | 0xfdd0..=0xfdef)
        || (number & 0xfffe) == 0xfffe;
    if invalid {
        return String::new();
    }
    char::from_u32(number).map(String::from).unwrap_or_default()
}

/// Decode the character references of an attribute value once, as HTML attribute values are decoded: decimal and
/// hexadecimal references always (with or without `;`), and a named reference only when the letters and digits
/// after `&`, with a directly following `;`, are exactly a name of the HTML5 table and no `=` follows them.
pub fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        if let Some(digits) = after.strip_prefix('#') {
            let (hex, body) = match digits.strip_prefix(['x', 'X']) {
                Some(body) => (true, body),
                None => (false, digits),
            };
            let len = body
                .bytes()
                .take_while(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() })
                .count();
            if len > 0 {
                let radix = if hex { 16 } else { 10 };
                let number = body[..len].chars().fold(0u32, |acc, ch| {
                    acc.saturating_mul(radix)
                        .saturating_add(ch.to_digit(radix).unwrap_or(0))
                });
                out.push_str(&numeric_reference(number));
                let mut consumed = 1 + (digits.len() - body.len()) + len;
                if body[len..].starts_with(';') {
                    consumed += 1;
                }
                rest = &after[consumed..];
                continue;
            }
            out.push('&');
            rest = after;
            continue;
        }
        let name_len = if after.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
            after.bytes().take_while(u8::is_ascii_alphanumeric).count()
        } else {
            0
        };
        let with_semicolon = after[name_len..].starts_with(';');
        let reference = &after[..name_len + usize::from(with_semicolon)];
        match named_references().get(reference) {
            Some(characters) if name_len > 0 && !after[name_len..].starts_with('=') => out.push_str(characters),
            _ => {
                out.push('&');
                out.push_str(reference);
            }
        }
        rest = &after[reference.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(tokens: &[Token]) -> Vec<(&str, usize)> {
        tokens
            .iter()
            .filter_map(|token| match token {
                Token::Start(tag) => Some((tag.name.as_str(), tag.line)),
                Token::End(_) | Token::Comment(..) => None,
            })
            .collect()
    }

    #[test]
    fn references_decode_once() {
        assert_eq!(unescape("a&amp;amp;b&#32;&#x4e2d;&eacute;"), "a&amp;b \u{4e2d}\u{e9}");
        assert_eq!(
            unescape("&ampx; &notit; &copy2 &amp=x &zz; & x &eacute"),
            "&ampx; &notit; &copy2 &amp=x &zz; & x \u{e9}"
        );
        assert_eq!(unescape("&#150;&#0;"), "\u{2013}\u{fffd}");
    }

    #[test]
    fn attributes_quoting_and_case() {
        let mut tokenizer = Tokenizer::default();
        let tokens = tokenizer.feed_at("<A HREF=a.md title='x>y' Data-x = \"1\" hidden>", 3);
        let Token::Start(tag) = &tokens[0] else {
            panic!("no start tag")
        };
        assert_eq!(tag.name, "a");
        assert_eq!(tag.line, 3);
        assert_eq!(
            tag.attrs,
            vec![
                ("href".to_string(), Some("a.md".to_string())),
                ("title".to_string(), Some("x>y".to_string())),
                ("data-x".to_string(), Some("1".to_string())),
                ("hidden".to_string(), None),
            ]
        );
    }

    #[test]
    fn raw_text_and_comments_span_fragments() {
        let mut tokenizer = Tokenizer::default();
        assert_eq!(starts(&tokenizer.feed_at("<script>", 1)), vec![("script", 1)]);
        assert!(starts(&tokenizer.feed_at("<a href=x>", 3)).is_empty());
        assert_eq!(starts(&tokenizer.feed_at("</script><b>", 5)), vec![("b", 5)]);
        assert!(starts(&tokenizer.feed_at("<!-- <i> -->", 6)).is_empty());
        assert_eq!(starts(&tokenizer.feed_at("<a\n", 7)), Vec::<(&str, usize)>::new());
        assert_eq!(starts(&tokenizer.feed_at("href=y>", 8)), vec![("a", 7)]);
    }
}
