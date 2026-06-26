//! Developer-facing rendering of unresolvable catalog references (REQ-013).
//!
//! When a lock fails because the catalog reports `unresolvable` references, the
//! binary surfaces the dependency chains to the developer. The rendering is
//! deliberately **cause-free** (it never claims *why* a reference is
//! unresolvable — auth, missing publish, retention, etc. are indistinguishable
//! and conflating them would leak information) and **hedged** in its
//! remediation hint.

use floxhub_client::UnresolvableEntry;
use indent::indent_all_by;
use indoc::formatdoc;

/// Render the unresolvable references from a failed lock into a developer-facing
/// error body per REQ-013:
/// - a `→`-arrow dependency path per reference, ending in `(unresolvable)`,
/// - numbered entries under a `build failed: N inputs could not be resolved.`
///   header when there is more than one,
/// - a single cause-free, hedged remediation footer.
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
            format!("{arrow}{reference}{leaf}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_single(entry: &UnresolvableEntry) -> String {
    formatdoc! {"
        '{reference}' is unresolvable in this context.

          Dependency path:
        {path}

          Possible causes: the input may not be visible to you, may have no
          published revision, or may have aged out of retention. Verify
          availability with the owner of the relevant catalog.",
        reference = entry.reference,
        path = indent_all_by(4, render_path(&entry.chain)),
    }
}

fn render_many(entries: &[UnresolvableEntry]) -> String {
    let blocks = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            formatdoc! {"
                  {n}. '{reference}' is unresolvable in this context.
                     Dependency path:
                {path}",
                n = i + 1,
                reference = entry.reference,
                path = indent_all_by(7, render_path(&entry.chain)),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    formatdoc! {"
        build failed: {n} inputs could not be resolved.

        {blocks}

          Possible causes (each independently): an input may not be visible to
          you, may have no published revision, or may have aged out of retention.
          Verify availability with the owner of the relevant catalog.",
        n = entries.len(),
    }
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
        let entries = [entry("catalogs.acme.tool", &[
            "catalogs.acme.app",
            "catalogs.acme.tool",
        ])];

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
            entry("catalogs.acme.tool", &[
                "catalogs.acme.app",
                "catalogs.acme.tool",
            ]),
            entry("catalogs.other.lib", &["catalogs.other.lib"]),
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
}
