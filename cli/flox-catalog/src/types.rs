//! Catalog interaction types.
//!
//! These types represent the domain model for catalog operations,
//! wrapping the auto-generated API types with richer semantics.

use std::collections::HashMap;
use std::fmt::Display;
use std::num::NonZeroU8;

use catalog_api_v1::types as api_types;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::error::CatalogClientError;

// ---------------------------------------------------------------------------
// Result / pagination types (from models/search.rs)
// ---------------------------------------------------------------------------

pub type SearchLimit = Option<NonZeroU8>;
pub type ResultCount = Option<u64>;

/// Generic paginated result container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultsPage<T> {
    pub results: Vec<T>,
    pub count: ResultCount,
}

pub use api_types::PackageInfoSearch as SearchResult;
pub type SearchResults = ResultsPage<SearchResult>;

pub use api_types::{PackageOutput, PackageOutputs, PackageResolutionInfo as PackageBuild};
pub type PackageDetails = ResultsPage<PackageBuild>;

// ---------------------------------------------------------------------------
// Package descriptors
// ---------------------------------------------------------------------------

/// Just an alias until the auto-generated PackageDescriptor diverges from what
/// we need.
pub use api_types::{
    PackageDescriptor,
    PackageSystem,
    ResolvedPackageDescriptor as PackageResolutionInfo,
};

#[derive(Debug, PartialEq, Clone)]
pub struct PackageGroup {
    pub name: String,
    pub descriptors: Vec<PackageDescriptor>,
}

impl TryFrom<PackageGroup> for api_types::PackageGroup {
    type Error = CatalogClientError;

    fn try_from(package_group: PackageGroup) -> Result<Self, CatalogClientError> {
        Ok(Self {
            descriptors: package_group.descriptors,
            name: package_group.name,
            stability: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Type aliases for API types used in trait signatures
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Resolution messages
// ---------------------------------------------------------------------------
pub use api_types::MessageLevel;
pub use api_types::{
    CatalogStoreConfig,
    CatalogStoreConfigNixCopy,
    CatalogStoreConfigPublisher,
    NarInfo,
    NarInfos,
    PackageBuildWithNarInfo as UserBuildPublish,
    PackageDerivationInput as UserDerivationInfo,
    PublishInfoResponseCatalog as PublishResponse,
    StoreInfo,
    StoreInfoRequest,
    StoreInfoResponse,
    StorepathStatusResponse,
};

/// The content of a generic message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgGeneral {
    pub level: MessageLevel,
    pub msg: String,
}

/// A message indicating a package attr_path is not present in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgAttrPathNotFoundNotInCatalog {
    pub level: MessageLevel,
    pub msg: String,
    pub attr_path: String,
    pub install_id: String,
}

/// A message indicating no single page contains a package for all requested
/// systems.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgAttrPathNotFoundSystemsNotOnSamePage {
    pub level: MessageLevel,
    pub msg: String,
    pub attr_path: String,
    pub install_id: String,
    pub system_groupings: String,
}

/// A message indicating an attr_path is not found for all requested systems.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgAttrPathNotFoundNotFoundForAllSystems {
    pub level: MessageLevel,
    pub msg: String,
    pub attr_path: String,
    pub install_id: String,
    pub valid_systems: Vec<String>,
}

/// A message indicating version constraints are too tight.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgConstraintsTooTight {
    pub level: MessageLevel,
    pub msg: String,
}

/// A (yet) unknown message type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgUnknown {
    pub msg_type: String,
    pub level: MessageLevel,
    pub msg: String,
    pub context: HashMap<String, String>,
}

/// The kinds of resolution messages we can receive.
///
/// Constructed from [`ResolutionMessageGeneral`] by matching on `type_` and
/// interpreting the `context` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolutionMessage {
    General(MsgGeneral),
    AttrPathNotFoundNotInCatalog(MsgAttrPathNotFoundNotInCatalog),
    AttrPathNotFoundSystemsNotOnSamePage(MsgAttrPathNotFoundSystemsNotOnSamePage),
    AttrPathNotFoundNotFoundForAllSystems(MsgAttrPathNotFoundNotFoundForAllSystems),
    ConstraintsTooTight(MsgConstraintsTooTight),
    Unknown(MsgUnknown),
}

impl ResolutionMessage {
    pub fn msg(&self) -> &str {
        match self {
            ResolutionMessage::General(msg) => &msg.msg,
            ResolutionMessage::AttrPathNotFoundNotInCatalog(msg) => &msg.msg,
            ResolutionMessage::AttrPathNotFoundSystemsNotOnSamePage(msg) => &msg.msg,
            ResolutionMessage::AttrPathNotFoundNotFoundForAllSystems(msg) => &msg.msg,
            ResolutionMessage::ConstraintsTooTight(msg) => &msg.msg,
            ResolutionMessage::Unknown(msg) => &msg.msg,
        }
    }

