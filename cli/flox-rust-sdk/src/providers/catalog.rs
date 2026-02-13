use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use flox_catalog::{
    ApiResponseValue,
    BaseCatalogInfo,
    BaseCatalogUrl,
    CatalogClient,
    CatalogClientConfig,
    CatalogClientError,
    CatalogMockMode,
    CatalogStoreConfig,
    ClientTrait,
    PackageDetails,
    PackageGroup,
    PackageSystem,
    PublishResponse,
    ResolveError,
    ResolvedPackageGroup,
    SearchError,
    SearchLimit,
    SearchResults,
    StoreInfo,
    StoreInfoResponse,
    StorepathStatusResponse,
    UserBuildPublish,
    VersionsError,
    str_to_catalog_name,
};
use indoc::formatdoc;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use super::publish::CheckedEnvironmentMetadata;
use crate::flox::Flox;

pub const FLOX_CATALOG_MOCK_DATA_VAR: &str = "_FLOX_USE_CATALOG_MOCK";
pub const FLOX_CATALOG_DUMP_DATA_VAR: &str = "_FLOX_CATALOG_DUMP_RESPONSE_FILE";

pub const DEFAULT_CATALOG_URL: &str = "https://api.flox.dev";

pub static GENERATED_DATA: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(std::env::var("GENERATED_DATA").unwrap()));
pub static MANUALLY_GENERATED: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(std::env::var("MANUALLY_GENERATED").unwrap()));

type ResolvedGroups = Vec<ResolvedPackageGroup>;

// Arc allows you to push things into the client from outside the client if necessary
// Mutex allows you to share across threads (necessary because of tokio)
type MockField<T> = Arc<Mutex<T>>;

/// A generic response that can be turned into a [ResponseValue]. This is only necessary for
/// representing error responses.
// TODO: we can handle headers later if we need to
#[derive(Debug, Serialize, Deserialize)]
pub struct GenericResponse<T> {
    pub(crate) inner: T,
    pub(crate) status: u16,
}

impl<T> TryFrom<GenericResponse<T>> for ApiResponseValue<T> {
    type Error = MockDataError;

    fn try_from(value: GenericResponse<T>) -> Result<Self, Self::Error> {
        let status_code = StatusCode::from_u16(value.status)
            .map_err(|_| MockDataError::InvalidData("invalid status code".into()))?;
        let headers = HeaderMap::new();
        Ok(ApiResponseValue::new(value.inner, status_code, headers))
    }
}

/// A mock response
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Resolve(ResolvedGroups),
    // Search results contain a subset of the package result fields, so the more specific type
    // needs to be listed first to deserialize correctly.
    Packages(PackageDetails),
    Search(SearchResults),
    GetStoreInfo(StoreInfoResponse),
    GetStorepathStatus(StorepathStatusResponse),
    Error(GenericResponse<flox_catalog::ApiErrorResponse>),
    Publish(PublishResponse),
    CreatePackage,
    PublishBuild,
    GetBaseCatalog(BaseCatalogInfo),
}

#[derive(Debug, Error)]
pub enum MockDataError {
    /// Failed to read the JSON file pointed at by the _FLOX_USE_CATALOG_MOCK var
    #[error("failed to read mock response file")]
    ReadMockFile(#[source] std::io::Error),
    /// Failed to parse the contents of the mock data file as JSON
    #[error("failed to parse mock data as JSON")]
    ParseJson(#[source] serde_json::Error),
    /// The data was parsed as JSON but it wasn't semantically valid
    #[error("invalid mocked data: {0}")]
    InvalidData(String),
    #[error("couldn't find generated data")]
    GeneratedDataVar,
}

// /// Reads a list of mock responses from disk.
// fn read_mock_responses(path: impl AsRef<Path>) -> Result<VecDeque<Response>, MockDataError> {
//     let mut responses = VecDeque::new();
//     let contents = std::fs::read_to_string(path).map_err(MockDataError::ReadMockFile)?;
//     let deserialized: Vec<Response> =
//         serde_json::from_str(&contents).map_err(MockDataError::ParseJson)?;
//     responses.extend(deserialized);
//     Ok(responses)
// }

/// Either a client for the actual catalog service,
/// or a mock client for testing.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Client {
    Catalog(CatalogClient),
    Mock(MockClient),
}

impl From<CatalogClient> for Client {
    fn from(c: CatalogClient) -> Self {
        Client::Catalog(c)
    }
}

impl From<MockClient> for Client {
    fn from(c: MockClient) -> Self {
        Client::Mock(c)
    }
}

impl ClientTrait for Client {
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, ResolveError> {
        match self {
            Client::Catalog(c) => c.resolve(package_groups).await,
            Client::Mock(c) => c.resolve(package_groups).await,
        }
    }

