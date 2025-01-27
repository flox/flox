use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;

use anyhow::{anyhow, bail, Context, Result};
use daggy::petgraph::visit::IntoNodeIdentifiers;
use daggy::{NodeIndex, Walker};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::{ManifestBuildDescriptor, ManifestBuildSandbox};
use flox_rust_sdk::providers::build::{BuildResult, BuildResults};
use flox_rust_sdk::providers::buildenv::{BuildEnvOutputs, BuiltStorePath};
use flox_rust_sdk::providers::git::GitProvider;
use flox_rust_sdk::providers::{git, nix};
use flox_rust_sdk::utils::CommandExt;
use indexmap::IndexMap;
use itertools::Itertools;
use regex::Regex;
use serde::{Deserialize, Serialize};

static DEPENDENCY_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\$\{(?<dependency_id>.+)\})"#).expect("static valid regex"));

/// A node in the build graph.
#[derive(Debug)]
struct BuildNode<'lf> {
    /// The package name, i.e. the key in the build section of the manifest.
    pname: &'lf str,
    /// The build descriptor from the manifest.
    build_descriptor: &'lf ManifestBuildDescriptor,
    /// The rendered build wrapper environment store path for this build.
    /// This environment is used to wrap the contents of bin, sbin.
    environment_build_wrapper: &'lf BuiltStorePath,
}

/// An unchecked representation of the build graph.
/// Esentially extracting the `build.*` attributes
/// from the manifest in the Lockfile,
/// and recording the dependencies between them.
/// The dependencies are validated and checked for cycles
/// via [BuildNodesGraph::try_from].
struct BuildNodesGraphUnchecked<'lf> {
    nodes: HashMap<&'lf str, BuildNode<'lf>>,
    dependencies: HashMap<&'lf str, Vec<&'lf str>>,
}

/// A validated representation of the build graph.
/// This is a directed acyclic graph (DAG) of [BuildNode]s,
/// with edges representing dependencies between them.
/// The graph is validated for cycles and unknown dependencies
/// during construction via [BuildNodesGraph::try_from].
#[derive(Debug, derive_more::Deref)]
struct BuildNodesGraph<'lf> {
    /// The actual graph representation.
    #[deref]
    graph: daggy::Dag<BuildNode<'lf>, ()>,
    /// A mapping from package names to their respective node indices in the graph.
    names_to_indices: HashMap<&'lf str, daggy::NodeIndex>,
}

impl BuildNodesGraph<'_> {
    fn node_index_for_package(&self, package: &str) -> Option<NodeIndex> {
        self.names_to_indices.get(package).copied()
    }
}

/// The result of a `nix build --json` command.
/// We only care about the `outputs` field currently,
/// and will move that into a `BuildResult`.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct NixBuildResult {
    pub outputs: IndexMap<String, BuiltStorePath>,
    // and some more fields that we don't yet use
}