    pub fn level(&self) -> MessageLevel {
        match self {
            ResolutionMessage::General(msg) => msg.level,
            ResolutionMessage::AttrPathNotFoundNotInCatalog(msg) => msg.level,
            ResolutionMessage::AttrPathNotFoundSystemsNotOnSamePage(msg) => msg.level,
            ResolutionMessage::AttrPathNotFoundNotFoundForAllSystems(msg) => msg.level,
            ResolutionMessage::ConstraintsTooTight(msg) => msg.level,
            ResolutionMessage::Unknown(msg) => msg.level,
        }
    }

    fn attr_path_from_context(context: &HashMap<String, String>) -> String {
        context
            .get("attr_path")
            .cloned()
            .unwrap_or("default_attr_path".into())
    }

    fn valid_systems_from_context(context: &HashMap<String, String>) -> Vec<String> {
        let Some(valid_systems_string) = context.get("valid_systems") else {
            return Vec::new();
        };
        valid_systems_string
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    fn install_id_from_context(context: &HashMap<String, String>) -> String {
        context
            .get("install_id")
            .map(|s| s.to_string())
            .unwrap_or("default_install_id".to_string())
    }
}

impl From<api_types::ResolutionMessageGeneral> for ResolutionMessage {
    fn from(r_msg: api_types::ResolutionMessageGeneral) -> Self {
        match r_msg.type_ {
            api_types::MessageType::General => ResolutionMessage::General(MsgGeneral {
                level: r_msg.level,
                msg: r_msg.message,
            }),
            api_types::MessageType::ResolutionTrace => ResolutionMessage::General(MsgGeneral {
                level: MessageLevel::Trace,
                msg: r_msg.message,
            }),
            api_types::MessageType::AttrPathNotFoundNotInCatalog => {
                ResolutionMessage::AttrPathNotFoundNotInCatalog(MsgAttrPathNotFoundNotInCatalog {
                    level: r_msg.level,
                    msg: r_msg.message,
                    attr_path: Self::attr_path_from_context(&r_msg.context),
                    install_id: Self::install_id_from_context(&r_msg.context),
                })
            },
            api_types::MessageType::AttrPathNotFoundSystemsNotOnSamePage => {
                ResolutionMessage::AttrPathNotFoundSystemsNotOnSamePage(
                    MsgAttrPathNotFoundSystemsNotOnSamePage {
                        level: r_msg.level,
                        msg: r_msg.message,
                        attr_path: Self::attr_path_from_context(&r_msg.context),
                        install_id: Self::install_id_from_context(&r_msg.context),
                        system_groupings: r_msg
                            .context
                            .get("system_groupings")
                            .cloned()
                            .unwrap_or("default_system_groupings".to_string()),
                    },
                )
            },
            api_types::MessageType::AttrPathNotFoundNotFoundForAllSystems => {
                ResolutionMessage::AttrPathNotFoundNotFoundForAllSystems(
                    MsgAttrPathNotFoundNotFoundForAllSystems {
                        level: r_msg.level,
                        msg: r_msg.message,
                        attr_path: Self::attr_path_from_context(&r_msg.context),
                        install_id: Self::install_id_from_context(&r_msg.context),
                        valid_systems: Self::valid_systems_from_context(&r_msg.context),
                    },
                )
            },
            api_types::MessageType::ConstraintsTooTight => {
                ResolutionMessage::ConstraintsTooTight(MsgConstraintsTooTight {
                    level: r_msg.level,
                    msg: r_msg.message,
                })
            },
            api_types::MessageType::Unknown(message_type) => {
                ResolutionMessage::Unknown(MsgUnknown {
                    msg_type: message_type,
                    level: r_msg.level,
                    msg: r_msg.message,
                    context: r_msg.context,
                })
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Resolved package groups and catalog pages
// ---------------------------------------------------------------------------

/// A resolved package group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPackageGroup {
    pub msgs: Vec<ResolutionMessage>,
    pub name: String,
    pub page: Option<CatalogPage>,
}

impl ResolvedPackageGroup {
    pub fn packages(&self) -> impl Iterator<Item = PackageResolutionInfo> {
        if let Some(page) = &self.page {
            page.packages.clone().unwrap_or_default().into_iter()
        } else {
            vec![].into_iter()
        }
    }
}

impl From<api_types::ResolvedPackageGroup> for ResolvedPackageGroup {
    fn from(resolved_package_group: api_types::ResolvedPackageGroup) -> Self {
        Self {
            name: resolved_package_group.name,
            page: resolved_package_group.page.map(CatalogPage::from),
            msgs: resolved_package_group
                .messages
                .into_iter()
                .map(|msg| msg.into())
                .collect::<Vec<_>>(),
        }
    }
}

/// Packages from a single revision of the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPage {
    pub complete: bool,
    pub packages: Option<Vec<PackageResolutionInfo>>,
    pub page: i64,
    pub url: String,
    pub msgs: Vec<ResolutionMessage>,
}

impl From<api_types::CatalogPage> for CatalogPage {
    fn from(catalog_page: api_types::CatalogPage) -> Self {
        Self {
            complete: catalog_page.complete,
            packages: catalog_page.packages,
            page: catalog_page.page,
            url: catalog_page.url,
            msgs: catalog_page
                .messages
                .into_iter()
                .map(|msg| msg.into())
                .collect::<Vec<_>>(),
        }
    }
}

// ---------------------------------------------------------------------------
// Base catalog info
// ---------------------------------------------------------------------------

pub use api_types::{PageInfo, StabilityInfo};

#[derive(Debug, Error)]
#[error(transparent)]
pub struct BaseCatalogUrlError(pub(crate) url::ParseError);

/// A base catalog url as tracked by the catalog server.
///
/// Used to associate expression builds with a catalog page and derive a nix
/// flakeref. The url acts as a key, so we store it as a raw string to avoid
/// escaping changes from [`Url`] parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseCatalogUrl(String);

impl BaseCatalogUrl {
    pub fn as_flake_ref(&self) -> Result<Url, BaseCatalogUrlError> {
        Url::parse(&format!("git+{}&shallow=1", self.0.as_str())).map_err(BaseCatalogUrlError)
    }
}

impl From<String> for BaseCatalogUrl {
    fn from(value: String) -> Self {
        BaseCatalogUrl(value)
    }
}

impl From<&str> for BaseCatalogUrl {
    fn from(value: &str) -> Self {
        BaseCatalogUrl(value.to_owned())
    }
}

impl Display for BaseCatalogUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Clone, PartialEq, derive_more::From, Serialize, Deserialize)]
pub struct BaseCatalogInfo(api_types::BaseCatalogInfo);

impl BaseCatalogInfo {
    /// Name of the default stability.
    pub const DEFAULT_STABILITY: &str = "stable";

