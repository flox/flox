use std::fmt::Display;

use flox_rust_sdk::models::manifest::PackageToInstall;
/// Write a message to stderr.
///
/// This is a wrapper around `eprintln!` that can be further extended
/// to include logging, word wrapping, ANSI filtereing etc.
fn print_message(v: impl Display) {
    eprintln!("{v}");
}

#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub enum Action {
    Plain(String),
    Error(String),
    Created(String),
    Deleted(String),
    Updated(String),
    Warning(String),
}

impl Action {
    pub fn print(self) {
        match self {
            Action::Plain(v) => plain(v),
            Action::Error(v) => error(v),
            Action::Created(v) => created(v),
            Action::Deleted(v) => deleted(v),
            Action::Updated(v) => updated(v),
            Action::Warning(v) => warning(v),
        }
    }
}

/// alias for [print_message]
pub(crate) fn plain(v: impl Display) {
    print_message(v);
}
pub(crate) fn error(v: impl Display) {
    print_message(std::format_args!("❌ ERROR: {v}"));
}
pub(crate) fn created(v: impl Display) {
    print_message(std::format_args!("✨ {v}"));
}
/// double width chracter, add an additional space for alignment
pub(crate) fn deleted(v: impl Display) {
    print_message(std::format_args!("🗑️  {v}"));
}
pub(crate) fn updated(v: impl Display) {
    print_message(std::format_args!("✅ {v}"));
}
/// double width chracter, add an additional space for alignment
pub(crate) fn warning(v: impl Display) {
    print_message(std::format_args!("⚠️  {v}"));
}

pub(crate) fn package_installed(pkg: &PackageToInstall, environment_description: &str) {
    updated(format!(
        "'{}' installed to environment {environment_description}",
        pkg.id
    ));
}
