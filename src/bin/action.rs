use std::collections::HashMap;
use std::process::ExitCode;

fn main() -> ExitCode {
    let env: HashMap<String, String> = std::env::vars_os()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
        .collect();
    ExitCode::from(mdlink_audit::action::run(&env))
}
