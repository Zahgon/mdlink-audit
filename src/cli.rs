//! The `mdlink-audit` command line: argument rules, the audit run, and the exit status.
//!
//! Long options may be abbreviated to any unique prefix and take their value either as the next
//! argument or after `=`. Positional paths form one run; a path after an option that follows
//! earlier paths is rejected. A rejected command line prints usage to standard error and exits 2.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::baseline::Baseline;
use crate::checker::{Auditor, Report, discover_root};
use crate::config::{Config, load_config};
use crate::error::AuditError;
use crate::output;
use crate::paths::{resolve, shown};
use crate::text::is_negative_number;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "usage: mdlink-audit [-h] [--root ROOT] [--format {text,json,github,sarif,junit}] [--config CONFIG] \
[--exclude GLOB] [--ignore-link GLOB] [--include-html | --no-include-html] [--fail-on-empty] [--walk-symlinks] [--baseline FILE] [--fix] [--audit-definitions] [--require-alt] [--audit-anchors] [--version] [paths ...]";
const HELP: &str = "Check local Markdown links offline. External URLs are skipped, never fetched.";
const FORMATS: [&str; 5] = ["text", "json", "github", "sarif", "junit"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Opt {
    Help,
    Root,
    Format,
    Config,
    Exclude,
    IgnoreLink,
    IncludeHtml,
    NoIncludeHtml,
    FailOnEmpty,
    WalkSymlinks,
    Baseline,
    Fix,
    AuditDefinitions,
    RequireAlt,
    AuditAnchors,
    Version,
}

const OPTIONS: [(&str, Opt); 17] = [
    ("-h", Opt::Help),
    ("--help", Opt::Help),
    ("--root", Opt::Root),
    ("--format", Opt::Format),
    ("--config", Opt::Config),
    ("--exclude", Opt::Exclude),
    ("--ignore-link", Opt::IgnoreLink),
    ("--include-html", Opt::IncludeHtml),
    ("--no-include-html", Opt::NoIncludeHtml),
    ("--fail-on-empty", Opt::FailOnEmpty),
    ("--walk-symlinks", Opt::WalkSymlinks),
    ("--baseline", Opt::Baseline),
    ("--fix", Opt::Fix),
    ("--audit-definitions", Opt::AuditDefinitions),
    ("--require-alt", Opt::RequireAlt),
    ("--audit-anchors", Opt::AuditAnchors),
    ("--version", Opt::Version),
];

impl Opt {
    fn takes_value(self) -> bool {
        matches!(
            self,
            Opt::Root | Opt::Format | Opt::Config | Opt::Exclude | Opt::IgnoreLink | Opt::Baseline
        )
    }

    fn name(self) -> &'static str {
        OPTIONS
            .iter()
            .rev()
            .find(|(_, opt)| *opt == self)
            .map(|(name, _)| *name)
            .unwrap_or("")
    }
}

/// How one command-line argument reads before any option consumes it.
enum Arg {
    Value,
    Separator,
    Known(Opt, Option<String>),
    Unknown,
}

#[derive(Debug, Default)]
struct Arguments {
    paths: Vec<String>,
    root: Option<String>,
    format: String,
    config: Option<String>,
    exclude: Vec<String>,
    ignore_link: Vec<String>,
    include_html: Option<bool>,
    fail_on_empty: bool,
    walk_symlinks: bool,
    baseline: Option<String>,
    fix: bool,
    audit_definitions: bool,
    require_alt: bool,
    audit_anchors: bool,
}

enum Outcome {
    Run(Arguments),
    Exit(u8),
}

fn exact(name: &str) -> Option<Opt> {
    OPTIONS.iter().find(|(option, _)| *option == name).map(|(_, opt)| *opt)
}

fn prefix_matches(arg: &str) -> Vec<(Opt, Option<String>)> {
    if arg.starts_with("--") {
        let (prefix, explicit) = match arg.split_once('=') {
            Some((prefix, value)) => (prefix, Some(value.to_string())),
            None => (arg, None),
        };
        return OPTIONS
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, opt)| (*opt, explicit.clone()))
            .collect();
    }
    let split = arg.char_indices().nth(2).map_or(arg.len(), |(index, _)| index);
    let (short, rest) = arg.split_at(split);
    let mut found = Vec::new();
    for (name, opt) in OPTIONS {
        if name == short {
            // `-h` takes no value: a tail that does not start with `-` is a cluster of further short options, and
            // `-h` itself acts (printing help) before the tail is looked at.
            let explicit = (opt.takes_value() || rest.is_empty() || rest.starts_with('-')).then(|| rest.to_string());
            found.push((opt, explicit));
        } else if name.starts_with(arg) {
            found.push((opt, None));
        }
    }
    found
}

