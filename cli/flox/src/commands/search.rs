use std::fmt::Write;
use std::num::NonZeroU8;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_config::Config;
use flox_events::{CliSearchPayload, EventKind, EventsHub};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::catalog::SearchTerm;
use floxhub_client::{
    ByCommandError,
    ByCommandResult,
    CatalogClientTrait,
    PackageSystem,
    SearchResults,
};
use indoc::{formatdoc, indoc};
use tracing::{debug, instrument};

use crate::commands::run::DISAMBIGUATION_LIMIT;
use crate::subcommand_metric;
use crate::utils::didyoumean::{DidYouMean, SearchSuggestion};
use crate::utils::message::{self, stderr_supports_color, stdout_supports_color};
use crate::utils::search::DisplaySearchResults;
use crate::utils::tracing::sentry_set_tag;

pub(crate) const DEFAULT_SEARCH_LIMIT: Option<NonZeroU8> = NonZeroU8::new(10);
const FLOX_SHOW_HINT: &str = "Use 'flox show <package>' to see available versions";

fn missing_search_term<T>() -> Result<T> {
    bail!(indoc! {"
        No search term provided.

        Try searching with a search term. For example, 'flox search curl'"});
}

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
            None => missing_search_term()?,
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

        let system: PackageSystem = flox
            .system
            .clone()
            .try_into()
            .expect("flox.system is always a valid PackageSystem");

        let result = flox
            .floxhub_client
            .by_command(command_name, system)
            .await
            .map_err(|e| classify_by_command_error(e, command_name))?;

        if self.json {
            let json = serde_json::to_string(&result)?;
            println!("{json}");
            return Ok(());
        }

        if result.providers.is_empty() {
            if result.listing_known {
                bail!("No packages provide '{command_name}'.");
            } else {
                bail!("'{command_name}' has not been indexed yet.");
            }
        }

        let limit = if self.all {
            None
        } else {
            Some(DISAMBIGUATION_LIMIT)
        };
        println!("{}", render_command_providers(command_name, &result, limit));

        if let Some(cap) = limit
            && result.total_count > cap as i64
        {
            eprintln!(
                "\nℹ️  There are {} packages that supply '{}'. \
                 Use 'flox search --command {} --all' and 'flox run --package' to specify.",
                result.total_count, command_name, command_name
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
/// Exact matches are listed first and marked with `*`. When `limit` is
/// `Some(n)`, at most `n` rows are printed and a count line is appended when
/// the total exceeds that cap. `None` prints all providers.
fn render_command_providers(
    command: &str,
    result: &ByCommandResult,
    limit: Option<usize>,
) -> String {
    use std::fmt::Write as _;

    let providers = &result.providers;
    let total = result.total_count;
    let exact_count = providers.iter().filter(|p| p.exact_name_match).count();

    let mut s = String::new();

    // Header: always mention exact match count so the output includes the term.
    let exact_str = match exact_count {
        0 => " (0 exact matches)".to_string(),
        1 => " (1 exact match)".to_string(),
        n => format!(" ({n} exact matches)"),
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

    let cap = limit.unwrap_or(usize::MAX);
    let shown = sorted.len().min(cap);
    for p in sorted.iter().take(cap) {
        let marker = if p.exact_name_match { " *" } else { "  " };
        let _ = writeln!(s, " {marker} {:<12} ({})", p.pname, p.attr_path);
    }

    if total > shown as i64 {
        let _ = writeln!(s, "  ... ({shown} shown, {total} total)");
    }

    s.trim_end().to_string()
}

fn classify_by_command_error(err: ByCommandError, command: &str) -> anyhow::Error {
    match err {
        ByCommandError::InvalidCommandName(e) => {
            anyhow::anyhow!("Invalid command name '{}': {}", command, e)
        },
        ByCommandError::FloxhubClientError(e) => {
            debug!(error = ?e, %command, "by_command lookup failed");
            anyhow::anyhow!(
                "Could not reach the Flox Catalog to look up '{command}'.\n\
                 Use 'flox run --package <PACKAGE> {command}' to run it directly."
            )
        },
    }
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
        let output = render_command_providers("rg", &result, Some(DISAMBIGUATION_LIMIT));
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
        let output = render_command_providers("vi", &result, Some(DISAMBIGUATION_LIMIT));
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
        let output = render_command_providers("vi", &result, Some(DISAMBIGUATION_LIMIT));
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
        let output = render_command_providers("vi", &result, Some(DISAMBIGUATION_LIMIT));
        let vim_row_pos = output.find("(vim)").unwrap();
        let neovim_row_pos = output.find("(neovim)").unwrap();
        assert!(
            vim_row_pos < neovim_row_pos,
            "exact match should sort before non-exact"
        );
    }

    // render_command_providers: truncation line shown when total > cap.
    #[test]
    fn render_truncation_line_when_over_limit() {
        let providers: Vec<_> = (0..12)
            .map(|i| make_provider(&format!("pkg{i}"), &format!("pkg{i}"), false))
            .collect();
        let mut result = make_result("cmd", providers, true);
        result.total_count = 12;
        let output = render_command_providers("cmd", &result, Some(10));
        assert!(output.contains("10 shown"), "output: {output}");
        assert!(output.contains("12 total"), "output: {output}");
    }

    // render_command_providers: --all (no limit) shows all rows, no truncation line.
    #[test]
    fn render_all_no_truncation() {
        let providers: Vec<_> = (0..15)
            .map(|i| make_provider(&format!("pkg{i}"), &format!("pkg{i}"), false))
            .collect();
        let result = make_result("cmd", providers, true);
        let output = render_command_providers("cmd", &result, None);
        assert!(!output.contains("shown"), "should not truncate: {output}");
        assert!(output.contains("pkg14"), "all providers shown: {output}");
    }

    // render_command_providers: output body never mentions "flox search".
    // The search hint appears only in the eprintln truncation line, not in the body.
    #[test]
    fn render_body_excludes_search_hint() {
        let result = make_result("rg", vec![make_provider("ripgrep", "ripgrep", false)], true);
        let output = render_command_providers("rg", &result, Some(DISAMBIGUATION_LIMIT));
        assert!(
            !output.contains("flox search"),
            "body should not mention flox search"
        );
    }
}
