//! Text, JSON and GitHub annotation renderings of a report.

use crate::checker::{Issue, Report};
use crate::json::Value;
use crate::urls::quote;

/// A JSON string literal with non-ASCII characters kept verbatim.
pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn annotation_escape(value: &str, property: bool) -> String {
    let mut out = value.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A");
    if property {
        out = out.replace(':', "%3A").replace(',', "%2C");
    }
    out
}

fn annotation(issue: &Issue) -> String {
    let mut detail = format!("[{}] {} (target: {})", issue.code, issue.message, issue.target);
    if let Some(suggestion) = &issue.suggestion {
        detail.push_str(&format!("; did you mean: {suggestion}"));
    }
    format!(
        "::{} file={},line={}::{}",
        if issue.severity == Some("warning") {
            "warning"
        } else {
            "error"
        },
        annotation_escape(&issue.file, true),
        issue.line,
        annotation_escape(&detail, false)
    )
}

fn json_report(report: &Report) -> String {
    let mut lines = vec![
        "{".to_string(),
        "  \"schema_version\": 1,".to_string(),
        format!("  \"root\": {},", json_string(&report.root)),
        "  \"summary\": {".to_string(),
        format!("    \"files_scanned\": {},", report.files_scanned),
        format!("    \"links_checked\": {},", report.links_checked),
        format!("    \"links_skipped\": {},", report.links_skipped),
        match report.baselined {
            Some(count) => format!("    \"issues\": {},\n    \"baselined\": {count}", report.issues.len()),
            None => format!("    \"issues\": {}", report.issues.len()),
        },
        "  },".to_string(),
        "  \"skipped\": {".to_string(),
        format!("    \"external\": {},", report.skipped_external),
        format!("    \"ignored\": {}", report.skipped_ignored),
        "  },".to_string(),
    ];
    if report.issues.is_empty() {
        lines.push("  \"issues\": []".to_string());
    } else {
        lines.push("  \"issues\": [".to_string());
        for (index, issue) in report.issues.iter().enumerate() {
            let suggestion = issue
                .suggestion
                .as_deref()
                .map(json_string)
                .unwrap_or_else(|| "null".to_string());
            lines.push("    {".to_string());
            lines.push(format!("      \"file\": {},", json_string(&issue.file)));
            lines.push(format!("      \"line\": {},", issue.line));
            lines.push(format!("      \"target\": {},", json_string(&issue.target)));
            lines.push(format!("      \"code\": {},", json_string(issue.code)));
            lines.push(format!("      \"message\": {},", json_string(&issue.message)));
            match issue.severity {
                Some(level) => {
                    lines.push(format!("      \"suggestion\": {suggestion},"));
                    lines.push(format!("      \"severity\": {}", json_string(level)));
                }
                None => lines.push(format!("      \"suggestion\": {suggestion}")),
            }
            lines.push(if index + 1 == report.issues.len() {
                "    }".to_string()
            } else {
                "    },".to_string()
            });
        }
        lines.push("  ]".to_string());
    }
    lines.push("}".to_string());
    lines.join("\n")
}

/// The short description of each issue code in a SARIF log.
fn rule_text(code: &str) -> &'static str {
    match code {
        "case_mismatch" => "Path casing differs from the file on disk",
        "duplicate_anchor" => "Anchor is defined more than once in the document",
        "duplicate_definition" => "Link reference definition repeats an earlier label",
        "invalid_target" => "Link target is not a valid repository path",
        "missing_alt" => "Image has no alternative text",
        "missing_anchor" => "Heading or custom anchor does not exist",
        "missing_file" => "Link target does not exist",
        "normalization_mismatch" => "Path Unicode normalization differs from the file on disk",
        "not_directory" => "Path ending in '/' does not refer to a directory",
        "undefined_reference" => "Reference link label matches no definition",
        "unused_definition" => "Link reference definition is never used",
        _ => "Link resolves outside the repository root",
    }
}

/// The report as a SARIF 2.1.0 log.
pub fn sarif(report: &Report) -> Value {
    let mut root_uri = format!("file://{}", quote(&report.root, "/"));
    if !root_uri.ends_with('/') {
        root_uri.push('/');
    }
    let results = report
        .issues
        .iter()
        .map(|issue| {
            let mut text = format!("{} (target: {})", issue.message, issue.target);
            if let Some(suggestion) = &issue.suggestion {
                text.push_str(&format!("; did you mean: {suggestion}"));
            }
            let location = Value::object(vec![(
                "physicalLocation",
                Value::object(vec![
                    (
                        "artifactLocation",
                        Value::object(vec![
                            ("uri", Value::str(quote(&issue.file, "/"))),
                            ("uriBaseId", Value::str("ROOT")),
                        ]),
                    ),
                    (
                        "region",
                        Value::object(vec![("startLine", Value::Int(issue.line as i64))]),
                    ),
                ]),
            )]);
            Value::object(vec![
                ("ruleId", Value::str(issue.code)),
                (
                    "level",
                    Value::str(if issue.severity == Some("warning") {
                        "warning"
                    } else {
                        "error"
                    }),
                ),
                ("message", Value::object(vec![("text", Value::str(text))])),
                ("locations", Value::Array(vec![location])),
            ])
        })
        .collect();
    let mut codes: Vec<&str> = report.issues.iter().map(|issue| issue.code).collect();
    codes.sort();
    codes.dedup();
    let rules = codes
        .into_iter()
        .map(|code| {
            Value::object(vec![
                ("id", Value::str(code)),
                (
                    "shortDescription",
                    Value::object(vec![("text", Value::str(rule_text(code)))]),
                ),
            ])
        })
        .collect();
    let driver = Value::object(vec![
        ("name", Value::str("mdlink-audit")),
        ("version", Value::str(env!("CARGO_PKG_VERSION"))),
        ("rules", Value::Array(rules)),
    ]);
    let mut properties = vec![
        ("filesScanned", Value::Int(report.files_scanned as i64)),
        ("linksChecked", Value::Int(report.links_checked as i64)),
        ("linksSkipped", Value::Int(report.links_skipped as i64)),
    ];
    if let Some(count) = report.baselined {
        properties.push(("baselined", Value::Int(count as i64)));
    }
    let run = Value::object(vec![
        ("tool", Value::object(vec![("driver", driver)])),
        (
            "originalUriBaseIds",
            Value::object(vec![("ROOT", Value::object(vec![("uri", Value::str(root_uri))]))]),
        ),
        ("results", Value::Array(results)),
        ("properties", Value::object(properties)),
    ]);
    Value::object(vec![
        ("$schema", Value::str("https://json.schemastore.org/sarif-2.1.0.json")),
        ("version", Value::str("2.1.0")),
        ("runs", Value::Array(vec![run])),
    ])
}