impl BuildNodesGraphUnchecked<'_> {
    /// Create a BuildNodesGraphUnchecked from a Lockfile.
    ///
    /// This constructs [BuildNode]s from the build section the manifest in the Lockfile,
    /// and tracks the dependencies between them.
    /// We call this "unchecked" because
    /// 1) we don't check that the dependencies are valid yet
    /// 2) we don't validate that there are no cycles in the graph
    ///
    /// In a way this is a "raw" representation of the build graph,
    /// that we use to construct a valdidated [BuildNodesGraph] from,
    /// using a DAG library for validation and access ([daggy] in this case).
    pub fn from_lockfile<'lf>(
        lockfile: &'lf Lockfile,
        built_lockfile: &'lf BuildEnvOutputs,
    ) -> Result<BuildNodesGraphUnchecked<'lf>> {
        let mut nodes = HashMap::new();
        let mut dependencies = HashMap::new();

        for (pname, build_descriptor) in lockfile.manifest.build.iter() {
            let mut build_deps = Vec::new();

            // extract dependencies from the build command using a regex
            // essentially looking for `${<dependency_id>}` in the command string.
            // Note, this _may_ find false positives, i.e. legitimate bash variable expansion.
            // Since we are overloading the syntax, we need to be careful with this.
            // Currently, unknown dependencies will be ignored when constructing the graph,
            // and later while preparing the build command.
            for dep in DEPENDENCY_REGEX.captures_iter(&build_descriptor.command) {
                let dep_id = dep
                    .name("dependency_id")
                    .expect("non-optional capture should exist")
                    .as_str();

                build_deps.push(dep_id);
            }

            let environment_build_wrapper = built_lockfile
                .manifest_build_runtimes
                .get(&format!("build-{pname}"))
                .with_context(|| {
                    format!("did not find a build wrapper environment for '{pname}'")
                })?;

            let build_node = BuildNode {
                environment_build_wrapper,
                build_descriptor,
                pname,
            };

            nodes.insert(pname.as_str(), build_node);
            dependencies.insert(pname.as_str(), build_deps);
        }

        Ok(BuildNodesGraphUnchecked {
            nodes,
            dependencies,
        })
    }
}
impl<'a> TryFrom<BuildNodesGraphUnchecked<'a>> for BuildNodesGraph<'a> {
    type Error = anyhow::Error;

    fn try_from(unchecked: BuildNodesGraphUnchecked<'a>) -> Result<Self, Self::Error> {
        let mut graph = daggy::Dag::new();

        let mut node_indices = HashMap::new();

        for (package, node) in unchecked.nodes {
            let node_index = graph.add_node(node);
            node_indices.insert(package, node_index);
        }

        for (package, deps) in unchecked.dependencies {
            let node_index = node_indices
                .get(package)
                .expect("indices created from nodes in the manifest");

            for dep in deps {
                // dependencies may refer to legitimate bash variables, so we ignore unknown dependencies
                if let Some(dep_index) = node_indices.get(dep) {
                    graph.add_edge(*node_index, *dep_index, ())?;
                } else {
                    // todo: in the make version this is silent!
                    eprintln!("warning: unknown dependency '{}'", dep);
                }
            }
        }

        Ok(BuildNodesGraph {
            names_to_indices: node_indices,
            graph,
        })
    }
}

fn postorder_build_traverse<'graph>(
    node_index: NodeIndex,
    environment_build_develop: &BuiltStorePath,
    session_build_cache: &mut HashMap<&'graph str, BuildResult>,
    graph: &'graph BuildNodesGraph,
    cache_dir: &Path,
) -> Result<BuildResult> {
    // recursively traverse dependencies first,
    // so when calling `build` we have all dependencies built already.
    let dependencies = graph.children(node_index).iter(graph);
    for (_, dependenct_node) in dependencies {
        // SAFETY: we checked the dependencies at creation of the BuildNodesGraph
        let build_node = graph
            .graph
            .node_weight(dependenct_node)
            .expect("dependencies checked at graph creation");

        if session_build_cache.contains_key(&build_node.pname) {
            continue;
        }
        let result = postorder_build_traverse(
            dependenct_node,
            environment_build_develop,
            session_build_cache,
            graph,
            cache_dir,
        )
        .with_context(|| format!("failed processing dependency '{}'", build_node.pname))?;
        session_build_cache.insert(build_node.pname, result);
    }

    let node = graph
        .graph
        .node_weight(node_index)
        .expect("caller provided node does not exist");
    let result = build(
        node,
        environment_build_develop,
        session_build_cache,
        cache_dir,
    )?;

    Ok(result)
}