    async fn search_with_spinner(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: PackageSystem,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        match self {
            Client::Catalog(c) => c.search_with_spinner(search_term, system, limit).await,
            Client::Mock(c) => c.search_with_spinner(search_term, system, limit).await,
        }
    }

    async fn search(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: PackageSystem,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        match self {
            Client::Catalog(c) => c.search(search_term, system, limit).await,
            Client::Mock(c) => c.search(search_term, system, limit).await,
        }
    }

    async fn package_versions(
        &self,
        attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<PackageDetails, VersionsError> {
        match self {
            Client::Catalog(c) => c.package_versions(attr_path).await,
            Client::Mock(c) => c.package_versions(attr_path).await,
        }
    }

    async fn publish_info(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
    ) -> Result<PublishResponse, CatalogClientError> {
        match self {
            Client::Catalog(c) => c.publish_info(catalog_name, package_name).await,
            Client::Mock(c) => c.publish_info(catalog_name, package_name).await,
        }
    }

    async fn create_package(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        original_url: impl AsRef<str> + Send + Sync,
    ) -> Result<(), CatalogClientError> {
        match self {
            Client::Catalog(c) => {
                c.create_package(catalog_name, package_name, original_url)
                    .await
            },
            Client::Mock(c) => {
                c.create_package(catalog_name, package_name, original_url)
                    .await
            },
        }
    }

    async fn publish_build(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        build_info: &UserBuildPublish,
    ) -> Result<(), CatalogClientError> {
        match self {
            Client::Catalog(c) => {
                c.publish_build(catalog_name, package_name, build_info)
                    .await
            },
            Client::Mock(c) => {
                c.publish_build(catalog_name, package_name, build_info)
                    .await
            },
        }
    }

    async fn get_store_info(
        &self,
        derivations: Vec<String>,
    ) -> Result<HashMap<String, Vec<StoreInfo>>, CatalogClientError> {
        match self {
            Client::Catalog(c) => c.get_store_info(derivations).await,
            Client::Mock(c) => c.get_store_info(derivations).await,
        }
    }

    async fn is_publish_complete(
        &self,
        store_paths: &[String],
    ) -> Result<bool, CatalogClientError> {
        match self {
            Client::Catalog(c) => c.is_publish_complete(store_paths).await,
            Client::Mock(c) => c.is_publish_complete(store_paths).await,
        }
    }

    async fn get_base_catalog_info(&self) -> Result<BaseCatalogInfo, CatalogClientError> {
        match self {
            Client::Catalog(c) => c.get_base_catalog_info().await,
            Client::Mock(c) => c.get_base_catalog_info().await,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, derive_more::Display, PartialEq)]
/// The QoS class of a catalog request.
///
/// Referencing macos perfomance classes, described [1].
///
/// [1]: <https://blog.xoria.org/macos-tips-threading/>
pub enum CatalogQoS {
    /// your app’s user interface will stutter if this work is preempted
    #[display("user_interactive")]
    UserInteractive,
    /// the user must wait for this work to finish before they can keep using your app, e.g. loading the contents of a document that was just opened
    #[display("user_initiated")]
    UserInitiated,
    /// used as a fallback for threads which don’t have a QoS class assigned
    #[default]
    #[display("default")]
    Default,
    /// the user knows this work is happening but doesn’t wait for it to finish because they can keep using your app while it’s in progress, e.g. exporting in a video editor, downloading a file in a web browser
    #[display("utility")]
    Utility,
    /// the user doesn’t know this work is happening, e.g. search indexing
    #[display("background")]
    Background,
    /// the user doesn’t know this work is happening, e.g. garbage collection?
    #[display("maintenance")]
    Maintenance,
}

impl CatalogQoS {
    pub fn as_header_pair(&self) -> (String, String) {
        ("X-Flox-QoS-Context".to_string(), self.to_string())
    }
}

/// A catalog client that can be seeded with mock responses
///
/// This is being deprecated in favour of httpmock and no longer supports
/// loading from fixture files.
#[derive(Debug, Default)]
pub struct MockClient {
    // We use a RefCell here so that we don't have to modify the trait to allow mutable access
    // to `self` just to get mock responses out.
    pub mock_responses: MockField<VecDeque<Response>>,
}

impl MockClient {
    /// Create a new mock client.
    pub fn new() -> Self {
        Self {
            mock_responses: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Push a new response into the list of mock responses
    pub fn push_store_info_response(&mut self, resp: StoreInfoResponse) {
        self.mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .push_back(Response::GetStoreInfo(resp));
    }

    /// See [test_helpers::reset_mocks].
    fn reset_mocks(&mut self, responses: impl IntoIterator<Item = Response>) {
        let mut locked_mock_responses = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock");
        locked_mock_responses.clear();
        locked_mock_responses.extend(responses);
    }
}

impl ClientTrait for MockClient {
    async fn resolve(
        &self,
        _package_groups: Vec<PackageGroup>,
    ) -> Result<ResolvedGroups, ResolveError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Resolve(resp)) => Ok(resp),
            Some(Response::Error(err)) => Err(ResolveError::CatalogClientError(
                CatalogClientError::APIError(flox_catalog::ApiError::ErrorResponse(
                    err.try_into()
                        .expect("couldn't convert mock error response"),
                )),
            )),
            _ => panic!("expected resolve response, found {:?}", &mock_resp),
        }
    }

    async fn search_with_spinner(
        &self,
        _search_term: impl AsRef<str> + Send + Sync,
        _system: PackageSystem,
        _limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        self.search(_search_term, _system, _limit).await
    }

    async fn search(
        &self,
        _search_term: impl AsRef<str> + Send + Sync,
        _system: PackageSystem,
        _limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Search(resp)) => Ok(resp),
            // Empty search results and empty packages responses are indistinguishable when
            // deserializing, so we might get a Packages response here as that variant is tried
            // first. That's okay. But if it has actual results of the wrong type, then it's an
            // error.
            Some(Response::Packages(PackageDetails {
                results: _,
                count: Some(0),
            })) => Ok(SearchResults {
                results: vec![],
                count: Some(0),
            }),
            Some(Response::Error(err)) => Err(SearchError::CatalogClientError(
                CatalogClientError::APIError(flox_catalog::ApiError::ErrorResponse(
                    err.try_into()
                        .expect("couldn't convert mock error response"),
                )),
            )),
            _ => panic!("expected search response, found {:?}", &mock_resp),
        }
    }

