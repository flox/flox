use std::path::{Path, PathBuf};

use indoc::formatdoc;

/// A scan failure that must stop locking.
///
/// `Eq` is absent because [rnix::ParseError] does not implement it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScanError {
    /// A file the scan must read is not valid Nix. rnix is error-tolerant and
    /// yields a partial tree for a malformed file, so the references in the
    /// broken region would be dropped without a signal.
    ///
    /// The parse error is the source rather than part of the message: it
    /// already describes what the parser rejected, and rewriting that would
    /// duplicate rnix's own reporting.
    #[error("{}", unparsable_file_message(file, *position))]
    UnparsableFile {
        file: PathBuf,
        /// 1-based `(line, column)` of the parse error, when it carries a
        /// source range. Resolved at construction: the error locates itself by
        /// byte offset, and only the file's content turns that into a position.
        position: Option<(usize, usize)>,
        #[source]
        error: rnix::ParseError,
    },

    /// A catalog root is referenced by a file whose top-level lambda does not
    /// declare it as a parameter. NEF supplies only declared arguments
    /// (callPackage semantics), so every reference through the root is
    /// guaranteed to fail evaluation as an undefined variable.
    #[error("{}", undeclared_root_message(root, file, *position))]
    UndeclaredRoot {
        root: String,
        file: PathBuf,
        /// 1-based `(line, column)` of the root's first use, when recorded.
        position: Option<(usize, usize)>,
    },

    /// An import that forwards catalog namespaces names a target file that
    /// cannot be read. The refs the imported file would contribute through
    /// the forwarded namespaces cannot be discovered, so the scan fails
    /// rather than silently under-locking.
    #[error("{}", unreadable_import_message(target, file, *position))]
    UnreadableImport {
        target: PathBuf,
        file: PathBuf,
        /// 1-based `(line, column)` of the import application.
        position: (usize, usize),
    },
}

/// Render a source location as a message suffix; the position is best-effort
/// (forwarded-only uses may lack one).
fn location_suffix(file: &Path, position: Option<(usize, usize)>) -> String {
    match position {
        Some((line, column)) => format!(" at {}:{line}:{column}", file.display()),
        None => format!(" in {}", file.display()),
    }
}

/// Render [ScanError::UnparsableFile] for the user. Kept to a single line with
/// no full stop so the parse error reads as its continuation when the source
/// chain is printed.
fn unparsable_file_message(file: &Path, position: Option<(usize, usize)>) -> String {
    format!("Invalid Nix syntax{}", location_suffix(file, position))
}

/// Render [ScanError::UndeclaredRoot] for the user.
fn undeclared_root_message(root: &str, file: &Path, position: Option<(usize, usize)>) -> String {
    let location = location_suffix(file, position);
    formatdoc! {"
        '{root}' is referenced{location} but is not declared in the function arguments.
        Add '{root}' to the function arguments, e.g. '{{ {root}, ... }}:'."}
}

/// Render [ScanError::UnreadableImport] for the user.
fn unreadable_import_message(target: &Path, file: &Path, position: (usize, usize)) -> String {
    let target = target.display();
    let location = location_suffix(file, Some(position));
    formatdoc! {"
        '{target}' is imported{location} but cannot be read.
        Check that the imported file exists and is readable."}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `lock` prints: the message plus the source chain, which is where
    /// the parse error itself surfaces.
    fn chained(err: &ScanError) -> String {
        let mut rendered = err.to_string();
        let mut source = std::error::Error::source(err);
        while let Some(err) = source {
            rendered.push_str(&format!(": {err}"));
            source = err.source();
        }
        rendered
    }

    #[test]
    fn unparsable_file_error_message_points_at_the_syntax_error() {
        let err = ScanError::UnparsableFile {
            file: PathBuf::from("pkgs/foo.nix"),
            position: Some((4, 12)),
            error: rnix::ParseError::UnexpectedEOF,
        };
        assert_eq!(
            chained(&err),
            "Invalid Nix syntax at pkgs/foo.nix:4:12: unexpected end of file"
        );
    }

    #[test]
    fn unparsable_file_error_message_without_a_position_names_the_file() {
        // An end-of-file error carries no source range, so there is no position
        // to report.
        let err = ScanError::UnparsableFile {
            file: PathBuf::from("pkgs/foo.nix"),
            position: None,
            error: rnix::ParseError::UnexpectedEOF,
        };
        assert_eq!(
            chained(&err),
            "Invalid Nix syntax in pkgs/foo.nix: unexpected end of file"
        );
    }

    #[test]
    fn unreadable_import_error_message_points_at_the_import() {
        let err = ScanError::UnreadableImport {
            target: PathBuf::from("pkgs/helper.nix"),
            file: PathBuf::from("pkgs/foo.nix"),
            position: (4, 1),
        };
        assert_eq!(err.to_string(), indoc::indoc! {"
                'pkgs/helper.nix' is imported at pkgs/foo.nix:4:1 but cannot be read.
                Check that the imported file exists and is readable."});
    }

    #[test]
    fn undeclared_root_error_message_points_at_the_arguments() {
        let err = ScanError::UndeclaredRoot {
            root: "catalogs".to_string(),
            file: PathBuf::from("pkgs/foo.nix"),
            position: Some((4, 13)),
        };
        assert_eq!(err.to_string(), indoc::indoc! {"
                'catalogs' is referenced at pkgs/foo.nix:4:13 but is not declared in the function arguments.
                Add 'catalogs' to the function arguments, e.g. '{ catalogs, ... }:'."});
    }
}