fn build(
    node: &BuildNode,
    environment_build_develop: &BuiltStorePath,
    session_build_cache: &HashMap<&str, BuildResult>,
    cache_dir: &Path,
) -> Result<BuildResult> {
    let version = node
        .build_descriptor
        .version
        .clone()
        .unwrap_or("unknown".to_string());

    // region: replace references to dependencies in the build command
    // replace references to dependencies in the build command
    // with the actual output path of the dependency
    // and record which dependencies were substituted
    // (to explicitly allow them into the sandbox in pure mode)
    struct DependencySwapper<'a> {
        session_build_cache: &'a HashMap<&'a str, BuildResult>,
        substituted_dependencies: HashSet<String>,
    }
    impl regex::Replacer for &mut DependencySwapper<'_> {
        fn replace_append(&mut self, captures: &regex::Captures, dst: &mut String) {
            let dep_id = captures
                .name("dependency_id")
                .expect("unconditional capture group")
                .as_str();
            if let Some(dep_result) = self.session_build_cache.get(dep_id) {
                let dep_out_path = dep_result
                    .outputs
                    .get("out")
                    .expect("manifest builds have an 'out' output")
                    .to_string_lossy();

                dst.push_str(&dep_out_path);
                self.substituted_dependencies
                    .insert(dep_out_path.to_string());
            } else {
                // build cache does not contain the dependency
                // bcause we run `build` in a topological order of the graph,
                // dep_id is not a known flox-build'able dependency.
                // We assume then that it is a regular bash variable expansion and pass it through.
                dst.push_str(&captures[0]);
            }
        }
    }

    let mut dependency_substituter = DependencySwapper {
        session_build_cache,
        substituted_dependencies: HashSet::new(),
    };

    let command = DEPENDENCY_REGEX
        .replace_all(&node.build_descriptor.command, &mut dependency_substituter)
        .to_string();

    // endregion

    let build_mode = node
        .build_descriptor
        .sandbox
        .unwrap_or(ManifestBuildSandbox::Off);

    // Note: this is a diviation from the make version,
    // `flox-build.mk` uses the activation command provided by FLOX_INTERPRETER,
    // which is set by the flox cli, and usually refers to the activate script bundled with flox.
    // OTOH we also use the _environment_ bundled activation script
    // from the build wrapper environment.
    let flox_interpreter = environment_build_develop.join("activate");

    let builds_dir = cache_dir.join("builds");
    if !builds_dir.exists() {
        std::fs::create_dir_all(&builds_dir)?;
    }
    let builds_dir = builds_dir.canonicalize().expect("builds dir should exist");

    // write build script
    let build_script = builds_dir.join(format!("{}-build-script.sh", node.pname));
    std::fs::write(&build_script, &command)?;

    let build_result = if build_mode == ManifestBuildSandbox::Off {
        // local / non-sandboxed build
        impure_build(
            node,
            environment_build_develop,
            cache_dir,
            &version,
            flox_interpreter,
            &builds_dir,
            &build_script,
        )?
    } else {
        // sandboxed build
        pure_build(
            node,
            environment_build_develop,
            &version,
            &builds_dir,
            &build_script,
            dependency_substituter.substituted_dependencies,
        )?
    };

    Ok(build_result)
}

