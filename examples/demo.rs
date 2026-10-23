//! A self-contained broken-then-fixed repository demo: `cargo run --example demo`.

use std::fs;
use std::process::ExitCode;

fn audit(root: &str) -> u8 {
    mdlink_audit::cli::run(&["--root".to_string(), root.to_string()])
}

fn main() -> ExitCode {
    let root = std::env::temp_dir().join(format!("mdlink-audit-demo-{}", std::process::id()));
    let status = demo(&root);
    let _ = fs::remove_dir_all(&root);
    match status {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("demo: {error}");
            ExitCode::from(1)
        }
    }
}

fn demo(root: &std::path::Path) -> std::io::Result<u8> {
    fs::create_dir_all(root.join("docs"))?;
    fs::write(root.join("docs").join("guide.md"), "# 安装指南\n")?;
    let readme = root.join("README.md");
    fs::write(
        &readme,
        "# Demo\n\n[guide](docs/Guide.md#安装指南)\n[setup](docs/guide.md#setup)\n",
    )?;
    let root_text = root.to_string_lossy().into_owned();
    println!("1. Catch a path casing error and a stale heading:");
    if audit(&root_text) != 1 {
        return Ok(1);
    }
    fs::write(&readme, "# Demo\n\n[guide](docs/guide.md#安装指南)\n")?;
    println!("\n2. Fix both links and rerun:");
    Ok(audit(&root_text))
}