    async fn package_versions(
        &self,
        _attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<PackageDetails, VersionsError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Packages(resp)) => Ok(resp),
            Some(Response::Error(err)) if err.status == 404 => Err(VersionsError::NotFound),
            Some(Response::Error(err)) => Err(VersionsError::CatalogClientError(
                CatalogClientError::APIError(flox_catalog::ApiError::ErrorResponse(
                    err.try_into()
                        .expect("couldn't convert mock error response"),
                )),
            )),
            _ => panic!("expected packages response, found {:?}", &mock_resp),
        }
    }

    async fn publish_info(
        &self,
        _catalog_name: impl AsRef<str> + Send + Sync,
        _package_name: impl AsRef<str> + Send + Sync,
    ) -> Result<PublishResponse, CatalogClientError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Publish(resp)) => Ok(resp),
            // We don't need to test errors at the moment
            _ => panic!("expected publish response, found {:?}", &mock_resp),
        }
    }

    async fn create_package(
        &self,
        _catalog_name: impl AsRef<str> + Send + Sync,
        _package_name: impl AsRef<str> + Send + Sync,
        _original_url: impl AsRef<str> + Send + Sync,
    ) -> Result<(), CatalogClientError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::CreatePackage) => Ok(()),
            // We don't need to test errors at the moment
            _ => panic!("expected create package response, found {:?}", &mock_resp),
        }
    }

    async fn publish_build(
        &self,
        _catalog_name: impl AsRef<str> + Send + Sync,
        _package_name: impl AsRef<str> + Send + Sync,
        _build_info: &UserBuildPublish,
    ) -> Result<(), CatalogClientError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::PublishBuild) => Ok(()),
            // We don't need to test errors at the moment
            _ => panic!("expected create package response, found {:?}", &mock_resp),
        }
    }

    async fn get_store_info(
        &self,
        _derivations: Vec<String>,
    ) -> Result<HashMap<String, Vec<StoreInfo>>, CatalogClientError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::GetStoreInfo(resp)) => Ok(resp.items),
            _ => panic!("expected get_store_info response, found {:?}", &mock_resp),
        }
    }

    async fn is_publish_complete(
        &self,
        _store_paths: &[String],
    ) -> Result<bool, CatalogClientError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        let statuses = match mock_resp {
            Some(Response::GetStorepathStatus(resp)) => resp,
            _ => panic!("expected get_store_info response, found {:?}", &mock_resp),
        };
        let all_narinfo_available = statuses.items.values().all(|storepath_statuses_for_drv| {
            storepath_statuses_for_drv
                .iter()
                .all(|status| status.narinfo_known)
        });
        Ok(all_narinfo_available)
    }

    async fn get_base_catalog_info(&self) -> Result<BaseCatalogInfo, CatalogClientError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();

        let resp = match mock_resp {
            Some(Response::GetBaseCatalog(resp)) => resp,
            _ => panic!("expected get_base_catalog response, found {:?}", &mock_resp),
        };

        Ok(resp)
    }
}

