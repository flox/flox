use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;

use derive_more::{Deref, DerefMut, Display};
use flox_types::catalog::cache::{CacheMeta, SubstituterUrl};
use flox_types::stability::Stability;
use futures::TryFutureExt;
use log::info;
use runix::arguments::flake::FlakeArgs;
use runix::command::Eval;
use runix::command_line::{NixCommandLine, NixCommandLineRunJsonError};
use runix::flake_metadata::FlakeMetadata;
use runix::flake_ref::git::{GitAttributes, GitRef};
use runix::flake_ref::git_service::{service, GitServiceRef};
use runix::flake_ref::indirect::IndirectRef;
use runix::flake_ref::path::PathRef;
use runix::flake_ref::protocol::{WrappedUrl, WrappedUrlParseError};
use runix::flake_ref::{protocol, FlakeRef};
use runix::installable::{AttrPath, FlakeAttribute};
use runix::{RunJson, RunTyped};
use serde_json::{json, Value};
use thiserror::Error;

use crate::flox::Flox;
use crate::providers::git::{GitCommandGetOriginError, GitCommandProvider as Git, GitProvider};

/// Publish state before analyzing
///
/// Prevents other actions to commence without analyzing the package first
pub struct Empty;

/// Publish state after collecting nix metadata
///
/// JSON value (ideally a [flox_types::catalog::CatalogEntry],
/// but that's currently broken on account of some flakerefs)
#[derive(Debug, Deref, DerefMut)]
pub struct NixAnalysis(Value);

/// State for the publish algorihm
///
/// The analysis field tracks the transition from Empty -> NixAnalysis to ensure we don't invoke invalid operations
pub struct Publish<'flox, State> {
    /// A shared flox session
    ///
    /// Nearly all commands require shared state from the [Flox] object.
    /// Save a reference to it to simplify the method signatures.
    flox: &'flox Flox,
    /// The published _upstream_ source
    publish_flake_ref: PublishFlakeRef,
    /// The attr_path of the published package in the source flake (`publish_flake_ref`)
    ///
    /// E.g. when publishing `git+https://github.com/flox/flox#packages.aarch64-darwin.flox`
    /// this is: `packages.aarch64-darwin.flox`
    ///
    /// Should be fully resolved to avoid ambiguity
    attr_path: AttrPath,
    stability: Stability,
    analysis: State,
}

