//! Context-aware replay of PATH-like variables recorded in a start diff.
//!
//! The start diff records the final values that the profile.d scripts and
//! `hook.on-activate` produced, and those values bake in the environment
//! stack that was active when the start ran (e.g.
//! `CPATH=<this env>/include:<other env>/include`). Replaying them verbatim
//! into an attach that runs under a different stack both leaks the start
//! context's directories and drops the attach context's directories.
//!
//! For a known set of PATH-like variables we instead replay only the
//! *segments the start added* (end value minus start value), drop segments
//! that belong to environments which were active at start but are not part
//! of the attach context, and splice the survivors into the attach
//! context's current value at the position (prepend or append) they
//! originally occupied. Scalar variables keep verbatim replay.
//!
//! The replay is additive: a hook that removes or clears segments of a
//! PATH-like variable is not replayed as a removal.

use std::collections::{HashMap, HashSet};

/// PATH-like variables whose start-diff values are replayed relative to the
/// attach context. This covers the variables the interpreter's profile.d
/// scripts manage plus the common search paths hooks prepend to.
pub(crate) const REPLAYED_PATHLIKE_VARS: &[&str] = &[
    "PATH",
    "MANPATH",
    "INFOPATH",
    "XDG_DATA_DIRS",
    "CPATH",
    "LIBRARY_PATH",
    "LD_LIBRARY_PATH",
    "PKG_CONFIG_PATH",
    "ACLOCAL_PATH",
    "JUPYTER_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "PYTHONPATH",
    "CMAKE_PREFIX_PATH",
    "LD_FLOXLIB_FILES_PATH",
];

/// Inputs shared by every PATH-like translation of one attach.
#[derive(Debug, Clone)]
pub(crate) struct PathlikeReplayCtx {
    /// Environment directories in `FLOX_ENV_DIRS` when the start ran.
    start_env_dirs: Vec<String>,
    /// The subset of `start_env_dirs` that is not part of the attach
    /// context. Segments under these directories belong to other
    /// environments' activations and must not be replayed.
    stale_env_dirs: Vec<String>,
}

impl PathlikeReplayCtx {
    pub(crate) fn new(start_env_dirs: Option<&str>, attach_env_dirs: &str) -> Self {
        let attach_dirs: HashSet<&str> = attach_env_dirs
            .split(':')
            .filter(|dir| !dir.is_empty())
            .collect();
        let start_env_dirs: Vec<String> = start_env_dirs
            .unwrap_or_default()
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(String::from)
            .collect();
        let stale_env_dirs = start_env_dirs
            .iter()
            .filter(|dir| !attach_dirs.contains(dir.as_str()))
            .cloned()
            .collect();
        Self {
            start_env_dirs,
            stale_env_dirs,
        }
    }

    /// Whether a recorded segment points into an environment that is not
    /// part of the attach context.
    fn is_stale(&self, segment: &str) -> bool {
        Self::under_any(segment, &self.stale_env_dirs)
    }

    /// Whether a recorded segment points into any environment that was
    /// active when the start ran.
    fn is_env_derived(&self, segment: &str) -> bool {
        Self::under_any(segment, &self.start_env_dirs)
    }

