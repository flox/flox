use std::io::Write;

use anyhow::Result;
use tabwriter::TabWriter;

use crate::beta::extensions::Extension;

/// Render the `flox extension list` table: each extension's name and the
/// source path it was installed from.
pub(super) fn render_table(extensions: &[Extension]) -> Result<String> {
    // TabWriter's minimum width applies to every column, so the NAME minimum
    // is formatted into the cells.
    let mut tw = TabWriter::new(Vec::new()).padding(2);
    writeln!(tw, "{:<20}\tPATH", "NAME")?;
    for ext in extensions {
        writeln!(tw, "{:<20}\t{}", ext.name, ext.state.source)?;
    }

    Ok(String::from_utf8(tw.into_inner()?)?)
}

#[cfg(test)]
#[cfg(feature = "beta-tests")]
mod tests {
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::beta::extensions::manifest::InstalledState;

    fn extension(name: &str, source: &str) -> Extension {
        Extension {
            name: name.to_string(),
            install_dir: format!("/home/u/.local/share/flox/extensions/{name}").into(),
            state: InstalledState {
                schema: "1".to_string(),
                name: name.to_string(),
                source: source.to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                path: format!("/home/u/.local/share/flox/extensions/{name}"),
            },
        }
    }

    #[test]
    fn render_table_sizes_name_column() {
        let cases = [
            (
                "short names keep the minimum width",
                vec![
                    extension("deploy", "/home/u/src/flox-deploy"),
                    extension("lint", "/home/u/src/flox-lint"),
                ],
                indoc! {"
                    NAME                  PATH
                    deploy                /home/u/src/flox-deploy
                    lint                  /home/u/src/flox-lint
                "},
            ),
            (
                "a long name widens the column",
                vec![
                    extension("deploy", "/home/u/src/flox-deploy"),
                    extension("a-very-long-extension-name", "/home/u/src/long"),
                ],
                indoc! {"
                    NAME                        PATH
                    deploy                      /home/u/src/flox-deploy
                    a-very-long-extension-name  /home/u/src/long
                "},
            ),
        ];

        for (name, extensions, expected) in cases {
            assert_eq!(render_table(&extensions).unwrap(), expected, "{name}");
        }
    }
}