/// An alias so the flox crate doesn't have to depend on the catalog-api crate
pub type SystemEnum = PackageSystem;

/// All available systems.
pub static ALL_SYSTEMS: [SystemEnum; 4] = [
    SystemEnum::Aarch64Darwin,
    SystemEnum::Aarch64Linux,
    SystemEnum::X8664Darwin,
    SystemEnum::X8664Linux,
];

#[derive(Debug, Clone, PartialEq)]
pub enum SearchTerm {
    Clean(String),
    VersionStripped(String),
}

impl SearchTerm {
    pub fn from_arg(search_term: &str) -> Self {
        match search_term.split_once('@') {
            Some((package, _version)) => SearchTerm::VersionStripped(package.to_string()),
            None => SearchTerm::Clean(search_term.to_string()),
        }
    }
}

/// Hardcoded locked URL for publishes of expression builds
///
/// Outisde of tests this should be replaced by a mechanism that fetches an actual locked URL,
/// in correspondence with the catalog server.
pub fn mock_base_catalog_url() -> BaseCatalogUrl {
    BaseCatalogUrl::from(env!("TESTING_BASE_CATALOG_URL"))
}

/// Derive the nixpkgs url to be used for builds.
/// If a stability is provided, try to retrieve a url for that stability from the catalog.
/// Else, if we can derive a stability from the toplevel group of the environment, use that.
/// Otherwise attrr
pub async fn base_catalog_url_for_stability_arg(
    stability: Option<&str>,
    base_catalog_info_fut: impl IntoFuture<Output = Result<BaseCatalogInfo, CatalogClientError>>,
    toplevel_derived_url: Option<&BaseCatalogUrl>,
) -> Result<BaseCatalogUrl, CatalogClientError> {
    let url = match (stability, toplevel_derived_url) {
        (Some(stability), _) => {
            let base_catalog_info = base_catalog_info_fut.await?;
            let make_error_message = || {
                let available_stabilities = base_catalog_info.available_stabilities().join(", ");
                formatdoc! {"
                    Stability '{stability}' does not exist (or has not yet been populated).
                    Available stabilities are: {available_stabilities}
                "}
            };

            let url = base_catalog_info
                .url_for_latest_page_with_stability(stability)
                .ok_or_else(|| CatalogClientError::StabilityError(make_error_message()))?;

            info!(%url, %stability, "using page from user provided stability");
            url
        },
        (None, Some(toplevel_derived_url)) => {
            info!(url=%toplevel_derived_url, "using nixpkgs derived from toplevel group");
            toplevel_derived_url.clone()
        },
        (None, None) => {
            let base_catalog_info = base_catalog_info_fut.await?;

            let make_error_message = || {
                let available_stabilities = base_catalog_info.available_stabilities().join(", ");
                formatdoc! {"
                    The default stability {} does not exist (or has not yet been populated).
                    Available stabilities are: {available_stabilities}
                ", BaseCatalogInfo::DEFAULT_STABILITY}
            };

            let url = base_catalog_info
                .url_for_latest_page_with_default_stability()
                .ok_or_else(|| CatalogClientError::StabilityError(make_error_message()))?;

            info!(%url, "using page from default stability");
            url
        },
    };
    Ok(url)
}

/// Returns the nixpkgs URL used for builds and publishes.
pub async fn get_base_nixpkgs_url(
    flox: &Flox,
    stability: Option<&str>,
    env_metadata: &CheckedEnvironmentMetadata,
) -> Result<BaseCatalogUrl, CatalogClientError> {
    let base_catalog_info_fut = flox.catalog_client.get_base_catalog_info();

    base_catalog_url_for_stability_arg(
        stability,
        base_catalog_info_fut,
        env_metadata.toplevel_catalog_ref.as_ref(),
    )
    .await
}

pub mod test_helpers {
    use pollster::FutureExt;
    use tempfile::TempDir;

    use super::*;
    use crate::flox::Flox;
    use crate::flox::test_helpers::{PublishTestUser, test_token_from_floxhub_test_users_file};
    use crate::providers::auth::{Auth, AuthProvider};

    pub static UNIT_TEST_GENERATED: LazyLock<PathBuf> =
        LazyLock::new(|| PathBuf::from(std::env::var("UNIT_TEST_GENERATED").unwrap()));

    /// Read a package version from the generated versions file.
    ///
    /// The versions file is generated by `just gen-unit-data-no-publish` which
    /// queries the production catalog API for the latest versions of packages
    /// used in tests.
    pub fn get_prod_package_version(package: &str) -> String {
        let versions_file = UNIT_TEST_GENERATED.join("latest_prod_versions.json");
        let contents = std::fs::read_to_string(&versions_file).unwrap_or_else(|_| {
            panic!(
                "failed to read {} - run `just gen-unit-data-no-publish` first",
                versions_file.display()
            )
        });
        let versions: serde_json::Value =
            serde_json::from_str(&contents).expect("failed to parse latest_prod_versions.json");
        versions[package]
            .as_str()
            .unwrap_or_else(|| panic!("package {} not found in versions file", package))
            .to_string()
    }

    /// Whether to record mock data and in which situations.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    enum RecordMockData {
        /// Only record new mock data if it's missing.
        Missing,
        /// Don't record new mock data.
        #[default]
        False,
        /// Re-record all mock data.
        Force,
    }

    /// Returns in which circumstances mock data should be recorded based on
    /// the value of the `_FLOX_UNIT_TEST_RECORD` environment variable.
    ///
    /// Values of "missing", "true", or "1" will generate a recording for a
    /// missing mock. An unset variable or a value of "false" will only replay
    /// existing recordings. The value "force" will unconditionally regenerate
    /// mock data. Any other value will cause a panic.
    fn get_record_directive() -> RecordMockData {
        let s = std::env::var("_FLOX_UNIT_TEST_RECORD").unwrap_or_default();
        match s.as_str() {
            "true" | "missing" | "1" => RecordMockData::Missing,
            "" | "false" => RecordMockData::False,
            "force" => RecordMockData::Force,
            _ => panic!("invalid value of _FLOX_UNIT_TEST_RECORD"),
        }
    }

    /// Create a mock client that will replay from a given file.
    ///
    /// Tests must be run with `#[tokio::test(flavor = "multi_thread")]` to
    /// allow the `MockServer` to run in another thread.
    ///
    /// This should be used to replay mocks generated by mk_data.
    /// In general, auto_recording_catalog_client is preferred.
    pub async fn catalog_replay_client(path: impl AsRef<Path>) -> Client {
        let catalog_config = CatalogClientConfig {
            catalog_url: "https://not_used".to_string(),
            floxhub_token: None,
            extra_headers: Default::default(),
            mock_mode: CatalogMockMode::Replay(path.as_ref().to_path_buf()),
            user_agent: None,
        };
        Client::Catalog(
            CatalogClient::new(catalog_config).expect("failed to create catalog client"),
        )
    }

    /// Create a mock client that will either record to or replay from a given
    /// file name depending on whether `_FLOX_UNIT_TEST_RECORD` is set.
    ///
    /// Tests must be run with `#[tokio::test(flavor = "multi_thread")]` to
    /// allow the `MockServer` to run in another thread.
    pub fn auto_recording_catalog_client(filename: &str) -> Client {
        let auth = Auth::from_tempdir_and_token(TempDir::new().unwrap(), None);
        let record = get_record_directive();
        auto_recording_client_inner(
            filename,
            DEFAULT_CATALOG_URL,
            PublishTestUser::NoCatalogs,
            &auth,
            record,
        )
    }

    /// Similar to [auto_recording_catalog_client] but authenticates against a dev
    /// instance of the catalog-server using a token from
    pub fn auto_recording_catalog_client_for_authed_local_services(
        mut flox: Flox,
        user: PublishTestUser,
        filename: &str,
    ) -> (Flox, Auth) {
        let record = get_record_directive();

        // FloxHub can load test users from a file, so we read the
        // corresponding token from that file. Just make sure you start
        // FloxHub with _FLOXHUB_TEST_USER_ROLES pointed at this file.
        let token = test_token_from_floxhub_test_users_file(user);

        flox.floxhub_token = Some(token);
        let auth = Auth::from_flox(&flox).unwrap();
        let base_url = "http://localhost:8000";
        let client = auto_recording_client_inner(filename, base_url, user, &auth, record);
        flox.catalog_client = client;

        (flox, auth)
    }

    /// Generic handler for creating a mock catalog client.
    fn auto_recording_client_inner(
        filename: &str,
        base_url: &str,
        user: PublishTestUser,
        auth: &Auth,
        record: RecordMockData,
    ) -> Client {
        let mut path = UNIT_TEST_GENERATED.join(filename);
        path.set_extension("yaml");
        let (mock_mode, catalog_url) = match record {
            RecordMockData::Missing => {
                // TODO(zmitchell, 2025-07-23): it would be convenient if we
                // also detected empty mock files as "missing" since a failed
                // test will create the file but won't get a chance to write
                // the contents (which is good, we don't want a recording of
                // a failed test).
                if path.exists() {
                    // Use an existing recording
                    (
                        CatalogMockMode::Replay(path),
                        "https://not_used".to_string(),
                    )
                } else {
                    // Generate a new recording
                    (CatalogMockMode::Record(path), base_url.to_string())
                }
            },
            RecordMockData::False => {
                // Use an existing recording
                (
                    CatalogMockMode::Replay(path),
                    "https://not_used".to_string(),
                )
            },
            RecordMockData::Force => {
                // Regenerate existing recording
                (CatalogMockMode::Record(path), base_url.to_string())
            },
        };

        let catalog_config = CatalogClientConfig {
            catalog_url,
            floxhub_token: auth.token().map(|token| token.secret().to_string()),
            extra_headers: Default::default(),
            mock_mode: mock_mode.clone(),
            user_agent: None,
        };
        let client_inner =
            CatalogClient::new(catalog_config).expect("failed to create catalog client");
        let mut client = Client::Catalog(client_inner);
        if matches!(mock_mode, CatalogMockMode::Record(_)) && user == PublishTestUser::WithCatalogs
        {
            ensure_test_catalogs_exist(&client).block_on();
            if let Client::Catalog(ref mut client_inner) = client {
                // Delete all of the setup operations from the recording.
                client_inner.reset_recording();
            }
        }
        client
    }

    /// Clear mock responses and then load provided responses
    pub fn reset_mocks(client: &mut Client, responses: Vec<Response>) {
        let Client::Mock(client) = client else {
            panic!("mocks can only be used with a MockClient");
        };

        client.reset_mocks(responses);
    }

    /// Create a catalog with the given name and config.
    ///
    /// Will continue with config and not return an error if the catalog already exists.
    pub async fn create_catalog_with_config(
        client: &Client,
        name: &str,
        config: &CatalogStoreConfig,
        exists_ok: bool,
    ) -> Result<(), CatalogClientError> {
        let Client::Catalog(client) = client else {
            panic!("can only be used with a CatalogClient");
        };

        // This also performs validation that the name meets the catalog name requirements.
        let catalog_name = str_to_catalog_name(name)?;

        let resp = client
            .api()
            .create_catalog_api_v1_catalog_catalogs_post(&catalog_name)
            .await;
        match resp {
            Ok(_) => {},
            // Continue if already exists.
            Err(e) if e.status() == Some(StatusCode::CONFLICT) => {
                if !exists_ok {
                    return Err(CatalogClientError::Other(
                        "catalog already existed".to_string(),
                    ));
                }
                // return Ok(());
            },
            Err(e) => {
                return Err(CatalogClientError::APIError(e));
            },
        }

        client
            .api()
            .set_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_put(
                &catalog_name,
                config,
            )
            .await
            .map_err(CatalogClientError::APIError)?;

        Ok(())
    }

    pub const TEST_READ_WRITE_CATALOG_NAME: &str = "publish_tests_read_write";
    pub const TEST_READ_ONLY_CATALOG_NAME: &str = "publish_tests_read_only";
    pub const TEST_USER_NO_CATALOG: &str = "test_user_no_catalogs";
    pub const TEST_USER_WITH_EXISTING_CATALOG: &str = "test1";

    /// Ensures that the test org catalog exists, ignoring errors that arise from
    /// trying to create it when it already exists.
    pub async fn ensure_test_catalogs_exist(client: &Client) {
        let config = CatalogStoreConfig::MetaOnly;
        create_catalog_with_config(client, TEST_READ_WRITE_CATALOG_NAME, &config, true)
            .await
            .expect("failed to create read/write test catalog");
        create_catalog_with_config(client, TEST_READ_ONLY_CATALOG_NAME, &config, true)
            .await
            .expect("failed to create read only test catalog");
        create_catalog_with_config(client, TEST_USER_WITH_EXISTING_CATALOG, &config, true)
            .await
            .expect("failed to create personal catalog for user with existing catalog");
    }
}

#[cfg(test)]
mod tests {

    use pollster::FutureExt;

    use super::*;
    use crate::flox::test_helpers::{PublishTestUser, flox_instance};
    use crate::providers::catalog::test_helpers::{
        auto_recording_catalog_client_for_authed_local_services,
        create_catalog_with_config,
    };

    #[test]
    fn search_term_without_version() {
        assert_eq!(
            SearchTerm::from_arg("hello"),
            SearchTerm::Clean("hello".to_string())
        );
    }

    #[test]
    fn search_term_with_version_specifiers() {
        let inputs = vec!["hello@", "hello@1.x", "hello@>=1", "hello@>1 <3"];
        for input in inputs {
            assert_eq!(
                SearchTerm::from_arg(input),
                SearchTerm::VersionStripped("hello".to_string())
            );
        }
    }

    #[test]
    fn search_term_with_at_in_attr_path() {
        let inputs = vec![
            "nodePackages.@angular/cli",
            "nodePackages.@angular/cli@_at_angular_slash_cli-18.0.2",
        ];
        for input in inputs {
            assert_eq!(
                SearchTerm::from_arg(input),
                // Catalog service indexes on the last tuple of `attr_path` so neither
                // of these searches will work. However at least the behaviour with
                // `split_once("@")` is consistently wrong whereas `rsplit_once("@")`
                // would be inconsistently wrong.
                SearchTerm::VersionStripped("nodePackages.".to_string())
            );
        }
    }

    #[test]
    fn can_push_responses_outside_of_client() {
        let client = MockClient::new();
        {
            // Need to drop the mutex guard otherwise `resolve` will block trying to read
            // the queue of mock responses
            let resp_handle = client.mock_responses.clone();
            let mut responses = resp_handle.lock().unwrap();
            responses.push_back(Response::Resolve(vec![]));
        }
        let resp = client.resolve(vec![]).block_on().unwrap();
        assert!(resp.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn creates_new_catalog() {
        let (flox, _tmpdir) = flox_instance();
        let (flox, _auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::NoCatalogs,
            "creates_new_catalog",
        );
        let catalog_name_raw = "dummy_unused_catalog";
        // Makes two calls:
        // - POST to /catalog/catalogs?name=<catalog_name_raw>
        // - PUT to /catalog/catalogs/<catalog_name_raw>/store/config
        create_catalog_with_config(
            &flox.catalog_client,
            catalog_name_raw,
            &CatalogStoreConfig::MetaOnly,
            false,
        )
        .await
        .expect("request to create new catalog failed");
        // FIXME(zmitchell, 2025-07-25): I wanted to test that trying to create the
        // catalog a second time returns 409, but for some reason I get back a
        // success, which makes this fail. I haven't been able to tell if that's an
        // error on the catalog-server side or a problem with httpmock where the
        // path of the request matches perfectly.
        // let Client::Catalog(client) = flox.catalog_client else {
        //     panic!("need a real catalog client");
        // };
        // let name = api_types::Name::from_str(catalog_name_raw).expect("invalid catalog name");
        // let resp = client
        //     .client
        //     .create_catalog_api_v1_catalog_catalogs_post(&name)
        //     .await;
        // eprintln!("response: {:?}", resp);
        // match resp {
        //     Ok(_) => panic!("catalog wasn't created the first time"),
        //     Err(e) if e.status() == Some(StatusCode::CONFLICT) => {},
        //     Err(e) => {
        //         panic!("encountered other error: {}", e)
        //     },
        // }
    }
}