    fn under_any(segment: &str, dirs: &[String]) -> bool {
        dirs.iter().any(|dir| {
            segment == dir
                || segment
                    .strip_prefix(dir.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// Replay a recorded PATH-like addition against the attach context.
    ///
    /// `end_value`/`start_value` come from the start diff; `current_value`
    /// is the attach context's value of the variable before this
    /// activation applies (for PATH and MANPATH, the value fix-paths
    /// recomputed for this attach).
    pub(crate) fn translate(
        &self,
        end_value: &str,
        start_value: Option<&str>,
        current_value: Option<&str>,
    ) -> String {
        let end_segments: Vec<&str> = end_value.split(':').collect();
        // Empty segments are structural markers (a leading or trailing ':'
        // means "insert the tool's default search path here" for MANPATH
        // and INFOPATH), not set members; they are excluded from the
        // segment algebra and re-attached below.
        let start_sequence: Vec<&str> = start_value
            .map(|value| value.split(':').filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        let start_set: HashSet<&str> = start_sequence.iter().copied().collect();

        // Where the start value's surviving content begins in the end
        // value. Everything before the anchor was prepended by the start —
        // including a segment that already existed and was re-prepended to
        // raise its precedence.
        let anchor = start_sequence
            .iter()
            .find_map(|old| end_segments.iter().position(|segment| segment == old));
        let last_old = end_segments
            .iter()
            .rposition(|segment| start_set.contains(segment));

        // For a value the start created from scratch, a trailing run of
        // segments outside every start-time environment is a default
        // fallback suffix (e.g. DYLD_FALLBACK_LIBRARY_PATH's
        // "/usr/local/lib:/usr/lib") and belongs after the attach
        // context's entries. Without any environment-derived segment there
        // is no such structure to detect, so everything stays a prepend.
        let mut created_appends_from = end_segments.len();
        if anchor.is_none()
            && end_segments
                .iter()
                .any(|segment| !segment.is_empty() && self.is_env_derived(segment))
        {
            for (position, segment) in end_segments.iter().enumerate().rev() {
                if segment.is_empty() {
                    continue;
                }
                if self.is_env_derived(segment) {
                    break;
                }
                created_appends_from = position;
            }
        }

        let mut prepends: Vec<&str> = Vec::new();
        let mut appends: Vec<&str> = Vec::new();
        for (position, segment) in end_segments.iter().enumerate() {
            if segment.is_empty() || self.is_stale(segment) {
                continue;
            }
            let in_prepend_block = anchor.is_some_and(|anchor| position < anchor);
            if !in_prepend_block && start_set.contains(segment) {
                continue;
            }
            // Segments interleaved between surviving old segments count as
            // prepends.
            let is_append = position >= created_appends_from
                || (!in_prepend_block && last_old.is_some_and(|last| position > last));
            let target = if is_append {
                &mut appends
            } else {
                &mut prepends
            };
            if !target.contains(segment) {
                target.push(segment);
            }
        }
        appends.retain(|segment| !prepends.contains(segment));

        let Some(current) = current_value else {
            // The variable is unset in the attach context: the added
            // segments (minus stale ones) are the whole value, keeping the
            // recorded default-search-path markers.
            let mut segments = prepends;
            segments.extend(appends);
            if segments.is_empty() {
                return String::new();
            }
            let mut result = segments.join(":");
            if end_value.starts_with(':') {
                result.insert(0, ':');
            }
            if end_value.ends_with(':') {
                result.push(':');
            }
            return result;
        };

        // Splice into the attach context's value. A prepended segment that
        // already appears in the current value moves to the front (the
        // start raised its precedence); the current value's own structure,
        // including empty segments, is otherwise preserved.
        let prepend_set: HashSet<&str> = prepends.iter().copied().collect();
        let kept_current: Vec<&str> = current
            .split(':')
            .filter(|segment| segment.is_empty() || !prepend_set.contains(segment))
            .collect();
        appends.retain(|segment| !kept_current.contains(segment));
        if prepends.is_empty() && appends.is_empty() {
            return current.to_string();
        }
        let mut segments = prepends;
        segments.extend(kept_current);
        segments.extend(appends);
        segments.join(":")
    }
}

/// Build the per-variable current values used as translation bases.
///
/// PATH and MANPATH use the values recomputed for this attach by
/// fix-paths (`fixed_vars`) so hook additions layer on top of a correct
/// base; everything else uses the attaching process's environment.
pub(crate) fn current_value<'a>(
    name: &str,
    fixed_vars: &'a HashMap<&'static str, String>,
    current_env: &'a HashMap<String, String>,
) -> Option<&'a str> {
    fixed_vars
        .get(name)
        .or_else(|| current_env.get(name))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THIS_ENV: &str = "/envs/pi";
    const START_PEER: &str = "/envs/core";
    const ATTACH_PEER: &str = "/envs/wandb";

    fn ctx() -> PathlikeReplayCtx {
        // Start ran under pi:core, attach runs under pi:wandb.
        PathlikeReplayCtx::new(
            Some(&format!("{THIS_ENV}:{START_PEER}")),
            &format!("{THIS_ENV}:{ATTACH_PEER}"),
        )
    }

    #[test]
    fn prepended_segment_moves_to_attach_context() {
        // CPATH-style: profile.d prepended this env's include dir to the
        // start context's value; attach keeps only the new segment and
        // prepends it to the attach context's value.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/include:{START_PEER}/include"),
            Some(&format!("{START_PEER}/include")),
            Some(&format!("{ATTACH_PEER}/include")),
        );
        assert_eq!(
            translated,
            format!("{THIS_ENV}/include:{ATTACH_PEER}/include")
        );
    }

    #[test]
    fn stale_env_segments_are_dropped_when_var_was_unset() {
        // PYTHONPATH-style: the variable did not exist at start, so the
        // whole recorded value is "new", but segments under the start
        // context's other environments are dropped.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/site-packages:{START_PEER}/site-packages"),
            None,
            None,
        );
        assert_eq!(translated, format!("{THIS_ENV}/site-packages"));
    }

    #[test]
    fn sibling_of_stale_env_dir_is_not_stale() {
        // The boundary check matters: /envs/core is stale but
        // /envs/core-extra is a different directory and must survive.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/lib:{START_PEER}-extra/lib:{START_PEER}"),
            None,
            None,
        );
        assert_eq!(translated, format!("{THIS_ENV}/lib:{START_PEER}-extra/lib"));
    }