fn classify(arg: &str) -> Result<Arg, AuditError> {
    if !arg.starts_with('-') {
        return Ok(Arg::Value);
    }
    if let Some(opt) = exact(arg) {
        return Ok(Arg::Known(opt, None));
    }
    if arg.len() == 1 {
        return Ok(Arg::Value);
    }
    if let Some((name, value)) = arg.split_once('=')
        && let Some(opt) = exact(name)
    {
        return Ok(Arg::Known(opt, Some(value.to_string())));
    }
    let mut found = prefix_matches(arg);
    if found.len() > 1 {
        let names: Vec<&str> = found.iter().map(|(opt, _)| opt.name()).collect();
        return Err(AuditError::usage(format!(
            "ambiguous option: {arg} could match {}",
            names.join(", ")
        )));
    }
    if let Some((opt, explicit)) = found.pop() {
        return Ok(Arg::Known(opt, explicit));
    }
    if is_negative_number(arg) || arg.contains(' ') {
        return Ok(Arg::Value);
    }
    Ok(Arg::Unknown)
}

fn parse_arguments(raw: &[String]) -> Result<Outcome, AuditError> {
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
    let mut args = Arguments {
        format: "text".to_string(),
        ..Arguments::default()
    };
    let mut paths_taken = false;
    let mut extras: Vec<String> = Vec::new();
    let mut index = 0;

    let take_paths = |start: usize, args: &mut Arguments| -> usize {
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
            args.paths.push(raw[position].clone());
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
            if !paths_taken {
                paths_taken = true;
                index = take_paths(index, &mut args);
                continue;
            }
            extras.extend(raw[index..next_option].iter().cloned());
            index = next_option;
        }
        let Arg::Known(opt, explicit) = &kinds[index] else {
            extras.push(raw[index].clone());
            index += 1;
            continue;
        };
        let value = match (opt.takes_value(), explicit) {
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
                        "argument {}: expected one argument",
                        opt.name()
                    )));
                }
            },
            (false, Some(value)) => {
                return Err(AuditError::usage(format!(
                    "argument {}: ignored explicit argument '{value}'",
                    opt.name()
                )));
            }
            (false, None) => {
                index += 1;
                None
            }
        };
        match (opt, value) {
            (Opt::Help, _) => {
                println!("{USAGE}\n\n{HELP}");
                return Ok(Outcome::Exit(0));
            }
            (Opt::Version, _) => {
                println!("mdlink-audit {VERSION}");
                return Ok(Outcome::Exit(0));
            }
            (Opt::Root, Some(value)) => args.root = Some(value),
            (Opt::Config, Some(value)) => args.config = Some(value),
            (Opt::Exclude, Some(value)) => args.exclude.push(value),
            (Opt::IgnoreLink, Some(value)) => args.ignore_link.push(value),
            (Opt::Format, Some(value)) => {
                if !FORMATS.contains(&value.as_str()) {
                    return Err(AuditError::usage(format!(
                        "argument --format: invalid choice: '{value}' (choose from text, json, github, sarif, junit)"
                    )));
                }
                args.format = value;
            }
            (Opt::IncludeHtml, _) => args.include_html = Some(true),
            (Opt::NoIncludeHtml, _) => args.include_html = Some(false),
            (Opt::FailOnEmpty, _) => args.fail_on_empty = true,
            (Opt::WalkSymlinks, _) => args.walk_symlinks = true,
            (Opt::Baseline, Some(value)) => args.baseline = Some(value),
            (Opt::Fix, _) => args.fix = true,
            (Opt::AuditDefinitions, _) => args.audit_definitions = true,
            (Opt::RequireAlt, _) => args.require_alt = true,
            (Opt::AuditAnchors, _) => args.audit_anchors = true,
            (_, None) => {}
        }
    }
    if !paths_taken {
        index = take_paths(index, &mut args);
    }
    extras.extend(raw[index.min(raw.len())..].iter().cloned());
    if !extras.is_empty() {
        return Err(AuditError::usage(format!(
            "unrecognized arguments: {}",
            extras.join(" ")
        )));
    }
    Ok(Outcome::Run(args))
}

