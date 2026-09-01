use std::fmt::Write;
use std::num::{NonZeroU8, NonZeroU32};

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_config::Config;
use flox_events::{CliSearchPayload, EventKind, EventsHub};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::catalog::SearchTerm;
use floxhub_client::{ByCommandResult, CatalogClientTrait, PackageSystem, SearchResults};
use indoc::{formatdoc, indoc};
use tracing::{debug, instrument};

use crate::commands::run::{DISAMBIGUATION_LIMIT, classify_by_command_error};
use crate::subcommand_metric;
use crate::utils::didyoumean::{DidYouMean, SearchSuggestion};
use crate::utils::message::{self, stderr_supports_color, stdout_supports_color};
use crate::utils::search::DisplaySearchResults;
use crate::utils::tracing::sentry_set_tag;

pub(crate) const DEFAULT_SEARCH_LIMIT: Option<NonZeroU8> = NonZeroU8::new(10);
const FLOX_SHOW_HINT: &str = "Use 'flox show <package>' to see available versions";

// Search for packages to install
#[derive(Debug, Bpaf, Clone)]
pub struct Search {
    /// Display search results as a JSON array
    #[bpaf(long)]
    pub json: bool,

    /// Print all search results
    #[bpaf(short, long)]
    pub all: bool,

    /// Look up which packages provide a named command
    #[bpaf(long, argument("name"))]
    pub command: Option<String>,

    /// The package to search for in the format '<pkg-path>'.
    ///
    /// ex. python310Packages.pip
    #[bpaf(positional("search-term"), optional)]
    pub search_term: Option<String>,
}

impl Search {
    #[instrument(name = "search", skip_all)]
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if let Some(ref command_name) = self.command {
            return self.handle_command_search(command_name, &flox).await;
        }

