//! Render unresolvable catalog references and their dependency chains.
//!
//! Only a server-classified, readable `legacy_unindexed` cause gets a
//! specific remedy. Other causes stay generic because inferring why a
//! private reference failed could reveal its identity.

use floxhub_client::UnresolvableEntry;
use indent::indent_all_by;
use indoc::formatdoc;

/// The NEF catalog root the scanner strips before sending references to the
/// server (see `lock::lookup::wire_reference`). The server echoes back
/// catalog-relative references (`<catalog>.<package>`); restore the root so the
/// developer sees the attribute exactly as written in their expression
/// (`catalogs.<catalog>.<package>`).
const CATALOG_ROOT: &str = "catalogs";

/// A readable identity without a v2 index needs a new publish, not a relock
/// of the same commit.
const LEGACY_UNINDEXED_REMEDY: &str =
    "Publish a new commit of this package with an upgraded CLI and a version 2 catalog lock.";

/// The readable unindexed cause used by the server
/// (`flox/floxhub@2f761a193:catalog_server/api/v1/endpoints/build_inputs.py`).
const LEGACY_UNINDEXED_CAUSE_KEY: &str = "legacy_unindexed";

/// Return a specific remedy only for a server-classified readable cause.
fn legacy_unindexed_remedy(entry: &UnresolvableEntry) -> Option<&'static str> {
    entry
        .leaf
        .unresolvable
        .contains_key(LEGACY_UNINDEXED_CAUSE_KEY)
        .then_some(LEGACY_UNINDEXED_REMEDY)
}

/// Prefix a server-returned, catalog-relative reference with the NEF
/// [`CATALOG_ROOT`] for display.
fn display_reference(reference: &str) -> String {
    format!("{CATALOG_ROOT}.{reference}")
}

/// Render failed references into a developer-facing error body:
/// - a `→`-arrow dependency path per reference, ending in `(unresolvable)`,
/// - numbered entries under a `build failed: N inputs could not be resolved.`
///   header when there is more than one,
/// - a generic remediation footer when any entry lacks a classified cause.
///
/// The returned string is the message body; the caller applies the `✘ ERROR:`
/// decoration (e.g. via `flox_core::util::message::format_error`) and exits
/// non-zero.
pub fn render_unresolvable(entries: &[UnresolvableEntry]) -> String {
    match entries {
        [single] => render_single(single),
        many => render_many(many),
    }
}

