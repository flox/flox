use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::str::FromStr;

use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::search::{
    do_search,
    Query,
    Registry,
    RegistryDefaults,
    RegistryInput,
    SearchParams,
    SearchResult,
    SearchResults,
    ShowError,
    Subtree,
};
use log::debug;
use serde_json::json;

use crate::subcommand_metric;

const SEARCH_INPUT_SEPARATOR: &'_ str = ":";
const DEFAULT_DESCRIPTION: &'_ str = "<no description provided>";

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

        let search_params = construct_search_params(&self.search_term, &flox)?;
        debug!(
            "search parameters: {}",
            serde_json::to_string(&search_params).unwrap_or("<failed to serialize>".to_string())
        );

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
            render_search_results_user_facing(results)?;
            eprintln!("\nUse `flox show {{package}}` to see available versions");
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

fn construct_search_params(search_term: &str, flox: &Flox) -> Result<SearchParams> {
    // Create `registry` parameter for `pkgdb`
    let (inputs, priority) = collect_manifest_inputs(flox);
    debug!(
        "collected manifest inputs named: {}",
        inputs.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    let registry = Registry {
        inputs,
        priority,
        defaults: RegistryDefaults::default(),
    };

    // We've already checked that the search term is Some(_)
    let query = Query::from_str(search_term)?;
    let params = SearchParams {
        registry,
        query,
        systems: Some(vec![flox.system.clone()]),
        ..SearchParams::default()
    };
    debug!("search params: {:?}", params);
    Ok(params)
}

/// This function is a hack to convert the current subscriptions into a format
/// that matches the search spec, which expects sources to come from the manifest.
///
/// This is temporary and will be removed once we have a functioning manifest.
fn collect_manifest_inputs(_flox: &Flox) -> (HashMap<String, RegistryInput>, Vec<String>) {
    let priority = vec!["nixpkgs".to_string()];
    let nixpkgs_json = json!({
        "type": "github",
        "owner": "NixOS",
        "repo": "nixpkgs",
        "rev": "e8039594435c68eb4f780f3e9bf3972a7399c4b1",
    });
    let reg_input = RegistryInput {
        from: serde_json::from_value(nixpkgs_json).unwrap(),
        subtrees: Some(vec!["legacyPackages".to_string()]),
    };
    let inputs = [("nixpkgs".to_string(), reg_input)]
        .into_iter()
        .collect::<HashMap<_, _>>();
    (inputs, priority)
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

fn render_search_results_user_facing(search_results: SearchResults) -> Result<()> {
    // Nothing to display
    if search_results.results.is_empty() {
        return Ok(());
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
        let search_params = construct_show_params(&self.search_term, &flox)?;

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

fn construct_show_params(search_term: &str, flox: &Flox) -> Result<SearchParams> {
    let parts = search_term
        .split(SEARCH_INPUT_SEPARATOR)
        .map(String::from)
        .collect::<Vec<_>>();
    let (input_name, package_name) = match parts.as_slice() {
        [package_name] => (None, Some(package_name.to_owned())),
        [input_name, package_name] => (Some(input_name.to_owned()), Some(package_name.to_owned())),
        _ => Err(ShowError::InvalidSearchTerm(search_term.to_owned()))?,
    };

    // If we're given a specific input to search, only search that one,
    // otherwise build the whole list of inputs to search
    let (inputs, priority) = if let Some(input_name) = input_name {
        let Some(reg_input) = flox
            .channels
            .iter()
            .find(|entry| entry.from.id == input_name)
            .map(|entry| RegistryInput {
                from: entry.to.clone(),
                subtrees: None,
            })
        else {
            bail!("manifest did not contain an input named '{}'", input_name)
        };
        let mut inputs = HashMap::new();
        inputs.insert(input_name.clone(), reg_input);
        (inputs, vec![input_name])
    } else {
        collect_manifest_inputs(flox)
    };

    // Only search the registry input that the search result comes from
    let registry = Registry {
        inputs,
        priority,
        ..Registry::default()
    };
    let query = Query {
        r#match: package_name,
        ..Query::default()
    };

    Ok(SearchParams {
        registry,
        query,
        ..SearchParams::default()
    })
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
