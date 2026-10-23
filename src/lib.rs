//! mdlink-audit: check local Markdown links offline. External URLs are skipped, never fetched.

pub mod action;
pub mod baseline;
pub mod checker;
pub mod cli;
pub mod config;
pub mod corpus;
pub mod error;
pub mod fix;
pub mod html;
pub mod json;
pub mod markdown;
pub mod output;
pub mod paths;
pub mod references;
pub mod sha256;
pub mod similarity;
pub mod text;
pub mod toml;
pub mod urls;