/// Render one entry's `chain` as a dependency path, one reference per line.
/// The first element has no arrow; later ones are prefixed with `→ `; the
/// final (unresolvable) element is annotated inline. The caller indents the
/// whole block with [`indent_all_by`].
fn render_path(chain: &[String]) -> String {
    let last = chain.len().saturating_sub(1);
    chain
        .iter()
        .enumerate()
        .map(|(i, reference)| {
            let arrow = if i == 0 { "" } else { "→ " };
            let leaf = if i == last { " (unresolvable)" } else { "" };
            format!(
                "{arrow}{reference}{leaf}",
                reference = display_reference(reference)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_single(entry: &UnresolvableEntry) -> String {
    let footer = match legacy_unindexed_remedy(entry) {
        Some(remedy) => format!("  {remedy}"),
        None => "  Possible causes: the input may not be visible to you, may have no\n  \
                 published revision, or may have aged out of retention. Verify\n  \
                 availability with the owner of the relevant catalog."
            .to_string(),
    };
    formatdoc! {"
        '{reference}' is unresolvable in this context.

          Dependency path:
        {path}

        {footer}",
        reference = display_reference(&entry.reference),
        path = indent_all_by(4, render_path(&entry.chain)),
    }
}

/// Attach a specific remedy only to the entry the server classified.
fn render_many_item(index: usize, entry: &UnresolvableEntry) -> String {
    let header = formatdoc! {"
          {n}. '{reference}' is unresolvable in this context.
             Dependency path:
        {path}",
        n = index + 1,
        reference = display_reference(&entry.reference),
        path = indent_all_by(7, render_path(&entry.chain)),
    };
    match legacy_unindexed_remedy(entry) {
        Some(remedy) => format!("{header}\n     {remedy}"),
        None => header,
    }
}

fn render_many(entries: &[UnresolvableEntry]) -> String {
    let blocks = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| render_many_item(i, entry))
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut message = formatdoc! {"
        build failed: {n} inputs could not be resolved.

        {blocks}",
        n = entries.len(),
    };

    // Unclassified entries still need the generic guidance in a mixed result.
    if !entries
        .iter()
        .all(|entry| legacy_unindexed_remedy(entry).is_some())
    {
        message.push_str(
            "\n\n  Possible causes (each independently): an input may not be visible to\n  \
             you, may have no published revision, or may have aged out of retention.\n  \
             Verify availability with the owner of the relevant catalog.",
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use floxhub_client::UnresolvableLeaf;

    use super::*;

    fn entry(reference: &str, chain: &[&str]) -> UnresolvableEntry {
        UnresolvableEntry {
            reference: reference.to_string(),
            chain: chain.iter().map(|s| s.to_string()).collect(),
            leaf: UnresolvableLeaf::default(),
        }
    }

    #[test]
    fn single_unresolvable() {
        // The server returns catalog-relative references; rendering restores
        // the `catalogs.` root for the developer.
        let entries = [entry("acme.tool", &["acme.app", "acme.tool"])];

        let expected = "\
'catalogs.acme.tool' is unresolvable in this context.

  Dependency path:
    catalogs.acme.app
    → catalogs.acme.tool (unresolvable)

  Possible causes: the input may not be visible to you, may have no
  published revision, or may have aged out of retention. Verify
  availability with the owner of the relevant catalog.";

        assert_eq!(render_unresolvable(&entries), expected);
    }

    #[test]
    fn multiple_unresolvable_numbered() {
        let entries = [
            entry("acme.tool", &["acme.app", "acme.tool"]),
            entry("other.lib", &["other.lib"]),
        ];

        let expected = "\
build failed: 2 inputs could not be resolved.

  1. 'catalogs.acme.tool' is unresolvable in this context.
     Dependency path:
       catalogs.acme.app
       → catalogs.acme.tool (unresolvable)

  2. 'catalogs.other.lib' is unresolvable in this context.
     Dependency path:
       catalogs.other.lib (unresolvable)

  Possible causes (each independently): an input may not be visible to
  you, may have no published revision, or may have aged out of retention.
  Verify availability with the owner of the relevant catalog.";

        assert_eq!(render_unresolvable(&entries), expected);
    }

    #[test]
    fn readable_legacy_unindexed_seed_shows_the_remedy() {
        let mut readable = entry("acme.tool", &["acme.tool"]);
        readable.leaf.unresolvable.insert(
            "legacy_unindexed".to_string(),
            serde_json::json!({"detail": "irrelevant to rendering"}),
        );
        let entries = [readable];

        let expected = "\
'catalogs.acme.tool' is unresolvable in this context.

  Dependency path:
    catalogs.acme.tool (unresolvable)

  Publish a new commit of this package with an upgraded CLI and a version 2 catalog lock.";

        assert_eq!(render_unresolvable(&entries), expected);
    }

    #[test]
    fn readable_legacy_unindexed_transitive_shows_the_remedy() {
        let mut readable = entry("acme.app", &["acme.app", "acme.tool"]);
        readable
            .leaf
            .unresolvable
            .insert("legacy_unindexed".to_string(), serde_json::json!(true));
        let entries = [readable];

        assert!(
            render_unresolvable(&entries).ends_with(
                "Publish a new commit of this package with an upgraded CLI and a version 2 catalog lock."
            )
        );
    }

    #[test]
    fn unknown_cause_key_falls_through_to_generic_rendering() {
        let mut unknown = entry("acme.tool", &["acme.tool"]);
        unknown
            .leaf
            .unresolvable
            .insert("some_other_cause".to_string(), serde_json::json!({}));
        let entries = [unknown];

        assert!(render_unresolvable(&entries).contains("Possible causes:"));
    }

    #[test]
    fn mixed_readable_and_private_preserves_privacy() {
        let mut readable = entry("acme.tool", &["acme.tool"]);
        readable
            .leaf
            .unresolvable
            .insert("legacy_unindexed".to_string(), serde_json::json!(true));
        let private = entry("acme.secret", &["acme.secret"]);
        let entries = [readable, private];

        let rendered = render_unresolvable(&entries);
        assert!(
            rendered.contains(
                "1. 'catalogs.acme.tool' is unresolvable in this context.\n     Dependency path:\n       catalogs.acme.tool (unresolvable)\n     Publish a new commit"
            ),
            "the readable entry must carry its own remedy inline, got: {rendered}"
        );
        assert!(
            !rendered.contains("acme.secret\n     Publish"),
            "the private entry must never carry the readable entry's remedy, got: {rendered}"
        );
        assert!(
            rendered.contains("Possible causes (each independently)"),
            "the shared hedge must remain for the entry rendering could not explain, got: {rendered}"
        );
    }
}
