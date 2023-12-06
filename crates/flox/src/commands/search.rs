use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::global_manifest_path;
use flox_rust_sdk::models::search::{
    do_search,
    PathOrJson,
    Query,
    SearchParams,
    SearchResult,
    SearchResults,
    ShowError,
    Subtree,
};
use log::debug;

use crate::commands::environment::hacky_environment_description;
use crate::commands::{detect_environment, open_environment};
use crate::config::features::{Features, SearchStrategy};
use crate::subcommand_metric;

const SEARCH_INPUT_SEPARATOR: &'_ str = ":";
const DEFAULT_DESCRIPTION: &'_ str = "<no description provided>";
const DEFAULT_NUM_RESULTS: u8 = 10;

#[derive(Bpaf, Clone)]
pub struct ChannelArgs {}

/// Search for packages to install
#[derive(Bpaf, Clone)]
pub struct Search {
    /// print search as JSON
    #[bpaf(long)]
    pub json: bool,

    /// force update of catalogs from remote sources before searching
    #[bpaf(long)]
    pub refresh: bool,

    /// Print all search results
    #[bpaf(short, long)]
    pub all: bool,

    /// query string of the form `<REGEX>[@<SEMVER-RANGE>]` used to filter
    /// match against package names/descriptions, and semantic version.
    /// Regex pattern is `PCRE` style, and semver ranges use the
    /// `node-semver` syntax.
    /// Exs: `(hello|coreutils)`, `node@>=16`, `coreutils@9.1`
    #[bpaf(positional("search-term"))]
    pub search_term: String,
}

// Your first run will be slow, it's creating databases, but after that -
//   it's fast!
//
// `NIX_CONFIG='allow-import-from-derivation = true'` may be required because
// `pkgdb` disables this by default, but some flakes require it.
// Ideally this setting should be controlled by Registry preferences,
// which is TODO.
// Luckily most flakes don't.
impl Search {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("search");
        debug!("performing search for term: {}", self.search_term);

        let (manifest, lockfile) = manifest_and_lockfile(&flox, "search for packages using")
            .context("failed while looking for manifest and lockfile")?;

        let limit = if self.all {
            None
        } else {
            Some(DEFAULT_NUM_RESULTS)
        };

        let search_params = construct_search_params(
            &self.search_term,
            limit,
            manifest.map(|p| p.try_into()).transpose()?,
            global_manifest_path(&flox).try_into()?,
            lockfile.map(|p| p.try_into()).transpose()?,
        )?;

        let (results, exit_status) = do_search(&search_params)?;
        debug!("search call exit status: {}", exit_status.to_string());

        // Render what we have no matter what, then indicate whether we encountered an error.
        // FIXME: We may have warnings on `stderr` even with a successful call to `pkgdb`.
        //        We aren't checking that at all at the moment because better overall error handling
        //        is coming in a later PR.
        if self.json {
            debug!("printing search results as JSON");
            render_search_results_json(results)?;
        } else {
            debug!("printing search results as user facing");
            render_search_results_user_facing(&self.search_term, results)?;
        }
        if !exit_status.success() {
            bail!(
                "pkgdb exited with status code: {}",
                exit_status.code().unwrap_or(-1),
            );
        };

        Ok(())
    }
}

fn construct_search_params(
    search_term: &str,
    results_limit: Option<u8>,
    manifest: Option<PathOrJson>,
    global_manifest: PathOrJson,
    lockfile: Option<PathOrJson>,
) -> Result<SearchParams> {
    let query = Query::from_term_and_limit(
        search_term,
        Features::parse()?.search_strategy == SearchStrategy::MatchName,
        results_limit,
    )?;
    let params = SearchParams {
        manifest,
        global_manifest,
        lockfile,
        query,
    };
    debug!("search params raw: {:?}", params);
    Ok(params)
}

/// An intermediate representation of a search result used for rendering
#[derive(Debug, PartialEq, Clone)]
struct DisplayItem {
    /// The input that the package came from
    input: String,
    /// The displayable part of the package's attribute path
    package: String,
    /// The package description
    description: Option<String>,
    /// Whether to join the `input` and `package` fields with a separator when rendering
    render_with_input: bool,
}

