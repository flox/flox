use std::str::FromStr;

use anyhow::{Result, bail};
use beta::extensions::{self, SearchQuery, SearchRow, SearchSort, validate_owner};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::message;

/// `--sort <stars|updated>` — single flag with a value, matching the P08
/// deliverable. `FromStr` gates the input so bpaf produces a type error
/// instead of silently accepting an unknown mode.
#[derive(Debug, Clone, Copy)]
pub enum SortArg {
    Stars,
    Updated,
}

impl FromStr for SortArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stars" => Ok(SortArg::Stars),
            "updated" => Ok(SortArg::Updated),
            other => Err(format!(
                "unknown sort '{other}'; expected 'stars' or 'updated'"
            )),
        }
    }
}

impl SortArg {
    fn into_search_sort(self) -> SearchSort {
        match self {
            SortArg::Stars => SearchSort::Stars,
            SortArg::Updated => SearchSort::Updated,
        }
    }
}

#[derive(Debug, Bpaf, Clone)]
pub struct Search {
    /// Limit results to a specific owner/organization
    #[bpaf(long, argument("OWNER"))]
    owner: Option<String>,

    /// Maximum number of results to return (1–100)
    #[bpaf(long, argument("N"), fallback(30))]
    limit: u8,

    /// Sort order: 'stars' (default) or 'updated'
    #[bpaf(long, argument("MODE"), fallback(SortArg::Stars))]
    sort: SortArg,

    /// Free-form search term matched against repo name/description
    #[bpaf(positional("QUERY"))]
    query: Option<String>,
}

impl Search {
    #[instrument(name = "extensions::search", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("extensions::search");

        if let Some(owner) = self.owner.as_deref()
            && let Err(e) = validate_owner(owner)
        {
            bail!("{e}");
        }

        let q = SearchQuery::new(
            self.query.clone(),
            self.owner.clone(),
            self.limit,
            self.sort.into_search_sort(),
        );

        let (rows, incomplete) = extensions::search(&flox, &q).await?;
        if rows.is_empty() {
            message::plain("No matching extensions found.");
            return Ok(());
        }

        println!("{}", render_header());
        for row in &rows {
            println!("{}", render_row(row));
        }

        if incomplete {
            message::warning("github reported incomplete results; re-run with a narrower query");
        }

        Ok(())
    }
}

fn render_header() -> String {
    format!(
        "{:<2}  {:<40}  {:>6}  {}",
        " ", "OWNER/REPO", "STARS", "DESCRIPTION"
    )
}

fn render_row(row: &SearchRow) -> String {
    let mark = if row.installed { "\u{2713}" } else { " " };
    let desc = row.description.as_deref().unwrap_or("-");
    let desc = truncate_description(desc, 60);
    format!(
        "{:<2}  {:<40}  {:>6}  {}",
        mark, row.full_name, row.stars, desc
    )
}

fn truncate_description(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use beta::extensions::SearchRow;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn row_marks_installed_with_check() {
        let row = SearchRow {
            full_name: "acme/flox-hello".to_string(),
            stars: 42,
            description: Some("hello world extension".to_string()),
            installed: true,
        };
        let rendered = render_row(&row);
        assert!(rendered.starts_with('\u{2713}'), "row: {rendered}");
        assert!(rendered.contains("acme/flox-hello"));
        assert!(rendered.contains("42"));
    }

    #[test]
    fn row_without_install_has_no_check() {
        let row = SearchRow {
            full_name: "acme/flox-hello".to_string(),
            stars: 1,
            description: None,
            installed: false,
        };
        let rendered = render_row(&row);
        assert!(!rendered.contains('\u{2713}'), "row: {rendered}");
        assert!(
            rendered.contains('-'),
            "missing description fallback: {rendered}"
        );
    }

    #[test]
    fn truncate_description_appends_ellipsis() {
        let s: String = "a".repeat(80);
        let out = truncate_description(&s, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('\u{2026}'));
    }

    #[test]
    fn truncate_description_passthrough_when_short() {
        assert_eq!(truncate_description("short", 60), "short");
    }

    #[test]
    fn truncate_description_with_zero_max_returns_empty() {
        // Guards against a subtle off-by-one: `max.saturating_sub(1)` is 0
        // when `max == 0`, and without the early return we would still push
        // an ellipsis, yielding a 1-char output exceeding the requested max.
        assert_eq!(truncate_description("anything", 0), "");
    }

    #[test]
    fn sort_arg_from_str_accepts_known_modes() {
        assert!(matches!("stars".parse::<SortArg>(), Ok(SortArg::Stars)));
        assert!(matches!("updated".parse::<SortArg>(), Ok(SortArg::Updated)));
    }

    #[test]
    fn sort_arg_from_str_rejects_unknown_modes() {
        let err = "random".parse::<SortArg>().unwrap_err();
        assert!(err.contains("random"), "message missing input: {err}");
        assert!(
            err.contains("stars") && err.contains("updated"),
            "message should list valid modes: {err}"
        );
    }
}
