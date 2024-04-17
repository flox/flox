use async_trait::async_trait;
use catalog_api_v1::types::{self as api_types, error as api_error};
use catalog_api_v1::{Client as APIClient, Error as APIError};
use enum_dispatch::enum_dispatch;
use thiserror::Error;

use crate::data::System;

pub const DEFAULT_CATALOG_URL: &str = "https://flox-catalog.flox.dev";

/// Either a client for the actual catalog service,
/// or a mock client for testing.
#[derive(Debug)]
#[enum_dispatch(ClientTrait)]
pub enum Client {
    Catalog(CatalogClient),
    Mock(MockClient),
}

/// A client for the catalog service.
///
/// This is a wrapper around the auto-generated APIClient.
#[derive(Debug)]
pub struct CatalogClient {
    client: APIClient,
}
impl CatalogClient {
    pub fn new() -> Self {
        Self {
            client: APIClient::new(DEFAULT_CATALOG_URL),
        }
    }
}

#[derive(Debug)]
pub struct MockClient;

impl Default for CatalogClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
#[enum_dispatch]
pub trait ClientTrait {
    /// Resolve a list of [PackageGroup]s into a list of
    /// [ResolvedPackageGroup]s.
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, CatalogClientError>;
}

#[async_trait]
impl ClientTrait for CatalogClient {
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, CatalogClientError> {
        let package_groups = api_types::PackageGroups {
            items: package_groups
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        };

        let response = self
            .client
            .resolve_api_v1_catalog_resolve_post(&package_groups)
            .await
            .map_err(CatalogClientError::Resolution)?;

        let resolved_package_groups = response.into_inner();

        resolved_package_groups
            .items
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }
}

#[async_trait]
impl ClientTrait for MockClient {
    async fn resolve(
        &self,
        _package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, CatalogClientError> {
        unimplemented!()
    }
}

/// Just an alias until the auto-generated PackageDescriptor diverges from what
/// we need.
pub type PackageDescriptor = api_types::PackageDescriptor;

pub struct PackageGroup {
    pub descriptors: Vec<PackageDescriptor>,
    pub name: String,
    pub system: System,
}

#[derive(Debug, Error)]
pub enum CatalogClientError {
    #[error("system not supported by catalog")]
    UnsupportedSystem(#[source] api_error::ConversionError),
    #[error("resolution failed")]
    Resolution(#[source] APIError<api_types::ErrorResponse>),
}

impl TryFrom<PackageGroup> for api_types::PackageGroup {
    type Error = CatalogClientError;

    fn try_from(package_group: PackageGroup) -> Result<Self, CatalogClientError> {
        Ok(Self {
            descriptors: package_group.descriptors,
            name: package_group.name,
            system: package_group
                .system
                .try_into()
                .map_err(CatalogClientError::UnsupportedSystem)?,
            stability: None,
        })
    }
}

pub struct ResolvedPackageGroup {
    pub name: String,
    pub pages: Vec<CatalogPage>,
    pub system: System,
}

impl TryFrom<api_types::ResolvedPackageGroupInput> for ResolvedPackageGroup {
    type Error = CatalogClientError;

    fn try_from(
        resolved_package_group: api_types::ResolvedPackageGroupInput,
    ) -> Result<Self, CatalogClientError> {
        Ok(Self {
            name: resolved_package_group.name,
            pages: resolved_package_group
                .pages
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>(),
            system: resolved_package_group.system.to_string(),
        })
    }
}

pub struct CatalogPage {
    pub packages: Vec<PackageResolutionInfo>,
    pub page: i64,
    pub url: String,
}

impl From<api_types::CatalogPage> for CatalogPage {
    fn from(catalog_page: api_types::CatalogPage) -> Self {
        Self {
            packages: catalog_page.packages,
            page: catalog_page.page,
            url: catalog_page.url,
        }
    }
}

/// TODO: fix types for outputs and outputs_to_install,
/// at which point this will probably no longer be an alias.
type PackageResolutionInfo = api_types::PackageResolutionInfo;
