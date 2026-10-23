//! The `mdlink-audit-corpus` program: Git-backed checkout records, scopes and output handling.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Json, Output, TempDir, corpus, parse_json, write};
use mdlink_audit::sha256::hex_digest;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=", "-C"])
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A committed checkout whose own configuration would hide every link if it were loaded.
fn checkout(dir: &TempDir) -> PathBuf {
    let repo = dir.path().join("checkout");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    write(&repo, "README.md", "# Hello\n[self](#hello)\n");
    write(&repo, ".mdlink-audit.toml", "ignore_links = [\"*\"]\n");
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "Seed",
        ],
    );
    repo
}

fn run_corpus(cwd: &Path, args: &[&str]) -> (Output, Json) {
    let result = corpus(cwd, args);
    let body = result.json();
    (result, body)
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn test_corpus_records_commit_clean_state_and_markdown_fingerprint() {
    let dir = TempDir::new("corpus-record");
    let repo = checkout(&dir);
    let (result, body) = run_corpus(dir.path(), &[path(&repo)]);
    assert_eq!(result.code, 0);
    let snapshot = body.get("checkouts").at(0);
    assert_eq!(snapshot.get("commit").str(), git(&repo, &["rev-parse", "HEAD"]));
    assert_eq!(snapshot.get("worktree_clean"), &Json::Bool(true));
    assert_eq!(snapshot.get("report").get("summary").get("links_checked").int(), 1);
    let files = snapshot.get("markdown_files").items();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].get("file").str(), "README.md");
    assert_eq!(
        files[0].get("sha256").str(),
        hex_digest(&std::fs::read(repo.join("README.md")).unwrap())
    );
    let tool = body.get("tool");
    assert_eq!(
        (tool.get("name").str(), tool.get("version").str()),
        ("mdlink-audit", "0.2.0")
    );
    assert!(matches!(tool, Json::Object(pairs) if pairs.len() == 2));
    assert_eq!(body.get("configuration").get("include_html"), &Json::Bool(false));
    let (_, repeated) = run_corpus(dir.path(), &[path(&repo)]);
    assert_eq!(repeated, body);
}

#[test]
fn test_corpus_reports_modified_worktree_and_does_not_load_checkout_config() {
    let dir = TempDir::new("corpus-modified");
    let repo = checkout(&dir);
    write(&repo, "README.md", "[bad](missing.md)");
    let (result, body) = run_corpus(dir.path(), &[path(&repo)]);
    assert_eq!(result.code, 1);
    let snapshot = body.get("checkouts").at(0);
    assert_eq!(snapshot.get("worktree_clean"), &Json::Bool(false));
    assert_eq!(
        snapshot.get("report").get("issues").at(0).get("code").str(),
        "missing_file"
    );
    assert_eq!(snapshot.get("report").get("skipped").get("ignored").int(), 0);
}

#[test]
fn test_corpus_html_is_opt_in_and_configuration_is_recorded() {
    let dir = TempDir::new("corpus-html");
    let repo = checkout(&dir);
    write(&repo, "README.md", "<a href=\"missing.md\">Missing</a>\n");
    let (default, baseline) = run_corpus(dir.path(), &[path(&repo)]);
    assert_eq!(default.code, 0);
    assert_eq!(baseline.get("configuration").get("include_html"), &Json::Bool(false));
    assert_eq!(
        baseline
            .get("checkouts")
            .at(0)
            .get("report")
            .get("summary")
            .get("links_checked")
            .int(),
        0
    );

    let (enabled, expanded) = run_corpus(dir.path(), &[path(&repo), "--include-html"]);
    assert_eq!(enabled.code, 1);
    assert_eq!(expanded.get("configuration").get("include_html"), &Json::Bool(true));
    let issue = expanded.get("checkouts").at(0).get("report").get("issues").at(0);
    assert_eq!(
        (issue.get("code").str(), issue.get("target").str()),
        ("missing_file", "missing.md")
    );
    assert_eq!(
        expanded.get("checkouts").at(0).get("markdown_manifest_sha256"),
        baseline.get("checkouts").at(0).get("markdown_manifest_sha256")
    );
}

#[test]
fn test_corpus_continues_after_invalid_checkout_and_returns_operational_error() {
    let dir = TempDir::new("corpus-invalid");
    let repo = checkout(&dir);
    let missing = dir.path().join("missing");
    let (result, body) = run_corpus(dir.path(), &[path(&missing), path(&repo)]);
    assert_eq!(result.code, 2);
    assert!(body.get("checkouts").at(0).has("error"));
    assert_eq!(body.get("checkouts").at(1).get("exit_code").int(), 0);
}

#[test]
fn test_corpus_scope_and_file_output() {
    let dir = TempDir::new("corpus-output");
    let repo = checkout(&dir);
    let output = dir.path().join("audit.json");
    let result = corpus(
        dir.path(),
        &[path(&repo), "--scope", "README.md", "--output", path(&output)],
    );
    assert_eq!(result.code, 0);
    assert_eq!(result.stdout, "");
    let written = parse_json(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let scope = written.get("checkouts").at(0).get("scope").items();
    assert_eq!(scope, [Json::Str("README.md".to_string())]);
}

#[test]
fn test_corpus_does_not_overwrite_files_in_audited_checkout() {
    let dir = TempDir::new("corpus-overwrite");
    let repo = checkout(&dir);
    let readme = repo.join("README.md");
    let original = std::fs::read(&readme).unwrap();
    let result = corpus(dir.path(), &[path(&repo), "--output", path(&readme)]);
    assert_eq!(result.code, 2);
    assert!(
        result.stderr.contains("outside every audited checkout"),
        "{}",
        result.stderr
    );
    assert_eq!(std::fs::read(&readme).unwrap(), original);
}

#[test]
fn test_corpus_rejects_scopes_outside_checkout() {
    for scope in ["../", "../outside.md"] {
        let dir = TempDir::new("corpus-scope");
        let repo = checkout(&dir);
        let (result, body) = run_corpus(dir.path(), &[path(&repo), "--scope", scope]);
        assert_eq!(result.code, 2, "{scope}");
        assert!(
            body.get("checkouts").at(0).get("error").str().contains("outside"),
            "{scope}"
        );
    }
}