impl<'flox> Publish<'flox, Empty> {
    /// Create a new [Publish] instance at first without any metadata
    pub fn new(
        flox: &'flox Flox,
        publish_flake_ref: PublishFlakeRef,
        attr_path: AttrPath,
        stability: Stability,
    ) -> Publish<'flox, Empty> {
        Self {
            flox,
            publish_flake_ref,
            attr_path,
            stability,
            analysis: Empty,
        }
    }

    /// Run analysis on the package and switch to next state.
    ///
    /// We evaluate package metadata as JSON, to which we add
    /// * source URLs for reproducibility
    /// * the nixpkgs stability being used to create the package
    pub async fn analyze(self) -> Result<Publish<'flox, NixAnalysis>, PublishError> {
        let mut drv_metadata_json = self.get_drv_metadata().await?;
        let flake_metadata = self.get_flake_metadata().await?;

        // DEVIATION FROM BASH: using `locked` here instead of `resolved`
        //                      this is used to reproduce the package,
        //                      but is essentially redundant because of the `source.locked`
        // TODO it would be better if we didn't have to do post processing of the analysis for parity with calls to readPackage in https://github.com/flox/floxpkgs/blob/master/modules/common.nix
        drv_metadata_json["element"]["url"] = json!(flake_metadata.locked.to_string());
        drv_metadata_json["source"] = json!({
            "locked": flake_metadata.locked,
            "original": flake_metadata.original,
            "remote": flake_metadata.original,
        });
        drv_metadata_json["eval"]["stability"] = json!(self.stability);

        Ok(Publish {
            flox: self.flox,
            publish_flake_ref: self.publish_flake_ref,
            attr_path: self.attr_path,
            stability: self.stability,
            analysis: NixAnalysis(drv_metadata_json),
        })
    }

    /// Extract metadata of the published derivation using the analyzer flake.
    ///
    /// It uses an analyzer flake to extract eval metadata of the derivation.
    /// The analyzer applies a function to all packages in a `target` flake
    /// and provides the result under `#analysis.eval.<full attrpath of the package>`.
    async fn get_drv_metadata(&self) -> Result<Value, PublishError> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

        // Create the analysis.eval.<full attrpath of the package> attr path
        // taking care to remove any leading `""` from the original attr_path
        // used to signal strict paths (a flox concept, to be upstreamed)
        let analysis_attr_path = {
            let mut attrpath = AttrPath::try_from(["", "analysis", "eval"]).unwrap();
            attrpath.extend(
                self.attr_path
                    .clone()
                    .into_iter()
                    .peekable()
                    .skip_while(|attr| attr.as_ref() == ""),
            );
            attrpath
        };

        let nixpkgs_flakeref = FlakeRef::Indirect(IndirectRef::new(
            format!("nixpkgs-{}", self.stability),
            Default::default(),
        ));

        // We bundle the analyzer flake with flox (see the package definition for flox)
        let analyzer_flakeref = FlakeRef::Path(PathRef::new(
            PathBuf::from(env!("FLOX_ANALYZER_SRC")),
            Default::default(),
        ));

        let eval_analysis_command = Eval {
            flake: FlakeArgs {
                override_inputs: [
                    // The analyzer flake provides analysis outputs for the flake input `target`
                    // Here, we're setting the target flake to our source flake.
                    (
                        "target".to_string(),
                        self.publish_flake_ref.clone().into_inner(),
                    )
                        .into(),
                    // Stabilities are managed by overriding the `flox-floxpkgs/nixpkgs/nixpkgs` input to
                    // `nixpkgs-<stability>`.
                    // The analyzer flake adds an additional indirection,
                    // so we have to do the override manually.
                    // This is the `nixpkgs-<stability>` portion.
                    (
                        "target/flox-floxpkgs/nixpkgs/nixpkgs".to_string(),
                        nixpkgs_flakeref,
                    )
                        .into(),
                ]
                .to_vec(),
                // The analyzer flake is bundled with flox as a nix store path and thus read-only.
                no_write_lock_file: true.into(),
            },
            eval_args: runix::arguments::EvalArgs {
                installable: Some(
                    FlakeAttribute {
                        flakeref: analyzer_flakeref,
                        attr_path: analysis_attr_path,
                    }
                    .into(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };

        eval_analysis_command
            .run_json(&nix, &Default::default())
            .map_err(|nix_error| {
                PublishError::DrvMetadata(
                    self.attr_path.clone(),
                    self.publish_flake_ref.clone(),
                    nix_error,
                )
            })
            .await
    }

    /// Resolve the metadata of the flake holding the published package
    async fn get_flake_metadata(&self) -> Result<FlakeMetadata, PublishError> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

        let locked_ref_command = runix::command::FlakeMetadata {
            flake_ref: Some(self.publish_flake_ref.clone().into_inner().into()),
            ..Default::default()
        };

        locked_ref_command
            .run_typed(&nix, &Default::default())
            .map_err(|nix_err| PublishError::FlakeMetadata(self.publish_flake_ref.clone(), nix_err))
            .await
    }
}

impl<'flox> Publish<'flox, NixAnalysis> {
    /// Copy the outputs and dependencies of the package to binary store
    pub async fn upload_binary(self) -> Result<Publish<'flox, NixAnalysis>, PublishError> {
        todo!()
    }

    /// Check whether a store path is substitutable by a given substituter
    /// and return the associated metadata.
    #[allow(unused)] // until implemented
    async fn get_binary_cache_metadata(
        &self,
        substituter: SubstituterUrl,
    ) -> Result<CacheMeta, PublishError> {
        todo!()
    }

    /// Write snapshot to catalog and push to origin
    pub async fn push_catalog(self) -> Result<(), PublishError> {
        let url = self.publish_flake_ref.clone_url();
        let repo_dir = tempfile::tempdir_in(&self.flox.temp_dir)
            .unwrap()
            .into_path(); // todo catch error
        let catalog = <Git as GitProvider>::clone(url, &repo_dir, false)
            .await
            .unwrap(); // todo: catch error

        if catalog.list_branches().await.unwrap() // todo: catch error
            .into_iter().any(|info| info.name == "catalog")
        {
            catalog.checkout("catalog", false).await.unwrap(); // todo: catch error
        } else {
            todo!();
            // catalog.checkout("catalog", true).await.unwrap(); // todo: catch error

            // catalog.set_upstream("origin", "catalog").await.unwrap();  // todo: implement
            //                                                               todo: catch error
        }
        todo!()
    }

    /// Read out the current publish state
    pub fn analysis(&self) -> &Value {
        self.analysis.deref()
    }
}

