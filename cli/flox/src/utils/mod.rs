use std::borrow::Cow;
use std::io::Stderr;
use std::sync::Mutex;

use once_cell::sync::Lazy;

pub mod colors;
mod completion;
pub mod dialog;
pub mod init;
pub mod logger;
pub mod metrics;

use regex::Regex;

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());
pub static TERMINAL_STDERR: Lazy<Mutex<Stderr>> = Lazy::new(|| Mutex::new(std::io::stderr()));

fn nix_str_safe(s: &str) -> Cow<str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{s:?}").into()
    }
}
