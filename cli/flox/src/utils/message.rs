use std::fmt::Display;
/// Write a message to stderr.
///
/// This is a wrapper around `eprintln!` that can be further extended
/// to include logging, word wrapping, ANSI filtereing etc.
///
/// It is not called directly, but through the [message!] macro.
fn print_message(v: impl Display) {
    eprintln!("{v}");
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