#[derive(Error, Debug)]
pub enum PublishError {
    #[error("Failed to load metadata for the package '{0}' in '{1}': {2}")]
    DrvMetadata(AttrPath, PublishFlakeRef, NixCommandLineRunJsonError),

    #[error("Failed to load metadata for flake '{0}': {1}")]
    FlakeMetadata(PublishFlakeRef, NixCommandLineRunJsonError),
}

/// Publishable FlakeRefs
///
/// `publish` modifies branches of the source repository.
/// Thus we can only publish to repositories in (remote*) git repositories.
/// This enum represents the subset of flakerefs we can use,
/// so we can avoid parsing and converting flakerefs within publish.
/// [GitRef<protocol::File>] should in most cases be resolved to a remote type.
///
/// \* A publish allows other users to substitute or build a package
/// (as long as repo name and references remain available).
/// If you publish a local repository, all urls will refer to local paths,
/// so a snapshot can't be reproduced anywhere but the local machine.
///
/// `flox publish git+file:///somewhere/local#package` would actually resolve git+file:///somewhere/local
/// to the upstream repo defined by the current branch's remote.
#[derive(PartialEq, Eq, Clone, Debug, Display)]
pub enum PublishFlakeRef {
    Ssh(GitRef<protocol::SSH>),
    Https(GitRef<protocol::HTTPS>),
    File(GitRef<protocol::File>),
}

impl PublishFlakeRef {
    /// Extract a URL for cloning with git
    fn clone_url(&self) -> String {
        match self {
            PublishFlakeRef::Ssh(ref ssh_ref) => ssh_ref.url.as_str().to_owned(),
            PublishFlakeRef::Https(ref https_ref) => https_ref.url.as_str().to_owned(),
            PublishFlakeRef::File(ref file_ref) => file_ref.url.as_str().to_owned(),
        }
    }

    /// Return the [FlakeRef] type for the wrapped refs
    fn into_inner(self) -> FlakeRef {
        match self {
            PublishFlakeRef::Ssh(ssh_ref) => FlakeRef::GitSsh(ssh_ref),
            PublishFlakeRef::Https(https_ref) => FlakeRef::GitHttps(https_ref),
            PublishFlakeRef::File(file_ref) => FlakeRef::GitPath(file_ref),
        }
    }

    /// Resolve `github:` flakerefs to a "proper" git url.
    ///
    /// Unlike nix operations publish is writing to the targeted git repository.
    /// To allow native git operations on the remote,
    /// the `github:` shorthand must be exanded to an `ssh` or `https` url.
    ///
    /// By default we resolve ssh urls,
    /// since (publishing) users will more likely have ssh configured,
    /// rather than (often insecure) token access.
    /// In cases where https token access is granted by another tool (e.g. gh),
    /// or within github actions we can prefer https URLs,
    /// so not to require an ssh setup to be present.
    #[allow(unused)]
    fn from_github_ref(
        GitServiceRef {
            owner,
            repo,
            attributes,
            ..
        }: GitServiceRef<service::Github>,
        prefer_https: bool,
    ) -> Result<Self, ConvertFlakeRefError> {
        let host = attributes.host.unwrap_or("github.com".to_string());

        let only_rev = attributes.rev.is_some() && attributes.reference.is_none();

        let git_attributes = GitAttributes {
            rev: attributes.rev,
            reference: attributes.reference,
            dir: attributes.dir,
            // Nix needs either a `?ref=` or `allRefs=1` set to fetch a revision
            all_refs: only_rev.then_some(true),
            ..Default::default()
        };

        let publish_flake_ref = if prefer_https {
            let url_str = format!("https://{host}/{owner}/{repo}");
            let url = WrappedUrl::from_str(&url_str)
                .map_err(|e| ConvertFlakeRefError::InvalidResultUrl(url_str, e))?;

            Self::Https(GitRef {
                url,
                attributes: git_attributes,
            })
        } else {
            let url_str = format!("ssh://git@{host}/{owner}/{repo}");
            let url = WrappedUrl::from_str(&url_str)
                .map_err(|e| ConvertFlakeRefError::InvalidResultUrl(url_str, e))?;

            Self::Ssh(GitRef {
                url,
                attributes: git_attributes,
            })
        };

        Ok(publish_flake_ref)
    }

