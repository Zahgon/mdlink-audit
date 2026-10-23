//! The GitHub Action entry point: input parsing, the audit command line and the program's exit status.

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::{TempDir, action, write};
use mdlink_audit::action::{annotation_escape, audit_command, parse_boolean, parse_lines, resolve_path};

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// A caller workspace and a downloaded action directory, as the runner provides them.
struct ActionEnv {
    _dir: TempDir,
    vars: HashMap<String, String>,
}

fn action_env() -> ActionEnv {
    let dir = TempDir::new("action");
    let workspace = dir.path().join("caller repo 空格");
    std::fs::create_dir(&workspace).unwrap();
    let action_path = dir.path().join("downloaded action");
    write(&action_path, "Cargo.toml", "[package]\n");
    let vars = env(&[
        ("GITHUB_WORKSPACE", workspace.to_str().unwrap()),
        ("MDLINK_ACTION_PATH", action_path.to_str().unwrap()),
    ]);
    ActionEnv { _dir: dir, vars }
}

fn command(pairs: &[(&str, &str)], workspace: &Path) -> Vec<String> {
    audit_command(&env(pairs), workspace).expect("audit command")
}

#[test]
fn test_line_inputs_preserve_spaces_and_literal_shell_text() {
    assert_eq!(
        parse_lines(
            " README.md\r\n\r\n docs/my guide.md \n$(touch marker).md\n'quoted'.md\n",
            "paths"
        )
        .unwrap(),
        vec!["README.md", "docs/my guide.md", "$(touch marker).md", "'quoted'.md"]
    );
}

#[test]
fn test_line_inputs_reject_nul() {
    for value in ["docs\0folder", "README.md\n\0"] {
        let error = parse_lines(value, "paths").unwrap_err();
        assert!(error.message().contains("NUL"), "{}", error.message());
    }
}

#[test]
fn test_boolean_inputs() {
    for (value, default, expected) in [
        ("true", None, Some(true)),
        (" FALSE ", Some(true), Some(false)),
        ("", None, None),
        ("\n", Some(true), Some(true)),
    ] {
        assert_eq!(
            parse_boolean(value, "include-html", default).unwrap(),
            expected,
            "{value:?}"
        );
    }
}

#[test]
fn test_boolean_inputs_reject_ambiguous_values() {
    for value in ["yes", "0", "1", "true\nfalse", "false; echo bad"] {
        let error = parse_boolean(value, "include-html", None).unwrap_err();
        assert_eq!(
            error.message(),
            "include-html must be true, false, or empty",
            "{value:?}"
        );
    }
}

#[test]
fn test_default_command_scans_workspace_fails_on_empty_and_follows_config() {
    let dir = TempDir::new("default-command");
    assert_eq!(
        command(&[], dir.path()),
        vec![
            "--format".to_string(),
            "github".to_string(),
            format!("--root={}", dir.path().display()),
            "--fail-on-empty".to_string()
        ]
    );
}

#[test]
fn test_paths_and_patterns_cannot_supply_options_or_shell_commands() {
    let dir = TempDir::new("no-injection");
    let built = command(
        &[
            (
                "MDLINK_INPUT_PATHS",
                "docs/my guide.md\n--format\njson\n$(touch marker).md",
            ),
            ("MDLINK_INPUT_EXCLUDE", "--format\ngenerated docs/**"),
            ("MDLINK_INPUT_IGNORE_LINKS", "--config=other.toml\n$(echo bad)/*"),
        ],
        dir.path(),
    );
    for expected in [
        "--exclude=--format",
        "--exclude=generated docs/**",
        "--ignore-link=--config=other.toml",
        "--ignore-link=$(echo bad)/*",
    ] {
        assert!(built.iter().any(|arg| arg == expected), "{expected}");
    }
    let separator = built.iter().position(|arg| arg == "--").unwrap();
    assert_eq!(
        built[separator + 1..],
        ["docs/my guide.md", "--format", "json", "$(touch marker).md"]
    );
    assert_eq!(built[0], "--format");
}

#[test]
fn test_relative_root_and_config_use_workspace_without_changing_paths() {
    let dir = TempDir::new("relative-root");
    std::fs::create_dir(dir.path().join("nested root")).unwrap();
    let built = command(
        &[
            ("MDLINK_INPUT_ROOT", "nested root"),
            ("MDLINK_INPUT_CONFIG", "config files/audit.toml"),
            ("MDLINK_INPUT_PATHS", "nested root/docs"),
        ],
        dir.path(),
    );
    assert!(built.contains(&format!("--root={}", dir.path().join("nested root").display())));
    assert!(built.contains(&format!(
        "--config={}",
        dir.path().join("config files").join("audit.toml").display()
    )));
    assert_eq!(built[built.len() - 2..], ["--", "nested root/docs"]);
}

#[test]
fn test_absolute_root_and_config_remain_absolute() {
    let dir = TempDir::new("absolute-root");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let config = dir.path().join("audit.toml").to_str().unwrap().to_string();
    let built = command(
        &[("MDLINK_INPUT_ROOT", &root), ("MDLINK_INPUT_CONFIG", &config)],
        &workspace,
    );
    assert!(built.contains(&format!("--root={root}")));
    assert!(built.contains(&format!("--config={config}")));
}

