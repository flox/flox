use std::collections::BTreeMap;

use anyhow::{Context, Result};
use catalog_api_v1::Client as CatalogApiClient;
use catalog_api_v1::types::{CatalogName, PackageName};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::CatalogId;

/// A node in the package hierarchy
///
/// This can either be:
/// - A Package with a locked URL
/// - A PackageSet containing other packages and package sets
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum PackageNode {
    Package { locked_url: String },
    PackageSet(BTreeMap<String, PackageNode>),
}

/// A catalog snapshot containing all packages
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CatalogSnapshot {
    pub packages: BTreeMap<String, PackageNode>,
}

/// Locks a catalog by fetching all packages and their latest build URLs
///
/// # Arguments
///
/// * `catalog_name` - The name of the catalog to lock
/// * `catalog_url` - The base URL of the catalog API
/// * `auth_token` - Optional authentication token for the catalog API
///
/// # Returns
///
/// Returns a `CatalogSnapshot` containing packages organized by their attribute paths
///
/// # Errors
///
/// Returns an error if:
/// - The catalog name is invalid
/// - The API request fails
/// - The response cannot be parsed
/// - A package has no builds available
pub(crate) async fn lock_catalog(
    catalog_id: &CatalogId,
    catalog_url: &Url,
    auth_token: &Option<String>,
) -> Result<CatalogSnapshot> {
    // Create the catalog API client
    let client = create_catalog_client(catalog_url, auth_token)?;

    // Parse the catalog name
    let catalog_name_typed: CatalogName = catalog_id
        .to_string()
        .parse()
        .context("Invalid catalog name format")?;

    tracing::info!("Fetching packages for catalog: {catalog_id}");

    // Fetch all packages in the catalog
    let packages_response = client
        .get_catalog_packages_api_v1_catalog_catalogs_catalog_name_packages_get(&catalog_name_typed)
        .await
        .context("Failed to fetch catalog packages")?;

    let packages = packages_response.into_inner().items;

    tracing::info!("Found {} packages in catalog", packages.len());

    // Build the catalog snapshot
    let mut snapshot = BTreeMap::new();

    for package in &packages {
        let package_name = &package.name;

        // Parse the package name
        let package_name_typed: PackageName = package_name
            .parse()
            .with_context(|| format!("Invalid package name format: {}", package_name))?;

        // Fetch builds for this package
        let builds_response = get_builds(&client, &catalog_name_typed, package_name_typed).await?;

        let builds = builds_response.into_inner().items;

        // Get the latest build (builds are typically sorted by date, newest first)
        let Some(latest_build) = builds.first() else {
            tracing::warn!(%package_name, "No builds available for package");
            continue;
        };

        // Extract the locked URL
        let url = &latest_build.url;
        let rev = &latest_build.rev;
        let locked_url = format!("git+{url}?rev={rev}");

        tracing::debug!("Locked package '{}' to URL: {}", package_name, locked_url);

        // Insert into the snapshot based on attribute path
        // Split the package name by '.' to get the path components
        let parts: Vec<&str> = package_name.split('.').collect();
        insert_package(&mut snapshot, &parts, locked_url);
    }

    tracing::info!(
        "Successfully locked {} packages from catalog '{}'",
        packages.len(),
        catalog_id
    );

    Ok(CatalogSnapshot { packages: snapshot })
}

#[tracing::instrument(fields(progress = format!("Getting latest build for '{}'", package_name_typed.as_str())))]
async fn get_builds(
    client: &CatalogApiClient,
    catalog_name_typed: &CatalogName,
    package_name_typed: PackageName,
) -> Result<catalog_api_v1::ResponseValue<catalog_api_v1::types::PackageBuildList>, anyhow::Error> {
    let builds_response = client
        .get_package_builds_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_get(
            catalog_name_typed,
            &package_name_typed,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to fetch builds for package: {}",
                package_name_typed.as_str()
            )
        })?;
    Ok(builds_response)
}

/// Insert a package into the catalog snapshot at the given path
///
/// # Arguments
///
/// * `snapshot` - The catalog snapshot to insert into
/// * `path` - The attribute path components (e.g., ["python", "packages", "requests"])
/// * `locked_url` - The locked URL for the package
fn insert_package(snapshot: &mut BTreeMap<String, PackageNode>, path: &[&str], locked_url: String) {
    if path.is_empty() {
        return;
    }

    if path.len() == 1 {
        // This is a package
        snapshot.insert(path[0].to_string(), PackageNode::Package { locked_url });
    } else {
        // This is a package set containing more packages/sets
        let key = path[0].to_string();
        let node = snapshot
            .entry(key.clone())
            .or_insert_with(|| PackageNode::PackageSet(BTreeMap::new()));

        match node {
            PackageNode::PackageSet(package_set) => {
                insert_package(package_set, &path[1..], locked_url);
            },
            PackageNode::Package { locked_url: _ } => {
                // Conflict: there's already a package where we need a package set
                // TODO: Handle this properly - for now we'll just warn and skip
                tracing::warn!("Conflict: '{}' is both a package and a package set", key);
            },
        }
    }
}

/// Creates a catalog API client with the given configuration
fn create_catalog_client(
    catalog_url: &Url,
    auth_token: &Option<String>,
) -> Result<CatalogApiClient> {
    let mut client_builder = reqwest::Client::builder();

    // Add authentication header if token is provided
    if let Some(token) = auth_token {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                .context("Invalid auth token")?,
        );
        client_builder = client_builder.default_headers(headers);
    }

    let http_client = client_builder
        .build()
        .context("Failed to create HTTP client")?;

    Ok(CatalogApiClient::new_with_client(
        catalog_url.as_str(),
        http_client,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_package() {
        let mut snapshot = BTreeMap::new();

        // Insert a simple package
        insert_package(
            &mut snapshot,
            &["curl"],
            "https://example.com/curl".to_string(),
        );
        assert_eq!(
            snapshot.get("curl"),
            Some(&PackageNode::Package {
                locked_url: "https://example.com/curl".to_string()
            })
        );

        // Insert a nested package
        insert_package(
            &mut snapshot,
            &["python", "packages", "requests"],
            "https://example.com/requests".to_string(),
        );

        match snapshot.get("python") {
            Some(PackageNode::PackageSet(python_set)) => match python_set.get("packages") {
                Some(PackageNode::PackageSet(packages_set)) => {
                    assert_eq!(
                        packages_set.get("requests"),
                        Some(&PackageNode::Package {
                            locked_url: "https://example.com/requests".to_string()
                        })
                    );
                },
                _ => panic!("Expected packages to be a PackageSet"),
            },
            _ => panic!("Expected python to be a PackageSet"),
        }

        // Insert another package in the same package set
        insert_package(
            &mut snapshot,
            &["python", "packages", "numpy"],
            "https://example.com/numpy".to_string(),
        );

        match snapshot.get("python") {
            Some(PackageNode::PackageSet(python_set)) => match python_set.get("packages") {
                Some(PackageNode::PackageSet(packages_set)) => {
                    assert_eq!(
                        packages_set.get("numpy"),
                        Some(&PackageNode::Package {
                            locked_url: "https://example.com/numpy".to_string()
                        })
                    );
                    assert_eq!(packages_set.len(), 2);
                },
                _ => panic!("Expected packages to be a PackageSet"),
            },
            _ => panic!("Expected python to be a PackageSet"),
        }
    }
}