    #[test]
    fn segments_for_envs_active_at_attach_are_kept() {
        // If the start-context peer is also active in the attach context,
        // its segments are not stale.
        let ctx = PathlikeReplayCtx::new(
            Some(&format!("{THIS_ENV}:{START_PEER}")),
            &format!("{THIS_ENV}:{ATTACH_PEER}:{START_PEER}"),
        );
        let translated = ctx.translate(&format!("{THIS_ENV}/lib:{START_PEER}/lib"), None, None);
        assert_eq!(translated, format!("{THIS_ENV}/lib:{START_PEER}/lib"));
    }

    #[test]
    fn hook_prepend_layers_onto_current_value() {
        // PATH-style: the hook prepended a non-environment dir; it is
        // spliced onto the attach context's recomputed PATH.
        let translated = ctx().translate(
            &format!("/pi-tools:{THIS_ENV}/bin:{START_PEER}/bin:/usr/bin"),
            Some(&format!("{THIS_ENV}/bin:{START_PEER}/bin:/usr/bin")),
            Some(&format!("{THIS_ENV}/bin:{ATTACH_PEER}/bin:/usr/bin")),
        );
        assert_eq!(
            translated,
            format!("/pi-tools:{THIS_ENV}/bin:{ATTACH_PEER}/bin:/usr/bin")
        );
    }

    #[test]
    fn hook_reprioritizing_existing_dir_moves_it_to_the_front() {
        // A hook doing `export PATH="$HOME/.local/bin:$PATH"` where the dir
        // is already on PATH re-prepends an existing segment. The replay
        // preserves that precedence by moving the segment to the front of
        // the attach context's value — in both the shape bash produces
        // (duplicate left behind) and the deduplicated shape.
        let start = format!("{THIS_ENV}/bin:/usr/bin:/home/u/.local/bin");
        for end in [
            format!("/home/u/.local/bin:{THIS_ENV}/bin:/usr/bin:/home/u/.local/bin"),
            format!("/home/u/.local/bin:{THIS_ENV}/bin:/usr/bin"),
        ] {
            let translated = ctx().translate(&end, Some(&start), Some(&start));
            assert_eq!(
                translated,
                format!("/home/u/.local/bin:{THIS_ENV}/bin:/usr/bin")
            );
        }
    }

    #[test]
    fn interleaved_addition_counts_as_prepend() {
        // A new segment between two surviving old segments is replayed as
        // a prepend rather than an append.
        let translated = ctx().translate(
            "/old-a:/inserted:/old-b",
            Some("/old-a:/old-b"),
            Some("/current"),
        );
        assert_eq!(translated, "/inserted:/current");
    }

