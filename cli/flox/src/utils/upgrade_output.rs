use std::collections::BTreeSet;
use std::iter;

use flox_manifest::interfaces::{AsLatestSchema, PackageLookup};
use flox_manifest::lockfile::{LockedPackage, Lockfile};
use flox_manifest::parsed::Inner;
use flox_manifest::parsed::latest::{ManifestLatest, PkgGroups};
use flox_rust_sdk::models::environment::SingleSystemUpgradeDiff;
use flox_rust_sdk::providers::lock_manifest::{BuildChange, HeldBackGroup};
use itertools::Itertools as _;
use tracing::debug;

/// Count version changes vs rebuilds in a diff.
pub(crate) fn count_upgrade_categories(diff: &SingleSystemUpgradeDiff) -> (usize, usize) {
    diff.values().fold((0, 0), |(vc, rb), (before, after)| {
        let old_version = before.version().unwrap_or("unknown");
        let new_version = after.version().unwrap_or("unknown");
        if new_version != old_version {
            (vc + 1, rb)
        } else {
            (vc, rb + 1)
        }
    })
}

/// Format a human-readable summary like "2 version changes and 1 rebuild".
pub(crate) fn format_upgrade_summary(version_changes: usize, rebuilds: usize) -> String {
    let version_part = match version_changes {
        0 => None,
        1 => Some("1 version change".to_string()),
        n => Some(format!("{n} version changes")),
    };
    let rebuild_part = match rebuilds {
        0 => None,
        1 => Some("1 rebuild".to_string()),
        n => Some(format!("{n} rebuilds")),
    };
    match (version_part, rebuild_part) {
        (Some(v), Some(b)) => format!("{v} and {b}"),
        (Some(v), None) => v,
        (None, Some(b)) => b,
        (None, None) => "Upgrades".to_string(),
    }
}

/// Render a diff of locked packages before and after an upgrade.
///
/// Version changes show: `- pkg: 1.0 -> 2.0`
/// Rebuilds show: `- pkg: 1.0 (rebuild, rev DATE -> DATE)` with fallback to
/// rev hash or bare `(rebuild)` when rev info is unavailable.
pub(crate) fn render_diff(diff: &SingleSystemUpgradeDiff) -> String {
    use itertools::Itertools as _;
    diff.iter()
        .map(|(_, (before, after))| {
            let install_id = before.install_id();
            let old_version = before.version().unwrap_or("unknown");
            let new_version = after.version().unwrap_or("unknown");

            if new_version != old_version {
                return format!("- {install_id}: {old_version} -> {new_version}");
            }

            match rebuild_detail(before, after) {
                Some(detail) => format!("- {install_id}: {old_version} (rebuild, {detail})"),
                None => format!("- {install_id}: {old_version} (rebuild)"),
            }
        })
        .join("\n")
}

/// Explain each group that some of its packages keep from upgrading, if any,
/// for Flox running on `system`.
///
/// `lockfile` is the lock the upgrade produced, which has the merged manifest
/// and the manifest the user edits.
pub(crate) fn render_held_back_groups(
    groups: &[HeldBackGroup],
    lockfile: &Lockfile,
    system: &str,
) -> Option<String> {
    let (merged, user) = match (
        lockfile.migrated_manifest(),
        lockfile.migrated_user_manifest(),
    ) {
        (Ok(merged), Ok(user)) => (merged, user),
        (Err(err), _) | (_, Err(err)) => {
            debug!(%err, "couldn't read the manifests to explain what holds a group back");
            return None;
        },
    };
    let (merged, user) = (merged.as_latest_schema(), user.as_latest_schema());
    let notices = groups
        .iter()
        .map(|group| render_held_back_group(group, merged, user, system))
        .collect_vec();
    (!notices.is_empty()).then(|| notices.join("\n\n"))
}

/// The dotted key that sets the package group of `install_id`, which names
/// that setting the same way whether the manifest writes the package as a
/// dotted key, an inline table or an `[install.<ID>]` table.
fn pkg_group_key(install_id: &str) -> String {
    format!("install.{}.pkg-group", toml_edit::Key::new(install_id))
}

/// `value` as a TOML string to copy, which [wrap] doesn't break.
fn toml_string(value: &str) -> String {
    toml_edit::Value::from(value)
        .to_string()
        .trim()
        .replace(' ', &NO_BREAK_SPACE.to_string())
}

