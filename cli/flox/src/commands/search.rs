use std::fmt::Write;
use std::num::NonZeroU8;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::global_manifest_path;
use flox_rust_sdk::models::search::{do_search, PathOrJson, SearchResults};
use flox_rust_sdk::providers::catalog::{ClientTrait, SearchTerm};
use indoc::formatdoc;
use log::debug;
use tracing::instrument;

use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::didyoumean::{DidYouMean, SearchSuggestion};
use crate::utils::message;
use crate::utils::search::{construct_search_params, manifest_and_lockfile, DisplaySearchResults};

pub(crate) const DEFAULT_SEARCH_LIMIT: Option<NonZeroU8> = NonZeroU8::new(10);
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

    /// The package to search for in the format '<pkg-path>'.
    ///
    /// ex. python310Packages.pip
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

        let (manifest, lockfile) = manifest_and_lockfile(&flox, "Search using")
            .context("failed while looking for manifest and lockfile")?;

        let manifest = manifest.map(|p| p.try_into()).transpose()?;
        let lockfile = PathOrJson::Path(lockfile);
        let global_manifest: PathOrJson = global_manifest_path(&flox).try_into()?;

        let limit = if self.all {
            None
        } else {
            config.flox.search_limit.or(DEFAULT_SEARCH_LIMIT)
        };

        let results = if let Some(client) = &flox.catalog_client {
            tracing::debug!("using catalog client for search");
            let parsed_search = match SearchTerm::from_arg(&self.search_term) {
                SearchTerm::Clean(term) => term,
                SearchTerm::VersionStripped(term) => {
                    message::warning(indoc::indoc! {"
                        'flox search' ignores version specifiers.
                        To see available versions of a package, use 'flox show'
                    "});
                    term
                },
            };
            Dialog {
                message: "Searching for packages...",
                help_message: None,
                typed: Spinner::new(|| {
                    tokio::runtime::Handle::current().block_on(client.search(
                        parsed_search,
                        flox.system.clone(),
                        limit,
                    ))
                }),
            }
            .spin_with_delay(Duration::from_secs(1))?
        } else {
            tracing::debug!("using pkgdb for search");

            let search_params = construct_search_params(
                &self.search_term,
                limit,
                manifest.clone(),
                global_manifest.clone(),
                lockfile.clone(),
                flox.features.search_strategy,
            )?;

            let (results, exit_status) = Dialog {
                message: "Searching for packages...",
                help_message: Some("This may take a while the first time you run it."),
                typed: Spinner::new(|| do_search(&search_params)),
            }
            .spin_with_delay(Duration::from_secs(1))?;
            tracing::debug!("search call exit status: {}", exit_status.to_string());
            results
        };

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
                flox.catalog_client,
                flox.system,
                manifest,
                global_manifest,
                lockfile,
                flox.features.search_strategy,
            );

            if results.results.is_empty() {
                let mut message = format!(
                    "No packages matched this search term: '{}'",
                    self.search_term
                );
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
