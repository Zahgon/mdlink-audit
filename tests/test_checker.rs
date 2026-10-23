//! Link resolution, issue codes, suggestions and configuration through the library API.

mod common;

use std::path::{Path, PathBuf};

use common::{TempDir, write};
use mdlink_audit::checker::{Auditor, Report, discover_root};
use mdlink_audit::config::{Config, load_config};

/// A repository with `docs/guide.md` holding three headings.
fn repo(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    write(dir.path(), "docs/guide.md", "# Install\n\n## 中文安装\n\n## Install\n");
    dir
}

fn check_with(root: &Path, text: &str, config: Config) -> Report {
    let source = write(root, "README.md", text);
    Auditor::new(root, config).run(Some(vec![source])).expect("audit")
}

fn check(root: &Path, text: &str) -> Report {
    check_with(root, text, Config::default())
}

fn run_all(root: &Path, config: Config) -> Report {
    Auditor::new(root, config).run(None).expect("audit")
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn test_relative_root_and_cross_file_anchors() {
    let dir = repo("cross-file");
    let report = check(
        dir.path(),
        "# Intro\n[a](docs/guide.md#install)\n[b](/docs/guide.md#中文安装)\n\
         [c](#intro)\n[d](docs/guide.md#install-1)\n[e](./docs/)\n",
    );
    assert!(report.issues.is_empty(), "{:?}", report.issues);
    assert_eq!(report.files_scanned, 1);
    assert_eq!(report.links_checked, 5);
}

#[test]
fn test_parent_relative_links() {
    let dir = repo("parent");
    write(dir.path(), "README.md", "# Home");
    write(dir.path(), "docs/guide.md", "[home](../README.md#home)");
    assert!(run_all(dir.path(), Config::default()).issues.is_empty());
}

#[test]
fn test_broken_links_have_specific_codes_and_source_lines() {
    let cases = [
        ("missing.md", "missing_file"),
        ("docs/guide.md#typo", "missing_anchor"),
        ("docs/Guide.md", "case_mismatch"),
        ("Docs/guide.md", "case_mismatch"),
        ("../outside.md", "outside_root"),
        ("docs/../../outside.md", "outside_root"),
        ("docs/guide.md/", "not_directory"),
        ("docs/guide.md/child.md", "missing_file"),
        ("docs/%00bad.md", "invalid_target"),
        ("docs/%FFbad.md", "invalid_target"),
        ("docs%5Cguide.md", "invalid_target"),
    ];
    for (target, code) in cases {
        let dir = repo("codes");
        let report = check(dir.path(), &format!("# Example\n\nSome text\n[link]({target})\n"));
        assert_eq!(report.issues.len(), 1, "{target}");
        let issue = &report.issues[0];
        assert_eq!(
            (issue.code, issue.file.as_str(), issue.line),
            (code, "README.md", 4),
            "{target}"
        );
    }
}

#[test]
fn test_non_markdown_fragments_and_query_strings() {
    let dir = repo("fragments");
    write(dir.path(), "diagram.svg", "<svg/>");
    let report = check(
        dir.path(),
        "![svg](diagram.svg#view)\n[guide](docs/guide.md?raw=1#install)",
    );
    assert!(report.issues.is_empty());
    assert_eq!(report.links_checked, 2);
}

#[test]
fn test_external_links_skipped_without_network() {
    let dir = repo("external");
    let report = check(
        dir.path(),
        "[https](https://invalid.example)\n[http](http://invalid.example)\n\
         [mail](mailto:x@example.com)\n[proto](//example.com/no.md)\n\
         [ssh](ssh://example.com)\n",
    );
    assert!(report.issues.is_empty());
    assert_eq!(report.links_checked, 0);
    assert_eq!((report.skipped_external, report.skipped_ignored), (5, 0));
}

#[test]
fn test_excludes_are_root_relative_and_do_not_hide_targets() {
    let dir = repo("excludes");
    write(dir.path(), "docs/guide.md", "# Install\n[bad](absent.md)");
    write(dir.path(), "README.md", "[guide](docs/guide.md#install)");
    let config = Config {
        exclude: strings(&["docs/**"]),
        ..Config::default()
    };
    let report = run_all(dir.path(), config);
    assert_eq!(report.files_scanned, 1);
    assert!(report.issues.is_empty());
}

#[test]
fn test_hidden_github_docs_scanned_but_dependency_directories_excluded() {
    let dir = repo("hidden");
    for folder in [".github", "node_modules", ".venv", "docs/node_modules"] {
        write(dir.path(), &format!("{folder}/README.md"), "[bad](missing.md)");
    }
    let report = run_all(dir.path(), Config::default());
    let files: Vec<&str> = report.issues.iter().map(|issue| issue.file.as_str()).collect();
    assert_eq!(files, vec![".github/README.md"]);
}

#[test]
fn test_ignore_link_decoded_and_encoded_forms() {
    let dir = repo("ignore-forms");
    let config = Config {
        ignore_links: strings(&["不存在.md"]),
        ..Config::default()
    };
    let report = check_with(dir.path(), "[one](不存在.md)\n[two](another.md)", config);
    assert_eq!(report.skipped_ignored, 1);
    assert_eq!(report.links_checked, 1);
    assert_eq!(report.issues[0].target, "another.md");
}

#[test]
fn test_overlapping_inputs_are_deduplicated() {
    let dir = repo("overlap");
    let root = dir.path();
    let inputs = vec![
        root.to_path_buf(),
        root.join("docs"),
        root.join("docs").join("guide.md"),
    ];
    let report = Auditor::new(root, Config::default()).run(Some(inputs)).unwrap();
    assert_eq!(report.files_scanned, 1);
}

#[test]
fn test_empty_repository_has_valid_report() {
    let dir = TempDir::new("empty-repo");
    let report = run_all(dir.path(), Config::default());
    assert_eq!(report.files_scanned, 0);
    assert!(report.issues.is_empty());
}

#[test]
fn test_invalid_explicit_inputs_raise() {
    for filename in ["missing.md", "not-markdown.txt"] {
        let dir = repo("invalid-inputs");
        if filename.ends_with(".txt") {
            write(dir.path(), filename, "x");
        }
        let result = Auditor::new(dir.path(), Config::default()).run(Some(vec![dir.path().join(filename)]));
        assert!(result.is_err(), "{filename}");
    }
}

#[test]
fn test_outside_input_rejected_even_if_it_exists() {
    let dir = repo("outside-input");
    let parent: PathBuf = dir.path().parent().unwrap().to_path_buf();
    let error = Auditor::new(dir.path(), Config::default())
        .run(Some(vec![parent]))
        .unwrap_err();
    assert!(error.message().contains("outside"), "{}", error.message());
}

#[test]
fn test_bad_utf8_is_operational_error() {
    let dir = repo("bad-utf8");
    std::fs::write(dir.path().join("bad.md"), b"\xff\xfe\x00").unwrap();
    let error = Auditor::new(dir.path(), Config::default()).run(None).unwrap_err();
    assert!(error.message().contains("Cannot read"), "{}", error.message());
}

#[test]
fn test_utf8_bom_readable() {
    let dir = repo("bom");
    write(dir.path(), "README.md", "\u{feff}# Intro\n[self](#intro)");
    assert!(run_all(dir.path(), Config::default()).issues.is_empty());
}

#[test]
fn test_git_root_discovery_including_worktree_git_file() {
    let dir = repo("git-file");
    write(dir.path(), ".git", "gitdir: elsewhere");
    assert_eq!(discover_root(&dir.path().join("docs")), dir.path());
}

#[test]
fn test_non_git_root_defaults_to_start() {
    let dir = repo("no-git");
    assert_eq!(discover_root(&dir.path().join("docs")), dir.path().join("docs"));
}

#[test]
fn test_configuration_loads_arrays() {
    let dir = TempDir::new("config-arrays");
    let path = write(
        dir.path(),
        "config.toml",
        "exclude = [\"generated/**\"]\nignore_links = [\"/api/*\"]",
    );
    let config = load_config(&path, false).unwrap();
    assert_eq!(config.exclude, strings(&["generated/**"]));
    assert_eq!(config.ignore_links, strings(&["/api/*"]));
    assert!(!config.include_html);
}

#[test]
fn test_configuration_rejects_mistakes() {
    for content in ["exclude = \"docs\"", "unknown = []", "exclude = [1]", "[oops"] {
        let dir = TempDir::new("config-mistakes");
        let path = write(dir.path(), "config.toml", content);
        assert!(load_config(&path, false).is_err(), "{content}");
    }
}

#[test]
fn test_explicit_missing_config_fails() {
    let dir = TempDir::new("config-missing");
    let path = dir.path().join("missing.toml");
    let config = load_config(&path, false).unwrap();
    assert!(config.exclude.is_empty() && config.ignore_links.is_empty() && !config.include_html);
    assert!(load_config(&path, true).is_err());
}

#[test]
fn test_missing_anchor_suggestion_is_specific_and_preserves_query() {
    let dir = repo("suggest-anchor");
    let report = check(dir.path(), "[guide](docs/guide.md?raw=1#instal)");
    assert_eq!(
        report.issues[0].suggestion.as_deref(),
        Some("docs/guide.md?raw=1#install")
    );
}

#[test]
fn test_missing_file_suggestion_uses_existing_sibling() {
    let dir = repo("suggest-file");
    let report = check(dir.path(), "[guide](docs/guied.md#install)");
    assert_eq!(report.issues[0].suggestion.as_deref(), Some("docs/guide.md#install"));
}

#[test]
fn test_case_suggestion_uses_repository_root_link_style() {
    let dir = repo("suggest-case");
    let report = check(dir.path(), "[guide](/Docs/Guide.md#install)");
    assert_eq!(report.issues[0].suggestion.as_deref(), Some("/docs/guide.md#install"));
}

#[test]
fn test_unrelated_missing_anchor_has_no_suggestion() {
    let dir = repo("suggest-none");
    let report = check(dir.path(), "[guide](docs/guide.md#xxxxxxxxx)");
    assert_eq!(report.issues[0].suggestion, None);
}

#[test]
fn test_reusing_auditor_after_a_fix_refreshes_document_and_directory_cache() {
    let dir = repo("reuse");
    let source = write(dir.path(), "README.md", "[new](new.md#changed)");
    let mut auditor = Auditor::new(dir.path(), Config::default());
    assert_eq!(
        auditor.run(Some(vec![source.clone()])).unwrap().issues[0].code,
        "missing_file"
    );
    write(dir.path(), "new.md", "# Changed\n");
    assert!(auditor.run(Some(vec![source.clone()])).unwrap().issues.is_empty());
    write(dir.path(), "new.md", "# Something Else\n");
    assert_eq!(
        auditor.run(Some(vec![source])).unwrap().issues[0].code,
        "missing_anchor"
    );
}
