use std::path::{Path, PathBuf};

use indoc::formatdoc;

/// Where a file was named, when the scan did not read it directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSite {
    /// The importing file.
    pub file: PathBuf,
    /// 1-based `(line, column)` of the import application.
    pub position: (usize, usize),
}

/// A scan failure that must stop locking.
///
/// Only `Debug` is derived: the wrapped [rnix::ParseError] rules out `Eq`, and
/// [std::io::Error] rules out `Clone` and `PartialEq` too. Tests assert on the
/// rendered message instead.
#[derive(Debug, thiserror::Error)]
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

    /// The scan discovered a reference [CatalogRef] refuses, so no lock could
    /// be produced from it. Today that means a namespace escaping static
    /// analysis as a whole — passing `catalogs` to an opaque function,
    /// selecting a catalog dynamically, forwarding it through an import that
    /// cannot be followed — which widens to a root wildcard.
    ///
    /// `reason` is whatever the rule that rejected it said, rendered into the
    /// message rather than exposed as the error's source: nothing inspects it,
    /// and keeping it opaque means a new rule needs no change here.
    #[error("{}", unlockable_reference_message(file, *position, reason))]
    UnlockableReference {
        file: PathBuf,
        /// 1-based `(line, column)` where the reference was emitted, when
        /// recorded.
        position: Option<(usize, usize)>,
        reason: String,
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

    /// A file the scan must read cannot be read. Whether it is the entry
    /// expression, a dependency argument resolved by file convention, or the
    /// target of an import that forwards a catalog namespace, the references
    /// it would contribute cannot be discovered, so the scan fails rather
    /// than silently under-locking.
    ///
    /// The IO error is the source rather than part of the message, so what
    /// went wrong (missing, unreadable) is stated once, by the OS.
    #[error("{}", unreadable_file_message(file, imported_from.as_ref()))]
    UnreadableFile {
        file: PathBuf,
        #[source]
        source: std::io::Error,
        /// `Some` when the file was reached by an `import`; `None` when the
        /// scan resolved it directly (entry file or dependency argument).
        imported_from: Option<ImportSite>,
    },
}

impl ScanError {
    /// Re-express every file this error names relative to `base_dir`.
    ///
    /// The scan joins `base_dir` onto the path it is given before reading
    /// anything, and follows imports through canonicalized absolute paths, so
    /// by the time a failure is built its paths are absolute. Under `flox
    /// build` that root is the source copy in the store, which is not a
    /// location the user can open. The package-set root is the frame they
    /// wrote the path in, and the builder runs the lock from the project
    /// directory, so a path relative to it also resolves from their shell.
    ///
    /// A path outside `base_dir` is left absolute: it is still the only
    /// honest way to name it.
    pub(super) fn relative_to(self, base_dir: &Path) -> Self {
        match self {
            Self::UnparsableFile {
                file,
                position,
                error,
            } => Self::UnparsableFile {
                file: relative_to(file, base_dir),
                position,
                error,
            },
            Self::UnlockableReference {
                file,
                position,
                reason,
            } => Self::UnlockableReference {
                file: relative_to(file, base_dir),
                position,
                reason,
            },
            Self::UndeclaredRoot {
                root,
                file,
                position,
            } => Self::UndeclaredRoot {
                root,
                file: relative_to(file, base_dir),
                position,
            },
            Self::UnreadableFile {
                file,
                source,
                imported_from,
            } => Self::UnreadableFile {
                file: relative_to(file, base_dir),
                source,
                imported_from: imported_from.map(|site| ImportSite {
                    file: relative_to(site.file, base_dir),
                    position: site.position,
                }),
            },
        }
    }
}

/// `path` expressed relative to `base_dir`, or unchanged when it lies outside.
pub(super) fn relative_to(path: PathBuf, base_dir: &Path) -> PathBuf {
    path.strip_prefix(base_dir)
        .map(Path::to_path_buf)
        .unwrap_or(path)
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

/// Render [ScanError::UnlockableReference] for the user.
fn unlockable_reference_message(
    file: &Path,
    position: Option<(usize, usize)>,
    reason: &str,
) -> String {
    let location = location_suffix(file, position);
    formatdoc! {"
        Cannot lock a catalog reference{location}: {reason}.
        Reference packages as 'catalogs.<CATALOG>.<PACKAGE>'."}
}

/// Render [ScanError::UndeclaredRoot] for the user.
fn undeclared_root_message(root: &str, file: &Path, position: Option<(usize, usize)>) -> String {
    let location = location_suffix(file, position);
    formatdoc! {"
        '{root}' is referenced{location} but is not declared in the function arguments.
        Add '{root}' to the function arguments, e.g. '{{ {root}, ... }}:'."}
}

/// Render [ScanError::UnreadableFile] for the user. Kept to a single line with
/// no full stop so the IO error reads as its continuation when the source chain
/// is printed.
fn unreadable_file_message(file: &Path, imported_from: Option<&ImportSite>) -> String {
    let file = file.display();
    match imported_from {
        Some(site) => {
            let location = location_suffix(&site.file, Some(site.position));
            format!("'{file}' is imported{location} but cannot be read")
        },
        None => format!("'{file}' cannot be read"),
    }
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
    fn unreadable_file_error_message_points_at_the_import() {
        let err = ScanError::UnreadableFile {
            file: PathBuf::from("pkgs/helper.nix"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
            imported_from: Some(ImportSite {
                file: PathBuf::from("pkgs/foo.nix"),
                position: (4, 1),
            }),
        };
        assert_eq!(
            chained(&err),
            "'pkgs/helper.nix' is imported at pkgs/foo.nix:4:1 but cannot be read: entity not found"
        );
    }

    #[test]
    fn unreadable_file_error_message_names_a_directly_resolved_file() {
        let err = ScanError::UnreadableFile {
            file: PathBuf::from("pkgs/foo.nix"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            imported_from: None,
        };
        assert_eq!(
            chained(&err),
            "'pkgs/foo.nix' cannot be read: permission denied"
        );
    }

    #[test]
    fn unlockable_reference_message_states_the_reason_and_the_location() {
        let err = ScanError::UnlockableReference {
            file: PathBuf::from("pkgs/foo.nix"),
            position: Some((7, 14)),
            reason: "'catalogs.*' references the whole catalog namespace".to_string(),
        };
        assert_eq!(err.to_string(), indoc::indoc! {"
                Cannot lock a catalog reference at pkgs/foo.nix:7:14: 'catalogs.*' references the whole catalog namespace.
                Reference packages as 'catalogs.<CATALOG>.<PACKAGE>'."});
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