fn audit(args: &Arguments) -> Result<Report, AuditError> {
    let root = match &args.root {
        Some(root) => resolve(Path::new(&shown(root))),
        None => {
            let cwd = std::env::current_dir().map_err(|error| AuditError::new(error.to_string()))?;
            discover_root(&cwd)
        }
    };
    let loaded = match &args.config {
        Some(path) => load_config(Path::new(&shown(path)), true)?,
        None => load_config(&root.join(".mdlink-audit.toml"), false)?,
    };
    let config = Config {
        exclude: loaded.exclude.into_iter().chain(args.exclude.iter().cloned()).collect(),
        ignore_links: loaded
            .ignore_links
            .into_iter()
            .chain(args.ignore_link.iter().cloned())
            .collect(),
        include_html: args.include_html.unwrap_or(loaded.include_html),
        walk_symlinks: args.walk_symlinks,
        link_rewrites: loaded.link_rewrites,
        severity: loaded.severity,
        audit_definitions: args.audit_definitions,
        require_alt: args.require_alt,
        audit_anchors: args.audit_anchors,
    };
    let baseline = match &args.baseline {
        Some(path) => Some(Baseline::load(Path::new(&shown(path)), &shown(path))?),
        None => None,
    };
    let inputs = if args.paths.is_empty() {
        None
    } else {
        Some(args.paths.iter().map(|path| PathBuf::from(shown(path))).collect())
    };
    let mut report = Auditor::new(&root, config).run(inputs)?;
    if args.fail_on_empty && report.files_scanned == 0 {
        return Err(AuditError::new("No Markdown files were selected"));
    }
    if let Some(baseline) = baseline {
        let (kept, suppressed) = baseline.filter(std::mem::take(&mut report.issues));
        report.issues = kept;
        report.baselined = Some(suppressed);
    }
    Ok(report)
}

/// Run the command line `argv` (without the program name) and return the exit status.
pub fn run(argv: &[String]) -> u8 {
    let args = match parse_arguments(argv) {
        Ok(Outcome::Run(args)) => args,
        Ok(Outcome::Exit(code)) => return code,
        Err(error) => {
            eprintln!("{USAGE}\nmdlink-audit: error: {error}");
            return 2;
        }
    };
    let mut stdout = std::io::stdout().lock();
    match audit(&args) {
        Ok(report) => {
            let written = stdout
                .write_all(output::render(&report, &args.format).as_bytes())
                .and_then(|()| stdout.flush());
            if written.is_err() {
                return 2;
            }
            if args.fix
                && let Err(error) = crate::fix::apply(&report)
            {
                eprintln!("mdlink-audit: {error}");
                return 2;
            }
            if report.failing() { 1 } else { 0 }
        }
        Err(error) => {
            match output::render_error(error.message(), &args.format) {
                Some(line) => {
                    let _ = stdout.write_all(line.as_bytes()).and_then(|()| stdout.flush());
                }
                None => eprintln!("mdlink-audit: {error}"),
            }
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Arguments, String> {
        let raw: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        match parse_arguments(&raw) {
            Ok(Outcome::Run(args)) => Ok(args),
            Ok(Outcome::Exit(code)) => Err(format!("exit {code}")),
            Err(error) => Err(error.to_string()),
        }
    }

    #[test]
    fn prefixes_and_inline_values() {
        let args = parse(&["--form", "json", "--ign=x", "--in"]).unwrap();
        assert_eq!(
            (args.format.as_str(), args.ignore_link, args.include_html),
            ("json", vec!["x".to_string()], Some(true))
        );
        assert!(parse(&["--i", "x"]).unwrap_err().starts_with("ambiguous option"));
    }

    #[test]
    fn option_like_values_and_path_runs() {
        assert!(parse(&["--exclude", "-x"]).is_err());
        assert_eq!(parse(&["--exclude", "-1"]).unwrap().exclude, vec!["-1".to_string()]);
        assert!(parse(&["a.md", "--root", ".", "b.md"]).is_err());
        assert_eq!(parse(&["--root", ".", "a.md", "b.md"]).unwrap().paths.len(), 2);
        assert_eq!(parse(&["--", "--format"]).unwrap().paths, vec!["--format".to_string()]);
    }
}
