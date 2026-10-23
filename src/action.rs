//! The GitHub Action entry point: turn the action's inputs into one `mdlink-audit` command line and
//! run it in the caller's workspace. No input is interpreted as shell source or as an option.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cli;
use crate::error::AuditError;
use crate::paths::{resolve, shown};
use crate::text;

fn input<'a>(env: &'a HashMap<String, String>, name: &str) -> &'a str {
    env.get(name).map(String::as_str).unwrap_or("")
}

/// Non-empty lines with surrounding whitespace removed; embedded spaces and punctuation are kept.
pub fn parse_lines(value: &str, name: &str) -> Result<Vec<String>, AuditError> {
    if value.contains('\0') {
        return Err(AuditError::new(format!("{name} must not contain a NUL character")));
    }
    Ok(text::lines(value)
        .into_iter()
        .map(text::strip)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// `true` or `false` in any letter case; an empty input takes `default`.
pub fn parse_boolean(value: &str, name: &str, default: Option<bool>) -> Result<Option<bool>, AuditError> {
    match text::strip(value).to_lowercase().as_str() {
        "" => Ok(default),
        "true" => Ok(Some(true)),
        "false" => Ok(Some(false)),
        _ => Err(AuditError::new(format!("{name} must be true, false, or empty"))),
    }
}

pub fn resolve_path(value: &str, base: &Path, name: &str) -> Result<PathBuf, AuditError> {
    if value.contains(['\0', '\n', '\r']) {
        return Err(AuditError::new(format!(
            "{name} must be a single path without NUL or newline characters"
        )));
    }
    let path = PathBuf::from(shown(text::strip(value)));
    Ok(resolve(&if path.is_absolute() { path } else { base.join(path) }))
}

pub fn audit_command(env: &HashMap<String, String>, workspace: &Path) -> Result<Vec<String>, AuditError> {
    let root_input = match input(env, "MDLINK_INPUT_ROOT") {
        "" => ".",
        value => value,
    };
    let root = resolve_path(root_input, workspace, "root")?;
    if !root.is_dir() {
        return Err(AuditError::new(format!("root is not a directory: {}", root.display())));
    }
    let mut command = vec![
        "--format".to_string(),
        "github".to_string(),
        format!("--root={}", root.display()),
    ];
    let config = text::strip(input(env, "MDLINK_INPUT_CONFIG"));
    if !config.is_empty() {
        command.push(format!(
            "--config={}",
            resolve_path(config, workspace, "config")?.display()
        ));
    }
    for (variable, name, option) in [
        ("MDLINK_INPUT_EXCLUDE", "exclude", "--exclude"),
        ("MDLINK_INPUT_IGNORE_LINKS", "ignore-links", "--ignore-link"),
    ] {
        command.extend(
            parse_lines(input(env, variable), name)?
                .into_iter()
                .map(|value| format!("{option}={value}")),
        );
    }
    match parse_boolean(input(env, "MDLINK_INPUT_INCLUDE_HTML"), "include-html", None)? {
        Some(true) => command.push("--include-html".to_string()),
        Some(false) => command.push("--no-include-html".to_string()),
        None => {}
    }
    if parse_boolean(input(env, "MDLINK_INPUT_FAIL_ON_EMPTY"), "fail-on-empty", Some(true))? == Some(true) {
        command.push("--fail-on-empty".to_string());
    }
    let paths = parse_lines(input(env, "MDLINK_INPUT_PATHS"), "paths")?;
    if !paths.is_empty() {
        command.push("--".to_string());
        command.extend(paths);
    }
    Ok(command)
}

fn prepare(env: &HashMap<String, String>) -> Result<(PathBuf, Vec<String>), AuditError> {
    for name in ["GITHUB_WORKSPACE", "MDLINK_ACTION_PATH"] {
        if text::strip(input(env, name)).is_empty() {
            return Err(AuditError::new(format!("{name} is required")));
        }
    }
    let cwd = std::env::current_dir().map_err(|error| AuditError::new(error.to_string()))?;
    let workspace = resolve_path(input(env, "GITHUB_WORKSPACE"), &cwd, "GITHUB_WORKSPACE")?;
    let action_path = resolve_path(input(env, "MDLINK_ACTION_PATH"), &cwd, "MDLINK_ACTION_PATH")?;
    if !workspace.is_dir() {
        return Err(AuditError::new(
            "GITHUB_WORKSPACE is not a directory; check out the repository first",
        ));
    }
    if !action_path.join("Cargo.toml").is_file() {
        return Err(AuditError::new(
            "MDLINK_ACTION_PATH does not contain the action's Cargo.toml",
        ));
    }
    let command = audit_command(env, &workspace)?;
    std::env::set_current_dir(&workspace).map_err(|error| AuditError::new(error.to_string()))?;
    Ok((workspace, command))
}

pub fn annotation_escape(message: &str) -> String {
    message.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

/// Run the action with the given environment and return the exit status.
pub fn run(env: &HashMap<String, String>) -> u8 {
    match prepare(env) {
        Ok((_, command)) => cli::run(&command),
        Err(error) => {
            println!("::error::{}", annotation_escape(error.message()));
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_booleans() {
        assert_eq!(
            parse_lines(" README.md\r\n\r\n docs/my guide.md \n", "paths").unwrap(),
            vec!["README.md", "docs/my guide.md"]
        );
        assert_eq!(parse_boolean(" FALSE ", "x", Some(true)).unwrap(), Some(false));
        assert_eq!(parse_boolean("\n", "x", Some(true)).unwrap(), Some(true));
        assert!(parse_boolean("yes", "x", None).is_err());
    }

    #[test]
    fn inputs_never_become_options() {
        let env: HashMap<String, String> = [
            ("MDLINK_INPUT_PATHS", "docs/a b.md\n--format"),
            ("MDLINK_INPUT_EXCLUDE", "--format"),
            ("MDLINK_INPUT_FAIL_ON_EMPTY", "false"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let command = audit_command(&env, Path::new("/")).unwrap();
        assert_eq!(
            command,
            vec![
                "--format",
                "github",
                "--root=/",
                "--exclude=--format",
                "--",
                "docs/a b.md",
                "--format"
            ]
        );
    }
}