fn render_search_results_user_facing(
    search_term: &str,
    search_results: SearchResults,
) -> Result<()> {
    // Nothing to display
    if search_results.results.is_empty() {
        bail!("No packages matched this search term: {}", search_term);
    }
    // Search results contain a lot of information, but all we need for rendering are
    // the input, the package subpath (e.g. "python310Packages.flask"), and the description.
    let display_items = search_results
        .results
        .into_iter()
        .map(|r| {
            Ok(DisplayItem {
                input: r.input,
                package: r.rel_path.join("."),
                description: r.description.map(|s| s.replace('\n', " ")),
                render_with_input: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let deduped_display_items = dedup_and_disambiguate_display_items(display_items);
    if deduped_display_items.is_empty() {
        bail!("deduplicating search results failed");
    }

    let column_width = deduped_display_items
        .iter()
        .map(|d| {
            if d.render_with_input {
                d.input.len() + d.package.len() + SEARCH_INPUT_SEPARATOR.len()
            } else {
                d.package.len()
            }
        })
        .max()
        .unwrap(); // SAFETY: could panic if `deduped_display_items` is empty, but we know it's not

    // Finally print something
    let mut writer = BufWriter::new(std::io::stdout());
    let default_desc = String::from(DEFAULT_DESCRIPTION);
    for d in deduped_display_items.into_iter() {
        let package = if d.render_with_input {
            [d.input, d.package].join(SEARCH_INPUT_SEPARATOR)
        } else {
            d.package
        };
        let desc: String = d.description.unwrap_or(default_desc.clone());
        writeln!(&mut writer, "{package:<column_width$}  {desc}")?;
    }
    writer.flush().context("couldn't flush search results")?;
    eprintln!("\nUse `flox show {{package}}` to see available versions");
    Ok(())
}

fn render_search_results_json(search_results: SearchResults) -> Result<()> {
    let json = serde_json::to_string(&search_results.results)?;
    println!("{}", json);
    Ok(())
}

/// Deduplicate and disambiguate display items.
///
/// This gets complicated because we have to satisfy a few constraints:
/// - The order of results from `pkgdb` is important (best matches come first),
///   so that order must be preserved.
/// - Versions shouldn't appear in the output, so multiple package versions from a single
///   input should be deduplicated.
/// - Packages that appear in more than one input need to be disambiguated by prepending
///   the name of the input and a separator.
fn dedup_and_disambiguate_display_items(mut display_items: Vec<DisplayItem>) -> Vec<DisplayItem> {
    let mut package_to_inputs: HashMap<String, HashSet<String>> = HashMap::new();
    for d in display_items.iter() {
        // Build a collection of packages and which inputs they are seen in so we can tell
        // which packages need to be disambiguated when rendering search results.
        package_to_inputs
            .entry(d.package.clone())
            .and_modify(|inputs| {
                inputs.insert(d.input.clone());
            })
            .or_insert_with(|| HashSet::from_iter([d.input.clone()]));
    }

    // For any package that comes from more than one input, mark it as needing to be joined
    for d in display_items.iter_mut() {
        if let Some(inputs) = package_to_inputs.get(&d.package) {
            d.render_with_input = inputs.len() > 1;
        }
    }

    // For each package in the search results, `package_to_inputs` contains the set of
    // inputs that the package is found in. Logically `package_to_inputs` contains
    // (package, input) pairs. If the `package` and `input` from a `DisplayItem` are
    // found in `package_to_inputs` it means that we have not yet seen this (package, input)
    // pair and we should render it (e.g. add it to `deduped_display_items`). Once we've
    // done that we remove this (package, input) pair from `package_to_inputs` so that
    // we never see that pair again.
    let mut deduped_display_items = Vec::new();
    for d in display_items.into_iter() {
        if let Some(inputs) = package_to_inputs.get_mut(d.package.as_str()) {
            // Remove this input so this (package, input) pair is never seen again
            if inputs.remove(&d.input) {
                deduped_display_items.push(d.clone());
            }
            if inputs.is_empty() {
                package_to_inputs.remove(&d.package);
            }
        }
    }

    deduped_display_items
}

/// Show detailed package information
#[derive(Bpaf, Clone)]
pub struct Show {
    /// Whether to show all available package versions
    #[bpaf(long)]
    pub all: bool,

    /// The package to show detailed information about. Must be an exact match
    /// for a package name e.g. something copy-pasted from the output of `flox search`.
    #[bpaf(positional("search-term"))]
    pub search_term: String,
}

impl Show {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");

        let (manifest, lockfile) = manifest_and_lockfile(&flox, "show packages using")
            .context("failed while looking for manifest and lockfile")?;
        let search_params = construct_show_params(
            &self.search_term,
            manifest.map(|p| p.try_into()).transpose()?,
            global_manifest_path(&flox).try_into()?,
            lockfile.map(|p| p.try_into()).transpose()?,
        )?;

        let (search_results, exit_status) = do_search(&search_params)?;

        if search_results.results.is_empty() {
            bail!("no packages matched this search term: {}", self.search_term);
        }
        // Render what we have no matter what, then indicate whether we encountered an error.
        // FIXME: We may have warnings on `stderr` even with a successful call to `pkgdb`.
        //        We aren't checking that at all at the moment because better overall error handling
        //        is coming in a later PR.
        render_show(search_results.results.as_slice(), self.all)?;
        if exit_status.success() {
            Ok(())
        } else {
            bail!(
                "pkgdb exited with status code: {}",
                exit_status.code().unwrap_or(-1),
            );
        }
    }
}

fn construct_show_params(
    search_term: &str,
    manifest: Option<PathOrJson>,
    global_manifest: PathOrJson,
    lockfile: Option<PathOrJson>,
) -> Result<SearchParams> {
    let parts = search_term
        .split(SEARCH_INPUT_SEPARATOR)
        .map(String::from)
        .collect::<Vec<_>>();
    let (_input_name, package_name) = match parts.as_slice() {
        [package_name] => (None, Some(package_name.to_owned())),
        [input_name, package_name] => (Some(input_name.to_owned()), Some(package_name.to_owned())),
        _ => Err(ShowError::InvalidSearchTerm(search_term.to_owned()))?,
    };

    let query = Query::from_term_and_limit(
        package_name.as_ref().unwrap(), // We already know it's Some(_)
        Features::parse()?.search_strategy == SearchStrategy::MatchName,
        None,
    )?;
    let search_params = SearchParams {
        manifest,
        global_manifest,
        lockfile,
        query,
    };
    debug!("show params raw: {:?}", search_params);
    Ok(search_params)
}

fn render_show(search_results: &[SearchResult], all: bool) -> Result<()> {
    let mut pkg_name = None;
    let mut results = Vec::new();
    // Collect all versions of the top search result
    for package in search_results.iter() {
        let this_pkg_name = package.rel_path.join(".");
        if pkg_name.is_none() {
            pkg_name = Some(this_pkg_name.clone());
        }
        if pkg_name == Some(this_pkg_name) {
            results.push(package);
        }
    }
    if results.is_empty() {
        // This should never happen since we've already checked that the
        // set of results is non-empty.
        bail!("no packages found");
    }
    let pkg_name = pkg_name.unwrap();
    let description = results[0]
        .description
        .as_ref()
        .map(|d| d.replace('\n', " "))
        .unwrap_or(DEFAULT_DESCRIPTION.into());
    let versions = if all {
        let multiple_versions = results
            .iter()
            .filter_map(|sr| {
                // Don't show a "latest" search result, it's just
                // a duplicate
                if sr.subtree == Subtree::Catalog
                    && sr
                        .abs_path
                        .last()
                        .map(|version| version == "latest")
                        .unwrap_or(false)
                {
                    return None;
                }
                let name = sr.rel_path.join(".");
                // We don't print packages that don't have a version since
                // the resolver will always rank versioned packages higher.
                sr.version.clone().map(|version| [name, version].join("@"))
            })
            .collect::<Vec<_>>();
        multiple_versions.join(", ")
    } else {
        let sr = results[0];
        let name = sr.rel_path.join(".");
        let version = sr.version.clone();
        if let Some(version) = version {
            [name, version].join("@")
        } else {
            name
        }
    };
    println!("{pkg_name} - {description}");
    println!("    {pkg_name} - {versions}");
    Ok(())
}

/// Searches for an environment to use, and if one is found, returns the path to
/// its manifest and optionally the path to its lockfile.
///
/// Note that this may perform network operations to pull a ManagedEnvironment,
/// since a freshly cloned user repo with a ManagedEnvironment may not have a
/// manifest or lockfile in floxmeta unless the environment is initialized.
pub fn manifest_and_lockfile(
    flox: &Flox,
    message: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let res = match detect_environment(message)? {
        None => {
            debug!("no environment found");
            (None, None)
        },
        Some(uninitialized) => {
            debug!(
                "using environment {}",
                hacky_environment_description(&uninitialized)?
            );
            let environment = open_environment(flox, uninitialized)?.into_dyn_environment();
            let lockfile_path = environment.lockfile_path();
            debug!("checking lockfile: path={}", lockfile_path.display());
            let lockfile = if lockfile_path.exists() {
                debug!("lockfile exists");
                Some(lockfile_path)
            } else {
                debug!("lockfile doesn't exist");
                None
            };
            (Some(environment.manifest_path()), lockfile)
        },
    };
    Ok(res)
}
