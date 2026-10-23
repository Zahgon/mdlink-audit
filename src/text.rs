//! String rules the reports depend on: which characters split lines, and which are whitespace.

use unicode_general_category::{GeneralCategory, get_general_category};

/// Whitespace for stripping text: Unicode white space plus the four ASCII information separators.
pub fn is_space(ch: char) -> bool {
    ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch)
}

pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

fn is_line_break(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Split into lines, each keeping its terminator; `\r\n` is one terminator.
pub fn lines_with_ends(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if !is_line_break(ch) {
            continue;
        }
        let mut end = index + ch.len_utf8();
        if ch == '\r'
            && let Some(&(next, '\n')) = chars.peek()
        {
            end = next + 1;
            chars.next();
        }
        lines.push(&text[start..end]);
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Split into lines without their terminators.
pub fn lines(text: &str) -> Vec<&str> {
    lines_with_ends(text)
        .into_iter()
        .map(|line| line.trim_end_matches(is_line_break))
        .collect()
}

/// Whether `text` reads as a negative number (`-1`, `-2.5`, `-.5`) rather than an option name.
pub fn is_negative_number(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('-') else {
        return false;
    };
    let digit = |ch: char| get_general_category(ch) == GeneralCategory::DecimalNumber;
    match rest.split_once('.') {
        None => !rest.is_empty() && rest.chars().all(digit),
        Some((whole, fraction)) => whole.chars().all(digit) && !fraction.is_empty() && fraction.chars().all(digit),
    }
}

const BOM: &[u8] = b"\xef\xbb\xbf";

/// The text of the error a strict UTF-8 decoder reports for the first ill-formed sequence of `bytes`, worded as
/// the reports quote it: `'utf-8' codec can't decode byte 0xNN in position P: <reason>` for a
/// one-byte sequence, `... bytes in position P-Q: <reason>` for a longer one, positions counted in bytes.
fn decode_error(bytes: &[u8], error: std::str::Utf8Error) -> String {
    let start = error.valid_up_to();
    let (end, reason) = match error.error_len() {
        None => (bytes.len(), "unexpected end of data"),
        Some(length) if matches!(bytes[start], 0x80..=0xc1 | 0xf5..=0xff) => (start + length, "invalid start byte"),
        Some(length) => (start + length, "invalid continuation byte"),
    };
    if end - start == 1 {
        format!(
            "'utf-8' codec can't decode byte 0x{:02x} in position {start}: {reason}",
            bytes[start]
        )
    } else {
        format!(
            "'utf-8' codec can't decode bytes in position {start}-{}: {reason}",
            end - 1
        )
    }
}

/// Decode a whole file as UTF-8; the error is the decoder's message.
pub fn decode_utf8(bytes: &[u8]) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|error| decode_error(bytes, error))
}

/// Decode a whole file as UTF-8 after removing one leading byte order mark; error positions count from after the
/// mark, and a file that is nothing but the first one or two bytes of a mark reads as empty text.
pub fn decode_utf8_sig(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() < BOM.len() && BOM.starts_with(bytes) {
        return Ok(String::new());
    }
    decode_utf8(bytes.strip_prefix(BOM).unwrap_or(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_breaks_include_form_feed_and_separators() {
        assert_eq!(
            lines_with_ends("a\x0cb\r\nc\u{2028}d"),
            vec!["a\x0c", "b\r\n", "c\u{2028}", "d"]
        );
        assert_eq!(lines(" x \r\n\r\ny"), vec![" x ", "", "y"]);
    }

    #[test]
    fn negative_numbers() {
        assert!(is_negative_number("-1"));
        assert!(is_negative_number("-.5"));
        assert!(is_negative_number("-2.5"));
        assert!(!is_negative_number("-x"));
        assert!(!is_negative_number("-1."));
        assert!(!is_negative_number("-"));
    }
}