    /// Resolve a git+file flake ref to a git+https reference
    ///
    /// Reproducing a snapshot from source requires access to the original repo.
    /// Including a local file reference in the snapshot,
    /// means its practically only possible to reproduce for the original creator.
    /// Thus, we resolve a local branch to it's upstream remote and branch.
    ///
    /// For local repositories we also check
    /// whether the repository contains any uncommitted changes
    /// and is in sync with its upstream branch.
    async fn from_git_file_flake_ref(
        file_ref: GitRef<protocol::File>,
        nix: &NixCommandLine,
    ) -> Result<Self, ConvertFlakeRefError> {
        // Get nix metadata for the referred flake
        // Successfully acquiring metadata proves the path
        // - is in fact a flake
        // - is a git repository
        // - the path is not dirty (no uncommitted changes)
        let local_metadata = {
            let command = runix::command::FlakeMetadata {
                flake_ref: Some(FlakeRef::GitPath(file_ref.clone()).into()),
                ..Default::default()
            };
            command
                .run_typed(nix, &Default::default())
                .await
                .map_err(ConvertFlakeRefError::LocalFlakeMetadata)?
        };

        if local_metadata.revision.is_none() {
            Err(ConvertFlakeRefError::LocalFlakeDirty)?;
        }

        // Create a handle to the git repo
        let repo = Git::discover(file_ref.url.path())
            .await
            .map_err(|_| ConvertFlakeRefError::RepoNotFound(file_ref.url.path().into()))?;

        // Get the upstream branch information.
        // This is essentialy
        //
        //   upstream_ref = git rev-parse @{u}
        //   (remote_name, branch_name) = split_once "/" upstream_ref
        //   upstream_url = git remote get-url ${remote_name}
        //   upstream_rev = git ls-remote ${remote_name} ${branch_name}
        //
        // The current branch MUST have an upstream ref configured
        // which resolves to a valid rev on the remote.
        let (remote_name, remote_url, remote_branch, remote_revision) = repo
            .get_origin()
            .await
            .map_err(ConvertFlakeRefError::NoRemote)?;

        // Ensure the remote branch exists
        let remote_revision = remote_revision.ok_or(ConvertFlakeRefError::RemoteBranchNotFound)?;

        info!("Resolved local flake to remote '{remote_name}:{remote_branch}' at '{remote_url}'");

        // Check whether the local branch is in sync with its upstream branch,
        // to ensure we publish the intended revision,
        // by comparing the local revision to the one found upstream.
        //
        // Dirty branches are already permitted due to the filter above.
        if let Some(local_rev) = local_metadata.revision {
            if local_rev.as_ref() != remote_revision {
                Err(ConvertFlakeRefError::RemoteBranchNotSync(
                    local_rev.to_string(),
                    remote_revision.clone(),
                ))?
            }
        }

        // Git supports special urls for e.g. ssh repos, e.g.
        //
        //   git@github.com:flox/flox
        //
        // Normalize these urls to proper URLs for the use with nix.
        let remote_url = git_url_parse::normalize_url(&remote_url)
            .map_err(|e| ConvertFlakeRefError::UnknownRemoteUrl(e.to_string()))?;

        // Copy the flakeref attributes but unlock it in support of preferred upstream refs
        let mut attributes = file_ref.attributes;
        attributes.last_modified = None;
        attributes.rev_count = None;
        attributes.nar_hash = None;
        attributes.rev = Some(remote_revision.parse().unwrap());
        attributes.reference = Some(remote_branch);

        let remote_flake_ref = match remote_url.scheme() {
            "ssh" => Self::Ssh(GitRef::new(
                WrappedUrl::try_from(remote_url).unwrap(),
                attributes,
            )),
            "https" => Self::Https(GitRef::new(
                WrappedUrl::try_from(remote_url).unwrap(),
                attributes,
            )),
            // Resolving to a local remote is an error case most of the time
            // but technically valid and required for testing.
            // `File` variants are filtered out at a higher level
            "file" => Self::File(GitRef::new(
                WrappedUrl::try_from(remote_url).unwrap(),
                attributes,
            )),
            _ => Err(ConvertFlakeRefError::UnsupportedGitUrl(remote_url))?,
        };
        Ok(remote_flake_ref)
    }

