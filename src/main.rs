use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    ExitCode::from(mdlink_audit::cli::run(&argv))
}
