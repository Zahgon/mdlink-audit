//! URL helpers with the exact semantics the report format depends on: CommonMark link
//! destination normalisation, RFC 3986 style splitting, and percent coding.

/// Characters a normalised link destination keeps verbatim besides ASCII letters and digits.
const DESTINATION_KEEP: &str = ";/?:@&=+$,-_.!~*'()#";

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
    }
}

fn push_escaped(out: &mut String, byte: u8) {
    out.push_str(&format!("%{byte:02X}"));
}

/// Normalise a link destination the way the CommonMark renderer the tool was written against
/// does: existing `%XX` escapes stay, every other byte outside the kept set is percent encoded.
pub fn normalize_destination(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' && index + 2 < bytes.len() && is_hex(bytes[index + 1]) && is_hex(bytes[index + 2]) {
            out.push_str(&raw[index..index + 3]);
            index += 3;
            continue;
        }
        if byte.is_ascii_alphanumeric() || DESTINATION_KEEP.as_bytes().contains(&byte) {
            out.push(byte as char);
        } else {
            push_escaped(&mut out, byte);
        }
        index += 1;
    }
    out
}

/// The parts of a split URL reference.
#[derive(Clone, Debug, Default)]
pub struct SplitUrl {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

const SCHEME_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-.";

/// Split a reference into scheme, authority, path, query and fragment.
pub fn split(reference: &str) -> SplitUrl {
    let mut url: String = reference
        .trim_start_matches(|c: char| (c as u32) <= 0x20)
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut result = SplitUrl::default();
    if let Some(colon) = url.find(':') {
        let head = &url[..colon];
        if colon > 0 && head.as_bytes()[0].is_ascii_alphabetic() && head.chars().all(|c| SCHEME_CHARS.contains(c)) {
            result.scheme = head.to_ascii_lowercase();
            url = url[colon + 1..].to_string();
        }
    }
    if url.starts_with("//") {
        let rest = &url[2..];
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        result.netloc = rest[..end].to_string();
        url = rest[end..].to_string();
    }
    if let Some(hash) = url.find('#') {
        result.fragment = url[hash + 1..].to_string();
        url.truncate(hash);
    }
    if let Some(question) = url.find('?') {
        result.query = url[question + 1..].to_string();
        url.truncate(question);
    }
    result.path = url;
    result
}

/// Reassemble a split reference, dropping empty query and fragment parts.
pub fn unsplit(url: &SplitUrl) -> String {
    let mut out = url.path.clone();
    if !url.netloc.is_empty() {
        if !out.is_empty() && !out.starts_with('/') {
            out.insert(0, '/');
        }
        out = format!("//{}{}", url.netloc, out);
    } else if out.starts_with("//") {
        out = format!("//{out}");
    }
    if !url.scheme.is_empty() {
        out = format!("{}:{}", url.scheme, out);
    }
    if !url.query.is_empty() {
        out.push('?');
        out.push_str(&url.query);
    }
    if !url.fragment.is_empty() {
        out.push('#');
        out.push_str(&url.fragment);
    }
    out
}

fn decode_bytes(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() && is_hex(bytes[index + 1]) && is_hex(bytes[index + 2]) {
            out.push(hex_value(bytes[index + 1]) * 16 + hex_value(bytes[index + 2]));
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

/// Percent-decode, failing when the decoded bytes are not UTF-8.
pub fn unquote_strict(text: &str) -> Option<String> {
    if !text.contains('%') {
        return Some(text.to_string());
    }
    String::from_utf8(decode_bytes(text)).ok()
}

/// Percent-decode, replacing invalid UTF-8 sequences.
pub fn unquote_lossy(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    String::from_utf8_lossy(&decode_bytes(text)).into_owned()
}

/// Percent-encode every byte except ASCII letters, digits, `_.-~` and the extra safe bytes.
pub fn quote(text: &str, safe: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || b"_.-~".contains(&byte)
            || (byte.is_ascii() && safe.as_bytes().contains(&byte))
        {
            out.push(byte as char);
        } else {
            push_escaped(&mut out, byte);
        }
    }
    out
}
