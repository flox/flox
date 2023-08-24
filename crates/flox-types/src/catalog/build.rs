use serde::{Deserialize, Serialize};

/// Proof that a package has been successfully built.
///
/// `Build` also includes metadata that can only be determined after a
/// successful build.
/// After a successful build, the contents of an output path are known, whereas
/// after an eval, only the output path is known.
#[derive(Clone, Serialize, Deserialize)]
pub struct Build(Vec<BuildMeta>);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct BuildMeta {
    pub has_bin: bool,
    pub has_man: bool,
    // at some point maybe add size, time to build, content addressed paths
}
