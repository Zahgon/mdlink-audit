//! The corpus audit (`mdlink-audit-corpus`): audit existing local Git checkouts and record reproducible evidence,
//! never fetching. Each checkout is audited with the built-in defaults only (its own configuration file is not
//! loaded), and the record holds the commit, the worktree state, the selected scope, a SHA-256 manifest of every
//! selected Markdown file and the full report.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::checker::Auditor;
use crate::config::{Config, DEFAULT_IGNORED_DIRS};
use crate::error::AuditError;
use crate::json::Value;
use crate::output::report_value;
use crate::paths::{resolve, shown};
use crate::sha256::hex_digest;
use crate::text::{is_negative_number, strip};

const USAGE: &str =
    "usage: mdlink-audit-corpus [-h] [--scope SCOPE] [--output OUTPUT] [--include-html] checkouts [checkouts ...]";
const OPTIONS: [(&str, bool); 5] = [
    ("-h", false),
    ("--help", false),
    ("--scope", true),
    ("--output", true),
    ("--include-html", false),
];

#[derive(Debug, Default)]
struct Arguments {
    checkouts: Vec<String>,
    scopes: Vec<String>,
    output: Option<String>,
    include_html: bool,
}

enum Arg {
    Value,
    Separator,
    Known(&'static str, Option<String>),
    Unknown,
}

fn classify(arg: &str) -> Result<Arg, AuditError> {
    if !arg.starts_with('-') || arg.len() == 1 {
        return Ok(Arg::Value);
    }
    if let Some((name, _)) = OPTIONS.iter().find(|(name, _)| *name == arg) {
        return Ok(Arg::Known(name, None));
    }
    if let Some((head, value)) = arg.split_once('=')
        && let Some((name, _)) = OPTIONS.iter().find(|(name, _)| *name == head)
    {
        return Ok(Arg::Known(name, Some(value.to_string())));
    }
    let mut found: Vec<(&'static str, Option<String>)> = Vec::new();
    if arg.starts_with("--") {
        let (prefix, explicit) = match arg.split_once('=') {
            Some((prefix, value)) => (prefix, Some(value.to_string())),
            None => (arg, None),
        };
        found.extend(
            OPTIONS
                .iter()
                .filter(|(name, _)| name.starts_with(prefix))
                .map(|(name, _)| (*name, explicit.clone())),
        );
    } else {
        let split = arg.char_indices().nth(2).map_or(arg.len(), |(index, _)| index);
        let (short, rest) = arg.split_at(split);
        for (name, takes_value) in OPTIONS {
            if name == short {
                // A value-less short option followed by a tail that does not start with `-` is a cluster; the
                // option itself acts first.
                let explicit = (takes_value || rest.is_empty() || rest.starts_with('-')).then(|| rest.to_string());
                found.push((name, explicit));
            } else if name.starts_with(arg) {
                found.push((name, None));
            }
        }
    }
    if found.len() > 1 {
        let names: Vec<&str> = found.iter().map(|(name, _)| *name).collect();
        return Err(AuditError::usage(format!(
            "ambiguous option: {arg} could match {}",
            names.join(", ")
        )));
    }
    if let Some((name, explicit)) = found.pop() {
        return Ok(Arg::Known(name, explicit));
    }
    if is_negative_number(arg) || arg.contains(' ') {
        return Ok(Arg::Value);
    }
    Ok(Arg::Unknown)
}

/// Read the command line into [`Arguments`]; `Ok(None)` means help was printed.
fn parse_arguments(raw: &[String]) -> Result<Option<Arguments>, AuditError> {
    let mut kinds = Vec::with_capacity(raw.len());
    let mut separated = false;
    for arg in raw {
        if separated {
            kinds.push(Arg::Value);
        } else if arg == "--" {
            separated = true;
            kinds.push(Arg::Separator);
        } else {
            kinds.push(classify(arg)?);
        }
    }
    let is_option = |kind: &Arg| matches!(kind, Arg::Known(..) | Arg::Unknown);
    let last_option = kinds.iter().rposition(is_option);
    let mut args = Arguments::default();
    let mut positionals_taken = false;
    let mut extras: Vec<String> = Vec::new();
    let mut index = 0;
    let take = |start: usize, args: &mut Arguments| -> usize {
        let mut end = start;
        while end < kinds.len() && matches!(kinds[end], Arg::Value | Arg::Separator) {
            end += 1;
        }
        let mut separator_dropped = false;
        for position in start..end {
            if matches!(kinds[position], Arg::Separator) && !separator_dropped {
                separator_dropped = true;
                continue;
            }
            args.checkouts.push(raw[position].clone());
        }
        end
    };
    while let Some(last) = last_option {
        if index > last {
            break;
        }
        let next_option = (index..=last)
            .find(|&position| is_option(&kinds[position]))
            .unwrap_or(last);
        if index != next_option {
            if !positionals_taken {
                positionals_taken = true;
                index = take(index, &mut args);
                continue;
            }
            extras.extend(raw[index..next_option].iter().cloned());
            index = next_option;
        }
        let Arg::Known(name, explicit) = &kinds[index] else {
            extras.push(raw[index].clone());
            index += 1;
            continue;
        };
        let takes_value = OPTIONS.iter().any(|(option, value)| option == name && *value);
        let shown_name = if *name == "-h" { "-h/--help" } else { name };
        let value = match (takes_value, explicit) {
            (true, Some(value)) => {
                index += 1;
                Some(value.clone())
            }
            (true, None) => match kinds.get(index + 1) {
                Some(Arg::Value) => {
                    index += 2;
                    Some(raw[index - 1].clone())
                }
                _ => {
                    return Err(AuditError::usage(format!(
                        "argument {shown_name}: expected one argument"
                    )));
                }
            },
            (false, Some(value)) => {
                return Err(AuditError::usage(format!(
                    "argument {shown_name}: ignored explicit argument '{value}'"
                )));
            }
            (false, None) => {
                index += 1;
                None
            }
        };
        match (*name, value) {
            ("-h" | "--help", _) => {
                println!(
                    "{USAGE}\n\nAudit existing local Git checkouts and record reproducible evidence; never fetch."
                );
                return Ok(None);
            }
            ("--scope", Some(value)) => args.scopes.push(value),
            ("--output", Some(value)) => args.output = Some(value),
            ("--include-html", _) => args.include_html = true,
            _ => {}
        }
    }
    if !positionals_taken {
        index = take(index, &mut args);
    }
    extras.extend(raw[index.min(raw.len())..].iter().cloned());
    if args.checkouts.is_empty() {
        return Err(AuditError::usage("the following arguments are required: checkouts"));
    }
    if !extras.is_empty() {
        return Err(AuditError::usage(format!(
            "unrecognized arguments: {}",
            extras.join(" ")
        )));
    }
    Ok(Some(args))
}

/// Read Git metadata with hooks and filesystem-monitor commands disabled; the output with surrounding whitespace
/// removed.
fn git(root: &Path, args: &[&str]) -> Result<String, AuditError> {
    let output = Command::new("git")
        .args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=", "-C"])
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| AuditError::new(error.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AuditError::new(format!(
            "Cannot read Git metadata for {}: {}",
            root.display(),
            strip(&stderr)
        )));
    }
    Ok(strip(&String::from_utf8_lossy(&output.stdout)).to_string())
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|rest| rest.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `[{file, sha256}]` for `paths`, sorted by root-relative path.
fn file_hashes(root: &Path, paths: &[PathBuf]) -> Result<Vec<(String, String)>, AuditError> {
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(path).map_err(|error| AuditError::new(format!("{error}")))?;
        out.push((relative(path, root), hex_digest(&bytes)));
    }
    out.sort();
    Ok(out)
}

fn manifest_value(manifest: &[(String, String)]) -> Value {
    Value::Array(
        manifest
            .iter()
            .map(|(file, digest)| {
                Value::object(vec![
                    ("file", Value::str(file.clone())),
                    ("sha256", Value::str(digest.clone())),
                ])
            })
            .collect(),
    )
}

fn audit_checkout(checkout: &Path, scopes: &[String], include_html: bool) -> Result<Value, AuditError> {
    let root = resolve(checkout);
    let top = git(&root, &["rev-parse", "--show-toplevel"])?;
    if resolve(Path::new(&top)) != root {
        return Err(AuditError::new(format!(
            "Use the Git checkout root, not a subdirectory: {}",
            root.display()
        )));
    }
    let revision = git(&root, &["rev-parse", "HEAD"])?;
    let status_before = git(&root, &["status", "--porcelain=v1", "--untracked-files=normal"])?;
    let mut selected = Vec::new();
    for scope in scopes {
        if scope.starts_with('/') {
            return Err(AuditError::new(format!(
                "Scope must be relative to each checkout: {scope}"
            )));
        }
        selected.push(PathBuf::from(shown(&format!("{}/{}", root.display(), scope))));
    }
    let config = Config {
        include_html,
        ..Config::default()
    };
    let mut auditor = Auditor::new(&root, config);
    let files = auditor.collect_inputs(&selected)?;
    let before = file_hashes(&root, &files)?;
    let report = auditor.run(Some(files.clone()))?;
    let after = file_hashes(&root, &files)?;
    let status_after = git(&root, &["status", "--porcelain=v1", "--untracked-files=normal"])?;
    if before != after || status_before != status_after || revision != git(&root, &["rev-parse", "HEAD"])? {
        return Err(AuditError::new(format!(
            "Checkout changed during the audit; rerun on a stable snapshot: {}",
            root.display()
        )));
    }
    let manifest = manifest_value(&after);
    let digest = hex_digest(manifest.compact(true).as_bytes());
    Ok(Value::object(vec![
        ("checkout", Value::str(root.to_string_lossy().into_owned())),
        ("commit", Value::str(revision)),
        ("worktree_clean", Value::Bool(status_after.is_empty())),
        (
            "scope",
            Value::Array(scopes.iter().map(|scope| Value::str(scope.clone())).collect()),
        ),
        ("markdown_manifest_sha256", Value::str(digest)),
        ("markdown_files", manifest),
        ("exit_code", Value::Int(if report.issues.is_empty() { 0 } else { 1 })),
        ("report", report_value(&report)),
    ]))
}

/// Run the corpus audit with the command line `argv` (without the program name); return the exit status.
pub fn run(argv: &[String]) -> u8 {
    let usage_error = |error: &AuditError| {
        eprintln!("{USAGE}\nmdlink-audit-corpus: error: {error}");
        2
    };
    let args = match parse_arguments(argv) {
        Ok(Some(args)) => args,
        Ok(None) => return 0,
        Err(error) => return usage_error(&error),
    };
    let checkouts: Vec<PathBuf> = args
        .checkouts
        .iter()
        .map(|checkout| PathBuf::from(shown(checkout)))
        .collect();
    if let Some(output) = &args.output {
        let output = resolve(Path::new(&shown(output)));
        if checkouts.iter().any(|checkout| output.starts_with(resolve(checkout))) {
            return usage_error(&AuditError::usage("--output must be outside every audited checkout"));
        }
    }
    let scopes = if args.scopes.is_empty() {
        vec![".".to_string()]
    } else {
        args.scopes.clone()
    };
    let mut ignored: Vec<&str> = DEFAULT_IGNORED_DIRS.to_vec();
    ignored.sort();
    let mut results = Vec::new();
    let mut exit_code = 0u8;
    for checkout in &checkouts {
        let result = audit_checkout(checkout, &scopes, args.include_html).unwrap_or_else(|error| {
            Value::object(vec![
                ("checkout", Value::str(resolve(checkout).to_string_lossy().into_owned())),
                ("error", Value::str(error.message())),
                ("exit_code", Value::Int(2)),
            ])
        });
        let code = match &result {
            Value::Object(pairs) => pairs.iter().find_map(|(key, value)| match (key.as_str(), value) {
                ("exit_code", Value::Int(code)) => Some(*code as u8),
                _ => None,
            }),
            _ => None,
        };
        exit_code = exit_code.max(code.unwrap_or(2));
        results.push(result);
    }
    let body = Value::object(vec![
        ("corpus_schema_version", Value::Int(1)),
        (
            "tool",
            Value::object(vec![
                ("name", Value::str("mdlink-audit")),
                ("version", Value::str(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (
            "configuration",
            Value::object(vec![
                (
                    "policy",
                    Value::str("built-in defaults; checkout configuration files are not loaded"),
                ),
                (
                    "ignored_directory_names",
                    Value::Array(ignored.into_iter().map(Value::str).collect()),
                ),
                ("exclude", Value::Array(Vec::new())),
                ("ignore_links", Value::Array(Vec::new())),
                ("include_html", Value::Bool(args.include_html)),
            ]),
        ),
        ("checkouts", Value::Array(results)),
    ]);
    let payload = body.pretty() + "\n";
    let written = match &args.output {
        Some(output) => fs::write(shown(output), payload.as_bytes()),
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(payload.as_bytes()).and_then(|()| stdout.flush())
        }
    };
    if let Err(error) = written {
        eprintln!("mdlink-audit-corpus: {error}");
        return 2;
    }
    exit_code
}