#[test]
fn test_explicit_html_overrides_and_empty_scan_opt_out() {
    for (value, flag) in [("true", "--include-html"), ("false", "--no-include-html")] {
        let dir = TempDir::new("html-override");
        let built = command(
            &[
                ("MDLINK_INPUT_INCLUDE_HTML", value),
                ("MDLINK_INPUT_FAIL_ON_EMPTY", "false"),
            ],
            dir.path(),
        );
        assert!(built.iter().any(|arg| arg == flag), "{flag}");
        assert!(!built.iter().any(|arg| arg == "--fail-on-empty"));
    }
}

#[test]
fn test_single_path_inputs_reject_newlines_and_nul() {
    let dir = TempDir::new("single-path");
    for value in ["some\nother", "some\rother", "some\0other"] {
        let error = resolve_path(value, dir.path(), "root").unwrap_err();
        assert!(error.message().contains("single path"), "{}", error.message());
    }
}

#[test]
fn test_install_and_audit_use_distinct_directories_and_preserve_exit_status() {
    // No installation step: the audit runs in the caller's workspace, and its exit status is the program's.
    let setup = action_env();
    let workspace = Path::new(&setup.vars["GITHUB_WORKSPACE"]);
    write(workspace, "README.md", "[bad](missing.md)\n");
    let elsewhere = TempDir::new("action-cwd");
    let result = action(elsewhere.path(), &setup.vars);
    assert_eq!(result.code, 1);
    assert!(
        result
            .stdout
            .starts_with("::error file=README.md,line=1::[missing_file]"),
        "{}",
        result.stdout
    );
    write(workspace, "README.md", "# Fine\n");
    assert_eq!(action(elsewhere.path(), &setup.vars).code, 0);
}

#[test]
fn test_install_failure_does_not_run_checker() {
    // The Rust action has nothing to install; an action directory without Cargo.toml stops before any audit.
    let setup = action_env();
    let workspace = Path::new(&setup.vars["GITHUB_WORKSPACE"]);
    write(workspace, "README.md", "[bad](missing.md)\n");
    std::fs::remove_file(Path::new(&setup.vars["MDLINK_ACTION_PATH"]).join("Cargo.toml")).unwrap();
    let result = action(workspace, &setup.vars);
    assert_eq!(result.code, 2);
    assert_eq!(
        result.stdout,
        "::error::MDLINK_ACTION_PATH does not contain the action's Cargo.toml\n"
    );
}

#[test]
fn test_invalid_configuration_fails_before_install() {
    for (name, value) in [
        ("MDLINK_INPUT_FAIL_ON_EMPTY", "off"),
        ("MDLINK_INPUT_INCLUDE_HTML", "yes"),
        ("MDLINK_INPUT_ROOT", "missing directory"),
        ("GITHUB_WORKSPACE", ""),
        ("MDLINK_ACTION_PATH", ""),
    ] {
        let mut setup = action_env();
        write(
            Path::new(&setup.vars["GITHUB_WORKSPACE"]),
            "README.md",
            "[bad](missing.md)\n",
        );
        setup.vars.insert(name.to_string(), value.to_string());
        let cwd = TempDir::new("action-invalid");
        let result = action(cwd.path(), &setup.vars);
        assert_eq!(result.code, 2, "{name}");
        assert!(result.stdout.starts_with("::error::"), "{name}: {}", result.stdout);
        assert!(!result.stdout.contains("missing_file"), "{name}");
    }
}

#[test]
fn test_python_version_error_is_actionable() {
    // The Rust action has no interpreter version check: a valid environment audits directly.
    let setup = action_env();
    write(Path::new(&setup.vars["GITHUB_WORKSPACE"]), "README.md", "# Fine\n");
    let result = action(Path::new(&setup.vars["GITHUB_WORKSPACE"]), &setup.vars);
    assert_eq!(result.code, 0);
    assert!(!result.stdout.contains("setup-python"));
}

#[test]
fn test_action_errors_cannot_inject_workflow_commands() {
    assert_eq!(
        format!("::error::{}", annotation_escape("bad%\r\n::warning::forged")),
        "::error::bad%25%0D%0A::warning::forged"
    );
    let mut setup = action_env();
    setup.vars.insert(
        "MDLINK_INPUT_INCLUDE_HTML".to_string(),
        "x\n::warning::forged".to_string(),
    );
    let result = action(Path::new(&setup.vars["GITHUB_WORKSPACE"]), &setup.vars);
    assert_eq!(result.stdout.lines().count(), 1);
    let mut setup = action_env();
    setup.vars.insert("MDLINK_INPUT_ROOT".to_string(), "100%".to_string());
    let workspace = setup.vars["GITHUB_WORKSPACE"].clone();
    let result = action(Path::new(&workspace), &setup.vars);
    assert_eq!(result.code, 2);
    assert_eq!(
        result.stdout,
        format!(
            "::error::root is not a directory: {}/100%25\n",
            annotation_escape(&workspace)
        )
    );
}
