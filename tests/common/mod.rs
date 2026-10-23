//! Shared helpers for the integration tests: temporary directories, running the three programs, and a
//! small JSON reader for their reports.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A directory under the system temporary directory, removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("mdlink-test-{}-{unique}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary directory");
        TempDir(path.canonicalize().expect("canonical temporary directory"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write `text` to `root/relative`, creating parent directories, and return the path.
pub fn write(root: &Path, relative: &str, text: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directories");
    }
    std::fs::write(&path, text).expect("write file");
    path
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn json(&self) -> Json {
        parse_json(&self.stdout).unwrap_or_else(|| panic!("not JSON: {}", self.stdout))
    }
}

/// Run `program` in `cwd` with `args`; `env` replaces the whole environment when given.
pub fn run(program: &str, cwd: &Path, args: &[&str], env: Option<&HashMap<String, String>>) -> Output {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if let Some(env) = env {
        command.env_clear().envs(env);
    }
    let output = command.output().expect("start program");
    Output {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The program the tests run: the one named by `variable` when it is set, otherwise the one Cargo built.
fn program(variable: &str, built: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| built.to_string())
}

pub fn cli(cwd: &Path, args: &[&str]) -> Output {
    let program = program("MDLINK_AUDIT_BIN", env!("CARGO_BIN_EXE_mdlink-audit"));
    run(&program, cwd, args, None)
}

pub fn action(cwd: &Path, env: &HashMap<String, String>) -> Output {
    let program = program("MDLINK_AUDIT_ACTION_BIN", env!("CARGO_BIN_EXE_mdlink-audit-action"));
    run(&program, cwd, &[], Some(env))
}

pub fn corpus(cwd: &Path, args: &[&str]) -> Output {
    let program = program("MDLINK_AUDIT_CORPUS_BIN", env!("CARGO_BIN_EXE_mdlink-audit-corpus"));
    run(&program, cwd, args, None)
}

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(pairs) => pairs
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value)
                .unwrap_or_else(|| panic!("no key {key} in {self:?}")),
            _ => panic!("not an object: {self:?}"),
        }
    }

    pub fn has(&self, key: &str) -> bool {
        matches!(self, Json::Object(pairs) if pairs.iter().any(|(name, _)| name == key))
    }

    pub fn at(&self, index: usize) -> &Json {
        match self {
            Json::Array(items) => &items[index],
            _ => panic!("not an array: {self:?}"),
        }
    }

    pub fn items(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            _ => panic!("not an array: {self:?}"),
        }
    }

    pub fn str(&self) -> &str {
        match self {
            Json::Str(text) => text,
            _ => panic!("not a string: {self:?}"),
        }
    }

    pub fn int(&self) -> i64 {
        match self {
            Json::Number(number) => *number as i64,
            _ => panic!("not a number: {self:?}"),
        }
    }
}

struct Reader<'a> {
    text: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.pos < self.text.len() && b" \t\r\n".contains(&self.text[self.pos]) {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        let rest = &self.text[self.pos..];
        match *rest.first()? {
            b'{' => {
                self.pos += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.text.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return Some(Json::Object(pairs));
                }
                loop {
                    self.ws();
                    let Json::Str(key) = self.value()? else { return None };
                    self.ws();
                    if self.text.get(self.pos) != Some(&b':') {
                        return None;
                    }
                    self.pos += 1;
                    pairs.push((key, self.value()?));
                    self.ws();
                    match self.text.get(self.pos)? {
                        b',' => self.pos += 1,
                        b'}' => {
                            self.pos += 1;
                            return Some(Json::Object(pairs));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.text.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Some(Json::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    match self.text.get(self.pos)? {
                        b',' => self.pos += 1,
                        b']' => {
                            self.pos += 1;
                            return Some(Json::Array(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => {
                self.pos += 1;
                let mut out: Vec<u8> = Vec::new();
                loop {
                    let byte = *self.text.get(self.pos)?;
                    self.pos += 1;
                    match byte {
                        b'"' => return String::from_utf8(out).ok().map(Json::Str),
                        b'\\' => {
                            let escape = *self.text.get(self.pos)?;
                            self.pos += 1;
                            let ch = match escape {
                                b'n' => '\n',
                                b'r' => '\r',
                                b't' => '\t',
                                b'b' => '\u{8}',
                                b'f' => '\u{c}',
                                b'u' => {
                                    let hex = std::str::from_utf8(self.text.get(self.pos..self.pos + 4)?).ok()?;
                                    self.pos += 4;
                                    char::from_u32(u32::from_str_radix(hex, 16).ok()?).unwrap_or('\u{fffd}')
                                }
                                other => other as char,
                            };
                            let mut buffer = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
            }
            _ if rest.starts_with(b"true") => {
                self.pos += 4;
                Some(Json::Bool(true))
            }
            _ if rest.starts_with(b"false") => {
                self.pos += 5;
                Some(Json::Bool(false))
            }
            _ if rest.starts_with(b"null") => {
                self.pos += 4;
                Some(Json::Null)
            }
            _ => {
                let start = self.pos;
                while self.pos < self.text.len() && b"+-.eE0123456789".contains(&self.text[self.pos]) {
                    self.pos += 1;
                }
                std::str::from_utf8(&self.text[start..self.pos])
                    .ok()?
                    .parse()
                    .ok()
                    .map(Json::Number)
            }
        }
    }
}

/// Parse one JSON document; `None` when the text is not exactly one JSON value.
pub fn parse_json(text: &str) -> Option<Json> {
    let mut reader = Reader {
        text: text.as_bytes(),
        pos: 0,
    };
    let value = reader.value()?;
    reader.ws();
    (reader.pos == reader.text.len()).then_some(value)
}

/// Codes of the issues of a `--format json` report.
pub fn codes(report: &Json) -> Vec<String> {
    report
        .get("issues")
        .items()
        .iter()
        .map(|issue| issue.get("code").str().to_string())
        .collect()
}