/// A name for the package group of `install_id` alone that no package group in
/// `manifest` has and no other package has as an install ID, which
/// `flox upgrade <NAME>` would take for the package.
fn new_group_name(install_id: &str, manifest: &ManifestLatest) -> String {
    let groups = manifest
        .catalog_pkgs_by_group()
        .into_keys()
        .chain(manifest.pkg_groups.inner().keys().cloned())
        .collect::<BTreeSet<_>>();
    let is_other_install_id =
        |name: &String| name != install_id && manifest.install.inner().contains_key(name);
    iter::once(install_id.to_string())
        .chain((2..).map(|n| format!("{install_id}-{n}")))
        .find(|name| !groups.contains(name) && !is_other_install_id(name))
        .expect("there are unboundedly many names")
}

/// Explain one group.
///
/// Each holder is isolated into a group of its own in the manifest the user
/// edits. A holder that only an included environment declares is declared
/// there first, because the descriptor in the composing manifest replaces the
/// included one whole (`merge_install` in flox-manifest at
/// src/compose/shallow.rs).
///
/// A holder that a build lists in `runtime-packages` can't be isolated:
/// `flox edit` refuses a manifest where such a package isn't in 'toplevel'
/// (`buildenv/builder.pl` in this repository). The notice then says so, for
/// each such holder, instead of giving steps to isolate any holder.
fn render_held_back_group(
    group: &HeldBackGroup,
    merged: &ManifestLatest,
    user: &ManifestLatest,
    system: &str,
) -> String {
    let changes = group
        .different_builds
        .iter()
        .map(|(install_id, by_system)| match by_system.get(system) {
            Some(BuildChange {
                version,
                newest_version,
            }) if version == newest_version => format!("- {install_id}: {version} (rebuild)"),
            Some(BuildChange {
                version,
                newest_version,
            }) => format!("- {install_id}: {version} -> {newest_version}"),
            None => format!("- {install_id}: on other systems only"),
        })
        .collect_vec();

    let holders = group.holding_back.keys().map(String::as_str).collect_vec();
    let holders_list = quoted_list(&holders);

    let holding = if holders.len() > 1 {
        format!(
            "{holders_list} are holding the package group {} back and prevent other packages from upgrading.",
            quoted(&group.group)
        )
    } else {
        format!(
            "{holders_list} is holding the package group {} back and prevents other packages from upgrading.",
            quoted(&group.group)
        )
    };
    let mut paragraphs = vec![
        wrap(&[holding]),
        format!(
            "{}\n{}",
            wrap(&[format!(
                "Without {holders_list}, these packages have upgrades:"
            )]),
            changes.join("\n")
        ),
    ];
    let listed_in_builds = holders
        .iter()
        .filter_map(|holder| {
            let builds = merged
                .build
                .inner()
                .iter()
                .filter(|(_, build)| {
                    build
                        .runtime_packages
                        .as_ref()
                        .is_some_and(|listed| listed.iter().any(|name| name == holder))
                })
                .map(|(name, _)| name.as_str())
                .collect_vec();
            (!builds.is_empty()).then_some((*holder, builds))
        })
        .collect_vec();
    if !listed_in_builds.is_empty() {
        paragraphs.push(wrap(
            &listed_in_builds
                .iter()
                .map(|(holder, builds)| {
                    let (noun, verb) = if builds.len() > 1 {
                        ("builds", "list")
                    } else {
                        ("build", "lists")
                    };
                    format!(
                        "{} can't be isolated into its own package group, because {noun} {} {verb} it in {}.",
                        quoted(holder),
                        quoted_list(builds),
                        quoted("runtime-packages")
                    )
                })
                .collect_vec(),
        ));
    } else {
        paragraphs.push(wrap(&isolate_steps(&holders, group, merged, user)));
    }
    paragraphs.join("\n\n")
}

