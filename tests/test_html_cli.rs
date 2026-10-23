//! Raw HTML auditing through the command line and the configuration file.

mod common;

use std::path::Path;

use common::{Output, TempDir, codes, write};
use mdlink_audit::config::load_config;

fn run_cli(root: &Path, args: &[&str]) -> Output {
    let mut all = vec!["--format", "json"];
    all.extend_from_slice(args);
    common::cli(root, &all)
}

#[test]
fn test_html_is_opt_in_and_reports_broken_image() {
    let dir = TempDir::new("html-opt-in");
    write(dir.path(), "README.md", "<img src=\"missing.svg\">\n");
    let default = run_cli(dir.path(), &[]);
    assert_eq!(default.code, 0);
    assert_eq!(default.json().get("summary").get("links_checked").int(), 0);
    let enabled = run_cli(dir.path(), &["--include-html"]);
    assert_eq!(enabled.code, 1);
    let report = enabled.json();
    let issues = report.get("issues").items();
    assert_eq!(issues.len(), 1);
    let issue = &issues[0];
    assert_eq!(
        (
            issue.get("code").str(),
            issue.get("target").str(),
            issue.get("line").int()
        ),
        ("missing_file", "missing.svg", 1)
    );
}

#[test]
fn test_html_config_and_explicit_cli_override() {
    let dir = TempDir::new("html-config");
    write(dir.path(), "README.md", "<a href=\"missing.md\">bad</a>\n");
    write(dir.path(), ".mdlink-audit.toml", "include_html = true\n");
    assert_eq!(run_cli(dir.path(), &[]).code, 1);
    assert_eq!(run_cli(dir.path(), &["--no-include-html"]).code, 0);
    write(dir.path(), ".mdlink-audit.toml", "include_html = false\n");
    assert_eq!(run_cli(dir.path(), &["--include-html"]).code, 1);
}

#[test]
fn test_html_config_requires_actual_boolean() {
    for value in ["\"true\"", "1", "[]", "{}"] {
        let dir = TempDir::new("html-boolean");
        let config = write(dir.path(), ".mdlink-audit.toml", &format!("include_html = {value}\n"));
        let error = load_config(&config, false).expect_err(value);
        assert!(error.message().contains("must be a boolean"), "{}", error.message());
    }
}

#[test]
fn test_markdown_links_can_target_opt_in_html_ids_in_other_files() {
    let dir = TempDir::new("html-ids");
    write(dir.path(), "README.md", "[section](guide.md#中文)\n");
    write(dir.path(), "guide.md", "<section id=\"中文\">text</section>\n");
    assert_eq!(run_cli(dir.path(), &["README.md"]).code, 1);
    assert_eq!(run_cli(dir.path(), &["--include-html", "README.md"]).code, 0);
}

#[test]
fn test_html_links_obey_ignores_and_repository_boundary() {
    let dir = TempDir::new("html-boundary");
    let root = dir.path().join("repo");
    write(dir.path(), "outside.md", "# Private\n");
    write(
        &root,
        "README.md",
        "<a href=\"../outside.md#private\">outside</a>\n\
         <img src=\"generated/image.svg\">\n\
         <a href=\"https://example.invalid/\">external</a>\n",
    );
    let result = run_cli(&root, &["--include-html", "--ignore-link=generated/*"]);
    assert_eq!(result.code, 1);
    let report = result.json();
    assert_eq!(codes(&report), vec!["outside_root"]);
    assert_eq!(report.get("skipped").get("ignored").int(), 1);
    assert_eq!(report.get("skipped").get("external").int(), 1);
}