fn impure_build(
    node: &BuildNode<'_>,
    environment_build_develop: &BuiltStorePath,
    cache_dir: &Path,
    version: &String,
    flox_interpreter: PathBuf,
    builds_dir: &Path,
    build_script: &PathBuf,
) -> Result<BuildResult> {
    let install_prefix = builds_dir.join(format!("{}-install-prefix", node.pname));
    // in the make version this is a tempfile
    let log_file = builds_dir.join(format!("{}-build.log", node.pname));

    // remove previous version of out dir if it exists
    if install_prefix.exists() {
        std::fs::remove_dir_all(&install_prefix)?;
    }

    // Run build the build command inside a flox environment
    // The following command is broken up into four sections:
    // 1. Activate the development environment,
    //    to provide all the necessary tools for the build (call it "build dependencies")
    // 2. Clear the environment, keeping only PATH, HOME,
    //    and other essential compiler variables that may be set by the outer activation.
    // 3. activate the build wrapper environment, which provides _runtime_ dependencies,
    //    for the resulting package.
    //    This is meant to verify later that the package only depends on its declared runtime,
    //    i.e. a runtime that itself only depends on a single catalog page for example.
    // 4. Run the build command
    //    Inside the the two layered environments, now execute the build command.
    //    Wrap the build command in a t3 invocation
    //    to add timestamps and write logs to a logfile.
    //
    // in short, cmd describes:
    //
    //    activation dev ( clear_env ( activation build_wrapper ( t3 ( build_command ) ) ) )

    let mut cmd = std::process::Command::new(flox_interpreter);

    // region: section 1: activate thw development environment
    // activate the environment in dev mode

    //todo: pass as arg or env
    cmd.env("FLOX_RUNTIME_DIR", cache_dir.join("runtime"));
    std::fs::create_dir_all(cache_dir.join("runtime")).unwrap();

    // activate the environment in dev mode
    cmd.arg("--env").arg(environment_build_develop.as_path());
    cmd.arg("--mode").arg("dev");
    cmd.arg("--turbo");
    cmd.arg("--");

    // endregion

    // region: section 2: env setup prior to the inner activation
    // clear the environment
    // and inherit some essential variables
    cmd.arg("env").arg("-i"); // todo use store path of env command (from coreutils i assume)
    cmd.arg(format!(
        "out={out_dir}",
        out_dir = install_prefix.to_string_lossy()
    ));

    // this is a bit clunky, but these are what flox-build.mk passes through
    let variables_to_pass_through_by_name: &[&str] = &["HOME", "PATH"];
    for var in variables_to_pass_through_by_name {
        if let Ok(value) = std::env::var(var) {
            cmd.arg(format!("{}={}", var, value));
        }
    }
    let variables_to_pass_through_by_prefix = std::env::vars()
        .filter(|(key, _)| key.starts_with("NIX_CFLAGS") || key.starts_with("NIX_CC"))
        .collect::<Vec<_>>();
    for (key, value) in variables_to_pass_through_by_prefix {
        cmd.arg(format!("{}={}", key, value));
    }
    // todo: should be passed in as an arg or communicated in some other way
    // for now just fake a runtime dir which is mainly used to store activation metadata
    cmd.arg(format!(
        "FLOX_RUNTIME_DIR={}",
        cache_dir.join("runtime").to_string_lossy()
    ));
    // endregion

    // region: section 3: activate the build wrapper environment
    // run the build command in an activation of the build wrapper environment
    let runtime_environment = &node.environment_build_wrapper;
    cmd.arg(runtime_environment.join("activate"));
    cmd.arg("--env").arg(runtime_environment.as_path());
    cmd.arg("--mode").arg("dev");
    cmd.arg("--turbo");
    cmd.arg("--");

    // endregion

    // region: section 4: run the build command
    // run t3
    // this uses t3 to add timestamps and write logs to a logfile
    // this is similar to the wiretap utility in flox,
    // but to stay closer to the make version and simlicity of the PoC,
    // we keep using t3 here for
    cmd.arg("t3"); // todo: use store path
    cmd.arg(&log_file);
    cmd.arg("--");

    // run the build command, actually
    cmd.arg("bash");
    cmd.arg("-e").arg(build_script);

    // endregion

    // execute the whole command
    eprintln!("running build command");
    let mut child = cmd.spawn()?;
    let status = child.wait()?; // run and inherit std{out,err}
    if !status.success() {
        bail!(
            "'{pname}' failed to build, see '{log_file}' for build logs",
            pname = node.pname,
            log_file = log_file.to_string_lossy()
        );
    }

    // region: add the logfile to the store
    // Note the implementation of this differs a bit from the make version,
    // by combining the `nix store add-file` and `nix build` steps
    let mut cmd = nix::nix_base_command();
    cmd.arg("build").arg("--impure").arg("--expr").arg(format!(
        r#"builtins.path {{ path = "{}"; }}"#,
        log_file.to_string_lossy()
    ));
    cmd.arg("--json");
    cmd.arg("--out-link")
        .arg(format!("result-{}-log", node.pname));

    cmd.stderr(Stdio::inherit()); // todo: tap?

    eprintln!("adding log file to nix store: {}", cmd.display());

    let output = cmd.output()?; // run and inherit std{out,err}
    if !output.status.success() {
        bail!("failed to add log file to the nix store")
    }

    let [log_file_store_path] = serde_json::from_slice(&output.stdout)?;
    // endregion

    // region: ingest build result into store, making it a package
    // `build-manifest.nix` also checks and wraps binaries in $out,
    // replaces paths that reference `install_prefix` with `$out`
    let manifest_build_nix = Path::new(env!("FLOX_PACKAGE_BUILDER"))
        .join("libexec")
        .join("build-manifest.nix");
    let mut cmd = nix::nix_base_command();
    cmd.arg("build")
        .arg("-L")
        .arg("--file")
        .arg(manifest_build_nix);
    cmd.arg("--argstr").arg("pname").arg(node.pname);
    cmd.arg("--argstr").arg("version").arg(version);
    cmd.arg("--argstr")
        .arg("flox-env")
        .arg(environment_build_develop.as_path());
    cmd.arg("--argstr")
        .arg("build-wrapper-env")
        .arg(runtime_environment.as_path());
    cmd.arg("--argstr")
        .arg("install-prefix")
        .arg(install_prefix);
    cmd.arg("--argstr")
        .arg("nixpkgs-url")
        .arg(env!("COMMON_NIXPKGS_URL"));
    cmd.arg("--out-link").arg(format!("result-{}", node.pname));
    cmd.arg("--json");
    cmd.arg("^*");
    cmd.stderr(Stdio::inherit()); // todo: tap?

    println!("running command: {}", cmd.display());
    let output = cmd.output()?;
    if !output.status.success() {
        bail!(
            "failed to create package from the build results of '{pname}'",
            pname = node.pname
        );
    }
    let [nix_result]: [NixBuildResult; 1] = serde_json::from_slice(&output.stdout)?;
    // endregion

    let build_result = BuildResult {
        outputs: nix_result.outputs,
        version: version.clone(),
        log: log_file_store_path,
        pname: node.pname.to_string(),
    };

    Ok(build_result)
}