        // Regular search path — require a search term.
        let search_term = match &self.search_term {
            Some(t) => t.clone(),
            None => {
                message::error(indoc! {"
                    No search term provided.
                    Try searching for a package. For example, 'flox search curl'"});
                return Err(crate::Exit(1).into());
            },
        };

        sentry_set_tag("json", self.json);
        sentry_set_tag("show_all", self.all);
        sentry_set_tag("search_term", &search_term);
        subcommand_metric!("search", search_term = search_term);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliSearch(
            CliSearchPayload::new(search_term.clone()),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        debug!("performing search for term: {}", search_term);

        let limit = if self.all {
            None
        } else {
            config.flox.search_limit.or(DEFAULT_SEARCH_LIMIT)
        };

        let results = {
            tracing::debug!("using catalog client for search");
            let parsed_search = match SearchTerm::from_arg(&search_term) {
                SearchTerm::Clean(term) => term,
                SearchTerm::VersionStripped(term) => {
                    message::warning(indoc::indoc! {"
                        'flox search' ignores version specifiers.
                        To see available versions of a package, use 'flox show'
                    "});
                    term
                },
            };

            let catalog = &flox.floxhub_client;
            catalog
                .search_with_spinner(parsed_search, flox.system.clone().try_into()?, limit)
                .await?
        };

        // Render what we have no matter what, then indicate whether we encountered an error.
        if self.json {
            debug!("printing search results as JSON");
            render_search_results_json(results)?;
        } else {
            debug!("printing search results as user facing");

            let system = flox.system.clone();
            let catalog = &flox.floxhub_client;
            let suggestion = DidYouMean::<SearchSuggestion>::new(
                &search_term,
                catalog,
                system,
                stderr_supports_color(),
            );

            if results.results.is_empty() {
                let mut message =
                    format!("No packages matched this search term: '{}'", search_term);
                if suggestion.has_suggestions() {
                    message = formatdoc! {"
                        {message}

                        {suggestion}

                        {FLOX_SHOW_HINT}
                    "};
                }
                bail!(message);
            }

            let results = DisplaySearchResults::from_search_results(
                &search_term,
                results,
                stdout_supports_color(),
            )?;
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

            // We should use message::plain once bold formatting is fixed in
            // tracing-subscriber
            // https://github.com/tokio-rs/tracing/issues/3369
            eprintln!("{hints}");
        }
        Ok(())
    }

    async fn handle_command_search(&self, command_name: &str, flox: &Flox) -> Result<()> {
        subcommand_metric!("search", command = command_name);
        debug!("searching for command providers: {}", command_name);

        let system: PackageSystem = flox.system.clone().try_into()?;

        // Pass None (all pages) when --all is requested; otherwise fetch a
        // single page of DISAMBIGUATION_LIMIT rows (cheap, total_count is
        // still the full catalog total for the truncation hint).
        let api_limit = if self.all {
            None
        } else {
            NonZeroU32::new(DISAMBIGUATION_LIMIT as u32)
        };

        let result = flox
            .floxhub_client
            .by_command(command_name, system, api_limit)
            .await
            .map_err(|e| classify_by_command_error(e, command_name.to_string()))?;

        if self.json {
            let json = serde_json::to_string(&result)?;
            println!("{json}");
            return Ok(());
        }

        if result.providers.is_empty() {
            return if result.listing_known {
                Err(crate::commands::run::RunError::NoCommandProvider {
                    command: command_name.to_string(),
                }
                .into())
            } else {
                Err(crate::commands::run::RunError::CommandNotIndexed {
                    command: command_name.to_string(),
                }
                .into())
            };
        }

        println!("{}", render_command_providers(command_name, &result));

        // Truncation hint: shown only when the API total exceeds what we
        // fetched (i.e. --all was not passed and results were capped).
        if result.total_count > result.providers.len() as i64 {
            eprintln!(
                "\nℹ️  There are {} packages that supply '{}'.\n\
                 Use 'flox search --command {} --all' or\n\
                 'flox run --package <PACKAGE> {}' to choose a specific package.",
                result.total_count, command_name, command_name, command_name
            );
        }

        Ok(())
    }
}

fn render_search_results_json(search_results: SearchResults) -> Result<()> {
    let json = serde_json::to_string(&search_results.results)?;
    println!("{json}");
    Ok(())
}

/// Render a `by_command` provider list for `flox search --command`.
///
/// Exact matches are listed first and marked with `*`. All providers in
/// `result.providers` are shown; the caller controls how many were fetched
/// via the `api_limit` passed to `by_command`.
fn render_command_providers(command: &str, result: &ByCommandResult) -> String {
    let providers = &result.providers;
    let total = result.total_count;
    let exact_count = providers.iter().filter(|p| p.exact_name_match).count();

    let mut s = String::new();

    // Header: always mention exact match count so "exact matches" is present.
    let exact_str = match exact_count {
        0 => " — 0 exact matches (*)".to_string(),
        1 => " — 1 exact match (*)".to_string(),
        n => format!(" — {n} exact matches (*)"),
    };
    let plural = if total == 1 { "" } else { "s" };
    let _ = writeln!(s, "{total} package{plural} provide '{command}'{exact_str}:");

    // Sort: exact matches first, then alphabetically by pname.
    let mut sorted = providers.clone();
    sorted.sort_by(|a, b| {
        b.exact_name_match
            .cmp(&a.exact_name_match)
            .then_with(|| a.pname.cmp(&b.pname))
    });

    for p in &sorted {
        let marker = if p.exact_name_match { " *" } else { "  " };
        let _ = writeln!(s, " {marker} {:<12} ({})", p.pname, p.attr_path);
    }

    s.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use floxhub_client::{ByCommandResult, CommandProvider};

    use super::*;

    fn make_provider(pname: &str, attr_path: &str, exact: bool) -> CommandProvider {
        CommandProvider {
            attr_path: attr_path.to_string(),
            exact_name_match: exact,
            pname: pname.to_string(),
            system: "x86_64-linux".to_string().try_into().unwrap(),
        }
    }

    fn make_result(
        command: &str,
        providers: Vec<CommandProvider>,
        listing_known: bool,
    ) -> ByCommandResult {
        let total = providers.len() as i64;
        ByCommandResult {
            command_name: command.to_string(),
            listing_known,
            providers,
            total_count: total,
        }
    }

    // render_command_providers: single exact match uses singular form.
    #[test]
    fn render_single_exact_match() {
        let result = make_result("rg", vec![make_provider("ripgrep", "ripgrep", true)], true);
        let output = render_command_providers("rg", &result);
        assert!(output.contains("1 exact match"), "output: {output}");
        assert!(output.contains("ripgrep"), "output: {output}");
        assert!(!output.contains("2 exact"), "output: {output}");
    }

    // render_command_providers: zero exact matches — "0 exact matches" in header.
    #[test]
    fn render_no_exact_matches() {
        let result = make_result(
            "vi",
            vec![
                make_provider("vim", "vim", false),
                make_provider("neovim", "neovim", false),
            ],
            true,
        );
        let output = render_command_providers("vi", &result);
        assert!(output.contains("0 exact matches"), "output: {output}");
    }

    // render_command_providers: multiple exact matches — "N exact matches" in header.
    #[test]
    fn render_multiple_exact_matches() {
        let result = make_result(
            "vi",
            vec![
                make_provider("vim", "vim", true),
                make_provider("vi", "vi", true),
                make_provider("neovim", "neovim", false),
            ],
            true,
        );
        let output = render_command_providers("vi", &result);
        assert!(output.contains("2 exact matches"), "output: {output}");
    }

    // render_command_providers: exact matches appear before non-exact.
    #[test]
    fn render_exact_matches_first() {
        let result = make_result(
            "vi",
            vec![
                make_provider("neovim", "neovim", false),
                make_provider("vim", "vim", true),
            ],
            true,
        );
        let output = render_command_providers("vi", &result);
        let vim_row_pos = output.find("(vim)").unwrap();
        let neovim_row_pos = output.find("(neovim)").unwrap();
        assert!(
            vim_row_pos < neovim_row_pos,
            "exact match should sort before non-exact"
        );
    }

    // render_command_providers: all providers in result are shown (no internal cap).
    // The API limit controls how many are fetched; render shows everything it receives.
    #[test]
    fn render_shows_all_received_providers() {
        let providers: Vec<_> = (0..15)
            .map(|i| make_provider(&format!("pkg{i}"), &format!("pkg{i}"), false))
            .collect();
        let result = make_result("cmd", providers, true);
        let output = render_command_providers("cmd", &result);
        assert!(output.contains("pkg14"), "all providers shown: {output}");
        assert!(!output.contains("shown,"), "no truncation line: {output}");
    }

    // render_command_providers: output body never mentions "flox search".
    // The search hint appears only in the eprintln truncation hint, not the body.
    #[test]
    fn render_body_excludes_search_hint() {
        let result = make_result("rg", vec![make_provider("ripgrep", "ripgrep", false)], true);
        let output = render_command_providers("rg", &result);
        assert!(
            !output.contains("flox search"),
            "body should not mention flox search"
        );
    }
}
