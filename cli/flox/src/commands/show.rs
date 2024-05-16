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
};
use flox_rust_sdk::providers::catalog::{ClientTrait, VersionsError};
use log::debug;
use tracing::instrument;

use crate::config::features::Features;
use crate::subcommand_metric;
use crate::utils::message;
use crate::utils::search::{manifest_and_lockfile, DEFAULT_DESCRIPTION, SEARCH_INPUT_SEPARATOR};

// Show detailed package information
#[derive(Debug, Bpaf, Clone)]
pub struct Show {
    /// Whether to show all available package versions
    #[bpaf(long, hide)]
    pub all: bool, // TODO: Remove in future release.

    /// The package to show detailed information about. Must be an exact match
    /// for a pkg-path e.g. something copy-pasted from the output of `flox search`.
    #[bpaf(positional("pkg-path"))]
    pub pkg_path: String,
}

impl Show {
    #[instrument(name = "show", fields(show_all = self.all, pkg_path = self.pkg_path), skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");

        if self.all {
            message::warning("'--all' is now the default and the flag has been deprecated.");
        }

        let (results, exit_status) = if let Some(client) = flox.catalog_client {
            tracing::debug!("using catalog client for show");
            match client.package_versions(&self.pkg_path).await {
                Ok(results) => (results, None),
                // Below, results.is_empty() is used to mean the search_term
                // didn't match a package.
                // So translate 404 into an empty vec![].
                // Once we drop the pkgdb code path, we can clean this up.
                Err(VersionsError::Versions(e)) if e.status() == 404 => (
                    SearchResults {
                        results: vec![],
                        count: None,
                    },
                    None,
                ),
                Err(e) => Err(e)?,
            }
        } else {
            tracing::debug!("using pkgdb for show");

            let (manifest, lockfile) = manifest_and_lockfile(&flox, "Show using")
                .context("failed while looking for manifest and lockfile")?;
            let search_params = construct_show_params(
                &self.pkg_path,
                manifest.map(|p| p.try_into()).transpose()?,
                global_manifest_path(&flox).try_into()?,
                PathOrJson::Path(lockfile),
            )?;

            let (search_results, exit_status) = do_search(&search_params)?;
            (search_results, Some(exit_status))
        };

        if results.results.is_empty() {
            bail!("no packages matched this pkg-path: '{}'", self.pkg_path);
        }
        // Render what we have no matter what, then indicate whether we encountered an error.
        render_show(results.results.as_slice())?;
        if let Some(status) = exit_status {
            if status.success() {
                return Ok(());
            } else {
                bail!(
                    "pkgdb exited with status code: {}",
                    status.code().unwrap_or(-1),
                );
            }
        }
        Ok(())
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

fn render_show(search_results: &[SearchResult]) -> Result<()> {
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

    println!("{pkg_name} - {description}");
    for result in results.iter() {
        let name = result.rel_path.join(".");
        // We don't print packages that don't have a version since
        // the resolver will always rank versioned packages higher.
        let Some(version) = result.version.clone() else {
            continue;
        };
        println!("    {name}@{version}");
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use flox_rust_sdk::flox::test_helpers::flox_instance_with_optional_floxhub_and_client;
    use flox_rust_sdk::providers::catalog::{ApiErrorResponse, Client};

    use super::*;

    #[tokio::test]
    async fn show_handles_404() {
        let (mut flox, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(None, true);
        let Client::Mock(ref mut client) = flox.catalog_client.as_mut().unwrap() else {
            panic!()
        };
        client.push_error_response(
            ApiErrorResponse {
                detail: "detail".to_string(),
            },
            404,
        );
        let search_term = "search_term";
        let err = Show {
            all: true, // unused
            pkg_path: search_term.to_string(),
        }
        .handle(flox)
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            format!("no packages matched this pkg-path: '{}'", search_term)
        );
    }
}