/// sandboxed build
fn pure_build(
    node: &BuildNode<'_>,
    environment_build_develop: &BuiltStorePath,
    version: &String,
    builds_dir: &PathBuf,
    build_script: &PathBuf,
    substituted_dependencies: HashSet<String>,
) -> Result<BuildResult> {
    // load src into store
    // into a tarball, we might want to use nix with `builtins.fetchGit`
    // or some of the prefetch commands here eventually,
    // but for now just follow the path of `flox-buld.mk` and pack our own tarball.

    let git = git::GitCommandProvider::discover("./").context("failed to open git repository")?;
    let ls_files_output = git.new_command().arg("ls-files").output()?;
    if !ls_files_output.status.success() {
        bail!(
            "failed to list files in git repository: {}",
            String::from_utf8_lossy(&ls_files_output.stderr)
        );
    }
    let files = String::from_utf8_lossy(&ls_files_output.stdout);

    let source_tar_path = builds_dir.join(format!("{}-source.tar", node.pname));
    {
        let source_tar_file = std::fs::File::create(&source_tar_path)?;
        let mut tar_builder = tar::Builder::new(source_tar_file);
        for file in files.lines() {
            tar_builder.append_path_with_name(git.path().join(file), file)?;
        }
        tar_builder.finish()?;
    }

    // region: check if a buildCache exists, or create one if not
    let build_cache_link_path = std::env::current_dir()
        .unwrap()
        .join(format!("result-{pname}-buildCache", pname = node.pname));

    // If a previous buildCache output exists, then copy the file.
    // Don't link or copy the symlink to the previous buildCache,
    // because we want nix to import it as a content-addressed input
    // rather than an ever-changing series of storePaths.
    // If it does not exist, then create a new tarball
    // containing only a single file indicating the time that
    // the buildCache was created to differentiate it from other
    // prior otherwise-empty buildCaches.
    // todo: check if buildCache feture is enabled at all
    let original_build_cache_store_path = match std::fs::read_link(&build_cache_link_path) {
        Ok(path) => {
            std::fs::remove_file(builds_dir.join(format!("{}-buildCache.tar", node.pname)))?;
            // copy cache to a stable path
            std::fs::copy(
                &build_cache_link_path,
                builds_dir.join(format!("{}-buildCache.tar", node.pname)),
            )?;
            Some(path)
        },
        Err(_) => {
            // create an initial cache
            // return None, because we don't have a previous cache
            let mut cache_tarball_init_file = tempfile::NamedTempFile::new_in(builds_dir)?;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            cache_tarball_init_file.write_all(format!("{timestamp}").as_bytes())?;

            let new_cache_tarball_path = builds_dir.join(format!("{}-buildCache.tar", node.pname));
            let new_cache_tarball_file = File::create(new_cache_tarball_path)?;
            let mut tar_builder = tar::Builder::new(new_cache_tarball_file);

            tar_builder
                .append_path_with_name(cache_tarball_init_file.path(), ".buildCache.init")?;

            tar_builder.finish()?;

            None
        },
    };
    // endregion

    // region: nix build that thang

    let mut cmd = nix::nix_base_command();
    cmd.arg("build");
    cmd.arg("-L");
    cmd.arg("--file").arg(
        Path::new(env!("FLOX_PACKAGE_BUILDER"))
            .join("libexec")
            .join("build-manifest.nix"),
    );
    cmd.arg("--argstr").arg("pname").arg(node.pname);
    cmd.arg("--argstr").arg("version").arg(version);
    cmd.arg("--argstr").arg("srcTarball").arg(&source_tar_path);
    cmd.arg("--argstr")
        .arg("flox-env")
        .arg(environment_build_develop.as_path());
    cmd.arg("--argstr")
        .arg("build-wrapper-env")
        .arg(node.environment_build_wrapper.as_path());
    cmd.arg("--argstr")
        .arg("nixpkgs-url")
        .arg(env!("COMMON_NIXPKGS_URL"));

    cmd.arg("--argstr")
        .arg("buildCache")
        .arg(builds_dir.join(format!("{}-buildCache.tar", node.pname)));

    cmd.arg("--argstr").arg("buildScript").arg(build_script);

    // Format a nix argument containing a list of dependency store paths
    // These paths will be imported explicitly by the nix expression
    // using `builtins.storePath`, which is necessary for nix to include them in the sandbox,
    // especially on linux where the sandbox is more strict.
    let nix_deps_arg = format!(
        r#"[ {} ]"#,
        substituted_dependencies
            .iter()
            .map(|s| format!("\"{s}\""))
            .join(" ")
    );
    cmd.arg("--arg").arg("buildDeps").arg(nix_deps_arg); // [sic] pass as --arg, not --argstr
    cmd.arg("--out-link").arg(format!("result-{}", node.pname));
    cmd.arg("--json");
    cmd.arg("^*");

    cmd.stderr(Stdio::inherit()); // todo: tap?

    println!("running command: {}", cmd.display());
    let output = cmd.output()?;
    if !output.status.success() {
        bail!(
            "failed to perform pure build of '{pname}'",
            pname = node.pname
        );
    }

    // endregion

    let [nix_result]: [NixBuildResult; 1] = serde_json::from_slice(&output.stdout)?;

    let build_cache_output_path = nix_result.outputs.get("buildCache").cloned();

    // cleanup old buildCache file from store
    'build_cache_cleanup: {
        // if we didn't have a buildCache before, we don't need to clean up anything
        // as there is only the freshly created cache.
        let Some(original) = original_build_cache_store_path.as_ref() else {
            break 'build_cache_cleanup;
        };

        // if the `buildCache` output path is the same as the original,
        // or we didn't create a `buildCache` output,
        // we don't need to clean up anything
        if let Some(new) = build_cache_output_path {
            // value of `buildCache` hasn't changed
            if *original == *new {
                break 'build_cache_cleanup;
            }
        } else {
            // didn't create a buildCache output ?!
            break 'build_cache_cleanup;
        }

        // remove old buildCache
        let mut command = nix::nix_base_command();
        command.arg("store").arg("delete").arg(original);
        command.stderr(Stdio::null());

        eprintln!("deleting old buildCache from store: {}", command.display());
        command.spawn()?.wait()?;
    }

    let log_output_path = nix_result
        .outputs
        .get("log")
        .expect("sandboxed manifest builds have a 'log' output")
        .to_owned();

    // Take this opportunity to fail the build if we spot fatal errors in the
    // build output. Recall that we force the Nix build to "succeed" in all
    // cases so that we can persist the buildCache, so when errors do happen
    // this is communicated by way of a $out that is 1) a file and 2) contains
    // the string "flox build failed (caching build dir)".
    let build_output_path = nix_result
        .outputs
        .get("out")
        .expect("sandboxed manifest builds have an 'out' output")
        .to_owned();

    if build_output_path.is_file() {
        let build_output = std::fs::read_to_string(&*build_output_path)?;
        if build_output.contains("flox build failed (caching build dir)") {
            bail!(
                "build failed with fatal error, see '{log_file}' for build logs",
                log_file = log_output_path.to_string_lossy()
            );
        }
    }

    let build_result = BuildResult {
        outputs: nix_result.outputs,
        version: version.clone(),
        log: log_output_path,
        pname: node.pname.to_string(),
    };

    Ok(build_result)
}

