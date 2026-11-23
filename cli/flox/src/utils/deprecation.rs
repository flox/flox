//! How `flox` words the catalog's deprecation of a package.
//!
//! The catalog reports a [DeprecationInfo] for pkg-paths that nixpkgs lists as
//! deprecated, renamed or removed. Every command that surfaces one goes
//! through [deprecation_notice], so the wording stays the same across
//! commands.

use floxhub_client::{DeprecationInfo, DeprecationKind};

/// One line telling the user that `pkg_path` is deprecated, without a leading
/// marker so each caller can attach its own (e.g. `message::warning`).
///
/// The line is a single line and always names `pkg_path`, because it can be
/// printed away from the output it is about: `flox show` prints it below a
/// long version list, and a command that prints notices for several packages
/// has nothing else to tie each one to its package. Whitespace in the
/// catalog's message, newlines included, collapses to single spaces, since
/// nixpkgs messages can span lines. The catalog's message is used verbatim
/// when it already names the package (`php81 is EOL`), because it can say more
/// than the kind does. A message that doesn't (`no longer maintained`)
/// follows a phrase from the kind that does: `'pkg' is deprecated in nixpkgs:
/// no longer maintained`. A message the catalog left out is replaced by that
/// phrase. A replacement is appended unless the text already has it in single
/// quotes, which is how nixpkgs' rename messages write it.
///
/// The phrases say what nixpkgs lists rather than what the catalog holds,
/// because the kind is a poor guide to the builds: `php81` is reported as
/// `removed` and still has builds, which `flox show` lists above the notice.
pub fn deprecation_notice(pkg_path: &str, deprecation: &DeprecationInfo) -> String {
    let kind_phrase = match deprecation.kind {
        DeprecationKind::Renamed => format!("'{pkg_path}' has been renamed in nixpkgs"),
        DeprecationKind::Removed => format!("'{pkg_path}' is marked as removed in nixpkgs"),
        DeprecationKind::Deprecated => format!("'{pkg_path}' is deprecated in nixpkgs"),
    };

    let message = deprecation
        .message
        .as_deref()
        .map(|message| message.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|message| !message.is_empty());
    let base = match message {
        Some(message) if names_pkg_path(&message, pkg_path) => message,
        Some(message) => format!("{kind_phrase}: {message}"),
        None => kind_phrase,
    };

    match deprecation.replacement.as_deref() {
        Some(replacement) if !base.contains(&format!("'{replacement}'")) => {
            format!(
                "{}. Use '{replacement}' instead.",
                base.trim_end_matches('.')
            )
        },
        _ => base,
    }
}

