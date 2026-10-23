//! Path spelling and resolution with the semantics the reports print: lexical normalisation for
//! inputs as given, and symbolic link resolution that tolerates missing components.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// The path as the reports print an argument: empty and `.` segments dropped, `..` kept.
pub fn shown(text: &str) -> String {
    let absolute = text.starts_with('/');
    let parts: Vec<&str> = text
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    match (absolute, parts.is_empty()) {
        (true, _) => format!("/{}", parts.join("/")),
        (false, true) => ".".to_string(),
        (false, false) => parts.join("/"),
    }
}

fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// Absolute against the working directory, with `.` and `..` removed lexically.
pub fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir().join(path)
    };
    let mut out = PathBuf::from("/");
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
            _ => {}
        }
    }
    out
}

/// Resolve every symbolic link along `path`, component by component; a component that does not
/// exist is kept as written and resolution continues with the rest.
pub fn resolve(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let mut resolved = if text.starts_with('/') {
        "/".to_string()
    } else {
        current_dir().to_string_lossy().into_owned()
    };
    let mut pending: Vec<Option<String>> = text.split('/').rev().map(|part| Some(part.to_string())).collect();
    let mut seen: HashMap<String, Option<String>> = HashMap::new();
    while let Some(entry) = pending.pop() {
        let Some(name) = entry else {
            if let Some(Some(link)) = pending.pop() {
                seen.insert(link, Some(resolved.clone()));
            }
            continue;
        };
        if name.is_empty() || name == "." {
            continue;
        }
        if name == ".." {
            let cut = resolved.rfind('/').unwrap_or(0);
            resolved.truncate(cut);
            if resolved.is_empty() {
                resolved.push('/');
            }
            continue;
        }
        let candidate = if resolved == "/" {
            format!("/{name}")
        } else {
            format!("{resolved}/{name}")
        };
        let is_link = std::fs::symlink_metadata(&candidate).is_ok_and(|meta| meta.file_type().is_symlink());
        if !is_link {
            resolved = candidate;
            continue;
        }
        if let Some(known) = seen.get(&candidate) {
            resolved = known.clone().unwrap_or(candidate);
            continue;
        }
        let Ok(target) = std::fs::read_link(&candidate) else {
            resolved = candidate;
            continue;
        };
        let target = target.to_string_lossy().into_owned();
        if target.starts_with('/') {
            resolved = "/".to_string();
        }
        seen.insert(candidate.clone(), None);
        pending.push(Some(candidate));
        pending.push(None);
        pending.extend(target.split('/').rev().map(|part| Some(part.to_string())));
    }
    PathBuf::from(resolved)
}

/// The relative spelling of `target` from the directory `start`, both absolute and normalised.
pub fn relative_from(target: &Path, start: &Path) -> String {
    let target: Vec<_> = target.components().collect();
    let start: Vec<_> = start.components().collect();
    let common = target.iter().zip(start.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); start.len() - common];
    parts.extend(
        target[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_drops_dot_segments_only() {
        assert_eq!(shown("./docs//a/./b/"), "docs/a/b");
        assert_eq!(shown("a/../b"), "a/../b");
        assert_eq!(shown(""), ".");
    }

    #[test]
    fn relative_spelling() {
        assert_eq!(
            relative_from(Path::new("/r/docs/a.md"), Path::new("/r/api")),
            "../docs/a.md"
        );
        assert_eq!(relative_from(Path::new("/r"), Path::new("/r")), ".");
    }
}