    #[test]
    fn appended_segments_stay_appended() {
        // LD_FLOXLIB_FILES_PATH-style: the start appended system libs after
        // the pre-existing value; the replay appends them to the attach
        // context's value instead of prepending.
        let translated = ctx().translate("/orig:/system/libs", Some("/orig"), Some("/other"));
        assert_eq!(translated, "/other:/system/libs");
    }

    #[test]
    fn created_var_default_suffix_stays_appended() {
        // DYLD_FALLBACK_LIBRARY_PATH-style: the start created the variable
        // as "<env libs>:<default fallback>". On attach the default
        // fallback run stays behind the attach context's own entries.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/lib:/usr/local/lib:/usr/lib"),
            None,
            Some("/opt/foo/lib"),
        );
        assert_eq!(
            translated,
            format!("{THIS_ENV}/lib:/opt/foo/lib:/usr/local/lib:/usr/lib")
        );
    }

    #[test]
    fn trailing_empty_segment_preserved_when_var_unset() {
        // INFOPATH-style: a recorded trailing ':' (empty segment) keeps
        // info(1)'s default search path when the attach context has no
        // value of its own.
        let translated = ctx().translate(&format!("{THIS_ENV}/share/info:"), None, None);
        assert_eq!(translated, format!("{THIS_ENV}/share/info:"));
    }

    #[test]
    fn trailing_marker_preserved_when_start_value_also_had_it() {
        // The start context's INFOPATH already ended in ':'; the replayed
        // value must keep the marker even though the empty segment also
        // appears in the start value.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/share/info:{START_PEER}/share/info:"),
            Some(&format!("{START_PEER}/share/info:")),
            None,
        );
        assert_eq!(translated, format!("{THIS_ENV}/share/info:"));
    }

    #[test]
    fn leading_marker_preserved_for_empty_start_value() {
        // A start value that is the empty string has no old segments; a
        // recorded leading ':' (MANPATH default-path marker) survives.
        let translated = ctx().translate(":/x", Some(""), None);
        assert_eq!(translated, ":/x");
    }

    #[test]
    fn empty_segments_not_duplicated_into_existing_value() {
        // When the attach context has its own value, recorded empty
        // segments are dropped rather than spliced into it.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/share/info:"),
            None,
            Some(&format!("{ATTACH_PEER}/share/info:")),
        );
        assert_eq!(
            translated,
            format!("{THIS_ENV}/share/info:{ATTACH_PEER}/share/info:")
        );
    }

    #[test]
    fn segments_already_present_are_not_duplicated() {
        let translated = ctx().translate(
            &format!("{THIS_ENV}/share:/shared"),
            None,
            Some(&format!("/shared:{ATTACH_PEER}/share")),
        );
        assert_eq!(
            translated,
            format!("{THIS_ENV}/share:/shared:{ATTACH_PEER}/share")
        );
    }

    #[test]
    fn unchanged_when_nothing_new_survives() {
        // Everything the start added is stale or already present: the
        // attach context's value is returned untouched.
        let translated = ctx().translate(
            &format!("{START_PEER}/include"),
            None,
            Some(&format!("{ATTACH_PEER}/include")),
        );
        assert_eq!(translated, format!("{ATTACH_PEER}/include"));
    }

    #[test]
    fn default_suffix_dedups_against_current_value() {
        // DYLD_FALLBACK_LIBRARY_PATH-style: the recorded default suffix is
        // already present in the attach context's value and is not
        // duplicated.
        let translated = ctx().translate(
            &format!("{THIS_ENV}/lib:{START_PEER}/lib:/usr/local/lib:/usr/lib"),
            None,
            Some(&format!("{ATTACH_PEER}/lib:/usr/local/lib:/usr/lib")),
        );
        assert_eq!(
            translated,
            format!("{THIS_ENV}/lib:{ATTACH_PEER}/lib:/usr/local/lib:/usr/lib")
        );
    }
}