pub fn build_all(
    packages: Vec<String>,
    lockfile: &Lockfile,
    built_lockfile: &BuildEnvOutputs,
    cache_dir: &Path,
) -> Result<BuildResults> {
    let unchecked_graph = BuildNodesGraphUnchecked::from_lockfile(lockfile, built_lockfile)?;
    let graph = BuildNodesGraph::try_from(unchecked_graph)?;

    // if packages is empty, build all packages
    // otherwise, look up the node index for each package
    let to_build = if packages.is_empty() {
        graph.node_identifiers().collect::<Vec<_>>()
    } else {
        packages
            .iter()
            .map(|package| {
                graph
                    .node_index_for_package(package)
                    .ok_or_else(|| anyhow!("Package '{}' not found in environment", package))
            })
            .collect::<Result<Vec<_>>>()?
    };

    let mut session_build_cache = HashMap::new();
    let mut results = Vec::new();
    for node_index in to_build {
        let result = postorder_build_traverse(
            node_index,
            &built_lockfile.develop,
            &mut session_build_cache,
            &graph,
            cache_dir,
        )
        .with_context(|| {
            format!(
                "failed to build '{}'",
                graph.node_weight(node_index).unwrap().pname
            )
        })?;
        results.push(result);
    }

    Ok(results.into())
}