/// The report as the JSON document of `--format json`.
pub fn report_value(report: &Report) -> Value {
    let issues = report
        .issues
        .iter()
        .map(|issue| {
            let mut pairs = vec![
                ("file", Value::str(issue.file.clone())),
                ("line", Value::Int(issue.line as i64)),
                ("target", Value::str(issue.target.clone())),
                ("code", Value::str(issue.code)),
                ("message", Value::str(issue.message.clone())),
                (
                    "suggestion",
                    issue.suggestion.clone().map(Value::Str).unwrap_or(Value::Null),
                ),
            ];
            if let Some(level) = issue.severity {
                pairs.push(("severity", Value::str(level)));
            }
            Value::object(pairs)
        })
        .collect();
    Value::object(vec![
        ("schema_version", Value::Int(1)),
        ("root", Value::str(report.root.clone())),
        (
            "summary",
            Value::object(vec![
                ("files_scanned", Value::Int(report.files_scanned as i64)),
                ("links_checked", Value::Int(report.links_checked as i64)),
                ("links_skipped", Value::Int(report.links_skipped as i64)),
                ("issues", Value::Int(report.issues.len() as i64)),
            ]),
        ),
        (
            "skipped",
            Value::object(vec![
                ("external", Value::Int(report.skipped_external as i64)),
                ("ignored", Value::Int(report.skipped_ignored as i64)),
            ]),
        ),
        ("issues", Value::Array(issues)),
    ])
}

/// Text and attribute content of the JUnit report: XML's special characters except `'`, tab and
/// line breaks as character references, and other characters XML 1.0 forbids as `\uXXXX`.
fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// The report as a JUnit XML document: one test suite per file with issues, one failing test case per issue.
fn junit(report: &Report) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"mdlink-audit\" tests=\"{}\" failures=\"{}\" skipped=\"{}\">\n",
        report.links_checked,
        report.issues.len(),
        report.links_skipped
    ));
    let mut files: Vec<&str> = Vec::new();
    for issue in &report.issues {
        if !files.contains(&issue.file.as_str()) {
            files.push(&issue.file);
        }
    }
    for file in files {
        let issues: Vec<_> = report.issues.iter().filter(|issue| issue.file == file).collect();
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\">\n",
            xml_escape(file),
            issues.len(),
            issues.len()
        ));
        for issue in issues {
            out.push_str(&format!(
                "    <testcase classname=\"{}\" name=\"{}: {}\">\n",
                issue.code,
                issue.line,
                xml_escape(&issue.target)
            ));
            out.push_str(&format!(
                "      <failure message=\"{}\" type=\"{}\">{}</failure>\n    </testcase>\n",
                xml_escape(&issue.message),
                issue.severity.unwrap_or("error"),
                xml_escape(issue.suggestion.as_deref().unwrap_or(""))
            ));
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// Render the report in `format` as the complete standard output text.
pub fn render(report: &Report, format: &str) -> String {
    if format == "junit" {
        return junit(report);
    }
    if format == "sarif" {
        return sarif(report).pretty() + "\n";
    }
    if format == "json" {
        return json_report(report) + "\n";
    }
    let mut out = String::new();
    for issue in &report.issues {
        if format == "github" {
            out.push_str(&annotation(issue));
            out.push('\n');
        } else {
            let escape = |text: &str| text.replace('\r', "\\r").replace('\n', "\\n");
            out.push_str(&format!(
                "{}:{}: {}{}: {} ({})\n",
                escape(&issue.file),
                issue.line,
                issue.code,
                if issue.severity == Some("warning") {
                    " (warning)"
                } else {
                    ""
                },
                escape(&issue.message),
                json_string(&issue.target)
            ));
            if let Some(suggestion) = &issue.suggestion {
                out.push_str(&format!("  suggestion: {}\n", json_string(suggestion)));
            }
        }
    }
    let state = if report.failing() { "FAIL" } else { "PASS" };
    out.push_str(&format!(
        "{state}: {} Markdown files, {} local links checked, {} links skipped, {} issues{}.\n",
        report.files_scanned,
        report.links_checked,
        report.links_skipped,
        report.issues.len(),
        report
            .baselined
            .map(|count| format!(", {count} baselined"))
            .unwrap_or_default()
    ));
    out
}

/// The error line printed on standard output for the machine formats.
pub fn render_error(message: &str, format: &str) -> Option<String> {
    match format {
        "json" | "sarif" => Some(format!(
            "{{\"schema_version\": 1, \"error\": {}}}\n",
            json_string(message)
        )),
        "github" => Some(format!("::error::{}\n", annotation_escape(message, false))),
        _ => None,
    }
}
