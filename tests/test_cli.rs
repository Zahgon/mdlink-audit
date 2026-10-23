//! The `mdlink-audit` command line: exit statuses, report formats and configuration handling.

mod common;

use common::{TempDir, cli, write};

#[test]
fn test_cli_json_reports_broken_file_with_exit_one() {
    let dir = TempDir::new("json-broken");
    write(dir.path(), "README.md", "# Hello\n\n[bad](缺失.md)");
    let result = cli(dir.path(), &["--format", "json"]);
    assert_eq!(result.code, 1);
    assert_eq!(result.stderr, "");
    let report = result.json();
    assert_eq!(report.get("schema_version").int(), 1);
    let issue = report.get("issues").at(0);
    assert_eq!(issue.get("code").str(), "missing_file");
    assert_eq!(issue.get("line").int(), 3);
    assert!(issue.get("message").str().contains("缺失.md"));
}

#[test]
fn test_cli_success_and_relative_explicit_inputs() {
    let dir = TempDir::new("relative-inputs");
    write(dir.path(), "README.md", "# Hello\n[self](#hello)");
    let result = cli(dir.path(), &["README.md", "--fail-on-empty"]);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("1 local links checked"));
}

#[test]
fn test_cli_empty_behavior() {
    for fail_on_empty in [false, true] {
        let dir = TempDir::new("empty");
        let mut args = vec!["--format", "json"];
        if fail_on_empty {
            args.push("--fail-on-empty");
        }
        let result = cli(dir.path(), &args);
        assert_eq!(result.code, if fail_on_empty { 2 } else { 0 });
        assert_eq!(result.json().has("error"), fail_on_empty);
    }
}

#[test]
fn test_cli_invalid_root_returns_json_error() {
    let dir = TempDir::new("invalid-root");
    let result = cli(dir.path(), &["--root", "absent", "--format", "json"]);
    assert_eq!(result.code, 2);
    assert!(result.json().get("error").str().contains("not a directory"));
}

#[test]
fn test_cli_config_and_flags_are_additive() {
    let dir = TempDir::new("additive");
    write(dir.path(), "README.md", "[a](first.md)\n[b](second.md)");
    write(dir.path(), ".mdlink-audit.toml", "ignore_links = [\"first.md\"]");
    let result = cli(dir.path(), &["--ignore-link", "second.md", "--format", "json"]);
    assert_eq!(result.code, 0);
    assert_eq!(result.json().get("skipped").get("ignored").int(), 2);
}

#[test]
fn test_cli_exclude_applies_to_selected_files() {
    let dir = TempDir::new("exclude");
    write(dir.path(), "README.md", "[a](missing.md)");
    assert_eq!(cli(dir.path(), &["--exclude", "**/README.md"]).code, 0);
}

#[test]
fn test_cli_bad_config_returns_error_not_traceback() {
    let dir = TempDir::new("bad-config");
    write(dir.path(), ".mdlink-audit.toml", "exclude = false");
    let result = cli(dir.path(), &[]);
    assert_eq!(result.code, 2);
    assert!(result.stderr.contains("array of strings"));
    assert!(!result.stderr.contains("panicked"));
}

#[test]
fn test_cli_explicit_config_not_found() {
    let dir = TempDir::new("config-missing");
    let result = cli(dir.path(), &["--config", "missing.toml"]);
    assert_eq!(result.code, 2);
    assert!(result.stderr.contains("does not exist"));
}

#[test]
fn test_cli_github_output() {
    let dir = TempDir::new("github");
    write(dir.path(), "README.md", "[a](missing.md)");
    let result = cli(dir.path(), &["--format", "github"]);
    assert_eq!(result.code, 1);
    assert!(result.stdout.contains("::error file=README.md,line=1::[missing_file]"));
}

#[test]
fn test_annotation_data_cannot_inject_workflow_commands() {
    let dir = TempDir::new("annotation");
    write(
        dir.path(),
        "docs/a,b:%.md\nname.md",
        "<a href=\"target&#10;::warning::oops\">x</a>\n",
    );
    let result = cli(dir.path(), &["--format", "github", "--include-html"]);
    assert_eq!(result.code, 1);
    let lines: Vec<&str> = result.stdout.lines().collect();
    let annotation = lines[0];
    assert!(annotation.starts_with("::error file="));
    assert!(!annotation.contains('\r'));
    assert!(annotation.contains("a%2Cb%3A%25.md%0Aname.md"));
    assert!(annotation.contains("target%0A::warning::oops"));
    assert!(!lines.iter().any(|line| line.starts_with("::warning::")));
}

#[test]
fn test_cli_nested_working_directory_detects_git_root() {
    let dir = TempDir::new("nested-git");
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    write(dir.path(), "README.md", "# Home");
    write(dir.path(), "docs/guide.md", "[home](/README.md#home)");
    let result = cli(&dir.path().join("docs"), &["guide.md", "--format", "json"]);
    assert_eq!(result.code, 0);
    assert_eq!(result.json().get("root").str(), dir.path().to_str().unwrap());
}

#[test]
fn test_cli_help_and_version() {
    let dir = TempDir::new("help");
    assert!(
        cli(dir.path(), &["--help"])
            .stdout
            .contains("External URLs are skipped")
    );
    let version = cli(dir.path(), &["--version"]);
    assert_eq!(version.code, 0);
    assert_eq!(version.stdout, "mdlink-audit 0.2.0\n");
}