    /// Return the url for the newest page with the given stability.
    pub fn url_for_latest_page_with_stability(&self, stability: &str) -> Option<BaseCatalogUrl> {
        let page_info = self.0.scraped_pages.iter().find(|page| {
            page.stability_tags
                .iter()
                .any(|page_stability| page_stability == stability)
        })?;

        let url = BaseCatalogUrl::from(format!(
            "{base_url}?rev={rev}",
            base_url = self.0.base_url,
            rev = page_info.rev
        ));

        Some(url)
    }

    /// Return a url for the "default" stability.
    pub fn url_for_latest_page_with_default_stability(&self) -> Option<BaseCatalogUrl> {
        self.url_for_latest_page_with_stability(Self::DEFAULT_STABILITY)
    }

    /// Return the names of available stabilities.
    pub fn available_stabilities(&self) -> Vec<&str> {
        self.0
            .stabilities
            .iter()
            .map(|stability_info| &*stability_info.name)
            .collect()
    }

    /// Create a mock BaseCatalogInfo for testing.
    #[cfg(feature = "tests")]
    pub fn new_mock() -> Self {
        api_types::BaseCatalogInfo {
            base_url: "https://mock.flox.dev".parse().unwrap(),
            scraped_pages: [
                api_types::PageInfo {
                    rev: "".into(),
                    rev_count: 3,
                    stability_tags: ["not-default".into()].to_vec(),
                },
                api_types::PageInfo {
                    rev: "".into(),
                    rev_count: 2,
                    stability_tags: [
                        BaseCatalogInfo::DEFAULT_STABILITY.into(),
                        "not-default".into(),
                    ]
                    .to_vec(),
                },
            ]
            .to_vec(),
            stabilities: [
                api_types::StabilityInfo {
                    name: BaseCatalogInfo::DEFAULT_STABILITY.into(),
                    ref_: BaseCatalogInfo::DEFAULT_STABILITY.into(),
                },
                api_types::StabilityInfo {
                    name: "not-default".into(),
                    ref_: "not-default".into(),
                },
            ]
            .to_vec(),
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_valid_systems_from_context() {
        let context = [(
            "valid_systems".to_string(),
            "aarch64-darwin,x86_64-linux".to_string(),
        )]
        .into();
        let systems = ResolutionMessage::valid_systems_from_context(&context);
        assert_eq!(systems, vec![
            "aarch64-darwin".to_string(),
            "x86_64-linux".to_string()
        ]);
    }

    #[test]
    fn extracts_valid_systems_from_context_with_suffix_comma() {
        let context = [("valid_systems".to_string(), "aarch64-darwin,".to_string())].into();
        let systems = ResolutionMessage::valid_systems_from_context(&context);
        assert_eq!(systems, vec!["aarch64-darwin".to_string()]);
    }

    #[test]
    fn extracts_valid_systems_from_context_if_empty() {
        let context = [("valid_systems".to_string(), "".to_string())].into();
        let systems = ResolutionMessage::valid_systems_from_context(&context);
        assert_eq!(systems, Vec::<String>::new());
    }
}