/// Whether `message` contains `pkg_path` as a whole name.
///
/// A plain substring search finds short names such as `b` inside any word, so
/// a match has to be bounded by characters that can't continue a name. A `.`
/// continues a name only when something follows it, so `foo.` at the end of a
/// sentence still counts. A name that is also an English word (`a`) can't be
/// told apart from the word.
fn names_pkg_path(message: &str, pkg_path: &str) -> bool {
    let continues_name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-');
    message.match_indices(pkg_path).any(|(start, matched)| {
        let before = message[..start].chars().next_back();
        let mut after = message[start + matched.len()..].chars();
        let next = after.next();
        let after_continues = match next {
            Some('.') => after.next().is_some_and(continues_name),
            Some(c) => continues_name(c),
            None => false,
        };
        let before_continues = before.is_some_and(|c| continues_name(c) || c == '.');
        !before_continues && !after_continues
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn info(
        kind: DeprecationKind,
        message: Option<&str>,
        replacement: Option<&str>,
    ) -> DeprecationInfo {
        DeprecationInfo {
            kind,
            message: message.map(str::to_string),
            replacement: replacement.map(str::to_string),
        }
    }

    /// The messages and replacements below are what the catalog returns for
    /// `_1password` and `php81`.
    #[test]
    fn catalog_message_naming_the_replacement_is_not_repeated() {
        let deprecation = info(
            DeprecationKind::Removed,
            Some("'_1password' has been renamed to/replaced by '_1password-cli'"),
            Some("_1password-cli"),
        );
        assert_eq!(
            deprecation_notice("_1password", &deprecation),
            "'_1password' has been renamed to/replaced by '_1password-cli'"
        );
    }

    #[test]
    fn catalog_message_without_the_replacement_gets_it_appended() {
        let deprecation = info(DeprecationKind::Renamed, Some("old is gone."), Some("new"));
        assert_eq!(
            deprecation_notice("old", &deprecation),
            "old is gone. Use 'new' instead."
        );
    }

    #[test]
    fn message_without_replacement_is_used_as_is() {
        let deprecation = info(DeprecationKind::Removed, Some("php81 is EOL"), None);
        assert_eq!(deprecation_notice("php81", &deprecation), "php81 is EOL");
    }

    #[test]
    fn message_not_naming_the_package_gets_a_phrase_that_does() {
        let deprecation = info(
            DeprecationKind::Deprecated,
            Some("no longer maintained"),
            None,
        );
        assert_eq!(
            deprecation_notice("foo", &deprecation),
            "'foo' is deprecated in nixpkgs: no longer maintained"
        );
    }

    #[test]
    fn message_not_naming_the_package_keeps_the_replacement() {
        let deprecation = info(DeprecationKind::Removed, Some("gone upstream"), Some("bar"));
        assert_eq!(
            deprecation_notice("foo", &deprecation),
            "'foo' is marked as removed in nixpkgs: gone upstream. Use 'bar' instead."
        );
    }

    /// nixpkgs `throw` messages can span lines, and a notice has to stay on
    /// one so a table or a list of notices keeps its layout.
    #[test]
    fn multi_line_message_becomes_one_line() {
        let deprecation = info(
            DeprecationKind::Removed,
            Some("foo is gone.\n  Use bar\tinstead.\n"),
            None,
        );
        assert_eq!(
            deprecation_notice("foo", &deprecation),
            "foo is gone. Use bar instead."
        );
    }

    /// `b` is inside `maybe`, so the message doesn't name the package.
    #[test]
    fn short_pkg_path_inside_a_word_is_not_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("maybe later"), None);
        assert_eq!(
            deprecation_notice("b", &deprecation),
            "'b' is deprecated in nixpkgs: maybe later"
        );
    }

    /// `b` is inside `a-b`, a different package.
    #[test]
    fn short_pkg_path_inside_a_hyphenated_name_is_not_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("use a-b"), None);
        assert_eq!(
            deprecation_notice("b", &deprecation),
            "'b' is deprecated in nixpkgs: use a-b"
        );
    }

    /// `b` is inside `b_c`, a different package.
    #[test]
    fn short_pkg_path_inside_an_underscored_name_is_not_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("b_c is old"), None);
        assert_eq!(
            deprecation_notice("b", &deprecation),
            "'b' is deprecated in nixpkgs: b_c is old"
        );
    }

    /// `b` is the tail of the nested path `foo.b`, a different package.
    #[test]
    fn short_pkg_path_at_the_end_of_a_nested_path_is_not_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("foo.b is old"), None);
        assert_eq!(
            deprecation_notice("b", &deprecation),
            "'b' is deprecated in nixpkgs: foo.b is old"
        );
    }

    #[test]
    fn short_pkg_path_as_a_word_is_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("b is EOL"), None);
        assert_eq!(deprecation_notice("b", &deprecation), "b is EOL");
    }

    #[test]
    fn short_pkg_path_in_quotes_is_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("'b' is EOL"), None);
        assert_eq!(deprecation_notice("b", &deprecation), "'b' is EOL");
    }

    #[test]
    fn short_pkg_path_before_a_full_stop_is_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("replaced b."), None);
        assert_eq!(deprecation_notice("b", &deprecation), "replaced b.");
    }

    #[test]
    fn short_pkg_path_in_brackets_is_named() {
        let deprecation = info(DeprecationKind::Deprecated, Some("(b)"), None);
        assert_eq!(deprecation_notice("b", &deprecation), "(b)");
    }

    #[test]
    fn nested_pkg_path_is_named_by_its_full_path() {
        let deprecation = info(DeprecationKind::Deprecated, Some("foo.bar is old"), None);
        assert_eq!(
            deprecation_notice("foo.bar", &deprecation),
            "foo.bar is old"
        );
    }

    #[test]
    fn nested_pkg_path_is_not_named_by_its_last_segment() {
        let deprecation = info(DeprecationKind::Deprecated, Some("foo.bar is old"), None);
        assert_eq!(
            deprecation_notice("bar", &deprecation),
            "'bar' is deprecated in nixpkgs: foo.bar is old"
        );
    }

    /// The wording has to hold when builds are listed above it: `php81` is
    /// reported as removed and has builds.
    #[test]
    fn renamed_without_message_says_where() {
        assert_eq!(
            deprecation_notice("a", &info(DeprecationKind::Renamed, None, Some("b"))),
            "'a' has been renamed in nixpkgs. Use 'b' instead."
        );
    }

    #[test]
    fn removed_without_message_says_where() {
        assert_eq!(
            deprecation_notice("a", &info(DeprecationKind::Removed, None, None)),
            "'a' is marked as removed in nixpkgs"
        );
    }

    #[test]
    fn deprecated_without_message_says_where() {
        assert_eq!(
            deprecation_notice("a", &info(DeprecationKind::Deprecated, None, None)),
            "'a' is deprecated in nixpkgs"
        );
    }

    #[test]
    fn blank_message_is_treated_as_missing() {
        let deprecation = info(DeprecationKind::Deprecated, Some("  "), None);
        assert_eq!(
            deprecation_notice("a", &deprecation),
            "'a' is deprecated in nixpkgs"
        );
    }
}
