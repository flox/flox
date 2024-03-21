use std::fmt::Write;
use std::time::Duration;

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
use indoc::formatdoc;
use log::debug;
use tracing::instrument;

use crate::config::features::Features;
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::didyoumean::{DidYouMean, SearchSuggestion};
use crate::utils::message;
use crate::utils::search::{
    construct_search_params,
    manifest_and_lockfile,
    DisplaySearchResults,
    DEFAULT_DESCRIPTION,
    SEARCH_INPUT_SEPARATOR,
};

const DEFAULT_SEARCH_LIMIT: Option<u8> = Some(10);
const FLOX_SHOW_HINT: &str = "Use 'flox show <package>' to see available versions";

#[derive(Bpaf, Clone)]
pub struct ChannelArgs {}

// Search for packages to install
#[derive(Debug, Bpaf, Clone)]
pub struct Search {
    /// Display search results as a JSON array
    #[bpaf(long)]
    pub json: bool,

    /// Print all search results
    #[bpaf(short, long)]
    pub all: bool,

    /// The package to search for in the format '<pkg-path>[@<semver-range>]' using 'node-semver' syntax.
    ///
    /// ex. python310Packages.pip
    ///
    /// ex. 'node@>=16' # quotes needed to prevent '>' redirection
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
    #[instrument(name = "search", fields(json = self.json, show_all = self.all, search_term = self.search_term), skip_all)]
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("search", search_term = &self.search_term);

        debug!("performing search for term: {}", self.search_term);

        let (manifest, lockfile) = manifest_and_lockfile(&flox, "search for packages using")
            .context("failed while looking for manifest and lockfile")?;

        let manifest = manifest.map(|p| p.try_into()).transpose()?;
        let lockfile = PathOrJson::Path(lockfile);
        let global_manifest: PathOrJson = global_manifest_path(&flox).try_into()?;

        let limit = if self.all {
            None
        } else {
            config.flox.search_limit.or(DEFAULT_SEARCH_LIMIT)
        };

        let search_params = construct_search_params(
            &self.search_term,
            limit,
            manifest.clone(),
            global_manifest.clone(),
            lockfile.clone(),
        )?;

        let (results, exit_status) = Dialog {
            message: "Searching for packages...",
            help_message: Some("This may take a while the first time you run it."),
            typed: Spinner::new(|| do_search(&search_params)),
        }
        .spin_with_delay(Duration::from_secs(1))?;

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

            let suggestion = DidYouMean::<SearchSuggestion>::new(
                &self.search_term,
                manifest,
                global_manifest,
                lockfile,
            );

            if results.results.is_empty() {
                let mut message =
                    format!("No packages matched this search term: {}", self.search_term);
                if suggestion.has_suggestions() {
                    message = formatdoc! {"
                        {message}

                        {suggestion}

                        {FLOX_SHOW_HINT}
                    "};
                }
                bail!(message);
            }

            let results = DisplaySearchResults::from_search_results(&self.search_term, results)?;
            println!("{results}");

            let mut hints = String::new();

            if let Some(hint) = results.search_results_truncated_hint() {
                writeln!(&mut hints)?;
                writeln!(&mut hints, "{hint}")?;
            }

            writeln!(&mut hints)?;
            writeln!(&mut hints, "{FLOX_SHOW_HINT}")?;

            if suggestion.has_suggestions() {
                writeln!(&mut hints)?;
                writeln!(&mut hints, "{suggestion}")?;
            };

            message::plain(hints);
        }
        Ok(())
    }
}

fn render_search_results_json(search_results: SearchResults) -> Result<()> {
    let json = serde_json::to_string(&search_results.results)?;
    println!("{}", json);
    Ok(())
}

// Show detailed package information
#[derive(Debug, Bpaf, Clone)]
pub struct Show {
    /// Whether to show all available package versions
    #[bpaf(long)]
    pub all: bool,

    /// The package to show detailed information about. Must be an exact match
    /// for a pkg-path e.g. something copy-pasted from the output of `flox search`.
    #[bpaf(positional("search-term"))]
    pub search_term: String,
}

impl Show {
    #[instrument(name = "show", fields(show_all = self.all, search_term = self.search_term), skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");

        let (manifest, lockfile) = manifest_and_lockfile(&flox, "show packages using")
            .context("failed while looking for manifest and lockfile")?;
        let search_params = construct_show_params(
            &self.search_term,
            manifest.map(|p| p.try_into()).transpose()?,
            global_manifest_path(&flox).try_into()?,
            PathOrJson::Path(lockfile),
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
    lockfile: PathOrJson,
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

    let query = Query::new(
        package_name.as_ref().unwrap(), // We already know it's Some(_)
        Features::parse()?.search_strategy,
        None,
        false,
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