/// The steps that isolate each of `holders` into a group of its own.
fn isolate_steps(
    holders: &[&str],
    group: &HeldBackGroup,
    merged: &ManifestLatest,
    user: &ManifestLatest,
) -> Vec<String> {
    let holders_list = quoted_list(holders);
    let own_groups = if holders.len() > 1 {
        "their own package groups"
    } else {
        "its own package group"
    };
    let mut steps = vec![format!(
        "To allow other packages to upgrade, isolate {holders_list} into {own_groups}."
    )];
    steps.extend(declare_step(holders, user));
    // A package isolated into a group of its own leaves the settings of the
    // group it was in, which the new group doesn't have.
    let stability = merged.group_stability_override(&group.group);
    let sets = holders
        .iter()
        .flat_map(|install_id| {
            let name = new_group_name(install_id, merged);
            let pkg_group = format!(
                "{} to {}",
                quoted(&pkg_group_key(install_id)),
                toml_string(&name)
            );
            let stability = stability.map(|stability| {
                format!(
                    "{} to {}",
                    quoted(&format!("{}.stability", PkgGroups::key_path(&name))),
                    toml_string(stability)
                )
            });
            [Some(pkg_group), stability].into_iter().flatten()
        })
        .collect_vec();
    steps.push(format!(
        "Run {} and set {}.",
        quoted("flox edit"),
        join_and(&sets)
    ));
    steps.push(format!("Then run {}.", quoted("flox upgrade")));
    steps
}

/// The sentence that says to declare the packages among `holders` that only an
/// included environment declares, because the descriptor in this manifest
/// replaces the included one whole.
fn declare_step(holders: &[&str], user: &ManifestLatest) -> Option<String> {
    let undeclared = holders
        .iter()
        .copied()
        .filter(|install_id| user.pkg_descriptor_with_id(install_id).is_none())
        .collect_vec();
    match undeclared.as_slice() {
        [] => None,
        [only] => Some(format!(
            "{} comes from an included environment, so first declare it in this manifest, with the options it has there.",
            quoted(only)
        )),
        _ => Some(format!(
            "{} come from included environments, so first declare them in this manifest, with the options they have there.",
            quoted_list(&undeclared)
        )),
    }
}

/// Stands in for a space inside text that has to be copied whole, which
/// [wrap] doesn't break at.
const NO_BREAK_SPACE: char = '\u{a0}';

/// `text` in single quotes, for a key, a name or a command to copy.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace(' ', &NO_BREAK_SPACE.to_string()))
}

/// Each sentence on its own line, wrapped at 80 columns, between the
/// words and not inside them, so a line that has a long name or a key in it
/// is longer.
fn wrap(sentences: &[String]) -> String {
    let options = textwrap::Options::new(80)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .break_words(false);
    sentences
        .iter()
        .map(|sentence| textwrap::fill(sentence, &options))
        .join("\n")
        .replace(NO_BREAK_SPACE, " ")
}

/// `'a'`, `'a' and 'b'` or `'a', 'b' and 'c'`.
fn quoted_list(names: &[&str]) -> String {
    join_and(&names.iter().map(|name| quoted(name)).collect_vec())
}