pub fn clean_all(packages: Vec<String>, lockfile: &Lockfile, cache_dir: &Path) -> Result<()> {
    let packages = if packages.is_empty() {
        lockfile.manifest.build.keys().cloned().collect()
    } else {
        packages
    };

    for package in packages {
        let build_dir = cache_dir
            .join("builds")
            .join(format!("{}-install-prefix", package));
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }

        // delete the buildCache, result, and log file  from the store
        let base_path = std::env::current_dir().unwrap();
        fn remove_link(path: &Path) -> Result<()> {
            if path.exists() {
                let store_path = std::fs::read_link(path)?;
                std::fs::remove_file(path)?;
                let mut cmd = nix::nix_base_command();
                cmd.arg("store").arg("delete").arg(store_path);

                // Nix will print errors if these paths are still referenced somewhere
                // we dont want to startle users with errors, so we suppress stderr.
                // This is the same as in the make version.
                cmd.stderr(Stdio::null());
                cmd.spawn()?.wait()?;
            }
            Ok(())
        }

        remove_link(&base_path.join(format!("result-{}", package)))?;
        remove_link(&base_path.join(format!("result-{}-log", package)))?;
        remove_link(&base_path.join(format!("result-{}-buildCache", package)))?;

        let build_script_path = cache_dir
            .join("builds")
            .join(format!("{}-build-script.sh", package));
        if build_script_path.exists() {
            std::fs::remove_file(&build_script_path)?;
        }

        let source_tar_path = cache_dir
            .join("builds")
            .join(format!("{}-source.tar", package));
        if source_tar_path.exists() {
            std::fs::remove_file(&source_tar_path)?;
        }

        let build_cache_path = cache_dir
            .join("builds")
            .join(format!("{}-buildCache.tar", package));
        if build_cache_path.exists() {
            std::fs::remove_file(&build_cache_path)?;
        }
    }
    Ok(())
}
