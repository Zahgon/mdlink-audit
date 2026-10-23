//! The one error type for a problem that stops a run: a rejected command line, or an input, configuration or
//! filesystem problem. A broken link is a finding, not an error.

use std::fmt;

#[derive(Debug)]
pub enum AuditError {
    /// The command line was rejected: the program prints its usage line before the message and exits 2.
    Usage(String),
    /// An input, configuration or filesystem problem.
    Input(String),
}

impl AuditError {
    /// An input, configuration or filesystem problem.
    pub fn new(message: impl Into<String>) -> Self {
        AuditError::Input(message.into())
    }

    /// A rejected command line.
    pub fn usage(message: impl Into<String>) -> Self {
        AuditError::Usage(message.into())
    }

    pub fn message(&self) -> &str {
        match self {
            AuditError::Usage(message) | AuditError::Input(message) => message,
        }
    }
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for AuditError {}