/// `a`, `a and b` or `a, b and c`.
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// Extract a human-readable detail string for build-only changes.
///
/// Tries rev_date first (formatted as YYYY-MM-DD), then rev hash (7 chars).
/// Returns `None` if no rev info is available (e.g. flake packages).
pub(crate) fn rebuild_detail(before: &LockedPackage, after: &LockedPackage) -> Option<String> {
    let (old, new) = (
        before.as_catalog_package_ref()?,
        after.as_catalog_package_ref()?,
    );

    let old_date = old.rev_date.format("%Y-%m-%d");
    let new_date = new.rev_date.format("%Y-%m-%d");
    if old_date.to_string() != new_date.to_string() {
        return Some(format!("rev {old_date} -> {new_date}"));
    }

    let old_rev = &old.rev[..7.min(old.rev.len())];
    let new_rev = &new.rev[..7.min(new.rev.len())];
    if old_rev != new_rev {
        return Some(format!("rev {old_rev} -> {new_rev}"));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use flox_manifest::interfaces::AsTypedOnlyManifest;
    use flox_manifest::lockfile::Compose;
    use flox_manifest::raw::test_helpers::mk_test_manifest_from_contents;
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_rust_sdk::providers::lock_manifest::HolderSystems;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn version_change_singular() {
        assert_eq!(format_upgrade_summary(1, 0), "1 version change");
    }

    #[test]
    fn version_changes_plural() {
        assert_eq!(format_upgrade_summary(3, 0), "3 version changes");
    }

    #[test]
    fn rebuild_singular() {
        assert_eq!(format_upgrade_summary(0, 1), "1 rebuild");
    }

    #[test]
    fn rebuilds_plural() {
        assert_eq!(format_upgrade_summary(0, 4), "4 rebuilds");
    }

    #[test]
    fn mixed() {
        assert_eq!(
            format_upgrade_summary(2, 1),
            "2 version changes and 1 rebuild"
        );
    }

    #[test]
    fn mixed_plural() {
        assert_eq!(
            format_upgrade_summary(3, 5),
            "3 version changes and 5 rebuilds"
        );
    }

    #[test]
    fn fallback_when_zero() {
        assert_eq!(format_upgrade_summary(0, 0), "Upgrades");
    }

    #[test]
    fn hello_changes_on_this_system_with_curl_holding_back() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from([
                    "aarch64-darwin".to_string(),
                    "x86_64-linux".to_string(),
                ]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'."#});
    }

    /// Every other package whose build differs is listed, whatever the
    /// versions are: a rebuild, an older version, and versions that can't be
    /// ordered.
    #[test]
    fn every_differing_build_is_listed() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
                jq.pkg-path = "jq"
                bat.pkg-path = "bat"
                yq.pkg-path = "yq"
                vim.pkg-path = "vim"
                tzdata.pkg-path = "tzdata"
                openssh.pkg-path = "openssh"
                nmap.pkg-path = "nmap"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([
                (
                    "bat".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "0.24.0".to_string(),
                        newest_version: "0.23.0".to_string(),
                    })]),
                ),
                (
                    "hello".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "2.9".to_string(),
                        newest_version: "2.12.3".to_string(),
                    })]),
                ),
                (
                    "jq".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "1.7.1".to_string(),
                        newest_version: "1.7.1".to_string(),
                    })]),
                ),
                (
                    "nmap".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "1.2.3.4".to_string(),
                        newest_version: "1.2.3.5".to_string(),
                    })]),
                ),
                (
                    "openssh".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "9.9p1".to_string(),
                        newest_version: "10.0p2".to_string(),
                    })]),
                ),
                (
                    "tzdata".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "2024.12.06".to_string(),
                        newest_version: "2025.01.26".to_string(),
                    })]),
                ),
                (
                    "vim".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "9.1.0905".to_string(),
                        newest_version: "9.1.1122".to_string(),
                    })]),
                ),
                (
                    "yq".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "1.2.3-unstable-2024-01-01".to_string(),
                        newest_version: "1.2.3".to_string(),
                    })]),
                ),
            ]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - bat: 0.24.0 -> 0.23.0
            - hello: 2.9 -> 2.12.3
            - jq: 1.7.1 (rebuild)
            - nmap: 1.2.3.4 -> 1.2.3.5
            - openssh: 9.9p1 -> 10.0p2
            - tzdata: 2024.12.06 -> 2025.01.26
            - vim: 9.1.0905 -> 9.1.1122
            - yq: 1.2.3-unstable-2024-01-01 -> 1.2.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'."#});
    }

    /// A package that changes only on other systems is listed as such, not
    /// left out and not given another system's versions.
    #[test]
    fn package_changing_on_other_systems_only_is_listed_as_such() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
                jq.pkg-path = "jq"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([
                (
                    "hello".to_string(),
                    BTreeMap::from([("aarch64-darwin".to_string(), BuildChange {
                        version: "2.12.1".to_string(),
                        newest_version: "2.12.3".to_string(),
                    })]),
                ),
                (
                    "jq".to_string(),
                    BTreeMap::from([
                        ("aarch64-darwin".to_string(), BuildChange {
                            version: "1.7.0".to_string(),
                            newest_version: "1.7.1".to_string(),
                        }),
                        ("x86_64-linux".to_string(), BuildChange {
                            version: "1.7.0".to_string(),
                            newest_version: "1.8.0".to_string(),
                        }),
                    ]),
                ),
            ]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: on other systems only
            - jq: 1.7.0 -> 1.8.0

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'."#});
    }

    /// A list of only lower versions is printed like any other, since a lower
    /// version from a newer revision is still a change of the package.
    #[test]
    fn list_of_only_lower_versions_is_printed() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
                bat.pkg-path = "bat"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([
                (
                    "bat".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "0.24.0".to_string(),
                        newest_version: "0.23.0".to_string(),
                    })]),
                ),
                (
                    "hello".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "2.12.3".to_string(),
                        newest_version: "2.12.1".to_string(),
                    })]),
                ),
            ]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - bat: 0.24.0 -> 0.23.0
            - hello: 2.12.3 -> 2.12.1

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'."#});
    }

    /// A group name that the manifest already uses, for a group of packages
    /// or for settings, is not offered again.
    #[test]
    fn group_name_for_the_holder_is_not_one_the_manifest_uses() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                curl.pkg-group = "curl"
                hello.pkg-path = "hello"
                hello.pkg-group = "curl"

                [pkg-groups.curl-2]
                stability = "stable"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "curl".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'curl' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl-3".
            Then run 'flox upgrade'."#});
    }

    /// `flox upgrade curl-2` would name the package `curl-2` as well as a
    /// group of that name, so another package's install ID isn't offered as
    /// a group name.
    #[test]
    fn group_name_for_the_holder_is_not_the_install_id_of_another_package() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                curl.pkg-group = "curl"
                hello.pkg-path = "hello"
                hello.pkg-group = "curl"
                curl-2.pkg-path = "wget"
                curl-2.pkg-group = "downloads"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "curl".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'curl' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl-3".
            Then run 'flox upgrade'."#});
    }

    /// An install ID that isn't a bare TOML key is quoted in the key.
    #[test]
    fn install_id_that_is_not_a_bare_key_is_quoted() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                "curl.8".pkg-path = "curl"
                hello.pkg-path = "hello"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl.8".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl.8' is holding the package group 'toplevel' back and prevents other
            packages from upgrading.

            Without 'curl.8', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl.8' into its own package group.
            Run 'flox edit' and set 'install."curl.8".pkg-group' to "curl.8".
            Then run 'flox upgrade'."#});
    }

    /// A package isolated into a group of its own would lose the stability
    /// that its group sets, so the notice says to set it on the new group.
    #[test]
    fn stability_of_the_group_is_set_on_the_new_group() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"

                [pkg-groups.toplevel]
                stability = "lts"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl" and
            'pkg-groups.curl.stability' to "lts".
            Then run 'flox upgrade'."#});
    }

    /// A package that only an included environment declares is declared in this
    /// manifest before its package group is set, because the descriptor in this
    /// manifest replaces the included one whole.
    #[test]
    fn holder_from_an_included_environment_is_declared_in_this_manifest_first() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
            "#}))
            .as_typed_only(),
            compose: Some(Compose {
                composer: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                hello.pkg-path = "hello"
            "#}))
                .as_typed_only(),
                include: vec![],
                warnings: vec![],
            }),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            'curl' comes from an included environment, so first declare it in this manifest,
            with the options it has there.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'."#});
    }

    /// Holders that an included environment declares are declared together, and
    /// the stability of the group, which only an included environment sets, is
    /// set for each holder, whether this manifest declares it or not.
    #[test]
    fn holders_from_included_environments_are_declared_in_this_manifest_first() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                bat.pkg-path = "bat"
                bun.pkg-path = "bun"
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"

                [pkg-groups.toplevel]
                stability = "lts"
            "#}))
            .as_typed_only(),
            compose: Some(Compose {
                composer: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                bat.pkg-path = "bat"
                hello.pkg-path = "hello"
            "#}))
                .as_typed_only(),
                include: vec![],
                warnings: vec![],
            }),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([
                ("bat".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                }),
                ("bun".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                }),
                ("curl".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                }),
            ]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'bat', 'bun' and 'curl' are holding the package group 'toplevel' back and
            prevent other packages from upgrading.

            Without 'bat', 'bun' and 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'bat', 'bun' and 'curl' into their
            own package groups.
            'bun' and 'curl' come from included environments, so first declare them in this
            manifest, with the options they have there.
            Run 'flox edit' and set 'install.bat.pkg-group' to "bat",
            'pkg-groups.bat.stability' to "lts", 'install.bun.pkg-group' to "bun",
            'pkg-groups.bun.stability' to "lts", 'install.curl.pkg-group' to "curl" and
            'pkg-groups.curl.stability' to "lts".
            Then run 'flox upgrade'."#});
    }

    /// A key, a name or a command to copy stays on one line, so a long one makes
    /// the line longer than 80 columns.
    #[test]
    fn names_to_copy_are_never_split_at_a_space_or_a_slash() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                "my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns".pkg-path = "curl"
                hello.pkg-path = "hello"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([(
                "my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns"
                    .to_string(),
                HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                },
            )]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns'
            is holding the package group 'toplevel' back and prevents other packages from
            upgrading.

            Without
            'my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns',
            these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate
            'my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns'
            into its own package group.
            Run 'flox edit' and set
            'install."my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns".pkg-group'
            to
            "my package name with spaces and a path/like/this/that/goes/on/past/eighty/columns".
            Then run 'flox upgrade'."#});
    }

    #[test]
    fn stability_key_of_the_new_group_is_quoted() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                "my.pkg".pkg-path = "curl"
                hello.pkg-path = "hello"

                [pkg-groups.toplevel]
                stability = "lts"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("my.pkg".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'my.pkg' is holding the package group 'toplevel' back and prevents other
            packages from upgrading.

            Without 'my.pkg', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'my.pkg' into its own package group.
            Run 'flox edit' and set 'install."my.pkg".pkg-group' to "my.pkg" and
            'pkg-groups."my.pkg".stability' to "lts".
            Then run 'flox upgrade'."#});
    }

    /// A name with a backslash and a quote in it is escaped in the key and in
    /// the value to copy.
    #[test]
    fn backslash_and_quote_in_the_group_name_are_escaped() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                "a\\b'c".pkg-path = "curl"
                hello.pkg-path = "hello"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("a\\b'c".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'a\b'c' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'a\b'c', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'a\b'c' into its own package group.
            Run 'flox edit' and set 'install."a\\b'c".pkg-group' to '''a\b'c'''.
            Then run 'flox upgrade'."#});
    }

    /// A package that a build lists in `runtime-packages` has to stay in
    /// 'toplevel', so the notice says why it can't be isolated and gives no steps.
    #[test]
    fn holder_listed_in_runtime_packages_of_a_build_cannot_be_isolated() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"

                [build.hello-build]
                command = "echo hello"
                runtime-packages = ["curl", "hello"]
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            'curl' can't be isolated into its own package group, because build 'hello-build'
            lists it in 'runtime-packages'."#});
    }

    /// A build that only an included environment declares also keeps its
    /// `runtime-packages` in 'toplevel'.
    #[test]
    fn holder_listed_in_runtime_packages_of_a_build_from_an_included_environment() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"

                [build.hello-build]
                command = "echo hello"
                runtime-packages = ["curl", "hello"]
            "#}))
            .as_typed_only(),
            compose: Some(Compose {
                composer: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                    [install]
                    curl.pkg-path = "curl"
                    hello.pkg-path = "hello"
                "#}))
                .as_typed_only(),
                include: vec![],
                warnings: vec![],
            }),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            'curl' can't be isolated into its own package group, because build 'hello-build'
            lists it in 'runtime-packages'."#});
    }

    /// Each build that lists the holder is named.
    #[test]
    fn holder_listed_by_several_builds_names_each_build() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"

                [build.hello-build]
                command = "echo hello"
                runtime-packages = ["curl"]

                [build.other-build]
                command = "echo other"
                runtime-packages = ["curl", "hello"]
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            'curl' can't be isolated into its own package group, because builds
            'hello-build' and 'other-build' list it in 'runtime-packages'."#});
    }

    /// With several holders, one that a build lists keeps the group from being
    /// isolated at all, so no steps are given for the others either.
    #[test]
    fn no_holder_is_isolated_when_one_is_listed_in_runtime_packages() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
                wget.pkg-path = "wget"

                [build.hello-build]
                command = "echo hello"
                runtime-packages = ["wget"]
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([
                ("curl".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                }),
                ("wget".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                }),
            ]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' and 'wget' are holding the package group 'toplevel' back and prevent
            other packages from upgrading.

            Without 'curl' and 'wget', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            'wget' can't be isolated into its own package group, because build 'hello-build'
            lists it in 'runtime-packages'."#});
    }

    /// A group that an included environment gives a package is taken, so the
    /// new group has another name.
    #[test]
    fn group_name_avoids_a_group_that_only_an_included_environment_has() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
                wget.pkg-path = "wget"
                wget.pkg-group = "curl"
            "#}))
            .as_typed_only(),
            compose: Some(Compose {
                composer: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                    [install]
                    curl.pkg-path = "curl"
                    hello.pkg-path = "hello"
                "#}))
                .as_typed_only(),
                include: vec![],
                warnings: vec![],
            }),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl-2".
            Then run 'flox upgrade'."#});
    }

    /// The stability in `[options]` applies to every group, so the new group has
    /// it without being told, and only a stability of the old group is set on it.
    #[test]
    fn stability_of_the_environment_is_not_set_on_the_new_group() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"

                [options]
                stability = "stable"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                with_build: BTreeSet::new(),
            })]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'."#});
    }

    /// Several holders are each given a group of their own, and a holder
    /// that has builds on some systems is no different from one that has none.
    #[test]
    fn several_holders_are_each_given_a_group_of_their_own() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                yq.pkg-path = "yq"
                hello.pkg-path = "hello"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![HeldBackGroup {
            group: "toplevel".to_string(),
            holding_back: BTreeMap::from([
                ("curl".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["aarch64-darwin".to_string()]),
                    with_build: BTreeSet::from(["x86_64-linux".to_string()]),
                }),
                ("yq".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                }),
            ]),
            different_builds: BTreeMap::from([(
                "hello".to_string(),
                BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                    version: "2.12.1".to_string(),
                    newest_version: "2.12.3".to_string(),
                })]),
            )]),
        }];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' and 'yq' are holding the package group 'toplevel' back and prevent other
            packages from upgrading.

            Without 'curl' and 'yq', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' and 'yq' into their own
            package groups.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl" and
            'install.yq.pkg-group' to "yq".
            Then run 'flox upgrade'."#});
    }

    /// Each group that something holds back is explained, one after another.
    #[test]
    fn every_held_back_group_is_explained() {
        let lockfile = Lockfile {
            manifest: mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
                [install]
                curl.pkg-path = "curl"
                hello.pkg-path = "hello"
                wget.pkg-path = "wget"
                wget.pkg-group = "tools"
                jq.pkg-path = "jq"
                jq.pkg-group = "tools"
            "#}))
            .as_typed_only(),
            ..Lockfile::default()
        };
        let groups = vec![
            HeldBackGroup {
                group: "toplevel".to_string(),
                holding_back: BTreeMap::from([("curl".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                })]),
                different_builds: BTreeMap::from([(
                    "hello".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "2.12.1".to_string(),
                        newest_version: "2.12.3".to_string(),
                    })]),
                )]),
            },
            HeldBackGroup {
                group: "tools".to_string(),
                holding_back: BTreeMap::from([("wget".to_string(), HolderSystems {
                    without_build: BTreeSet::from(["x86_64-linux".to_string()]),
                    with_build: BTreeSet::new(),
                })]),
                different_builds: BTreeMap::from([(
                    "jq".to_string(),
                    BTreeMap::from([("x86_64-linux".to_string(), BuildChange {
                        version: "1.7.0".to_string(),
                        newest_version: "1.7.1".to_string(),
                    })]),
                )]),
            },
        ];

        let notice = render_held_back_groups(&groups, &lockfile, "x86_64-linux").unwrap();

        assert_eq!(notice, indoc! {r#"
            'curl' is holding the package group 'toplevel' back and prevents other packages
            from upgrading.

            Without 'curl', these packages have upgrades:
            - hello: 2.12.1 -> 2.12.3

            To allow other packages to upgrade, isolate 'curl' into its own package group.
            Run 'flox edit' and set 'install.curl.pkg-group' to "curl".
            Then run 'flox upgrade'.

            'wget' is holding the package group 'tools' back and prevents other packages
            from upgrading.

            Without 'wget', these packages have upgrades:
            - jq: 1.7.0 -> 1.7.1

            To allow other packages to upgrade, isolate 'wget' into its own package group.
            Run 'flox edit' and set 'install.wget.pkg-group' to "wget".
            Then run 'flox upgrade'."#});
    }
}