    pub async fn from_flake_ref(
        flake_ref: FlakeRef,
        flox: &Flox,
        git_service_prefer_https: bool,
    ) -> Result<Self, ConvertFlakeRefError> {
        let publish_flake_ref = match flake_ref {
            FlakeRef::GitSsh(ssh_ref) => Self::Ssh(ssh_ref),
            FlakeRef::GitHttps(https_ref) => Self::Https(https_ref),
            // resolve upstream for local git repo
            FlakeRef::GitPath(file_ref) => {
                Self::from_git_file_flake_ref(file_ref, &flox.nix(Default::default())).await?
            },
            // resolve indirect ref to direct ref (recursively)
            FlakeRef::Indirect(_) => todo!(),
            FlakeRef::Github(github_ref) => {
                Self::from_github_ref(github_ref, git_service_prefer_https)?
            },
            FlakeRef::Gitlab(_) => todo!(),
            _ => Err(ConvertFlakeRefError::UnsupportedTarget(flake_ref))?,
        };
        Ok(publish_flake_ref)
    }
}

/// Errors arising from convert
#[derive(Error, Debug)]
pub enum ConvertFlakeRefError {
    #[error("Unsupported flakeref for publish: {0}")]
    UnsupportedTarget(FlakeRef),

    #[error("Invalid URL after conversion: {0}: {1}")]
    InvalidResultUrl(String, WrappedUrlParseError),

    #[error("Couldn't find a local git repository in: {0}")]
    RepoNotFound(PathBuf),

    #[error("Couldn't find remote")]
    NoRemote(GitCommandGetOriginError),

    #[error("Couldn't get metadata for local flake")]
    LocalFlakeMetadata(NixCommandLineRunJsonError),

    #[error("Local flake contains uncommitted changes")]
    LocalFlakeDirty,

    #[error("Current branch in local flake does not have a remote configured")]
    RemoteBranchNotFound,

    #[error("Local repo out of sync with remote: local: {0}, remote: {1}")]
    RemoteBranchNotSync(String, String),

    #[error("Failed normalizing git url: {0}")]
    UnknownRemoteUrl(String),
    #[error("Unsupported git remote URL: {0}")]
    UnsupportedGitUrl(url::Url),
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flox::tests::flox_instance;
    use crate::prelude::Channel;

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn creates_catalog_entry() {
        env_logger::init();

        let (mut flox, _temp_dir_handle) = flox_instance();

        flox.channels.register_channel(
            "nixpkgs-stable",
            Channel::from("github:flox/nixpkgs/stable".parse::<FlakeRef>().unwrap()),
        );

        let flake_ref = "git+ssh://git@github.com/flox/flox"
            .parse::<FlakeRef>()
            .unwrap();

        let publish_flake_ref = PublishFlakeRef::from_flake_ref(flake_ref, &flox, false)
            .await
            .unwrap();

        let attr_path = ["", "packages", "aarch64-darwin", "flox"]
            .try_into()
            .unwrap();
        let stability = Stability::Stable;
        let publish = Publish::new(&flox, publish_flake_ref, attr_path, stability);

        let value = publish.analyze().await.unwrap().analysis().to_owned();

        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    }

