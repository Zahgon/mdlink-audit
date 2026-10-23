//! Filesystem and URL boundaries that commonly differ between local and CI runs.

mod common;

use std::path::Path;

use common::{TempDir, write};
use mdlink_audit::checker::{Auditor, Report};
use mdlink_audit::config::{Config, load_config, matches};

fn audit(root: &Path, source: &Path) -> Report {
    Auditor::new(root, Config::default())
        .run(Some(vec![source.to_path_buf()]))
        .expect("audit")
}

fn codes(report: &Report) -> Vec<&'static str> {
    report.issues.iter().map(|issue| issue.code).collect()
}

#[cfg(unix)]
fn symlink(link: &Path, target: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create symlink");
}

#[cfg(windows)]
fn symlink(link: &Path, target: &Path) {
    let made = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    made.expect("create symlink");
}

/// Create a Windows directory junction; false when the host cannot.
#[cfg(windows)]
fn junction(link: &Path, target: &Path) -> bool {
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn test_unicode_spaces_query_and_fragment_are_resolved_separately() {
    let dir = TempDir::new("unicode-query");
    write(dir.path(), "文档/使用 指南.md", "# 中文 标题\n");
    let source = write(
        dir.path(),
        "README.md",
        "[guide](<文档/使用 指南.md?raw=true#中文-标题>)\n\
         [encoded](%E6%96%87%E6%A1%A3/%E4%BD%BF%E7%94%A8%20%E6%8C%87%E5%8D%97.md\
         ?download=1#%E4%B8%AD%E6%96%87-%E6%A0%87%E9%A2%98)\n",
    );
    let report = audit(dir.path(), &source);
    assert_eq!(report.links_checked, 2);
    assert!(report.issues.is_empty(), "{:?}", report.issues);
}

#[test]
fn test_percent_decoding_happens_exactly_once_for_path_and_anchor() {
    let dir = TempDir::new("percent-once");
    write(dir.path(), "literal%20name.md", "<a name=\"percent%20anchor\"></a>\n");
    write(dir.path(), "hash#file.md", "# Works\n");
    let source = write(
        dir.path(),
        "README.md",
        "[percent](literal%2520name.md#percent%2520anchor)\n[hash](hash%23file.md#works)\n",
    );
    assert!(audit(dir.path(), &source).issues.is_empty());
}

#[test]
fn test_double_encoded_path_does_not_accidentally_match_a_space() {
    let dir = TempDir::new("double-encoded");
    write(dir.path(), "a b.md", "# Works\n");
    let source = write(dir.path(), "README.md", "[bad](a%2520b.md)\n");
    assert_eq!(codes(&audit(dir.path(), &source)), vec!["missing_file"]);
}

#[test]
fn test_empty_path_and_query_only_links_resolve_against_current_document() {
    let dir = TempDir::new("empty-path");
    let source = write(
        dir.path(),
        "docs/guide.md",
        "# Local\n[anchor](#local)\n[query](?raw=1#local)\n[self](./guide.md#local)\n[empty]()\n",
    );
    let report = audit(dir.path(), &source);
    assert_eq!(report.links_checked, 4);
    assert!(report.issues.is_empty());
}

#[test]
fn test_root_relative_paths_resolve_under_repository_not_filesystem_root() {
    let dir = TempDir::new("root-relative");
    write(dir.path(), "README.md", "# Root\n");
    let source = write(dir.path(), "docs/guide.md", "[root](/README.md#root)\n");
    assert!(audit(dir.path(), &source).issues.is_empty());
}

#[test]
fn test_directory_and_non_markdown_fragments_check_only_existence() {
    let dir = TempDir::new("existence");
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::create_dir(dir.path().join("directory.md")).unwrap();
    write(dir.path(), "manual.pdf", "not actually a PDF; existence is sufficient");
    let source = write(
        dir.path(),
        "README.md",
        "[directory](docs/#unverified)\n[md-directory](directory.md#unverified)\n[PDF](manual.pdf#page=9)\n",
    );
    assert!(audit(dir.path(), &source).issues.is_empty());
}

#[test]
fn test_file_target_with_trailing_slash_is_not_a_directory() {
    let dir = TempDir::new("trailing-slash");
    write(dir.path(), "guide.md", "# Guide\n");
    let source = write(dir.path(), "README.md", "[bad](guide.md/)\n");
    assert_eq!(codes(&audit(dir.path(), &source)), vec!["not_directory"]);
}

#[test]
fn test_encoded_parent_directory_escape_is_reported() {
    let dir = TempDir::new("encoded-parent");
    let source = write(dir.path(), "README.md", "[outside](%2e%2e/secret.md)\n");
    assert_eq!(codes(&audit(dir.path(), &source)), vec!["outside_root"]);
}

#[test]
fn test_invalid_local_path_encodings_produce_issues_instead_of_crashing() {
    for target in ["bad%00.md", "docs/%5Csecret.md", "bad%FF.md"] {
        let dir = TempDir::new("invalid-encoding");
        let source = write(dir.path(), "README.md", &format!("[bad]({target})\n"));
        assert_eq!(codes(&audit(dir.path(), &source)), vec!["invalid_target"], "{target}");
    }
}

#[test]
fn test_windows_drive_segments_do_not_escape_or_crash_path_resolution() {
    // Drive-reset behaviour is specific to Windows paths; elsewhere these are ordinary missing names.
    if !cfg!(windows) {
        return;
    }
    for target in ["/Z:/file.md", "docs/Z:/file.md", "/%5A:/file.md"] {
        let dir = TempDir::new("drive");
        let source = write(dir.path(), "README.md", &format!("[bad]({target})\n"));
        assert_eq!(codes(&audit(dir.path(), &source)), vec!["invalid_target"], "{target}");
    }
}

#[test]
fn test_each_path_component_is_case_checked_on_every_platform() {
    let dir = TempDir::new("case-components");
    write(dir.path(), "Docs/Guide.md", "# Start\n");
    let source = write(dir.path(), "README.md", "[bad](docs/guide.md#start)\n");
    let report = audit(dir.path(), &source);
    assert_eq!(codes(&report), vec!["case_mismatch"]);
    assert!(report.issues[0].message.contains("Docs/Guide.md"));
}

#[test]
fn test_external_links_never_require_network_requests() {
    let dir = TempDir::new("external");
    let source = write(
        dir.path(),
        "README.md",
        "[https](https://example.invalid/guide.md#missing)\n[http](http://example.invalid/)\n\
         [protocol-relative](//example.invalid/guide.md)\n[email](mailto:user@example.invalid)\n\
         [custom](custom-protocol:example)\n",
    );
    let report = audit(dir.path(), &source);
    assert_eq!(report.links_checked, 0);
    assert_eq!(report.links_skipped, 5);
    assert_eq!(report.skipped_external, 5);
    assert!(report.issues.is_empty());
}

#[test]
fn test_internal_file_and_directory_symlink_targets_are_checked() {
    let dir = TempDir::new("internal-symlinks");
    let target = write(dir.path(), "docs/guide.md", "# Existing\n");
    symlink(&dir.path().join("guide-alias.md"), &target);
    symlink(&dir.path().join("docs-alias"), &dir.path().join("docs"));
    let source = write(
        dir.path(),
        "README.md",
        "[file](guide-alias.md#existing)\n[directory](docs-alias/guide.md#existing)\n[broken-anchor](guide-alias.md#missing)\n",
    );
    let report = audit(dir.path(), &source);
    assert_eq!(codes(&report), vec!["missing_anchor"]);
    assert_eq!(report.issues[0].line, 3);
}

#[test]
fn test_symlink_targets_outside_root_are_reported_without_reading_them() {
    let dir = TempDir::new("outside-symlinks");
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    let outside = write(dir.path(), "outside/secret.md", "# Secret\n");
    symlink(&root.join("escape.md"), &outside);
    symlink(&root.join("escape-dir"), outside.parent().unwrap());
    let source = write(
        &root,
        "README.md",
        "[file](escape.md#secret)\n[dir](escape-dir/secret.md#secret)\n",
    );
    let report = audit(&root, &source);
    assert_eq!(codes(&report), vec!["outside_root", "outside_root"]);
}

#[test]
fn test_outside_symlink_source_is_an_input_error() {
    let dir = TempDir::new("outside-source");
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    let outside = write(dir.path(), "outside.md", "# Outside\n");
    let alias = root.join("outside.md");
    symlink(&alias, &outside);
    let error = Auditor::new(&root, Config::default())
        .run(Some(vec![alias]))
        .unwrap_err();
    assert!(error.message().contains("outside"), "{}", error.message());
}

#[test]
fn test_dangling_internal_symlink_is_missing_file() {
    let dir = TempDir::new("dangling");
    symlink(&dir.path().join("dangling.md"), &dir.path().join("absent.md"));
    let source = write(dir.path(), "README.md", "[bad](dangling.md)\n");
    assert_eq!(codes(&audit(dir.path(), &source)), vec!["missing_file"]);
}

#[test]
fn test_symlinked_directories_are_not_recursively_scanned() {
    let dir = TempDir::new("symlink-loop");
    write(dir.path(), "docs/guide.md", "# Guide\n");
    symlink(&dir.path().join("docs").join("loop"), dir.path());
    let report = Auditor::new(dir.path(), Config::default()).run(None).unwrap();
    assert_eq!(report.files_scanned, 1);
    assert!(report.issues.is_empty());
}

#[test]
fn test_ignore_patterns_match_unicode_decoded_targets() {
    let dir = TempDir::new("ignore-unicode");
    let source = write(dir.path(), "README.md", "[generated](<文档/生成 文件.md#标题>)\n");
    let config = Config {
        ignore_links: vec!["文档/*".to_string()],
        ..Config::default()
    };
    let report = Auditor::new(dir.path(), config).run(Some(vec![source])).unwrap();
    assert_eq!(report.links_checked, 0);
    assert_eq!(report.skipped_ignored, 1);
    assert!(report.issues.is_empty());
}

#[test]
fn test_excluding_a_source_does_not_exempt_links_pointing_to_it() {
    let dir = TempDir::new("exclude-target");
    write(dir.path(), "generated/output.md", "# Existing\n[bad](absent.md)\n");
    write(dir.path(), "README.md", "[bad](generated/output.md#missing)\n");
    let config = Config {
        exclude: vec!["generated/".to_string()],
        ..Config::default()
    };
    let report = Auditor::new(dir.path(), config).run(None).unwrap();
    assert_eq!(report.files_scanned, 1);
    assert_eq!(codes(&report), vec!["missing_anchor"]);
}

#[test]
fn test_pattern_matching_is_case_sensitive_and_double_star_covers_root() {
    let pattern = vec!["**/*.md".to_string()];
    assert!(matches("README.md", &pattern));
    assert!(matches("docs/README.md", &pattern));
    assert!(!matches("Docs/a.md", &["docs/*".to_string()]));
}

#[test]
fn test_bom_encoded_configuration_and_markdown_are_supported() {
    let dir = TempDir::new("bom-config");
    let config_path = write(
        dir.path(),
        ".mdlink-audit.toml",
        "\u{feff}ignore_links = [\"generated/*\"]\n",
    );
    let source = write(
        dir.path(),
        "README.md",
        "\u{feff}# Start\n[self](#start)\n[generated](generated/a.md)\n",
    );
    let config = load_config(&config_path, false).unwrap();
    let report = Auditor::new(dir.path(), config).run(Some(vec![source])).unwrap();
    assert_eq!(report.links_checked, 1);
    assert_eq!(report.links_skipped, 1);
    assert!(report.issues.is_empty());
}

#[test]
fn test_windows_directory_junction_loop_not_recursively_scanned() {
    #[cfg(windows)]
    {
        let dir = TempDir::new("junction-loop");
        write(dir.path(), "docs/guide.md", "# Guide\n");
        if junction(&dir.path().join("docs").join("loop"), dir.path()) {
            let report = Auditor::new(dir.path(), Config::default()).run(None).unwrap();
            assert_eq!(report.files_scanned, 1);
            assert!(report.issues.is_empty());
        }
    }
}

#[test]
fn test_windows_directory_junction_cannot_read_outside_root() {
    #[cfg(windows)]
    {
        let dir = TempDir::new("junction-outside");
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let outside = write(dir.path(), "outside/secret.md", "# Secret\n");
        if junction(&root.join("escape"), outside.parent().unwrap()) {
            let source = write(&root, "README.md", "[outside](escape/secret.md#secret)\n");
            assert_eq!(codes(&audit(&root, &source)), vec!["outside_root"]);
        }
    }
}