    #[test]
    fn convert_github_ref() {
        // simple github references
        let flake_ref = GitServiceRef::<service::Github>::from_str("github:flox/flox").unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox"
        );

        // github references with explicit host param
        let flake_ref = GitServiceRef::<service::Github>::from_str(
            "github:flox/flox?host=github.myenterprise.com",
        )
        .unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.myenterprise.com/flox/flox"
        );

        // github references with dir param
        let flake_ref =
            GitServiceRef::<service::Github>::from_str("github:flox/flox?dir=somwhere/inside")
                .unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox?dir=somwhere%2Finside"
        );

        // github references with git ref
        let flake_ref =
            GitServiceRef::<service::Github>::from_str("github:flox/flox/feat/test").unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox?ref=feat%2Ftest"
        );

        // github references with git rev
        let flake_ref = GitServiceRef::<service::Github>::from_str(
            "github:flox/flox/49335c4bade5b3feb7378f9af8e9a528d9c4103e",
        )
        .unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox?allRefs=1&rev=49335c4bade5b3feb7378f9af8e9a528d9c4103e"
        );

        // simple github references
        let flake_ref = GitServiceRef::<service::Github>::from_str("github:flox/flox").unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, true).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+https://github.com/flox/flox"
        );
    }

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn git_file_error_if_dirty() {
        use std::fs;
        use std::path::Path;
        use std::str::FromStr;
        env_logger::init();

        let (flox, _temp_dir_handle) = flox_instance();
        let repo_dir = _temp_dir_handle.path().join("repo");

        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        fs::write(repo_dir.join("flake.nix"), "{ outputs = _: {}; }").unwrap();
        repo.add(&[Path::new(".")]).await.unwrap();

        let flake_ref =
            GitRef::from_str(&format!("git+file://{}", repo_dir.to_string_lossy())).unwrap();

        assert!(matches!(
            PublishFlakeRef::from_git_file_flake_ref(
                flake_ref.clone(),
                &flox.nix(Default::default())
            )
            .await,
            Err(ConvertFlakeRefError::LocalFlakeDirty)
        ));
    }

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn git_file_error_if_dirty2() {
        use std::fs;
        use std::path::Path;
        use std::str::FromStr;
        env_logger::init();

        let (flox, _temp_dir_handle) = flox_instance();
        let repo_dir = _temp_dir_handle.path().join("repo");

        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        fs::write(repo_dir.join("flake.nix"), "{ outputs = _: {}; }").unwrap();
        repo.add(&[Path::new(".")]).await.unwrap();
        repo.commit("Commit flake").await.unwrap();
        repo.add_remote("upstream", &repo_dir.to_string_lossy())
            .await
            .unwrap();
        repo.fetch().await.unwrap();
        repo.set_origin("master", "upstream").await.unwrap();

        let flake_ref =
            GitRef::from_str(&format!("git+file://{}", repo_dir.to_string_lossy())).unwrap();

        PublishFlakeRef::from_git_file_flake_ref(flake_ref.clone(), &flox.nix(Default::default()))
            .await
            .unwrap();
    }

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn git_file_error_if_no_upstream() {
        use std::fs;
        use std::path::Path;
        use std::str::FromStr;
        env_logger::init();

        let (flox, _temp_dir_handle) = flox_instance();
        let repo_dir = _temp_dir_handle.path().join("repo");

        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        fs::write(repo_dir.join("flake.nix"), "{ outputs = _: {}; }").unwrap();
        repo.add(&[Path::new(".")]).await.unwrap();
        repo.commit("Commit flake").await.unwrap();

        let flake_ref =
            GitRef::from_str(&format!("git+file://{}", repo_dir.to_string_lossy())).unwrap();

        let result = PublishFlakeRef::from_git_file_flake_ref(
            flake_ref.clone(),
            &flox.nix(Default::default()),
        )
        .await;

        assert!(matches!(result, Err(ConvertFlakeRefError::NoRemote(_))));
    }
}
